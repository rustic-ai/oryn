// Modern Window Fetch adapter. No synthetic parent page or legacy fetch binding.
use super::*;
use crate::network::{NetworkBroker, NetworkEvent, NetworkOperation, NetworkRequest};
use oryn_js::platform::*;
use std::cell::{Cell, RefCell};
const LIMIT: usize = 64;
struct Frozen {
    source: Url,
    base: Url,
    origin: String,
    top: String,
    ancestors: Vec<String>,
    policy: crate::csp::CspPolicySet,
    referrer: String,
    blob: Option<(Vec<u8>, String)>,
    lease: Option<Arc<crate::keepalive::Lease>>,
    drain: Option<crate::keepalive_drain::Ticket>,
    reports: Arc<crate::executable::module_graph_load::ScriptCspReports>,
}
struct Pending {
    cancel: Arc<AtomicBool>,
    keepalive: bool,
    stream: Option<u64>,
}
struct Body {
    operation: NetworkOperation,
    current: bytes::Bytes,
    read: usize,
    done: bool,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    cleanup: Option<crate::http_cleanup_custody::Ticket>,
}
struct OwnedBody {
    body: Mutex<Body>,
    cancel: Arc<AtomicBool>,
    drain: Option<crate::keepalive_drain::Ticket>,
}
struct Engine {
    network: SharedNetworkBroker,
    retired: AtomicBool,
    next: std::sync::atomic::AtomicU64,
    cleanup: crate::http_cleanup_custody::Scope,
    streams: Mutex<BTreeMap<u64, Arc<OwnedBody>>>,
}
type CaptureHttpRequest = dyn Fn(&PlatformHttpRequest) -> Result<Frozen, String>;

