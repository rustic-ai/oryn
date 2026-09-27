//! Native I/O executor for an actual parent-registry AgentId. No PageId is
//! synthesized, and request metadata cannot replace the bootstrap principal.
use crate::{
    agent_platform_service::AgentOperationExecutor,
    network::{NetworkBroker, NetworkEvent, NetworkOperation, NetworkRequest, SharedNetworkBroker},
    runtime::WeakBrowserContext,
};
use oryn_js::{platform::*, worker_wire::*};
use std::{
    collections::BTreeMap,
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tungstenite::{WebSocket, stream::MaybeTlsStream};
use url::Url;
static REQUEST: AtomicU64 = AtomicU64::new(1u64 << 61);
const MAX_AGENT_RESOURCES: usize = 64;
fn network_request_id() -> Result<u64, String> {
    REQUEST
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
        .map_err(|_| "agent network IDs exhausted".into())
}
struct PreparedKeepalive {
    request: PlatformHttpRequest,
    drain: crate::keepalive_drain::Ticket,
    controller: Option<Arc<crate::service_registry::FrozenFetchController>>,
}
struct HttpStream {
    deadline: Instant,
    _cleanup: Option<crate::http_cleanup_custody::Ticket>,
    _preload_owner: Option<crate::service_registry::PreloadCustody>,
    cancelled: Option<Arc<AtomicBool>>,
    navigation: Option<Arc<AtomicBool>>,
    _network: SharedNetworkBroker,
    operation: NetworkOperation,
    current: bytes::Bytes,
    read: usize,
    done: bool,
}
impl Drop for HttpStream {
    fn drop(&mut self) {
        if !self.done
            && let Some(cancelled) = &self.cancelled
        {
            cancelled.store(true, Ordering::Release);
        }
    }
}
type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
type OwnedHttpStream = (
    u64,
    Arc<AtomicBool>,
    Arc<Mutex<HttpStream>>,
    Option<crate::keepalive_drain::Ticket>,
);
fn discard_stream((_, cancelled, stream, _): OwnedHttpStream) {
    cancelled.store(true, Ordering::Release);
    drop(stream);
}
// Cleanup runs after the IO owner joins all operations. Snapshot before
// calling storage so registry ownership locks never span persistence work.
fn drain_idb_ledger(
    ledger: &Mutex<BTreeMap<String, String>>,
    mut release: impl FnMut(&str, &str) -> Result<(), String>,
) -> Vec<String> {
    let pending = match ledger.lock() {
        Ok(entries) => entries.clone(),
        Err(_) => return vec!["agent IDB cleanup ledger poisoned".into()],
    };
    let mut errors = Vec::new();
    for (token, database) in pending {
        match release(&token, &database) {
            Ok(()) => match ledger.lock() {
                Ok(mut entries) => {
                    if entries.get(&token) == Some(&database) {
                        entries.remove(&token);
                    }
                }
                Err(_) => errors.push("agent IDB cleanup ledger poisoned".into()),
            },
            Err(error) => errors.push(error),
        }
    }
    errors
}
pub(crate) struct AgentIo {
    context: WeakBrowserContext,
    bootstrap: AgentBootstrap,
    network: SharedNetworkBroker,
    retired: AtomicBool,
    cleanup: crate::http_cleanup_custody::Scope,
    keepalive_cleanup: crate::http_cleanup_custody::Scope,
    prepared_keepalive: Mutex<BTreeMap<HostOperationId, PreparedKeepalive>>,
    next: AtomicU64,
    streams: Mutex<BTreeMap<u64, OwnedHttpStream>>,
    preload_owners: Mutex<Vec<SharedNetworkBroker>>,
    sockets: Mutex<BTreeMap<u64, Arc<Mutex<Socket>>>>,
    connecting: Mutex<BTreeMap<HostOperationId, TcpStream>>,
    socket_cancels: Mutex<BTreeMap<u64, TcpStream>>,
    idb: Mutex<()>,
    transactions: Mutex<BTreeMap<String, String>>,
    idb_connections: Mutex<BTreeMap<String, String>>,
    storage: Option<crate::storage::SharedContextStore>,
}
impl AgentIo {
    pub(crate) fn new(
        context: WeakBrowserContext,
        bootstrap: AgentBootstrap,
        network: SharedNetworkBroker,
    ) -> Self {
        let storage = context.upgrade().map(|context| context.context_store());
        Self {
            keepalive_cleanup: Default::default(),
            prepared_keepalive: Default::default(),
            cleanup: Default::default(),
            storage,
            context,
            bootstrap,
            network,
            retired: AtomicBool::new(false),
            next: AtomicU64::new(1),
            streams: Mutex::new(BTreeMap::new()),
            preload_owners: Mutex::new(Vec::new()),
            sockets: Mutex::new(BTreeMap::new()),
            connecting: Mutex::new(BTreeMap::new()),
            socket_cancels: Mutex::new(BTreeMap::new()),
            idb: Mutex::new(()),
            transactions: Mutex::new(BTreeMap::new()),
            idb_connections: Mutex::new(BTreeMap::new()),
        }
    }
    fn check(&self, cancel: &AtomicBool) -> Result<(), String> {
        if cancel.load(Ordering::Acquire) || self.retired.load(Ordering::Acquire) {
            return Err("AbortError: agent I/O retired".into());
        }
        self.context
            .upgrade()
            .ok_or("agent context retired")?
            .worker_bootstrap(self.bootstrap.id)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    fn check_http(&self, cancel: &AtomicBool, keepalive: bool) -> Result<(), String> {
        if !keepalive {
            return self.check(cancel);
        }
        if cancel.load(Ordering::Acquire)
            || self
                .context
                .upgrade()
                .is_none_or(|context| !context.keepalive_context_active())
        {
            return Err("AbortError: keepalive context retired".into());
        }
        Ok(())
    }
    fn prepare_keepalive(
        &self,
        request: &PlatformHttpRequest,
    ) -> Result<PreparedKeepalive, String> {
        self.check(&AtomicBool::new(false))?;
        let drain = crate::keepalive_drain::Ticket::reserve()?;
        let context = self.context.upgrade().ok_or("agent context retired")?;
        let controller = if let Some(services) = context.existing_service_registry() {
            services
                .ensure_agent_controller(&self.bootstrap)
                .map_err(|error| error.to_string())?;
            services
                .freeze_fetch_controller(&crate::agent_registry::AgentOwner::Agent(
                    self.bootstrap.id,
                ))
                .map_err(|error| error.to_string())?
        } else {
            None
        };
        Ok(PreparedKeepalive {
            request: request.clone(),
            drain,
            controller,
        })
    }
    fn id(&self) -> Result<u64, String> {
        self.next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| "agent resource IDs exhausted".into())
    }
    fn event(
        &self,
        operation: &mut NetworkOperation,
        cancel: &AtomicBool,
        deadline: Instant,
        external: Option<&AtomicBool>,
        navigation: Option<&AtomicBool>,
        keepalive: bool,
    ) -> Result<NetworkEvent, String> {
        loop {
            self.check_http(cancel, keepalive)?;
            if external
                .into_iter()
                .chain(navigation)
                .any(|cancel| cancel.load(Ordering::Acquire))
            {
                return Err("AbortError: native navigation cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("TimeoutError: agent network deadline".into());
            }
            match operation.try_event() {
                Ok(event) => return Ok(event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(_) => return Err("agent network operation disconnected".into()),
            }
        }
    }
    fn http_start(
        &self,
        _id: HostOperationId,
        mut request: PlatformHttpRequest,
        image: bool,
        cors_enabled: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Result<PlatformHostResult, String> {
        let keepalive = request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive);
        if keepalive && image {
            return Err("invalid image keepalive authority".into());
        }
        let prepared = if keepalive {
            let value = self
                .prepared_keepalive
                .lock()
                .map_err(|_| "agent keepalive custody poisoned")?
                .remove(&_id);
            let value = match value {
                Some(value) => value,
                None => self.prepare_keepalive(&request)?,
            };
            if value.request != request {
                return Err("agent keepalive prepared request differs".into());
            }
            Some(value)
        } else {
            None
        };
        let mut lease: Option<Arc<crate::keepalive::Lease>> = None;
        use oryn_net::cors::{
            cors_exposed_response_headers, cors_headers_allowed, cors_non_safelisted_headers,
            cors_preflight_required,
        };
        if request.upgrade_mixed_image && !image {
            return Err("image upgrade category on a non-image request".into());
        }
        if let Some(fetch) = &request.fetch {
            fetch.validate()?;
        }
        let cors_enabled = request.fetch.as_ref().map_or(cors_enabled, |fetch| {
            matches!(fetch.mode.as_str(), "cors" | "same-origin")
        });
        let timing_started = Instant::now();
        let mut timing_allowed = true;
        let mut timing_hops = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        let source = Url::parse(&self.bootstrap.script_url).map_err(|error| error.to_string())?;
        let mut referrer_source = request
            .initiator_url
            .as_deref()
            .and_then(|value| Url::parse(value).ok())
            .filter(|value| {
                !self.bootstrap.origin.opaque
                    && value.origin().ascii_serialization() == self.bootstrap.origin.serialized
            })
            .or_else(|| Some(source.clone()));
        let mut target = source
            .join(&request.url)
            .map_err(|error| error.to_string())?;
        let original_request_url = target.to_string();
        let policy = crate::csp::CspPolicySet::from_agent_policy(&self.bootstrap.policy);
        let embedder = crate::agent_isolation::DocumentIsolationPolicy::from_headers(
            &self.bootstrap.policy.headers,
        );
        let coep_reporting = crate::coep_reporting::Policy::new(
            &self.bootstrap.policy.url,
            &self.bootstrap.policy.headers,
        );
        let mut origin = self.bootstrap.origin.serialized.clone();
        let mut referrer = crate::referrer::ReferrerPolicy::parse_list(
            request
                .referrer_policy
                .as_deref()
                .unwrap_or(&self.bootstrap.referrer_policy),
        );
        request.headers = crate::network::sanitize_worker_headers(request.headers);
        let mut method = tungstenite::http::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| error.to_string())?
            .to_string();
        if matches!(
            method.to_ascii_uppercase().as_str(),
            "CONNECT" | "TRACE" | "TRACK"
        ) {
            return Err("SecurityError: forbidden HTTP method".into());
        }
        if !matches!(
            request.credentials.as_str(),
            "omit" | "same-origin" | "include"
        ) {
            return Err("invalid agent credentials mode".into());
        }
        if request
            .fetch
            .as_ref()
            .is_some_and(|fetch| fetch.mode == "no-cors")
        {
            if !matches!(method.as_str(), "GET" | "HEAD" | "POST") {
                return Err("no-cors method is not safelisted".into());
            }
            request.headers.retain(|(name, value)| {
                oryn_common::http_headers::no_cors_request_header(name, value)
                    || (request.method.eq_ignore_ascii_case("GET")
                        && request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.privileged_range)
                        && name.eq_ignore_ascii_case("range")
                        && oryn_common::http_headers::safelisted_request_header(name, value))
            });
        }
        let mut body = request.body;
        let mut upload = None;
        let initial_site = crate::cookies::SchemefulSite::from_url(&target);
        let mut redirect_chain_cross_site = false;
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            request
                .fetch
                .as_ref()
                .map(|f| f.mode.as_str())
                .unwrap_or(if cors_enabled { "cors" } else { "no-cors" }),
            if image { "image" } else { "" },
            self.bootstrap.origin.opaque,
        );
        for redirects in 0..=20 {
            self.check_http(cancel, keepalive)?;
            if Instant::now() >= deadline {
                return Err("TimeoutError: agent fetch deadline".into());
            }
            if let Some(upgraded) = policy.upgraded_url(&target) {
                target = upgraded;
            }
            crate::context_service::upgrade_mixed_image(
                &self.bootstrap.origin.serialized,
                &mut target,
                request.upgrade_mixed_image,
            );
            let destination = if image {
                crate::csp::CspDestination::Image
            } else {
                crate::csp::CspDestination::Connect
            };
            let decision = policy.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination,
                    url: target.to_string(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                },
                redirects != 0,
                &original_request_url,
            );
            if !decision.violations.is_empty() {
                self.context
                    .upgrade()
                    .ok_or("agent context retired")?
                    .report_worker_csp(
                        &crate::agent_registry::AgentOwner::Agent(self.bootstrap.id),
                        &policy,
                        decision.violations,
                        self.bootstrap
                            .top_level_url
                            .as_deref()
                            .unwrap_or(&self.bootstrap.script_url),
                        keepalive,
                    )
                    .map_err(|error| error.to_string())?;
            }
            if !decision.allowed {
                return Err("SecurityError: agent fetch blocked by CSP".into());
            }
            crate::context_service::enforce_mixed_content(&self.bootstrap.script_url, &target)
                .map_err(|error| error.to_string())?;
            let local_data = target.scheme() == "data";
            if local_data && redirects != 0 {
                return Err("HTTP redirect to data URL denied".into());
            }
            let cross = !local_data
                && (self.bootstrap.origin.opaque
                    || origin == "null"
                    || target.origin().ascii_serialization() != self.bootstrap.origin.serialized);
            if request
                .fetch
                .as_ref()
                .is_some_and(|fetch| fetch.mode == "same-origin")
                && cross
            {
                return Err("same-origin fetch crossed an origin boundary".into());
            }
            let credentials = (request.credentials == "include"
                || (request.credentials == "same-origin" && !cross))
                && !embedder.omit_credentials(cors_enabled, cross);
            let mut headers = request.headers.clone();
            let referer = referrer_source
                .as_ref()
                .and_then(|source| referrer.header(source, &target));
            referrer_source = referer.as_deref().and_then(|value| Url::parse(value).ok());
            if let Some(value) = referer {
                headers.push(("referer".into(), value));
            }
            if let Some(value) = crate::referrer::request_origin_header(
                &origin,
                &source,
                &target,
                &method,
                cors_enabled && cross,
                referrer,
            ) {
                headers.push(("origin".into(), value));
            }
            let request_id = network_request_id()?;
            fetch_metadata.observe(Some(&self.bootstrap.origin.serialized), &target);
            let security = crate::cookies::CookieRequestContext {
                fetch_metadata: fetch_metadata.clone(),
                dns_timing: None,
                fetch_cache: request
                    .fetch
                    .as_ref()
                    .map_or_else(Default::default, |fetch| fetch.cache),
                credentials: if credentials {
                    crate::cookies::CookieCredentials::Include
                } else {
                    crate::cookies::CookieCredentials::Omit
                },
                kind: if image {
                    crate::cookies::CookieRequestKind::Image
                } else {
                    crate::cookies::CookieRequestKind::Fetch
                },
                initiator_url: Some(source.to_string()),
                top_level_url: self.bootstrap.top_level_url.clone(),
                ancestor_urls: self.bootstrap.cookie_ancestor_urls.clone(),
                redirect_chain_cross_site,
                ..Default::default()
            };
            let context = self.context.upgrade().ok_or("agent context retired")?;
            let service_request = AgentFetchRequest {
                keepalive: request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive),
                privileged_range: request
                    .fetch
                    .as_ref()
                    .is_some_and(|fetch| fetch.privileged_range),
                cache: request
                    .fetch
                    .as_ref()
                    .map_or_else(Default::default, |fetch| fetch.cache),
                navigation: Default::default(),
                integrity: request
                    .fetch
                    .as_ref()
                    .map(|fetch| fetch.integrity.clone())
                    .unwrap_or_default(),
                navigation_preload: false,
                streaming: true,
                id: 0,
                client_id: String::new(),
                url: target.to_string(),
                method: method.clone(),
                headers: headers.clone(),
                body: body.clone(),
                destination: if image { "image".into() } else { String::new() },
                mode: request
                    .fetch
                    .as_ref()
                    .map(|fetch| fetch.mode.clone())
                    .unwrap_or_else(|| {
                        if cors_enabled {
                            "cors".into()
                        } else {
                            "no-cors".into()
                        }
                    }),
                credentials: if embedder.omit_credentials(cors_enabled, cross) {
                    "omit".into()
                } else {
                    request.credentials.clone()
                },
                redirect: request
                    .fetch
                    .as_ref()
                    .map(|fetch| fetch.redirect.clone())
                    .unwrap_or_else(|| "follow".into()),
            };
            let service_response = if local_data {
                None
            } else if let Some(prepared) = &prepared {
                if let Some(controller) = &prepared.controller {
                    let weak = self.context.clone();
                    controller
                        .fetch_operation(
                            service_request,
                            request_id,
                            cancel.clone(),
                            Some(Arc::new(move || {
                                weak.upgrade()
                                    .is_none_or(|context| !context.keepalive_context_active())
                            })),
                        )
                        .map_err(|error| error.to_string())?
                } else {
                    None
                }
            } else if let Some(services) = context.existing_service_registry() {
                services
                    .ensure_agent_controller(&self.bootstrap)
                    .map_err(|error| error.to_string())?;
                services
                    .fetch_operation_cancellable(
                        &crate::agent_registry::AgentOwner::Agent(self.bootstrap.id),
                        service_request,
                        request_id,
                        cancel.clone(),
                    )
                    .map_err(|error| error.to_string())?
            } else {
                None
            };
            self.check_http(cancel, keepalive)?;
            let service_handled = service_response.is_some();
            let service_timing = service_response
                .as_ref()
                .and_then(|(response, _)| response.timing.clone());
            let service_range_requested = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.range_requested);
            let service_redirected = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.redirected);
            let service_cors = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.response_type == "cors");
            let service_readable = service_response.as_ref().is_some_and(|(response, _)| {
                matches!(
                    response.response_type.as_str(),
                    "basic" | "cors" | "default"
                )
            });
            let service_response_url = service_response
                .as_ref()
                .filter(|(response, _)| !response.response_url.is_empty())
                .map(|(response, _)| response.response_url.clone());
            if !service_handled
                && cors_enabled
                && cross
                && (request.use_cors_preflight
                    || cors_preflight_required(&method, &request.headers))
            {
                let requested_headers = cors_non_safelisted_headers(&request.headers);
                let mut preflight_headers = vec![
                    ("origin".into(), origin.clone()),
                    ("access-control-request-method".into(), method.clone()),
                ];
                if !requested_headers.is_empty() {
                    preflight_headers.push((
                        "access-control-request-headers".into(),
                        requested_headers.join(", "),
                    ));
                }
                let preflight_id = network_request_id()?;
                let preflight_cleanup = crate::http_cleanup_custody::Ticket::reserve(
                    if keepalive {
                        &self.keepalive_cleanup
                    } else {
                        &self.cleanup
                    },
                    preflight_id,
                    self.network.clone(),
                )
                .map_err(|error| error.to_string())?;
                preflight_cleanup.attach(preflight_id);
                let preflight = crate::network::collect_operation_bounded_with_cancel(
                    &self.network,
                    NetworkRequest {
                        request_id: preflight_id,
                        method: "OPTIONS".into(),
                        url: target.to_string(),
                        headers: preflight_headers,
                        body: Vec::new(),
                        credentials: false,
                        security: crate::cookies::CookieRequestContext {
                            dns_timing: None,
                            credentials: crate::cookies::CookieCredentials::Omit,
                            ..security.clone()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(10)),
                    64 * 1024,
                    cancel,
                )
                .map_err(|error| error.to_string())?;
                crate::context_service::enforce_cors_preflight(
                    &origin,
                    credentials,
                    &method,
                    &requested_headers,
                    &preflight,
                )
                .map_err(|error| error.to_string())?;
            }
            if keepalive && !service_handled && !local_data && lease.is_none() {
                lease = Some(Arc::new(context.reserve_keepalive(
                    &crate::agent_registry::AgentOwner::Agent(self.bootstrap.id),
                    body.len(),
                )?));
            }
            let cleanup = if !service_handled && !local_data {
                Some(
                    crate::http_cleanup_custody::Ticket::reserve(
                        if keepalive {
                            &self.keepalive_cleanup
                        } else {
                            &self.cleanup
                        },
                        request_id,
                        self.network.clone(),
                    )
                    .map_err(|error| error.to_string())?,
                )
            } else {
                None
            };
            if let (Some(cleanup), Some(lease)) = (&cleanup, &lease) {
                cleanup.retain_keepalive(lease.clone());
            }
            let mut response_type = None;
            let mut operation = if let Some((response, operation)) = service_response {
                response_type = match response.response_type.as_str() {
                    "opaque" if !cors_enabled => Some(FetchResponseType::Opaque),
                    "opaqueredirect"
                        if request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.redirect == "manual") =>
                    {
                        Some(FetchResponseType::OpaqueRedirect)
                    }
                    "error" | "opaque" | "opaqueredirect" => {
                        return Err("NetworkError: unusable service-worker response".into());
                    }
                    _ if response.status == 0 => {
                        return Err("NetworkError: unusable service-worker response".into());
                    }
                    "basic" | "cors" | "default" => None,
                    _ => return Err("NetworkError: invalid native service response type".into()),
                };
                operation
            } else if local_data {
                let (media, bytes) =
                    crate::network::decode_data_url(&target).map_err(|error| error.to_string())?;
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err("decoded data URL exceeds resource budget".into());
                }
                crate::network::buffered_response_operation(
                    request_id,
                    200,
                    vec![(
                        "content-type".into(),
                        media.unwrap_or_else(|| "text/plain;charset=US-ASCII".into()),
                    )],
                    bytes,
                )
                .map_err(|error| error.to_string())?
            } else {
                if let Some(cleanup) = &cleanup {
                    cleanup.attach(request_id);
                }
                self.network
                    .start_operation(NetworkRequest {
                        request_id,
                        method: method.clone(),
                        url: target.to_string(),
                        headers,
                        body: body.clone(),
                        credentials,
                        security,
                        resolved_addrs: Vec::new(),
                    })
                    .map_err(|error| error.to_string())?
            };
            let (status, headers, length, progress, hop_timing) =
                match self.event(&mut operation, cancel, deadline, None, None, keepalive)? {
                    NetworkEvent::Headers {
                        response_policy: _,
                        timing,
                        request_id,
                        status,
                        headers,
                        content_length,
                        upload,
                    } if request_id == operation.request_id => (
                        status,
                        headers,
                        content_length,
                        upload,
                        timing.map(|timing| *timing),
                    ),
                    _ => {
                        return Err(
                            "agent network did not provide correlated response headers".into()
                        );
                    }
                };
            let response_start_micros =
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            let timing_url = service_response_url
                .as_deref()
                .and_then(|url| Url::parse(url).ok())
                .unwrap_or_else(|| target.clone());
            if let Some(metadata) = service_timing.as_ref() {
                timing_allowed &= crate::network::timing_metadata_allows(
                    metadata,
                    &self.bootstrap.origin.serialized,
                    self.bootstrap.origin.opaque,
                );
                timing_hops.extend(metadata.hops.iter().cloned());
            } else {
                timing_allowed &= crate::network::timing_allow_origin(
                    &headers,
                    &self.bootstrap.origin.serialized,
                    !self.bootstrap.origin.opaque
                        && timing_url.origin().ascii_serialization()
                            == self.bootstrap.origin.serialized,
                ) && !service_redirected;
                timing_hops.push(crate::network::timing_hop(&timing_url, &headers));
            }
            if timing_hops.len() > 21 {
                timing_hops.clear();
                timing_allowed = false;
            }

            if redirects == 0 {
                upload = progress;
            }
            let response_policy_url = service_response_url
                .as_deref()
                .and_then(|value| Url::parse(value).ok())
                .unwrap_or_else(|| target.clone());
            if !local_data {
                let _ = context.deliver_coep_reports(
                    self.bootstrap
                        .top_level_url
                        .as_deref()
                        .unwrap_or(&self.bootstrap.script_url),
                    coep_reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                        origin: &self.bootstrap.origin.serialized,
                        opaque: self.bootstrap.origin.opaque,
                        target: &response_policy_url,
                        cors: cors_enabled || service_readable,
                        credentials,
                        headers: &headers,
                        destination: if image { "image" } else { "" },
                    }),
                );
            }
            if !local_data {
                embedder.authorize_response(
                    &self.bootstrap.origin.serialized,
                    self.bootstrap.origin.opaque,
                    &response_policy_url,
                    cors_enabled || service_readable,
                    credentials,
                    &headers,
                )?;
            }
            if !service_handled
                && cors_enabled
                && cross
                && !cors_headers_allowed(&origin, credentials, &headers)
            {
                return Err("NetworkError: agent CORS response rejected".into());
            }
            let manual = request
                .fetch
                .as_ref()
                .is_some_and(|fetch| fetch.redirect == "manual");
            if oryn_net::fetch::is_redirect_status(status)
                && let Some(location) = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
            {
                if request
                    .fetch
                    .as_ref()
                    .is_some_and(|fetch| fetch.redirect == "error")
                {
                    let _ = self.network.cancel_operation(operation.request_id);
                    return Err("fetch redirect mode is error".into());
                }
                if manual {
                    response_type = Some(FetchResponseType::OpaqueRedirect);
                } else {
                    if redirects == 20 {
                        return Err("agent redirect limit".into());
                    }
                    let next = target.join(location).map_err(|error| error.to_string())?;
                    oryn_net::fetch::rewrite_redirect_request(
                        status,
                        &target,
                        &next,
                        &mut method,
                        &mut request.headers,
                        &mut body,
                    );
                    if let Some(lease) = &lease {
                        lease.shrink_to(body.len());
                    }
                    if target.origin() != next.origin() && target.origin() != source.origin() {
                        origin = "null".into();
                    }
                    redirect_chain_cross_site |=
                        crate::cookies::SchemefulSite::from_url(&next) != initial_site;
                    let _ = self.network.cancel_operation(operation.request_id);
                    referrer = referrer.after_response(&headers);
                    target = next;
                    continue;
                }
            }
            if response_type.is_none() && length.is_some_and(|length| length > 16 * 1024 * 1024) {
                return Err("agent response body exceeds resource budget".into());
            }
            if let Some(fetch) = &request.fetch
                && !fetch.integrity.is_empty()
            {
                let verified = (|| -> Result<crate::network::CollectedBody, String> {
                    let bytes = crate::network::collect_started_body_with_metadata(
                        &mut operation,
                        deadline,
                        16 * 1024 * 1024,
                        cancel,
                        None,
                    )
                    .map_err(|error| error.to_string())?;
                    crate::integrity::verify(
                        &fetch.integrity,
                        &bytes.bytes,
                        response_type.is_none()
                            && (service_handled
                                || cors_enabled
                                || !cross
                                || request
                                    .fetch
                                    .as_ref()
                                    .is_some_and(|fetch| fetch.mode == "navigate")),
                    )
                    .map_err(|error| format!("fetch integrity check failed: {error:?}"))?;
                    Ok(bytes)
                })();
                let bytes = match verified {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = self.network.cancel_operation(operation.request_id);
                        return Err(error);
                    }
                };
                if let Some(cleanup) = &cleanup {
                    cleanup.finish_keepalive();
                }
                drop(lease.take());
                operation = crate::network::buffered_body_operation(operation.request_id, bytes)
                    .map_err(|error| error.to_string())?;
            }
            if (response_type.is_some()
                || (!service_handled
                    && request
                        .fetch
                        .as_ref()
                        .is_some_and(|fetch| fetch.mode == "no-cors")
                    && cross))
                && response_type.is_none()
            {
                response_type = Some(FetchResponseType::Opaque);
            }
            let stream_id = self.id()?;
            self.check_http(cancel, keepalive)?;
            {
                let mut streams = self.streams.lock().map_err(|_| "agent streams poisoned")?;
                if streams.len() >= MAX_AGENT_RESOURCES {
                    return Err("QuotaExceededError: agent response stream limit".into());
                }
                let cancelled = if service_handled {
                    cancel.clone()
                } else {
                    Arc::new(AtomicBool::new(false))
                };
                streams.insert(
                    stream_id,
                    (
                        operation.request_id,
                        cancelled.clone(),
                        Arc::new(Mutex::new(HttpStream {
                            deadline,
                            _cleanup: cleanup,
                            _preload_owner: None,
                            cancelled: Some(cancelled),
                            navigation: None,
                            _network: self.network.clone(),
                            operation,
                            current: bytes::Bytes::new(),
                            read: 0,
                            done: false,
                        })),
                        prepared.as_ref().map(|value| value.drain.clone()),
                    ),
                );
            }
            let retained_encoded_body_size = hop_timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size);
            let timing = Some(oryn_js::platform::ResourceTimingMetadata {
                server_timing: oryn_js::server_timing::parse(&headers),
                response_end_micros: None,
                navigation_phases: None,
                producer_clock_micros: crate::network::timing_clock_origin(
                    timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                ),
                decoded_body_size: None,
                origin: self.bootstrap.origin.serialized.clone(),
                hops: timing_hops,
                transport: hop_timing.map(|timing| {
                    let offset = response_start_micros.saturating_sub(timing.response_start);
                    timing.shifted(offset)
                }),
                allow_details: timing_allowed,
                response_status: status,
                response_start_micros,
                encoded_body_size: retained_encoded_body_size.or_else(|| {
                    headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.parse().ok())
                }),
                content_type: oryn_common::mime_type::MimeType::from_headers(&headers)
                    .map(|mime| mime.essence().to_owned())
                    .unwrap_or_default(),
            });
            let internal_response = response_type.map(|_| InternalResponseMetadata {
                status,
                headers: crate::network::sanitize_parent_response_headers(headers.clone()),
                url: service_response_url
                    .clone()
                    .unwrap_or_else(|| target.to_string()),
                redirected: service_redirected || redirects != 0,
                range_requested: service_range_requested
                    || request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("range")),
            });
            let headers = if !service_handled && cors_enabled && cross {
                cors_exposed_response_headers(headers, credentials)
            } else {
                headers
            };
            return Ok(PlatformHostResult::HttpStarted {
                timing: timing.map(Box::new),
                internal_response,
                response_type,
                upload: upload.map(|upload| PlatformUploadProgress {
                    bytes_consumed: upload.bytes_consumed,
                    total_bytes: upload.total_bytes,
                    body_complete: upload.body_complete,
                }),
                cors_filtered: response_type.is_some()
                    || (service_cors && !image)
                    || !service_handled && cross && (cors_enabled || image),
                redirected: response_type.is_none() && (service_redirected || redirects != 0),
                stream_id,
                status: if response_type.is_some() { 0 } else { status },
                url: if response_type.is_some() {
                    String::new()
                } else {
                    service_response_url.unwrap_or_else(|| target.to_string())
                },
                headers: if response_type.is_some() {
                    Vec::new()
                } else {
                    crate::network::sanitize_parent_response_headers(headers)
                },
            });
        }
        Err("agent fetch did not complete".into())
    }
    fn navigation_preload(
        &self,
        _id: HostOperationId,
        task: u64,
        cancel: &Arc<AtomicBool>,
    ) -> Result<PlatformHostResult, String> {
        if self.bootstrap.kind != AgentKind::Service {
            return Err("SecurityError: preload requires native service principal".into());
        }
        let context = self.context.upgrade().ok_or("service context retired")?;
        let services = context
            .existing_service_registry()
            .ok_or("service registry unavailable")?;
        let mut preload = services
            .take_preload(self.bootstrap.id, task)
            .map_err(|failure| failure.to_string())?;
        {
            let mut owners = self
                .preload_owners
                .lock()
                .map_err(|_| "preload cancellation owners poisoned")?;
            // Zero-budget drain only inspects this adapter's retained identities.
            owners.retain(|owner| owner.drain_owned_operations(Duration::ZERO).is_err());
            if owners.len() >= MAX_AGENT_RESOURCES {
                return Err("QuotaExceededError: retained preload cancellation capacity".into());
            }
            owners.push(preload.network.clone());
        }
        let mut operation = preload
            .operation
            .take()
            .ok_or("preload already consumed")??;
        let mut target = Url::parse(&preload.request.url).map_err(|failure| failure.to_string())?;
        let mut redirected = false;
        for redirects in 0..=20 {
            self.check(cancel)?;
            if preload.cancelled.load(Ordering::Acquire) {
                return Err("AbortError: navigation preload cancelled".into());
            }
            let (status, headers) = match self.event(
                &mut operation,
                cancel,
                Instant::now() + Duration::from_secs(30),
                Some(&preload.cancelled),
                None,
                false,
            )? {
                NetworkEvent::Headers {
                    request_id,
                    status,
                    headers,
                    ..
                } if request_id == operation.request_id => (status, headers),
                NetworkEvent::Failed { error, .. } => return Err(error.to_string()),
                _ => return Err("invalid navigation preload headers".into()),
            };
            if oryn_net::fetch::is_redirect_status(status) {
                if redirects == 20 {
                    return Err("navigation preload redirect limit".into());
                }
                let location = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or("preload redirect has no Location")?;
                let next = target
                    .join(location)
                    .map_err(|failure| failure.to_string())?;
                if !matches!(next.scheme(), "http" | "https")
                    || !next.username().is_empty()
                    || next.password().is_some()
                {
                    return Err("SecurityError: invalid preload redirect".into());
                }
                let _ = preload.network.cancel_operation(operation.request_id);
                oryn_net::fetch::rewrite_redirect_request(
                    status,
                    &target,
                    &next,
                    &mut preload.request.method,
                    &mut preload.request.headers,
                    &mut preload.request.body,
                );
                preload.redirect_referrer(&headers, &next);
                preload.request.request_id = network_request_id()?;
                preload.request.url = next.to_string();
                preload.request.resolved_addrs.clear();
                operation = preload
                    .network
                    .start_operation(preload.request.clone())
                    .map_err(|failure| failure.to_string())?;
                target = next;
                redirected = true;
                continue;
            }
            let stream_id = self.id()?;
            self.check(cancel)?;
            let mut streams = self.streams.lock().map_err(|_| "agent streams poisoned")?;
            if streams.len() >= MAX_AGENT_RESOURCES {
                return Err("QuotaExceededError: preload stream limit".into());
            }
            let stream_cancel = Arc::new(AtomicBool::new(false));
            streams.insert(
                stream_id,
                (
                    operation.request_id,
                    stream_cancel.clone(),
                    Arc::new(Mutex::new(HttpStream {
                        deadline: Instant::now() + Duration::from_secs(30),
                        _cleanup: None,
                        _preload_owner: preload.custody.take(),
                        cancelled: Some(stream_cancel),
                        navigation: Some(preload.cancelled.clone()),
                        _network: preload.network.clone(),
                        operation,
                        current: bytes::Bytes::new(),
                        read: 0,
                        done: false,
                    })),
                    None,
                ),
            );
            preload.handed_off = true;
            return Ok(PlatformHostResult::HttpStarted {
                timing: None,
                internal_response: None,
                response_type: None,
                upload: None,
                cors_filtered: false,
                redirected,
                stream_id,
                status,
                url: target.to_string(),
                headers: crate::network::sanitize_parent_response_headers(headers),
            });
        }
        Err("navigation preload failed".into())
    }
    fn http_read(
        &self,
        _id: HostOperationId,
        stream_id: u64,
        max: usize,
        cancel: &AtomicBool,
    ) -> Result<PlatformHostResult, String> {
        let (_, _, stream, _) = self
            .streams
            .lock()
            .map_err(|_| "agent streams poisoned")?
            .get(&stream_id)
            .cloned()
            .ok_or("foreign or retired agent stream")?;
        let mut stream = stream.lock().map_err(|_| "agent stream poisoned")?;
        let max = max.clamp(1, 1024 * 1024);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut terminal_encoded_bytes = None;
        while stream.current.is_empty() && !stream.done {
            stream.operation.request_body_chunk();
            let external = stream.cancelled.clone();
            let navigation = stream.navigation.clone();
            match self.event(
                &mut stream.operation,
                cancel,
                deadline,
                external.as_deref(),
                navigation.as_deref(),
                false,
            )? {
                NetworkEvent::Chunk { request_id, bytes }
                    if request_id == stream.operation.request_id =>
                {
                    stream.read = stream
                        .read
                        .checked_add(bytes.len())
                        .ok_or("agent body length overflow")?;
                    if stream.read > 16 * 1024 * 1024 {
                        return Err("agent body limit".into());
                    }
                    stream.current = bytes;
                }
                NetworkEvent::Complete {
                    encoded_body_size,
                    request_id,
                } if request_id == stream.operation.request_id => {
                    stream.done = true;
                    if let Some(cleanup) = &stream._cleanup {
                        cleanup.finish_keepalive();
                    }
                    terminal_encoded_bytes = encoded_body_size;
                }
                NetworkEvent::Failed { error, .. } => return Err(error.to_string()),
                _ => return Err("invalid agent response stream event".into()),
            }
        }
        let count = max.min(stream.current.len());
        let bytes = stream.current.split_to(count).to_vec();
        let done = stream.done && stream.current.is_empty();
        drop(stream);
        if done {
            self.streams
                .lock()
                .map_err(|_| "agent streams poisoned")?
                .remove(&stream_id);
        }
        Ok(PlatformHostResult::HttpChunk {
            encoded_body_size: terminal_encoded_bytes,
            stream_id,
            bytes,
            done,
        })
    }
    fn abort_stream(&self, stream_id: u64) {
        let stream = self
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&stream_id);
        if let Some(stream) = stream {
            discard_stream(stream);
        }
    }
    fn orphan_stream(&self, stream_id: u64) {
        if let Some(stream) = self
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&stream_id)
        {
            self.dispose_owned_stream(stream);
        }
    }
    fn dispose_owned_stream(&self, owned: OwnedHttpStream) {
        let ticket = owned.3.clone();
        let Some(ticket) = ticket else {
            discard_stream(owned);
            return;
        };
        let context = self.context.clone();
        let (_, cancelled, stream, _) = owned;
        ticket.orphan(move || {
            let Ok(mut stream) = stream.try_lock() else {
                return false;
            };
            if cancelled.load(Ordering::Acquire)
                || Instant::now() >= stream.deadline
                || context
                    .upgrade()
                    .is_none_or(|context| !context.keepalive_context_active())
            {
                return true;
            }
            stream.current = bytes::Bytes::new();
            for _ in 0..8 {
                stream.operation.request_body_chunk();
                match stream.operation.try_event() {
                    Ok(NetworkEvent::Chunk { request_id, bytes })
                        if request_id == stream.operation.request_id =>
                    {
                        let Some(read) = stream.read.checked_add(bytes.len()) else {
                            return true;
                        };
                        stream.read = read;
                        if read > 16 * 1024 * 1024 {
                            return true;
                        }
                    }
                    Ok(NetworkEvent::Complete { request_id, .. })
                        if request_id == stream.operation.request_id =>
                    {
                        stream.done = true;
                        if let Some(cleanup) = &stream._cleanup {
                            cleanup.finish_keepalive();
                        }
                        return true;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return false,
                    _ => return true,
                }
            }
            false
        });
    }
}

