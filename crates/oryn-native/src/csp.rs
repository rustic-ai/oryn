//! CSP Level 3 policy parsing and deterministic authorization.
//!
//! Network effects are checked by the privileged parent. The worker receives
//! only a generation-bound compiled snapshot for synchronous inline and DOM
//! sink checks; the policy representation is shared so both sides agree.

use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha384, Sha512};
use url::Url;

pub const CSP_POLICY_VERSION: &str = "csp3-agent-browser-2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CspDisposition {
    Enforce,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CspDestination {
    Document,
    /// Resource-hint initiator; Fetch destination remains the empty string.
    Prefetch,
    Script,
    ScriptAttribute,
    JavascriptNavigation,
    Style,
    StyleAttribute,
    Image,
    Font,
    Frame,
    Connect,
    Object,
    Media,
    Manifest,
    Worker,
    Form,
    Base,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspRequest {
    pub destination: CspDestination,
    pub url: String,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub inline_source: Option<String>,
    #[serde(default)]
    pub parser_inserted: bool,
    #[serde(default)]
    pub initiated_by_trusted_script: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspViolation {
    pub disposition: CspDisposition,
    pub effective_directive: String,
    pub blocked_uri: String,
    /// Local worker-event URL, distinct from the redacted network report URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_event_blocked_uri: Option<String>,
    pub document_uri: String,
    pub source: String,
    pub original_policy: String,
    #[serde(default)]
    pub referrer: String,
    #[serde(default)]
    pub source_file: String,
    #[serde(default)]
    pub sample: String,
    #[serde(default)]
    pub status_code: u16,
    #[serde(default)]
    pub line_number: u32,
    #[serde(default)]
    pub column_number: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CspReportFormat {
    ReportingApi,
    LegacyReportUri,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspReportTarget {
    pub url: String,
    pub format: CspReportFormat,
}

/// Native raw-response digest evidence retained by preload ownership. Opaque
/// responses retain no digest, even when later consumed under another policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScriptHashEvidence {
    pub sha256: String,
    pub sha384: String,
    pub sha512: String,
    pub exposed: bool,
}
impl ScriptHashEvidence {
    pub(crate) fn from_bytes(bytes: &[u8], exposed: bool) -> Self {
        if !exposed {
            return Self::default();
        }
        Self {
            sha256: BASE64.encode(Sha256::digest(bytes)),
            sha384: BASE64.encode(Sha384::digest(bytes)),
            sha512: BASE64.encode(Sha512::digest(bytes)),
            exposed: true,
        }
    }
}

/// Network-only Reporting API payload. It is not a violation event and must
/// never enter ReportingObserver or SecurityPolicyViolationEvent delivery.
#[derive(Debug, Clone)]
pub struct ScriptHashReport {
    pub endpoint: String,
    pub document_url: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspDecision {
    pub allowed: bool,
    pub violations: Vec<CspViolation>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxFlags {
    pub allow_scripts: bool,
    pub allow_same_origin: bool,
    pub allow_forms: bool,
    pub allow_popups: bool,
    pub allow_popups_to_escape_sandbox: bool,
    pub allow_modals: bool,
    #[serde(default)]
    pub allow_storage_access_by_user_activation: bool,
    pub allow_downloads: bool,
    pub allow_top_navigation: bool,
    pub allow_top_navigation_by_user_activation: bool,
    pub allow_top_navigation_to_custom_protocols: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedTypesDirective {
    pub require_script: bool,
    pub policy_names: BTreeSet<String>,
    pub allow_any_policy: bool,
    pub allow_duplicates: bool,
    pub deny_policy_creation: bool,
}

impl TrustedTypesDirective {
    pub fn allows_policy(&self, name: &str, already_created: bool) -> bool {
        if self.deny_policy_creation || already_created && !self.allow_duplicates {
            return false;
        }
        self.allow_any_policy || self.policy_names.contains(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CspPolicy {
    #[serde(default)]
    self_url: Option<String>,
    disposition: CspDisposition,
    directives: BTreeMap<String, Vec<String>>,
    source: String,
    serialized: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspPolicySet {
    #[serde(default)]
    header_policy_present: bool,
    document_url: String,
    #[serde(default)]
    policy_self_url: Option<String>,
    #[serde(default)]
    document_self_url: Option<String>,
    policies: Vec<CspPolicy>,
    #[serde(default)]
    reporting_endpoints: BTreeMap<String, String>,
}

impl CspPolicySet {
    #[cfg(feature = "v8-host")]
    pub(crate) fn retained_bytes(&self) -> Result<usize, String> {
        let mut size = crate::retained_size::Size(std::mem::size_of::<Self>());
        size.string(&self.document_url)?;
        size.optional(&self.policy_self_url)?;
        size.optional(&self.document_self_url)?;
        size.vector(&self.policies)?;
        for policy in &self.policies {
            size.optional(&policy.self_url)?;
            size.string(&policy.source)?;
            size.string(&policy.serialized)?;
            size.tree(&policy.directives)?;
            for (name, values) in &policy.directives {
                size.string(name)?;
                size.vector(values)?;
                for value in values {
                    size.string(value)?;
                }
            }
        }
        size.tree(&self.reporting_endpoints)?;
        for (name, value) in &self.reporting_endpoints {
            size.string(name)?;
            size.string(value)?;
        }
        Ok(size.0)
    }

    /// Export the effective native policy, including meta restrictions but not
    /// directives deliberately ignored when a meta policy was installed.
    pub(crate) fn worker_snapshot(&self) -> oryn_js::worker_wire::AgentPolicy {
        let mut headers: Vec<(String, String)> = self
            .policies
            .iter()
            .map(|policy| {
                let name = match policy.disposition {
                    CspDisposition::Enforce => "content-security-policy",
                    CspDisposition::Report => "content-security-policy-report-only",
                };
                let value = policy
                    .directives
                    .iter()
                    .map(|(name, values)| {
                        if values.is_empty() {
                            name.clone()
                        } else {
                            format!("{} {}", name, values.join(" "))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                (name.into(), value)
            })
            .collect();
        // The snapshot carries the already resolved native endpoint map. Keep
        // legacy group names as JSON strings, and one group per header so a
        // dictionary's entry limit cannot silently truncate inherited policy.
        for (group, url) in &self.reporting_endpoints {
            headers.push((
                "report-to".into(),
                serde_json::json!({
                    "group":group,"endpoints":[{"url":url}]
                })
                .to_string(),
            ));
        }
        oryn_js::worker_wire::AgentPolicy {
            hide_nonces: self.has_header_policy(),
            policy_sources: self
                .policies
                .iter()
                .map(|policy| policy.source.clone())
                .collect(),
            policy_self_urls: self
                .policies
                .iter()
                .map(|policy| policy.self_url.clone())
                .collect(),
            policy_self_url: self.policy_self_url.clone(),
            document_self_url: self.document_self_url.clone(),
            url: self.document_url.clone(),
            headers,
        }
    }
    pub(crate) fn sandbox_headers(&self) -> Vec<(String, String)> {
        self.policies
            .iter()
            .filter(|policy| policy.disposition == CspDisposition::Enforce)
            .filter_map(|policy| {
                policy.directives.get("sandbox").map(|values| {
                    (
                        "content-security-policy".into(),
                        format!("sandbox {}", values.join(" ")),
                    )
                })
            })
            .collect()
    }

    /// The initial auxiliary about:blank inherits its creator's policy
    /// container; allow-popups-to-escape-sandbox removes only sandbox flags.
    pub(crate) fn initial_auxiliary_policy(&self) -> Self {
        let mut inherited = self.clone();
        if self
            .sandbox()
            .is_some_and(|flags| flags.allow_popups_to_escape_sandbox)
        {
            for policy in &mut inherited.policies {
                policy.directives.remove("sandbox");
            }
        }
        inherited
    }
    pub fn new(document_url: impl Into<String>) -> Self {
        Self {
            header_policy_present: false,
            document_url: document_url.into(),
            policy_self_url: None,
            document_self_url: None,
            policies: Vec::new(),
            reporting_endpoints: BTreeMap::new(),
        }
    }

    pub(crate) fn from_agent_policy(policy: &oryn_js::worker_wire::AgentPolicy) -> Self {
        let mut result = Self::from_headers(
            policy.policy_self_url.as_deref().unwrap_or(&policy.url),
            &policy.headers,
        );
        result.header_policy_present = policy.hide_nonces;
        for (entry, source) in result.policies.iter_mut().zip(&policy.policy_sources) {
            entry.source = source.clone();
        }
        result.document_url = policy.url.clone();
        result.policy_self_url = policy.policy_self_url.clone();
        // A sandbox-derived opaque origin takes precedence over supplied or
        // missing optional wire metadata, including historical policy records.
        if result.document_self_url.is_none() {
            result.document_self_url = policy.document_self_url.clone();
        }
        // Blank/srcdoc inherit their creator's document origin. Reconstituting
        // the policy must retain that origin for newly parsed meta directives;
        // data documents deliberately keep their distinct (opaque) origin.
        if result.document_self_url.is_none()
            && Url::parse(&policy.url).is_ok_and(|url| {
                url.scheme() == "about" && matches!(url.path(), "blank" | "srcdoc")
            })
        {
            result.document_self_url = policy.policy_self_url.clone();
        }
        for (entry, origin) in result.policies.iter_mut().zip(&policy.policy_self_urls) {
            entry.self_url = origin.clone();
        }
        result
    }
    pub(crate) fn rebase_document(&mut self, url: impl Into<String>) {
        let url = url.into();
        if self.document_url != url && self.policy_self_url.is_none() {
            self.policy_self_url = Some(self.document_url.clone());
        }
        self.document_url = url;
        self.document_self_url = None;
    }
    /// Inherited policy self origins and the new document's origin are distinct.
    /// Blank/srcdoc preserve the latter; data documents use normal rebasing.
    pub(crate) fn rebase_inherited_document(&mut self, url: impl Into<String>) {
        let origin = self
            .document_self_url
            .clone()
            .unwrap_or_else(|| self.document_url.clone());
        self.rebase_document(url);
        self.document_self_url = Some(origin);
    }
    pub(crate) fn set_document_self_origin(&mut self, serialized: &str) {
        // Opaque native origins cannot match any tuple source. Use a local
        // opaque URL only as CSP's URL parser input, never as native identity.
        self.document_self_url = Some(if serialized == "null" {
            "data:,".into()
        } else {
            serialized.into()
        });
    }
    fn meta_self_url(&self) -> &str {
        self.document_self_url
            .as_deref()
            .unwrap_or(&self.document_url)
    }
    fn self_url(&self) -> &str {
        self.policy_self_url
            .as_deref()
            .unwrap_or(&self.document_url)
    }
    pub fn from_headers(document_url: impl Into<String>, headers: &[(String, String)]) -> Self {
        let mut set = Self::new(document_url);
        set.reporting_endpoints = parse_reporting_endpoints(headers, &set.document_url);
        for (name, value) in headers {
            let disposition = if name.eq_ignore_ascii_case("content-security-policy") {
                Some(CspDisposition::Enforce)
            } else if name.eq_ignore_ascii_case("content-security-policy-report-only") {
                Some(CspDisposition::Report)
            } else {
                None
            };
            if let Some(disposition) = disposition {
                for serialized in split_policy_list(value) {
                    set.install(serialized, disposition, "header");
                }
            }
        }
        if set
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_same_origin)
        {
            set.set_document_self_origin("null");
        }
        set
    }

    pub fn install_meta(&mut self, serialized: &str) {
        let mut policy = parse_policy(serialized, CspDisposition::Enforce, "meta");
        policy.self_url = Some(self.meta_self_url().into());
        for ignored in ["report-uri", "frame-ancestors", "sandbox"] {
            policy.directives.remove(ignored);
        }
        self.policies.push(policy);
    }

    pub fn contains_meta(&self, serialized: &str) -> bool {
        let mut candidate = parse_policy(serialized, CspDisposition::Enforce, "meta");
        candidate.self_url = Some(self.meta_self_url().into());
        for ignored in ["report-uri", "frame-ancestors", "sandbox"] {
            candidate.directives.remove(ignored);
        }
        self.policies.contains(&candidate)
    }

    pub fn has_header_policy(&self) -> bool {
        self.header_policy_present
    }

    pub fn install(&mut self, serialized: &str, disposition: CspDisposition, source: &str) {
        self.header_policy_present |= source == "header";
        self.policies
            .push(parse_policy(serialized, disposition, source));
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Confirm the response policy independently of meta policies already observed
    /// by the parent while response body chunks were in flight.
    pub fn equivalent_response_policy(&self, other: &Self) -> bool {
        self.policies
            .iter()
            .filter(|policy| policy.source == "header")
            .eq(other.policies.iter())
            && self.reporting_endpoints == other.reporting_endpoints
            && (self.policies.is_empty()
                || Url::parse(self.self_url()).ok().map(|url| url.origin())
                    == Url::parse(other.self_url()).ok().map(|url| url.origin()))
    }

    pub fn authorize(&self, request: &CspRequest) -> CspDecision {
        self.authorize_redirect(request, false)
    }

    pub(crate) fn authorize_redirect(&self, request: &CspRequest, redirected: bool) -> CspDecision {
        self.authorize_inline_kind(request, redirected, false)
    }
    /// Match the current hop, but report only the original request URL. CSP's
    /// create-violation-for-request deliberately excludes undisclosed redirect
    /// targets from both events and network reports.
    pub(crate) fn authorize_redirect_from(
        &self,
        request: &CspRequest,
        redirected: bool,
        original_url: &str,
    ) -> CspDecision {
        let mut decision = self.authorize_redirect(request, redirected);
        if request.inline_source.is_none() {
            let blocked = request_url_for_report(original_url, request.destination, &self.document_url);
            for violation in &mut decision.violations {
                violation.blocked_uri = blocked.clone();
                violation.worker_event_blocked_uri = worker_event_url(original_url);
            }
        }
        decision
    }
    pub(crate) fn authorize_speculation_rules(&self, request: &CspRequest) -> CspDecision {
        self.authorize_inline_kind(request, false, true)
    }
    fn authorize_inline_kind(
        &self,
        request: &CspRequest,
        redirected: bool,
        speculation_rules: bool,
    ) -> CspDecision {
        let mut allowed = true;
        let mut violations = Vec::new();
        for policy in &self.policies {
            let document = Url::parse(policy.self_url.as_deref().unwrap_or(self.self_url())).ok();
            if request.destination == CspDestination::Prefetch {
                if !prefetch_policy_allows(policy, document.as_ref(), request, redirected) {
                    if policy.disposition == CspDisposition::Enforce {
                        allowed = false;
                    }
                    violations.push(self.violation(
                        policy,
                        "default-src",
                        blocked_uri(request, &self.document_url),
                        "",
                    ));
                }
                continue;
            }
            let Some((directive, sources)) = effective_sources(policy, request.destination) else {
                continue;
            };
            let permits = (speculation_rules
                && request.destination == CspDestination::Script
                && request.inline_source.is_some()
                && sources
                    .iter()
                    .any(|source| source == "'inline-speculation-rules'"))
                || source_list_allows_redirect(sources, document.as_ref(), request, redirected);
            if !permits {
                if policy.disposition == CspDisposition::Enforce {
                    allowed = false;
                }
                let mut violation = self.violation(
                    policy,
                    directive,
                    blocked_uri(request, &self.document_url),
                    request.inline_source.as_deref().unwrap_or_default(),
                );
                // A fallback source list supplies policy and report-sample, but
                // the violated effective directive remains the checked sink.
                if let Some(effective) = source_directives(request.destination).first() {
                    violation.effective_directive = (*effective).into();
                }
                violations.push(violation);
            }
        }
        if request.inline_source.is_none() {
            for violation in &mut violations {
                violation.worker_event_blocked_uri = worker_event_url(&request.url);
            }
        }
        CspDecision {
            allowed,
            violations,
        }
    }

    pub fn allows_eval(&self, wasm: bool) -> CspDecision {
        let request = CspRequest {
            destination: CspDestination::Script,
            url: if wasm { "wasm-eval" } else { "eval" }.into(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: false,
        };
        let mut allowed = true;
        let mut violations = Vec::new();
        for policy in &self.policies {
            // Eval is governed by script-src, never script-src-elem. Element
            // policy cannot grant or revoke string/wasm compilation authority.
            let Some((directive, sources)) =
                sources_from_candidates(policy, &["script-src", "default-src"])
            else {
                continue;
            };
            let keyword = if wasm {
                "'wasm-unsafe-eval'"
            } else {
                "'unsafe-eval'"
            };
            if !sources.iter().any(|source| source == keyword)
                && !(wasm && sources.iter().any(|source| source == "'unsafe-eval'"))
            {
                if policy.disposition == CspDisposition::Enforce {
                    allowed = false;
                }
                let mut violation = self.violation(policy, directive, request.url.clone(), "");
                violation.effective_directive = "script-src".into();
                violations.push(violation);
            }
        }
        CspDecision {
            allowed,
            violations,
        }
    }

    pub(crate) fn has_enforced_frame_ancestors(&self) -> bool {
        self.policies.iter().any(|policy| {
            policy.disposition == CspDisposition::Enforce
                && policy.directives.contains_key("frame-ancestors")
        })
    }

    pub fn authorize_frame_ancestors(&self, ancestors: &[String]) -> CspDecision {
        let mut allowed = true;
        let mut violations = Vec::new();
        for policy in &self.policies {
            let document = Url::parse(policy.self_url.as_deref().unwrap_or(self.self_url())).ok();
            let Some(sources) = policy.directives.get("frame-ancestors") else {
                continue;
            };
            let permits = ancestors.iter().all(|ancestor| {
                source_list_allows(
                    sources,
                    document.as_ref(),
                    &CspRequest {
                        destination: CspDestination::Document,
                        url: ancestor.clone(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: false,
                        initiated_by_trusted_script: false,
                    },
                )
            });
            if !permits {
                if policy.disposition == CspDisposition::Enforce {
                    allowed = false;
                }
                violations.push(self.violation(
                    policy,
                    "frame-ancestors",
                    ancestors.last().map_or_else(
                        || "opaque".into(),
                        |url| strip_url_for_report(url, &self.document_url),
                    ),
                    "",
                ));
            }
        }
        CspDecision {
            allowed,
            violations,
        }
    }

    pub fn upgraded_url(&self, url: &Url) -> Option<Url> {
        if url.scheme() != "http" && url.scheme() != "ws" {
            return None;
        }
        if !self.policies.iter().any(|policy| {
            policy.disposition == CspDisposition::Enforce
                && policy.directives.contains_key("upgrade-insecure-requests")
        }) {
            return None;
        }
        let mut upgraded = url.clone();
        upgraded
            .set_scheme(if url.scheme() == "http" {
                "https"
            } else {
                "wss"
            })
            .ok()?;
        if url.port() == Some(80) {
            upgraded.set_port(Some(443)).ok()?;
        } else if url.port().is_none() {
            let _ = upgraded.set_port(None);
        }
        Some(upgraded)
    }

    pub fn sandbox(&self) -> Option<SandboxFlags> {
        let policies = self.policies.iter().filter(|policy| {
            policy.disposition == CspDisposition::Enforce
                && matches!(policy.source.as_str(), "header" | "frame-container")
                && policy.directives.contains_key("sandbox")
        });
        let mut result: Option<SandboxFlags> = None;
        for policy in policies {
            let tokens = &policy.directives["sandbox"];
            let flags = SandboxFlags {
                allow_scripts: tokens.iter().any(|v| v == "allow-scripts"),
                allow_same_origin: tokens.iter().any(|v| v == "allow-same-origin"),
                allow_forms: tokens.iter().any(|v| v == "allow-forms"),
                allow_popups: tokens.iter().any(|v| v == "allow-popups"),
                allow_popups_to_escape_sandbox: tokens
                    .iter()
                    .any(|v| v == "allow-popups-to-escape-sandbox"),
                allow_modals: tokens.iter().any(|v| v == "allow-modals"),
                allow_storage_access_by_user_activation: tokens
                    .iter()
                    .any(|v| v == "allow-storage-access-by-user-activation"),
                allow_downloads: tokens.iter().any(|v| v == "allow-downloads"),
                allow_top_navigation: tokens.iter().any(|v| v == "allow-top-navigation"),
                allow_top_navigation_by_user_activation: tokens
                    .iter()
                    .any(|v| v == "allow-top-navigation-by-user-activation"),
                allow_top_navigation_to_custom_protocols: tokens
                    .iter()
                    .any(|v| v == "allow-top-navigation-to-custom-protocols"),
            };
            result = Some(match result {
                None => flags,
                Some(previous) => intersect_sandbox(previous, flags),
            });
        }
        result
    }

    /// Called only after a script response passes status, MIME and integrity
    /// checks. Hashes describe response bytes, never decoded JavaScript text.
    pub(crate) fn script_hash_reports(
        &self,
        subresource_url: &str,
        bytes: &[u8],
        exposed: bool,
    ) -> Vec<ScriptHashReport> {
        self.hash_reports(
            CspDestination::Script,
            "script",
            subresource_url,
            bytes,
            exposed,
        )
    }
    pub(crate) fn hash_reports(
        &self,
        destination: CspDestination,
        request_destination: &str,
        subresource_url: &str,
        bytes: &[u8],
        exposed: bool,
    ) -> Vec<ScriptHashReport> {
        self.hash_reports_from_evidence(
            destination,
            request_destination,
            subresource_url,
            &ScriptHashEvidence::from_bytes(bytes, exposed),
        )
    }
    pub(crate) fn hash_reports_from_evidence(
        &self,
        destination: CspDestination,
        request_destination: &str,
        subresource_url: &str,
        evidence: &ScriptHashEvidence,
    ) -> Vec<ScriptHashReport> {
        let mut reports = Vec::new();
        for policy in &self.policies {
            // Worklet responses use the script-src-elem post-request algorithm,
            // which passes its selected fallback directive to hash reporting.
            let selected = effective_sources(
                policy,
                if matches!(request_destination, "audioworklet" | "paintworklet") {
                    CspDestination::Script
                } else {
                    destination
                },
            );
            let Some((directive, sources)) = selected else {
                continue;
            };
            // Worker/child/default post-request checks delegate to worker-src,
            // whose algorithm does not produce hash reports. Only an actual
            // script-src fallback invokes the shared script post-request path.
            if destination == CspDestination::Worker && directive != "script-src" {
                continue;
            }
            if !matches!(directive, "script-src" | "script-src-elem" | "default-src") {
                continue;
            }

            let Some(algorithm) = ["sha512", "sha384", "sha256"]
                .into_iter()
                .find(|algorithm| {
                    sources
                        .iter()
                        .any(|source| source == &format!("'report-{algorithm}'"))
                })
            else {
                continue;
            };
            let Some(endpoint) = policy
                .directives
                .get("report-to")
                .and_then(|groups| groups.first())
                .and_then(|group| self.reporting_endpoints.get(group))
            else {
                continue;
            };
            let hash = if !evidence.exposed {
                String::new()
            } else {
                let digest = match algorithm {
                    "sha512" => &evidence.sha512,
                    "sha384" => &evidence.sha384,
                    _ => &evidence.sha256,
                };
                format!("{algorithm}-{digest}")
            };
            let document_url = strip_url_for_report(&self.document_url, &self.document_url);
            let body = serde_json::to_vec(&serde_json::json!([{
                "age":0,"type":"csp-hash","url":document_url,
                "body":{"documentURL":document_url,"subresourceURL":subresource_url,
                    "hash":hash,"type":"subresource","destination":request_destination}
            }]))
            .expect("CSP hash report JSON");
            reports.push(ScriptHashReport {
                endpoint: endpoint.clone(),
                document_url,
                body,
            });
        }
        reports
    }

    /// Resolve the reporting destinations for the policy which produced a
    /// violation. A present `report-to` directive takes precedence over the
    /// deprecated `report-uri` directive so a violation is never duplicated.
    pub fn report_targets(&self, violation: &CspViolation) -> Vec<CspReportTarget> {
        let Some(policy) = self.policies.iter().find(|policy| {
            policy.serialized == violation.original_policy
                && policy.disposition == violation.disposition
        }) else {
            return Vec::new();
        };
        if let Some(groups) = policy.directives.get("report-to") {
            let targets = groups
                .first()
                .and_then(|group| self.reporting_endpoints.get(group))
                .into_iter()
                .map(|url| CspReportTarget {
                    url: url.clone(),
                    format: CspReportFormat::ReportingApi,
                })
                .collect::<Vec<_>>();
            // Supporting report-to suppresses report-uri even when endpoint
            // resolution yields no destination. Do not send a legacy report
            // to a destination the author explicitly superseded.
            return targets;
        }
        policy
            .directives
            .get("report-uri")
            .into_iter()
            .flatten()
            .filter_map(|value| {
                resolve_endpoint(policy.self_url.as_deref().unwrap_or(self.self_url()), value)
            })
            .take(16)
            .map(|url| CspReportTarget {
                url,
                format: CspReportFormat::LegacyReportUri,
            })
            .collect()
    }

    fn violation(
        &self,
        policy: &CspPolicy,
        effective_directive: &str,
        blocked_uri: String,
        inline_source: &str,
    ) -> CspViolation {
        let report_sample = policy
            .directives
            .get(effective_directive)
            .is_some_and(|sources| sources.iter().any(|source| source == "'report-sample'"));
        CspViolation {
            disposition: policy.disposition,
            effective_directive: effective_directive.into(),
            blocked_uri,
            worker_event_blocked_uri: None,
            document_uri: strip_url_for_report(&self.document_url, &self.document_url),
            source: policy.source.clone(),
            original_policy: policy.serialized.clone(),
            referrer: String::new(),
            source_file: String::new(),
            sample: if report_sample {
                inline_source.chars().take(40).collect()
            } else {
                String::new()
            },
            status_code: 0,
            line_number: 0,
            column_number: 0,
        }
    }

    pub fn check_trusted_types(&self, check: &oryn_js::trusted_types::PolicyCheck) -> CspDecision {
        let (mut decision, location) = match check {
            oryn_js::trusted_types::PolicyCheck::SinkRequired => {
                return CspDecision {
                    allowed: self.checks_trusted_type_sinks(),
                    violations: Vec::new(),
                };
            }
            oryn_js::trusted_types::PolicyCheck::Policy {
                name,
                already_created,
                location,
            } => (
                self.authorize_trusted_type_policy(name, *already_created),
                location,
            ),
            oryn_js::trusted_types::PolicyCheck::Sink {
                sink,
                input,
                location,
            } => (self.authorize_trusted_type_sink(sink, input), location),
        };
        for violation in &mut decision.violations {
            violation.source_file = strip_url_for_report(&location.source_file, &self.document_url);
            violation.line_number = location.line;
            violation.column_number = location.column;
        }
        decision
    }

    /// Report-only requirements also invoke the default policy. This query is
    /// separate from whether failed conversion must block the assignment.
    pub fn checks_trusted_type_sinks(&self) -> bool {
        self.policies.iter().any(|policy| {
            policy
                .directives
                .get("require-trusted-types-for")
                .is_some_and(|tokens| tokens.iter().any(|token| token == "'script'"))
        })
    }

    /// Called only after the matching trusted brand/default policy failed to
    /// provide a value. Every applicable policy retains its own disposition.
    pub fn authorize_trusted_type_sink(&self, sink: &str, input: &str) -> CspDecision {
        let mut decision = CspDecision {
            allowed: true,
            violations: Vec::new(),
        };
        for policy in &self.policies {
            if !policy
                .directives
                .get("require-trusted-types-for")
                .is_some_and(|tokens| tokens.iter().any(|token| token == "'script'"))
            {
                continue;
            }
            decision.allowed &= policy.disposition != CspDisposition::Enforce;
            let mut violation = self.violation(
                policy,
                "require-trusted-types-for",
                "trusted-types-sink".into(),
                "",
            );
            let mut sample = oryn_dom::dom_string::units(sink);
            sample.push(u16::from(b'|'));
            sample.extend(oryn_dom::dom_string::units(input).into_iter().take(40));
            violation.sample = oryn_dom::dom_string::from_units(&sample);
            decision.violations.push(violation);
        }
        decision
    }

    /// Policy admission runs after Web IDL conversion, using the name history
    /// retained by the receiver realm rather than mutating CSP state.
    pub fn authorize_trusted_type_policy(&self, name: &str, already_created: bool) -> CspDecision {
        let mut decision = CspDecision {
            allowed: true,
            violations: Vec::new(),
        };
        for policy in &self.policies {
            let Some(tokens) = policy.directives.get("trusted-types") else {
                continue;
            };
            let named = tokens
                .iter()
                .any(|token| token == "*" || (trusted_policy_name_token(token) && token == name));
            let duplicate_allowed =
                !already_created || tokens.iter().any(|token| token == "'allow-duplicates'");
            if named && duplicate_allowed {
                continue;
            }
            decision.allowed &= policy.disposition != CspDisposition::Enforce;
            let mut violation =
                self.violation(policy, "trusted-types", "trusted-types-policy".into(), "");
            let sample = oryn_dom::dom_string::units(name)
                .into_iter()
                .take(40)
                .collect::<Vec<_>>();
            violation.sample = oryn_dom::dom_string::from_units(&sample);
            decision.violations.push(violation);
        }
        decision
    }

    pub fn trusted_types(&self) -> TrustedTypesDirective {
        let mut result = TrustedTypesDirective::default();
        let mut configurations = Vec::new();
        for policy in self
            .policies
            .iter()
            .filter(|policy| policy.disposition == CspDisposition::Enforce)
        {
            if policy
                .directives
                .get("require-trusted-types-for")
                .is_some_and(|tokens| tokens.iter().any(|token| token == "'script'"))
            {
                result.require_script = true;
            }
            if let Some(tokens) = policy.directives.get("trusted-types") {
                configurations.push(tokens);
            }
        }
        if configurations.is_empty() {
            result.allow_any_policy = true;
            result.allow_duplicates = true;
            return result;
        }
        result.allow_any_policy = configurations
            .iter()
            .all(|tokens| tokens.iter().any(|token| token == "*"));
        result.allow_duplicates = configurations
            .iter()
            .all(|tokens| tokens.iter().any(|token| token == "'allow-duplicates'"));
        result.deny_policy_creation = configurations.iter().any(|tokens| {
            !tokens
                .iter()
                .any(|token| token == "*" || trusted_policy_name_token(token))
        });
        // A wildcard directive does not constrain another policy's named set.
        // Quoted 'none' contributes no name; it does not override names or '*'.
        let mut names = configurations
            .iter()
            .filter(|tokens| !tokens.iter().any(|token| token == "*"))
            .map(|tokens| {
                tokens
                    .iter()
                    .filter(|token| trusted_policy_name_token(token))
                    .cloned()
                    .collect::<BTreeSet<_>>()
            });
        result.policy_names = names.next().unwrap_or_default();
        for current in names {
            result.policy_names = result
                .policy_names
                .intersection(&current)
                .cloned()
                .collect();
        }
        result
    }
}

/// Grammar of a named entry in the CSP trusted-types directive. This is
/// intentionally not validation of createPolicy's unrestricted DOMString name.
fn trusted_policy_name_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'#' | b'=' | b'_' | b'/' | b'@' | b'.' | b'%')
        })
}

fn canonical_source_expression(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "'self'"
            | "'none'"
            | "'unsafe-inline'"
            | "'unsafe-eval'"
            | "'wasm-unsafe-eval'"
            | "'strict-dynamic'"
            | "'unsafe-hashes'"
            | "'report-sample'"
            | "'unsafe-allow-redirects'"
            | "'inline-speculation-rules'"
            | "'report-sha256'"
            | "'report-sha384'"
            | "'report-sha512'"
    ) {
        return lower;
    }
    for prefix in ["'nonce-", "'sha256-", "'sha384-", "'sha512-"] {
        if lower.starts_with(prefix) {
            return format!("{prefix}{}", &value[prefix.len()..]);
        }
    }
    value.to_owned()
}

fn parse_policy(serialized: &str, disposition: CspDisposition, source: &str) -> CspPolicy {
    let mut directives = BTreeMap::new();
    for directive in serialized.split(';') {
        let mut tokens = directive.split_ascii_whitespace();
        let Some(name) = tokens.next() else { continue };
        let name = name.to_ascii_lowercase();
        let source_list = matches!(
            name.as_str(),
            "default-src"
                | "script-src"
                | "script-src-elem"
                | "script-src-attr"
                | "style-src"
                | "style-src-elem"
                | "style-src-attr"
                | "img-src"
                | "font-src"
                | "connect-src"
                | "frame-src"
                | "child-src"
                | "worker-src"
                | "object-src"
                | "media-src"
                | "manifest-src"
                | "base-uri"
                | "form-action"
                | "frame-ancestors"
        );
        directives.entry(name).or_insert_with(|| {
            tokens
                .map(|token| {
                    if source_list {
                        canonical_source_expression(token)
                    } else {
                        token.to_owned()
                    }
                })
                .collect()
        });
    }
    CspPolicy {
        self_url: None,
        disposition,
        directives,
        source: source.into(),
        serialized: serialized.trim().into(),
    }
}

pub(crate) fn parse_reporting_endpoints(
    headers: &[(String, String)],
    document_url: &str,
) -> BTreeMap<String, String> {
    let mut endpoints = BTreeMap::new();
    let combined = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("reporting-endpoints"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    if let Some(entries) = parse_reporting_dictionary(&combined, document_url) {
        for (name, url) in entries {
            endpoints.insert(name, url);
        }
    }
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("report-to"))
    {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(value) else {
            continue;
        };
        let groups: Vec<_> = value
            .as_array()
            .map_or_else(|| vec![&value], |items| items.iter().collect());
        for group in groups.into_iter().take(16) {
            let Some(name) = group.get("group").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if group.get("max_age").and_then(serde_json::Value::as_u64) == Some(0) {
                continue;
            }
            let Some(url) = group
                .get("endpoints")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| {
                    items.iter().find_map(|item| {
                        item.get("url")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|url| resolve_endpoint(document_url, url))
                    })
                })
            else {
                continue;
            };
            endpoints.entry(name.to_string()).or_insert(url);
        }
    }
    endpoints
}

fn parse_reporting_dictionary(value: &str, document_url: &str) -> Option<Vec<(String, String)>> {
    if value.len() > 16 * 1024 {
        return None;
    }
    let mut parts = Vec::new();
    let (mut quoted, mut escaped, mut start) = (false, false, 0);
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted
        }
        if !quoted && ch == ',' {
            parts.push(&value[start..index]);
            start = index + 1;
        }
    }
    if quoted || escaped {
        return None;
    }
    parts.push(&value[start..]);
    if parts.len() > 16 {
        return None;
    }
    let mut entries = BTreeMap::new();
    for part in parts {
        let part = part.trim();
        let at = part.find(['=', ';']).unwrap_or(part.len());
        let name = &part[..at];
        let remainder = &part[at..];
        let implicit;
        let value = if let Some(value) = remainder.strip_prefix('=') {
            value
        } else {
            implicit = format!("?1{remainder}");
            &implicit
        };
        let mut chars = name.bytes();
        if !chars
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b == b'*')
            || !chars.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.*-".contains(&b))
        {
            return None;
        }
        let value = value.trim_start();
        let bytes = value.as_bytes();
        if bytes.first() != Some(&b'"') {
            validate_reporting_non_string(value)?;
            entries.insert(name.to_owned(), None);
            continue;
        }
        let mut at = 1;
        let mut url = String::new();
        loop {
            let byte = *bytes.get(at)?;
            at += 1;
            if byte == b'"' {
                break;
            }
            if byte == b'\\' {
                let escaped = *bytes.get(at)?;
                at += 1;
                if !matches!(escaped, b'"' | b'\\') {
                    return None;
                }
                url.push(escaped as char);
            } else {
                if !(0x20..=0x7e).contains(&byte) {
                    return None;
                }
                url.push(byte as char);
            }
        }
        let tail = value[at..].trim();
        if !tail.is_empty() {
            crate::agent_isolation::structured_token(&format!("x{tail}"))?;
        }
        entries.insert(name.to_owned(), resolve_endpoint(document_url, &url));
    }
    Some(
        entries
            .into_iter()
            .filter_map(|(name, url)| url.map(|url| (name, url)))
            .collect(),
    )
}
// Dictionary values may be any structured item or inner list. Only strings
// register endpoints, but valid unrelated members must not invalidate the field.
fn validate_reporting_non_string(value: &str) -> Option<()> {
    if !value.starts_with('(') {
        crate::agent_isolation::structured_token(&format!("x;v={value}"))?;
        return Some(());
    }
    let (mut quoted, mut escaped) = (false, false);
    let mut close = None;
    for (index, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted
        }
        if !quoted && ch == ')' {
            close = Some(index);
            break;
        }
        if !quoted && ch == '(' {
            return None;
        }
    }
    let close = close?;
    let tail = &value[close + 1..];
    crate::agent_isolation::structured_token(&format!("x{tail}"))?;
    let mut remaining = value[1..close].trim_matches(' ');
    while !remaining.is_empty() {
        // A valid maximal item includes all of its parameters, including the
        // optional spaces after semicolons, before the next list item.
        let mut end = None;
        for boundary in remaining
            .char_indices()
            .filter_map(|(at, ch)| (ch == ' ').then_some(at))
            .chain(std::iter::once(remaining.len()))
        {
            if boundary > 0
                && crate::agent_isolation::structured_token(&format!(
                    "x;v={}",
                    &remaining[..boundary]
                ))
                .is_some()
            {
                end = Some(boundary)
            }
        }
        let end = end?;
        remaining = remaining[end..].trim_start_matches(' ');
    }
    Some(())
}

fn resolve_endpoint(document_url: &str, value: &str) -> Option<String> {
    let document = Url::parse(document_url).ok()?;
    let mut endpoint = document.join(value).ok()?;
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return None;
    }
    endpoint.set_fragment(None);
    let secure = endpoint.scheme() == "https"
        || endpoint.scheme() == "http"
            && (document.scheme() == "http"
                || endpoint
                    .host_str()
                    .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1")));
    secure.then(|| endpoint.to_string())
}

fn split_policy_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn source_directives(destination: CspDestination) -> &'static [&'static str] {
    match destination {
        CspDestination::Script | CspDestination::JavascriptNavigation => {
            &["script-src-elem", "script-src", "default-src"]
        }
        CspDestination::ScriptAttribute => &["script-src-attr", "script-src", "default-src"],
        CspDestination::Style => &["style-src-elem", "style-src", "default-src"],
        CspDestination::StyleAttribute => &["style-src-attr", "style-src", "default-src"],
        CspDestination::Image => &["img-src", "default-src"],
        CspDestination::Font => &["font-src", "default-src"],
        CspDestination::Frame => &["frame-src", "child-src", "default-src"],
        CspDestination::Connect => &["connect-src", "default-src"],
        CspDestination::Object => &["object-src", "default-src"],
        CspDestination::Media => &["media-src", "default-src"],
        CspDestination::Manifest => &["manifest-src", "default-src"],
        CspDestination::Worker => &["worker-src", "child-src", "script-src", "default-src"],
        CspDestination::Form => &["form-action"],
        CspDestination::Base => &["base-uri"],
        CspDestination::Prefetch => &["default-src"],
        CspDestination::Document => &[],
    }
}
// Resource hints match a union within EACH policy; policies still intersect.
// Do not use effective_sources here: e.g. script-src-elem must not hide an
// independently permissive script-src. Missing default-src is unrestricted.
fn prefetch_policy_allows(
    policy: &CspPolicy,
    document: Option<&Url>,
    request: &CspRequest,
    redirected: bool,
) -> bool {
    if !policy.directives.contains_key("default-src") {
        return true;
    }
    let Ok(target) = Url::parse(&request.url) else {
        return false;
    };
    [
        "default-src",
        "child-src",
        "connect-src",
        "font-src",
        "frame-src",
        "img-src",
        "manifest-src",
        "media-src",
        "object-src",
        "script-src",
        "script-src-elem",
        "style-src",
        "style-src-elem",
        "worker-src",
    ]
    .iter()
    .any(|name| {
        policy.directives.get(*name).is_some_and(|sources| {
            sources
                .iter()
                .any(|source| source_matches(source, document, &target, redirected))
        })
    })
}

fn effective_sources(policy: &CspPolicy, destination: CspDestination) -> Option<(&str, &[String])> {
    sources_from_candidates(policy, source_directives(destination))
}
fn sources_from_candidates<'a>(
    policy: &'a CspPolicy,
    candidates: &[&'static str],
) -> Option<(&'static str, &'a [String])> {
    candidates.iter().find_map(|name| {
        policy
            .directives
            .get(*name)
            .map(|sources| (*name, sources.as_slice()))
    })
}

fn nonce_or_hash_source(source: &str) -> bool {
    let Some(inner) = source
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    else {
        return false;
    };
    let Some((algorithm, value)) = inner.split_once('-') else {
        return false;
    };
    if !matches!(algorithm, "nonce" | "sha256" | "sha384" | "sha512") {
        return false;
    }
    let unpadded = value.trim_end_matches('=');
    !unpadded.is_empty()
        && value.len() - unpadded.len() <= 2
        && unpadded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_'))
}

fn source_list_allows(sources: &[String], document: Option<&Url>, request: &CspRequest) -> bool {
    source_list_allows_redirect(sources, document, request, false)
}

fn source_list_allows_redirect(
    sources: &[String],
    document: Option<&Url>,
    request: &CspRequest,
    redirected: bool,
) -> bool {
    if sources.len() == 1 && sources[0] == "'none'" {
        return false;
    }
    if request.inline_source.is_some() {
        // A nonce/hash source disables unsafe-inline even when the element
        // supplied neither. Trust does not propagate to new inline source.
        if sources.iter().any(|source| source == "'unsafe-inline'")
            && !sources.iter().any(|source| {
                (source == "'strict-dynamic'"
                    && matches!(
                        request.destination,
                        CspDestination::Script
                            | CspDestination::ScriptAttribute
                            | CspDestination::JavascriptNavigation
                    ))
                    || nonce_or_hash_source(source)
            })
        {
            return true;
        }
        if request.nonce.as_ref().is_some_and(|nonce| {
            sources.iter().any(|source| {
                nonce_or_hash_source(source)
                    && source
                        .strip_prefix("'nonce-")
                        .and_then(|source| source.strip_suffix('\''))
                        == Some(nonce)
            })
        }) && !matches!(
            request.destination,
            CspDestination::ScriptAttribute
                | CspDestination::StyleAttribute
                | CspDestination::JavascriptNavigation
        ) {
            return true;
        }
        let hash_allowed = request.inline_source.as_ref().is_some_and(|source| {
            sources
                .iter()
                .any(|expression| hash_matches(expression, source.as_bytes()))
        });
        return hash_allowed
            && (!matches!(
                request.destination,
                CspDestination::ScriptAttribute
                    | CspDestination::StyleAttribute
                    | CspDestination::JavascriptNavigation
            ) || sources.iter().any(|source| source == "'unsafe-hashes'"));
    }
    if request.nonce.as_ref().is_some_and(|nonce| {
        sources.iter().any(|source| {
            nonce_or_hash_source(source)
                && source
                    .strip_prefix("'nonce-")
                    .and_then(|source| source.strip_suffix('\''))
                    == Some(nonce)
        })
    }) {
        return true;
    }
    if matches!(
        request.destination,
        CspDestination::Script | CspDestination::Worker
    ) && sources.iter().any(|source| source == "'strict-dynamic'")
    {
        // After nonce matching, strict-dynamic replaces host/scheme matching
        // for script-like requests. Parser-inserted roots cannot fall through
        // to a listed host or 'self'; non-parser requests use parser metadata.
        return !request.parser_inserted;
    }
    let Ok(target) = Url::parse(&request.url) else {
        return false;
    };
    sources
        .iter()
        .any(|source| source_matches(source, document, &target, redirected))
}

fn source_matches(source: &str, document: Option<&Url>, target: &Url, redirected: bool) -> bool {
    if source == "*" {
        return !matches!(target.scheme(), "data" | "blob" | "filesystem");
    }
    if source == "'self'" {
        return document.is_some_and(|document| {
            document.origin() == target.origin()
                || document.scheme() == "http"
                    && target.scheme() == "https"
                    && document.host_str() == target.host_str()
                    && (document.port_or_known_default() == target.port_or_known_default()
                        || (document.port_or_known_default() == Some(80)
                            && target.port_or_known_default() == Some(443)))
        });
    }
    if let Some(scheme) = source.strip_suffix(':') {
        return target.scheme() == scheme
            || scheme == "http" && target.scheme() == "https"
            || scheme == "ws" && target.scheme() == "wss";
    }
    let normalized = if source.contains("://") {
        source.to_owned()
    } else if !source.is_empty() && !source.starts_with('\'') {
        format!("{}://{}", document.map_or("https", Url::scheme), source)
    } else {
        return false;
    };
    let Some((scheme_name, rest)) = normalized.split_once("://") else {
        return false;
    };
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, ""), |(authority, path)| (authority, path));
    let any_port = authority.ends_with(":*");
    let authority = if any_port {
        &authority[..authority.len() - 2]
    } else {
        authority
    };
    let wildcard = authority.starts_with("*.");
    let any_host = authority == "*";
    let authority = authority.strip_prefix("*.").unwrap_or(authority);
    let parsed = Url::parse(&format!(
        "{scheme_name}://{}/{path}",
        if any_host {
            "csp-wildcard.invalid"
        } else {
            authority
        }
    ))
    .ok();
    parsed.is_some_and(|allowed| {
        let scheme = allowed.scheme() == target.scheme()
            || allowed.scheme() == "http" && target.scheme() == "https";
        let hosts = any_host
            || allowed
                .host_str()
                .zip(target.host_str())
                .is_some_and(|(allowed, target)| {
                    if wildcard {
                        target.ends_with(&format!(".{allowed}"))
                    } else {
                        target == allowed
                    }
                });
        let ports = any_port
            || allowed.port_or_known_default() == target.port_or_known_default()
            || allowed.scheme() == "http"
                && target.scheme() == "https"
                && allowed.port_or_known_default() == Some(80)
                && target.port_or_known_default() == Some(443);
        let paths = redirected || csp_path_matches(allowed.path(), target.path());
        scheme && hosts && ports && paths
    })
}

fn csp_path_matches(allowed: &str, target: &str) -> bool {
    if allowed == "/" {
        return true;
    }
    fn decode(value: &str) -> Vec<u8> {
        let bytes = value.as_bytes();
        let (mut result, mut index) = (Vec::new(), 0);
        while index < bytes.len() {
            if bytes[index] == b'%'
                && index + 2 < bytes.len()
                && let (Some(a), Some(b)) = (
                    (bytes[index + 1] as char).to_digit(16),
                    (bytes[index + 2] as char).to_digit(16),
                )
            {
                result.push((a * 16 + b) as u8);
                index += 3;
                continue;
            }
            result.push(bytes[index]);
            index += 1;
        }
        result
    }
    let prefix = allowed.ends_with('/');
    let allowed = allowed.split('/').collect::<Vec<_>>();
    let target = target.split('/').collect::<Vec<_>>();
    let count = allowed.len() - usize::from(prefix);
    if target.len() < count || (!prefix && target.len() != count) {
        return false;
    }
    allowed
        .iter()
        .take(count)
        .zip(target.iter())
        .all(|(allowed, target)| decode(allowed) == decode(target))
}

fn hash_matches(expression: &str, source: &[u8]) -> bool {
    let expression = expression.trim_matches('\'');
    let Some((algorithm, expected)) = expression.split_once('-') else {
        return false;
    };
    let actual = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => BASE64.encode(Sha256::digest(source)),
        "sha384" => BASE64.encode(Sha384::digest(source)),
        "sha512" => BASE64.encode(Sha512::digest(source)),
        _ => return false,
    };
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

fn intersect_sandbox(left: SandboxFlags, right: SandboxFlags) -> SandboxFlags {
    SandboxFlags {
        allow_scripts: left.allow_scripts && right.allow_scripts,
        allow_same_origin: left.allow_same_origin && right.allow_same_origin,
        allow_forms: left.allow_forms && right.allow_forms,
        allow_popups: left.allow_popups && right.allow_popups,
        allow_popups_to_escape_sandbox: left.allow_popups_to_escape_sandbox
            && right.allow_popups_to_escape_sandbox,
        allow_modals: left.allow_modals && right.allow_modals,
        allow_storage_access_by_user_activation: left.allow_storage_access_by_user_activation
            && right.allow_storage_access_by_user_activation,
        allow_downloads: left.allow_downloads && right.allow_downloads,
        allow_top_navigation: left.allow_top_navigation && right.allow_top_navigation,
        allow_top_navigation_by_user_activation: left.allow_top_navigation_by_user_activation
            && right.allow_top_navigation_by_user_activation,
        allow_top_navigation_to_custom_protocols: left.allow_top_navigation_to_custom_protocols
            && right.allow_top_navigation_to_custom_protocols,
    }
}

fn blocked_uri(request: &CspRequest, document_url: &str) -> String {
    if request.inline_source.is_some() {
        "inline".into()
    } else {
        request_url_for_report(&request.url, request.destination, document_url)
    }
}

fn request_url_for_report(value: &str, destination: CspDestination, document_url: &str) -> String {
    // WebSocket CSP checks concern the HTTP(S) handshake request, not the
    // WebSocket URL. Other non-HTTP resources retain the scheme-only rule.
    if destination == CspDestination::Connect {
        if let Some(url) = worker_event_url(value) {
            return strip_url_for_report(&url, document_url);
        }
    }
    strip_url_for_report(value, document_url)
}

/// CSP's strip-URL-for-use-in-reports algorithm, shared by initial and frozen
/// redirect producers. Credentials/fragments never enter network reports;
/// HTTP(S) paths and queries remain observable regardless of origin.
pub(crate) fn strip_url_for_report(value: &str, _document_url: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return url.scheme().into();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    url.to_string()
}

fn worker_event_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    // WebSocket CSP observations identify the HTTP(S) handshake URL.
    if url.scheme() == "ws" {
        let _ = url.set_scheme("http");
    } else if url.scheme() == "wss" {
        let _ = url.set_scheme("https");
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn captured_frame_sandbox_is_enforced_and_round_trips_without_header_relabeling() {
        for (tokens, allowed) in [
            ("allow-scripts allow-same-origin", false),
            ("allow-scripts allow-same-origin allow-popups", true),
        ] {
            let mut policy = super::CspPolicySet::new("https://example.test/");
            policy.install(
                &format!("sandbox {tokens}"),
                super::CspDisposition::Enforce,
                "frame-container",
            );
            assert_eq!(policy.sandbox().unwrap().allow_popups, allowed);
            assert!(!policy.has_header_policy());
            let snapshot = policy.worker_snapshot();
            assert_eq!(snapshot.policy_sources, vec!["frame-container"]);
            let restored = super::CspPolicySet::from_agent_policy(&snapshot);
            assert_eq!(restored.sandbox().unwrap().allow_popups, allowed);
            assert!(!restored.has_header_policy());
            assert_eq!(
                restored.worker_snapshot().policy_sources,
                snapshot.policy_sources
            );
            assert_eq!(
                restored
                    .initial_auxiliary_policy()
                    .sandbox()
                    .unwrap()
                    .allow_popups,
                allowed
            );
        }
    }

    #[test]
    fn header_and_frame_sandbox_permissions_intersect_in_both_orders() {
        for reverse in [false, true] {
            for (header, frame, expected) in [
                ("sandbox allow-popups", "sandbox", false),
                ("sandbox", "sandbox allow-popups", false),
                ("sandbox allow-popups", "sandbox allow-popups", true),
            ] {
                let mut policy = super::CspPolicySet::new("https://example.test/");
                let mut inputs = [(header, "header"), (frame, "frame-container")];
                if reverse {
                    inputs.reverse();
                }
                for (text, source) in inputs {
                    policy.install(text, super::CspDisposition::Enforce, source);
                }
                assert_eq!(policy.sandbox().unwrap().allow_popups, expected);
                assert!(policy.has_header_policy());
            }
        }
    }

    #[test]
    fn sandbox_source_admission_and_auxiliary_escape_preserve_other_csp() {
        let mut ignored = super::CspPolicySet::new("https://example.test/");
        ignored.install("sandbox", super::CspDisposition::Report, "frame-container");
        ignored.install("sandbox", super::CspDisposition::Report, "header");
        ignored.install(
            "sandbox",
            super::CspDisposition::Enforce,
            "unrecognized-source",
        );
        ignored.install_meta("sandbox; script-src 'none'");
        assert!(ignored.sandbox().is_none());
        for escape in [false, true] {
            let mut policy = super::CspPolicySet::new("https://example.test/");
            policy.install(
                if escape {
                    "sandbox allow-popups allow-popups-to-escape-sandbox; script-src 'none'"
                } else {
                    "sandbox allow-popups; script-src 'none'"
                },
                super::CspDisposition::Enforce,
                "frame-container",
            );
            let inherited = policy.initial_auxiliary_policy();
            assert_eq!(inherited.sandbox().is_none(), escape);
            assert_eq!(inherited.policies[0].source, "frame-container");
            assert_eq!(
                inherited.policies[0].directives.get("script-src"),
                policy.policies[0].directives.get("script-src")
            );
            assert!(
                policy.sandbox().is_some(),
                "escape must not mutate the creator"
            );
            if escape {
                policy.install(
                    "sandbox allow-popups",
                    super::CspDisposition::Enforce,
                    "header",
                );
                assert!(
                    policy.initial_auxiliary_policy().sandbox().is_some(),
                    "a second enforced policy still denies escape"
                );
            }
        }
    }
    #[test]
    fn storage_access_sandbox_flag_is_conjoined_and_round_trips() {
        let allowed = super::CspPolicySet::from_headers(
            "https://site.example/",
            &[(
                "content-security-policy".into(),
                "sandbox allow-scripts allow-same-origin allow-storage-access-by-user-activation"
                    .into(),
            )],
        );
        assert!(
            allowed
                .sandbox()
                .unwrap()
                .allow_storage_access_by_user_activation
        );
        let inherited = super::CspPolicySet::from_agent_policy(&allowed.worker_snapshot());
        assert!(
            inherited
                .sandbox()
                .unwrap()
                .allow_storage_access_by_user_activation
        );
        let blocked = super::CspPolicySet::from_headers(
            "https://site.example/",
            &[
                (
                    "content-security-policy".into(),
                    "sandbox allow-storage-access-by-user-activation".into(),
                ),
                (
                    "content-security-policy".into(),
                    "sandbox allow-scripts allow-same-origin".into(),
                ),
            ],
        );
        assert!(
            !blocked
                .sandbox()
                .unwrap()
                .allow_storage_access_by_user_activation
        );
    }
    use super::*;

    #[test]
    fn reporting_endpoint_dictionary_preserves_quoted_commas_and_rejects_malformed_items() {
        let mixed = parse_reporting_endpoints(
            &[(
                "reporting-endpoints".into(),
                "one=\"/one\", unused=(1;a=2 \"a,b\"; flag :YQ==: ?1);p=2, one=42, two=\"/two\""
                    .into(),
            )],
            "https://app.test/",
        );
        assert_eq!(mixed.len(), 1);
        assert_eq!(
            mixed.get("two").map(String::as_str),
            Some("https://app.test/two")
        );
        for malformed in [
            "unused=(1 (2)), one=\"/one\"",
            "unused=(1 ?2), one=\"/one\"",
            "unused=(1), one=\"/one\",",
        ] {
            assert!(
                parse_reporting_endpoints(
                    &[("reporting-endpoints".into(), malformed.into())],
                    "https://app.test/"
                )
                .is_empty()
            );
        }

        let combined = parse_reporting_endpoints(
            &[
                (
                    "reporting-endpoints".into(),
                    "unused;flag, one=\"/one\"".into(),
                ),
                ("reporting-endpoints".into(), "two=\"/two\"".into()),
            ],
            "https://app.test/",
        );
        assert_eq!(combined.len(), 2);
        assert!(
            parse_reporting_endpoints(
                &[
                    ("reporting-endpoints".into(), "one=\"/one\"".into()),
                    ("reporting-endpoints".into(), "Broken=\"/two\"".into())
                ],
                "https://app.test/"
            )
            .is_empty()
        );

        let endpoints = parse_reporting_endpoints(
            &[(
                "Reporting-Endpoints".into(),
                "one=\"https://report.test/a,b\", two=\"/report\";priority=1".into(),
            )],
            "https://app.test/page",
        );
        assert_eq!(
            endpoints.get("one").map(String::as_str),
            Some("https://report.test/a,b")
        );
        assert_eq!(
            endpoints.get("two").map(String::as_str),
            Some("https://app.test/report")
        );
        for value in [
            "one=unquoted",
            "one=\"https://report.test/\", Broken=\"/x\"",
            "one=\"/x\\q\"",
        ] {
            assert!(
                parse_reporting_endpoints(
                    &[("reporting-endpoints".into(), value.into())],
                    "https://app.test/"
                )
                .is_empty()
            );
        }
        assert!(
            parse_reporting_endpoints(
                &[(
                    "reporting-endpoints".into(),
                    "one=\"https://secret:password@report.test/x\"".into()
                )],
                "https://app.test/"
            )
            .is_empty()
        );
    }

    #[test]
    fn reporting_urls_preserve_http_paths_and_queries_and_strip_credentials() {
        let document = "https://app.test/private?document=1#fragment";
        assert_eq!(
            strip_url_for_report(
                "https://user:pass@app.test/script.js?q=1#fragment",
                document
            ),
            "https://app.test/script.js?q=1"
        );
        assert_eq!(
            strip_url_for_report(
                "https://user:pass@foreign.test:8443/private?q=secret#fragment",
                document
            ),
            "https://foreign.test:8443/private?q=secret"
        );
        for (value, expected) in [
            ("data:text/javascript,secret", "data"),
            ("blob:https://app.test/private", "blob"),
            ("ws://app.test/private", "ws"),
            ("not a URL", ""),
        ] {
            assert_eq!(strip_url_for_report(value, document), expected);
        }
        let policy = CspPolicySet::from_headers(
            document,
            &[("content-security-policy".into(), "img-src 'none'".into())],
        );
        let violation = policy
            .authorize(&request(
                CspDestination::Image,
                "https://foreign.test/secret?x=1",
            ))
            .violations
            .remove(0);
        assert_eq!(
            violation.document_uri,
            "https://app.test/private?document=1"
        );
        assert_eq!(violation.blocked_uri, "https://foreign.test/secret?x=1");
        assert_eq!(violation.worker_event_blocked_uri.as_deref(),
            Some("https://foreign.test/secret?x=1"));
        let original = "https://user:pass@foreign.test/private?x=1#hidden";
        let redirected = policy.authorize_redirect_from(
            &request(CspDestination::Image, "https://undisclosed.test/secret"),
            true, original,
        );
        assert!(!redirected.allowed);
        assert_eq!(redirected.violations[0].blocked_uri, "https://foreign.test/private?x=1");
        assert_eq!(redirected.violations[0].worker_event_blocked_uri.as_deref(),
            Some("https://foreign.test/private?x=1"));
        assert_eq!(super::worker_event_url("wss://user:pass@foreign.test/socket#hidden"),
            Some("https://foreign.test/socket".into()));
        let connect = CspPolicySet::from_headers(document,
            &[("content-security-policy".into(), "connect-src 'none'".into())]);
        let socket = connect.authorize(&request(CspDestination::Connect,
            "wss://user:pass@foreign.test/echo?query=1#hidden"));
        assert!(!socket.allowed);
        assert_eq!(socket.violations[0].blocked_uri, "https://foreign.test/echo?query=1");
    }

    #[test]
    fn redirected_destination_reports_original_url_without_weakening_hop_admission() {
        for (destination, directive) in [
            (CspDestination::Script, "script-src"),
            (CspDestination::Style, "style-src"),
            (CspDestination::Font, "font-src"),
            (CspDestination::Image, "img-src"),
            (CspDestination::Frame, "frame-src"),
            (CspDestination::Connect, "connect-src"),
            (CspDestination::Worker, "worker-src"),
            (CspDestination::Object, "object-src"),
            (CspDestination::Media, "media-src"),
        ] {
            for (header, enforced) in [
                ("content-security-policy", true),
                ("content-security-policy-report-only", false),
            ] {
                let policy = CspPolicySet::from_headers(
                    "https://source.test/page",
                    &[(
                        header.into(),
                        format!("{directive} https://source.test/exact"),
                    )],
                );
                let decision = policy.authorize_redirect_from(
                    &request(destination, "https://secret.test/undisclosed?token=secret"),
                    true,
                    "https://user:password@source.test/exact?public=1#fragment",
                );
                assert_eq!(decision.allowed, !enforced);
                assert_eq!(decision.violations.len(), 1);
                assert_eq!(
                    decision.violations[0].blocked_uri,
                    "https://source.test/exact?public=1"
                );
                assert!(
                    policy
                        .authorize_redirect_from(
                            &request(destination, "https://source.test/elsewhere"),
                            true,
                            "https://source.test/exact"
                        )
                        .violations
                        .is_empty()
                );
                let foreign = policy.authorize_redirect_from(
                    &request(destination, "https://secret.test/private"),
                    true,
                    "https://public.test/original?q=1",
                );
                assert_eq!(foreign.violations[0].blocked_uri, "https://public.test/original?q=1");
            }
        }
    }

    #[test]
    fn redirect_skips_path_only_and_initial_host_port_path_matching_is_exact() {
        for (destination, directive) in [
            (CspDestination::Script, "script-src"),
            (CspDestination::Style, "style-src"),
            (CspDestination::Font, "font-src"),
            (CspDestination::Image, "img-src"),
            (CspDestination::Frame, "frame-src"),
            (CspDestination::Connect, "connect-src"),
            (CspDestination::Worker, "worker-src"),
        ] {
            let policy = CspPolicySet::from_headers(
                "https://source.test/",
                &[(
                    "content-security-policy".into(),
                    format!("{directive} https://allowed.test/exact"),
                )],
            );
            assert!(
                policy
                    .authorize(&request(destination, "https://allowed.test/exact"))
                    .allowed
            );
            assert!(
                !policy
                    .authorize(&request(destination, "https://allowed.test/exact-more"))
                    .allowed
            );
            assert!(
                policy
                    .authorize_redirect(
                        &request(destination, "https://allowed.test/elsewhere"),
                        true
                    )
                    .allowed
            );
            assert!(
                !policy
                    .authorize_redirect(
                        &request(destination, "https://elsewhere.test/elsewhere"),
                        true
                    )
                    .allowed
            );
            assert!(
                !policy
                    .authorize_redirect(
                        &request(destination, "https://allowed.test:8443/elsewhere"),
                        true
                    )
                    .allowed
            );
        }
        let permits = |source: &str, target: &str| {
            CspPolicySet::from_headers(
                "https://source.test/",
                &[(
                    "content-security-policy".into(),
                    format!("img-src {source}"),
                )],
            )
            .authorize(&request(CspDestination::Image, target))
            .allowed
        };
        assert!(permits(
            "https://*.test:*/dir/",
            "https://a.test:8443/dir/child"
        ));
        assert!(!permits("https://*.test/", "https://test/"));
        assert!(permits("http://localhost/", "https://localhost/"));
        assert!(permits(
            "https://allowed.test/a%20b/",
            "https://allowed.test/a%20b/c"
        ));
        assert!(!permits(
            "https://allowed.test/a%2fb/",
            "https://allowed.test/a/b/c"
        ));
        assert!(permits("https://*:*/", "https://any.test:8443/"));
    }

    #[test]
    fn new_meta_self_uses_local_document_origin_while_inherited_policy_keeps_creator() {
        let mut policy = CspPolicySet::from_headers(
            "https://creator.test/",
            &[("content-security-policy".into(), "img-src 'self'".into())],
        );
        policy.rebase_document("data:text/html,child");
        assert!(
            policy
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/image.png"
                ))
                .allowed
        );
        policy.install_meta("img-src 'self'");
        assert!(
            !policy
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/image.png"
                ))
                .allowed
        );
        let roundtrip = CspPolicySet::from_agent_policy(&policy.worker_snapshot());
        assert!(
            !roundtrip
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/image.png"
                ))
                .allowed
        );
    }

    #[test]
    fn inherited_local_policy_separates_document_reporting_url_from_self_origin() {
        let mut policy = CspPolicySet::from_headers(
            "https://creator.test/path",
            &[("content-security-policy".into(), "img-src 'self'".into())],
        );
        policy.rebase_document("data:text/html,child");
        let snapshot = policy.worker_snapshot();
        assert_eq!(snapshot.url, "data:text/html,child");
        assert_eq!(
            snapshot.policy_self_url.as_deref(),
            Some("https://creator.test/path")
        );
        let restored = CspPolicySet::from_agent_policy(&snapshot);
        assert!(
            restored
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/image.png"
                ))
                .allowed
        );
        let denied = restored.authorize(&request(
            CspDestination::Image,
            "https://foreign.test/image.png",
        ));
        assert!(!denied.allowed);
        assert_eq!(denied.violations[0].document_uri, "data");
        let worker = snapshot.with_document_url("blob:https://creator.test/worker".into());
        assert_eq!(
            worker.policy_self_url.as_deref(),
            Some("https://creator.test/path")
        );
        assert_eq!(worker.url, "blob:https://creator.test/worker");
        assert!(
            CspPolicySet::from_agent_policy(&worker)
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/second.png"
                ))
                .allowed
        );
        let legacy: oryn_js::worker_wire::AgentPolicy =
            serde_json::from_str(r#"{"url":"https://creator.test/","headers":[]}"#).unwrap();
        assert!(legacy.policy_self_url.is_none());
    }

    #[test]
    fn inherited_blank_srcdoc_meta_self_uses_document_origin_without_mutating_parent() {
        let parent = CspPolicySet::from_headers(
            "https://creator.test/path",
            &[("content-security-policy".into(), "img-src 'self'".into())],
        );
        for url in ["about:blank", "about:srcdoc"] {
            let mut child = parent.clone();
            child.rebase_inherited_document(url);
            child.install_meta("img-src 'self'");
            assert!(
                child
                    .authorize(&request(
                        CspDestination::Image,
                        "https://creator.test/p.png"
                    ))
                    .allowed
            );
            assert!(
                !child
                    .authorize(&request(CspDestination::Image, "https://other.test/p.png"))
                    .allowed
            );
            assert_eq!(child.worker_snapshot().url, url);
            assert!(child.contains_meta("img-src 'self'"));
            let roundtrip = CspPolicySet::from_agent_policy(&child.worker_snapshot());
            assert!(
                roundtrip
                    .authorize(&request(
                        CspDestination::Image,
                        "https://creator.test/p.png"
                    ))
                    .allowed
            );
            child.install_meta("img-src 'none'");
            assert!(
                parent
                    .authorize(&request(
                        CspDestination::Image,
                        "https://creator.test/p.png"
                    ))
                    .allowed
            );
        }
        let mut opaque = parent.clone();
        opaque.rebase_inherited_document("about:srcdoc");
        opaque.set_document_self_origin("null");
        opaque.install_meta("img-src 'self'");
        assert!(
            !opaque
                .authorize(&request(
                    CspDestination::Image,
                    "https://creator.test/p.png"
                ))
                .allowed
        );
        let mut blob = parent.clone();
        blob.rebase_document("blob:https://creator.test/id");
        blob.set_document_self_origin("https://creator.test");
        blob.install_meta("img-src 'self'");
        assert!(
            blob.authorize(&request(
                CspDestination::Image,
                "https://creator.test/p.png"
            ))
            .allowed
        );
    }

    fn request(destination: CspDestination, url: &str) -> CspRequest {
        CspRequest {
            destination,
            url: url.into(),
            nonce: None,
            inline_source: None,
            parser_inserted: true,
            initiated_by_trusted_script: false,
        }
    }

    #[test]
    fn response_confirmation_preserves_meta_enforcement_and_rejects_forgery() {
        let url = "https://app.example/index";
        let headers = vec![("Content-Security-Policy".into(), "script-src 'self'".into())];
        let response = CspPolicySet::from_headers(url, &headers);
        let mut installed = response.clone();
        installed.install_meta("script-src 'none'");
        assert!(installed.equivalent_response_policy(&response));
        assert!(
            !installed
                .authorize(&request(CspDestination::Script, "https://app.example/a.js"))
                .allowed
        );
        assert!(!installed.equivalent_response_policy(&CspPolicySet::new(url)));
        for (name, value) in [
            ("Content-Security-Policy", "script-src *"),
            ("Content-Security-Policy-Report-Only", "script-src 'self'"),
        ] {
            let forged = CspPolicySet::from_headers(url, &[(name.into(), value.into())]);
            assert!(!installed.equivalent_response_policy(&forged));
        }
        let mut forged = response.clone();
        forged
            .reporting_endpoints
            .insert("default".into(), "https://evil.example/report".into());
        assert!(!installed.equivalent_response_policy(&forged));
        let other_origin = CspPolicySet::from_headers("https://evil.example/", &headers);
        assert!(!installed.equivalent_response_policy(&other_origin));
    }

    #[test]
    fn meta_only_response_confirmation_does_not_remove_policy() {
        let url = "https://app.example/";
        let response = CspPolicySet::new(url);
        let mut installed = response.clone();
        installed.install_meta("script-src 'none'");
        assert!(installed.equivalent_response_policy(&response));
        assert!(installed.contains_meta("script-src 'none'"));
        assert!(
            !installed
                .authorize(&request(CspDestination::Script, "https://app.example/a.js"))
                .allowed
        );
        assert!(!installed.equivalent_response_policy(&CspPolicySet::new("https://evil.example/")));
    }

    #[test]
    fn multiple_enforced_policies_intersect_and_report_only_does_not_block() {
        let mut policies = CspPolicySet::new("https://app.example/index");
        policies.install(
            "script-src 'self' https://cdn.example",
            CspDisposition::Enforce,
            "header",
        );
        policies.install(
            "script-src https://cdn.example",
            CspDisposition::Enforce,
            "header",
        );
        policies.install("script-src 'none'", CspDisposition::Report, "header");
        let allowed =
            policies.authorize(&request(CspDestination::Script, "https://cdn.example/a.js"));
        assert!(allowed.allowed);
        assert_eq!(allowed.violations.len(), 1);
        let denied =
            policies.authorize(&request(CspDestination::Script, "https://app.example/a.js"));
        assert!(!denied.allowed);
    }

    #[test]
    fn nonces_hashes_and_strict_dynamic_are_enforced() {
        let mut policies = CspPolicySet::new("https://app.example/");
        policies.install(
            "script-src 'nonce-good' 'strict-dynamic'",
            CspDisposition::Enforce,
            "header",
        );
        let mut inline = request(CspDestination::Script, "inline");
        inline.inline_source = Some("boot()".into());
        inline.nonce = Some("good".into());
        assert!(policies.authorize(&inline).allowed);
        let mut dynamic = request(CspDestination::Script, "https://unlisted.example/chunk.js");
        dynamic.parser_inserted = false;
        dynamic.initiated_by_trusted_script = true;
        assert!(policies.authorize(&dynamic).allowed);
    }

    #[test]
    fn strict_dynamic_discards_hosts_only_for_script_like_requests() {
        for directive in [
            "script-src 'self' https: 'strict-dynamic' 'nonce-good'",
            "script-src-elem 'self' https: 'strict-dynamic' 'nonce-good'",
        ] {
            let mut policies = CspPolicySet::new("https://app.example/");
            policies.install(directive, CspDisposition::Enforce, "header");
            let mut external = request(CspDestination::Script, "https://app.example/listed.js");
            external.parser_inserted = true;
            external.initiated_by_trusted_script = true;
            assert!(
                !policies.authorize(&external).allowed,
                "listed host bypassed parser restriction"
            );
            external.nonce = Some("good".into());
            assert!(policies.authorize(&external).allowed);
            external.nonce = None;
            external.parser_inserted = false;
            external.initiated_by_trusted_script = false;
            external.url = "https://unlisted.example/dynamic.js".into();
            assert!(
                policies.authorize(&external).allowed,
                "non-parser metadata determines strict-dynamic admission"
            );
        }
        for (destination, directive) in [
            (CspDestination::Style, "style-src"),
            (CspDestination::Image, "img-src"),
            (CspDestination::Connect, "connect-src"),
        ] {
            let mut policies = CspPolicySet::new("https://app.example/");
            policies.install(
                &format!("{directive} 'self' 'strict-dynamic'"),
                CspDisposition::Enforce,
                "header",
            );
            let mut resource = request(destination, "https://app.example/asset");
            resource.parser_inserted = false;
            resource.initiated_by_trusted_script = true;
            assert!(policies.authorize(&resource).allowed);
            resource.url = "https://unlisted.example/asset".into();
            assert!(!policies.authorize(&resource).allowed);
        }
    }
    #[test]
    fn source_keywords_are_case_insensitive_without_lowercasing_nonce_values() {
        let mut policies = CspPolicySet::new("https://app.example/");
        policies.install(
            "ScRiPt-SrC 'NoNcE-AbCd' 'StRiCt-DyNaMiC' 'SeLf'",
            CspDisposition::Enforce,
            "header",
        );
        let mut script = request(CspDestination::Script, "https://app.example/a.js");
        script.parser_inserted = true;
        assert!(!policies.authorize(&script).allowed);
        script.nonce = Some("abcd".into());
        assert!(!policies.authorize(&script).allowed);
        script.nonce = Some("AbCd".into());
        assert!(policies.authorize(&script).allowed);
        let policy = parse_policy(
            "trusted-types MyPolicy; report-uri /MixedCase",
            CspDisposition::Enforce,
            "header",
        );
        assert_eq!(policy.directives["trusted-types"], vec!["MyPolicy"]);
        assert_eq!(policy.directives["report-uri"], vec!["/MixedCase"]);
        let mut images = CspPolicySet::new("https://app.example/");
        images.install("IMG-SRC 'SELF'", CspDisposition::Enforce, "header");
        assert!(
            images
                .authorize(&request(CspDestination::Image, "https://app.example/a.png"))
                .allowed
        );
    }
    #[test]
    fn meta_ignores_sandbox_and_frame_ancestors() {
        let mut policies = CspPolicySet::new("https://app.example/");
        policies.install_meta("sandbox; frame-ancestors 'none'; script-src 'none'");
        assert!(policies.sandbox().is_none());
        assert!(
            !policies
                .authorize(&request(CspDestination::Script, "https://app.example/a.js"))
                .allowed
        );
    }

    #[test]
    fn trusted_policy_directive_names_require_tokens_but_wildcard_names_do_not() {
        for name in [
            "has?query",
            "colon:name",
            "nonascii-é",
            "contains*star",
            "$name",
        ] {
            let policies = CspPolicySet::from_headers(
                "https://app.test/",
                &[(
                    "Content-Security-Policy".into(),
                    format!("trusted-types {name}"),
                )],
            );
            assert!(!policies.authorize_trusted_type_policy(name, false).allowed);
            assert!(!policies.trusted_types().allows_policy(name, false));
            let wildcard = CspPolicySet::from_headers(
                "https://app.test/",
                &[("Content-Security-Policy".into(), "trusted-types *".into())],
            );
            assert!(wildcard.authorize_trusted_type_policy(name, false).allowed);
            assert!(wildcard.trusted_types().allows_policy(name, false));
        }
        for name in ["none", "allow-duplicates", "A0-#=_/@.%"] {
            let policies = CspPolicySet::from_headers(
                "https://app.test/",
                &[(
                    "Content-Security-Policy".into(),
                    format!("trusted-types {name}"),
                )],
            );
            assert!(policies.authorize_trusted_type_policy(name, false).allowed);
            assert!(policies.trusted_types().allows_policy(name, false));
        }
        let report = CspPolicySet::from_headers(
            "https://app.test/",
            &[(
                "Content-Security-Policy-Report-Only".into(),
                "trusted-types invalid?".into(),
            )],
        );
        let decision = report.authorize_trusted_type_policy("invalid?", false);
        assert!(decision.allowed);
        assert_eq!(decision.violations.len(), 1);
        assert!(report.trusted_types().allows_policy("invalid?", false));
    }

    #[test]
    fn trusted_policy_admission_matrix_preserves_per_policy_duplicates_and_reports() {
        use oryn_js::trusted_types::{PolicyCheck, SourceLocation};
        // Explicit expected outcomes follow pinned should-block-create-policy
        // cases, rather than deriving the oracle from native directive parsing.
        type PolicyCase<'a> = (Vec<(&'a str, CspDisposition)>, [bool; 4], [usize; 4]);
        let cases: Vec<PolicyCase<'_>> = vec![
            (
                vec![("trusted-types B", CspDisposition::Enforce)],
                [false, false, true, false],
                [1, 1, 0, 1],
            ),
            (
                vec![("trusted-types B", CspDisposition::Report)],
                [true, true, true, true],
                [1, 1, 0, 1],
            ),
            (
                vec![("trusted-types *", CspDisposition::Enforce)],
                [true, false, true, false],
                [0, 1, 0, 1],
            ),
            (
                vec![("trusted-types *", CspDisposition::Report)],
                [true, true, true, true],
                [0, 1, 0, 1],
            ),
            (
                vec![(
                    "trusted-types * 'allow-duplicates'",
                    CspDisposition::Enforce,
                )],
                [true, true, true, true],
                [0, 0, 0, 0],
            ),
            (
                vec![(
                    "trusted-types 'none' B 'allow-duplicates'",
                    CspDisposition::Enforce,
                )],
                [false, false, true, true],
                [1, 1, 0, 0],
            ),
            (
                vec![
                    ("trusted-types B", CspDisposition::Enforce),
                    ("trusted-types *", CspDisposition::Report),
                ],
                [false, false, true, false],
                [1, 1, 0, 2],
            ),
            (
                vec![
                    ("trusted-types B", CspDisposition::Report),
                    (
                        "trusted-types * 'allow-duplicates'",
                        CspDisposition::Enforce,
                    ),
                ],
                [true, true, true, true],
                [1, 1, 0, 1],
            ),
        ];
        for (directives, allowed, reports) in cases {
            let mut policies = CspPolicySet::new("https://app.test/");
            for (text, disposition) in &directives {
                policies.install(text, *disposition, "header");
            }
            let mut created = std::collections::BTreeSet::new();
            for (index, name) in ["A", "A", "B", "B"].into_iter().enumerate() {
                let decision = policies.check_trusted_types(&PolicyCheck::Policy {
                    name: name.into(),
                    already_created: created.contains(name),
                    location: SourceLocation::default(),
                });
                assert_eq!(
                    decision.allowed, allowed[index],
                    "{directives:?}, attempt {index}"
                );
                assert_eq!(
                    decision.violations.len(),
                    reports[index],
                    "{directives:?}, attempt {index}"
                );
                for violation in &decision.violations {
                    assert_eq!(violation.effective_directive, "trusted-types");
                    assert_eq!(violation.blocked_uri, "trusted-types-policy");
                    assert_eq!(violation.sample, name);
                    assert!(
                        directives
                            .iter()
                            .any(|(text, disposition)| *text == violation.original_policy
                                && *disposition == violation.disposition)
                    );
                }
                if decision.allowed {
                    created.insert(name);
                }
            }
        }
    }

    #[test]
    fn trusted_type_reports_keep_location_and_strip_private_url_parts() {
        use oryn_js::trusted_types::{PolicyCheck, SourceLocation};
        let mut policies = CspPolicySet::new("https://app.example/page");
        policies.install(
            "require-trusted-types-for 'script'; trusted-types 'none'",
            CspDisposition::Enforce,
            "header",
        );
        policies.install(
            "require-trusted-types-for 'script'; trusted-types 'none'",
            CspDisposition::Report,
            "header",
        );
        for (source_file, expected) in [
            (
                "https://user:secret@app.example/script.js?query=1#fragment",
                "https://app.example/script.js?query=1",
            ),
            (
                "https://other.example/private/script.js?secret=1",
                "https://other.example/private/script.js?secret=1",
            ),
        ] {
            let location = SourceLocation {
                source_file: source_file.into(),
                line: 17,
                column: 9,
            };
            for check in [
                PolicyCheck::Sink {
                    sink: "Element innerHTML".into(),
                    input: "raw".into(),
                    location: location.clone(),
                },
                PolicyCheck::Policy {
                    name: "denied".into(),
                    already_created: false,
                    location: location.clone(),
                },
            ] {
                let decision = policies.check_trusted_types(&check);
                assert!(!decision.allowed);
                assert_eq!(decision.violations.len(), 2);
                for violation in decision.violations {
                    assert_eq!(violation.source_file, expected);
                    assert_eq!(violation.line_number, 17);
                    assert_eq!(violation.column_number, 9);
                }
            }
        }
    }

    #[test]
    fn trusted_type_decisions_retain_individual_policy_reports() {
        let mut policies = CspPolicySet::new("https://app.example/page");
        assert!(!policies.checks_trusted_type_sinks());
        policies.install(
            "require-trusted-types-for 'script'; trusted-types allowed; report-uri /report",
            CspDisposition::Report,
            "header",
        );
        assert!(policies.checks_trusted_type_sinks());
        assert!(!policies.trusted_types().require_script);
        let input = "x".repeat(41);
        let report = policies.authorize_trusted_type_sink("Element innerHTML", &input);
        assert!(report.allowed);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(
            report.violations[0].sample,
            format!("Element innerHTML|{}", "x".repeat(40))
        );
        assert_eq!(report.violations[0].blocked_uri, "trusted-types-sink");
        assert_eq!(report.violations[0].disposition, CspDisposition::Report);
        policies.install(
            "require-trusted-types-for 'script'; trusted-types allowed",
            CspDisposition::Enforce,
            "header",
        );
        let blocked = policies.authorize_trusted_type_sink("HTMLScriptElement text", "input");
        assert!(!blocked.allowed);
        assert_eq!(blocked.violations.len(), 2);
        assert_ne!(
            blocked.violations[0].original_policy,
            blocked.violations[1].original_policy
        );
        assert!(
            policies
                .authorize_trusted_type_policy("allowed", false)
                .violations
                .is_empty()
        );
        let duplicate = policies.authorize_trusted_type_policy("allowed", true);
        assert!(!duplicate.allowed);
        assert_eq!(duplicate.violations.len(), 2);
        assert!(
            duplicate
                .violations
                .iter()
                .all(|v| v.blocked_uri == "trusted-types-policy" && v.sample == "allowed")
        );
        let unpaired = oryn_dom::dom_string::from_units(&[0xd800; 41]);
        let sample = policies
            .authorize_trusted_type_policy(&unpaired, false)
            .violations
            .remove(0)
            .sample;
        assert_eq!(oryn_dom::dom_string::units(&sample), vec![0xd800; 40]);
    }

    #[test]
    fn trusted_type_names_intersect_wildcards_and_ignore_mixed_none() {
        for first in [
            "trusted-types * 'none' 'allow-duplicates'",
            "trusted-types ordinary 'none' 'allow-duplicates'",
        ] {
            let mut policies = CspPolicySet::new("https://app.example/");
            policies.install(first, CspDisposition::Enforce, "header");
            assert!(policies.trusted_types().allows_policy("ordinary", true));
            policies.install(
                "trusted-types ordinary second 'allow-duplicates'",
                CspDisposition::Enforce,
                "header",
            );
            assert!(policies.trusted_types().allows_policy("ordinary", true));
            assert!(!policies.trusted_types().allows_policy("other", false));
            policies.install("trusted-types 'none'", CspDisposition::Enforce, "header");
            assert!(!policies.trusted_types().allows_policy("ordinary", false));
        }
    }

    #[test]
    fn trusted_type_duplicates_require_an_enforcing_name_directive() {
        let mut policies = CspPolicySet::new("https://app.example/");
        assert!(policies.trusted_types().allows_policy("ordinary", true));
        policies.install(
            "require-trusted-types-for 'script'",
            CspDisposition::Enforce,
            "header",
        );
        assert!(policies.trusted_types().require_script);
        assert!(policies.trusted_types().allows_policy("ordinary", true));
        policies.install("trusted-types ordinary", CspDisposition::Report, "header");
        assert!(policies.trusted_types().allows_policy("ordinary", true));
        policies.install("trusted-types ordinary", CspDisposition::Enforce, "header");
        assert!(!policies.trusted_types().allows_policy("ordinary", true));
        assert!(policies.trusted_types().allows_policy("ordinary", false));
    }

    #[test]
    fn trusted_types_and_sandbox_compile_deterministically() {
        let mut policies = CspPolicySet::new("https://app.example/");
        policies.install("require-trusted-types-for 'script'; trusted-types default sanitizer 'allow-duplicates'; sandbox allow-scripts allow-forms", CspDisposition::Enforce, "header");
        let trusted = policies.trusted_types();
        assert!(trusted.require_script);
        assert!(trusted.allows_policy("default", false));
        assert!(!trusted.allows_policy("other", false));
        let sandbox = policies.sandbox().unwrap();
        assert!(sandbox.allow_scripts && sandbox.allow_forms);
        assert!(!sandbox.allow_same_origin && !sandbox.allow_popups);
    }

    #[test]
    fn upgrades_are_enforced_before_network_policy() {
        let mut policies = CspPolicySet::new("https://app.example/");
        policies.install(
            "upgrade-insecure-requests",
            CspDisposition::Enforce,
            "header",
        );
        assert_eq!(
            policies
                .upgraded_url(&Url::parse("http://cdn.example/a.js").unwrap())
                .unwrap()
                .scheme(),
            "https"
        );
    }

    #[test]
    fn frame_ancestors_checks_every_embedding_document() {
        let mut policies = CspPolicySet::new("https://child.example/");
        policies.install(
            "frame-ancestors https://parent.example",
            CspDisposition::Enforce,
            "header",
        );
        assert!(
            policies
                .authorize_frame_ancestors(&["https://parent.example/app".into()])
                .allowed
        );
        assert!(
            !policies
                .authorize_frame_ancestors(&["https://other.example/app".into()])
                .allowed
        );
    }

    #[test]
    fn successful_script_hash_reports_use_policy_algorithms_and_hide_opaque_digest() {
        let policies = CspPolicySet::from_headers(
            "https://app.test/document",
            &[
                (
                    "Reporting-Endpoints".into(),
                    "one=\"/one\", two=\"/two\"".into(),
                ),
                (
                    "Content-Security-Policy".into(),
                    "script-src-elem * 'report-sha256'; report-to one".into(),
                ),
                (
                    "Content-Security-Policy-Report-Only".into(),
                    "default-src 'none' 'report-sha512'; report-to two".into(),
                ),
            ],
        );
        let bytes = b"// Ceci n'est pas un commentaire\n";
        let reports = policies.script_hash_reports("https://remote.test/script.js?1", bytes, true);
        assert_eq!(reports.len(), 2);
        let first: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
        let second: serde_json::Value = serde_json::from_slice(&reports[1].body).unwrap();
        assert_eq!(
            first[0]["body"]["hash"],
            "sha256-1XF/E08XndkoxwN6eIa5J89hYn3OVZ/UyB8BrU5jgzk="
        );
        assert_eq!(
            second[0]["body"]["hash"],
            "sha512-hG4x56V5IhUUepZdYU/lX7UOQJ2M7f6ud2EI7os4JV3OwXSZ002P3zkb9tXQkjpOO8UbtjuEufvdcU67Qt2tlw=="
        );
        assert_eq!(first[0]["type"], "csp-hash");
        assert_eq!(first[0]["body"]["type"], "subresource");
        assert_eq!(
            first[0]["body"]["subresourceURL"],
            "https://remote.test/script.js?1"
        );
        assert_eq!(first[0]["body"]["destination"], "script");
        assert_eq!(reports[0].endpoint, "https://app.test/one");
        assert_eq!(reports[1].endpoint, "https://app.test/two");
        for report in policies.script_hash_reports("https://remote.test/script.js", bytes, false) {
            let body: serde_json::Value = serde_json::from_slice(&report.body).unwrap();
            assert_eq!(body[0]["body"]["hash"], "");
        }
        let no_reports = CspPolicySet::from_headers(
            "https://app.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src * 'report-sha256'; report-uri /legacy".into(),
            )],
        );
        assert!(
            no_reports
                .script_hash_reports("https://app.test/a.js", bytes, true)
                .is_empty()
        );
    }

    #[test]
    fn script_hash_uses_strongest_effective_directive_and_strips_document_credentials() {
        let policy=CspPolicySet::from_headers("https://user:password@app.test/document?q=1#fragment",&[
            ("Reporting-Endpoints".into(),"hash=\"https://app.test/report\"".into()),
            ("Content-Security-Policy".into(),"default-src * 'report-sha512'; script-src * 'report-sha256' 'report-sha384'; report-to hash".into()),
        ]);
        let reports =
            policy.script_hash_reports("https://remote.test/original?query=1", b"abc", true);
        assert_eq!(reports.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
        assert_eq!(body[0]["url"], "https://app.test/document?q=1");
        assert_eq!(
            body[0]["body"]["documentURL"],
            "https://app.test/document?q=1"
        );
        assert_eq!(
            body[0]["body"]["subresourceURL"],
            "https://remote.test/original?query=1"
        );
        assert_eq!(
            body[0]["body"]["hash"],
            "sha384-ywB1P0WjXou1oD1pmsZQBycsMqsO3tFjGotgWkP/W+2AhgcroefMI1i67KE0yCWn"
        );
    }

    #[test]
    fn worklet_hash_uses_effective_element_directive_before_script_fallback() {
        for destination in ["audioworklet", "paintworklet"] {
            let policy = CspPolicySet::from_headers(
                "https://app.test/",
                &[
                    ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                    (
                        "Content-Security-Policy".into(),
                        "script-src-elem *; script-src * 'report-sha256'; report-to hash".into(),
                    ),
                ],
            );
            assert!(
                policy
                    .hash_reports(
                        CspDestination::Script,
                        destination,
                        "https://app.test/code.js",
                        b"abc",
                        true
                    )
                    .is_empty()
            );
            let fallback = CspPolicySet::from_headers(
                "https://app.test/",
                &[
                    ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                    (
                        "Content-Security-Policy".into(),
                        "script-src * 'report-sha384'; report-to hash".into(),
                    ),
                ],
            );
            let reports = fallback.hash_reports(
                CspDestination::Script,
                destination,
                "https://app.test/code.js",
                b"abc",
                true,
            );
            assert_eq!(reports.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
            assert!(
                body[0]["body"]["hash"]
                    .as_str()
                    .unwrap()
                    .starts_with("sha384-")
            );
        }
    }

    #[test]
    fn hash_reports_obey_worker_fallback_and_worklet_script_directives() {
        for (directives, destination, name, expected) in [
            (
                "worker-src * 'report-sha256'",
                CspDestination::Worker,
                "worker",
                false,
            ),
            (
                "child-src * 'report-sha256'",
                CspDestination::Worker,
                "sharedworker",
                false,
            ),
            (
                "script-src * 'report-sha256'",
                CspDestination::Worker,
                "worker",
                true,
            ),
            (
                "default-src * 'report-sha256'",
                CspDestination::Worker,
                "serviceworker",
                false,
            ),
            (
                "script-src-elem * 'report-sha512'; script-src * 'report-sha256'",
                CspDestination::Script,
                "audioworklet",
                true,
            ),
        ] {
            let policy = CspPolicySet::from_headers(
                "https://app.test/",
                &[
                    ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                    (
                        "Content-Security-Policy".into(),
                        format!("{directives}; report-to hash"),
                    ),
                ],
            );
            let reports =
                policy.hash_reports(destination, name, "https://app.test/code.js", b"abc", true);
            assert_eq!(reports.len(), usize::from(expected), "{directives}");
            if expected {
                let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
                assert_eq!(body[0]["body"]["destination"], name);
                assert!(body[0]["body"]["hash"].as_str().unwrap().starts_with(
                    if name == "audioworklet" {
                        "sha512-"
                    } else {
                        "sha256-"
                    }
                ));
            }
        }
    }

    #[test]
    fn modern_reporting_endpoint_precedes_legacy_report_uri() {
        let policies = CspPolicySet::from_headers(
            "https://app.example/private?token=secret#fragment",
            &[
                (
                    "Reporting-Endpoints".into(),
                    "csp=\"https://reports.example/csp\"".into(),
                ),
                (
                    "Content-Security-Policy-Report-Only".into(),
                    "script-src 'none'; report-to csp; report-uri /legacy".into(),
                ),
            ],
        );
        let decision = policies.authorize(&request(
            CspDestination::Script,
            "https://app.example/app.js?secret=yes",
        ));
        assert!(decision.allowed);
        assert_eq!(
            decision.violations[0].document_uri,
            "https://app.example/private?token=secret"
        );
        assert_eq!(
            decision.violations[0].blocked_uri,
            "https://app.example/app.js?secret=yes"
        );
        assert_eq!(
            policies.report_targets(&decision.violations[0]),
            vec![CspReportTarget {
                url: "https://reports.example/csp".into(),
                format: CspReportFormat::ReportingApi,
            }]
        );
    }

    #[test]
    fn report_to_suppresses_legacy_delivery_even_without_resolved_endpoint() {
        for directive in [
            "report-to missing; report-uri /legacy",
            "report-uri /legacy; report-to missing",
            "report-to; report-uri /legacy",
        ] {
            for disposition in [CspDisposition::Enforce, CspDisposition::Report] {
                let mut policies = CspPolicySet::new("https://app.example/");
                policies.install(
                    &format!("script-src 'none'; {directive}"),
                    disposition,
                    "header",
                );
                let decision = policies.authorize(&request(
                    CspDestination::Script,
                    "https://app.example/script.js",
                ));
                assert_eq!(decision.violations.len(), 1);
                assert!(
                    policies.report_targets(&decision.violations[0]).is_empty(),
                    "{directive}"
                );
            }
        }
        // The absence of report-to still selects legacy delivery.
        let policies = CspPolicySet::from_headers(
            "https://app.example/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /legacy".into(),
            )],
        );
        let decision = policies.authorize(&request(
            CspDestination::Script,
            "https://app.example/script.js",
        ));
        assert_eq!(
            policies.report_targets(&decision.violations[0]),
            vec![CspReportTarget {
                url: "https://app.example/legacy".into(),
                format: CspReportFormat::LegacyReportUri,
            }]
        );
    }

    #[test]
    fn report_to_uses_only_first_token_and_survives_meta_installation() {
        let headers = vec![(
            "Reporting-Endpoints".into(),
            "first=\"/first\", second=\"/second\"".into(),
        )];
        for meta in [false, true] {
            for (groups, expected) in [
                ("first second", Some("https://app.example/first")),
                ("missing second", None),
            ] {
                let mut policies = CspPolicySet::from_headers("https://app.example/", &headers);
                let serialized = format!("img-src 'none'; report-to {groups}; report-uri /legacy");
                if meta {
                    policies.install_meta(&serialized);
                    assert!(policies.contains_meta(&serialized));
                } else {
                    policies.install(&serialized, CspDisposition::Enforce, "header");
                }
                let decision = policies.authorize(&request(
                    CspDestination::Image,
                    "https://app.example/image.png",
                ));
                assert_eq!(decision.violations.len(), 1);
                let targets = policies.report_targets(&decision.violations[0]);
                assert_eq!(targets.len(), usize::from(expected.is_some()));
                if let Some(expected) = expected {
                    assert_eq!(targets[0].url, expected);
                    assert_eq!(targets[0].format, CspReportFormat::ReportingApi);
                }
                if meta {
                    assert!(!policies.policies[0].directives.contains_key("report-uri"));
                }
            }
        }
    }

    #[test]
    fn legacy_report_to_header_is_accepted() {
        let policies = CspPolicySet::from_headers(
            "https://app.example/",
            &[
                (
                    "Report-To".into(),
                    r#"{"group":"legacy","max_age":60,"endpoints":[{"url":"/reports"}]}"#.into(),
                ),
                (
                    "Content-Security-Policy".into(),
                    "img-src 'none'; report-to legacy".into(),
                ),
            ],
        );
        let decision = policies.authorize(&request(
            CspDestination::Image,
            "https://app.example/image.png",
        ));
        assert_eq!(
            policies.report_targets(&decision.violations[0])[0].url,
            "https://app.example/reports"
        );
    }
}

