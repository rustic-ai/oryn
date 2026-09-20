// Transport custody for declarative template responses. Included by the loader.
// No DOM/V8 references enter executor jobs; every backend has the same byte stream.
enum TemplateResponse {
    Started {
        request_id: u64,
        stream_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    Chunk {
        stream_id: u64,
        bytes: Vec<u8>,
        done: bool,
    },
}
type TemplatePending = std::sync::mpsc::Receiver<Result<TemplateResponse, String>>;
struct TemplateRequest {
    request_id: u64,
    url: String,
    initiator: String,
    nonce: Option<String>,
    integrity: String,
    referrer_policy: String,
}
enum ExternalTemplateTransport {
    #[cfg(unix)]
    Parent {
        client: crate::context_service::SharedContextClient,
        page: oryn_common::v3::PageId,
        creator: oryn_js::worker_wire::Creator,
        cleanup: Arc<crate::context_service::ContextNetworkReply>,
        request_cleanup: Arc<crate::context_service::ContextHttpCancellation>,
    },
    Standalone(Arc<TemplateStandalone>),
}
impl ExternalTemplateTransport {
    fn new(backend: &ModuleGraphBackend, request_id: u64) -> Result<Self, String> {
        match backend {
            #[cfg(unix)]
            ModuleGraphBackend::Window(client, page, Some((creator, _)), _) => Ok(Self::Parent {
                client: client.clone(),
                page: *page,
                creator: creator.clone(),
                cleanup: Arc::new(
                    client
                        .reserve_http_network_reply(*page, Some(creator.clone()))
                        .map_err(|e| e.to_string())?,
                ),
                request_cleanup: client
                    .reserve_http_cancellation(*page, request_id)
                    .map_err(|e| e.to_string())?,
            }),
            ModuleGraphBackend::Broker { broker, .. } => Ok(Self::Standalone(Arc::new(
                TemplateStandalone::new(backend.clone(), Some(broker.clone().new_request_scope())),
            ))),
            ModuleGraphBackend::Sources(..) => Ok(Self::Standalone(Arc::new(
                TemplateStandalone::new(backend.clone(), None),
            ))),
            _ => Err("external template requires native document resource authority".into()),
        }
    }
    fn start(&self, request: TemplateRequest) -> Result<TemplatePending, String> {
        match self {
            #[cfg(unix)]
            Self::Parent {
                client,
                page,
                creator,
                cleanup,
                ..
            } => {
                let wire = crate::context_service::ContextServiceRequest::TemplateStreamStart {
                    page_id: *page,
                    nonce: request.nonce,
                    request: crate::context_service::HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: Some(oryn_js::platform::FetchOptions {
                            keepalive: false,
                            privileged_range: false,
                            cache: Default::default(),
                            mode: "cors".into(),
                            redirect: "follow".into(),
                            integrity: request.integrity,
                        }),
                        client: Some(creator.clone()),
                        use_cors_preflight: false,
                        request_id: request.request_id,
                        method: "GET".into(),
                        url: request.url,
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: true,
                        credentials_mode: Some("same-origin".into()),
                        initiator_url: Some(request.initiator),
                        referrer_policy: Some(request.referrer_policy),
                    },
                };
                template_parent_io(client, wire, cleanup.clone())
            }
            Self::Standalone(owner) => {
                let owner = owner.clone();
                template_submit(move || {
                    let result = owner.start(request);
                    if result.is_err() {
                        owner.cancel()
                    }
                    result
                })
            }
        }
    }
    fn read(&self, stream_id: u64) -> Result<TemplatePending, String> {
        match self {
            #[cfg(unix)]
            Self::Parent {
                client,
                page,
                creator,
                cleanup,
                ..
            } => template_parent_io(
                client,
                crate::context_service::ContextServiceRequest::HttpStreamRead {
                    page_id: *page,
                    stream_id,
                    max_bytes: 64 * 1024,
                    creator: Some(creator.clone()),
                },
                cleanup.clone(),
            ),
            Self::Standalone(owner) => {
                let owner = owner.clone();
                template_submit(move || {
                    let result = owner.read(stream_id);
                    if result.is_err() {
                        owner.cancel()
                    }
                    result
                })
            }
        }
    }
    fn request_finished(&self) {
        #[cfg(unix)]
        if let Self::Parent {
            request_cleanup, ..
        } = self
        {
            request_cleanup.finish();
        }
    }
    fn stream_finished(&self) {
        #[cfg(unix)]
        if let Self::Parent { cleanup, .. } = self {
            cleanup.finish();
        }
    }
}
impl Drop for ExternalTemplateTransport {
    fn drop(&mut self) {
        match self {
            #[cfg(unix)]
            Self::Parent {
                cleanup,
                request_cleanup,
                ..
            } => {
                cleanup.cancel();
                request_cleanup.cancel();
            }
            Self::Standalone(owner) => owner.cancel(),
        }
    }
}
fn template_submit(
    work: impl FnOnce() -> Result<TemplateResponse, String> + Send + 'static,
) -> Result<TemplatePending, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    crate::resource_io_executor::submit(move || {
        let _ = sender.send(work());
    })?;
    Ok(receiver)
}
#[cfg(unix)]
fn template_parent_io(
    client: &crate::context_service::SharedContextClient,
    request: crate::context_service::ContextServiceRequest,
    cleanup: Arc<crate::context_service::ContextNetworkReply>,
) -> Result<TemplatePending, String> {
    let ownership = client.track_resource_job().map_err(|e| e.to_string())?;
    let client = client.clone();
    template_submit(move || {
        let _ownership = ownership;
        match client.request(request).map_err(|e| e.to_string())? {
            crate::context_service::ContextServiceResponse::HttpStreamStarted {
                request_id,
                stream_id,
                status,
                headers,
                ..
            } => {
                cleanup.attach_stream(stream_id);
                Ok(TemplateResponse::Started {
                    request_id,
                    stream_id,
                    status,
                    headers,
                })
            }
            crate::context_service::ContextServiceResponse::HttpStreamChunk {
                stream_id,
                body_base64,
                done,
                ..
            } => Ok(TemplateResponse::Chunk {
                stream_id,
                bytes: external_template_chunk(&body_base64, done)?,
                done,
            }),
            _ => Err("external template response kind".into()),
        }
    })
}
struct TemplateStandalone {
    backend: ModuleGraphBackend,
    broker: Option<crate::network::SharedNetworkBroker>,
    cancelled: std::sync::atomic::AtomicBool,
    active: std::sync::atomic::AtomicU64,
    deadline: Instant,
    state: std::sync::Mutex<TemplateStream>,
}
#[derive(Default)]
struct TemplateStream {
    stream_id: Option<u64>,
    operation: Option<crate::network::NetworkOperation>,
    buffer: std::collections::VecDeque<u8>,
    done: bool,
    bytes: usize,
    timing: Option<crate::executable::module_graph_load::ExternalTimingGuard>,
    details: Option<oryn_js::platform::ResourceTimingMetadata>,
    started: Option<Instant>,
}
impl TemplateStandalone {
    fn new(
        backend: ModuleGraphBackend,
        broker: Option<crate::network::SharedNetworkBroker>,
    ) -> Self {
        Self {
            backend,
            broker,
            cancelled: std::sync::atomic::AtomicBool::new(false),
            active: std::sync::atomic::AtomicU64::new(0),
            deadline: Instant::now() + Duration::from_secs(60),
            state: Default::default(),
        }
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            Err("external template cancelled".into())
        } else if Instant::now() >= self.deadline {
            Err("external template deadline".into())
        } else {
            Ok(())
        }
    }
    fn cancel(&self) {
        use crate::network::NetworkBroker;
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        let id = self.active.swap(0, std::sync::atomic::Ordering::AcqRel);
        if id != 0
            && let Some(broker) = &self.broker
        {
            let _ = broker.cancel_operation(id);
        }
    }
    fn authorize(
        &self,
        url: &url::Url,
        original: &str,
        nonce: &Option<String>,
        redirect: bool,
    ) -> Result<(), String> {
        match &self.backend {
            ModuleGraphBackend::Broker {
                csp: Some(snapshot),
                ..
            }
            | ModuleGraphBackend::Sources(_, Some(snapshot), _)
                if snapshot.native::<crate::csp::CspPolicySet>().is_none() =>
            {
                return Err("external template native CSP authority absent".into());
            }
            _ => {}
        }
        if let Some(policy) = self.backend.local_script_policy() {
            let decision = policy.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Script,
                    url: url.to_string(),
                    nonce: nonce.clone(),
                    inline_source: None,
                    parser_inserted: true,
                    initiated_by_trusted_script: false,
                },
                redirect,
                original,
            );
            self.backend
                .report_violations(policy, decision.violations)
                .map_err(|e| e.to_string())?;
            if !decision.allowed
                || policy
                    .sandbox()
                    .is_some_and(|sandbox| !sandbox.allow_scripts)
            {
                return Err("external template blocked by script CSP".into());
            }
        }
        Ok(())
    }
    fn event(
        &self,
        operation: &mut crate::network::NetworkOperation,
    ) -> Result<crate::network::NetworkEvent, String> {
        loop {
            self.check()?;
            match operation.try_event() {
                Ok(event) => return Ok(event),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Err(
                        "external template response disconnected before terminal acknowledgement"
                            .into(),
                    );
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    std::thread::park_timeout(Duration::from_millis(1))
                }
            }
        }
    }
    fn start(&self, request: TemplateRequest) -> Result<TemplateResponse, String> {
        use crate::cookies::{CookieCredentials, CookieRequestContext, CookieRequestKind};
        use crate::network::{NetworkBroker, NetworkEvent, NetworkRequest};
        self.check()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "external template stream poisoned")?;
        if state.stream_id.is_some() {
            return Err("external template already started".into());
        }
        state.started = Some(Instant::now());
        state.timing = self
            .backend
            .reports()
            .map(|reports| reports.time_resource(&request.url, "other"));
        let initiator = url::Url::parse(&request.initiator).map_err(|e| e.to_string())?;
        let origin = initiator.origin();
        let mut request_origin = origin.ascii_serialization();
        let mut opaque = false;
        let mut url = url::Url::parse(&request.url).map_err(|e| e.to_string())?;
        let mut referrer = Some(initiator.clone());
        let mut referrer_policy =
            crate::referrer::ReferrerPolicy::parse_list(&request.referrer_policy);
        let mut metadata = crate::fetch_metadata::Metadata::resource(
            "cors",
            "script",
            matches!(origin, url::Origin::Opaque(_)),
        );
        let mut response_headers = None;
        let mut timing_hops = Vec::new();
        for hop in 0..=10 {
            self.check()?;
            if let Some(policy) = self.backend.local_script_policy()
                && let Some(upgraded) = policy.upgraded_url(&url)
            {
                url = upgraded;
            }
            self.authorize(&url, &request.url, &request.nonce, hop != 0)?;
            crate::mixed_content::enforce(&request.initiator, &url).map_err(|e| e.to_string())?;
            if url.scheme() == "data" {
                let (mime, body) =
                    crate::network::decode_data_url(&url).map_err(|e| e.to_string())?;
                if body.len() > 8 * 1024 * 1024 {
                    return Err("external template body limit".into());
                }
                state.details = Some(crate::network::local_resource_timing_metadata(
                    &origin.ascii_serialization(),
                    &body,
                    mime.as_deref(),
                    0,
                ));
                state.bytes = body.len();
                state.buffer = body.into();
                state.done = true;
                response_headers = Some(
                    mime.map(|mime| vec![("content-type".into(), mime)])
                        .unwrap_or_default(),
                );
                break;
            }
            if let ModuleGraphBackend::Sources(sources, ..) = &self.backend {
                if !matches!(url.scheme(), "http" | "https")
                    || url.origin() != origin
                    || matches!(origin, url::Origin::Opaque(_))
                {
                    return Err(
                        "decoded template sources cannot establish cross-origin response authority"
                            .into(),
                    );
                }
                if !request.integrity.is_empty() {
                    return Err("decoded template sources cannot verify response integrity".into());
                }
                let source = sources
                    .get(url.as_str())
                    .ok_or("external template exact local source unavailable")?;
                if source.len() > 8 * 1024 * 1024 {
                    return Err("external template body limit".into());
                }
                state.details = Some(crate::network::local_resource_timing_metadata(
                    &origin.ascii_serialization(),
                    source.as_bytes(),
                    Some("text/html;charset=utf-8"),
                    0,
                ));
                state.bytes = source.len();
                state.buffer = source.as_bytes().to_vec().into();
                state.done = true;
                response_headers = Some(vec![(
                    "content-type".into(),
                    "text/html;charset=utf-8".into(),
                )]);
                break;
            }
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("external template CORS URL unavailable".into());
            }
            let ModuleGraphBackend::Broker {
                base,
                isolation,
                coep,
                reports,
                ..
            } = &self.backend
            else {
                return Err("external template transport authority unavailable".into());
            };
            let broker = self.broker.as_ref().ok_or("template broker absent")?;
            let id = hop + 1;
            let cross = opaque || url.origin() != origin;
            let omit = isolation.omit_credentials(true, cross);
            let credentials = !cross && !omit;
            let mut headers = vec![("accept".into(), "*/*".into())];
            if cross {
                headers.push(("origin".into(), request_origin.clone()))
            }
            let sent = referrer
                .as_ref()
                .and_then(|source| referrer_policy.header(source, &url));
            referrer = sent.as_ref().and_then(|value| url::Url::parse(value).ok());
            if let Some(sent) = sent {
                headers.push(("referer".into(), sent))
            }
            metadata.observe(Some(&request.initiator), &url);
            self.active.store(id, std::sync::atomic::Ordering::Release);
            self.check()?;
            let mut operation = broker
                .start_operation(NetworkRequest {
                    request_id: id,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials,
                    security: CookieRequestContext {
                        fetch_metadata: metadata.clone(),
                        dns_timing: None,
                        credentials: if omit {
                            CookieCredentials::Omit
                        } else {
                            CookieCredentials::SameOrigin
                        },
                        kind: CookieRequestKind::Script,
                        initiator_url: Some(request.initiator.clone()),
                        top_level_url: Some(base.to_string()),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                })
                .map_err(|e| e.to_string())?;
            if let Err(error) = self.check() {
                let _ = broker.cancel_operation(id);
                return Err(error);
            }
            let (status, headers, timing) = match self.event(&mut operation)? {
                NetworkEvent::Headers {
                    request_id,
                    status,
                    headers,
                    timing,
                    ..
                } if request_id == id => (status, headers, timing.map(|timing| *timing)),
                _ => return Err("external template initial response identity/order".into()),
            };
            if headers.len() > 256
                || headers
                    .iter()
                    .try_fold(0usize, |size, (name, value)| {
                        size.checked_add(name.len())
                            .and_then(|size| size.checked_add(value.len()))
                    })
                    .is_none_or(|size| size > 256 * 1024)
            {
                return Err("external template response header limit".into());
            }
            timing_hops.push(crate::network::timing_hop(&url, &headers));
            if cross {
                let origins = headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"))
                    .map(|(_, value)| value.trim())
                    .collect::<Vec<_>>();
                if origins.len() != 1 || !(origins[0] == "*" || origins[0] == request_origin) {
                    return Err("external template CORS response denied".into());
                }
            }
            reports.push_coep(coep.response(crate::coep_reporting::ResponsePolicyInputs {
                origin: &origin.ascii_serialization(),
                opaque: matches!(origin, url::Origin::Opaque(_)),
                target: &url,
                cors: true,
                credentials,
                headers: &headers,
                destination: "script",
            }));
            isolation.authorize_response(
                &origin.ascii_serialization(),
                matches!(origin, url::Origin::Opaque(_)),
                &url,
                true,
                credentials,
                &headers,
            )?;
            if oryn_net::fetch::is_redirect_status(status) {
                let _ = broker.cancel_operation(id);
                self.active.store(0, std::sync::atomic::Ordering::Release);
                if hop == 10 {
                    return Err("external template redirect limit".into());
                }
                let next = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .and_then(|(_, value)| url.join(value).ok())
                    .ok_or("external template redirect location")?;
                if !matches!(next.scheme(), "http" | "https")
                    || !next.username().is_empty()
                    || next.password().is_some()
                {
                    return Err("external template CORS redirect URL unavailable".into());
                }
                if url.origin() != next.origin() && url.origin() != origin {
                    opaque = true;
                    request_origin = "null".into()
                }
                referrer_policy = referrer_policy.after_response(&headers);
                url = next;
                continue;
            }
            if !(200..300).contains(&status) {
                return Err("external template HTTP status".into());
            }
            state.details = Some(crate::network::resource_timing_metadata(
                &origin.ascii_serialization(),
                matches!(origin, url::Origin::Opaque(_)),
                timing_hops.clone(),
                &crate::network::NetworkResponse {
                    response_policy: None,
                    request_id: id,
                    status,
                    headers: headers.clone(),
                    body: Vec::new(),
                    timing,
                },
                state
                    .started
                    .unwrap()
                    .elapsed()
                    .as_micros()
                    .min(u64::MAX as u128) as u64,
            ));
            state.operation = Some(operation);
            response_headers = Some(headers);
            break;
        }
        let headers = response_headers.ok_or("external template missing final response")?;
        state.stream_id = Some(request.request_id);
        if !request.integrity.is_empty() {
            let mut body = Vec::new();
            loop {
                let (bytes, done) = self.chunk(&mut state)?;
                body.extend(bytes);
                if done {
                    break;
                }
            }
            crate::integrity::verify(&request.integrity, &body, true)
                .map_err(|error| format!("external template integrity: {error:?}"))?;
            state.buffer = body.into();
            state.done = true;
        }
        self.check()?;
        Ok(TemplateResponse::Started {
            request_id: request.request_id,
            stream_id: request.request_id,
            status: 200,
            headers,
        })
    }
    fn chunk(&self, state: &mut TemplateStream) -> Result<(Vec<u8>, bool), String> {
        use crate::network::NetworkEvent;
        self.check()?;
        if !state.buffer.is_empty() {
            let count = state.buffer.len().min(64 * 1024);
            let bytes = state.buffer.drain(..count).collect();
            let done = state.done && state.buffer.is_empty();
            if done {
                Self::finish_timing(state, None)
            }
            return Ok((bytes, done));
        }
        if state.done {
            Self::finish_timing(state, None);
            return Ok((Vec::new(), true));
        }
        let operation = state
            .operation
            .as_mut()
            .ok_or("external template operation missing")?;
        operation.request_body_chunk();
        let expected = operation.request_id;
        match self.event(operation)? {
            NetworkEvent::Chunk { request_id, bytes }
                if request_id == expected && !bytes.is_empty() =>
            {
                state.bytes = state
                    .bytes
                    .checked_add(bytes.len())
                    .filter(|bytes| *bytes <= 8 * 1024 * 1024)
                    .ok_or("external template body limit")?;
                state.buffer.extend(bytes.iter().copied());
                self.chunk(state)
            }
            NetworkEvent::Complete {
                request_id,
                encoded_body_size,
            } if request_id == expected => {
                Self::finish_timing(state, encoded_body_size);
                state.done = true;
                state.operation = None;
                self.active.store(0, std::sync::atomic::Ordering::Release);
                Ok((Vec::new(), true))
            }
            NetworkEvent::Failed { request_id, error } if request_id == expected => Err(error),
            _ => Err("external template body identity/order".into()),
        }
    }
    fn finish_timing(state: &mut TemplateStream, encoded: Option<u64>) {
        if let Some(mut timing) = state.timing.take()
            && let Some(mut details) = state.details.take()
        {
            details.decoded_body_size = Some(state.bytes as u64);
            details.encoded_body_size = encoded
                .or(details
                    .transport
                    .as_ref()
                    .and_then(|timing| timing.encoded_body_size))
                .or_else(|| details.hops.is_empty().then_some(state.bytes as u64));
            details.response_end_micros = state
                .started
                .map(|started| started.elapsed().as_micros().min(u64::MAX as u128) as u64);
            timing.set_details(details);
        }
    }
    fn read(&self, stream_id: u64) -> Result<TemplateResponse, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "external template stream poisoned")?;
        if state.stream_id != Some(stream_id) {
            return Err("external template stream identity".into());
        }
        let (bytes, done) = self.chunk(&mut state)?;
        Ok(TemplateResponse::Chunk {
            stream_id,
            bytes,
            done,
        })
    }
}
impl Drop for TemplateStandalone {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod template_transport_tests {
    use super::*;
    use crate::network::{
        NetworkBroker, NetworkError, NetworkEvent, NetworkOperation, NetworkRequest,
        NetworkResponse,
    };
    #[derive(Clone)]
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        chunks: Vec<Vec<u8>>,
        wrong_identity: bool,
        hold_open: bool,
    }
    #[derive(Clone, Default)]
    struct Probe {
        replies: Arc<std::sync::Mutex<std::collections::VecDeque<Reply>>>,
        requests: Arc<std::sync::Mutex<Vec<NetworkRequest>>>,
        cancelled: Arc<std::sync::Mutex<Vec<u64>>>,
        held: Arc<std::sync::Mutex<Vec<tokio::sync::mpsc::Sender<NetworkEvent>>>>,
    }
    impl NetworkBroker for Probe {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            panic!("template transport must retain native operation custody")
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let id = request.request_id;
            self.requests.lock().unwrap().push(request);
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("expected admitted request");
            let response_id = if reply.wrong_identity { id + 1 } else { id };
            let mut prefetched = std::collections::VecDeque::from([NetworkEvent::Headers {
                response_policy: None,
                request_id: response_id,
                status: reply.status,
                headers: reply.headers,
                content_length: None,
                upload: None,
                timing: None,
            }]);
            let mut size = 0;
            for bytes in reply.chunks {
                size += bytes.len();
                prefetched.push_back(NetworkEvent::Chunk {
                    request_id: response_id,
                    bytes: bytes.into(),
                });
            }
            if !reply.hold_open {
                prefetched.push_back(NetworkEvent::Complete {
                    request_id: response_id,
                    encoded_body_size: Some(size as u64),
                });
            }
            let (sender, events) = tokio::sync::mpsc::channel(1);
            if reply.hold_open {
                self.held.lock().unwrap().push(sender);
            }
            Ok(NetworkOperation {
                request_id: id,
                events,
                demand: None,
                body_demand_pending: std::cell::Cell::new(false),
                prefetched,
            })
        }
        fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
            self.cancelled.lock().unwrap().push(id);
            Ok(())
        }
    }
    fn reply(body: &[u8]) -> Reply {
        Reply {
            status: 200,
            headers: vec![("content-type".into(), "text/html;charset=utf-8".into())],
            chunks: vec![body.to_vec()],
            wrong_identity: false,
            hold_open: false,
        }
    }
    fn request(url: &str) -> TemplateRequest {
        TemplateRequest {
            request_id: 77,
            url: url.into(),
            initiator: "https://example.test/path/document".into(),
            nonce: None,
            integrity: String::new(),
            referrer_policy: "strict-origin-when-cross-origin".into(),
        }
    }
    fn backend(probe: &Probe, policy: Option<&str>) -> ModuleGraphBackend {
        let base = url::Url::parse("https://example.test/path/document").unwrap();
        let csp = policy.map(|source| {
            let policy = crate::csp::CspPolicySet::from_headers(
                base.as_str(),
                &[("content-security-policy".into(), source.into())],
            );
            oryn_js::v8_bindings::ScriptPolicySnapshot::new(
                serde_json::to_string(&policy).unwrap(),
                policy,
            )
            .unwrap()
        });
        ModuleGraphBackend::Broker {
            coep: Default::default(),
            broker: crate::network::SharedNetworkBroker::new(probe.clone()),
            base: base.clone(),
            initiator: base.to_string(),
            isolation: Default::default(),
            csp,
            reports: Arc::new(crate::executable::module_graph_load::ScriptCspReports::default()),
        }
    }
    fn template_owner(probe: &Probe, policy: Option<&str>) -> TemplateStandalone {
        let backend = backend(probe, policy);
        let ModuleGraphBackend::Broker { broker, .. } = &backend else {
            unreachable!()
        };
        TemplateStandalone::new(backend.clone(), Some(broker.clone().new_request_scope()))
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
    fn broker_stream_keeps_headers_and_each_bounded_body_read_separate() {
        let probe = Probe::default();
        let mut response = reply(b"first");
        response.chunks.push(vec![b'x'; 65537]);
        probe.replies.lock().unwrap().push_back(response);
        let owner = template_owner(&probe, None);
        assert!(matches!(
            owner
                .start(request("https://example.test/fragment"))
                .unwrap(),
            TemplateResponse::Started { request_id: 77, .. }
        ));
        assert_eq!(owner.state.lock().unwrap().bytes, 0);
        assert!(
            matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:false,..}if bytes==b"first")
        );
        assert!(
            matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:false,..}if bytes.len()==65536)
        );
        assert!(
            matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:false,..}if bytes.len()==1)
        );
        assert!(matches!(
            owner.read(77).unwrap(),
            TemplateResponse::Chunk { done: true, .. }
        ));
        assert!(owner.read(78).is_err());
        let requests = probe.requests.lock().unwrap();
        assert_eq!(
            requests[0].security.kind,
            crate::cookies::CookieRequestKind::Script
        );
        assert_eq!(
            requests[0].security.fetch_metadata.mode,
            Some(crate::fetch_metadata::Mode::Cors)
        );
        assert_eq!(
            requests[0].security.fetch_metadata.destination.as_deref(),
            Some("script")
        );
        assert!(requests[0].credentials);
        assert!(probe.cancelled.lock().unwrap().is_empty());
    }
    #[test]
    fn cors_nonce_and_script_policy_are_independent_gates() {
        for (allow, nonce, policy, accepted) in [
            (
                true,
                Some("correctnonce"),
                "script-src 'nonce-correctnonce';connect-src 'none'",
                true,
            ),
            (
                true,
                Some("wrongnonce"),
                "script-src 'nonce-correctnonce'",
                false,
            ),
            (true, None, "script-src 'self'", false),
            (
                false,
                Some("correctnonce"),
                "script-src 'nonce-correctnonce'",
                false,
            ),
        ] {
            let probe = Probe::default();
            let mut response = reply(b"<b>ok</b>");
            if allow {
                response
                    .headers
                    .push(("access-control-allow-origin".into(), "*".into()))
            }
            probe.replies.lock().unwrap().push_back(response);
            let owner = template_owner(&probe, Some(policy));
            let mut req = request("https://other.test/fragment");
            req.nonce = nonce.map(str::to_owned);
            let result = owner.start(req);
            assert_eq!(result.is_ok(), accepted);
            if accepted {
                assert!(!probe.requests.lock().unwrap()[0].credentials);
            } else {
                owner.cancel();
            }
        }
    }
    #[test]
    fn integrity_uses_original_bytes_and_never_publishes_mismatched_input() {
        let raw = b"<b>\xe9</b>";
        for valid in [false, true] {
            let probe = Probe::default();
            probe.replies.lock().unwrap().push_back(reply(raw));
            let owner = template_owner(&probe, None);
            let mut req = request("https://example.test/fragment");
            req.integrity = digest(if valid { raw } else { b"other" });
            assert_eq!(owner.start(req).is_ok(), valid);
            if valid {
                assert!(
                    matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:true,..}if bytes==raw)
                );
            } else {
                owner.cancel();
            }
        }
    }
    #[test]
    fn redirects_update_referrer_and_taint_origin_without_credential_escalation() {
        let probe = Probe::default();
        let mut redirect = reply(b"");
        redirect.status = 302;
        redirect.headers.extend([
            ("location".into(), "https://last.test/fragment".into()),
            ("access-control-allow-origin".into(), "*".into()),
            ("referrer-policy".into(), "no-referrer".into()),
        ]);
        let mut final_reply = reply(b"ok");
        final_reply
            .headers
            .push(("access-control-allow-origin".into(), "null".into()));
        probe
            .replies
            .lock()
            .unwrap()
            .extend([redirect, final_reply]);
        let owner = template_owner(&probe, None);
        let mut req = request("https://other.test/redirect");
        req.referrer_policy = "origin".into();
        owner.start(req).unwrap();
        let requests = probe.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0]
                .headers
                .contains(&("referer".into(), "https://example.test/".into()))
        );
        assert!(
            requests[1]
                .headers
                .contains(&("origin".into(), "null".into()))
        );
        assert!(
            !requests[1]
                .headers
                .iter()
                .any(|(name, _)| name == "referer")
        );
        assert!(requests.iter().all(|request| !request.credentials));
    }
    #[test]
    fn cancellation_and_invalid_response_identity_never_return_body() {
        // Keep transport custody live: a prefetched Complete can retire the native
        // identity before cancellation even when the consumer has not read its body.
        let probe = Probe::default();
        let mut response = reply(b"late");
        response.hold_open = true;
        probe.replies.lock().unwrap().push_back(response);
        let owner = template_owner(&probe, None);
        owner
            .start(request("https://example.test/fragment"))
            .unwrap();
        assert!(owner.broker.as_ref().unwrap().owned_operation_activity().0 > 0);
        owner.cancel();
        assert!(owner.read(77).is_err());
        assert!(!probe.cancelled.lock().unwrap().is_empty());
        owner.cancel();
        owner
            .broker
            .as_ref()
            .unwrap()
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            *probe.cancelled.lock().unwrap(),
            vec![probe.requests.lock().unwrap()[0].request_id]
        );
        let probe = Probe::default();
        let mut response = reply(b"wrong");
        response.wrong_identity = true;
        probe.replies.lock().unwrap().push_back(response);
        let owner = template_owner(&probe, None);
        assert!(
            owner
                .start(request("https://example.test/fragment"))
                .is_err()
        );
        owner.cancel();
    }
    #[test]
    fn completed_template_response_does_not_reacquire_transport_cancellation() {
        let probe = Probe::default();
        probe.replies.lock().unwrap().push_back(reply(b"complete"));
        let owner = template_owner(&probe, None);
        owner
            .start(request("https://example.test/fragment"))
            .unwrap();
        assert!(
            matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:false,..}if bytes==b"complete")
        );
        assert!(matches!(
            owner.read(77).unwrap(),
            TemplateResponse::Chunk { done: true, .. }
        ));
        owner
            .broker
            .as_ref()
            .unwrap()
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        owner.cancel();
        assert!(owner.read(77).is_err());
        assert!(probe.cancelled.lock().unwrap().is_empty());
    }
    #[test]
    fn local_sources_require_exact_same_origin_and_cannot_claim_raw_integrity() {
        let sources = Arc::new(BTreeMap::from([
            (
                "https://example.test/fragment".into(),
                "<b>local</b>".into(),
            ),
            (
                "https://other.test/fragment".into(),
                "untrusted cross-origin".into(),
            ),
            ("fragment".into(), "filename fallback forbidden".into()),
        ]));
        let backend = ModuleGraphBackend::Sources(
            sources,
            None,
            Arc::new(crate::executable::module_graph_load::ScriptCspReports::default()),
        );
        for (url, integrity, accepted) in [
            ("https://example.test/fragment", String::new(), true),
            ("https://other.test/fragment", String::new(), false),
            ("https://example.test/else/fragment", String::new(), false),
            (
                "https://example.test/fragment",
                digest(b"<b>local</b>"),
                false,
            ),
        ] {
            let owner = TemplateStandalone::new(backend.clone(), None);
            let mut req = request(url);
            req.integrity = integrity;
            assert_eq!(owner.start(req).is_ok(), accepted);
        }
        let owner = TemplateStandalone::new(backend, None);
        let mut req = request("data:text/html,%3Cb%3Eraw%3C/b%3E");
        req.integrity = digest(b"<b>raw</b>");
        assert!(owner.start(req).is_ok());
        assert!(
            matches!(owner.read(77).unwrap(),TemplateResponse::Chunk{bytes,done:true,..}if bytes==b"<b>raw</b>")
        );
    }
}
