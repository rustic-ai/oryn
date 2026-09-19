//! Shared HTML canvas bitmap dimensions for bindings and native layout.

/// Canvas dimensions use HTML non-negative integer parsing and the limited
/// unsigned-long range. Invalid or absent values use the element defaults.
pub fn dimensions(width: Option<&str>, height: Option<&str>) -> (u32, u32) {
    (dimension(width, 300), dimension(height, 150))
}

fn dimension(value: Option<&str>, fallback: u32) -> u32 {
    value
        .and_then(|value| {
            let value = value.trim_start_matches([' ', '\t', '\n', '\r', '\u{c}']);
            let value = value.strip_prefix('+').unwrap_or(value);
            let digits = value.bytes().take_while(u8::is_ascii_digit).count();
            value[..digits]
                .parse::<u32>()
                .ok()
                .filter(|value| *value <= i32::MAX as u32)
        })
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::dimensions;

    #[test]
    fn canvas_attributes_follow_html_integer_rules() {
        assert_eq!(dimensions(None, None), (300, 150));
        assert_eq!(dimensions(Some("0"), Some("0")), (0, 0));
        assert_eq!(dimensions(Some(" \t+80px"), Some("40.9")), (80, 40));
        assert_eq!(
            dimensions(Some("2147483647"), Some("12e3")),
            (2147483647, 12)
        );
        for invalid in [
            "",
            "-1",
            "+",
            "2147483648",
            "99999999999999999999",
            "\u{b}12",
            "\u{a0}12",
        ] {
            assert_eq!(
                dimensions(Some(invalid), Some(invalid)),
                (300, 150),
                "{invalid:?}"
            );
        }
    }
}