#[cfg(test)]
mod auxiliary_inheritance_tests {
    use super::*;
    #[test]
    fn popup_escape_removes_only_inherited_sandbox_not_creator_policy() {
        let policy=CspPolicySet::from_headers("https://creator.test/",&[("content-security-policy".into(),"sandbox allow-popups allow-popups-to-escape-sandbox; script-src 'none'; connect-src 'none'".into())]);
        let inherited = policy.initial_auxiliary_policy();
        assert!(policy.sandbox().is_some());
        assert!(inherited.sandbox().is_none());
        let snapshot = inherited.worker_snapshot();
        assert!(
            snapshot
                .headers
                .iter()
                .any(|(_, value)| value.contains("script-src 'none'"))
        );
        assert!(
            snapshot
                .headers
                .iter()
                .any(|(_, value)| value.contains("connect-src 'none'"))
        );
        assert!(
            snapshot
                .headers
                .iter()
                .all(|(_, value)| !value.contains("sandbox"))
        );
    }
    #[test]
    fn popup_without_escape_retains_all_sandbox_constraints() {
        let policy = CspPolicySet::from_headers(
            "https://creator.test/",
            &[(
                "content-security-policy".into(),
                "sandbox allow-popups; default-src 'self'".into(),
            )],
        );
        let inherited = policy.initial_auxiliary_policy();
        let flags = inherited.sandbox().unwrap();
        assert!(flags.allow_popups);
        assert!(!flags.allow_same_origin);
        assert!(!flags.allow_scripts);
    }
}

