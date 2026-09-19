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
    NativeOpaque(String),
    BrokerOpaque(String),
}

impl Origin {
    /// Retained native origin storage, including reserved String capacity.
    pub fn retained_bytes(&self) -> Option<usize> {
        let owned = match &self.0 {
            Identity::Tuple { scheme, host, .. } => scheme.capacity().checked_add(match host {
                url::Host::Domain(value) => value.capacity(),
                _ => 0,
            })?,
            Identity::NativeOpaque(value) | Identity::BrokerOpaque(value) => value.capacity(),
            Identity::Opaque(_) => 0,
        };
        std::mem::size_of::<Self>().checked_add(owned)
    }

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

    /// Native embedder transport only. Opaque authority is namespaced away
    /// from this process's locally allocated opaque IDs. Author origin strings
    /// always use from_url and cannot instantiate an inherited native identity.
    pub fn from_native_authority(
        serialized: &str,
        opaque_key: Option<&str>,
    ) -> Result<Self, &'static str> {
        if let Some(key) = opaque_key {
            if serialized != "null" || !valid_broker_opaque_key(key) {
                return Err("invalid native opaque origin authority");
            }
            return Ok(Self(Identity::NativeOpaque(key.into())));
        }
        let origin = Self::from_url(serialized);
        if origin.is_opaque() || origin.serialize() != serialized {
            return Err("invalid native tuple origin authority");
        }
        Ok(origin)
    }

    /// Parent broker only: reconstruct a previously authenticated registry
    /// identity. Callers must never pass an author-provided serialized token.
    pub fn from_broker_authority(serialized: &str, key: &str) -> Result<Self, &'static str> {
        if serialized != "null" {
            let origin = Self::from_native_authority(serialized, None)?;
            return if key == origin.security_token_key() {
                Ok(origin)
            } else {
                Err("broker tuple key mismatch")
            };
        }
        if !valid_broker_opaque_key(key) {
            return Err("invalid broker opaque identity");
        }
        if let Some(id) = key
            .strip_prefix("oryn:opaque:")
            .and_then(|id| id.parse::<u64>().ok())
        {
            return Ok(Self(Identity::Opaque(id)));
        }
        Ok(Self(Identity::BrokerOpaque(key.into())))
    }

    pub fn serialize(&self) -> String {
        match &self.0 {
            Identity::Tuple { scheme, host, port } => {
                url::Origin::Tuple(scheme.clone(), host.clone(), *port).ascii_serialization()
            }
            Identity::Opaque(_) | Identity::NativeOpaque(_) | Identity::BrokerOpaque(_) => {
                "null".into()
            }
        }
    }

    pub const fn is_opaque(&self) -> bool {
        matches!(
            self.0,
            Identity::Opaque(_) | Identity::NativeOpaque(_) | Identity::BrokerOpaque(_)
        )
    }

    /// Embedder-only token key, never a page-visible origin string.
    pub fn security_token_key(&self) -> String {
        match &self.0 {
            Identity::Tuple { .. } => format!("oryn:origin:{}", self.serialize()),
            Identity::Opaque(id) => format!("oryn:opaque:{id}"),
            Identity::NativeOpaque(key) => format!("oryn:remote-opaque:{key}"),
            Identity::BrokerOpaque(key) => key.clone(),
        }
    }
}

