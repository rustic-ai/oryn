use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::IpAddr,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::Zeroize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub allow_loopback: bool,
    pub allow_private_networks: bool,
    pub allowed_schemes: Vec<String>,
    #[serde(default)]
    pub allowed_hosts: BTreeSet<String>,
    #[serde(default)]
    pub denied_hosts: BTreeSet<String>,
    #[serde(default)]
    pub allowed_ports: BTreeSet<u16>,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allow_loopback: false,
            allow_private_networks: false,
            allowed_schemes: vec!["http".into(), "https".into(), "ws".into(), "wss".into()],
            allowed_hosts: BTreeSet::new(),
            denied_hosts: BTreeSet::new(),
            allowed_ports: BTreeSet::new(),
        }
    }
}

impl NetworkPolicy {
    pub fn check_ip(&self, address: IpAddr) -> Result<(), PolicyDenied> {
        if address.is_loopback() && !self.allow_loopback {
            return Err(PolicyDenied::Loopback);
        }
        if is_private(address) && !self.allow_private_networks {
            return Err(PolicyDenied::PrivateNetwork);
        }
        Ok(())
    }

    pub fn check_scheme(&self, scheme: &str) -> Result<(), PolicyDenied> {
        if self.allowed_schemes.iter().any(|item| item == scheme) {
            Ok(())
        } else {
            Err(PolicyDenied::Scheme(scheme.to_string()))
        }
    }

    pub fn check_url(&self, url: &Url) -> Result<(), PolicyDenied> {
        self.check_scheme(url.scheme())?;
        let host = url
            .host_str()
            .ok_or_else(|| PolicyDenied::MissingHost(url.as_str().to_string()))?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if matches_host_set(&self.denied_hosts, &host) {
            return Err(PolicyDenied::HostDenied(host));
        }
        if !self.allowed_hosts.is_empty() && !matches_host_set(&self.allowed_hosts, &host) {
            return Err(PolicyDenied::HostNotAllowed(host));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| PolicyDenied::MissingPort(url.as_str().to_string()))?;
        if !self.allowed_ports.is_empty() && !self.allowed_ports.contains(&port) {
            return Err(PolicyDenied::Port(port));
        }
        Ok(())
    }
}

