//! Context-owned service-worker versions and controlled document lifetimes.
//! Native process events, not successful script evaluation alone, advance
//! installing/waiting/active state. Blocking work never holds registry locks.

/// Inputs for update job.
struct ServiceUpdateJobInputs {
    operation: u64,
    owner: AgentOwner,
    network: SharedNetworkBroker,
    partition: String,
    origin: AgentOrigin,
    policy: AgentPolicy,
    script: String,
    scope: Option<String>,
    script_type: ScriptType,
    cache: String,
    register: bool,
}
/// Inputs for update.
struct ServiceUpdateInputs {
    operation: u64,
    owner: AgentOwner,
    network: SharedNetworkBroker,
    partition: String,
    origin: AgentOrigin,
    policy: AgentPolicy,
    script: String,
    scope: Option<String>,
    script_type: ScriptType,
    cache: String,
}
/// Inputs for run.
struct ServiceRunInputs {
    operation: u64,
    owner: AgentOwner,
    request: ServiceRequest,
    network: SharedNetworkBroker,
    partition: String,
    origin: AgentOrigin,
    policy: AgentPolicy,
}
type NavigationFetchTable = Arc<Mutex<BTreeMap<(oryn_common::v3::PageId, u64), Arc<AtomicBool>>>>;

type MaintenanceEntry = (Arc<()>, Option<SoftUpdateSource>);

