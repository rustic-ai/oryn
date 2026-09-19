//! Broker-backed script fetch shared by executable and module-loading callers.
//! Parent-owned contexts retain their additional CSP and lifecycle authority.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);
use crate::cookies::{CookieCredentials, CookieRequestContext, CookieRequestKind};
use crate::network::{
    NetworkRequest, SharedNetworkBroker, collect_operation_bounded_with_cancel, decode_data_url,
};
use crate::referrer::ReferrerPolicy;
use crate::script_load_failure::ScriptLoadFailure;
use url::Url;

pub(crate) struct ScriptFetch<'a> {
    pub hash_report: Option<&'a dyn Fn(Vec<crate::csp::ScriptHashReport>)>,
    pub fallback_encoding: Option<&'a str>,
    pub coep_report: Option<&'a dyn Fn(&Url, bool, bool, &[(String, String)])>,
    pub csp: Option<&'a crate::csp::CspPolicySet>,
    pub nonce: Option<&'a str>,
    pub parser_inserted: bool,
    pub csp_report:
        Option<&'a (dyn Fn(Vec<crate::csp::CspViolation>) -> Result<(), String> + Send + Sync)>,
    pub embedder_policy: crate::agent_isolation::DocumentIsolationPolicy,
    pub url: Url,
    pub initiator: &'a Url,
    pub referrer_source: Option<&'a Url>,
    pub top_level: &'a Url,
    pub module: bool,
    pub cross_origin: Option<&'a str>,
    pub integrity: Option<&'a str>,
    pub referrer_policy: Option<&'a str>,
}

pub(crate) fn load(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
) -> Result<(String, String), String> {
    load_with_taint(broker, next_id, options).map(|(url, source, _)| (url, source))
}

pub(crate) struct ScriptFetchResult {
    pub hash_evidence: crate::csp::ScriptHashEvidence,
    pub response_urls: Vec<String>,
    pub timing: Option<oryn_js::platform::ResourceTimingMetadata>,
    pub url: String,
    pub source: String,
    pub muted_errors: bool,
    pub referrer_policy: String,
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use std::sync::{Arc, Mutex};
    struct Resource(Arc<Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Resource {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let request_id = request.request_id;
            self.0.lock().unwrap().push(request);
            Ok(NetworkResponse {
                timing: None,
                request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/javascript".into())],
                body: b"globalThis.loaded=true".to_vec(),
            })
        }
    }
    #[test]
    fn standalone_script_reports_before_enforcement_without_blocking_report_only() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Resource(seen));
        let source = Url::parse("https://app.example/").unwrap();
        for enforced in [false, true] {
            let headers = vec![
                (
                    if enforced {
                        "cross-origin-embedder-policy"
                    } else {
                        "cross-origin-embedder-policy-report-only"
                    }
                    .into(),
                    "require-corp; report-to=\"reports\"".into(),
                ),
                (
                    "reporting-endpoints".into(),
                    "reports=\"https://app.example/reports\"".into(),
                ),
            ];
            let policy = crate::coep_reporting::Policy::new(source.as_str(), &headers);
            let recorded = Mutex::new(Vec::new());
            let report = |target: &Url, cors, credentials, headers: &[(String, String)]| {
                recorded.lock().unwrap().extend(policy.response(
                    "https://app.example",
                    false,
                    target,
                    cors,
                    credentials,
                    headers,
                    "script",
                ))
            };
            let result = load(
                &broker,
                &mut 1,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: Some(&report),
                    csp: None,
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: crate::agent_isolation::DocumentIsolationPolicy::from_headers(
                        &headers,
                    ),
                    url: Url::parse("https://remote.example/script.js").unwrap(),
                    initiator: &source,
                    referrer_source: None,
                    top_level: &source,
                    module: false,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                },
            );
            assert_eq!(result.is_err(), enforced);
            let reports = recorded.lock().unwrap();
            assert_eq!(reports.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
            assert_eq!(body[0]["body"]["destination"], "script");
            assert_eq!(
                body[0]["body"]["disposition"],
                if enforced { "enforce" } else { "reporting" }
            );
        }
    }
    #[test]
    fn partial_content_script_status_remains_loadable() {
        struct Partial;
        impl NetworkBroker for Partial {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 206,
                    headers: vec![
                        ("content-type".into(), "text/javascript".into()),
                        ("content-range".into(), "bytes 90000-90010/100000".into()),
                    ],
                    body: b"loaded=true".to_vec(),
                })
            }
        }
        let broker = SharedNetworkBroker::new(Partial);
        let source = Url::parse("https://app.example/").unwrap();
        let (_, text) = load(
            &broker,
            &mut 1,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: source.join("partial.js").unwrap(),
                initiator: &source,
                referrer_source: None,
                top_level: &source,
                module: false,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
        )
        .unwrap();
        assert_eq!(text, "loaded=true");
    }
    #[test]
    fn standalone_classic_scripts_enforce_committed_coep_and_credentialless_cookie_mode() {
        use crate::agent_isolation::{DocumentIsolationPolicy, EmbedderPolicy};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Resource(seen.clone()));
        let source = Url::parse("https://app.example/").unwrap();
        let options = |embedder| ScriptFetch {
            hash_report: None,
            fallback_encoding: None,
            coep_report: None,
            csp: None,
            nonce: None,
            parser_inserted: false,
            csp_report: None,
            embedder_policy: DocumentIsolationPolicy {
                embedder,
                same_origin_opener: true,
                same_origin_permission: true,
            },
            url: Url::parse("https://remote.example/script.js").unwrap(),
            initiator: &source,
            referrer_source: None,
            top_level: &source,
            module: false,
            cross_origin: None,
            integrity: None,
            referrer_policy: None,
        };
        assert!(load(&broker, &mut 1, options(EmbedderPolicy::RequireCorp)).is_err());
        assert!(load(&broker, &mut 2, options(EmbedderPolicy::Credentialless)).is_ok());
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[1].credentials);
        assert_eq!(requests[1].security.credentials, CookieCredentials::Omit);
    }
}

