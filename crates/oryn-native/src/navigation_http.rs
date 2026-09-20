//! One retained HTTP navigation transaction, including redirect policy waits.
//! The page owner checks native admission before every poll and alone commits
//! the returned response into a Document or download.
#[cfg(unix)]
use super::navigation_fetch::PendingPolicy;
use super::navigation_fetch::{Headers, PendingHeaders};
use super::*;
use crate::network::NetworkOperation;
use std::task::Poll;

pub(super) struct HttpNavigation {
    timing: oryn_js::navigation_timing::NavigationTiming,
    prerender: bool,
    broker: SharedNetworkBroker,
    #[cfg(unix)]
    client: Option<SharedContextClient>,
    page_id: PageId,
    #[cfg(unix)]
    source: crate::context_service::NavigationPolicySource,
    current: Url,
    original_url: String,
    post: Option<oryn_js::navigation::PostResource>,
    form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    form_navigation: bool,
    cookie_context: crate::cookies::CookieRequestContext,
    initial_site: Option<crate::cookies::SchemefulSite>,
    initial_origin: url::Origin,
    redirect_cross_site: bool,
    cross_origin_redirect: bool,
    origin_source: Option<Url>,
    request_origin: String,
    referrer_source: Option<Url>,
    referrer_policy: crate::referrer::ReferrerPolicy,
    redirects: usize,
    max_redirects: usize,
    deadline: Option<Instant>,
    phase: Phase,
    started: Option<(String, String)>,
}

enum Phase {
    Admission,
    #[cfg(unix)]
    SourcePolicy(PendingPolicy),
    Start,
    Headers(PendingHeaders),
    #[cfg(unix)]
    FormPolicy(PendingPolicy),
    #[cfg(unix)]
    NavigationPolicy(PendingPolicy),
    Finished,
}

pub(super) struct Response<H = Headers> {
    #[cfg(unix)]
    pub response_policy_cleanup: Option<Arc<crate::context_service::ContextResponsePolicyLease>>,
    pub timing: oryn_js::navigation_timing::NavigationTiming,
    pub request_state: NavigationRequestState,
    pub broker: SharedNetworkBroker,
    pub current: Url,
    pub post: Option<oryn_js::navigation::PostResource>,
    pub cross_origin_redirect: bool,
    pub headers: H,
}

/// The actor keeps response metadata while prefix inspection owns the body.
pub(super) type SniffedHeaders = (
    u16,
    Vec<(String, String)>,
    Option<oryn_net::TransportTiming>,
    Instant,
    Instant,
    Option<crate::network::ResponsePolicyToken>,
);
impl Response {
    pub fn split_body(self) -> (Response<SniffedHeaders>, NetworkOperation) {
        let Self {
            #[cfg(unix)]
            response_policy_cleanup,
            timing,
            request_state,
            broker,
            current,
            post,
            cross_origin_redirect,
            headers,
        } = self;
        let Headers {
            response_policy,
            status,
            headers,
            operation,
            timing: transport,
            started,
            received,
        } = headers;
        (
            Response {
                #[cfg(unix)]
                response_policy_cleanup,
                timing,
                request_state,
                broker,
                current,
                post,
                cross_origin_redirect,
                headers: (
                    status,
                    headers,
                    transport,
                    started,
                    received,
                    response_policy,
                ),
            },
            operation,
        )
    }
}
impl Response<SniffedHeaders> {
    pub fn restore_body(self, operation: NetworkOperation) -> Response {
        let Self {
            #[cfg(unix)]
            response_policy_cleanup,
            timing,
            request_state,
            broker,
            current,
            post,
            cross_origin_redirect,
            headers: (status, headers, transport, started, received, response_policy),
        } = self;
        Response {
            #[cfg(unix)]
            response_policy_cleanup,
            timing,
            request_state,
            broker,
            current,
            post,
            cross_origin_redirect,
            headers: Headers {
                response_policy,
                status,
                headers,
                operation,
                timing: transport,
                started,
                received,
            },
        }
    }
}

