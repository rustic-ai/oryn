//! Asynchronous candidate reservation with custody across both IPC receipt and
//! the later IO-thread-to-actor handoff. Dropping either receiver cancels the
//! exact nonactive scope; it never waits for a parent response on the actor.

/// Inputs retained together for construct.
pub struct PrerenderConstructionInput<'a> {
    pub previous: oryn_js::realm::RealmId,
    pub client: SharedContextClient,
    pub page: PageId,
    pub max_heap: usize,
    pub deadline: Instant,
    pub host: &'a mut Option<crate::host::RawV8Host>,
    pub provisional: &'a mut Option<oryn_js::realm::RealmId>,
    pub terminal_control: Arc<crate::page_terminal_control::PageTerminalControl>,
}
use super::*;
use std::task::Poll;

pub(super) struct Reservation {
    pub page_id: PageId,
    pub client: SharedContextClient,
    pub destination: Option<crate::context_service::RelatedRootAdmission>,
    pub token: oryn_js::message_wire::PrerenderToken,
    pub generation: u64,
    pub epoch: u64,
    lease: crate::context_service::ContextPrerenderLease,
}
impl Reservation {
    /// Only after exact native promotion and actor ownership transfer.
    pub fn activated(self) {
        self.lease.activated();
    }
}
pub(super) struct PendingReservation {
    receiver: Option<oneshot::Receiver<Result<Reservation, RuntimeError>>>,
    deadline: Instant,
}
impl PendingReservation {
    pub fn start(
        client: SharedContextClient,
        page: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        target: Option<oryn_js::speculation_rules::TargetHint>,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        let lease = client.reserve_prerender_cleanup(page)?;
        let activity = client.track_resource_job()?;
        let (sender, receiver) = oneshot::channel();
        crate::executable::resource_io_executor::submit(move || {
            let _activity = activity;
            if sender.is_closed() {
                return;
            }
            let result = (|| {
                if Instant::now() >= deadline {
                    return Err(RuntimeError::NavigationCancelled);
                }
                let response = client.request(
                    if target == Some(oryn_js::speculation_rules::TargetHint::Blank) {
                        crate::context_service::ContextServiceRequest::ReserveBlankPrerender {
                            page_id: page,
                            creator,
                            url,
                        }
                    } else {
                        crate::context_service::ContextServiceRequest::ReservePrerender {
                            page_id: page,
                            creator,
                            url,
                        }
                    },
                )?;
                let (token, generation, epoch, page_id, destination) = match response {
                    crate::context_service::ContextServiceResponse::PrerenderReserved {
                        token,
                        generation,
                        epoch,
                    } => {
                        lease.attach(&token, generation, epoch);
                        (token, generation, epoch, page, None)
                    }
                    crate::context_service::ContextServiceResponse::BlankPrerenderReserved {
                        page_id,
                        admission,
                        token,
                        generation,
                        epoch,
                    } => {
                        lease.attach_blank(page_id, &token, generation, epoch);
                        (token, generation, epoch, page_id, Some(admission))
                    }
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "invalid prerender reservation response".into(),
                        ));
                    }
                };
                let endpoint = destination
                    .as_ref()
                    .map(|admission| client.related_endpoint(admission))
                    .transpose()?
                    .unwrap_or(client);
                if Instant::now() >= deadline {
                    return Err(RuntimeError::NavigationCancelled);
                }
                Ok(Reservation {
                    page_id,
                    client: endpoint,
                    destination,
                    token,
                    generation,
                    epoch,
                    lease,
                })
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| RuntimeError::Network(format!("prerender IO admission: {error}")))?;
        Ok(Self {
            receiver: Some(receiver),
            deadline,
        })
    }
    pub fn poll(&mut self) -> Poll<Result<Reservation, RuntimeError>> {
        if Instant::now() >= self.deadline {
            self.receiver.take();
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        let Some(receiver) = self.receiver.as_mut() else {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver.take();
                Poll::Ready(result)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Poll::Pending,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.receiver.take();
                Poll::Ready(Err(RuntimeError::Network(
                    "prerender reservation owner ended without result".into(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_service::{
        ContextServiceRequest as Q, ContextServiceResponse as R, ContextTransport,
    };
    struct Delayed {
        context: BrowserContext,
        page: PageId,
        entered: std::sync::mpsc::Sender<()>,
        released: Mutex<std::sync::mpsc::Receiver<()>>,
        cleaned: std::sync::mpsc::Sender<()>,
    }
    impl ContextTransport for Delayed {
        fn request(&self, request: Q) -> Result<R, RuntimeError> {
            match request {
                Q::ReservePrerender {
                    page_id,
                    creator,
                    url,
                } => {
                    assert_eq!(page_id, self.page);
                    let scope = self
                        .context
                        .reserve_native_prerender(page_id, &creator, &url)?;
                    self.entered.send(()).unwrap();
                    self.released
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    Ok(R::PrerenderReserved {
                        token: scope.token,
                        generation: scope.lifetime.0,
                        epoch: scope.lifetime.1,
                    })
                }
                Q::CancelPrerender {
                    page_id,
                    token,
                    generation,
                    epoch,
                } => {
                    self.context
                        .cancel_native_prerender(page_id, &token, (generation, epoch))?;
                    self.cleaned.send(()).unwrap();
                    Ok(R::PrerenderCancelled)
                }
                _ => Err(RuntimeError::ProtocolViolation(
                    "unexpected prerender fixture request".into(),
                )),
            }
        }
        fn request_routed(
            &self,
            _route: Option<&crate::worker_route::WorkerRoute>,
            request: Q,
        ) -> Result<R, RuntimeError> {
            self.request(request)
        }
    }
    #[test]
    fn dropped_actor_reservation_cancels_late_native_scope_after_wire_receipt() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (entered, wait_entered) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (cleaned, wait_cleaned) = std::sync::mpsc::channel();
        let transport = Arc::new(Delayed {
            context: context.clone(),
            page,
            entered,
            released: Mutex::new(released),
            cleaned,
        });
        let client = SharedContextClient::for_test_transport(
            transport,
            crate::worker_route::WorkerRoute::issue(774, page.0, 0).unwrap(),
        );
        let pending = PendingReservation::start(
            client,
            page,
            oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            "https://prerender.test/next".into(),
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        wait_entered.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(pending);
        release.send(()).unwrap();
        wait_cleaned.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .prerenders
                .is_empty()
        );
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some((1, 0))
        );
    }
}

pub(super) struct PreparedCandidate {
    pub reservation: Reservation,
    pub response: navigation_http::Response,
    pub content_type: String,
    pub policy: oryn_js::worker_wire::AgentPolicy,
    pub origin: oryn_js::worker_wire::AgentOrigin,
}
enum LoadPhase {
    Http(Box<navigation_http::HttpNavigation>),
    Mime {
        pending: crate::network::DocumentMimeSniff,
        response: navigation_http::Response<navigation_http::SniffedHeaders>,
    },
    Commit {
        pending: navigation_fetch::PendingPolicy,
        response: navigation_http::Response,
        content_type: String,
    },
    Finished,
}
/// Parent policy and HTTP/MIME preparation use the same bounded native lanes
/// as ordinary navigation, but never commit the active page or fire departure
/// callbacks. The actor checks the source-rule identity until blank-target
/// selection transfers the candidate to its destination scope.
pub(super) struct PendingCandidate {
    reservation: Option<Reservation>,
    client: SharedContextClient,
    page: PageId,
    phase: LoadPhase,
    deadline: Instant,
}
impl PendingCandidate {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        reservation: Reservation,
        client: SharedContextClient,
        page: PageId,
        creator: oryn_js::worker_wire::Creator,
        broker: SharedNetworkBroker,
        url: Url,
        cookie_context: crate::cookies::CookieRequestContext,
        referrer: Option<Url>,
        policy: crate::referrer::ReferrerPolicy,
        max_redirects: usize,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        // The final native destination owns transport before the first byte.
        // Keeping the source route here would cancel a claimed blank candidate
        // when its initiating document closes, even though its scope survives.
        let broker = match &reservation.destination {
            Some(admission) if admission.direct => reservation
                .client
                .related_network_broker(admission)?
                .ok_or_else(|| {
                    RuntimeError::Network("prerender destination broker unavailable".into())
                })?,
            Some(admission) => broker.for_route(admission.route.clone()),
            None => broker,
        };
        let _original_source = (client, page, creator);
        let client = reservation.client.clone();
        let page = reservation.page_id;
        let source = crate::context_service::NavigationPolicySource::Prerender {
            token: reservation.token.clone(),
            generation: reservation.generation,
            epoch: reservation.epoch,
        };
        let fetch = navigation_http::HttpNavigation::new(
            broker,
            Some(client.clone()),
            page,
            source,
            url,
            None,
            None,
            cookie_context,
            referrer,
            policy,
            max_redirects,
            Some(deadline),
        )
        .for_prerender();
        Ok(Self {
            reservation: Some(reservation),
            client,
            page,
            phase: LoadPhase::Http(Box::new(fetch)),
            deadline,
        })
    }
    pub fn poll(&mut self) -> Poll<Result<PreparedCandidate, RuntimeError>> {
        if Instant::now() >= self.deadline {
            self.phase = LoadPhase::Finished;
            self.reservation.take();
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        let phase = std::mem::replace(&mut self.phase, LoadPhase::Finished);
        let result = match phase {
            LoadPhase::Http(mut pending) => match pending.poll() {
                Poll::Pending => {
                    self.phase = LoadPhase::Http(pending);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(response)) => {
                    if !(200..300).contains(&response.headers.status)
                        || matches!(response.headers.status, 204 | 205)
                        || crate::download::response_download_filename(
                            &response.headers.headers,
                            &response.current,
                            None,
                        )
                        .is_some()
                    {
                        Poll::Ready(Err(RuntimeError::ProtocolViolation(
                            "prerender response cannot create a document".into(),
                        )))
                    } else {
                        let supplied = oryn_common::mime_type::MimeType::from_headers(
                            &response.headers.headers,
                        )
                        .map(|mime| mime.serialize());
                        let no_sniff = crate::network::response_no_sniff(&response.headers.headers);
                        let (response, operation) = response.split_body();
                        self.phase = LoadPhase::Mime {
                            pending: crate::network::DocumentMimeSniff::new(
                                operation, supplied, no_sniff,
                            ),
                            response,
                        };
                        Poll::Pending
                    }
                }
            },
            LoadPhase::Mime {
                mut pending,
                response,
            } => match pending.poll() {
                Poll::Pending => {
                    self.phase = LoadPhase::Mime { pending, response };
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(RuntimeError::Network(error))),
                Poll::Ready(Ok((operation, content_type))) => {
                    let response = response.restore_body(operation);
                    if crate::download::computed_type_is_download(&content_type) {
                        Poll::Ready(Err(RuntimeError::ProtocolViolation(
                            "prerender computed MIME requires download".into(),
                        )))
                    } else {
                        let reservation = self
                            .reservation
                            .as_ref()
                            .expect("prerender reservation retained");
                        match navigation_fetch::PendingPolicy::start_prerender_response(self.client.clone(),crate::context_service::ContextServiceRequest::CommitPrerenderResponse{response_policy:response.headers.response_policy.as_ref().map(ToString::to_string),page_id:self.page,token:reservation.token.clone(),generation:reservation.generation,epoch:reservation.epoch,url:response.current.to_string(),headers:response.headers.headers.clone()},Some(self.deadline)){
                            Ok(pending)=>{self.phase=LoadPhase::Commit{pending,response,content_type};Poll::Pending},
                            Err(error)=>Poll::Ready(Err(error)),
                        }
                    }
                }
            },
            LoadPhase::Commit {
                mut pending,
                response,
                content_type,
            } => match pending.poll() {
                Poll::Pending => {
                    self.phase = LoadPhase::Commit {
                        pending,
                        response,
                        content_type,
                    };
                    Poll::Pending
                }
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::PrerenderResponseCommitted {
                        policy,
                        origin,
                    },
                )) => {
                    if let Some(token) = &response.headers.response_policy {
                        token.committed();
                    }
                    Poll::Ready(Ok(PreparedCandidate {
                        reservation: self
                            .reservation
                            .take()
                            .expect("prerender reservation retained"),
                        response,
                        content_type,
                        policy,
                        origin,
                    }))
                }
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "prerender response policy ACK mismatch".into(),
                ))),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            },
            LoadPhase::Finished => Poll::Ready(Err(RuntimeError::NavigationCancelled)),
        };
        if matches!(&result, Poll::Ready(Err(_))) {
            self.reservation.take();
        }
        result
    }
}

