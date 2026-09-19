//! Browser MIME records, parsed and serialized per the MIME Sniffing Standard.
//!
//! This deliberately differs from strict HTTP header validation: malformed
//! parameters are discarded, while a valid type/subtype is retained.
//! https://mimesniff.spec.whatwg.org/#parse-a-mime-type

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MimeType {
    essence: String,
    parameters: Vec<(String, String)>,
}

fn http_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' ')
}

fn token(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

fn quoted_token(c: char) -> bool {
    c == '\t' || (' '..='~').contains(&c) || ('\u{80}'..='\u{ff}').contains(&c)
}

impl MimeType {
    /// Fetch's extract-a-MIME-type algorithm. Header values are combined before
    /// splitting: an open quoted string can continue across field occurrences.
    pub fn from_headers(headers: &[(String, String)]) -> Option<Self> {
        let combined = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Self::extract(&combined)
    }
    pub fn extract(combined: &str) -> Option<Self> {
        let mut start = 0;
        let mut quoted = false;
        let mut escaped = false;
        let mut ranges = Vec::new();
        for (index, ch) in combined.char_indices() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = false;
                }
            } else if ch == '"' {
                quoted = true;
            } else if ch == ',' {
                ranges.push(start..index);
                start = index + 1;
            }
        }
        ranges.push(start..combined.len());
        let mut result = None;
        let mut essence = String::new();
        let mut charset: Option<String> = None;
        for range in ranges {
            let Some(mut mime) = Self::parse(&combined[range]) else {
                continue;
            };
            if mime.essence() == "*/*" {
                continue;
            }
            if mime.essence() != essence {
                charset = mime.parameter("charset").map(str::to_owned);
                essence = mime.essence().to_owned();
            } else if mime.parameter("charset").is_none()
                && let Some(charset) = charset.as_deref()
            {
                mime.set_parameter("charset", charset);
            }
            result = Some(mime);
        }
        result
    }

    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim_matches(http_whitespace);
        let (kind, rest) = input.split_once('/')?;
        if kind.is_empty() || !kind.chars().all(token) {
            return None;
        }
        let (subtype, rest) = rest.split_once(';').unwrap_or((rest, ""));
        let subtype = subtype.trim_end_matches(http_whitespace);
        if subtype.is_empty() || !subtype.chars().all(token) {
            return None;
        }
        let mut mime = Self {
            essence: format!(
                "{}/{}",
                kind.to_ascii_lowercase(),
                subtype.to_ascii_lowercase()
            ),
            parameters: Vec::new(),
        };
        let mut names = HashSet::new();
        // Keep separators in the stream: an invalid bare parameter must not
        // swallow the next parameter's value.
        let mut chars = rest.chars().peekable();
        while chars.peek().is_some() {
            while chars.peek().is_some_and(|c| http_whitespace(*c)) {
                chars.next();
            }
            let mut name = String::new();
            while chars.peek().is_some_and(|c| !matches!(c, ';' | '=')) {
                name.push(chars.next()?);
            }
            match chars.next() {
                Some(';') => continue,
                Some('=') => (),
                _ => break,
            }
            let name = name.to_ascii_lowercase();
            let mut value = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => value.push(chars.next().unwrap_or('\\')),
                        c => value.push(c),
                    }
                }
                while chars.peek().is_some_and(|c| *c != ';') {
                    chars.next();
                }
            } else {
                while chars.peek().is_some_and(|c| *c != ';') {
                    value.push(chars.next()?);
                }
                value.truncate(value.trim_end_matches(http_whitespace).len());
                if value.is_empty() {
                    chars.next();
                    continue;
                }
            }
            if !name.is_empty()
                && name.chars().all(token)
                && value.chars().all(quoted_token)
                && names.insert(name.clone())
            {
                mime.parameters.push((name, value));
            }
            chars.next(); // semicolon, if present
        }
        Some(mime)
    }

    pub fn essence(&self) -> &str {
        &self.essence
    }

    pub fn is_javascript(&self) -> bool {
        matches!(
            self.essence.as_str(),
            "application/ecmascript"
                | "application/javascript"
                | "application/x-ecmascript"
                | "application/x-javascript"
                | "text/ecmascript"
                | "text/javascript"
                | "text/javascript1.0"
                | "text/javascript1.1"
                | "text/javascript1.2"
                | "text/javascript1.3"
                | "text/javascript1.4"
                | "text/javascript1.5"
                | "text/jscript"
                | "text/livescript"
                | "text/x-ecmascript"
                | "text/x-javascript"
        )
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Update a MIME parameter without reparsing or losing parameter order.
    /// Invalid names/values leave the record unchanged.
    pub fn set_parameter(&mut self, name: &str, value: &str) -> bool {
        if name.is_empty() || !name.chars().all(token) || !value.chars().all(quoted_token) {
            return false;
        }
        let name = name.to_ascii_lowercase();
        if let Some((_, current)) = self.parameters.iter_mut().find(|(key, _)| key == &name) {
            *current = value.to_owned();
        } else {
            self.parameters.push((name, value.to_owned()));
        }
        true
    }

    pub fn serialize(&self) -> String {
        let mut result = self.essence.clone();
        for (name, value) in &self.parameters {
            result.push(';');
            result.push_str(name);
            result.push('=');
            if value.is_empty() || !value.chars().all(token) {
                result.push('"');
                for c in value.chars() {
                    if matches!(c, '"' | '\\') {
                        result.push('\\');
                    }
                    result.push(c);
                }
                result.push('"');
            } else {
                result.push_str(value);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_mime_extraction_combines_fields_and_keeps_quoted_commas() {
        // Expectations from pinned WPT fetch/content-type/resources/content-types.json.
        let cases: &[(&[&str], &str)] = &[
            (&["", "text/plain"], "text/plain"),
            (&["text/html", "text/plain"], "text/plain"),
            (
                &["text/plain;charset=gbk", "text/plain"],
                "text/plain;charset=gbk",
            ),
            (
                &["text/plain;charset=gbk", "text/plain;charset=windows-1252"],
                "text/plain;charset=windows-1252",
            ),
            (
                &["text/html;charset=gbk", "text/plain", "text/html"],
                "text/html",
            ),
            (&["text/html", "*/*;charset=gbk"], "text/html"),
            (
                &[r#"text/html;charset=gbk"#, r#"text/html;x=",text/plain"#],
                r#"text/html;x=",text/plain";charset=gbk"#,
            ),
            (
                &[r#"text/html;x=""#, "text/plain"],
                r#"text/html;x=", text/plain""#,
            ),
            (&[r#"text/html;""#, r#"""#, "text/plain"], "text/plain"),
        ];
        for (values, expected) in cases {
            let headers = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    (
                        if index % 2 == 0 {
                            "Content-Type"
                        } else {
                            "content-type"
                        }
                        .into(),
                        (*value).into(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                MimeType::from_headers(&headers).unwrap().serialize(),
                *expected,
                "{values:?}"
            );
        }
        assert!(MimeType::extract("*/*, invalid, ;").is_none());
        assert_eq!(
            MimeType::extract("text/plain, invalid, */*")
                .unwrap()
                .essence(),
            "text/plain"
        );
    }

    #[test]
    fn parameter_updates_preserve_order_and_validate_values() {
        let mut mime =
            MimeType::parse("Text/Plain; boundary=CaseSensitive; CHARSET=latin1").unwrap();
        assert!(mime.set_parameter("Charset", "UTF-8"));
        assert_eq!(
            mime.serialize(),
            "text/plain;boundary=CaseSensitive;charset=UTF-8"
        );
        let original = mime.clone();
        assert!(!mime.set_parameter("bad name", "value"));
        assert!(!mime.set_parameter("charset", "bad\nvalue"));
        assert_eq!(mime, original);
        assert!(mime.set_parameter("note", "two words"));
        assert_eq!(mime.parameter("NOTE"), Some("two words"));
        assert!(mime.serialize().ends_with(";note=\"two words\""));
    }

    #[test]
    fn javascript_mime_classification_uses_the_standard_essence_set() {
        for essence in [
            "application/ecmascript",
            "application/javascript",
            "application/x-ecmascript",
            "application/x-javascript",
            "text/ecmascript",
            "text/javascript",
            "text/javascript1.0",
            "text/javascript1.1",
            "text/javascript1.2",
            "text/javascript1.3",
            "text/javascript1.4",
            "text/javascript1.5",
            "text/jscript",
            "text/livescript",
            "text/x-ecmascript",
            "text/x-javascript",
        ] {
            let value = format!("{};charset=utf-8", essence.to_ascii_uppercase());
            assert!(MimeType::parse(&value).unwrap().is_javascript(), "{value}");
        }
        for value in [
            "text/plain",
            "application/json",
            "application/node",
            "text/javascript1.6",
            "application/example+javascript",
            "text/css;type=text/javascript",
        ] {
            assert!(!MimeType::parse(value).unwrap().is_javascript(), "{value}");
        }
    }

    #[test]
    fn browser_mime_preserves_parameter_case_and_order() {
        let mime = MimeType::parse(" \tTEXT/HTML \r\n; CHARSET=GBK;z=1;A=2;charset=utf-8").unwrap();
        assert_eq!(mime.serialize(), "text/html;charset=GBK;z=1;a=2");
        assert_eq!(mime.parameter("Charset"), Some("GBK"));
    }

    #[test]
    fn browser_mime_recovers_parameters_without_rejecting_essence() {
        for (input, expected) in [
            ("text/html;charset=gbk(", "text/html;charset=\"gbk(\""),
            (
                "text/html;bad;=no; bad name=no;x=;charset=UTF-8;",
                "text/html;charset=UTF-8",
            ),
            ("text/html;a=\u{100};a=valid", "text/html;a=valid"),
            (
                "text/html;foo=bar\t ;empty=\"\"",
                "text/html;foo=bar;empty=\"\"",
            ),
            (
                "text/html;charset=\"shift_jis\"junk;x=2",
                "text/html;charset=shift_jis;x=2",
            ),
            ("text/html;a=\"abc", "text/html;a=abc"),
            ("text/html;a=\"x\\", "text/html;a=\"x\\\\\""),
            ("text/html;a=\"x\\\"y\\\\z\"", "text/html;a=\"x\\\"y\\\\z\""),
            ("text/html;a=é", "text/html;a=\"é\""),
        ] {
            let mime = MimeType::parse(input).unwrap();
            assert_eq!(mime.serialize(), expected, "{input:?}");
            assert_eq!(MimeType::parse(expected), Some(mime));
        }
    }

    #[test]
    fn browser_mime_rejects_invalid_essence_and_non_http_whitespace() {
        for input in [
            "",
            "text",
            "/html",
            "text/",
            "text /html",
            "text/ html",
            "text/html/extra",
            "\u{c}text/html",
            "text/html\u{c}",
            "téxt/html",
        ] {
            assert_eq!(MimeType::parse(input), None, "{input:?}");
        }
    }
}