#[cfg(test)]
mod inline_nonce_precedence_tests {
    use super::*;
    #[test]
    fn invalid_nonce_and_inapplicable_strict_dynamic_do_not_disable_style_inline() {
        for directive in [
            "style-src 'unsafe-inline' 'nonce-!'",
            "style-src 'unsafe-inline' 'strict-dynamic'",
            "style-src 'none' 'unsafe-inline'",
        ] {
            let mut policy = CspPolicySet::new("https://a.test/");
            policy.install(directive, CspDisposition::Enforce, "header");
            let request = CspRequest {
                destination: CspDestination::Style,
                url: "inline".into(),
                nonce: None,
                inline_source: Some("body{color:red}".into()),
                parser_inserted: true,
                initiated_by_trusted_script: false,
            };
            assert!(policy.authorize(&request).allowed, "{directive}");
        }
    }
    #[test]
    fn nonce_does_not_authorize_script_or_style_attributes() {
        for (directive, destination) in [
            ("script-src 'nonce-good'", CspDestination::ScriptAttribute),
            ("style-src 'nonce-good'", CspDestination::StyleAttribute),
        ] {
            let mut policy = CspPolicySet::new("https://a.test/");
            policy.install(directive, CspDisposition::Enforce, "header");
            let request = CspRequest {
                destination,
                url: "inline".into(),
                nonce: Some("good".into()),
                inline_source: Some("anything".into()),
                parser_inserted: true,
                initiated_by_trusted_script: false,
            };
            assert!(!policy.authorize(&request).allowed);
        }
    }
    #[test]
    fn nonce_or_hash_source_disables_unsafe_inline_without_inherited_script_trust() {
        for directive in [
            "script-src 'unsafe-inline' 'nonce-good'",
            "script-src 'unsafe-inline' 'sha256-YWJj'",
            "script-src 'unsafe-inline' 'nonce-good' 'strict-dynamic'",
        ] {
            let mut policy = CspPolicySet::new("https://a.test/");
            policy.install(directive, CspDisposition::Enforce, "header");
            let mut request = CspRequest {
                destination: CspDestination::Script,
                url: "inline".into(),
                nonce: None,
                inline_source: Some("untrusted()".into()),
                parser_inserted: false,
                initiated_by_trusted_script: true,
            };
            assert!(!policy.authorize(&request).allowed, "{directive}");
            if directive.contains("nonce-good") {
                request.nonce = Some("good".into());
                assert!(policy.authorize(&request).allowed);
            }
        }
    }
}

