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

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
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
