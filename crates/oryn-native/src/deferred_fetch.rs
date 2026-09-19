//! Parent-owned deferred Fetch. Only preparation reads a live Window identity;
//! an accepted capability retains its original policy through document retirement.
use super::*;
use crate::http_cleanup_custody::{Scope, Ticket};
use oryn_js::{
    fetch_later::{DeferredFetchError, DeferredFetchState, DeferredFetchToken},
    platform::PlatformHttpRequest,
    worker_wire::Creator,
};
use sha2::{Digest, Sha256};
use std::sync::{Condvar, OnceLock, Weak, atomic::AtomicU8};
const ORIGIN_BYTES: usize = 64 * 1024;
const CHILD_BYTES: usize = 8 * 1024;
const TOP_BYTES: usize = 640 * 1024;
const MINIMAL_POOL: usize = 128 * 1024;
use crate::agent_registry::AgentOwner;
fn container_key(owner: &AgentOwner) -> (PageId, String) {
    match owner {
        AgentOwner::Window {
            page,
            target: oryn_js::message_wire::WindowTarget::Frame(frame),
            ..
        } => (*page, frame.clone()),
        AgentOwner::Window { page, .. } => (*page, String::new()),
        AgentOwner::Agent(_) => (PageId(0), String::new()),
    }
}
#[derive(Clone)]
struct Reservation {
    control: AgentOwner,
    bytes: usize,
    minimal: bool,
}