#[cfg(test)]
mod inline_style_effective_directive_tests {
    use super::*;
    #[test]
    fn fallback_style_reports_sink_directive_and_keeps_fallback_sample() {
        for directive in [
            "default-src 'none' 'report-sample'",
            "style-src 'self' 'report-sample'",
        ] {
            let policy = CspPolicySet::from_headers(
                "https://style.test/",
                &[("Content-Security-Policy".into(), directive.into())],
            );
            for (destination, expected) in [
                (CspDestination::Style, "style-src-elem"),
                (CspDestination::StyleAttribute, "style-src-attr"),
            ] {
                let result = policy.authorize(&CspRequest {
                    destination,
                    url: "inline".into(),
                    nonce: None,
                    inline_source: Some("color:red".into()),
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                });
                assert!(!result.allowed);
                assert_eq!(result.violations[0].effective_directive, expected);
                assert_eq!(result.violations[0].sample, "color:red");
                assert_eq!(result.violations[0].original_policy, directive);
            }
        }
    }
}

#[cfg(test)]
mod effective_directive_matrix_tests {
    use super::*;
    #[test]
    fn fallback_lists_report_actual_sink_for_every_request_destination() {
        let sinks = [
            (CspDestination::Script, "script-src-elem"),
            (CspDestination::ScriptAttribute, "script-src-attr"),
            (CspDestination::Style, "style-src-elem"),
            (CspDestination::StyleAttribute, "style-src-attr"),
            (CspDestination::Image, "img-src"),
            (CspDestination::Font, "font-src"),
            (CspDestination::Frame, "frame-src"),
            (CspDestination::Connect, "connect-src"),
            (CspDestination::Object, "object-src"),
            (CspDestination::Media, "media-src"),
            (CspDestination::Manifest, "manifest-src"),
            (CspDestination::Worker, "worker-src"),
            (CspDestination::Form, "form-action"),
            (CspDestination::Base, "base-uri"),
        ];
        for (destination, expected) in sinks {
            for fallback in source_directives(destination) {
                for disposition in [CspDisposition::Enforce, CspDisposition::Report] {
                    let mut policy = CspPolicySet::new("https://parent.test/document");
                    let original = format!("{fallback} 'none' 'report-sample'");
                    policy.install(&original, disposition, "header");
                    let inline = matches!(
                        destination,
                        CspDestination::ScriptAttribute
                            | CspDestination::StyleAttribute
                            | CspDestination::JavascriptNavigation
                    );
                    let request = CspRequest {
                        destination,
                        url: if inline {
                            "inline".into()
                        } else {
                            "https://blocked.test/resource".into()
                        },
                        nonce: None,
                        inline_source: inline.then(|| "native sample".into()),
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    };
                    let decision = policy.authorize(&request);
                    assert_eq!(
                        decision.allowed,
                        disposition == CspDisposition::Report,
                        "{expected} via {fallback}"
                    );
                    assert_eq!(decision.violations.len(), 1);
                    let violation = &decision.violations[0];
                    assert_eq!(violation.effective_directive, expected);
                    assert_eq!(violation.original_policy, original);
                    assert_eq!(violation.disposition, disposition);
                    assert_eq!(violation.sample, if inline { "native sample" } else { "" });
                }
            }
        }
    }
    #[test]
    fn explicit_sink_supersedes_fallback_and_document_has_no_default_src_navigation_gate() {
        for (destination, name) in [
            (CspDestination::Frame, "frame-src"),
            (CspDestination::Worker, "worker-src"),
            (CspDestination::Connect, "connect-src"),
            (CspDestination::Script, "script-src-elem"),
        ] {
            let policy = CspPolicySet::from_headers(
                "https://parent.test/",
                &[(
                    "Content-Security-Policy".into(),
                    format!("default-src 'none'; {name} https://allowed.test"),
                )],
            );
            let request = CspRequest {
                destination,
                url: "https://allowed.test/a".into(),
                nonce: None,
                inline_source: None,
                parser_inserted: true,
                initiated_by_trusted_script: false,
            };
            assert!(policy.authorize(&request).allowed);
            assert!(policy.authorize(&request).violations.is_empty());
        }
        let policy = CspPolicySet::from_headers(
            "https://parent.test/",
            &[(
                "Content-Security-Policy".into(),
                "default-src 'none'".into(),
            )],
        );
        assert!(
            policy
                .authorize(&CspRequest {
                    destination: CspDestination::Document,
                    url: "https://destination.test/".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false
                })
                .allowed
        );
    }
    #[test]
    fn eval_ignores_element_directive_and_reports_script_src_for_default_fallback() {
        let policy = |value: &str| {
            CspPolicySet::from_headers(
                "https://eval.test/",
                &[("Content-Security-Policy".into(), value.into())],
            )
        };
        for wasm in [false, true] {
            assert!(
                policy("script-src 'unsafe-eval'; script-src-elem 'none'")
                    .allows_eval(wasm)
                    .allowed
            );
            assert!(policy("script-src-elem 'none'").allows_eval(wasm).allowed);
            let decision =
                policy("default-src 'none'; script-src-elem 'unsafe-eval'").allows_eval(wasm);
            assert!(!decision.allowed);
            assert_eq!(decision.violations[0].effective_directive, "script-src");
            assert_eq!(
                decision.violations[0].blocked_uri,
                if wasm { "wasm-eval" } else { "eval" }
            );
        }
        assert!(
            policy("script-src 'wasm-unsafe-eval'; script-src-elem 'none'")
                .allows_eval(true)
                .allowed
        );
        assert!(
            !policy("script-src 'wasm-unsafe-eval'; script-src-elem 'unsafe-eval'")
                .allows_eval(false)
                .allowed
        );
    }
}

