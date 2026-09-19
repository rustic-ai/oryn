//! Parent-owned streaming service response. The body operation retains its
//! event/version until both transport and waitUntil lifetimes have settled.
use super::*;
use crate::network::{NetworkEvent, NetworkOperation};
/// An accepted fetch retains its selected controller even after its client is
/// removed or a replacement worker activates. Each redirect event gets its own
/// event lease from this exact retained version.
pub(crate) struct FrozenFetchController {
    executor: Arc<ServiceRegistry>,
    lease: ServiceEventLease,
    client_id: String,
}
impl FrozenFetchController {
    pub(crate) fn fetch_operation(
        &self,
        mut request: AgentFetchRequest,
        request_id: u64,
        cancel: Arc<AtomicBool>,
        external: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<Option<(AgentFetchResponse, NetworkOperation)>, WorkerError> {
        if cancel.load(Ordering::Acquire) || external.as_ref().is_some_and(|check| check()) {
            return Err(WorkerError::new("AbortError", "frozen fetch cancelled"));
        }
        let lease = {
            let state = self.executor.state()?;
            let version = state
                .versions
                .get(&self.lease.agent)
                .ok_or_else(|| error("retained fetch controller version unavailable"))?;
            self.executor
                .reserve_version_event(&state, self.lease.agent, version, true)?
        };
        request.client_id = self.client_id.clone();
        self.executor
            .fetch_operation_reserved(request, request_id, cancel, lease, None, external)
    }
}
impl ServiceRegistry {
    pub(crate) fn freeze_fetch_controller(
        &self,
        owner: &AgentOwner,
    ) -> Result<Option<Arc<FrozenFetchController>>, WorkerError> {
        let client_id = self.client_snapshot(owner)?.id;
        let selected = {
            let state = self.state()?;
            state
                .clients
                .get(owner)
                .and_then(|id| state.registrations.get(id))
                .and_then(|reg| reg.snapshot.active.as_ref())
                .filter(|active| active.state == WorkerState::Activated)
                .cloned()
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        let (executor, lease) = self.fetch_executor(&selected)?;
        Ok(Some(Arc::new(FrozenFetchController {
            executor,
            lease,
            client_id,
        })))
    }
    pub(crate) fn fetch_operation_cancellable(
        &self,
        owner: &AgentOwner,
        request: AgentFetchRequest,
        request_id: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<(AgentFetchResponse, NetworkOperation)>, WorkerError> {
        self.fetch_operation_with_check(owner, request, request_id, cancel, None)
    }
    pub(crate) fn fetch_operation_with_check(
        &self,
        owner: &AgentOwner,
        mut request: AgentFetchRequest,
        request_id: u64,
        cancel: Arc<AtomicBool>,
        external: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<Option<(AgentFetchResponse, NetworkOperation)>, WorkerError> {
        if cancel.load(Ordering::Acquire) || external.as_ref().is_some_and(|check| check()) {
            return Err(WorkerError::new(
                "AbortError",
                "service fetch cancelled before admission",
            ));
        }
        let selected = {
            let state = self.state()?;
            state
                .clients
                .get(owner)
                .and_then(|id| state.registrations.get(id))
                .and_then(|reg| reg.snapshot.active.as_ref())
                .filter(|active| active.state == WorkerState::Activated)
                .cloned()
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        let (executor, lease) = self.fetch_executor(&selected)?;
        request.client_id = self.client_snapshot(owner)?.id;
        executor.fetch_operation_reserved(request, request_id, cancel, lease, None, external)
    }
    pub(super) fn fetch_operation_reserved(
        &self,
        mut request: AgentFetchRequest,
        request_id: u64,
        cancel: Arc<AtomicBool>,
        lease: ServiceEventLease,
        navigation: Option<NavigationFetchWait>,
        external: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<Option<(AgentFetchResponse, NetworkOperation)>, WorkerError> {
        request.streaming = true;
        if navigation.is_none() {
            request.id = self.id()?;
            request.navigation_preload = false;
        }
        let id = request.id;
        let agent = lease.agent;
        let context = self.context()?;
        {
            let mut inbox = lease
                .inbox
                .lock()
                .map_err(|_| error("service inbox poisoned"))?;
            if inbox.active.len() >= 256 || !inbox.active.insert(id) {
                return Err(error("service event admission budget"));
            }
        }
        let slot = ServiceTaskSlot {
            registry: self.weak_self.clone(),
            inbox: lease.inbox.clone(),
            id,
        };
        let serial = lease.serial.clone();
        let inbox = slot.inbox.clone();
        let deadline = Instant::now() + Duration::from_secs(30);
        let cancellation = stream_custody::Ticket::reserve(self, lease, slot, deadline)?;
        context.dispatch_worker_registry(|agents, _| {
            agents.send_service_control(agent, AgentCommand::Fetch(request))
        })?;
        let mut lifetime_finished = false;
        let metadata = loop {
            if cancel.load(Ordering::Acquire)
                || external.as_ref().is_some_and(|check| check())
                || Instant::now() >= deadline
            {
                return Err(WorkerError::new(
                    "AbortError",
                    "service response cancelled before headers",
                ));
            }
            let (response, finished) =
                match self.poll_service_task(&context, agent, &serial, &inbox, id) {
                    Ok(value) => value,
                    Err(failure) => {
                        let _ = self.retire_version(agent);
                        return Err(failure);
                    }
                };
            lifetime_finished |= finished.is_some();
            cancellation.observe_lifetime(lifetime_finished);
            if let Some(response) = response {
                match response {
                    AgentEvent::FetchResponseStart { response, .. } => {
                        if !response.body.is_empty() {
                            let _ = self.retire_version(agent);
                            return Err(error("streaming headers contained body bytes"));
                        }
                        break response;
                    }
                    AgentEvent::FetchFallback { .. } => {
                        cancellation.finish(lifetime_finished);
                        return Ok(None);
                    }
                    AgentEvent::FetchResponse {
                        response: Err(failure),
                        ..
                    } => {
                        cancellation.finish(lifetime_finished);
                        return Err(failure);
                    }
                    _ => {
                        let _ = self.retire_version(agent);
                        return Err(error("invalid streaming header event"));
                    }
                }
            }
            if lifetime_finished {
                return Err(error("service lifetime ended before fetch headers"));
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let registry = self
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service registry retired"))?;
        let (sender, events) = tokio::sync::mpsc::channel(2);
        let (demand, mut credits) = tokio::sync::mpsc::channel(1);
        sender
            .try_send(NetworkEvent::Headers {
                timing: crate::network::service_timing_provenance(metadata.timing.as_ref()),
                request_id,
                status: metadata.status,
                headers: metadata.headers.clone(),
                content_length: None,
                upload: None,
            })
            .map_err(|_| error("service header channel unavailable"))?;
        let job = std::thread::Builder::new()
            .name(format!("oryn-service-body-{id}"))
            .spawn(move || {
                let cancellation = cancellation;
                let _navigation = navigation;
                let mut pending_demand = false;
                let mut body_finished = false;
                let mut terminal_sent = false;
                loop {
                    let cancelled = cancel.load(Ordering::Acquire)
                        || external.as_ref().is_some_and(|check| check())
                        || sender.is_closed()
                        || registry.retired.load(Ordering::Acquire);
                    if cancelled || Instant::now() >= deadline {
                        if !terminal_sent {
                            let _ = sender.try_send(NetworkEvent::Failed {
                                request_id,
                                error: if cancelled {
                                    "service body cancelled"
                                } else {
                                    "service body timed out"
                                }
                                .into(),
                            });
                        }
                        return;
                    }
                    let polled = registry.poll_service_task(&context, agent, &serial, &inbox, id);
                    match polled {
                        Ok((response, finished)) => {
                            lifetime_finished |= finished.is_some();
                            cancellation.observe_lifetime(lifetime_finished);
                            if response.is_some() {
                                cancel.store(true, Ordering::Release);
                                continue;
                            }
                        }
                        Err(failure) => {
                            let _ = sender.try_send(NetworkEvent::Failed {
                                request_id,
                                error: failure.to_string(),
                            });
                            cancel.store(true, Ordering::Release);
                            continue;
                        }
                    }
                    let event = {
                        let mut inbox = inbox
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        let event = inbox.body.get_mut(&id).and_then(|queue| queue.pop_front());
                        event.map(|(event, bytes)| {
                            inbox.bytes = inbox.bytes.saturating_sub(bytes);
                            event
                        })
                    };
                    if let Some(event) = event {
                        match event {
                            AgentEvent::FetchResponseChunk { bytes, .. } => {
                                if !pending_demand || body_finished {
                                    cancel.store(true, Ordering::Release);
                                    continue;
                                }
                                pending_demand = false;
                                // A single issued demand has one reserved body-channel
                                // slot. Headers consume the other slot until read.
                                if sender
                                    .try_send(NetworkEvent::Chunk {
                                        request_id,
                                        bytes: bytes.into(),
                                    })
                                    .is_err()
                                {
                                    cancel.store(true, Ordering::Release);
                                    continue;
                                }
                            }
                            AgentEvent::FetchResponseEnd { result, .. } => {
                                body_finished = true;
                                pending_demand = false;
                                let event = match result {
                                    Ok(()) => NetworkEvent::Complete {
                                        encoded_body_size: None,
                                        request_id,
                                    },
                                    Err(failure) => NetworkEvent::Failed {
                                        request_id,
                                        error: failure.to_string(),
                                    },
                                };
                                if sender.try_send(event).is_err() {
                                    cancel.store(true, Ordering::Release);
                                    continue;
                                }
                                terminal_sent = true;
                            }
                            _ => {}
                        }
                    }
                    if body_finished {
                        cancellation.finish(lifetime_finished);
                        return;
                    }
                    let ended = inbox
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .ended
                        .contains(&id);
                    if lifetime_finished && !ended {
                        cancel.store(true, Ordering::Release);
                        continue;
                    }
                    if !ended && !pending_demand && credits.try_recv().is_ok() {
                        if context
                            .dispatch_worker_registry(|agents, _| {
                                agents.send_service_control(agent, AgentCommand::FetchPull { id })
                            })
                            .is_err()
                        {
                            cancel.store(true, Ordering::Release);
                            continue;
                        }
                        pending_demand = true;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        let job = match job {
            Ok(job) => job,
            Err(failure) => {
                let _ = self.retire_version(agent);
                return Err(error(failure.to_string()));
            }
        };
        self.jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(job);
        Ok(Some((
            metadata,
            NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id,
                events,
                demand: Some(demand),
            },
        )))
    }
}