/// A document's DOM access identity. This deliberately does not replace `Origin`:
/// storage keys, Fetch, postMessage targetOrigin and origin serialization stay strict.
#[derive(Debug, Clone)]
pub struct DomainAccessIdentity {
    origin: Origin,
    relaxed_domain: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainError {
    OpaqueOrigin,
    InvalidHost,
    NotDomainSuffix,
    PublicSuffix,
}

impl DomainAccessIdentity {
    pub fn new(origin: Origin) -> Self {
        Self {
            origin,
            relaxed_domain: None,
        }
    }
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn relaxed_domain(&self) -> Option<&str> {
        self.relaxed_domain.as_deref()
    }
    /// The domain getter returns the effective domain, or the empty string for
    /// opaque origins. A host-sized assignment is still an explicit relaxation.
    pub fn domain(&self) -> String {
        self.relaxed_domain
            .clone()
            .unwrap_or_else(|| match &self.origin.0 {
                Identity::Tuple { host, .. } => host.to_string(),
                _ => String::new(),
            })
    }
    /// Browsing-context, sandbox and origin-keyed-agent-cluster checks belong to
    /// the native Document setter and must precede this state mutation.
    pub fn set_domain(&mut self, candidate: &str) -> Result<(), DomainError> {
        let domain =
            validate_domain_candidate(&self.origin, self.relaxed_domain.as_deref(), candidate)?;
        self.relaxed_domain = Some(domain);
        Ok(())
    }
    pub fn same_origin_domain(&self, other: &Self) -> bool {
        match (&self.relaxed_domain, &other.relaxed_domain) {
            (None, None) => self.origin == other.origin,
            (Some(left), Some(right)) => match (&self.origin.0, &other.origin.0) {
                (
                    Identity::Tuple {
                        scheme: left_scheme,
                        ..
                    },
                    Identity::Tuple {
                        scheme: right_scheme,
                        ..
                    },
                ) => left_scheme == right_scheme && left == right,
                _ => false,
            },
            _ => false,
        }
    }
    /// For native access checks only; never expose as serialized origin or use
    /// as a storage/network key. The namespace distinguishes even same-host sets.
    pub fn security_token_key(&self) -> String {
        match (&self.origin.0, &self.relaxed_domain) {
            (Identity::Tuple { scheme, .. }, Some(domain)) => format!(
                "oryn:domain:{}:{scheme}:{}:{domain}",
                scheme.len(),
                domain.len()
            ),
            _ => self.origin.security_token_key(),
        }
    }
}

/// HTML's registrable-domain-suffix-or-equal check against the current effective
/// domain. URL host parsing supplies IDNA, IPv4 and IPv6 canonicalization; it does
/// not trim whitespace or interpret a full URL as a hostname. Equality precedes
/// the domain/IP and public-suffix restrictions, as required by the algorithm.
pub fn validate_domain_candidate(
    origin: &Origin,
    current_domain: Option<&str>,
    candidate: &str,
) -> Result<String, DomainError> {
    let Identity::Tuple { host, .. } = &origin.0 else {
        return Err(DomainError::OpaqueOrigin);
    };
    let effective = match current_domain {
        Some(value) => url::Host::parse(value).map_err(|_| DomainError::InvalidHost)?,
        None => host.clone(),
    };
    let proposed = url::Host::parse(candidate).map_err(|_| DomainError::InvalidHost)?;
    if proposed == effective {
        return Ok(proposed.to_string());
    }
    let (url::Host::Domain(proposed), url::Host::Domain(effective)) = (proposed, effective) else {
        return Err(DomainError::NotDomainSuffix);
    };
    if proposed.is_empty()
        || !effective
            .strip_suffix(&proposed)
            .is_some_and(|prefix| prefix.ends_with('.'))
    {
        return Err(DomainError::NotDomainSuffix);
    }
    // PSL helpers strip the DNS root dot; compare on that same representation.
    if psl::suffix_str(&proposed)
        .is_none_or(|suffix| suffix == proposed.strip_suffix('.').unwrap_or(&proposed))
    {
        return Err(DomainError::PublicSuffix);
    }
    Ok(proposed)
}

fn valid_broker_opaque_key(key: &str) -> bool {
    if let Some(id) = key.strip_prefix("oryn:opaque:") {
        return id.parse::<u64>().is_ok_and(|id| id != 0);
    }
    if let Some(frame) = key.strip_prefix("opaque-frame:") {
        let mut parts = frame.split(':');
        let Some(token) = parts.next() else {
            return false;
        };
        let Some(generation) = parts.next() else {
            return false;
        };
        let epoch = parts.next();
        return token.len() == 64
            && token.bytes().all(|b| b.is_ascii_hexdigit())
            && generation.parse::<u64>().is_ok()
            && epoch.is_none_or(|epoch| epoch.parse::<u64>().is_ok())
            && parts.next().is_none();
    }
    false
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
    fn transported_opaque_authority_is_inherited_without_local_identity_collision() {
        let local = Origin::opaque();
        let key = local.security_token_key();
        let remote = Origin::from_native_authority("null", Some(&key)).unwrap();
        assert!(remote.is_opaque());
        assert_eq!(remote.serialize(), "null");
        assert_ne!(local, remote);
        assert_eq!(
            remote,
            Origin::from_native_authority("null", Some(&key)).unwrap()
        );
        assert_ne!(remote, Origin::from_url("null"));
        assert!(Origin::from_native_authority("https://a.test", Some(&key)).is_err());
        assert!(Origin::from_native_authority("null", Some("oryn:opaque:0")).is_err());
        assert!(Origin::from_native_authority("null", None).is_err());
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

#[cfg(test)]
mod broker_inheritance_tests {
    use super::*;
    #[test]
    fn authenticated_frame_origin_can_be_inherited_without_parsing_null() {
        let key = format!("opaque-frame:{}:7", "a".repeat(64));
        let parent = Origin::from_broker_authority("null", &key).unwrap();
        assert!(parent.is_opaque());
        assert_eq!(parent.security_token_key(), key);
        let frame = Origin::from_native_authority("null", Some(&key)).unwrap();
        let popup =
            Origin::from_native_authority(&parent.serialize(), Some(&parent.security_token_key()))
                .unwrap();
        assert_eq!(frame, popup);
        assert_ne!(frame, Origin::from_url("null"));
        assert!(Origin::from_broker_authority("null", "opaque-frame:forged:7").is_err());
        assert!(Origin::from_native_authority("https://a.test", Some(&key)).is_err());
    }
    #[test]
    fn broker_root_identity_reconstruction_preserves_actual_local_opaque_identity() {
        let original = Origin::opaque();
        assert_eq!(
            Origin::from_broker_authority("null", &original.security_token_key()).unwrap(),
            original
        );
    }
}

#[cfg(test)]
mod frame_authority_key_tests {
    use super::Origin;
    #[test]
    fn native_frame_opaque_authority_accepts_stable_epoch_and_rejects_extra_components() {
        let key = format!("opaque-frame:{}:3:7", "a".repeat(64));
        assert!(Origin::from_native_authority("null", Some(&key)).is_ok());
        assert!(Origin::from_native_authority("null", Some(&format!("{key}:9"))).is_err());
        assert!(Origin::from_native_authority("https://example.test", Some(&key)).is_err());
    }
}

#[cfg(test)]
mod domain_access_tests {
    use super::*;
    #[test]
    fn relaxation_is_bilateral_and_preserves_strict_identity_and_port() {
        let original = Origin::from_url("https://www.example.test:8443/");
        let mut first = DomainAccessIdentity::new(original.clone());
        let mut second = first.clone();
        assert!(first.same_origin_domain(&second));
        let strict_token = first.security_token_key();
        first.set_domain("www.example.test").unwrap();
        assert!(!first.same_origin_domain(&second));
        assert_ne!(first.security_token_key(), strict_token);
        second.set_domain("www.example.test").unwrap();
        assert!(first.same_origin_domain(&second));
        first.set_domain("example.test").unwrap();
        assert!(!first.same_origin_domain(&second));
        second.set_domain("example.test").unwrap();
        assert!(first.same_origin_domain(&second));
        let mut different_port =
            DomainAccessIdentity::new(Origin::from_url("https://other.example.test:9443"));
        different_port.set_domain("example.test").unwrap();
        assert!(first.same_origin_domain(&different_port));
        assert_eq!(
            first.security_token_key(),
            different_port.security_token_key()
        );
        let mut http = DomainAccessIdentity::new(Origin::from_url("http://www.example.test"));
        http.set_domain("example.test").unwrap();
        assert!(!first.same_origin_domain(&http));
        assert_eq!(first.origin(), &original);
        assert_eq!(first.origin().serialize(), "https://www.example.test:8443");
        assert!(first.set_domain("www.example.test").is_err());
        assert_eq!(first.domain(), "example.test");
    }
    #[test]
    fn host_parsing_and_public_suffix_boundaries_follow_native_url_rules() {
        let origin = Origin::from_url("https://a.b.example.co.uk");
        for candidate in [
            "co.uk",
            "uk",
            ".co.uk",
            "example.com",
            "evil-example.co.uk",
            "https://example.co.uk",
            "example.co.uk:443",
            " example.co.uk",
            "",
        ] {
            assert!(
                validate_domain_candidate(&origin, None, candidate).is_err(),
                "{candidate}"
            );
        }
        assert_eq!(
            validate_domain_candidate(&origin, None, "EXAMPLE.CO.UK").unwrap(),
            "example.co.uk"
        );
        let private = Origin::from_url("https://a.github.io");
        assert_eq!(
            validate_domain_candidate(&private, None, "github.io"),
            Err(DomainError::PublicSuffix)
        );
        let idna = Origin::from_url("https://a.xn--bcher-kva.example");
        assert_eq!(
            validate_domain_candidate(&idna, None, "bücher.example").unwrap(),
            "xn--bcher-kva.example"
        );
        let ip = Origin::from_url("http://127.0.0.1:8080");
        assert_eq!(
            validate_domain_candidate(&ip, None, "127.1").unwrap(),
            "127.0.0.1"
        );
        assert!(validate_domain_candidate(&ip, None, "0.0.1").is_err());
        let ip6 = Origin::from_url("http://[::1]");
        assert_eq!(
            validate_domain_candidate(&ip6, None, "[0:0:0:0:0:0:0:1]").unwrap(),
            "[::1]"
        );
        let suffix = Origin::from_url("https://com");
        assert_eq!(
            validate_domain_candidate(&suffix, None, "com").unwrap(),
            "com"
        );
        let rooted_suffix = Origin::from_url("https://a.example.co.uk.");
        assert_eq!(
            validate_domain_candidate(&rooted_suffix, None, "co.uk."),
            Err(DomainError::PublicSuffix)
        );
        let dotted = Origin::from_url("https://a.example.test.");
        assert!(validate_domain_candidate(&dotted, None, "example.test").is_err());
        assert_eq!(
            validate_domain_candidate(&dotted, None, "example.test.").unwrap(),
            "example.test."
        );
    }
    #[test]
    fn opaque_inheritance_retains_identity_but_cannot_relax() {
        let mut first = DomainAccessIdentity::new(Origin::opaque());
        let inherited = first.clone();
        assert!(first.same_origin_domain(&inherited));
        assert_eq!(first.domain(), "");
        assert_eq!(
            first.set_domain("example.test"),
            Err(DomainError::OpaqueOrigin)
        );
        assert!(first.same_origin_domain(&inherited));
        assert!(!first.same_origin_domain(&DomainAccessIdentity::new(Origin::opaque())));
    }
}
