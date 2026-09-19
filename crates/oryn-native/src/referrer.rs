use url::Url;

#[cfg(test)]
mod response_initialization_tests {
    use super::ReferrerPolicy;

    #[test]
    fn meta_values_are_single_tokens_and_invalid_values_are_ignored() {
        assert_eq!(
            ReferrerPolicy::parse_meta("  SAME-ORIGIN  "),
            Some(ReferrerPolicy::SameOrigin)
        );
        assert_eq!(
            ReferrerPolicy::parse_meta("never"),
            Some(ReferrerPolicy::NoReferrer)
        );
        for invalid in ["", "unknown", "origin, no-referrer"] {
            assert_eq!(ReferrerPolicy::parse_meta(invalid), None);
        }
        assert_eq!(
            ReferrerPolicy::parse_attribute("origin"),
            Some(ReferrerPolicy::Origin)
        );
        for invalid in ["never", "always", "default", "origin, no-referrer"] {
            assert_eq!(ReferrerPolicy::parse_attribute(invalid), None);
        }
    }

    #[test]
    fn repeated_headers_keep_the_last_recognized_policy() {
        let headers = vec![
            ("Referrer-Policy".into(), "no-referrer, origin".into()),
            ("referrer-policy".into(), "invalid".into()),
        ];
        assert_eq!(
            ReferrerPolicy::default().after_response(&headers),
            ReferrerPolicy::Origin
        );
        assert_eq!(
            ReferrerPolicy::default().after_response(&[]),
            ReferrerPolicy::StrictOriginWhenCrossOrigin
        );
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReferrerPolicy {
    NoReferrer,
    NoReferrerWhenDowngrade,
    Origin,
    OriginWhenCrossOrigin,
    SameOrigin,
    StrictOrigin,
    #[default]
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}

impl ReferrerPolicy {
    /// Element referrerpolicy attributes accept only the modern policy names.
    pub fn parse_attribute(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "no-referrer"
            | "no-referrer-when-downgrade"
            | "origin"
            | "origin-when-cross-origin"
            | "same-origin"
            | "strict-origin"
            | "strict-origin-when-cross-origin"
            | "unsafe-url" => Self::parse_meta(value),
            _ => None,
        }
    }

    /// Meta content is one policy token, not a response-header list.
    pub fn parse_meta(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "no-referrer" | "never" => Self::NoReferrer,
            "no-referrer-when-downgrade" => Self::NoReferrerWhenDowngrade,
            "origin" => Self::Origin,
            "origin-when-cross-origin" | "origin-when-crossorigin" => Self::OriginWhenCrossOrigin,
            "same-origin" => Self::SameOrigin,
            "strict-origin" => Self::StrictOrigin,
            "strict-origin-when-cross-origin" | "default" => Self::StrictOriginWhenCrossOrigin,
            "unsafe-url" | "always" => Self::UnsafeUrl,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoReferrer => "no-referrer",
            Self::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
            Self::Origin => "origin",
            Self::OriginWhenCrossOrigin => "origin-when-cross-origin",
            Self::SameOrigin => "same-origin",
            Self::StrictOrigin => "strict-origin",
            Self::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
            Self::UnsafeUrl => "unsafe-url",
        }
    }

    pub fn parse_list(value: &str) -> Self {
        value
            .split(',')
            .filter_map(|token| match token.trim().to_ascii_lowercase().as_str() {
                "no-referrer" | "never" => Some(Self::NoReferrer),
                "no-referrer-when-downgrade" => Some(Self::NoReferrerWhenDowngrade),
                "default" => Some(Self::StrictOriginWhenCrossOrigin),
                "origin" => Some(Self::Origin),
                "origin-when-cross-origin" | "origin-when-crossorigin" => {
                    Some(Self::OriginWhenCrossOrigin)
                }
                "same-origin" => Some(Self::SameOrigin),
                "strict-origin" => Some(Self::StrictOrigin),
                "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
                "unsafe-url" | "always" => Some(Self::UnsafeUrl),
                _ => None,
            })
            .next_back()
            .unwrap_or_default()
    }