/// Detached parser driver. Each actor turn lends it the same agent host, then
/// recovers that host before servicing the incumbent or another candidate.
pub(super) struct CandidateDocument {
    pub reservation: Reservation,
    pub executable: crate::executable::RustExecutableDocument,
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub broker: SharedNetworkBroker,
    pub csp: crate::csp::CspPolicySet,
    pub origin: oryn_js::origin::Origin,
    pub request_state: NavigationRequestState,
    pub capture: Arc<Mutex<NavigationBodyCapture>>,
    activation_available: bool,
}
impl PreparedCandidate {
    pub fn endpoint(&self) -> (SharedContextClient, PageId) {
        (self.reservation.client.clone(), self.reservation.page_id)
    }
    pub fn construct(
        self,
        params: PrerenderConstructionInput<'_>,
    ) -> Result<Box<CandidateDocument>, RuntimeError> {
        let PrerenderConstructionInput {
            previous,
            client,
            page,
            max_heap,
            deadline,
            host,
            provisional,
            terminal_control,
        } = params;

        let csp = crate::csp::CspPolicySet::from_agent_policy(&self.policy);
        let origin = oryn_js::origin::Origin::from_native_authority(
            &self.origin.serialized,
            self.origin.opaque.then_some(self.origin.authority.as_str()),
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.into()))?;
        let trusted = csp.trusted_types();
        let mut resources = BTreeMap::new();
        resources.insert(
            "oryn:csp-policy".into(),
            serde_json::to_string(&csp)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?,
        );
        resources.insert(
            "oryn:trusted-types".into(),
            serde_json::to_string(&oryn_js::trusted_types::TrustedTypesConfig {
                require_script: trusted.require_script,
                allowed_policy_names: trusted.policy_names,
                allow_any_policy: trusted.allow_any_policy,
                allow_duplicates: trusted.allow_duplicates,
                deny_policy_creation: trusted.deny_policy_creation,
            })
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?,
        );
        resources.insert(
            "oryn:csp-allow-eval".into(),
            csp.allows_eval(false).allowed.to_string(),
        );
        resources.insert(
            "oryn:response-policy-headers".into(),
            serde_json::to_string(&self.response.headers.headers)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?,
        );
        resources.insert(
            "oryn:script-isolation".into(),
            serde_json::to_string(
                &crate::agent_isolation::DocumentIsolationPolicy::from_headers(
                    &self.response.headers.headers,
                ),
            )
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?,
        );
        let referrer = crate::referrer::ReferrerPolicy::default()
            .after_response(&self.response.headers.headers);
        resources.insert("oryn:referrer-policy".into(), referrer.as_str().into());
        resources.insert(
            "oryn:document-referrer".into(),
            self.response
                .request_state
                .referrer
                .clone()
                .unwrap_or_default(),
        );
        let base = self.response.current.clone();
        // Load::start already binds the request broker to the reserved native
        // destination. Keep that exact owner with the executable while its
        // response body is live. Acquiring a fresh scope here drops the original
        // IdentityBroker and cancels the body before its terminal event.
        let broker = self.response.broker.clone();
        let realm = host
            .as_mut()
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation("candidate construction has no shared host".into())
            })?
            .create_prerender_root(crate::host::PrerenderRootInput {
                previous,
                broker: Some(broker.clone()),
                client: client.clone(),
                page_id: page,
                token: self.reservation.token.clone(),
                generation: self.reservation.generation,
                epoch: self.reservation.epoch,
            })
            .map_err(map_host_error)?;
        *provisional = Some(realm);
        let capture = Arc::new(Mutex::new(NavigationBodyCapture::default()));
        let encoding = Arc::new(Mutex::new(None));
        let navigation_http::Response {
            timing,
            request_state,
            headers:
                navigation_fetch::Headers {
                    status,
                    headers,
                    operation,
                    ..
                },
            ..
        } = self.response;
        let source = NavigationBodyChunks {
            timing: Some(timing),
            replay: Default::default(),
            operation,
            decoder: oryn_net::IncrementalTextDecoder::new(
                Some(&self.content_type),
                crate::network::navigation_text_kind(Some(&self.content_type)),
            ),
            capture: capture.clone(),
            encoding: encoding.clone(),
            deadline: Some(deadline),
            navigation: None,
            finished: false,
            demand_pending: false,
        };
        let content_type = self.content_type.clone();
        let stream = crate::host::DocumentStream {
            content_language: oryn_dom::language::content_language_header(
                headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                    .map(|(_, value)| value.as_str()),
            ),
            chunks: Box::new(source),
            content_type: Some(self.content_type),
            encoding,
        };
        let mut executable = crate::executable::RustExecutableDocument::load_for_page(
            crate::executable::PageDocumentLoadInput {
                parsed: None,
                source: Some(stream),
                html: "",
                resources: &resources,
                broker: Some(broker.clone()),
                base_url: Some(base.clone()),
                max_heap_bytes: max_heap,
                scheduled: true,
                inherited_origin: None,
                terminal_control,
                context: Some((client, page, None)),
                recovery: Some(host),
            },
        )
        .map_err(map_host_error)?;
        // Construction is scheduled: no parser bytes or author script have
        // been consumed before the actor receives this retained driver.
        *host = Some(executable.take_agent_host().map_err(map_host_error)?);
        *provisional = None;
        Ok(assemble_candidate_document(
            executable,
            CandidateParts {
                reservation: self.reservation,
                url: base.to_string(),
                status,
                content_type,
                headers,
                broker,
                csp,
                origin,
                request_state,
                capture,
            },
        ))
    }
}
// Keep the final CandidateDocument allocation frame after load_in_agent has
// returned and the unique host is recovered. The executable return slot remains
// part of the existing by-value construction API.
struct CandidateParts {
    reservation: Reservation,
    url: String,
    status: u16,
    content_type: String,
    headers: Vec<(String, String)>,
    broker: SharedNetworkBroker,
    csp: crate::csp::CspPolicySet,
    origin: oryn_js::origin::Origin,
    request_state: NavigationRequestState,
    capture: Arc<Mutex<NavigationBodyCapture>>,
}
#[inline(never)]
fn assemble_candidate_document(
    executable: crate::executable::RustExecutableDocument,
    parts: CandidateParts,
) -> Box<CandidateDocument> {
    let CandidateParts {
        reservation,
        url,
        status,
        content_type,
        headers,
        broker,
        csp,
        origin,
        request_state,
        capture,
    } = parts;
    Box::new(CandidateDocument {
        reservation,
        executable,
        url,
        status,
        content_type,
        headers,
        broker,
        csp,
        origin,
        request_state,
        capture,
        activation_available: true,
    })
}
impl CandidateDocument {
    pub fn poll_with_host(
        &mut self,
        host: &mut Option<crate::host::RawV8Host>,
    ) -> Result<(), RuntimeError> {
        let borrowed = host.take().ok_or_else(|| {
            RuntimeError::ProtocolViolation("candidate poll has no shared host".into())
        })?;
        if let Err((error, returned)) = self.executable.install_agent_host(borrowed) {
            *host = Some(*returned);
            return Err(map_host_error(error));
        }
        let result = self
            .executable
            .drain_tasks()
            .and_then(|_| self.executable.poll_context_events());
        *host = Some(self.executable.take_agent_host().map_err(map_host_error)?);
        result.map_err(map_host_error)?;
        if self.executable.terminal_retired() {
            return Ok(());
        }
        self.activation_available = host.as_ref().is_some_and(|host| {
            host.prerender_activation_available(self.executable.agent_root_realm())
        });
        Ok(())
    }
    pub fn activation_available(&self) -> bool {
        self.activation_available
    }
    pub fn load_complete(&self) -> bool {
        self.executable.document_load_complete()
    }
}

