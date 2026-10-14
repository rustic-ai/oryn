//! Nonblocking generated-platform adapter for a route-bound worker principal.
//! Parent admission, resource ownership, networking and partitioned storage remain
//! authoritative. No creator PageId is stored or supplied by this adapter.
use oryn_js::platform::*;
use oryn_js::worker_wire::{AgentServiceRequest, AgentServiceResponse, WorkerTransport};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

#[path = "agent_platform_cleanup.rs"]
mod cleanup;
const ACTIVE: usize = 32;
const WIRE_BYTES: usize = 32 * 1024 * 1024;
enum StreamOperation {
    Read(u64),
    Abort(u64),
}
#[derive(Default)]
struct State {
    pending: BTreeSet<HostOperationId>,
    ready: VecDeque<PlatformHostCompletion>,
    receipts: BTreeMap<HostOperationId, (cleanup::Ticket, Option<u64>, Option<u64>)>,
    streams: BTreeMap<u64, cleanup::Ticket>,
    stream_operations: BTreeMap<HostOperationId, StreamOperation>,
    sockets: BTreeMap<u64, cleanup::Ticket>,
    socket_closes: BTreeMap<HostOperationId, u64>,
}
pub struct AgentPlatformHost {
    transport: Arc<dyn WorkerTransport>,
    origin: String,
    state: RefCell<State>,
    retired: Cell<bool>,
    cleanup: RefCell<Option<cleanup::Ticket>>,
}
impl AgentPlatformHost {
    pub fn new(transport: Arc<dyn WorkerTransport>, serialized_origin: String) -> Self {
        Self {
            transport,
            origin: serialized_origin,
            state: RefCell::new(State::default()),
            retired: Cell::new(false),
            cleanup: RefCell::new(None),
        }
    }
    fn fail(&self, message: String) {
        if self.retired.replace(true) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.receipts.clear();
            state.streams.clear();
            state.stream_operations.clear();
            state.sockets.clear();
            state.socket_closes.clear();
            let mut pending = std::mem::take(&mut state.pending);
            pending.extend(
                state
                    .ready
                    .drain(..)
                    .map(|completion| completion.operation_id),
            );
            state.ready.extend(
                pending
                    .into_iter()
                    .map(|operation_id| PlatformHostCompletion {
                        operation_id,
                        result: Err(message.clone()),
                    }),
            );
        }
        self.cleanup.borrow_mut().take();
    }
    fn receive(&self, max: usize) {
        if self.retired.get() || max == 0 || self.state.borrow().pending.is_empty() {
            return;
        }
        let mut reservations = Vec::new();
        for _ in 0..max {
            match cleanup::Ticket::reserve_result(self.transport.clone()) {
                Ok(ticket) => reservations.push(ticket),
                Err(error) => {
                    self.fail(error);
                    return;
                }
            }
        }
        let response = self
            .transport
            .request(AgentServiceRequest::PlatformPoll { max });
        let completions = match response {
            Ok(AgentServiceResponse::PlatformCompletions(completions)) => completions,
            Ok(_) => {
                self.fail("agent platform poll response mismatch".into());
                return;
            }
            Err(error) => {
                self.fail(error.to_string());
                return;
            }
        };
        if completions.len() > max
            || crate::agent_thread::wire_size(&completions, WIRE_BYTES).is_err()
        {
            self.fail("agent platform completion budget exceeded".into());
            return;
        }
        // Validate the whole batch before releasing any correlation. A duplicate
        // or foreign operation must not consume another request's completion.
        let valid = {
            let state = self.state.borrow();
            let mut seen = BTreeSet::new();
            completions.iter().all(|completion| {
                state.pending.contains(&completion.operation_id)
                    && seen.insert(completion.operation_id)
            })
        };
        if !valid {
            self.fail("agent platform completion has an unknown or duplicate operation".into());
            return;
        }
        let mut state = self.state.borrow_mut();
        for completion in completions {
            let ticket = reservations
                .pop()
                .expect("completion reservations cover admitted poll");
            if let Ok(
                result @ (PlatformHostResult::HttpStarted { .. }
                | PlatformHostResult::WebSocketOpened { .. }),
            ) = &completion.result
            {
                let (stream_id, socket_id) = match result {
                    PlatformHostResult::HttpStarted { stream_id, .. } => (Some(*stream_id), None),
                    PlatformHostResult::WebSocketOpened { socket_id, .. } => {
                        (None, Some(*socket_id))
                    }
                    _ => unreachable!(),
                };
                ticket.attach(AgentServiceRequest::PlatformReleaseResult {
                    stream_id,
                    socket_id,
                });
                state
                    .receipts
                    .insert(completion.operation_id, (ticket, socket_id, stream_id));
            } else {
                ticket.accept();
            }
            if let Some(operation) = state.stream_operations.remove(&completion.operation_id) {
                let stream = match &operation {
                    StreamOperation::Read(id) | StreamOperation::Abort(id) => *id,
                };
                let terminal_success = match (&operation, &completion.result) {
                    (
                        StreamOperation::Read(_),
                        Ok(PlatformHostResult::HttpChunk {
                            stream_id,
                            done: true,
                            ..
                        }),
                    ) => *stream_id == stream,
                    (StreamOperation::Abort(_), Ok(PlatformHostResult::HttpAborted)) => true,
                    _ => false,
                };
                let continuing = matches!((&operation, &completion.result),
                    (StreamOperation::Read(_), Ok(PlatformHostResult::HttpChunk { stream_id, done: false, .. })) if *stream_id == stream);
                if terminal_success {
                    if let Some(ticket) = state.streams.remove(&stream) {
                        ticket.accept();
                    }
                } else if !continuing {
                    // Cancellation/admission failures may precede driver
                    // execution. An error is not proof the stream was closed:
                    // activate its pre-reserved exact-stream cleanup instead.
                    state.streams.remove(&stream);
                }
            }
            if let Some(socket) = state.socket_closes.remove(&completion.operation_id)
                && let Some(ticket) = state.sockets.remove(&socket)
                && matches!(
                    &completion.result,
                    Ok(PlatformHostResult::WebSocketClosed { .. })
                )
            {
                ticket.accept();
            }
            state.pending.remove(&completion.operation_id);
            state.ready.push_back(completion);
        }
        for ticket in reservations {
            ticket.accept();
        }
    }
}
impl PlatformHost for AgentPlatformHost {
    fn submit_owned_idb_admission(
        &self,
        operation_id: HostOperationId,
        origin: String,
        admission: oryn_js::idb_owned_admission::PreparedAdmission,
    ) -> Result<(), OwnedIdbSubmitError> {
        if self.retired.get() {
            return Err(OwnedIdbSubmitError::Failure(
                "agent platform endpoint retired".into(),
            ));
        }
        // Busy is reported before consuming the independently admitted copy.
        // The caller retains the original Admission for a later checkpoint.
        {
            let state = self.state.borrow();
            if state.pending.contains(&operation_id)
                || state.receipts.contains_key(&operation_id)
                || state.ready.iter().any(|entry| entry.operation_id == operation_id)
            {
                return Err(OwnedIdbSubmitError::Failure(
                    "agent platform operation identity is already admitted".into(),
                ));
            }
            if state.pending.len() + state.ready.len() + state.receipts.len() >= ACTIVE {
                return Err(OwnedIdbSubmitError::Busy);
            }
        }
        let (command, custody) = admission.into_command();
        // Reuse the authoritative worker route, origin overwrite, wire bound,
        // cleanup reservation and correlation-before-transport publication.
        // request() consumes the local DTO synchronously. Its custody cannot
        // be released until that call returns, including transport errors.
        let result = self.submit(
            operation_id,
            PlatformHostRequest::IndexedDb { origin, command },
        );
        drop(custody);
        result.map_err(OwnedIdbSubmitError::Failure)
    }
    fn submit(
        &self,
        operation_id: HostOperationId,
        mut request: PlatformHostRequest,
    ) -> Result<(), String> {
        if self.retired.get() {
            return Err("agent platform endpoint retired".into());
        }
        {
            let state = self.state.borrow();
            if state.pending.len() + state.ready.len() + state.receipts.len() >= ACTIVE {
                return Err("agent platform admission limit".into());
            }
            if state.receipts.contains_key(&operation_id)
                || state.pending.contains(&operation_id)
                || state
                    .ready
                    .iter()
                    .any(|entry| entry.operation_id == operation_id)
            {
                return Err("agent platform operation identity is already admitted".into());
            }
        }
        // This field is descriptive. The parent MUST use its bound registry
        // principal/partition (including opaque identity), never this string.
        if let PlatformHostRequest::IndexedDb { origin, .. } = &mut request {
            *origin = self.origin.clone();
        }
        crate::agent_thread::wire_size(&request, WIRE_BYTES).map_err(|error| error.to_string())?;
        if self.cleanup.borrow().is_none() {
            *self.cleanup.borrow_mut() = Some(cleanup::Ticket::reserve(self.transport.clone())?);
        }
        let stream_operation = match &request {
            PlatformHostRequest::HttpRead { stream_id, .. } => {
                Some(StreamOperation::Read(*stream_id))
            }
            PlatformHostRequest::HttpAbort { stream_id } => {
                Some(StreamOperation::Abort(*stream_id))
            }
            _ => None,
        };
        if let Some(operation) = stream_operation {
            self.state
                .borrow_mut()
                .stream_operations
                .insert(operation_id, operation);
        }
        if let PlatformHostRequest::WebSocketClose { socket_id, .. } = &request {
            self.state
                .borrow_mut()
                .socket_closes
                .insert(operation_id, *socket_id);
        }
        self.state.borrow_mut().pending.insert(operation_id);
        match self.transport.request(AgentServiceRequest::PlatformSubmit {
            operation_id,
            request,
        }) {
            Ok(AgentServiceResponse::Done) => Ok(()),
            result => {
                self.state.borrow_mut().pending.remove(&operation_id);
                let message = match result {
                    Err(error) => error.to_string(),
                    _ => "agent platform submit response mismatch".into(),
                };
                // The parent may have admitted work before a transport failure.
                // Revoke all ownership rather than leave an untracked resource.
                self.fail(message.clone());
                Err(message)
            }
        }
    }
    fn cancel(&self, operation_id: HostOperationId) {
        if self.retired.get() || !self.state.borrow().pending.contains(&operation_id) {
            return;
        }
        match self
            .transport
            .request(AgentServiceRequest::PlatformCancel { operation_id })
        {
            Ok(AgentServiceResponse::Done) => {}
            Err(error) => self.fail(error.to_string()),
            _ => self.fail("agent platform cancellation response mismatch".into()),
        }
        // Cancellation does not fabricate success or free the correlation:
        // the parent's terminal completion confirms underlying cleanup.
    }
    fn retire(&self) {
        if self.retired.replace(true) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.pending.clear();
            state.ready.clear();
            state.receipts.clear();
            state.streams.clear();
            state.stream_operations.clear();
            state.sockets.clear();
            state.socket_closes.clear();
        }
        self.cleanup.borrow_mut().take();
    }
    fn abandon_http_stream(&self, stream: u64) {
        // The result receipt was reserved before polling and retained when the
        // consumer accepted this stream. Drop only activates that existing job;
        // it neither admits a new RPC job nor retires this worker's endpoint.
        self.state.borrow_mut().streams.remove(&stream);
    }
    fn abandon_websocket(&self, socket: u64) {
        self.state.borrow_mut().sockets.remove(&socket);
    }
    fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
        let receipt = self.state.borrow_mut().receipts.remove(&id);
        if let Some((ticket, socket, stream)) = receipt
            && accepted
        {
            if let Some(socket) = socket {
                self.state.borrow_mut().sockets.insert(socket, ticket);
            } else if let Some(stream) = stream {
                self.state.borrow_mut().streams.insert(stream, ticket);
            } else {
                ticket.accept();
            }
        }
    }
    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
        if limit == 0 {
            return Vec::new();
        }
        let available = limit
            .min(ACTIVE)
            .saturating_sub(self.state.borrow().ready.len());
        self.receive(available);
        let mut state = self.state.borrow_mut();
        let count = limit.min(state.ready.len());
        state.ready.drain(..count).collect()
    }
}
impl Drop for AgentPlatformHost {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::worker_wire::WorkerError;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Parent {
        requests: Mutex<Vec<AgentServiceRequest>>,
        completions: Mutex<Vec<PlatformHostCompletion>>,
    }
    impl WorkerTransport for Parent {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            if matches!(&request, AgentServiceRequest::PlatformPoll { .. }) {
                self.requests.lock().unwrap().push(request);
                return Ok(AgentServiceResponse::PlatformCompletions(std::mem::take(
                    &mut *self.completions.lock().unwrap(),
                )));
            }
            self.requests.lock().unwrap().push(request);
            Ok(AgentServiceResponse::Done)
        }
    }
    fn owned(
        ledger: &Arc<oryn_js::idb_owned_admission::Ledger>,
    ) -> oryn_js::idb_owned_admission::PreparedAdmission {
        use oryn_js::idb_owned_admission::{Admission, Scope};
        let scope = Scope::copy_names(ledger, &["records".into()], 1024 * 1024, 0).unwrap();
        let admission = Admission::new(
            "transaction".into(), "database".into(), scope,
            "readonly".into(), Some("connection".into()),
        );
        admission.prepare(ledger, 1024 * 1024, 0).unwrap()
    }
    #[test]
    fn owned_idb_worker_admission_reaches_bound_parent_and_releases_local_custody() {
        struct ConsumingParent {
            seen: Mutex<bool>,
            ledger: Arc<oryn_js::idb_owned_admission::Ledger>,
        }
        impl WorkerTransport for ConsumingParent {
            fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
                if let AgentServiceRequest::PlatformSubmit { operation_id, request: PlatformHostRequest::IndexedDb { origin, command: IdbHostCommand::BeginTransaction { scope, .. } } } = &request {
                    assert!(self.ledger.live_bytes() > 0);
                    assert_eq!(*operation_id, HostOperationId(91));
                    assert_eq!(origin, "https://bound.test");
                    assert_eq!(scope, &["records".to_string()]);
                    *self.seen.lock().unwrap() = true;
                }
                drop(request);
                Ok(AgentServiceResponse::Done)
            }
        }
        let ledger = Arc::new(oryn_js::idb_owned_admission::Ledger::default());
        let parent = Arc::new(ConsumingParent { seen: Mutex::new(false), ledger: ledger.clone() });
        let host = AgentPlatformHost::new(parent.clone(), "https://bound.test".into());
        let admission = owned(&ledger);
        assert!(ledger.live_bytes() > 0);
        host.submit_owned_idb_admission(HostOperationId(91), "https://forged.test".into(), admission).unwrap();
        assert_eq!(ledger.live_bytes(), 0);
        assert!(host.state.borrow().pending.contains(&HostOperationId(91)));
        assert!(*parent.seen.lock().unwrap());
    }
    #[test]
    fn owned_idb_worker_busy_and_retired_do_not_publish_or_leak_custody() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "null".into());
        let ledger = Arc::new(oryn_js::idb_owned_admission::Ledger::default());
        for id in 0..ACTIVE {
            host.state.borrow_mut().pending.insert(HostOperationId(id as u64));
        }
        assert!(matches!(host.submit_owned_idb_admission(HostOperationId(100), "null".into(), owned(&ledger)), Err(OwnedIdbSubmitError::Busy)));
        assert_eq!(ledger.live_bytes(), 0);
        assert!(parent.requests.lock().unwrap().is_empty());
        host.retired.set(true);
        assert!(matches!(host.submit_owned_idb_admission(HostOperationId(101), "null".into(), owned(&ledger)), Err(OwnedIdbSubmitError::Failure(_))));
        assert_eq!(ledger.live_bytes(), 0);
        assert!(parent.requests.lock().unwrap().is_empty());
    }
    fn read() -> PlatformHostRequest {
        PlatformHostRequest::HttpRead {
            stream_id: 17,
            max_bytes: 64 * 1024,
        }
    }
    #[test]
    fn owned_idb_worker_transport_failure_keeps_custody_until_request_consumption() {
        struct RejectingParent {
            ledger: Arc<oryn_js::idb_owned_admission::Ledger>,
        }
        impl WorkerTransport for RejectingParent {
            fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
                if matches!(&request, AgentServiceRequest::PlatformSubmit { .. }) {
                    assert!(self.ledger.live_bytes() > 0);
                    drop(request);
                    assert!(self.ledger.live_bytes() > 0);
                    return Err(WorkerError::new("NetworkError", "transport rejected"));
                }
                Ok(AgentServiceResponse::Done)
            }
        }
        let ledger = Arc::new(oryn_js::idb_owned_admission::Ledger::default());
        let host = AgentPlatformHost::new(Arc::new(RejectingParent { ledger: ledger.clone() }), "null".into());
        assert!(matches!(host.submit_owned_idb_admission(HostOperationId(102), "null".into(), owned(&ledger)), Err(OwnedIdbSubmitError::Failure(_))));
        assert_eq!(ledger.live_bytes(), 0);
        assert!(host.retired.get());
        assert!(!host.state.borrow().pending.contains(&HostOperationId(102)));
    }
    #[test]
    fn owned_idb_worker_duplicate_identity_is_terminal_not_busy() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "null".into());
        let ledger = Arc::new(oryn_js::idb_owned_admission::Ledger::default());
        host.state.borrow_mut().pending.insert(HostOperationId(103));
        for id in 200..200 + ACTIVE - 1 {
            host.state.borrow_mut().pending.insert(HostOperationId(id as u64));
        }
        assert!(matches!(host.submit_owned_idb_admission(HostOperationId(103), "null".into(), owned(&ledger)), Err(OwnedIdbSubmitError::Failure(_))));
        assert_eq!(ledger.live_bytes(), 0);
        assert!(parent.requests.lock().unwrap().is_empty());
    }
    #[test]
    fn parent_identity_is_bound_and_idb_origin_is_not_author_authority() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "null".into());
        host.submit(
            HostOperationId(1),
            PlatformHostRequest::IndexedDb {
                origin: "https://forged.test".into(),
                command: IdbHostCommand::ListDatabases,
            },
        )
        .unwrap();
        assert!(
            matches!(&parent.requests.lock().unwrap()[0],AgentServiceRequest::PlatformSubmit{request:PlatformHostRequest::IndexedDb{origin,..},..}if origin=="null")
        );
        assert!(host.drain_completions(1).is_empty());
    }
    #[test]
    fn cancellation_keeps_correlation_until_parent_terminal_completion() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
        let id = HostOperationId(1);
        host.submit(id, read()).unwrap();
        host.cancel(id);
        assert!(host.state.borrow().pending.contains(&id));
        assert!(host.drain_completions(1).is_empty());
        parent
            .completions
            .lock()
            .unwrap()
            .push(PlatformHostCompletion {
                operation_id: id,
                result: Err("AbortError".into()),
            });
        assert!(host.drain_completions(1)[0].result.is_err());
        assert!(!host.state.borrow().pending.contains(&id));
    }
    #[test]
    fn duplicate_or_foreign_batch_retires_without_misattributed_results() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
        for id in [1, 2] {
            host.submit(HostOperationId(id), read()).unwrap();
        }
        parent.completions.lock().unwrap().extend([
            PlatformHostCompletion {
                operation_id: HostOperationId(1),
                result: Ok(PlatformHostResult::HttpAborted),
            },
            PlatformHostCompletion {
                operation_id: HostOperationId(99),
                result: Ok(PlatformHostResult::HttpAborted),
            },
        ]);
        let values = host.drain_completions(2);
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|value| value.result.is_err()));
        assert!(host.retired.get());
        assert!(host.submit(HostOperationId(3), read()).is_err());
    }
    #[test]
    fn retirement_revokes_retained_endpoint_and_discards_late_completion() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
        host.submit(HostOperationId(1), read()).unwrap();
        host.retire();
        host.retire();
        parent
            .completions
            .lock()
            .unwrap()
            .push(PlatformHostCompletion {
                operation_id: HostOperationId(1),
                result: Ok(PlatformHostResult::HttpAborted),
            });
        assert!(host.drain_completions(1).is_empty());
        assert!(host.submit(HostOperationId(2), read()).is_err());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !parent
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, AgentServiceRequest::PlatformRetire))
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            parent
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| matches!(request, AgentServiceRequest::PlatformRetire))
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod reply_custody_tests {
    use super::*;
    use oryn_js::worker_wire::WorkerError;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Parent {
        requests: Mutex<Vec<AgentServiceRequest>>,
        replies: Mutex<Vec<PlatformHostCompletion>>,
    }
    impl WorkerTransport for Parent {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            if matches!(&request, AgentServiceRequest::PlatformPoll { .. }) {
                return Ok(AgentServiceResponse::PlatformCompletions(std::mem::take(
                    &mut *self.replies.lock().unwrap(),
                )));
            }
            self.requests.lock().unwrap().push(request);
            Ok(AgentServiceResponse::Done)
        }
    }
    #[test]
    fn resource_reply_stays_owned_until_receiver_accepts_or_rejects() {
        let parent = Arc::new(Parent::default());
        let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
        for value in [1, 2] {
            let id = HostOperationId(value);
            host.submit(
                id,
                PlatformHostRequest::HttpRead {
                    stream_id: 17,
                    max_bytes: 1,
                },
            )
            .unwrap();
            parent.replies.lock().unwrap().push(PlatformHostCompletion {
                operation_id: id,
                result: Ok(PlatformHostResult::WebSocketOpened {
                    socket_id: value,
                    protocol: String::new(),
                }),
            });
        }
        assert_eq!(host.drain_completions(2).len(), 2);
        assert!(
            host.submit(
                HostOperationId(1),
                PlatformHostRequest::HttpRead {
                    stream_id: 17,
                    max_bytes: 1
                }
            )
            .is_err()
        );
        host.acknowledge_completion(HostOperationId(1), true);
        host.acknowledge_completion(HostOperationId(2), false);
        assert!(host.state.borrow().receipts.is_empty());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let requests = parent.requests.lock().unwrap();
            let releases: Vec<_> = requests
                .iter()
                .filter_map(|request| match request {
                    AgentServiceRequest::PlatformReleaseResult {
                        socket_id: Some(socket_id),
                        stream_id: None,
                    } => Some(*socket_id),
                    _ => None,
                })
                .collect();
            if !releases.is_empty() {
                assert_eq!(releases, vec![2]);
                break;
            }
            drop(requests);
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
mod stream_cleanup_tests {
    use super::*;
    use oryn_js::worker_wire::WorkerError;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Parent {
        requests: Mutex<Vec<AgentServiceRequest>>,
        completions: Mutex<VecDeque<PlatformHostCompletion>>,
        release_attempts: std::sync::atomic::AtomicUsize,
    }
    impl WorkerTransport for Parent {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            self.requests.lock().unwrap().push(request.clone());
            match request {
                AgentServiceRequest::PlatformPoll { max } => {
                    let mut ready = self.completions.lock().unwrap();
                    let count = max.min(ready.len());
                    Ok(AgentServiceResponse::PlatformCompletions(
                        ready.drain(..count).collect(),
                    ))
                }
                AgentServiceRequest::PlatformReleaseResult {
                    stream_id: Some(7),
                    socket_id: None,
                } => {
                    if self
                        .release_attempts
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        == 0
                    {
                        // Wrong acknowledgement must retain the exact stream job
                        // for retry; it must not retire the worker endpoint.
                        Ok(AgentServiceResponse::PlatformCompletions(Vec::new()))
                    } else {
                        Ok(AgentServiceResponse::Done)
                    }
                }
                _ => Ok(AgentServiceResponse::Done),
            }
        }
    }
    fn start() -> PlatformHostRequest {
        PlatformHostRequest::HttpStart {
            request: PlatformHttpRequest {
                upgrade_mixed_image: false,
                fetch: None,
                use_cors_preflight: false,
                method: "GET".into(),
                url: "https://a.test/data".into(),
                headers: vec![],
                body: vec![],
                credentials: "same-origin".into(),
                initiator_url: Some("https://a.test/".into()),
                referrer_policy: None,
            },
        }
    }
    fn started(operation: u64, stream: u64) -> PlatformHostCompletion {
        PlatformHostCompletion {
            operation_id: HostOperationId(operation),
            result: Ok(PlatformHostResult::HttpStarted {
                timing: None,
                internal_response: None,
                response_type: None,
                upload: None,
                cors_filtered: false,
                redirected: false,
                stream_id: stream,
                status: 200,
                url: "https://a.test/data".into(),
                headers: vec![],
            }),
        }
    }

    #[test]
    fn abandoned_worker_xhr_stream_preserves_siblings_and_subsequent_requests() {
        for timeout in [false, true] {
            let parent = Arc::new(Parent::default());
            let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
            host.submit(HostOperationId(1), start()).unwrap();
            parent.completions.lock().unwrap().push_back(started(1, 7));
            assert_eq!(host.drain_completions(1).len(), 1);
            host.acknowledge_completion(HostOperationId(1), true);
            assert!(host.state.borrow().streams.contains_key(&7));
            // A separate fetch is admitted before the synchronous reader fails.
            host.submit(HostOperationId(2), start()).unwrap();
            host.submit(
                HostOperationId(3),
                PlatformHostRequest::HttpRead {
                    stream_id: 7,
                    max_bytes: 64 * 1024,
                },
            )
            .unwrap();
            if timeout {
                host.cancel(HostOperationId(3));
            } else {
                // Models a synchronous consumer rejecting a collected-body
                // limit after a successful, nonterminal read completion.
                parent
                    .completions
                    .lock()
                    .unwrap()
                    .push_back(PlatformHostCompletion {
                        operation_id: HostOperationId(3),
                        result: Ok(PlatformHostResult::HttpChunk {
                            encoded_body_size: None,
                            stream_id: 7,
                            bytes: vec![0; 6],
                            done: false,
                        }),
                    });
                assert_eq!(host.drain_completions(1).len(), 1);
                host.acknowledge_completion(HostOperationId(3), true);
            }
            host.abandon_http_stream(7);
            host.abandon_http_stream(7); // repeated handoff must not duplicate jobs
            assert!(!host.retired.get());
            assert!(host.state.borrow().pending.contains(&HostOperationId(2)));
            parent.completions.lock().unwrap().push_back(started(2, 8));
            if timeout {
                parent
                    .completions
                    .lock()
                    .unwrap()
                    .push_back(PlatformHostCompletion {
                        operation_id: HostOperationId(3),
                        result: Err("AbortError".into()),
                    });
            }
            let completions = host.drain_completions(2);
            assert!(
                completions
                    .iter()
                    .any(|entry| entry.operation_id == HostOperationId(2)
                        && matches!(
                            entry.result,
                            Ok(PlatformHostResult::HttpStarted { stream_id: 8, .. })
                        ))
            );
            host.acknowledge_completion(HostOperationId(2), true);
            // A new request still traverses the real adapter and receives its
            // correlated start/body, after the failed request's cleanup handoff.
            host.submit(HostOperationId(4), start()).unwrap();
            parent.completions.lock().unwrap().push_back(started(4, 9));
            assert_eq!(host.drain_completions(1).len(), 1);
            host.acknowledge_completion(HostOperationId(4), true);
            host.submit(
                HostOperationId(5),
                PlatformHostRequest::HttpRead {
                    stream_id: 9,
                    max_bytes: 1,
                },
            )
            .unwrap();
            parent
                .completions
                .lock()
                .unwrap()
                .push_back(PlatformHostCompletion {
                    operation_id: HostOperationId(5),
                    result: Ok(PlatformHostResult::HttpChunk {
                        encoded_body_size: None,
                        stream_id: 9,
                        bytes: b"x".to_vec(),
                        done: true,
                    }),
                });
            let delivered = host.drain_completions(1);
            assert!(
                matches!(&delivered[0].result, Ok(PlatformHostResult::HttpChunk { bytes, done: true, .. }) if bytes == b"x")
            );
            host.acknowledge_completion(HostOperationId(5), true);
            assert!(!host.state.borrow().streams.contains_key(&9));
            assert!(host.state.borrow().streams.contains_key(&8));
            let deadline = Instant::now() + Duration::from_secs(5);
            while parent
                .release_attempts
                .load(std::sync::atomic::Ordering::SeqCst)
                < 2
            {
                assert!(
                    Instant::now() < deadline,
                    "stream cleanup did not retry wrong ACK"
                );
                std::thread::yield_now();
            }
            assert!(!host.retired.get());
            assert!(
                !parent
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| matches!(
                        request,
                        AgentServiceRequest::PlatformRetire
                            | AgentServiceRequest::PlatformReleaseResult {
                                stream_id: Some(8 | 9),
                                ..
                            }
                    ))
            );
        }
    }
    #[test]
    fn cancellation_before_driver_execution_does_not_disarm_stream_cleanup() {
        for abort in [false, true] {
            let parent = Arc::new(Parent::default());
            let host = AgentPlatformHost::new(parent.clone(), "https://a.test".into());
            host.submit(HostOperationId(1), start()).unwrap();
            parent.completions.lock().unwrap().push_back(started(1, 7));
            host.drain_completions(1);
            host.acknowledge_completion(HostOperationId(1), true);
            host.submit(
                HostOperationId(2),
                if abort {
                    PlatformHostRequest::HttpAbort { stream_id: 7 }
                } else {
                    PlatformHostRequest::HttpRead {
                        stream_id: 7,
                        max_bytes: 1,
                    }
                },
            )
            .unwrap();
            host.cancel(HostOperationId(2));
            // Parent can return this before AgentIo::execute runs. The live
            // parent stream still requires the retained release-result job.
            parent
                .completions
                .lock()
                .unwrap()
                .push_back(PlatformHostCompletion {
                    operation_id: HostOperationId(2),
                    result: Err("cancelled before execute".into()),
                });
            assert!(host.drain_completions(1)[0].result.is_err());
            let deadline = Instant::now() + Duration::from_secs(5);
            while parent
                .release_attempts
                .load(std::sync::atomic::Ordering::SeqCst)
                < 2
            {
                assert!(
                    Instant::now() < deadline,
                    "failed operation discarded cleanup custody"
                );
                std::thread::yield_now();
            }
            assert!(!host.retired.get());
            host.submit(HostOperationId(3), start()).unwrap();
            parent.completions.lock().unwrap().push_back(started(3, 8));
            assert_eq!(host.drain_completions(1).len(), 1);
            host.acknowledge_completion(HostOperationId(3), true);
            assert!(host.state.borrow().streams.contains_key(&8));
            assert!(
                !parent
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| matches!(request, AgentServiceRequest::PlatformRetire))
            );
        }
    }
}
