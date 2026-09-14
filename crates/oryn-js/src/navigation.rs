//! Owned navigation data shared by the JS boundary and native loaders.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostResource {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl PostResource {
    /// Validate at each privileged loader boundary, including redirects.
    pub fn valid_for(&self, url: &url::Url) -> bool {
        matches!(url.scheme(), "http" | "https")
            && self.bytes.len() <= 16 * 1024 * 1024
            && !self.content_type.contains(['\r', '\n'])
    }
    /// Fetch redirect semantics: 307/308 retain a POST resource, 301/302/303 do not.
    pub fn redirect(resource: &mut Option<Self>, status: u16) {
        if matches!(status, 301..=303) {
            *resource = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_resources_reject_non_http_and_header_injection() {
        let mut resource = PostResource {
            bytes: vec![1],
            content_type: "text/plain".into(),
        };
        assert!(resource.valid_for(&url::Url::parse("https://example.test/").unwrap()));
        for url in [
            "data:text/plain,ignored",
            "file:///tmp/ignored",
            "about:blank",
        ] {
            assert!(!resource.valid_for(&url::Url::parse(url).unwrap()));
        }
        resource.content_type = "text/plain\r\nx-injected: yes".into();
        assert!(!resource.valid_for(&url::Url::parse("https://example.test/").unwrap()));
    }
}