    /// Apply the last recognized response-header policy; absent or invalid
    /// tokens preserve the policy already carried by the fetch request.
    pub fn after_response(self, headers: &[(String, String)]) -> Self {
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
            .flat_map(|(_, value)| value.split(','))
            .filter_map(|token| {
                match token
                    .trim_matches([' ', '\t', '\r', '\n'])
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "no-referrer" => Some(Self::NoReferrer),
                    "no-referrer-when-downgrade" => Some(Self::NoReferrerWhenDowngrade),
                    "origin" => Some(Self::Origin),
                    "origin-when-cross-origin" => Some(Self::OriginWhenCrossOrigin),
                    "same-origin" => Some(Self::SameOrigin),
                    "strict-origin" => Some(Self::StrictOrigin),
                    "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
                    "unsafe-url" => Some(Self::UnsafeUrl),
                    _ => None,
                }
            })
            .last()
            .unwrap_or(self)
    }

    pub fn header(self, source: &Url, destination: &Url) -> Option<String> {
        if !matches!(source.scheme(), "http" | "https") {
            return None;
        }
        let same_origin = source.origin() == destination.origin();
        let downgrade = source.scheme() == "https" && destination.scheme() == "http";
        let origin = serialized_origin(source);
        let sanitized = sanitized(source);
        // Referrer Policy permits user agents to reduce referrers for privacy.
        // The upstream interoperability corpus (and current browsers) reduces
        // a serialized referrer larger than 4096 bytes to its origin. Besides
        // limiting disclosure, this prevents intermediaries from rejecting
        // the request solely because its Referer field is oversized.
        let full = if sanitized.len() > 4096 {
            origin.clone()
        } else {
            sanitized
        };
        match self {
            Self::NoReferrer => None,
            Self::NoReferrerWhenDowngrade => (!downgrade).then_some(full),
            Self::Origin => Some(origin),
            Self::OriginWhenCrossOrigin => Some(if same_origin { full } else { origin }),
            Self::SameOrigin => same_origin.then_some(full),
            Self::StrictOrigin => (!downgrade).then_some(origin),
            Self::StrictOriginWhenCrossOrigin if same_origin => Some(full),
            Self::StrictOriginWhenCrossOrigin => (!downgrade).then_some(origin),
            Self::UnsafeUrl => Some(full),
        }
    }
}

fn sanitized(url: &Url) -> String {
    let mut value = url.clone();
    let _ = value.set_username("");
    let _ = value.set_password(None);
    value.set_fragment(None);
    value.to_string()
}

fn serialized_origin(url: &Url) -> String {
    let mut value = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        value.push(':');
        value.push_str(&port.to_string());
    }
    value.push('/');
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_response_policy_uses_last_recognized_header_token() {
        let initial = ReferrerPolicy::NoReferrer;
        assert_eq!(initial.after_response(&[]), initial);
        assert_eq!(
            initial.after_response(&[("referrer-policy".into(), "unknown,default,always".into())]),
            initial
        );
        let policy = initial.after_response(&[
            ("Referrer-Policy".into(), "unsafe-url,unknown".into()),
            (
                "referrer-policy".into(),
                "origin, no-referrer,unknown".into(),
            ),
        ]);
        assert_eq!(policy, ReferrerPolicy::NoReferrer);
        let source = Url::parse("https://example.test/private?q=1").unwrap();
        let target = Url::parse("https://other.test/script.js").unwrap();
        assert_eq!(policy.header(&source, &target), None);
    }

    #[test]
    fn strict_origin_when_cross_origin_strips_and_blocks_downgrades() {
        let source = Url::parse("https://user:secret@example.test/path?q=1#fragment").unwrap();
        let same = Url::parse("https://example.test/next").unwrap();
        let cross = Url::parse("https://other.test/next").unwrap();
        let downgrade = Url::parse("http://other.test/next").unwrap();
        let policy = ReferrerPolicy::StrictOriginWhenCrossOrigin;
        assert_eq!(
            policy.header(&source, &same).as_deref(),
            Some("https://example.test/path?q=1")
        );
        assert_eq!(
            policy.header(&source, &cross).as_deref(),
            Some("https://example.test/")
        );
        assert_eq!(policy.header(&source, &downgrade), None);
    }

    #[test]
    fn oversized_full_referrer_is_reduced_to_origin() {
        let source = Url::parse(&format!("https://example.test/{}", "a".repeat(4096))).unwrap();
        let destination = Url::parse("https://example.test/next").unwrap();
        assert_eq!(
            ReferrerPolicy::UnsafeUrl
                .header(&source, &destination)
                .as_deref(),
            Some("https://example.test/")
        );
    }

    #[test]
    fn legacy_tokens_map_to_their_standard_policies() {
        assert_eq!(
            ReferrerPolicy::parse_list("never"),
            ReferrerPolicy::NoReferrer
        );
        assert_eq!(
            ReferrerPolicy::parse_list("default"),
            ReferrerPolicy::StrictOriginWhenCrossOrigin
        );
        assert_eq!(
            ReferrerPolicy::parse_list("always"),
            ReferrerPolicy::UnsafeUrl
        );
        assert_eq!(
            ReferrerPolicy::parse_list("origin-when-crossorigin"),
            ReferrerPolicy::OriginWhenCrossOrigin
        );
    }
}

// Fetch appends Origin for CORS tainting and non-GET/HEAD methods. Referrer
// policy can suppress the origin only in the latter, non-CORS branch.
pub(crate) fn request_origin_header(
    origin: &str,
    source: &Url,
    target: &Url,
    method: &str,
    cors_tainted: bool,
    policy: crate::referrer::ReferrerPolicy,
) -> Option<String> {
    if cors_tainted {
        return Some(origin.into());
    }
    if matches!(method, "GET" | "HEAD") {
        return None;
    }
    use crate::referrer::ReferrerPolicy::*;
    let downgrade = source.scheme() == "https" && target.scheme() != "https";
    let suppress = match policy {
        NoReferrer => true,
        SameOrigin => source.origin() != target.origin(),
        NoReferrerWhenDowngrade | StrictOrigin | StrictOriginWhenCrossOrigin => downgrade,
        _ => false,
    };
    Some(if suppress { "null" } else { origin }.into())
}