impl AgentIo {
    fn socket_open(
        &self,
        id: HostOperationId,
        url: &str,
        protocols: &[String],
        cancel: &AtomicBool,
    ) -> Result<PlatformHostResult, String> {
        use tungstenite::client::IntoClientRequest;
        let context = self.context.upgrade().ok_or("agent context retired")?;
        let mut url = Url::parse(url).map_err(|error| error.to_string())?;
        let policy = crate::csp::CspPolicySet::from_agent_policy(&self.bootstrap.policy);
        if let Some(upgraded) = policy.upgraded_url(&url) {
            url = upgraded;
        }
        if !matches!(url.scheme(), "ws" | "wss") || url.fragment().is_some() {
            return Err("invalid agent WebSocket URL".into());
        }
        if !policy
            .authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Connect,
                url: url.to_string(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            })
            .allowed
        {
            return Err("agent WebSocket blocked by CSP".into());
        }
        crate::context_service::enforce_mixed_content(&self.bootstrap.script_url, &url)
            .map_err(|error| error.to_string())?;
        let mut seen = std::collections::BTreeSet::new();
        for protocol in protocols {
            if protocol.is_empty()
                || !protocol
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
                || !seen.insert(protocol)
            {
                return Err("invalid WebSocket protocol".into());
            }
        }
        let addresses = context
            .resolve_network_endpoint(&url)
            .map_err(|error| error.to_string())?;
        let connector = match context.fixture_ca_pem() {
            Some(ca) => crate::context_service::explicit_websocket_connector(&url, ca)
                .map_err(|error| error.to_string())?,
            None => None,
        };
        let mut connected = None;
        let mut last = None;
        for address in addresses {
            self.check(cancel)?;
            match TcpStream::connect_timeout(&address, Duration::from_millis(250)) {
                Ok(stream) => {
                    connected = Some(stream);
                    break;
                }
                Err(error) => last = Some(error.to_string()),
            }
        }
        let stream = connected.ok_or_else(|| {
            last.unwrap_or_else(|| "agent WebSocket resolver returned no address".into())
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(10))))
            .map_err(|error| error.to_string())?;
        self.connecting
            .lock()
            .map_err(|_| "agent connecting sockets poisoned")?
            .insert(id, stream.try_clone().map_err(|error| error.to_string())?);
        self.check(cancel)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        for (name, value) in crate::fetch_metadata::websocket_headers(
            &url,
            &self.bootstrap.origin.serialized,
            self.bootstrap.origin.opaque,
        ) {
            request.headers_mut().insert(
                name,
                value
                    .parse()
                    .map_err(|error| format!("invalid native Fetch Metadata: {error}"))?,
            );
        }
        request.headers_mut().insert(
            "origin",
            self.bootstrap
                .origin
                .serialized
                .parse()
                .map_err(|error| format!("invalid agent origin: {error}"))?,
        );
        let mut cookie_url = url.clone();
        let _ = cookie_url.set_scheme(if url.scheme() == "wss" {
            "https"
        } else {
            "http"
        });
        if let Some(cookie) = context.agent_cookie_header(&self.bootstrap, &cookie_url) {
            request.headers_mut().insert(
                "cookie",
                cookie
                    .parse()
                    .map_err(|error| format!("invalid agent cookie: {error}"))?,
            );
        }
        if !protocols.is_empty() {
            request.headers_mut().insert(
                "sec-websocket-protocol",
                protocols
                    .join(", ")
                    .parse()
                    .map_err(|error| format!("invalid agent protocol: {error}"))?,
            );
        }
        let (mut socket, response) =
            tungstenite::client_tls_with_config(request, stream, None, connector)
                .map_err(|error| error.to_string())?;
        let protocol =
            crate::context_service::validate_websocket_negotiation(response.headers(), protocols)?;
        let stream = match socket.get_mut() {
            MaybeTlsStream::Plain(stream) => stream,
            MaybeTlsStream::Rustls(stream) => &mut stream.sock,
            _ => return Err("unsupported agent WebSocket TLS transport".into()),
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(2))))
            .map_err(|error| error.to_string())?;
        let cancel_stream = stream.try_clone().map_err(|error| error.to_string())?;
        self.check(cancel)?;
        let socket_id = self.id()?;
        {
            let mut sockets = self.sockets.lock().map_err(|_| "agent sockets poisoned")?;
            if sockets.len() >= MAX_AGENT_RESOURCES {
                return Err("QuotaExceededError: agent WebSocket limit".into());
            }
            self.socket_cancels
                .lock()
                .map_err(|_| "agent socket cancellations poisoned")?
                .insert(socket_id, cancel_stream);
            sockets.insert(socket_id, Arc::new(Mutex::new(socket)));
        }
        Ok(PlatformHostResult::WebSocketOpened {
            socket_id,
            protocol,
        })
    }
    fn close_socket(&self, id: u64) {
        if let Some(stream) = self
            .socket_cancels
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id)
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
    }
    fn socket_request(
        &self,
        request: PlatformHostRequest,
        cancel: &AtomicBool,
    ) -> Result<PlatformHostResult, String> {
        let id = match &request {
            PlatformHostRequest::WebSocketSend { socket_id, .. }
            | PlatformHostRequest::WebSocketPoll { socket_id, .. }
            | PlatformHostRequest::WebSocketClose { socket_id, .. } => *socket_id,
            _ => return Err("invalid agent socket operation".into()),
        };
        let socket = self
            .sockets
            .lock()
            .map_err(|_| "agent sockets poisoned")?
            .get(&id)
            .cloned()
            .ok_or("foreign or retired agent socket")?;
        let mut socket = socket.lock().map_err(|_| "agent socket poisoned")?;
        self.check(cancel)?;
        match request {
            PlatformHostRequest::WebSocketSend { frame, .. } => {
                let frame = match frame {
                    PlatformWebSocketFrame::Text { value } => {
                        tungstenite::Message::Text(value.into())
                    }
                    PlatformWebSocketFrame::Binary { bytes } => {
                        tungstenite::Message::Binary(bytes.into())
                    }
                    PlatformWebSocketFrame::Close { code, reason } => {
                        if reason.len() > 123
                            || code
                                .is_some_and(|code| code != 1000 && !(3000..=4999).contains(&code))
                        {
                            return Err("invalid WebSocket close frame".into());
                        }
                        tungstenite::Message::Close(code.map(|code| {
                            tungstenite::protocol::CloseFrame {
                                code: code.into(),
                                reason: reason.into(),
                            }
                        }))
                    }
                };
                socket.send(frame).map_err(|error| error.to_string())?;
                Ok(PlatformHostResult::WebSocketSent)
            }
            PlatformHostRequest::WebSocketPoll { max_frames, .. } => {
                let mut frames = Vec::new();
                let mut bytes = 0usize;
                for _ in 0..max_frames.min(32) {
                    self.check(cancel)?;
                    match socket.read() {
                        Ok(tungstenite::Message::Text(text)) => {
                            bytes += text.len();
                            frames.push(PlatformWebSocketFrame::Text {
                                value: text.to_string(),
                            });
                        }
                        Ok(tungstenite::Message::Binary(value)) => {
                            bytes += value.len();
                            frames.push(PlatformWebSocketFrame::Binary {
                                bytes: value.to_vec(),
                            });
                        }
                        Ok(tungstenite::Message::Close(frame)) => {
                            socket.flush().map_err(|error| error.to_string())?;
                            frames.push(PlatformWebSocketFrame::Close {
                                code: frame.as_ref().map(|frame| frame.code.into()),
                                reason: frame
                                    .map(|frame| frame.reason.to_string())
                                    .unwrap_or_default(),
                            });
                            break;
                        }
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(error))
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            break;
                        }
                        Err(
                            tungstenite::Error::ConnectionClosed
                            | tungstenite::Error::AlreadyClosed,
                        ) => return Err("WebSocket terminated without closing handshake".into()),
                        Err(error) => return Err(error.to_string()),
                    }
                    if bytes > 16 * 1024 * 1024 {
                        return Err("agent WebSocket frame budget".into());
                    }
                }
                Ok(PlatformHostResult::WebSocketEvents { frames })
            }
            PlatformHostRequest::WebSocketClose { code, reason, .. } => {
                if reason.len() > 123
                    || code.is_some_and(|code| code != 1000 && !(3000..=4999).contains(&code))
                {
                    return Err("invalid WebSocket close".into());
                }
                let (code, reason, was_clean) = crate::context_service::finish_websocket_close(
                    &mut socket,
                    code,
                    reason,
                    || self.check(cancel).is_err(),
                );
                drop(socket);
                self.close_socket(id);
                Ok(PlatformHostResult::WebSocketClosed {
                    code,
                    reason,
                    was_clean,
                })
            }
            _ => Err("invalid agent socket operation".into()),
        }
    }
}
#[cfg(test)]
mod custody_tests {
    use super::*;
    #[test]
    fn discarded_preload_stream_does_not_cancel_its_navigation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let navigation = Arc::new(AtomicBool::new(false));
        let (_sender, events) = tokio::sync::mpsc::channel(1);
        let stream = Arc::new(Mutex::new(HttpStream {
            deadline: Instant::now() + Duration::from_secs(30),
            _cleanup: None,
            _preload_owner: None,
            cancelled: Some(cancelled.clone()),
            navigation: Some(navigation.clone()),
            _network: SharedNetworkBroker::new(crate::network::UnavailableBroker),
            operation: NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 8,
                events,
                demand: None,
            },
            current: bytes::Bytes::new(),
            read: 0,
            done: false,
        }));
        discard_stream((8, cancelled.clone(), stream, None));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(!navigation.load(Ordering::Acquire));
    }
    #[test]
    fn discarded_stream_signals_cancellation_without_waiting_for_its_reader_lock() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (_sender, events) = tokio::sync::mpsc::channel(1);
        let stream = Arc::new(Mutex::new(HttpStream {
            deadline: Instant::now() + Duration::from_secs(30),
            _cleanup: None,
            _preload_owner: None,
            cancelled: Some(cancelled.clone()),
            navigation: None,
            _network: SharedNetworkBroker::new(crate::network::UnavailableBroker),
            operation: NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 7,
                events,
                demand: None,
            },
            current: bytes::Bytes::new(),
            read: 0,
            done: false,
        }));
        let reader = stream.lock().unwrap();
        discard_stream((7, cancelled.clone(), stream.clone(), None));
        assert!(cancelled.load(Ordering::Acquire));
        drop(reader);
        drop(stream);
    }
}