use crate::{
    agent_registry::AgentOwner,
    network::SharedNetworkBroker,
    runtime::{BrowserContext, WeakBrowserContext},
};
use oryn_js::{service_worker_wire::*, worker_wire::*};
#[path = "service_registry_persistence.rs"]
mod persistence;
#[path = "service_registry_stream.rs"]
mod streaming;
pub(crate) use streaming::FrozenFetchController;
#[path = "service_registry_preload.rs"]
mod preload;
#[path = "service_registry_jobs.rs"]
mod profile_jobs;
#[path = "service_stream_custody.rs"]
mod stream_custody;
pub(crate) use preload::Admission as PreloadCustody;
pub(crate) use profile_jobs::ProfileJobs;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone)]
struct Version {
    directory: Option<Arc<crate::storage::ServiceVersionReservation>>,
    retiring: bool,
    revalidate_imports: bool,
    snapshot: WorkerSnapshot,
    bootstrap: AgentBootstrap,
    serial: Arc<Mutex<()>>,
    inbox: Arc<Mutex<ServiceControlInbox>>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
    skip_waiting: bool,
}
#[derive(Clone)]
struct RemoteVersion {
    owner: std::sync::Weak<ServiceRegistry>,
    snapshot: WorkerSnapshot,
    bootstrap: AgentBootstrap,
}
#[derive(Clone)]
struct Registration {
    lineage: String,
    durable_fingerprint: Option<[u8; 32]>,
    last_update_check_ms: u64,
    snapshot: RegistrationSnapshot,
    partition: String,
    origin: String,
    script_url: String,
    script_type: ScriptType,
    source: Vec<u16>,
    unregistered: bool,
}
struct Operation {
    owner: AgentOwner,
    group: u64,
    result: Option<Result<ServiceResult, WorkerError>>,
}
#[derive(Clone)]
struct ServiceJobAuthority {
    owner: AgentOwner,
    cookie_ancestor_urls: Vec<String>,
    cookie_top_level_url: Option<String>,
}
#[derive(Clone)]
struct SoftUpdateSource {
    agent: AgentId,
    registration: RegistrationId,
    partition: String,
    origin: AgentOrigin,
    policy: AgentPolicy,
    authority: ServiceJobAuthority,
}
impl SoftUpdateSource {
    fn from_bootstrap(bootstrap: &AgentBootstrap) -> Option<Self> {
        Some(Self {
            agent: bootstrap.id,
            registration: bootstrap.service_registration?,
            partition: bootstrap.partition.clone(),
            origin: bootstrap.origin.clone(),
            policy: bootstrap.policy.clone(),
            authority: ServiceJobAuthority {
                owner: AgentOwner::Agent(bootstrap.id),
                cookie_ancestor_urls: bootstrap.cookie_ancestor_urls.clone(),
                cookie_top_level_url: bootstrap.top_level_url.clone(),
            },
        })
    }
}
struct EquivalentJob {
    settled: bool,
    authority: ServiceJobAuthority,
    profile: Option<Arc<profile_jobs::SharedJob>>,
}
#[derive(Default)]
struct State {
    pending_restore_admissions: BTreeMap<AgentId, crate::agent_service::ServiceAdmission>,
    soft_updates: std::collections::BTreeSet<RegistrationId>,
    equivalent_jobs: BTreeMap<u64, EquivalentJob>,
    remote_versions: BTreeMap<ServiceVersionId, RemoteVersion>,
    observed_registration_ids: BTreeMap<(String, String, String), RegistrationId>,
    retiring_orphans: std::collections::BTreeSet<AgentId>,
    orphaned_services: std::collections::BTreeSet<AgentId>,
    dirty_registrations: std::collections::BTreeSet<RegistrationId>,
    client_leases: BTreeMap<AgentOwner, crate::storage::ServiceClientLease>,
    activation_watches: BTreeMap<RegistrationId, crate::storage::ServiceScopeWatch>,
    restored_namespaces: BTreeMap<(String, String), u64>,
    cache_owners: BTreeMap<AgentOwner, crate::storage::cache::CacheLeaseOwner>,
    cache_cleanup: Vec<crate::storage::cache::CacheLeaseOwner>,
    registrations: BTreeMap<RegistrationId, Registration>,
    versions: BTreeMap<AgentId, Version>,
    clients: BTreeMap<AgentOwner, RegistrationId>,
    operations: BTreeMap<u64, Operation>,
    observed: BTreeMap<AgentOwner, String>,
    early_skip: std::collections::BTreeSet<AgentId>,
    client_ids: BTreeMap<AgentOwner, String>,
    messages: BTreeMap<AgentOwner, std::collections::VecDeque<ClientMessage>>,
    reserved_bytes: usize,
    reserved_messages: BTreeMap<AgentOwner, usize>,
}
pub(crate) struct ServiceRegistry {
    maintenance: Mutex<BTreeMap<AgentId, MaintenanceEntry>>,
    maintenance_running: AtomicBool,
    maintenance_failure: Mutex<Option<WorkerError>>,
    preloads: Mutex<BTreeMap<u64, preload::PendingPreload>>,
    preload_owners: Mutex<Vec<Arc<preload::PreloadOwner>>>,
    persistence: Mutex<()>,
    navigation_fetches: NavigationFetchTable,
    weak_self: std::sync::Weak<ServiceRegistry>,
    context: WeakBrowserContext,
    state: Mutex<State>,
    next: AtomicU64,
    retired: AtomicBool,
    message_jobs: std::sync::atomic::AtomicUsize,
    message_bytes: std::sync::atomic::AtomicUsize,
    jobs: Mutex<Vec<std::thread::JoinHandle<()>>>,
    updates: Mutex<std::collections::BTreeSet<u64>>,
    updates_changed: std::sync::Condvar,
}
// Native-only labels add a bounded suffix without replacing the original
// exception name, message cause, or loader classification.
fn service_task_stage(
    mut failure: WorkerError,
    stage: &'static str,
    agent: AgentId,
    task: u64,
) -> WorkerError {
    failure.message.push_str(&format!(
        " [service stage={stage} agent={} task={task}]",
        agent.0
    ));
    failure
}
fn service_window_error(
    mut failure: WorkerError,
    stage: &'static str,
    agent: AgentId,
    page: oryn_common::v3::PageId,
    document: Option<(u64, u64)>,
) -> WorkerError {
    failure.message.push_str(&format!(
        " [service stage={stage} agent={} page={}",
        agent.0, page.0
    ));
    if let Some((generation, epoch)) = document {
        failure.message.push_str(&format!(
            " source-generation={generation} source-epoch={epoch}"
        ));
    }
    failure.message.push(']');
    failure
}
#[cfg(test)]
thread_local! {static FAIL_SERVICE_WINDOW_STAGE:std::cell::RefCell<Option<(&'static str,WorkerError)>>=const{std::cell::RefCell::new(None)};}
fn service_window_stage<T>(
    stage: &'static str,
    agent: AgentId,
    page: oryn_common::v3::PageId,
    document: Option<(u64, u64)>,
    operation: impl FnOnce() -> Result<T, WorkerError>,
) -> Result<T, WorkerError> {
    #[cfg(test)]
    if let Some(failure) = FAIL_SERVICE_WINDOW_STAGE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(target, _)| *target == stage) {
            slot.take().map(|(_, failure)| failure)
        } else {
            None
        }
    }) {
        return Err(service_window_error(failure, stage, agent, page, document));
    }
    operation().map_err(|failure| service_window_error(failure, stage, agent, page, document))
}
#[derive(Default)]
struct ServiceControlInbox {
    ready: bool,
    runtime_errors: std::collections::VecDeque<WorkerError>,
    runtime_error_bytes: usize,
    dropped_runtime_errors: u64,
    body: BTreeMap<u64, std::collections::VecDeque<(AgentEvent, usize)>>,
    started: std::collections::BTreeSet<u64>,
    ended: std::collections::BTreeSet<u64>,
    active: std::collections::BTreeSet<u64>,
    completed: BTreeMap<(u64, u8), (AgentEvent, usize)>,
    bytes: usize,
    fatal: Option<WorkerError>,
}
impl ServiceControlInbox {
    fn accept_runtime_marker(&mut self, event: &AgentEvent) -> bool {
        match event {
            AgentEvent::Ready => {
                if self.ready {
                    self.fatal = Some(error("duplicate service startup readiness"));
                }
                self.ready = true;
                true
            }
            AgentEvent::Error {
                message,
                filename,
                line,
                column,
            } => {
                let diagnostic = service_script_diagnostic(message, filename, *line, *column);
                // ErrorEvent may come from an unrelated timer while startup
                // awaits a module. Evaluation and task results decide success.
                const LIMIT: usize = 512 * 1024;
                let bytes = diagnostic.message.len();
                while self.runtime_errors.len() >= 16
                    || self.runtime_error_bytes.saturating_add(bytes) > LIMIT
                {
                    let Some(old) = self.runtime_errors.pop_front() else {
                        break;
                    };
                    self.runtime_error_bytes =
                        self.runtime_error_bytes.saturating_sub(old.message.len());
                    self.dropped_runtime_errors = self.dropped_runtime_errors.saturating_add(1);
                }
                if bytes <= LIMIT {
                    self.runtime_error_bytes += bytes;
                    self.runtime_errors.push_back(diagnostic);
                } else {
                    self.dropped_runtime_errors = self.dropped_runtime_errors.saturating_add(1);
                }
                true
            }
            _ => false,
        }
    }
}
// Preserve the runtime's error text and its already disclosed script location;
// do not substitute a guessed native cause or serialize source/body data.
fn service_script_diagnostic(
    message: &[u16],
    filename: &str,
    line: u32,
    column: u32,
) -> WorkerError {
    let units = &message[..message.len().min(8192)];
    let mut detail = String::from_utf16_lossy(units);
    if units.len() < message.len() {
        detail.push_str(" [truncated]");
    }
    let mut end = filename.len().min(4096);
    while !filename.is_char_boundary(end) {
        end -= 1;
    }
    let location = &filename[..end];
    let suffix = if end < filename.len() {
        " [truncated]"
    } else {
        ""
    };
    WorkerError::new(
        "TypeError",
        format!("{detail} [service script {location}{suffix}:{line}:{column}]"),
    )
}
struct ServiceTaskSlot {
    registry: std::sync::Weak<ServiceRegistry>,
    inbox: Arc<Mutex<ServiceControlInbox>>,
    id: u64,
}
impl Drop for ServiceTaskSlot {
    fn drop(&mut self) {
        let mut inbox = self
            .inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inbox.active.remove(&self.id);
        inbox.started.remove(&self.id);
        inbox.ended.remove(&self.id);
        if let Some(queue) = inbox.body.remove(&self.id) {
            for (_, bytes) in queue {
                inbox.bytes = inbox.bytes.saturating_sub(bytes);
            }
        }
        for stage in [0, 1] {
            if let Some((_, bytes)) = inbox.completed.remove(&(self.id, stage)) {
                inbox.bytes = inbox.bytes.saturating_sub(bytes);
            }
        }
        drop(inbox);
        if let Some(registry) = self.registry.upgrade() {
            registry.discard_preload(self.id);
        }
    }
}
struct NavigationFetchWait {
    table: NavigationFetchTable,
    key: (oryn_common::v3::PageId, u64),
}
impl Drop for NavigationFetchWait {
    fn drop(&mut self) {
        self.table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}
struct ServiceEventLease {
    check_soft_update: bool,
    soft_update_source: Option<SoftUpdateSource>,
    activity: Option<crate::storage::ServiceActivityLease>,
    registry: std::sync::Weak<ServiceRegistry>,
    agent: AgentId,
    serial: Arc<Mutex<()>>,
    inbox: Arc<Mutex<ServiceControlInbox>>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
}
impl Drop for ServiceEventLease {
    fn drop(&mut self) {
        let final_local = self.inflight.fetch_sub(1, Ordering::AcqRel) == 1;
        drop(self.activity.take());
        if (final_local || self.check_soft_update)
            && let Some(registry) = self.registry.upgrade()
        {
            registry.enqueue_maintenance(
                self.agent,
                self.check_soft_update
                    .then(|| self.soft_update_source.take())
                    .flatten(),
            );
        }
    }
}

struct UpdateTicket {
    registry: Arc<ServiceRegistry>,
    operation: Option<u64>,
}
impl Drop for UpdateTicket {
    fn drop(&mut self) {
        if let Some(operation) = self.operation {
            let job = {
                let mut updates = self
                    .registry
                    .updates
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                updates.remove(&operation);
                let job = self
                    .registry
                    .state()
                    .ok()
                    .and_then(|mut state| state.equivalent_jobs.remove(&operation));
                self.registry.updates_changed.notify_all();
                job
            };
            if let Some(job) = job
                && let Some(profile) = job.profile
            {
                profile.finish();
            }
        }
    }
}
struct MessageJobBudget {
    registry: Arc<ServiceRegistry>,
    bytes: usize,
}
impl Drop for MessageJobBudget {
    fn drop(&mut self) {
        self.registry.message_jobs.fetch_sub(1, Ordering::AcqRel);
        self.registry
            .message_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
fn service_version_identity() -> Result<ServiceVersionId, WorkerError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|failure| error(failure.to_string()))?;
    if bytes == [0; 16] {
        return Err(error("service version entropy unavailable"));
    }
    Ok(ServiceVersionId(bytes))
}
fn registration_lineage() -> Result<String, WorkerError> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|failure| error(failure.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn error(message: impl Into<String>) -> WorkerError {
    WorkerError::new("InvalidStateError", message)
}
impl ServiceRegistry {
    pub(crate) fn new(context: &BrowserContext) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            maintenance: Mutex::new(BTreeMap::new()),
            maintenance_running: AtomicBool::new(false),
            maintenance_failure: Mutex::new(None),
            preloads: Mutex::new(BTreeMap::new()),
            preload_owners: Mutex::new(Vec::new()),
            navigation_fetches: Arc::new(Mutex::new(BTreeMap::new())),
            persistence: Mutex::new(()),
            weak_self: weak.clone(),
            context: context.downgrade(),
            state: Mutex::new(State::default()),
            next: AtomicU64::new(1),
            retired: AtomicBool::new(false),
            message_jobs: std::sync::atomic::AtomicUsize::new(0),
            message_bytes: std::sync::atomic::AtomicUsize::new(0),
            jobs: Mutex::new(Vec::new()),
            updates: Mutex::new(Default::default()),
            updates_changed: std::sync::Condvar::new(),
        })
    }
    /// Event/stream destructors only transfer a bounded native version ticket.
    /// Activation and process retirement run on this retained background job.
    fn enqueue_maintenance(
        self: &Arc<Self>,
        agent: AgentId,
        soft_update: Option<SoftUpdateSource>,
    ) {
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        {
            let mut pending = self
                .maintenance
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.retired.load(Ordering::Acquire) {
                return;
            }
            let soft_update =
                soft_update.or_else(|| pending.get(&agent).and_then(|(_, soft)| soft.clone()));
            pending.insert(agent, (Arc::new(()), soft_update));
        }
        self.start_maintenance();
    }
    fn start_maintenance(self: &Arc<Self>) {
        if self.retired.load(Ordering::Acquire)
            || self.maintenance_running.swap(true, Ordering::AcqRel)
        {
            return;
        }
        struct Ticket(Arc<ServiceRegistry>);
        impl Drop for Ticket {
            fn drop(&mut self) {
                self.0.maintenance_running.store(false, Ordering::Release);
            }
        }
        let registry = self.clone();
        let ticket = Ticket(registry.clone());
        match std::thread::Builder::new()
            .name("oryn-service-maintenance".into())
            .spawn(move || {
                let result = registry.run_maintenance();
                let succeeded = result.is_ok();
                if let Err(failure) = result {
                    *registry
                        .maintenance_failure
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(failure);
                }
                drop(ticket);
                if succeeded
                    && !registry
                        .maintenance
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_empty()
                {
                    registry.start_maintenance();
                }
            }) {
            Ok(job) => self
                .jobs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(job),
            Err(failure) => {
                *self
                    .maintenance_failure
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error(format!(
                    "service maintenance spawn failed: {failure}"
                )));
            }
        }
    }
    fn run_maintenance(&self) -> Result<(), WorkerError> {
        if self.retired.load(Ordering::Acquire) {
            return Ok(());
        }
        let pending = self
            .maintenance
            .lock()
            .map_err(|_| error("service maintenance poisoned"))?
            .clone();
        let mut failure = None;
        for (agent, (ticket, soft_update)) in pending {
            let result = (|| {
                if let Some(source) = soft_update
                    && let Some(registry) = self.weak_self.upgrade()
                {
                    registry.schedule_stale_update(source)?;
                }
                let waiting = self
                    .state()?
                    .registrations
                    .iter()
                    .filter(|(_, registration)| {
                        registration.snapshot.waiting.is_some()
                            && registration
                                .snapshot
                                .active
                                .as_ref()
                                .is_some_and(|active| active.id == agent)
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for registration in waiting {
                    self.try_activate(registration)?;
                }
                self.cleanup_unregistered()
            })();
            match result {
                Ok(()) => {
                    let mut current = self
                        .maintenance
                        .lock()
                        .map_err(|_| error("service maintenance poisoned"))?;
                    if current
                        .get(&agent)
                        .is_some_and(|(active, _)| Arc::ptr_eq(active, &ticket))
                    {
                        current.remove(&agent);
                    }
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
    /// Join only completed jobs, and never while holding the ownership lock:
    /// job finalizers may re-enter registry cleanup during unwinding.
    fn reap_jobs(&self) -> Result<(), WorkerError> {
        // Only the retained worker executes maintenance. Re-entrant service
        // requests during activation must never wait for their own lifecycle.
        let failure = self
            .maintenance_failure
            .lock()
            .map_err(|_| error("service maintenance failure lock poisoned"))?
            .take();
        if !self
            .maintenance
            .lock()
            .map_err(|_| error("service maintenance poisoned"))?
            .is_empty()
            && let Some(registry) = self.weak_self.upgrade()
        {
            registry.start_maintenance();
        }
        if let Some(failure) = failure {
            return Err(failure);
        }
        self.retry_restore_publications()?;
        self.reap_cache_owners()?;
        self.reap_orphaned_services()?;
        let dirty = self
            .state()?
            .dirty_registrations
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for id in dirty {
            self.persist_registration(id)?;
        }
        let waiting = self
            .state()?
            .registrations
            .iter()
            .filter(|(_, registration)| {
                !registration.unregistered && registration.snapshot.waiting.is_some()
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in waiting {
            self.try_activate(id)?;
        }
        let completed = {
            let mut jobs = self
                .jobs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut completed = Vec::new();
            let mut index = 0;
            while index < jobs.len() {
                if jobs[index].is_finished() {
                    completed.push(jobs.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            completed
        };
        let mut panicked = false;
        for job in completed {
            panicked |= job.join().is_err();
        }
        let cleanup = self.cleanup_unregistered();
        if panicked {
            Err(error("service background job panicked"))
        } else {
            cleanup
        }
    }
    fn context(&self) -> Result<BrowserContext, WorkerError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(error("service registry retired"));
        }
        self.context
            .upgrade()
            .ok_or_else(|| error("service context retired"))
    }
    fn id(&self) -> Result<u64, WorkerError> {
        self.next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| error("service identity exhausted"))
    }
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, WorkerError> {
        self.state
            .lock()
            .map_err(|_| error("service registry poisoned"))
    }
    fn visible(reg: &Registration, partition: &str, origin: &str) -> bool {
        !reg.unregistered && reg.partition == partition && reg.origin == origin
    }
    pub(crate) fn request(
        self: &Arc<Self>,
        owner: AgentOwner,
        request: ServiceRequest,
        network: SharedNetworkBroker,
    ) -> Result<ServiceResponse, WorkerError> {
        let context = self.context()?;
        let (partition, origin, policy) = context.native_worker_authority(&owner)?;
        if !context.worker_secure_context(&owner)? {
            return Err(WorkerError::new(
                "SecurityError",
                "service API requires committed secure-context authority",
            ));
        }
        if matches!(&request, ServiceRequest::Update { .. })
            && let AgentOwner::Agent(agent) = &owner
        {
            let bootstrap = context.worker_bootstrap(*agent)?;
            if bootstrap.kind == AgentKind::Service
                && self
                    .state()?
                    .versions
                    .get(agent)
                    .is_none_or(|version| version.snapshot.state == WorkerState::Installing)
            {
                return Err(WorkerError::new(
                    "InvalidStateError",
                    "an installing service worker cannot update a registration",
                ));
            }
        }
        if !matches!(&request, ServiceRequest::Cache { .. }) {
            self.restore_for_owner(&owner, network.clone())?;
        }
        self.reap_jobs()?;
        if let AgentOwner::Agent(agent) = &owner {
            self.ensure_agent_controller(&context.worker_bootstrap(*agent)?)?;
        }
        if origin.opaque
            || !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(
                &origin.serialized,
            ))
        {
            return Err(WorkerError::new(
                "SecurityError",
                "service workers require a trustworthy non-opaque origin",
            ));
        }
        if matches!(&owner, AgentOwner::Window { .. }) {
            self.state()?
                .observed
                .insert(owner.clone(), policy.url.clone());
        }
        match request {
            ServiceRequest::Cache { operation, .. } => {
                let lease = {
                    let mut state = self.state()?;
                    match state.cache_owners.get(&owner) {
                        Some(lease) => *lease,
                        None => {
                            let lease = crate::storage::cache::CacheLeaseOwner::new()
                                .map_err(|failure| error(failure.to_string()))?;
                            state.cache_owners.insert(owner.clone(), lease);
                            lease
                        }
                    }
                };
                context
                    .service_cache_operation(&owner, lease, operation)
                    .map(ServiceResponse::Cache)
            }
            ServiceRequest::Poll { operation, .. } => {
                let mut state = self.state()?;
                let entry = state
                    .operations
                    .get_mut(&operation)
                    .ok_or_else(|| error("unknown service operation"))?;
                if entry.owner != owner {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "foreign service operation",
                    ));
                }
                if let Some(result) = entry.result.clone() {
                    Ok(ServiceResponse::Complete { operation, result })
                } else {
                    Ok(ServiceResponse::Pending { operation })
                }
            }
            ServiceRequest::Acknowledge { operation, .. } => {
                let mut state = self.state()?;
                if let Some(entry) = state.operations.get(&operation) {
                    if entry.owner != owner {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "foreign service operation acknowledgement",
                        ));
                    }
                    if entry.result.is_none() {
                        return Err(error("service operation not completed"));
                    }
                    state.operations.remove(&operation);
                }
                Ok(ServiceResponse::Done)
            }
            ServiceRequest::Controller { .. } => {
                let state = self.state()?;
                Ok(ServiceResponse::Controller(
                    state
                        .clients
                        .get(&owner)
                        .and_then(|id| state.registrations.get(id))
                        .and_then(|registration| registration.snapshot.active.clone()),
                ))
            }
            ServiceRequest::Snapshot { take_messages, .. } => {
                // Complete fallible metadata reads before taking custody of
                // messages whose native transfer capabilities must be delivered
                // or explicitly reclaimed by the response publication owner.
                let registrations = self.registration_snapshots(&partition, &origin.authority)?;
                let mut state = self.state()?;
                let controller = state
                    .clients
                    .get(&owner)
                    .and_then(|id| state.registrations.get(id))
                    .and_then(|registration| registration.snapshot.active.clone());
                let messages = if take_messages {
                    state
                        .messages
                        .get_mut(&owner)
                        .map(|messages| {
                            let count = messages.len().min(16);
                            messages.drain(..count).collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                drop(state);
                Ok(ServiceResponse::Snapshot(ContainerSnapshot {
                    controller,
                    registrations,
                    messages,
                }))
            }
            ServiceRequest::PostMessage { worker, value, .. } => {
                self.post_message(owner, worker, value)?;
                Ok(ServiceResponse::Done)
            }
            ServiceRequest::ClientPostMessage { client, value } => {
                self.post_client_message(owner, &client, value)?;
                Ok(ServiceResponse::Done)
            }
            ServiceRequest::WorkerRegistration => {
                let AgentOwner::Agent(agent) = owner else {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "service registration requires service principal",
                    ));
                };
                let bootstrap = context.worker_bootstrap(agent)?;
                if bootstrap.kind != AgentKind::Service {
                    return Err(WorkerError::new("SecurityError", "not a service agent"));
                }
                let id = bootstrap
                    .service_registration
                    .ok_or_else(|| error("service registration metadata missing"))?;
                let registration = self
                    .state()?
                    .registrations
                    .get(&id)
                    .map(|registration| registration.snapshot.clone())
                    .ok_or_else(|| error("service registration retired"))?;
                Ok(ServiceResponse::Registration(registration))
            }
            ServiceRequest::SkipWaiting => {
                let AgentOwner::Agent(agent) = owner else {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "skipWaiting requires service principal",
                    ));
                };
                if context.worker_bootstrap(agent)?.kind != AgentKind::Service {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "skipWaiting requires service version",
                    ));
                }
                let registration = {
                    let mut state = self.state()?;
                    if let Some(version) = state.versions.get_mut(&agent) {
                        version.skip_waiting = true;
                        version.bootstrap.service_skip_waiting = true;
                    } else {
                        state.early_skip.insert(agent);
                    }
                    state
                        .registrations
                        .iter()
                        .find(|(_, reg)| {
                            reg.snapshot
                                .waiting
                                .as_ref()
                                .is_some_and(|waiting| waiting.id == agent)
                        })
                        .map(|(id, _)| *id)
                };
                if let Some(registration) = registration {
                    let registry = self.clone();
                    let job = std::thread::Builder::new()
                        .name("oryn-service-activate".into())
                        .spawn(move || {
                            if registry.persist_registration(registration).is_ok() {
                                let _ = registry.try_activate(registration);
                            }
                        })
                        .map_err(|failure| error(failure.to_string()))?;
                    self.jobs
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(job);
                }
                Ok(ServiceResponse::Done)
            }
            ServiceRequest::Claim => {
                let AgentOwner::Agent(agent) = owner else {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "claim requires service principal",
                    ));
                };
                let (registration, version) = {
                    let state = self.state()?;
                    let version = state
                        .versions
                        .get(&agent)
                        .ok_or_else(|| error("claim requires live service version"))?;
                    let registration = state
                        .registrations
                        .values()
                        .find(|reg| {
                            !reg.unregistered
                                && reg.snapshot.active.as_ref().is_some_and(|active| {
                                    active.version_id == version.snapshot.version_id
                                        && matches!(
                                            active.state,
                                            WorkerState::Activating | WorkerState::Activated
                                        )
                                })
                        })
                        .ok_or_else(|| error("claim requires active service version"))?;
                    (registration.clone(), version.clone())
                };
                let coordinator = context
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                for registry in coordinator
                    .versions
                    .registries()
                    .map_err(|failure| error(failure.to_string()))?
                {
                    registry.claim_profile_clients(self, &registration, &version)?;
                }
                Ok(ServiceResponse::Done)
            }
            request => {
                let ordered = matches!(
                    &request,
                    ServiceRequest::Register { .. }
                        | ServiceRequest::Update { .. }
                        | ServiceRequest::Unregister { .. }
                );
                let mut admission = if ordered {
                    Some(
                        self.updates
                            .lock()
                            .map_err(|_| error("service update queue poisoned"))?,
                    )
                } else {
                    None
                };
                let identity = self.job_identity(&request, &partition, &origin, &policy)?;
                let authority = if identity.is_some() {
                    Some(Self::capture_job_authority(&context, &owner)?)
                } else {
                    None
                };
                let operation = self.id()?;
                {
                    let mut state = self.state()?;
                    if state.operations.len() >= 64 {
                        return Err(WorkerError::new(
                            "QuotaExceededError",
                            "service operation capacity",
                        ));
                    }
                    state.operations.insert(
                        operation,
                        Operation {
                            owner: owner.clone(),
                            group: operation,
                            result: None,
                        },
                    );
                    if identity.is_some() {
                        state.equivalent_jobs.insert(
                            operation,
                            EquivalentJob {
                                settled: false,
                                authority: authority.expect("ordered service job authority"),
                                profile: None,
                            },
                        );
                    }
                }
                if let Some((scope, key)) = identity {
                    let shared = (|| {
                        let coordinator = context
                            .context_store()
                            .service_coordinator()
                            .map_err(|failure| error(failure.to_string()))?;
                        let weak = self.weak_self.clone();
                        let recipient = owner.clone();
                        let recipient_partition = partition.clone();
                        let recipient_origin = origin.authority.clone();
                        coordinator.equivalent.admit(
                            scope,
                            key,
                            Box::new(move |result| {
                                let Some(registry) = weak.upgrade() else {
                                    return;
                                };
                                let pending = registry.state().ok().is_some_and(|state| {
                                    state.operations.get(&operation).is_some_and(|entry| {
                                        entry.owner == recipient && entry.result.is_none()
                                    })
                                });
                                if !pending {
                                    return;
                                }
                                // A result grants no fresh authority: each original recipient must
                                // still own its operation when delivery is attempted.
                                if registry
                                    .context()
                                    .and_then(|context| {
                                        context.retained_worker_authority(&recipient)
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                                let result = result.and_then(|(result, lineage)| match result {
                                    ServiceResult::Registration(Some(mut snapshot)) => {
                                        snapshot.id = registry.observed_registration_id(
                                            &recipient_partition,
                                            &recipient_origin,
                                            lineage.as_deref().ok_or_else(|| {
                                                error("shared registration lineage absent")
                                            })?,
                                        )?;
                                        Ok(ServiceResult::Registration(Some(snapshot)))
                                    }
                                    other => Ok(other),
                                });
                                registry.publish_local_job_result(operation, result);
                            }),
                        )
                    })();
                    match shared {
                        Ok(Some(shared)) => {
                            if let Some(job) = self.state()?.equivalent_jobs.get_mut(&operation) {
                                job.profile = Some(shared);
                            }
                        }
                        Ok(None) => {
                            self.state()?.equivalent_jobs.remove(&operation);
                            return Ok(ServiceResponse::Pending { operation });
                        }
                        Err(failure) => {
                            let mut state = self.state()?;
                            state.operations.remove(&operation);
                            state.equivalent_jobs.remove(&operation);
                            return Err(failure);
                        }
                    }
                }
                if let Some(queue) = admission.as_mut() {
                    queue.insert(operation);
                }
                drop(admission);
                let ticket = UpdateTicket {
                    registry: self.clone(),
                    operation: ordered.then_some(operation),
                };
                let registry = self.clone();
                let thread = std::thread::Builder::new()
                    .name(format!("oryn-service-job-{operation}"))
                    .spawn(move || {
                        let _ticket = ticket;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            registry.run(ServiceRunInputs {
                                operation,
                                owner,
                                request,
                                network,
                                partition,
                                origin,
                                policy,
                            })
                        }))
                        .unwrap_or_else(|_| Err(error("service job panicked")));
                        registry.publish_job_result(operation, result);
                    })
                    .map_err(|failure| {
                        if let Ok(mut state) = self.state() {
                            state.operations.remove(&operation);
                        }
                        error(failure.to_string())
                    })?;
                self.jobs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(thread);
                Ok(ServiceResponse::Pending { operation })
            }
        }
    }
    fn capture_job_authority(
        context: &BrowserContext,
        owner: &AgentOwner,
    ) -> Result<ServiceJobAuthority, WorkerError> {
        context.retained_worker_authority(owner)?;
        let cookie_ancestor_urls = context.worker_cookie_ancestor_urls(owner)?;
        let cookie_top_level_url = match owner {
            AgentOwner::Window { page, .. } => Some(
                context
                    .page_url(*page)
                    .map_err(|failure| error(failure.to_string()))?,
            ),
            AgentOwner::Agent(id) => context.worker_bootstrap(*id)?.top_level_url,
        };
        context.retained_worker_authority(owner)?;
        Ok(ServiceJobAuthority {
            owner: owner.clone(),
            cookie_ancestor_urls,
            cookie_top_level_url,
        })
    }
    fn publish_job_result(&self, group: u64, result: Result<ServiceResult, WorkerError>) {
        let shared = self.state().ok().and_then(|state| {
            state
                .equivalent_jobs
                .get(&group)
                .and_then(|job| job.profile.clone())
        });
        if let Some(shared) = shared {
            let publication = result.clone().and_then(|result| {
                let lineage = match &result {
                    ServiceResult::Registration(Some(snapshot)) => Some(
                        self.state()?
                            .registrations
                            .get(&snapshot.id)
                            .ok_or_else(|| error("shared registration retired before publication"))?
                            .lineage
                            .clone(),
                    ),
                    _ => None,
                };
                Ok((result, lineage))
            });
            shared.publish(publication);
        }
        self.publish_local_job_result(group, result);
    }
    fn publish_local_job_result(&self, group: u64, result: Result<ServiceResult, WorkerError>) {
        if let Ok(mut state) = self.state() {
            if let Some(job) = state.equivalent_jobs.get_mut(&group) {
                job.settled = true;
            }
            for operation in state
                .operations
                .values_mut()
                .filter(|operation| operation.group == group && operation.result.is_none())
            {
                operation.result = Some(result.clone());
            }
        }
    }
    fn job_identity(
        &self,
        request: &ServiceRequest,
        partition: &str,
        origin: &AgentOrigin,
        policy: &AgentPolicy,
    ) -> Result<Option<(String, String)>, WorkerError> {
        let (kind, scope, script, script_type, cache) = match request {
            ServiceRequest::Register {
                url,
                scope,
                script_type,
                update_via_cache,
                ..
            } => {
                let base = url::Url::parse(&policy.url)
                    .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
                let mut script = base
                    .join(url)
                    .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
                script.set_fragment(None);
                let mut scope = match scope {
                    Some(scope) => base.join(scope),
                    None => script.join("./"),
                }
                .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
                scope.set_fragment(None);
                for url in [&script, &scope] {
                    validate_service_registration_url(&base, url)?;
                }
                (
                    "register",
                    scope.to_string(),
                    script.to_string(),
                    *script_type,
                    update_via_cache.clone(),
                )
            }
            ServiceRequest::Update { registration, .. }
            | ServiceRequest::Unregister { registration, .. } => {
                let state = self.state()?;
                let Some(registration) =
                    state
                        .registrations
                        .get(registration)
                        .filter(|registration| {
                            Self::visible(registration, partition, &origin.authority)
                        })
                else {
                    return Ok(None);
                };
                (
                    if matches!(request, ServiceRequest::Update { .. }) {
                        "update"
                    } else {
                        "unregister"
                    },
                    registration.snapshot.scope.clone(),
                    registration.script_url.clone(),
                    registration.script_type,
                    registration.snapshot.update_via_cache.clone(),
                )
            }
            _ => return Ok(None),
        };
        if scope.len() > 65536 || script.len() > 65536 {
            return Err(WorkerError::new("TypeError", "service job URL capacity"));
        }
        let scope_key = serde_json::to_string(&(
            persistence::partition_key(partition)?,
            &origin.authority,
            &scope,
        ))
        .map_err(|failure| error(failure.to_string()))?;
        let key = serde_json::to_string(&(kind, &script, script_type, &cache))
            .map_err(|failure| error(failure.to_string()))?;
        Ok(Some((scope_key, key)))
    }
    fn reap_cache_owners(&self) -> Result<(), WorkerError> {
        let Some(context) = self.context.upgrade() else {
            return Ok(());
        };
        let owners = self
            .state()?
            .cache_owners
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let stale = owners
            .into_iter()
            .filter(|owner| context.retained_worker_authority(owner).is_err())
            .collect::<Vec<_>>();
        let leases = {
            let mut state = self.state()?;
            for owner in stale {
                if let Some(lease) = state.cache_owners.remove(&owner) {
                    state.cache_cleanup.push(lease);
                }
            }
            std::mem::take(&mut state.cache_cleanup)
        };
        let mut failure = None;
        for lease in leases {
            if let Err(error) = context.context_store().release_caches(lease) {
                self.state()?.cache_cleanup.push(lease);
                failure.get_or_insert_with(|| {
                    WorkerError::new("InvalidStateError", error.to_string())
                });
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn wait_profile_turn(&self, operation: u64) -> Result<(), WorkerError> {
        let shared = self
            .state()?
            .equivalent_jobs
            .get(&operation)
            .and_then(|job| job.profile.clone());
        if let Some(shared) = shared {
            shared.wait_turn(&self.retired)?;
        }
        Ok(())
    }
    fn wait_update_turn(&self, operation: u64) -> Result<(), WorkerError> {
        let mut updates = self
            .updates
            .lock()
            .map_err(|_| error("service update queue poisoned"))?;
        loop {
            if self.retired.load(Ordering::Acquire) {
                return Err(error("service registry retired"));
            }
            if updates.first().copied() == Some(operation) {
                return Ok(());
            }
            if !updates.contains(&operation) {
                return Err(error("service update ticket retired"));
            }
            updates = self
                .updates_changed
                .wait(updates)
                .map_err(|_| error("service update queue poisoned"))?;
        }
    }
    fn run(&self, params: ServiceRunInputs) -> Result<ServiceResult, WorkerError> {
        let ServiceRunInputs {
            operation,
            owner,
            request,
            network,
            partition,
            origin,
            policy,
        } = params;

        if matches!(
            &request,
            ServiceRequest::Register { .. }
                | ServiceRequest::Update { .. }
                | ServiceRequest::Unregister { .. }
        ) {
            self.wait_profile_turn(operation)?;
            self.wait_update_turn(operation)?;
        }
        match request {
            action @ (ServiceRequest::ClientFocus { .. }
            | ServiceRequest::ClientNavigate { .. }
            | ServiceRequest::ClientsOpenWindow { .. }) => self.window_action(owner, action),
            ServiceRequest::ClientsGet { client } => {
                let AgentOwner::Agent(agent) = owner else {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "clients requires service principal",
                    ));
                };
                if self.context()?.worker_bootstrap(agent)?.kind != AgentKind::Service {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "clients requires service principal",
                    ));
                }
                let coordinator = self
                    .context()?
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                let stable = persistence::partition_key(&partition)?;
                let target = coordinator
                    .versions
                    .client(&stable, &origin.authority, &client);
                let snapshot = target.and_then(|(registry, target)| {
                    let context = registry.context().ok()?;
                    let (target_partition, target_origin, _) =
                        context.native_worker_authority(&target).ok()?;
                    if persistence::partition_key(&target_partition)
                        .ok()
                        .as_deref()
                        != Some(stable.as_str())
                        || target_origin.authority != origin.authority
                    {
                        return None;
                    }
                    registry.client_snapshot(&target).ok()
                });
                Ok(ServiceResult::Client(snapshot))
            }
            ServiceRequest::ClientsMatchAll {
                include_uncontrolled,
                client_type,
            } => {
                let AgentOwner::Agent(agent) = owner else {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "clients requires service principal",
                    ));
                };
                let context = self.context()?;
                if context.worker_bootstrap(agent)?.kind != AgentKind::Service {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "clients requires service principal",
                    ));
                }
                let version = self
                    .state()?
                    .versions
                    .get(&agent)
                    .map(|version| version.snapshot.version_id)
                    .ok_or_else(|| error("service client query version retired"))?;
                let coordinator = context
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                let stable = persistence::partition_key(&partition)?;
                let mut clients = Vec::new();
                for registry in coordinator
                    .versions
                    .registries()
                    .map_err(|failure| error(failure.to_string()))?
                {
                    let Ok(candidate_context) = registry.context() else {
                        continue;
                    };
                    for candidate in candidate_context.live_service_clients() {
                        let Ok((target_partition, target_origin, _)) =
                            candidate_context.native_worker_authority(&candidate)
                        else {
                            continue;
                        };
                        if persistence::partition_key(&target_partition)
                            .ok()
                            .as_deref()
                            != Some(stable.as_str())
                            || target_origin.authority != origin.authority
                        {
                            continue;
                        }
                        if !include_uncontrolled {
                            let state = registry.state()?;
                            if !state
                                .clients
                                .get(&candidate)
                                .and_then(|id| state.registrations.get(id))
                                .and_then(|reg| reg.snapshot.active.as_ref())
                                .is_some_and(|active| active.version_id == version)
                            {
                                continue;
                            }
                        }
                        if let Ok(client) = registry.client_snapshot(&candidate)
                            && client_type
                                .as_ref()
                                .is_none_or(|kind| *kind == client.client_type)
                        {
                            clients.push(client);
                        }
                    }
                }
                clients.sort_by(|a, b| b.focused.cmp(&a.focused).then_with(|| a.id.cmp(&b.id)));
                Ok(ServiceResult::Clients(clients))
            }
            ServiceRequest::NavigationPreload {
                registration,
                operation,
                ..
            } => {
                let scope = {
                    let state = self.state()?;
                    let reg = state
                        .registrations
                        .get(&registration)
                        .filter(|reg| Self::visible(reg, &partition, &origin.authority))
                        .ok_or_else(|| error("foreign or retired registration"))?;
                    reg.snapshot.scope.clone()
                };
                let own_activation = {
                    let state = self.state()?;
                    matches!(&owner,AgentOwner::Agent(agent)if state.registrations.get(&registration).and_then(|registration|registration.snapshot.active.as_ref()).is_some_and(|active|active.id==*agent&&active.state==WorkerState::Activating))
                };
                let context = self.context()?;
                let _job = if own_activation {
                    None
                } else {
                    Some(
                        context
                            .context_store()
                            .service_job(
                                &persistence::partition_key(&partition)?,
                                &origin.authority,
                                &scope,
                                &self.retired,
                            )
                            .map_err(|failure| error(failure.to_string()))?,
                    )
                };
                if !own_activation {
                    self.restore_namespace(partition.clone(), origin.clone(), network)?;
                }
                let get_state = matches!(&operation, NavigationPreloadOperation::GetState);
                let value = {
                    let mut state = self.state()?;
                    let reg = state
                        .registrations
                        .get_mut(&registration)
                        .filter(|reg| !reg.unregistered)
                        .ok_or_else(|| error("registration retired"))?;
                    if reg.snapshot.active.is_none() {
                        return Err(error("navigation preload requires an active worker"));
                    }
                    match operation {
                        NavigationPreloadOperation::Enable => {
                            reg.snapshot.navigation_preload.enabled = true
                        }
                        NavigationPreloadOperation::Disable => {
                            reg.snapshot.navigation_preload.enabled = false
                        }
                        NavigationPreloadOperation::SetHeaderValue(value) => {
                            if value.len() > 8192
                                || value.chars().any(|character| u32::from(character) > 255)
                                || value
                                    .bytes()
                                    .any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
                            {
                                return Err(WorkerError::new(
                                    "TypeError",
                                    "invalid navigation preload header value",
                                ));
                            }
                            reg.snapshot.navigation_preload.header_value = value;
                        }
                        NavigationPreloadOperation::GetState => {}
                    }
                    reg.snapshot.navigation_preload.clone()
                };
                if !own_activation && !get_state {
                    self.persist_registration(registration)?;
                }
                Ok(if get_state {
                    ServiceResult::NavigationPreload(value)
                } else {
                    ServiceResult::Done
                })
            }
            ServiceRequest::GetRegistrations { .. } => Ok(ServiceResult::Registrations(
                self.registration_snapshots(&partition, &origin.authority)?,
            )),
            ServiceRequest::GetRegistration { url, .. } => {
                let url = registration_lookup_url(&policy.url, &origin, &url)?;
                let matched = self
                    .registration_snapshots(&partition, &origin.authority)?
                    .into_iter()
                    .filter(|registration| url.as_str().starts_with(&registration.scope))
                    .max_by_key(|registration| registration.scope.len());
                Ok(ServiceResult::Registration(matched))
            }
            ServiceRequest::Unregister { registration, .. } => {
                let scope = {
                    let state = self.state()?;
                    let Some(reg) = state.registrations.get(&registration) else {
                        return Ok(ServiceResult::Bool(false));
                    };
                    if !Self::visible(reg, &partition, &origin.authority) {
                        return Ok(ServiceResult::Bool(false));
                    }
                    reg.snapshot.scope.clone()
                };
                let _profile_job = self
                    .context()?
                    .context_store()
                    .service_job(
                        &persistence::partition_key(&partition)?,
                        &origin.authority,
                        &scope,
                        &self.retired,
                    )
                    .map_err(|failure| error(failure.to_string()))?;
                self.restore_namespace(partition.clone(), origin.clone(), network.clone())?;
                let mut state = self.state()?;
                let Some(reg) = state.registrations.get_mut(&registration) else {
                    return Ok(ServiceResult::Bool(false));
                };
                if !Self::visible(reg, &partition, &origin.authority) {
                    return Ok(ServiceResult::Bool(false));
                }
                reg.unregistered = true;
                // Active controlled clients and admitted events retain their
                // version; idle removed registrations release real processes.
                drop(state);
                self.persist_registration(registration)?;
                self.cleanup_unregistered()?;
                Ok(ServiceResult::Bool(true))
            }
            ServiceRequest::Ready { .. } => loop {
                self.context()?.retained_worker_authority(&owner)?;
                {
                    let state = self.state()?;
                    if let Some(reg) = state
                        .registrations
                        .values()
                        .filter(|reg| {
                            Self::visible(reg, &partition, &origin.authority)
                                && policy.url.starts_with(&reg.snapshot.scope)
                                && reg.snapshot.active.is_some()
                        })
                        .max_by_key(|reg| reg.snapshot.scope.len())
                    {
                        return Ok(ServiceResult::Registration(Some(reg.snapshot.clone())));
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            },
            ServiceRequest::Register {
                url,
                scope,
                script_type,
                update_via_cache,
                ..
            } => self.update_job(ServiceUpdateJobInputs {
                operation,
                owner,
                network,
                partition,
                origin,
                policy,
                script: url,
                scope,
                script_type,
                cache: update_via_cache,
                register: true,
            }),
            ServiceRequest::Update { registration, .. } => {
                let (url, scope, script_type, cache) = {
                    let state = self.state()?;
                    let reg = state
                        .registrations
                        .get(&registration)
                        .filter(|reg| Self::visible(reg, &partition, &origin.authority))
                        .ok_or_else(|| WorkerError::new("SecurityError", "foreign registration"))?;
                    (
                        reg.script_url.clone(),
                        Some(reg.snapshot.scope.clone()),
                        reg.script_type,
                        reg.snapshot.update_via_cache.clone(),
                    )
                };
                self.update(ServiceUpdateInputs {
                    operation,
                    owner,
                    network,
                    partition,
                    origin,
                    policy,
                    script: url,
                    scope,
                    script_type,
                    cache,
                })
            }
            _ => Err(error("invalid service job")),
        }
    }
    pub(crate) fn service_import_revalidation(&self, agent: AgentId) -> Result<bool, WorkerError> {
        self.state()?
            .versions
            .get(&agent)
            .map(|version| version.revalidate_imports)
            .ok_or_else(|| error("service import version retired"))
    }
    pub(crate) fn imported_script(
        &self,
        agent: AgentId,
        url: &str,
        kind: crate::agent_script::ScriptLoadKind,
    ) -> Result<Option<(Vec<u16>, AgentImportMetadata)>, WorkerError> {
        let state = self.state()?;
        let version = state
            .versions
            .get(&agent)
            .ok_or_else(|| error("service version retired"))?;
        if let Some(source) = version.bootstrap.modules.get(url) {
            let metadata = version.bootstrap.import_metadata.get(url).ok_or_else(|| {
                WorkerError::new(
                    "NetworkError",
                    "legacy service import lacks response provenance; registration update required",
                )
                .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Infrastructure)
            })?;
            crate::agent_script::validate_import_metadata(metadata, kind)?;
            return Ok(Some((source.clone(), metadata.clone())));
        }
        if version.snapshot.state != WorkerState::Installing {
            return Err(WorkerError::new(
                "NetworkError",
                "service import was not admitted in its installed resource map",
            )
            .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Fetch));
        }
        Ok(None)
    }
    pub(crate) fn record_import(
        &self,
        agent: AgentId,
        url: String,
        source: Vec<u16>,
        digest: [u8; 32],
        metadata: AgentImportMetadata,
    ) -> Result<(), WorkerError> {
        let mut state = self.state()?;
        let version = state
            .versions
            .get_mut(&agent)
            .ok_or_else(|| error("service version retired"))?;
        if version.snapshot.state != WorkerState::Installing {
            return Err(WorkerError::new(
                "NetworkError",
                "installed service resource graph is immutable",
            )
            .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Infrastructure));
        }
        crate::agent_script::validate_import_metadata(
            &metadata,
            crate::agent_script::ScriptLoadKind::from_import_kind(metadata.kind),
        )?;
        let bytes = version
            .bootstrap
            .modules
            .iter()
            .filter(|(key, _)| *key != &url)
            .fold(0usize, |bytes, (key, source)| {
                bytes
                    .saturating_add(key.len())
                    .saturating_add(source.len().saturating_mul(2))
            });
        if version.bootstrap.modules.len() >= 1024 && !version.bootstrap.modules.contains_key(&url)
            || bytes
                .saturating_add(source.len().saturating_mul(2))
                .saturating_add(url.len())
                > 32 * 1024 * 1024
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "service script resource map limit",
            )
            .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Capacity));
        }
        version
            .bootstrap
            .import_metadata
            .insert(url.clone(), metadata);
        version
            .bootstrap
            .resource_digests
            .insert(url.clone(), digest);
        version.bootstrap.modules.insert(url, source);
        Ok(())
    }
    fn update(&self, params: ServiceUpdateInputs) -> Result<ServiceResult, WorkerError> {
        let ServiceUpdateInputs {
            operation,
            owner,
            network,
            partition,
            origin,
            policy,
            script,
            scope,
            script_type,
            cache,
        } = params;

        self.update_job(ServiceUpdateJobInputs {
            operation,
            owner,
            network,
            partition,
            origin,
            policy,
            script,
            scope,
            script_type,
            cache,
            register: false,
        })
    }
    fn update_job(&self, params: ServiceUpdateJobInputs) -> Result<ServiceResult, WorkerError> {
        let ServiceUpdateJobInputs {
            operation,
            owner,
            network,
            partition,
            origin,
            policy,
            script,
            scope,
            script_type,
            cache,
            register,
        } = params;

        if !matches!(cache.as_str(), "imports" | "all" | "none") {
            return Err(WorkerError::new("TypeError", "invalid updateViaCache"));
        }
        let base = url::Url::parse(&policy.url)
            .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
        let mut url = base
            .join(&script)
            .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
        url.set_fragment(None);
        let mut scope = match scope {
            Some(scope) => base.join(&scope),
            None => url.join("./"),
        }
        .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
        scope.set_fragment(None);
        for value in [&url, &scope] {
            validate_service_registration_url(&base, value)?;
        }
        // Updates are serialized in this context. Script loading and event waits
        // hold this job lock only, never the process/message/page registries.
        let context = self.context()?;
        // An admitted service job belongs to its profile queue, independently
        // of the originating document's promise. Raw internal callers still
        // require current authority and cannot borrow another job's snapshot.
        let admitted = self
            .state()?
            .equivalent_jobs
            .get(&operation)
            .filter(|job| job.authority.owner == owner)
            .map(|job| job.authority.clone());
        let retained_job = admitted.is_some();
        let authority = match admitted {
            Some(authority) => authority,
            None => Self::capture_job_authority(&context, &owner)?,
        };
        let cookie_ancestor_urls = authority.cookie_ancestor_urls;
        let cookie_top_level_url = authority.cookie_top_level_url;
        let _profile_job = context
            .context_store()
            .service_job(
                &persistence::partition_key(&partition)?,
                &origin.authority,
                scope.as_str(),
                &self.retired,
            )
            .map_err(|failure| error(failure.to_string()))?;
        self.restore_namespace(partition.clone(), origin.clone(), network.clone())?;
        if !retained_job {
            context.retained_worker_authority(&owner)?;
        }
        if register {
            let equivalent = {
                let mut state = self.state()?;
                state
                    .registrations
                    .values_mut()
                    .find(|reg| {
                        Self::visible(reg, &partition, &origin.authority)
                            && reg.snapshot.scope == scope.as_str()
                            && reg.script_url == url.as_str()
                            && reg.script_type == script_type
                            && (reg.snapshot.waiting.is_some() || reg.snapshot.active.is_some())
                    })
                    .map(|reg| {
                        reg.snapshot.update_via_cache = cache.clone();
                        reg.snapshot.id
                    })
            };
            if let Some(id) = equivalent {
                self.persist_registration(id)?;
                return Ok(ServiceResult::Registration(
                    self.state()?
                        .registrations
                        .get(&id)
                        .map(|reg| reg.snapshot.clone()),
                ));
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| error("service update clock predates epoch"))?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let last_check = self
            .state()?
            .registrations
            .values()
            .find(|reg| {
                !reg.unregistered
                    && reg.partition == partition
                    && reg.origin == origin.authority
                    && reg.snapshot.scope == scope.as_str()
            })
            .map(|reg| reg.last_update_check_ms)
            .unwrap_or(now);
        let aged = Self::registration_is_stale(last_check, now);
        let report = |policy: &crate::csp::CspPolicySet,
                      violations: Vec<crate::csp::CspViolation>| {
            if self.retired.load(std::sync::atomic::Ordering::Acquire) {
                return Err(WorkerError::new(
                    "AbortError",
                    "service job retired before CSP report",
                ));
            }
            context.report_worker_csp(
                &owner,
                policy,
                violations,
                cookie_top_level_url
                    .as_deref()
                    .unwrap_or(&policy.worker_snapshot().url),
                retained_job,
            )
        };
        let coep_report = |reports| {
            if !self.retired.load(std::sync::atomic::Ordering::Acquire) {
                let _ = context.deliver_coep_reports(
                    cookie_top_level_url.as_deref().unwrap_or(&policy.url),
                    reports,
                );
            }
        };
        let hash_report = |reports| {
            if let AgentOwner::Window { page, .. } = &owner
                && !self.retired.load(std::sync::atomic::Ordering::Acquire)
                && context.retained_worker_authority(&owner).is_ok()
            {
                context.deliver_script_hash_reports(
                    Some(*page),
                    cookie_top_level_url.as_deref().unwrap_or(&policy.url),
                    reports,
                );
            }
        };
        let revalidate_main = aged || cache != "all";
        let revalidate_imports = aged || cache == "none";
        let loaded = crate::agent_script::load_service(
            &network,
            crate::agent_script::ScriptAuthority {
                hash_report: matches!(&owner, AgentOwner::Window { .. })
                    .then_some(&hash_report as &dyn Fn(Vec<crate::csp::ScriptHashReport>)),
                worker_destination: "serviceworker",
                coep_report: Some(&coep_report),
                report: Some(&report),
                cookie_ancestor_urls: &cookie_ancestor_urls,
                referrer_source: None,
                origin: &origin,
                policy: &policy,
                script_url: &policy.url,
                referrer_policy: "strict-origin-when-cross-origin",
                top_level_url: cookie_top_level_url.as_deref(),
                credentials: WorkerCredentials::SameOrigin,
            },
            url.as_str(),
            if script_type == ScriptType::Module {
                crate::agent_script::ScriptLoadKind::EntryModule
            } else {
                crate::agent_script::ScriptLoadKind::EntryClassic
            },
            &self.retired,
            revalidate_main,
        )?;
        // The authorized loader retains Service-Worker-Allowed separately from
        // executable policy; a response cannot widen another origin's scope.
        let allowed = loaded
            .service_worker_allowed
            .as_deref()
            .map(|value| url.join(value))
            .transpose()
            .map_err(|failure| WorkerError::new("SecurityError", failure.to_string()))?
            .unwrap_or(
                url.join("./")
                    .map_err(|failure| error(failure.to_string()))?,
            );
        if allowed.origin() != url.origin()
            || !allowed.username().is_empty()
            || allowed.password().is_some()
            || !scope.path().starts_with(allowed.path())
        {
            return Err(WorkerError::new(
                "SecurityError",
                "scope exceeds Service-Worker-Allowed",
            ));
        }
        if !retained_job {
            context.retained_worker_authority(&owner)?;
        }
        let previous_resources = {
            let state = self.state()?;
            state
                .registrations
                .values()
                .find(|reg| {
                    !reg.unregistered
                        && reg.partition == partition
                        && reg.origin == origin.authority
                        && reg.snapshot.scope == scope.as_str()
                })
                .and_then(|reg| {
                    reg.snapshot
                        .waiting
                        .as_ref()
                        .or(reg.snapshot.active.as_ref())
                })
                .and_then(|worker| Self::version_bootstrap(&state, worker))
                .map(|bootstrap| bootstrap.modules.clone())
                .unwrap_or_default()
        };
        let previous_digests = {
            let state = self.state()?;
            state
                .registrations
                .values()
                .find(|reg| {
                    !reg.unregistered
                        && reg.partition == partition
                        && reg.origin == origin.authority
                        && reg.snapshot.scope == scope.as_str()
                })
                .and_then(|reg| {
                    reg.snapshot
                        .waiting
                        .as_ref()
                        .or(reg.snapshot.active.as_ref())
                })
                .and_then(|worker| Self::version_bootstrap(&state, worker))
                .map(|bootstrap| bootstrap.resource_digests.clone())
                .unwrap_or_default()
        };
        let previous_metadata = {
            let state = self.state()?;
            state
                .registrations
                .values()
                .find(|reg| {
                    !reg.unregistered
                        && reg.partition == partition
                        && reg.origin == origin.authority
                        && reg.snapshot.scope == scope.as_str()
                })
                .and_then(|reg| {
                    reg.snapshot
                        .waiting
                        .as_ref()
                        .or(reg.snapshot.active.as_ref())
                })
                .and_then(|worker| Self::version_bootstrap(&state, worker))
                .map(|bootstrap| bootstrap.import_metadata.clone())
                .unwrap_or_default()
        };
        let mut import_metadata = BTreeMap::new();
        let mut resources = BTreeMap::new();
        let mut resource_digests = BTreeMap::from([(loaded.url.clone(), loaded.digest)]);
        let mut resources_equal = previous_digests.get(&loaded.url) == Some(&loaded.digest);
        for (resource, previous) in &previous_resources {
            let old_metadata = previous_metadata.get(resource);
            let kind = old_metadata
                .map(|metadata| {
                    crate::agent_script::ScriptLoadKind::from_import_kind(metadata.kind)
                })
                .unwrap_or(if script_type == ScriptType::Module {
                    crate::agent_script::ScriptLoadKind::ModuleImport
                } else {
                    crate::agent_script::ScriptLoadKind::ClassicImport
                });
            let request_source =
                old_metadata.and_then(|metadata| metadata.request_referrer_source.as_deref());
            let request_policy = old_metadata
                .map(|metadata| metadata.request_referrer_policy.as_str())
                .unwrap_or(&loaded.referrer_policy);
            let imported = crate::agent_script::load_service(
                &network,
                crate::agent_script::ScriptAuthority {
                    hash_report: None,
                    worker_destination: "serviceworker",
                    coep_report: Some(&coep_report),
                    report: Some(&report),
                    cookie_ancestor_urls: &cookie_ancestor_urls,
                    referrer_source: request_source,
                    origin: &origin,
                    policy: &loaded.policy,
                    script_url: &loaded.url,
                    referrer_policy: request_policy,
                    top_level_url: cookie_top_level_url.as_deref(),
                    credentials: WorkerCredentials::SameOrigin,
                },
                resource,
                kind,
                &self.retired,
                revalidate_imports,
            )?;
            let metadata = imported.import_metadata(kind, request_source, request_policy);
            resources_equal &= previous == &imported.source
                && previous_digests.get(resource) == Some(&imported.digest)
                && old_metadata == Some(&metadata);
            import_metadata.insert(resource.clone(), metadata);
            resource_digests.insert(resource.clone(), imported.digest);
            resources.insert(resource.clone(), imported.source);
        }
        let mut unchanged = false;
        let registration = {
            let mut state = self.state()?;
            let existing = state
                .registrations
                .iter()
                .find(|(_, reg)| {
                    !reg.unregistered
                        && reg.partition == partition
                        && reg.origin == origin.authority
                        && reg.snapshot.scope == scope.as_str()
                })
                .map(|(id, _)| *id);
            if let Some(id) = existing {
                let reg = state.registrations.get_mut(&id).unwrap();
                if revalidate_main {
                    reg.last_update_check_ms = now;
                }
                reg.unregistered = false;
                if reg.script_url == loaded.url
                    && reg.script_type == script_type
                    && reg.source == loaded.source
                    && resources_equal
                {
                    reg.snapshot.update_via_cache = cache.clone();
                    unchanged = true;
                }
                id
            } else {
                if state.registrations.len() >= 128 {
                    return Err(WorkerError::new(
                        "QuotaExceededError",
                        "service registration capacity",
                    ));
                }
                let id = RegistrationId(self.id()?);
                state.registrations.insert(
                    id,
                    Registration {
                        lineage: registration_lineage()?,
                        durable_fingerprint: None,
                        last_update_check_ms: now,
                        snapshot: RegistrationSnapshot {
                            navigation_preload: Default::default(),
                            id,
                            scope: scope.to_string(),
                            update_via_cache: cache.clone(),
                            installing: None,
                            waiting: None,
                            active: None,
                        },
                        partition: partition.clone(),
                        origin: origin.authority.clone(),
                        script_url: loaded.url.clone(),
                        script_type,
                        source: Vec::new(),
                        unregistered: false,
                    },
                );
                id
            }
        };
        if unchanged {
            self.persist_registration(registration)?;
            return Ok(ServiceResult::Registration(
                self.state()?
                    .registrations
                    .get(&registration)
                    .map(|registration| registration.snapshot.clone()),
            ));
        }
        let bootstrap = AgentBootstrap {
            cookie_ancestor_urls,
            import_metadata,
            service_skip_waiting: false,
            resource_digests,
            secure_context: true,
            service_registration: Some(registration),
            service_controller: None,
            id: AgentId(0),
            kind: AgentKind::Service,
            partition,
            origin,
            script_url: loaded.url,
            source: loaded.source.clone(),
            modules: resources,
            options: WorkerOptions {
                name: Vec::new(),
                script_type,
                credentials: WorkerCredentials::SameOrigin,
                audio_sample_rate: None,
            },
            referrer_policy: loaded.referrer_policy,
            limits: context.agent_limits(),
            policy: loaded.policy,
            top_level_url: cookie_top_level_url,
        };
        let version_id = service_version_identity()?;
        let coordinator = context
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        let (scope, lineage) = {
            let state = self.state()?;
            let registration = &state.registrations[&registration];
            (
                registration.snapshot.scope.clone(),
                registration.lineage.clone(),
            )
        };
        let registry = self
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service registry retired before version admission"))?;
        let crate::storage::ServiceVersionClaim::Reserved(directory) = coordinator
            .versions
            .claim(
                &persistence::partition_key(&bootstrap.partition)?,
                &bootstrap.origin.authority,
                &scope,
                &lineage,
                version_id,
                &registry,
            )
            .map_err(|failure| error(failure.to_string()))?
        else {
            return Err(error("new service version identity collision"));
        };
        let directory = Arc::new(directory);
        let (agent, admission) =
            crate::agent_service::launch_service_agent(&context, bootstrap.clone(), network)?;
        let snapshot = WorkerSnapshot {
            version_id,
            id: agent,
            script_url: bootstrap.script_url.clone(),
            state: WorkerState::Installing,
        };
        {
            let mut state = self.state()?;
            let mut bootstrap = bootstrap;
            bootstrap.id = agent;
            let skip_waiting = state.early_skip.remove(&agent);
            bootstrap.service_skip_waiting = skip_waiting;
            state.versions.insert(
                agent,
                Version {
                    directory: Some(directory),
                    retiring: false,
                    revalidate_imports,
                    snapshot: snapshot.clone(),
                    bootstrap,
                    serial: Arc::new(Mutex::new(())),
                    inbox: Arc::new(Mutex::new(ServiceControlInbox::default())),
                    inflight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    skip_waiting,
                },
            );
            state
                .registrations
                .get_mut(&registration)
                .unwrap()
                .snapshot
                .installing = Some(snapshot);
        }
        {
            let snapshot = self
                .state()?
                .registrations
                .get(&registration)
                .unwrap()
                .snapshot
                .clone();
            self.publish_job_result(operation, Ok(ServiceResult::Registration(Some(snapshot))));
        }
        if let Err(failure) = self.publish_version(agent) {
            self.fail_installation(registration, agent)?;
            drop(_profile_job);
            self.cleanup_unregistered()?;
            return Err(failure);
        }
        admission.publish();
        if let Err(failure) = self.lifecycle(agent, LifecycleEvent::Install) {
            self.fail_installation(registration, agent)?;
            drop(_profile_job);
            self.cleanup_unregistered()?;
            return Err(failure);
        }
        let previous_waiting = {
            let mut state = self.state()?;
            let version = state
                .versions
                .get_mut(&agent)
                .ok_or_else(|| error("installing service retired"))?;
            version.snapshot.state = WorkerState::Installed;
            let snapshot = version.snapshot.clone();
            let reg = state.registrations.get_mut(&registration).unwrap();
            reg.source = loaded.source;
            reg.script_url = snapshot.script_url.clone();
            reg.script_type = script_type;
            reg.snapshot.update_via_cache = cache;
            reg.snapshot.installing = None;
            let previous = reg.snapshot.waiting.replace(snapshot);
            state.dirty_registrations.insert(registration);
            previous
        };
        self.publish_version(agent)?;
        if let Some(previous) = previous_waiting {
            if let Some((owner, current)) = self.remote_version(&previous)? {
                owner.retire_version(current.id)?;
            } else {
                self.retire_version(previous.id)?;
            }
        }
        self.persist_registration(registration)?;
        drop(_profile_job);
        self.try_activate(registration)?;
        Ok(ServiceResult::Registration(
            self.state()?
                .registrations
                .get(&registration)
                .map(|reg| reg.snapshot.clone()),
        ))
    }
    fn fail_installation(
        &self,
        registration: RegistrationId,
        agent: AgentId,
    ) -> Result<(), WorkerError> {
        {
            let mut state = self.state()?;
            let reg = state
                .registrations
                .get_mut(&registration)
                .ok_or_else(|| error("failed installation registration retired"))?;
            if reg
                .snapshot
                .installing
                .as_ref()
                .is_some_and(|worker| worker.id == agent)
            {
                reg.snapshot.installing = None;
            }
            if reg.snapshot.active.is_none() && reg.snapshot.waiting.is_none() {
                reg.unregistered = true;
            }
        }
        self.retire_version(agent)?;
        self.persist_registration(registration)
    }
    fn principal(owner: &AgentOwner) -> oryn_js::message_wire::Principal {
        match owner {
            AgentOwner::Window { page, .. } => oryn_js::message_wire::Principal::Window(page.0),
            AgentOwner::Agent(id) => oryn_js::message_wire::Principal::Agent(id.0),
        }
    }
    fn consume_profile_interaction(&self, agent: AgentId) -> Result<(), WorkerError> {
        let context = self.context()?;
        let bootstrap = context.worker_bootstrap(agent)?;
        if bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "SecurityError",
                "interaction requires service principal",
            ));
        }
        let stable = persistence::partition_key(&bootstrap.partition)?;
        let coordinator = context
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        for registry in coordinator
            .versions
            .registries()
            .map_err(|failure| error(failure.to_string()))?
        {
            if let Ok(target) = registry.context() {
                match target
                    .consume_native_service_interaction(&stable, &bootstrap.origin.authority)
                {
                    Ok(()) => return Ok(()),
                    Err(failure) if failure.name == "InvalidAccessError" => {}
                    Err(failure) => return Err(failure),
                }
            }
        }
        Err(WorkerError::new(
            "InvalidAccessError",
            "service interaction requires unconsumed native activation",
        ))
    }
    fn window_action(
        &self,
        source: AgentOwner,
        request: ServiceRequest,
    ) -> Result<ServiceResult, WorkerError> {
        use crate::host::webdriver_script::{FrameCommand, FrameResponse};
        let AgentOwner::Agent(agent) = source else {
            return Err(WorkerError::new(
                "SecurityError",
                "client actions require service principal",
            ));
        };
        let context = self.context()?;
        let bootstrap = context.worker_bootstrap(agent)?;
        if bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "SecurityError",
                "client actions require service version",
            ));
        }
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|failure| error(failure.to_string()))?;
        let mut opened_window: Option<ServiceOpenedWindow> = None;
        let find = |page: &crate::runtime::PageHandle,
                    target: &oryn_js::message_wire::WindowTarget,
                    generation: u64,
                    epoch: u64|
         -> Result<
            crate::host::webdriver_script::AutomationRealmSnapshot,
            WorkerError,
        > {
            let FrameResponse::Inventory(inventory) = executor
                .block_on(page.webdriver_frames(FrameCommand::Inventory))
                .map_err(|failure| error(failure.to_string()))?
            else {
                return Err(error("native client inventory response mismatch"));
            };
            inventory
                .into_iter()
                .find(|entry| {
                    entry.live
                        && entry.document_generation == generation
                        && entry.document_authority_epoch == epoch
                        && match target {
                            oryn_js::message_wire::WindowTarget::Root => {
                                entry.parent_realm.is_none()
                            }
                            oryn_js::message_wire::WindowTarget::Frame(token) => {
                                entry.frame_token.as_ref() == Some(token)
                            }
                        }
                })
                .ok_or_else(|| error("native client document retired"))
        };
        let (client_registry, client_context, page, selected, navigate) = match request {
            ServiceRequest::ClientsOpenWindow { url } => {
                let url = url::Url::parse(&bootstrap.script_url)
                    .and_then(|base| base.join(&url))
                    .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
                // The Service Worker algorithm rejects about:blank even though
                // ordinary Window navigation supports it. Other about URLs have
                // no native local document handler.
                if !matches!(url.scheme(), "http" | "https" | "data" | "blob") {
                    return Err(WorkerError::new(
                        "TypeError",
                        "unsupported service openWindow URL",
                    ));
                }
                if !context.popups_allowed()
                    || crate::csp::CspPolicySet::from_agent_policy(&bootstrap.policy)
                        .sandbox()
                        .is_some_and(|sandbox| !sandbox.allow_popups)
                {
                    return Err(WorkerError::new(
                        "InvalidAccessError",
                        "native popup policy denied service window",
                    ));
                }
                self.consume_profile_interaction(agent)?;
                let page = context
                    .new_service_window(agent)
                    .map_err(|failure| error(failure.to_string()))?;
                opened_window = Some(ServiceOpenedWindow {
                    context: context.clone(),
                    page: page.page_id(),
                    keep: false,
                });
                let opened: Result<_, WorkerError> = (|| {
                    context.worker_bootstrap(agent).map_err(|failure| {
                        service_window_error(
                            failure,
                            "check-opening-source",
                            agent,
                            page.page_id(),
                            None,
                        )
                    })?;
                    let target = service_window_stage(
                        "capture-initial-target",
                        agent,
                        page.page_id(),
                        None,
                        || {
                            context
                                .native_initial_document_creator(page.page_id())
                                .map_err(|failure| error(failure.to_string()))
                        },
                    )?;
                    let document = match &target {
                        Creator::Window {
                            document_generation,
                            stream_epoch,
                            ..
                        } => Some((*document_generation, *stream_epoch)),
                        Creator::Agent(_) => None,
                    };
                    let admission = service_window_stage(
                        "admit-navigation",
                        agent,
                        page.page_id(),
                        document,
                        || {
                            context
                                .issue_service_navigation_authority(
                                    agent,
                                    page.page_id(),
                                    target,
                                    url.as_str(),
                                )
                                .map_err(|failure| error(failure.to_string()))
                        },
                    )?;
                    service_window_stage(
                        "navigate-initial-window",
                        agent,
                        page.page_id(),
                        document,
                        || {
                            executor
                                .block_on(page.goto_service_resource(url.to_string(), admission))
                                .map_err(|failure| error(failure.to_string()))
                        },
                    )?;
                    context.worker_bootstrap(agent).map_err(|failure| {
                        service_window_error(
                            failure,
                            "check-navigated-source",
                            agent,
                            page.page_id(),
                            None,
                        )
                    })?;
                    service_window_stage("activate-window", agent, page.page_id(), None, || {
                        context
                            .activate_page(page.page_id())
                            .map_err(|failure| error(failure.to_string()))
                    })?;
                    let selected = service_window_stage(
                        "inventory-opened-window",
                        agent,
                        page.page_id(),
                        None,
                        || {
                            let FrameResponse::Inventory(inventory) = executor
                                .block_on(page.webdriver_frames(FrameCommand::Inventory))
                                .map_err(|failure| error(failure.to_string()))?
                            else {
                                return Err(error("native window inventory mismatch"));
                            };
                            inventory
                                .into_iter()
                                .find(|entry| entry.live && entry.parent_realm.is_none())
                                .ok_or_else(|| error("opened document unavailable"))
                        },
                    )?;
                    let registry = self
                        .weak_self
                        .upgrade()
                        .ok_or_else(|| error("service registry retired"))?;
                    Ok((registry, selected))
                })();
                let (registry, selected) = match opened {
                    Ok(value) => value,
                    Err(failure) => {
                        // Logical revoke plus retained physical cleanup owns failed
                        // activation, navigation and inventory outcomes alike.
                        if let Err(cleanup) = context.queue_script_close(page.page_id()) {
                            let mut failure = failure;
                            failure
                                .message
                                .push_str(&format!("; retained service window cleanup: {cleanup}"));
                            return Err(service_window_error(
                                failure,
                                "cleanup-failed-window",
                                agent,
                                page.page_id(),
                                None,
                            ));
                        }
                        return Err(failure);
                    }
                };
                (registry, context.clone(), page, selected, None)
            }
            action => {
                let client = match &action {
                    ServiceRequest::ClientFocus { client }
                    | ServiceRequest::ClientNavigate { client, .. } => client,
                    _ => return Err(error("invalid window client action")),
                };
                let coordinator = context
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                let (client_registry, owner) = coordinator
                    .versions
                    .client(
                        &persistence::partition_key(&bootstrap.partition)?,
                        &bootstrap.origin.authority,
                        client,
                    )
                    .ok_or_else(|| error("unknown window client"))?;
                let client_context = client_registry.context()?;
                let (partition, origin, _) = client_context.native_worker_authority(&owner)?;
                if persistence::partition_key(&partition)?
                    != persistence::partition_key(&bootstrap.partition)?
                    || origin.authority != bootstrap.origin.authority
                {
                    return Err(WorkerError::new("SecurityError", "foreign window client"));
                }
                let AgentOwner::Window {
                    target,
                    generation,
                    epoch,
                    ..
                } = &owner
                else {
                    return Err(WorkerError::new("TypeError", "client is not a Window"));
                };
                let page = client_context.service_window_handle(&owner)?;
                let selected = find(&page, target, *generation, *epoch)?;
                match action {
                    ServiceRequest::ClientFocus { .. } => {
                        self.consume_profile_interaction(agent)?;
                        client_context
                            .activate_page(page.page_id())
                            .map_err(|failure| error(failure.to_string()))?;
                        executor
                            .block_on(page.webdriver_frames(FrameCommand::FocusWindow {
                                current: selected.clone(),
                            }))
                            .map_err(|failure| error(failure.to_string()))?;
                        (client_registry, client_context, page, selected, None)
                    }
                    ServiceRequest::ClientNavigate { url, .. } => {
                        let version = self
                            .state()?
                            .versions
                            .get(&agent)
                            .map(|version| version.snapshot.version_id)
                            .ok_or_else(|| error("service version retired"))?;
                        let controlled = {
                            let state = client_registry.state()?;
                            state
                                .clients
                                .get(&owner)
                                .and_then(|id| state.registrations.get(id))
                                .and_then(|registration| registration.snapshot.active.as_ref())
                                .is_some_and(|active| active.version_id == version)
                        };
                        if !controlled {
                            return Err(WorkerError::new(
                                "TypeError",
                                "service worker does not control navigation client",
                            ));
                        }
                        let url = url::Url::parse(&bootstrap.script_url)
                            .and_then(|base| base.join(&url))
                            .map_err(|failure| {
                                WorkerError::new("TypeError", failure.to_string())
                            })?;
                        if !matches!(url.scheme(), "http" | "https" | "data" | "blob") {
                            return Err(WorkerError::new(
                                "TypeError",
                                "unsupported WindowClient navigation URL",
                            ));
                        }
                        (
                            client_registry,
                            client_context,
                            page,
                            selected,
                            Some(url.to_string()),
                        )
                    }
                    _ => unreachable!(),
                }
            }
        };
        let selected = if let Some(url) = navigate {
            let target = Creator::Window {
                target: selected
                    .frame_token
                    .clone()
                    .map(oryn_js::message_wire::WindowTarget::Frame)
                    .unwrap_or(oryn_js::message_wire::WindowTarget::Root),
                document_generation: selected.document_generation,
                stream_epoch: selected.document_authority_epoch,
            };
            let document = Some((
                selected.document_generation,
                selected.document_authority_epoch,
            ));
            let admission = service_window_stage(
                "admit-controlled-navigation",
                agent,
                page.page_id(),
                document,
                || {
                    client_context
                        .issue_service_navigation_from(
                            &context,
                            agent,
                            page.page_id(),
                            target,
                            &url,
                        )
                        .map_err(|failure| error(failure.to_string()))
                },
            )?;
            if selected.parent_realm.is_none() {
                service_window_stage(
                    "navigate-controlled-root",
                    agent,
                    page.page_id(),
                    document,
                    || {
                        executor
                            .block_on(page.goto_service_resource(url, admission))
                            .map_err(|failure| error(failure.to_string()))
                    },
                )?;
                service_window_stage(
                    "inventory-navigated-root",
                    agent,
                    page.page_id(),
                    None,
                    || {
                        let FrameResponse::Inventory(inventory) = executor
                            .block_on(page.webdriver_frames(FrameCommand::Inventory))
                            .map_err(|failure| error(failure.to_string()))?
                        else {
                            return Err(error("navigation inventory mismatch"));
                        };
                        inventory
                            .into_iter()
                            .find(|entry| entry.live && entry.parent_realm.is_none())
                            .ok_or_else(|| error("navigated document unavailable"))
                    },
                )?
            } else {
                service_window_stage(
                    "navigate-controlled-frame",
                    agent,
                    page.page_id(),
                    document,
                    || {
                        let FrameResponse::Selected(next) = executor
                            .block_on(page.webdriver_frames(FrameCommand::NavigateServiceWindow {
                                current: selected,
                                url,
                                authority: admission,
                            }))
                            .map_err(|failure| error(failure.to_string()))?
                        else {
                            return Err(error("frame navigation response mismatch"));
                        };
                        Ok(next)
                    },
                )?
            }
        } else {
            selected
        };
        let target = selected
            .frame_token
            .clone()
            .map(oryn_js::message_wire::WindowTarget::Frame)
            .unwrap_or(oryn_js::message_wire::WindowTarget::Root);
        if matches!(target, oryn_js::message_wire::WindowTarget::Root) {
            client_context
                .handle_message_wire(
                    page.page_id(),
                    oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: selected.document_generation,
                        stream_epoch: selected.document_authority_epoch,
                    },
                )
                .map_err(error)?;
        }
        let owner = AgentOwner::Window {
            page: page.page_id(),
            target,
            generation: selected.document_generation,
            epoch: selected.document_authority_epoch,
        };
        let (partition, origin, _) = client_context.native_worker_authority(&owner)?;
        let client = if persistence::partition_key(&partition)?
            != persistence::partition_key(&bootstrap.partition)?
            || origin.authority != bootstrap.origin.authority
        {
            None
        } else {
            Some(client_registry.client_snapshot(&owner)?)
        };
        if let Some(guard) = opened_window.as_mut() {
            guard.keep = true;
        }
        Ok(ServiceResult::WindowClient(client))
    }

    fn claim_profile_clients(
        &self,
        source: &ServiceRegistry,
        registration: &Registration,
        version: &Version,
    ) -> Result<(), WorkerError> {
        let serialization = self
            .persistence
            .lock()
            .map_err(|_| error("service claim persistence lock poisoned"))?;
        let context = self.context()?;
        let stable = persistence::partition_key(&registration.partition)?;
        let mut local_registration = None;
        for client in context.live_service_clients() {
            let Ok((partition, origin, policy)) = context.native_worker_authority(&client) else {
                continue;
            };
            if persistence::partition_key(&partition)? != stable
                || origin.authority != registration.origin
                || !policy.url.starts_with(&registration.snapshot.scope)
            {
                continue;
            }
            let id = if let Some(id) = local_registration {
                id
            } else {
                let id = self.observed_registration_id(
                    &partition,
                    &origin.authority,
                    &registration.lineage,
                )?;
                let mut state = self.state()?;
                if state.registrations.len() >= 128 && !state.registrations.contains_key(&id) {
                    return Err(WorkerError::new(
                        "QuotaExceededError",
                        "service registration capacity",
                    ));
                }
                // The source version has been admitted by this profile's native
                // registry. Copy only its registration view; execution remains
                // in the original agent and keeps its own process principal.
                let mut view = state
                    .registrations
                    .get(&id)
                    .filter(|existing| existing.lineage == registration.lineage)
                    .cloned()
                    .unwrap_or_else(|| registration.clone());
                view.partition = partition.clone();
                view.snapshot.id = id;
                view.snapshot.active = Some(version.snapshot.clone());
                state.registrations.insert(id, view);
                if !std::ptr::eq(self, source) {
                    let mut bootstrap = version.bootstrap.clone();
                    bootstrap.partition = partition;
                    bootstrap.service_registration = Some(id);
                    state.remote_versions.insert(
                        version.snapshot.version_id,
                        RemoteVersion {
                            owner: source.weak_self.clone(),
                            snapshot: version.snapshot.clone(),
                            bootstrap,
                        },
                    );
                }
                local_registration = Some(id);
                id
            };
            self.control_client(client, Some(id))?;
        }
        drop(serialization);
        self.wake_clients();
        Ok(())
    }

    fn client_snapshot(&self, owner: &AgentOwner) -> Result<ClientSnapshot, WorkerError> {
        let context = self.context()?;
        let (partition, origin, policy) = context.native_worker_authority(owner)?;
        let (url, client_type, frame_type, focused) = match owner {
            AgentOwner::Window { page, target, .. } => {
                let focused = context
                    .active_page()
                    .is_some_and(|active| active.page_id() == *page);
                let url = if matches!(target, oryn_js::message_wire::WindowTarget::Root) {
                    context
                        .page_url(*page)
                        .map_err(|failure| error(failure.to_string()))?
                } else {
                    policy.url
                };
                (
                    url,
                    ClientType::Window,
                    if matches!(target, oryn_js::message_wire::WindowTarget::Root) {
                        "top-level"
                    } else {
                        "nested"
                    }
                    .to_owned(),
                    focused,
                )
            }
            AgentOwner::Agent(id) => {
                let bootstrap = context.worker_bootstrap(*id)?;
                (
                    bootstrap.script_url,
                    if bootstrap.kind == AgentKind::Shared {
                        ClientType::SharedWorker
                    } else {
                        ClientType::DedicatedWorker
                    },
                    "none".into(),
                    false,
                )
            }
        };
        let mut state = self.state()?;
        let id = match state.client_ids.get(owner) {
            Some(id) => id.clone(),
            None => {
                let mut random = [0u8; 32];
                getrandom::fill(&mut random).map_err(|failure| error(failure.to_string()))?;
                let id = random
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                state.client_ids.insert(owner.clone(), id.clone());
                id
            }
        };
        drop(state);
        let coordinator = context
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        let registry = self
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service client registry retired"))?;
        coordinator
            .versions
            .publish_client(
                &persistence::partition_key(&partition)?,
                &origin.authority,
                &id,
                &registry,
                owner,
            )
            .map_err(|failure| error(failure.to_string()))?;
        Ok(ClientSnapshot {
            id,
            url,
            client_type,
            frame_type,
            focused,
            visibility_state: if focused { "visible" } else { "hidden" }.into(),
        })
    }
    fn source_snapshot(&self, owner: &AgentOwner) -> Result<MessageSource, WorkerError> {
        let context = self.context()?;
        let (_, origin, _) = context.native_worker_authority(owner)?;
        if let AgentOwner::Agent(id) = owner
            && context.worker_bootstrap(*id)?.kind == AgentKind::Service
        {
            let worker = self
                .state()?
                .versions
                .get(id)
                .map(|version| version.snapshot.clone())
                .ok_or_else(|| error("service source version unavailable"))?;
            return Ok(MessageSource::ServiceWorker {
                worker,
                origin: origin.serialized,
            });
        }
        Ok(MessageSource::Client {
            client: self.client_snapshot(owner)?,
            origin: origin.serialized,
        })
    }
    fn post_message(
        self: &Arc<Self>,
        owner: AgentOwner,
        worker: ServiceVersionId,
        value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        value.validate().map_err(error)?;
        let context = self.context()?;
        let (partition, origin, _) = context.native_worker_authority(&owner)?;
        let local = {
            let state = self.state()?;
            state
                .versions
                .iter()
                .find(|(_, version)| {
                    version.snapshot.version_id == worker
                        && version.bootstrap.partition == partition
                        && version.bootstrap.origin.authority == origin.authority
                })
                .map(|(agent, _)| *agent)
        };
        let worker = match local {
            Some(agent) => agent,
            None => {
                let coordinator = context
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                let target = coordinator
                    .versions
                    .lookup(
                        &persistence::partition_key(&partition)?,
                        &origin.authority,
                        worker,
                    )
                    .ok_or_else(|| {
                        WorkerError::new(
                            "InvalidStateError",
                            "service version is retired or foreign",
                        )
                    })?;
                return target.post_profile_message(self, owner, worker, value);
            }
        };
        let target = context.worker_bootstrap(worker)?;
        if target.kind != AgentKind::Service
            || target.partition != partition
            || target.origin.authority != origin.authority
        {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign service message recipient",
            ));
        }
        let source = self.source_snapshot(&owner)?;
        let id = self.id()?;
        let bytes = value.retained_bytes();
        self.message_jobs
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 256).then_some(count + 1)
            })
            .map_err(|_| WorkerError::new("QuotaExceededError", "service message job capacity"))?;
        if self
            .message_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |retained| {
                retained
                    .checked_add(bytes)
                    .filter(|total| *total <= 32 * 1024 * 1024)
            })
            .is_err()
        {
            self.message_jobs.fetch_sub(1, Ordering::AcqRel);
            return Err(WorkerError::new(
                "QuotaExceededError",
                "service message bytes",
            ));
        }
        let retained = value.cleanup_clone();
        let failed_admission = retained.clone();
        let registry = self.clone();
        let cleanup_owner = owner.clone();
        let job = std::thread::Builder::new()
            .name(format!("oryn-service-message-{id}"))
            .spawn(move || {
                let _budget = MessageJobBudget {
                    registry: registry.clone(),
                    bytes,
                };
                if registry
                    .dispatch_from(
                        worker,
                        AgentCommand::ServiceMessage { id, source, value },
                        id,
                        Some(&owner),
                    )
                    .is_err()
                    && let Some(context) = registry.context.upgrade()
                {
                    let _ = context.dispatch_worker_registry(|_, hub| {
                        hub.discard_transit_value(Self::principal(&owner), retained)
                            .map_err(error)
                    });
                }
            })
            .map_err(|failure| {
                self.message_jobs.fetch_sub(1, Ordering::AcqRel);
                self.message_bytes.fetch_sub(bytes, Ordering::AcqRel);
                let _ = context.dispatch_worker_registry(|_, hub| {
                    hub.discard_transit_value(Self::principal(&cleanup_owner), failed_admission)
                        .map_err(error)
                });
                error(failure.to_string())
            })?;
        self.jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(job);
        Ok(())
    }
    fn post_profile_message(
        self: &Arc<Self>,
        source_registry: &Arc<Self>,
        owner: AgentOwner,
        version: ServiceVersionId,
        mut value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        let source_context = source_registry.context()?;
        let target_context = self.context()?;
        let source = source_registry.source_snapshot(&owner)?;
        let agent = {
            let state = self.state()?;
            state
                .versions
                .iter()
                .find(|(_, entry)| entry.snapshot.version_id == version)
                .map(|(agent, _)| *agent)
                .ok_or_else(|| error("profile service version retired"))?
        };
        let bytes = value.retained_bytes();
        self.message_jobs
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 256).then_some(count + 1)
            })
            .map_err(|_| error("profile service message job capacity"))?;
        if self
            .message_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |retained| {
                retained
                    .checked_add(bytes)
                    .filter(|total| *total <= 32 * 1024 * 1024)
            })
            .is_err()
        {
            self.message_jobs.fetch_sub(1, Ordering::AcqRel);
            return Err(error("profile service message byte capacity"));
        }
        let cleanup = value.cleanup_clone();
        let failed = cleanup.clone();
        let failed_owner = owner.clone();
        let failed_context = source_context.clone();
        let registry = self.clone();
        let job = std::thread::Builder::new()
            .name("oryn-profile-service-message".into())
            .spawn(move || {
                let _budget = MessageJobBudget {
                    registry: registry.clone(),
                    bytes,
                };
                if source_context
                    .route_service_value_to(
                        &target_context,
                        &owner,
                        &AgentOwner::Agent(agent),
                        &mut value,
                    )
                    .is_err()
                {
                    let _ = source_context.dispatch_worker_registry(|_, hub| {
                        hub.discard_transit_value(Self::principal(&owner), cleanup)
                            .map_err(error)
                    });
                    return;
                }
                let cleanup = value.cleanup_clone();
                let result = (|| {
                    let id = registry.id()?;
                    let lease = {
                        let state = registry.state()?;
                        let entry = state
                            .versions
                            .get(&agent)
                            .filter(|entry| entry.snapshot.version_id == version)
                            .ok_or_else(|| error("profile service version replaced"))?;
                        registry.reserve_version_event(&state, agent, entry, true)?
                    };
                    registry.dispatch_reserved_admitted(
                        AgentCommand::ServiceMessage { id, source, value },
                        id,
                        None,
                        lease,
                        None,
                        true,
                    )
                })();
                if result.is_err() {
                    let _ = target_context.dispatch_worker_registry(|_, hub| {
                        hub.dispose_queued_value(
                            oryn_js::message_wire::Principal::Agent(agent.0),
                            cleanup,
                        )
                        .map_err(error)
                    });
                }
            });
        match job {
            Ok(job) => {
                self.jobs
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(job);
                Ok(())
            }
            Err(failure) => {
                self.message_jobs.fetch_sub(1, Ordering::AcqRel);
                self.message_bytes.fetch_sub(bytes, Ordering::AcqRel);
                let _ = failed_context.dispatch_worker_registry(|_, hub| {
                    hub.discard_transit_value(Self::principal(&failed_owner), failed)
                        .map_err(error)
                });
                Err(error(failure.to_string()))
            }
        }
    }
    fn post_client_message(
        &self,
        source: AgentOwner,
        client: &str,
        mut value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        value.validate().map_err(error)?;
        let AgentOwner::Agent(agent) = source else {
            return Err(WorkerError::new(
                "SecurityError",
                "Client.postMessage requires service principal",
            ));
        };
        let context = self.context()?;
        let bootstrap = context.worker_bootstrap(agent)?;
        if bootstrap.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "SecurityError",
                "Client.postMessage requires service principal",
            ));
        }
        let local = {
            let state = self.state()?;
            state
                .client_ids
                .iter()
                .find(|(_, id)| id.as_str() == client)
                .map(|(owner, _)| owner.clone())
        };
        let owner = match local {
            Some(owner) => owner,
            None => {
                let coordinator = context
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?;
                let (target, owner) = coordinator
                    .versions
                    .client(
                        &persistence::partition_key(&bootstrap.partition)?,
                        &bootstrap.origin.authority,
                        client,
                    )
                    .ok_or_else(|| error("unknown service client"))?;
                return target.post_profile_client_message(self, agent, owner, value);
            }
        };
        let (partition, origin, _) = context.retained_worker_authority(&owner)?;
        if partition != bootstrap.partition || origin.authority != bootstrap.origin.authority {
            return Err(WorkerError::new("SecurityError", "foreign service client"));
        }
        let worker = self
            .state()?
            .versions
            .get(&agent)
            .map(|version| version.snapshot.clone())
            .ok_or_else(|| error("service sender retired"))?;
        let mut state = self.state()?;
        let bytes = state
            .messages
            .values()
            .flatten()
            .fold(0usize, |sum, message| {
                sum.saturating_add(message.value.retained_bytes())
            });
        let size = value.retained_bytes();
        let count = state
            .messages
            .get(&owner)
            .map_or(0, |messages| messages.len())
            + state.reserved_messages.get(&owner).copied().unwrap_or(0);
        if bytes
            .saturating_add(state.reserved_bytes)
            .saturating_add(size)
            > 32 * 1024 * 1024
            || count >= 256
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "service client message queue",
            ));
        }
        state.reserved_bytes += size;
        *state.reserved_messages.entry(owner.clone()).or_default() += 1;
        drop(state);
        let routed = context.dispatch_worker_registry(|registry, hub| {
            registry.principal(agent)?;
            if let AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } = &owner
            {
                hub.retained_document_authority(page.0, target, *generation, *epoch)
                    .map_err(error)?;
            } else if let AgentOwner::Agent(target) = &owner {
                registry.principal(*target)?;
            }
            hub.route_value_for_document(
                oryn_js::message_wire::Principal::Agent(agent.0),
                owner.shared_recipient(),
                &mut value,
            )
            .map_err(error)
        });
        let alive = context.retained_worker_authority(&owner).is_ok();
        let mut state = self.state()?;
        state.reserved_bytes = state.reserved_bytes.saturating_sub(size);
        if let Some(count) = state.reserved_messages.get_mut(&owner) {
            *count = count.saturating_sub(1);
        }
        routed?;
        if !alive || self.retired.load(Ordering::Acquire) {
            drop(state);
            context.dispatch_worker_registry(|_, hub| {
                hub.dispose_queued_value(Self::principal(&owner), value)
                    .map_err(error)
            })?;
            return Err(error("service client retired during delivery"));
        }
        state
            .messages
            .entry(owner.clone())
            .or_default()
            .push_back(ClientMessage {
                source: worker,
                origin: bootstrap.origin.serialized,
                value,
            });
        drop(state);
        if let AgentOwner::Window { page, .. } = owner {
            context.wake_context_events(page);
        }
        Ok(())
    }
    fn post_profile_client_message(
        &self,
        source_registry: &Self,
        agent: AgentId,
        owner: AgentOwner,
        mut value: oryn_js::message_wire::WireValue,
    ) -> Result<(), WorkerError> {
        value.validate().map_err(error)?;
        let source_context = source_registry.context()?;
        let context = self.context()?;
        let bootstrap = source_context.worker_bootstrap(agent)?;
        if bootstrap.kind != AgentKind::Service {
            return Err(error("profile client sender is not a service worker"));
        }
        let (partition, origin, _) = context.retained_worker_authority(&owner)?;
        if persistence::partition_key(&partition)?
            != persistence::partition_key(&bootstrap.partition)?
            || origin.authority != bootstrap.origin.authority
        {
            return Err(WorkerError::new(
                "SecurityError",
                "profile client authority mismatch",
            ));
        }
        let worker = source_registry
            .state()?
            .versions
            .get(&agent)
            .map(|version| version.snapshot.clone())
            .ok_or_else(|| error("profile service sender retired"))?;
        let size = value.retained_bytes();
        {
            let mut state = self.state()?;
            let retained = state
                .messages
                .values()
                .flatten()
                .fold(state.reserved_bytes, |sum, message| {
                    sum.saturating_add(message.value.retained_bytes())
                });
            let count = state
                .messages
                .get(&owner)
                .map_or(0, |messages| messages.len())
                + state.reserved_messages.get(&owner).copied().unwrap_or(0);
            if retained.saturating_add(size) > 32 * 1024 * 1024 || count >= 256 {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "profile client message queue",
                ));
            }
            state.reserved_bytes += size;
            *state.reserved_messages.entry(owner.clone()).or_default() += 1;
        }
        let failed = value.cleanup_clone();
        let routed = source_context.route_service_value_to(
            &context,
            &AgentOwner::Agent(agent),
            &owner,
            &mut value,
        );
        let alive = context.retained_worker_authority(&owner).is_ok();
        let mut state = self.state()?;
        state.reserved_bytes = state.reserved_bytes.saturating_sub(size);
        if let Some(count) = state.reserved_messages.get_mut(&owner) {
            *count = count.saturating_sub(1);
        }
        if let Err(failure) = routed {
            drop(state);
            let _ = source_context.dispatch_worker_registry(|_, hub| {
                hub.discard_transit_value(oryn_js::message_wire::Principal::Agent(agent.0), failed)
                    .map_err(error)
            });
            return Err(failure);
        }
        if !alive || self.retired.load(Ordering::Acquire) {
            drop(state);
            context.dispatch_worker_registry(|_, hub| {
                hub.dispose_queued_value(Self::principal(&owner), value)
                    .map_err(error)
            })?;
            return Err(error("profile client retired during delivery"));
        }
        state
            .messages
            .entry(owner.clone())
            .or_default()
            .push_back(ClientMessage {
                source: worker,
                origin: bootstrap.origin.serialized,
                value,
            });
        drop(state);
        if let AgentOwner::Window { page, .. } = owner {
            context.wake_context_events(page);
        }
        Ok(())
    }
    fn lifecycle(&self, agent: AgentId, event: LifecycleEvent) -> Result<(), WorkerError> {
        let id = self.id()?;
        let (dispatch_stage, completion_stage) = match event {
            LifecycleEvent::Install => ("install-dispatch", "install-completion"),
            LifecycleEvent::Activate => ("activate-dispatch", "activate-completion"),
            LifecycleEvent::Message => ("message-dispatch", "message-completion"),
            LifecycleEvent::Push => ("push-dispatch", "push-completion"),
            LifecycleEvent::Sync => ("sync-dispatch", "sync-completion"),
        };
        match self
            .dispatch(
                agent,
                AgentCommand::Lifecycle {
                    id,
                    event,
                    payload: None,
                },
                id,
            )
            .map_err(|failure| service_task_stage(failure, dispatch_stage, agent, id))?
        {
            AgentEvent::LifecycleFinished { result, .. } => {
                // Rejected activation extensions do not undo activation. A
                // dispatch/transport failure still propagates through `?` above.
                if matches!(event, LifecycleEvent::Activate) {
                    self.context()?
                        .native_worker_authority(&AgentOwner::Agent(agent))
                        .map_err(|failure| {
                            service_task_stage(failure, completion_stage, agent, id)
                        })?;
                    Ok(())
                } else {
                    result
                        .map_err(|failure| service_task_stage(failure, completion_stage, agent, id))
                }
            }
            _ => Err(service_task_stage(
                error("invalid service lifecycle completion"),
                completion_stage,
                agent,
                id,
            )),
        }
    }
    fn reserve_version_event(
        &self,
        state: &State,
        agent: AgentId,
        version: &Version,
        track: bool,
    ) -> Result<ServiceEventLease, WorkerError> {
        if state.pending_restore_admissions.contains_key(&agent) {
            return Err(WorkerError::new(
                "ServiceAdmissionPending",
                "restored service publication is pending",
            ));
        }
        let activity = if track {
            let registration = version
                .bootstrap
                .service_registration
                .and_then(|id| state.registrations.get(&id))
                .ok_or_else(|| error("service event registration retired"))?;
            Some(
                self.context()?
                    .context_store()
                    .service_coordinator()
                    .map_err(|failure| error(failure.to_string()))?
                    .retain_event_for_lifetime(
                        &persistence::partition_key(&registration.partition)?,
                        &registration.origin,
                        &registration.snapshot.scope,
                        &registration.lineage,
                    )
                    .map_err(|failure| error(failure.to_string()))?,
            )
        } else {
            None
        };
        version.inflight.fetch_add(1, Ordering::AcqRel);
        Ok(ServiceEventLease {
            check_soft_update: false,
            // Capture admitted authority while the version is owned. Event Drop
            // must neither lock this state nor query a possibly retired process.
            soft_update_source: SoftUpdateSource::from_bootstrap(&version.bootstrap),
            activity,
            registry: self.weak_self.clone(),
            agent,
            serial: version.serial.clone(),
            inbox: version.inbox.clone(),
            inflight: version.inflight.clone(),
        })
    }
    fn dispatch(
        &self,
        agent: AgentId,
        command: AgentCommand,
        id: u64,
    ) -> Result<AgentEvent, WorkerError> {
        self.dispatch_from(agent, command, id, None)
    }
    fn dispatch_from(
        &self,
        agent: AgentId,
        command: AgentCommand,
        id: u64,
        source_owner: Option<&AgentOwner>,
    ) -> Result<AgentEvent, WorkerError> {
        let lease = {
            let state = self.state()?;
            let version = state
                .versions
                .get(&agent)
                .ok_or_else(|| error("unknown service version"))?;
            self.reserve_version_event(
                &state,
                agent,
                version,
                !matches!(
                    &command,
                    AgentCommand::Lifecycle {
                        event: LifecycleEvent::Install | LifecycleEvent::Activate,
                        ..
                    }
                ),
            )?
        };
        self.dispatch_reserved(command, id, source_owner, lease)
    }
    fn dispatch_reserved(
        &self,
        command: AgentCommand,
        id: u64,
        source_owner: Option<&AgentOwner>,
        lease: ServiceEventLease,
    ) -> Result<AgentEvent, WorkerError> {
        self.dispatch_reserved_with_cancel(command, id, source_owner, lease, None)
    }
    fn dispatch_reserved_with_cancel(
        &self,
        command: AgentCommand,
        id: u64,
        source_owner: Option<&AgentOwner>,
        lease: ServiceEventLease,
        cancel: Option<&dyn Fn() -> bool>,
    ) -> Result<AgentEvent, WorkerError> {
        self.dispatch_reserved_admitted(command, id, source_owner, lease, cancel, false)
    }
    fn dispatch_reserved_admitted(
        &self,
        command: AgentCommand,
        id: u64,
        source_owner: Option<&AgentOwner>,
        mut lease: ServiceEventLease,
        cancel: Option<&dyn Fn() -> bool>,
        profile_admitted: bool,
    ) -> Result<AgentEvent, WorkerError> {
        if cancel.is_some_and(|cancel| cancel()) {
            return Err(
                WorkerError::new("AbortError", "service fetch cancelled before dispatch")
                    .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Cancelled),
            );
        }
        let check_soft_update = matches!(
            &command,
            AgentCommand::Fetch(_)
                | AgentCommand::Lifecycle {
                    event: LifecycleEvent::Push | LifecycleEvent::Sync,
                    ..
                }
        );
        let agent = lease.agent;
        let serial = lease.serial.clone();
        let context = self.context()?;
        {
            let mut inbox = lease
                .inbox
                .lock()
                .map_err(|_| error("service control inbox poisoned"))?;
            if inbox.active.len() >= 256 {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "service concurrent event limit",
                )
                .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Capacity));
            }
            if !inbox.active.insert(id) {
                return Err(error("duplicate service event identity"));
            }
        }
        let slot = ServiceTaskSlot {
            registry: self.weak_self.clone(),
            inbox: lease.inbox.clone(),
            id,
        };
        {
            let _serial = serial
                .lock()
                .map_err(|_| error("service event queue poisoned"))?;
            context.dispatch_worker_registry(|registry, hub| match command {
                AgentCommand::ServiceMessage { id, source, value } => {
                    if profile_admitted {
                        return registry
                            .post_profile_service_message(hub, agent, id, source, value);
                    }
                    let owner = source_owner
                        .ok_or_else(|| error("service message source authority missing"))?;
                    if let AgentOwner::Window {
                        page,
                        target,
                        generation,
                        epoch,
                    } = owner
                    {
                        hub.retained_document_authority(page.0, target, *generation, *epoch)
                            .map_err(error)?;
                    }
                    registry.post_service_message(hub, owner, agent, id, source, value)
                }
                command => registry.send_service_control(agent, command),
            })?;
        }
        lease.check_soft_update = check_soft_update;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.retired.load(Ordering::Acquire) {
                return Err(error("service registry retired during event"));
            }
            if cancel.is_some_and(|cancel| cancel()) {
                // Revoke the event-local response work without abandoning its
                // version lease or killing unrelated events in this worker.
                context.dispatch_worker_registry(|registry, _| {
                    registry.send_service_control(agent, AgentCommand::CancelTask { id })
                })?;
                self.retain_service_lifetime(lease, slot, deadline)?;
                return Err(WorkerError::new("AbortError", "service fetch cancelled")
                    .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Cancelled));
            }
            let (response, finished) =
                self.poll_service_task(&context, agent, &serial, &slot.inbox, id)?;
            if let Some(finished) = finished {
                return Ok(response.unwrap_or(finished));
            }
            if let Some(response) = response {
                // respondWith settles the fetch independently of waitUntil.
                // A parent-owned background task retains this version lease,
                // completion mailbox and cancellation duty until lifetime end.
                self.retain_service_lifetime(lease, slot, deadline)?;
                return Ok(response);
            }
            if Instant::now() >= deadline {
                let _ = context.dispatch_worker_registry(|registry, _| {
                    registry.send_service_control(agent, AgentCommand::CancelTask { id })
                });
                self.retire_version(agent)?;
                return Err(
                    WorkerError::new("TimeoutError", "service event deadline exceeded")
                        .with_script_failure(
                            oryn_js::worker_wire::ScriptFailureKind::Infrastructure,
                        ),
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn poll_service_task(
        &self,
        context: &BrowserContext,
        agent: AgentId,
        serial: &Mutex<()>,
        mailbox: &Mutex<ServiceControlInbox>,
        id: u64,
    ) -> Result<(Option<AgentEvent>, Option<AgentEvent>), WorkerError> {
        let _pump = serial
            .lock()
            .map_err(|_| error("service event pump poisoned"))?;
        let events = context
            .dispatch_worker_registry(|registry, hub| registry.poll_control(hub, agent, 32))
            .map_err(|failure| service_task_stage(failure, "process-poll", agent, id))?;
        let mut inbox = mailbox
            .lock()
            .map_err(|_| error("service control inbox poisoned"))?;
        for event in events {
            if inbox.accept_runtime_marker(&event) {
                continue;
            }
            match &event {
                AgentEvent::LifecycleFinished { id: actual, .. }
                | AgentEvent::FetchResponse { id: actual, .. }
                | AgentEvent::FetchFallback { id: actual }
                | AgentEvent::FetchResponseStart { id: actual, .. } => {
                    if !inbox.active.contains(actual) {
                        continue;
                    }
                    if matches!(event, AgentEvent::FetchResponseStart { .. })
                        && !inbox.started.insert(*actual)
                    {
                        inbox.fatal = Some(error("duplicate service response headers"));
                        continue;
                    }
                    let key = (
                        *actual,
                        if matches!(event, AgentEvent::LifecycleFinished { .. }) {
                            1
                        } else {
                            0
                        },
                    );
                    if inbox.completed.contains_key(&key) {
                        inbox.fatal = Some(error("duplicate service task stage completion"));
                        continue;
                    }
                    let bytes = crate::agent_thread::wire_size(&event, 32 * 1024 * 1024)?;
                    if inbox.bytes.saturating_add(bytes) > 32 * 1024 * 1024 {
                        inbox.fatal = Some(WorkerError::new(
                            "QuotaExceededError",
                            "service completion bytes",
                        ));
                        continue;
                    }
                    inbox.bytes += bytes;
                    inbox.completed.insert(key, (event, bytes));
                }
                AgentEvent::FetchResponseChunk { id: actual, .. }
                | AgentEvent::FetchResponseEnd { id: actual, .. } => {
                    if !inbox.active.contains(actual) {
                        continue;
                    }
                    if matches!(&event,AgentEvent::FetchResponseChunk{bytes,..}if bytes.len()>64*1024)
                    {
                        inbox.fatal = Some(error("service body chunk exceeds negotiated bound"));
                        continue;
                    }
                    if !inbox.started.contains(actual) || inbox.ended.contains(actual) {
                        inbox.fatal = Some(error("service body event outside live headers"));
                        continue;
                    }
                    if matches!(event, AgentEvent::FetchResponseEnd { .. }) {
                        inbox.ended.insert(*actual);
                    }
                    let bytes = crate::agent_thread::wire_size(&event, 128 * 1024)?;
                    if inbox.body.get(actual).is_some_and(|queue| queue.len() >= 4)
                        || inbox.bytes.saturating_add(bytes) > 32 * 1024 * 1024
                    {
                        inbox.fatal = Some(error("service body queue budget exceeded"));
                        continue;
                    }
                    inbox.bytes += bytes;
                    inbox
                        .body
                        .entry(*actual)
                        .or_default()
                        .push_back((event, bytes));
                }
                AgentEvent::Closed => {
                    inbox.fatal = Some(error("service worker closed during event"))
                }
                _ => {}
            }
        }
        if let Some(error) = &inbox.fatal {
            return Err(service_task_stage(error.clone(), "task-inbox", agent, id));
        }
        let response = inbox.completed.remove(&(id, 0)).map(|(event, bytes)| {
            inbox.bytes = inbox.bytes.saturating_sub(bytes);
            event
        });
        let finished = inbox.completed.remove(&(id, 1)).map(|(event, bytes)| {
            inbox.bytes = inbox.bytes.saturating_sub(bytes);
            event
        });
        Ok((response, finished))
    }
    fn retain_service_lifetime(
        &self,
        lease: ServiceEventLease,
        slot: ServiceTaskSlot,
        deadline: Instant,
    ) -> Result<(), WorkerError> {
        let registry = self
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service registry retired"))?;
        let agent = lease.agent;
        let id = slot.id;
        let job = std::thread::Builder::new()
            .name(format!("oryn-service-lifetime-{id}"))
            .spawn(move || {
                let _lease = lease;
                let context = match registry.context() {
                    Ok(context) => context,
                    Err(_) => return,
                };
                while !registry.retired.load(Ordering::Acquire) {
                    match registry.poll_service_task(
                        &context,
                        agent,
                        &_lease.serial,
                        &slot.inbox,
                        id,
                    ) {
                        Ok((_, Some(_))) => return,
                        Ok(_) => {}
                        Err(_) => {
                            let _ = registry.retire_version(agent);
                            return;
                        }
                    }
                    if Instant::now() >= deadline {
                        let _ = context.dispatch_worker_registry(|agents, _| {
                            agents.send_service_control(agent, AgentCommand::CancelTask { id })
                        });
                        let _ = registry.retire_version(agent);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        match job {
            Ok(job) => {
                self.jobs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(job);
                Ok(())
            }
            Err(failure) => {
                let _ = self.retire_version(agent);
                Err(error(failure.to_string()))
            }
        }
    }
    fn wake_clients(&self) {
        let Some(context) = self.context.upgrade() else {
            return;
        };
        let pages = self
            .state()
            .map(|state| {
                state
                    .observed
                    .keys()
                    .chain(state.clients.keys())
                    .filter_map(|owner| {
                        if let AgentOwner::Window { page, .. } = owner {
                            Some(*page)
                        } else {
                            None
                        }
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        for page in pages {
            context.wake_context_events(page);
        }
    }
    fn retain_profile_client(
        &self,
        state: &mut State,
        owner: &AgentOwner,
        id: RegistrationId,
        claim: bool,
    ) -> Result<(), WorkerError> {
        if state.clients.get(owner) == Some(&id) && state.client_leases.contains_key(owner) {
            return Ok(());
        }
        let registration = state
            .registrations
            .get(&id)
            .ok_or_else(|| error("service registration retired"))?;
        let lease = self
            .context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?
            .retain_client_for_activation(
                &persistence::partition_key(&registration.partition)?,
                &registration.origin,
                &registration.snapshot.scope,
                &registration.lineage,
                claim
                    && registration
                        .snapshot
                        .active
                        .as_ref()
                        .is_some_and(|active| active.state == WorkerState::Activating),
            )
            .map_err(|failure| error(failure.to_string()))?;
        state.client_leases.insert(owner.clone(), lease);
        Ok(())
    }
    fn watch_profile_activation(
        &self,
        state: &mut State,
        id: RegistrationId,
    ) -> Result<(), WorkerError> {
        if state.activation_watches.contains_key(&id) {
            return Ok(());
        }
        let Some(registration) = state.registrations.get(&id) else {
            return Ok(());
        };
        let weak = self.weak_self.clone();
        let queued = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(move || {
            let Some(registry) = weak.upgrade() else {
                return;
            };
            if registry.retired.load(Ordering::Acquire) || queued.swap(true, Ordering::AcqRel) {
                return;
            }
            let pending = queued.clone();
            let active = registry.clone();
            match std::thread::Builder::new()
                .name("oryn-service-profile-activation".into())
                .spawn(move || {
                    pending.store(false, Ordering::Release);
                    if !active.retired.load(Ordering::Acquire) {
                        let refresh = (|| -> Result<(), WorkerError> {
                            let context = active.context()?;
                            let saved = {
                                let state = active.state()?;
                                let Some(registration) = state.registrations.get(&id) else {
                                    return Ok(());
                                };
                                let Some(bootstrap) = registration
                                    .snapshot
                                    .waiting
                                    .as_ref()
                                    .or(registration.snapshot.active.as_ref())
                                    .and_then(|worker| Self::version_bootstrap(&state, worker))
                                else {
                                    return Ok(());
                                };
                                (registration.partition.clone(), bootstrap.origin.clone())
                            };
                            if let Some(network) = context.shared_network_broker() {
                                active.restore_namespace(saved.0, saved.1, network)?;
                            }
                            active.try_activate(id)?;
                            active.cleanup_unregistered()
                        })();
                        let _ = refresh;
                    }
                }) {
                Ok(job) => registry
                    .jobs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(job),
                Err(_) => {
                    queued.store(false, Ordering::Release);
                }
            }
        });
        let watch = self
            .context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?
            .watch(
                &persistence::partition_key(&registration.partition)?,
                &registration.origin,
                &registration.snapshot.scope,
                wake,
            )
            .map_err(|failure| error(failure.to_string()))?;
        state.activation_watches.insert(id, watch);
        Ok(())
    }
    fn try_activate(&self, id: RegistrationId) -> Result<(), WorkerError> {
        if !self.registration_current(id)? {
            return Ok(());
        }
        let waiting = {
            let mut state = self.state()?;
            self.watch_profile_activation(&mut state, id)?;
            state
                .registrations
                .get(&id)
                .and_then(|registration| registration.snapshot.waiting.clone())
        };
        if let Some(waiting) = waiting
            && let Some((owner, current)) = self.remote_version(&waiting)?
        {
            let registration = owner
                .state()?
                .versions
                .get(&current.id)
                .and_then(|version| version.bootstrap.service_registration)
                .ok_or_else(|| error("profile waiting registration retired"))?;
            return owner.try_activate(registration);
        }
        // Serialize entry into Activating with namespace restoration. Once
        // published, restoration's locked recheck defers to this transition.
        // Never hold this lock across process teardown or author callbacks.
        let activation_publication = self
            .persistence
            .lock()
            .map_err(|_| error("service persistence lock poisoned"))?;
        let candidate = {
            let mut state = self.state()?;
            self.watch_profile_activation(&mut state, id)?;
            let Some(reg) = state.registrations.get(&id) else {
                return Ok(());
            };
            let Some(waiting) = reg.snapshot.waiting.clone() else {
                return Ok(());
            };
            let skip = state
                .versions
                .get(&waiting.id)
                .filter(|version| version.snapshot.version_id == waiting.version_id)
                .is_some_and(|version| version.skip_waiting);
            if reg.snapshot.active.is_some()
                && !skip
                && state.clients.values().any(|registered| *registered == id)
            {
                return Ok(());
            }
            if reg
                .snapshot
                .active
                .as_ref()
                .and_then(|active| {
                    state
                        .versions
                        .get(&active.id)
                        .filter(|version| version.snapshot.version_id == active.version_id)
                })
                .is_some_and(|version| version.inflight.load(Ordering::Acquire) > 0)
            {
                return Ok(());
            }
            let coordinator = self
                .context()?
                .context_store()
                .service_coordinator()
                .map_err(|failure| error(failure.to_string()))?;
            let Some(activation) = coordinator
                .begin_activation_for_lifetime(
                    &persistence::partition_key(&reg.partition)?,
                    &reg.origin,
                    &reg.snapshot.scope,
                    &reg.lineage,
                    skip || reg.snapshot.active.is_none(),
                )
                .map_err(|failure| error(failure.to_string()))?
            else {
                return Ok(());
            };
            if !self.registration_fields_current(
                &reg.partition,
                &reg.origin,
                &reg.snapshot.scope,
                reg.durable_fingerprint,
            )? {
                return Ok(());
            }
            let old = reg.snapshot.active.clone();
            let mut active = waiting;
            active.state = WorkerState::Activating;
            state
                .versions
                .get_mut(&active.id)
                .filter(|version| version.snapshot.version_id == active.version_id)
                .ok_or_else(|| error("waiting service retired"))?
                .snapshot = active.clone();
            (active.id, old, activation)
        };
        drop(activation_publication);
        let mut old_reservation = None;
        if let Some(old) = candidate.1 {
            let result = (|| {
                let (owner, current) = match self.remote_version(&old)? {
                    Some(remote) => remote,
                    None => (
                        self.weak_self
                            .upgrade()
                            .ok_or_else(|| error("service registry retired"))?,
                        old,
                    ),
                };
                old_reservation = owner
                    .state()?
                    .versions
                    .get(&current.id)
                    .filter(|version| version.snapshot.version_id == current.version_id)
                    .and_then(|version| version.directory.clone());
                owner.retire_version(current.id)
            })();
            if let Err(failure) = result {
                if let Some(version) = self.state()?.versions.get_mut(&candidate.0) {
                    version.snapshot.state = WorkerState::Installed;
                }
                return Err(failure);
            }
        }
        {
            let mut state = self.state()?;
            let active = state
                .versions
                .get(&candidate.0)
                .ok_or_else(|| error("waiting service retired during activation"))?
                .snapshot
                .clone();
            let reg = state
                .registrations
                .get_mut(&id)
                .ok_or_else(|| error("registration retired during activation"))?;
            if reg.snapshot.waiting.as_ref().map(|waiting| waiting.id) != Some(candidate.0) {
                return Err(error("waiting service changed during activation"));
            }
            reg.snapshot.waiting = None;
            reg.snapshot.active = Some(active);
        }
        self.publish_version(candidate.0)?;
        let result = self.lifecycle(candidate.0, LifecycleEvent::Activate);
        {
            let mut state = self.state()?;
            // Keep the completed native transition authoritative until its
            // durable compare-and-replace has committed.
            state.dirty_registrations.insert(id);
            if result.is_ok() {
                // Publish active-work readiness before Activated becomes
                // visible. The lease still excludes competing profile jobs
                // until persistence completes and the existing final drop runs.
                candidate
                    .2
                    .allow_active_work()
                    .map_err(|failure| error(failure.to_string()))?;
                if let Some(version) = state.versions.get_mut(&candidate.0) {
                    version.snapshot.state = WorkerState::Activated;
                }
                if let Some(active) = state
                    .registrations
                    .get_mut(&id)
                    .and_then(|reg| reg.snapshot.active.as_mut())
                {
                    active.state = WorkerState::Activated;
                }
            } else if let Some(reg) = state.registrations.get_mut(&id) {
                reg.snapshot.active = None;
            }
        }
        if result.is_err() {
            self.retire_version(candidate.0)?;
        } else {
            self.publish_version(candidate.0)?;
        }
        self.persist_registration(id)?;
        drop(old_reservation);
        drop(candidate.2);
        self.wake_clients();
        result
    }
    fn cleanup_unregistered(&self) -> Result<(), WorkerError> {
        // Cleanup remains authorized after registry admission is retired.
        let context = self
            .context
            .upgrade()
            .ok_or_else(|| error("service cleanup context retired"))?;
        let coordinator = context
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        // A scope lease excludes new clients and events atomically across the
        // profile. A count snapshot would race a sibling attaching a client.
        let mut retirement_leases = Vec::new();
        let agents = {
            let mut state = self.state()?;
            let candidates = state
                .registrations
                .iter()
                .filter(|(id, registration)| {
                    registration.unregistered
                        && !state.clients.values().any(|client| client == *id)
                        && !state.versions.values().any(|version| {
                            version.bootstrap.service_registration == Some(**id)
                                && version.inflight.load(Ordering::Acquire) > 0
                        })
                })
                .map(|(id, registration)| {
                    (
                        *id,
                        registration.partition.clone(),
                        registration.origin.clone(),
                        registration.snapshot.scope.clone(),
                        registration.lineage.clone(),
                    )
                })
                .collect::<Vec<_>>();
            for (id, partition, origin, scope, lineage) in candidates {
                let Some(lease) = coordinator
                    .begin_activation_for_lifetime(
                        &persistence::partition_key(&partition)?,
                        &origin,
                        &scope,
                        &lineage,
                        false,
                    )
                    .map_err(|failure| error(failure.to_string()))?
                else {
                    continue;
                };
                retirement_leases.push(lease);
                state.registrations.remove(&id);
                state.activation_watches.remove(&id);
                state.dirty_registrations.remove(&id);
            }
            // Remote snapshots retain identity only while a local registration
            // (including an unregistered controller) still references them.
            let referenced = state
                .registrations
                .values()
                .flat_map(|registration| {
                    [
                        registration.snapshot.installing.as_ref(),
                        registration.snapshot.waiting.as_ref(),
                        registration.snapshot.active.as_ref(),
                    ]
                })
                .flatten()
                .map(|snapshot| snapshot.version_id)
                .collect::<std::collections::BTreeSet<_>>();
            state
                .remote_versions
                .retain(|version, _| referenced.contains(version));
            // Failed process cleanup remains discoverable on the next request;
            // never discard the Version owner before parent teardown succeeds.
            state
                .versions
                .iter()
                .filter(|(_, version)| {
                    version.bootstrap.service_registration.is_some_and(|id| {
                        state.registrations.get(&id).is_none_or(|registration| {
                            ![
                                registration.snapshot.installing.as_ref(),
                                registration.snapshot.waiting.as_ref(),
                                registration.snapshot.active.as_ref(),
                            ]
                            .into_iter()
                            .flatten()
                            .any(|snapshot| snapshot.version_id == version.snapshot.version_id)
                        })
                    }) && version.inflight.load(Ordering::Acquire) == 0
                })
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        let mut failure = None;
        for agent in agents {
            // This is an opportunistic sweep, not a teardown completion
            // barrier. A competing owner keeps the Version and its receipt;
            // it will finish retirement or leave the version retryable. Do not
            // poison an unrelated service request merely because it is busy.
            if let Err(error) = self.retire_version_if_idle(agent) {
                failure.get_or_insert(error);
            }
        }
        drop(retirement_leases);
        failure.map_or(Ok(()), Err)
    }
    fn reap_orphaned_services(&self) -> Result<(), WorkerError> {
        let agents = self
            .state()?
            .orphaned_services
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let mut failure = None;
        for agent in agents {
            if let Err(error) = self.retire_orphan(agent) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn retire_orphan(&self, agent: AgentId) -> Result<(), WorkerError> {
        self.retire_orphan_with(agent, || {
            let context = self
                .context
                .upgrade()
                .ok_or_else(|| error("orphaned service context unavailable during retirement"))?;
            Self::retire_process(&context, agent)
        })
    }
    fn retire_orphan_with(
        &self,
        agent: AgentId,
        retire: impl FnOnce() -> Result<(), WorkerError>,
    ) -> Result<(), WorkerError> {
        {
            let mut state = self.state()?;
            if !state.orphaned_services.contains(&agent) {
                return Ok(());
            }
            if !state.retiring_orphans.insert(agent) {
                return Err(error("orphaned service retirement already in progress"));
            }
        }
        // Keep the authoritative orphan owner throughout teardown, including
        // unwinding. Only this admission can consume its parent receipt.
        struct Admission<'a> {
            registry: &'a ServiceRegistry,
            agent: AgentId,
        }
        impl Drop for Admission<'_> {
            fn drop(&mut self) {
                self.registry
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .retiring_orphans
                    .remove(&self.agent);
            }
        }
        let _admission = Admission {
            registry: self,
            agent,
        };
        retire()?;
        self.state()?.orphaned_services.remove(&agent);
        Ok(())
    }
    fn fetch_executor(
        &self,
        snapshot: &WorkerSnapshot,
    ) -> Result<(Arc<Self>, ServiceEventLease), WorkerError> {
        // A persistent service can outlive the context that last executed it.
        // Re-admit its saved native graph in this context only after the prior
        // logical owner has actually released its directory reservation.
        let mut resolved = self.remote_version(snapshot);
        if resolved.is_err() {
            let bootstrap = {
                let state = self.state()?;
                state
                    .remote_versions
                    .get(&snapshot.version_id)
                    .map(|remote| remote.bootstrap.clone())
            };
            if let Some(bootstrap) = bootstrap {
                let context = self.context()?;
                let network = context
                    .shared_network_broker()
                    .ok_or_else(|| error("service restoration network unavailable"))?;
                self.restore_namespace(bootstrap.partition, bootstrap.origin, network)?;
                resolved = self.remote_version(snapshot);
            }
        }
        let (executor, current) = match resolved? {
            Some(remote) => remote,
            None => (
                self.weak_self
                    .upgrade()
                    .ok_or_else(|| error("service registry retired"))?,
                snapshot.clone(),
            ),
        };
        let lease = {
            let state = executor.state()?;
            let (agent, version) = state
                .versions
                .iter()
                .find(|(_, version)| {
                    version.snapshot.version_id == current.version_id
                        && version.snapshot.state == WorkerState::Activated
                })
                .ok_or_else(|| error("active service version unavailable"))?;
            executor.reserve_version_event(&state, *agent, version, true)?
        };
        Ok((executor, lease))
    }
    fn remote_version(
        &self,
        snapshot: &WorkerSnapshot,
    ) -> Result<Option<(Arc<Self>, WorkerSnapshot)>, WorkerError> {
        let remote = {
            let state = self.state()?;
            if state
                .versions
                .values()
                .any(|version| version.snapshot.version_id == snapshot.version_id)
            {
                return Ok(None);
            }
            state.remote_versions.get(&snapshot.version_id).cloned()
        };
        let Some(remote) = remote else {
            return Ok(None);
        };
        let registry = remote
            .owner
            .upgrade()
            .ok_or_else(|| error("profile service owner retired; restoration required"))?;
        let current = registry
            .state()?
            .versions
            .values()
            .find(|version| version.snapshot.version_id == snapshot.version_id)
            .map(|version| version.snapshot.clone())
            .ok_or_else(|| error("profile service process retired; restoration required"))?;
        Ok(Some((registry, current)))
    }
    fn version_bootstrap<'a>(
        state: &'a State,
        snapshot: &WorkerSnapshot,
    ) -> Option<&'a AgentBootstrap> {
        state
            .versions
            .get(&snapshot.id)
            .filter(|version| version.snapshot.version_id == snapshot.version_id)
            .map(|version| &version.bootstrap)
            .or_else(|| {
                state
                    .remote_versions
                    .get(&snapshot.version_id)
                    .map(|version| &version.bootstrap)
            })
    }
    fn observed_registration_id(
        &self,
        partition: &str,
        origin: &str,
        lineage: &str,
    ) -> Result<RegistrationId, WorkerError> {
        let mut state = self.state()?;
        if let Some(id) = state
            .registrations
            .iter()
            .find(|(_, registration)| {
                registration.partition == partition
                    && registration.origin == origin
                    && registration.lineage == lineage
            })
            .map(|(id, _)| *id)
        {
            return Ok(id);
        }
        let key = (partition.into(), origin.into(), lineage.into());
        if let Some(id) = state.observed_registration_ids.get(&key) {
            return Ok(*id);
        }
        if state.observed_registration_ids.len() >= 4096 {
            return Err(error("service observation identity capacity"));
        }
        let id = RegistrationId(self.id()?);
        state.observed_registration_ids.insert(key, id);
        Ok(id)
    }
    fn registration_snapshots(
        &self,
        partition: &str,
        origin: &str,
    ) -> Result<Vec<RegistrationSnapshot>, WorkerError> {
        let mut snapshots = self
            .state()?
            .registrations
            .values()
            .filter(|registration| Self::visible(registration, partition, origin))
            .map(|registration| registration.snapshot.clone())
            .collect::<Vec<_>>();
        let coordinator = self
            .context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        let observations = coordinator
            .versions
            .observations(&persistence::partition_key(partition)?, origin)
            .map_err(|failure| error(failure.to_string()))?;
        for observation in observations
            .into_iter()
            .filter(|observation| observation.snapshot.state == WorkerState::Installing)
        {
            let source = {
                let state = observation.owner.state()?;
                state
                    .registrations
                    .values()
                    .find(|registration| {
                        registration.lineage == observation.lineage
                            && registration.snapshot.scope == observation.scope
                            && !registration.unregistered
                            && registration
                                .snapshot
                                .installing
                                .as_ref()
                                .is_some_and(|worker| {
                                    worker.version_id == observation.snapshot.version_id
                                })
                    })
                    .map(|registration| registration.snapshot.clone())
            };
            let Some(mut source) = source else { continue };
            source.id = self.observed_registration_id(partition, origin, &observation.lineage)?;
            if let Some(existing) = snapshots
                .iter_mut()
                .find(|snapshot| snapshot.id == source.id)
            {
                existing.installing = source.installing;
            } else {
                snapshots.push(source);
            }
        }
        Ok(snapshots)
    }
    fn retry_restore_publications(&self) -> Result<(), WorkerError> {
        let agents = self
            .state()?
            .pending_restore_admissions
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for agent in agents {
            self.publish_version(agent)?;
        }
        Ok(())
    }
    fn publish_version(&self, agent: AgentId) -> Result<(), WorkerError> {
        #[cfg(test)]
        persistence::publication_fault()?;
        let publication = {
            let state = self.state()?;
            state.versions.get(&agent).and_then(|version| {
                version
                    .directory
                    .clone()
                    .map(|directory| (directory, version.snapshot.clone()))
            })
        };
        if let Some((directory, snapshot)) = publication {
            directory
                .publish(snapshot)
                .map_err(|failure| error(failure.to_string()))?;
        }
        // Installing and state transitions precede durable publication. Wake
        // profile observers now so their native snapshot polling can deliver
        // updatefound/statechange without waiting for another unrelated task.
        let coordinator = self
            .context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?;
        for registry in coordinator
            .versions
            .registries()
            .map_err(|failure| error(failure.to_string()))?
        {
            registry.wake_clients();
        }
        let admission = self.state()?.pending_restore_admissions.remove(&agent);
        if let Some(admission) = admission {
            admission.publish();
        }
        Ok(())
    }
    fn retire_process(context: &BrowserContext, agent: AgentId) -> Result<(), WorkerError> {
        context.dispatch_worker_registry(|registry, hub| registry.retire_service(hub, agent))?;
        context.acknowledge_service_retirement(agent)
    }
    fn retire_version(&self, agent: AgentId) -> Result<(), WorkerError> {
        if self.retire_version_if_idle(agent)? {
            Ok(())
        } else {
            Err(error("service retirement already in progress"))
        }
    }
    /// False retains the other retirement owner's duty; it is not completion.
    fn retire_version_if_idle(&self, agent: AgentId) -> Result<bool, WorkerError> {
        {
            let mut state = self.state()?;
            let Some(version) = state.versions.get_mut(&agent) else {
                return Ok(true);
            };
            if version.retiring {
                return Ok(false);
            }
            version.retiring = true;
        }
        struct Admission<'a> {
            registry: &'a ServiceRegistry,
            agent: AgentId,
        }
        impl Drop for Admission<'_> {
            fn drop(&mut self) {
                if let Some(version) = self
                    .registry
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .versions
                    .get_mut(&self.agent)
                {
                    version.retiring = false;
                }
            }
        }
        let _admission = Admission {
            registry: self,
            agent,
        };
        let context = self.context.upgrade().ok_or_else(|| {
            error("service context unavailable before retirement acknowledgement")
        })?;
        Self::retire_process(&context, agent)?;
        let mut state = self.state()?;
        state.pending_restore_admissions.remove(&agent);
        if let Some(version) = state.versions.remove(&agent) {
            // A terminated process is not an unregistered service. Force the
            // next authorized lookup to reload its retained durable graph.
            if state.registrations.values().any(|registration| {
                [
                    registration.snapshot.installing.as_ref(),
                    registration.snapshot.waiting.as_ref(),
                    registration.snapshot.active.as_ref(),
                ]
                .into_iter()
                .flatten()
                .any(|worker| worker.id == agent)
            }) {
                state.restored_namespaces.remove(&(
                    version.bootstrap.partition,
                    version.bootstrap.origin.authority,
                ));
            }
        }
        Ok(true)
    }
    pub(crate) fn controller_registration(&self, owner: &AgentOwner) -> Option<RegistrationId> {
        self.state()
            .ok()
            .and_then(|state| state.clients.get(owner).copied())
    }
    pub(crate) fn ensure_agent_controller(
        &self,
        bootstrap: &AgentBootstrap,
    ) -> Result<(), WorkerError> {
        if bootstrap.kind == AgentKind::Service {
            return Ok(());
        }
        let owner = AgentOwner::Agent(bootstrap.id);
        let mut state = self.state()?;
        if state.observed.contains_key(&owner) {
            return Ok(());
        }
        state
            .observed
            .insert(owner.clone(), bootstrap.script_url.clone());
        if let Some(id) = bootstrap.service_controller
            && state.registrations.get(&id).is_some_and(|registration| {
                registration.partition == bootstrap.partition
                    && registration.origin == bootstrap.origin.authority
                    && registration.snapshot.active.is_some()
            })
        {
            self.retain_profile_client(&mut state, &owner, id, false)?;
            state.clients.insert(owner, id);
        }
        Ok(())
    }
    pub(crate) fn control_client(
        &self,
        owner: AgentOwner,
        claimed: Option<RegistrationId>,
    ) -> Result<Option<WorkerSnapshot>, WorkerError> {
        let context = self.context()?;
        let (partition, origin, policy) = context.native_worker_authority(&owner)?;
        let mut state = self.state()?;
        state.observed.insert(owner.clone(), policy.url.clone());
        let candidate = state
            .registrations
            .iter()
            .filter(|(id, reg)| {
                claimed.is_none_or(|claimed| claimed == **id)
                    && Self::visible(reg, &partition, &origin.authority)
                    && policy.url.starts_with(&reg.snapshot.scope)
                    && reg.snapshot.active.as_ref().is_some_and(|active| {
                        matches!(
                            active.state,
                            WorkerState::Activating | WorkerState::Activated
                        )
                    })
            })
            .max_by_key(|(_, reg)| reg.snapshot.scope.len())
            .map(|(id, reg)| (*id, reg.snapshot.active.clone().unwrap()));
        if let Some((id, worker)) = candidate {
            self.retain_profile_client(&mut state, &owner, id, claimed == Some(id))?;
            state.clients.insert(owner, id);
            Ok(Some(worker))
        } else {
            Ok(None)
        }
    }
    fn dispose_messages(
        &self,
        owner: AgentOwner,
        messages: std::collections::VecDeque<ClientMessage>,
    ) -> Result<(), WorkerError> {
        let Some(context) = self.context.upgrade() else {
            return Ok(());
        };
        let mut failure = None;
        for message in messages {
            let value = oryn_js::message_wire::WireValue {
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
                version: 1,
                bytes: Vec::new(),
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports: message.value.ports.clone(),
            };
            if let Err(error) = context.dispatch_worker_registry(|_, hub| {
                hub.dispose_queued_value(Self::principal(&owner), value)
                    .map_err(error)
            }) {
                self.state()?
                    .messages
                    .entry(owner.clone())
                    .or_default()
                    .push_back(message);
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub(crate) fn retire_page(&self, page: oryn_common::v3::PageId) -> Result<(), WorkerError> {
        for ((owner, _), cancel) in self
            .navigation_fetches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
        {
            if *owner == page {
                cancel.store(true, Ordering::Release);
            }
        }
        let owners = {
            let state = self.state()?;
            state
                .observed
                .keys()
                .chain(state.clients.keys())
                .chain(state.client_ids.keys())
                .chain(state.messages.keys())
                .filter(|owner| matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        let mut failure = None;
        for owner in owners {
            if let Err(error) = self.retire_client(&owner) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub(crate) fn retire_client(&self, owner: &AgentOwner) -> Result<(), WorkerError> {
        if let Some(context) = self.context.upgrade() {
            context.retire_worker_secure_context(owner);
            let client = self.state()?.client_ids.get(owner).cloned();
            if let Some(client) = client
                && let Ok(coordinator) = context.context_store().service_coordinator()
            {
                coordinator.versions.retire_client(&client, &self.weak_self);
            }
        }
        {
            let mut state = self.state()?;
            if let Some(lease) = state.cache_owners.remove(owner) {
                state.cache_cleanup.push(lease);
            }
        }
        let cache_cleanup = self.reap_cache_owners();
        let (registration, messages) = {
            let mut state = self.state()?;
            state
                .operations
                .retain(|_, operation| &operation.owner != owner);
            state.observed.remove(owner);
            state.client_ids.remove(owner);
            state.client_leases.remove(owner);
            (
                state.clients.remove(owner),
                state.messages.remove(owner).unwrap_or_default(),
            )
        };
        let cleanup = self.dispose_messages(owner.clone(), messages);
        let activation = if !self.retired.load(Ordering::Acquire) {
            if let Some(id) = registration {
                self.try_activate(id)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        let idle = self.cleanup_unregistered();
        cleanup.and(activation).and(idle).and(cache_cleanup)
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn fetch(
        &self,
        owner: &AgentOwner,
        request: AgentFetchRequest,
    ) -> Result<Option<AgentFetchResponse>, WorkerError> {
        self.fetch_with_cancel(owner, request, None)
    }
    pub(crate) fn fetch_with_cancel(
        &self,
        owner: &AgentOwner,
        request: AgentFetchRequest,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<AgentFetchResponse>, WorkerError> {
        self.fetch_cancellable(owner, request, &|| {
            cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire))
        })
    }
    pub(crate) fn fetch_cancellable(
        &self,
        owner: &AgentOwner,
        mut request: AgentFetchRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<AgentFetchResponse>, WorkerError> {
        if cancel() {
            return Err(
                WorkerError::new("AbortError", "service fetch cancelled before admission")
                    .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Cancelled),
            );
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
        // A client identifier is issued from the exact live native owner;
        // request metadata never supplies authority or an arbitrary client ID.
        request.client_id = self.client_snapshot(owner)?.id;
        request.id = executor.id()?;
        let id = request.id;
        let agent = lease.agent;
        match executor
            .dispatch_reserved_with_cancel(
                AgentCommand::Fetch(request),
                id,
                None,
                lease,
                Some(cancel),
            )
            .map_err(|failure| service_task_stage(failure, "fetch-dispatch", agent, id))?
        {
            AgentEvent::FetchFallback { .. } => Ok(None),
            AgentEvent::FetchResponse { response, .. } => response.map(Some).map_err(|failure| {
                // An accepted respondWith rejection is a fetch outcome, unlike
                // dispatch/admission failures returned before this match.
                let failure = if failure.script_failure.is_some() {
                    failure
                } else {
                    failure.with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Fetch)
                };
                service_task_stage(failure, "fetch-completion", agent, id)
            }),
            _ => Err(service_task_stage(
                error("invalid service fetch completion"),
                "fetch-completion",
                agent,
                id,
            )),
        }
    }
    pub(crate) fn cancel_navigation(&self, page: oryn_common::v3::PageId, request: u64) {
        if let Some(cancel) = self
            .navigation_fetches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(page, request))
        {
            cancel.store(true, Ordering::Release);
        }
    }
    fn registration_is_stale(last_check: u64, now: u64) -> bool {
        last_check > now || now.saturating_sub(last_check) > 24 * 60 * 60 * 1000
    }
    fn schedule_stale_update(
        self: &Arc<Self>,
        source: SoftUpdateSource,
    ) -> Result<(), WorkerError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| error("service update clock predates epoch"))?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let stale = {
            let state = self.state()?;
            state
                .registrations
                .get(&source.registration)
                .is_some_and(|registration| {
                    !registration.unregistered
                        && registration.partition == source.partition
                        && registration.origin == source.origin.authority
                        && registration.snapshot.active.is_some()
                        && Self::registration_is_stale(registration.last_update_check_ms, now)
                })
        };
        if stale && let Some(network) = self.context()?.shared_network_broker() {
            self.schedule_soft_update_from_source(source, network)?;
        }
        Ok(())
    }
    fn schedule_soft_update(
        self: &Arc<Self>,
        agent: AgentId,
        network: SharedNetworkBroker,
    ) -> Result<(), WorkerError> {
        let context = self.context()?;
        let bootstrap = context.worker_bootstrap(agent)?;
        let mut source = SoftUpdateSource::from_bootstrap(&bootstrap)
            .ok_or_else(|| error("soft update registration unavailable"))?;
        source.authority = Self::capture_job_authority(&context, &AgentOwner::Agent(agent))?;
        self.schedule_soft_update_from_source(source, network)
    }
    fn schedule_soft_update_from_source(
        self: &Arc<Self>,
        source: SoftUpdateSource,
        network: SharedNetworkBroker,
    ) -> Result<(), WorkerError> {
        let context = self.context()?;
        let SoftUpdateSource {
            agent,
            registration,
            partition,
            origin,
            policy,
            authority,
        } = source;
        let mut updates = self
            .updates
            .lock()
            .map_err(|_| error("service update queue poisoned"))?;
        let operation = self.id()?;
        {
            let mut state = self.state()?;
            if state.soft_updates.contains(&registration) {
                return Ok(());
            }
            if state.soft_updates.len() >= 128 {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "soft update capacity",
                ));
            }
            if !state
                .registrations
                .get(&registration)
                .is_some_and(|registration| !registration.unregistered)
            {
                return Ok(());
            }
            state.soft_updates.insert(registration);
        }
        let shared = (|| {
            let identity = self
                .job_identity(
                    &ServiceRequest::Update {
                        creator: Creator::Agent(agent),
                        registration,
                    },
                    &partition,
                    &origin,
                    &policy,
                )?
                .ok_or_else(|| error("soft update registration retired"))?;
            let coordinator = context
                .context_store()
                .service_coordinator()
                .map_err(|failure| error(failure.to_string()))?;
            coordinator
                .equivalent
                .admit(identity.0, identity.1, Box::new(|_| {}))
        })();
        match shared {
            Ok(Some(profile)) => {
                self.state()?.equivalent_jobs.insert(
                    operation,
                    EquivalentJob {
                        settled: false,
                        authority,
                        profile: Some(profile),
                    },
                );
            }
            Ok(None) => {
                self.state()?.soft_updates.remove(&registration);
                return Ok(());
            }
            Err(failure) => {
                self.state()?.soft_updates.remove(&registration);
                return Err(failure);
            }
        }
        updates.insert(operation);
        drop(updates);
        let ticket = UpdateTicket {
            registry: self.clone(),
            operation: Some(operation),
        };
        struct SoftUpdateTicket {
            registry: Arc<ServiceRegistry>,
            registration: RegistrationId,
        }
        impl Drop for SoftUpdateTicket {
            fn drop(&mut self) {
                if let Ok(mut state) = self.registry.state() {
                    state.soft_updates.remove(&self.registration);
                }
            }
        }
        let soft = SoftUpdateTicket {
            registry: self.clone(),
            registration,
        };
        let registry = self.clone();
        let job = std::thread::Builder::new()
            .name("oryn-service-soft-update".into())
            .spawn(move || {
                let _ticket = ticket;
                let _soft = soft;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    registry.wait_profile_turn(operation)?;
                    registry.wait_update_turn(operation)?;
                    let (url, scope, script_type, cache) = {
                        let state = registry.state()?;
                        let registration = state
                            .registrations
                            .get(&registration)
                            .filter(|registration| !registration.unregistered)
                            .ok_or_else(|| error("soft update registration retired"))?;
                        (
                            registration.script_url.clone(),
                            Some(registration.snapshot.scope.clone()),
                            registration.script_type,
                            registration.snapshot.update_via_cache.clone(),
                        )
                    };
                    registry.update(ServiceUpdateInputs {
                        operation,
                        owner: AgentOwner::Agent(agent),
                        network,
                        partition,
                        origin,
                        policy,
                        script: url,
                        scope,
                        script_type,
                        cache,
                    })
                }))
                .unwrap_or_else(|_| Err(error("service soft update panicked")));
                registry.publish_job_result(operation, result);
            })
            .map_err(|failure| error(failure.to_string()))?;
        self.jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(job);
        Ok(())
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn intercept_navigation(
        &self,
        page: oryn_common::v3::PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
    ) -> Result<Option<crate::network::NetworkResponse>, WorkerError> {
        self.intercept_navigation_timed(
            page,
            request,
            network,
            &crate::network::ServiceWorkerTiming::default(),
        )
    }
    pub(crate) fn intercept_navigation_timed(
        &self,
        page: oryn_common::v3::PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
        timing: &crate::network::ServiceWorkerTiming,
    ) -> Result<Option<crate::network::NetworkResponse>, WorkerError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let key = (page, request.request_id);
        {
            let mut pending = self
                .navigation_fetches
                .lock()
                .map_err(|_| error("navigation fetch ownership poisoned"))?;
            if pending.len() >= 256 || pending.contains_key(&key) {
                return Err(error("duplicate or excess navigation fetch"));
            }
            pending.insert(key, cancel.clone());
        }
        let _wait = NavigationFetchWait {
            table: self.navigation_fetches.clone(),
            key,
        };
        let context = self.context()?;
        let target = url::Url::parse(&request.url).map_err(|failure| error(failure.to_string()))?;
        let partition = context.service_navigation_partition(page, &target)?;
        let native_origin = oryn_js::origin::Origin::from_url(target.as_str());
        let origin = native_origin.security_token_key();
        let native = AgentOrigin {
            serialized: native_origin.serialize(),
            authority: origin.clone(),
            opaque: native_origin.is_opaque(),
        };
        let referrer_policy = context
            .page_referrer_policy(page)
            .map_err(|failure| error(failure.to_string()))?;
        let prepared = self.prepare_navigation_preload(
            &partition,
            &native,
            request,
            network.clone(),
            cancel.clone(),
            referrer_policy,
        )?;
        self.restore_namespace(partition.clone(), native, network.clone())?;
        if cancel.load(Ordering::Acquire) {
            return Err(WorkerError::new(
                "AbortError",
                "navigation fetch cancelled during restoration",
            ));
        }
        let selected = {
            let state = self.state()?;
            state
                .registrations
                .values()
                .filter(|registration| {
                    Self::visible(registration, &partition, &origin)
                        && target.as_str().starts_with(&registration.snapshot.scope)
                })
                .max_by_key(|registration| registration.snapshot.scope.len())
                .and_then(|registration| registration.snapshot.active.as_ref())
                .filter(|worker| worker.state == WorkerState::Activated)
                .cloned()
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        timing.selected();
        let (executor, lease) = self.fetch_executor(&selected)?;
        let _ = executor.schedule_soft_update(lease.agent, network.clone());
        let id = executor.id()?;
        let enabled = if let Some(prepared) = prepared {
            executor.arm_navigation_preload(lease.agent, id, prepared)?;
            true
        } else {
            executor.prepare_preload(
                lease.agent,
                id,
                request,
                network.clone(),
                cancel.clone(),
                referrer_policy,
            )?
        };
        let fetch = AgentFetchRequest {
            keepalive: false,
            privileged_range: false,
            cache: request.security.fetch_cache,
            navigation: request.security.navigation,
            integrity: String::new(),
            navigation_preload: enabled,
            streaming: false,
            id,
            client_id: String::new(),
            url: request.url.clone(),
            method: request.method.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            destination: if request.security.kind == crate::cookies::CookieRequestKind::Frame {
                "iframe".into()
            } else {
                "document".into()
            },
            mode: "navigate".into(),
            credentials: "include".into(),
            redirect: "manual".into(),
        };
        let response = match executor.dispatch_reserved_with_cancel(
            AgentCommand::Fetch(fetch),
            id,
            None,
            lease,
            Some(&|| cancel.load(Ordering::Acquire)),
        )? {
            AgentEvent::FetchFallback { .. } => return Ok(None),
            AgentEvent::FetchResponse { response, .. } => response?,
            _ => return Err(error("invalid navigation fetch event response")),
        };
        if response.status == 0 || matches!(response.response_type.as_str(), "error" | "opaque") {
            return Err(WorkerError::new(
                "NetworkError",
                "service navigation returned unusable response",
            ));
        }
        Ok(Some(crate::network::NetworkResponse {
            response_policy: None,
            timing: crate::network::service_timing_provenance(response.timing.as_ref()),
            request_id: request.request_id,
            status: response.status,
            headers: response.headers,
            body: response.body,
        }))
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn intercept_navigation_operation(
        &self,
        page: oryn_common::v3::PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
    ) -> Result<Option<crate::network::NetworkOperation>, WorkerError> {
        self.intercept_navigation_operation_timed(
            page,
            request,
            network,
            &crate::network::ServiceWorkerTiming::default(),
        )
    }
    pub(crate) fn intercept_navigation_operation_timed(
        &self,
        page: oryn_common::v3::PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
        timing: &crate::network::ServiceWorkerTiming,
    ) -> Result<Option<crate::network::NetworkOperation>, WorkerError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let key = (page, request.request_id);
        {
            let mut pending = self
                .navigation_fetches
                .lock()
                .map_err(|_| error("navigation fetch ownership poisoned"))?;
            if pending.len() >= 256 || pending.contains_key(&key) {
                return Err(error("duplicate or excess navigation fetch"));
            }
            pending.insert(key, cancel.clone());
        }
        let wait = NavigationFetchWait {
            table: self.navigation_fetches.clone(),
            key,
        };
        let context = self.context()?;
        let target = url::Url::parse(&request.url).map_err(|failure| error(failure.to_string()))?;
        let partition = context.service_navigation_partition(page, &target)?;
        let native_origin = oryn_js::origin::Origin::from_url(target.as_str());
        let origin = native_origin.security_token_key();
        let native = AgentOrigin {
            serialized: native_origin.serialize(),
            authority: origin.clone(),
            opaque: native_origin.is_opaque(),
        };
        let referrer_policy = context
            .page_referrer_policy(page)
            .map_err(|failure| error(failure.to_string()))?;
        let prepared = self.prepare_navigation_preload(
            &partition,
            &native,
            request,
            network.clone(),
            cancel.clone(),
            referrer_policy,
        )?;
        self.restore_namespace(partition.clone(), native, network.clone())?;
        if cancel.load(Ordering::Acquire) {
            return Err(WorkerError::new(
                "AbortError",
                "navigation fetch cancelled during restoration",
            ));
        }
        let selected = {
            let state = self.state()?;
            state
                .registrations
                .values()
                .filter(|registration| {
                    Self::visible(registration, &partition, &origin)
                        && target.as_str().starts_with(&registration.snapshot.scope)
                })
                .max_by_key(|registration| registration.snapshot.scope.len())
                .and_then(|registration| registration.snapshot.active.as_ref())
                .filter(|worker| worker.state == WorkerState::Activated)
                .cloned()
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        timing.selected();
        let (executor, lease) = self.fetch_executor(&selected)?;
        let _ = executor.schedule_soft_update(lease.agent, network.clone());
        let id = executor.id()?;
        let enabled = if let Some(prepared) = prepared {
            executor.arm_navigation_preload(lease.agent, id, prepared)?;
            true
        } else {
            executor.prepare_preload(
                lease.agent,
                id,
                request,
                network.clone(),
                cancel.clone(),
                referrer_policy,
            )?
        };
        let fetch = AgentFetchRequest {
            keepalive: false,
            privileged_range: false,
            cache: request.security.fetch_cache,
            navigation: request.security.navigation,
            integrity: String::new(),
            navigation_preload: enabled,
            streaming: false,
            id,
            client_id: String::new(),
            url: request.url.clone(),
            method: request.method.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            destination: if request.security.kind == crate::cookies::CookieRequestKind::Frame {
                "iframe".into()
            } else {
                "document".into()
            },
            mode: "navigate".into(),
            credentials: "include".into(),
            redirect: "manual".into(),
        };
        let Some((response, operation)) = executor.fetch_operation_reserved(
            fetch,
            request.request_id,
            cancel.clone(),
            lease,
            Some(wait),
            None,
        )?
        else {
            return Ok(None);
        };
        if response.status == 0 || matches!(response.response_type.as_str(), "error" | "opaque") {
            cancel.store(true, Ordering::Release);
            return Err(WorkerError::new(
                "NetworkError",
                "service navigation returned unusable response",
            ));
        }
        Ok(Some(operation))
    }
    pub(crate) fn retire(&self) -> Result<(), WorkerError> {
        self.retired.store(true, Ordering::Release);
        self.updates_changed.notify_all();
        self.maintenance
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        if let Some(context) = self.context.upgrade()
            && let Ok(coordinator) = context.context_store().service_coordinator()
        {
            coordinator.versions.retire_registry(&self.weak_self);
        }
        let preloads = std::mem::take(
            &mut *self
                .preloads
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        drop(preloads);
        for cancel in self
            .navigation_fetches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
        {
            cancel.store(true, Ordering::Release);
        }
        {
            let mut state = self.state()?;
            let owners = std::mem::take(&mut state.cache_owners);
            state.cache_cleanup.extend(owners.into_values());
            state.activation_watches.clear();
            state.client_leases.clear();
            state.clients.clear();
            state.remote_versions.clear();
        }
        let cache_cleanup = self.reap_cache_owners();
        let messages = std::mem::take(&mut self.state()?.messages);
        let mut failure = None;
        for (owner, messages) in messages {
            if let Err(error) = self.dispose_messages(owner, messages) {
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = self.reap_orphaned_services() {
            failure.get_or_insert(error);
        }
        let agents = self.state()?.versions.keys().copied().collect::<Vec<_>>();
        for agent in agents {
            // An existing background owner may be finishing the same process.
            // Join retained work below, then verify every remaining version.
            if let Err(error) = self.retire_version_if_idle(agent) {
                failure.get_or_insert(error);
            }
        }
        let mut jobs = self
            .jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline = Instant::now() + Duration::from_secs(5);
        while jobs.iter().any(|job| !job.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut index = 0;
        while index < jobs.len() {
            if jobs[index].is_finished() {
                if jobs.swap_remove(index).join().is_err() {
                    failure.get_or_insert_with(|| error("service job panicked"));
                }
            } else {
                index += 1;
            }
        }
        if !jobs.is_empty() {
            failure.get_or_insert_with(|| error("service jobs remain owned during retirement"));
        }
        drop(jobs);
        if let Err(error) = self.drain_stream_cleanup(Duration::from_secs(2)) {
            failure.get_or_insert(error);
        }
        if let Err(error) = self.drain_preload_owners(Duration::from_secs(2)) {
            failure.get_or_insert(error);
        }
        let remaining = self.state()?.versions.keys().copied().collect::<Vec<_>>();
        for agent in remaining {
            match self.retire_version_if_idle(agent) {
                Ok(true) => {}
                Ok(false) => {
                    failure.get_or_insert_with(|| error("service retirement remains owned"));
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), Err).and(cache_cleanup)
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    #[test]
    fn error_reports_preserve_diagnostics_without_replacing_task_outcomes() {
        let reported = AgentEvent::Error {
            message: "Uncaught Error: runtime".encode_utf16().collect(),
            filename: "https://example.test/sw.js".into(),
            line: 7,
            column: 3,
        };
        let mut startup = ServiceControlInbox::default();
        assert!(startup.accept_runtime_marker(&reported));
        assert!(
            startup.fatal.is_none(),
            "an unrelated timer error can occur while module evaluation is pending"
        );
        startup.accept_runtime_marker(&AgentEvent::Ready);
        assert!(startup.ready);
        assert_eq!(startup.runtime_errors.len(), 1);
        let mut active = ServiceControlInbox::default();
        active.accept_runtime_marker(&AgentEvent::Ready);
        active.accept_runtime_marker(&reported);
        assert!(active.fatal.is_none());
        assert_eq!(active.runtime_errors.len(), 1);
        assert!(active.runtime_errors[0].message.contains("sw.js:7:3"));
        let failed = AgentEvent::LifecycleFinished {
            id: 19,
            result: Err(WorkerError::new("AbortError", "waitUntil rejected")),
        };
        assert!(
            !active.accept_runtime_marker(&failed),
            "explicit lifetime rejection remains a task result"
        );
        for _ in 0..32 {
            active.accept_runtime_marker(&reported);
        }
        assert_eq!(active.runtime_errors.len(), 16);
        assert_eq!(active.dropped_runtime_errors, 17);
    }
    #[test]
    fn service_runtime_diagnostic_preserves_error_and_native_location() {
        let diagnostic = service_script_diagnostic(
            &"Uncaught InvalidStateError: rejected"
                .encode_utf16()
                .collect::<Vec<_>>(),
            "https://worker.test/sw.js",
            17,
            9,
        );
        assert_eq!(diagnostic.name, "TypeError");
        assert_eq!(
            diagnostic.message,
            "Uncaught InvalidStateError: rejected [service script https://worker.test/sw.js:17:9]"
        );
    }
    #[test]
    fn service_runtime_diagnostics_bound_unicode_payload_and_retention() {
        let message = "🦀".repeat(16_384).encode_utf16().collect::<Vec<_>>();
        let filename = "é".repeat(8192);
        let diagnostic = service_script_diagnostic(&message, &filename, 9, 4);
        assert!(diagnostic.message.contains("[truncated]"));
        assert!(diagnostic.message.ends_with(":9:4]"));
        assert!(diagnostic.message.len() < 32 * 1024);
        let reported = AgentEvent::Error {
            message,
            filename,
            line: 9,
            column: 4,
        };
        let mut inbox = ServiceControlInbox::default();
        for _ in 0..100 {
            inbox.accept_runtime_marker(&reported);
        }
        assert_eq!(inbox.runtime_errors.len(), 16);
        assert_eq!(inbox.dropped_runtime_errors, 84);
        assert!(inbox.runtime_error_bytes <= 512 * 1024);
        assert!(inbox.fatal.is_none());
    }
    #[test]
    fn retired_registry_still_accepts_idempotent_native_client_cleanup() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let registry = ServiceRegistry::new(&context);
        let owner = AgentOwner::Window {
            page: oryn_common::v3::PageId(999),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        registry.retire().unwrap();
        for _ in 0..2 {
            registry.retire_client(&owner).unwrap();
        }
        assert!(
            registry.context().is_err(),
            "cleanup does not reopen service admission"
        );
        assert!(registry.state().unwrap().clients.is_empty());
    }
    #[test]
    fn service_event_drop_queues_cleanup_without_waiting_for_registry_or_lifecycle() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let registry = ServiceRegistry::new(&context);
        let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let lease = ServiceEventLease {
            check_soft_update: true,
            soft_update_source: None,
            activity: None,
            registry: Arc::downgrade(&registry),
            agent: AgentId(999),
            serial: Arc::new(Mutex::new(())),
            inbox: Arc::new(Mutex::new(ServiceControlInbox::default())),
            inflight: inflight.clone(),
        };
        let held = registry.state.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(lease);
            tx.send(()).unwrap();
        });
        let completed = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(held);
        dropper.join().unwrap();
        assert!(completed.is_ok(), "event Drop blocked on lifecycle work");
        assert_eq!(inflight.load(Ordering::Acquire), 0);
    }
    #[test]
    fn failed_service_maintenance_preserves_exact_retry_ticket() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let registry = ServiceRegistry::new(&context);
        drop(context);
        let ticket = Arc::new(());
        registry
            .maintenance
            .lock()
            .unwrap()
            .insert(AgentId(998), (ticket.clone(), None));
        assert!(registry.run_maintenance().is_err());
        assert!(
            registry
                .maintenance
                .lock()
                .unwrap()
                .get(&AgentId(998))
                .is_some_and(|(retained, _)| Arc::ptr_eq(retained, &ticket))
        );
    }

    #[test]
    fn orphan_retirement_admits_one_owner_and_preserves_failed_or_panicked_cleanup() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let registry = ServiceRegistry::new(&context);
        let agent = AgentId(73);
        registry.state().unwrap().orphaned_services.insert(agent);
        assert!(
            registry
                .retire_orphan_with(agent, || {
                    assert!(
                        registry.state.try_lock().is_ok(),
                        "teardown must run without the registry lock"
                    );
                    assert!(
                        registry
                            .retire_orphan_with(agent, || panic!("duplicate receipt consumption"))
                            .is_err()
                    );
                    Err(error("retryable process stop failure"))
                })
                .is_err()
        );
        assert!(registry.state().unwrap().orphaned_services.contains(&agent));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.retire_orphan_with(agent, || panic!("process stop panic"))
        }));
        assert!(result.is_err());
        assert!(registry.state().unwrap().retiring_orphans.is_empty());
        registry.retire_orphan_with(agent, || Ok(())).unwrap();
        assert!(!registry.state().unwrap().orphaned_services.contains(&agent));
        registry
            .retire_orphan_with(agent, || panic!("completed teardown repeated"))
            .unwrap();
    }

    #[test]
    fn update_tickets_preserve_admission_order_and_release_after_job_exit() {
        let context = crate::runtime::Browser::in_process_with_config(
            crate::runtime::BrowserConfig::default(),
        )
        .new_context();
        let registry = ServiceRegistry::new(&context);
        registry.updates.lock().unwrap().extend([1, 2]);
        let first = UpdateTicket {
            registry: registry.clone(),
            operation: Some(1),
        };
        let second = UpdateTicket {
            registry: registry.clone(),
            operation: Some(2),
        };
        let (done, received) = std::sync::mpsc::channel();
        let worker = registry.clone();
        let thread = std::thread::spawn(move || {
            worker.wait_update_turn(2).unwrap();
            done.send(()).unwrap();
            drop(second);
        });
        registry.wait_update_turn(1).unwrap();
        assert!(received.try_recv().is_err());
        drop(first);
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        thread.join().unwrap();
        assert!(registry.updates.lock().unwrap().is_empty());
        registry.updates.lock().unwrap().insert(3);
        let ticket = UpdateTicket {
            registry: registry.clone(),
            operation: Some(3),
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ticket = ticket;
            panic!("simulated job panic")
        }));
        assert!(registry.updates.lock().unwrap().is_empty());
    }
    use super::*;
    use crate::{
        network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse},
        runtime::{Browser, BrowserConfig},
    };
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    struct CacheScripts {
        requests: Arc<Mutex<Vec<(String, bool)>>>,
    }
    impl NetworkBroker for CacheScripts {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let path = url::Url::parse(&request.url).unwrap().path().to_string();
            let revalidate = request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("cache-control") && value == "no-cache"
            });
            if path == "/sw.js" {
                assert!(
                    request
                        .headers
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("service-worker")
                            && value == "script")
                );
            }
            self.requests
                .lock()
                .unwrap()
                .push((path.clone(), revalidate));
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("cache-control".into(), "max-age=86400".into()),
                ],
                body: if path == "/sw.js" {
                    b"importScripts('/import.js');".to_vec()
                } else {
                    b"self.imported=1;".to_vec()
                },
            })
        }
    }
    #[test]
    fn service_update_cache_modes_control_root_import_and_daily_revalidation() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(crate::network::MemoryCachingBroker::new(
            CacheScripts {
                requests: requests.clone(),
            },
            "service-test",
            16,
            1024 * 1024,
            128 * 1024,
        ));
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let update = |cache: &str| {
            registry
                .update(ServiceUpdateInputs {
                    operation: 0,
                    owner: owner.clone(),
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone(),
                    script: "/sw.js".into(),
                    scope: Some("/".into()),
                    script_type: ScriptType::Classic,
                    cache: cache.into(),
                })
                .unwrap()
        };
        update("all");
        assert_eq!(
            *requests.lock().unwrap(),
            vec![("/sw.js".into(), false), ("/import.js".into(), false)]
        );
        update("all");
        assert_eq!(requests.lock().unwrap().len(), 2);
        update("imports");
        assert_eq!(
            requests.lock().unwrap().last().unwrap(),
            &("/sw.js".into(), true)
        );
        assert_eq!(requests.lock().unwrap().len(), 3);
        update("none");
        assert_eq!(
            &requests.lock().unwrap()[3..],
            &[("/sw.js".into(), true), ("/import.js".into(), true)]
        );
        let registrations = registry
            .state()
            .unwrap()
            .registrations
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in registrations {
            registry
                .persist_registration_with_update(id, |registration| {
                    registration.last_update_check_ms = 0;
                })
                .unwrap();
        }
        update("all");
        assert_eq!(
            &requests.lock().unwrap()[5..],
            &[("/sw.js".into(), true), ("/import.js".into(), true)]
        );
        let before = requests.lock().unwrap().len();
        let result = registry
            .update_job(ServiceUpdateJobInputs {
                operation: 0,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: "/sw.js".into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
                register: true,
            })
            .unwrap();
        assert_eq!(
            requests.lock().unwrap().len(),
            before,
            "equivalent register is not an explicit update fetch"
        );
        let ServiceResult::Registration(Some(registration)) = result else {
            panic!("registration result")
        };
        assert_eq!(registration.update_via_cache, "none");
        {
            let agent = registration.active.as_ref().unwrap().id;
            let bootstrap = context.worker_bootstrap(agent).unwrap();
            let (scope, key) = registry
                .job_identity(
                    &ServiceRequest::Update {
                        creator: Creator::Agent(agent),
                        registration: registration.id,
                    },
                    &bootstrap.partition,
                    &bootstrap.origin,
                    &bootstrap.policy,
                )
                .unwrap()
                .unwrap();
            let coordinator = context.context_store().service_coordinator().unwrap();
            let delivered = Arc::new(Mutex::new(Vec::new()));
            let recipient = delivered.clone();
            let leader = coordinator
                .equivalent
                .admit(
                    scope,
                    key,
                    Box::new(move |result| {
                        recipient
                            .lock()
                            .unwrap()
                            .push(matches!(result, Ok((ServiceResult::Bool(true), None))));
                    }),
                )
                .unwrap()
                .unwrap();
            registry
                .schedule_soft_update(agent, network.clone())
                .unwrap();
            assert!(
                !registry
                    .state()
                    .unwrap()
                    .soft_updates
                    .contains(&registration.id),
                "joining an equivalent profile job releases the local soft-update marker"
            );
            assert_eq!(
                requests.lock().unwrap().len(),
                before,
                "an equivalent in-flight update must not start another script fetch"
            );
            leader.publish(Ok((ServiceResult::Bool(true), None)));
            leader.finish();
            assert_eq!(*delivered.lock().unwrap(), vec![true]);
        }
        registry
            .schedule_soft_update(registration.active.as_ref().unwrap().id, network.clone())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !registry.state().unwrap().soft_updates.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(registry.state().unwrap().soft_updates.is_empty());
        assert_eq!(
            requests.lock().unwrap().len(),
            before + 2,
            "soft update revalidates root and import using stored none mode"
        );
        registry
            .persist_registration_with_update(registration.id, |registration| {
                registration.last_update_check_ms = 0;
            })
            .unwrap();
        let before = requests.lock().unwrap().len();
        registry
            .lifecycle(
                registration.active.as_ref().unwrap().id,
                LifecycleEvent::Push,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while registry.maintenance_running.load(Ordering::Acquire)
            || !registry.maintenance.lock().unwrap().is_empty()
            || !registry.state().unwrap().soft_updates.is_empty()
        {
            assert!(
                Instant::now() < deadline,
                "functional-event maintenance did not finish"
            );
            registry.reap_jobs().unwrap();
            std::thread::yield_now();
        }
        assert_eq!(
            requests.lock().unwrap().len(),
            before + 2,
            "stale functional-event completion schedules root and import revalidation"
        );
        let operation = registry.id().unwrap();
        let authority = ServiceRegistry::capture_job_authority(&context, &owner).unwrap();
        registry.state().unwrap().equivalent_jobs.insert(
            operation,
            EquivalentJob {
                settled: false,
                authority,
                profile: None,
            },
        );
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        registry.retire_client(&owner).unwrap();
        assert!(context.retained_worker_authority(&owner).is_err());
        registry
            .update(ServiceUpdateInputs {
                operation,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: "/sw.js".into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
            })
            .unwrap();
        let before = requests.lock().unwrap().len();
        assert!(
            registry
                .update(ServiceUpdateInputs {
                    operation: 0,
                    owner: owner.clone(),
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone(),
                    script: "/sw.js".into(),
                    scope: Some("/".into()),
                    script_type: ScriptType::Classic,
                    cache: "none".into()
                })
                .is_err()
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            before,
            "raw compatibility caller cannot borrow admitted authority after retirement"
        );
        registry.state().unwrap().equivalent_jobs.remove(&operation);
        registry.retire().unwrap();
    }

    struct TypedImportScripts {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for TypedImportScripts {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            let (status, headers, body) = if request.url.ends_with("/sw.js") {
                (200,vec![("content-type".into(),"text/javascript".into()),("referrer-policy".into(),"unsafe-url".into())],br#"import data from './alias.json' with {type:'json'}; if(data.answer!==42)throw Error('bad JSON import');onfetch=e=>e.respondWith(new Response(String(data.answer)));"#.to_vec())
            } else if request.url.ends_with("/alias.json") {
                (
                    302,
                    vec![("location".into(), "/data/config.json".into())],
                    Vec::new(),
                )
            } else {
                (
                    200,
                    vec![
                        ("content-type".into(), "application/json".into()),
                        ("referrer-policy".into(), "no-referrer".into()),
                    ],
                    br#"{"answer":42}"#.to_vec(),
                )
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status,
                headers,
                body,
            })
        }
    }
    #[test]
    fn service_update_retains_json_response_provenance_and_never_reinterprets_as_javascript() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(TypedImportScripts {
            requests: requests.clone(),
        });
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let update = || {
            registry
                .update(ServiceUpdateInputs {
                    operation: 0,
                    owner: owner.clone(),
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone(),
                    script: "/sw.js".into(),
                    scope: Some("/".into()),
                    script_type: ScriptType::Module,
                    cache: "none".into(),
                })
                .unwrap()
        };
        update();
        let first = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .active
            .as_ref()
            .unwrap()
            .id;
        let resource = "https://example.test/alias.json";
        let (source, metadata) = registry
            .imported_script(
                first,
                resource,
                crate::agent_script::ScriptLoadKind::JsonModuleImport,
            )
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf16(&source).unwrap(), r#"{"answer":42}"#);
        assert_eq!(
            metadata.response_url,
            "https://example.test/data/config.json"
        );
        assert_eq!(metadata.content_type.as_deref(), Some("application/json"));
        assert_eq!(metadata.referrer_policy, "no-referrer");
        assert_eq!(
            metadata.request_referrer_source.as_deref(),
            Some("https://example.test/sw.js")
        );
        assert_eq!(metadata.request_referrer_policy, "unsafe-url");
        let calls = requests.lock().unwrap().len();
        assert_eq!(
            registry
                .imported_script(
                    first,
                    resource,
                    crate::agent_script::ScriptLoadKind::ModuleImport
                )
                .err()
                .unwrap()
                .script_failure,
            Some(oryn_js::worker_wire::ScriptFailureKind::Fetch)
        );
        assert_eq!(requests.lock().unwrap().len(), calls);
        assert_eq!(
            registry
                .record_import(first, resource.into(), source, [0; 32], metadata.clone())
                .err()
                .unwrap()
                .script_failure,
            Some(oryn_js::worker_wire::ScriptFailureKind::Infrastructure)
        );
        assert_eq!(
            registry
                .imported_script(
                    first,
                    "https://example.test/not-installed.js",
                    crate::agent_script::ScriptLoadKind::ModuleImport
                )
                .err()
                .unwrap()
                .script_failure,
            Some(oryn_js::worker_wire::ScriptFailureKind::Fetch)
        );
        update();
        let second = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .active
            .as_ref()
            .unwrap()
            .id;
        assert_eq!(
            first, second,
            "unchanged JSON graph must remain the same logical installation"
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.ends_with("/alias.json"))
                .count(),
            2
        );
        assert!(
            requests
                .iter()
                .filter(|request| request.url.ends_with("/alias.json"))
                .all(|request| request
                    .headers
                    .contains(&("referer".into(), "https://example.test/sw.js".into())))
        );
        drop(requests);
        registry.retire().unwrap();
    }

    struct ByteChangedScript {
        calls: AtomicU64,
    }
    impl NetworkBroker for ByteChangedScript {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let count = self.calls.fetch_add(1, Ordering::AcqRel);
            let mut body = Vec::new();
            if count != 0 {
                body.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            body.extend_from_slice(
                b"onfetch=e=>e.respondWith(new Response('same decoded source'));",
            );
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/javascript".into())],
                body,
            })
        }
    }
    #[test]
    fn raw_service_resource_change_updates_even_when_decoded_script_is_identical() {
        let network = SharedNetworkBroker::new(ByteChangedScript {
            calls: AtomicU64::new(0),
        });
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let update = || {
            registry
                .update(ServiceUpdateInputs {
                    operation: 0,
                    owner: owner.clone(),
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone(),
                    script: "/sw.js".into(),
                    scope: Some("/".into()),
                    script_type: ScriptType::Classic,
                    cache: "none".into(),
                })
                .unwrap()
        };
        update();
        let first = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .clone();
        let first = first.waiting.as_ref().or(first.active.as_ref()).unwrap().id;
        update();
        let second = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .clone();
        let second = second.waiting.as_ref().or(second.active.as_ref()).unwrap().id;
        assert_ne!(
            first, second,
            "UTF-8 BOM changes raw bytes even though native decoding yields identical JavaScript"
        );
        update();
        let third = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .clone();
        let third = third.waiting.as_ref().or(third.active.as_ref()).unwrap().id;
        assert_eq!(second, third);
        registry.retire().unwrap();
    }
    struct PreloadNetwork {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for PreloadNetwork {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            let script = request.url.ends_with("/sw.js");
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![(
                    "content-type".into(),
                    if script {
                        "text/javascript"
                    } else {
                        "text/plain"
                    }
                    .into(),
                )],
                body: if script {
                    br#"onactivate=e=>e.waitUntil(registration.navigationPreload.enable());onfetch=e=>e.respondWith(e.preloadResponse.then(response=>response||new Response('disabled')));"#.to_vec()
                } else {
                    b"native-preloaded-body".to_vec()
                },
            })
        }
    }
    #[test]
    fn navigation_preload_is_streamed_through_exact_service_event_and_disabled_without_network() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(PreloadNetwork {
            requests: requests.clone(),
        });
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        registry
            .update(ServiceUpdateInputs {
                operation: 0,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: "/sw.js".into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
            })
            .unwrap();
        let registration = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .clone();
        assert!(
            registration.navigation_preload.enabled,
            "activate.waitUntil must update native preload configuration without deadlocking activation"
        );
        let active = registration.active.as_ref().unwrap().id;
        let request = NetworkRequest {
            request_id: 971,
            method: "GET".into(),
            url: "https://example.test/destination".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: true,
            security: crate::cookies::CookieRequestContext {
                dns_timing: None,
                kind: crate::cookies::CookieRequestKind::Document,
                ..Default::default()
            },
            resolved_addrs: Vec::new(),
        };
        assert!(
            registry
                .prepare_preload(
                    active,
                    9700,
                    &request,
                    network.clone(),
                    Arc::new(AtomicBool::new(false)),
                    Default::default()
                )
                .unwrap()
        );
        assert!(registry.take_preload(AgentId(u64::MAX), 9700).is_err());
        let mut claimed = registry.take_preload(active, 9700).unwrap();
        assert!(registry.take_preload(active, 9700).is_err());
        claimed
            .request
            .headers
            .push(("referer".into(), "https://source.test/private/path".into()));
        claimed.redirect_referrer(&[], &url::Url::parse("https://other.test/next").unwrap());
        assert_eq!(
            claimed
                .request
                .headers
                .iter()
                .find(|(name, _)| name == "referer")
                .unwrap()
                .1,
            "https://source.test/"
        );
        claimed.redirect_referrer(
            &[("referrer-policy".into(), "unsafe-url".into())],
            &url::Url::parse("https://source.test/back").unwrap(),
        );
        assert_eq!(
            claimed
                .request
                .headers
                .iter()
                .find(|(name, _)| name == "referer")
                .unwrap()
                .1,
            "https://source.test/",
            "later policy cannot recreate previously stripped source path"
        );
        claimed.redirect_referrer(
            &[("referrer-policy".into(), "no-referrer".into())],
            &url::Url::parse("https://other.test/none").unwrap(),
        );
        claimed.redirect_referrer(
            &[("referrer-policy".into(), "unsafe-url".into())],
            &url::Url::parse("https://other.test/later").unwrap(),
        );
        assert!(
            !claimed
                .request
                .headers
                .iter()
                .any(|(name, _)| name == "referer"),
            "redirect must never synthesize service-worker or redirect URL as referrer"
        );
        drop(claimed);
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(
            registry
                .prepare_preload(
                    active,
                    9701,
                    &request,
                    network.clone(),
                    cancelled,
                    Default::default()
                )
                .unwrap()
        );
        let cancelled = registry.take_preload(active, 9701).unwrap();
        assert!(cancelled.operation.as_ref().unwrap().is_err());
        drop(cancelled);
        let mut operation = registry
            .intercept_navigation_operation(page.page_id(), &request, network.clone())
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut body = Vec::new();
        loop {
            assert!(Instant::now() < deadline);
            operation.request_body_chunk();
            match operation.try_event() {
                Ok(crate::network::NetworkEvent::Chunk { bytes, .. }) => {
                    body.extend_from_slice(&bytes)
                }
                Ok(crate::network::NetworkEvent::Complete { .. }) => break,
                Ok(crate::network::NetworkEvent::Failed { error, .. }) => panic!("{error}"),
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => std::thread::yield_now(),
                Err(failure) => panic!("{failure}"),
            }
        }
        assert_eq!(body, b"native-preloaded-body");
        let captured = requests.lock().unwrap();
        let preload = captured
            .iter()
            .find(|request| request.url.ends_with("/destination"))
            .unwrap();
        assert!(preload.headers.iter().any(|(name,value)|name=="service-worker-navigation-preload"&&value=="true"));
        assert_ne!(preload.request_id, request.request_id);
        drop(captured);
        assert!(registry.take_preload(active, u64::MAX).is_err());
        registry
            .run(ServiceRunInputs {
                operation: 0,
                owner: owner.clone(),
                request: ServiceRequest::NavigationPreload {
                    creator: creator.clone(),
                    registration: registration.id,
                    operation: NavigationPreloadOperation::Disable,
                },
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
            })
            .unwrap();
        let before = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| !request.url.ends_with("/sw.js"))
            .count();
        let mut disabled = request.clone();
        disabled.request_id = 972;
        let response = registry
            .intercept_navigation(page.page_id(), &disabled, network.clone())
            .unwrap()
            .unwrap();
        assert_eq!(response.body, b"disabled");
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| !request.url.ends_with("/sw.js"))
                .count(),
            before
        );
        for invalid in ["bad\r\nheader", "\u{100}", "\u{1f600}"] {
            assert!(
                registry
                    .run(ServiceRunInputs {
                        operation: 0,
                        owner: owner.clone(),
                        request: ServiceRequest::NavigationPreload {
                            creator: creator.clone(),
                            registration: registration.id,
                            operation: NavigationPreloadOperation::SetHeaderValue(invalid.into())
                        },
                        network: network.clone(),
                        partition: partition.clone(),
                        origin: origin.clone(),
                        policy: policy.clone()
                    })
                    .is_err()
            );
        }
        registry.retire().unwrap();
    }
    #[test]
    fn automatic_update_age_uses_native_daily_threshold_and_clock_rollback() {
        let day = 24 * 60 * 60 * 1000;
        assert!(!ServiceRegistry::registration_is_stale(100, 100 + day));
        assert!(ServiceRegistry::registration_is_stale(100, 101 + day));
        assert!(ServiceRegistry::registration_is_stale(101, 100));
        assert!(!ServiceRegistry::registration_is_stale(100, 100));
    }
    #[test]
    fn equivalent_job_identity_and_retired_leader_preserve_follower_result() {
        let context = Browser::new().new_context();
        let registry = ServiceRegistry::new(&context);
        let origin = AgentOrigin {
            serialized: "https://a.test".into(),
            authority: "https://a.test".into(),
            opaque: false,
        };
        let policy = AgentPolicy {
            hide_nonces: false,
            policy_sources: Vec::new(),
            policy_self_urls: Vec::new(),
            document_self_url: None,
            policy_self_url: None,
            url: "https://a.test/page".into(),
            headers: Vec::new(),
        };
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let registration = |script: &str, cache: &str| ServiceRequest::Register {
            creator: creator.clone(),
            url: script.into(),
            scope: Some("/".into()),
            script_type: ScriptType::Classic,
            update_via_cache: cache.into(),
        };
        let first = registry
            .job_identity(
                &registration("/sw.js", "imports"),
                r#"[1,"site","https","a.test"]"#,
                &origin,
                &policy,
            )
            .unwrap();
        assert_eq!(
            first,
            registry
                .job_identity(
                    &registration("https://a.test/sw.js#fragment", "imports"),
                    r#"[1,"site","https","a.test"]"#,
                    &origin,
                    &policy
                )
                .unwrap()
        );
        assert_ne!(
            first,
            registry
                .job_identity(
                    &registration("/sw.js", "none"),
                    r#"[1,"site","https","a.test"]"#,
                    &origin,
                    &policy
                )
                .unwrap()
        );
        assert_ne!(
            first,
            registry
                .job_identity(
                    &registration("/different.js", "imports"),
                    r#"[1,"site","https","a.test"]"#,
                    &origin,
                    &policy
                )
                .unwrap()
        );
        assert_eq!(
            first,
            registry
                .job_identity(
                    &registration("/sw.js", "imports"),
                    r#"[99,"site","https","a.test"]"#,
                    &origin,
                    &policy
                )
                .unwrap()
        );
        assert_ne!(
            first,
            registry
                .job_identity(
                    &registration("/sw.js", "imports"),
                    r#"[99,"site","https","other.test"]"#,
                    &origin,
                    &policy
                )
                .unwrap()
        );
        let owner = AgentOwner::Agent(AgentId(91));
        {
            let mut state = registry.state().unwrap();
            state.operations.insert(
                2,
                Operation {
                    owner,
                    group: 1,
                    result: None,
                },
            );
        }
        registry.publish_job_result(1, Ok(ServiceResult::Bool(true)));
        assert!(matches!(
            registry.state().unwrap().operations.get(&2).unwrap().result,
            Some(Ok(ServiceResult::Bool(true)))
        ));
        registry.publish_job_result(1, Err(error("late failure")));
        assert!(matches!(
            registry.state().unwrap().operations.get(&2).unwrap().result,
            Some(Ok(ServiceResult::Bool(true)))
        ));
    }
    struct RuntimeErrorScripts;
    impl NetworkBroker for RuntimeErrorScripts {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let body = if request.url.ends_with("/startup.js") {
                b"addEventListener('error',e=>e.preventDefault());throw new Error('startup must reject even with canceled report');".to_vec()
            } else if request.url.ends_with("/immediate-module.js") {
                b"addEventListener('error',e=>e.preventDefault());throw new Error('module must reject');export {};".to_vec()
            } else if request.url.ends_with("/async-module.js") {
                b"addEventListener('error',e=>e.preventDefault());await new Promise(resolve=>setTimeout(resolve,0));throw new Error('async module must reject');".to_vec()
            } else if request.url.ends_with("/timer-module.js") {
                b"await new Promise(resolve=>{setTimeout(()=>{throw Error('timer while startup pending')},0);setTimeout(resolve,10)});onfetch=e=>e.respondWith(new Response('module healthy'));".to_vec()
            } else {
                b"onpush=()=>{throw new Error('runtime remains alive')};onfetch=e=>e.respondWith(new Response('healthy'));".to_vec()
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("service-worker-allowed".into(), "/".into()),
                ],
                body,
            })
        }
    }
    #[test]
    fn service_startup_throw_rejects_but_runtime_throw_does_not_poison_next_fetch() {
        let network = SharedNetworkBroker::new(RuntimeErrorScripts);
        let context = Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let update = |script: &str| {
            registry.update(ServiceUpdateInputs {
                operation: 0,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: script.into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
            })
        };
        assert!(update("/startup.js").is_err());
        assert!(
            registry
                .registration_snapshots(&partition, &origin.authority)
                .unwrap()
                .is_empty()
        );
        for script in ["/immediate-module.js", "/async-module.js"] {
            assert!(
                registry
                    .update(ServiceUpdateInputs {
                        operation: 0,
                        owner: owner.clone(),
                        network: network.clone(),
                        partition: partition.clone(),
                        origin: origin.clone(),
                        policy: policy.clone(),
                        script: script.into(),
                        scope: Some("/".into()),
                        script_type: ScriptType::Module,
                        cache: "none".into()
                    })
                    .is_err(),
                "{script} must fail despite canceled ErrorEvent"
            );
            assert!(
                registry
                    .registration_snapshots(&partition, &origin.authority)
                    .unwrap()
                    .is_empty()
            );
        }
        let ServiceResult::Registration(Some(pending)) = registry
            .update(ServiceUpdateInputs {
                operation: 0,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: "/timer-module.js".into(),
                scope: Some("/".into()),
                script_type: ScriptType::Module,
                cache: "none".into(),
            })
            .unwrap()
        else {
            panic!("pending module registration")
        };
        let pending_agent = pending.active.unwrap().id;
        let pending_inbox = registry.state().unwrap().versions[&pending_agent]
            .inbox
            .clone();
        {
            let inbox = pending_inbox.lock().unwrap();
            assert!(inbox.ready);
            assert!(inbox.fatal.is_none());
            assert!(
                inbox
                    .runtime_errors
                    .iter()
                    .any(|error| error.message.contains("timer while startup pending"))
            );
        }
        let ServiceResult::Registration(Some(registration)) = update("/runtime.js").unwrap() else {
            panic!("runtime registration")
        };
        let agent = registration.active.unwrap().id;
        registry.control_client(owner.clone(), None).unwrap();
        registry.lifecycle(agent, LifecycleEvent::Push).unwrap();
        let response = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://example.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(response.body, b"healthy");
        let inbox = registry.state().unwrap().versions[&agent].inbox.clone();
        let inbox = inbox.lock().unwrap();
        assert!(inbox.fatal.is_none());
        assert!(
            !inbox.runtime_errors.is_empty(),
            "runtime exception remains recorded"
        );
        drop(inbox);
        registry.retire().unwrap();
    }
    struct RejectedInstall;
    impl NetworkBroker for RejectedInstall {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("service-worker-allowed".into(), "/".into()),
                ],
                body:
                    b"oninstall=e=>e.waitUntil(Promise.reject(new Error('failed installation')));"
                        .to_vec(),
            })
        }
    }
    fn settled_active(registry: &ServiceRegistry, partition: &str, origin: &str) -> WorkerSnapshot {
        // update_job can leave activation/publication to an existing background
        // owner. These fault-injection tests need a settled active baseline.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            registry.reap_jobs().unwrap();
            let snapshots = registry.registration_snapshots(partition, origin).unwrap();
            if let Some(active) = snapshots.first().and_then(|reg| reg.active.as_ref())
                && active.state == WorkerState::Activated
                && !registry.maintenance_running.load(Ordering::Acquire)
                && registry
                    .jobs
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|job| job.is_finished())
            {
                return active.clone();
            }
            assert!(
                Instant::now() < deadline,
                "service baseline did not settle: {snapshots:?}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn rejected_install_removes_empty_registration_and_preserves_existing_active() {
        let network = SharedNetworkBroker::new(Scripts);
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let register = |url: &str, network: SharedNetworkBroker| {
            registry.update_job(ServiceUpdateJobInputs {
                operation: 0,
                owner: owner.clone(),
                network,
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: url.into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
                register: true,
            })
        };
        assert!(register("/failure.js", SharedNetworkBroker::new(RejectedInstall)).is_err());
        assert!(
            registry
                .registration_snapshots(&partition, &origin.authority)
                .unwrap()
                .is_empty()
        );
        register("/success.js", network).unwrap();
        let active = settled_active(&registry, &partition, &origin.authority);
        assert_eq!(active.state, WorkerState::Activated);
        assert!(register("/failure.js", SharedNetworkBroker::new(RejectedInstall)).is_err());
        let after = registry
            .registration_snapshots(&partition, &origin.authority)
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].active.as_ref().unwrap().version_id,
            active.version_id
        );
        assert!(after[0].installing.is_none());
        registry.retire().unwrap();
    }
    #[test]
    fn failed_installing_publication_retires_candidate_and_preserves_active_version() {
        let network = SharedNetworkBroker::new(Scripts);
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let register = |url: &str| {
            registry.update_job(ServiceUpdateJobInputs {
                operation: 0,
                owner: owner.clone(),
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
                script: url.into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
                register: true,
            })
        };
        persistence::fail_next_publication();
        assert!(register("/first.js").is_err());
        assert!(
            registry
                .registration_snapshots(&partition, &origin.authority)
                .unwrap()
                .is_empty()
        );
        assert!(registry.state().unwrap().versions.is_empty());
        register("/active.js").unwrap();
        let active = settled_active(&registry, &partition, &origin.authority);
        registry.restore_for_owner(&owner, network.clone()).unwrap();
        persistence::fail_next_publication();
        assert!(register("/replacement.js").is_err());
        let after = registry
            .registration_snapshots(&partition, &origin.authority)
            .unwrap();
        assert_eq!(
            after[0].active.as_ref().unwrap().version_id,
            active.version_id
        );
        assert!(after[0].installing.is_none());
        assert_eq!(registry.state().unwrap().versions.len(), 1);
        registry.retire().unwrap();
    }
    struct Scripts;
    impl NetworkBroker for Scripts {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/javascript".into()),("service-worker-allowed".into(),"/".into())],body:br#"addEventListener('install',e=>e.waitUntil(registration.update().then(()=>{throw Error('install update was accepted')},error=>{if(error.name!=='InvalidStateError')throw error})));addEventListener('activate',e=>e.waitUntil(Promise.reject(new Error('activation extension rejection'))));addEventListener('fetch',e=>{if(e.request.url.endsWith('/response-error')){e.respondWith(Response.error());return;}if(e.request.destination==='iframe'){e.respondWith(new Response('native-frame-body',{headers:{'content-type':'text/html'}}));return;}if(e.request.url.endsWith('/intercepted.js')){e.respondWith(new Response('self.intercepted=true',{headers:{'content-type':'text/javascript'}}));return;}if(e.request.url.endsWith('/streaming-response')){e.respondWith(new Response(new ReadableStream({start(c){c.enqueue(new Uint8Array([4,5,6]));}}),{headers:{'content-type':'text/plain'}}));return;}if(e.request.url.endsWith('/pending-response')){e.respondWith(new Promise(()=>{}));return;}e.respondWith(Promise.resolve(new Response('native-service-body',{headers:{'content-type':'text/plain'}})));if(e.request.url.endsWith('/held'))e.waitUntil(new Promise(resolve=>globalThis.releaseHeld=resolve));});addEventListener('push',()=>{if(globalThis.releaseHeld)releaseHeld()});"#.to_vec()})
        }
    }
    #[test]
    fn registration_waits_for_native_lifecycle_and_controls_real_fetch_dispatch() {
        // Pinned extendable-event-waituntil: rejected activate still controls.
        let network = SharedNetworkBroker::new(Scripts);
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let registry = context.service_registry().unwrap();
        let ServiceResponse::Pending { operation } = registry
            .request(
                owner.clone(),
                ServiceRequest::Register {
                    creator: creator.clone(),
                    url: "/sw.js".into(),
                    scope: Some("/".into()),
                    script_type: ScriptType::Classic,
                    update_via_cache: "none".into(),
                },
                network.clone(),
            )
            .unwrap()
        else {
            panic!("registration job was not admitted")
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut registration = None;
        while Instant::now() < deadline {
            match registry
                .request(
                    owner.clone(),
                    ServiceRequest::Poll {
                        creator: creator.clone(),
                        operation,
                    },
                    network.clone(),
                )
                .unwrap()
            {
                ServiceResponse::Complete { result, .. } => {
                    let ServiceResult::Registration(value) = result.unwrap() else {
                        panic!("registration result")
                    };
                    registration = value;
                    break;
                }
                ServiceResponse::Pending { .. } => std::thread::sleep(Duration::from_millis(1)),
                _ => panic!("unexpected operation response"),
            }
        }
        let mut registration = registration.expect("native registration did not settle");
        assert!(
            matches!(
                registry
                    .request(
                        owner.clone(),
                        ServiceRequest::Poll {
                            creator: creator.clone(),
                            operation
                        },
                        network.clone()
                    )
                    .unwrap(),
                ServiceResponse::Complete { .. }
            ),
            "completion survives a lost receipt"
        );
        for _ in 0..2 {
            assert!(matches!(
                registry
                    .request(
                        owner.clone(),
                        ServiceRequest::Acknowledge {
                            creator: creator.clone(),
                            operation
                        },
                        network.clone()
                    )
                    .unwrap(),
                ServiceResponse::Done
            ));
        }
        while Instant::now() < deadline
            && registration
                .active
                .as_ref()
                .is_none_or(|worker| worker.state != WorkerState::Activated)
        {
            let ServiceResponse::Snapshot(snapshot) = registry
                .request(
                    owner.clone(),
                    ServiceRequest::Snapshot {
                        creator: creator.clone(),
                        take_messages: true,
                    },
                    network.clone(),
                )
                .unwrap()
            else {
                panic!("registration snapshot")
            };
            registration = snapshot
                .registrations
                .into_iter()
                .find(|current| current.id == registration.id)
                .unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            registration.active.as_ref().unwrap().state,
            WorkerState::Activated
        );
        assert!(registration.installing.is_none());
        registry.control_client(owner.clone(), None).unwrap();
        let service_owner = AgentOwner::Agent(registration.active.as_ref().unwrap().id);
        let (partition, origin, policy) = context.native_worker_authority(&service_owner).unwrap();
        let clients = registry
            .run(ServiceRunInputs {
                operation: 0,
                owner: service_owner.clone(),
                request: ServiceRequest::ClientsMatchAll {
                    include_uncontrolled: false,
                    client_type: Some(ClientType::Window),
                },
                network: network.clone(),
                partition: partition.clone(),
                origin: origin.clone(),
                policy: policy.clone(),
            })
            .unwrap();
        let ServiceResult::Clients(clients) = clients else {
            panic!("client list")
        };
        assert_eq!(clients.len(), 1);
        let client_id = clients[0].id.clone();
        assert_eq!(client_id.len(), 64);
        let queued = oryn_js::message_wire::WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![7],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: Vec::new(),
        };
        registry
            .state()
            .unwrap()
            .messages
            .entry(owner.clone())
            .or_default()
            .push_back(ClientMessage {
                source: registration.active.clone().unwrap(),
                origin: "https://example.test".into(),
                value: queued,
            });
        let ServiceResponse::Snapshot(metadata) = registry
            .request(
                owner.clone(),
                ServiceRequest::Snapshot {
                    creator: creator.clone(),
                    take_messages: false,
                },
                network.clone(),
            )
            .unwrap()
        else {
            panic!("metadata snapshot")
        };
        assert!(metadata.messages.is_empty());
        assert!(metadata.controller.is_some());
        assert_eq!(registry.state().unwrap().messages[&owner].len(), 1);
        let ServiceResponse::Snapshot(delivery) = registry
            .request(
                owner.clone(),
                ServiceRequest::Snapshot {
                    creator: creator.clone(),
                    take_messages: true,
                },
                network.clone(),
            )
            .unwrap()
        else {
            panic!("message snapshot")
        };
        assert_eq!(delivery.messages.len(), 1);
        assert_eq!(delivery.messages[0].value.bytes, [7]);
        assert!(registry.state().unwrap().messages[&owner].is_empty());

        assert!(matches!(
            registry
                .run(ServiceRunInputs {
                    operation: 0,
                    owner: service_owner.clone(),
                    request: ServiceRequest::ClientsGet {
                        client: client_id.clone()
                    },
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone()
                })
                .unwrap(),
            ServiceResult::Client(Some(_))
        ));
        assert!(matches!(
            registry
                .run(ServiceRunInputs {
                    operation: 0,
                    owner: service_owner.clone(),
                    request: ServiceRequest::ClientsGet {
                        client: "forged".into()
                    },
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone()
                })
                .unwrap(),
            ServiceResult::Client(None)
        ));
        assert!(
            registry
                .run(ServiceRunInputs {
                    operation: 0,
                    owner: owner.clone(),
                    request: ServiceRequest::ClientsMatchAll {
                        include_uncontrolled: true,
                        client_type: None
                    },
                    network: network.clone(),
                    partition: partition.clone(),
                    origin: origin.clone(),
                    policy: policy.clone()
                })
                .is_err()
        );
        let response = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: "native-test-client".into(),
                    url: "https://example.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .expect("fetch handler response");
        assert_eq!(response.body, b"native-service-body");
        assert_eq!(response.status, 200);
        let frozen = registry
            .freeze_fetch_controller(&owner)
            .unwrap()
            .expect("controlled client must freeze its selected version");
        let detached = {
            let mut state = registry.state().unwrap();
            (
                state.clients.remove(&owner),
                state.client_ids.remove(&owner),
            )
        };
        let stream_cancel = Arc::new(AtomicBool::new(false));
        let (metadata, mut operation) = frozen
            .fetch_operation(
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: true,
                    id: 0,
                    client_id: String::new(),
                    url: "https://example.test/streaming-response".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
                918,
                stream_cancel.clone(),
                None,
            )
            .unwrap()
            .unwrap();
        {
            let mut state = registry.state().unwrap();
            state.clients.insert(owner.clone(), detached.0.unwrap());
            state.client_ids.insert(owner.clone(), detached.1.unwrap());
        }
        drop(frozen);
        assert_eq!(metadata.status, 200);
        assert!(metadata.body.is_empty());
        assert!(matches!(
            operation.try_event().unwrap(),
            crate::network::NetworkEvent::Headers {
                request_id: 918,
                ..
            }
        ));
        assert!(
            operation.try_event().is_err(),
            "body requires explicit consumer demand"
        );
        operation.request_body_chunk();
        let deadline_chunk = Instant::now() + Duration::from_secs(2);
        loop {
            match operation.try_event() {
                Ok(crate::network::NetworkEvent::Chunk {
                    request_id: 918,
                    bytes,
                }) => {
                    assert_eq!(&bytes[..], &[4, 5, 6]);
                    break;
                }
                Ok(other) => panic!("unexpected stream event: {other:?}"),
                Err(_) => {
                    assert!(Instant::now() < deadline_chunk);
                    std::thread::yield_now();
                }
            }
        }
        stream_cancel.store(true, Ordering::Release);
        drop(operation);

        let (_, native_origin, native_policy) = context.native_worker_authority(&owner).unwrap();
        let script = crate::agent_script::load_controlled(
            &network,
            crate::agent_script::ScriptAuthority {
                hash_report: None,
                worker_destination: "worker",
                coep_report: None,
                report: None,
                cookie_ancestor_urls: &[],
                referrer_source: None,
                origin: &native_origin,
                policy: &native_policy,
                script_url: "https://example.test/app",
                referrer_policy: "no-referrer",
                top_level_url: Some("https://example.test/app"),
                credentials: WorkerCredentials::SameOrigin,
            },
            "/controlled-worker.js",
            crate::agent_script::ScriptLoadKind::EntryClassic,
            &AtomicBool::new(false),
            Some((&registry, &owner)),
        );
        assert!(
            script.is_err(),
            "intercepted text/plain response must still fail strict worker MIME validation"
        );
        let script = crate::agent_script::load_controlled(
            &network,
            crate::agent_script::ScriptAuthority {
                hash_report: None,
                worker_destination: "worker",
                coep_report: None,
                report: None,
                cookie_ancestor_urls: &[],
                referrer_source: None,
                origin: &native_origin,
                policy: &native_policy,
                script_url: "https://example.test/app",
                referrer_policy: "no-referrer",
                top_level_url: Some("https://example.test/app"),
                credentials: WorkerCredentials::SameOrigin,
            },
            "/intercepted.js",
            crate::agent_script::ScriptLoadKind::EntryClassic,
            &AtomicBool::new(false),
            Some((&registry, &owner)),
        )
        .unwrap();
        assert_eq!(
            String::from_utf16(&script.source).unwrap(),
            "self.intercepted=true"
        );
        let service_id = registration.active.as_ref().unwrap().id;
        let pending_request = AgentFetchRequest {
            keepalive: false,
            privileged_range: false,
            cache: Default::default(),
            navigation: Default::default(),
            integrity: String::new(),
            navigation_preload: false,
            streaming: false,
            id: 0,
            client_id: String::new(),
            url: "https://example.test/pending-response".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: Vec::new(),
            destination: String::new(),
            mode: "cors".into(),
            credentials: "same-origin".into(),
            redirect: "follow".into(),
        };
        let already_cancelled = AtomicBool::new(true);
        assert_eq!(
            registry
                .fetch_with_cancel(&owner, pending_request.clone(), Some(&already_cancelled))
                .err()
                .unwrap()
                .script_failure,
            Some(oryn_js::worker_wire::ScriptFailureKind::Cancelled)
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let pending_registry = registry.clone();
        let pending_owner = owner.clone();
        let pending_cancel = cancel.clone();
        let pending = std::thread::spawn(move || {
            pending_registry.fetch_with_cancel(
                &pending_owner,
                pending_request,
                Some(&pending_cancel),
            )
        });
        let inflight = registry.state().unwrap().versions[&service_id]
            .inflight
            .clone();
        let admitted = Instant::now() + Duration::from_secs(2);
        while inflight.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < admitted);
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::Release);
        assert_eq!(
            pending.join().unwrap().err().unwrap().script_failure,
            Some(oryn_js::worker_wire::ScriptFailureKind::Cancelled)
        );
        // Cancellation must not revoke this active service version or prevent
        // an unrelated event from completing while response cleanup drains.
        registry
            .lifecycle(service_id, LifecycleEvent::Push)
            .unwrap();
        let navigation_registry = registry.clone();
        let navigation_network = network.clone();
        let navigation_page = page.page_id();
        let navigation = std::thread::spawn(move || {
            navigation_registry.intercept_navigation(
                navigation_page,
                &NetworkRequest {
                    request_id: 919,
                    method: "GET".into(),
                    url: "https://example.test/pending-response".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    credentials: true,
                    security: Default::default(),
                    resolved_addrs: Vec::new(),
                },
                navigation_network,
            )
        });
        let admitted = Instant::now() + Duration::from_secs(2);
        loop {
            if registry
                .navigation_fetches
                .lock()
                .unwrap()
                .contains_key(&(page.page_id(), 919))
            {
                break;
            }
            assert!(Instant::now() < admitted);
            std::thread::yield_now();
        }
        registry.cancel_navigation(oryn_common::v3::PageId(page.page_id().0 + 1), 919);
        assert!(
            !registry.navigation_fetches.lock().unwrap()[&(page.page_id(), 919)]
                .load(Ordering::Acquire)
        );
        registry.cancel_navigation(page.page_id(), 919);
        assert!(navigation.join().unwrap().is_err());
        assert!(context.consume_service_interaction(service_id).is_err());
        context.record_native_activation(owner.clone()).unwrap();
        let service_bootstrap = context.worker_bootstrap(service_id).unwrap();
        let stable = stable_partition(&service_bootstrap.partition).unwrap();
        assert!(
            context
                .consume_native_service_interaction(
                    "foreign-site",
                    &service_bootstrap.origin.authority
                )
                .is_err()
        );
        assert!(
            context
                .consume_native_service_interaction(&stable, "foreign-native-origin")
                .is_err()
        );
        registry.consume_profile_interaction(service_id).unwrap();
        assert!(
            context.consume_service_interaction(service_id).is_err(),
            "native input grants are consumed once"
        );
        let held = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://example.test/held".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(held.body, b"native-service-body");
        let inflight = registry.state().unwrap().versions[&service_id]
            .inflight
            .clone();
        assert!(
            inflight.load(Ordering::Acquire) > 0,
            "fetch response does not retire waitUntil ownership"
        );
        registry
            .lifecycle(service_id, LifecycleEvent::Push)
            .unwrap();
        let released = Instant::now() + Duration::from_secs(2);
        while inflight.load(Ordering::Acquire) > 0 {
            assert!(Instant::now() < released);
            std::thread::yield_now();
        }

        let client =
            crate::context_service::SharedContextClient::direct(context.clone(), page.page_id());
        let frame = client
            .request(
                crate::context_service::ContextServiceRequest::LoadPageResource {
                    download: None,
                    script_authority: None,
                    page_id: page.page_id(),
                    initiator: Box::new(crate::context_service::ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator: creator.clone(),
                        stylesheet: None,
                        prepared_policy: None,
                    }),
                    form_authority: None,
                    url: "https://example.test/frame".into(),
                    destination: "frame".into(),
                    parent_encoding: None,
                    initiator_url: None,
                    referrer_policy: None,
                    post: None,
                },
            )
            .unwrap();
        let crate::context_service::ContextServiceResponse::PageResource {
            source,
            stream_id: Some(frame_stream),
            worker_policy_token: Some(frame_policy),
            ..
        } = frame
        else {
            panic!("frame stream response")
        };
        assert!(source.is_empty());
        let frame_token = "f".repeat(64);
        context
            .handle_message_wire(
                page.page_id(),
                oryn_js::message_wire::MessageRequest::RegisterWindow {
                    frame: frame_token.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://example.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let oryn_js::worker_wire::Creator::Window {
            target: parent,
            document_generation: parent_generation,
            stream_epoch: parent_epoch,
        } = creator.clone()
        else {
            panic!()
        };
        client
            .request(
                crate::context_service::ContextServiceRequest::BindFrameWorkerPolicy {
                    page_id: page.page_id(),
                    frame: frame_token.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    url: "https://example.test/frame".into(),
                    token: frame_policy,
                    parent,
                    parent_generation,
                    parent_epoch,
                    container_allow: String::new(),
                    container_source: "https://example.test/frame".into(),
                    container_sandbox: None,
                },
            )
            .unwrap();
        let frame_creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame(frame_token.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let crate::context_service::ContextServiceResponse::HttpStreamChunk { body_base64, .. } =
            client
                .request(
                    crate::context_service::ContextServiceRequest::HttpStreamRead {
                        page_id: page.page_id(),
                        stream_id: frame_stream,
                        creator: Some(frame_creator.clone()),
                        max_bytes: 4096,
                    },
                )
                .unwrap()
        else {
            panic!("frame body chunk")
        };
        assert_eq!(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                .unwrap(),
            b"native-frame-body",
            "native frame stream dispatches service worker navigation fetch"
        );
        let frame_eof = client
            .request(
                crate::context_service::ContextServiceRequest::HttpStreamRead {
                    page_id: page.page_id(),
                    stream_id: frame_stream,
                    creator: Some(frame_creator),
                    max_bytes: 4096,
                },
            )
            .unwrap();
        assert!(
            matches!(
                frame_eof,
                crate::context_service::ContextServiceResponse::HttpStreamChunk { done: true, .. }
            ),
            "release terminal frame response custody before client retirement"
        );

        let start = client
            .request(
                crate::context_service::ContextServiceRequest::HttpStreamStart {
                    page_id: page.page_id(),
                    request: crate::context_service::HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: Some(creator.clone()),
                        use_cors_preflight: false,
                        request_id: 771,
                        method: "GET".into(),
                        url: "https://example.test/controlled".into(),
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: true,
                        credentials_mode: Some("same-origin".into()),
                        initiator_url: Some("https://forged.test/".into()),
                        referrer_policy: None,
                    },
                },
            )
            .unwrap();
        let crate::context_service::ContextServiceResponse::HttpStreamStarted { stream_id, .. } =
            start
        else {
            panic!("native controlled fetch did not open")
        };
        let chunk = client
            .request(
                crate::context_service::ContextServiceRequest::HttpStreamRead {
                    page_id: page.page_id(),
                    creator: Some(creator.clone()),
                    stream_id,
                    max_bytes: 4096,
                },
            )
            .unwrap();
        let crate::context_service::ContextServiceResponse::HttpStreamChunk {
            body_base64,
            done,
            ..
        } = chunk
        else {
            panic!("native controlled fetch did not stream")
        };
        assert_eq!(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                .unwrap(),
            b"native-service-body"
        );
        if !done {
            let eof = client
                .request(
                    crate::context_service::ContextServiceRequest::HttpStreamRead {
                        page_id: page.page_id(),
                        creator: Some(creator.clone()),
                        stream_id,
                        max_bytes: 4096,
                    },
                )
                .unwrap();
            assert!(
                matches!(
                    eof,
                    crate::context_service::ContextServiceResponse::HttpStreamChunk {
                        done: true,
                        ..
                    }
                ),
                "consume terminal response custody before retiring its service"
            );
        }
        context
            .record_document_isolation(
                page.page_id(),
                &[("cross-origin-embedder-policy".into(), "require-corp".into())],
            )
            .unwrap();
        for (request_id, path, allowed) in [
            (778, "readable-synthetic", true),
            (779, "response-error", false),
        ] {
            let result = client.request(
                crate::context_service::ContextServiceRequest::HttpStreamStart {
                    page_id: page.page_id(),
                    request: crate::context_service::HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: Some(oryn_js::platform::FetchOptions {
                            keepalive: false,
                            privileged_range: false,
                            cache: Default::default(),
                            mode: "no-cors".into(),
                            redirect: "follow".into(),
                            integrity: String::new(),
                        }),
                        client: Some(creator.clone()),
                        use_cors_preflight: false,
                        request_id,
                        method: "GET".into(),
                        url: format!("https://foreign.test/{path}"),
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: false,
                        credentials_mode: Some("omit".into()),
                        initiator_url: None,
                        referrer_policy: None,
                    },
                },
            );
            if allowed {
                let crate::context_service::ContextServiceResponse::HttpStreamStarted {
                    stream_id,
                    status,
                    response_type,
                    ..
                } = result.unwrap()
                else {
                    panic!("synthetic response metadata")
                };
                assert_eq!(status, 200);
                assert!(
                    response_type.is_none(),
                    "readable service response must not be made opaque by request origin"
                );
                let crate::context_service::ContextServiceResponse::HttpStreamChunk {
                    body_base64,
                    ..
                } = client
                    .request(
                        crate::context_service::ContextServiceRequest::HttpStreamRead {
                            page_id: page.page_id(),
                            creator: Some(creator.clone()),
                            stream_id,
                            max_bytes: 4096,
                        },
                    )
                    .unwrap()
                else {
                    panic!("synthetic body")
                };
                assert_eq!(
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                        .unwrap(),
                    b"native-service-body"
                );
                client
                    .request(
                        crate::context_service::ContextServiceRequest::HttpRequestAbort {
                            page_id: page.page_id(),
                            request_id,
                        },
                    )
                    .unwrap();
            } else {
                assert!(
                    result.is_err(),
                    "error response cannot acquire readable synthetic authority"
                );
            }
        }
        let stale = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 0,
            stream_epoch: 0,
        };
        assert!(
            client
                .request(
                    crate::context_service::ContextServiceRequest::HttpStreamStart {
                        page_id: page.page_id(),
                        request: crate::context_service::HttpStreamRequest {
                            upgrade_mixed_image: false,
                            fetch: None,
                            client: Some(stale),
                            use_cors_preflight: false,
                            request_id: 772,
                            method: "GET".into(),
                            url: "https://example.test/controlled".into(),
                            headers: Vec::new(),
                            body_base64: String::new(),
                            credentials: true,
                            credentials_mode: None,
                            initiator_url: None,
                            referrer_policy: None
                        }
                    }
                )
                .is_err()
        );
        let waiting_client = client.clone();
        let waiting_page = page.page_id();
        let waiting_creator = creator.clone();
        let request = std::thread::spawn(move || {
            waiting_client.request(
                crate::context_service::ContextServiceRequest::HttpStreamStart {
                    page_id: waiting_page,
                    request: crate::context_service::HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: Some(waiting_creator),
                        use_cors_preflight: false,
                        request_id: 773,
                        method: "GET".into(),
                        url: "https://example.test/pending-response".into(),
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: true,
                        credentials_mode: None,
                        initiator_url: None,
                        referrer_policy: None,
                    },
                },
            )
        });
        let admitted = Instant::now() + Duration::from_secs(2);
        while inflight.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < admitted);
            std::thread::yield_now();
        }
        client
            .request(
                crate::context_service::ContextServiceRequest::HttpRequestAbort {
                    page_id: page.page_id(),
                    request_id: 773,
                },
            )
            .unwrap();
        let cancellation_started = Instant::now();
        assert!(request.join().unwrap().is_err());
        assert!(
            cancellation_started.elapsed() < Duration::from_secs(2),
            "HTTP abort must interrupt service response waiting before the worker event deadline"
        );
        let unregister_id = registry.id().unwrap();
        registry.updates.lock().unwrap().insert(unregister_id);
        let ticket = UpdateTicket {
            registry: registry.clone(),
            operation: Some(unregister_id),
        };
        assert!(matches!(
            registry
                .run(ServiceRunInputs {
                    operation: unregister_id,
                    owner: owner.clone(),
                    request: ServiceRequest::Unregister {
                        creator: creator.clone(),
                        registration: registration.id
                    },
                    network: network.clone(),
                    partition,
                    origin,
                    policy
                })
                .unwrap(),
            ServiceResult::Bool(true)
        ));
        drop(ticket);
        assert!(
            context.worker_bootstrap(service_id).is_ok(),
            "unregister retains a version while its controlled client lives"
        );
        registry.retire_client(&owner).unwrap();
        let frame_owner = AgentOwner::Window {
            page: page.page_id(),
            target: WindowTarget::Frame(frame_token.clone()),
            generation: 2,
            epoch: 0,
        };
        assert_eq!(
            registry.controller_registration(&frame_owner),
            Some(registration.id)
        );
        assert!(
            context.worker_bootstrap(service_id).is_ok(),
            "the live controlled frame retains the unregistered service after root retirement"
        );
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireWindow {
                    frame: frame_token,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(registry.controller_registration(&frame_owner).is_none());
        let cleanup_deadline = Instant::now() + Duration::from_secs(2);
        while registry
            .state()
            .unwrap()
            .registrations
            .contains_key(&registration.id)
            || context.worker_bootstrap(service_id).is_ok()
        {
            assert!(Instant::now() < cleanup_deadline);
            registry.reap_jobs().unwrap();
            std::thread::yield_now();
        }
        assert!(
            !registry
                .state()
                .unwrap()
                .registrations
                .contains_key(&registration.id)
        );
        assert!(
            context.worker_bootstrap(service_id).is_err(),
            "last client retirement tears down idle unregistered worker"
        );
        registry.retire().unwrap();
        assert!(
            context
                .worker_bootstrap(registration.active.unwrap().id)
                .is_err()
        );
    }
}