pub(crate) fn load_with_taint(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
) -> Result<(String, String, bool), String> {
    load_with_metadata(broker, next_id, options)
        .map(|result| (result.url, result.source, result.muted_errors))
        .map_err(|error| error.to_string())
}

pub(crate) fn load_with_metadata(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
) -> Result<ScriptFetchResult, ScriptLoadFailure> {
    load_metadata_for_type(broker, next_id, options, None, &NOT_CANCELLED)
}

pub(crate) fn load_typed_module(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
    kind: crate::ModuleType,
) -> Result<ScriptFetchResult, ScriptLoadFailure> {
    if !options.module {
        return Err(ScriptLoadFailure::Infrastructure(
            "typed module fetch requires module mode".into(),
        ));
    }
    load_metadata_for_type(broker, next_id, options, Some(kind), &NOT_CANCELLED)
}

pub(crate) fn load_with_metadata_cancellable(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
    cancel: &AtomicBool,
) -> Result<ScriptFetchResult, ScriptLoadFailure> {
    load_metadata_for_type(broker, next_id, options, None, cancel)
}

pub(crate) fn load_module_cancellable(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
    kind: crate::ModuleType,
    cancel: &AtomicBool,
) -> Result<ScriptFetchResult, ScriptLoadFailure> {
    if !options.module {
        return Err(ScriptLoadFailure::Infrastructure(
            "module fetch requires module mode".into(),
        ));
    }
    load_metadata_for_type(
        broker,
        next_id,
        options,
        (kind != crate::ModuleType::JavaScript).then_some(kind),
        cancel,
    )
}

