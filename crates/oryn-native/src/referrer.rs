use url::Url;

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
