//! Conservative negative resource detection without allocating a CSSOM tree.
//! A positive answer still goes through the existing Stylo validation and limits.
use cssparser::{Parser, ParserInput, Token};

const MAX_NESTING: usize = 64;

#[derive(Clone, Copy)]
enum Purpose {
    Resources,
    Fonts,
}

pub(super) fn may_require_resources(source: &str) -> bool {
    // Every supported resource spelling has an at-keyword or function opener,
    // including escaped names. This avoids tokenizing ordinary declaration lists.
    if !source.contains('@') && !source.contains('(') {
        return false;
    }
    scan_source(source, Purpose::Resources)
}

pub(super) fn may_have_font_faces(source: &str) -> bool {
    if !source.contains('@') {
        return false;
    }
    scan_source(source, Purpose::Fonts)
}

fn scan_source(source: &str, purpose: Purpose) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    scan(&mut parser, purpose, 0)
}

fn scan(parser: &mut Parser<'_, '_>, purpose: Purpose, depth: usize) -> bool {
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match token {
            // At-rule serializers can introduce URL notation (for example a
            // quoted @namespace URI). Keep all at-rules on the existing resource
            // parser path rather than predicting each native serialization.
            Token::AtKeyword(name)
                if matches!(purpose, Purpose::Resources)
                    || name.eq_ignore_ascii_case("font-face") =>
            {
                return true;
            }
            Token::UnquotedUrl(_) if matches!(purpose, Purpose::Resources) => return true,
            Token::Function(ref name)
                if matches!(purpose, Purpose::Resources)
                    && (name.eq_ignore_ascii_case("url")
                        || name.eq_ignore_ascii_case("image-set")
                        || name.eq_ignore_ascii_case("-webkit-image-set")) =>
            {
                return true;
            }
            Token::Function(_)
            | Token::CurlyBracketBlock
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock => {
                // Never turn a traversal limit into a false resource-free answer.
                // Keep this negative preflight's stack bounded and let the existing
                // parser/resource pipeline decide deeper or malformed input.
                if depth >= MAX_NESTING {
                    return true;
                }
                match parser
                    .parse_nested_block::<_, _, ()>(|nested| Ok(scan(nested, purpose, depth + 1)))
                {
                    Ok(false) => {}
                    Ok(true) | Err(_) => return true,
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::cssom::{stylesheet_has_font_sources, stylesheet_requires_resources};
    const BASE: &str = "https://example.test/style.css";

    #[test]
    fn large_resource_free_sheet_does_not_enter_font_rule_traversal() {
        // Exact rule count and text from css-cascade/tons-of-declarations-crash.html.
        let css = "input { border: 1px solid }".repeat(2 << 16);
        assert!(!stylesheet_requires_resources(&css, BASE).unwrap());
        assert!(!stylesheet_has_font_sources(&css, BASE).unwrap());
        // Resource-looking author text must not defeat the negative scan.
        let css = "/* @font-face{src:url(no)} */ input::before { content: '@import url(no) image-set(no)'; border: 1px solid }".repeat(8193);
        assert!(!stylesheet_requires_resources(&css, BASE).unwrap());
        assert!(!stylesheet_has_font_sources(&css, BASE).unwrap());
    }

    #[test]
    fn resource_preflight_preserves_escaped_nested_and_imported_sources() {
        for css in [
            r"@\69mport 'child.css';",
            "@import url(child.css);",
            "@namespace prefix 'https://example.test/ns';",
            r"input { background-image: u\72l(../image.png) }",
            "input { background-image: URL('../image.png') }",
            "@media screen { @supports(display:block) { input { background-image: image-set('image.png' 1x) } } }",
            "input { background-image: -webkit-image-set(url(image.png) 1x) }",
            r"@media screen { @supports(display:block) { @\66ont-face { font-family: Demo; src: local('Local Face') } } }",
            "@FONT-FACE { font-family: Demo; src: url(font.woff2) format('woff2') }",
        ] {
            assert!(stylesheet_requires_resources(css, BASE).unwrap(), "{css}");
        }
        assert!(
            stylesheet_has_font_sources(
                r"@media screen { @\66ont-face { font-family: Demo; src: local('Local Face') } }",
                BASE,
            )
            .unwrap()
        );
        for css in [
            "@import 'child.css' supports(unknown-property: unknown);",
            "input { not-a-property: url(ignored.png) }",
            "input { filter: url(#local); content: 'url(ignored.png)' }",
            "@font-face { font-family: Demo; src: url(font.svg) format(svg) }",
        ] {
            // Unsupported font URLs remain resource references under the existing
            // resource predicate; font-source selection independently filters them.
            if css.starts_with("@font-face") {
                assert!(!stylesheet_has_font_sources(css, BASE).unwrap());
            } else {
                assert!(!stylesheet_requires_resources(css, BASE).unwrap(), "{css}");
            }
        }
    }

    #[test]
    fn negative_preflight_preserves_base_errors_and_positive_traversal_bound() {
        assert!(stylesheet_requires_resources("input { color: green }", "not a URL").is_err());
        assert!(stylesheet_has_font_sources("input { color: green }", "not a URL").is_err());
        let css = format!(
            "@font-face {{ font-family: Demo; src: local('Local Face') }}{}",
            "input { border: 1px solid }".repeat(8193)
        );
        assert_eq!(
            stylesheet_has_font_sources(&css, BASE).unwrap_err(),
            "font source traversal exceeds native bounds"
        );
        // Harmless nested syntax stays negative at ordinary depth. At the
        // preflight stack bound, it must defer instead of claiming absence;
        // this source has no at-keyword or URL to trigger an earlier positive.
        assert!(!super::may_require_resources(
            "input { width: calc(1px + var(--width)); }"
        ));
        let deep_functions = format!(
            "input {{ --value: {}0{} }}",
            "harmless(".repeat(70),
            ")".repeat(70)
        );
        assert!(super::may_require_resources(&deep_functions));
        // The full parser still decides that the unknown function has no URL.
        assert!(!stylesheet_requires_resources(&deep_functions, BASE).unwrap());
        let deep_groups = format!(
            "{}input {{ color: green }}{}",
            "@media all {".repeat(70),
            "}".repeat(70)
        );
        assert!(super::may_have_font_faces(&deep_groups));
        assert!(!stylesheet_has_font_sources(&deep_groups, BASE).unwrap());
        // Keep an actual valid resource nested beyond the preflight bound too.
        let css = format!(
            "{}input {{ background: url(image.png) }}{}",
            "@media all {".repeat(70),
            "}".repeat(70)
        );
        assert!(stylesheet_requires_resources(&css, BASE).unwrap());
    }
}
