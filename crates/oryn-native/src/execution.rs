//! Capability-driven execution-domain selection and explicit state handoff.
//!
//! Native Oryn remains the canonical domain. Compatibility adapters consume
//! these contracts from a separate package/feature and never become native
//! runtime dependencies.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use oryn_common::v3::{ExecutionDomain, PageId, SemanticAction, SupportLevel};
use serde::{Deserialize, Serialize};

use crate::runtime::RuntimeError;

pub const EXECUTION_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityId(pub String);

impl From<&str> for CapabilityId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainDescriptor {
    pub domain: ExecutionDomain,
    pub installed: bool,
    pub canonical_semantics: bool,
    pub user_visible: bool,
    pub owns_authenticated_session: bool,
    pub capabilities: BTreeMap<CapabilityId, SupportLevel>,
    #[serde(default)]
    pub known_bad_origins: BTreeSet<String>,
}

impl DomainDescriptor {
    pub fn support(&self, capability: &CapabilityId) -> SupportLevel {
        self.capabilities
            .get(capability)
            .cloned()
            .unwrap_or(SupportLevel::Unsupported)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityLedger {
    pub contract_version: u16,
    pub domains: Vec<DomainDescriptor>,
}

impl CapabilityLedger {
    pub fn new() -> Self {
        Self {
            contract_version: EXECUTION_CONTRACT_VERSION,
            domains: Vec::new(),
        }
    }

    pub fn register(&mut self, descriptor: DomainDescriptor) {
        if let Some(existing) = self
            .domains
            .iter_mut()
            .find(|item| item.domain == descriptor.domain)
        {
            *existing = descriptor;
        } else {
            self.domains.push(descriptor);
        }
    }

    pub fn domain(&self, domain: ExecutionDomain) -> Option<&DomainDescriptor> {
        self.domains.iter().find(|item| item.domain == domain)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceLevel {
    ReadOnly,
    Reversible,
    Consequential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRequirements {
    #[serde(default)]
    pub capabilities: BTreeSet<CapabilityId>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub requires_authenticated_session: bool,
    #[serde(default)]
    pub requires_user_presence: bool,
    /// Set by challenge detection rather than inferred from page content.
    #[serde(default)]
    pub site_challenged: bool,
    pub consequence: ConsequenceLevel,
    #[serde(default)]
    pub explicit_domain: Option<ExecutionDomain>,
}

impl Default for ExecutionRequirements {
    fn default() -> Self {
        Self {
            capabilities: BTreeSet::new(),
            origin: None,
            requires_authenticated_session: false,
            requires_user_presence: false,
            site_challenged: false,
            consequence: ConsequenceLevel::ReadOnly,
            explicit_domain: None,
        }
    }
}

impl ExecutionRequirements {
    /// Apply detector output without allowing suspected challenges or native
    /// failures to masquerade as confirmed site challenges.
    pub fn with_challenge_assessment(
        mut self,
        assessment: &crate::challenge::ChallengeAssessment,
    ) -> Self {
        self.site_challenged = assessment.is_confirmed();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDomainPolicy {
    pub allowed: Vec<ExecutionDomain>,
    #[serde(default)]
    pub preference: Vec<ExecutionDomain>,
    #[serde(default = "default_require_visible_consequential")]
    pub require_user_visible_for_consequential: bool,
    #[serde(default)]
    pub allow_lossy_handoff: bool,
}

const fn default_require_visible_consequential() -> bool {
    true
}

impl Default for ExecutionDomainPolicy {
    fn default() -> Self {
        Self {
            allowed: vec![
                ExecutionDomain::Native,
                ExecutionDomain::Chromium,
                ExecutionDomain::UserBrowser,
            ],
            preference: vec![
                ExecutionDomain::Native,
                ExecutionDomain::Chromium,
                ExecutionDomain::UserBrowser,
            ],
            require_user_visible_for_consequential: true,
            allow_lossy_handoff: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainReasonCode {
    ExplicitSelection,
    NativePreferred,
    RequiredCapabilityUnavailable,
    AuthenticatedSessionRequired,
    UserPresenceRequired,
    ConsequentialActionRequiresVisibleDomain,
    OriginKnownBad,
    SiteChallenge,
    DomainNotInstalled,
    DomainDeniedByPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainRejection {
    pub domain: ExecutionDomain,
    pub reasons: Vec<DomainReasonCode>,
    #[serde(default)]
    pub missing_capabilities: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainDecision {
    pub contract_version: u16,
    pub selected: ExecutionDomain,
    pub reason: DomainReasonCode,
    #[serde(default)]
    pub rejected: Vec<DomainRejection>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainSelectionError {
    #[error("explicit execution domain {0:?} is unavailable or denied")]
    ExplicitDomainUnavailable(ExecutionDomain),
    #[error("no installed execution domain satisfies the task requirements")]
    NoEligibleDomain { rejected: Vec<DomainRejection> },
}

pub fn select_execution_domain(
    ledger: &CapabilityLedger,
    policy: &ExecutionDomainPolicy,
    requirements: &ExecutionRequirements,
) -> Result<DomainDecision, DomainSelectionError> {
    let candidates = requirements
        .explicit_domain
        .map(|domain| vec![domain])
        .unwrap_or_else(|| policy.preference.clone());
    let mut rejected = Vec::new();
    for domain in candidates {
        let mut reasons = Vec::new();
        let mut missing_capabilities = Vec::new();
        let Some(descriptor) = ledger.domain(domain) else {
            reasons.push(DomainReasonCode::DomainNotInstalled);
            rejected.push(DomainRejection {
                domain,
                reasons,
                missing_capabilities,
            });
            continue;
        };
        if !descriptor.installed {
            reasons.push(DomainReasonCode::DomainNotInstalled);
        }
        if !policy.allowed.contains(&domain) {
            reasons.push(DomainReasonCode::DomainDeniedByPolicy);
        }
        if requirements
            .origin
            .as_ref()
            .is_some_and(|origin| descriptor.known_bad_origins.contains(origin))
        {
            reasons.push(DomainReasonCode::OriginKnownBad);
        }
        if requirements.site_challenged && domain == ExecutionDomain::Native {
            reasons.push(DomainReasonCode::SiteChallenge);
        }
        for capability in &requirements.capabilities {
            if descriptor.support(capability) != SupportLevel::Supported {
                missing_capabilities.push(capability.clone());
            }
        }
        if !missing_capabilities.is_empty() {
            reasons.push(DomainReasonCode::RequiredCapabilityUnavailable);
        }
        if requirements.requires_authenticated_session && !descriptor.owns_authenticated_session {
            reasons.push(DomainReasonCode::AuthenticatedSessionRequired);
        }
        if requirements.requires_user_presence && !descriptor.user_visible {
            reasons.push(DomainReasonCode::UserPresenceRequired);
        }
        if requirements.consequence == ConsequenceLevel::Consequential
            && policy.require_user_visible_for_consequential
            && !descriptor.user_visible
        {
            reasons.push(DomainReasonCode::ConsequentialActionRequiresVisibleDomain);
        }
        if reasons.is_empty() {
            let reason = if requirements.explicit_domain.is_some() {
                DomainReasonCode::ExplicitSelection
            } else if domain == ExecutionDomain::Native {
                DomainReasonCode::NativePreferred
            } else if requirements.requires_authenticated_session {
                DomainReasonCode::AuthenticatedSessionRequired
            } else if requirements.requires_user_presence {
                DomainReasonCode::UserPresenceRequired
            } else if requirements.consequence == ConsequenceLevel::Consequential {
                DomainReasonCode::ConsequentialActionRequiresVisibleDomain
            } else if rejected.iter().any(|candidate: &DomainRejection| {
                candidate.reasons.contains(&DomainReasonCode::SiteChallenge)
            }) {
                DomainReasonCode::SiteChallenge
            } else if rejected.iter().any(|candidate: &DomainRejection| {
                candidate
                    .reasons
                    .contains(&DomainReasonCode::OriginKnownBad)
            }) {
                DomainReasonCode::OriginKnownBad
            } else {
                DomainReasonCode::RequiredCapabilityUnavailable
            };
            return Ok(DomainDecision {
                contract_version: EXECUTION_CONTRACT_VERSION,
                selected: domain,
                reason,
                rejected,
            });
        }
        rejected.push(DomainRejection {
            domain,
            reasons,
            missing_capabilities,
        });
    }
    if let Some(domain) = requirements.explicit_domain {
        Err(DomainSelectionError::ExplicitDomainUnavailable(domain))
    } else {
        Err(DomainSelectionError::NoEligibleDomain { rejected })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormValueState {
    pub selector: String,
    pub value: String,
    pub action: SemanticAction,
    #[serde(default)]
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginStorageState {
    pub origin: String,
    pub local: Vec<(String, String)>,
    pub session: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SensitiveCookieHeader(String);

impl SensitiveCookieHeader {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_to_adapter(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SensitiveCookieHeader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SensitiveCookieHeader([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffLoss {
    JavascriptHeap,
    PendingPromises,
    OpenSockets,
    NativeHostObjects,
    HttpOnlyCookiesNotTransferred,
    IndexedDbNotTransferred,
    SecretHandlesRemainAtSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffOptions {
    #[serde(default = "default_true")]
    pub transfer_local_storage: bool,
    #[serde(default = "default_true")]
    pub transfer_session_storage: bool,
    #[serde(default = "default_true")]
    pub transfer_form_values: bool,
    #[serde(default)]
    pub transfer_cookie_header: bool,
    #[serde(default)]
    pub allow_lossy: bool,
    #[serde(default)]
    pub task_metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub checkpoint: Option<String>,
}

const fn default_true() -> bool {
    true
}

impl Default for HandoffOptions {
    fn default() -> Self {
        Self {
            transfer_local_storage: true,
            transfer_session_storage: true,
            transfer_form_values: true,
            transfer_cookie_header: false,
            allow_lossy: false,
            task_metadata: BTreeMap::new(),
            checkpoint: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandoffBundle {
    pub contract_version: u16,
    pub source_domain: ExecutionDomain,
    pub target_domain: ExecutionDomain,
    pub source_page_id: PageId,
    pub url: String,
    pub document_generation: u64,
    pub storage: Vec<OriginStorageState>,
    pub form_values: Vec<FormValueState>,
    pub cookie_header: Option<SensitiveCookieHeader>,
    pub task_metadata: BTreeMap<String, String>,
    pub checkpoint: Option<String>,
    pub losses: Vec<HandoffLoss>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffReceipt {
    pub contract_version: u16,
    pub source_domain: ExecutionDomain,
    pub target_domain: ExecutionDomain,
    pub destination_url: String,
    pub destination_revision: u64,
    pub state_applied: Vec<String>,
    pub losses: Vec<HandoffLoss>,
}

#[async_trait]
pub trait ExecutionDomainAdapter: Send {
    fn descriptor(&self) -> DomainDescriptor;

    async fn accept_handoff(
        &mut self,
        bundle: HandoffBundle,
    ) -> Result<HandoffReceipt, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        domain: ExecutionDomain,
        capabilities: &[&str],
        visible: bool,
        session: bool,
    ) -> DomainDescriptor {
        DomainDescriptor {
            domain,
            installed: true,
            canonical_semantics: domain == ExecutionDomain::Native,
            user_visible: visible,
            owns_authenticated_session: session,
            capabilities: capabilities
                .iter()
                .map(|capability| ((*capability).into(), SupportLevel::Supported))
                .collect(),
            known_bad_origins: BTreeSet::new(),
        }
    }

    #[test]
    fn native_is_selected_when_it_satisfies_requirements() {
        let mut ledger = CapabilityLedger::new();
        ledger.register(descriptor(
            ExecutionDomain::Native,
            &["dom", "layout"],
            false,
            false,
        ));
        let requirements = ExecutionRequirements {
            capabilities: [CapabilityId::from("layout")].into_iter().collect(),
            ..ExecutionRequirements::default()
        };
        let decision =
            select_execution_domain(&ledger, &ExecutionDomainPolicy::default(), &requirements)
                .expect("native is eligible");
        assert_eq!(decision.selected, ExecutionDomain::Native);
        assert_eq!(decision.reason, DomainReasonCode::NativePreferred);
    }

    #[test]
    fn authenticated_consequential_work_selects_visible_user_domain() {
        let mut ledger = CapabilityLedger::new();
        ledger.register(descriptor(ExecutionDomain::Native, &["dom"], false, false));
        ledger.register(descriptor(
            ExecutionDomain::UserBrowser,
            &["dom"],
            true,
            true,
        ));
        let requirements = ExecutionRequirements {
            capabilities: [CapabilityId::from("dom")].into_iter().collect(),
            requires_authenticated_session: true,
            consequence: ConsequenceLevel::Consequential,
            ..ExecutionRequirements::default()
        };
        let decision =
            select_execution_domain(&ledger, &ExecutionDomainPolicy::default(), &requirements)
                .expect("user browser is eligible");
        assert_eq!(decision.selected, ExecutionDomain::UserBrowser);
        assert!(decision.rejected.iter().any(|item| {
            item.domain == ExecutionDomain::Native
                && item
                    .reasons
                    .contains(&DomainReasonCode::AuthenticatedSessionRequired)
        }));
    }

    #[test]
    fn explicit_denied_domain_fails_instead_of_silently_falling_back() {
        let mut ledger = CapabilityLedger::new();
        ledger.register(descriptor(
            ExecutionDomain::Chromium,
            &["paint"],
            false,
            false,
        ));
        let requirements = ExecutionRequirements {
            explicit_domain: Some(ExecutionDomain::Chromium),
            ..ExecutionRequirements::default()
        };
        let policy = ExecutionDomainPolicy {
            allowed: vec![ExecutionDomain::Native],
            ..ExecutionDomainPolicy::default()
        };
        assert_eq!(
            select_execution_domain(&ledger, &policy, &requirements),
            Err(DomainSelectionError::ExplicitDomainUnavailable(
                ExecutionDomain::Chromium
            ))
        );
    }

    #[test]
    fn site_challenge_moves_execution_off_native_with_a_measurable_reason() {
        let mut ledger = CapabilityLedger::new();
        ledger.register(descriptor(ExecutionDomain::Native, &["dom"], false, false));
        ledger.register(descriptor(
            ExecutionDomain::Chromium,
            &["dom"],
            false,
            false,
        ));
        let decision = select_execution_domain(
            &ledger,
            &ExecutionDomainPolicy::default(),
            &ExecutionRequirements {
                site_challenged: true,
                ..ExecutionRequirements::default()
            },
        )
        .expect("select compatibility domain");
        assert_eq!(decision.selected, ExecutionDomain::Chromium);
        assert_eq!(decision.reason, DomainReasonCode::SiteChallenge);
        assert!(
            decision.rejected[0]
                .reasons
                .contains(&DomainReasonCode::SiteChallenge)
        );
    }

    #[test]
    fn user_presence_selects_visible_user_browser() {
        let mut ledger = CapabilityLedger::new();
        ledger.register(descriptor(ExecutionDomain::Native, &["dom"], false, false));
        ledger.register(descriptor(
            ExecutionDomain::UserBrowser,
            &["dom"],
            true,
            true,
        ));
        let decision = select_execution_domain(
            &ledger,
            &ExecutionDomainPolicy::default(),
            &ExecutionRequirements {
                requires_user_presence: true,
                ..ExecutionRequirements::default()
            },
        )
        .expect("select visible browser");
        assert_eq!(decision.selected, ExecutionDomain::UserBrowser);
        assert_eq!(decision.reason, DomainReasonCode::UserPresenceRequired);
    }

    #[test]
    fn known_bad_origin_selects_compatibility_with_reason() {
        let mut native = descriptor(ExecutionDomain::Native, &["dom"], false, false);
        native
            .known_bad_origins
            .insert("https://challenged.test".into());
        let mut ledger = CapabilityLedger::new();
        ledger.register(native);
        ledger.register(descriptor(
            ExecutionDomain::Chromium,
            &["dom"],
            false,
            false,
        ));
        let decision = select_execution_domain(
            &ledger,
            &ExecutionDomainPolicy::default(),
            &ExecutionRequirements {
                origin: Some("https://challenged.test".into()),
                ..ExecutionRequirements::default()
            },
        )
        .expect("select compatibility domain");
        assert_eq!(decision.selected, ExecutionDomain::Chromium);
        assert_eq!(decision.reason, DomainReasonCode::OriginKnownBad);
    }

    #[test]
    fn suspected_challenge_does_not_change_domain_requirements() {
        let suspected = crate::challenge::assess_challenge(403, &[], "Access denied");
        let requirements = ExecutionRequirements::default().with_challenge_assessment(&suspected);
        assert!(!requirements.site_challenged);

        let confirmed = crate::challenge::assess_challenge(
            403,
            &[],
            "<script src='https://challenges.cloudflare.com/turnstile/v0/api.js'></script><input name='cf-turnstile-response'>",
        );
        let requirements = ExecutionRequirements::default().with_challenge_assessment(&confirmed);
        assert!(requirements.site_challenged);
    }
}