impl AgentOperationExecutor for AgentIo {
    fn retains_keepalive(&self) -> bool {
        true
    }
    fn prepare(&self, id: HostOperationId, request: &PlatformHostRequest) -> Result<(), String> {
        if let PlatformHostRequest::HttpStart { request } = request
            && request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive)
        {
            let prepared = self.prepare_keepalive(request)?;
            let mut entries = self
                .prepared_keepalive
                .lock()
                .map_err(|_| "agent keepalive custody poisoned")?;
            if entries.len() >= MAX_AGENT_RESOURCES || entries.contains_key(&id) {
                return Err("agent keepalive admission capacity or duplicate".into());
            }
            entries.insert(id, prepared);
        }
        Ok(())
    }
    fn execute(
        &self,
        id: HostOperationId,
        request: PlatformHostRequest,
        cancel: Arc<AtomicBool>,
    ) -> Result<PlatformHostResult, String> {
        let retained = self
            .prepared_keepalive
            .lock()
            .map_err(|_| "agent keepalive custody poisoned")?
            .contains_key(&id);
        self.check_http(&cancel, retained)?;
        let read_stream = if let PlatformHostRequest::HttpRead { stream_id, .. } = &request {
            Some(*stream_id)
        } else {
            None
        };
        let used_socket = match &request {
            PlatformHostRequest::WebSocketSend { socket_id, .. }
            | PlatformHostRequest::WebSocketPoll { socket_id, .. } => Some(*socket_id),
            _ => None,
        };
        let result = match request {
            PlatformHostRequest::NavigationPreload { task } => {
                self.navigation_preload(id, task, &cancel)
            }
            PlatformHostRequest::HttpStart { request } => {
                self.http_start(id, request, false, true, &cancel)
            }
            PlatformHostRequest::ImageStart {
                request,
                cors_enabled,
            } => self.http_start(id, request, true, cors_enabled, &cancel),
            PlatformHostRequest::XsltTransform { .. }
            | PlatformHostRequest::XsltStart { .. }
            | PlatformHostRequest::PrefetchStart { .. }
            | PlatformHostRequest::ImagePreloadStart { .. }
            | PlatformHostRequest::MediaStart { .. } => {
                Err("SecurityError: workers cannot create object element resources".into())
            }
            PlatformHostRequest::HttpRead {
                stream_id,
                max_bytes,
            } => self.http_read(id, stream_id, max_bytes, &cancel),
            PlatformHostRequest::HttpAbort { stream_id } => {
                self.abort_stream(stream_id);
                Ok(PlatformHostResult::HttpAborted)
            }
            PlatformHostRequest::WebSocketOpen { url, protocols } => {
                self.socket_open(id, &url, &protocols, &cancel)
            }
            request @ (PlatformHostRequest::WebSocketSend { .. }
            | PlatformHostRequest::WebSocketPoll { .. }
            | PlatformHostRequest::WebSocketClose { .. }) => self.socket_request(request, &cancel),
            PlatformHostRequest::IndexedDb { command, .. } => {
                let _serial = self
                    .idb
                    .lock()
                    .map_err(|_| "agent IndexedDB queue poisoned")?;
                self.check(&cancel)?;
                let closing_connection = match &command {
                    IdbHostCommand::CloseConnection { token, .. }
                    | IdbHostCommand::RetireConnection { token, .. }
                    | IdbHostCommand::DeleteConnection { token, .. } => Some(token.clone()),
                    _ => None,
                };
                match &command {
                    IdbHostCommand::OpenConnection { token, name, .. }
                    | IdbHostCommand::DeleteConnection { token, name } => {
                        let mut entries = self
                            .idb_connections
                            .lock()
                            .map_err(|_| "agent IDB connection custody poisoned")?;
                        if !entries.contains_key(token) && entries.len() >= MAX_AGENT_RESOURCES {
                            return Err("QuotaExceededError: agent IDB connection capacity".into());
                        }
                        if entries.get(token).is_some_and(|old| old != name) {
                            return Err("SecurityError: IDB connection database changed".into());
                        }
                        entries.insert(token.clone(), name.clone());
                    }
                    IdbHostCommand::UpgradeConnection { token, name, .. }
                    | IdbHostCommand::PollConnection { token, name }
                    | IdbHostCommand::CloseConnection { token, name }
                    | IdbHostCommand::RetireConnection { token, name }
                        if self
                            .idb_connections
                            .lock()
                            .map_err(|_| "agent IDB connection custody poisoned")?
                            .get(token)
                            != Some(name) =>
                    {
                        return Err("SecurityError: foreign agent IDB connection".into());
                    }
                    _ => {}
                }
                let ending = match &command {
                    IdbHostCommand::CommitTransaction { token, .. }
                    | IdbHostCommand::AbortTransaction { token, .. } => Some(token.clone()),
                    _ => None,
                };
                if let IdbHostCommand::BeginTransaction {
                    token, database, ..
                }
                | IdbHostCommand::Transaction {
                    token, database, ..
                } = &command
                {
                    let mut transactions = self
                        .transactions
                        .lock()
                        .map_err(|_| "agent transactions poisoned")?;
                    if !transactions.contains_key(token)
                        && transactions.len() >= MAX_AGENT_RESOURCES
                    {
                        return Err("QuotaExceededError: agent transaction limit".into());
                    }
                    if transactions
                        .get(token)
                        .is_some_and(|existing| existing != database)
                    {
                        return Err("InvalidStateError: transaction database changed".into());
                    }
                    transactions.insert(token.clone(), database.clone());
                }
                let result = self
                    .context
                    .upgrade()
                    .ok_or("agent context retired")?
                    .agent_indexed_db(self.bootstrap.id, command)
                    .map(|response| PlatformHostResult::IndexedDb { response })
                    .map_err(|error| error.to_string());
                if matches!(&result,Ok(PlatformHostResult::IndexedDb{response})if !response.transaction_blocked)
                    && let Some(token) = closing_connection
                {
                    self.idb_connections
                        .lock()
                        .map_err(|_| "agent IDB connection custody poisoned")?
                        .remove(&token);
                }
                if matches!(&result,Ok(PlatformHostResult::IndexedDb{response})if !response.transaction_blocked)
                    && let Some(token) = ending
                {
                    self.transactions
                        .lock()
                        .map_err(|_| "agent transactions poisoned")?
                        .remove(&token);
                }
                result
            }
        };
        self.connecting
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        if result.is_err() {
            if let Some(stream) = read_stream {
                self.abort_stream(stream);
            }
            if let Some(socket) = used_socket {
                self.close_socket(socket);
            }
        }
        result
    }
    fn cancel(&self, id: HostOperationId) {
        self.prepared_keepalive
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        if let Some(stream) = self
            .connecting
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    fn dispose(&self, result: PlatformHostResult) {
        match result {
            PlatformHostResult::HttpStarted { stream_id, .. } => self.orphan_stream(stream_id),
            PlatformHostResult::WebSocketOpened { socket_id, .. } => self.close_socket(socket_id),
            _ => {}
        }
    }
    fn dispose_canceled(&self, result: PlatformHostResult) {
        match result {
            PlatformHostResult::HttpStarted { stream_id, .. } => self.abort_stream(stream_id),
            other => self.dispose(other),
        }
    }
    fn finish_retirement(&self) -> Result<(), String> {
        // One failed scope must not starve independent owners of cleanup.
        let mut errors = Vec::new();
        if let Err(error) = self.cleanup.drain(Duration::from_secs(2)) {
            errors.push(error.to_string());
        }
        if self
            .context
            .upgrade()
            .is_none_or(|context| !context.keepalive_context_active())
        {
            self.keepalive_cleanup.cancel(None);
            if let Err(error) = self.keepalive_cleanup.drain(Duration::from_secs(2)) {
                errors.push(error.to_string());
            }
        }
        if let Err(error) = self.network.drain_owned_operations(Duration::from_secs(2)) {
            errors.push(error.to_string());
        }
        let owners = self.preload_owners.lock().map(|owners| owners.clone());
        match owners {
            Ok(owners) => {
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut failed = Vec::new();
                for owner in owners {
                    if let Err(error) = owner
                        .drain_owned_operations(deadline.saturating_duration_since(Instant::now()))
                    {
                        errors.push(error.to_string());
                        failed.push(owner);
                    }
                }
                match self.preload_owners.lock() {
                    Ok(mut owners) => *owners = failed,
                    Err(_) => errors.push("preload cancellation owners poisoned".into()),
                }
            }
            Err(_) => errors.push("preload cancellation owners poisoned".into()),
        }
        // The operation owner invokes this only after joining every I/O task.
        // Retain each failed abort and continue with all other exact owners.
        if let Some(store) = self.storage.as_ref() {
            let origin = crate::storage::partitioned_idb_origin(
                &self.bootstrap.partition,
                &self.bootstrap.origin.authority,
            );
            let principal = format!("agent:{}", self.bootstrap.id.0);
            errors.extend(drain_idb_ledger(&self.transactions, |token, database| {
                let mut command = crate::storage::IdbCommand::AbortTransaction {
                    token: token.into(),
                    database: database.into(),
                };
                crate::storage::scope_idb_transaction(&mut command, &principal);
                store
                    .execute(crate::storage::StorageRequest::IndexedDb {
                        origin: origin.clone(),
                        command,
                    })
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }));
            errors.extend(drain_idb_ledger(&self.idb_connections, |token, name| {
                let mut command = crate::storage::IdbCommand::RetireConnection {
                    token: token.into(),
                    name: name.into(),
                };
                crate::storage::scope_idb_transaction(&mut command, &principal);
                store
                    .execute(crate::storage::StorageRequest::IndexedDb {
                        origin: origin.clone(),
                        command,
                    })
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }));
        } else {
            errors.push("agent retained storage owner unavailable".into());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.cleanup.cancel(None);
        if self
            .context
            .upgrade()
            .is_none_or(|context| !context.keepalive_context_active())
        {
            self.keepalive_cleanup.cancel(None);
            self.prepared_keepalive
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
        }
        let streams = std::mem::take(
            &mut *self
                .streams
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        for stream in streams.into_values() {
            self.dispose_owned_stream(stream);
        }
        for stream in std::mem::take(
            &mut *self
                .connecting
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
        .into_values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        for stream in std::mem::take(
            &mut *self
                .socket_cancels
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
        .into_values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

#[cfg(all(test, unix))]
mod live_redirect_authority_regressions {
    use super::*;
    use crate::agent_registry::AgentProcess;
    use crate::network::{NetworkError, NetworkResponse};
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    struct Process;
    impl AgentProcess for Process {
        fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
            Ok(())
        }
        fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
            Ok(Vec::new())
        }
        fn terminate(&mut self) -> Result<(), WorkerError> {
            Ok(())
        }
    }
    struct Redirects(Arc<Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Redirects {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let mut seen = self.0.lock().unwrap();
            seen.push(request.clone());
            let mut headers = vec![
                (
                    "access-control-allow-origin".into(),
                    request
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("origin"))
                        .map(|(_, value)| value.clone())
                        .unwrap_or_else(|| "https://a.example".into()),
                ),
                ("access-control-allow-credentials".into(), "true".into()),
            ];
            match seen.len() {
                1 => headers.push(("location".into(), "https://b.example/hop".into())),
                2 => headers.push(("location".into(), "https://a.example/end".into())),
                _ => headers.push(("content-type".into(), "text/plain".into())),
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: if seen.len() < 3 { 302 } else { 200 },
                headers,
                body: Vec::new(),
            })
        }
    }
    #[test]
    fn registered_agent_redirects_preserve_native_cookie_ancestry_and_reject_retired_owner() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://a.example/".into())
            .unwrap();
        context
            .install_document_security(id, "https://a.example/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (owner, partition, origin) = context
            .window_worker_creator(id, &WindowTarget::Root, 1, 0)
            .unwrap();
        let ancestry = vec![
            "https://a.example/".into(),
            "https://cross.example/frame".into(),
            "https://a.example/frame".into(),
        ];
        let bootstrap = AgentBootstrap {
            cookie_ancestor_urls: ancestry.clone(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: AgentPolicy {
                hide_nonces: false,
                policy_sources: Vec::new(),
                policy_self_urls: Vec::new(),
                document_self_url: None,
                policy_self_url: None,
                url: "https://a.example/worker.js".into(),
                headers: Vec::new(),
            },
            top_level_url: Some("https://a.example/".into()),
            id: AgentId(0),
            kind: AgentKind::Dedicated,
            partition,
            origin,
            script_url: "https://a.example/worker.js".into(),
            source: Vec::new(),
            modules: Default::default(),
            options: Default::default(),
            referrer_policy: "no-referrer".into(),
            limits: Default::default(),
        };
        let connection = context
            .dispatch_worker_registry(|registry, hub| {
                registry.connect_with_ports(hub, owner, bootstrap, |_| Ok(Box::new(Process)))
            })
            .unwrap();
        let bootstrap = context.worker_bootstrap(connection.agent).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let io = AgentIo::new(
            context.downgrade(),
            bootstrap,
            SharedNetworkBroker::new(Redirects(seen.clone())),
        );
        let request = || PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: None,
            use_cors_preflight: false,
            method: "GET".into(),
            url: "https://a.example/start".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: "include".into(),
            initiator_url: Some("https://forged.example/".into()),
            referrer_policy: None,
        };
        let response = io
            .http_start(
                HostOperationId(1),
                request(),
                false,
                true,
                &Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        io.dispose(response);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 3);
            let origins = seen
                .iter()
                .map(|request| {
                    request
                        .headers
                        .iter()
                        .find(|(name, _)| name == "origin")
                        .map(|(_, value)| value.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(origins, vec![None, Some("https://a.example"), Some("null")]);
            assert!(!seen[0].security.redirect_chain_cross_site);
            assert!(seen[1].security.redirect_chain_cross_site);
            assert!(seen[2].security.redirect_chain_cross_site);
            for request in seen.iter() {
                assert_eq!(request.security.ancestor_urls, ancestry);
                assert_eq!(
                    request.security.initiator_url.as_deref(),
                    Some("https://a.example/worker.js")
                );
                assert_eq!(
                    request.security.top_level_url.as_deref(),
                    Some("https://a.example/")
                );
            }
        }
        struct FetchReplies(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for FetchReplies {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let redirect = request.url.ends_with("/start");
                let mut headers = vec![("access-control-allow-origin".into(), "*".into())];
                if redirect {
                    headers.push(("location".into(), "https://b.example/end".into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers,
                    body: b"hello".to_vec(),
                })
            }
        }
        let good = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        for (mode, redirect, integrity, url, success, hops, filter) in [
            (
                "cors",
                "follow",
                good,
                "https://a.example/start",
                true,
                2,
                None,
            ),
            (
                "cors",
                "follow",
                "sha256-AA==",
                "https://a.example/start",
                false,
                2,
                None,
            ),
            (
                "cors",
                "error",
                "",
                "https://a.example/start",
                false,
                1,
                None,
            ),
            (
                "cors",
                "manual",
                "",
                "https://a.example/start",
                true,
                1,
                Some(FetchResponseType::OpaqueRedirect),
            ),
            (
                "same-origin",
                "follow",
                "",
                "https://a.example/start",
                false,
                1,
                None,
            ),
            (
                "no-cors",
                "follow",
                "",
                "https://b.example/end",
                true,
                1,
                Some(FetchResponseType::Opaque),
            ),
            (
                "no-cors",
                "follow",
                good,
                "https://b.example/end",
                false,
                1,
                None,
            ),
            (
                "cors",
                "follow",
                good,
                "data:text/plain,hello",
                true,
                0,
                None,
            ),
            (
                "cors",
                "follow",
                "sha256-AA==",
                "data:text/plain,hello",
                false,
                0,
                None,
            ),
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let fetch_io = AgentIo::new(
                context.downgrade(),
                io.bootstrap.clone(),
                SharedNetworkBroker::new(FetchReplies(requests.clone())),
            );
            let mut request = request();
            request.credentials = "omit".into();
            request.url = url.into();
            request.fetch = Some(FetchOptions {
                keepalive: false,
                privileged_range: false,
                cache: Default::default(),
                mode: mode.into(),
                redirect: redirect.into(),
                integrity: integrity.into(),
            });
            let result = fetch_io.http_start(
                HostOperationId(3),
                request,
                false,
                true,
                &Arc::new(AtomicBool::new(false)),
            );
            assert_eq!(
                result.is_ok(),
                success,
                "{mode}/{redirect}/{url}/{integrity}: {result:?}"
            );
            assert_eq!(requests.lock().unwrap().len(), hops);
            if let Ok(result) = result {
                if let PlatformHostResult::HttpStarted {
                    internal_response,
                    stream_id,
                    response_type,
                    status,
                    url,
                    headers,
                    ..
                } = &result
                {
                    assert_eq!(*response_type, filter);
                    if filter.is_some() {
                        assert_eq!(*status, 0);
                        assert!(url.is_empty());
                        assert!(headers.is_empty());
                        assert_eq!(
                            internal_response.as_ref().unwrap().status,
                            if redirect == "manual" { 302 } else { 200 }
                        );
                        let PlatformHostResult::HttpChunk { bytes, .. } = fetch_io
                            .http_read(
                                HostOperationId(4),
                                *stream_id,
                                4096,
                                &AtomicBool::new(false),
                            )
                            .unwrap()
                        else {
                            panic!("private body")
                        };
                        assert_eq!(bytes, b"hello");
                    }
                } else {
                    panic!("fetch result");
                }
                fetch_io.dispose(result);
            }
        }
        struct RejectedHeaders {
            ack: Arc<AtomicBool>,
        }
        impl NetworkBroker for RejectedHeaders {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: vec![1; 4096],
                })
            }
            fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                if self.ack.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(NetworkError::Transport(
                        "retained cancellation fixture".into(),
                    ))
                }
            }
        }
        let ack = Arc::new(AtomicBool::new(false));
        let rejected = AgentIo::new(
            context.downgrade(),
            io.bootstrap.clone(),
            SharedNetworkBroker::new(RejectedHeaders { ack: ack.clone() }),
        );
        let mut cross = request();
        cross.url = "https://b.example/rejected".into();
        cross.credentials = "omit".into();
        assert!(
            rejected
                .http_start(
                    HostOperationId(5),
                    cross,
                    false,
                    true,
                    &Arc::new(AtomicBool::new(false))
                )
                .is_err()
        );
        assert!(
            rejected.streams.lock().unwrap().is_empty(),
            "CORS-rejected response is never published"
        );
        assert!(
            !rejected.cleanup.is_drained().unwrap(),
            "pre-publication body retains cancellation receipt"
        );
        ack.store(true, Ordering::Release);
        rejected.cleanup.drain(Duration::from_secs(3)).unwrap();
        assert!(rejected.cleanup.is_drained().unwrap());
        let keepalive_seen = Arc::new(Mutex::new(Vec::new()));
        let keepalive_io = AgentIo::new(
            context.downgrade(),
            io.bootstrap.clone(),
            SharedNetworkBroker::new(FetchReplies(keepalive_seen.clone())),
        );
        let mut retained = request();
        retained.url = "https://a.example/end".into();
        retained.method = "POST".into();
        retained.body = vec![1; 32];
        retained.fetch = Some(FetchOptions {
            keepalive: true,
            privileged_range: false,
            cache: Default::default(),
            mode: "cors".into(),
            redirect: "follow".into(),
            integrity: String::new(),
        });
        keepalive_io
            .prepare(
                HostOperationId(60),
                &PlatformHostRequest::HttpStart {
                    request: retained.clone(),
                },
            )
            .unwrap();
        keepalive_io
            .prepare(
                HostOperationId(61),
                &PlatformHostRequest::HttpStart {
                    request: retained.clone(),
                },
            )
            .unwrap();
        keepalive_io.cancel(HostOperationId(61));
        keepalive_io.retire();
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            io.http_start(
                HostOperationId(2),
                request(),
                false,
                true,
                &Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert!(
            keepalive_io
                .execute(
                    HostOperationId(61),
                    PlatformHostRequest::HttpStart {
                        request: retained.clone()
                    },
                    Arc::new(AtomicBool::new(false))
                )
                .is_err(),
            "canceled prepared admission cannot reopen retired authority"
        );
        let result = keepalive_io
            .execute(
                HostOperationId(60),
                PlatformHostRequest::HttpStart { request: retained },
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        keepalive_io.dispose(result);
        keepalive_io
            .keepalive_cleanup
            .drain(Duration::from_secs(3))
            .unwrap();
        assert_eq!(keepalive_seen.lock().unwrap().len(), 1);
        assert!(keepalive_io.streams.lock().unwrap().is_empty());
        assert!(
            context
                .reserve_keepalive(
                    &crate::agent_registry::AgentOwner::Agent(io.bootstrap.id),
                    64 * 1024
                )
                .is_ok(),
            "orphan EOF released the original agent body budget"
        );
    }
}

#[cfg(test)]
mod retirement_fanout_regressions {
    use super::*;
    #[test]
    fn failed_cleanup_retains_only_failed_owner_and_never_holds_ledger_during_release() {
        let ledger = Mutex::new(BTreeMap::from([
            ("first".into(), "db1".into()),
            ("second".into(), "db2".into()),
        ]));
        let mut seen = Vec::new();
        let errors = drain_idb_ledger(&ledger, |token, database| {
            assert!(
                ledger.try_lock().is_ok(),
                "persistence must not run under the ledger lock"
            );
            seen.push((token.to_string(), database.to_string()));
            if token == "first" {
                Err("injected abort failure".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(seen.len(), 2);
        assert_eq!(errors, vec!["injected abort failure"]);
        assert_eq!(
            *ledger.lock().unwrap(),
            BTreeMap::from([("first".into(), "db1".into())])
        );
        assert!(drain_idb_ledger(&ledger, |_, _| Ok(())).is_empty());
        assert!(ledger.lock().unwrap().is_empty());
    }
    #[test]
    fn cleanup_ack_does_not_remove_changed_ledger_identity() {
        let ledger = Mutex::new(BTreeMap::from([("original".into(), "old-db".into())]));
        assert!(
            drain_idb_ledger(&ledger, |token, _| {
                ledger
                    .lock()
                    .unwrap()
                    .insert(token.into(), "replacement-db".into());
                Ok(())
            })
            .is_empty()
        );
        assert_eq!(
            ledger.lock().unwrap().get("original").map(String::as_str),
            Some("replacement-db")
        );
    }
}
