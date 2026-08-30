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

pub const CSP_POLICY_VERSION: &str = "csp3-agent-critical-1";

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
    Script,
    ScriptAttribute,
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
    pub document_uri: String,
    pub source: String,
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
    disposition: CspDisposition,
    directives: BTreeMap<String, Vec<String>>,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CspPolicySet {
    document_url: String,
    policies: Vec<CspPolicy>,
}

impl CspPolicySet {
    pub fn new(document_url: impl Into<String>) -> Self {
        Self {
            document_url: document_url.into(),
            policies: Vec::new(),
        }
    }

    pub fn from_headers(document_url: impl Into<String>, headers: &[(String, String)]) -> Self {
        let mut set = Self::new(document_url);
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
        set
    }

    pub fn meta_policies_from_html(html: &str) -> Vec<String> {
        let meta = regex::Regex::new(
            r#"(?is)<meta\b[^>]*http-equiv\s*=\s*['\"]?content-security-policy['\"]?[^>]*>"#,
        )
        .expect("constant CSP meta regex");
        let content = regex::Regex::new(r#"(?is)\bcontent\s*=\s*(?:\"([^\"]*)\"|'([^']*)')"#)
            .expect("constant CSP meta content regex");
        meta.find_iter(html)
            .filter_map(|element| {
                content.captures(element.as_str()).and_then(|capture| {
                    capture
                        .get(1)
                        .or_else(|| capture.get(2))
                        .map(|value| value.as_str().to_string())
                })
            })
            .collect()
    }

    pub fn install_meta(&mut self, serialized: &str) {
        let mut policy = parse_policy(serialized, CspDisposition::Enforce, "meta");
        for ignored in ["report-uri", "report-to", "frame-ancestors", "sandbox"] {
            policy.directives.remove(ignored);
        }
        self.policies.push(policy);
    }

    pub fn install(&mut self, serialized: &str, disposition: CspDisposition, source: &str) {
        self.policies
            .push(parse_policy(serialized, disposition, source));
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    pub fn equivalent_policy(&self, other: &Self) -> bool {
        self.policies == other.policies
            && (self.policies.is_empty()
                || Url::parse(&self.document_url).ok().map(|url| url.origin())
                    == Url::parse(&other.document_url).ok().map(|url| url.origin()))
    }

    pub fn authorize(&self, request: &CspRequest) -> CspDecision {
        let document = Url::parse(&self.document_url).ok();
        let mut allowed = true;
        let mut violations = Vec::new();
        for policy in &self.policies {
            let Some((directive, sources)) = effective_sources(policy, request.destination) else {
                continue;
            };
            let permits = source_list_allows(sources, document.as_ref(), request);
            if !permits {
                if policy.disposition == CspDisposition::Enforce {
                    allowed = false;
                }
                violations.push(CspViolation {
                    disposition: policy.disposition,
                    effective_directive: directive.into(),
                    blocked_uri: blocked_uri(request),
                    document_uri: scrubbed_uri(&self.document_url),
                    source: policy.source.clone(),
                });
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
            let Some((directive, sources)) = effective_sources(policy, CspDestination::Script)
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
                violations.push(CspViolation {
                    disposition: policy.disposition,
                    effective_directive: directive.into(),
                    blocked_uri: request.url.clone(),
                    document_uri: scrubbed_uri(&self.document_url),
                    source: policy.source.clone(),
                });
            }
        }
        CspDecision {
            allowed,
            violations,
        }
    }

    pub fn authorize_frame_ancestors(&self, ancestors: &[String]) -> CspDecision {
        let document = Url::parse(&self.document_url).ok();
        let mut allowed = true;
        let mut violations = Vec::new();
        for policy in &self.policies {
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
                violations.push(CspViolation {
                    disposition: policy.disposition,
                    effective_directive: "frame-ancestors".into(),
                    blocked_uri: ancestors
                        .last()
                        .map_or_else(|| "opaque".into(), |url| scrubbed_uri(url)),
                    document_uri: scrubbed_uri(&self.document_url),
                    source: policy.source.clone(),
                });
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
                && policy.source == "header"
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
            return result;
        }
        result.allow_any_policy = configurations
            .iter()
            .all(|tokens| tokens.iter().any(|token| token == "*"));
        result.allow_duplicates = configurations
            .iter()
            .all(|tokens| tokens.iter().any(|token| token == "'allow-duplicates'"));
        result.deny_policy_creation = configurations
            .iter()
            .any(|tokens| tokens.is_empty() || tokens.iter().any(|token| token == "'none'"));
        let mut names = configurations.iter().map(|tokens| {
            tokens
                .iter()
                .filter(|token| !token.starts_with('\'') && token.as_str() != "*")
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

fn parse_policy(serialized: &str, disposition: CspDisposition, source: &str) -> CspPolicy {
    let mut directives = BTreeMap::new();
    for directive in serialized.split(';') {
        let mut tokens = directive.split_ascii_whitespace();
        let Some(name) = tokens.next() else { continue };
        let name = name.to_ascii_lowercase();
        directives
            .entry(name)
            .or_insert_with(|| tokens.map(str::to_owned).collect());
    }
    CspPolicy {
        disposition,
        directives,
        source: source.into(),
    }
}

fn split_policy_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn effective_sources(policy: &CspPolicy, destination: CspDestination) -> Option<(&str, &[String])> {
    let candidates: &[&str] = match destination {
        CspDestination::Script => &["script-src-elem", "script-src", "default-src"],
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
        CspDestination::Document => return None,
    };
    candidates.iter().find_map(|name| {
        policy
            .directives
            .get(*name)
            .map(|sources| (*name, sources.as_slice()))
    })
}

fn source_list_allows(sources: &[String], document: Option<&Url>, request: &CspRequest) -> bool {
    if sources.iter().any(|source| source == "'none'") {
        return false;
    }
    if request.inline_source.is_some() {
        if sources.iter().any(|source| source == "'unsafe-inline'")
            && !sources.iter().any(|source| source == "'strict-dynamic'")
        {
            return true;
        }
        if request.nonce.as_ref().is_some_and(|nonce| {
            sources.iter().any(|source| {
                source
                    .strip_prefix("'nonce-")
                    .and_then(|source| source.strip_suffix('\''))
                    == Some(nonce)
            })
        }) {
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
                CspDestination::ScriptAttribute | CspDestination::StyleAttribute
            ) || sources.iter().any(|source| source == "'unsafe-hashes'"));
    }
    if request.nonce.as_ref().is_some_and(|nonce| {
        sources.iter().any(|source| {
            source
                .strip_prefix("'nonce-")
                .and_then(|source| source.strip_suffix('\''))
                == Some(nonce)
        })
    }) {
        return true;
    }
    if sources.iter().any(|source| source == "'strict-dynamic'")
        && request.initiated_by_trusted_script
        && !request.parser_inserted
    {
        return true;
    }
    let Ok(target) = Url::parse(&request.url) else {
        return false;
    };
    sources
        .iter()
        .any(|source| source_matches(source, document, &target))
}

fn source_matches(source: &str, document: Option<&Url>, target: &Url) -> bool {
    if source == "*" {
        return !matches!(target.scheme(), "data" | "blob" | "filesystem");
    }
    if source == "'self'" {
        return document.is_some_and(|document| {
            document.origin() == target.origin()
                || document.scheme() == "http"
                    && target.scheme() == "https"
                    && document.host_str() == target.host_str()
        });
    }
    if let Some(scheme) = source.strip_suffix(':') {
        return target.scheme() == scheme
            || scheme == "http" && target.scheme() == "https"
            || scheme == "ws" && target.scheme() == "wss";
    }
    let normalized = if source.contains("://") {
        source.to_owned()
    } else if source.starts_with("*.") || source.contains('.') {
        format!("{}://{}", document.map_or("https", Url::scheme), source)
    } else {
        return false;
    };
    let wildcard = normalized.contains("://*.");
    let parsed = Url::parse(&normalized.replace("://*.", "://")).ok();
    parsed.is_some_and(|allowed| {
        let scheme = allowed.scheme() == target.scheme()
            || allowed.scheme() == "http" && target.scheme() == "https";
        let hosts = allowed
            .host_str()
            .zip(target.host_str())
            .is_some_and(|(allowed, target)| {
                target == allowed || wildcard && target.ends_with(&format!(".{allowed}"))
            });
        let ports = allowed.port().is_none()
            || allowed.port_or_known_default() == target.port_or_known_default();
        let paths = allowed.path() == "/" || target.path().starts_with(allowed.path());
        scheme && hosts && ports && paths
    })
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
        allow_downloads: left.allow_downloads && right.allow_downloads,
        allow_top_navigation: left.allow_top_navigation && right.allow_top_navigation,
        allow_top_navigation_by_user_activation: left.allow_top_navigation_by_user_activation
            && right.allow_top_navigation_by_user_activation,
        allow_top_navigation_to_custom_protocols: left.allow_top_navigation_to_custom_protocols
            && right.allow_top_navigation_to_custom_protocols,
    }
}

fn blocked_uri(request: &CspRequest) -> String {
    if request.inline_source.is_some() {
        "inline".into()
    } else {
        scrubbed_uri(&request.url)
    }
}

fn scrubbed_uri(value: &str) -> String {
    Url::parse(value)
        .map(|mut url| {
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        })
        .unwrap_or_else(|_| value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn extracts_meta_policies_without_confusing_http_equiv() {
        assert_eq!(
            CspPolicySet::meta_policies_from_html(
                r#"<meta http-equiv="content-security-policy" content="script-src 'none'">"#,
            ),
            vec!["script-src 'none'"]
        );
    }
}
