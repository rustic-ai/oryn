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
#[derive(Default)]
struct State {
    pending: BTreeSet<HostOperationId>,
    ready: VecDeque<PlatformHostCompletion>,
    receipts: BTreeMap<HostOperationId, (cleanup::Ticket, Option<u64>)>,
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
                    .insert(completion.operation_id, (ticket, socket_id));
            } else {
                ticket.accept();
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
            state.sockets.clear();
            state.socket_closes.clear();
        }
        self.cleanup.borrow_mut().take();
    }
    fn abandon_websocket(&self, socket: u64) {
        self.state.borrow_mut().sockets.remove(&socket);
    }
    fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
        let receipt = self.state.borrow_mut().receipts.remove(&id);
        if let Some((ticket, socket)) = receipt
            && accepted
        {
            if let Some(socket) = socket {
                self.state.borrow_mut().sockets.insert(socket, ticket);
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
    fn read() -> PlatformHostRequest {
        PlatformHostRequest::HttpRead {
            stream_id: 17,
            max_bytes: 64 * 1024,
        }
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