const MAX_CAPABILITIES: usize = 4096;
const ACK_HISTORY: usize = 1024;
fn sequence(token: &DeferredFetchToken) -> Result<u64, DeferredFetchError> {
    if token.0.len() != 32 {
        return Err(invalid("invalid deferred Fetch capability"));
    }
    let value = u64::from_be_bytes(token.0[..8].try_into().expect("capability sequence"));
    if value == 0 {
        return Err(invalid("invalid deferred Fetch sequence"));
    }
    Ok(value)
}
fn issuer(page: PageId, creator: &Creator) -> Result<AgentOwner, DeferredFetchError> {
    match creator {
        Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } => Ok(AgentOwner::Window {
            page,
            target: target.clone(),
            generation: *document_generation,
            epoch: *stream_epoch,
        }),
        Creator::Agent(_) => Err(invalid("deferred Fetch requires Window")),
    }
}
#[derive(Clone)]
enum SecurityReport {
    Csp(crate::csp::CspViolation),
    Coep(Vec<crate::coep_reporting::Report>),
}
#[derive(Clone)]
struct Plan {
    keepalive: crate::keepalive::Budget,
    page: PageId,
    creator: Creator,
    source: Url,
    origin: String,
    opaque: bool,
    top: String,
    ancestors: Vec<String>,
    policy: crate::csp::CspPolicySet,
    embedder: crate::agent_isolation::DocumentIsolationPolicy,
    coep: crate::coep_reporting::Policy,
    referrer: crate::referrer::ReferrerPolicy,
    request: PlatformHttpRequest,
    bytes: usize,
    control: AgentOwner,
    quota: usize,
}
struct Entry {
    page: PageId,
    creator: Creator,
    fingerprint: [u8; 32],
    pending: Option<Arc<Plan>>,
    activation: Arc<AtomicU8>,
    deadline: Option<Instant>,
    acknowledged: bool,
}
#[derive(Default)]
pub(super) struct Registry {
    entries: BTreeMap<Vec<u8>, Entry>,
    cancelled: BTreeMap<Vec<u8>, (PageId, Creator)>,
    watermarks: BTreeMap<AgentOwner, u64>,
    history: std::collections::VecDeque<Vec<u8>>,
    scope: Scope,
    closed: bool,
    reservations: BTreeMap<(PageId, String), Reservation>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.scope.cancel(None);
    }
}
fn transport(error: impl std::fmt::Display) -> DeferredFetchError {
    DeferredFetchError::Transport(error.to_string())
}
fn invalid(message: &str) -> DeferredFetchError {
    DeferredFetchError::Type(message.into())
}
fn fingerprint(request: &PlatformHttpRequest) -> Result<[u8; 32], DeferredFetchError> {
    if request.headers.iter().any(|(name, value)| {
        tungstenite::http::HeaderName::from_bytes(name.as_bytes()).is_err()
            || tungstenite::http::HeaderValue::from_str(value).is_err()
    }) {
        return Err(invalid("invalid deferred Fetch header"));
    }
    Ok(Sha256::digest(serde_json::to_vec(request).map_err(transport)?).into())
}
fn deferred_fingerprint(
    request: &PlatformHttpRequest,
    delay: Option<Duration>,
) -> Result<[u8; 32], DeferredFetchError> {
    let hash = fingerprint(request)?;
    if let Some(delay) = delay {
        let mut digest = Sha256::new();
        digest.update(hash);
        digest.update(delay.as_secs().to_be_bytes());
        digest.update(delay.subsec_nanos().to_be_bytes());
        Ok(digest.finalize().into())
    } else {
        Ok(hash)
    }
}
fn state_value(value: u8) -> DeferredFetchState {
    match value {
        0 => DeferredFetchState::Pending,
        1 => DeferredFetchState::Activated,
        _ => DeferredFetchState::Canceled,
    }
}
fn promote_due(entry: &Entry, now: Instant) {
    if entry.deadline.is_some_and(|deadline| deadline <= now) {
        let _ = entry
            .activation
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
}
impl Registry {
    fn retire_issuer(&mut self, owner: &AgentOwner) {
        self.watermarks.remove(owner);
        self.cancelled
            .retain(|_, (page, creator)| issuer(*page, creator).as_ref() != Ok(owner));
        let tokens = self
            .entries
            .iter_mut()
            .filter(|(_, entry)| issuer(entry.page, &entry.creator).as_ref() == Ok(owner))
            .filter_map(|(token, entry)| {
                let previously_acknowledged = entry.acknowledged;
                entry.acknowledged = true;
                (!previously_acknowledged && entry.pending.is_none()).then(|| token.clone())
            })
            .collect::<Vec<_>>();
        for token in tokens {
            self.remember_terminal(&token);
        }
    }
    fn remember_terminal(&mut self, token: &[u8]) {
        self.history.push_back(token.to_vec());
        while self.history.len() > ACK_HISTORY {
            if let Some(token) = self.history.pop_front() {
                if self
                    .entries
                    .get(&token)
                    .is_some_and(|entry| entry.pending.is_none() && entry.acknowledged)
                {
                    self.entries.remove(&token);
                }
                self.cancelled.remove(&token);
            }
        }
    }
    fn replay(
        &self,
        page: PageId,
        creator: &Creator,
        token: &DeferredFetchToken,
        hash: &[u8; 32],
    ) -> Result<bool, DeferredFetchError> {
        let seq = sequence(token)?;
        let owner = issuer(page, creator)?;
        if self.closed {
            return Err(transport("deferred Fetch context retired"));
        }
        if let Some((owned_page, owned_creator)) = self.cancelled.get(&token.0) {
            if *owned_page != page || owned_creator != creator {
                return Err(invalid("foreign cancelled deferred Fetch capability"));
            }
            return Ok(true);
        }
        if let Some(entry) = self.entries.get(&token.0) {
            if entry.page != page || &entry.creator != creator || &entry.fingerprint != hash {
                return Err(invalid("deferred Fetch capability replay differs"));
            }
            return Ok(true);
        }
        if self
            .watermarks
            .get(&owner)
            .is_some_and(|watermark| seq <= *watermark)
        {
            return Err(invalid("deferred Fetch capability expired"));
        }
        Ok(false)
    }
    fn insert(
        &mut self,
        token: DeferredFetchToken,
        plan: Plan,
        hash: [u8; 32],
        activation: Arc<AtomicU8>,
        deadline: Option<Instant>,
    ) -> Result<(), DeferredFetchError> {
        if self.replay(plan.page, &plan.creator, &token, &hash)? {
            return Ok(());
        }
        for entry in self.entries.values() {
            promote_due(entry, Instant::now());
        }
        if self
            .entries
            .values()
            .filter(|entry| entry.pending.is_some() || !entry.acknowledged)
            .count()
            >= MAX_CAPABILITIES
        {
            return Err(DeferredFetchError::Quota(
                "deferred Fetch capability capacity exceeded".into(),
            ));
        }
        let destination = Url::parse(&plan.request.url).map_err(transport)?.origin();
        let mut total = plan.bytes;
        let mut destination_bytes = plan.bytes;
        for previous in self
            .entries
            .values()
            .filter(|entry| entry.activation.load(Ordering::Acquire) == 0)
            .filter_map(|entry| entry.pending.as_ref())
            .filter(|previous| previous.control == plan.control)
        {
            total = total.saturating_add(previous.bytes);
            if Url::parse(&previous.request.url).is_ok_and(|url| url.origin() == destination) {
                destination_bytes = destination_bytes.saturating_add(previous.bytes);
            }
        }
        let delegated = self
            .reservations
            .values()
            .filter(|reservation| reservation.control == plan.control && !reservation.minimal)
            .fold(0usize, |sum, reservation| {
                sum.saturating_add(reservation.bytes)
            });
        if total > plan.quota.saturating_sub(delegated) || destination_bytes > ORIGIN_BYTES {
            return Err(DeferredFetchError::Quota(
                "deferred Fetch request quota exceeded".into(),
            ));
        }
        let owner = issuer(plan.page, &plan.creator)?;
        if !self.watermarks.contains_key(&owner) && self.watermarks.len() >= MAX_CAPABILITIES {
            return Err(DeferredFetchError::Quota(
                "deferred Fetch live issuers exceeded".into(),
            ));
        }
        self.watermarks.insert(owner, sequence(&token)?);
        self.entries.insert(
            token.0,
            Entry {
                page: plan.page,
                creator: plan.creator.clone(),
                fingerprint: hash,
                pending: Some(Arc::new(plan)),
                activation: activation.clone(),
                deadline,
                acknowledged: false,
            },
        );
        activation.store(0, Ordering::Release);
        Ok(())
    }
    fn cancel(
        &mut self,
        page: PageId,
        creator: &Creator,
        token: &DeferredFetchToken,
    ) -> Result<DeferredFetchState, DeferredFetchError> {
        let seq = sequence(token)?;
        let owner = issuer(page, creator)?;
        if let Some(entry) = self.entries.get_mut(&token.0) {
            if entry.page != page || &entry.creator != creator {
                return Err(invalid("foreign deferred Fetch capability"));
            }
            promote_due(entry, Instant::now());
            if entry
                .activation
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                entry.pending = None;
                if entry.acknowledged {
                    self.remember_terminal(&token.0);
                }
            }
            return self.state(page, token);
        }
        if let Some((owned_page, owned_creator)) = self.cancelled.get(&token.0) {
            return if *owned_page == page && owned_creator == creator {
                Ok(DeferredFetchState::Canceled)
            } else {
                Err(invalid("foreign cancelled deferred Fetch capability"))
            };
        }
        if self
            .watermarks
            .get(&owner)
            .is_some_and(|watermark| seq <= *watermark)
        {
            return Err(invalid("deferred Fetch capability expired"));
        }
        if !self.watermarks.contains_key(&owner) && self.watermarks.len() >= MAX_CAPABILITIES {
            return Err(DeferredFetchError::Quota(
                "deferred Fetch live issuers exceeded".into(),
            ));
        }
        self.watermarks.insert(owner, seq);
        self.cancelled
            .insert(token.0.clone(), (page, creator.clone()));
        self.remember_terminal(&token.0);
        Ok(DeferredFetchState::Canceled)
    }
    fn submitted(&mut self, token: &[u8]) {
        if let Some(entry) = self.entries.get_mut(token) {
            entry.pending = None;
            if entry.acknowledged {
                self.remember_terminal(token);
            }
        }
    }
    fn acknowledge(
        &mut self,
        page: PageId,
        token: &DeferredFetchToken,
    ) -> Result<(), DeferredFetchError> {
        sequence(token)?;
        if let Some(entry) = self.entries.get_mut(&token.0) {
            if entry.page != page {
                return Err(invalid("foreign deferred Fetch capability"));
            }
            promote_due(entry, Instant::now());
            if entry.activation.load(Ordering::Acquire) == 0 {
                return Err(invalid("deferred Fetch outcome is still pending"));
            }
            if !entry.acknowledged {
                entry.acknowledged = true;
                if entry.pending.is_none() {
                    self.remember_terminal(&token.0);
                }
            }
            return Ok(());
        }
        if self
            .cancelled
            .get(&token.0)
            .is_some_and(|(owner, _)| *owner != page)
        {
            return Err(invalid("foreign deferred Fetch capability"));
        }
        // Cleanup is idempotent after an acknowledged outcome leaves history.
        Ok(())
    }
    fn state(
        &self,
        page: PageId,
        token: &DeferredFetchToken,
    ) -> Result<DeferredFetchState, DeferredFetchError> {
        sequence(token)?;
        if let Some(entry) = self.entries.get(&token.0) {
            if entry.page != page {
                return Err(invalid("foreign deferred Fetch capability"));
            }
            promote_due(entry, Instant::now());
            return Ok(state_value(entry.activation.load(Ordering::Acquire)));
        }
        if let Some((owner, _)) = self.cancelled.get(&token.0) {
            return if *owner == page {
                Ok(DeferredFetchState::Canceled)
            } else {
                Err(invalid("foreign deferred Fetch capability"))
            };
        }
        Err(invalid("deferred Fetch capability expired"))
    }
    pub(super) fn close(&mut self) {
        self.closed = true;
        for entry in self.entries.values_mut() {
            entry.pending = None;
            entry.activation.store(2, Ordering::Release);
        }
        self.scope.cancel(None);
    }
}
enum ScheduledOwner {
    #[cfg(unix)]
    Context(Weak<BrowserContextInner>),
    #[cfg(feature = "v8-host")]
    Standalone(Weak<StandaloneOwner>),
}
impl ScheduledOwner {
    fn live(&self) -> bool {
        match self {
            #[cfg(unix)]
            Self::Context(owner) => owner.strong_count() != 0,
            #[cfg(feature = "v8-host")]
            Self::Standalone(owner) => owner.strong_count() != 0,
        }
    }
    fn dispatch(&self, page: PageId, token: &DeferredFetchToken) -> bool {
        match self {
            #[cfg(unix)]
            Self::Context(owner) => owner.upgrade().is_none_or(|inner| {
                BrowserContext { inner }
                    .dispatch_deferred_fetch(page, token)
                    .is_ok()
            }),
            #[cfg(feature = "v8-host")]
            Self::Standalone(owner) => owner
                .upgrade()
                .is_none_or(|owner| owner.dispatch(token).is_ok()),
        }
    }
}
struct Scheduled {
    context: ScheduledOwner,
    page: PageId,
    token: DeferredFetchToken,
    activation: Weak<AtomicU8>,
    deadline: Option<Instant>,
}
struct Scheduler {
    jobs: Mutex<Vec<Arc<Scheduled>>>,
    changed: Condvar,
}
static SCHEDULER: OnceLock<Result<Arc<Scheduler>, String>> = OnceLock::new();
fn scheduler() -> Result<&'static Arc<Scheduler>, DeferredFetchError> {
    SCHEDULER
        .get_or_init(|| {
            let scheduler = Arc::new(Scheduler {
                jobs: Mutex::new(Vec::new()),
                changed: Condvar::new(),
            });
            let worker = scheduler.clone();
            std::thread::Builder::new()
                .name("oryn-deferred-fetch".into())
                .spawn(move || worker.run())
                .map_err(|error| error.to_string())?;
            Ok(scheduler)
        })
        .as_ref()
        .map_err(transport)
}
impl Scheduler {
    fn reserve(
        &self,
        context: ScheduledOwner,
        page: PageId,
        token: DeferredFetchToken,
        activation: Weak<AtomicU8>,
        deadline: Option<Instant>,
    ) -> Result<(), DeferredFetchError> {
        let mut jobs = self.jobs.lock().map_err(transport)?;
        jobs.retain(|job| {
            job.context.live()
                && job
                    .activation
                    .upgrade()
                    .is_some_and(|state| state.load(Ordering::Acquire) != 2)
        });
        if jobs.len() >= MAX_CAPABILITIES {
            return Err(DeferredFetchError::Quota(
                "deferred Fetch scheduler capacity exceeded".into(),
            ));
        }
        jobs.push(Arc::new(Scheduled {
            context,
            page,
            token,
            activation,
            deadline,
        }));
        self.changed.notify_one();
        Ok(())
    }
    fn run(&self) {
        loop {
            let ready = {
                let jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                let (mut jobs, _) = self
                    .changed
                    .wait_timeout(jobs, Duration::from_millis(100))
                    .unwrap_or_else(|error| error.into_inner());
                jobs.retain(|job| {
                    job.context.live()
                        && job
                            .activation
                            .upgrade()
                            .is_some_and(|state| state.load(Ordering::Acquire) != 2)
                });
                for job in jobs.iter() {
                    if job
                        .deadline
                        .is_some_and(|deadline| deadline <= Instant::now())
                    {
                        if let Some(state) = job.activation.upgrade() {
                            let _ =
                                state.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                        }
                    }
                }
                jobs.iter()
                    .filter(|job| {
                        job.activation
                            .upgrade()
                            .is_some_and(|state| state.load(Ordering::Acquire) == 1)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            for job in ready {
                let complete = job.context.dispatch(job.page, &job.token);
                if complete {
                    self.jobs
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .retain(|other| !Arc::ptr_eq(other, &job));
                }
            }
        }
    }
}

#[cfg(unix)]
include!("deferred_fetch_context.rs");

fn deliver(
    broker: &SharedNetworkBroker,
    scope: &Scope,
    plan: &Plan,
    ticket: &Ticket,
    mut report: impl FnMut(SecurityReport),
) -> Result<(), String> {
    use oryn_net::cors::{
        cors_headers_allowed, cors_non_safelisted_headers, cors_preflight_required,
    };
    // Fetch PR1647 queue-a-deferred-fetch sets service-workers mode none.
    // This frozen native request deliberately never dispatches a worker event.
    let deadline = Instant::now() + Duration::from_secs(30);
    let fetch = plan
        .request
        .fetch
        .as_ref()
        .ok_or("missing deferred Fetch options")?;
    let cors = matches!(fetch.mode.as_str(), "cors" | "same-origin");
    let mut target = Url::parse(&plan.request.url).map_err(|error| error.to_string())?;
    target.set_fragment(None);
    let mut method = plan.request.method.clone();
    let mut body = plan.request.body.clone();
    let mut base = crate::network::sanitize_worker_headers(plan.request.headers.clone());
    if !cors {
        base.retain(|(name, value)| oryn_common::http_headers::no_cors_request_header(name, value));
    }
    let mut request_origin = plan.origin.clone();
    let mut opaque = plan.opaque;
    let mut referrer_policy = plan.referrer;
    let mut referrer_source = if plan.request.initiator_url.as_deref() == Some("") {
        None
    } else {
        plan.request
            .initiator_url
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .filter(|url| !plan.opaque && url.origin().ascii_serialization() == plan.origin)
            .or_else(|| Some(plan.source.clone()))
    };
    let mut metadata = crate::fetch_metadata::Metadata::resource(&fetch.mode, "", plan.opaque);
    let initial_site = crate::cookies::SchemefulSite::from_url(&target);
    let mut crossed = false;
    for hop in 0..=20u64 {
        if ticket.cancelled() || Instant::now() >= deadline {
            return Err("deferred Fetch cancelled or timed out".into());
        }
        if let Some(upgraded) = plan.policy.upgraded_url(&target) {
            target = upgraded;
        }
        if !matches!(target.scheme(), "http" | "https")
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return Err("invalid deferred Fetch redirect URL".into());
        }
        let decision = plan.policy.authorize_redirect_from(
            &crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Connect,
                url: target.to_string(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            },
            hop != 0,
            &plan.request.url,
        );
        for violation in decision.violations {
            report(SecurityReport::Csp(violation));
        }
        if !decision.allowed {
            return Err("deferred Fetch CSP denied".into());
        }
        crate::mixed_content::enforce(
            if plan.source.scheme() == "https" {
                plan.source.as_str()
            } else {
                &plan.origin
            },
            &target,
        )
        .map_err(|error| error.to_string())?;
        let cross = opaque || target.origin().ascii_serialization() != plan.origin;
        if fetch.mode == "same-origin" && cross {
            return Err("same-origin deferred Fetch crossed an origin".into());
        }
        let credentials = match plan.request.credentials.as_str() {
            "include" => true,
            "same-origin" => !cross,
            _ => false,
        } && !plan.embedder.omit_credentials(cors, cross);
        metadata.observe(Some(&plan.origin), &target);
        let mut headers = base.clone();
        if let Some(value) = crate::referrer::request_origin_header(
            &request_origin,
            &plan.source,
            &target,
            &method,
            cors && cross,
            referrer_policy,
        ) {
            headers.push(("origin".into(), value));
        }
        let referrer = referrer_source
            .as_ref()
            .and_then(|source| referrer_policy.header(source, &target));
        referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
        if let Some(value) = referrer {
            headers.push(("referer".into(), value));
        }
        let security = crate::cookies::CookieRequestContext {
            fetch_metadata: metadata.clone(),
            fetch_cache: fetch.cache,
            credentials: if credentials {
                crate::cookies::CookieCredentials::Include
            } else {
                crate::cookies::CookieCredentials::Omit
            },
            kind: crate::cookies::CookieRequestKind::Fetch,
            initiator_url: Some(plan.origin.clone()),
            top_level_url: Some(plan.top.clone()),
            ancestor_urls: plan.ancestors.clone(),
            redirect_chain_cross_site: crossed,
            ..Default::default()
        };
        if cors
            && cross
            && (plan.request.use_cors_preflight || cors_preflight_required(&method, &headers))
        {
            let names = cors_non_safelisted_headers(&headers);
            let mut preflight_headers = vec![
                ("origin".into(), request_origin.clone()),
                ("access-control-request-method".into(), method.clone()),
            ];
            if !names.is_empty() {
                preflight_headers.push(("access-control-request-headers".into(), names.join(", ")));
            }
            let cleanup = Ticket::reserve(scope, hop * 2 + 1, broker.clone())
                .map_err(|error| error.to_string())?;
            cleanup.attach(hop * 2 + 1);
            let response = crate::network::collect_operation_bounded_with_cancel(
                broker,
                NetworkRequest {
                    request_id: hop * 2 + 1,
                    method: "OPTIONS".into(),
                    url: target.to_string(),
                    headers: preflight_headers,
                    body: Vec::new(),
                    credentials: false,
                    resolved_addrs: Vec::new(),
                    security: crate::cookies::CookieRequestContext {
                        credentials: crate::cookies::CookieCredentials::Omit,
                        ..security.clone()
                    },
                },
                deadline.saturating_duration_since(Instant::now()),
                64 * 1024,
                ticket.cancellation(),
            )
            .map_err(|error| error.to_string())?;
            if !oryn_net::cors::cors_preflight_allowed(
                &request_origin,
                credentials,
                &method,
                &names,
                response.status,
                &response.headers,
            ) {
                return Err("cross-origin request denied by CORS preflight".into());
            }
        }
        if ticket.keepalive().is_none() {
            let owner = issuer(plan.page, &plan.creator).map_err(|error| format!("{error:?}"))?;
            ticket.retain_keepalive(Arc::new(plan.keepalive.reserve(owner, body.len())?));
        }
        let cleanup = Ticket::reserve(scope, hop * 2 + 2, broker.clone())
            .map_err(|error| error.to_string())?;
        cleanup.attach(hop * 2 + 2);
        if let Some(lease) = ticket.keepalive() {
            cleanup.retain_keepalive(lease);
        }
        let response = crate::network::collect_operation_bounded_with_cancel(
            broker,
            NetworkRequest {
                request_id: hop * 2 + 2,
                method: method.clone(),
                url: target.to_string(),
                headers,
                body: body.clone(),
                credentials,
                resolved_addrs: Vec::new(),
                security,
            },
            deadline.saturating_duration_since(Instant::now()),
            16 * 1024 * 1024,
            ticket.cancellation(),
        )
        .map_err(|error| error.to_string())?;
        cleanup.finish_keepalive();
        report(SecurityReport::Coep(plan.coep.response(
            &plan.origin,
            plan.opaque,
            &target,
            cors,
            credentials,
            &response.headers,
            "",
        )));
        plan.embedder.authorize_response(
            &plan.origin,
            plan.opaque,
            &target,
            cors,
            credentials,
            &response.headers,
        )?;
        if cors && cross && !cors_headers_allowed(&request_origin, credentials, &response.headers) {
            return Err("deferred Fetch CORS response denied".into());
        }
        if oryn_net::fetch::is_redirect_status(response.status)
            && let Some(location) = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                .map(|(_, value)| value)
        {
            if fetch.redirect == "error" {
                return Err("deferred Fetch redirect mode is error".into());
            }
            if fetch.redirect == "manual" {
                crate::integrity::verify(&fetch.integrity, &[], false)
                    .map_err(|error| format!("deferred Fetch integrity failed: {error:?}"))?;
                return Ok(());
            }
            if hop == 20 {
                return Err("deferred Fetch redirect limit exceeded".into());
            }
            let mut next = target.join(location).map_err(|error| error.to_string())?;
            next.set_fragment(None);
            oryn_net::fetch::rewrite_redirect_request(
                response.status,
                &target,
                &next,
                &mut method,
                &mut base,
                &mut body,
            );
            if let Some(lease) = ticket.keepalive() {
                lease.shrink_to(body.len());
            }
            referrer_policy = referrer_policy.after_response(&response.headers);
            if target.origin() != next.origin() && target.origin() != plan.source.origin() {
                opaque = true;
                request_origin = "null".into();
            }
            crossed |= crate::cookies::SchemefulSite::from_url(&next) != initial_site;
            target = next;
        } else {
            crate::integrity::verify(&fetch.integrity, &response.body, cors || !cross)
                .map_err(|error| format!("deferred Fetch integrity failed: {error:?}"))?;
            return Ok(());
        }
    }
    Err("deferred Fetch redirect limit exceeded".into())
}

#[cfg(feature = "v8-host")]
pub(crate) struct StandaloneAuthority {
    pub page: PageId,
    pub creator: Creator,
    pub control: AgentOwner,
    pub quota: usize,
    pub source: Url,
    pub origin: String,
    pub top: String,
    pub ancestors: Vec<String>,
    pub policy: crate::csp::CspPolicySet,
}
/// Owned by the standalone isolate, independent of any one Document's V8 state.
#[cfg(feature = "v8-host")]
pub(crate) struct StandaloneOwner {
    keepalive: crate::keepalive::Budget,
    registry: Mutex<Registry>,
    broker: SharedNetworkBroker,
    reports: Arc<crate::executable::module_graph_load::ScriptCspReports>,
}
#[cfg(feature = "v8-host")]
impl StandaloneOwner {
    pub(crate) fn reserve_keepalive(
        &self,
        page: PageId,
        creator: &Creator,
        bytes: usize,
    ) -> Result<crate::keepalive::Lease, String> {
        self.keepalive.reserve(
            issuer(page, creator).map_err(|error| format!("{error:?}"))?,
            bytes,
        )
    }
    pub(crate) fn new(
        broker: SharedNetworkBroker,
        reports: Arc<crate::executable::module_graph_load::ScriptCspReports>,
    ) -> Self {
        Self {
            keepalive: Default::default(),
            registry: Mutex::new(Registry::default()),
            broker,
            reports,
        }
    }
    pub(crate) fn backend(
        self: &Arc<Self>,
        authority: StandaloneAuthority,
        live: std::rc::Rc<dyn Fn() -> bool>,
    ) -> std::rc::Rc<dyn oryn_js::fetch_later::DeferredFetchBackend> {
        let StandaloneAuthority {
            page,
            creator,
            control,
            quota,
            source,
            origin,
            top,
            ancestors,
            policy,
        } = authority;
        let headers = policy.worker_snapshot().headers;
        std::rc::Rc::new(StandaloneBackend {
            owner: self.clone(),
            live,
            authority: Plan {
                keepalive: self.keepalive.clone(),
                page,
                creator,
                control,
                quota,
                source,
                opaque: origin == "null",
                origin,
                top,
                ancestors,
                embedder: crate::agent_isolation::DocumentIsolationPolicy::from_headers(&headers),
                coep: crate::coep_reporting::Policy::new(
                    policy.worker_snapshot().url.as_str(),
                    &headers,
                ),
                referrer: crate::referrer::ReferrerPolicy::default().after_response(&headers),
                policy,
                bytes: 0,
                request: PlatformHttpRequest {
                    upgrade_mixed_image: false,
                    fetch: None,
                    use_cors_preflight: false,
                    method: String::new(),
                    url: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    credentials: String::new(),
                    initiator_url: None,
                    referrer_policy: None,
                },
            },
        })
    }
    pub(crate) fn quota(&self, control: &AgentOwner, normal: bool, minimal: bool) -> usize {
        self.registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reservations
            .get(&container_key(control))
            .filter(|value| if value.minimal { minimal } else { normal })
            .map_or(0, |value| value.bytes)
    }
    pub(crate) fn reserve(
        &self,
        authority: &StandaloneAuthority,
        frame: String,
        normal: bool,
        minimal: bool,
    ) -> Result<(), String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "standalone deferred registry poisoned")?;
        let key = (authority.page, frame);
        registry.reservations.remove(&key);
        let used = registry
            .entries
            .values()
            .filter(|entry| entry.activation.load(Ordering::Acquire) == 0)
            .filter_map(|entry| entry.pending.as_ref())
            .filter(|plan| plan.control == authority.control)
            .fold(0usize, |sum, plan| sum.saturating_add(plan.bytes));
        let delegated = registry
            .reservations
            .values()
            .filter(|value| value.control == authority.control && !value.minimal)
            .fold(0usize, |sum, value| sum.saturating_add(value.bytes));
        let normal = normal
            && authority
                .quota
                .saturating_sub(used.saturating_add(delegated))
                >= ORIGIN_BYTES;
        let minimal = !normal
            && minimal
            && matches!(
                &authority.control,
                AgentOwner::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    ..
                }
            )
            && registry
                .reservations
                .values()
                .filter(|value| value.control == authority.control && value.minimal)
                .count()
                < MINIMAL_POOL / CHILD_BYTES;
        if registry.reservations.len() >= MAX_CAPABILITIES {
            return Err("standalone deferred containers exhausted".into());
        }
        registry.reservations.insert(
            key,
            Reservation {
                control: authority.control.clone(),
                bytes: if normal {
                    ORIGIN_BYTES
                } else if minimal {
                    CHILD_BYTES
                } else {
                    0
                },
                minimal,
            },
        );
        Ok(())
    }
    pub(crate) fn retire_creator(&self, page: PageId, creator: &Creator) {
        if let Ok(owner) = issuer(page, creator) {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for entry in registry
                .entries
                .values()
                .filter(|entry| entry.page == page && &entry.creator == creator)
            {
                let _ =
                    entry
                        .activation
                        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
            }
            registry.retire_issuer(&owner);
            if let Some(Ok(pool)) = SCHEDULER.get() {
                pool.changed.notify_one();
            }
        }
    }
    pub(crate) fn remove(&self, frame: &str) {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        registry.reservations.retain(|(_, token), _| token != frame);
    }

    fn dispatch(&self, token: &DeferredFetchToken) -> Result<(), DeferredFetchError> {
        let mut registry = self.registry.lock().map_err(transport)?;
        if registry.closed {
            return Ok(());
        }
        let Some(entry) = registry.entries.get(&token.0) else {
            return Ok(());
        };
        if entry.activation.load(Ordering::Acquire) != 1 {
            return Ok(());
        }
        let Some(plan) = entry.pending.clone() else {
            return Ok(());
        };
        let scope = registry.scope.clone();
        let broker = self.broker.clone().new_request_scope();
        let ticket = Ticket::reserve(&scope, 0, broker.clone()).map_err(transport)?;
        let reports = self.reports.clone();
        crate::resource_io_executor::submit(move || {
            if !ticket.cancelled() {
                let _ = deliver(
                    &broker,
                    &scope,
                    &plan,
                    &ticket,
                    |violation| match violation {
                        SecurityReport::Csp(violation) => {
                            let _ = reports.push_with_policy(&plan.policy, vec![violation]);
                        }
                        SecurityReport::Coep(values) => reports.push_coep(values),
                    },
                );
            }
        })
        .map_err(transport)?;
        registry.submitted(&token.0);
        Ok(())
    }
}
#[cfg(feature = "v8-host")]
struct StandaloneBackend {
    owner: Arc<StandaloneOwner>,
    authority: Plan,
    live: std::rc::Rc<dyn Fn() -> bool>,
}
#[cfg(feature = "v8-host")]
impl oryn_js::fetch_later::DeferredFetchBackend for StandaloneBackend {
    fn prepare(
        &self,
        token: &DeferredFetchToken,
        request: PlatformHttpRequest,
        activate_after: Option<Duration>,
    ) -> Result<(), DeferredFetchError> {
        let hash = deferred_fingerprint(&request, activate_after)?;
        if self.owner.registry.lock().map_err(transport)?.replay(
            self.authority.page,
            &self.authority.creator,
            token,
            &hash,
        )? {
            return Ok(());
        }
        if !(self.live)() {
            return Err(invalid("deferred Fetch issuer retired"));
        }
        let target = Url::parse(&request.url).map_err(transport)?;
        if !matches!(target.scheme(), "http" | "https")
            || !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(
                target.as_str(),
            ))
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return Err(invalid("invalid deferred Fetch URL"));
        }
        let fetch = request
            .fetch
            .as_ref()
            .ok_or_else(|| invalid("missing deferred Fetch options"))?;
        fetch.validate().map_err(invalid)?;
        if request.upgrade_mixed_image || fetch.privileged_range || fetch.mode == "navigate" {
            return Err(invalid("invalid deferred Fetch authority"));
        }
        let method = tungstenite::http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| invalid("invalid deferred Fetch method"))?;
        if matches!(
            method.as_str().to_ascii_uppercase().as_str(),
            "CONNECT" | "TRACE" | "TRACK"
        ) || (!request.body.is_empty() && matches!(method.as_str(), "GET" | "HEAD"))
        {
            return Err(invalid("invalid deferred Fetch method/body"));
        }
        if !matches!(
            request.credentials.as_str(),
            "include" | "omit" | "same-origin"
        ) || (fetch.mode == "no-cors" && !matches!(method.as_str(), "GET" | "HEAD" | "POST"))
        {
            return Err(invalid("invalid deferred Fetch credentials/mode"));
        }
        let mut plan = self.authority.clone();
        if let Some(value) = request
            .referrer_policy
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            plan.referrer = crate::referrer::ReferrerPolicy::parse_list(value);
        }
        let source = if request.initiator_url.as_deref() == Some("") {
            None
        } else {
            request
                .initiator_url
                .as_deref()
                .and_then(|value| Url::parse(value).ok())
                .filter(|source| {
                    !plan.opaque && source.origin().ascii_serialization() == plan.origin
                })
                .or_else(|| Some(plan.source.clone()))
        };
        let referrer_bytes = source
            .as_ref()
            .and_then(|source| plan.referrer.header(source, &target))
            .map_or(0, |value| value.len());
        plan.bytes = oryn_js::fetch_later::request_bytes(&request)
            .and_then(|bytes| bytes.checked_add(referrer_bytes))
            .ok_or_else(|| DeferredFetchError::Quota("deferred size overflow".into()))?;
        plan.request = request;
        crate::resource_io_executor::ready().map_err(transport)?;
        crate::http_cleanup_custody::ready().map_err(transport)?;
        let deadline = activate_after
            .map(|delay| {
                Instant::now()
                    .checked_add(delay)
                    .ok_or_else(|| invalid("deferred Fetch deadline overflow"))
            })
            .transpose()?;
        let activation = Arc::new(AtomicU8::new(3));
        scheduler()?.reserve(
            ScheduledOwner::Standalone(Arc::downgrade(&self.owner)),
            self.authority.page,
            token.clone(),
            Arc::downgrade(&activation),
            deadline,
        )?;
        self.owner.registry.lock().map_err(transport)?.insert(
            token.clone(),
            plan,
            hash,
            activation,
            deadline,
        )
    }
    fn activate(
        &self,
        token: &DeferredFetchToken,
    ) -> Result<DeferredFetchState, DeferredFetchError> {
        let registry = self.owner.registry.lock().map_err(transport)?;
        let _ = registry.state(self.authority.page, token)?;
        if let Some(entry) = registry.entries.get(&token.0) {
            let _ = entry
                .activation
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        }
        if let Some(Ok(pool)) = SCHEDULER.get() {
            pool.changed.notify_one();
        }
        registry.state(self.authority.page, token)
    }
    fn cancel(&self, token: &DeferredFetchToken) -> Result<DeferredFetchState, DeferredFetchError> {
        let mut registry = self.owner.registry.lock().map_err(transport)?;
        if !registry.entries.contains_key(&token.0)
            && !registry.cancelled.contains_key(&token.0)
            && !(self.live)()
        {
            return Err(invalid("deferred Fetch issuer retired"));
        }
        registry.cancel(self.authority.page, &self.authority.creator, token)
    }
    fn acknowledge(&self, token: &DeferredFetchToken) -> Result<(), DeferredFetchError> {
        self.owner
            .registry
            .lock()
            .map_err(transport)?
            .acknowledge(self.authority.page, token)
    }
    fn state(&self, token: &DeferredFetchToken) -> Result<DeferredFetchState, DeferredFetchError> {
        self.owner
            .registry
            .lock()
            .map_err(transport)?
            .state(self.authority.page, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::WindowTarget;
    fn owner(page: u64, generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(page),
            target: WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }
    fn plan(page: u64, generation: u64, destination: &str, bytes: usize) -> Plan {
        Plan {
            keepalive: Default::default(),
            page: PageId(page),
            creator: Creator::Window {
                target: WindowTarget::Root,
                document_generation: generation,
                stream_epoch: 0,
            },
            source: Url::parse("https://source.test/document").unwrap(),
            origin: "https://source.test".into(),
            opaque: false,
            top: "https://source.test/document".into(),
            ancestors: Vec::new(),
            policy: crate::csp::CspPolicySet::from_headers("https://source.test/document", &[]),
            embedder: Default::default(),
            coep: Default::default(),
            referrer: Default::default(),
            request: PlatformHttpRequest {
                upgrade_mixed_image: false,
                fetch: Some(oryn_js::platform::FetchOptions {
                    keepalive: true,
                    privileged_range: false,
                    cache: Default::default(),
                    mode: "cors".into(),
                    redirect: "follow".into(),
                    integrity: String::new(),
                }),
                use_cors_preflight: false,
                method: "POST".into(),
                url: destination.into(),
                headers: Vec::new(),
                body: vec![1],
                credentials: "same-origin".into(),
                initiator_url: Some(String::new()),
                referrer_policy: Some("no-referrer".into()),
            },
            bytes,
            control: owner(page, generation),
            quota: TOP_BYTES - MINIMAL_POOL,
        }
    }
    fn admit(
        registry: &mut Registry,
        id: u8,
        plan: Plan,
    ) -> Result<Arc<AtomicU8>, DeferredFetchError> {
        let state = Arc::new(AtomicU8::new(0));
        let hash = fingerprint(&plan.request)?;
        registry.insert(
            DeferredFetchToken(vec![id; 32]),
            plan,
            hash,
            state.clone(),
            None,
        )?;
        Ok(state)
    }
    #[derive(Clone)]
    struct Broker {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        responses: Arc<Mutex<std::collections::VecDeque<(u16, Vec<(String, String)>)>>>,
    }
    impl crate::network::NetworkBroker for Broker {
        fn send(
            &self,
            request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            let id = request.request_id;
            self.requests.lock().unwrap().push(request);
            let (status, headers) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected deferred Fetch IO");
            Ok(crate::network::NetworkResponse {
                request_id: id,
                status,
                headers,
                body: Vec::new(),
                timing: None,
            })
        }
    }
    fn broker(
        responses: Vec<(u16, Vec<(String, String)>)>,
    ) -> (SharedNetworkBroker, Arc<Mutex<Vec<NetworkRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            SharedNetworkBroker::new(Broker {
                requests: requests.clone(),
                responses: Arc::new(Mutex::new(responses.into())),
            }),
            requests,
        )
    }
    #[cfg(unix)]
    #[test]
    fn production_prepare_uses_document_url_not_storage_partition_and_freezes_authority() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://source.test/old".into())
            .unwrap();
        context
            .install_document_security(id, "https://source.test/old".into(), &[])
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
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let token = DeferredFetchToken(vec![10; 32]);
        let request = plan(id.0, 1, "https://report.test/", 1).request;
        context
            .prepare_deferred_fetch(id, creator.clone(), token.clone(), request.clone(), None)
            .unwrap();
        context
            .prepare_deferred_fetch(id, creator, token.clone(), request, None)
            .unwrap();
        {
            let registry = context.inner.deferred_fetches.lock().unwrap();
            let plan = registry.entries[&token.0].pending.as_ref().unwrap();
            assert_eq!(plan.source.as_str(), "https://source.test/old");
            assert_eq!(plan.origin, "https://source.test");
        }
        context
            .cancel_deferred_fetch(
                id,
                &plan(id.0, 1, "https://report.test/", 1).creator,
                &token,
            )
            .unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn navigation_reservations_apply_before_fetch_and_cap_minimal_children() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(id, "https://source.test/".into(), &[])
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
        let parent = owner(id.0, 1);
        for index in 1..=17 {
            context
                .reserve_deferred_frame(
                    &parent,
                    id,
                    format!("{index:064x}"),
                    "https://child.test/",
                    "",
                )
                .unwrap();
        }
        {
            let registry = context.inner.deferred_fetches.lock().unwrap();
            assert_eq!(
                registry
                    .reservations
                    .values()
                    .filter(|value| value.bytes == CHILD_BYTES)
                    .count(),
                16
            );
            assert_eq!(
                registry.reservations[&(id, format!("{:064x}", 17))].bytes,
                0
            );
        }
        context.remove_deferred_container(id, &format!("{:064x}", 1));
        context
            .reserve_deferred_frame(
                &parent,
                id,
                format!("{:064x}", 17),
                "https://child.test/",
                "",
            )
            .unwrap();
        assert_eq!(
            context.inner.deferred_fetches.lock().unwrap().reservations
                [&(id, format!("{:064x}", 17))]
                .bytes,
            CHILD_BYTES
        );
        context
            .reserve_deferred_frame(
                &parent,
                id,
                "f".repeat(64),
                "https://delegated.test/",
                "deferred-fetch *",
            )
            .unwrap();
        assert_eq!(
            context.inner.deferred_fetches.lock().unwrap().reservations[&(id, "f".repeat(64))]
                .bytes,
            ORIGIN_BYTES
        );
        assert!(
            context
                .reserve_deferred_frame(
                    &parent,
                    PageId(id.0 + 1),
                    "e".repeat(64),
                    "https://child.test/",
                    ""
                )
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn same_origin_children_cannot_borrow_quota_through_disabled_own_policy() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(id, "https://source.test/".into(), &[])
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
        for (index, headers, allow, permitted) in [
            (
                1,
                vec![(
                    "permissions-policy".into(),
                    "deferred-fetch=(), deferred-fetch-minimal=()".into(),
                )],
                "",
                false,
            ),
            (
                2,
                Vec::new(),
                "deferred-fetch 'none'; deferred-fetch-minimal 'none'",
                false,
            ),
            (3, Vec::new(), "", true),
        ] {
            let frame = format!("{index:064x}");
            let url = "https://source.test/frame";
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 1,
                        stream_epoch: 0,
                        origin: "https://source.test".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            let policy = context
                .prepare_frame_worker_policy(id, url, &headers, "")
                .unwrap();
            context
                .bind_frame_worker_policy(id, frame.clone(), 1, 0, url, &policy)
                .unwrap();
            context
                .register_activation_parent(id, frame.clone(), 1, 0, WindowTarget::Root, 1, 0)
                .unwrap();
            context
                .register_isolation_container(
                    id,
                    frame.clone(),
                    1,
                    0,
                    allow.into(),
                    url.into(),
                    None,
                )
                .unwrap();
            let creator = Creator::Window {
                target: WindowTarget::Frame(frame),
                document_generation: 1,
                stream_epoch: 0,
            };
            let token = sequenced(index);
            let result = context.prepare_deferred_fetch(
                id,
                creator.clone(),
                token.clone(),
                plan(id.0, 1, "https://report.test/", 1).request,
                None,
            );
            assert_eq!(result.is_ok(), permitted);
            if permitted {
                context.cancel_deferred_fetch(id, &creator, &token).unwrap();
            }
        }
    }
    #[cfg(feature = "v8-host")]
    fn standalone_authority(page: u64) -> StandaloneAuthority {
        let plan = plan(page, 1, "https://report.test/", 1);
        StandaloneAuthority {
            page: plan.page,
            creator: plan.creator,
            control: plan.control,
            quota: plan.quota,
            source: plan.source,
            origin: plan.origin,
            top: plan.top,
            ancestors: plan.ancestors,
            policy: plan.policy,
        }
    }
    #[cfg(feature = "v8-host")]
    #[test]
    fn standalone_roots_have_independent_first_sequence_and_retired_provider_cannot_reopen_issuer()
    {
        let (network, _) = broker(Vec::new());
        let owner = Arc::new(StandaloneOwner::new(network, Arc::new(Default::default())));
        let live = std::rc::Rc::new(std::cell::Cell::new(true));
        let check = live.clone();
        let first = owner.backend(
            standalone_authority(1),
            std::rc::Rc::new(move || check.get()),
        );
        let second = owner.backend(standalone_authority(2), std::rc::Rc::new(|| true));
        let first_token = sequenced(1);
        let mut second_token = sequenced(1);
        second_token.0[31] = 10;
        first
            .prepare(
                &first_token,
                plan(1, 1, "https://report.test/", 1).request,
                None,
            )
            .unwrap();
        second
            .prepare(
                &second_token,
                plan(2, 1, "https://report.test/", 1).request,
                None,
            )
            .unwrap();
        first.cancel(&first_token).unwrap();
        second.cancel(&second_token).unwrap();
        assert_eq!(owner.registry.lock().unwrap().watermarks.len(), 2);
        live.set(false);
        owner.retire_creator(PageId(1), &plan(1, 1, "https://report.test/", 1).creator);
        assert!(
            first
                .prepare(
                    &sequenced(2),
                    plan(1, 1, "https://report.test/", 1).request,
                    None
                )
                .is_err()
        );
        assert!(first.cancel(&sequenced(2)).is_err());
        assert_eq!(owner.registry.lock().unwrap().watermarks.len(), 1);
    }
    #[test]
    fn activated_deferred_fetch_observes_ordinary_keepalive_body_budget() {
        let (network, requests) = broker(Vec::new());
        let plan = plan(1, 1, "https://source.test/", 1);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 913, network.clone()).unwrap();
        let ordinary = plan.keepalive.reserve(owner(1, 1), 64 * 1024).unwrap();
        assert!(
            deliver(&network, &scope, &plan, &ticket, |_| {})
                .unwrap_err()
                .contains("keepalive body budget")
        );
        assert!(requests.lock().unwrap().is_empty());
        drop(ordinary);
        assert!(plan.keepalive.reserve(owner(1, 1), 64 * 1024).is_ok());
    }
    #[test]
    fn native_deadline_beats_late_abort_and_canceled_deadline_never_reactivates() {
        let mut registry = Registry::default();
        let first = plan(1, 1, "https://source.test/", 1);
        let hash = fingerprint(&first.request).unwrap();
        let due = sequenced(1);
        registry
            .insert(
                due.clone(),
                first.clone(),
                hash,
                Arc::new(AtomicU8::new(3)),
                Some(Instant::now() - Duration::from_secs(1)),
            )
            .unwrap();
        assert_eq!(
            registry.cancel(PageId(1), &first.creator, &due).unwrap(),
            DeferredFetchState::Activated
        );
        assert_eq!(
            registry.state(PageId(1), &due).unwrap(),
            DeferredFetchState::Activated
        );
        let canceled = sequenced(2);
        registry
            .insert(
                canceled.clone(),
                first.clone(),
                hash,
                Arc::new(AtomicU8::new(3)),
                Some(Instant::now() + Duration::from_secs(60)),
            )
            .unwrap();
        assert_eq!(
            registry
                .cancel(PageId(1), &first.creator, &canceled)
                .unwrap(),
            DeferredFetchState::Canceled
        );
        promote_due(
            &registry.entries[&canceled.0],
            Instant::now() + Duration::from_secs(120),
        );
        assert_eq!(
            registry.state(PageId(1), &canceled).unwrap(),
            DeferredFetchState::Canceled
        );
        assert_ne!(
            deferred_fingerprint(&first.request, None).unwrap(),
            deferred_fingerprint(&first.request, Some(Duration::ZERO)).unwrap()
        );
        assert_ne!(
            deferred_fingerprint(&first.request, Some(Duration::from_millis(1))).unwrap(),
            deferred_fingerprint(&first.request, Some(Duration::from_millis(2))).unwrap()
        );
    }
    #[cfg(feature = "v8-host")]
    #[test]
    fn scheduler_sends_expired_standalone_request_without_any_js_poll_or_activation_call() {
        struct DeadlineBroker(std::sync::mpsc::Sender<()>);
        impl crate::network::NetworkBroker for DeadlineBroker {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                self.0.send(()).unwrap();
                Ok(crate::network::NetworkResponse {
                    request_id: request.request_id,
                    status: 204,
                    headers: Vec::new(),
                    body: Vec::new(),
                    timing: None,
                })
            }
        }
        let (sent, received) = std::sync::mpsc::channel();
        let owner = Arc::new(StandaloneOwner::new(
            SharedNetworkBroker::new(DeadlineBroker(sent)),
            Arc::new(Default::default()),
        ));
        let backend = owner.backend(standalone_authority(1), std::rc::Rc::new(|| true));
        let token = sequenced(1);
        backend
            .prepare(
                &token,
                plan(1, 1, "https://source.test/", 1).request,
                Some(Duration::ZERO),
            )
            .unwrap();
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            backend.state(&token).unwrap(),
            DeferredFetchState::Activated
        );
        assert_eq!(
            backend.cancel(&token).unwrap(),
            DeferredFetchState::Activated
        );
        assert_eq!(
            backend.activate(&token).unwrap(),
            DeferredFetchState::Activated
        );
        assert!(received.try_recv().is_err());
    }
    #[test]
    fn native_preflight_acceptance_precedes_actual_request_and_enforces_credentials() {
        let mut plan = plan(1, 1, "https://report.test/", 1);
        plan.request.method = "PUT".into();
        plan.request.headers = vec![("x-deferred".into(), "yes".into())];
        plan.request.credentials = "include".into();
        let cors = vec![
            (
                "access-control-allow-origin".into(),
                "https://source.test".into(),
            ),
            ("access-control-allow-credentials".into(), "true".into()),
        ];
        let mut preflight = cors.clone();
        preflight.extend([
            ("access-control-allow-methods".into(), "PUT".into()),
            ("access-control-allow-headers".into(), "x-deferred".into()),
        ]);
        let (network, requests) = broker(vec![(204, preflight), (204, cors)]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 900, network.clone()).unwrap();
        deliver(&network, &scope, &plan, &ticket, |_| {}).unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "OPTIONS");
        assert!(!requests[0].credentials);
        assert_eq!(requests[1].method, "PUT");
        assert!(requests[1].credentials);
        assert_eq!(requests[1].body, vec![1]);
        assert!(
            !requests[1]
                .headers
                .iter()
                .any(|(name, _)| name == "referer")
        );
    }
    #[test]
    fn failed_preflight_never_sends_deferred_payload() {
        let mut plan = plan(1, 1, "https://report.test/", 1);
        plan.request.method = "PUT".into();
        let (network, requests) = broker(vec![(403, Vec::new())]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 901, network.clone()).unwrap();
        assert!(deliver(&network, &scope, &plan, &ticket, |_| {}).is_err());
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(requests.lock().unwrap()[0].method, "OPTIONS");
    }
    #[test]
    fn redirect_rechecks_frozen_csp_and_does_not_contact_blocked_target() {
        let mut plan = plan(1, 1, "https://source.test/first", 1);
        plan.policy = crate::csp::CspPolicySet::from_headers(
            plan.source.as_str(),
            &[(
                "content-security-policy".into(),
                "connect-src 'self'".into(),
            )],
        );
        let (network, requests) = broker(vec![(
            307,
            vec![("location".into(), "https://blocked.test/".into())],
        )]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 902, network.clone()).unwrap();
        let mut violations = Vec::new();
        assert!(
            deliver(&network, &scope, &plan, &ticket, |violation| violations
                .push(violation))
            .is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(
            violations
                .iter()
                .any(|report| matches!(report, SecurityReport::Csp(_)))
        );
    }
    #[test]
    fn redirected_return_to_source_retains_cors_taint_and_null_origin() {
        for allow in [true, false] {
            let plan = plan(1, 1, "https://source.test/start", 1);
            let mut final_headers = Vec::new();
            if allow {
                final_headers.push(("access-control-allow-origin".into(), "null".into()));
            }
            let (network, requests) = broker(vec![
                (
                    302,
                    vec![("location".into(), "https://report.test/away".into())],
                ),
                (
                    302,
                    vec![
                        ("location".into(), "https://source.test/final".into()),
                        ("access-control-allow-origin".into(), "*".into()),
                    ],
                ),
                (200, final_headers),
            ]);
            let scope = Scope::default();
            let ticket = Ticket::reserve(&scope, 990, network.clone()).unwrap();
            assert_eq!(
                deliver(&network, &scope, &plan, &ticket, |_| {}).is_ok(),
                allow
            );
            let requests = requests.lock().unwrap();
            assert!(
                requests[2]
                    .headers
                    .contains(&("origin".into(), "null".into()))
            );
            assert!(!requests[2].credentials);
        }
    }
    #[test]
    fn integrity_uses_final_representation_and_rejects_opaque_and_manual_redirect() {
        for (mode, redirect, url, status, headers, metadata, ok) in [
            (
                "cors",
                "follow",
                "https://source.test/",
                200,
                Vec::new(),
                "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=",
                true,
            ),
            (
                "cors",
                "follow",
                "https://source.test/",
                200,
                Vec::new(),
                "sha256-AA==",
                false,
            ),
            (
                "no-cors",
                "follow",
                "https://report.test/",
                200,
                Vec::new(),
                "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=",
                false,
            ),
            (
                "cors",
                "manual",
                "https://source.test/",
                302,
                vec![("location".into(), "/next".into())],
                "sha256-AA==",
                false,
            ),
        ] {
            let mut plan = plan(1, 1, url, 1);
            let fetch = plan.request.fetch.as_mut().unwrap();
            fetch.mode = mode.into();
            fetch.redirect = redirect.into();
            fetch.integrity = metadata.into();
            let (network, requests) = broker(vec![(status, headers)]);
            let scope = Scope::default();
            let ticket = Ticket::reserve(&scope, 909, network.clone()).unwrap();
            assert_eq!(
                deliver(&network, &scope, &plan, &ticket, |_| {}).is_ok(),
                ok
            );
            assert_eq!(requests.lock().unwrap().len(), 1);
        }
    }
    #[test]
    fn redirected_post_rewrites_body_auth_credentials_and_rejects_same_origin_mode() {
        let mut plan = plan(1, 1, "https://source.test/first#discarded", 1);
        plan.request
            .headers
            .push(("authorization".into(), "secret".into()));
        let (network, requests) = broker(vec![
            (
                303,
                vec![("location".into(), "https://report.test/last".into())],
            ),
            (
                200,
                vec![("access-control-allow-origin".into(), "*".into())],
            ),
        ]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 910, network.clone()).unwrap();
        deliver(&network, &scope, &plan, &ticket, |_| {}).unwrap();
        {
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(!requests[0].url.contains('#'));
            assert_eq!(requests[1].method, "GET");
            assert!(requests[1].body.is_empty());
            assert!(!requests[1].credentials);
            assert!(
                !requests[1]
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            );
        }
        plan.request.url = "https://report.test/".into();
        plan.request.fetch.as_mut().unwrap().mode = "same-origin".into();
        let (network, requests) = broker(Vec::new());
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 911, network.clone()).unwrap();
        assert!(deliver(&network, &scope, &plan, &ticket, |_| {}).is_err());
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn report_only_coep_uses_frozen_reporting_endpoint_without_blocking_payload() {
        let mut plan = plan(1, 1, "https://report.test/", 1);
        plan.request.fetch.as_mut().unwrap().mode = "no-cors".into();
        plan.coep = crate::coep_reporting::Policy::new(
            plan.source.as_str(),
            &[
                (
                    "cross-origin-embedder-policy-report-only".into(),
                    "require-corp; report-to=\"deferred\"".into(),
                ),
                (
                    "reporting-endpoints".into(),
                    "deferred=\"https://source.test/reports\"".into(),
                ),
            ],
        );
        let (network, _) = broker(vec![(200, Vec::new())]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 912, network.clone()).unwrap();
        let mut reports = Vec::new();
        deliver(&network, &scope, &plan, &ticket, |report| {
            reports.push(report)
        })
        .unwrap();
        assert!(reports.iter().any(|report|matches!(report,SecurityReport::Coep(values)if values.iter().any(|value|value.endpoint=="https://source.test/reports"))));
    }
    #[test]
    fn cancelled_scope_and_mixed_content_never_enter_broker() {
        let plan = plan(1, 1, "http://report.test/", 1);
        let (network, requests) = broker(Vec::new());
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 903, network.clone()).unwrap();
        assert!(deliver(&network, &scope, &plan, &ticket, |_| {}).is_err());
        scope.cancel(None);
        assert!(deliver(&network, &scope, &plan, &ticket, |_| {}).is_err());
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn reporting_origin_quota_is_per_control_group_and_allows_multiple_origins() {
        let mut registry = Registry::default();
        admit(
            &mut registry,
            1,
            plan(1, 1, "https://one.test/r", ORIGIN_BYTES),
        )
        .unwrap();
        admit(
            &mut registry,
            2,
            plan(1, 1, "https://two.test/r", ORIGIN_BYTES),
        )
        .unwrap();
        assert!(matches!(
            admit(&mut registry, 3, plan(1, 1, "https://one.test/r", 1)),
            Err(DeferredFetchError::Quota(_))
        ));
        let mut independent_child = plan(1, 2, "https://one.test/r", CHILD_BYTES);
        independent_child.quota = CHILD_BYTES;
        admit(&mut registry, 4, independent_child).unwrap();
        admit(
            &mut registry,
            5,
            plan(2, 1, "https://one.test/r", ORIGIN_BYTES),
        )
        .unwrap();
    }
    #[test]
    fn same_origin_siblings_share_but_cross_origin_siblings_have_independent_quota() {
        let mut registry = Registry::default();
        let root = plan(1, 1, "https://report.test/r", ORIGIN_BYTES);
        admit(&mut registry, 1, root.clone()).unwrap();
        let mut same_origin = root.clone();
        same_origin.creator = Creator::Window {
            target: WindowTarget::Frame("a".repeat(64)),
            document_generation: 2,
            stream_epoch: 0,
        };
        same_origin.bytes = 1;
        assert!(matches!(
            admit(&mut registry, 2, same_origin),
            Err(DeferredFetchError::Quota(_))
        ));
        for id in 3..5 {
            let mut cross = root.clone();
            cross.control = AgentOwner::Window {
                page: PageId(1),
                target: WindowTarget::Frame(format!("{id:064x}")),
                generation: 1,
                epoch: 0,
            };
            cross.bytes = CHILD_BYTES;
            cross.quota = CHILD_BYTES;
            admit(&mut registry, id, cross).unwrap();
        }
    }
    #[test]
    fn aggregate_pool_and_delegated_reservations_are_enforced() {
        let mut registry = Registry::default();
        let control = owner(1, 1);
        registry.reservations.insert(
            (PageId(1), "child".into()),
            Reservation {
                control: control.clone(),
                bytes: ORIGIN_BYTES,
                minimal: false,
            },
        );
        for id in 1..=7 {
            admit(
                &mut registry,
                id,
                plan(1, 1, &format!("https://r{id}.test/"), ORIGIN_BYTES),
            )
            .unwrap();
        }
        assert!(matches!(
            admit(&mut registry, 8, plan(1, 1, "https://r8.test/", 1)),
            Err(DeferredFetchError::Quota(_))
        ));
        let mut blocked = plan(3, 1, "https://report.test/", 1);
        blocked.quota = 0;
        assert!(matches!(
            admit(&mut registry, 9, blocked),
            Err(DeferredFetchError::Quota(_))
        ));
    }
    #[test]
    fn cancelled_and_activated_requests_release_quota_without_replay_io() {
        let mut registry = Registry::default();
        let first = plan(1, 1, "https://report.test/", ORIGIN_BYTES);
        let request_hash = fingerprint(&first.request).unwrap();
        let creator = first.creator.clone();
        let first_state = admit(&mut registry, 1, first).unwrap();
        let first_token = DeferredFetchToken(vec![1; 32]);
        registry.cancel(PageId(1), &creator, &first_token).unwrap();
        assert_eq!(first_state.load(Ordering::Acquire), 2);
        assert!(registry.entries[&first_token.0].pending.is_none());
        assert!(
            registry
                .replay(PageId(1), &creator, &first_token, &request_hash)
                .unwrap()
        );
        let second = admit(
            &mut registry,
            2,
            plan(1, 1, "https://report.test/", ORIGIN_BYTES),
        )
        .unwrap();
        second.store(1, Ordering::Release);
        registry
            .cancel(PageId(1), &creator, &DeferredFetchToken(vec![2; 32]))
            .unwrap();
        assert_eq!(second.load(Ordering::Acquire), 1);
        admit(
            &mut registry,
            3,
            plan(1, 1, "https://report.test/", ORIGIN_BYTES),
        )
        .unwrap();
    }
    #[test]
    fn cancellation_before_lost_prepare_response_is_durable_and_page_bound() {
        let mut registry = Registry::default();
        let token = DeferredFetchToken(vec![6; 32]);
        let plan = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&plan.request).unwrap();
        registry.cancel(PageId(1), &plan.creator, &token).unwrap();
        registry.cancel(PageId(1), &plan.creator, &token).unwrap();
        assert!(registry.cancel(PageId(2), &plan.creator, &token).is_err());
        assert!(
            registry
                .replay(PageId(1), &plan.creator, &token, &hash)
                .unwrap()
        );
        assert!(
            registry
                .replay(PageId(2), &plan.creator, &token, &hash)
                .is_err()
        );
        registry
            .insert(token.clone(), plan, hash, Arc::new(AtomicU8::new(0)), None)
            .unwrap();
        assert!(!registry.entries.contains_key(&token.0));
    }
    #[test]
    fn capability_replay_rejects_changed_request_creator_and_page() {
        let mut registry = Registry::default();
        let original = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&original.request).unwrap();
        admit(&mut registry, 1, original.clone()).unwrap();
        let token = DeferredFetchToken(vec![1; 32]);
        assert!(
            registry
                .replay(PageId(1), &original.creator, &token, &hash)
                .unwrap()
        );
        let mut changed = original.request.clone();
        changed.body.push(0);
        assert!(
            registry
                .replay(
                    PageId(1),
                    &original.creator,
                    &token,
                    &fingerprint(&changed).unwrap()
                )
                .is_err()
        );
        assert!(
            registry
                .replay(PageId(2), &original.creator, &token, &hash)
                .is_err()
        );
        assert!(
            registry
                .replay(
                    PageId(1),
                    &plan(1, 2, "https://report.test/", 1).creator,
                    &token,
                    &hash
                )
                .is_err()
        );
        assert!(
            registry
                .replay(
                    PageId(1),
                    &original.creator,
                    &DeferredFetchToken(vec![1; 31]),
                    &hash
                )
                .is_err()
        );
    }
    fn sequenced(value: u64) -> DeferredFetchToken {
        let mut bytes = vec![9; 32];
        bytes[..8].copy_from_slice(&value.to_be_bytes());
        DeferredFetchToken(bytes)
    }
    #[test]
    fn terminal_ack_history_is_bounded_under_same_document_churn_and_old_replays_expire() {
        let mut registry = Registry::default();
        let original = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&original.request).unwrap();
        for index in 1..=8192 {
            let token = sequenced(index);
            registry
                .insert(
                    token.clone(),
                    original.clone(),
                    hash,
                    Arc::new(AtomicU8::new(0)),
                    None,
                )
                .unwrap();
            registry
                .cancel(PageId(1), &original.creator, &token)
                .unwrap();
            registry.acknowledge(PageId(1), &token).unwrap();
        }
        assert!(registry.entries.len() <= ACK_HISTORY);
        assert_eq!(registry.watermarks.len(), 1);
        assert_eq!(registry.watermarks[&owner(1, 1)], 8192);
        assert!(
            matches!(registry.replay(PageId(1),&original.creator,&sequenced(1),&hash),Err(DeferredFetchError::Type(message))if message.contains("expired"))
        );
        assert!(
            registry
                .cancel(PageId(1), &original.creator, &sequenced(1))
                .is_err()
        );
        assert!(
            registry
                .replay(PageId(1), &original.creator, &sequenced(8192), &hash)
                .unwrap()
        );
        registry
            .insert(
                sequenced(8193),
                original,
                hash,
                Arc::new(AtomicU8::new(0)),
                None,
            )
            .unwrap();
    }
    #[test]
    fn unobserved_activation_survives_other_document_ack_history_churn() {
        let mut registry = Registry::default();
        let first = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&first.request).unwrap();
        let token = sequenced(1);
        let activation = Arc::new(AtomicU8::new(3));
        registry
            .insert(token.clone(), first.clone(), hash, activation.clone(), None)
            .unwrap();
        activation.store(1, Ordering::Release);
        registry.submitted(&token.0);
        let other = plan(2, 1, "https://report.test/", 1);
        for index in 1..=8192 {
            let mut next = sequenced(index);
            next.0[31] = 10;
            registry
                .insert(
                    next.clone(),
                    other.clone(),
                    hash,
                    Arc::new(AtomicU8::new(3)),
                    None,
                )
                .unwrap();
            assert_eq!(
                registry.cancel(PageId(2), &other.creator, &next).unwrap(),
                DeferredFetchState::Canceled
            );
            registry.acknowledge(PageId(2), &next).unwrap();
        }
        assert_eq!(
            registry.state(PageId(1), &token).unwrap(),
            DeferredFetchState::Activated
        );
        assert!(!registry.entries[&token.0].acknowledged);
        assert!(registry.entries.len() <= ACK_HISTORY + 1);
        registry.acknowledge(PageId(1), &token).unwrap();
        assert!(registry.entries[&token.0].acknowledged);
    }
    #[test]
    fn unacknowledged_outcomes_are_bounded_and_ack_frees_admission() {
        let mut registry = Registry::default();
        let first = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&first.request).unwrap();
        for index in 1..=MAX_CAPABILITIES as u64 {
            let token = sequenced(index);
            registry
                .insert(
                    token.clone(),
                    first.clone(),
                    hash,
                    Arc::new(AtomicU8::new(3)),
                    None,
                )
                .unwrap();
            registry.cancel(PageId(1), &first.creator, &token).unwrap();
        }
        assert!(matches!(
            registry.insert(
                sequenced(MAX_CAPABILITIES as u64 + 1),
                first.clone(),
                hash,
                Arc::new(AtomicU8::new(3)),
                None
            ),
            Err(DeferredFetchError::Quota(_))
        ));
        registry.acknowledge(PageId(1), &sequenced(1)).unwrap();
        registry
            .insert(
                sequenced(MAX_CAPABILITIES as u64 + 1),
                first,
                hash,
                Arc::new(AtomicU8::new(3)),
                None,
            )
            .unwrap();
    }
    #[test]
    fn unknown_cancel_churn_is_bounded_and_retired_issuer_watermark_is_reclaimed() {
        let mut registry = Registry::default();
        let original = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&original.request).unwrap();
        for index in 1..=8192 {
            registry
                .cancel(PageId(1), &original.creator, &sequenced(index))
                .unwrap();
        }
        assert!(registry.cancelled.len() <= ACK_HISTORY);
        assert!(
            registry
                .replay(PageId(1), &original.creator, &sequenced(1), &hash)
                .is_err()
        );
        assert!(
            registry
                .replay(PageId(1), &original.creator, &sequenced(8192), &hash)
                .unwrap()
        );
        registry.retire_issuer(&owner(1, 1));
        assert!(registry.watermarks.is_empty());
        assert!(registry.cancelled.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn foreign_creator_cannot_advance_live_issuer_sequence_with_unknown_cancel() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(id, "https://source.test/".into(), &[])
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
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let foreign = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 99,
            stream_epoch: 0,
        };
        assert!(
            context
                .cancel_deferred_fetch(id, &foreign, &sequenced(u64::MAX))
                .is_err()
        );
        assert!(
            context
                .inner
                .deferred_fetches
                .lock()
                .unwrap()
                .watermarks
                .is_empty()
        );
        context
            .cancel_deferred_fetch(id, &creator, &sequenced(1))
            .unwrap();
        assert_eq!(
            context.inner.deferred_fetches.lock().unwrap().watermarks[&owner(id.0, 1)],
            1
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .inner
                .deferred_fetches
                .lock()
                .unwrap()
                .watermarks
                .is_empty()
        );
        assert!(
            context
                .cancel_deferred_fetch(id, &creator, &sequenced(2))
                .is_err()
        );
    }
    #[test]
    fn shutdown_discards_pending_and_revokes_admission() {
        let mut registry = Registry::default();
        let queued = plan(1, 1, "https://report.test/", 1);
        let hash = fingerprint(&queued.request).unwrap();
        let state = admit(&mut registry, 1, queued.clone()).unwrap();
        registry.close();
        assert_eq!(state.load(Ordering::Acquire), 2);
        assert!(
            registry
                .entries
                .values()
                .all(|entry| entry.pending.is_none())
        );
        assert!(
            registry
                .replay(
                    PageId(1),
                    &queued.creator,
                    &DeferredFetchToken(vec![1; 32]),
                    &hash
                )
                .is_err()
        );
    }
    #[test]
    fn retained_plan_owns_csp_and_referrer_after_caller_mutation() {
        let mut registry = Registry::default();
        let mut original = plan(1, 1, "https://report.test/", 1);
        original.policy = crate::csp::CspPolicySet::from_headers(
            original.source.as_str(),
            &[(
                "content-security-policy".into(),
                "connect-src 'none'".into(),
            )],
        );
        admit(&mut registry, 1, original.clone()).unwrap();
        original.request.initiator_url = Some("https://successor.test/".into());
        original.policy = crate::csp::CspPolicySet::from_headers("https://successor.test/", &[]);
        let retained = registry.entries[&vec![1; 32]].pending.as_ref().unwrap();
        assert_eq!(retained.request.initiator_url.as_deref(), Some(""));
        assert!(
            !retained
                .policy
                .authorize_redirect(
                    &crate::csp::CspRequest {
                        destination: crate::csp::CspDestination::Connect,
                        url: "https://report.test/".into(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: false,
                        initiated_by_trusted_script: false
                    },
                    false
                )
                .allowed
        );
    }
}