#[cfg(test)]
mod nonce_delivery_marker_tests {
    use super::*;
    #[test]
    fn header_nonce_hiding_and_meta_source_survive_native_policy_roundtrip() {
        let mut meta = CspPolicySet::new("https://source.test/");
        meta.install_meta("script-src 'nonce-meta'");
        assert!(!meta.has_header_policy());
        let snapshot = meta.worker_snapshot();
        assert!(!snapshot.hide_nonces);
        assert_eq!(snapshot.policy_sources, vec!["meta"]);
        let roundtrip = CspPolicySet::from_agent_policy(&snapshot);
        assert!(!roundtrip.has_header_policy());
        assert!(roundtrip.contains_meta("script-src 'nonce-meta'"));
        let mut header = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy-Report-Only".into(),
                "script-src 'none'".into(),
            )],
        );
        header.install_meta("style-src 'none'");
        assert!(header.has_header_policy());
        let inherited = header
            .initial_auxiliary_policy()
            .worker_snapshot()
            .with_document_url("about:blank".into());
        assert!(inherited.hide_nonces);
        let restored = CspPolicySet::from_agent_policy(&inherited);
        assert!(restored.has_header_policy());
        assert!(restored.contains_meta("style-src 'none'"));
        assert_eq!(
            restored.worker_snapshot().policy_sources,
            vec!["header", "meta"]
        );
    }
}