/// Native profile identity, never a script-provided storage key.
pub(crate) fn stable_partition(partition: &str) -> Result<String, WorkerError> {
    persistence::partition_key(partition)
}

// getRegistration validates the resolved client URL before scope matching;
// cross-origin lookup must reject rather than quietly returning no registration.
fn registration_lookup_url(
    base: &str,
    origin: &AgentOrigin,
    value: &str,
) -> Result<url::Url, WorkerError> {
    let mut url = url::Url::parse(base)
        .and_then(|base| base.join(value))
        .map_err(|failure| WorkerError::new("TypeError", failure.to_string()))?;
    if origin.opaque || url.origin().ascii_serialization() != origin.serialized {
        return Err(WorkerError::new(
            "SecurityError",
            "registration lookup URL is outside the client origin",
        ));
    }
    url.set_fragment(None);
    Ok(url)
}
#[cfg(test)]
mod registration_lookup_authority_regressions {
    use super::*;
    #[test]
    fn client_url_lookup_resolves_relative_paths_and_rejects_cross_origin() {
        let origin = AgentOrigin {
            serialized: "https://a.example".into(),
            authority: "https://a.example".into(),
            opaque: false,
        };
        assert_eq!(
            registration_lookup_url("https://a.example/app/", &origin, "../scope#fragment")
                .unwrap()
                .as_str(),
            "https://a.example/scope"
        );
        assert_eq!(
            registration_lookup_url("https://a.example/app/", &origin, "https://b.example/")
                .unwrap_err()
                .name,
            "SecurityError"
        );
        assert_eq!(
            registration_lookup_url("https://a.example/app/", &origin, "http://a.example/")
                .unwrap_err()
                .name,
            "SecurityError"
        );
        assert_eq!(
            registration_lookup_url("https://a.example/app/", &origin, "https://[")
                .unwrap_err()
                .name,
            "TypeError"
        );
    }
}

