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
    pub fn parse_list(value: &str) -> Self {
        value
            .split(',')
            .filter_map(|token| match token.trim().to_ascii_lowercase().as_str() {
                "no-referrer" => Some(Self::NoReferrer),
                "no-referrer-when-downgrade" => Some(Self::NoReferrerWhenDowngrade),
                "origin" => Some(Self::Origin),
                "origin-when-cross-origin" => Some(Self::OriginWhenCrossOrigin),
                "same-origin" => Some(Self::SameOrigin),
                "strict-origin" => Some(Self::StrictOrigin),
                "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
                "unsafe-url" => Some(Self::UnsafeUrl),
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
        let full = sanitized(source);
        let origin = serialized_origin(source);
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
}
