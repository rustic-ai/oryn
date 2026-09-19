//! Shared native clipboard/drag markup URL and CSS processing. CSS is tokenized
//! with the engine's pinned cssparser, so escaped identifiers/URLs are decoded
//! before policy decisions. Never inspect public DOM baseURI or style objects.
use super::*;
use cssparser::{Delimiter, Parser, ParserInput, ToCss, Token};
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XLINK: &str = "http://www.w3.org/1999/xlink";
const HTML: &str = "http://www.w3.org/1999/xhtml";
const SVG: &str = "http://www.w3.org/2000/svg";
const MATH: &str = "http://www.w3.org/1998/Math/MathML";

fn resource_url(value: &str, base: Option<&url::Url>, navigation: bool) -> Option<String> {
    // WHATWG URL parsing removes embedded tab/newline and trims C0 ends.
    // Resolve unknown-base relatives against a private sentinel solely to
    // classify their scheme; never expose the sentinel in serialized output.
    let sentinel = url::Url::parse("https://clipboard.invalid/").ok()?;
    let parsed = base.unwrap_or(&sentinel).join(value).ok()?;
    let allowed = match parsed.scheme() {
        "http" | "https" | "ftp" => true,
        "mailto" | "tel" | "sms" => navigation,
        "blob" => true, // Native Blob/resource service still checks ownership.
        "data" if !navigation => {
            let mime = parsed.path().split([',', ';']).next()?.to_ascii_lowercase();
            matches!(
                mime.as_str(),
                "image/png"
                    | "image/jpeg"
                    | "image/gif"
                    | "image/webp"
                    | "image/avif"
                    | "image/bmp"
                    | "image/x-icon"
            )
        }
        _ => false,
    };
    if !allowed {
        return None;
    }
    if value.trim().starts_with('#') {
        return Some(value.trim().to_owned());
    }
    if base.is_some() || url::Url::parse(value).is_ok() {
        Some(parsed.to_string())
    } else {
        Some(
            value
                .trim_matches(|character: char| character <= ' ')
                .replace(['\t', '\n', '\r'], ""),
        )
    }
}
fn css_tokens<'i>(
    parser: &mut Parser<'i, '_>,
    base: Option<&url::Url>,
    depth: usize,
    image_strings: bool,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    if depth > 32 {
        return Err(parser.new_custom_error(()));
    }
    let mut out = String::new();
    while !parser.is_exhausted() {
        let start = parser.position();
        let token = parser.next_including_whitespace_and_comments()?.clone();
        match token.clone() {
            Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket
            | Token::CDO
            | Token::CDC => return Err(parser.new_custom_error(())),
            Token::UnquotedUrl(value) => {
                let url =
                    resource_url(&value, base, false).ok_or_else(|| parser.new_custom_error(()))?;
                out.push_str("url(");
                out.push_str(&Token::QuotedString(url.into()).to_css_string());
                out.push(')');
            }
            Token::QuotedString(value) if image_strings => {
                let url =
                    resource_url(&value, base, false).ok_or_else(|| parser.new_custom_error(()))?;
                out.push_str(&Token::QuotedString(url.into()).to_css_string());
            }
            Token::Function(name) => {
                if name.eq_ignore_ascii_case("expression")
                    || name.eq_ignore_ascii_case("-moz-binding")
                {
                    return Err(parser.new_custom_error(()));
                }
                if name.eq_ignore_ascii_case("url") {
                    let value = parser.parse_nested_block(|nested| {
                        let value = nested.expect_string_cloned()?;
                        nested.expect_exhausted()?;
                        resource_url(&value, base, false).ok_or_else(|| nested.new_custom_error(()))
                    })?;
                    out.push_str("url(");
                    out.push_str(&Token::QuotedString(value.into()).to_css_string());
                    out.push(')');
                } else {
                    let urls = name.eq_ignore_ascii_case("image-set")
                        || name.eq_ignore_ascii_case("-webkit-image-set")
                        || name.eq_ignore_ascii_case("image");
                    let contents = parser
                        .parse_nested_block(|nested| css_tokens(nested, base, depth + 1, urls))?;
                    out.push_str(&token.to_css_string());
                    out.push_str(&contents);
                    out.push(')');
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let contents = parser
                    .parse_nested_block(|nested| css_tokens(nested, base, depth + 1, false))?;
                let (open, close) = match token {
                    Token::ParenthesisBlock => ('(', ')'),
                    Token::CurlyBracketBlock => ('{', '}'),
                    _ => ('[', ']'),
                };
                out.push(open);
                out.push_str(&contents);
                out.push(close);
            }
            Token::Comment(_) => out.push_str("/**/"),
            Token::Number { .. } | Token::Dimension { .. } | Token::Percentage { .. } => {
                let raw = parser.slice_from(start);
                if raw.contains('\\') {
                    out.push_str(&token.to_css_string());
                } else {
                    out.push_str(raw);
                }
            }
            _ => out.push_str(&token.to_css_string()),
        }
        if out.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
            return Err(parser.new_custom_error(()));
        }
    }
    Ok(out)
}
pub(super) fn css_declarations(value: &str, base: Option<&url::Url>) -> String {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut out = String::new();
    while !parser.is_exhausted() {
        let declaration = parser.parse_until_after(Delimiter::Semicolon, |nested| {
            let name = nested.expect_ident_cloned()?;
            if name.eq_ignore_ascii_case("behavior") || name.eq_ignore_ascii_case("-moz-binding") {
                return Err(nested.new_custom_error(()));
            }
            nested.expect_colon()?;
            let value = css_tokens(nested, base, 0, false)?;
            if value.trim().is_empty() && !name.starts_with("--") {
                return Err(nested.new_custom_error(()));
            }
            Ok(format!(
                "{}:{}",
                Token::Ident(name).to_css_string(),
                value.trim_end()
            ))
        });
        if let Ok(declaration) = declaration {
            if out
                .len()
                .saturating_add(declaration.len())
                .saturating_add(1)
                > crate::clipboard::MAX_CLIPBOARD_BYTES
            {
                break;
            }
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(&declaration);
        }
    }
    out
}
/// Parse HTML srcset candidates without splitting the commas inside data URLs.
/// Bad/duplicate descriptors reject only their candidate.
pub(super) fn srcset(value: &str, base: Option<&url::Url>) -> String {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut candidates = Vec::new();
    let mut output_bytes = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b',') {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if start == index {
            break;
        }
        let raw = &value[start..index];
        let comma = raw.ends_with(',');
        let url = raw.trim_end_matches(',');
        let mut descriptors = Vec::new();
        if !comma {
            let mut token = String::new();
            let mut depth = 0usize;
            while index < bytes.len() {
                let character = bytes[index] as char;
                index += 1;
                if character == ',' && depth == 0 {
                    if !token.is_empty() {
                        descriptors.push(std::mem::take(&mut token));
                    }
                    break;
                }
                if character.is_ascii_whitespace() && depth == 0 {
                    if !token.is_empty() {
                        descriptors.push(std::mem::take(&mut token));
                    }
                } else {
                    token.push(character);
                    if character == '(' {
                        depth += 1;
                    } else if character == ')' {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            if !token.is_empty() {
                descriptors.push(token);
            }
        }
        let mut width = false;
        let mut density = false;
        let mut height = false;
        let mut valid = true;
        for descriptor in &descriptors {
            let numeric = descriptor
                .get(..descriptor.len().saturating_sub(1))
                .unwrap_or("");
            match descriptor.as_bytes().last().copied() {
                Some(b'w')
                    if !width
                        && !density
                        && numeric.parse::<u64>().is_ok_and(|value| value > 0)
                        && numeric.bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    width = true
                }
                Some(b'h')
                    if !height
                        && !density
                        && numeric.parse::<u64>().is_ok_and(|value| value > 0)
                        && numeric.bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    height = true
                }
                Some(b'x') if !density && !width && !height && density_number(numeric) => {
                    density = true
                }
                _ => valid = false,
            }
        }
        if height && !width {
            valid = false;
        }
        if valid {
            if let Some(url) = resource_url(url, base, false) {
                let candidate = if descriptors.is_empty() {
                    url
                } else {
                    format!("{} {}", url, descriptors.join(" "))
                };
                output_bytes = output_bytes
                    .saturating_add(candidate.len())
                    .saturating_add(2);
                if output_bytes > crate::clipboard::MAX_CLIPBOARD_BYTES {
                    break;
                }
                candidates.push(candidate);
            }
        }
    }
    candidates.join(", ")
}
fn density_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    if bytes.first() == Some(&b'-') {
        index += 1;
    }
    let first = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut digits = index - first;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let first = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        digits += index - first;
    }
    if digits == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let first = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == first {
            return false;
        }
    }
    index == bytes.len()
        && value
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite() && value >= 0.)
}
// HTML foreign-attribute adjustment does not namespace the literal xml:base.
// Clipboard HTML can nevertheless contain serialized foreign XML ancestry;
// interpret that spelling only on foreign elements, before removing it.
fn declared_base<'a>(dom: &'a DomRuntime, node: NodeId) -> Option<&'a str> {
    if let Ok(Some(value)) = dom.attribute_ns(node, Some(XML), "base") {
        return Some(value);
    }
    let record = dom.arena().get(node).ok()?;
    if matches!(&record.kind,NodeKind::Element{name} if matches!(name.ns.as_ref(),SVG|MATH)) {
        return dom.attribute_ns(node, None, "xml:base").ok().flatten();
    }
    None
}
fn node_base(dom: &DomRuntime, node: NodeId, base: Option<&url::Url>) -> Option<url::Url> {
    let mut base = base.cloned();
    let mut lineage = dom.arena().ancestors(node).ok()?;
    lineage.reverse();
    lineage.push(node);
    for node in lineage {
        if let Some(value) = declared_base(dom, node) {
            if let Some(current) = base.as_ref() {
                base = current.join(value).ok();
            } else {
                base = url::Url::parse(value).ok();
            }
        }
    }
    base
}
pub(super) fn rewrite_from_source(
    dom: &mut DomRuntime,
    source: NodeId,
    clone: NodeId,
    base: Option<&url::Url>,
) -> Result<(), V8BindingError> {
    let base = node_base(dom, source, base);
    rewrite_attributes(dom, clone, base.as_ref(), false)
}
fn rewrite_attributes(
    dom: &mut DomRuntime,
    node: NodeId,
    base: Option<&url::Url>,
    sanitize: bool,
) -> Result<(), V8BindingError> {
    let link = matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name} if matches!(name.local.as_ref(),"a"|"area"));
    let mut output_bytes = 0usize;
    let attributes = dom
        .arena()
        .get(node)?
        .attributes
        .records()
        .map(|attribute| {
            (
                attribute.namespace.clone(),
                attribute.name.clone(),
                attribute.local_name.clone(),
                attribute.value.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (namespace, qualified, name, value) in attributes {
        let local = name.to_ascii_lowercase();
        let ns = namespace.as_deref();
        let blocked = (ns == Some(XML) && local == "base")
            || (ns.is_none() && local == "xml:base")
            || sanitize
                && (local.starts_with("on")
                    || matches!(local.as_str(), "srcdoc" | "is")
                    || (ns.is_some()
                        && !matches!(
                            ns,
                            Some(XML) | Some(XLINK) | Some("http://www.w3.org/2000/xmlns/")
                        ))
                    || (ns == Some("http://www.w3.org/2000/xmlns/")
                        && !matches!(value.as_str(), HTML | SVG | MATH | XML | XLINK)));
        let next = if blocked {
            None
        } else if ns.is_none() && local == "style" {
            Some(css_declarations(&value, base))
        } else if ns.is_none() && matches!(local.as_str(), "srcset" | "imagesrcset") {
            Some(srcset(&value, base))
        } else if (ns.is_none()
            && matches!(
                local.as_str(),
                "href"
                    | "src"
                    | "cite"
                    | "action"
                    | "formaction"
                    | "poster"
                    | "background"
                    | "longdesc"
                    | "data"
            ))
            || (ns == Some(XLINK) && local == "href")
        {
            resource_url(
                &value,
                base,
                (local == "href" && link)
                    || matches!(
                        local.as_str(),
                        "cite" | "action" | "formaction" | "longdesc"
                    ),
            )
        } else if ns.is_none()
            && matches!(
                local.as_str(),
                "fill"
                    | "stroke"
                    | "filter"
                    | "clip-path"
                    | "mask"
                    | "marker"
                    | "marker-start"
                    | "marker-mid"
                    | "marker-end"
                    | "cursor"
            )
        {
            let mut input = ParserInput::new(&value);
            let mut parser = Parser::new(&mut input);
            css_tokens(&mut parser, base, 0, false).ok()
        } else if ns.is_none() && matches!(local.as_str(), "ping" | "archive") {
            Some(
                value
                    .split_ascii_whitespace()
                    .filter_map(|value| resource_url(value, base, true))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        } else {
            Some(value.clone())
        };
        output_bytes = output_bytes.saturating_add(next.as_ref().map_or(0, String::len));
        if output_bytes > crate::clipboard::MAX_CLIPBOARD_BYTES {
            return Err(V8BindingError::Allocation("clipboard attribute URL budget"));
        }
        match next {
            Some(next) if next != value => {
                dom.set_attribute_ns(node, namespace.clone(), qualified, next)?
            }
            None => dom.remove_attribute_ns(node, ns, &name)?,
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn sanitize(markup: &str, base: Option<&url::Url>) -> Result<String, V8BindingError> {
    if markup.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
        return Err(V8BindingError::Allocation("clipboard markup budget"));
    }
    let mut dom = oryn_dom::html::parse_fragment(markup, "div").into_runtime()?;
    let root = dom.document();
    // Template contents are separately owned inert fragments, not light-tree
    // children. They must be sanitized before a later clone can instantiate them.
    let mut pending = vec![(root, base.cloned())];
    let mut nodes = Vec::new();
    while let Some((node, mut inherited)) = pending.pop() {
        if nodes.len() >= 32768 {
            return Err(V8BindingError::Allocation("clipboard markup node budget"));
        }
        let record = dom.arena().get(node)?;
        if let Some(value) = declared_base(&dom, node) {
            inherited = match inherited.as_ref() {
                Some(base) => base.join(value).ok(),
                None => url::Url::parse(value).ok(),
            };
        }
        for child in record.children.iter().rev() {
            pending.push((*child, inherited.clone()));
        }
        if let Some(content) = record.template_content {
            pending.push((content, inherited.clone()));
        }
        nodes.push((node, inherited));
    }
    for (node, base) in nodes {
        let Ok(record) = dom.arena().get(node) else {
            continue;
        };
        let NodeKind::Element { name } = &record.kind else {
            continue;
        };
        let local = name.local.as_ref().to_ascii_lowercase();
        if !matches!(name.ns.as_ref(), HTML | SVG | MATH)
            || matches!(
                local.as_str(),
                "script"
                    | "style"
                    | "iframe"
                    | "object"
                    | "embed"
                    | "base"
                    | "meta"
                    | "link"
                    | "foreignobject"
                    | "animate"
                    | "animatemotion"
                    | "animatetransform"
                    | "set"
            )
        {
            if let Some(parent) = dom.parent(node)? {
                dom.remove_child(parent, node)?;
            }
            continue;
        }
        rewrite_attributes(&mut dom, node, base.as_ref(), true)?;
    }
    let mut roots = dom.children(root)?.to_vec();
    if roots.len() == 1 && dom.is_element(roots[0], HTML, "html") {
        roots = dom.children(roots[0])?.to_vec();
    }
    let result = roots
        .into_iter()
        .map(|node| dom.serialize(node))
        .collect::<Result<Vec<_>, _>>()?
        .join("");
    if result.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
        return Err(V8BindingError::Allocation("clipboard markup budget"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn css_tokenization_decodes_escapes_and_preserves_unrelated_declarations() {
        let base = url::Url::parse("https://example.test/assets/page.html").unwrap();
        let css = css_declarations(
            r#"color:red;background:url("\6a avascript:alert(1)");width:ex\70 ression(alert(1));be\68 avior:url(x);font-weight:bold;background-image:url('../safe.png')"#,
            Some(&base),
        );
        assert!(css.contains("color:red"));
        assert!(css.contains("font-weight:bold"));
        assert!(css.contains("https://example.test/safe.png"));
        assert!(!css.contains("javascript"));
        assert!(!css.contains("expression"));
        assert!(!css.contains("behavior"));
    }
    #[test]
    fn css_image_set_strings_nested_urls_and_fragment_references_are_rewritten() {
        let base = url::Url::parse("https://example.test/images/").unwrap();
        let css = css_declarations(
            r#"background-image:image-set("one.png" 1x,url(two.png) 2x);filter:url(#filter);color:rgb(1 2 3 / .5)"#,
            Some(&base),
        );
        assert!(css.contains("https://example.test/images/one.png"));
        assert!(css.contains("https://example.test/images/two.png"));
        assert!(css.contains("#filter"));
        assert!(css.contains("rgb("));
        assert!(
            !css_declarations(
                r#"background:image-set("javascript:bad()" 1x);color:blue"#,
                Some(&base)
            )
            .contains("background")
        );
    }
    #[test]
    fn custom_property_token_boundaries_and_empty_values_are_preserved() {
        let css = css_declarations(
            "--empty:;--tokens:{a:b};font-size:calc(1px + 2px);--split:ex/**/pression;",
            None,
        );
        assert!(css.contains("--empty:"));
        assert!(css.contains("--tokens:{a:b}"));
        assert!(css.contains("calc(1px + 2px)"));
        assert!(css.contains("ex/**/pression"));
    }
    #[test]
    fn srcset_retains_data_url_commas_and_rejects_bad_candidates() {
        let base = url::Url::parse("https://example.test/images/").unwrap();
        assert_eq!(
            srcset("data:image/png;base64,AAAA 1x, two.png 2x", Some(&base)),
            "data:image/png;base64,AAAA 1x, https://example.test/images/two.png 2x"
        );
        assert_eq!(
            srcset(
                "bad.png 1x 2x, good.png 640w, wrong.png +2x, broken.png NaNx, final.png 1.5x",
                Some(&base)
            ),
            "https://example.test/images/good.png 640w, https://example.test/images/final.png 1.5x"
        );
        assert_eq!(
            srcset("javascript:bad 1x, safe.png 2x", Some(&base)),
            "https://example.test/images/safe.png 2x"
        );
        assert!(srcset("x.png \u{1f600}", Some(&base)).is_empty());
    }
    #[test]
    fn safe_declaration_spacing_and_serialized_foreign_base_are_preserved() {
        assert_eq!(
            css_declarations("color: red;font-weight:bold", None),
            "color: red;font-weight:bold"
        );
        let base = url::Url::parse("https://example.test/root/").unwrap();
        let html=sanitize("<svg xml:base='https://assets.test/a/'><g xml:base='../b/'><image href='photo.png'/></g></svg><img xml:base='https://ignored.test/' src='local.png'>",Some(&base)).unwrap();
        assert!(html.contains("https://assets.test/b/photo.png"));
        assert!(html.contains("https://example.test/root/local.png"));
        assert!(!html.contains("xml:base"));
    }
    #[test]
    fn namespace_urls_svg_presentation_and_inert_templates_are_sanitized() {
        let base = url::Url::parse("https://example.test/root/").unwrap();
        let html=sanitize(r#"<svg xml:base="https://assets.test/a/"><g xml:base="../b/"><image xlink:href="photo.png"/><use xlink:href="java&#10;script:bad()"/><path fill="url('javascript:bad()')"/></g></svg><template><script>bad()</script><img src="safe.png" onerror="bad()"></template>"#,Some(&base)).unwrap();
        assert!(html.contains("https://assets.test/b/photo.png"));
        assert!(html.contains("https://example.test/root/safe.png"));
        assert!(!html.contains("javascript"));
        assert!(!html.contains("java\nscript"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("xml:base"));
    }
    #[test]
    fn author_base_element_does_not_override_native_document_base() {
        let base = url::Url::parse("https://example.test/committed/").unwrap();
        let html=sanitize("<base href='https://attacker.test/'><img src='one.png' srcset='two.png 2x'><a href='mailto:user@example.test'>mail</a>",Some(&base)).unwrap();
        assert!(html.contains("https://example.test/committed/one.png"));
        assert!(html.contains("https://example.test/committed/two.png"));
        assert!(!html.contains("attacker.test"));
        assert!(html.contains("mailto:user@example.test"));
        assert!(resource_url("data:text/html,<script>bad()</script>", Some(&base), true).is_none());
        assert!(resource_url("JaVa\tScRiPt:bad()", Some(&base), true).is_none());
    }
}
