//! HTML shared declarative refresh parsing. URL parsing remains owner-dependent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub delay_seconds: u64,
    pub target: Option<String>,
}
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}
pub fn parse(input: &str) -> Option<Refresh> {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while bytes.get(cursor).is_some_and(|b| space(*b)) {
        cursor += 1;
    }
    let start = cursor;
    let mut delay = 0u64;
    while let Some(byte) = bytes.get(cursor).filter(|byte| byte.is_ascii_digit()) {
        delay = delay
            .saturating_mul(10)
            .saturating_add(u64::from(*byte - b'0'));
        cursor += 1;
    }
    if cursor == start && bytes.get(cursor) != Some(&b'.') {
        return None;
    }
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
    {
        cursor += 1;
    }
    if bytes
        .get(cursor)
        .is_some_and(|byte| !space(*byte) && !matches!(*byte, b';' | b','))
    {
        return None;
    }
    while bytes.get(cursor).is_some_and(|byte| space(*byte)) {
        cursor += 1;
    }
    if bytes
        .get(cursor)
        .is_some_and(|byte| matches!(*byte, b';' | b','))
    {
        cursor += 1;
    }
    while bytes.get(cursor).is_some_and(|byte| space(*byte)) {
        cursor += 1;
    }
    if cursor == bytes.len() {
        return Some(Refresh {
            delay_seconds: delay,
            target: None,
        });
    }
    let original = cursor;
    if bytes
        .get(cursor..cursor + 3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"url"))
    {
        cursor += 3;
        while bytes.get(cursor).is_some_and(|byte| space(*byte)) {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while bytes.get(cursor).is_some_and(|byte| space(*byte)) {
                cursor += 1;
            }
        } else {
            cursor = original;
        }
    }
    let quote = bytes
        .get(cursor)
        .copied()
        .filter(|byte| matches!(*byte, b'\'' | b'"'));
    if quote.is_some() {
        cursor += 1;
    }
    let rest = &input[cursor..];
    let target = quote
        .and_then(|quote| rest.find(char::from(quote)))
        .map_or(rest, |end| &rest[..end]);
    Some(Refresh {
        delay_seconds: delay,
        target: Some(target.to_owned()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_accepts_html_delimiters_fractions_and_matching_quotes() {
        for (input, delay, target) in [
            ("1", 1, None),
            ("1,", 1, None),
            ("1 url=foo", 1, Some("foo")),
            ("1;;url=foo", 1, Some(";url=foo")),
            (" 1 ; URL = 'foo'bar", 1, Some("foo")),
            ("1; url foo", 1, Some("url foo")),
            (".9;url=foo", 0, Some("foo")),
            ("1.9..5.;url=foo", 1, Some("foo")),
            ("1;url=\"foo'bar", 1, Some("foo'bar")),
            ("0;url=", 0, Some("")),
        ] {
            assert_eq!(
                parse(input),
                Some(Refresh {
                    delay_seconds: delay,
                    target: target.map(str::to_owned)
                }),
                "{input}"
            );
        }
        for input in ["", "foo", ";foo", "+0", "-1", "1e1;url=x", "1x;url=x"] {
            assert_eq!(parse(input), None, "{input}");
        }
    }
}
