//! Shared Fetch response authorization and script-visible header filtering.

/// Parse the HTTP list grammar without granting a partial list when any token
/// is malformed. Empty list members are ignored; method token case is retained.
fn permission_tokens(headers: &[(String, String)], field: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(field))
    {
        for token in value
            .split(',')
            .map(|value| value.trim_matches([' ', '\t']))
        {
            if token.is_empty() {
                continue;
            }
            if !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            {
                return None;
            }
            tokens.push(token.to_owned());
        }
    }
    Some(tokens)
}

/// Filter only after CORS authorization and redirect processing have consumed
/// the full network header list. Cookie storage remains in the network owner.
pub fn cors_exposed_response_headers(
    headers: Vec<(String, String)>,
    include_credentials: bool,
) -> Vec<(String, String)> {
    const SAFELIST: &[&str] = &[
        "cache-control",
        "content-language",
        "content-length",
        "content-type",
        "expires",
        "last-modified",
        "pragma",
    ];
    let exposed = permission_tokens(&headers, "access-control-expose-headers")
        .unwrap_or_default()
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    let wildcard = !include_credentials && exposed.contains("*");
    headers
        .into_iter()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            !matches!(name.as_str(), "set-cookie" | "set-cookie2")
                && (wildcard || SAFELIST.contains(&name.as_str()) || exposed.contains(&name))
        })
        .collect()
}

#[cfg(test)]
mod response_exposure_tests {
    use super::cors_exposed_response_headers;

    fn names(expose: &[&str], credentials: bool) -> Vec<String> {
        let mut headers = vec![
            ("Content-Type".into(), "text/plain".into()),
            ("X-Visible".into(), "one".into()),
            ("x-visible".into(), "two".into()),
            ("X-Hidden".into(), "secret".into()),
            ("Set-Cookie".into(), "session=secret".into()),
            ("*".into(), "literal".into()),
        ];
        headers.extend(
            expose
                .iter()
                .map(|value| ("Access-Control-Expose-Headers".into(), (*value).into())),
        );
        cors_exposed_response_headers(headers, credentials)
            .into_iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect()
    }

    #[test]
    fn response_authorization_rejects_duplicates_and_requires_exact_credentials_token() {
        let allowed = |headers: Vec<(&str, &str)>, credentials| {
            let headers = headers
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect::<Vec<_>>();
            super::cors_headers_allowed("https://source.test", credentials, &headers)
        };
        assert!(allowed(
            vec![("Access-Control-Allow-Origin", " https://source.test\t")],
            false
        ));
        assert!(allowed(vec![("Access-Control-Allow-Origin", "*")], false));
        assert!(!allowed(
            vec![
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Credentials", "true")
            ],
            true
        ));
        assert!(!allowed(
            vec![
                ("Access-Control-Allow-Origin", "https://source.test"),
                ("access-control-allow-origin", "https://source.test")
            ],
            false
        ));
        for value in ["TRUE", "True", "true, true", "false"] {
            assert!(!allowed(
                vec![
                    ("Access-Control-Allow-Origin", "https://source.test"),
                    ("Access-Control-Allow-Credentials", value)
                ],
                true
            ));
        }
        assert!(allowed(
            vec![
                ("Access-Control-Allow-Origin", "https://source.test"),
                ("Access-Control-Allow-Credentials", "true")
            ],
            true
        ));
    }

    #[test]
    fn safelist_and_explicit_names_preserve_duplicates() {
        assert_eq!(names(&[], false), vec!["content-type"]);
        assert_eq!(
            names(&["X-VISIBLE"], true),
            vec!["content-type", "x-visible", "x-visible"]
        );
        assert_eq!(
            names(&["X-Visible", "X-Hidden"], false),
            vec!["content-type", "x-visible", "x-visible", "x-hidden"]
        );
    }

