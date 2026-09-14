//! Structured, process-local origin identity. Serialization is not authority.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPAQUE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin(Identity);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Identity {
    Tuple {
        scheme: String,
        host: url::Host<String>,
        port: u16,
    },
    Opaque(u64),
}

impl Origin {
    pub fn from_url(value: &str) -> Self {
        match url::Url::parse(value).ok().map(|url| url.origin()) {
            Some(url::Origin::Tuple(scheme, host, port)) => {
                Self(Identity::Tuple { scheme, host, port })
            }
            _ => Self::opaque(),
        }
    }

    /// Inheritance must clone this identity, not parse its serialized `null`.
    pub fn opaque() -> Self {
        Self(Identity::Opaque(
            NEXT_OPAQUE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("opaque origin identity space exhausted"),
        ))
    }

    pub fn serialize(&self) -> String {
        match &self.0 {
            Identity::Tuple { scheme, host, port } => {
                url::Origin::Tuple(scheme.clone(), host.clone(), *port).ascii_serialization()
            }
            Identity::Opaque(_) => "null".into(),
        }
    }

    pub const fn is_opaque(&self) -> bool {
        matches!(self.0, Identity::Opaque(_))
    }

    /// Embedder-only token key, never a page-visible origin string.
    pub fn security_token_key(&self) -> String {
        match &self.0 {
            Identity::Tuple { .. } => format!("oryn:origin:{}", self.serialize()),
            Identity::Opaque(id) => format!("oryn:opaque:{id}"),
        }
    }
}

impl From<String> for Origin {
    fn from(value: String) -> Self {
        Self::from_url(&value)
    }
}

impl From<&str> for Origin {
    fn from(value: &str) -> Self {
        Self::from_url(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_normalization_and_blob_origin_follow_url() {
        let origin = Origin::from_url("https://EXAMPLE.test:443/path");
        assert_eq!(origin.serialize(), "https://example.test");
        assert_eq!(origin, Origin::from_url("blob:https://example.test/id"));
        assert_ne!(origin, Origin::from_url("https://example.test:444"));
        assert_ne!(origin, Origin::from_url("http://example.test"));
    }

    #[test]
    fn opaque_urls_and_serialization_never_alias_but_inheritance_does() {
        for url in [
            "null",
            "about:blank",
            "data:text/html,hello",
            "file:///tmp/page",
            "invalid",
        ] {
            let first = Origin::from_url(url);
            let second = Origin::from_url(url);
            assert!(first.is_opaque());
            assert_eq!(first.serialize(), "null");
            assert_ne!(first, second);
            assert_ne!(first.security_token_key(), second.security_token_key());
            assert_eq!(first, first.clone());
        }
    }
}
