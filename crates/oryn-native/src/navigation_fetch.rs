//! Retained root response-header work. Transport admission runs on the shared
//! bounded IO lanes; polling never waits for either admission or headers.
use super::*;
use crate::network::{NetworkEvent, NetworkOperation};
use std::task::Poll;

#[cfg(unix)]
pub(super) struct PendingPolicy {
    receiver: Option<
        oneshot::Receiver<Result<crate::context_service::ContextServiceResponse, RuntimeError>>,
    >,
    deadline: Option<Instant>,
}

#[cfg(unix)]
impl PendingPolicy {
    // Deterministic unit fixture for the existing asynchronous policy receiver.
    // Production admission still uses start()/start_inner() and parent authority.
    #[cfg(test)]
    pub(super) fn from_receiver_for_test(
        receiver: oneshot::Receiver<
            Result<crate::context_service::ContextServiceResponse, RuntimeError>,
        >,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            receiver: Some(receiver),
            deadline,
        }
    }
    pub fn start(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
    ) -> Result<Self, RuntimeError> {
        Self::start_inner(client, request, deadline, false)
    }
    #[cfg(feature = "v8-host")]
    pub fn start_navigation_source(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
        lease: &Arc<crate::context_service::ContextNavigationSourceLease>,
    ) -> Result<Self, RuntimeError> {
        if !matches!(&request,
            crate::context_service::ContextServiceRequest::AcquireNavigationNetworkSource { nonce, .. }
            if nonce == &lease.nonce)
        {
            return Err(RuntimeError::ProtocolViolation(
                "navigation source lease mismatch".into(),
            ));
        }
        Self::start_inner(client, request, deadline, true)
    }
    pub fn start_blob_read(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            &request,
            crate::context_service::ContextServiceRequest::WorkerService {
                request: oryn_js::worker_wire::AgentServiceRequest::BlobResolve { .. },
                ..
            }
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid blob download read".into(),
            ));
        }
        Self::start_inner(client, request, deadline, true)
    }
    pub fn start_download_publication(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            &request,
            crate::context_service::ContextServiceRequest::QuarantinePageDownload { .. }
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid download publication request".into(),
            ));
        }
        Self::start_inner(client, request, deadline, true)
    }
    pub fn start_prerender_response(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
    ) -> Result<Self, RuntimeError> {
        // The caller already holds the reservation lease; losing this ACK
        // never loses scope custody. A reservation itself must use its owning
        // handoff type, not this unowned policy-result continuation.
        if !matches!(
            &request,
            crate::context_service::ContextServiceRequest::CommitPrerenderResponse { .. }
                | crate::context_service::ContextServiceRequest::MarkPrerenderLoaded { .. }
                | crate::context_service::ContextServiceRequest::MarkPrerenderReady { .. }
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid prerender response commitment".into(),
            ));
        }
        Self::start_inner(client, request, deadline, true)
    }
    pub fn start_prerender_activation(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            &request,
            crate::context_service::ContextServiceRequest::ActivatePrerender { .. }
                | crate::context_service::ContextServiceRequest::ActivateBlankPrerender { .. }
                | crate::context_service::ContextServiceRequest::PublishBlankPrerender { .. }
                | crate::context_service::ContextServiceRequest::AcknowledgeBlankPrerender { .. }
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid retained prerender activation request".into(),
            ));
        }
        // The PrerenderActivationTransport owner retains candidate and source
        // recovery through every uncertain reply; this API is not a fire-and-
        // forget policy query.
        Self::start_inner(
            client,
            request,
            Some(Instant::now() + Duration::from_secs(5)),
            true,
        )
    }
    pub fn start_prerender_retirement(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            &request,
            crate::context_service::ContextServiceRequest::MessageWire {
                request: oryn_js::message_wire::MessageRequest::RetireRootDocument { .. }
                    | oryn_js::message_wire::MessageRequest::RetireWindow { .. },
                ..
            }
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid retained prerender retirement request".into(),
            ));
        }
        Self::start_inner(
            client,
            request,
            Some(Instant::now() + Duration::from_secs(5)),
            true,
        )
    }
    fn start_inner(
        client: SharedContextClient,
        request: crate::context_service::ContextServiceRequest,
        deadline: Option<Instant>,
        publication: bool,
    ) -> Result<Self, RuntimeError> {
        use crate::context_service::ContextServiceRequest as Request;
        if !publication
            && !matches!(
                &request,
                Request::CaptureCommandNavigationSource { .. }
                    | Request::AuthorizeNavigation { .. }
                    | Request::AuthorizeFormSubmission { .. }
                    | Request::AuthorizePageDownload { .. }
                    | Request::CheckFormNavigationPolicy { .. }
                    | Request::ReadScriptNavigationSource { .. }
                    | Request::ReadServiceNavigationSource { .. }
            )
        {
            return Err(RuntimeError::ProtocolViolation(
                "navigation policy continuation accepts only non-publishing policy queries".into(),
            ));
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            });
        }
        let job = client.track_resource_job()?;
        let (sender, receiver) = oneshot::channel();
        crate::executable::resource_io_executor::submit(move || {
            let _job = job;
            if sender.is_closed() {
                return;
            }
            let result = if deadline.is_some_and(|limit| Instant::now() >= limit) {
                Err(RuntimeError::WorkerLimitExceeded {
                    resource: "navigation wall time".into(),
                })
            } else {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.request(request)))
                    .map_err(|_| RuntimeError::Network("navigation policy request panicked".into()))
                    .and_then(|result| result)
            };
            let _ = sender.send(result);
        })
        .map_err(|error| {
            RuntimeError::Network(format!("navigation policy IO admission: {error}"))
        })?;
        Ok(Self {
            receiver: Some(receiver),
            deadline,
        })
    }

    pub fn poll(
        &mut self,
    ) -> Poll<Result<crate::context_service::ContextServiceResponse, RuntimeError>> {
        if self.receiver.is_none() {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        if self.deadline.is_some_and(|limit| Instant::now() >= limit) {
            self.receiver.take();
            return Poll::Ready(Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            }));
        }
        let Some(receiver) = self.receiver.as_mut() else {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                Poll::Ready(result)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Poll::Pending,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.receiver = None;
                Poll::Ready(Err(RuntimeError::Network(
                    "navigation policy owner ended without a result".into(),
                )))
            }
        }
    }
}

