//! Parent-owned worker identity and client lifetime admission.
//! Script loading and process launch are supplied by trusted context services.
use oryn_common::v3::PageId;
use oryn_js::message_wire::WindowTarget;
use oryn_js::worker_wire::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentOwner {
    Window {
        page: PageId,
        target: WindowTarget,
        generation: u64,
        epoch: u64,
    },
    Agent(AgentId),
}
impl AgentOwner {
    pub(crate) fn shared_recipient(&self) -> oryn_js::shared_memory::SharedMemoryRecipient {
        match self {
            Self::Window {
                page,
                target,
                generation,
                epoch,
            } => oryn_js::shared_memory::SharedMemoryRecipient::Window {
                page: page.0,
                target: target.clone(),
                document_generation: *generation,
                stream_epoch: *epoch,
            },
            Self::Agent(id) => oryn_js::shared_memory::SharedMemoryRecipient::Agent(id.0),
        }
    }
    fn principal(&self) -> oryn_js::message_wire::Principal {
        match self {
            Self::Window { page, .. } => oryn_js::message_wire::Principal::Window(page.0),
            Self::Agent(agent) => oryn_js::message_wire::Principal::Agent(agent.0),
        }
    }
}

/// Implemented by the attested agent process transport. Shutdown acknowledges
/// owned process teardown; merely dropping a sender is not termination evidence.
pub trait AgentProcess: Send {
    fn execution_placement(&self) -> Option<crate::execution_placement::ExecutionPlacement> {
        None
    }
    /// Publish admission before author code may issue parent broker requests.
    fn start(&mut self) -> Result<(), WorkerError> {
        Ok(())
    }
    /// Out-of-band task gate. True acknowledgement means the runtime reached
    /// a safe checkpoint; a running author callback is never reported frozen.
    fn set_suspended(&mut self, suspended: bool) -> Result<(), WorkerError> {
        if suspended {
            Err(WorkerError::new(
                "InvalidStateError",
                "agent suspension unsupported",
            ))
        } else {
            Ok(())
        }
    }
    fn suspension_status(&mut self) -> Result<bool, WorkerError> {
        Ok(false)
    }
    fn send(&mut self, command: AgentCommand) -> Result<(), WorkerError>;
    fn poll(&mut self, max: usize) -> Result<Vec<AgentEvent>, WorkerError>;
    fn terminate(&mut self) -> Result<(), WorkerError>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SharedKey {
    partition: String,
    origin: String,
    name: Vec<u16>,
    unnamed_url: Option<String>,
}
struct Client {
    owner: AgentOwner,
    capability: AgentClientId,
    events: std::collections::VecDeque<AgentEvent>,
    bytes: usize,
}
struct Agent {
    bootstrap: AgentBootstrap,
    clients: BTreeMap<u64, Client>,
    process: Box<dyn AgentProcess>,
    shared: Option<SharedKey>,
    control: std::collections::VecDeque<AgentEvent>,
    control_bytes: usize,
    failure: Option<WorkerError>,
    terminated: bool,
    closed: bool,
    suspended: bool,
}
impl Agent {
    fn pump(&mut self, hub: &mut oryn_js::message_wire::MessageHub) -> Result<(), WorkerError> {
        use oryn_js::message_wire::Principal;
        let sender = Principal::Agent(self.bootstrap.id.0);
        let mut remapped = Vec::new();
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        if self.closed {
            return Ok(());
        }
        let result = (|| {
            let incoming = self.process.poll(32)?;
            if incoming.len() > 32 {
                return Err(error("worker exceeded event batch limit"));
            }
            let mut usage = self
                .clients
                .iter()
                .map(|(id, client)| (*id, (client.events.len(), client.bytes)))
                .collect::<BTreeMap<_, _>>();
            let mut control = (self.control.len(), self.control_bytes);
            let mut deliveries = Vec::new();
            let mut closed = false;
            for event in incoming {
                if closed {
                    return Err(error("agent emitted an event after Closed"));
                }
                if matches!(&event, AgentEvent::Closed) {
                    closed = true;
                }
                if let AgentEvent::Message { value, .. } = &event {
                    value
                        .validate()
                        .map_err(|error| WorkerError::new("DataCloneError", error))?;
                }
                let bytes = event_bytes(&event);
                let recipients = match &event {
                    AgentEvent::Ready | AgentEvent::Error { .. } | AgentEvent::Closed
                        if self.bootstrap.kind == AgentKind::Service =>
                    {
                        None
                    }
                    AgentEvent::LifecycleFinished { .. }
                    | AgentEvent::FetchResponse { .. }
                    | AgentEvent::FetchFallback { .. }
                    | AgentEvent::FetchResponseStart { .. }
                    | AgentEvent::FetchResponseChunk { .. }
                    | AgentEvent::FetchResponseEnd { .. } => None,
                    AgentEvent::Message {
                        client: Some(capability),
                        ..
                    } => Some(
                        self.clients
                            .iter()
                            .filter(|(_, client)| client.capability == *capability)
                            .map(|(id, _)| *id)
                            .collect::<Vec<_>>(),
                    ),
                    _ => Some(self.clients.keys().copied().collect()),
                };
                let reserve = |usage: &mut (usize, usize)| -> Result<(), WorkerError> {
                    usage.0 = usage.0.saturating_add(1);
                    usage.1 = usage.1.saturating_add(bytes);
                    if usage.0 > self.bootstrap.limits.queue_messages
                        || usage.1 > self.bootstrap.limits.queue_bytes
                    {
                        return Err(WorkerError::new(
                            "QuotaExceededError",
                            "worker event queue limit",
                        ));
                    }
                    Ok(())
                };
                match &recipients {
                    None => reserve(&mut control)?,
                    Some(ids) => {
                        for id in ids {
                            reserve(usage.get_mut(id).unwrap())?;
                        }
                    }
                }
                deliveries.push((event, recipients, bytes));
            }
            // Retain shared backing once, then mint a distinct delivery claim
            // for every live client. Cloning a first client's claim is invalid.
            let mut expanded = Vec::new();
            for (mut event, recipients, bytes) in deliveries {
                if let (AgentEvent::Message { value, .. }, Some(ids)) = (&mut event, &recipients) {
                    if ids.len() > 1 && !value.ports.is_empty() {
                        return Err(WorkerError::new(
                            "DataCloneError",
                            "transferred ports require one worker client destination",
                        ));
                    }
                    if ids.len() > 1 && !value.shared.is_empty() {
                        hub.retain_value(sender, value)
                            .map_err(|error| WorkerError::new("DataCloneError", error))?;
                        for id in ids {
                            expanded.push((event.clone(), Some(vec![*id]), bytes));
                        }
                        continue;
                    }
                }
                expanded.push((event, recipients, bytes));
            }
            let mut deliveries = expanded;
            // Remap only after the entire batch has reserved capacity. Ports
            // have one recipient; they cannot be duplicated by event fan-out.
            for (event, recipients, _) in &mut deliveries {
                if let AgentEvent::Message { value, .. } = event {
                    let ids = recipients
                        .as_ref()
                        .expect("message has client destinations");
                    if !value.ports.is_empty() && ids.len() > 1 {
                        return Err(WorkerError::new(
                            "DataCloneError",
                            "transferred ports require one worker client destination",
                        ));
                    }
                    let recipient = ids
                        .first()
                        .map(|id| self.clients[id].owner.principal())
                        .unwrap_or(sender);
                    let exact = ids
                        .first()
                        .map(|id| self.clients[id].owner.shared_recipient())
                        .unwrap_or(oryn_js::shared_memory::SharedMemoryRecipient::Agent(
                            self.bootstrap.id.0,
                        ));
                    hub.route_value_for_document(sender, exact, value)
                        .map_err(|error| WorkerError::new("DataCloneError", error))?;
                    let cleanup = value.cleanup_clone();
                    if ids.is_empty() {
                        hub.release_value(recipient, cleanup)
                            .map_err(|error| WorkerError::new("DataCloneError", error))?;
                    } else {
                        remapped.push((recipient, cleanup));
                    }
                }
            }
            // Commit only after every destination in this process batch fits.
            for (event, recipients, bytes) in deliveries {
                match recipients {
                    None => {
                        self.control_bytes += bytes;
                        self.control.push_back(event);
                    }
                    Some(ids) => {
                        for id in ids {
                            let client = self.clients.get_mut(&id).unwrap();
                            client.bytes += bytes;
                            client.events.push_back(event.clone());
                        }
                    }
                }
            }
            if closed {
                self.closed = true;
                hub.retire_agent(self.bootstrap.id.0);
            }
            Ok(())
        })();
        if let Err(failure) = result {
            for (recipient, value) in remapped {
                let _ = hub.release_value(recipient, value);
            }
            hub.retire_agent(self.bootstrap.id.0);
            // The broker caller drains failed processes after releasing locks.
            // Joining here can deadlock against an in-flight agent callback.
            self.failure = Some(failure.clone());
            return Err(failure);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceRetirementStatus {
    Live,
    Pending,
    Retired,
}

pub struct AgentRegistry {
    next_agent: u64,
    next_client: u64,
    max_agents: usize,
    agents: BTreeMap<AgentId, Agent>,
    shared: BTreeMap<SharedKey, AgentId>,
    retired: bool,
    retirement_inflight: bool,
    retirement_owner: Option<std::thread::ThreadId>,
    pending_termination: BTreeMap<AgentId, Agent>,
    service_retirements: BTreeMap<AgentId, ServiceRetirementStatus>,
    frozen_owners: std::collections::BTreeSet<AgentOwner>,
}

/// Owns revoked processes while their termination runs outside broker locks.
/// Failed processes remain owned here until returned to the retired registry.
pub struct DetachedAgentRetirement {
    completed_services: std::collections::BTreeSet<AgentId>,
    agents: BTreeMap<AgentId, Agent>,
    cleanup_failure: Option<WorkerError>,
    active: bool,
}
impl DetachedAgentRetirement {
    pub fn terminate(&mut self) -> Result<(), WorkerError> {
        let mut failure = self.cleanup_failure.take();
        let mut completed = Vec::new();
        for (id, agent) in &mut self.agents {
            if agent.terminated {
                completed.push(*id);
                continue;
            }
            // Teardown runs outside the broker locks. An implementation panic
            // must not unwind the whole batch, lose the remaining processes,
            // or strand the registry's retirement guard. Keep the failed
            // process quarantined and continue terminating its peers.
            let termination = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                agent.process.terminate()
            }))
            .unwrap_or_else(|_| {
                Err(WorkerError::new(
                    "InvalidStateError",
                    "agent termination panicked; process remains owned for teardown retry",
                ))
            });
            match termination {
                Ok(()) => {
                    agent.terminated = true;
                    completed.push(*id);
                }
                Err(error) => {
                    agent.failure = Some(error.clone());
                    failure.get_or_insert(error);
                }
            }
        }
        for id in completed {
            if self
                .agents
                .remove(&id)
                .is_some_and(|agent| agent.bootstrap.kind == AgentKind::Service)
            {
                self.completed_services.insert(id);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
fn error(message: impl Into<String>) -> WorkerError {
    WorkerError::new("InvalidStateError", message)
}
impl AgentRegistry {
    pub fn new(max_agents: usize) -> Self {
        Self {
            next_agent: 0,
            next_client: 0,
            max_agents,
            agents: BTreeMap::new(),
            shared: BTreeMap::new(),
            retired: false,
            retirement_inflight: false,
            retirement_owner: None,
            pending_termination: BTreeMap::new(),
            service_retirements: BTreeMap::new(),
            frozen_owners: Default::default(),
        }
    }

    /// Recompute reachability from runnable Window clients. Shared workers keep
    /// running while any live unfrozen owner remains; nested dedicated/shared
    /// agents inherit reachability rather than a creator-page shortcut.
    fn refresh_suspension(&mut self) -> Result<(), WorkerError> {
        let mut runnable = std::collections::BTreeSet::new();
        loop {
            let before = runnable.len();
            for (id, agent) in &self.agents {
                if agent.closed || agent.failure.is_some() {
                    continue;
                }
                if agent.bootstrap.kind == AgentKind::Service
                    || agent.clients.values().any(|client| match &client.owner {
                        AgentOwner::Window { .. } => !self.frozen_owners.contains(&client.owner),
                        AgentOwner::Agent(owner) => runnable.contains(owner),
                    })
                {
                    runnable.insert(*id);
                }
            }
            if before == runnable.len() {
                break;
            }
        }
        let mut failures = Vec::new();
        for (id, agent) in &mut self.agents {
            if agent.closed || agent.failure.is_some() {
                continue;
            }
            let suspended = !runnable.contains(id);
            if suspended != agent.suspended {
                match agent.process.set_suspended(suspended) {
                    Ok(()) => agent.suspended = suspended,
                    Err(error) => {
                        if suspended {
                            failures.push(error.to_string());
                        } else {
                            agent.failure = Some(error);
                            agent.suspended = false;
                        }
                    }
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(error(failures.join("; ")))
        }
    }
    pub fn freeze_document_owners(&mut self, owners: &[AgentOwner]) -> Result<(), WorkerError> {
        if self.retired {
            return Err(error("agent registry retired"));
        }
        let previous = self.frozen_owners.clone();
        self.frozen_owners.extend(owners.iter().cloned());
        if let Err(failure) = self.refresh_suspension() {
            self.frozen_owners = previous;
            let rollback = self.refresh_suspension();
            return Err(error(format!(
                "freeze failed: {failure}; rollback: {rollback:?}"
            )));
        }
        Ok(())
    }
    pub fn resume_document_owners(&mut self, owners: &[AgentOwner]) -> Result<(), WorkerError> {
        for owner in owners {
            self.frozen_owners.remove(owner);
        }
        self.refresh_suspension()
    }
    pub fn frozen_agents_quiescent(&mut self) -> Result<bool, WorkerError> {
        for agent in self.agents.values_mut() {
            if agent.suspended
                && !agent.closed
                && agent.failure.is_none()
                && !agent.process.suspension_status()?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// A registration owns its service process directly; it has no synthetic
    /// window client or client capability. The native loader validates policy
    /// before calling this admission boundary.
    pub fn launch_service(
        &mut self,
        mut bootstrap: AgentBootstrap,
        launch: impl FnOnce(&AgentBootstrap) -> Result<Box<dyn AgentProcess>, WorkerError>,
    ) -> Result<AgentId, WorkerError> {
        if bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "service admission requires Service kind",
            ));
        }
        if self.service_retirements.len() >= self.max_agents.saturating_mul(16).clamp(128, 65536) {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "unacknowledged service retirement capacity",
            ));
        }
        if self.retired {
            return Err(error("context agents are retired"));
        }
        if self.retirement_inflight {
            return Err(error("agent retirement already in progress"));
        }
        if self.agents.len() + self.pending_termination.len() >= self.max_agents {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "native agent capacity",
            ));
        }
        let mut url = url::Url::parse(&bootstrap.script_url)
            .map_err(|_| WorkerError::new("SyntaxError", "invalid service worker URL"))?;
        url.set_fragment(None);
        bootstrap.script_url = url.to_string();
        let next = self
            .next_agent
            .checked_add(1)
            .ok_or_else(|| error("agent identity exhausted"))?;
        let id = AgentId(next);
        bootstrap.id = id;
        let process = launch(&bootstrap)?;
        self.agents.insert(
            id,
            Agent {
                bootstrap,
                clients: BTreeMap::new(),
                process,
                shared: None,
                control: Default::default(),
                control_bytes: 0,
                failure: None,
                terminated: false,
                closed: false,
                suspended: false,
            },
        );
        self.next_agent = next;
        if let Err(failure) = self.agents.get_mut(&id).unwrap().process.start() {
            let mut agent = self.agents.remove(&id).unwrap();
            agent.failure = Some(failure.clone());
            self.pending_termination.insert(id, agent);
            return Err(failure);
        }
        self.service_retirements
            .insert(id, ServiceRetirementStatus::Live);
        Ok(id)
    }

    pub fn send_service_control(
        &mut self,
        id: AgentId,
        command: AgentCommand,
    ) -> Result<(), WorkerError> {
        self.principal(id)?;
        let agent = self.agents.get_mut(&id).unwrap();
        if agent.bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "control target is not a service worker",
            ));
        }
        if !matches!(
            &command,
            AgentCommand::Lifecycle { .. }
                | AgentCommand::Fetch(..)
                | AgentCommand::CancelTask { .. }
                | AgentCommand::FetchPull { .. }
        ) {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "invalid service control command",
            ));
        }
        if let AgentCommand::Lifecycle {
            payload: Some(value),
            ..
        } = &command
        {
            value
                .validate()
                .map_err(|message| WorkerError::new("DataCloneError", message))?;
            if !value.ports.is_empty() {
                return Err(WorkerError::new(
                    "DataCloneError",
                    "service lifecycle payload cannot transfer client ports",
                ));
            }
            if value.retained_bytes() > agent.bootstrap.limits.queue_bytes {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "service lifecycle payload limit",
                ));
            }
        }
        agent.process.send(command)
    }

    /// The service registry supplies an authenticated client snapshot after
    /// checking the exact Window document lifetime. Agent principals can also
    /// be checked here; caller-supplied URLs never establish their authority.
    pub fn post_service_message(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        id: AgentId,
        task: u64,
        source: oryn_js::service_worker_wire::MessageSource,
        mut value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        use oryn_js::message_wire::Principal;
        use oryn_js::service_worker_wire::{ClientType, MessageSource};
        value
            .validate()
            .map_err(|message| WorkerError::new("DataCloneError", message))?;
        let (partition, origin) = self.principal(id)?;
        let recipient = self.agents.get(&id).unwrap();
        if recipient.bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "message target is not a service worker",
            ));
        }
        let (claimed_origin, metadata) = match &source {
            MessageSource::Client { client, origin } => (
                origin,
                client
                    .id
                    .len()
                    .saturating_add(client.url.len())
                    .saturating_add(client.frame_type.len())
                    .saturating_add(client.visibility_state.len())
                    .saturating_add(origin.len()),
            ),
            MessageSource::ServiceWorker { worker, origin } => {
                (origin, worker.script_url.len().saturating_add(origin.len()))
            }
        };
        if claimed_origin != &origin.serialized {
            return Err(WorkerError::new(
                "SecurityError",
                "service message origin mismatch",
            ));
        }
        match owner {
            AgentOwner::Window { .. } => {
                if !matches!(&source,MessageSource::Client{client,..} if client.client_type==ClientType::Window)
                {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "window source requires a native window client snapshot",
                    ));
                }
            }
            AgentOwner::Agent(sender) => {
                let (sender_partition, sender_origin) = self.principal(*sender)?;
                if sender_partition != partition || sender_origin.authority != origin.authority {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "service message crosses native storage authority",
                    ));
                }
                let bootstrap = &self.agents[sender].bootstrap;
                let matches = match &source {
                    MessageSource::ServiceWorker { worker, .. } => {
                        bootstrap.kind == AgentKind::Service
                            && worker.id == *sender
                            && worker.script_url == bootstrap.script_url
                    }
                    MessageSource::Client { client, .. } => matches!(
                        (bootstrap.kind, &client.client_type),
                        (AgentKind::Dedicated, ClientType::DedicatedWorker)
                            | (AgentKind::Shared, ClientType::SharedWorker)
                    ),
                };
                if !matches {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "service message source snapshot mismatch",
                    ));
                }
            }
        }
        if value.retained_bytes().saturating_add(metadata) > recipient.bootstrap.limits.queue_bytes
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "service message payload limit",
            ));
        }
        let recipient = Principal::Agent(id.0);
        hub.route_value(owner.principal(), recipient, &mut value)
            .map_err(|message| WorkerError::new("DataCloneError", message))?;
        let cleanup = value.cleanup_clone();
        match self
            .agents
            .get_mut(&id)
            .unwrap()
            .process
            .send(AgentCommand::ServiceMessage {
                id: task,
                source,
                value,
            }) {
            Ok(()) => Ok(()),
            Err(failure) => {
                hub.release_value(recipient, cleanup).map_err(|cleanup| {
                    WorkerError::new(
                        "DataCloneError",
                        format!("{failure}; transfer cleanup: {cleanup}"),
                    )
                })?;
                Err(failure)
            }
        }
    }

    /// Only the native profile bridge may supply already-remapped endpoints.
    /// Its caller validates both live contexts and exact source authority in
    /// one ordered admission snapshot before invoking this process route.
    pub(crate) fn post_profile_service_message(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        id: AgentId,
        task: u64,
        source: oryn_js::service_worker_wire::MessageSource,
        value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        use oryn_js::{message_wire::Principal, service_worker_wire::MessageSource};
        let (_, origin) = self.principal(id)?;
        let claimed = match &source {
            MessageSource::Client { origin, .. } | MessageSource::ServiceWorker { origin, .. } => {
                origin
            }
        };
        if claimed != &origin.serialized {
            return Err(WorkerError::new(
                "SecurityError",
                "profile service source origin mismatch",
            ));
        }
        let agent = self.agents.get_mut(&id).unwrap();
        if agent.bootstrap.kind != AgentKind::Service {
            return Err(error("profile target is not a service worker"));
        }
        let metadata = serde_json::to_vec(&source)
            .map_err(|failure| error(failure.to_string()))?
            .len();
        if metadata.saturating_add(value.retained_bytes()) > agent.bootstrap.limits.queue_bytes {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "profile service message queue limit",
            ));
        }
        let recipient = Principal::Agent(id.0);
        hub.validate_received_value(recipient, &value)
            .map_err(|failure| WorkerError::new("DataCloneError", failure))?;
        let cleanup = value.cleanup_clone();
        match agent.process.send(AgentCommand::ServiceMessage {
            id: task,
            source,
            value,
        }) {
            Ok(()) => Ok(()),
            Err(failure) => {
                hub.dispose_queued_value(recipient, cleanup)
                    .map_err(|cleanup| error(format!("{failure}; {cleanup}")))?;
                Err(failure)
            }
        }
    }

    pub fn retire_service(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        id: AgentId,
    ) -> Result<(), WorkerError> {
        if self.service_retirements.get(&id) == Some(&ServiceRetirementStatus::Retired) {
            return Ok(());
        }
        let Some(agent) = self.agents.get(&id) else {
            return if self.service_retirements.get(&id) == Some(&ServiceRetirementStatus::Pending)
                || self
                    .pending_termination
                    .get(&id)
                    .is_some_and(|agent| agent.bootstrap.kind == AgentKind::Service)
            {
                Ok(())
            } else {
                Err(error("unknown service worker"))
            };
        };
        if agent.bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "retirement target is not a service worker",
            ));
        }
        self.remove(hub, id)
    }

    /// Only IDs admitted by this registry have retirement receipts. An absent
    /// executable/principal alone never proves a service process terminated.
    pub fn service_retirement_status(
        &self,
        id: AgentId,
    ) -> Result<ServiceRetirementStatus, WorkerError> {
        self.service_retirements
            .get(&id)
            .copied()
            .ok_or_else(|| error("unknown or acknowledged service retirement"))
    }
    pub fn acknowledge_service_retirement(&mut self, id: AgentId) -> Result<(), WorkerError> {
        if self.service_retirement_status(id)? != ServiceRetirementStatus::Retired {
            return Err(error("service termination has not completed"));
        }
        self.service_retirements.remove(&id);
        Ok(())
    }

    /// `bootstrap` has already passed parent URL/origin/policy validation. This
    /// registry never accepts author-supplied source or storage partition claims.
    pub fn connect(
        &mut self,
        owner: AgentOwner,
        mut bootstrap: AgentBootstrap,
        launch: impl FnOnce(&AgentBootstrap) -> Result<Box<dyn AgentProcess>, WorkerError>,
    ) -> Result<AgentConnection, WorkerError> {
        if self.retired {
            return Err(error("context agents are retired"));
        }
        if self.retirement_inflight {
            return Err(error("agent retirement already in progress"));
        }
        let mut url = url::Url::parse(&bootstrap.script_url)
            .map_err(|_| WorkerError::new("SyntaxError", "invalid worker URL"))?;
        url.set_fragment(None);
        bootstrap.script_url = url.to_string();
        let key = (bootstrap.kind == AgentKind::Shared).then(|| SharedKey {
            partition: bootstrap.partition.clone(),
            origin: bootstrap.origin.authority.clone(),
            name: bootstrap.options.name.clone(),
            unnamed_url: bootstrap
                .options
                .name
                .is_empty()
                .then(|| bootstrap.script_url.clone()),
        });
        let existing = key
            .as_ref()
            .and_then(|key| self.shared.get(key))
            .copied()
            .filter(|id| self.agents.get(id).is_some_and(|agent| !agent.closed));
        if let Some(id) = existing {
            let agent = self
                .agents
                .get(&id)
                .ok_or_else(|| error("missing shared worker"))?;
            if let Some(failure) = &agent.failure {
                return Err(failure.clone());
            }
            let original = &agent.bootstrap;
            if agent.clients.len() >= original.limits.queue_messages {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "worker client capacity",
                ));
            }
            if original.secure_context != bootstrap.secure_context {
                return Err(WorkerError::new(
                    "SecurityError",
                    "shared worker secure-context state differs",
                ));
            }
            if crate::agent_isolation::worker_isolated_capability(original)
                != crate::agent_isolation::worker_isolated_capability(&bootstrap)
            {
                return Err(WorkerError::new(
                    "SecurityError",
                    "shared worker isolation capability differs",
                ));
            }
            if original.script_url != bootstrap.script_url
                || original.options.script_type != bootstrap.options.script_type
                || original.options.credentials != bootstrap.options.credentials
            {
                return Err(WorkerError::new(
                    "URLMismatchError",
                    "existing shared worker URL, type or credentials differ",
                ));
            }
        } else if self.agents.len() + self.pending_termination.len() >= self.max_agents {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "native agent capacity",
            ));
        }
        let client_id = self
            .next_client
            .checked_add(1)
            .ok_or_else(|| error("agent client identity exhausted"))?;
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).map_err(|_| error("worker client entropy unavailable"))?;
        let capability = AgentClientId {
            id: client_id,
            secret,
        };
        let id = if let Some(id) = existing {
            id
        } else {
            let next = self
                .next_agent
                .checked_add(1)
                .ok_or_else(|| error("agent identity exhausted"))?;
            let id = AgentId(next);
            bootstrap.id = id;
            let process = launch(&bootstrap)?;
            self.agents.insert(
                id,
                Agent {
                    bootstrap,
                    clients: BTreeMap::new(),
                    process,
                    shared: key.clone(),
                    control: Default::default(),
                    control_bytes: 0,
                    failure: None,
                    terminated: false,
                    closed: false,
                    suspended: false,
                },
            );
            if let Some(key) = key {
                self.shared.insert(key, id);
            }
            self.next_agent = next;
            id
        };
        self.next_client = client_id;
        self.agents.get_mut(&id).unwrap().clients.insert(
            client_id,
            Client {
                owner,
                capability: capability.clone(),
                events: Default::default(),
                bytes: 0,
            },
        );
        self.refresh_suspension()?;
        Ok(AgentConnection {
            agent: id,
            client: capability,
            port: None,
        })
    }

    /// Complete parent-side admission, including the native SharedWorker port
    /// pair. A failed connection leaves existing shared clients untouched.
    pub fn connect_with_ports(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: AgentOwner,
        bootstrap: AgentBootstrap,
        launch: impl FnOnce(&AgentBootstrap) -> Result<Box<dyn AgentProcess>, WorkerError>,
    ) -> Result<AgentConnection, WorkerError> {
        use oryn_js::message_wire::{Principal, WireValue};
        let shared = bootstrap.kind == AgentKind::Shared;
        let mut connection = self.connect(owner.clone(), bootstrap, launch)?;
        if !shared {
            if let Err(error) = self.authorized(&owner, &connection)?.process.start() {
                let _ = self.release(hub, &owner, &connection);
                return Err(error);
            }
            return Ok(connection);
        }
        let agent_principal = Principal::Agent(connection.agent.0);
        let pair = match hub.create_entangled(owner.principal(), agent_principal) {
            Ok(pair) => pair,
            Err(message) => {
                let original = WorkerError::new("QuotaExceededError", message);
                let cleanup = self.release(hub, &owner, &connection);
                return Err(match cleanup {
                    Ok(()) => original,
                    Err(cleanup) => WorkerError::new(
                        &original.name,
                        format!("{}; rollback: {cleanup}", original.message),
                    ),
                });
            }
        };
        let [client_port, agent_port] = pair;
        let result = self
            .authorized(&owner, &connection)?
            .process
            .send(AgentCommand::Connect {
                client: connection.client.clone(),
                port: agent_port.clone(),
            })
            .and_then(|()| self.authorized(&owner, &connection)?.process.start());
        if let Err(original) = result {
            let mut failures = Vec::new();
            for (principal, port) in [
                (owner.principal(), client_port),
                (agent_principal, agent_port),
            ] {
                let value = WireValue {
                    shared: Vec::new(),
                    shared_queue: None,
                    shared_recipient: None,
                    wasm_modules: Vec::new(),
                    version: 1,
                    bytes: Vec::new(),
                    hosts: Vec::new(),
                    buffers: Vec::new(),
                    ports: vec![port],
                };
                if let Err(error) = hub.release_value(principal, value) {
                    failures.push(error);
                }
            }
            if let Err(error) = self.release(hub, &owner, &connection) {
                failures.push(error.to_string());
            }
            return if failures.is_empty() {
                Err(original)
            } else {
                Err(WorkerError::new(
                    &original.name,
                    format!("{}; rollback: {}", original.message, failures.join("; ")),
                ))
            };
        }
        connection.port = Some(client_port);
        Ok(connection)
    }

    fn authorized(
        &mut self,
        owner: &AgentOwner,
        connection: &AgentConnection,
    ) -> Result<&mut Agent, WorkerError> {
        if self.retired {
            return Err(error("context agents are retired"));
        }
        let agent = self
            .agents
            .get_mut(&connection.agent)
            .ok_or_else(|| error("worker no longer exists"))?;
        let client = agent
            .clients
            .get(&connection.client.id)
            .ok_or_else(|| error("worker client is retired"))?;
        if &client.owner != owner || client.capability != connection.client {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign worker client capability",
            ));
        }
        Ok(agent)
    }

    pub fn worklet(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        connection: &AgentConnection,
        mut command: oryn_js::worker_wire::WorkletCommand,
    ) -> Result<(), WorkerError> {
        let agent = self.authorized(owner, connection)?;
        if agent.bootstrap.kind != AgentKind::AudioWorklet || agent.closed || agent.terminated {
            return Err(error("worklet owner unavailable"));
        }
        let recipient = oryn_js::message_wire::Principal::Agent(connection.agent.0);
        let cleanup = match &mut command {
            oryn_js::worker_wire::WorkletCommand::LoadModule { url, .. } => {
                if url.len() > 65536 {
                    return Err(error("worklet URL bound"));
                }
                None
            }
            oryn_js::worker_wire::WorkletCommand::Construct { name, value, .. } => {
                if name.is_empty() || name.len() > 65536 {
                    return Err(error("worklet processor name bound"));
                }
                value.validate().map_err(error)?;
                if value.retained_bytes() > agent.bootstrap.limits.queue_bytes {
                    return Err(error("worklet options bound"));
                }
                hub.route_value(owner.principal(), recipient, value)
                    .map_err(error)?;
                Some(value.cleanup_clone())
            }
        };
        match agent.process.send(AgentCommand::Worklet(command)) {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Some(cleanup) = cleanup {
                    hub.release_value(recipient, cleanup).map_err(|cleanup| {
                        WorkerError::new("DataCloneError", format!("{error}; {cleanup}"))
                    })?;
                }
                Err(error)
            }
        }
    }

    pub fn post(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        connection: &AgentConnection,
        mut value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        value
            .validate()
            .map_err(|error| WorkerError::new("DataCloneError", error))?;
        let agent = self.authorized(owner, connection)?;
        if let Some(failure) = &agent.failure {
            return Err(failure.clone());
        }
        if agent.closed {
            return Err(error("worker agent closed"));
        }
        if value.retained_bytes() > agent.bootstrap.limits.queue_bytes {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "worker message byte limit",
            ));
        }
        let recipient = oryn_js::message_wire::Principal::Agent(connection.agent.0);
        hub.route_value(owner.principal(), recipient, &mut value)
            .map_err(|error| WorkerError::new("DataCloneError", error))?;
        // Only retain capability metadata for rejected-queue cleanup, not a
        // duplicate copy of potentially large cloned buffers/blob payloads.
        let cleanup = value.cleanup_clone();
        match agent.process.send(AgentCommand::Message {
            client: Some(connection.client.clone()),
            value,
        }) {
            Ok(()) => Ok(()),
            Err(error) => {
                hub.release_value(recipient, cleanup).map_err(|cleanup| {
                    WorkerError::new(
                        "DataCloneError",
                        format!("{error}; transfer cleanup: {cleanup}"),
                    )
                })?;
                Err(error)
            }
        }
    }

    pub fn poll(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        connection: &AgentConnection,
        max: usize,
    ) -> Result<Vec<AgentEvent>, WorkerError> {
        let agent = self.authorized(owner, connection)?;
        // Deliver this client's already admitted events before requesting a
        // further process batch. Small polls must not fill the same queue again
        // or let a later process failure replace its accepted ordered events.
        if agent.clients[&connection.client.id].events.is_empty() {
            agent.pump(hub)?;
        }
        let client = agent.clients.get_mut(&connection.client.id).unwrap();
        let mut result = Vec::new();
        for _ in 0..max.min(32) {
            let Some(event) = client.events.pop_front() else {
                break;
            };
            client.bytes = client.bytes.saturating_sub(event_bytes(&event));
            result.push(event);
        }
        if agent.closed {
            self.retire_owner(hub, &AgentOwner::Agent(connection.agent))?;
        }
        Ok(result)
    }

    /// Native parent consumers alone receive install/activate/fetch results.
    pub fn poll_control(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        id: AgentId,
        max: usize,
    ) -> Result<Vec<AgentEvent>, WorkerError> {
        if self.retired {
            return Err(error("context agents are retired"));
        }
        let agent = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| error("unknown worker agent"))?;
        // Preserve accepted control receipts before observing a later process
        // failure, including when a consumer drains one event at a time.
        if agent.control.is_empty() {
            agent.pump(hub)?;
        }
        let mut events = Vec::new();
        for _ in 0..max.min(32) {
            let Some(event) = agent.control.pop_front() else {
                break;
            };
            agent.control_bytes = agent.control_bytes.saturating_sub(event_bytes(&event));
            events.push(event);
        }
        if agent.closed {
            self.retire_owner(hub, &AgentOwner::Agent(id))?;
        }
        Ok(events)
    }

    /// Resolve only a genuine client capability; request envelopes do not supply
    /// an origin, partition, or replacement document authority.
    pub fn connection_owner(
        &self,
        connection: &AgentConnection,
    ) -> Result<AgentOwner, WorkerError> {
        if self.retired {
            return Err(error("context agents are retired"));
        }
        let agent = self
            .agents
            .get(&connection.agent)
            .ok_or_else(|| error("unknown worker agent"))?;
        if let Some(failure) = &agent.failure {
            return Err(failure.clone());
        }
        let client = agent
            .clients
            .get(&connection.client.id)
            .ok_or_else(|| error("worker client is retired"))?;
        if client.capability != connection.client {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign worker client capability",
            ));
        }
        Ok(client.owner.clone())
    }
    /// Release is an idempotent revocation. Monotonic agent/client identifiers
    /// cannot be reused, so absence consumes a retry without a tombstone table.
    /// An extant client still requires its exact random capability.
    pub(crate) fn release_owner(
        &self,
        connection: &AgentConnection,
    ) -> Result<Option<AgentOwner>, WorkerError> {
        let Some(agent) = self.agents.get(&connection.agent) else {
            return Ok(None);
        };
        let Some(client) = agent.clients.get(&connection.client.id) else {
            return Ok(None);
        };
        if client.capability != connection.client {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign worker client capability",
            ));
        }
        Ok(Some(client.owner.clone()))
    }
    /// Immutable parent bootstrap used for nested creation and authorized import
    /// loading. The source worker cannot replace its URL, credentials or origin.
    pub fn live_service_client_agents(&self) -> Vec<AgentId> {
        if self.retired {
            return Vec::new();
        }
        self.agents
            .iter()
            .filter(|(_, agent)| {
                agent.bootstrap.kind != AgentKind::Service
                    && !agent.closed
                    && !agent.terminated
                    && agent.failure.is_none()
            })
            .map(|(id, _)| *id)
            .collect()
    }
    pub fn bootstrap_snapshot(&self, id: AgentId) -> Result<AgentBootstrap, WorkerError> {
        self.principal(id)?;
        Ok(self
            .agents
            .get(&id)
            .ok_or_else(|| error("unknown worker agent"))?
            .bootstrap
            .clone())
    }

    pub fn principal(&self, id: AgentId) -> Result<(&str, &AgentOrigin), WorkerError> {
        if self.retired {
            return Err(error("context agents are retired"));
        }
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| error("unknown worker agent"))?;
        if let Some(failure) = &agent.failure {
            return Err(failure.clone());
        }
        if agent.terminated || agent.closed {
            return Err(error("worker agent terminated or closed"));
        }
        Ok((&agent.bootstrap.partition, &agent.bootstrap.origin))
    }

    pub fn release(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        connection: &AgentConnection,
    ) -> Result<(), WorkerError> {
        let agent = self.authorized(owner, connection)?;
        let client = agent.clients.remove(&connection.client.id).unwrap();
        let cleanup = dispose_client(hub, client);
        let retire = if agent.clients.is_empty()
            && (agent.closed || agent.bootstrap.kind != AgentKind::Service)
        {
            self.remove(hub, connection.agent)
        } else {
            Ok(())
        };
        let suspension = self.refresh_suspension();
        cleanup.and(retire).and(suspension)
    }
    pub fn terminate(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
        connection: &AgentConnection,
    ) -> Result<(), WorkerError> {
        if self.authorized(owner, connection)?.bootstrap.kind != AgentKind::Dedicated {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "only a dedicated worker client may terminate its agent",
            ));
        }
        self.remove(hub, connection.agent)
    }
    fn remove(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        id: AgentId,
    ) -> Result<(), WorkerError> {
        let Some(agent) = self.agents.get_mut(&id) else {
            return Ok(());
        };
        let mut failure = None;
        for client in std::mem::take(&mut agent.clients).into_values() {
            if let Err(error) = dispose_client(hub, client) {
                failure.get_or_insert(error);
            }
        }
        let agent = self.agents.remove(&id).unwrap();
        if let Some(key) = &agent.shared
            && self.shared.get(key) == Some(&id)
        {
            self.shared.remove(key);
        }
        // Dedicated descendants must not survive their owning worker.
        if let Err(error) = self.retire_owner(hub, &AgentOwner::Agent(id)) {
            failure.get_or_insert(error);
        }
        hub.retire_agent(id.0);
        if let Some(status) = self.service_retirements.get_mut(&id) {
            *status = ServiceRetirementStatus::Pending;
        }
        self.pending_termination.insert(id, agent);
        failure.map_or(Ok(()), Err)
    }
    pub fn retire_owner(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        owner: &AgentOwner,
    ) -> Result<(), WorkerError> {
        self.retire_matching_owners(hub, |candidate| candidate == owner)
    }

    /// Retire one exact document lifetime. A delayed old-document notification
    /// must not revoke workers created by its successor in the same frame.
    pub fn retire_document(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        page: PageId,
        target: WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Result<(), WorkerError> {
        self.retire_owner(
            hub,
            &AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            },
        )
    }

    /// Page teardown includes all hosted frames and document generations, while
    /// shared workers with clients in other pages retain their own lifetimes.
    pub fn retire_page(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        page: PageId,
    ) -> Result<(), WorkerError> {
        self.retire_matching_owners(
            hub,
            |owner| matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page),
        )
    }

    fn retire_matching_owners(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        matches: impl Fn(&AgentOwner) -> bool,
    ) -> Result<(), WorkerError> {
        self.frozen_owners.retain(|owner| !matches(owner));
        let mut remove = Vec::new();
        let mut failure = None;
        for (id, agent) in &mut self.agents {
            let clients = agent
                .clients
                .iter()
                .filter(|(_, client)| matches(&client.owner))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            for id in clients {
                if let Some(client) = agent.clients.remove(&id)
                    && let Err(error) = dispose_client(hub, client)
                {
                    failure.get_or_insert(error);
                }
            }
            if agent.clients.is_empty() && agent.bootstrap.kind != AgentKind::Service {
                remove.push(*id);
            }
        }
        for id in remove {
            if let Err(error) = self.remove(hub, id) {
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = self.refresh_suspension() {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    }
    pub fn retire(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
    ) -> Result<(), WorkerError> {
        let mut batch = self.detach_retirement(hub)?;
        let result = batch.terminate();
        self.retain_failed_retirement(batch);
        result
    }

    /// Wait only for the current detached owner, without holding broker locks.
    /// A timeout is failure, not evidence that the owned processes have exited.
    /// Callers must not hold the message hub lock while entering this function.
    pub(crate) fn await_retirement<'a>(
        registry: std::sync::MutexGuard<'a, Self>,
        done: &std::sync::Condvar,
        budget: std::time::Duration,
    ) -> Result<std::sync::MutexGuard<'a, Self>, WorkerError> {
        let started = std::time::Instant::now();
        let mut registry = registry;
        while registry.retirement_inflight {
            if registry.retirement_owner == Some(std::thread::current().id()) {
                return Err(error(
                    "reentrant agent retirement cannot wait for its own owner",
                ));
            }
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(error(
                    "agent retirement handoff deadline exceeded; processes remain owned",
                ));
            }
            registry = done
                .wait_timeout(registry, remaining)
                .map_err(|_| error("agent registry lock poisoned during retirement handoff"))?
                .0;
        }
        Ok(registry)
    }

    /// Revoke all admissions while locked, but do not join any agent here.
    /// Call terminate only after releasing both registry and message locks.
    pub fn detach_retirement(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
    ) -> Result<DetachedAgentRetirement, WorkerError> {
        if self.retirement_inflight {
            return Err(error("agent retirement already in progress"));
        }
        self.retired = true;
        self.retirement_inflight = true;
        self.retirement_owner = Some(std::thread::current().id());
        self.shared.clear();
        let mut agents = std::mem::take(&mut self.agents);
        let mut failure = hub
            .retry_shared_disposals()
            .err()
            .map(|error| WorkerError::new("DataCloneError", error));
        agents.append(&mut self.pending_termination);
        // Dispose every client's queued transfers before retiring principals:
        // a client may itself belong to another agent in this same batch.
        for agent in agents.values_mut() {
            for client in std::mem::take(&mut agent.clients).into_values() {
                if let Err(error) = dispose_client(hub, client) {
                    failure.get_or_insert(error);
                }
            }
            agent.control.clear();
            agent.control_bytes = 0;
        }
        for id in agents.keys() {
            hub.retire_agent(id.0);
            if let Some(status) = self.service_retirements.get_mut(id) {
                *status = ServiceRetirementStatus::Pending;
            }
        }
        Ok(DetachedAgentRetirement {
            completed_services: Default::default(),
            agents,
            cleanup_failure: failure,
            active: true,
        })
    }

    /// Preserve failed termination ownership for a later teardown attempt.
    pub fn retain_failed_retirement(&mut self, batch: DetachedAgentRetirement) {
        if batch.active {
            self.retirement_inflight = false;
            self.retirement_owner = None;
        }
        for id in batch.completed_services {
            if let Some(status) = self.service_retirements.get_mut(&id) {
                *status = ServiceRetirementStatus::Retired;
            }
        }
        for (id, agent) in batch.agents {
            // Retired registries cannot launch or admit replacement agents.
            debug_assert!(!self.agents.contains_key(&id));
            self.agents.insert(id, agent);
        }
    }

    /// Broker dispatch drains this batch outside registry/message locks after
    /// release, termination, failed creation, or a terminal poll failure.
    pub fn detach_pending_retirement(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
    ) -> Result<DetachedAgentRetirement, WorkerError> {
        let shared_cleanup = hub
            .retry_shared_disposals()
            .err()
            .map(|error| WorkerError::new("DataCloneError", error));
        if self.pending_termination.is_empty()
            && self.agents.values().all(|agent| {
                agent.failure.is_none()
                    && !(agent.closed && agent.clients.is_empty() && agent.control.is_empty())
            })
        {
            return Ok(DetachedAgentRetirement {
                completed_services: Default::default(),
                agents: BTreeMap::new(),
                cleanup_failure: shared_cleanup,
                active: false,
            });
        }
        if self.retirement_inflight {
            return Err(error("agent retirement already in progress"));
        }
        let failed = self
            .agents
            .iter()
            .filter(|(_, agent)| {
                agent.failure.is_some()
                    || (agent.closed && agent.clients.is_empty() && agent.control.is_empty())
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut failure = shared_cleanup;
        for id in failed {
            if let Err(error) = self.remove(hub, id) {
                failure.get_or_insert(error);
            }
        }
        self.retirement_inflight = true;
        self.retirement_owner = Some(std::thread::current().id());
        Ok(DetachedAgentRetirement {
            completed_services: Default::default(),
            agents: std::mem::take(&mut self.pending_termination),
            cleanup_failure: failure,
            active: true,
        })
    }

    pub fn detach_page_retirement(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        page: PageId,
    ) -> Result<DetachedAgentRetirement, WorkerError> {
        self.detach_matching_retirement(
            hub,
            |owner| matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page),
        )
    }

    pub fn detach_document_retirement(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        page: PageId,
        target: WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Result<DetachedAgentRetirement, WorkerError> {
        let owner = AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        };
        self.detach_matching_retirement(hub, |candidate| candidate == &owner)
    }

    fn detach_matching_retirement(
        &mut self,
        hub: &mut oryn_js::message_wire::MessageHub,
        matches: impl Fn(&AgentOwner) -> bool,
    ) -> Result<DetachedAgentRetirement, WorkerError> {
        if self.retirement_inflight {
            return Err(error("agent retirement already in progress"));
        }
        self.retirement_inflight = true;
        self.retirement_owner = Some(std::thread::current().id());
        let mut removed = std::collections::BTreeSet::new();
        let mut failure = hub
            .retry_shared_disposals()
            .err()
            .map(|error| WorkerError::new("DataCloneError", error));
        loop {
            let before = removed.len();
            for (id, agent) in &mut self.agents {
                let clients = agent
                    .clients
                    .iter()
                    .filter(|(_, client)| {
                        matches(&client.owner)
                            || matches!(&client.owner,AgentOwner::Agent(id) if removed.contains(id))
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in clients {
                    if let Some(client) = agent.clients.remove(&id)
                        && let Err(error) = dispose_client(hub, client)
                    {
                        failure.get_or_insert(error);
                    }
                }
                if agent.clients.is_empty() && agent.bootstrap.kind != AgentKind::Service {
                    removed.insert(*id);
                }
            }
            if removed.len() == before {
                break;
            }
        }
        let mut agents = std::mem::take(&mut self.pending_termination);
        for id in removed {
            if let Some(mut agent) = self.agents.remove(&id) {
                if let Some(key) = &agent.shared
                    && self.shared.get(key) == Some(&id)
                {
                    self.shared.remove(key);
                }
                agent.control.clear();
                agent.control_bytes = 0;
                hub.retire_agent(id.0);
                agents.insert(id, agent);
            }
        }
        Ok(DetachedAgentRetirement {
            completed_services: Default::default(),
            agents,
            cleanup_failure: failure,
            active: true,
        })
    }
}
fn dispose_client(
    hub: &mut oryn_js::message_wire::MessageHub,
    client: Client,
) -> Result<(), WorkerError> {
    let principal = client.owner.principal();
    let mut failure = None;
    for event in client.events {
        if let AgentEvent::Message { mut value, .. } = event {
            // A stale/foreign port cannot prevent releasing independent valid
            // ports or shared delivery claims. Never duplicate backing bytes.
            for port in std::mem::take(&mut value.ports) {
                let port_value = oryn_js::message_wire::WireValue {
                    shared: Vec::new(),
                    shared_queue: None,
                    shared_recipient: None,
                    wasm_modules: Vec::new(),
                    version: 1,
                    bytes: Vec::new(),
                    hosts: Vec::new(),
                    buffers: Vec::new(),
                    ports: vec![port],
                };
                if let Err(error) = hub.dispose_queued_value(principal, port_value) {
                    failure.get_or_insert(WorkerError::new("DataCloneError", error));
                }
            }
            if let Err(error) = hub.dispose_queued_value(principal, value) {
                failure.get_or_insert(WorkerError::new("DataCloneError", error));
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
fn event_bytes(event: &AgentEvent) -> usize {
    match event {
        AgentEvent::Message { value, .. } => value.retained_bytes().saturating_add(64),
        AgentEvent::Error {
            message, filename, ..
        } => message
            .len()
            .saturating_mul(2)
            .saturating_add(filename.len()),
        AgentEvent::FetchResponse {
            response: Ok(response),
            ..
        }
        | AgentEvent::FetchResponseStart { response, .. } => response
            .body
            .len()
            .saturating_add(response.status_text.len())
            .saturating_add(response.response_url.len())
            .saturating_add(response.response_type.len())
            .saturating_add(response.headers.iter().fold(0usize, |total, (a, b)| {
                total.saturating_add(a.len()).saturating_add(b.len())
            })),
        AgentEvent::FetchResponseChunk { bytes, .. } => bytes.len().saturating_add(32),
        AgentEvent::FetchResponseEnd {
            result: Err(error), ..
        }
        | AgentEvent::FetchResponse {
            response: Err(error),
            ..
        }
        | AgentEvent::LifecycleFinished {
            result: Err(error), ..
        } => error.name.len().saturating_add(error.message.len()),
        _ => 128,
    }
}

impl AgentRegistry {
    pub(crate) fn execution_placements(&self) -> Vec<serde_json::Value> {
        self.agents.iter().map(|(id,agent)|serde_json::json!({"id":id.0,"kind":agent.bootstrap.kind,"script_url":agent.bootstrap.script_url,
            "live":!agent.closed&&!agent.terminated&&agent.failure.is_none(),"placement":agent.process.execution_placement(),
            "owners":agent.clients.values().map(|client|crate::execution_placement::owner(&client.owner)).collect::<Vec<_>>()})).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn inactive_dispatch_settlement_does_not_release_page_retirement_owner() {
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let page = registry.detach_page_retirement(&mut hub, PageId(1)).unwrap();
        let owner = registry.retirement_owner;
        // Reentrant broker progress may produce an empty, inactive batch while
        // another owner terminates outside the registry and hub locks.
        let mut dispatch = registry.detach_pending_retirement(&mut hub).unwrap();
        dispatch.terminate().unwrap();
        registry.retain_failed_retirement(dispatch);
        assert!(registry.retirement_inflight);
        assert_eq!(registry.retirement_owner, owner);
        assert!(registry.detach_retirement(&mut hub).is_err());
        registry.retain_failed_retirement(page);
        assert!(!registry.retirement_inflight);
        assert!(registry.retirement_owner.is_none());
    }

    #[test]
    fn page_retirement_handoff_preserves_shared_peer_until_context_cleanup() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let state = Arc::new(Mutex::new(State::default()));
        let process = state.clone();
        let first = registry.lock().unwrap().connect(owner(1), bootstrap(), move |_| {
            Ok(Box::new(Process(process)))
        }).unwrap();
        let second = registry.lock().unwrap().connect(owner(2), bootstrap(), |_| {
            panic!("shared peer must reuse its original process")
        }).unwrap();
        assert_eq!(first.agent, second.agent);
        let mut page = registry.lock().unwrap()
            .detach_page_retirement(&mut hub, PageId(1)).unwrap();
        let (ready, waiting) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let cleanup = scope.spawn(|| {
                let guard = registry.lock().unwrap();
                ready.send(()).unwrap();
                let mut guard = AgentRegistry::await_retirement(
                    guard, &done, std::time::Duration::from_secs(5),
                ).unwrap();
                assert!(guard.agents.contains_key(&second.agent));
                assert_eq!(state.lock().unwrap().stops, 0);
                let mut context = guard.detach_retirement(&mut hub).unwrap();
                drop(guard);
                context.terminate().unwrap();
                registry.lock().unwrap().retain_failed_retirement(context);
                done.notify_all();
            });
            waiting.recv().unwrap();
            page.terminate().unwrap();
            registry.lock().unwrap().retain_failed_retirement(page);
            done.notify_all();
            cleanup.join().unwrap();
        });
        assert_eq!(state.lock().unwrap().stops, 1);
        let guard = registry.lock().unwrap();
        assert!(guard.agents.is_empty());
        assert!(!guard.retirement_inflight);
        assert!(guard.retirement_owner.is_none());
    }

    #[test]
    fn retirement_handoff_rejects_self_wait_and_preserves_owner() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let batch = registry
            .lock()
            .unwrap()
            .detach_retirement(&mut hub)
            .unwrap();
        let failure = AgentRegistry::await_retirement(
            registry.lock().unwrap(),
            &done,
            std::time::Duration::from_secs(60),
        )
        .err()
        .unwrap();
        assert!(failure.message.contains("reentrant"));
        assert!(registry.lock().unwrap().retirement_inflight);
        registry.lock().unwrap().retain_failed_retirement(batch);
        assert!(
            AgentRegistry::await_retirement(
                registry.lock().unwrap(),
                &done,
                std::time::Duration::ZERO,
            )
            .is_ok()
        );
    }

    #[test]
    fn retirement_handoff_deadline_does_not_release_detached_custody() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let batch = registry
            .lock()
            .unwrap()
            .detach_retirement(&mut hub)
            .unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let failure = AgentRegistry::await_retirement(
                        registry.lock().unwrap(),
                        &done,
                        std::time::Duration::from_millis(1),
                    )
                    .err()
                    .unwrap();
                    assert!(failure.message.contains("deadline"));
                    assert!(registry.lock().unwrap().retirement_inflight);
                })
                .join()
                .unwrap();
        });
        registry.lock().unwrap().retain_failed_retirement(batch);
        assert!(registry.lock().unwrap().retirement_owner.is_none());
    }

    #[test]
    fn retirement_handoff_releases_registry_and_observes_settlement() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let batch = registry
            .lock()
            .unwrap()
            .detach_retirement(&mut hub)
            .unwrap();
        let (ready, waiting) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let guard = registry.lock().unwrap();
                ready.send(()).unwrap();
                let guard = AgentRegistry::await_retirement(
                    guard,
                    &done,
                    std::time::Duration::from_secs(5),
                )
                .unwrap();
                assert!(!guard.retirement_inflight);
                assert!(guard.retirement_owner.is_none());
            });
            waiting.recv().unwrap();
            // Acquiring this lock proves the waiter releases it. No sleeps or
            // scheduling guesses are needed to order retention/notification.
            registry.lock().unwrap().retain_failed_retirement(batch);
            done.notify_all();
            waiter.join().unwrap();
        });
    }

    #[test]
    fn retirement_handoff_poison_fails_closed() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let batch = registry
            .lock()
            .unwrap()
            .detach_retirement(&mut hub)
            .unwrap();
        let (ready, waiting) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let guard = registry.lock().unwrap();
                ready.send(()).unwrap();
                let failure = AgentRegistry::await_retirement(
                    guard,
                    &done,
                    std::time::Duration::from_secs(5),
                )
                .err()
                .unwrap();
                assert!(failure.message.contains("poisoned"));
            });
            waiting.recv().unwrap();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = registry.lock().unwrap();
                panic!("poison while owner settles");
            }));
            done.notify_all();
            waiter.join().unwrap();
        });
        let mut guard = registry.lock().unwrap_or_else(|error| error.into_inner());
        assert!(guard.retirement_inflight);
        guard.retain_failed_retirement(batch);
        assert!(registry.is_poisoned());
    }

    #[test]
    fn retirement_handoff_retains_failed_process_before_waiter_retries() {
        let registry = Mutex::new(AgentRegistry::new(8));
        let done = std::sync::Condvar::new();
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let state = Arc::new(Mutex::new(State {
            reject_stop: true,
            ..State::default()
        }));
        let process = state.clone();
        let connection = registry
            .lock()
            .unwrap()
            .connect(owner(1), bootstrap(), move |_| {
                Ok(Box::new(Process(process)))
            })
            .unwrap();
        let mut batch = registry
            .lock()
            .unwrap()
            .detach_retirement(&mut hub)
            .unwrap();
        assert!(batch.terminate().is_err());
        let (ready, waiting) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let guard = registry.lock().unwrap();
                ready.send(()).unwrap();
                let mut guard = AgentRegistry::await_retirement(
                    guard,
                    &done,
                    std::time::Duration::from_secs(5),
                )
                .unwrap();
                assert!(guard.agents.contains_key(&connection.agent));
                assert!(guard.agents[&connection.agent].failure.is_some());
                assert!(guard.principal(connection.agent).is_err());
                state.lock().unwrap().reject_stop = false;
                let mut retry = guard.detach_retirement(&mut hub).unwrap();
                drop(guard);
                retry.terminate().unwrap();
                registry.lock().unwrap().retain_failed_retirement(retry);
                done.notify_all();
            });
            waiting.recv().unwrap();
            registry.lock().unwrap().retain_failed_retirement(batch);
            done.notify_all();
            waiter.join().unwrap();
        });
        let guard = registry.lock().unwrap();
        assert!(guard.agents.is_empty());
        assert!(!guard.retirement_inflight);
    }
    #[derive(Default)]
    struct State {
        starts: usize,
        stops: usize,
        polls: usize,
        commands: usize,
        admission_commands: Vec<usize>,
        events: Vec<AgentEvent>,
        reject_send: bool,
        reject_stop: bool,
        last_message: Option<oryn_js::message_wire::WireValue>,
        connections: Vec<(AgentClientId, oryn_js::message_wire::PortToken)>,
    }
    struct Process(Arc<Mutex<State>>);
    impl AgentProcess for Process {
        fn start(&mut self) -> Result<(), WorkerError> {
            let mut state = self.0.lock().unwrap();
            let commands = state.commands;
            state.admission_commands.push(commands);
            Ok(())
        }
        fn send(&mut self, command: AgentCommand) -> Result<(), WorkerError> {
            let mut state = self.0.lock().unwrap();
            state.commands += 1;
            match command {
                AgentCommand::Message { value, .. }
                | AgentCommand::ServiceMessage { value, .. } => state.last_message = Some(value),
                AgentCommand::Connect { client, port } => state.connections.push((client, port)),
                _ => {}
            }
            if state.reject_send {
                Err(error("process queue rejected message"))
            } else {
                Ok(())
            }
        }
        fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
            let mut state = self.0.lock().unwrap();
            state.polls += 1;
            Ok(std::mem::take(&mut state.events))
        }
        fn terminate(&mut self) -> Result<(), WorkerError> {
            let mut state = self.0.lock().unwrap();
            state.stops += 1;
            if state.reject_stop {
                Err(error("termination rejected"))
            } else {
                Ok(())
            }
        }
    }
    fn owner(page: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(page),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        }
    }
    fn bootstrap() -> AgentBootstrap {
        AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: oryn_js::worker_wire::AgentPolicy::default(),
            top_level_url: None,
            id: AgentId(0),
            kind: AgentKind::Shared,
            partition: "context-1".into(),
            origin: AgentOrigin {
                serialized: "https://example.test".into(),
                authority: "origin-1".into(),
                opaque: false,
            },
            script_url: "https://example.test/worker.js".into(),
            source: Vec::new(),
            modules: BTreeMap::new(),
            options: WorkerOptions {
                name: "shared".encode_utf16().collect(),
                ..Default::default()
            },
            referrer_policy: "strict-origin-when-cross-origin".into(),
            limits: AgentLimits::default(),
        }
    }
    fn connect(
        registry: &mut AgentRegistry,
        owner: AgentOwner,
        bootstrap: AgentBootstrap,
        state: &Arc<Mutex<State>>,
    ) -> Result<AgentConnection, WorkerError> {
        let state = state.clone();
        registry.connect(owner, bootstrap, move |_| {
            state.lock().unwrap().starts += 1;
            Ok(Box::new(Process(state)))
        })
    }
    fn finish_pending(registry: &mut AgentRegistry, hub: &mut oryn_js::message_wire::MessageHub) {
        let mut batch = registry.detach_pending_retirement(hub).unwrap();
        batch.terminate().unwrap();
        registry.retain_failed_retirement(batch);
    }
    #[test]
    fn service_retirement_receipt_survives_failed_and_unrelated_cleanup_until_owner_acknowledges() {
        let state = Arc::new(Mutex::new(State {
            reject_stop: true,
            ..Default::default()
        }));
        let process = state.clone();
        let mut registry = AgentRegistry::new(4);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut service = bootstrap();
        service.kind = AgentKind::Service;
        let id = registry
            .launch_service(service, move |_| Ok(Box::new(Process(process))))
            .unwrap();
        assert_eq!(
            registry.service_retirement_status(id).unwrap(),
            ServiceRetirementStatus::Live
        );
        assert!(registry.acknowledge_service_retirement(id).is_err());
        registry.retire_service(&mut hub, id).unwrap();
        let mut failed = registry.detach_pending_retirement(&mut hub).unwrap();
        assert_eq!(
            registry.service_retirement_status(id).unwrap(),
            ServiceRetirementStatus::Pending
        );
        assert!(registry.acknowledge_service_retirement(id).is_err());
        assert!(registry.retire_service(&mut hub, id).is_ok());
        assert!(failed.terminate().is_err());
        registry.retain_failed_retirement(failed);
        assert_eq!(
            registry.service_retirement_status(id).unwrap(),
            ServiceRetirementStatus::Pending
        );
        state.lock().unwrap().reject_stop = false;
        // An unrelated broker dispatch can complete retained teardown before the
        // registration retries; its exact receipt must remain available.
        finish_pending(&mut registry, &mut hub);
        assert_eq!(
            registry.service_retirement_status(id).unwrap(),
            ServiceRetirementStatus::Retired
        );
        let stops = state.lock().unwrap().stops;
        registry.retire_service(&mut hub, id).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, stops);
        assert!(
            registry
                .service_retirement_status(AgentId(u64::MAX))
                .is_err()
        );
        registry.acknowledge_service_retirement(id).unwrap();
        assert!(registry.service_retirement_status(id).is_err());
        assert!(registry.retire_service(&mut hub, id).is_err());
    }
    #[test]
    fn shared_suspension_follows_all_window_owners_and_native_descendants() {
        struct Gate(Arc<std::sync::atomic::AtomicBool>);
        impl AgentProcess for Gate {
            fn set_suspended(&mut self, value: bool) -> Result<(), WorkerError> {
                self.0.store(value, std::sync::atomic::Ordering::Release);
                Ok(())
            }
            fn suspension_status(&mut self) -> Result<bool, WorkerError> {
                Ok(self.0.load(std::sync::atomic::Ordering::Acquire))
            }
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
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first_gate = gate.clone();
        let first = registry
            .connect(owner(1), bootstrap(), move |_| {
                Ok(Box::new(Gate(first_gate)))
            })
            .unwrap();
        let second = registry
            .connect(owner(2), bootstrap(), move |_| {
                panic!("shared agent must reuse process")
            })
            .unwrap();
        let child_gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let child_copy = child_gate.clone();
        let mut child = bootstrap();
        child.kind = AgentKind::Dedicated;
        registry
            .connect(AgentOwner::Agent(first.agent), child, move |_| {
                Ok(Box::new(Gate(child_copy)))
            })
            .unwrap();
        registry.freeze_document_owners(&[owner(1)]).unwrap();
        assert!(!gate.load(std::sync::atomic::Ordering::Acquire));
        assert!(!child_gate.load(std::sync::atomic::Ordering::Acquire));
        registry.freeze_document_owners(&[owner(2)]).unwrap();
        assert!(registry.frozen_agents_quiescent().unwrap());
        assert!(gate.load(std::sync::atomic::Ordering::Acquire));
        assert!(child_gate.load(std::sync::atomic::Ordering::Acquire));
        registry.resume_document_owners(&[owner(1)]).unwrap();
        assert!(!child_gate.load(std::sync::atomic::Ordering::Acquire));
        registry.release(&mut hub, &owner(1), &first).unwrap();
        assert!(gate.load(std::sync::atomic::Ordering::Acquire));
        registry.resume_document_owners(&[owner(2)]).unwrap();
        assert!(!gate.load(std::sync::atomic::Ordering::Acquire));
        registry.release(&mut hub, &owner(2), &second).unwrap();
        finish_pending(&mut registry, &mut hub);
    }

    #[test]
    fn service_message_rotates_port_authority_and_disposes_rejected_delivery() {
        use oryn_js::message_wire::*;
        use oryn_js::service_worker_wire::*;
        for reject in [false, true] {
            let mut registry = AgentRegistry::new(8);
            let mut hub = MessageHub::default();
            let state = Arc::new(Mutex::new(State {
                reject_send: reject,
                ..Default::default()
            }));
            let process = state.clone();
            let mut service = bootstrap();
            service.kind = AgentKind::Service;
            let id = registry
                .launch_service(service, move |_| Ok(Box::new(Process(process))))
                .unwrap();
            let live = BTreeMap::from([(7, "https://example.test".into())]);
            let MessageResponse::Channel(ports) =
                hub.handle(7, MessageRequest::CreateChannel, &live).unwrap()
            else {
                panic!("channel")
            };
            let MessageResponse::Transferred(ports) = hub
                .handle(
                    7,
                    MessageRequest::Transfer {
                        ports: vec![ports[0].clone()],
                    },
                    &live,
                )
                .unwrap()
            else {
                panic!("transfer")
            };
            let previous = ports[0].clone();
            let source = MessageSource::Client {
                client: ClientSnapshot {
                    id: "native-client".into(),
                    url: "about:blank".into(),
                    client_type: ClientType::Window,
                    frame_type: "top-level".into(),
                    focused: true,
                    visibility_state: "visible".into(),
                },
                origin: "https://example.test".into(),
            };
            let value = WireValue {
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
                version: 1,
                bytes: vec![1],
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports,
            };
            let result =
                registry.post_service_message(&mut hub, &owner(7), id, 11, source.clone(), value);
            assert_eq!(result.is_err(), reject);
            let delivered = state.lock().unwrap().last_message.clone().unwrap();
            assert_ne!(delivered.ports[0], previous);
            assert!(
                hub.handle(
                    7,
                    MessageRequest::PollPort {
                        port: delivered.ports[0].clone(),
                        max: 1
                    },
                    &live
                )
                .is_err()
            );
            assert_eq!(
                hub.handle_agent(
                    id.0,
                    MessageRequest::PollPort {
                        port: delivered.ports[0].clone(),
                        max: 1
                    },
                    "https://example.test",
                    "origin-1"
                )
                .is_err(),
                reject
            );
            assert!(
                registry
                    .send_service_control(
                        id,
                        AgentCommand::ServiceMessage {
                            id: 12,
                            source,
                            value: WireValue {
                                shared: Vec::new(),
                                shared_queue: None,
                                shared_recipient: None,
                                wasm_modules: Vec::new(),
                                version: 1,
                                bytes: vec![1],
                                hosts: Vec::new(),
                                buffers: Vec::new(),
                                ports: Vec::new()
                            }
                        }
                    )
                    .is_err(),
                "control shortcut cannot bypass transfer routing"
            );
            registry.retire_service(&mut hub, id).unwrap();
            finish_pending(&mut registry, &mut hub);
        }
    }

    #[test]
    fn profile_service_delivery_requires_remapped_capabilities_and_disposes_rejected_send() {
        use oryn_js::{message_wire::*, service_worker_wire::*};
        for reject in [false, true] {
            let mut registry = AgentRegistry::new(8);
            let mut source_hub = MessageHub::default();
            let mut destination = MessageHub::default();
            let state = Arc::new(Mutex::new(State {
                reject_send: reject,
                ..Default::default()
            }));
            let process = state.clone();
            let mut service = bootstrap();
            service.kind = AgentKind::Service;
            let target = registry
                .launch_service(service, move |_| Ok(Box::new(Process(process))))
                .unwrap();
            let [_, port] = source_hub
                .create_entangled(Principal::Agent(99), Principal::Agent(99))
                .unwrap();
            let MessageResponse::Transferred(ports) = source_hub
                .handle_agent(
                    99,
                    MessageRequest::Transfer { ports: vec![port] },
                    "https://example.test",
                    "origin-1",
                )
                .unwrap()
            else {
                panic!("transfer")
            };
            let mut value = WireValue {
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
                version: 1,
                bytes: vec![1],
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports,
            };
            let source = MessageSource::Client {
                client: ClientSnapshot {
                    id: "authenticated-profile-client".into(),
                    url: "https://example.test/source".into(),
                    client_type: ClientType::Window,
                    frame_type: "top-level".into(),
                    focused: true,
                    visibility_state: "visible".into(),
                },
                origin: "https://example.test".into(),
            };
            assert!(
                registry
                    .post_profile_service_message(
                        &mut destination,
                        target,
                        1,
                        source.clone(),
                        value.clone()
                    )
                    .is_err(),
                "source-hub tokens are never local receiving authority"
            );
            let bridges = route_cross_hub_value(
                &mut source_hub,
                &mut destination,
                Principal::Agent(99),
                Principal::Agent(target.0),
                &mut value,
            )
            .unwrap();
            let received = value.clone();
            let result =
                registry.post_profile_service_message(&mut destination, target, 2, source, value);
            assert_eq!(result.is_err(), reject);
            assert_eq!(
                destination
                    .validate_received_value(Principal::Agent(target.0), &received)
                    .is_err(),
                reject
            );
            for bridge in bridges {
                bridge.retire(&mut source_hub, &mut destination).unwrap();
            }
            registry.retire_service(&mut destination, target).unwrap();
            finish_pending(&mut registry, &mut destination);
        }
    }

    #[test]
    fn service_message_rejects_foreign_native_agent_authority_before_enqueue() {
        use oryn_js::service_worker_wire::*;
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let state = Arc::new(Mutex::new(State::default()));
        let process = state.clone();
        let mut service = bootstrap();
        service.kind = AgentKind::Service;
        let recipient = registry
            .launch_service(service.clone(), move |_| Ok(Box::new(Process(process))))
            .unwrap();
        service.partition = "other-context".into();
        let process = state.clone();
        let sender = registry
            .launch_service(service, move |_| Ok(Box::new(Process(process))))
            .unwrap();
        let source = MessageSource::ServiceWorker {
            worker: WorkerSnapshot {
                version_id: ServiceVersionId([1; 16]),
                id: sender,
                script_url: bootstrap().script_url,
                state: WorkerState::Activated,
            },
            origin: "https://example.test".into(),
        };
        let value = oryn_js::message_wire::WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: Vec::new(),
        };
        assert_eq!(
            registry
                .post_service_message(
                    &mut hub,
                    &AgentOwner::Agent(sender),
                    recipient,
                    1,
                    source,
                    value
                )
                .unwrap_err()
                .name,
            "SecurityError"
        );
        assert_eq!(state.lock().unwrap().commands, 0);
    }
    #[test]
    fn service_admission_has_no_window_client_and_control_is_kind_checked() {
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let state = Arc::new(Mutex::new(State::default()));
        let process = state.clone();
        let mut service = bootstrap();
        service.kind = AgentKind::Service;
        let id = registry
            .launch_service(service, move |bootstrap| {
                assert_ne!(bootstrap.id, AgentId(0));
                Ok(Box::new(Process(process)))
            })
            .unwrap();
        assert!(registry.agents[&id].clients.is_empty());
        assert_eq!(registry.next_client, 0);
        assert_eq!(state.lock().unwrap().admission_commands, vec![0]);
        registry
            .send_service_control(id, AgentCommand::CancelTask { id: 9 })
            .unwrap();
        assert!(
            registry
                .send_service_control(id, AgentCommand::Stop)
                .is_err()
        );
        assert_eq!(state.lock().unwrap().commands, 1);
        let other = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        assert!(
            registry
                .send_service_control(other.agent, AgentCommand::CancelTask { id: 9 })
                .is_err()
        );
        assert!(registry.retire_service(&mut hub, other.agent).is_err());
        registry.retire_service(&mut hub, id).unwrap();
        assert!(registry.principal(id).is_err());
        assert!(
            registry
                .send_service_control(id, AgentCommand::CancelTask { id: 9 })
                .is_err()
        );
        assert_eq!(state.lock().unwrap().stops, 0);
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
        assert!(registry.principal(other.agent).is_ok());
    }

    #[test]
    fn service_start_failure_retains_process_and_capacity_until_detached_join() {
        struct RejectStart(Arc<Mutex<State>>);
        impl AgentProcess for RejectStart {
            fn start(&mut self) -> Result<(), WorkerError> {
                Err(error("service startup rejected"))
            }
            fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
                panic!("failed agent cannot receive commands")
            }
            fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
                panic!("failed agent cannot be polled")
            }
            fn terminate(&mut self) -> Result<(), WorkerError> {
                self.0.lock().unwrap().stops += 1;
                Ok(())
            }
        }
        let mut registry = AgentRegistry::new(1);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let state = Arc::new(Mutex::new(State::default()));
        let process = state.clone();
        let mut service = bootstrap();
        service.kind = AgentKind::Service;
        assert!(
            registry
                .launch_service(bootstrap(), |_| panic!("wrong kind must not launch"))
                .is_err()
        );
        assert!(
            registry
                .launch_service(service.clone(), move |_| Ok(Box::new(RejectStart(process))))
                .is_err()
        );
        assert!(registry.agents.is_empty());
        assert_eq!(registry.pending_termination.len(), 1);
        assert!(
            registry
                .launch_service(service.clone(), |_| panic!("capacity still reserved"))
                .is_err()
        );
        assert_eq!(state.lock().unwrap().stops, 0);
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
        let process = state.clone();
        assert!(
            registry
                .launch_service(service, move |_| Ok(Box::new(Process(process))))
                .is_ok()
        );
    }

    #[test]
    fn client_release_retry_is_absent_only_and_never_targets_a_successor() {
        let process = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &process).unwrap();
        let mut forged = connection.clone();
        forged.client.secret[0] ^= 1;
        assert!(registry.release_owner(&forged).is_err());
        registry.agents.get_mut(&connection.agent).unwrap().failure = Some(error("process failed"));
        assert_eq!(
            registry.release_owner(&connection).unwrap(),
            Some(owner(1)),
            "failed process must still permit exact client cleanup"
        );
        registry.release(&mut hub, &owner(1), &connection).unwrap();
        assert_eq!(registry.release_owner(&connection).unwrap(), None);
        let replacement = connect(&mut registry, owner(1), bootstrap(), &process).unwrap();
        assert_ne!(replacement.agent, connection.agent);
        assert_eq!(registry.release_owner(&connection).unwrap(), None);
        assert_eq!(
            registry.release_owner(&replacement).unwrap(),
            Some(owner(1))
        );
        finish_pending(&mut registry, &mut hub);
    }

    #[test]
    fn natural_close_revokes_execution_but_preserves_queued_client_delivery() {
        use oryn_js::message_wire::WireValue;
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let value = WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![4, 2],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: Vec::new(),
        };
        state.lock().unwrap().events = vec![
            AgentEvent::Message {
                client: Some(connection.client.clone()),
                value: value.clone(),
            },
            AgentEvent::Closed,
        ];
        assert!(
            matches!(registry.poll(&mut hub,&owner(1),&connection,1).unwrap().as_slice(),
            [AgentEvent::Message{value,..}] if value.bytes==[4,2])
        );
        assert!(registry.principal(connection.agent).is_err());
        assert!(registry.bootstrap_snapshot(connection.agent).is_err());
        assert!(
            registry
                .post(&mut hub, &owner(1), &connection, value)
                .is_err()
        );
        assert_eq!(registry.connection_owner(&connection).unwrap(), owner(1));
        assert!(matches!(
            registry
                .poll(&mut hub, &owner(1), &connection, 1)
                .unwrap()
                .as_slice(),
            [AgentEvent::Closed]
        ));
        assert!(
            registry
                .poll(&mut hub, &owner(1), &connection, 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            state.lock().unwrap().polls,
            1,
            "closed processes are never polled again"
        );
        registry.release(&mut hub, &owner(1), &connection).unwrap();
        assert!(registry.connection_owner(&connection).is_err());
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }

    #[test]
    fn closed_shared_worker_cleanup_preserves_replacement_identity() {
        let old = Arc::new(Mutex::new(State::default()));
        let new = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &old).unwrap();
        old.lock().unwrap().events.push(AgentEvent::Closed);
        registry.poll(&mut hub, &owner(1), &first, 0).unwrap();
        let replacement = connect(&mut registry, owner(2), bootstrap(), &new).unwrap();
        assert_ne!(first.agent, replacement.agent);
        registry.release(&mut hub, &owner(1), &first).unwrap();
        finish_pending(&mut registry, &mut hub);
        let sibling = connect(&mut registry, owner(3), bootstrap(), &new).unwrap();
        assert_eq!(sibling.agent, replacement.agent);
        assert_eq!(new.lock().unwrap().starts, 1);
        assert_eq!(old.lock().unwrap().stops, 1);
    }

    #[test]
    fn event_after_closed_rejects_whole_batch_before_delivery() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        state.lock().unwrap().events = vec![AgentEvent::Closed, AgentEvent::Closed];
        assert!(
            registry
                .poll(&mut hub, &owner(1), &connection, 32)
                .unwrap_err()
                .message
                .contains("after Closed")
        );
        assert!(
            registry.agents[&connection.agent].clients[&connection.client.id]
                .events
                .is_empty()
        );
        assert!(registry.connection_owner(&connection).is_err());
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }

    #[test]
    fn control_poll_close_retires_descendants_and_drains_unowned_service_results() {
        let state = Arc::new(Mutex::new(State::default()));
        let child_state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut service = bootstrap();
        service.kind = AgentKind::Service;
        let parent = connect(&mut registry, owner(1), service, &state).unwrap();
        let mut dedicated = bootstrap();
        dedicated.kind = AgentKind::Dedicated;
        let child = connect(
            &mut registry,
            AgentOwner::Agent(parent.agent),
            dedicated,
            &child_state,
        )
        .unwrap();
        registry.release(&mut hub, &owner(1), &parent).unwrap();
        state.lock().unwrap().events = vec![
            AgentEvent::LifecycleFinished {
                id: 17,
                result: Ok(()),
            },
            AgentEvent::Closed,
        ];
        assert!(
            registry
                .poll_control(&mut hub, parent.agent, 0)
                .unwrap()
                .is_empty()
        );
        assert!(registry.principal(child.agent).is_err());
        finish_pending(&mut registry, &mut hub);
        assert_eq!(child_state.lock().unwrap().stops, 1);
        assert_eq!(
            state.lock().unwrap().stops,
            0,
            "pending control result must remain drainable"
        );
        assert!(matches!(
            registry
                .poll_control(&mut hub, parent.agent, 1)
                .unwrap()
                .as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 17,
                result: Ok(())
            }]
        ));
        assert!(matches!(
            registry
                .poll_control(&mut hub, parent.agent, 1)
                .unwrap()
                .as_slice(),
            [AgentEvent::Closed]
        ));
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
        assert_eq!(state.lock().unwrap().polls, 1);
    }

    #[test]
    fn service_ready_error_and_closed_reach_control_without_a_window_client() {
        let process = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut boot = bootstrap();
        boot.kind = AgentKind::Service;
        let process_copy = process.clone();
        let id = registry
            .launch_service(boot, move |_| Ok(Box::new(Process(process_copy))))
            .unwrap();
        process.lock().unwrap().events = vec![
            AgentEvent::Ready,
            AgentEvent::Error {
                message: "startup failure".encode_utf16().collect(),
                filename: "service.js".into(),
                line: 1,
                column: 1,
            },
            AgentEvent::Closed,
        ];
        assert!(matches!(
            registry.poll_control(&mut hub, id, 1).unwrap().as_slice(),
            [AgentEvent::Ready]
        ));
        assert!(matches!(
            registry.poll_control(&mut hub, id, 1).unwrap().as_slice(),
            [AgentEvent::Error { line: 1, .. }]
        ));
        finish_pending(&mut registry, &mut hub);
        assert_eq!(
            process.lock().unwrap().stops,
            0,
            "terminal control receipt remains owned"
        );
        assert!(matches!(
            registry.poll_control(&mut hub, id, 1).unwrap().as_slice(),
            [AgentEvent::Closed]
        ));
        finish_pending(&mut registry, &mut hub);
        assert_eq!(process.lock().unwrap().stops, 1);
    }

    #[test]
    fn service_control_receipts_drain_before_later_process_failure() {
        struct ReportsThenFailure(bool);
        impl AgentProcess for ReportsThenFailure {
            fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
                Ok(())
            }
            fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
                if std::mem::replace(&mut self.0, true) {
                    return Err(WorkerError::new("TypeError", "original startup failure"));
                }
                Ok([1, 2]
                    .into_iter()
                    .map(|line| AgentEvent::Error {
                        message: "source failure".encode_utf16().collect(),
                        filename: "worker.js".into(),
                        line,
                        column: 1,
                    })
                    .collect())
            }
            fn terminate(&mut self) -> Result<(), WorkerError> {
                Ok(())
            }
        }
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut boot = bootstrap();
        boot.kind = AgentKind::Service;
        let id = registry
            .launch_service(boot, |_| Ok(Box::new(ReportsThenFailure(false))))
            .unwrap();
        assert!(registry.poll_control(&mut hub, id, 0).unwrap().is_empty());
        for expected in [1, 2] {
            assert!(
                matches!(registry.poll_control(&mut hub,id,1).unwrap().as_slice(),[AgentEvent::Error{line,..}] if *line==expected)
            );
        }
        assert_eq!(registry.agents[&id].control_bytes, 0);
        assert_eq!(
            registry.poll_control(&mut hub, id, 1).unwrap_err().message,
            "original startup failure"
        );
        finish_pending(&mut registry, &mut hub);
    }

    #[test]
    fn empty_pending_batch_does_not_take_or_release_another_retirement_guard() {
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let batch = registry.detach_pending_retirement(&mut hub).unwrap();
        assert!(!registry.retirement_inflight);
        let mut retirement = registry.detach_retirement(&mut hub).unwrap();
        registry.retain_failed_retirement(batch);
        assert!(
            registry.retirement_inflight,
            "empty batch cannot clear another teardown's guard"
        );
        retirement.terminate().unwrap();
        registry.retain_failed_retirement(retirement);
        assert!(!registry.retirement_inflight);
    }

    #[test]
    fn shared_admission_starts_after_connect_and_last_release_defers_join() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(1);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let process = state.clone();
        let connection = registry
            .connect_with_ports(&mut hub, owner(1), bootstrap(), move |_| {
                Ok(Box::new(Process(process)))
            })
            .unwrap();
        assert_eq!(state.lock().unwrap().admission_commands, vec![1]);
        registry.release(&mut hub, &owner(1), &connection).unwrap();
        assert_eq!(state.lock().unwrap().stops, 0);
        assert!(registry.principal(connection.agent).is_err());
        assert!(
            connect(&mut registry, owner(2), bootstrap(), &state).is_err(),
            "pending owned process still consumes capacity"
        );
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
        assert!(connect(&mut registry, owner(2), bootstrap(), &state).is_ok());
    }

    #[test]
    fn detached_page_retirement_keeps_shared_sibling_and_unrelated_admission() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let sibling = connect(&mut registry, owner(2), bootstrap(), &state).unwrap();
        let dedicated_state = Arc::new(Mutex::new(State::default()));
        let mut dedicated = bootstrap();
        dedicated.kind = AgentKind::Dedicated;
        let parent = connect(&mut registry, owner(1), dedicated.clone(), &dedicated_state).unwrap();
        let child = connect(
            &mut registry,
            AgentOwner::Agent(parent.agent),
            dedicated,
            &dedicated_state,
        )
        .unwrap();
        let mut batch = registry
            .detach_page_retirement(&mut hub, PageId(1))
            .unwrap();
        assert_eq!(
            dedicated_state.lock().unwrap().stops,
            0,
            "detachment cannot join under locks"
        );
        assert!(registry.poll(&mut hub, &owner(1), &first, 1).is_err());
        assert!(registry.poll(&mut hub, &owner(2), &sibling, 1).is_ok());
        assert!(registry.principal(parent.agent).is_err());
        assert!(registry.principal(child.agent).is_err());
        batch.terminate().unwrap();
        registry.retain_failed_retirement(batch);
        assert_eq!(dedicated_state.lock().unwrap().stops, 2);
        assert_eq!(state.lock().unwrap().stops, 0);
        assert!(!registry.retired);
        assert!(connect(&mut registry, owner(3), bootstrap(), &state).is_ok());
    }

    #[test]
    fn detached_document_retirement_does_not_select_replacement_generation() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let newer = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Root,
            generation: 2,
            epoch: 0,
        };
        let second = connect(&mut registry, newer.clone(), bootstrap(), &state).unwrap();
        let mut batch = registry
            .detach_document_retirement(&mut hub, PageId(1), WindowTarget::Root, 1, 0)
            .unwrap();
        batch.terminate().unwrap();
        registry.retain_failed_retirement(batch);
        assert!(registry.poll(&mut hub, &owner(1), &first, 1).is_err());
        assert!(registry.poll(&mut hub, &newer, &second, 1).is_ok());
        assert_eq!(state.lock().unwrap().stops, 0);
    }

    #[test]
    fn detached_retirement_allows_parent_broker_lock_access_during_termination() {
        struct LockAwareProcess {
            registry: std::sync::Weak<Mutex<AgentRegistry>>,
            hub: Arc<Mutex<oryn_js::message_wire::MessageHub>>,
        }
        impl AgentProcess for LockAwareProcess {
            fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
                Ok(())
            }
            fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
                Ok(Vec::new())
            }
            fn terminate(&mut self) -> Result<(), WorkerError> {
                let registry = self.registry.upgrade().unwrap();
                let registry = registry
                    .try_lock()
                    .map_err(|_| error("registry held across termination"))?;
                assert!(registry.retired);
                let _hub = self
                    .hub
                    .try_lock()
                    .map_err(|_| error("hub held across termination"))?;
                Ok(())
            }
        }
        let registry = Arc::new(Mutex::new(AgentRegistry::new(8)));
        let hub = Arc::new(Mutex::new(oryn_js::message_wire::MessageHub::default()));
        let process = LockAwareProcess {
            registry: Arc::downgrade(&registry),
            hub: hub.clone(),
        };
        registry
            .lock()
            .unwrap()
            .connect(owner(1), bootstrap(), move |_| Ok(Box::new(process)))
            .unwrap();
        let mut batch = {
            let mut registry = registry.lock().unwrap();
            let mut hub = hub.lock().unwrap();
            registry.detach_retirement(&mut hub).unwrap()
        };
        batch.terminate().unwrap();
        registry.lock().unwrap().retain_failed_retirement(batch);
        assert!(registry.lock().unwrap().agents.is_empty());
    }

    #[test]
    fn termination_panic_preserves_ownership_and_does_not_skip_other_agents() {
        struct PanicOnce {
            attempts: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl AgentProcess for PanicOnce {
            fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
                Ok(())
            }
            fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
                Ok(Vec::new())
            }
            fn terminate(&mut self) -> Result<(), WorkerError> {
                if self
                    .attempts
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    == 0
                {
                    panic!("injected agent termination panic");
                }
                Ok(())
            }
        }
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probe = attempts.clone();
        let mut dedicated = bootstrap();
        dedicated.kind = AgentKind::Dedicated;
        let failed = registry
            .connect(owner(1), dedicated.clone(), move |_| {
                Ok(Box::new(PanicOnce { attempts: probe }))
            })
            .unwrap();
        let peer = Arc::new(Mutex::new(State::default()));
        let completed = connect(&mut registry, owner(1), dedicated, &peer).unwrap();
        let mut batch = registry
            .detach_page_retirement(&mut hub, PageId(1))
            .unwrap();
        assert!(batch.terminate().unwrap_err().message.contains("panicked"));
        registry.retain_failed_retirement(batch);
        assert!(!registry.retirement_inflight);
        assert_eq!(
            peer.lock().unwrap().stops,
            1,
            "peer termination still runs after panic"
        );
        assert!(!registry.agents.contains_key(&completed.agent));
        assert!(registry.agents.contains_key(&failed.agent));
        assert!(
            registry.principal(failed.agent).is_err(),
            "failed process stays quarantined"
        );
        let mut retry = registry.detach_pending_retirement(&mut hub).unwrap();
        retry.terminate().unwrap();
        registry.retain_failed_retirement(retry);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(registry.agents.is_empty());
        assert!(registry.pending_termination.is_empty());
        assert!(!registry.retirement_inflight);
    }

    #[test]
    fn detached_failed_termination_retains_process_without_restoring_admission() {
        let state = Arc::new(Mutex::new(State {
            reject_stop: true,
            ..Default::default()
        }));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let mut batch = registry.detach_retirement(&mut hub).unwrap();
        assert!(
            registry.detach_retirement(&mut hub).is_err(),
            "concurrent cleanup cannot report an in-flight process as retired"
        );
        assert!(registry.principal(connection.agent).is_err());
        assert!(batch.terminate().is_err());
        registry.retain_failed_retirement(batch);
        assert!(registry.agents.contains_key(&connection.agent));
        assert!(registry.principal(connection.agent).is_err());
        assert!(connect(&mut registry, owner(2), bootstrap(), &state).is_err());
        state.lock().unwrap().reject_stop = false;
        let mut retry = registry.detach_retirement(&mut hub).unwrap();
        retry.terminate().unwrap();
        registry.retain_failed_retirement(retry);
        assert!(registry.agents.is_empty());
        assert_eq!(state.lock().unwrap().stops, 2);
    }

    #[test]
    fn document_retirement_is_generation_exact_and_page_retirement_covers_frames() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let old = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Frame("frame".into()),
            generation: 4,
            epoch: 0,
        };
        let new = AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Frame("frame".into()),
            generation: 5,
            epoch: 1,
        };
        let first = connect(&mut registry, old.clone(), bootstrap(), &state).unwrap();
        let replacement = connect(&mut registry, new.clone(), bootstrap(), &state).unwrap();
        let root = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let sibling = connect(&mut registry, owner(2), bootstrap(), &state).unwrap();
        registry
            .retire_document(
                &mut hub,
                PageId(1),
                WindowTarget::Frame("frame".into()),
                4,
                0,
            )
            .unwrap();
        assert!(registry.poll(&mut hub, &old, &first, 1).is_err());
        assert!(registry.poll(&mut hub, &new, &replacement, 1).is_ok());
        // Repeated stale notification cannot target the replacement document.
        registry
            .retire_document(
                &mut hub,
                PageId(1),
                WindowTarget::Frame("frame".into()),
                4,
                0,
            )
            .unwrap();
        assert!(registry.poll(&mut hub, &new, &replacement, 1).is_ok());
        registry.retire_page(&mut hub, PageId(1)).unwrap();
        assert!(registry.poll(&mut hub, &new, &replacement, 1).is_err());
        assert!(registry.poll(&mut hub, &owner(1), &root, 1).is_err());
        assert!(registry.poll(&mut hub, &owner(2), &sibling, 1).is_ok());
        assert_eq!(state.lock().unwrap().stops, 0);
        registry.retire_page(&mut hub, PageId(2)).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }

    #[test]
    fn page_retirement_terminates_dedicated_agent_descendants() {
        let parent_state = Arc::new(Mutex::new(State::default()));
        let child_state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut dedicated = bootstrap();
        dedicated.kind = AgentKind::Dedicated;
        let parent = connect(&mut registry, owner(1), dedicated.clone(), &parent_state).unwrap();
        let child = connect(
            &mut registry,
            AgentOwner::Agent(parent.agent),
            dedicated,
            &child_state,
        )
        .unwrap();
        registry.retire_page(&mut hub, PageId(1)).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert!(registry.principal(parent.agent).is_err());
        assert!(registry.principal(child.agent).is_err());
        assert_eq!(parent_state.lock().unwrap().stops, 1);
        assert_eq!(child_state.lock().unwrap().stops, 1);
        registry.retire_page(&mut hub, PageId(1)).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(child_state.lock().unwrap().stops, 1);
    }

    #[test]
    fn shared_connection_ports_are_entangled_and_bound_to_distinct_principals() {
        use oryn_js::message_wire::{MessageRequest, MessageResponse, WireValue};
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let process = state.clone();
        let connection = registry
            .connect_with_ports(&mut hub, owner(1), bootstrap(), move |_| {
                Ok(Box::new(Process(process)))
            })
            .unwrap();
        let client_port = connection.port.unwrap();
        let (client, agent_port) = state.lock().unwrap().connections[0].clone();
        assert_eq!(client, connection.client);
        let live = BTreeMap::from([
            (1, "https://example.test".into()),
            (2, "https://example.test".into()),
        ]);
        assert!(
            hub.handle(
                2,
                MessageRequest::PollPort {
                    port: client_port.clone(),
                    max: 1
                },
                &live
            )
            .is_err()
        );
        assert!(
            hub.handle(
                1,
                MessageRequest::PollPort {
                    port: agent_port.clone(),
                    max: 1
                },
                &live
            )
            .is_err()
        );
        let value = WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![4, 2],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: Vec::new(),
        };
        hub.handle(
            1,
            MessageRequest::PostPort {
                port: client_port,
                payload: value,
            },
            &live,
        )
        .unwrap();
        assert!(
            matches!(hub.handle_agent(connection.agent.0,MessageRequest::PollPort{port:agent_port,max:1},"https://example.test","origin-1").unwrap(),
            MessageResponse::PortMessages{messages,..} if messages.len()==1 && messages[0].bytes==[4,2])
        );
    }

    #[test]
    fn failed_shared_connect_revokes_new_ports_without_retiring_existing_client() {
        use oryn_js::message_wire::MessageRequest;
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let process = state.clone();
        let first = registry
            .connect_with_ports(&mut hub, owner(1), bootstrap(), move |_| {
                Ok(Box::new(Process(process)))
            })
            .unwrap();
        state.lock().unwrap().reject_send = true;
        assert!(
            registry
                .connect_with_ports(&mut hub, owner(2), bootstrap(), |_| panic!(
                    "shared worker must be reused"
                ))
                .is_err()
        );
        let (_, rejected_port) = state.lock().unwrap().connections[1].clone();
        assert!(
            hub.handle_agent(
                first.agent.0,
                MessageRequest::PollPort {
                    port: rejected_port,
                    max: 1
                },
                "https://example.test",
                "origin-1"
            )
            .is_err()
        );
        assert_eq!(registry.agents[&first.agent].clients.len(), 1);
        assert_eq!(state.lock().unwrap().stops, 0);
        assert!(registry.principal(first.agent).is_ok());
    }

    #[test]
    fn client_owner_lookup_rejects_forged_secret_and_retired_principal() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(4);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let expected = bootstrap();
        let connection = connect(&mut registry, owner(11), expected.clone(), &state).unwrap();
        assert_eq!(registry.connection_owner(&connection).unwrap(), owner(11));
        let snapshot = registry.bootstrap_snapshot(connection.agent).unwrap();
        assert_eq!(snapshot.origin.authority, expected.origin.authority);
        assert_eq!(snapshot.partition, expected.partition);
        let mut forged = connection.clone();
        forged.client.secret[0] ^= 1;
        assert_eq!(
            registry.connection_owner(&forged).unwrap_err().name,
            "SecurityError"
        );
        registry.release(&mut hub, &owner(11), &connection).unwrap();
        assert!(registry.connection_owner(&connection).is_err());
        assert!(registry.bootstrap_snapshot(connection.agent).is_err());
        finish_pending(&mut registry, &mut hub);
    }
    #[test]
    fn shared_identity_validates_options_and_releases_only_last_client() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let second = connect(&mut registry, owner(2), bootstrap(), &state).unwrap();
        assert_eq!(first.agent, second.agent);
        assert_ne!(first.client, second.client);
        assert_eq!(state.lock().unwrap().starts, 1);
        for mismatch in 0..5 {
            let mut options = bootstrap();
            match mismatch {
                0 => options.script_url = "https://example.test/other.js".into(),
                1 => options.options.script_type = ScriptType::Module,
                2 => options.options.credentials = WorkerCredentials::Include,
                3 => options.secure_context = false,
                _ => options
                    .policy
                    .headers
                    .push(("cross-origin-embedder-policy".into(), "require-corp".into())),
            }
            assert!(connect(&mut registry, owner(3), options, &state).is_err());
        }
        assert_eq!(state.lock().unwrap().starts, 1);
        assert!(registry.release(&mut hub, &owner(2), &first).is_err());
        registry.release(&mut hub, &owner(1), &first).unwrap();
        assert_eq!(state.lock().unwrap().stops, 0);
        registry.release(&mut hub, &owner(2), &second).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }
    #[test]
    fn partition_and_opaque_authority_are_distinct_and_retirement_revokes_capabilities() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let mut other = bootstrap();
        other.partition = "context-2".into();
        let second = connect(&mut registry, owner(1), other, &state).unwrap();
        assert_ne!(first.agent, second.agent);
        let mut other = bootstrap();
        other.origin.authority = "opaque-unique".into();
        other.origin.serialized = "null".into();
        other.origin.opaque = true;
        let third = connect(&mut registry, owner(1), other, &state).unwrap();
        assert_ne!(first.agent, third.agent);
        let mut forged = first.clone();
        forged.client.secret[0] ^= 1;
        assert!(registry.poll(&mut hub, &owner(1), &forged, 1).is_err());
        registry.retire_owner(&mut hub, &owner(1)).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 3);
        assert!(registry.poll(&mut hub, &owner(1), &first, 1).is_err());
        registry.retire(&mut hub).unwrap();
        assert!(connect(&mut registry, owner(1), bootstrap(), &state).is_err());
    }
    #[test]
    fn shared_client_poll_does_not_steal_another_clients_event() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let first = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let second = connect(&mut registry, owner(2), bootstrap(), &state).unwrap();
        let value = oryn_js::message_wire::WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: Vec::new(),
        };
        state.lock().unwrap().events.push(AgentEvent::Message {
            client: Some(second.client.clone()),
            value,
        });
        assert!(
            registry
                .poll(&mut hub, &owner(1), &first, 32)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            registry
                .poll(&mut hub, &owner(2), &second, 32)
                .unwrap()
                .len(),
            1
        );
        assert!(
            registry
                .poll(&mut hub, &owner(2), &second, 32)
                .unwrap()
                .is_empty()
        );
        registry.retire(&mut hub).unwrap();
    }

    #[test]
    fn fetch_fallback_is_parent_control_and_preserves_lifetime_event_order() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        state.lock().unwrap().events.extend([
            AgentEvent::FetchFallback { id: 17 },
            AgentEvent::LifecycleFinished {
                id: 17,
                result: Ok(()),
            },
        ]);
        assert!(
            registry
                .poll(&mut hub, &owner(1), &connection, 32)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            registry
                .poll_control(&mut hub, connection.agent, 1)
                .unwrap()
                .as_slice(),
            [AgentEvent::FetchFallback { id: 17 }]
        ));
        assert!(matches!(
            registry
                .poll_control(&mut hub, connection.agent, 1)
                .unwrap()
                .as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 17,
                result: Ok(())
            }]
        ));
        assert!(
            registry
                .poll_control(&mut hub, connection.agent, 32)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn lifecycle_results_are_parent_only_and_queue_overflow_is_terminal() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        state
            .lock()
            .unwrap()
            .events
            .push(AgentEvent::LifecycleFinished {
                id: 42,
                result: Ok(()),
            });
        assert!(
            registry
                .poll(&mut hub, &owner(1), &connection, 32)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            registry
                .poll_control(&mut hub, connection.agent, 32)
                .unwrap()
                .as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 42,
                result: Ok(())
            }]
        ));
        registry
            .agents
            .get_mut(&connection.agent)
            .unwrap()
            .bootstrap
            .limits
            .queue_messages = 1;
        state
            .lock()
            .unwrap()
            .events
            .extend([AgentEvent::Ready, AgentEvent::Closed]);
        assert!(registry.poll(&mut hub, &owner(1), &connection, 32).is_err());
        assert_eq!(
            state.lock().unwrap().stops,
            0,
            "poll cannot join while broker locks are held"
        );
        assert!(
            registry.agents[&connection.agent].clients[&connection.client.id]
                .events
                .is_empty(),
            "overflow must not partially publish a batch"
        );
        assert!(registry.principal(connection.agent).is_err());
        assert!(registry.poll(&mut hub, &owner(1), &connection, 32).is_err());
        registry.release(&mut hub, &owner(1), &connection).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(
            state.lock().unwrap().stops,
            1,
            "successful teardown is not repeated"
        );
    }

    #[test]
    fn rejected_agent_queue_releases_remapped_transferred_ports() {
        use oryn_js::message_wire::*;
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let connection = connect(&mut registry, owner(1), bootstrap(), &state).unwrap();
        let live = BTreeMap::from([(1, "https://example.test".into())]);
        let MessageResponse::Channel(ports) =
            hub.handle(1, MessageRequest::CreateChannel, &live).unwrap()
        else {
            panic!("channel")
        };
        let MessageResponse::Transferred(ports) = hub
            .handle(
                1,
                MessageRequest::Transfer {
                    ports: vec![ports[0].clone()],
                },
                &live,
            )
            .unwrap()
        else {
            panic!("transfer")
        };
        let value = WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports,
        };
        assert!(
            registry
                .post(&mut hub, &owner(2), &connection, value.clone())
                .is_err()
        );
        state.lock().unwrap().reject_send = true;
        assert!(
            registry
                .post(&mut hub, &owner(1), &connection, value)
                .is_err()
        );
        let port = state.lock().unwrap().last_message.as_ref().unwrap().ports[0].clone();
        assert!(
            hub.handle_agent(
                connection.agent.0,
                MessageRequest::PollPort { port, max: 1 },
                "https://example.test",
                "origin-1"
            )
            .is_err()
        );
        assert_eq!(state.lock().unwrap().commands, 1);
        registry.retire(&mut hub).unwrap();
    }

    #[test]
    fn returned_worker_ports_are_owned_by_the_actual_receiving_window() {
        use oryn_js::message_wire::*;
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = MessageHub::default();
        let connection = connect(&mut registry, owner(7), bootstrap(), &state).unwrap();
        let agent = connection.agent.0;
        let MessageResponse::Channel(ports) = hub
            .handle_agent(
                agent,
                MessageRequest::CreateChannel,
                "https://example.test",
                "origin-1",
            )
            .unwrap()
        else {
            panic!("channel")
        };
        let MessageResponse::Transferred(ports) = hub
            .handle_agent(
                agent,
                MessageRequest::Transfer {
                    ports: vec![ports[0].clone()],
                },
                "https://example.test",
                "origin-1",
            )
            .unwrap()
        else {
            panic!("transfer")
        };
        state.lock().unwrap().events.push(AgentEvent::Message {
            client: Some(connection.client.clone()),
            value: WireValue {
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
                version: 1,
                bytes: vec![1],
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports,
            },
        });
        let events = registry.poll(&mut hub, &owner(7), &connection, 32).unwrap();
        let AgentEvent::Message { value, .. } = &events[0] else {
            panic!("message")
        };
        let token = value.ports[0].clone();
        let live = BTreeMap::from([(7, "https://example.test".into())]);
        assert!(
            hub.handle(
                7,
                MessageRequest::PollPort {
                    port: token.clone(),
                    max: 1
                },
                &live
            )
            .is_ok()
        );
        assert!(
            hub.handle_agent(
                agent,
                MessageRequest::PollPort {
                    port: token,
                    max: 1
                },
                "https://example.test",
                "origin-1"
            )
            .is_err()
        );
        registry.retire(&mut hub).unwrap();
    }

    #[test]
    fn queued_shared_claims_survive_failed_release_cleanup_and_do_not_consume_sibling_claims() {
        use oryn_js::{message_wire::*, shared_memory::*};
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Shared {
            fail: AtomicBool,
            live: Mutex<std::collections::BTreeSet<(Principal, SharedMemoryHandoff)>>,
        }
        impl SharedMessageBackend for Shared {
            fn adopt(
                &self,
                _: Principal,
                _: &[SharedMemoryReference],
            ) -> Result<Arc<dyn SharedMessageLease>, String> {
                Err("unused".into())
            }
            fn discard_delivered(
                &self,
                target: Principal,
                refs: &[SharedMemoryReference],
            ) -> Result<(), String> {
                if self.fail.load(Ordering::SeqCst) {
                    return Err("injected cleanup failure".into());
                }
                for reference in refs {
                    self.live
                        .lock()
                        .unwrap()
                        .remove(&(target, reference.handoff));
                }
                Ok(())
            }
            fn discard_offered(
                &self,
                _: Principal,
                _: &[SharedMemoryReference],
            ) -> Result<(), String> {
                Ok(())
            }
            fn retire_principal(&self, _: Principal) {}
        }
        let reference = |id| SharedMemoryReference {
            handoff: SharedMemoryHandoff([id; 32]),
            descriptor: SharedMemoryDescriptor {
                token: SharedMemoryToken([8; 32]),
                byte_length: 8,
                mapped_length: 4096,
                maximum_length: 8,
                data_offset: 0,
                reserved_length: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
                growable: false,
            },
        };
        let backend = Arc::new(Shared {
            fail: AtomicBool::new(true),
            live: Mutex::new(std::collections::BTreeSet::from([
                (Principal::Window(7), reference(1).handoff),
                (Principal::Window(8), reference(2).handoff),
            ])),
        });
        let process = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = MessageHub::default();
        hub.set_shared_message_backend(backend.clone());
        let first = connect(&mut registry, owner(7), bootstrap(), &process).unwrap();
        let sibling = connect(&mut registry, owner(8), bootstrap(), &process).unwrap();
        for (connection, id) in [(&first, 1), (&sibling, 2)] {
            let client = registry
                .agents
                .get_mut(&connection.agent)
                .unwrap()
                .clients
                .get_mut(&connection.client.id)
                .unwrap();
            client.events.push_back(AgentEvent::Message {
                client: Some(connection.client.clone()),
                value: WireValue {
                    shared: vec![reference(id)],
                    shared_queue: None,
                    shared_recipient: None,
                    wasm_modules: Vec::new(),
                    version: 1,
                    bytes: Vec::new(),
                    hosts: Vec::new(),
                    buffers: Vec::new(),
                    ports: Vec::new(),
                },
            });
        }
        assert!(registry.release(&mut hub, &owner(7), &first).is_err());
        assert!(registry.release_owner(&first).unwrap().is_none());
        backend.fail.store(false, Ordering::SeqCst);
        finish_pending(&mut registry, &mut hub);
        assert_eq!(
            *backend.live.lock().unwrap(),
            std::collections::BTreeSet::from([(Principal::Window(8), reference(2).handoff)])
        );
        registry.agents.get_mut(&sibling.agent).unwrap().failure = Some(error("process failed"));
        finish_pending(&mut registry, &mut hub);
        assert!(backend.live.lock().unwrap().is_empty());
        assert!(registry.release_owner(&sibling).unwrap().is_none());
        assert_eq!(process.lock().unwrap().stops, 1);
    }

    #[test]
    fn releasing_client_disposes_ports_in_its_undelivered_queue() {
        use oryn_js::message_wire::*;
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = MessageHub::default();
        let first = connect(&mut registry, owner(7), bootstrap(), &state).unwrap();
        let sibling = connect(&mut registry, owner(8), bootstrap(), &state).unwrap();
        let agent = first.agent.0;
        let MessageResponse::Channel(ports) = hub
            .handle_agent(
                agent,
                MessageRequest::CreateChannel,
                "https://example.test",
                "origin-1",
            )
            .unwrap()
        else {
            panic!("channel")
        };
        let MessageResponse::Transferred(ports) = hub
            .handle_agent(
                agent,
                MessageRequest::Transfer {
                    ports: vec![ports[0].clone()],
                },
                "https://example.test",
                "origin-1",
            )
            .unwrap()
        else {
            panic!("transfer")
        };
        state.lock().unwrap().events.push(AgentEvent::Message {
            client: Some(first.client.clone()),
            value: WireValue {
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
                version: 1,
                bytes: vec![1],
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports,
            },
        });
        assert!(
            registry
                .poll(&mut hub, &owner(7), &first, 0)
                .unwrap()
                .is_empty()
        );
        let AgentEvent::Message { value, .. } =
            &registry.agents[&first.agent].clients[&first.client.id].events[0]
        else {
            panic!("queued message")
        };
        let token = value.ports[0].clone();
        registry.release(&mut hub, &owner(7), &first).unwrap();
        let live = BTreeMap::from([(7, "https://example.test".into())]);
        assert!(
            hub.handle(
                7,
                MessageRequest::PollPort {
                    port: token,
                    max: 1
                },
                &live
            )
            .is_err()
        );
        assert_eq!(state.lock().unwrap().stops, 0);
        assert!(
            registry
                .poll(&mut hub, &owner(8), &sibling, 32)
                .unwrap()
                .is_empty()
        );
        registry.release(&mut hub, &owner(8), &sibling).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }
    fn numbered_client_event(line: u32) -> AgentEvent {
        AgentEvent::Error {
            message: "original worker event".encode_utf16().collect(),
            filename: "worker.js".into(),
            line,
            column: 1,
        }
    }

    #[test]
    fn client_poll_drains_admitted_events_before_fetching_another_bounded_batch() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut boot = bootstrap();
        boot.kind = AgentKind::Dedicated;
        boot.limits.queue_messages = 2;
        boot.limits.queue_bytes = 2 * event_bytes(&numbered_client_event(1));
        let connection = connect(&mut registry, owner(1), boot, &state).unwrap();
        state.lock().unwrap().events = vec![numbered_client_event(1), numbered_client_event(2)];
        assert!(
            registry
                .poll(&mut hub, &owner(1), &connection, 0)
                .unwrap()
                .is_empty()
        );
        state.lock().unwrap().events = vec![numbered_client_event(3), numbered_client_event(4)];
        // Authorization still precedes access to accepted queued events.
        assert_eq!(
            registry
                .poll(&mut hub, &owner(2), &connection, 1)
                .unwrap_err()
                .name,
            "SecurityError"
        );
        for expected in [1, 2] {
            assert!(
                matches!(registry.poll(&mut hub, &owner(1), &connection, 1).unwrap().as_slice(),
                [AgentEvent::Error { line, .. }] if *line == expected)
            );
            assert_eq!(
                state.lock().unwrap().polls,
                1,
                "queued delivery must not consume another process batch"
            );
        }
        assert_eq!(
            registry.agents[&connection.agent].clients[&connection.client.id].bytes,
            0
        );
        assert!(matches!(
            registry
                .poll(&mut hub, &owner(1), &connection, 32)
                .unwrap()
                .as_slice(),
            [
                AgentEvent::Error { line: 3, .. },
                AgentEvent::Error { line: 4, .. }
            ]
        ));
        assert_eq!(state.lock().unwrap().polls, 2);
        assert_eq!(
            registry.agents[&connection.agent].clients[&connection.client.id].bytes,
            0
        );
        assert!(registry.principal(connection.agent).is_ok());
        registry.release(&mut hub, &owner(1), &connection).unwrap();
        finish_pending(&mut registry, &mut hub);
        assert_eq!(state.lock().unwrap().stops, 1);
    }

    #[test]
    fn client_poll_preserves_admitted_events_before_original_later_process_failure() {
        struct ReportsThenFailure(bool);
        impl AgentProcess for ReportsThenFailure {
            fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
                Ok(())
            }
            fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
                if std::mem::replace(&mut self.0, true) {
                    return Err(WorkerError::new(
                        "TypeError",
                        "original later process failure",
                    ));
                }
                Ok(vec![numbered_client_event(1), numbered_client_event(2)])
            }
            fn terminate(&mut self) -> Result<(), WorkerError> {
                Ok(())
            }
        }
        let mut registry = AgentRegistry::new(8);
        let mut hub = oryn_js::message_wire::MessageHub::default();
        let mut boot = bootstrap();
        boot.kind = AgentKind::Dedicated;
        let connection = registry
            .connect(owner(1), boot, |_| Ok(Box::new(ReportsThenFailure(false))))
            .unwrap();
        for expected in [1, 2] {
            assert!(
                matches!(registry.poll(&mut hub, &owner(1), &connection, 1).unwrap().as_slice(),
                [AgentEvent::Error { line, .. }] if *line == expected)
            );
        }
        assert_eq!(
            registry.agents[&connection.agent].clients[&connection.client.id].bytes,
            0
        );
        let original = registry
            .poll(&mut hub, &owner(1), &connection, 1)
            .unwrap_err();
        assert_eq!(original.name, "TypeError");
        assert_eq!(original.message, "original later process failure");
        assert!(registry.principal(connection.agent).is_err());
        finish_pending(&mut registry, &mut hub);
    }
}