#[cfg(test)]
mod speculation_inline_tests {
    use super::*;
    #[test]
    fn rules_keyword_never_authorizes_executable_script_or_ignores_intersection() {
        let request = CspRequest {
            destination: CspDestination::Script,
            url: "inline".into(),
            nonce: None,
            inline_source: Some("{}".into()),
            parser_inserted: true,
            initiated_by_trusted_script: false,
        };
        for directive in [
            "script-src 'inline-speculation-rules'",
            "default-src 'inline-speculation-rules'",
            "script-src-elem 'inline-speculation-rules' 'nonce-other' 'strict-dynamic'",
        ] {
            let mut policy = CspPolicySet::new("https://rules.test/");
            policy.install(directive, CspDisposition::Enforce, "header");
            assert!(
                policy.authorize_speculation_rules(&request).allowed,
                "{directive}"
            );
            assert!(!policy.authorize(&request).allowed, "{directive}");
            let mut external = request.clone();
            external.inline_source = None;
            external.url = "https://rules.test/data.json".into();
            assert!(!policy.authorize_speculation_rules(&external).allowed);
            policy.install("script-src 'none'", CspDisposition::Enforce, "meta");
            let decision = policy.authorize_speculation_rules(&request);
            assert!(!decision.allowed);
            assert_eq!(decision.violations.len(), 1);
            assert_eq!(
                decision.violations[0].effective_directive,
                "script-src-elem"
            );
        }
        let mut policy = CspPolicySet::new("https://rules.test/");
        policy.install("script-src 'nonce-good'", CspDisposition::Enforce, "header");
        let mut nonce = request.clone();
        nonce.nonce = Some("good".into());
        assert!(policy.authorize_speculation_rules(&nonce).allowed);
        policy.install("script-src 'none'", CspDisposition::Report, "header");
        let decision = policy.authorize_speculation_rules(&nonce);
        assert!(decision.allowed);
        assert_eq!(decision.violations.len(), 1);
    }
}