fn load_metadata_for_type(
    broker: &SharedNetworkBroker,
    next_id: &mut u64,
    options: ScriptFetch<'_>,
    kind: Option<crate::ModuleType>,
    cancel: &AtomicBool,
) -> Result<ScriptFetchResult, ScriptLoadFailure> {
    let module_type = kind.unwrap_or_default();
    let hash_request_url = options.url.to_string();
    let mut url = options.url;
    let cors = options.module || options.cross_origin.is_some();
    let include = options
        .cross_origin
        .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"));
    let origin = options.initiator.origin();
    let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
        if cors { "cors" } else { "no-cors" },
        module_type.fetch_destination(),
        matches!(origin, url::Origin::Opaque(_)),
    );
    let mut request_origin = origin.ascii_serialization();
    let mut opaque = false;
    let mut eligible = true;
    let mut referrer_source = Some(options.referrer_source.unwrap_or(options.initiator).clone());
    let mut policy = options
        .referrer_policy
        .map(ReferrerPolicy::parse_list)
        .unwrap_or_default();
    let script_policy = policy;
    let timing_started = Instant::now();
    let mut timing_hops = Vec::new();
    let mut timing = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut response_urls = Vec::new();
    for redirect in 0..=10 {
        if cancel.load(Ordering::Acquire) {
            return Err(ScriptLoadFailure::Cancelled);
        }
        if let Some(csp) = options.csp {
            if let Some(upgraded) = csp.upgraded_url(&url) {
                url = upgraded;
            }
            let decision = csp.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination: module_type.csp_destination(),
                    url: url.to_string(),
                    nonce: options.nonce.map(str::to_owned),
                    inline_source: None,
                    parser_inserted: options.parser_inserted,
                    initiated_by_trusted_script: !options.parser_inserted,
                },
                redirect != 0,
                &hash_request_url,
            );
            if !decision.violations.is_empty()
                && let Some(report) = options.csp_report
            {
                report(decision.violations).map_err(ScriptLoadFailure::Infrastructure)?;
            }
            if cancel.load(Ordering::Acquire) {
                return Err(ScriptLoadFailure::Cancelled);
            }
            if !decision.allowed {
                return Err(ScriptLoadFailure::Fetch(
                    "script blocked by captured Content-Security-Policy".into(),
                ));
            }
        }
        response_urls.push(url.to_string());
        crate::mixed_content::enforce(options.initiator.as_str(), &url)
            .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
        if url.scheme() != "data" && url.origin() != origin && !cors {
            eligible = false;
        }
        let (headers, body) = if url.scheme() == "data" {
            let (mime, bytes) = decode_data_url(&url)
                .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
            (
                mime.map(|value| vec![("content-type".into(), value)])
                    .unwrap_or_default(),
                bytes,
            )
        } else {
            let cross = cors && (opaque || url.origin() != origin);
            let mut headers = vec![(
                "accept".into(),
                "text/javascript,application/javascript,*/*;q=0.1".into(),
            )];
            if cross {
                headers.push(("origin".into(), request_origin.clone()));
            }
            let referrer = referrer_source
                .as_ref()
                .and_then(|source| policy.header(source, &url));
            referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
            if let Some(value) = referrer {
                headers.push(("referer".into(), value));
            }
            fetch_metadata.observe(Some(options.initiator.as_str()), &url);
            let request_id = *next_id;
            *next_id = next_id
                .checked_add(1)
                .ok_or_else(|| ScriptLoadFailure::Capacity("script request identifiers".into()))?;
            let omit = options
                .embedder_policy
                .omit_credentials(cors, opaque || url.origin() != origin);
            let response = collect_operation_bounded_with_cancel(
                broker,
                NetworkRequest {
                    request_id,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: (include || !cross) && !omit,
                    security: CookieRequestContext {
                        fetch_metadata: fetch_metadata.clone(),
                        dns_timing: None,
                        credentials: if omit {
                            CookieCredentials::Omit
                        } else if cors && !include {
                            CookieCredentials::SameOrigin
                        } else {
                            CookieCredentials::Include
                        },
                        kind: CookieRequestKind::Script,
                        initiator_url: Some(options.initiator.to_string()),
                        top_level_url: Some(options.top_level.to_string()),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(|| {
                        ScriptLoadFailure::Fetch("script fetch deadline exceeded".into())
                    })?,
                8 * 1024 * 1024,
                cancel,
            )
            .map_err(ScriptLoadFailure::network)?;
            timing_hops.push(crate::network::timing_hop(&url, &response.headers));
            if cross {
                let mut origins = response
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"));
                let allowed = origins.next().is_some_and(|(_, value)| {
                    let value = value.trim_matches([' ', '\t']);
                    (!include && value == "*") || value == request_origin
                }) && origins.next().is_none();
                let credentials_allowed = !include || {
                    let mut values = response.headers.iter().filter(|(name, _)| {
                        name.eq_ignore_ascii_case("access-control-allow-credentials")
                    });
                    values
                        .next()
                        .is_some_and(|(_, value)| value.trim_matches([' ', '\t']) == "true")
                        && values.next().is_none()
                };
                if !allowed || !credentials_allowed {
                    return Err(ScriptLoadFailure::Fetch(
                        "cross-origin script failed CORS".into(),
                    ));
                }
            }
            if let Some(report) = options.coep_report {
                report(&url, cors, (include || !cross) && !omit, &response.headers);
            }
            options
                .embedder_policy
                .authorize_response(
                    &origin.ascii_serialization(),
                    false,
                    &url,
                    cors,
                    (include || !cross) && !omit,
                    &response.headers,
                )
                .map_err(ScriptLoadFailure::Fetch)?;
            if oryn_net::fetch::is_redirect_status(response.status) {
                if redirect == 10 {
                    return Err(ScriptLoadFailure::Fetch(
                        "script redirect limit exceeded".into(),
                    ));
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        ScriptLoadFailure::Fetch("script redirect omitted Location".into())
                    })?;
                let next = url
                    .join(location)
                    .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
                if cors && (!next.username().is_empty() || next.password().is_some()) {
                    return Err(ScriptLoadFailure::Fetch(
                        "CORS script redirect contains credentials".into(),
                    ));
                }
                if cors && url.origin() != next.origin() && url.origin() != origin {
                    opaque = true;
                    request_origin = "null".into();
                }
                policy = policy.after_response(&response.headers);
                url = next;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(ScriptLoadFailure::Fetch(format!(
                    "script response returned HTTP {}",
                    response.status
                )));
            }
            timing = Some(crate::network::resource_timing_metadata(
                &origin.ascii_serialization(),
                matches!(origin, url::Origin::Opaque(_)),
                timing_hops.clone(),
                &response,
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ));
            (response.headers, response.body)
        };
        if cancel.load(Ordering::Acquire) {
            return Err(ScriptLoadFailure::Cancelled);
        }
        if body.len() > 8 * 1024 * 1024 {
            return Err(ScriptLoadFailure::Capacity("script resource bytes".into()));
        }
        if !kind.map_or_else(
            || oryn_net::script_response_mime_allowed(&headers, options.module),
            |kind| kind.mime_allowed(&headers),
        ) {
            return Err(ScriptLoadFailure::Fetch(
                "script response blocked by MIME type or nosniff".into(),
            ));
        }
        crate::integrity::verify(options.integrity.unwrap_or(""), &body, cors || eligible)
            .map_err(|error| {
                ScriptLoadFailure::Integrity(format!(
                    "script integrity verification failed: {error:?}"
                ))
            })?;
        let extracted_type =
            oryn_common::mime_type::MimeType::from_headers(&headers).map(|mime| mime.serialize());
        let content_type = extracted_type.as_deref();
        let (source, _) = oryn_net::decode_resource_text_with_fallback(
            &body,
            content_type,
            if options.module {
                oryn_net::TextResourceKind::ModuleJavaScript
            } else {
                oryn_net::TextResourceKind::JavaScript
            },
            options.fallback_encoding,
        );
        if source.len() > 8 * 1024 * 1024 {
            return Err(ScriptLoadFailure::Capacity(
                "decoded script source bytes".into(),
            ));
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ScriptLoadFailure::Cancelled);
        }
        if module_type == crate::ModuleType::JavaScript
            && let (Some(policy), Some(report)) = (options.csp, options.hash_report)
        {
            report(policy.script_hash_reports(&hash_request_url, &body, cors || eligible));
        }
        let timing = timing.or_else(|| {
            Some(crate::network::local_resource_timing_metadata(
                &origin.ascii_serialization(),
                &body,
                content_type,
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ))
        });
        return Ok(ScriptFetchResult {
            hash_evidence: crate::csp::ScriptHashEvidence::from_bytes(&body, cors || eligible),
            response_urls,
            timing,
            url: url.to_string(),
            source,
            muted_errors: !cors && !eligible,
            referrer_policy: script_policy.after_response(&headers).as_str().to_owned(),
        });
    }
    Err(ScriptLoadFailure::Fetch(
        "script fetch did not produce a response".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn mixed_script_redirect_stops_before_insecure_transport() {
        struct Redirect(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url);
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 302,
                    headers: vec![("location".into(), "http://insecure.test/end.js".into())],
                    body: Vec::new(),
                })
            }
        }
        let source = Url::parse("https://source.test/path?q=secret").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirect(seen.clone()));
        let result = load_with_metadata(
            &broker,
            &mut 1,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: true,
                csp_report: None,
                embedder_policy: Default::default(),
                url: source.join("start.js").unwrap(),
                initiator: &source,
                referrer_source: None,
                top_level: &source,
                module: false,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
        );
        assert!(
            matches!(result,Err(ScriptLoadFailure::Fetch(message)) if message.contains("mixed content"))
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    #[test]
    fn captured_csp_checks_redirects_before_admission_and_keeps_report_only_results() {
        use crate::csp::{CspDisposition, CspPolicySet};
        struct Hops(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Hops {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url.clone());
                let redirect = request.url.ends_with("start.js");
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "https://blocked.test/end.js".into())]
                    } else {
                        vec![("content-type".into(), "text/javascript".into())]
                    },
                    body: b"void 0;".to_vec(),
                })
            }
        }
        let source = Url::parse("https://app.test/").unwrap();
        for (directive, disposition, expected_requests, allowed) in [
            ("script-src 'self'", CspDisposition::Enforce, 1, false),
            ("script-src 'none'", CspDisposition::Report, 2, true),
        ] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let reports = Mutex::new(Vec::new());
            let broker = SharedNetworkBroker::new(Hops(seen.clone()));
            let mut csp = CspPolicySet::new(source.as_str());
            csp.install(directive, disposition, "header");
            let report = |values: Vec<crate::csp::CspViolation>| {
                reports.lock().unwrap().extend(values);
                Ok(())
            };
            let result = load_with_metadata(
                &broker,
                &mut 1,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: Some(&csp),
                    nonce: None,
                    parser_inserted: true,
                    csp_report: Some(&report),
                    embedder_policy: Default::default(),
                    url: source.join("start.js").unwrap(),
                    initiator: &source,
                    referrer_source: None,
                    top_level: &source,
                    module: false,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                },
            );
            assert_eq!(result.is_ok(), allowed);
            assert_eq!(seen.lock().unwrap().len(), expected_requests);
            let reports = reports.lock().unwrap();
            assert_eq!(reports.len(), if allowed { 2 } else { 1 });
            assert!(
                reports.iter().all(
                    |violation: &crate::csp::CspViolation| violation.disposition == disposition
                )
            );
            assert_eq!(
                reports.last().unwrap().blocked_uri,
                "https://app.test/start.js"
            );
        }
    }
    #[test]
    fn captured_csp_rejects_data_and_report_failure_before_any_io() {
        struct Forbidden;
        impl NetworkBroker for Forbidden {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("denied script fetched")
            }
        }
        let broker = SharedNetworkBroker::new(Forbidden);
        let source = Url::parse("https://app.test/").unwrap();
        let mut csp = crate::csp::CspPolicySet::new(source.as_str());
        csp.install(
            "script-src 'none'",
            crate::csp::CspDisposition::Enforce,
            "header",
        );
        for reporting_failure in [false, true] {
            let report = move |_: Vec<crate::csp::CspViolation>| {
                if reporting_failure {
                    Err("report queue unavailable".into())
                } else {
                    Ok(())
                }
            };
            let mut next = 19;
            let result = load_with_metadata(
                &broker,
                &mut next,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: Some(&csp),
                    nonce: None,
                    parser_inserted: true,
                    csp_report: Some(&report),
                    embedder_policy: Default::default(),
                    url: Url::parse("data:text/javascript,void%200").unwrap(),
                    initiator: &source,
                    referrer_source: None,
                    top_level: &source,
                    module: false,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                },
            );
            assert_eq!(next, 19);
            assert!(if reporting_failure {
                matches!(result, Err(ScriptLoadFailure::Infrastructure(_)))
            } else {
                matches!(result, Err(ScriptLoadFailure::Fetch(_)))
            });
        }
    }

    #[test]
    fn captured_upgrade_precedes_source_matching_and_reporting_can_cancel_admission() {
        struct Reply(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url);
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: b"void 0;".to_vec(),
                })
            }
        }
        let source = Url::parse("https://app.test/").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Reply(seen.clone()));
        let mut policy = crate::csp::CspPolicySet::new(source.as_str());
        policy.install(
            "upgrade-insecure-requests; script-src 'self'",
            crate::csp::CspDisposition::Enforce,
            "header",
        );
        let result = load_with_metadata(
            &broker,
            &mut 1,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: Some(&policy),
                nonce: None,
                parser_inserted: true,
                csp_report: None,
                embedder_policy: Default::default(),
                url: Url::parse("http://app.test/script.js").unwrap(),
                initiator: &source,
                referrer_source: None,
                top_level: &source,
                module: false,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
        )
        .unwrap();
        assert_eq!(result.url, "https://app.test/script.js");
        assert_eq!(*seen.lock().unwrap(), vec!["https://app.test/script.js"]);
        seen.lock().unwrap().clear();
        let mut report_only = crate::csp::CspPolicySet::new(source.as_str());
        report_only.install(
            "script-src 'none'",
            crate::csp::CspDisposition::Report,
            "header",
        );
        let cancelled = AtomicBool::new(false);
        let report = |_: Vec<crate::csp::CspViolation>| {
            cancelled.store(true, Ordering::Release);
            Ok(())
        };
        let mut next = 29;
        let result = load_with_metadata_cancellable(
            &broker,
            &mut next,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: Some(&report_only),
                nonce: None,
                parser_inserted: false,
                csp_report: Some(&report),
                embedder_policy: Default::default(),
                url: source.join("script.js").unwrap(),
                initiator: &source,
                referrer_source: None,
                top_level: &source,
                module: false,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
            &cancelled,
        );
        assert!(matches!(result, Err(ScriptLoadFailure::Cancelled)));
        assert_eq!(next, 29);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn hash_reports_follow_successful_integrity_and_cors_visibility() {
        struct Reply;
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/javascript".into()),
                        ("access-control-allow-origin".into(), "*".into()),
                    ],
                    body: b"// Ceci n'est pas un commentaire\n".to_vec(),
                })
            }
        }
        let origin = Url::parse("https://app.test/document").unwrap();
        let broker = SharedNetworkBroker::new(Reply);
        let policy = crate::csp::CspPolicySet::from_headers(
            origin.as_str(),
            &[
                ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                (
                    "Content-Security-Policy".into(),
                    "script-src * 'report-sha256'; report-to hash".into(),
                ),
            ],
        );
        for (url, cross_origin, integrity, expected) in [
            ("https://app.test/a.js", None, None, Some(true)),
            (
                "https://remote.test/a.js",
                Some("anonymous"),
                None,
                Some(true),
            ),
            ("https://remote.test/a.js", None, None, Some(false)),
            (
                "https://app.test/a.js",
                Some("anonymous"),
                Some("sha256-foobar"),
                None,
            ),
        ] {
            let collected = Mutex::new(Vec::new());
            let report =
                |items: Vec<crate::csp::ScriptHashReport>| collected.lock().unwrap().extend(items);
            let result = load_with_metadata(
                &broker,
                &mut 1,
                ScriptFetch {
                    hash_report: Some(&report),
                    fallback_encoding: None,
                    coep_report: None,
                    csp: Some(&policy),
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: Default::default(),
                    url: Url::parse(url).unwrap(),
                    initiator: &origin,
                    referrer_source: None,
                    top_level: &origin,
                    module: false,
                    cross_origin,
                    integrity,
                    referrer_policy: None,
                },
            );
            let reports = collected.lock().unwrap();
            assert_eq!(result.is_ok(), expected.is_some());
            assert_eq!(reports.len(), usize::from(expected.is_some()));
            if let Some(exposed) = expected {
                let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
                assert_eq!(
                    body[0]["body"]["hash"],
                    if exposed {
                        "sha256-1XF/E08XndkoxwN6eIa5J89hYn3OVZ/UyB8BrU5jgzk="
                    } else {
                        ""
                    }
                );
            }
        }
    }

    #[test]
    fn script_hash_redirect_reports_original_request_url() {
        struct Redirect;
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let redirect = request.url.ends_with("start.js");
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "/final.js".into())]
                    } else {
                        vec![("content-type".into(), "text/javascript".into())]
                    },
                    body: if redirect { vec![] } else { b"void 0".to_vec() },
                })
            }
        }
        let origin = Url::parse("https://app.test/document").unwrap();
        let policy = crate::csp::CspPolicySet::from_headers(
            origin.as_str(),
            &[
                ("Reporting-Endpoints".into(), "hash=\"/reports\"".into()),
                (
                    "Content-Security-Policy".into(),
                    "script-src 'self' 'report-sha256'; report-to hash".into(),
                ),
            ],
        );
        let recorded = Mutex::new(Vec::new());
        let report =
            |items: Vec<crate::csp::ScriptHashReport>| recorded.lock().unwrap().extend(items);
        let loaded = load_with_metadata(
            &SharedNetworkBroker::new(Redirect),
            &mut 1,
            ScriptFetch {
                hash_report: Some(&report),
                fallback_encoding: None,
                coep_report: None,
                csp: Some(&policy),
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: origin.join("start.js").unwrap(),
                initiator: &origin,
                referrer_source: None,
                top_level: &origin,
                module: false,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
        )
        .unwrap();
        assert_eq!(loaded.url, "https://app.test/final.js");
        let reports = recorded.lock().unwrap();
        assert_eq!(reports.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
        assert_eq!(
            body[0]["body"]["subresourceURL"],
            "https://app.test/start.js"
        );
    }

    #[test]
    fn decoded_source_limit_is_distinct_from_fetch_failure() {
        struct Reply {
            status: u16,
            body: Vec<u8>,
        }
        impl NetworkBroker for Reply {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: self.status,
                    headers: vec![(
                        "content-type".into(),
                        "text/javascript;charset=windows-1252".into(),
                    )],
                    body: self.body.clone(),
                })
            }
        }
        let origin = Url::parse("https://example.test/").unwrap();
        for (status, body, capacity) in [
            (200, vec![0x80; 3 * 1024 * 1024], true),
            (404, Vec::new(), false),
        ] {
            let broker = SharedNetworkBroker::new(Reply { status, body });
            let error = load_with_metadata(
                &broker,
                &mut 1,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: None,
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: Default::default(),
                    url: origin.join("script.js").unwrap(),
                    initiator: &origin,
                    referrer_source: None,
                    top_level: &origin,
                    module: false,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                },
            )
            .err()
            .unwrap();
            if capacity {
                assert!(matches!(error, ScriptLoadFailure::Capacity(_)));
            } else {
                assert!(matches!(error, ScriptLoadFailure::Fetch(_)));
            }
        }
    }
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use base64::Engine;
    use sha2::Digest;

    #[test]
    fn cancelled_script_fetch_never_opens_network_or_decodes_data() {
        struct Forbidden;
        impl NetworkBroker for Forbidden {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("cancelled fetch opened network")
            }
        }
        let broker = SharedNetworkBroker::new(Forbidden);
        let origin = Url::parse("https://example.test/").unwrap();
        let cancel = AtomicBool::new(true);
        for value in [
            "https://example.test/a.js",
            "data:text/javascript,throw%201",
        ] {
            let mut next = 7;
            let error = load_with_metadata_cancellable(
                &broker,
                &mut next,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: None,
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: Default::default(),
                    url: Url::parse(value).unwrap(),
                    initiator: &origin,
                    referrer_source: None,
                    top_level: &origin,
                    module: false,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                },
                &cancel,
            )
            .err()
            .unwrap();
            assert!(matches!(error, ScriptLoadFailure::Cancelled));
            assert_eq!(next, 7);
        }
    }

    struct Redirects {
        seen: Arc<Mutex<Vec<NetworkRequest>>>,
        missing_cors: bool,
    }
    impl NetworkBroker for Redirects {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.seen.lock().unwrap().push(request.clone());
            let url = Url::parse(&request.url).unwrap();
            let mut headers = vec![("content-type".into(), "text/javascript".into())];
            if !self.missing_cors {
                headers.push((
                    "access-control-allow-origin".into(),
                    if url.host_str() == Some("first.test") {
                        "https://example.test"
                    } else {
                        "null"
                    }
                    .into(),
                ));
            }
            let status = match url.path() {
                "/start" => {
                    headers.push(("location".into(), "https://second.test/middle".into()));
                    headers.push(("referrer-policy".into(), "no-referrer".into()));
                    302
                }
                "/middle" => {
                    headers.push(("location".into(), "https://example.test/final".into()));
                    302
                }
                _ => 200,
            };
            Ok(NetworkResponse {
                timing: None,
                request_id: request.request_id,
                status,
                headers,
                body: b"export{}".to_vec(),
            })
        }
    }
    #[test]
    fn module_redirect_keeps_opaque_origin_credentials_and_reduced_referrer() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirects {
            seen: seen.clone(),
            missing_cors: false,
        });
        let initiator = Url::parse("https://example.test/private/page?secret").unwrap();
        let hash = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"export{}"))
        );
        let mut next = 100;
        let (url, source) = load(
            &broker,
            &mut next,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: Url::parse("https://first.test/start").unwrap(),
                initiator: &initiator,
                top_level: &initiator,
                referrer_source: None,
                module: true,
                cross_origin: None,
                integrity: Some(&hash),
                referrer_policy: Some("unsafe-url"),
            },
        )
        .unwrap();
        assert_eq!(url, "https://example.test/final");
        assert_eq!(source, "export{}");
        assert_eq!(
            next, 103,
            "caller reserves one identity per actual redirect hop"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            seen.iter()
                .map(|request| request.request_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "transport remaps caller IDs to unique owned operations"
        );
        for (index, request) in seen.iter().enumerate() {
            assert!(!request.credentials);
            assert!(
                request.headers.contains(&(
                    "origin".into(),
                    if index == 0 {
                        "https://example.test"
                    } else {
                        "null"
                    }
                    .into()
                ))
            );
            if index > 0 {
                assert!(!request.headers.iter().any(|(name, _)| name == "referer"));
            }
        }
    }
    #[test]
    fn typed_destinations_apply_on_every_redirect_and_do_not_emit_script_hashes() {
        struct Redirect;
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let kind = if request.url.contains("style") {
                    crate::ModuleType::Css
                } else if request.url.contains("json") {
                    crate::ModuleType::Json
                } else {
                    crate::ModuleType::Text
                };
                assert_eq!(
                    request.security.fetch_metadata.destination.as_deref(),
                    Some(kind.fetch_destination())
                );
                let redirect = request.url.ends_with("start");
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), request.url.replace("start", "end"))]
                    } else {
                        vec![(
                            "content-type".into(),
                            if kind == crate::ModuleType::Css {
                                "text/css"
                            } else {
                                "application/json"
                            }
                            .into(),
                        )]
                    },
                    body: b"{}".to_vec(),
                })
            }
        }
        let broker = SharedNetworkBroker::new(Redirect);
        let origin = Url::parse("https://app.test/").unwrap();
        for kind in [
            crate::ModuleType::JavaScript,
            crate::ModuleType::Css,
            crate::ModuleType::Json,
            crate::ModuleType::Text,
        ] {
            for allowed in [false, true] {
                let directive = if kind == crate::ModuleType::Css {
                    "style-src"
                } else {
                    "connect-src"
                };
                let policy = crate::csp::CspPolicySet::from_headers(
                    origin.as_str(),
                    &[(
                        "content-security-policy".into(),
                        format!(
                            "sandbox allow-same-origin; script-src 'none'; {directive} {}",
                            if allowed { "'self'" } else { "'none'" }
                        ),
                    )],
                );
                let hashes = std::cell::Cell::new(0);
                let report = |_| hashes.set(hashes.get() + 1);
                let result = load_typed_module(
                    &broker,
                    &mut 0,
                    ScriptFetch {
                        hash_report: Some(&report),
                        fallback_encoding: None,
                        coep_report: None,
                        csp: Some(&policy),
                        nonce: None,
                        parser_inserted: true,
                        csp_report: None,
                        embedder_policy: Default::default(),
                        url: origin
                            .join(&format!("{}/start", kind.fetch_destination()))
                            .unwrap(),
                        initiator: &origin,
                        referrer_source: None,
                        top_level: &origin,
                        module: true,
                        cross_origin: None,
                        integrity: None,
                        referrer_policy: None,
                    },
                    kind,
                );
                assert_eq!(
                    result.is_ok(),
                    allowed && kind != crate::ModuleType::JavaScript
                );
                assert_eq!(hashes.get(), 0);
                if let Ok(result) = result {
                    assert_eq!(result.response_urls.len(), 2);
                }
            }
        }
    }

    #[test]
    fn typed_module_fetch_uses_declared_mime_and_utf8_not_response_charset() {
        struct Typed;
        impl NetworkBroker for Typed {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let css = request.url.ends_with("sheet.css");
                Ok(NetworkResponse {
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![(
                        "content-type".into(),
                        if css {
                            "text/css; charset=windows-1252"
                        } else {
                            "application/json; charset=windows-1252"
                        }
                        .into(),
                    )],
                    body: if css {
                        ".café { color: red }".as_bytes().to_vec()
                    } else {
                        "{\"value\":\"é\"}".as_bytes().to_vec()
                    },
                })
            }
        }
        let broker = SharedNetworkBroker::new(Typed);
        let initiator = Url::parse("https://example.test/page").unwrap();
        for (path, kind, expected) in [
            ("data.json", crate::ModuleType::Json, "{\"value\":\"é\"}"),
            ("sheet.css", crate::ModuleType::Css, ".café { color: red }"),
        ] {
            let options = || ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: initiator.join(path).unwrap(),
                initiator: &initiator,
                referrer_source: None,
                top_level: &initiator,
                module: true,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            };
            assert_eq!(
                load_typed_module(&broker, &mut 0, options(), kind)
                    .unwrap()
                    .source,
                expected
            );
            assert!(
                load_typed_module(&broker, &mut 0, options(), crate::ModuleType::JavaScript)
                    .is_err()
            );
        }
    }

    #[test]
    fn script_result_preserves_options_policy_separately_from_redirect_policy() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirects {
            seen,
            missing_cors: false,
        });
        let initiator = Url::parse("https://example.test/private/page").unwrap();
        let result = load_with_metadata(
            &broker,
            &mut 0,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: Url::parse("https://first.test/start").unwrap(),
                initiator: &initiator,
                top_level: &initiator,
                referrer_source: None,
                module: true,
                cross_origin: None,
                integrity: None,
                referrer_policy: Some("unsafe-url"),
            },
        )
        .unwrap();
        assert_eq!(
            result.referrer_policy, "unsafe-url",
            "a redirect policy changes the request, not the original script options"
        );
    }

    #[test]
    fn module_referrer_uses_importing_script_while_cors_uses_document_origin() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirects {
            seen: seen.clone(),
            missing_cors: false,
        });
        let initiator = Url::parse("https://example.test/page").unwrap();
        let importing = Url::parse("https://first.test/importer.mjs").unwrap();
        load(
            &broker,
            &mut 0,
            ScriptFetch {
                hash_report: None,
                fallback_encoding: None,
                coep_report: None,
                csp: None,
                nonce: None,
                parser_inserted: false,
                csp_report: None,
                embedder_policy: Default::default(),
                url: Url::parse("https://first.test/final").unwrap(),
                initiator: &initiator,
                top_level: &initiator,
                referrer_source: Some(&importing),
                module: true,
                cross_origin: None,
                integrity: None,
                referrer_policy: None,
            },
        )
        .unwrap();
        let seen = seen.lock().unwrap();
        assert!(
            seen[0]
                .headers
                .contains(&("origin".into(), "https://example.test".into()))
        );
        assert!(
            seen[0]
                .headers
                .contains(&("referer".into(), "https://first.test/importer.mjs".into()))
        );
        assert!(!seen[0].credentials);
    }

    #[test]
    fn redirect_response_cors_failure_stops_before_next_fetch() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirects {
            seen: seen.clone(),
            missing_cors: true,
        });
        let initiator = Url::parse("https://example.test/").unwrap();
        assert!(
            load(
                &broker,
                &mut 0,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: None,
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: Default::default(),
                    url: Url::parse("https://first.test/start").unwrap(),
                    initiator: &initiator,
                    top_level: &initiator,
                    referrer_source: None,
                    module: true,
                    cross_origin: None,
                    integrity: None,
                    referrer_policy: None,
                }
            )
            .is_err()
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    #[test]
    fn classic_redirect_cannot_regain_integrity_eligibility_when_returning_home() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Redirects {
            seen,
            missing_cors: false,
        });
        let initiator = Url::parse("https://example.test/").unwrap();
        let hash = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"export{}"))
        );
        assert!(
            load(
                &broker,
                &mut 0,
                ScriptFetch {
                    hash_report: None,
                    fallback_encoding: None,
                    coep_report: None,
                    csp: None,
                    nonce: None,
                    parser_inserted: false,
                    csp_report: None,
                    embedder_policy: Default::default(),
                    url: Url::parse("https://first.test/start").unwrap(),
                    initiator: &initiator,
                    top_level: &initiator,
                    referrer_source: None,
                    module: false,
                    cross_origin: None,
                    integrity: Some(&hash),
                    referrer_policy: None,
                }
            )
            .unwrap_err()
            .contains("IneligibleResponse")
        );
    }
}
