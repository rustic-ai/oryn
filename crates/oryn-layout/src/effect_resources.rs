//! External effect resources use the ordinary parent-authorized CSS image path.
//! Fragment-only references never become network requests.
use cssparser::{Parser, ParserInput, Token};
use std::collections::BTreeSet;

fn urls<'i>(
    parser: &mut Parser<'i, '_>,
    depth: usize,
    result: &mut Vec<String>,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    if depth > 32 {
        return Err(parser.new_custom_error(()));
    }
    while !parser.is_exhausted() {
        parser.skip_whitespace();
        let start = parser.position();
        match parser.next()?.clone() {
            Token::BadString(_) | Token::BadUrl(_) => return Err(parser.new_custom_error(())),
            Token::QuotedString(_) if !closed_string_token(parser.slice_from(start)) => {
                return Err(parser.new_custom_error(()));
            }
            Token::UnquotedUrl(value) => result.push(value.to_string()),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let value = parser.parse_nested_block(|nested| {
                    nested.skip_whitespace();
                    let start = nested.position();
                    let value = nested.expect_string_cloned()?.to_string();
                    if !closed_string_token(nested.slice_from(start)) {
                        return Err(nested.new_custom_error(()));
                    }
                    nested.expect_exhausted()?;
                    Ok(value)
                })?;
                result.push(value);
            }
            Token::Function(_) => {
                parser.parse_nested_block(|nested| urls(nested, depth + 1, result))?
            }
            _ => {}
        }
        if result.len() > 256 {
            return Err(parser.new_custom_error(()));
        }
    }
    Ok(())
}
// cssparser recovers an EOF-terminated quoted string as a string token.
// Computed resource values must not publish dependencies from such a suffix.
fn closed_string_token(raw: &str) -> bool {
    let raw = raw.trim_start();
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || !matches!(bytes[0], b'\'' | b'"') || bytes.last() != Some(&bytes[0]) {
        return false;
    }
    let escapes = bytes[..bytes.len() - 1]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    escapes % 2 == 0
}
fn collect(value: &str, base: &url::Url, result: &mut BTreeSet<String>) {
    if value.len() > 64 * 1024 {
        return;
    }
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut found = Vec::new();
    if urls(&mut parser, 0, &mut found).is_err() {
        return;
    }
    for value in found {
        if value.trim_start().starts_with('#') {
            continue;
        }
        let Ok(mut resource) = base.join(&value) else {
            continue;
        };
        let mut document = base.clone();
        document.set_fragment(None);
        let has_fragment = resource.fragment().is_some();
        resource.set_fragment(None);
        if has_fragment && resource == document {
            continue;
        }
        result.insert(resource.to_string());
    }
}
pub(super) fn discover(styles: &super::ComputedStyles, base: &str, result: &mut BTreeSet<String>) {
    let Ok(base) = url::Url::parse(base) else {
        return;
    };
    for style in styles.values() {
        if style.get("display").is_some_and(|value| value == "none") {
            continue;
        }
        for property in [
            "filter",
            "backdrop-filter",
            "mask-image",
            "mask-border-source",
        ] {
            if let Some(value) = style.get(property) {
                collect(value, &base, result);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_filter_fragment_is_not_part_of_the_fetch_and_local_references_are_skipped() {
        let base = url::Url::parse("https://example.test/docs/page.html").unwrap();
        let mut result = BTreeSet::new();
        collect(
            "url(../filter.svg#bar) blur(2px) url(#local)",
            &base,
            &mut result,
        );
        collect(
            "url(https://example.test/docs/page.html#same)",
            &base,
            &mut result,
        );
        assert_eq!(
            result,
            BTreeSet::from(["https://example.test/filter.svg".into()])
        );
    }
    #[test]
    fn escaped_and_nested_urls_preserve_computed_stylesheet_authority() {
        let base = url::Url::parse("https://document.test/").unwrap();
        let mut result = BTreeSet::new();
        collect(
            r#"image-set(url("https://sheet.test/css/f\69 lter.svg#f") 1x)"#,
            &base,
            &mut result,
        );
        assert_eq!(
            result,
            BTreeSet::from(["https://sheet.test/css/filter.svg".into()])
        );
    }
    #[test]
    fn invalid_css_does_not_publish_partial_dependencies() {
        let mut result = BTreeSet::new();
        collect(
            "url('https://example.test/a') url(\"unterminated)",
            &url::Url::parse("https://example.test/").unwrap(),
            &mut result,
        );
        assert!(result.is_empty());
    }
    #[test]
    fn quoted_url_comments_and_escapes_remain_valid_without_partial_eof_publication() {
        let base = url::Url::parse("https://example.test/").unwrap();
        let mut result = BTreeSet::new();
        collect(
            r#"image-set(/* quote " in comment */ url("a.svg") 1x)"#,
            &base,
            &mut result,
        );
        assert!(result.contains("https://example.test/a.svg"));
        let accepted = result.clone();
        collect(
            r#"url("b.svg") image-set("unterminated"#,
            &base,
            &mut result,
        );
        assert_eq!(result, accepted);
        collect(r#"url("c\"d.svg")"#, &base, &mut result);
        assert!(result.contains("https://example.test/c%22d.svg"));
    }
}