pub(super) struct Headers {
    pub response_policy: Option<crate::network::ResponsePolicyToken>,
    pub timing: Option<oryn_net::TransportTiming>,
    pub started: Instant,
    pub received: Instant,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub operation: NetworkOperation,
}

/// Byte custody for a download response. A turn consumes bounded ready input;
/// it never waits for the transport and never publishes a download itself.
pub(super) struct PendingDownloadBody {
    operation: Option<NetworkOperation>,
    bytes: Vec<u8>,
    deadline: Option<Instant>,
    demand_pending: bool,
}

impl PendingDownloadBody {
    pub fn new(operation: NetworkOperation, deadline: Option<Instant>) -> Self {
        Self {
            operation: Some(operation),
            bytes: Vec::new(),
            deadline,
            demand_pending: false,
        }
    }
    fn fail(&mut self, error: RuntimeError) -> Poll<Result<Vec<u8>, RuntimeError>> {
        self.operation.take();
        self.bytes = Vec::new();
        Poll::Ready(Err(error))
    }
    pub fn poll(&mut self) -> Poll<Result<Vec<u8>, RuntimeError>> {
        if self.operation.is_none() {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        for _ in 0..32 {
            if self.deadline.is_some_and(|limit| Instant::now() >= limit) {
                return self.fail(RuntimeError::WorkerLimitExceeded {
                    resource: "download wall time".into(),
                });
            }
            let operation = self.operation.as_mut().expect("live download operation");
            if !self.demand_pending {
                operation.request_body_chunk();
                self.demand_pending = true;
            }
            let expected = operation.request_id;
            let event = match operation.try_event() {
                Ok(event) => {
                    self.demand_pending = false;
                    event
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return Poll::Pending,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return self.fail(RuntimeError::Network(
                        "download response ended before completion".into(),
                    ));
                }
            };
            match event {
                NetworkEvent::Chunk { request_id, bytes } if request_id == expected => {
                    if self
                        .bytes
                        .len()
                        .checked_add(bytes.len())
                        .is_none_or(|size| size > 16 * 1024 * 1024)
                        || self.bytes.try_reserve(bytes.len()).is_err()
                    {
                        return self.fail(RuntimeError::WorkerLimitExceeded {
                            resource: "download bytes".into(),
                        });
                    }
                    self.bytes.extend_from_slice(&bytes);
                }
                NetworkEvent::Complete {
                    encoded_body_size: _,
                    request_id,
                } if request_id == expected => {
                    self.operation.take();
                    return Poll::Ready(Ok(std::mem::take(&mut self.bytes)));
                }
                NetworkEvent::Failed { request_id, error } if request_id == expected => {
                    return self.fail(RuntimeError::from(error));
                }
                _ => {
                    return self.fail(RuntimeError::ProtocolViolation(
                        "unexpected download response event".into(),
                    ));
                }
            }
        }
        Poll::Pending
    }
}

/// A page-owned observer of the actual admission path. It carries no network
/// authority, request body, credentials or policy bypass.
#[derive(Clone)]
pub(super) struct HeaderDiagnostics {
    sink: crate::trace::DiagnosticSink,
    ipc_identity: Option<crate::ipc_progress::Identity>,
    navigation_id: u64,
    source_generation: u64,
    request_id: u64,
    queued: Instant,
    deadline: Option<Instant>,
}
impl HeaderDiagnostics {
    pub fn new(
        sink: crate::trace::DiagnosticSink,
        ipc_identity: Option<crate::ipc_progress::Identity>,
        navigation_id: u64,
        source_generation: u64,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            sink,
            ipc_identity,
            navigation_id,
            source_generation,
            request_id: 0,
            queued: Instant::now(),
            deadline,
        }
    }
    pub fn for_request(&self, request_id: u64) -> Self {
        Self {
            request_id,
            queued: Instant::now(),
            ..self.clone()
        }
    }
    pub fn record(&self, phase: &'static str, error: Option<String>) {
        let now = Instant::now();
        crate::ipc_progress::navigation::record(crate::ipc_progress::navigation::Observation {
            identity: self.ipc_identity,
            navigation_id: self.navigation_id,
            navigation_request_id: (self.request_id != 0).then_some(self.request_id),
            source_generation: self.source_generation,
            phase,
            error_present: error.is_some(),
            elapsed_ns: now.saturating_duration_since(self.queued).as_nanos(),
            deadline_remaining_ns: self
                .deadline
                .map(|deadline| deadline.saturating_duration_since(now).as_nanos()),
        });
        self.sink.record(
            "navigation.io",
            serde_json::json!({
                "navigation_id": self.navigation_id, "source_generation": self.source_generation,
                "request_id": self.request_id, "phase": phase, "error": error,
                "elapsed_us": now.saturating_duration_since(self.queued).as_micros(),
                "deadline_remaining_us": self.deadline.map(|deadline|
                    deadline.saturating_duration_since(now).as_micros()),
            })
            .to_string(),
        );
    }
}

