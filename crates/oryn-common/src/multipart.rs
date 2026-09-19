//! Byte-oriented multipart/form-data decoding shared by browser body consumers.
#[derive(Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidMultipart;

fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn disposition(value: &str) -> Result<(String, Option<String>), InvalidMultipart> {
    let (kind, mut rest) = value.split_once(';').ok_or(InvalidMultipart)?;
    if !kind
        .trim_matches([' ', '\t'])
        .eq_ignore_ascii_case("form-data")
    {
        return Err(InvalidMultipart);
    }
    let mut name = None;
    let mut filename = None;
    while !rest.trim_matches([' ', '\t']).is_empty() {
        rest = rest.trim_start_matches([' ', '\t']);
        let (key, value) = rest.split_once('=').ok_or(InvalidMultipart)?;
        let key = key.trim_matches([' ', '\t']).to_ascii_lowercase();
        if !token(&key) {
            return Err(InvalidMultipart);
        }
        let value = value.trim_start_matches([' ', '\t']);
        let parsed;
        if let Some(value) = value.strip_prefix('"') {
            let mut result = String::new();
            let mut escaped = false;
            let mut closing = None;
            for (index, ch) in value.char_indices() {
                if escaped {
                    result.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    closing = Some(index + 1);
                    break;
                } else {
                    result.push(ch);
                }
            }
            rest = &value[closing.ok_or(InvalidMultipart)?..];
            parsed = result;
        } else {
            let end = value.find(';').unwrap_or(value.len());
            parsed = value[..end].trim_end_matches([' ', '\t']).to_owned();
            rest = &value[end..];
        }
        if key == "name" && name.is_none() {
            name = Some(parsed);
        } else if key == "filename" && filename.is_none() {
            filename = Some(parsed);
        }
        rest = rest.trim_start_matches([' ', '\t']);
        if !rest.is_empty() {
            rest = rest.strip_prefix(';').ok_or(InvalidMultipart)?;
        }
    }
    Ok((name.ok_or(InvalidMultipart)?, filename))
}

fn delimiter(bytes: &[u8], marker: &[u8], from: usize) -> Option<(usize, usize, bool)> {
    let mut offset = from;
    while offset + marker.len() <= bytes.len() {
        let found = bytes[offset..]
            .windows(marker.len())
            .position(|window| window == marker)?
            + offset;
        if found == 0 || bytes.get(found.wrapping_sub(2)..found) == Some(b"\r\n") {
            let mut end = found + marker.len();
            let closing = bytes.get(end..end + 2) == Some(b"--");
            if closing {
                end += 2;
            }
            while matches!(bytes.get(end), Some(b' ' | b'\t')) {
                end += 1;
            }
            if bytes.get(end..end + 2) == Some(b"\r\n") {
                return Some((found, end + 2, closing));
            }
            if closing && end == bytes.len() {
                return Some((found, end, true));
            }
        }
        offset = found + 1;
    }
    None
}

pub fn parse(bytes: &[u8], boundary: &str) -> Result<Vec<Part>, InvalidMultipart> {
    if boundary.is_empty() || !boundary.is_ascii() || boundary.contains(['\r', '\n', '\0']) {
        return Err(InvalidMultipart);
    }
    let marker = format!("--{boundary}").into_bytes();
    let (_, mut cursor, closing) = delimiter(bytes, &marker, 0).ok_or(InvalidMultipart)?;
    let mut parts = Vec::new();
    if closing {
        return Ok(parts);
    }
    loop {
        let header_end = bytes[cursor..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|offset| cursor + offset)
            .ok_or(InvalidMultipart)?;
        let headers = String::from_utf8_lossy(&bytes[cursor..header_end]);
        let mut content_disposition = None;
        let mut content_type = None;
        for line in headers.split("\r\n") {
            let (name, value) = line.split_once(':').ok_or(InvalidMultipart)?;
            if !token(name) || value.contains(['\0', '\r', '\n']) {
                return Err(InvalidMultipart);
            }
            let value = value.trim_matches([' ', '\t']);
            if name.eq_ignore_ascii_case("content-disposition") && content_disposition.is_none() {
                content_disposition = Some(value);
            }
            if name.eq_ignore_ascii_case("content-type") && content_type.is_none() {
                content_type = Some(value.to_owned());
            }
        }
        let (name, filename) = disposition(content_disposition.ok_or(InvalidMultipart)?)?;
        let start = header_end + 4;
        let (next, after, closing) = delimiter(bytes, &marker, start).ok_or(InvalidMultipart)?;
        let end = next
            .checked_sub(2)
            .filter(|end| *end >= start)
            .ok_or(InvalidMultipart)?;
        parts.push(Part {
            name,
            filename,
            content_type,
            bytes: bytes[start..end].to_vec(),
        });
        if closing {
            return Ok(parts);
        }
        cursor = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_binary_and_boundary_prefixes() {
        let body = b"--Case\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"a.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n\0\xff\r\n--Case-extra\r\nend\r\n--Case--\r\n";
        let parts = parse(body, "Case").unwrap();
        assert_eq!(parts[0].bytes, b"\0\xff\r\n--Case-extra\r\nend");
        assert_eq!(parts[0].filename.as_deref(), Some("a.bin"));
        assert!(parse(body, "case").is_err());
    }
    #[test]
    fn parses_repeated_empty_and_quoted_parts_with_preamble() {
        let body = b"preamble\r\n--AbC\r\nContent-Disposition: form-data; name=\"same\"\r\n\r\n\r\n--AbC\r\nContent-Disposition: form-data; name=\"same\"; filename=\"semi;colon.bin\"\r\n\r\nbytes\r\n--AbC--\r\nepilogue";
        let parts = parse(body, "AbC").unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "same");
        assert!(parts[0].bytes.is_empty());
        assert_eq!(parts[1].name, "same");
        assert_eq!(parts[1].filename.as_deref(), Some("semi;colon.bin"));
        assert_eq!(parts[1].bytes, b"bytes");
    }

    #[test]
    fn rejects_header_controls_without_rejecting_binary_payloads() {
        for value in [
            "form-data; name=\"a\0b\"",
            "form-data; name=\"a\nb\"",
            "form-data; name=\"a\rb\"",
        ] {
            let body = format!("--x\r\nContent-Disposition: {value}\r\n\r\nbody\r\n--x--");
            assert!(parse(body.as_bytes(), "x").is_err());
        }
        let body = b"--x\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n\0\n\r\xff\r\n--x--";
        assert_eq!(parse(body, "x").unwrap()[0].bytes, b"\0\n\r\xff");
        assert!(disposition("form-data; bad key=value; name=x").is_err());
    }

    #[test]
    fn validates_termination_and_required_disposition() {
        assert_eq!(parse(b"--x--", "x").unwrap(), vec![]);
        assert!(
            parse(
                b"--x\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nvalue",
                "x"
            )
            .is_err()
        );
        assert!(parse(b"--x\r\nX: y\r\n\r\nv\r\n--x--", "x").is_err());
        assert!(parse(b"", "").is_err());
    }
}