pub(super) struct Host {
    engine: Arc<Engine>,
    capture: Box<CaptureHttpRequest>,
    retired: Cell<bool>,
    pending: RefCell<BTreeMap<HostOperationId, Pending>>,
    receipts: RefCell<BTreeMap<HostOperationId, u64>>,
    sender: mpsc::Sender<PlatformHostCompletion>,
    receiver: RefCell<mpsc::Receiver<PlatformHostCompletion>>,
}
struct Installed {
    state: std::rc::Weak<V8BindingState>,
    page: oryn_common::v3::PageId,
    creator: oryn_js::worker_wire::Creator,
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), HostError> {
    let fetch = current_fetch_endpoint(scope)
        .ok_or_else(|| HostError::Execution("standalone Fetch endpoint unavailable".into()))?;
    let (owner, authority, _) = capture_standalone_deferred(scope, state)
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let context = scope.get_current_context();
    if context.get_slot::<Installed>().is_some_and(|installed| {
        installed.page == authority.page
            && installed.creator == authority.creator
            && installed
                .state
                .upgrade()
                .is_some_and(|previous| Rc::ptr_eq(&previous, state))
    }) {
        return Ok(());
    }
    context.set_slot(Rc::new(Installed {
        state: Rc::downgrade(state),
        page: authority.page,
        creator: authority.creator.clone(),
    }));
    let reports = native_script_report_queue(scope);
    let weak = Rc::downgrade(state);
    let creator = authority.creator;
    let page = authority.page;
    let origin = authority.origin;
    let top = authority.top;
    let ancestors = authority.ancestors;
    let fallback_policy = authority.policy;
    let capture = Box::new(
        move |request: &PlatformHttpRequest| -> Result<Frozen, String> {
            let state = weak
                .upgrade()
                .filter(|state| !state.execution_retired() && !state.execution_suspended())
                .ok_or("standalone Fetch document retired")?;
            let source = Url::parse(
                &state
                    .request_initiator_url()
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let base = Url::parse(
                &state
                    .native_document_base_url(true)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let policy = match state
                .capture_local_script_policy()
                .map_err(|error| error.to_string())?
            {
                Some(snapshot) => snapshot
                    .native::<crate::csp::CspPolicySet>()
                    .cloned()
                    .ok_or("standalone Fetch native policy absent")?,
                None => fallback_policy.clone(),
            };
            let oryn_js::worker_wire::Creator::Window { target, .. } = &creator else {
                return Err("standalone Fetch requires Window authority".into());
            };
            let creator = oryn_js::worker_wire::Creator::Window {
                target: target.clone(),
                document_generation: state.document_generation(),
                stream_epoch: state.document_authority_epoch(),
            };
            let keepalive = request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive);
            let drain = keepalive
                .then(crate::keepalive_drain::Ticket::reserve)
                .transpose()?;
            let lease = if keepalive {
                Some(Arc::new(owner.reserve_keepalive(
                    page,
                    &creator,
                    request.body.len(),
                )?))
            } else {
                None
            };
            let target = base.join(&request.url).map_err(|error| error.to_string())?;
            let blob = if target.scheme() == "blob" {
                let blob = state
                    .local_blob_resource(target.as_str())
                    .ok_or("standalone blob revoked or foreign")?;
                if blob.0.len() > 16 * 1024 * 1024 {
                    return Err("standalone blob body limit".into());
                }
                Some(blob)
            } else {
                None
            };
            Ok(Frozen {
                source,
                base,
                origin: origin.clone(),
                top: top.clone(),
                ancestors: ancestors.clone(),
                policy,
                referrer: state.document_referrer_policy(),
                blob,
                lease,
                drain,
                reports: reports.clone(),
            })
        },
    );
    let (sender, receiver) = mpsc::channel();
    state.set_platform_host(Rc::new(Host {
        engine: Arc::new(Engine {
            network: fetch.broker.new_request_scope(),
            retired: AtomicBool::new(false),
            next: std::sync::atomic::AtomicU64::new(1),
            cleanup: Default::default(),
            streams: Default::default(),
        }),
        capture,
        retired: Cell::new(false),
        pending: Default::default(),
        receipts: Default::default(),
        sender,
        receiver: RefCell::new(receiver),
    }));
    Ok(())
}
impl PlatformHost for Host {
    fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
        if self.retired.get() {
            return Err("standalone Fetch endpoint retired".into());
        }
        if self.pending.borrow().len() + self.receipts.borrow().len() >= LIMIT
            || self.pending.borrow().contains_key(&id)
            || self.receipts.borrow().contains_key(&id)
        {
            return Err("standalone Fetch operation capacity or identity".into());
        }
        let frozen = match &request {
            PlatformHostRequest::HttpStart { request }
            | PlatformHostRequest::ImageStart { request, .. }
            | PlatformHostRequest::ImagePreloadStart { request, .. }
            | PlatformHostRequest::PrefetchStart { request }
            | PlatformHostRequest::MediaStart { request, .. } => Some((self.capture)(request)?),
            PlatformHostRequest::HttpRead { .. } | PlatformHostRequest::HttpAbort { .. } => None,
            _ => return Err("standalone HTTP adapter request category unavailable".into()),
        };
        let keepalive = frozen.as_ref().is_some_and(|frozen| frozen.lease.is_some());
        if keepalive && !matches!(&request, PlatformHostRequest::HttpStart { .. }) {
            return Err("keepalive requires ordinary Fetch".into());
        }
        let stream = match &request {
            PlatformHostRequest::HttpRead { stream_id, .. }
            | PlatformHostRequest::HttpAbort { stream_id } => Some(*stream_id),
            _ => None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.pending.borrow_mut().insert(
            id,
            Pending {
                cancel: cancel.clone(),
                keepalive,
                stream,
            },
        );
        let engine = self.engine.clone();
        let sender = self.sender.clone();
        let accepted = crate::resource_io_executor::submit(move || {
            let result = match request {
                PlatformHostRequest::HttpStart { request } => {
                    engine.start(id, request, "", true, frozen.unwrap(), &cancel)
                }
                PlatformHostRequest::PrefetchStart { request } => {
                    engine.start(id, request, "prefetch", false, frozen.unwrap(), &cancel)
                }
                PlatformHostRequest::ImageStart {
                    request,
                    cors_enabled,
                }
                | PlatformHostRequest::ImagePreloadStart {
                    request,
                    cors_enabled,
                } => engine.start(id, request, "image", cors_enabled, frozen.unwrap(), &cancel),
                PlatformHostRequest::MediaStart {
                    request,
                    cors_enabled,
                    destination,
                } if matches!(destination.as_str(), "audio" | "video" | "track") => engine.start(
                    id,
                    request,
                    &destination,
                    cors_enabled,
                    frozen.unwrap(),
                    &cancel,
                ),
                PlatformHostRequest::HttpRead {
                    stream_id,
                    max_bytes,
                } => engine.read(stream_id, max_bytes, &cancel),
                PlatformHostRequest::HttpAbort { stream_id } => {
                    engine.abort(stream_id);
                    Ok(PlatformHostResult::HttpAborted)
                }
                _ => Err("invalid standalone HTTP request".into()),
            };
            let stream = match &result {
                Ok(PlatformHostResult::HttpStarted { stream_id, .. }) => Some(*stream_id),
                _ => None,
            };
            if engine.retired.load(Ordering::Acquire) {
                if let Some(stream) = stream {
                    engine.abandon(stream)
                }
                return;
            }
            if sender
                .send(PlatformHostCompletion {
                    operation_id: id,
                    result,
                })
                .is_err()
                && let Some(stream) = stream
            {
                engine.abandon(stream)
            }
        });
        if let Err(error) = accepted {
            self.pending.borrow_mut().remove(&id);
            return Err(error);
        }
        Ok(())
    }
    fn cancel(&self, id: HostOperationId) {
        if let Some(pending) = self.pending.borrow().get(&id) {
            pending.cancel.store(true, Ordering::Release);
            if let Some(stream) = pending.stream {
                self.engine.abort(stream)
            }
            self.engine.cleanup.cancel(Some(id.0));
        }
    }
    fn abandon_http_stream(&self, id: u64) {
        self.engine.abandon(id);
    }
    fn retire(&self) {
        if self.retired.replace(true) {
            return;
        }
        self.engine.retired.store(true, Ordering::Release);
        for pending in self.pending.borrow().values() {
            if !pending.keepalive {
                pending.cancel.store(true, Ordering::Release);
            }
        }
        self.engine.cleanup.retire_preserving_keepalive();
        self.pending.borrow_mut().clear();
        self.receipts.borrow_mut().clear();
        let streams = self
            .engine
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for stream in streams {
            self.engine.abandon(stream)
        }
    }
    fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
        if let Some(stream) = self.receipts.borrow_mut().remove(&id)
            && !accepted
        {
            self.engine.abandon(stream)
        }
    }
    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
        let mut output = Vec::new();
        for _ in 0..limit.min(LIMIT) {
            let Ok(completion) = self.receiver.borrow_mut().try_recv() else {
                break;
            };
            if self
                .pending
                .borrow_mut()
                .remove(&completion.operation_id)
                .is_none()
            {
                if let Ok(PlatformHostResult::HttpStarted { stream_id, .. }) = &completion.result {
                    self.engine.abandon(*stream_id)
                }
                continue;
            }
            if let Ok(PlatformHostResult::HttpStarted { stream_id, .. }) = &completion.result {
                self.receipts
                    .borrow_mut()
                    .insert(completion.operation_id, *stream_id);
            }
            output.push(completion);
        }
        output
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.retire();
    }
}
impl Engine {
    fn id(&self) -> Result<u64, String> {
        self.next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "standalone Fetch identity exhausted".into())
    }
    fn check(&self, cancel: &AtomicBool, keepalive: bool, deadline: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Acquire) || !keepalive && self.retired.load(Ordering::Acquire) {
            Err("AbortError: standalone Fetch cancelled".into())
        } else if Instant::now() >= deadline {
            Err("TimeoutError: standalone Fetch deadline".into())
        } else {
            Ok(())
        }
    }
    fn event(
        &self,
        operation: &mut NetworkOperation,
        cancel: &AtomicBool,
        keepalive: bool,
        deadline: Instant,
        external: Option<&AtomicBool>,
    ) -> Result<NetworkEvent, String> {
        loop {
            self.check(cancel, keepalive, deadline)?;
            if external.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
                return Err("AbortError: Fetch body cancelled".into());
            }
            match operation.try_event() {
                Ok(event) => return Ok(event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    std::thread::park_timeout(Duration::from_millis(1))
                }
                Err(_) => {
                    return Err(
                        "standalone Fetch disconnected before terminal acknowledgement".into(),
                    );
                }
            }
        }
    }
    fn ticket(
        &self,
        id: HostOperationId,
        request_id: u64,
        frozen: &Frozen,
    ) -> Result<crate::http_cleanup_custody::Ticket, String> {
        let ticket = if frozen.lease.is_some() {
            crate::http_cleanup_custody::Ticket::reserve_keepalive(
                &self.cleanup,
                id.0,
                self.network.clone(),
            )
        } else {
            crate::http_cleanup_custody::Ticket::reserve(&self.cleanup, id.0, self.network.clone())
        }
        .map_err(|error| error.to_string())?;
        if let Some(lease) = &frozen.lease {
            ticket.retain_keepalive(lease.clone());
        }
        ticket.attach(request_id);
        Ok(ticket)
    }
    fn start(
        &self,
        id: HostOperationId,
        mut request: PlatformHttpRequest,
        destination: &str,
        default_cors: bool,
        mut frozen: Frozen,
        cancel: &Arc<AtomicBool>,
    ) -> Result<PlatformHostResult, String> {
        use oryn_net::cors::{
            cors_exposed_response_headers, cors_headers_allowed, cors_non_safelisted_headers,
            cors_preflight_required,
        };
        let keepalive = frozen.lease.is_some();
        let deadline = Instant::now() + Duration::from_secs(30);
        self.check(cancel, keepalive, deadline)?;
        if request.body.len() > 16 * 1024 * 1024 {
            return Err("Fetch upload body limit".into());
        }
        if let Some(fetch) = &request.fetch {
            fetch.validate()?;
        }
        if destination == "prefetch"
            && (request.method != "GET"
                || !request.body.is_empty()
                || request.fetch.as_ref().is_some_and(|fetch| {
                    fetch.mode != "no-cors" || fetch.redirect != "follow" || fetch.keepalive
                }))
        {
            return Err("invalid native prefetch request".into());
        }
        if request.upgrade_mixed_image && destination != "image" {
            return Err("invalid image upgrade authority".into());
        }
        let cors = request.fetch.as_ref().map_or(default_cors, |fetch| {
            matches!(fetch.mode.as_str(), "cors" | "same-origin")
        });
        let mode = request
            .fetch
            .as_ref()
            .map(|fetch| fetch.mode.as_str())
            .unwrap_or(if cors { "cors" } else { "no-cors" });
        if !matches!(
            request.credentials.as_str(),
            "omit" | "same-origin" | "include"
        ) {
            return Err("invalid Fetch credentials".into());
        }
        for (name, value) in &request.headers {
            tungstenite::http::HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| error.to_string())?;
            tungstenite::http::HeaderValue::from_str(value).map_err(|error| error.to_string())?;
        }
        request.headers = crate::network::sanitize_worker_headers(request.headers);
        let mut method = tungstenite::http::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| error.to_string())?
            .to_string();
        if matches!(
            method.to_ascii_uppercase().as_str(),
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
        ) {
            method = method.to_ascii_uppercase();
        }
        if matches!(
            method.to_ascii_uppercase().as_str(),
            "CONNECT" | "TRACE" | "TRACK"
        ) {
            return Err("forbidden Fetch method".into());
        }
        if matches!(method.as_str(), "GET" | "HEAD") && !request.body.is_empty() {
            return Err("GET/HEAD Fetch body".into());
        }
        if mode == "no-cors" {
            if !matches!(method.as_str(), "GET" | "HEAD" | "POST") {
                return Err("no-cors method is not safelisted".into());
            }
            request.headers.retain(|(name, value)| {
                oryn_common::http_headers::no_cors_request_header(name, value)
                    || (method == "GET"
                        && request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.privileged_range)
                        && name.eq_ignore_ascii_case("range")
                        && oryn_common::http_headers::safelisted_request_header(name, value))
            });
        }
        let policy_headers = frozen.policy.worker_snapshot().headers;
        let embedder =
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy_headers);
        let coep = crate::coep_reporting::Policy::new(frozen.source.as_str(), &policy_headers);
        let mut target = frozen
            .base
            .join(&request.url)
            .map_err(|error| error.to_string())?;
        let original = target.to_string();
        let mut origin = frozen.origin.clone();
        let opaque = frozen.origin == "null";
        let mut body = request.body;
        let mut referrer = crate::referrer::ReferrerPolicy::parse_list(
            request
                .referrer_policy
                .as_deref()
                .unwrap_or(&frozen.referrer),
        );
        let mut referrer_source = match request.initiator_url.as_deref() {
            Some("") => None,
            Some(value) => Url::parse(value)
                .ok()
                .filter(|value| !opaque && value.origin().ascii_serialization() == frozen.origin),
            None => Some(frozen.source.clone()),
        };
        let initial_site = crate::cookies::SchemefulSite::from_url(&target);
        let mut cross_site = false;
        let mut metadata = crate::fetch_metadata::Metadata::resource(
            mode,
            if destination == "prefetch" {
                ""
            } else {
                destination
            },
            opaque,
        );
        let started = Instant::now();
        let mut hops = Vec::new();
        let mut upload = None;
        for redirects in 0..=20 {
            self.check(cancel, keepalive, deadline)?;
            if let Some(upgraded) = frozen.policy.upgraded_url(&target) {
                target = upgraded
            }
            if request.upgrade_mixed_image
                && target.scheme() == "http"
                && matches!(target.host(), Some(url::Host::Domain(_)))
                && crate::mixed_content::enforce(&frozen.origin, &target).is_err()
            {
                let _ = target.set_scheme("https");
            }
            let csp_destination = match destination {
                "prefetch" => crate::csp::CspDestination::Prefetch,
                "image" => crate::csp::CspDestination::Image,
                "audio" | "video" | "track" => crate::csp::CspDestination::Media,
                _ => crate::csp::CspDestination::Connect,
            };
            let decision = frozen.policy.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination: csp_destination,
                    url: target.to_string(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                },
                redirects != 0,
                &original,
            );
            frozen
                .reports
                .push_with_policy(&frozen.policy, decision.violations)?;
            if !decision.allowed {
                return Err("Fetch blocked by Content-Security-Policy".into());
            }
            crate::mixed_content::enforce(
                if frozen.source.scheme() == "about" {
                    &frozen.origin
                } else {
                    frozen.source.as_str()
                },
                &target,
            )
            .map_err(|error| error.to_string())?;
            let local = matches!(target.scheme(), "data" | "blob");
            if local && redirects != 0 {
                return Err("HTTP redirect to local resource denied".into());
            }
            if !local && !matches!(target.scheme(), "http" | "https") {
                return Err("Fetch URL scheme unavailable".into());
            }
            if !target.username().is_empty() || target.password().is_some() {
                return Err("Fetch URL credentials forbidden".into());
            }
            let cross = !local
                && (opaque
                    || origin == "null"
                    || target.origin().ascii_serialization() != frozen.origin);
            if mode == "same-origin" && cross {
                return Err("same-origin Fetch crossed origin".into());
            }
            let credentials = (request.credentials == "include"
                || request.credentials == "same-origin" && !cross)
                && !embedder.omit_credentials(cors, cross);
            let mut headers = request.headers.clone();
            if destination == "prefetch" {
                headers.push(("sec-purpose".into(), "prefetch".into()));
            }
            let referer = referrer_source
                .as_ref()
                .and_then(|source| referrer.header(source, &target));
            referrer_source = referer.as_deref().and_then(|value| Url::parse(value).ok());
            if let Some(value) = referer {
                headers.push(("referer".into(), value))
            }
            if let Some(value) = crate::referrer::request_origin_header(
                &origin,
                &frozen.source,
                &target,
                &method,
                cors && cross,
                referrer,
            ) {
                headers.push(("origin".into(), value))
            }
            metadata.observe(Some(&frozen.origin), &target);
            let security = crate::cookies::CookieRequestContext {
                fetch_metadata: metadata.clone(),
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
                kind: match destination {
                    "image" => crate::cookies::CookieRequestKind::Image,
                    "audio" | "video" | "track" => crate::cookies::CookieRequestKind::Media,
                    _ => crate::cookies::CookieRequestKind::Fetch,
                },
                initiator_url: Some(frozen.source.to_string()),
                top_level_url: Some(frozen.top.clone()),
                ancestor_urls: frozen.ancestors.clone(),
                redirect_chain_cross_site: cross_site,
                ..Default::default()
            };
            if !local
                && cors
                && cross
                && (request.use_cors_preflight
                    || cors_preflight_required(&method, &request.headers))
            {
                let names = cors_non_safelisted_headers(&request.headers);
                let mut headers = vec![
                    ("origin".into(), origin.clone()),
                    ("access-control-request-method".into(), method.clone()),
                ];
                if !names.is_empty() {
                    headers.push(("access-control-request-headers".into(), names.join(", ")))
                }
                let request_id = self.id()?;
                let _cleanup = self.ticket(id, request_id, &frozen)?;
                let response = crate::network::collect_operation_bounded_with_cancel(
                    &self.network,
                    NetworkRequest {
                        request_id,
                        method: "OPTIONS".into(),
                        url: target.to_string(),
                        headers,
                        body: Vec::new(),
                        credentials: false,
                        security: crate::cookies::CookieRequestContext {
                            credentials: crate::cookies::CookieCredentials::Omit,
                            ..security.clone()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    deadline.saturating_duration_since(Instant::now()),
                    64 * 1024,
                    cancel,
                )
                .map_err(|error| error.to_string())?;
                if !oryn_net::cors::cors_preflight_allowed(
                    &origin,
                    credentials,
                    &method,
                    &names,
                    response.status,
                    &response.headers,
                ) {
                    return Err("cross-origin request denied by CORS preflight".into());
                }
                _cleanup.finish_keepalive();
            }
            let request_id = self.id()?;
            let cleanup = if local {
                None
            } else {
                Some(self.ticket(id, request_id, &frozen)?)
            };
            let mut operation = if target.scheme() == "data" {
                if method != "GET" {
                    return Err("data Fetch requires GET".into());
                }
                let (mime, bytes) =
                    crate::network::decode_data_url(&target).map_err(|error| error.to_string())?;
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err("data Fetch body limit".into());
                }
                crate::network::buffered_response_operation(
                    request_id,
                    200,
                    vec![(
                        "content-type".into(),
                        mime.unwrap_or_else(|| "text/plain;charset=US-ASCII".into()),
                    )],
                    bytes,
                )
                .map_err(|error| error.to_string())?
            } else if target.scheme() == "blob" {
                if method != "GET" {
                    return Err("blob Fetch requires GET".into());
                }
                let (bytes, mime) = frozen
                    .blob
                    .take()
                    .ok_or("blob Fetch resource authority absent")?;
                let (status, headers, bytes) = crate::local_resource_response::blob(
                    bytes,
                    mime,
                    request
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
                        .map(|(_, value)| value.as_str()),
                )?;
                crate::network::buffered_response_operation(request_id, status, headers, bytes)
                    .map_err(|error| error.to_string())?
            } else {
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
            let (status, headers, length, progress, transport) =
                match self.event(&mut operation, cancel, keepalive, deadline, None)? {
                    NetworkEvent::Headers {
                        response_policy: _,
                        request_id: response_id,
                        status,
                        headers,
                        content_length,
                        upload,
                        timing,
                    } if response_id == request_id => (
                        status,
                        headers,
                        content_length,
                        upload,
                        timing.map(|timing| *timing),
                    ),
                    _ => return Err("Fetch header identity/order mismatch".into()),
                };
            if redirects == 0 {
                upload = progress
            }
            if headers.len() > 256
                || headers
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>()
                    > 256 * 1024
            {
                return Err("Fetch response header limit".into());
            }
            hops.push(crate::network::timing_hop(&target, &headers));
            if !local {
                frozen.reports.push_coep(coep.response(
                    crate::coep_reporting::ResponsePolicyInputs {
                        origin: &frozen.origin,
                        opaque,
                        target: &target,
                        cors,
                        credentials,
                        headers: &headers,
                        destination,
                    },
                ));
                embedder.authorize_response(
                    &frozen.origin,
                    opaque,
                    &target,
                    cors,
                    credentials,
                    &headers,
                )?;
            }
            if cors && cross && !cors_headers_allowed(&origin, credentials, &headers) {
                return Err("Fetch CORS response rejected".into());
            }
            let mut response_type = None;
            if oryn_net::fetch::is_redirect_status(status)
                && let Some(location) = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
            {
                match request
                    .fetch
                    .as_ref()
                    .map(|fetch| fetch.redirect.as_str())
                    .unwrap_or("follow")
                {
                    "error" => return Err("Fetch redirect mode error".into()),
                    "manual" => response_type = Some(FetchResponseType::OpaqueRedirect),
                    _ => {
                        if redirects == 20 {
                            return Err("Fetch redirect limit".into());
                        }
                        let next = target.join(location).map_err(|error| error.to_string())?;
                        if !matches!(next.scheme(), "http" | "https")
                            || !next.username().is_empty()
                            || next.password().is_some()
                        {
                            return Err("Fetch redirect URL forbidden".into());
                        }
                        oryn_net::fetch::rewrite_redirect_request(
                            status,
                            &target,
                            &next,
                            &mut method,
                            &mut request.headers,
                            &mut body,
                        );
                        if let Some(lease) = &frozen.lease {
                            lease.shrink_to(body.len())
                        }
                        if target.origin() != next.origin()
                            && target.origin() != frozen.source.origin()
                        {
                            origin = "null".into()
                        }
                        cross_site |=
                            crate::cookies::SchemefulSite::from_url(&next) != initial_site;
                        referrer = referrer.after_response(&headers);
                        target = next;
                        continue;
                    }
                }
            }
            if length.is_some_and(|length| length > 16 * 1024 * 1024) {
                return Err("Fetch response body limit".into());
            }
            let mut terminal_size = None;
            if let Some(fetch) = &request.fetch
                && !fetch.integrity.is_empty()
            {
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
                    response_type.is_none() && (cors || !cross || mode == "navigate"),
                )
                .map_err(|error| format!("Fetch integrity rejected: {error:?}"))?;
                terminal_size = Some(bytes.bytes.len() as u64);
                if let Some(cleanup) = &cleanup {
                    cleanup.finish_keepalive()
                }
                frozen.lease.take();
                operation = crate::network::buffered_body_operation(request_id, bytes)
                    .map_err(|error| error.to_string())?;
            }
            if response_type.is_none() && mode == "no-cors" && cross {
                response_type = Some(FetchResponseType::Opaque)
            }
            let elapsed = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            let mut timing = crate::network::resource_timing_metadata(
                &frozen.origin,
                opaque,
                hops,
                &crate::network::NetworkResponse {
                    response_policy: None,
                    request_id,
                    status,
                    headers: headers.clone(),
                    body: Vec::new(),
                    timing: transport,
                },
                elapsed,
            );
            timing.response_end_micros = None;
            timing.decoded_body_size = terminal_size;
            timing.encoded_body_size = timing
                .transport
                .as_ref()
                .and_then(|timing| timing.encoded_body_size)
                .or_else(|| {
                    headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.parse().ok())
                });
            let internal_response = response_type.map(|_| InternalResponseMetadata {
                status,
                headers: crate::network::sanitize_parent_response_headers(headers.clone()),
                url: target.to_string(),
                redirected: redirects != 0,
                range_requested: request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("range")),
            });
            let exposed = if cors && cross {
                cors_exposed_response_headers(headers, credentials)
            } else {
                headers
            };
            self.check(cancel, keepalive, deadline)?;
            let stream_id = self.id()?;
            let mut streams = self.streams.lock().map_err(|_| "Fetch streams poisoned")?;
            if streams.len() >= LIMIT {
                return Err("Fetch response stream capacity".into());
            }
            let body_cancel = Arc::new(AtomicBool::new(false));
            let drain = frozen.drain.take();
            streams.insert(
                stream_id,
                Arc::new(OwnedBody {
                    body: Mutex::new(Body {
                        operation,
                        current: bytes::Bytes::new(),
                        read: 0,
                        done: false,
                        deadline,
                        cancel: body_cancel.clone(),
                        cleanup,
                    }),
                    cancel: body_cancel,
                    drain,
                }),
            );
            return Ok(PlatformHostResult::HttpStarted {
                timing: Some(Box::new(timing)),
                internal_response,
                response_type,
                upload: upload.map(|upload| PlatformUploadProgress {
                    bytes_consumed: upload.bytes_consumed,
                    total_bytes: upload.total_bytes,
                    body_complete: upload.body_complete,
                }),
                cors_filtered: response_type.is_some() || cors && cross,
                redirected: response_type.is_none() && redirects != 0,
                stream_id,
                status: if response_type.is_some() { 0 } else { status },
                url: if response_type.is_some() {
                    String::new()
                } else {
                    target.to_string()
                },
                headers: if response_type.is_some() {
                    Vec::new()
                } else {
                    crate::network::sanitize_parent_response_headers(exposed)
                },
            });
        }
        Err("Fetch response unavailable".into())
    }
    fn read(&self, id: u64, max: usize, cancel: &AtomicBool) -> Result<PlatformHostResult, String> {
        let stream = self
            .streams
            .lock()
            .map_err(|_| "Fetch streams poisoned")?
            .get(&id)
            .cloned()
            .ok_or("Fetch stream absent or foreign")?;
        let result = (|| {
            let mut body = stream.body.lock().map_err(|_| "Fetch body poisoned")?;
            let mut encoded = None;
            while body.current.is_empty() && !body.done {
                if body.cancel.load(Ordering::Acquire) {
                    return Err("AbortError: Fetch stream cancelled".into());
                }
                body.operation.request_body_chunk();
                let deadline = body.deadline;
                match self.event(
                    &mut body.operation,
                    cancel,
                    false,
                    deadline,
                    Some(&stream.cancel),
                )? {
                    NetworkEvent::Chunk { request_id, bytes }
                        if request_id == body.operation.request_id =>
                    {
                        body.read = body
                            .read
                            .checked_add(bytes.len())
                            .filter(|size| *size <= 16 * 1024 * 1024)
                            .ok_or("Fetch body limit")?;
                        body.current = bytes
                    }
                    NetworkEvent::Complete {
                        request_id,
                        encoded_body_size,
                    } if request_id == body.operation.request_id => {
                        body.done = true;
                        encoded = encoded_body_size;
                        if let Some(cleanup) = &body.cleanup {
                            cleanup.finish_keepalive()
                        }
                    }
                    NetworkEvent::Failed { request_id, error }
                        if request_id == body.operation.request_id =>
                    {
                        return Err(error);
                    }
                    _ => return Err("Fetch body identity/order mismatch".into()),
                }
            }
            let length = body.current.len().min(max.clamp(1, 1024 * 1024));
            let bytes = body.current.split_to(length).to_vec();
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: encoded,
                stream_id: id,
                bytes,
                done: body.done && body.current.is_empty(),
            })
        })();
        if result.is_err()
            || matches!(
                &result,
                Ok(PlatformHostResult::HttpChunk { done: true, .. })
            )
        {
            self.streams
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&id);
        }
        result
    }
    fn abort(&self, id: u64) {
        if let Some(body) = self
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id)
        {
            body.cancel.store(true, Ordering::Release);
        }
    }
    fn abandon(&self, id: u64) {
        let Some(owned) = self
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id)
        else {
            return;
        };
        let Some(drain) = owned.drain.clone() else {
            owned.cancel.store(true, Ordering::Release);
            return;
        };
        drain.orphan(move || {
            let mut body = match owned.body.try_lock() {
                Ok(body) => body,
                Err(std::sync::TryLockError::WouldBlock) => return false,
                Err(std::sync::TryLockError::Poisoned(_)) => return true,
            };
            if owned.cancel.load(Ordering::Acquire) || Instant::now() >= body.deadline {
                return true;
            }
            body.current = bytes::Bytes::new();
            for _ in 0..8 {
                body.operation.request_body_chunk();
                match body.operation.try_event() {
                    Ok(NetworkEvent::Chunk { request_id, bytes })
                        if request_id == body.operation.request_id =>
                    {
                        body.read = body.read.saturating_add(bytes.len());
                        if body.read > 16 * 1024 * 1024 {
                            return true;
                        }
                    }
                    Ok(NetworkEvent::Complete { request_id, .. })
                        if request_id == body.operation.request_id =>
                    {
                        body.done = true;
                        if let Some(cleanup) = &body.cleanup {
                            cleanup.finish_keepalive()
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefetch_uses_empty_metadata_native_purpose_and_hint_policy() {
        let probe = Probe::default();
        probe
            .replies
            .lock()
            .unwrap()
            .push_back(reply(b"future body"));
        let engine = engine(&probe);
        let mut request = request("https://example.test/future");
        request.fetch = None;
        request.credentials = "include".into();
        let result = engine
            .start(
                HostOperationId(8),
                request,
                "prefetch",
                false,
                frozen("default-src 'none'; img-src 'self'"),
                &Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        let stream = id(result);
        let requests = probe.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .headers
                .contains(&("sec-purpose".into(), "prefetch".into()))
        );
        assert_eq!(
            requests[0].security.fetch_metadata.mode,
            crate::fetch_metadata::Metadata::resource("no-cors", "", false).mode
        );
        assert_eq!(
            requests[0].security.fetch_metadata.destination,
            crate::fetch_metadata::Metadata::resource("no-cors", "", false).destination
        );
        drop(requests);
        engine.abort(stream);
    }
    #[test]
    fn integrity_and_streaming_fetch_preserve_transport_terminal_sizes() {
        for integrity in [false, true] {
            for encoded in [None, Some(0), Some(17)] {
                let probe = Probe::default();
                let mut response = reply(b"hello");
                response.encoded = encoded;
                probe.replies.lock().unwrap().push_back(response);
                let engine = engine(&probe);
                let mut req = request("https://example.test/encoded");
                if integrity {
                    req.fetch.as_mut().unwrap().integrity = digest(b"hello");
                }
                let stream = id(start(&engine, req, frozen("connect-src 'self'")).unwrap());
                let mut bytes = Vec::new();
                loop {
                    let PlatformHostResult::HttpChunk {
                        bytes: chunk,
                        done,
                        encoded_body_size,
                        ..
                    } = engine.read(stream, 1024, &AtomicBool::new(false)).unwrap()
                    else {
                        panic!("body chunk")
                    };
                    bytes.extend(chunk);
                    if done {
                        assert_eq!(encoded_body_size, encoded, "integrity={integrity}");
                        break;
                    } else {
                        assert_eq!(encoded_body_size, None);
                    }
                }
                assert_eq!(bytes, b"hello");
                assert!(engine.streams.lock().unwrap().is_empty());
            }
        }
    }
    #[test]
    fn empty_integrity_verified_response_reaches_eof() {
        let probe = Probe::default();
        probe.replies.lock().unwrap().push_back(reply(b""));
        let engine = engine(&probe);
        let mut req = request("https://example.test/empty");
        req.fetch.as_mut().unwrap().integrity = digest(b"");
        let stream = id(start(&engine, req, frozen("connect-src 'self'")).unwrap());
        assert!(
            matches!(engine.read(stream,1024,&AtomicBool::new(false)).unwrap(),PlatformHostResult::HttpChunk{done:true,bytes,..}if bytes.is_empty())
        );
    }
    #[test]
    fn origin_header_uses_fetch_method_taint_and_referrer_policy() {
        use crate::referrer::ReferrerPolicy::*;
        let source = Url::parse("https://example.test/").unwrap();
        let cross = Url::parse("https://other.test/").unwrap();
        let insecure = Url::parse("http://other.test/").unwrap();
        assert_eq!(
            crate::referrer::request_origin_header(
                "https://example.test",
                &source,
                &source,
                "POST",
                false,
                StrictOriginWhenCrossOrigin
            )
            .as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            crate::referrer::request_origin_header(
                "https://example.test",
                &source,
                &source,
                "GET",
                false,
                NoReferrer
            ),
            None
        );
        assert_eq!(
            crate::referrer::request_origin_header(
                "https://example.test",
                &source,
                &cross,
                "POST",
                false,
                SameOrigin
            )
            .as_deref(),
            Some("null")
        );
        assert_eq!(
            crate::referrer::request_origin_header(
                "https://example.test",
                &source,
                &insecure,
                "POST",
                false,
                StrictOrigin
            )
            .as_deref(),
            Some("null")
        );
        assert_eq!(
            crate::referrer::request_origin_header(
                "https://example.test",
                &source,
                &cross,
                "GET",
                true,
                NoReferrer
            )
            .as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            crate::referrer::request_origin_header(
                "null", &source, &source, "POST", true, UnsafeUrl
            )
            .as_deref(),
            Some("null")
        );
    }
    #[derive(Clone)]
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        bytes: Vec<u8>,
        encoded: Option<u64>,
    }
    #[derive(Clone, Default)]
    struct Probe {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        replies: Arc<Mutex<std::collections::VecDeque<Reply>>>,
    }
    impl NetworkBroker for Probe {
        fn send(
            &self,
            _: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            panic!("modern Fetch must use native operation")
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, crate::network::NetworkError> {
            let id = request.request_id;
            self.requests.lock().unwrap().push(request);
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("admitted expected request");
            let (mut operation, encoded) = (
                crate::network::buffered_response_operation(
                    id,
                    reply.status,
                    reply.headers,
                    reply.bytes,
                )?,
                reply.encoded,
            );
            let mut events = std::collections::VecDeque::new();
            while let Ok(mut event) = operation.try_event() {
                if let NetworkEvent::Complete {
                    encoded_body_size, ..
                } = &mut event
                {
                    *encoded_body_size = encoded;
                }
                events.push_back(event);
            }
            operation.prefetched = events;
            Ok(operation)
        }
    }
    fn reply(bytes: &[u8]) -> Reply {
        Reply {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            bytes: bytes.to_vec(),
            encoded: None,
        }
    }
    fn engine(probe: &Probe) -> Engine {
        Engine {
            network: SharedNetworkBroker::new(probe.clone()).new_request_scope(),
            retired: AtomicBool::new(false),
            next: std::sync::atomic::AtomicU64::new(1),
            cleanup: Default::default(),
            streams: Default::default(),
        }
    }
    fn frozen(policy: &str) -> Frozen {
        Frozen {
            source: Url::parse("https://example.test/path/page").unwrap(),
            base: Url::parse("https://example.test/path/page").unwrap(),
            origin: "https://example.test".into(),
            top: "https://example.test/path/page".into(),
            ancestors: Vec::new(),
            policy: crate::csp::CspPolicySet::from_headers(
                "https://example.test/path/page",
                &[("content-security-policy".into(), policy.into())],
            ),
            referrer: "strict-origin-when-cross-origin".into(),
            blob: None,
            lease: None,
            drain: None,
            reports: Arc::new(crate::executable::module_graph_load::ScriptCspReports::default()),
        }
    }
    fn request(url: &str) -> PlatformHttpRequest {
        PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: Some(FetchOptions {
                keepalive: false,
                privileged_range: false,
                cache: Default::default(),
                mode: "cors".into(),
                redirect: "follow".into(),
                integrity: String::new(),
            }),
            use_cors_preflight: false,
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: "same-origin".into(),
            initiator_url: None,
            referrer_policy: None,
        }
    }
    fn start(
        engine: &Engine,
        request: PlatformHttpRequest,
        frozen: Frozen,
    ) -> Result<PlatformHostResult, String> {
        engine.start(
            HostOperationId(1),
            request,
            "",
            true,
            frozen,
            &Arc::new(AtomicBool::new(false)),
        )
    }
    fn id(response: PlatformHostResult) -> u64 {
        let PlatformHostResult::HttpStarted { stream_id, .. } = response else {
            panic!("started")
        };
        stream_id
    }
    fn digest(bytes: &[u8]) -> String {
        use base64::Engine;
        use sha2::Digest;
        format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
        )
    }
    #[test]
    fn initial_frame_adapter_uses_bound_native_frame_identity_on_all_platforms() {
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(Probe::default()),
            Url::parse("https://example.test/").unwrap(),
        );
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            1,
        )
        .unwrap();
        host.execute(
            "globalThis.child=document.createElement('iframe');document.body.appendChild(child)",
        )
        .unwrap();
        v8::scope!(let scope,&mut host.isolate);
        let contexts = scope
            .get_slot::<NativeRealmIdentities>()
            .unwrap()
            .contexts
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut found = false;
        for weak in contexts {
            let Some(context) = weak.to_local(scope) else {
                continue;
            };
            let Some(frame) = context.get_slot::<FrameLifetime>() else {
                continue;
            };
            let installed = context
                .get_slot::<Installed>()
                .expect("standalone frame HTTP adapter installed");
            assert_eq!(
                installed.page,
                oryn_common::v3::PageId(u64::from(frame.root_realm.0))
            );
            assert!(
                matches!(&installed.creator,oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Frame(token),..}if token==&frame.message_token)
            );
            found = true;
        }
        assert!(found, "initial frame has native transport authority");
    }
    #[test]
    fn modern_window_fetch_is_installed_and_resolves_streamed_response() {
        let probe = Probe::default();
        probe.replies.lock().unwrap().push_back(reply(b"modern"));
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(probe.clone()),
            Url::parse("https://example.test/").unwrap(),
        );
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            1,
        )
        .unwrap();
        host.commit_location("https://example.test/").unwrap();
        assert_eq!(host.location_url().unwrap(), "https://example.test/");
        host.execute("globalThis.fetchResult='pending';fetch('/data').then(r=>r.text()).then(text=>fetchResult=text,e=>fetchResult=String(e))").unwrap();
        for _ in 0..200 {
            host.drain_async_fetches(false).unwrap();
            host.run_microtask_checkpoint().unwrap();
            if host.evaluate_string("fetchResult").unwrap() == "modern" {
                break;
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(host.evaluate_string("fetchResult").unwrap(), "modern");
        assert_eq!(probe.requests.lock().unwrap().len(), 1);
        assert_eq!(
            probe.requests.lock().unwrap()[0].url,
            "https://example.test/data"
        );
        assert_eq!(
            probe.requests.lock().unwrap()[0]
                .security
                .fetch_metadata
                .mode,
            Some(crate::fetch_metadata::Mode::Cors)
        );
    }
    #[test]
    fn cors_preflight_credentials_cache_and_response_headers_use_native_fetch_rules() {
        let probe = Probe::default();
        let preflight = Reply {
            status: 204,
            headers: vec![
                (
                    "access-control-allow-origin".into(),
                    "https://example.test".into(),
                ),
                ("access-control-allow-credentials".into(), "true".into()),
                ("access-control-allow-methods".into(), "PUT".into()),
                ("access-control-allow-headers".into(), "x-test".into()),
            ],
            bytes: Vec::new(),
            encoded: None,
        };
        let mut response = reply(b"ok");
        response.headers.extend([
            (
                "access-control-allow-origin".into(),
                "https://example.test".into(),
            ),
            ("access-control-allow-credentials".into(), "true".into()),
            ("access-control-expose-headers".into(), "x-visible".into()),
            ("x-visible".into(), "yes".into()),
            ("x-hidden".into(), "no".into()),
            ("set-cookie".into(), "private=1".into()),
        ]);
        probe.replies.lock().unwrap().extend([preflight, response]);
        let engine = engine(&probe);
        let mut req = request("https://other.test/data");
        req.method = "PUT".into();
        req.headers.push(("x-test".into(), "value".into()));
        req.credentials = "include".into();
        req.fetch.as_mut().unwrap().cache = oryn_js::platform::FetchCacheMode::Reload;
        let response = start(&engine, req, frozen("connect-src *")).unwrap();
        let PlatformHostResult::HttpStarted {
            stream_id,
            headers,
            cors_filtered,
            ..
        } = response
        else {
            panic!()
        };
        assert!(cors_filtered);
        assert!(headers.iter().any(|(name, _)| name == "x-visible"));
        assert!(
            !headers
                .iter()
                .any(|(name, _)| matches!(name.as_str(), "x-hidden" | "set-cookie"))
        );
        let requests = probe.requests.lock().unwrap();
        assert_eq!(requests[0].method, "OPTIONS");
        assert!(!requests[0].credentials);
        assert_eq!(requests[1].method, "PUT");
        assert!(requests[1].credentials);
        assert_eq!(
            requests[1].security.fetch_cache,
            oryn_js::platform::FetchCacheMode::Reload
        );
        drop(requests);
        engine.abort(stream_id);
    }
    #[test]
    fn cross_origin_redirect_back_to_source_keeps_null_origin_and_cors_filter() {
        for allow in [true, false] {
            let probe = Probe::default();
            let mut away = reply(b"");
            away.status = 302;
            away.headers = vec![("location".into(), "https://other.test/away".into())];
            let mut back = away.clone();
            back.headers = vec![
                ("location".into(), "https://example.test/final".into()),
                ("access-control-allow-origin".into(), "*".into()),
            ];
            let mut final_reply = reply(b"final");
            if allow {
                final_reply
                    .headers
                    .push(("access-control-allow-origin".into(), "null".into()));
            }
            probe
                .replies
                .lock()
                .unwrap()
                .extend([away, back, final_reply]);
            let engine = engine(&probe);
            let result = start(
                &engine,
                request("https://example.test/start"),
                frozen("connect-src *"),
            );
            assert_eq!(result.is_ok(), allow);
            let requests = probe.requests.lock().unwrap();
            assert!(
                requests[2]
                    .headers
                    .contains(&("origin".into(), "null".into()))
            );
            assert!(!requests[2].credentials);
            drop(requests);
            if let Ok(PlatformHostResult::HttpStarted {
                stream_id,
                cors_filtered,
                ..
            }) = result
            {
                assert!(cors_filtered);
                engine.abort(stream_id);
            }
        }
    }
    #[test]
    fn same_origin_csp_and_bad_integrity_cannot_publish_response() {
        let probe = Probe::default();
        let engine = engine(&probe);
        let mut req = request("https://other.test/data");
        req.fetch.as_mut().unwrap().mode = "same-origin".into();
        assert!(start(&engine, req, frozen("connect-src *")).is_err());
        assert!(
            start(
                &engine,
                request("https://example.test/data"),
                frozen("connect-src 'none'")
            )
            .is_err()
        );
        assert!(probe.requests.lock().unwrap().is_empty());
        probe.replies.lock().unwrap().push_back(reply(b"actual"));
        let mut req = request("https://example.test/data");
        req.fetch.as_mut().unwrap().integrity = digest(b"other");
        assert!(start(&engine, req, frozen("connect-src 'self'")).is_err());
        assert!(engine.streams.lock().unwrap().is_empty());
    }
    #[test]
    fn redirects_rewrite_body_auth_referrer_and_manual_response_is_opaque() {
        let probe = Probe::default();
        let mut redirect = reply(b"");
        redirect.status = 302;
        redirect.headers.extend([
            ("location".into(), "https://other.test/final".into()),
            ("referrer-policy".into(), "no-referrer".into()),
        ]);
        let mut response = reply(b"ok");
        response
            .headers
            .push(("access-control-allow-origin".into(), "*".into()));
        probe
            .replies
            .lock()
            .unwrap()
            .extend([redirect.clone(), response]);
        let engine = engine(&probe);
        let mut req = request("https://example.test/start");
        req.method = "POST".into();
        req.body = b"request".to_vec();
        req.headers.extend([
            ("authorization".into(), "native-author-token".into()),
            ("content-type".into(), "text/plain".into()),
        ]);
        let stream = id(start(&engine, req, frozen("connect-src *")).unwrap());
        let requests = probe.requests.lock().unwrap();
        assert_eq!(requests[1].method, "GET");
        assert!(requests[1].body.is_empty());
        assert!(!requests[1].headers.iter().any(|(name, _)| matches!(
            name.as_str(),
            "authorization" | "content-type" | "referer"
        )));
        drop(requests);
        engine.abort(stream);
        probe.replies.lock().unwrap().push_back(redirect);
        let mut req = request("https://example.test/start");
        req.fetch.as_mut().unwrap().redirect = "manual".into();
        assert!(
            matches!(start(&engine,req,frozen("connect-src *")).unwrap(),PlatformHostResult::HttpStarted{status:0,response_type:Some(FetchResponseType::OpaqueRedirect),headers,url,..}if headers.is_empty()&&url.is_empty())
        );
    }
    #[test]
    fn data_blob_range_and_raw_integrity_keep_bytes_and_filtered_response_metadata() {
        let probe = Probe::default();
        let engine = engine(&probe);
        let mut req = request("data:text/plain,%E9");
        req.fetch.as_mut().unwrap().integrity = digest(&[0xe9]);
        let stream = id(start(&engine, req, frozen("connect-src data: blob:")).unwrap());
        assert!(
            matches!(engine.read(stream,1024,&AtomicBool::new(false)).unwrap(),PlatformHostResult::HttpChunk{bytes,..}if bytes==[0xe9])
        );
        engine.abort(stream);
        let mut authority = frozen("connect-src blob:");
        authority.blob = Some((vec![0, 1, 2, 3], "application/octet-stream".into()));
        let mut req = request("blob:https://example.test/native");
        req.headers.push(("range".into(), "bytes=1-2".into()));
        let result = start(&engine, req, authority).unwrap();
        let PlatformHostResult::HttpStarted {
            status,
            stream_id,
            headers,
            ..
        } = result
        else {
            panic!()
        };
        assert_eq!(status, 206);
        assert!(headers.contains(&("content-range".into(), "bytes 1-2/4".into())));
        assert!(
            matches!(engine.read(stream_id,1024,&AtomicBool::new(false)).unwrap(),PlatformHostResult::HttpChunk{bytes,..}if bytes==[1,2])
        );
        assert!(probe.requests.lock().unwrap().is_empty());
        engine.abort(stream_id);
    }
    #[test]
    fn keepalive_and_deferred_share_quota_until_terminal_response_ack() {
        let probe = Probe::default();
        probe.replies.lock().unwrap().push_back(reply(b"response"));
        let engine = engine(&probe);
        let owner = crate::runtime::deferred_fetch::StandaloneOwner::new(
            engine.network.clone(),
            Arc::new(crate::executable::module_graph_load::ScriptCspReports::default()),
        );
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 1,
        };
        let page = oryn_common::v3::PageId(7);
        let deferred = owner.reserve_keepalive(page, &creator, 16 * 1024).unwrap();
        let mut authority = frozen("connect-src 'self'");
        authority.lease = Some(Arc::new(
            owner.reserve_keepalive(page, &creator, 48 * 1024).unwrap(),
        ));
        authority.drain = Some(crate::keepalive_drain::Ticket::reserve().unwrap());
        let mut req = request("https://example.test/data");
        req.method = "POST".into();
        req.body = vec![0; 48 * 1024];
        req.fetch.as_mut().unwrap().keepalive = true;
        let stream = id(start(&engine, req, authority).unwrap());
        assert!(owner.reserve_keepalive(page, &creator, 1).is_err());
        while !matches!(
            engine.read(stream, 1024, &AtomicBool::new(false)).unwrap(),
            PlatformHostResult::HttpChunk { done: true, .. }
        ) {}
        assert!(owner.reserve_keepalive(page, &creator, 48 * 1024).is_ok());
        drop(deferred);
    }
    #[test]
    fn accepted_keepalive_survives_retirement_but_explicit_cancel_does_not() {
        let probe = Probe::default();
        probe
            .replies
            .lock()
            .unwrap()
            .extend([reply(b"survive"), reply(b"cancel")]);
        let engine = engine(&probe);
        let owner = crate::runtime::deferred_fetch::StandaloneOwner::new(
            engine.network.clone(),
            Arc::new(crate::executable::module_graph_load::ScriptCspReports::default()),
        );
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 1,
        };
        let make = || {
            let mut authority = frozen("connect-src 'self'");
            authority.lease = Some(Arc::new(
                owner
                    .reserve_keepalive(oryn_common::v3::PageId(7), &creator, 1)
                    .unwrap(),
            ));
            authority.drain = Some(crate::keepalive_drain::Ticket::reserve().unwrap());
            authority
        };
        let first = make();
        engine.retired.store(true, Ordering::Release);
        engine.cleanup.retire_preserving_keepalive();
        let mut req = request("https://example.test/data");
        req.method = "POST".into();
        req.body = vec![0];
        req.fetch.as_mut().unwrap().keepalive = true;
        let stream = id(start(&engine, req.clone(), first).unwrap());
        engine.abandon(stream);
        assert!(engine.streams.lock().unwrap().is_empty());
        let cancel = Arc::new(AtomicBool::new(true));
        assert!(
            engine
                .start(HostOperationId(2), req, "", true, make(), &cancel)
                .is_err()
        );
        assert!(
            start(
                &engine,
                request("https://example.test/ordinary"),
                frozen("connect-src *")
            )
            .is_err()
        );
    }
}