    #[test]
    fn wildcard_is_literal_with_credentials_and_never_exposes_cookies() {
        let wildcard = names(&["*"], false);
        assert!(wildcard.contains(&"x-hidden".into()));
        assert!(!wildcard.contains(&"set-cookie".into()));
        assert_eq!(names(&["*"], true), vec!["content-type", "*"]);
        assert_eq!(names(&["set-cookie"], true), vec!["content-type"]);
    }

    #[test]
    fn invalid_exposure_list_does_not_partially_grant_access() {
        assert_eq!(names(&["x-visible, bad name"], false), vec!["content-type"]);
        assert_eq!(
            names(&["x-visible", "bad:name"], false),
            vec!["content-type"]
        );
        assert_eq!(
            names(&[", \tX-Visible, ,"], true),
            vec!["content-type", "x-visible", "x-visible"]
        );
    }
}

pub fn cors_headers_allowed(
    source_origin: &str,
    credentials: bool,
    headers: &[(String, String)],
) -> bool {
    // Fetch combines duplicate field values with commas. Neither of these
    // fields accepts a list, so duplicates must fail even when values match.
    let header = |requested: &str| {
        let mut values = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(requested));
        let value = values.next()?.1.trim_matches([' ', '\t']);
        if values.next().is_some() {
            None
        } else {
            Some(value)
        }
    };
    header("access-control-allow-origin")
        .is_some_and(|allowed| allowed == source_origin || (!credentials && allowed == "*"))
        && (!credentials || header("access-control-allow-credentials") == Some("true"))
}

pub use oryn_common::http_headers::safelisted_request_header;

pub fn cors_non_safelisted_headers(headers: &[(String, String)]) -> Vec<String> {
    let mut unsafe_names = std::collections::BTreeSet::new();
    let mut potentially_unsafe = std::collections::BTreeSet::new();
    let mut safelist_size = 0usize;
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        // These entries are inserted by the browser after author-header guards.
        if matches!(name.as_str(), "origin" | "referer") {
            continue;
        }
        if safelisted_request_header(&name, value) {
            safelist_size = safelist_size.saturating_add(value.len());
            potentially_unsafe.insert(name);
        } else {
            unsafe_names.insert(name);
        }
    }
    if safelist_size > 1024 {
        unsafe_names.extend(potentially_unsafe);
    }
    unsafe_names.into_iter().collect()
}

pub fn cors_preflight_required(method: &str, headers: &[(String, String)]) -> bool {
    !matches!(method, "GET" | "HEAD" | "POST") || !cors_non_safelisted_headers(headers).is_empty()
}