fn record_header_diagnostic(
    diagnostics: &Option<HeaderDiagnostics>,
    phase: &'static str,
    error: Option<String>,
) {
    if let Some(diagnostics) = diagnostics {
        diagnostics.record(phase, error);
    }
}

fn run_header_admission(
    broker: SharedNetworkBroker,
    request: NetworkRequest,
    deadline: Option<Instant>,
    sender: oneshot::Sender<Result<(NetworkOperation, Instant), RuntimeError>>,
    diagnostics: Option<HeaderDiagnostics>,
) {
    record_header_diagnostic(&diagnostics, "io_job_entered", None);
    if sender.is_closed() {
        record_header_diagnostic(&diagnostics, "consumer_closed_before_broker", None);
        return;
    }
    let started = Instant::now();
    let result = if deadline.is_some_and(|limit| started >= limit) {
        record_header_diagnostic(&diagnostics, "deadline_before_broker", None);
        Err(RuntimeError::WorkerLimitExceeded {
            resource: "navigation wall time".into(),
        })
    } else {
        record_header_diagnostic(&diagnostics, "broker_entered", None);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            broker.start_operation(request)
        }))
        .map_err(|_| RuntimeError::Network("navigation transport admission panicked".into()))
        .and_then(|result| result.map_err(RuntimeError::from));
        record_header_diagnostic(
            &diagnostics,
            if result.is_ok() {
                "broker_returned"
            } else {
                "broker_failed"
            },
            result.as_ref().err().map(ToString::to_string),
        );
        result
    };
    // A rejected late success is dropped with its native broker lease intact.
    // Diagnostics never assume this handoff means headers or a server GET arrived.
    if sender
        .send(result.map(|operation| (operation, started)))
        .is_ok()
    {
        record_header_diagnostic(&diagnostics, "result_delivered", None);
    } else {
        record_header_diagnostic(&diagnostics, "consumer_closed_after_broker", None);
    }
}

