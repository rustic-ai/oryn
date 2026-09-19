//! Document URL parsing with the Encoding Standard query encoder.
use url::Url;

fn encode_query<'a>(
    input: &'a str,
    encoding: &'static encoding_rs::Encoding,
) -> std::borrow::Cow<'a, [u8]> {
    let (encoded, _, errors) = encoding.encode(input);
    if !errors {
        return encoded;
    }
    // URL encoding escapes the numeric-reference fallback itself; form
    // serialization instead percent-encodes the complete name/value pair.
    let mut replaced = String::with_capacity(input.len());
    for character in input.chars() {
        let mut buffer = [0u8; 4];
        let scalar = character.encode_utf8(&mut buffer);
        if encoding.encode(scalar).2 {
            let codepoint = if encoding == encoding_rs::ISO_2022_JP
                && matches!(character, '\u{0e}' | '\u{0f}' | '\u{1b}')
            {
                0xfffd
            } else {
                character as u32
            };
            replaced.push_str(&format!("%26%23{codepoint}%3B"));
        } else {
            replaced.push(character);
        }
    }
    std::borrow::Cow::Owned(encoding.encode(&replaced).0.into_owned())
}

pub fn parse(
    input: &str,
    base: Option<&Url>,
    encoding: &'static encoding_rs::Encoding,
) -> Result<Url, url::ParseError> {
    let encoding = encoding.output_encoding();
    let encode: url::form_urlencoded::EncodingOverride<'_> =
        Some(&|text| encode_query(text, encoding));
    Url::options()
        .base_url(base)
        .encoding_override(encode)
        .parse(input)
}

pub fn parse_with_label(
    input: &str,
    base: Option<&Url>,
    label: &str,
) -> Result<Url, url::ParseError> {
    parse(
        input,
        base,
        encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_queries_keep_path_and_fragment_utf8_and_escape_numeric_fallback() {
        let base = Url::parse("https://example.test/base/").unwrap();
        for (label, query) in [
            ("windows-1252", "%FF"),
            ("windows-1251", "%26%23255%3B"),
            ("utf-8", "%C3%BF"),
            ("utf-16be", "%C3%BF"),
        ] {
            let url = parse_with_label("ÿ?ÿ#ÿ", Some(&base), label).unwrap();
            assert_eq!(
                url.as_str(),
                format!("https://example.test/base/%C3%BF?{query}#%C3%BF")
            );
        }
        assert_eq!(
            parse_with_label("?�", Some(&base), "windows-1252")
                .unwrap()
                .query(),
            Some("%26%2365533%3B")
        );
        assert_eq!(
            parse_with_label("?ÿ", Some(&base), "bad-label")
                .unwrap()
                .query(),
            Some("%C3%BF")
        );
    }
}