#[cfg(test)]
mod prefetch_policy_tests {
    use super::*;
    fn policy(value: &str) -> CspPolicySet {
        CspPolicySet::from_headers(
            "https://source.test/document",
            &[("Content-Security-Policy".into(), value.into())],
        )
    }
    fn request(url: &str) -> CspRequest {
        CspRequest {
            destination: CspDestination::Prefetch,
            url: url.into(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: false,
        }
    }
    #[test]
    fn every_supported_fetch_source_list_can_allow_resource_hints() {
        for directive in [
            "default-src",
            "child-src",
            "connect-src",
            "font-src",
            "frame-src",
            "img-src",
            "manifest-src",
            "media-src",
            "object-src",
            "script-src",
            "script-src-elem",
            "style-src",
            "style-src-elem",
            "worker-src",
        ] {
            let rules = if directive == "default-src" {
                "default-src https://allowed.test".into()
            } else {
                format!("default-src 'none'; {directive} https://allowed.test")
            };
            let set = policy(&rules);
            assert!(
                set.authorize(&request("https://allowed.test/resource"))
                    .allowed,
                "{directive}"
            );
            let denied = set.authorize(&request("https://denied.test/resource"));
            assert!(!denied.allowed, "{directive}");
            assert_eq!(denied.violations.len(), 1);
            assert_eq!(denied.violations[0].effective_directive, "default-src");
        }
        for directive in [
            "prefetch-src",
            "base-uri",
            "form-action",
            "frame-ancestors",
            "script-src-attr",
            "style-src-attr",
        ] {
            assert!(
                !policy(&format!("default-src 'none'; {directive} *"))
                    .authorize(&request("https://allowed.test/resource"))
                    .allowed,
                "unsupported resource-hint directive {directive}"
            );
        }
        assert!(
            policy("script-src 'none'; img-src 'none'")
                .authorize(&request("https://other.test/"))
                .allowed
        );
    }
    #[test]
    fn union_is_per_policy_and_element_directives_do_not_override_it() {
        let mut set =
            policy("default-src 'none'; img-src https://allowed.test; connect-src 'none'");
        set.install(
            "default-src 'none'; img-src 'none'; connect-src https://allowed.test",
            CspDisposition::Enforce,
            "header",
        );
        assert!(
            set.authorize(&request("https://allowed.test/resource"))
                .allowed
        );
        set.install("default-src 'none'", CspDisposition::Enforce, "header");
        assert!(
            !set.authorize(&request("https://allowed.test/resource"))
                .allowed
        );
        for rules in [
            "default-src 'none'; script-src *; script-src-elem 'none'",
            "default-src 'none'; script-src 'none'; script-src-elem *",
        ] {
            assert!(
                policy(rules)
                    .authorize(&request("https://allowed.test/resource"))
                    .allowed
            );
        }
    }
    #[test]
    fn report_only_redirect_and_non_url_tokens_preserve_resource_hint_semantics() {
        let mut set = policy("default-src 'self'");
        set.install("default-src 'none'", CspDisposition::Report, "header");
        let result = set.authorize(&request("https://source.test/resource"));
        assert!(result.allowed);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].disposition, CspDisposition::Report);
        let mut hint = request("https://other.test/resource");
        hint.nonce = Some("dGVzdA==".into());
        hint.inline_source = Some("ignored".into());
        hint.initiated_by_trusted_script = true;
        assert!(
            !policy(
                "default-src 'none'; script-src 'nonce-dGVzdA==' 'unsafe-inline' 'strict-dynamic'"
            )
            .authorize(&hint)
            .allowed
        );
        let set = policy("default-src 'none'; img-src https://allowed.test/path/");
        assert!(
            !set.authorize(&request("https://allowed.test/other"))
                .allowed
        );
        assert!(
            set.authorize_redirect(&request("https://allowed.test/other"), true)
                .allowed
        );
        let denied = set.authorize_redirect_from(
            &request("https://denied.test/private/path"),
            true,
            "https://source.test/original",
        );
        assert!(!denied.allowed);
        assert_eq!(
            denied.violations[0].blocked_uri,
            "https://source.test/original"
        );
    }
}

