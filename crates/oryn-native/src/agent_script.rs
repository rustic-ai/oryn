//! Parent-authorized worker entry/import loading, independent of PageId.
use crate::{
    cookies::{CookieCredentials, CookieRequestContext, CookieRequestKind},
    csp::{CspDestination, CspPolicySet, CspRequest},
    network::{
        NetworkRequest, NetworkResponse, SharedNetworkBroker, collect_operation_bounded_with_cancel,
    },
    referrer::ReferrerPolicy,
};
use oryn_js::worker_wire::*;
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use url::Url;
static REQUEST_ID: AtomicU64 = AtomicU64::new(1u64 << 63);
#[derive(Clone, Copy)]
pub enum ScriptLoadKind {
    EntryClassic,
    EntryModule,
    ClassicImport,
    ModuleImport,
    JsonModuleImport,
    TextModuleImport,
}
impl ScriptLoadKind {
    pub(crate) fn import_kind(self) -> AgentImportKind {
        match self {
            Self::EntryClassic | Self::ClassicImport => AgentImportKind::Classic,
            Self::EntryModule | Self::ModuleImport => AgentImportKind::JavaScript,
            Self::TextModuleImport => AgentImportKind::Text,
            Self::JsonModuleImport => AgentImportKind::Json,
        }
    }
    pub(crate) fn from_import_kind(kind: AgentImportKind) -> Self {
        match kind {
            AgentImportKind::Classic => Self::ClassicImport,
            AgentImportKind::JavaScript => Self::ModuleImport,
            AgentImportKind::Text => Self::TextModuleImport,
            AgentImportKind::Json => Self::JsonModuleImport,
        }
    }
    fn entry(self) -> bool {
        matches!(self, Self::EntryClassic | Self::EntryModule)
    }
    fn module(self) -> bool {
        matches!(
            self,
            Self::EntryModule
                | Self::ModuleImport
                | Self::JsonModuleImport
                | Self::TextModuleImport
        )
    }
}
pub struct ScriptAuthority<'a> {
    pub hash_report: Option<&'a dyn Fn(Vec<crate::csp::ScriptHashReport>)>,
    pub worker_destination: &'static str,
    pub coep_report: Option<&'a dyn Fn(Vec<crate::coep_reporting::Report>)>,
    pub report:
        Option<&'a dyn Fn(&CspPolicySet, Vec<crate::csp::CspViolation>) -> Result<(), WorkerError>>,
    pub cookie_ancestor_urls: &'a [String],
    pub referrer_source: Option<&'a str>,
    pub origin: &'a AgentOrigin,
    pub policy: &'a AgentPolicy,
    pub script_url: &'a str,
    pub referrer_policy: &'a str,
    pub top_level_url: Option<&'a str>,
    pub credentials: WorkerCredentials,
}
pub struct LoadedAgentScript {
    pub content_type: Option<String>,
    pub digest: [u8; 32],
    pub service_worker_allowed: Option<String>,
    pub url: String,
    pub source: Vec<u16>,
    pub policy: AgentPolicy,
    pub referrer_policy: String,
}
impl LoadedAgentScript {
    pub(crate) fn import_metadata(
        &self,
        kind: ScriptLoadKind,
        source: Option<&str>,
        policy: &str,
    ) -> AgentImportMetadata {
        AgentImportMetadata {
            kind: kind.import_kind(),
            response_url: self.url.clone(),
            content_type: self.content_type.clone(),
            referrer_policy: self.referrer_policy.clone(),
            request_referrer_source: source.map(str::to_owned),
            request_referrer_policy: policy.to_owned(),
        }
    }
}
pub(crate) fn validate_import_metadata(
    metadata: &AgentImportMetadata,
    kind: ScriptLoadKind,
) -> Result<(), WorkerError> {
    let headers = metadata
        .content_type
        .iter()
        .map(|value| ("content-type".into(), value.clone()))
        .collect::<Vec<_>>();
    let allowed = match kind.import_kind() {
        AgentImportKind::JavaScript => crate::ModuleType::JavaScript.mime_allowed(&headers),
        AgentImportKind::Json => crate::ModuleType::Json.mime_allowed(&headers),
        AgentImportKind::Text => true,
        AgentImportKind::Classic => {
            metadata.kind == AgentImportKind::Classic
                && crate::ModuleType::JavaScript.mime_allowed(&headers)
        }
    };
    if allowed && metadata.kind == kind.import_kind() {
        Ok(())
    } else {
        Err(error(
            ScriptFailureKind::Fetch,
            "NetworkError",
            "stored service import MIME or script kind mismatch",
        ))
    }
}
pub(crate) fn representation_digest(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
fn error(kind: ScriptFailureKind, name: &str, message: impl Into<String>) -> WorkerError {
    WorkerError::new(name, message).with_script_failure(kind)
}
pub(crate) fn decode_imported_source(
    response: &NetworkResponse,
    kind: ScriptLoadKind,
) -> Result<String, WorkerError> {
    if matches!(
        kind,
        ScriptLoadKind::JsonModuleImport | ScriptLoadKind::TextModuleImport
    ) {
        if !(200..=299).contains(&response.status) {
            return Err(error(
                ScriptFailureKind::Fetch,
                "NetworkError",
                format!("JSON module returned HTTP {}", response.status),
            ));
        }
        if response.body.len() > 8 * 1024 * 1024 {
            return Err(error(
                ScriptFailureKind::Capacity,
                "QuotaExceededError",
                "JSON module source exceeds limit",
            ));
        }
        if matches!(kind, ScriptLoadKind::JsonModuleImport)
            && !crate::ModuleType::Json.mime_allowed(&response.headers)
        {
            return Err(error(
                ScriptFailureKind::Fetch,
                "NetworkError",
                "JSON module response MIME mismatch",
            ));
        }
        let (source, _) = oryn_net::decode_resource_text(
            &response.body,
            None,
            oryn_net::TextResourceKind::ModuleJavaScript,
        );
        if source.len() > 8 * 1024 * 1024 {
            return Err(error(
                ScriptFailureKind::Capacity,
                "QuotaExceededError",
                "decoded JSON module source exceeds limit",
            ));
        }
        Ok(source)
    } else {
        crate::context_service::decode_worker_script(response, kind.module()).map_err(|failure| {
            crate::script_load_failure::ScriptLoadFailure::decoded(failure).into_worker()
        })
    }
}
fn loaded(
    response: NetworkResponse,
    url: &Url,
    kind: ScriptLoadKind,
    inherited: Option<&AgentPolicy>,
    referrer_policy: &str,
) -> Result<LoadedAgentScript, WorkerError> {
    let source = decode_imported_source(&response, kind)?;
    let policy = inherited
        .cloned()
        .map(|policy| policy.with_document_url(url.to_string()))
        .unwrap_or_else(|| {
            let mut policy =
                CspPolicySet::from_headers(url.as_str(), &response.headers).worker_snapshot();
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&response.headers)
                .append_native_headers(&mut policy.headers);
            policy
                .headers
                .retain(|(name, _)| !name.eq_ignore_ascii_case("permissions-policy"));
            policy.headers.extend(
                response
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
                    .cloned(),
            );
            policy
        });
    Ok(LoadedAgentScript {
        content_type: oryn_common::mime_type::MimeType::from_headers(&response.headers)
            .map(|mime| mime.serialize()),
        digest: representation_digest(&response.body),
        service_worker_allowed: response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("service-worker-allowed"))
            .map(|(_, value)| value.clone()),
        url: url.to_string(),
        source: source.encode_utf16().collect(),
        policy,
        referrer_policy: ReferrerPolicy::parse_list(referrer_policy)
            .after_response(&response.headers)
            .as_str()
            .into(),
    })
}
/// The caller validates its registry principal before loading and again before
/// admission. This function never accepts an origin/partition from author JSON.
pub fn load(
    network: &SharedNetworkBroker,
    authority: ScriptAuthority<'_>,
    requested: &str,
    kind: ScriptLoadKind,
    cancel: &AtomicBool,
) -> Result<LoadedAgentScript, WorkerError> {
    load_controlled(network, authority, requested, kind, cancel, None)
}
pub(crate) fn load_controlled(
    network: &SharedNetworkBroker,
    authority: ScriptAuthority<'_>,
    requested: &str,
    kind: ScriptLoadKind,
    cancel: &AtomicBool,
    controller: Option<(
        &crate::service_registry::ServiceRegistry,
        &crate::agent_registry::AgentOwner,
    )>,
) -> Result<LoadedAgentScript, WorkerError> {
    load_with_cache(
        network, authority, requested, kind, cancel, controller, false, false,
    )
}
/// Service update cache policy is supplied by the native registration job.
/// Revalidation bypasses local reuse but allows storing the new response.
pub(crate) fn load_service(
    network: &SharedNetworkBroker,
    authority: ScriptAuthority<'_>,
    requested: &str,
    kind: ScriptLoadKind,
    cancel: &AtomicBool,
    revalidate: bool,
) -> Result<LoadedAgentScript, WorkerError> {
    load_with_cache(
        network, authority, requested, kind, cancel, None, revalidate, true,
    )
}
fn load_with_cache(
    network: &SharedNetworkBroker,
    authority: ScriptAuthority<'_>,
    requested: &str,
    kind: ScriptLoadKind,
    cancel: &AtomicBool,
    controller: Option<(
        &crate::service_registry::ServiceRegistry,
        &crate::agent_registry::AgentOwner,
    )>,
    revalidate: bool,
    service: bool,
) -> Result<LoadedAgentScript, WorkerError> {
    let base = Url::parse(authority.script_url)
        .map_err(|failure| error(ScriptFailureKind::Fetch, "SyntaxError", failure.to_string()))?;
    let mut url = base
        .join(requested)
        .map_err(|failure| error(ScriptFailureKind::Fetch, "SyntaxError", failure.to_string()))?;
    let hash_request_url = url.to_string();
    let policy = CspPolicySet::from_agent_policy(&authority.policy);
    let embedder =
        crate::agent_isolation::DocumentIsolationPolicy::from_headers(&authority.policy.headers);
    let mut referrer_policy = ReferrerPolicy::parse_list(authority.referrer_policy);
    let credentials = if kind.module() {
        authority.credentials
    } else {
        WorkerCredentials::SameOrigin
    };
    let mut crossed_site = false;
    let mut referrer_source = Some(
        Url::parse(authority.referrer_source.unwrap_or(authority.script_url)).map_err(
            |failure| error(ScriptFailureKind::Fetch, "SyntaxError", failure.to_string()),
        )?,
    );
    let mut request_origin = authority.origin.serialized.clone();
    let mut opaque_redirect_origin = false;
    let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
        if kind.module() {
            "cors"
        } else if kind.entry() {
            "same-origin"
        } else {
            "no-cors"
        },
        if kind.entry() || authority.worker_destination == "audioworklet" {
            authority.worker_destination
        } else {
            "script"
        },
        authority.origin.opaque,
    );
    for redirect in 0..=10 {
        if cancel.load(Ordering::Acquire) {
            return Err(error(
                ScriptFailureKind::Cancelled,
                "AbortError",
                "worker script loading cancelled",
            ));
        }
        if let Some(upgraded) = policy.upgraded_url(&url) {
            url = upgraded;
        }
        crate::context_service::enforce_mixed_content(&authority.origin.serialized, &url).map_err(
            |failure| {
                error(
                    ScriptFailureKind::Fetch,
                    "SecurityError",
                    failure.to_string(),
                )
            },
        )?;
        let decision = policy.authorize_redirect_from(
            &CspRequest {
                destination: if kind.entry() && authority.worker_destination != "audioworklet" {
                    CspDestination::Worker
                } else {
                    CspDestination::Script
                },
                url: url.to_string(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: true,
            },
            redirect != 0,
            &hash_request_url,
        );
        if let Some(report) = authority.report {
            report(&policy, decision.violations)?;
        }
        if !decision.allowed {
            return Err(error(
                ScriptFailureKind::Fetch,
                "SecurityError",
                "worker script blocked by creator policy",
            ));
        }
        if url.scheme() == "data" {
            let (content_type, body) =
                crate::network::decode_data_url(&url).map_err(|failure| {
                    error(
                        ScriptFailureKind::Fetch,
                        "NetworkError",
                        failure.to_string(),
                    )
                })?;
            let response = NetworkResponse {
                timing: None,
                request_id: 0,
                status: 200,
                headers: content_type
                    .into_iter()
                    .map(|value| ("content-type".into(), value))
                    .collect(),
                body,
            };
            if cancel.load(Ordering::Acquire) {
                return Err(error(
                    ScriptFailureKind::Cancelled,
                    "AbortError",
                    "worker script loading cancelled before publication",
                ));
            }
            let hashes = authority
                .hash_report
                .map(|_| {
                    policy.hash_reports(
                        if authority.worker_destination == "audioworklet" {
                            CspDestination::Script
                        } else {
                            CspDestination::Worker
                        },
                        authority.worker_destination,
                        &hash_request_url,
                        &response.body,
                        true,
                    )
                })
                .unwrap_or_default();
            let result = loaded(
                response,
                &url,
                kind,
                Some(authority.policy),
                authority.referrer_policy,
            );
            if cancel.load(Ordering::Acquire) {
                return Err(error(
                    ScriptFailureKind::Cancelled,
                    "AbortError",
                    "worker script loading cancelled during decoding",
                ));
            }
            if result.is_ok() {
                if let Some(report) = authority.hash_report {
                    report(hashes);
                }
            }
            return result;
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(error(
                ScriptFailureKind::Fetch,
                "SecurityError",
                "worker script URL contains credentials",
            ));
        }
        if !matches!(url.scheme(), "https" | "http") {
            return Err(error(
                ScriptFailureKind::Fetch,
                "NetworkError",
                "worker script URL has no registered native resource loader",
            ));
        }
        let same_origin = !opaque_redirect_origin
            && !authority.origin.opaque
            && url.origin().ascii_serialization() == authority.origin.serialized;
        if kind.entry() && authority.worker_destination != "audioworklet" && !same_origin {
            return Err(error(
                ScriptFailureKind::Fetch,
                "SecurityError",
                "worker entry script must be same-origin",
            ));
        }
        let use_credentials = (matches!(credentials, WorkerCredentials::Include)
            || (matches!(credentials, WorkerCredentials::SameOrigin) && same_origin))
            && !embedder.omit_credentials(kind.module(), !same_origin);
        let mut headers = vec![(
            "accept".into(),
            "text/javascript,application/javascript,*/*;q=0.1".into(),
        )];
        if revalidate {
            headers.push(("cache-control".into(), "no-cache".into()));
        }
        if service && kind.entry() {
            headers.push(("service-worker".into(), "script".into()));
        }
        let referrer = referrer_source
            .as_ref()
            .and_then(|source| referrer_policy.header(source, &url));
        referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
        if let Some(value) = referrer {
            headers.push(("referer".into(), value));
        }
        if kind.module() && !same_origin {
            headers.push(("origin".into(), request_origin.clone()));
        }
        let request_id = REQUEST_ID
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| {
                error(
                    ScriptFailureKind::Capacity,
                    "QuotaExceededError",
                    "worker request IDs exhausted",
                )
            })?;
        let service_response = if let Some((services, owner)) = controller {
            services.fetch_with_cancel(
                owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: url.to_string(),
                    method: "GET".into(),
                    headers: headers.clone(),
                    body: Vec::new(),
                    destination: if kind.entry() || authority.worker_destination == "audioworklet" {
                        authority.worker_destination.into()
                    } else {
                        "script".into()
                    },
                    mode: if kind.module() {
                        "cors".into()
                    } else if kind.entry() {
                        "same-origin".into()
                    } else {
                        "no-cors".into()
                    },
                    credentials: match credentials {
                        WorkerCredentials::Omit => "omit",
                        WorkerCredentials::SameOrigin => "same-origin",
                        WorkerCredentials::Include => "include",
                    }
                    .into(),
                    redirect: "follow".into(),
                },
                Some(cancel),
            )?
        } else {
            None
        };
        let service_handled = service_response.is_some();
        fetch_metadata.observe(Some(&authority.origin.serialized), &url);
        let response = if let Some(response) = service_response {
            if response.status == 0
                || matches!(
                    response.response_type.as_str(),
                    "error" | "opaque" | "opaqueredirect"
                )
            {
                return Err(error(
                    ScriptFailureKind::Fetch,
                    "NetworkError",
                    "service returned unusable worker script response",
                ));
            }
            if response.body.len() > 8 * 1024 * 1024 {
                return Err(error(
                    ScriptFailureKind::Capacity,
                    "QuotaExceededError",
                    "service worker script response too large",
                ));
            }
            NetworkResponse {
                timing: None,
                request_id,
                status: response.status,
                headers: response.headers,
                body: response.body,
            }
        } else {
            collect_operation_bounded_with_cancel(
                network,
                NetworkRequest {
                    request_id,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: use_credentials,
                    resolved_addrs: Vec::new(),
                    security: CookieRequestContext {
                        fetch_metadata: fetch_metadata.clone(),
                        dns_timing: None,
                        credentials: match credentials {
                            WorkerCredentials::Omit => CookieCredentials::Omit,
                            WorkerCredentials::SameOrigin => CookieCredentials::SameOrigin,
                            WorkerCredentials::Include => CookieCredentials::Include,
                        },
                        kind: if kind.entry() {
                            CookieRequestKind::Worker
                        } else {
                            CookieRequestKind::Script
                        },
                        initiator_url: Some(base.to_string()),
                        top_level_url: authority.top_level_url.map(str::to_owned),
                        ancestor_urls: authority.cookie_ancestor_urls.to_vec(),
                        redirect_chain_cross_site: crossed_site,
                        ..Default::default()
                    },
                },
                Duration::from_secs(30),
                8 * 1024 * 1024,
                cancel,
            )
            .map_err(|failure| {
                crate::script_load_failure::ScriptLoadFailure::network(failure).into_worker()
            })?
        };
        if !service_handled && kind.module() && !same_origin {
            let origins = response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"))
                .map(|(_, value)| value.trim())
                .collect::<Vec<_>>();
            let allow_origin = origins.len() == 1
                && (origins[0] == request_origin || (!use_credentials && origins[0] == "*"));
            let allow_credentials = !use_credentials
                || response
                    .headers
                    .iter()
                    .filter(|(name, _)| {
                        name.eq_ignore_ascii_case("access-control-allow-credentials")
                    })
                    .map(|(_, value)| value.as_str())
                    .collect::<Vec<_>>()
                    == vec!["true"];
            if !allow_origin || !allow_credentials {
                return Err(error(
                    ScriptFailureKind::Fetch,
                    "NetworkError",
                    "worker module response failed CORS",
                ));
            }
        }
        if let Some(report) = authority.coep_report {
            report(
                crate::coep_reporting::Policy::new(
                    &authority.policy.url,
                    &authority.policy.headers,
                )
                .response(
                    &authority.origin.serialized,
                    authority.origin.opaque,
                    &url,
                    kind.module(),
                    use_credentials,
                    &response.headers,
                    if kind.entry() || authority.worker_destination == "audioworklet" {
                        authority.worker_destination
                    } else {
                        "script"
                    },
                ),
            );
        }
        embedder
            .authorize_response(
                &authority.origin.serialized,
                authority.origin.opaque,
                &url,
                kind.module(),
                use_credentials,
                &response.headers,
            )
            .map_err(|message| error(ScriptFailureKind::Fetch, "NetworkError", message))?;
        referrer_policy = referrer_policy.after_response(&response.headers);
        if oryn_net::fetch::is_redirect_status(response.status) {
            if service && kind.entry() {
                return Err(error(
                    ScriptFailureKind::Fetch,
                    "NetworkError",
                    "service worker entry script redirects are forbidden",
                ));
            }
            if redirect == 10 {
                return Err(error(
                    ScriptFailureKind::Fetch,
                    "NetworkError",
                    "worker script redirect limit",
                ));
            }
            let location = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                .map(|(_, value)| value)
                .ok_or_else(|| {
                    error(
                        ScriptFailureKind::Fetch,
                        "NetworkError",
                        "worker redirect omitted Location",
                    )
                })?;
            let next = url.join(location).map_err(|failure| {
                error(
                    ScriptFailureKind::Fetch,
                    "NetworkError",
                    failure.to_string(),
                )
            })?;
            if kind.module()
                && next.origin() != url.origin()
                && url.origin().ascii_serialization() != authority.origin.serialized
            {
                opaque_redirect_origin = true;
                request_origin = "null".into();
            }
            crossed_site |= next.origin() != url.origin();
            url = next;
            continue;
        }
        if cancel.load(Ordering::Acquire) {
            return Err(error(
                ScriptFailureKind::Cancelled,
                "AbortError",
                "worker script loading cancelled before publication",
            ));
        }
        let hashes = authority
            .hash_report
            .map(|_| {
                policy.hash_reports(
                    if authority.worker_destination == "audioworklet" {
                        CspDestination::Script
                    } else {
                        CspDestination::Worker
                    },
                    authority.worker_destination,
                    &hash_request_url,
                    &response.body,
                    kind.module() || same_origin,
                )
            })
            .unwrap_or_default();
        let result = loaded(response, &url, kind, None, authority.referrer_policy);
        if cancel.load(Ordering::Acquire) {
            return Err(error(
                ScriptFailureKind::Cancelled,
                "AbortError",
                "worker script loading cancelled during decoding",
            ));
        }
        if result.is_ok() {
            if let Some(report) = authority.hash_report {
                report(hashes);
            }
        }
        return result;
    }
    Err(error(
        ScriptFailureKind::Fetch,
        "NetworkError",
        "worker script loading did not complete",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError};
    use std::sync::{Arc, Mutex};
    #[test]
    fn window_owned_worker_entry_can_report_hash_but_worker_import_has_no_producer() {
        struct Reply;
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: b"void 0".to_vec(),
                })
            }
        }
        let policy = CspPolicySet::from_headers(
            "https://a.test/document",
            &[
                ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                (
                    "Content-Security-Policy".into(),
                    "script-src 'self' 'report-sha256'; report-to hash".into(),
                ),
            ],
        )
        .worker_snapshot();
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "https://a.test".into(),
            opaque: false,
        };
        let reports = std::cell::RefCell::new(Vec::new());
        let report = |items: Vec<crate::csp::ScriptHashReport>| reports.borrow_mut().extend(items);
        let broker = SharedNetworkBroker::new(Reply);
        let mut entry = authority(&origin, &policy);
        entry.hash_report = Some(&report);
        entry.worker_destination = "sharedworker";
        load(
            &broker,
            entry,
            "entry.js",
            ScriptLoadKind::EntryClassic,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reports.borrow().len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&reports.borrow()[0].body).unwrap();
        assert_eq!(body[0]["body"]["destination"], "sharedworker");
        load(
            &broker,
            authority(&origin, &policy),
            "import.js",
            ScriptLoadKind::ClassicImport,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reports.borrow().len(), 1);
    }

    #[test]
    fn text_modules_decode_utf8_ignore_labels_and_preserve_non_utf8_bom_bytes() {
        for (body, expected) in [
            (b"\xef\xbb\xbftext\n".to_vec(), "text\n"),
            (vec![0xff, 0xfe, b't', 0], "\u{fffd}\u{fffd}t\0"),
        ] {
            let response = NetworkResponse {
                timing: None,
                request_id: 0,
                status: 200,
                headers: vec![(
                    "content-type".into(),
                    "application/octet-stream;charset=utf-16".into(),
                )],
                body,
            };
            assert_eq!(
                decode_imported_source(&response, ScriptLoadKind::TextModuleImport).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn json_module_http_failure_is_fetch_provenance_and_limits_are_capacity() {
        let response = NetworkResponse {
            timing: None,
            request_id: 1,
            status: 404,
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{}".to_vec(),
        };
        assert_eq!(
            decode_imported_source(&response, ScriptLoadKind::JsonModuleImport)
                .unwrap_err()
                .script_failure,
            Some(ScriptFailureKind::Fetch)
        );
        let response = NetworkResponse {
            status: 200,
            body: vec![b' '; 8 * 1024 * 1024 + 1],
            ..response
        };
        assert_eq!(
            decode_imported_source(&response, ScriptLoadKind::JsonModuleImport)
                .unwrap_err()
                .script_failure,
            Some(ScriptFailureKind::Capacity)
        );
    }
    struct Fixture {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        allow_origin: Option<String>,
    }
    impl NetworkBroker for Fixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let mut headers = vec![
                (
                    "content-type".into(),
                    "text/javascript;charset=windows-1252".into(),
                ),
                ("content-security-policy".into(), "script-src 'self'".into()),
            ];
            if let Some(origin) = &self.allow_origin {
                headers.push(("access-control-allow-origin".into(), origin.clone()));
            }
            self.requests.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers,
                body: "self.value='é';".as_bytes().to_vec(),
            })
        }
    }
    fn authority<'a>(origin: &'a AgentOrigin, policy: &'a AgentPolicy) -> ScriptAuthority<'a> {
        ScriptAuthority {
            hash_report: None,
            worker_destination: "worker",
            coep_report: None,
            report: None,
            cookie_ancestor_urls: &[],
            referrer_source: None,
            origin,
            policy,
            script_url: "https://a.test/main.js",
            referrer_policy: "no-referrer",
            top_level_url: Some("https://top.test/"),
            credentials: WorkerCredentials::SameOrigin,
        }
    }
    #[test]
    fn data_worker_entries_require_javascript_mime_and_decode_utf8_without_network() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: None,
        });
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy::default();
        for destination in ["worker", "sharedworker"] {
            for kind in [ScriptLoadKind::EntryClassic, ScriptLoadKind::EntryModule] {
                // An omitted data MIME is text/plain, not executable script.
                for url in [
                    "data:,postMessage(1)",
                    "data:text/plain,postMessage(1)",
                    "data:application/json,postMessage(1)",
                ] {
                    let mut options = authority(&origin, &policy);
                    options.worker_destination = destination;
                    let failure = load(&network, options, url, kind, &AtomicBool::new(false))
                        .err()
                        .expect("non-JavaScript data worker must be rejected");
                    assert_eq!(failure.name, "NetworkError");
                    assert_eq!(failure.script_failure, Some(ScriptFailureKind::Fetch));
                    assert!(
                        failure.message.contains("JavaScript MIME type"),
                        "{}",
                        failure.message
                    );
                }
                for mime in ["text/javascript", "application/javascript"] {
                    let url =
                        format!("data:{mime};charset=windows-1252,postMessage('%F0%9F%92%A1')");
                    let mut options = authority(&origin, &policy);
                    options.worker_destination = destination;
                    let loaded =
                        load(&network, options, &url, kind, &AtomicBool::new(false)).unwrap();
                    assert_eq!(
                        String::from_utf16(&loaded.source).unwrap(),
                        "postMessage('💡')"
                    );
                    assert_eq!(
                        loaded.content_type.as_deref(),
                        Some(format!("{mime};charset=windows-1252").as_str())
                    );
                }
            }
        }
        assert!(
            requests.lock().unwrap().is_empty(),
            "data worker loading must not issue a network request"
        );
    }
    #[test]
    fn worker_import_rejects_mixed_content_before_transport() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: None,
        });
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy::default();
        assert!(
            load(
                &network,
                authority(&origin, &policy),
                "http://insecure.test/import.js",
                ScriptLoadKind::ClassicImport,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn worker_json_module_preserves_initiator_and_importing_referrer() {
        struct Json(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Json {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        (
                            "content-type".into(),
                            "application/json;charset=windows-1252".into(),
                        ),
                        (
                            "access-control-allow-origin".into(),
                            "https://a.test".into(),
                        ),
                    ],
                    body: "{\"value\":\"é\"}".as_bytes().to_vec(),
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Json(requests.clone()));
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy::default();
        let mut options = authority(&origin, &policy);
        let ancestors = vec![
            "https://top.test/".into(),
            "https://cross.test/".into(),
            "https://top.test/frame".into(),
        ];
        options.cookie_ancestor_urls = &ancestors;
        options.referrer_source = Some("https://cdn.test/modules/main.js");
        options.referrer_policy = "unsafe-url";
        let loaded = load(
            &network,
            options,
            "https://cdn.test/data.json",
            ScriptLoadKind::JsonModuleImport,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            String::from_utf16(&loaded.source).unwrap(),
            "{\"value\":\"é\"}"
        );
        assert_eq!(loaded.referrer_policy, "unsafe-url");
        let request = &requests.lock().unwrap()[0];
        assert_eq!(request.security.ancestor_urls, ancestors);
        assert_eq!(
            request.security.top_level_url.as_deref(),
            Some("https://top.test/")
        );
        assert_eq!(
            request.security.initiator_url.as_deref(),
            Some("https://a.test/main.js")
        );
        assert!(
            request
                .headers
                .contains(&("referer".into(), "https://cdn.test/modules/main.js".into()))
        );
        assert!(
            request
                .headers
                .contains(&("origin".into(), "https://a.test".into()))
        );
        assert!(!request.credentials);
        assert!(
            decode_imported_source(
                &NetworkResponse {
                    timing: None,
                    request_id: 0,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: b"{}".to_vec()
                },
                ScriptLoadKind::JsonModuleImport
            )
            .is_err()
        );
    }

    #[test]
    fn restored_classic_worker_import_requires_the_same_strict_mime_as_network_loading() {
        let metadata = AgentImportMetadata {
            kind: AgentImportKind::Classic,
            response_url: "https://example.test/import.js".into(),
            content_type: Some("text/javascript".into()),
            referrer_policy: "no-referrer".into(),
            request_referrer_source: None,
            request_referrer_policy: "no-referrer".into(),
        };
        assert!(validate_import_metadata(&metadata, ScriptLoadKind::ClassicImport).is_ok());
        for mime in [
            None,
            Some("text/html"),
            Some("application/json"),
            Some("text/plain"),
        ] {
            let mut invalid = metadata.clone();
            invalid.content_type = mime.map(str::to_owned);
            assert!(validate_import_metadata(&invalid, ScriptLoadKind::ClassicImport).is_err());
        }
    }

    #[test]
    fn durable_import_provenance_roundtrip_keeps_mime_and_request_response_policy_separate() {
        let response = NetworkResponse {
            timing: None,
            request_id: 0,
            status: 200,
            headers: vec![
                (
                    "content-type".into(),
                    "application/manifest+json;charset=windows-1252".into(),
                ),
                ("referrer-policy".into(), "no-referrer".into()),
            ],
            body: b"{}".to_vec(),
        };
        let script = loaded(
            response,
            &Url::parse("https://example.test/final.json").unwrap(),
            ScriptLoadKind::JsonModuleImport,
            None,
            "unsafe-url",
        )
        .unwrap();
        let metadata = script.import_metadata(
            ScriptLoadKind::JsonModuleImport,
            Some("https://example.test/importer.js"),
            "unsafe-url",
        );
        let saved = serde_json::to_vec(&metadata).unwrap();
        let restored: AgentImportMetadata = serde_json::from_slice(&saved).unwrap();
        assert_eq!(metadata, restored);
        assert_eq!(restored.response_url, "https://example.test/final.json");
        assert_eq!(restored.referrer_policy, "no-referrer");
        assert_eq!(restored.request_referrer_policy, "unsafe-url");
        assert!(validate_import_metadata(&restored, ScriptLoadKind::JsonModuleImport).is_ok());
        assert!(validate_import_metadata(&restored, ScriptLoadKind::ModuleImport).is_err());
        let mut missing = restored.clone();
        missing.content_type = None;
        assert!(validate_import_metadata(&missing, ScriptLoadKind::JsonModuleImport).is_err());
    }

    #[test]
    fn loaded_worker_preserves_enforcing_embedder_policy_and_inherited_blob_policy() {
        let url = Url::parse("https://a.test/worker.js").unwrap();
        let response = NetworkResponse {
            timing: None,
            request_id: 0,
            status: 200,
            headers: vec![
                ("content-type".into(), "text/javascript".into()),
                ("cross-origin-embedder-policy".into(), "require-corp".into()),
                (
                    "cross-origin-embedder-policy-report-only".into(),
                    "credentialless".into(),
                ),
                (
                    "permissions-policy".into(),
                    "cross-origin-isolated=()".into(),
                ),
            ],
            body: b"self.ready=true".to_vec(),
        };
        let script = loaded(
            response,
            &url,
            ScriptLoadKind::EntryClassic,
            None,
            "no-referrer",
        )
        .unwrap();
        let policy =
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&script.policy.headers);
        assert_eq!(
            policy.embedder,
            crate::agent_isolation::EmbedderPolicy::RequireCorp
        );
        assert!(!policy.same_origin_permission);
        let inherited = script.policy;
        let blob = loaded(
            NetworkResponse {
                timing: None,
                request_id: 0,
                status: 200,
                headers: vec![("content-type".into(), "text/javascript".into())],
                body: Vec::new(),
            },
            &Url::parse("blob:https://a.test/native").unwrap(),
            ScriptLoadKind::EntryClassic,
            Some(&inherited),
            "no-referrer",
        )
        .unwrap();
        assert_eq!(blob.policy.headers, inherited.headers);
    }
    #[test]
    fn worker_source_load_uses_native_authority_and_response_policy() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: None,
        });
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy {
            hide_nonces: false,
            policy_sources: Vec::new(),
            policy_self_urls: Vec::new(),
            policy_self_url: None,
            url: "https://a.test/page".into(),
            headers: vec![("content-security-policy".into(), "worker-src 'self'".into())],
        };
        let loaded = load(
            &network,
            authority(&origin, &policy),
            "./worker.js",
            ScriptLoadKind::EntryClassic,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            String::from_utf16(&loaded.source).unwrap(),
            "self.value='é';"
        );
        assert!(
            !CspPolicySet::from_agent_policy(&loaded.policy)
                .allows_eval(false)
                .allowed
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].credentials);
        assert_eq!(
            requests[0].security.top_level_url.as_deref(),
            Some("https://top.test/")
        );
        assert!(
            !requests[0]
                .headers
                .iter()
                .any(|(name, _)| name == "referer")
        );
    }
    #[test]
    fn creator_policy_and_cross_origin_entry_reject_before_network() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: None,
        });
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy {
            hide_nonces: false,
            policy_sources: Vec::new(),
            policy_self_urls: Vec::new(),
            policy_self_url: None,
            url: "https://a.test/page".into(),
            headers: vec![("content-security-policy".into(), "worker-src 'none'".into())],
        };
        assert!(
            load(
                &network,
                authority(&origin, &policy),
                "./worker.js",
                ScriptLoadKind::EntryClassic,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let policy = AgentPolicy::default();
        assert!(
            load(
                &network,
                authority(&origin, &policy),
                "https://b.test/worker.js",
                ScriptLoadKind::EntryModule,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn audio_worklet_entry_uses_script_policy_cors_and_worklet_metadata() {
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = CspPolicySet::from_headers(
            "https://a.test/page",
            &[(
                "Content-Security-Policy".into(),
                "script-src https://b.test; worker-src 'none'".into(),
            )],
        )
        .worker_snapshot();
        for allowed in [None, Some("https://a.test".into())] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let network = SharedNetworkBroker::new(Fixture {
                requests: requests.clone(),
                allow_origin: allowed.clone(),
            });
            let mut options = authority(&origin, &policy);
            options.worker_destination = "audioworklet";
            assert_eq!(
                load(
                    &network,
                    options,
                    "https://b.test/module.js",
                    ScriptLoadKind::EntryModule,
                    &AtomicBool::new(false)
                )
                .is_ok(),
                allowed.is_some()
            );
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(!requests[0].credentials);
            assert!(
                requests[0]
                    .headers
                    .contains(&("sec-fetch-dest".into(), "audioworklet".into()))
            );
            assert!(
                requests[0]
                    .headers
                    .contains(&("sec-fetch-mode".into(), "cors".into()))
            );
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: Some("https://a.test".into()),
        });
        assert!(
            load(
                &network,
                authority(&origin, &policy),
                "https://b.test/module.js",
                ScriptLoadKind::EntryModule,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn audio_worklet_data_import_retains_secure_creator_mixed_content_authority() {
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = CspPolicySet::from_headers(
            "https://a.test/page",
            &[(
                "Content-Security-Policy".into(),
                "script-src * data:; worker-src 'none'".into(),
            )],
        )
        .worker_snapshot();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
            allow_origin: Some("https://a.test".into()),
        });
        let mut options = authority(&origin, &policy);
        options.worker_destination = "audioworklet";
        options.script_url = "data:text/javascript,import 'http://insecure.test/module.js'";
        assert!(
            load(
                &network,
                options,
                "http://insecure.test/module.js",
                ScriptLoadKind::ModuleImport,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn cross_origin_module_import_requires_cors_and_omits_same_origin_credentials() {
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "origin:a".into(),
            opaque: false,
        };
        let policy = AgentPolicy::default();
        for allowed in [None, Some("https://a.test".into())] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let network = SharedNetworkBroker::new(Fixture {
                requests: requests.clone(),
                allow_origin: allowed.clone(),
            });
            assert_eq!(
                load(
                    &network,
                    authority(&origin, &policy),
                    "https://b.test/module.js",
                    ScriptLoadKind::ModuleImport,
                    &AtomicBool::new(false)
                )
                .is_ok(),
                allowed.is_some()
            );
            assert!(!requests.lock().unwrap()[0].credentials);
        }
    }
}

#[cfg(test)]
mod csp_delivery_tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError};
    use std::sync::{Arc, Mutex};
    struct Fixture(Arc<Mutex<Vec<String>>>);
    impl NetworkBroker for Fixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.0.lock().unwrap().push(request.url.clone());
            Ok(NetworkResponse {
                timing: None,
                request_id: request.request_id,
                status: if request.url.ends_with("start.js") {
                    302
                } else {
                    200
                },
                headers: if request.url.ends_with("start.js") {
                    vec![("location".into(), "https://blocked.test/end.js".into())]
                } else {
                    vec![("content-type".into(), "text/javascript".into())]
                },
                body: b"self.loaded=true".to_vec(),
            })
        }
    }
    #[test]
    fn worker_csp_reports_enforced_and_report_only_decisions_before_denial_and_redirect_fetch() {
        let origin = AgentOrigin {
            serialized: "https://source.test".into(),
            authority: "source".into(),
            opaque: false,
        };
        for report_only in [false, true] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let broker = SharedNetworkBroker::new(Fixture(requests.clone()));
            let mut policy = CspPolicySet::new("https://source.test/document");
            policy.install(
                "worker-src 'none'",
                if report_only {
                    crate::csp::CspDisposition::Report
                } else {
                    crate::csp::CspDisposition::Enforce
                },
                "header",
            );
            let policy = policy.worker_snapshot();
            let reports = std::cell::RefCell::new(Vec::new());
            let report = |_: &CspPolicySet, violations: Vec<crate::csp::CspViolation>| {
                reports.borrow_mut().extend(violations);
                Ok(())
            };
            let result = load(
                &broker,
                ScriptAuthority {
                    hash_report: None,
                    worker_destination: "worker",
                    coep_report: None,
                    report: Some(&report),
                    cookie_ancestor_urls: &[],
                    referrer_source: None,
                    origin: &origin,
                    policy: &policy,
                    script_url: "https://source.test/document",
                    referrer_policy: "no-referrer",
                    top_level_url: None,
                    credentials: WorkerCredentials::SameOrigin,
                },
                "worker.js",
                ScriptLoadKind::EntryClassic,
                &AtomicBool::new(false),
            );
            assert_eq!(result.is_ok(), report_only);
            assert_eq!(requests.lock().unwrap().len(), usize::from(report_only));
            assert_eq!(reports.borrow().len(), 1);
            assert_eq!(reports.borrow()[0].effective_directive, "worker-src");
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Fixture(requests.clone()));
        let policy = CspPolicySet::from_headers(
            "https://source.test/document",
            &[(
                "Content-Security-Policy".into(),
                "script-src https://source.test".into(),
            )],
        )
        .worker_snapshot();
        let reports = std::cell::RefCell::new(Vec::new());
        let report = |_: &CspPolicySet, violations: Vec<crate::csp::CspViolation>| {
            reports.borrow_mut().extend(violations);
            Ok(())
        };
        assert!(
            load(
                &broker,
                ScriptAuthority {
                    hash_report: None,
                    worker_destination: "worker",
                    coep_report: None,
                    report: Some(&report),
                    cookie_ancestor_urls: &[],
                    referrer_source: None,
                    origin: &origin,
                    policy: &policy,
                    script_url: "https://source.test/document",
                    referrer_policy: "no-referrer",
                    top_level_url: None,
                    credentials: WorkerCredentials::SameOrigin
                },
                "start.js",
                ScriptLoadKind::ClassicImport,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(
            &*requests.lock().unwrap(),
            &["https://source.test/start.js"]
        );
        assert_eq!(reports.borrow().len(), 1);
        assert_eq!(reports.borrow()[0].effective_directive, "script-src-elem");
        let reject = |_: &CspPolicySet, _: Vec<crate::csp::CspViolation>| {
            Err(WorkerError::new("InvalidStateError", "retired exact owner"))
        };
        assert!(
            load(
                &broker,
                ScriptAuthority {
                    hash_report: None,
                    worker_destination: "worker",
                    coep_report: None,
                    report: Some(&reject),
                    cookie_ancestor_urls: &[],
                    referrer_source: None,
                    origin: &origin,
                    policy: &policy,
                    script_url: "https://source.test/document",
                    referrer_policy: "no-referrer",
                    top_level_url: None,
                    credentials: WorkerCredentials::SameOrigin
                },
                "never.js",
                ScriptLoadKind::ClassicImport,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}