impl HttpNavigation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        broker: SharedNetworkBroker,
        #[cfg(unix)] client: Option<SharedContextClient>,
        page_id: PageId,
        #[cfg(unix)] source: crate::context_service::NavigationPolicySource,
        current: Url,
        post: Option<oryn_js::navigation::PostResource>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        cookie_context: crate::cookies::CookieRequestContext,
        referrer_source: Option<Url>,
        referrer_policy: crate::referrer::ReferrerPolicy,
        max_redirects: usize,
        deadline: Option<Instant>,
    ) -> Self {
        let origin_source = cookie_context
            .initiator_url
            .as_deref()
            .and_then(|value| Url::parse(value).ok());
        let request_origin = if cookie_context.fetch_metadata.opaque_initiator {
            "null".into()
        } else {
            origin_source
                .as_ref()
                .map(|source| source.origin().ascii_serialization())
                .unwrap_or_else(|| "null".into())
        };
        Self {
            origin_source,
            request_origin,
            timing: oryn_js::navigation_timing::NavigationTiming::new(
                Instant::now(),
                oryn_js::navigation_timing::Kind::Navigate,
            ),
            prerender: false,
            broker,
            #[cfg(unix)]
            client,
            page_id,
            #[cfg(unix)]
            source,
            initial_site: crate::cookies::SchemefulSite::from_url(&current),
            initial_origin: current.origin(),
            original_url: current.to_string(),
            current,
            form_navigation: form_authority.is_some() || post.is_some(),
            post,
            form_authority,
            cookie_context,
            redirect_cross_site: false,
            cross_origin_redirect: false,
            referrer_source,
            referrer_policy,
            redirects: 0,
            max_redirects,
            deadline,
            phase: Phase::Admission,
            started: None,
        }
    }

    pub fn with_timing(mut self, timing: oryn_js::navigation_timing::NavigationTiming) -> Self {
        self.timing = timing;
        self
    }
    pub fn for_prerender(mut self) -> Self {
        self.prerender = true;
        self
    }

    pub fn take_started(&mut self) -> Option<(String, String)> {
        self.started.take()
    }

    #[cfg(unix)]
    fn navigation_policy(&mut self, client: SharedContextClient) -> Result<(), RuntimeError> {
        if matches!(
            &self.source,
            crate::context_service::NavigationPolicySource::InitialAboutBlank
        ) {
            self.phase = Phase::SourcePolicy(PendingPolicy::start(
                client,
                crate::context_service::ContextServiceRequest::CaptureCommandNavigationSource {
                    page_id: self.page_id,
                    initial_only: true,
                },
                self.deadline,
            )?);
            return Ok(());
        }
        self.phase = Phase::NavigationPolicy(PendingPolicy::start(
            client,
            crate::context_service::ContextServiceRequest::AuthorizeNavigation {
                page_id: self.page_id,
                source: self.source.clone(),
                url: self.current.to_string(),
            },
            self.deadline,
        )?);
        Ok(())
    }

    pub fn poll(&mut self) -> Poll<Result<Response, RuntimeError>> {
        let result = self.step();
        if matches!(result, Poll::Ready(_)) {
            self.phase = Phase::Finished;
        }
        result
    }

    fn step(&mut self) -> Poll<Result<Response, RuntimeError>> {
        if matches!(self.phase, Phase::Finished) {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        if self.deadline.is_some_and(|limit| Instant::now() >= limit) {
            return Poll::Ready(Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            }));
        }
        let phase = std::mem::replace(&mut self.phase, Phase::Finished);
        match phase {
            Phase::Admission => {
                #[cfg(unix)]
                if let Some(client) = self.client.clone() {
                    self.navigation_policy(client)?;
                    return Poll::Pending;
                }
                self.phase = Phase::Start;
                Poll::Pending
            }
            #[cfg(unix)]
            Phase::SourcePolicy(mut pending) => match pending.poll() {
                Poll::Pending => {
                    self.phase = Phase::SourcePolicy(pending);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::CommandNavigationSource {
                        creator,
                    },
                )) => {
                    self.source = crate::context_service::NavigationPolicySource::Document(creator);
                    let client = self
                        .client
                        .clone()
                        .ok_or(RuntimeError::NavigationCancelled)?;
                    self.navigation_policy(client)?;
                    Poll::Pending
                }
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "wrong native command source response".into(),
                ))),
            },
            Phase::Start => {
                let request_id = u64::try_from(self.redirects)
                    .ok()
                    .and_then(|id| id.checked_add(1))
                    .ok_or_else(|| RuntimeError::IdentityExhausted {
                        namespace: "navigation request identity".into(),
                    })?;
                let mut headers = vec![(
                    "accept".into(),
                    "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
                )];
                if self.prerender {
                    headers.push(("sec-purpose".into(), "prefetch;prerender".into()));
                }
                let referrer = self
                    .referrer_source
                    .as_ref()
                    .and_then(|source| self.referrer_policy.header(source, &self.current));
                self.referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
                if let Some(referrer) = referrer {
                    headers.push(("referer".into(), referrer));
                }
                if let Some(post) = &self.post {
                    headers.push(("content-type".into(), post.content_type.clone()));
                }
                let method = if self.post.is_some() { "POST" } else { "GET" }.to_owned();
                if let Some(value) = crate::referrer::request_origin_header(
                    &self.request_origin,
                    self.origin_source.as_ref().unwrap_or(&self.current),
                    &self.current,
                    &method,
                    false,
                    self.referrer_policy,
                ) {
                    headers.push(("origin".into(), value));
                }
                let url = self.current.to_string();
                self.cookie_context
                    .fetch_metadata
                    .observe(self.cookie_context.initiator_url.as_deref(), &self.current);
                let request = NetworkRequest {
                    request_id,
                    method: method.clone(),
                    url: url.clone(),
                    headers,
                    body: self
                        .post
                        .as_ref()
                        .map_or_else(Vec::new, |post| post.bytes.clone()),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        dns_timing: None,
                        redirect_chain_cross_site: self.redirect_cross_site,
                        ..self.cookie_context.clone()
                    },
                    resolved_addrs: Vec::new(),
                };
                self.phase = Phase::Headers(PendingHeaders::start(
                    self.broker.clone(),
                    request,
                    self.deadline,
                    #[cfg(unix)]
                    self.client.as_ref(),
                )?);
                self.started = Some((method, url));
                Poll::Pending
            }
            Phase::Headers(mut pending) => match pending.poll() {
                Poll::Pending => {
                    self.phase = Phase::Headers(pending);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(headers)) => {
                    #[cfg(unix)]
                    let response_policy_cleanup = headers
                        .response_policy
                        .as_deref()
                        .map(|token| {
                            self.client
                                .as_ref()
                                .ok_or_else(|| {
                                    RuntimeError::ProtocolViolation(
                                        "navigation response grant requires a context client"
                                            .into(),
                                    )
                                })?
                                .retain_response_policy(self.page_id, token)
                        })
                        .transpose()?;
                    if !matches!(headers.status, 301 | 302 | 303 | 307 | 308) {
                        if let Some(start) =
                            crate::network::native_worker_start(headers.timing.as_ref())
                        {
                            self.timing.worker_start(start);
                        }
                        let details = oryn_js::platform::ResourceTimingMetadata {
                            server_timing: oryn_js::server_timing::parse(&headers.headers),
                            response_end_micros: None,
                            navigation_phases: None,
                            producer_clock_micros: None,
                            decoded_body_size: None,
                            origin: self.current.origin().ascii_serialization(),
                            hops: Vec::new(),
                            transport: headers
                                .timing
                                .clone()
                                .filter(|timing| timing.valid_snapshot()),
                            allow_details: true,
                            response_status: headers.status,
                            response_start_micros: headers
                                .timing
                                .as_ref()
                                .filter(|timing| timing.measured_response)
                                .map_or_else(
                                    || {
                                        headers
                                            .received
                                            .saturating_duration_since(headers.started)
                                            .as_micros()
                                            .min(u128::from(u64::MAX))
                                            as u64
                                    },
                                    |timing| timing.response_start,
                                ),
                            encoded_body_size: headers
                                .timing
                                .as_ref()
                                .and_then(|timing| timing.encoded_body_size)
                                .or_else(|| {
                                    headers
                                        .headers
                                        .iter()
                                        .find(|(name, _)| {
                                            name.eq_ignore_ascii_case("content-length")
                                        })
                                        .and_then(|(_, value)| value.parse().ok())
                                }),
                            content_type: oryn_common::mime_type::MimeType::from_headers(
                                &headers.headers,
                            )
                            .map(|mime| mime.essence().to_owned())
                            .unwrap_or_default(),
                        };
                        self.timing
                            .response(oryn_js::navigation_timing::NavigationResponse {
                                url: self.current.to_string(),
                                status: headers.status,
                                headers: &headers.headers,
                                fetch: headers.started,
                                received: headers.received,
                                details: Some(details),
                                no_referrer: self.referrer_policy.as_str() == "no-referrer",
                            })
                            .map_err(RuntimeError::Network)?;
                        return Poll::Ready(Ok(Response {
                            #[cfg(unix)]
                            response_policy_cleanup,
                            timing: self.timing.clone(),
                            request_state: NavigationRequestState {
                                initiator_url: self
                                    .cookie_context
                                    .initiator_url
                                    .clone()
                                    .filter(|value| Url::parse(value).is_ok()),
                                opaque_initiator: self
                                    .cookie_context
                                    .fetch_metadata
                                    .opaque_initiator,
                                referrer: self.referrer_source.as_ref().map(ToString::to_string),
                                referrer_policy: self.referrer_policy.as_str().into(),
                            },
                            broker: self.broker.clone(),
                            current: self.current.clone(),
                            post: self.post.take(),
                            cross_origin_redirect: self.cross_origin_redirect,
                            headers,
                        }));
                    }
                    self.timing
                        .redirect(
                            self.current.to_string(),
                            &headers.headers,
                            headers.started,
                            headers.received,
                        )
                        .map_err(RuntimeError::Network)?;
                    self.referrer_policy = self.referrer_policy.after_response(&headers.headers);
                    oryn_js::navigation::PostResource::redirect(&mut self.post, headers.status);
                    let location = headers
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| RuntimeError::Network("redirect omitted Location".into()))?;
                    // Dropping this exact response starts retained cancellation;
                    // no synchronous cancellation RPC blocks a page task.
                    drop(headers);
                    self.redirects = self
                        .redirects
                        .checked_add(1)
                        .ok_or_else(|| RuntimeError::Network("redirect count exhausted".into()))?;
                    if self.redirects > self.max_redirects {
                        return Poll::Ready(Err(RuntimeError::Network(format!(
                            "redirect limit ({}) exceeded",
                            self.max_redirects
                        ))));
                    }
                    let next = self.current.join(&location).map_err(|error| {
                        RuntimeError::Network(format!("invalid redirect: {error}"))
                    })?;
                    if self.current.origin() != next.origin()
                        && self
                            .origin_source
                            .as_ref()
                            .is_none_or(|source| source.origin() != self.current.origin())
                    {
                        self.request_origin = "null".into();
                    }
                    self.current = next;
                    if self.prerender && self.current.origin() != self.initial_origin {
                        return Poll::Ready(Err(RuntimeError::ProtocolViolation(
                            "prerender redirect left its admitted origin".into(),
                        )));
                    }
                    if self
                        .post
                        .as_ref()
                        .is_some_and(|post| !post.valid_for(&self.current))
                    {
                        return Poll::Ready(Err(RuntimeError::ProtocolViolation(
                            "invalid redirected POST resource".into(),
                        )));
                    }
                    self.redirect_cross_site |=
                        crate::cookies::SchemefulSite::from_url(&self.current) != self.initial_site;
                    self.cross_origin_redirect |= self.current.origin() != self.initial_origin;
                    #[cfg(unix)]
                    if let Some(client) = self.client.clone() {
                        if self.form_navigation {
                            self.phase=Phase::FormPolicy(PendingPolicy::start(client,crate::context_service::ContextServiceRequest::AuthorizeFormSubmission{report_source:Some(self.original_url.clone()),
                                authority:self.form_authority.clone(),page_id:self.page_id,url:self.current.to_string(),method:if self.post.is_some(){"POST"}else{"GET"}.into(),
                            },self.deadline)?);
                        } else {
                            self.navigation_policy(client)?;
                        }
                        return Poll::Pending;
                    }
                    self.phase = Phase::Start;
                    Poll::Pending
                }
            },
            #[cfg(unix)]
            Phase::FormPolicy(mut pending) => match pending.poll() {
                Poll::Pending => {
                    self.phase = Phase::FormPolicy(pending);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::FormSubmissionAuthorized,
                )) => {
                    self.navigation_policy(
                        self.client.clone().expect("owned redirect policy client"),
                    )?;
                    Poll::Pending
                }
                Poll::Ready(Ok(crate::context_service::ContextServiceResponse::PageDenied {
                    ..
                })) => Poll::Ready(Err(RuntimeError::PolicyDenied {
                    action: ConsequentialAction::FormSubmission,
                })),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "invalid redirected form authorization response".into(),
                ))),
            },
            #[cfg(unix)]
            Phase::NavigationPolicy(mut pending) => match pending.poll() {
                Poll::Pending => {
                    self.phase = Phase::NavigationPolicy(pending);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::NavigationAuthorized,
                )) => {
                    self.phase = Phase::Start;
                    Poll::Pending
                }
                Poll::Ready(Ok(crate::context_service::ContextServiceResponse::PageDenied {
                    ..
                })) => Poll::Ready(Err(RuntimeError::PolicyDenied {
                    action: ConsequentialAction::CrossOriginNavigation,
                })),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "parent returned the wrong redirect-authorization response".into(),
                ))),
            },
            Phase::Finished => Poll::Ready(Err(RuntimeError::NavigationCancelled)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkError, NetworkResponse};
    struct RedirectFixture {
        status: u16,
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for RedirectFixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let id = request.request_id;
            let first = request.url == "https://first.test/start";
            self.requests.lock().unwrap().push(request);
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: id,
                status: if first { self.status } else { 200 },
                headers: if first {
                    vec![
                        ("location".into(), "https://second.test/final".into()),
                        ("referrer-policy".into(), "no-referrer".into()),
                    ]
                } else {
                    Vec::new()
                },
                body: Vec::new(),
            })
        }
    }
    fn transaction(broker: SharedNetworkBroker, max_redirects: usize) -> HttpNavigation {
        let mut cookies = crate::cookies::CookieRequestContext::navigation(Some(
            "https://source.test/source".into(),
        ));
        cookies.initiator_url = Some("https://source.test/source".into());
        HttpNavigation::new(
            broker,
            #[cfg(unix)]
            None,
            PageId(1),
            #[cfg(unix)]
            crate::context_service::NavigationPolicySource::InitialAboutBlank,
            Url::parse("https://first.test/start").unwrap(),
            Some(oryn_js::navigation::PostResource {
                bytes: b"key=value".to_vec(),
                content_type: "application/x-www-form-urlencoded".into(),
            }),
            None,
            cookies,
            Some(Url::parse("https://source.test/source").unwrap()),
            crate::referrer::ReferrerPolicy::UnsafeUrl,
            max_redirects,
            Some(Instant::now() + Duration::from_secs(5)),
        )
    }
    #[cfg(unix)]
    #[test]
    fn initial_source_is_captured_once_and_retained_through_redirect_policy() {
        use crate::context_service::{
            CONTEXT_SERVICE_PROTOCOL_VERSION, ContextServiceRequest as Request,
            ContextServiceResponse as Response, IpcContextClient, NavigationPolicySource,
        };
        use std::io::{BufRead, Write};
        let (local, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 17,
            stream_epoch: 4,
        };
        let expected = creator.clone();
        let server = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
            for index in 0..3 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let envelope: serde_json::Value = serde_json::from_str(&line).unwrap();
                let request: Request = serde_json::from_value(envelope["request"].clone()).unwrap();
                let response = if index == 0 {
                    assert!(matches!(
                        request,
                        Request::CaptureCommandNavigationSource {
                            initial_only: true,
                            ..
                        }
                    ));
                    Response::CommandNavigationSource {
                        creator: expected.clone(),
                    }
                } else {
                    let Request::AuthorizeNavigation {
                        source: NavigationPolicySource::Document(actual),
                        url,
                        ..
                    } = request
                    else {
                        panic!("redirect lost captured source")
                    };
                    assert_eq!(actual, expected);
                    assert_eq!(
                        url,
                        if index == 1 {
                            "https://first.test/start"
                        } else {
                            "https://second.test/final"
                        }
                    );
                    Response::NavigationAuthorized
                };
                writeln!(peer,"{}",serde_json::json!({"protocol_version":CONTEXT_SERVICE_PROTOCOL_VERSION,"route":null,"correlation_id":envelope["message_id"],"response":response,"error":null})).unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                let receipt: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(receipt["accepted"], true);
            }
        });
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut fetch = transaction(
            SharedNetworkBroker::new(RedirectFixture {
                status: 302,
                requests: requests.clone(),
            }),
            10,
        );
        fetch.client = Some(SharedContextClient::new(IpcContextClient::new(local)));
        fetch.post = None;
        fetch.form_navigation = false;
        finish(&mut fetch).unwrap();
        assert!(
            matches!(&fetch.source,NavigationPolicySource::Document(actual)if actual==&creator)
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
        server.join().unwrap();
    }
    #[test]
    fn prerender_redirect_does_not_fetch_an_unadmitted_origin() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut fetch = transaction(
            SharedNetworkBroker::new(RedirectFixture {
                status: 302,
                requests: requests.clone(),
            }),
            10,
        )
        .for_prerender();
        fetch.post = None;
        fetch.form_navigation = false;
        assert!(matches!(
            finish(&mut fetch),
            Err(RuntimeError::ProtocolViolation(_))
        ));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(name, value)| name == "sec-purpose" && value == "prefetch;prerender")
        );
    }
    #[test]
    fn navigation_provenance_survives_redirect_method_and_referrer_changes() {
        for flags in [
            oryn_js::platform::NavigationRequestFlags {
                reload: true,
                history: false,
            },
            oryn_js::platform::NavigationRequestFlags {
                reload: false,
                history: true,
            },
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let mut fetch = transaction(
                SharedNetworkBroker::new(RedirectFixture {
                    status: 303,
                    requests: requests.clone(),
                }),
                10,
            );
            fetch.cookie_context.navigation = flags;
            finish(&mut fetch).unwrap();
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(
                requests
                    .iter()
                    .all(|request| request.security.navigation == flags)
            );
            assert_eq!(requests[1].method, "GET");
        }
    }
    fn finish(fetch: &mut HttpNavigation) -> Result<Response, RuntimeError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = fetch.poll();
            fetch.take_started();
            if let Poll::Ready(result) = result {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "HTTP state machine did not settle"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
    #[test]
    fn redirects_retain_source_policy_cookie_context_and_apply_post_rules() {
        for status in [303, 307] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let broker = SharedNetworkBroker::new(RedirectFixture {
                status,
                requests: requests.clone(),
            });
            let mut fetch = transaction(broker.clone(), 5);
            let response = finish(&mut fetch).unwrap();
            assert_eq!(response.current.as_str(), "https://second.test/final");
            assert!(response.cross_origin_redirect);
            assert_eq!(
                response.request_state.initiator_url.as_deref(),
                Some("https://source.test/source")
            );
            assert!(!response.request_state.opaque_initiator);
            assert_eq!(
                response.request_state.referrer, None,
                "no-referrer redirect persists suppression in history document state"
            );
            assert_eq!(
                response.request_state.referrer_policy.as_str(),
                "no-referrer"
            );
            assert_eq!(response.headers.status, 200);
            assert_eq!(response.post.is_some(), status == 307);
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].method, "POST");
            assert_eq!(requests[0].body, b"key=value");
            assert_eq!(
                requests[0]
                    .headers
                    .iter()
                    .find(|(name, _)| name == "origin")
                    .map(|(_, value)| value.as_str()),
                Some("https://source.test")
            );
            assert_eq!(
                requests[1]
                    .headers
                    .iter()
                    .find(|(name, _)| name == "origin")
                    .map(|(_, value)| value.as_str()),
                if status == 303 { None } else { Some("null") }
            );
            assert_eq!(
                requests[1].method,
                if status == 303 { "GET" } else { "POST" }
            );
            assert_eq!(requests[1].body.is_empty(), status == 303);
            assert!(!requests[0].security.redirect_chain_cross_site);
            assert!(requests[1].security.redirect_chain_cross_site);
            assert_eq!(
                requests[1].security.initiator_url,
                requests[0].security.initiator_url
            );
            assert!(requests[0].headers.iter().any(|(name,value)|name=="referer"&&value=="https://source.test/source"));
            assert!(
                !requests[1]
                    .headers
                    .iter()
                    .any(|(name, _)| name == "referer")
            );
            assert_ne!(requests[0].request_id, requests[1].request_id);
            assert!(matches!(
                fetch.poll(),
                Poll::Ready(Err(RuntimeError::NavigationCancelled))
            ));
        }
    }
    #[test]
    fn navigation_origin_cannot_be_recovered_from_referrer_after_opaque_capture() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(RedirectFixture {
            status: 307,
            requests: requests.clone(),
        });
        let mut context = crate::cookies::CookieRequestContext::navigation(Some(
            "https://source.test/source".into(),
        ));
        context.fetch_metadata.opaque_initiator = true;
        let mut fetch = HttpNavigation::new(
            broker,
            #[cfg(unix)]
            None,
            PageId(1),
            #[cfg(unix)]
            crate::context_service::NavigationPolicySource::InitialAboutBlank,
            Url::parse("https://first.test/start").unwrap(),
            Some(oryn_js::navigation::PostResource {
                bytes: vec![1],
                content_type: "text/plain".into(),
            }),
            None,
            context,
            Some(Url::parse("https://source.test/source").unwrap()),
            crate::referrer::ReferrerPolicy::UnsafeUrl,
            5,
            Some(Instant::now() + Duration::from_secs(5)),
        );
        finish(&mut fetch).unwrap();
        assert!(requests.lock().unwrap().iter().all(|request| {
            request
                .headers
                .iter()
                .find(|(name, _)| name == "origin")
                .map(|(_, value)| value.as_str())
                == Some("null")
        }));
    }
    #[test]
    fn redirect_limit_and_original_deadline_prevent_another_request() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(RedirectFixture {
            status: 307,
            requests: requests.clone(),
        });
        let mut fetch = transaction(broker.clone(), 0);
        assert!(
            matches!(finish(&mut fetch),Err(RuntimeError::Network(message)) if message.contains("redirect limit"))
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        let mut expired = transaction(broker, 5);
        expired.deadline = Some(Instant::now());
        assert!(matches!(
            expired.poll(),
            Poll::Ready(Err(RuntimeError::WorkerLimitExceeded { .. }))
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(matches!(
            expired.poll(),
            Poll::Ready(Err(RuntimeError::NavigationCancelled))
        ));
    }
}