pub(super) struct PendingHeaders {
    started: Option<Instant>,
    starting: Option<oneshot::Receiver<Result<(NetworkOperation, Instant), RuntimeError>>>,
    operation: Option<NetworkOperation>,
    deadline: Option<Instant>,
    terminal: bool,
    diagnostics: Option<HeaderDiagnostics>,
}

impl PendingHeaders {
    pub fn start(
        broker: SharedNetworkBroker,
        request: NetworkRequest,
        deadline: Option<Instant>,
        #[cfg(unix)] client: Option<&SharedContextClient>,
        diagnostics: Option<HeaderDiagnostics>,
    ) -> Result<Self, RuntimeError> {
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            record_header_diagnostic(&diagnostics, "deadline_before_submit", None);
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            });
        }
        // The worker's teardown barrier includes queued and running admission.
        #[cfg(unix)]
        let resource_job = client
            .map(|client| client.track_resource_job())
            .transpose()
            .inspect_err(|error| {
                record_header_diagnostic(
                    &diagnostics,
                    "resource_job_rejected",
                    Some(error.to_string()),
                )
            })?;
        let (sender, receiver) = oneshot::channel();
        let observed = diagnostics.clone();
        record_header_diagnostic(&diagnostics, "io_submit_started", None);
        crate::executable::resource_io_executor::submit(move || {
            #[cfg(unix)]
            let _resource_job = resource_job;
            run_header_admission(broker, request, deadline, sender, observed);
        })
        .map_err(|error| {
            record_header_diagnostic(&diagnostics, "io_submit_failed", Some(error.clone()));
            RuntimeError::Network(format!("navigation IO admission: {error}"))
        })?;
        record_header_diagnostic(&diagnostics, "io_submit_accepted", None);
        Ok(Self {
            started: None,
            starting: Some(receiver),
            operation: None,
            deadline,
            terminal: false,
            diagnostics,
        })
    }

    fn fail(&mut self, error: RuntimeError) -> Poll<Result<Headers, RuntimeError>> {
        record_header_diagnostic(
            &self.diagnostics,
            "consumer_failed",
            Some(error.to_string()),
        );
        self.starting.take();
        self.operation.take();
        self.terminal = true;
        Poll::Ready(Err(error))
    }

    pub fn poll(&mut self) -> Poll<Result<Headers, RuntimeError>> {
        if self.terminal {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        }
        if self.deadline.is_some_and(|limit| Instant::now() >= limit) {
            return self.fail(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            });
        }
        if let Some(starting) = self.starting.as_mut() {
            match starting.try_recv() {
                Ok(Ok((operation, started))) => {
                    record_header_diagnostic(&self.diagnostics, "admission_received", None);
                    self.started = Some(started);
                    self.operation = Some(operation);
                    self.starting = None;
                }
                Ok(Err(error)) => return self.fail(error),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return Poll::Pending,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return self.fail(RuntimeError::Network(
                        "navigation admission ended without a result".into(),
                    ));
                }
            }
        }
        let Some(operation) = self.operation.as_mut() else {
            return self.fail(RuntimeError::ProtocolViolation(
                "navigation header owner lost its operation".into(),
            ));
        };
        let expected = operation.request_id;
        match operation.try_event() {
            Ok(NetworkEvent::Headers {
                response_policy,
                request_id,
                status,
                headers,
                timing,
                ..
            }) if request_id == expected => {
                let Some(started) = self.started else {
                    return self.fail(RuntimeError::ProtocolViolation(
                        "navigation lost measured request admission".into(),
                    ));
                };
                record_header_diagnostic(&self.diagnostics, "headers_received", None);
                self.terminal = true;
                Poll::Ready(Ok(Headers {
                    response_policy,
                    timing: timing.map(|timing| *timing),
                    started,
                    received: Instant::now(),
                    status,
                    headers,
                    operation: self.operation.take().expect("owned headers operation"),
                }))
            }
            Ok(NetworkEvent::Failed { request_id, error }) if request_id == expected => {
                self.fail(RuntimeError::from(error))
            }
            Ok(NetworkEvent::Chunk { .. } | NetworkEvent::Complete { .. }) => {
                self.fail(RuntimeError::ProtocolViolation(
                    "network body arrived before accepted response headers".into(),
                ))
            }
            Ok(_) => self.fail(RuntimeError::ProtocolViolation(
                "navigation received a stale or unmatched completion".into(),
            )),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Poll::Pending,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => self.fail(
                RuntimeError::Network("network operation ended before response headers".into()),
            ),
        }
    }
}