fn validate_service_registration_url(base: &url::Url, value: &url::Url) -> Result<(), WorkerError> {
    let path = value.path().to_ascii_lowercase();
    if !matches!(value.scheme(), "http" | "https") || path.contains("%2f") || path.contains("%5c") {
        return Err(WorkerError::new(
            "TypeError",
            "invalid service script or scope URL scheme/path",
        ));
    }
    if value.origin() != base.origin() || !value.username().is_empty() || value.password().is_some()
    {
        return Err(WorkerError::new(
            "SecurityError",
            "service script or scope URL is outside permitted authority",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod service_url_error_regressions {
    use super::*;
    #[test]
    fn service_registration_distinguishes_url_type_from_origin_errors() {
        let base = url::Url::parse("https://a.example/app/").unwrap();
        for value in [
            "https://a.example/app%2fworker.js",
            "https://a.example/app%5Cworker.js",
            "filesystem:https://a.example/worker.js",
        ] {
            assert_eq!(
                validate_service_registration_url(&base, &url::Url::parse(value).unwrap())
                    .unwrap_err()
                    .name,
                "TypeError"
            );
        }
        assert_eq!(
            validate_service_registration_url(
                &base,
                &url::Url::parse("https://b.example/worker.js").unwrap()
            )
            .unwrap_err()
            .name,
            "SecurityError"
        );
        assert!(validate_service_registration_url(&base, &base.join("worker.js").unwrap()).is_ok());
    }
}

/// A successfully opened foreign-origin document intentionally survives a null
/// WindowClient result. Every earlier failure retains physical close custody.
struct ServiceOpenedWindow {
    context: BrowserContext,
    page: oryn_common::v3::PageId,
    keep: bool,
}
impl Drop for ServiceOpenedWindow {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.context.queue_script_close(self.page);
        }
    }
}

#[cfg(test)]
mod local_window_creation_tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    struct Script {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for Script {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            if !request.url.ends_with("/sw.js") {
                let redirect = request.url.ends_with("/redirect");
                return Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "https://final.test/receipt".into())]
                    } else {
                        vec![("content-type".into(), "text/html".into())]
                    },
                    body: b"<!doctype html><title>service navigation</title>".to_vec(),
                });
            }
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/javascript".into()),
                    (
                        "content-security-policy".into(),
                        "script-src 'self'; img-src 'none'".into(),
                    ),
                    ("referrer-policy".into(), "no-referrer".into()),
                ],
                body: b"self.ready=true;".to_vec(),
            })
        }
    }
    fn setup() -> (
        BrowserContext,
        Arc<ServiceRegistry>,
        crate::runtime::PageHandle,
        AgentOwner,
        AgentId,
    ) {
        setup_with_requests(Arc::new(Mutex::new(Vec::new())))
    }
    fn setup_with_requests(
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    ) -> (
        BrowserContext,
        Arc<ServiceRegistry>,
        crate::runtime::PageHandle,
        AgentOwner,
        AgentId,
    ) {
        let network = SharedNetworkBroker::new(Script { requests });
        let context = crate::runtime::Browser::in_process_with_config(Default::default())
            .new_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://service.test/app".into())
            .unwrap();
        context
            .install_document_security(id, "https://service.test/app".into(), &[])
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
        let owner = context
            .window_worker_creator(id, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        register_service(context, page, network, owner)
    }

    // Navigation tests need a native document. The default test binary delegates
    // to the signed worker; V8-enabled tests use the same native engine locally.
    // Capture its published creator instead of inventing a document generation.
    fn setup_native_with_requests(
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    ) -> (
        BrowserContext,
        Arc<ServiceRegistry>,
        crate::runtime::PageHandle,
        AgentOwner,
        AgentId,
    ) {
        let network = SharedNetworkBroker::new(Script { requests });
        let context = crate::runtime::Browser::native_test_with_config(Default::default())
            .new_native_test_context_with_broker(Default::default(), network.clone());
        let page = context.new_page();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(page.load_html(
                "https://service.test/app",
                "<!doctype html><title>service source</title>",
            ))
            .expect("load native service source document");
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = context.root_navigation_creator(page.page_id()).unwrap()
        else {
            panic!("native service source Window creator")
        };
        let owner = context
            .window_worker_creator(page.page_id(), &target, document_generation, stream_epoch)
            .unwrap()
            .0;
        register_service(context, page, network, owner)
    }

    fn register_service(
        context: BrowserContext,
        page: crate::runtime::PageHandle,
        network: SharedNetworkBroker,
        owner: AgentOwner,
    ) -> (
        BrowserContext,
        Arc<ServiceRegistry>,
        crate::runtime::PageHandle,
        AgentOwner,
        AgentId,
    ) {
        let registry = context.service_registry().unwrap();
        let (partition, origin, policy) = context.native_worker_authority(&owner).unwrap();
        let ServiceResult::Registration(Some(registration)) = registry
            .update(ServiceUpdateInputs {
                operation: 0,
                owner: owner.clone(),
                network,
                partition,
                origin,
                policy,
                script: "/sw.js".into(),
                scope: Some("/".into()),
                script_type: ScriptType::Classic,
                cache: "none".into(),
            })
            .unwrap()
        else {
            panic!("registration")
        };
        let agent = registration.active.unwrap().id;
        (context, registry, page, owner, agent)
    }
    #[test]
    fn concurrent_cleanup_sweeps_preserve_the_existing_retirement_owner() {
        let (context, registry, _page, _owner, agent) = setup();
        {
            let mut state = registry.state().unwrap();
            let version = state.versions.get_mut(&agent).unwrap();
            version.retiring = true;
            let registration = version.bootstrap.service_registration.unwrap();
            state
                .registrations
                .get_mut(&registration)
                .unwrap()
                .snapshot
                .active = None;
        }
        // Reproduce a request and a maintenance sweep arriving while the
        // process teardown owner is outside the registry lock.
        std::thread::scope(|threads| {
            for _ in 0..2 {
                let registry = registry.clone();
                threads.spawn(move || registry.cleanup_unregistered().unwrap());
            }
        });
        assert!(registry.state().unwrap().versions[&agent].retiring);
        assert!(context.worker_bootstrap(agent).is_ok());
        // A synchronous completion barrier still refuses to claim completion.
        assert!(registry.retire_version(agent).is_err());
        registry
            .state()
            .unwrap()
            .versions
            .get_mut(&agent)
            .unwrap()
            .retiring = false;
        registry.cleanup_unregistered().unwrap();
        assert!(!registry.state().unwrap().versions.contains_key(&agent));
        assert!(context.worker_bootstrap(agent).is_err());
        registry.retire().unwrap();
    }

    #[test]
    fn native_window_stage_failure_preserves_original_error_and_closes_only_failed_target() {
        for stage in [
            "capture-initial-target",
            "admit-navigation",
            "navigate-initial-window",
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (context, registry, source, owner, agent) = setup_with_requests(requests.clone());
            requests.lock().unwrap().clear();
            context.record_native_activation(owner.clone()).unwrap();
            let original =
                WorkerError::new("NetworkError", "original native navigation cause \u{1f980}")
                    .with_script_failure(ScriptFailureKind::Fetch);
            FAIL_SERVICE_WINDOW_STAGE
                .with(|slot| *slot.borrow_mut() = Some((stage, original.clone())));
            let failure = registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientsOpenWindow {
                        url: "https://service.test/client".into(),
                    },
                )
                .unwrap_err();
            assert!(
                FAIL_SERVICE_WINDOW_STAGE.with(|slot| slot.borrow().is_none()),
                "the injected native boundary must be reached"
            );
            assert_eq!(failure.name, original.name);
            assert_eq!(failure.script_failure, original.script_failure);
            assert!(failure.message.starts_with(&original.message));
            assert!(
                failure.message.len() <= original.message.len() + 256,
                "native diagnostic suffix must stay bounded"
            );
            assert!(failure.message.contains(&format!("stage={stage}")));
            let pages = context.pages();
            assert_eq!(pages.len(), 2);
            let failed = pages
                .iter()
                .find(|page| page.page.page_id != source.page_id())
                .unwrap();
            assert!(
                failure
                    .message
                    .contains(&format!("page={}", failed.page.page_id.0))
            );
            assert_eq!(failed.status, oryn_common::v3::PageStatus::Closed);
            assert_eq!(
                pages
                    .iter()
                    .find(|page| page.page.page_id == source.page_id())
                    .unwrap()
                    .status,
                oryn_common::v3::PageStatus::Open
            );
            assert!(
                context
                    .native_initial_document_creator(failed.page.page_id)
                    .is_err()
            );
            assert!(context.worker_bootstrap(agent).is_ok());
            assert!(
                requests.lock().unwrap().is_empty(),
                "failure before navigation dispatch must send no page request"
            );
            assert_eq!(
                registry
                    .window_action(
                        AgentOwner::Agent(agent),
                        ServiceRequest::ClientsOpenWindow {
                            url: "https://service.test/another".into()
                        }
                    )
                    .unwrap_err()
                    .name,
                "InvalidAccessError",
                "failed attempt must not restore consumed activation"
            );
            assert_eq!(context.pages().len(), 2);
            context.queue_script_close(source.page_id()).unwrap();
            registry.retire().unwrap();
        }
    }

    #[test]
    fn controlled_root_client_can_navigate_to_data_and_about_is_rejected() {
        let (context, registry, source, owner, agent) =
            setup_native_with_requests(Arc::new(Mutex::new(Vec::new())));
        context.record_native_activation(owner).unwrap();
        let ServiceResult::WindowClient(Some(client)) = registry
            .window_action(
                AgentOwner::Agent(agent),
                ServiceRequest::ClientsOpenWindow {
                    url: "https://service.test/client".into(),
                },
            )
            .unwrap()
        else {
            panic!("HTTP client")
        };
        assert_eq!(
            registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientNavigate {
                        client: client.id.clone(),
                        url: "about:blank".into()
                    }
                )
                .unwrap_err()
                .name,
            "TypeError"
        );
        assert!(matches!(
            registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientNavigate {
                        client: client.id,
                        url: "data:text/html,<p>opaque client</p>".into()
                    }
                )
                .unwrap(),
            ServiceResult::WindowClient(None)
        ));
        let child = context
            .pages()
            .into_iter()
            .find(|page| page.page.page_id != source.page_id())
            .unwrap();
        assert!(child.page.url.starts_with("data:"));
        assert_eq!(context.page_origin(child.page.page_id).unwrap(), "null");
        context.queue_script_close(child.page.page_id).unwrap();
        registry.retire().unwrap();
    }
    #[test]
    fn service_frame_conversion_carries_service_policy_into_data_response_grant() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ParentContextService, ResourceInitiator,
        };
        let (context, registry, page, _, agent) = setup();
        let id = page.page_id();
        let frame = "d".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://service.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(id, "https://service.test/client", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(
                id,
                frame.clone(),
                2,
                0,
                "https://service.test/client",
                &ticket,
            )
            .unwrap();
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page: id,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 2,
            stream_epoch: 0,
        };
        let url = "data:text/html,<p>child</p>";
        let admission = context
            .issue_service_navigation_authority(agent, id, creator, url)
            .unwrap();
        assert!(
            context
                .prepare_service_local_navigation(id, admission.clone(), url, false)
                .is_err(),
            "frame token must not commit root security"
        );
        let script = context
            .service_navigation_to_script(id, admission, url)
            .unwrap();
        assert!(
            context
                .script_navigation_local_policy(&script, id, "data:text/html,wrong")
                .is_err()
        );
        let service = ParentContextService::new(context.clone(), id);
        let response = service
            .handle(ContextServiceRequest::LoadPageResource {
                download: None,
                script_authority: Some(script),
                page_id: id,
                initiator: Box::new(ResourceInitiator {
                    embedded_type: None,
                    request_state: None,
                    navigation: Default::default(),
                    creator: Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    stylesheet: None,
                    prepared_policy: None,
                }),
                form_authority: None,
                url: url.into(),
                destination: "frame".into(),
                parent_encoding: None,
                initiator_url: Some("https://forged.test/".into()),
                referrer_policy: Some("unsafe-url".into()),
                post: None,
            })
            .unwrap();
        let ContextServiceResponse::PageResource {
            worker_policy_token: Some(ticket),
            native_origin: Some(origin),
            referrer_policy,
            document_referrer,
            ..
        } = response
        else {
            panic!("frame local grant")
        };
        assert!(origin.opaque);
        assert_eq!(referrer_policy, "no-referrer");
        assert!(document_referrer.is_empty());
        let policy = context.prepared_frame_resource_policy(id, &ticket).unwrap();
        assert!(policy.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-security-policy") && value.contains("img-src 'none'")
        }));
        assert_eq!(
            policy.policy_self_url.as_deref(),
            Some("https://service.test/sw.js")
        );
        context.discard_prepared_frame_policy(id, &ticket).unwrap();
        registry.retire().unwrap();
    }
    #[test]
    fn service_http_navigation_uses_sealed_metadata_and_rejected_admissions_send_nothing() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (context, registry, source_page, _, agent) =
            setup_native_with_requests(requests.clone());
        requests.lock().unwrap().clear();
        let bootstrap = context.worker_bootstrap(agent).unwrap();
        let page = context.new_service_window(agent).unwrap();
        let target = context
            .native_initial_document_creator(page.page_id())
            .unwrap();
        let url = "https://destination.test/redirect";
        let admission = context
            .issue_service_navigation_authority(agent, page.page_id(), target, url)
            .unwrap();
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(
            executor
                .block_on(page.goto_service_resource(
                    "https://destination.test/substitution".into(),
                    admission.clone()
                ))
                .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
        let wrong = context.new_service_window(agent).unwrap();
        assert!(
            executor
                .block_on(wrong.goto_service_resource(url.into(), admission.clone()))
                .is_err()
        );
        assert!(requests.lock().unwrap().is_empty());
        executor
            .block_on(page.goto_service_resource(url.into(), admission))
            .unwrap();
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let mut ancestors = bootstrap.cookie_ancestor_urls.clone();
        ancestors.insert(0, bootstrap.script_url.clone());
        for request in seen.iter() {
            assert_eq!(
                request.security.initiator_url.as_deref(),
                Some(bootstrap.script_url.as_str())
            );
            assert_eq!(
                request.security.top_level_url,
                bootstrap
                    .top_level_url
                    .clone()
                    .or_else(|| Some(bootstrap.script_url.clone()))
            );
            assert_eq!(request.security.ancestor_urls, ancestors);
            assert!(request.security.top_level_navigation);
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("referer")),
                "native no-referrer applies across redirects"
            );
        }
        assert!(seen[1].security.redirect_chain_cross_site);
        drop(seen);
        context.queue_script_close(page.page_id()).unwrap();
        context.queue_script_close(wrong.page_id()).unwrap();
        context.queue_script_close(source_page.page_id()).unwrap();
        registry.retire().unwrap();
    }
    #[test]
    fn service_navigation_source_is_sealed_to_exact_target_and_survives_accepted_source_retirement()
    {
        let (context, registry, _source, _, agent) = setup();
        let bootstrap = context.worker_bootstrap(agent).unwrap();
        let page = context.new_service_window(agent).unwrap();
        let target = context
            .native_initial_document_creator(page.page_id())
            .unwrap();
        let url = "https://destination.test/path";
        let admission = context
            .issue_service_navigation_authority(agent, page.page_id(), target.clone(), url)
            .unwrap();
        let source = context
            .service_navigation_source(&admission.authority, page.page_id(), &target, url)
            .unwrap();
        assert_eq!(source.url, bootstrap.script_url);
        assert_eq!(source.referrer_policy, "no-referrer");
        assert_eq!(
            source.top_level_url,
            bootstrap
                .top_level_url
                .unwrap_or_else(|| source.url.clone())
        );
        assert_eq!(source.ancestor_urls.first(), Some(&source.url));
        assert!(
            context
                .service_navigation_source(
                    &admission.authority,
                    page.page_id(),
                    &target,
                    "https://destination.test/other"
                )
                .is_err()
        );
        let other = context.new_service_window(agent).unwrap();
        let other_target = context
            .native_initial_document_creator(other.page_id())
            .unwrap();
        assert!(
            context
                .service_navigation_source(
                    &admission.authority,
                    other.page_id(),
                    &other_target,
                    url
                )
                .is_err()
        );
        let mut forged = admission.authority.clone();
        forged.0[0] ^= 1;
        assert!(
            context
                .service_navigation_source(&forged, page.page_id(), &target, url)
                .is_err()
        );
        let foreign =
            crate::runtime::Browser::in_process_with_config(Default::default()).new_context();
        let foreign_page = foreign.new_page();
        foreign
            .install_document_security(
                foreign_page.page_id(),
                "https://service.test/foreign".into(),
                &[],
            )
            .unwrap();
        foreign
            .handle_message_wire(
                foreign_page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let foreign_target = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            foreign
                .issue_service_navigation_from(
                    &context,
                    agent,
                    foreign_page.page_id(),
                    foreign_target,
                    url
                )
                .is_err(),
            "equal serialized origins cannot cross profile coordinator custody"
        );
        foreign.queue_script_close(foreign_page.page_id()).unwrap();
        registry.retire().unwrap();
        assert!(
            context
                .issue_service_navigation_authority(agent, page.page_id(), target.clone(), url)
                .is_err()
        );
        assert_eq!(
            context
                .service_navigation_source(&admission.authority, page.page_id(), &target, url)
                .unwrap()
                .url,
            source.url
        );
        let Creator::Window {
            document_generation,
            stream_epoch,
            ..
        } = target.clone()
        else {
            panic!("Window")
        };
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation,
                    stream_epoch,
                },
            )
            .unwrap();
        assert!(
            context
                .service_navigation_source(&admission.authority, page.page_id(), &target, url)
                .is_err()
        );
        context.queue_script_close(page.page_id()).unwrap();
        context.queue_script_close(other.page_id()).unwrap();
    }
    #[test]
    fn service_window_failure_guard_revokes_failed_activation_but_keeps_successful_null_client() {
        let context =
            crate::runtime::Browser::in_process_with_config(Default::default()).new_context();
        let failed = context.new_page();
        let successful = context.new_page();
        drop(ServiceOpenedWindow {
            context: context.clone(),
            page: failed.page_id(),
            keep: false,
        });
        drop(ServiceOpenedWindow {
            context: context.clone(),
            page: successful.page_id(),
            keep: true,
        });
        let pages = context.pages();
        assert_eq!(
            pages
                .iter()
                .find(|page| page.page.page_id == failed.page_id())
                .unwrap()
                .status,
            oryn_common::v3::PageStatus::Closed
        );
        assert_eq!(
            pages
                .iter()
                .find(|page| page.page.page_id == successful.page_id())
                .unwrap()
                .status,
            oryn_common::v3::PageStatus::Open
        );
        context.queue_script_close(successful.page_id()).unwrap();
    }
    #[test]
    fn service_open_window_data_returns_null_and_blob_returns_same_origin_client() {
        let (context, registry, source, owner, agent) =
            setup_native_with_requests(Arc::new(Mutex::new(Vec::new())));
        context.record_native_activation(owner.clone()).unwrap();
        assert!(matches!(
            registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientsOpenWindow {
                        url: "data:text/html,<p>opaque</p>".into()
                    }
                )
                .unwrap(),
            ServiceResult::WindowClient(None)
        ));
        let blob = context
            .register_native_blob(
                owner.clone(),
                b"<p>same origin</p>".to_vec(),
                "text/html".into(),
            )
            .unwrap();
        context.record_native_activation(owner).unwrap();
        let ServiceResult::WindowClient(Some(client)) = registry
            .window_action(
                AgentOwner::Agent(agent),
                ServiceRequest::ClientsOpenWindow { url: blob.clone() },
            )
            .unwrap()
        else {
            panic!("blob WindowClient")
        };
        assert_eq!(client.url, blob);
        assert!(client.focused);
        let opened = context
            .pages()
            .into_iter()
            .filter(|record| record.page.page_id != source.page_id())
            .collect::<Vec<_>>();
        assert_eq!(opened.len(), 2);
        assert!(
            opened
                .iter()
                .all(|record| record.page.opener_page_id.is_none())
        );
        for page in opened {
            context.queue_script_close(page.page.page_id).unwrap();
        }
        registry.retire().unwrap();
    }
    #[test]
    fn service_initial_local_policy_is_native_and_data_origin_is_fresh() {
        let (context, registry, _source, _, agent) = setup();
        let bootstrap = context.worker_bootstrap(agent).unwrap();
        let page = context.new_service_window(agent).unwrap();
        let id = page.page_id();
        let creator = context.native_initial_document_creator(id).unwrap();
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &creator
        else {
            panic!("Window")
        };
        let (_, _, initial) = context
            .window_worker_creator(id, target, *document_generation, *stream_epoch)
            .unwrap();
        assert_eq!(initial.authority, bootstrap.origin.authority);
        let policy = context
            .window_worker_policy(id, target, *document_generation, *stream_epoch)
            .unwrap();
        assert_eq!(policy.url, "about:blank");
        assert!(policy.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-security-policy") && value.contains("img-src 'none'")
        }));
        assert_eq!(
            policy.policy_self_url.as_deref(),
            Some(bootstrap.script_url.as_str())
        );
        assert!(
            policy
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("referrer-policy")
                    && value == "no-referrer")
        );
        let url = "data:text/html,<p>service</p>";
        let (token, _, _, _, origin) = context
            .prepare_local_navigation(id, creator, url, false)
            .unwrap();
        assert!(origin.opaque);
        assert_ne!(origin.authority, bootstrap.origin.authority);
        let (_, policy) = context
            .commit_prepared_local_navigation(id, &token)
            .unwrap();
        assert!(
            !policy
                .authorize(&crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Image,
                    url: "https://service.test/image.png".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false
                })
                .allowed
        );
        context.queue_script_close(id).unwrap();
        registry.retire().unwrap();
        assert!(context.new_service_window(agent).is_err());
    }
    #[test]
    fn service_initial_blob_uses_registry_origin_and_denies_revocation() {
        let (context, registry, _source, owner, agent) = setup();
        let blob = context
            .register_native_blob(owner.clone(), b"<p>blob</p>".to_vec(), "text/html".into())
            .unwrap();
        let page = context.new_service_window(agent).unwrap();
        let creator = context
            .native_initial_document_creator(page.page_id())
            .unwrap();
        let (token, body, _, _, origin) = context
            .prepare_local_navigation(page.page_id(), creator.clone(), &blob, true)
            .unwrap();
        assert_eq!(body, b"<p>blob</p>");
        assert_eq!(
            origin.authority,
            context.worker_bootstrap(agent).unwrap().origin.authority
        );
        context
            .discard_prepared_local_navigation(page.page_id(), &token)
            .unwrap();
        context.revoke_native_blob(&owner, &blob).unwrap();
        assert!(
            context
                .prepare_local_navigation(page.page_id(), creator, &blob, true)
                .is_err()
        );
        context.queue_script_close(page.page_id()).unwrap();
        registry.retire().unwrap();
    }
    #[test]
    fn service_open_window_rejects_about_before_activation_and_keeps_failed_target_cleanup() {
        let (context, registry, _source, owner, agent) = setup();
        let initial = context.pages().len();
        for url in [
            "about:blank",
            "about:blank#fragment",
            "about:srcdoc",
            "javascript:1",
        ] {
            assert_eq!(
                registry
                    .window_action(
                        AgentOwner::Agent(agent),
                        ServiceRequest::ClientsOpenWindow { url: url.into() }
                    )
                    .unwrap_err()
                    .name,
                "TypeError"
            );
        }
        assert_eq!(context.pages().len(), initial);
        assert_eq!(
            registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientsOpenWindow {
                        url: "data:text/html,hello".into()
                    }
                )
                .unwrap_err()
                .name,
            "InvalidAccessError"
        );
        assert_eq!(context.pages().len(), initial);
        context.record_native_activation(owner).unwrap();
        assert!(
            registry
                .window_action(
                    AgentOwner::Agent(agent),
                    ServiceRequest::ClientsOpenWindow {
                        url: "blob:https://service.test/missing".into()
                    }
                )
                .is_err()
        );
        let created = context
            .pages()
            .into_iter()
            .filter(|record| record.page.page_id != _source.page_id())
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].status, oryn_common::v3::PageStatus::Closed);
        registry.retire().unwrap();
    }
}
