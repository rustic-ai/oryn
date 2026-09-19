//! Pure XML byte decoding shared by native loading and resource geometry.
//! This module performs no resource discovery or IO.

pub fn declaration_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if bytes.starts_with(&[0, b'<', 0, b'?']) {
        return Some(encoding_rs::UTF_16BE);
    }
    if bytes.starts_with(&[b'<', 0, b'?', 0]) {
        return Some(encoding_rs::UTF_16LE);
    }
    let rest = bytes.strip_prefix(b"<?xml")?;
    let xml_space = |ch: char| matches!(ch, '\t' | '\n' | '\r' | ' ');
    if !rest
        .first()
        .is_some_and(|byte| xml_space(char::from(*byte)))
    {
        return None;
    }
    let end = rest.windows(2).position(|pair| pair == b"?>")?;
    let declaration = std::str::from_utf8(&rest[..end]).ok()?;
    let mut remaining = declaration.trim_start_matches(xml_space);
    while !remaining.is_empty() {
        let split = remaining.find(|ch: char| xml_space(ch) || ch == '=')?;
        let name = &remaining[..split];
        remaining = remaining[split..]
            .trim_start_matches(xml_space)
            .strip_prefix('=')?
            .trim_start_matches(xml_space);
        let quote = remaining.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        remaining = &remaining[1..];
        let close = remaining.find(quote)?;
        let value = &remaining[..close];
        if name == "encoding" {
            let mut name_bytes = value.bytes();
            if !name_bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic())
                || !name_bytes
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return None;
            }
            return encoding_rs::Encoding::for_label(value.as_bytes());
        }
        remaining = &remaining[close + 1..];
        if !remaining.is_empty() && !remaining.starts_with(xml_space) {
            return None;
        }
        remaining = remaining.trim_start_matches(xml_space);
    }
    None
}

/// Preserve all response field occurrences in Fetch's combined-header order.
pub fn response_content_type(headers: &[(String, String)]) -> Option<String> {
    let values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(", "))
}

/// Decode a complete XML resource. A BOM takes precedence over the HTTP charset,
/// which takes precedence over the XML declaration/signature. Malformed encoded
/// input is fatal for XML rather than silently introducing replacement text.
pub fn decode(bytes: &[u8], content_type: Option<&str>) -> Result<String, &'static str> {
    let bom = encoding_rs::Encoding::for_bom(bytes);
    let mime = content_type.and_then(crate::mime_type::MimeType::extract);
    let header = mime
        .as_ref()
        .and_then(|mime| mime.parameter("charset"))
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()));
    let encoding = bom
        .map(|(encoding, _)| encoding)
        .or(header)
        .or_else(|| declaration_encoding(bytes))
        .unwrap_or(encoding_rs::UTF_8);
    if encoding == encoding_rs::REPLACEMENT {
        return Err("unsupported XML encoding");
    }
    let (text, malformed) =
        encoding.decode_without_bom_handling(&bytes[bom.map_or(0, |(_, size)| size)..]);
    if malformed {
        return Err("malformed XML byte encoding");
    }
    Ok(text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_bom_header_and_declaration_have_native_precedence() {
        let legacy = b"<?xml version='1.0' encoding='windows-1252'?><svg>\xe9</svg>";
        assert!(decode(legacy, None).unwrap().ends_with("<svg>é</svg>"));
        assert!(decode(legacy, Some("image/svg+xml; charset=utf-8")).is_err());
        let headers = vec![
            (
                "Content-Type".into(),
                "image/svg+xml; charset=windows-1252".into(),
            ),
            ("content-type".into(), "image/svg+xml".into()),
        ];
        assert!(
            decode(legacy, response_content_type(&headers).as_deref())
                .unwrap()
                .ends_with("<svg>é</svg>")
        );
        let mut little = vec![0xff, 0xfe];
        little.extend("<svg>水</svg>".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(
            decode(&little, Some("image/svg+xml; charset=windows-1252")).unwrap(),
            "<svg>水</svg>"
        );
        let big = "<?xml version='1.0' encoding='UTF-16'?><svg>水</svg>"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert!(decode(&big, None).unwrap().ends_with("<svg>水</svg>"));
        assert!(decode(&[0xff, 0xfe, 0x00], None).is_err());
        assert!(decode(b"<svg>\xff</svg>", None).is_err());
    }
    #[test]
    fn declaration_scanner_does_not_accept_non_xml_whitespace_or_pi() {
        for source in [
            "<?xml-stylesheet encoding='windows-1252'?>",
            "<?xml\u{a0}encoding='windows-1252'?>",
            "<?xml version='1.0'encoding='windows-1252'?>",
        ] {
            assert_eq!(declaration_encoding(source.as_bytes()), None);
        }
    }
}