/// Retained idempotent publication: a timeout or lost transport reply never
/// proves that parent promotion did not happen. The actor keeps both native
/// roots until this owner receives a definitive current/disposed result.
pub(super) struct PrerenderActivationTransport {
    client: SharedContextClient,
    request: crate::context_service::ContextServiceRequest,
    pending: Option<navigation_fetch::PendingPolicy>,
    retry: Option<Instant>,
    outcome: Option<bool>,
}
impl PrerenderActivationTransport {
    pub fn new(
        client: SharedContextClient,
        page: PageId,
        reservation: &Reservation,
        predecessor: oryn_js::message_wire::ParkedDocumentToken,
    ) -> Self {
        Self {
            client,
            request: crate::context_service::ContextServiceRequest::ActivatePrerender {
                page_id: page,
                token: reservation.token.clone(),
                generation: reservation.generation,
                epoch: reservation.epoch,
                predecessor,
            },
            pending: None,
            retry: None,
            outcome: None,
        }
    }
    pub fn delay(&self) -> Duration {
        self.retry
            .map(|retry| retry.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::ZERO)
    }
    pub fn poll(&mut self) -> Poll<bool> {
        if let Some(outcome) = self.outcome {
            return Poll::Ready(outcome);
        }
        if self.retry.is_some_and(|retry| Instant::now() < retry) {
            return Poll::Pending;
        }
        if self.pending.is_none() {
            match navigation_fetch::PendingPolicy::start_prerender_activation(
                self.client.clone(),
                self.request.clone(),
            ) {
                Ok(pending) => self.pending = Some(pending),
                Err(_) => {
                    self.retry = Some(Instant::now() + Duration::from_millis(25));
                    return Poll::Pending;
                }
            }
        }
        match self.pending.as_mut().unwrap().poll() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending = None;
                let outcome = match result {
                    Ok(crate::context_service::ContextServiceResponse::PrerenderActivated) => {
                        Some(true)
                    }
                    Ok(
                        crate::context_service::ContextServiceResponse::PrerenderActivationObsolete,
                    )
                    | Err(RuntimeError::PageNotFound(_) | RuntimeError::Closed) => Some(false),
                    _ => None,
                };
                if let Some(outcome) = outcome {
                    self.outcome = Some(outcome);
                    Poll::Ready(outcome)
                } else {
                    self.retry = Some(Instant::now() + Duration::from_millis(25));
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod activation_transport_tests {
    use super::*;
    use crate::context_service::{
        ContextServiceRequest as Q, ContextServiceResponse as R, ContextTransport,
    };
    struct LostReply {
        context: BrowserContext,
        page: PageId,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl ContextTransport for LostReply {
        fn request(&self, request: Q) -> Result<R, RuntimeError> {
            match request {
                Q::ActivatePrerender {
                    page_id,
                    token,
                    generation,
                    epoch,
                    predecessor,
                } => {
                    assert_eq!(page_id, self.page);
                    let result = self.context.activate_native_prerender(
                        page_id,
                        &token,
                        (generation, epoch),
                        &predecessor,
                    )?;
                    if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        return Err(RuntimeError::ContextService(
                            "lost activation reply after native publication".into(),
                        ));
                    }
                    Ok(match result {
                        crate::runtime::PrerenderActivationAck::Current => R::PrerenderActivated,
                        crate::runtime::PrerenderActivationAck::Obsolete => {
                            R::PrerenderActivationObsolete
                        }
                    })
                }
                Q::CancelPrerender {
                    page_id,
                    token,
                    generation,
                    epoch,
                } => {
                    self.context
                        .cancel_native_prerender(page_id, &token, (generation, epoch))?;
                    Ok(R::PrerenderCancelled)
                }
                _ => Err(RuntimeError::ProtocolViolation(
                    "unexpected activation fixture request".into(),
                )),
            }
        }
        fn request_routed(
            &self,
            _route: Option<&crate::worker_route::WorkerRoute>,
            request: Q,
        ) -> Result<R, RuntimeError> {
            self.request(request)
        }
    }
    #[test]
    fn uncertain_activation_reply_retains_exact_publication_until_idempotent_ack() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let candidate = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/next")
            .unwrap();
        context
            .commit_prerender_response_policy(page, &candidate, &candidate.url, &[])
            .unwrap();
        context
            .mark_native_prerender_loaded(page, &candidate.token, candidate.lifetime)
            .unwrap();
        let oryn_js::message_wire::MessageResponse::DocumentParked(predecessor) = context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!("park")
        };
        let backend = Arc::new(LostReply {
            context: context.clone(),
            page,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let client = SharedContextClient::for_test_transport(
            backend.clone(),
            crate::worker_route::WorkerRoute::issue(778, page.0, 0).unwrap(),
        );
        let lease = client.reserve_prerender_cleanup(page).unwrap();
        lease.attach(&candidate.token, candidate.lifetime.0, candidate.lifetime.1);
        let reservation = Reservation {
            page_id: page,
            client: client.clone(),
            destination: None,
            token: candidate.token.clone(),
            generation: candidate.lifetime.0,
            epoch: candidate.lifetime.1,
            lease,
        };
        let mut pending =
            PrerenderActivationTransport::new(client, page, &reservation, predecessor);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Poll::Ready(current) = pending.poll() {
                assert!(current);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(pending.poll(), Poll::Ready(true));
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some(candidate.lifetime)
        );
        reservation.activated();
    }
}

#[cfg(test)]
mod accepted_loading_tests {
    use super::*;
    use crate::context_service::{
        ContextServiceRequest as Q, ContextServiceResponse as R, ContextTransport,
        NavigationPolicySource,
    };
    use crate::network::{
        NetworkBroker, NetworkError, NetworkOperation, NetworkRequest, NetworkResponse,
    };
    struct DestinationPolicy {
        context: BrowserContext,
        destination: PageId,
    }
    impl ContextTransport for DestinationPolicy {
        fn request(&self, _: Q) -> Result<R, RuntimeError> {
            Err(RuntimeError::ProtocolViolation(
                "candidate omitted destination route".into(),
            ))
        }
        fn request_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            request: Q,
        ) -> Result<R, RuntimeError> {
            match request {
                Q::AuthorizeNavigation {
                    page_id,
                    source:
                        NavigationPolicySource::Prerender {
                            token,
                            generation,
                            epoch,
                        },
                    url,
                } => {
                    assert_eq!(route.unwrap().environment, self.destination.0);
                    assert_eq!(page_id, self.destination);
                    self.context.authorize_prerender_navigation(
                        page_id,
                        &token,
                        (generation, epoch),
                        &url,
                    )?;
                    Ok(R::NavigationAuthorized)
                }
                Q::CommitPrerenderResponse {
                    response_policy: _,
                    page_id,
                    token,
                    generation,
                    epoch,
                    url,
                    headers,
                } => {
                    assert_eq!(page_id, self.destination);
                    let scope = self.context.native_prerender_scope(
                        page_id,
                        &token,
                        (generation, epoch),
                    )?;
                    let policy = self
                        .context
                        .commit_prerender_response_policy(page_id, &scope, &url, &headers)?;
                    let origin = self
                        .context
                        .native_prerender_scope(page_id, &token, (generation, epoch))?
                        .origin;
                    Ok(R::PrerenderResponseCommitted { policy, origin })
                }
                Q::CancelBlankPrerender {
                    page_id,
                    destination,
                    token,
                    generation,
                    epoch,
                } => {
                    self.context.cancel_blank_prerender(
                        page_id,
                        destination,
                        &token,
                        generation,
                        epoch,
                    )?;
                    Ok(R::PrerenderCancelled)
                }
                _ => Err(RuntimeError::ProtocolViolation(
                    "candidate used live source authority".into(),
                )),
            }
        }
    }
    struct RoutedReplies {
        destination: PageId,
        seen: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for RoutedReplies {
        fn start_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            assert_eq!(route.unwrap().environment, self.destination.0);
            self.start_operation(request)
        }
        fn cancel_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            _: u64,
        ) -> Result<(), NetworkError> {
            assert_eq!(route.unwrap().environment, self.destination.0);
            Ok(())
        }
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.seen.lock().unwrap().push(request.clone());
            let redirect = request.url.ends_with("start");
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: if redirect { 302 } else { 200 },
                headers: if redirect {
                    vec![("location".into(), "/final".into())]
                } else {
                    vec![("content-type".into(), "text/html".into())]
                },
                body: b"<!doctype html><body>accepted".to_vec(),
            })
        }
    }
    #[test]
    fn accepted_candidate_load_uses_destination_route_after_original_creator_retirement() {
        use oryn_js::message_wire::{MessageRequest, WindowTarget};
        let context = Browser::new().new_context();
        let source = context.new_page().page_id();
        context
            .install_document_security(source, "https://blank.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                source,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = oryn_js::worker_wire::Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let (destination, mut admission, scope) = context
            .reserve_blank_prerender(source, &creator, "https://blank.test/start")
            .unwrap();
        admission.direct = false;
        context
            .claim_blank_prerender(source, &creator, Some(destination), &scope.url)
            .unwrap()
            .unwrap();
        let client = SharedContextClient::for_test_transport(
            Arc::new(DestinationPolicy {
                context: context.clone(),
                destination,
            }),
            crate::worker_route::WorkerRoute::issue(789, source.0, 0).unwrap(),
        );
        let lease = client.reserve_prerender_cleanup(source).unwrap();
        lease.attach_blank(
            destination,
            &scope.token,
            scope.lifetime.0,
            scope.lifetime.1,
        );
        let reservation = Reservation {
            page_id: destination,
            client: client.related_endpoint(&admission).unwrap(),
            destination: Some(admission),
            token: scope.token.clone(),
            generation: scope.lifetime.0,
            epoch: scope.lifetime.1,
            lease,
        };
        context
            .install_document_security(source, "https://replacement.test/".into(), &[])
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(RoutedReplies {
            destination,
            seen: seen.clone(),
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut pending = PendingCandidate::start(
            reservation,
            client,
            source,
            creator,
            broker,
            Url::parse(&scope.url).unwrap(),
            crate::cookies::CookieRequestContext {
                initiator_url: Some("https://blank.test/source".into()),
                top_level_url: Some("https://blank.test/source".into()),
                ..Default::default()
            },
            Some(Url::parse("https://blank.test/source").unwrap()),
            Default::default(),
            10,
            deadline,
        )
        .unwrap();
        let prepared = loop {
            match pending.poll() {
                Poll::Ready(result) => break result.unwrap(),
                Poll::Pending => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        assert_eq!(
            prepared.response.current.as_str(),
            "https://blank.test/final"
        );
        assert_eq!(prepared.reservation.page_id, destination);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter()
                .all(|request| request.security.initiator_url.as_deref()
                    == Some("https://blank.test/source"))
        );
        drop(seen);
        drop(prepared);
    }
}