fn matches_host_set(patterns: &BTreeSet<String>, host: &str) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('.').to_ascii_lowercase();
        host == pattern
            || host
                .strip_suffix(&pattern)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyDenied {
    #[error("loopback access is denied")]
    Loopback,
    #[error("private-network access is denied")]
    PrivateNetwork,
    #[error("URL scheme is denied: {0}")]
    Scheme(String),
    #[error("URL has no host: {0}")]
    MissingHost(String),
    #[error("URL has no usable port: {0}")]
    MissingPort(String),
    #[error("host is denied: {0}")]
    HostDenied(String),
    #[error("host is not in the context allowlist: {0}")]
    HostNotAllowed(String),
    #[error("port is denied: {0}")]
    Port(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsequentialAction {
    CrossOriginNavigation,
    FormSubmission,
    Popup,
    /// Reserved contract value. Native file-input upload is currently
    /// unavailable and capability diagnostics direct callers elsewhere.
    FileUpload,
    Download,
    /// Policy boundary for the context-owned virtual clipboard. Host system
    /// clipboard access remains unavailable.
    Clipboard,
    CredentialFill,
    ProtectedRequest,
    UserBrowserDelegation,
    /// Explicit raw evaluation, selector, or extraction access. The default
    /// remains available for compatibility, but every use is parent-audited.
    RawPageInspection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyRule {
    Allow,
    Deny,
    RequireConfirmation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPolicy {
    pub rules: BTreeMap<ConsequentialAction, PolicyRule>,
    #[serde(default)]
    pub denied_origins: BTreeSet<String>,
    #[serde(default)]
    pub allowed_origins: BTreeSet<String>,
    #[serde(default = "default_max_download_bytes")]
    pub max_download_bytes: usize,
}

const fn default_max_download_bytes() -> usize {
    64 * 1024 * 1024
}

impl Default for ActionPolicy {
    fn default() -> Self {
        let mut rules = BTreeMap::new();
        rules.insert(
            ConsequentialAction::CrossOriginNavigation,
            PolicyRule::Allow,
        );
        rules.insert(ConsequentialAction::FormSubmission, PolicyRule::Allow);
        rules.insert(ConsequentialAction::Popup, PolicyRule::Allow);
        rules.insert(
            ConsequentialAction::FileUpload,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(
            ConsequentialAction::Download,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(
            ConsequentialAction::Clipboard,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(
            ConsequentialAction::CredentialFill,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(
            ConsequentialAction::ProtectedRequest,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(
            ConsequentialAction::UserBrowserDelegation,
            PolicyRule::RequireConfirmation,
        );
        rules.insert(ConsequentialAction::RawPageInspection, PolicyRule::Allow);
        Self {
            rules,
            denied_origins: BTreeSet::new(),
            allowed_origins: BTreeSet::new(),
            max_download_bytes: default_max_download_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub action: ConsequentialAction,
    pub source_origin: String,
    pub target_origin: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allowed,
    Denied,
    ConfirmationRequired,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfirmationToken([u8; 32]);

impl fmt::Debug for ConfirmationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConfirmationToken([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAuditRecord {
    pub sequence: u64,
    pub request: PolicyRequest,
    pub decision: PolicyDecision,
    pub confirmation_consumed: bool,
}

pub struct PolicyEngine {
    policy: ActionPolicy,
    confirmations: Mutex<BTreeMap<ConfirmationToken, PolicyRequest>>,
    audit: Mutex<Vec<SecurityAuditRecord>>,
    next_sequence: AtomicU64,
}

impl PolicyEngine {
    pub fn new(policy: ActionPolicy) -> Self {
        Self {
            policy,
            confirmations: Mutex::new(BTreeMap::new()),
            audit: Mutex::new(Vec::new()),
            next_sequence: AtomicU64::new(1),
        }
    }

    pub fn policy(&self) -> &ActionPolicy {
        &self.policy
    }

    pub fn prepare_confirmation(&self, request: PolicyRequest) -> ConfirmationToken {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("operating-system randomness is required");
        let token = ConfirmationToken(bytes);
        self.confirmations
            .lock()
            .expect("confirmation lock")
            .insert(token, request);
        token
    }

    pub fn authorize(
        &self,
        request: PolicyRequest,
        confirmation: Option<ConfirmationToken>,
    ) -> PolicyDecision {
        self.authorize_inner(request, confirmation, false)
    }

    /// Re-check origin restrictions at the concrete effect boundary after a
    /// parent-side confirmation has already been consumed for this action.
    /// A preconfirmed request can satisfy only `RequireConfirmation`; an
    /// origin or action rule that resolves to `Deny` remains denied.
    pub(crate) fn authorize_preconfirmed(&self, request: PolicyRequest) -> PolicyDecision {
        self.authorize_inner(request, None, true)
    }

    fn authorize_inner(
        &self,
        request: PolicyRequest,
        confirmation: Option<ConfirmationToken>,
        preconfirmed: bool,
    ) -> PolicyDecision {
        let origin_denied = origin_matches(&self.policy.denied_origins, &request.source_origin)
            || request
                .target_origin
                .as_deref()
                .is_some_and(|origin| origin_matches(&self.policy.denied_origins, origin));
        let origin_not_allowed = !self.policy.allowed_origins.is_empty()
            && (!origin_matches(&self.policy.allowed_origins, &request.source_origin)
                || request
                    .target_origin
                    .as_deref()
                    .is_some_and(|origin| !origin_matches(&self.policy.allowed_origins, origin)));
        let rule = if origin_denied || origin_not_allowed {
            PolicyRule::Deny
        } else {
            self.policy
                .rules
                .get(&request.action)
                .copied()
                .unwrap_or(PolicyRule::Deny)
        };
        let mut consumed = false;
        let decision = match rule {
            PolicyRule::Allow => PolicyDecision::Allowed,
            PolicyRule::Deny => PolicyDecision::Denied,
            PolicyRule::RequireConfirmation => {
                let valid = preconfirmed
                    || confirmation.is_some_and(|token| {
                        self.confirmations
                            .lock()
                            .expect("confirmation lock")
                            .remove(&token)
                            .is_some_and(|approved| approved == request)
                    });
                if valid {
                    consumed = !preconfirmed;
                    PolicyDecision::Allowed
                } else {
                    PolicyDecision::ConfirmationRequired
                }
            }
        };
        self.audit
            .lock()
            .expect("security audit lock")
            .push(SecurityAuditRecord {
                sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
                request,
                decision,
                confirmation_consumed: consumed,
            });
        decision
    }

    pub fn audit_records(&self) -> Vec<SecurityAuditRecord> {
        self.audit.lock().expect("security audit lock").clone()
    }
}

fn origin_matches(patterns: &BTreeSet<String>, origin: &str) -> bool {
    patterns.iter().any(|pattern| pattern == origin)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretHandle([u8; 32]);

impl fmt::Debug for SecretHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretHandle([REDACTED])")
    }
}

struct SecretEntry {
    origin: String,
    label: String,
    bytes: Vec<u8>,
}

impl Drop for SecretEntry {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Default)]
pub struct SecretStore {
    entries: Mutex<BTreeMap<SecretHandle, SecretEntry>>,
    labels: Mutex<BTreeMap<(String, String), SecretHandle>>,
}

impl SecretStore {
    pub fn insert(&self, origin: String, label: String, bytes: Vec<u8>) -> SecretHandle {
        let mut handle = [0_u8; 32];
        getrandom::fill(&mut handle).expect("operating-system randomness is required");
        let handle = SecretHandle(handle);
        self.entries.lock().expect("secret store lock").insert(
            handle,
            SecretEntry {
                origin: origin.clone(),
                label: label.clone(),
                bytes,
            },
        );
        self.labels
            .lock()
            .expect("secret label lock")
            .insert((origin, label), handle);
        handle
    }

    pub fn materialize(&self, handle: SecretHandle, origin: &str) -> Result<Vec<u8>, SecretError> {
        let entries = self.entries.lock().expect("secret store lock");
        let entry = entries.get(&handle).ok_or(SecretError::UnknownHandle)?;
        if entry.origin != origin {
            return Err(SecretError::WrongOrigin);
        }
        Ok(entry.bytes.clone())
    }

    pub fn revoke(&self, handle: SecretHandle) -> bool {
        let removed = self
            .entries
            .lock()
            .expect("secret store lock")
            .remove(&handle);
        if let Some(entry) = &removed {
            self.labels
                .lock()
                .expect("secret label lock")
                .remove(&(entry.origin.clone(), entry.label.clone()));
        }
        removed.is_some()
    }

    pub fn handle_for_label(&self, origin: &str, label: &str) -> Result<SecretHandle, SecretError> {
        self.labels
            .lock()
            .expect("secret label lock")
            .get(&(origin.to_owned(), label.to_owned()))
            .copied()
            .ok_or(SecretError::UnknownLabel)
    }

    pub fn labels(&self) -> Vec<String> {
        self.entries
            .lock()
            .expect("secret store lock")
            .values()
            .map(|entry| entry.label.clone())
            .collect()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("secret handle is unknown or revoked")]
    UnknownHandle,
    #[error("secret label is unknown for this origin")]
    UnknownLabel,
    #[error("secret handle is not valid for this origin")]
    WrongOrigin,
    #[error("secret is not valid UTF-8")]
    InvalidUtf8,
}

fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            address.is_private()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_multicast()
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || octets[0] >= 240
        }
        IpAddr::V6(address) => {
            address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || address.segments()[0..2] == [0x2001, 0x0db8]
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private(IpAddr::V4(mapped)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn default_policy_denies_local_targets_and_file_urls() {
        let policy = NetworkPolicy::default();
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Err(PolicyDenied::Loopback)
        );
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            Err(PolicyDenied::PrivateNetwork)
        );
        assert_eq!(
            policy.check_scheme("file"),
            Err(PolicyDenied::Scheme("file".into()))
        );
    }

    #[test]
    fn host_allow_and_deny_lists_cover_subdomains() {
        let mut policy = NetworkPolicy::default();
        policy.allowed_hosts.insert("example.com".into());
        assert!(
            policy
                .check_url(&Url::parse("https://api.example.com/a").unwrap())
                .is_ok()
        );
        assert!(matches!(
            policy.check_url(&Url::parse("https://other.test/a").unwrap()),
            Err(PolicyDenied::HostNotAllowed(_))
        ));
        policy.denied_hosts.insert("api.example.com".into());
        assert!(matches!(
            policy.check_url(&Url::parse("https://api.example.com/a").unwrap()),
            Err(PolicyDenied::HostDenied(_))
        ));
    }

    #[test]
    fn confirmation_is_scoped_and_one_use() {
        let engine = PolicyEngine::new(ActionPolicy::default());
        let request = PolicyRequest {
            action: ConsequentialAction::CredentialFill,
            source_origin: "https://app.example".into(),
            target_origin: None,
            detail: Some("password".into()),
        };
        assert_eq!(
            engine.authorize(request.clone(), None),
            PolicyDecision::ConfirmationRequired
        );
        let token = engine.prepare_confirmation(request.clone());
        assert_eq!(
            engine.authorize(request.clone(), Some(token)),
            PolicyDecision::Allowed
        );
        assert_eq!(
            engine.authorize(request, Some(token)),
            PolicyDecision::ConfirmationRequired
        );
    }

    #[test]
    fn every_origin_in_a_request_must_be_allowed() {
        let mut policy = ActionPolicy::default();
        policy.allowed_origins.insert("https://app.example".into());
        let engine = PolicyEngine::new(policy);
        assert_eq!(
            engine.authorize(
                PolicyRequest {
                    action: ConsequentialAction::CrossOriginNavigation,
                    source_origin: "https://app.example".into(),
                    target_origin: Some("https://evil.example".into()),
                    detail: None,
                },
                None,
            ),
            PolicyDecision::Denied
        );
        assert_eq!(
            engine.authorize(
                PolicyRequest {
                    action: ConsequentialAction::CrossOriginNavigation,
                    source_origin: "https://evil.example".into(),
                    target_origin: Some("https://app.example".into()),
                    detail: None,
                },
                None,
            ),
            PolicyDecision::Denied
        );
    }

    #[test]
    fn secret_handles_are_origin_scoped_and_redacted() {
        let store = SecretStore::default();
        let handle = store.insert(
            "https://app.example".into(),
            "test-password".into(),
            b"correct horse".to_vec(),
        );
        assert!(!format!("{handle:?}").contains("correct horse"));
        assert_eq!(
            store.materialize(handle, "https://app.example").unwrap(),
            b"correct horse"
        );
        assert_eq!(
            store.materialize(handle, "https://evil.example"),
            Err(SecretError::WrongOrigin)
        );
        assert!(store.revoke(handle));
        assert_eq!(
            store.materialize(handle, "https://app.example"),
            Err(SecretError::UnknownHandle)
        );
    }
}
