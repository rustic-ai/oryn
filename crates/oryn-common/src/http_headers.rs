//! Header rules shared by Web API guards and the parent IPC boundary.

pub fn forbidden_request_header(name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "accept-charset"
            | "accept-encoding"
            | "access-control-request-headers"
            | "access-control-request-method"
            | "connection"
            | "content-length"
            | "cookie"
            | "cookie2"
            | "date"
            | "dnt"
            | "expect"
            | "host"
            | "keep-alive"
            | "origin"
            | "referer"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "via"
    ) || name.starts_with("proxy-")
        || name.starts_with("sec-")
        || (matches!(
            name.as_str(),
            "x-http-method" | "x-http-method-override" | "x-method-override"
        ) && value.split(',').any(|method| {
            matches!(
                method
                    .trim_matches([' ', '\t'])
                    .to_ascii_uppercase()
                    .as_str(),
                "CONNECT" | "TRACE" | "TRACK"
            )
        }))
}

pub fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

pub fn valid_header_value(value: &str) -> bool {
    !value.bytes().any(|byte| matches!(byte, 0 | 10 | 13))
}

fn unsafe_request_header_byte(byte: u8) -> bool {
    matches!(byte, 0..=8 | 11..=12 | 14..=31 | 127) || b"\"():<>?@[\\]{}".contains(&byte)
}

pub fn safelisted_request_header(name: &str, value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.to_ascii_lowercase().as_str() {
        "accept" => !value.bytes().any(unsafe_request_header_byte),
        "accept-language" | "content-language" => value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b" *,-.;=".contains(&byte)),
        "content-type" => {
            !value.bytes().any(unsafe_request_header_byte)
                && matches!(
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                )
        }
        "range" => {
            let Some(range) = value.strip_prefix("bytes=") else {
                return false;
            };
            let Some((start, end)) = range.split_once('-') else {
                return false;
            };
            if start.is_empty()
                || !start.bytes().all(|byte| byte.is_ascii_digit())
                || !end.bytes().all(|byte| byte.is_ascii_digit())
            {
                return false;
            }
            // Compare decimal integers without overflowing for large ranges.
            let start = start.trim_start_matches('0');
            let end_number = end.trim_start_matches('0');
            end.is_empty() || (end_number.len(), end_number) >= (start.len(), start)
        }
        _ => false,
    }
}

/// Request-no-cors excludes privileged Range even though Range is CORS safelisted.
pub fn no_cors_request_header(name: &str, value: &str) -> bool {
    !name.eq_ignore_ascii_case("range") && safelisted_request_header(name, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_owned_headers_and_method_overrides_are_blocked() {
        for name in [
            "Host",
            "Cookie",
            "Origin",
            "Content-Length",
            "Sec-Fetch-Site",
            "Proxy-Authorization",
        ] {
            assert!(forbidden_request_header(name, "value"), "{name}");
        }
        assert!(forbidden_request_header(
            "X-HTTP-Method-Override",
            "PATCH, trace"
        ));
        assert!(!forbidden_request_header("X-HTTP-Method-Override", "PATCH"));
        assert!(!forbidden_request_header(
            "Authorization",
            "Bearer author-token"
        ));
        assert!(!forbidden_request_header(
            "X-Application-Token",
            "author-token"
        ));
    }

    #[test]
    fn ipc_header_validation_rejects_injection() {
        assert!(!valid_header_name("bad name"));
        assert!(!valid_header_name(""));
        for value in ["a\r\nb: c", "a\0b", "a\nb"] {
            assert!(!valid_header_value(value));
        }
        assert!(valid_header_name("x-application-token"));
        assert!(valid_header_value("visible\tvalue"));
    }
}