impl Drop for PendingHeaders {
    fn drop(&mut self) {
        if !self.terminal {
            record_header_diagnostic(&self.diagnostics, "consumer_retired", None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkError, NetworkResponse};
    use std::sync::mpsc;

    struct HeldStart {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        cancelled: mpsc::Sender<u64>,
        sender: Mutex<Option<tokio::sync::mpsc::Sender<NetworkEvent>>>,
        demand: Arc<Mutex<Option<tokio::sync::mpsc::Receiver<()>>>>,
    }
    impl NetworkBroker for HeldStart {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!()
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let (sender, events) = tokio::sync::mpsc::channel(4);
            sender
                .try_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("x-owner".into(), "retained".into())],
                    content_length: None,
                    upload: None,
                })
                .unwrap();
            *self.sender.lock().unwrap() = Some(sender);
            let (demand, requested) = tokio::sync::mpsc::channel(4);
            *self.demand.lock().unwrap() = Some(requested);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: Some(demand),
            })
        }
        fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
            self.sender.lock().unwrap().take();
            let _ = self.cancelled.send(id);
            Ok(())
        }
    }
    fn request() -> NetworkRequest {
        NetworkRequest {
            request_id: 7,
            method: "GET".into(),
            url: "https://headers.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: false,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        }
    }

    #[test]
    fn download_body_waits_without_repeated_demand_and_transfers_bytes_once() {
        let (sender, events) = tokio::sync::mpsc::channel(4);
        let (demand, mut requests) = tokio::sync::mpsc::channel(4);
        let mut pending = PendingDownloadBody::new(
            NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 7,
                events,
                demand: Some(demand),
            },
            None,
        );
        assert!(matches!(pending.poll(), Poll::Pending));
        assert!(requests.try_recv().is_ok());
        assert!(matches!(pending.poll(), Poll::Pending));
        assert!(requests.try_recv().is_err());
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 7,
                bytes: vec![0, 255, 17].into(),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 7,
            })
            .unwrap();
        assert!(matches!(pending.poll(),Poll::Ready(Ok(bytes)) if bytes==vec![0,255,17]));
        assert!(sender.is_closed());
        assert!(pending.bytes.is_empty());
        assert!(matches!(
            pending.poll(),
            Poll::Ready(Err(RuntimeError::NavigationCancelled))
        ));
    }

    #[test]
    fn download_ready_burst_yields_after_bounded_slice() {
        let (sender, events) = tokio::sync::mpsc::channel(64);
        for _ in 0..40 {
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: 7,
                    bytes: Vec::new().into(),
                })
                .unwrap();
        }
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 7,
            })
            .unwrap();
        let mut pending = PendingDownloadBody::new(
            NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 7,
                events,
                demand: None,
            },
            None,
        );
        assert!(matches!(pending.poll(), Poll::Pending));
        assert_eq!(pending.operation.as_ref().unwrap().events.len(), 9);
        assert!(matches!(pending.poll(),Poll::Ready(Ok(bytes)) if bytes.is_empty()));
    }

    #[test]
    fn failed_download_body_releases_transport_and_retained_bytes() {
        for cause in ["identity", "capacity", "timeout", "disconnect"] {
            let (sender, events) = tokio::sync::mpsc::channel(4);
            let mut pending = PendingDownloadBody::new(
                NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 7,
                    events,
                    demand: None,
                },
                None,
            );
            pending.bytes.extend_from_slice(b"partial");
            match cause {
                "identity" => sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: 8,
                        bytes: vec![1].into(),
                    })
                    .unwrap(),
                "capacity" => sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: 7,
                        bytes: vec![0; 16 * 1024 * 1024].into(),
                    })
                    .unwrap(),
                "timeout" => pending.deadline = Some(Instant::now()),
                _ => {
                    drop(sender);
                    assert!(matches!(
                        pending.poll(),
                        Poll::Ready(Err(RuntimeError::Network(_)))
                    ));
                    assert!(pending.bytes.is_empty());
                    assert!(pending.operation.is_none());
                    continue;
                }
            }
            let error = match pending.poll() {
                Poll::Ready(Err(error)) => error,
                _ => panic!("invalid response remained live"),
            };
            match cause {
                "identity" => assert!(matches!(error, RuntimeError::ProtocolViolation(_))),
                _ => assert!(matches!(error, RuntimeError::WorkerLimitExceeded { .. })),
            }
            assert!(sender.is_closed());
            assert!(pending.bytes.is_empty());
            assert!(pending.operation.is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn policy_retirement_keeps_admitted_io_in_teardown_barrier_until_reply() {
        use crate::context_service::{
            CONTEXT_SERVICE_PROTOCOL_VERSION, ContextServiceRequest, ContextServiceResponse,
            IpcContextClient,
        };
        use std::io::{BufRead, BufReader, Write};
        let (local, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(peer.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            let response = serde_json::json!({"protocol_version":CONTEXT_SERVICE_PROTOCOL_VERSION,"route":null,
                "correlation_id":request["message_id"],"response":ContextServiceResponse::NavigationAuthorized,"error":null});
            writeln!(peer, "{response}").unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let receipt: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(receipt["accepted"], true);
        });
        let client = SharedContextClient::new(IpcContextClient::new(local));
        let mut pending = PendingPolicy::start(
            client.clone(),
            ContextServiceRequest::AuthorizeNavigation {
                page_id: PageId(1),
                source: crate::context_service::NavigationPolicySource::InitialAboutBlank,
                url: "https://policy.test/".into(),
            },
            None,
        )
        .unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(pending.poll(), Poll::Pending));
        pending.deadline = Some(Instant::now());
        assert!(matches!(
            pending.poll(),
            Poll::Ready(Err(RuntimeError::WorkerLimitExceeded { .. }))
        ));
        assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
        release.send(()).unwrap();
        server.join().unwrap();
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
        assert!(pending.receiver.is_none());
    }

    #[test]
    fn rejected_header_events_release_operation_without_body_demand() {
        for event in [
            NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: 8,
                status: 200,
                headers: Vec::new(),
                content_length: None,
                upload: None,
            },
            NetworkEvent::Chunk {
                request_id: 7,
                bytes: vec![1].into(),
            },
            NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 7,
            },
        ] {
            let (sender, events) = tokio::sync::mpsc::channel(4);
            let (demand, mut requested) = tokio::sync::mpsc::channel(4);
            sender.try_send(event).unwrap();
            let mut pending = PendingHeaders {
                started: Some(Instant::now()),
                starting: None,
                operation: Some(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 7,
                    events,
                    demand: Some(demand),
                }),
                deadline: None,
                terminal: false,
                diagnostics: None,
            };
            assert!(matches!(
                pending.poll(),
                Poll::Ready(Err(RuntimeError::ProtocolViolation(_)))
            ));
            assert!(sender.is_closed());
            assert!(matches!(
                requested.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ));
            assert!(pending.operation.is_none());
        }
    }

    #[test]
    fn delayed_admission_yields_and_late_operation_remains_owned_after_retirement() {
        for timeout in [false, true] {
            let (entered, started) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let (cancelled, cancel) = mpsc::channel();
            let demand = Arc::new(Mutex::new(None));
            let broker = SharedNetworkBroker::new(HeldStart {
                entered,
                release: Mutex::new(wait),
                cancelled,
                sender: Mutex::new(None),
                demand,
            });
            let mut pending = PendingHeaders::start(
                broker.clone(),
                request(),
                None,
                #[cfg(unix)]
                None,
                None,
            )
            .unwrap();
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(matches!(pending.poll(), Poll::Pending));
            if timeout {
                pending.deadline = Some(Instant::now());
                assert!(matches!(
                    pending.poll(),
                    Poll::Ready(Err(RuntimeError::WorkerLimitExceeded { .. }))
                ));
            } else {
                drop(pending);
            }
            release.send(()).unwrap();
            cancel.recv_timeout(Duration::from_secs(2)).unwrap();
            broker
                .drain_owned_operations(Duration::from_secs(2))
                .unwrap();
        }
    }

    #[test]
    fn accepted_headers_transfer_operation_once_without_body_demand() {
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (cancelled, cancel) = mpsc::channel();
        let demand = Arc::new(Mutex::new(None));
        let broker = SharedNetworkBroker::new(HeldStart {
            entered,
            release: Mutex::new(wait),
            cancelled,
            sender: Mutex::new(None),
            demand: demand.clone(),
        });
        let mut pending = PendingHeaders::start(
            broker.clone(),
            request(),
            None,
            #[cfg(unix)]
            None,
            None,
        )
        .unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let response = loop {
            match pending.poll() {
                Poll::Ready(result) => break result.unwrap(),
                Poll::Pending => {}
            }
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(Duration::from_millis(1));
        };
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers,
            vec![("x-owner".into(), "retained".into())]
        );
        assert_eq!(response.operation.request_id, 7);
        assert!(matches!(
            demand.lock().unwrap().as_mut().unwrap().try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            pending.poll(),
            Poll::Ready(Err(RuntimeError::NavigationCancelled))
        ));
        drop(pending);
        assert!(matches!(cancel.try_recv(), Err(mpsc::TryRecvError::Empty)));
        drop(response);
        cancel.recv_timeout(Duration::from_secs(2)).unwrap();
        broker
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
    }
    struct RejectedStart {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        panic: bool,
    }
    impl NetworkBroker for RejectedStart {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!()
        }
        fn start_operation(&self, _: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(!self.panic, "injected broker admission panic");
            Err(NetworkError::Transport(
                "original broker source rejection".into(),
            ))
        }
    }

    fn diagnostic_records(trace: &mut crate::trace::TraceLog) -> Vec<serde_json::Value> {
        trace
            .snapshot()
            .into_iter()
            .filter_map(|event| match event.kind {
                TraceEventKind::ScriptDiagnostic { capability, detail }
                    if capability == "navigation.io" =>
                {
                    assert_eq!(event.page_id, PageId(29));
                    let value: serde_json::Value = serde_json::from_str(&detail).unwrap();
                    assert_eq!(value["navigation_id"], 71);
                    assert_eq!(value["source_generation"], 13);
                    assert_eq!(value["request_id"], 7);
                    Some(value)
                }
                _ => None,
            })
            .collect()
    }
    fn diagnostic(trace: &crate::trace::TraceLog) -> HeaderDiagnostics {
        HeaderDiagnostics::new(trace.diagnostic_sink(Revision(5)), None, 71, 13, None)
            .for_request(7)
    }

    #[test]
    fn repeated_navigation_breadcrumbs_never_replay_broker_admission() {
        let mut trace = crate::trace::TraceLog::new(PageId(29));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broker = SharedNetworkBroker::new(RejectedStart {
            calls: calls.clone(),
            panic: false,
        });
        let observed = diagnostic(&trace);
        let (sender, mut receiver) = oneshot::channel();
        for _ in 0..4 {
            observed.record("navigation_policy_consumed", None);
            observed.record("request_start", None);
        }
        run_header_admission(broker, request(), None, sender, Some(observed.clone()));
        for _ in 0..4 {
            observed.record("consumer_failed", Some("private error URL".into()));
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        match receiver.try_recv().unwrap() {
            Err(RuntimeError::Network(error)) => {
                assert!(error.contains("original broker source rejection"))
            }
            _ => panic!("diagnostics replaced the original admission error"),
        }
        let rows = diagnostic_records(&mut trace);
        assert_eq!(
            rows.iter()
                .filter(|row| row["phase"] == "broker_entered")
                .count(),
            1
        );
        assert!(!rows.iter().any(|row| row["phase"] == "headers_received"));
    }

    #[test]
    fn header_diagnostics_distinguish_closed_consumer_and_expired_queued_job() {
        for closed in [false, true] {
            let mut trace = crate::trace::TraceLog::new(PageId(29));
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let broker = SharedNetworkBroker::new(RejectedStart {
                calls: calls.clone(),
                panic: false,
            });
            let (sender, receiver) = oneshot::channel();
            let mut receiver = Some(receiver);
            if closed {
                receiver.take();
            }
            run_header_admission(
                broker,
                request(),
                Some(Instant::now()),
                sender,
                Some(diagnostic(&trace)),
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            if let Some(mut receiver) = receiver {
                assert!(matches!(
                    receiver.try_recv().unwrap(),
                    Err(RuntimeError::WorkerLimitExceeded { .. })
                ));
            }
            let observed = diagnostic_records(&mut trace);
            assert_eq!(observed[0]["phase"], "io_job_entered");
            assert!(observed.iter().any(|row| row["phase"]
                == if closed {
                    "consumer_closed_before_broker"
                } else {
                    "deadline_before_broker"
                }));
            assert!(!observed.iter().any(|row| row["phase"] == "broker_entered"));
        }
    }

    #[test]
    fn header_diagnostics_retain_broker_error_and_panic_without_promoting_submission() {
        for panic in [false, true] {
            let mut trace = crate::trace::TraceLog::new(PageId(29));
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let broker = SharedNetworkBroker::new(RejectedStart {
                calls: calls.clone(),
                panic,
            });
            let (sender, mut receiver) = oneshot::channel();
            run_header_admission(broker, request(), None, sender, Some(diagnostic(&trace)));
            assert!(matches!(
                receiver.try_recv().unwrap(),
                Err(RuntimeError::Network(_))
            ));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            let observed = diagnostic_records(&mut trace);
            let failed = observed
                .iter()
                .find(|row| row["phase"] == "broker_failed")
                .unwrap();
            assert!(failed["error"].as_str().unwrap().contains(if panic {
                "navigation transport admission panicked"
            } else {
                "original broker source rejection"
            }));
            assert!(
                !observed
                    .iter()
                    .any(|row| row["phase"] == "headers_received")
            );
        }
    }

    #[test]
    fn header_diagnostics_keep_late_handoff_rejection_and_native_cancellation() {
        let mut trace = crate::trace::TraceLog::new(PageId(29));
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (cancelled, cancel) = mpsc::channel();
        let broker = SharedNetworkBroker::new(HeldStart {
            entered,
            release: Mutex::new(wait),
            cancelled,
            sender: Mutex::new(None),
            demand: Arc::new(Mutex::new(None)),
        });
        let retained = broker.clone();
        let (sender, receiver) = oneshot::channel();
        let observed = diagnostic(&trace);
        let worker = std::thread::spawn(move || {
            run_header_admission(broker, request(), None, sender, Some(observed))
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(receiver);
        release.send(()).unwrap();
        worker.join().unwrap();
        cancel.recv_timeout(Duration::from_secs(2)).unwrap();
        retained
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        let observed = diagnostic_records(&mut trace);
        assert!(observed.iter().any(|row| row["phase"] == "broker_returned"));
        assert!(
            observed
                .iter()
                .any(|row| row["phase"] == "consumer_closed_after_broker")
        );
        assert!(
            !observed
                .iter()
                .any(|row| row["phase"] == "headers_received")
        );
    }

    #[test]
    fn header_diagnostics_cover_real_io_submission_and_owner_failure() {
        let mut trace = crate::trace::TraceLog::new(PageId(29));
        let broker = SharedNetworkBroker::new(RejectedStart {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            panic: false,
        });
        let mut pending = PendingHeaders::start(
            broker,
            request(),
            None,
            #[cfg(unix)]
            None,
            Some(diagnostic(&trace)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match pending.poll() {
                Poll::Ready(Err(RuntimeError::Network(error))) => {
                    assert!(error.contains("original broker source rejection"));
                    break;
                }
                Poll::Ready(_) => panic!("unexpected admission outcome"),
                Poll::Pending => {
                    assert!(Instant::now() < deadline);
                    std::thread::park_timeout(Duration::from_millis(1));
                }
            }
        }
        let observed = diagnostic_records(&mut trace);
        for phase in [
            "io_submit_started",
            "io_submit_accepted",
            "io_job_entered",
            "broker_entered",
            "broker_failed",
            "consumer_failed",
        ] {
            assert!(
                observed.iter().any(|row| row["phase"] == phase),
                "missing {phase}"
            );
        }
    }
}

#[cfg(test)]
mod certificate_navigation_tests {
    use super::*;
    #[test]
    fn certificate_headers_preserve_cause_and_require_matching_operation() {
        for (id, certificate) in [(7, true), (7, false), (8, true)] {
            let (sender, events) = tokio::sync::mpsc::channel(1);
            let error = if certificate {
                crate::network::NetworkFailure::Certificate {
                    certificate_rejected: "UnknownIssuer".into(),
                }
            } else {
                "invalid certificate: UnknownIssuer".into()
            };
            sender
                .try_send(NetworkEvent::Failed {
                    request_id: id,
                    error,
                })
                .unwrap();
            let mut pending = PendingHeaders {
                started: Some(Instant::now()),
                starting: None,
                operation: Some(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 7,
                    events,
                    demand: None,
                }),
                deadline: None,
                terminal: false,
                diagnostics: None,
            };
            let Poll::Ready(Err(error)) = pending.poll() else {
                panic!("failure did not settle");
            };
            if id != 7 {
                assert!(matches!(error, RuntimeError::ProtocolViolation(_)));
            } else if certificate {
                assert!(matches!(
                    error.navigation_stop(),
                    crate::navigation_control::NavigationStop::CertificateRejected(_)
                ));
            } else {
                assert!(matches!(
                    error.navigation_stop(),
                    crate::navigation_control::NavigationStop::Failed(_)
                ));
            }
            assert!(pending.operation.is_none());
            assert!(sender.is_closed());
        }
    }
}