pub fn cors_preflight_allowed(
    source_origin: &str,
    credentials: bool,
    method: &str,
    requested_headers: &[String],
    status: u16,
    headers: &[(String, String)],
) -> bool {
    let Some(allowed_methods) = permission_tokens(headers, "access-control-allow-methods") else {
        return false;
    };
    let Some(allowed_headers) = permission_tokens(headers, "access-control-allow-headers") else {
        return false;
    };
    let method_allowed = matches!(method, "GET" | "HEAD" | "POST")
        || allowed_methods
            .iter()
            .any(|allowed| allowed == method || (!credentials && allowed == "*"));
    let headers_allowed = requested_headers.iter().all(|header| {
        allowed_headers.iter().any(|allowed| {
            allowed.eq_ignore_ascii_case(header)
                || (!credentials && allowed == "*" && !header.eq_ignore_ascii_case("authorization"))
        })
    });
    (200..300).contains(&status)
        && cors_headers_allowed(source_origin, credentials, headers)
        && method_allowed
        && headers_allowed
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    #[test]
    fn request_safelist_validates_values_and_range_grammar() {
        assert!(safelisted_request_header("Accept", "text/html,*/*"));
        assert!(!safelisted_request_header("Accept", "text/html;v=\"x\""));
        assert!(safelisted_request_header(
            "Content-Language",
            "en-US, fr;q=0.8"
        ));
        assert!(!safelisted_request_header("Accept-Language", "en_US"));
        assert!(safelisted_request_header(
            "Content-Type",
            "text/plain;charset=UTF-8"
        ));
        assert!(!safelisted_request_header(
            "Content-Type",
            "application/json"
        ));
        for value in [
            "bytes=0-",
            "bytes=0-9",
            "bytes=0009-10",
            "bytes=999999999999999999999999-",
        ] {
            assert!(safelisted_request_header("Range", value), "{value}");
        }
        for value in [
            "bytes=-5",
            "bytes=9-2",
            "bytes=0-1,2-3",
            "Bytes=0-1",
            "bytes= 0-1",
        ] {
            assert!(!safelisted_request_header("Range", value), "{value}");
        }
        assert!(safelisted_request_header("accept", &"a".repeat(128)));
        assert!(!safelisted_request_header("accept", &"a".repeat(129)));
    }

    #[test]
    fn aggregate_limit_and_method_case_affect_preflight() {
        let mut headers = vec![("Accept".into(), "a".repeat(128)); 8];
        assert!(cors_non_safelisted_headers(&headers).is_empty());
        headers.push(("Content-Language".into(), "en".into()));
        assert_eq!(
            cors_non_safelisted_headers(&headers),
            vec!["accept", "content-language"]
        );
        assert!(!cors_preflight_required("GET", &[]));
        assert!(cors_preflight_required("get", &[]));
        assert!(cors_preflight_required("PATCH", &[]));
        assert!(!cors_preflight_required(
            "POST",
            &[("Origin".into(), "https://source.test".into())]
        ));
    }

    #[test]
    fn malformed_permission_lists_never_grant_partial_permissions() {
        for (field, valid, invalid) in [
            ("access-control-allow-methods", "PATCH", "PATCH, bad method"),
            (
                "access-control-allow-headers",
                "x-token",
                "x-token, bad:header",
            ),
            (
                "access-control-allow-headers",
                "x-token",
                "x-token, \"quoted\"",
            ),
        ] {
            let base = vec![
                (
                    "access-control-allow-origin".into(),
                    "https://source.test".into(),
                ),
                ("access-control-allow-methods".into(), "PATCH".into()),
                ("access-control-allow-headers".into(), "x-token".into()),
            ];
            let mut headers = base.clone();
            headers.push((field.into(), invalid.into()));
            assert!(!cors_preflight_allowed(
                "https://source.test",
                false,
                "PATCH",
                &["x-token".into()],
                200,
                &headers
            ));
            let mut headers = base;
            headers.push((field.into(), format!(", \t{valid},,")));
            assert!(cors_preflight_allowed(
                "https://source.test",
                false,
                "PATCH",
                &["x-token".into()],
                200,
                &headers
            ));
        }
        let headers = vec![
            ("access-control-allow-origin".into(), "*".into()),
            ("access-control-allow-methods".into(), "patch".into()),
        ];
        assert!(!cors_preflight_allowed(
            "https://source.test",
            false,
            "PATCH",
            &[],
            200,
            &headers
        ));
        assert!(cors_preflight_allowed(
            "https://source.test",
            false,
            "patch",
            &[],
            200,
            &headers
        ));
    }

    #[test]
    fn authorization_header_requires_explicit_preflight_permission() {
        let headers = vec![
            ("access-control-allow-origin".into(), "*".into()),
            ("access-control-allow-methods".into(), "*".into()),
            ("access-control-allow-headers".into(), "*".into()),
        ];
        assert!(cors_preflight_allowed(
            "https://source.test",
            false,
            "PATCH",
            &["x-token".into()],
            204,
            &headers
        ));
        assert!(!cors_preflight_allowed(
            "https://source.test",
            false,
            "PATCH",
            &["authorization".into()],
            204,
            &headers
        ));
        assert!(!cors_preflight_allowed(
            "https://source.test",
            false,
            "PATCH",
            &[],
            302,
            &headers
        ));
    }
}