#[cfg(test)]
mod worker_reporting_snapshot_tests {
    use super::*;
    #[test]
    fn snapshot_retains_resolved_endpoints_and_hash_report_bytes() {
        let policy = CspPolicySet::from_headers(
            "https://app.test/path/document",
            &[
                ("Reporting-Endpoints".into(), "hash=\"../reports\"".into()),
                (
                    "Report-To".into(),
                    r#"{"group":"legacy group","endpoints":[{"url":"/legacy"}]}"#.into(),
                ),
                (
                    "Content-Security-Policy".into(),
                    "script-src 'self' 'report-sha256'; report-to hash".into(),
                ),
            ],
        );
        let snapshot = policy.worker_snapshot();
        let restored = CspPolicySet::from_agent_policy(&snapshot);
        assert_eq!(restored.reporting_endpoints, policy.reporting_endpoints);
        assert_eq!(
            restored.reporting_endpoints["hash"],
            "https://app.test/reports"
        );
        assert_eq!(
            restored.reporting_endpoints["legacy group"],
            "https://app.test/legacy"
        );
        let before = policy.hash_reports(
            CspDestination::Worker,
            "sharedworker",
            "https://app.test/worker.js",
            b"void 0",
            true,
        );
        let after = restored.hash_reports(
            CspDestination::Worker,
            "sharedworker",
            "https://app.test/worker.js",
            b"void 0",
            true,
        );
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 1);
        assert_eq!(before[0].body, after[0].body);
        assert_eq!(
            CspPolicySet::from_agent_policy(&restored.worker_snapshot()).reporting_endpoints,
            policy.reporting_endpoints
        );
    }
}

#[cfg(test)]
mod inherited_meta_roundtrip_tests {
    use super::*;
    #[test]
    fn blank_and_srcdoc_meta_identity_survives_without_rebasing_data_origin() {
        let mut parent = CspPolicySet::new("https://parent.test/");
        parent.install_meta("style-src 'self'");
        for url in ["about:blank", "about:srcdoc"] {
            let restored = CspPolicySet::from_agent_policy(
                &parent.worker_snapshot().with_document_url(url.into()),
            );
            assert!(restored.contains_meta("style-src 'self'"), "{url}");
            assert_eq!(restored.meta_self_url(), "https://parent.test/");
        }
        let data = CspPolicySet::from_agent_policy(
            &parent
                .worker_snapshot()
                .with_document_url("data:text/html,child".into()),
        );
        assert_eq!(data.meta_self_url(), "data:text/html,child");
        assert!(!data.contains_meta("style-src 'self'"));
    }
}

#[cfg(test)]
mod native_meta_origin_snapshot_tests {
    use super::*;

    #[test]
    fn snapshot_retains_meta_origin_separately_from_inherited_policy_origins() {
        for (url, origin) in [
            ("https://prerender.test/path", "https://prerender.test"),
            ("about:srcdoc", "https://parent.test"),
            ("data:text/html,child", "null"),
        ] {
            let mut policy = CspPolicySet::from_headers(
                "https://policy.test/original",
                &[("content-security-policy".into(), "script-src 'self'".into())],
            );
            policy.rebase_document(url);
            policy.set_document_self_origin(origin);
            policy.install_meta("img-src 'none'");
            let snapshot = policy.worker_snapshot();
            let roundtrip = CspPolicySet::from_agent_policy(&snapshot);
            assert!(roundtrip.contains_meta("img-src 'none'"), "{url}");
            assert_eq!(roundtrip.worker_snapshot(), snapshot);
            assert_eq!(roundtrip.policy_self_url, policy.policy_self_url);
            assert_eq!(roundtrip.policies, policy.policies);
            let request = CspRequest {
                destination: CspDestination::Image,
                url: "https://parent.test/image.png".into(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            };
            assert!(!roundtrip.authorize(&request).allowed);
        }
    }
    #[test]
    fn legacy_sandbox_snapshot_never_admits_http_self_for_new_meta() {
        for url in ["https://sandbox.test/path", "about:blank", "about:srcdoc"] {
            for supplied in [None, Some("https://sandbox.test")] {
                let mut wire = serde_json::json!({"url":url, "policy_self_url":"https://sandbox.test/path",
                    "headers":[["content-security-policy","sandbox allow-scripts"]]});
                if let Some(origin) = supplied {
                    wire["document_self_url"] = origin.into();
                }
                let snapshot: oryn_js::worker_wire::AgentPolicy =
                    serde_json::from_value(wire).unwrap();
                let mut policy = CspPolicySet::from_agent_policy(&snapshot);
                assert_eq!(policy.meta_self_url(), "data:,");
                policy.install_meta("img-src 'self'");
                let request = CspRequest {
                    destination: CspDestination::Image,
                    url: "https://sandbox.test/image.png".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                };
                assert!(!policy.authorize(&request).allowed, "{url}");
                let roundtrip = CspPolicySet::from_agent_policy(&policy.worker_snapshot());
                assert!(!roundtrip.authorize(&request).allowed);
            }
        }
    }
}

#[cfg(test)]
mod javascript_navigation_tests {
    use super::*;
    #[test]
    fn javascript_navigation_hashes_require_unsafe_hashes_and_cannot_use_nonces() {
        let source = "javascript:opener.postMessage('pass', '*')";
        let hash = BASE64.encode(Sha256::digest(source.as_bytes()));
        let request = CspRequest {
            destination: CspDestination::JavascriptNavigation,
            url: source.into(),
            inline_source: Some(source.into()),
            nonce: Some("abc".into()),
            parser_inserted: false,
            initiated_by_trusted_script: true,
        };
        for (policy, allowed) in [
            ("script-src 'nonce-abc'".to_owned(), false),
            (format!("script-src 'sha256-{hash}'"), false),
            (format!("script-src 'unsafe-hashes' 'sha256-{hash}'"), true),
            ("script-src 'unsafe-inline'".to_owned(), true),
            (
                "script-src 'unsafe-inline' 'strict-dynamic'".to_owned(),
                false,
            ),
            (
                format!("script-src 'unsafe-hashes' 'sha256-{hash}'; script-src-elem 'none'"),
                false,
            ),
        ] {
            let csp = CspPolicySet::from_headers(
                "https://source.test/",
                &[("content-security-policy".into(), policy.clone())],
            );
            let decision = csp.authorize(&request);
            assert_eq!(decision.allowed, allowed, "{policy}");
            if !allowed {
                assert_eq!(
                    decision.violations[0].effective_directive,
                    "script-src-elem"
                );
                assert_eq!(decision.violations[0].blocked_uri, "inline");
            }
        }
        let csp = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "content-security-policy-report-only".into(),
                "script-src 'none'".into(),
            )],
        );
        let result = csp.authorize(&request);
        assert!(result.allowed);
        assert_eq!(result.violations.len(), 1);
    }
}
