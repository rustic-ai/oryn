//! Discover resource URL syntax without treating strings or comments as CSS.
use cssparser::{Parser, ParserInput, Token};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CssResourceDestination {
    Image,
    Font,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CssResourceReference {
    pub url: String,
    pub destination: CssResourceDestination,
}

/// Return decoded URL references in source order. This is lexical discovery;
/// callers still decide whether a declaration applies and whether to fetch it.
/// Local fragments and empty references are excluded from external resources.
pub fn resource_urls(source: &str) -> Vec<String> {
    resource_references(source)
        .into_iter()
        .map(|reference| reference.url)
        .collect()
}

/// Discover references with their containing font-face context. A font rule
/// does not change the destination of sibling image declarations.
pub fn resource_references(source: &str) -> Vec<CssResourceReference> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut urls = Vec::new();
    collect(&mut parser, false, CssResourceDestination::Image, &mut urls);
    urls
}

fn push_external(
    urls: &mut Vec<CssResourceReference>,
    value: &str,
    destination: CssResourceDestination,
) {
    if !value.is_empty() && !value.starts_with('#') {
        urls.push(CssResourceReference {
            url: value.to_owned(),
            destination,
        });
    }
}

fn collect(
    parser: &mut Parser<'_, '_>,
    image_set: bool,
    destination: CssResourceDestination,
    urls: &mut Vec<CssResourceReference>,
) {
    let mut font_face_prelude = false;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::AtKeyword(name) => font_face_prelude = name.eq_ignore_ascii_case("font-face"),
            Token::Semicolon => font_face_prelude = false,
            Token::UnquotedUrl(value) => push_external(urls, &value, destination),
            // Only direct string children of image-set are image references.
            // For example, the string in type("image/png") is not a URL.
            Token::QuotedString(value) if image_set => push_external(urls, &value, destination),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let value = parser.parse_nested_block::<_, _, ()>(|nested| {
                    Ok(nested.expect_string()?.to_string())
                });
                if let Ok(value) = value {
                    push_external(urls, &value, destination);
                }
            }
            Token::Function(name) => {
                let image_set = name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set");
                let _ = parser.parse_nested_block::<_, _, ()>(|nested| {
                    collect(nested, image_set, destination, urls);
                    Ok(())
                });
            }
            Token::CurlyBracketBlock => {
                let child_destination = if font_face_prelude {
                    CssResourceDestination::Font
                } else {
                    destination
                };
                font_face_prelude = false;
                let _ = parser.parse_nested_block::<_, _, ()>(|nested| {
                    collect(nested, false, child_destination, urls);
                    Ok(())
                });
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock => {
                let _ = parser.parse_nested_block::<_, _, ()>(|nested| {
                    collect(nested, false, destination, urls);
                    Ok(())
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resource_urls;

    #[test]
    fn stylesheet_resource_requirement_uses_parsed_css() {
        use crate::cssom::stylesheet_requires_resources;
        let base = "https://example.test/style.css";
        for css in [
            "",
            "/* url(ignored.png) */ p { color: green }",
            "p::before { content: 'url(ignored.png)' }",
        ] {
            assert!(!stylesheet_requires_resources(css, base).unwrap(), "{css}");
        }
        for css in [
            "@import 'child.css';",
            "p { background-image: url(image.png) }",
            r"p { background-image: u\72l(image.png) }",
        ] {
            assert!(stylesheet_requires_resources(css, base).unwrap(), "{css}");
        }
    }

    #[test]
    fn font_destination_is_scoped_to_its_rule() {
        use super::{
            CssResourceDestination::{Font, Image},
            resource_references,
        };
        let references = resource_references(
            r#"
            @media screen {
                @font-face { src: url(font.woff); }
                .a { content: "@font-face"; background: url(image.png); }
            }
            .b { background: url(other.png); }
        "#,
        );
        assert_eq!(
            references
                .into_iter()
                .map(|reference| (reference.url, reference.destination))
                .collect::<Vec<_>>(),
            [
                ("font.woff".into(), Font),
                ("image.png".into(), Image),
                ("other.png".into(), Image)
            ]
        );
    }

    #[test]
    fn ignores_comments_strings_and_local_fragments() {
        assert_eq!(
            resource_urls(
                r##"
            /* url(comment.png) */
            .a { content: "url(string.png)"; filter: url(#paint); }
            .b { background: url(real.png); mask: url(""); }
        "##
            ),
            ["real.png"]
        );
    }

    #[test]
    fn decodes_escaped_names_values_and_quoted_parentheses() {
        assert_eq!(
            resource_urls(
                r#"
            .a { background: u\72l(im\61ge.png), URL("image (2).png"),
                 url(image\(3\).png); }
        "#
            ),
            ["image.png", "image (2).png", "image(3).png"]
        );
    }

    #[test]
    fn walks_nested_rules_and_image_set_candidates_in_order() {
        assert_eq!(
            resource_urls(
                r#"
            @media screen {
                .a { background: image-set("one.png" 1x type("image/png"),
                     url(two.png) 2x), cross-fade(url(a.png), url(b.png), 50%); }
            }
            @font-face { src: local("url(fake.woff)"), url(font.woff); }
        "#
            ),
            ["one.png", "two.png", "a.png", "b.png", "font.woff"]
        );
    }

    #[test]
    fn malformed_url_does_not_consume_following_declarations() {
        assert_eq!(
            resource_urls(
                r#"
            .a { background: url("bad.png" junk); mask: url(good.svg); }
        "#
            ),
            ["good.svg"]
        );
    }
}
