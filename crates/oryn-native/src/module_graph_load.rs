//! Persistent native module discovery. Poll never waits for a network response.
use super::script_admission::SharedScriptAdmission;
use super::*;
use crate::script_load_failure::ScriptLoadFailure;
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
include!("module_graph_backend.rs");

fn infrastructure(error: impl std::fmt::Display) -> HostError {
    HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(error.to_string()))
}
#[cfg(all(test, unix))]
use super::script_admission::publish_script_admission;

type Completion = (
    String,
    crate::ModuleType,
    Result<(String, String, String, bool, String), HostError>,
    Option<ScriptSourceReservation>,
);
type FetchKey = (
    RealmId,
    u64,
    u64,
    String,
    crate::ModuleType,
    Option<String>,
    bool,
);
pub(crate) type SharedFetchRegistry = Rc<RefCell<BTreeMap<FetchKey, std::rc::Weak<SharedFetch>>>>;
pub(crate) struct SharedFetch {
    request: Option<Fetch>,
    render_status: crate::render_request_status::Status,
    _lease: SharedScriptAdmission,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: RefCell<mpsc::Receiver<Completion>>,
    completion: RefCell<Option<Completion>>,
    disconnected: std::cell::Cell<bool>,
}
impl Drop for SharedFetch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl SharedFetch {
    fn fail_start(
        &self,
        requested: String,
        kind: crate::ModuleType,
        error: impl std::fmt::Display,
    ) {
        *self.completion.borrow_mut() = Some((
            requested,
            kind,
            Err(infrastructure(format!(
                "module fetch worker could not start: {error}"
            ))),
            None,
        ));
    }
    fn poll(&self) -> bool {
        if self.completion.borrow().is_some() || self.disconnected.get() {
            return true;
        }
        match self.receiver.borrow().try_recv() {
            Ok(completion) => {
                *self.completion.borrow_mut() = Some(completion);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.disconnected.set(true);
                true
            }
        }
    }
    fn is_runnable(&self) -> bool {
        // Polling the channel also detects a producer that terminated without
        // publishing a response. An atomic ready counter cannot detect that.
        self.poll()
    }
    #[cfg(test)]
    fn wait(&self) -> Result<(), HostError> {
        if self.poll() {
            return if self.disconnected.get() {
                Err(infrastructure("module scheduler disconnected"))
            } else {
                Ok(())
            };
        }
        let completion = self
            .receiver
            .borrow()
            .recv()
            .map_err(|_| infrastructure("module scheduler disconnected"))?;
        *self.completion.borrow_mut() = Some(completion);
        Ok(())
    }
}
#[derive(Clone)]
struct Fetch {
    render_blocking: bool,
    parser_inserted: bool,
    url: String,
    kind: crate::ModuleType,
    parent: String,
    include: bool,
    policy: String,
    integrity: Option<String>,
    nonce: Option<String>,
}
struct FetchPermit(Arc<AtomicUsize>);
impl FetchPermit {
    fn acquire(slots: &Arc<AtomicUsize>) -> Option<Self> {
        slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < 8).then_some(used + 1)
            })
            .ok()?;
        Some(Self(slots.clone()))
    }
}
impl Drop for FetchPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) struct ModuleGraphLoad {
    retry: Option<std::time::Instant>,
    retry_deadline: std::time::Instant,
    pending: VecDeque<(
        String,
        oryn_js::script_source::ScriptSource,
        ScriptSourceReservation,
    )>,
    budget: Arc<AtomicUsize>,
    slots: Arc<AtomicUsize>,
    fetches: VecDeque<Fetch>,
    visited: BTreeSet<(String, crate::ModuleType)>,
    dependencies: usize,
    active: BTreeMap<(String, crate::ModuleType), Rc<SharedFetch>>,
    registry: SharedFetchRegistry,
    identity: Option<(RealmId, u64, u64)>,
    stable_document_identity: bool,
    fetch_epoch: Option<u64>,
    source_only: bool,
    request_bytes: Vec<ScriptSourceReservation>,
    owned_backend: Option<ModuleGraphBackend>,
    cache_revisions: Option<(Rc<std::cell::Cell<u64>>, Rc<std::cell::Cell<u64>>)>,
    seen_cache_revisions: (u64, u64),
    received: VecDeque<Completion>,
    bytes: usize,
    root_parse_key: Option<(String, [u8; 32])>,
    parse_errors: BTreeMap<(String, crate::ModuleType), u64>,
    parse_edges: BTreeMap<(String, crate::ModuleType), Vec<(String, crate::ModuleType)>>,
    fetch_failed: bool,
    cache_root: bool,
    pub(crate) error_token: Option<u64>,
    error_leases: Vec<crate::host::ModuleGraphErrorLease>,
}
impl ModuleGraphLoad {
    pub(crate) fn new<'text>(
        name: &str,
        source: impl Into<oryn_js::script_source::ScriptSourceRef<'text>>,
    ) -> Result<Self, HostError> {
        let source = source.into();
        Self::with_budget(name, source, Arc::new(AtomicUsize::new(0)))
    }
    pub(crate) fn with_budget<'text>(
        name: &str,
        source: impl Into<oryn_js::script_source::ScriptSourceRef<'text>>,
        budget: Arc<AtomicUsize>,
    ) -> Result<Self, HostError> {
        let source = source.into();
        Self::with_limits(name, source, budget, Arc::new(AtomicUsize::new(0)))
    }
    pub(crate) fn with_limits<'text>(
        name: &str,
        source: impl Into<oryn_js::script_source::ScriptSourceRef<'text>>,
        budget: Arc<AtomicUsize>,
        slots: Arc<AtomicUsize>,
    ) -> Result<Self, HostError> {
        let source = source.into();
        Self::with_registry(name, source, budget, slots, Default::default())
    }
    pub(crate) fn with_registry<'text>(
        name: &str,
        source: impl Into<oryn_js::script_source::ScriptSourceRef<'text>>,
        budget: Arc<AtomicUsize>,
        slots: Arc<AtomicUsize>,
        registry: SharedFetchRegistry,
    ) -> Result<Self, HostError> {
        let source = source.into();
        Self::with_root_request(name, source, Some(name), budget, slots, registry)
    }
    pub(crate) fn with_root_request<'text>(
        name: &str,
        source: impl Into<oryn_js::script_source::ScriptSourceRef<'text>>,
        requested: Option<&str>,
        budget: Arc<AtomicUsize>,
        slots: Arc<AtomicUsize>,
        registry: SharedFetchRegistry,
    ) -> Result<Self, HostError> {
        let source = source.into();
        let reservation = ScriptSourceReservation::reserve(
            &budget,
            name.len().saturating_add(source.raw().len()),
        )
        .ok_or_else(|| Self::limit("queued module graph source bytes"))?;
        if source.raw().len() > 32 * 1024 * 1024 {
            return Err(Self::limit("module graph source bytes"));
        }
        Ok(Self {
            retry: None,
            retry_deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
            pending: VecDeque::from([(name.into(), source.owned(), reservation)]),
            budget,
            slots,
            fetches: VecDeque::new(),
            visited: requested
                .into_iter()
                .map(|url| (url.to_owned(), crate::ModuleType::JavaScript))
                .collect(),
            dependencies: 0,
            active: BTreeMap::new(),
            registry,
            identity: None,
            stable_document_identity: false,
            fetch_epoch: None,
            source_only: false,
            request_bytes: Vec::new(),
            owned_backend: None,
            cache_revisions: None,
            seen_cache_revisions: (0, 0),
            received: VecDeque::new(),
            bytes: source.raw().len(),
            root_parse_key: Some(RawV8Host::module_graph_key(name, source)),
            parse_errors: BTreeMap::new(),
            parse_edges: BTreeMap::new(),
            fetch_failed: false,
            cache_root: requested.is_some(),
            error_token: None,
            error_leases: Vec::new(),
        })
    }
    pub(crate) fn external_root(
        requested: &str,
        parent: &str,
        kind: crate::ModuleType,
        include: bool,
        policy: &str,
        integrity: Option<String>,
        budget: Arc<AtomicUsize>,
        slots: Arc<AtomicUsize>,
        registry: SharedFetchRegistry,
    ) -> Result<Self, HostError> {
        let mut graph = Self::with_root_request(requested, "", None, budget, slots, registry)?;
        graph.pending.clear();
        graph.stable_document_identity = true;
        graph.root_parse_key = None;
        graph.cache_root = true;
        graph.visited.insert((requested.to_owned(), kind));
        graph.queue_fetch(Fetch {
            render_blocking: false,
            parser_inserted: false,
            url: requested.into(),
            kind,
            parent: parent.into(),
            include,
            policy: policy.into(),
            integrity,
            nonce: None,
        })?;
        Ok(graph)
    }
    pub(crate) fn external_source(
        requested: &str,
        parent: &str,
        include: bool,
        policy: &str,
        integrity: Option<String>,
        nonce: Option<String>,
        budget: Arc<AtomicUsize>,
        slots: Arc<AtomicUsize>,
        registry: SharedFetchRegistry,
    ) -> Result<Self, HostError> {
        let mut graph = Self::external_root(
            requested,
            parent,
            crate::ModuleType::JavaScript,
            include,
            policy,
            integrity,
            budget,
            slots,
            registry,
        )?;
        graph.source_only = true;
        graph.stable_document_identity = false;
        if let Some(nonce) = &nonce {
            let reservation = ScriptSourceReservation::reserve(&graph.budget, nonce.len())
                .ok_or_else(|| Self::limit("module request nonce bytes"))?;
            graph.request_bytes.push(reservation);
        }
        graph.fetches.front_mut().expect("external root").nonce = nonce;
        Ok(graph)
    }

    pub(crate) fn with_render_blocking_root(mut self, value: bool) -> Self {
        if let Some(fetch) = self.fetches.front_mut() {
            fetch.render_blocking = value;
        }
        self
    }
    pub(crate) fn with_root_request_metadata(
        mut self,
        nonce: Option<String>,
        parser_inserted: bool,
    ) -> Result<Self, HostError> {
        if let Some(nonce) = &nonce {
            let reservation = ScriptSourceReservation::reserve(&self.budget, nonce.len())
                .ok_or_else(|| Self::limit("module request nonce bytes"))?;
            self.request_bytes.push(reservation);
        }
        if let Some(fetch) = self.fetches.front_mut() {
            fetch.nonce = nonce;
            fetch.parser_inserted = parser_inserted;
        }
        Ok(self)
    }
    pub(crate) fn parser_inserted_root(mut self) -> Self {
        if let Some(fetch) = self.fetches.front_mut() {
            fetch.parser_inserted = true;
        }
        self
    }

    pub(crate) fn defer_retry(&mut self, host: &mut RawV8Host) -> bool {
        let now = std::time::Instant::now();
        if now >= self.retry_deadline {
            return false;
        }
        let retry = (now + std::time::Duration::from_millis(10)).min(self.retry_deadline);
        self.retry = Some(retry);
        host.schedule_module_graph_retry(retry);
        true
    }
    pub(crate) fn is_runnable(&self) -> bool {
        if let Some(retry) = self.retry {
            return std::time::Instant::now() >= retry;
        }
        self.active.values().any(|fetch| fetch.is_runnable())
            || !self.pending.is_empty()
            || !self.received.is_empty()
            || (!self.fetches.is_empty()
                && (self.slots.load(Ordering::Acquire) < 8
                    || self
                        .cache_revisions
                        .as_ref()
                        .is_some_and(|(javascript, typed)| {
                            (javascript.get(), typed.get()) != self.seen_cache_revisions
                        })
                    || self.identity.is_some_and(|(realm, generation, epoch)| {
                        let epoch = self.fetch_epoch.unwrap_or(epoch);
                        let registry = self.registry.borrow();
                        self.fetches.iter().any(|fetch| {
                            registry
                                .get(&(
                                    realm,
                                    generation,
                                    epoch,
                                    fetch.url.clone(),
                                    fetch.kind,
                                    self.owned_backend
                                        .as_ref()
                                        .and_then(ModuleGraphBackend::script_policy),
                                    fetch.parser_inserted,
                                ))
                                .is_some_and(|fetch| fetch.strong_count() != 0)
                        })
                    })))
    }
    fn queue_fetch(&mut self, fetch: Fetch) -> Result<(), HostError> {
        let bytes = fetch
            .url
            .len()
            .saturating_mul(3)
            .saturating_add(fetch.parent.len())
            .saturating_add(fetch.policy.len())
            .saturating_add(fetch.integrity.as_ref().map_or(0, String::len))
            .saturating_add(fetch.nonce.as_ref().map_or(0, String::len));
        let reservation = ScriptSourceReservation::reserve(&self.budget, bytes)
            .ok_or_else(|| Self::limit("module request metadata bytes"))?;
        self.request_bytes.push(reservation);
        self.fetches.push_back(fetch);
        Ok(())
    }
    fn limit(resource: &str) -> HostError {
        HostError::LimitExceeded {
            resource: resource.into(),
        }
    }
    fn account_source_bytes(&mut self, bytes: usize) -> Result<(), HostError> {
        let next = self
            .bytes
            .checked_add(bytes)
            .filter(|size| *size <= 32 * 1024 * 1024)
            .ok_or_else(|| Self::limit("module graph source bytes"))?;
        self.bytes = next;
        Ok(())
    }
    fn retain_source(
        &mut self,
        url: String,
        source: String,
        reservation: Option<ScriptSourceReservation>,
    ) -> Result<(), HostError> {
        let next = self
            .bytes
            .checked_add(source.len())
            .filter(|size| *size <= 32 * 1024 * 1024)
            .ok_or_else(|| Self::limit("module graph source bytes"))?;
        let reservation = match reservation {
            Some(reservation) => reservation,
            None => ScriptSourceReservation::reserve(
                &self.budget,
                url.len().saturating_add(source.len()),
            )
            .ok_or_else(|| Self::limit("queued module graph source bytes"))?,
        };
        self.bytes = next;
        self.pending.push_back((url, source.into(), reservation));
        Ok(())
    }
    pub(crate) fn retain_document_authority(&mut self) {
        self.stable_document_identity = true;
    }
    pub(crate) fn set_backend(&mut self, backend: ModuleGraphBackend) {
        self.owned_backend = Some(backend);
    }
    pub(crate) fn capture_backend(&mut self, host: &mut RawV8Host) -> Result<(), HostError> {
        self.owned_backend = Some(host.module_fetch_backend()?);
        Ok(())
    }
    pub(crate) fn poll_native(
        &mut self,
        host: &mut RawV8Host,
        resources: &mut BTreeMap<String, String>,
    ) -> Result<bool, HostError> {
        if self.owned_backend.is_none() {
            self.capture_backend(host)?;
        }
        let backend = self
            .owned_backend
            .as_ref()
            .expect("captured module backend")
            .clone();
        self.poll_with_backend(host, &backend, resources)
    }
    #[cfg(all(unix, test))]
    pub(crate) fn poll(
        &mut self,
        host: &mut RawV8Host,
        client: &crate::context_service::SharedContextClient,
        page_id: oryn_common::v3::PageId,
        resources: &mut BTreeMap<String, String>,
    ) -> Result<bool, HostError> {
        if self.owned_backend.is_none() {
            self.owned_backend = Some(ModuleGraphBackend::Window(
                client.clone(),
                page_id,
                None,
                host.capture_script_policy()?,
            ));
        }
        let backend = self.owned_backend.clone().expect("captured policy backend");
        self.poll_with_backend(host, &backend, resources)
    }
    pub(crate) fn poll_with_backend(
        &mut self,
        host: &mut RawV8Host,
        backend: &ModuleGraphBackend,
        resources: &mut BTreeMap<String, String>,
    ) -> Result<bool, HostError> {
        if let Some(retry) = self.retry
            && std::time::Instant::now() < retry
        {
            host.schedule_module_graph_retry(retry);
            return Ok(false);
        }
        self.retry = None;
        if self.owned_backend.is_none() {
            self.owned_backend = Some(backend.clone());
        }
        let realm = host.active_realm();
        if !host.realm_execution_live(realm)
            && !(self.stable_document_identity && host.import_realm_live(realm))
        {
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled));
        }
        let generation = host
            .realm_descriptor(realm)
            .ok_or_else(|| HostError::ScriptLoad(ScriptLoadFailure::Cancelled))?
            .document_generation;
        let epoch = if self.stable_document_identity {
            Some(host.document_authority_epoch(realm).unwrap_or(0))
        } else {
            host.document_stream_epoch(realm)
        }
        .ok_or_else(|| HostError::ScriptLoad(ScriptLoadFailure::Cancelled))?;
        let identity = (realm, generation, epoch);
        if self.identity.is_some_and(|previous| previous != identity) {
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled));
        }
        self.identity = Some(identity);
        self.fetch_epoch = Some(host.document_authority_epoch(realm).unwrap_or(epoch));
        if self.cache_revisions.is_none() {
            self.cache_revisions = Some(host.module_response_revisions()?);
        }
        for ((url, kind), fetch) in &self.active {
            if self
                .received
                .iter()
                .any(|(requested, received_kind, _, _)| requested == url && received_kind == kind)
            {
                continue;
            }
            if !fetch.poll() {
                continue;
            }
            if fetch.disconnected.get() {
                self.received.push_back((
                    url.clone(),
                    *kind,
                    Err(infrastructure("module scheduler disconnected")),
                    None,
                ));
                continue;
            }
            let cached = if *kind == crate::ModuleType::JavaScript {
                host.retained_module_fetch(url).is_some()
            } else {
                host.retained_typed_module_fetch(url, *kind).is_some()
            };
            if cached {
                // The normal completion path adopts the authoritative cache;
                // do not copy an already published shared response body.
                self.received.push_back((
                    url.clone(),
                    *kind,
                    Err(HostError::Execution("retained module result".into())),
                    None,
                ));
                continue;
            }
            let completion = fetch.completion.borrow();
            let (_, _, result, _) = completion.as_ref().expect("completed shared fetch");
            let (result, reservation) = match result {
                Ok(value) => {
                    let bytes = value
                        .0
                        .len()
                        .saturating_add(value.1.len())
                        .saturating_add(value.2.len())
                        .saturating_add(value.4.len());
                    match ScriptSourceReservation::reserve(&self.budget, bytes) {
                        Some(reservation) => (Ok(value.clone()), Some(reservation)),
                        None => (Err(Self::limit("queued module graph source bytes")), None),
                    }
                }
                Err(HostError::LimitExceeded { resource }) => (Err(Self::limit(resource)), None),
                Err(HostError::ScriptLoad(failure)) => {
                    (Err(HostError::ScriptLoad(failure.clone())), None)
                }
                Err(error) => (Err(HostError::Execution(error.to_string())), None),
            };
            self.received
                .push_back((url.clone(), *kind, result, reservation));
        }
        while let Some(completion) = self.received.pop_front() {
            let (requested, kind, result, _reservation) = &completion;
            let applied = (|| -> Result<(), HostError> {
                let retained = if *kind == crate::ModuleType::JavaScript {
                    host.retained_module_fetch(requested)
                } else {
                    host.retained_typed_module_fetch(requested, *kind)
                };
                if let Some(retained) = retained {
                    if let Some((url, source)) = retained {
                        if *kind == crate::ModuleType::JavaScript {
                            if !self.source_only {
                                self.retain_source(url, source, None)?;
                            }
                        } else {
                            self.account_source_bytes(source.len())?;
                        }
                    } else {
                        self.fetch_failed = true;
                    }
                    return Ok(());
                }
                match result {
                    Err(HostError::ScriptLoad(ScriptLoadFailure::Integrity(_))) => {
                        self.fetch_failed = true;
                    }
                    Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_))) => {
                        if *kind == crate::ModuleType::JavaScript {
                            host.retain_failed_module_fetch(requested)?;
                        } else {
                            host.retain_typed_module_fetch(requested, *kind, None)?;
                        }
                        self.fetch_failed = true;
                    }
                    Err(HostError::ScriptLoad(failure)) => {
                        return Err(HostError::ScriptLoad(failure.clone()));
                    }
                    Err(HostError::LimitExceeded { resource }) => {
                        return Err(Self::limit(resource));
                    }
                    Err(error) => return Err(infrastructure(error)),
                    Ok((requested, url, source, include, policy)) => {
                        #[cfg(test)]
                        graph_publication_failure(1)?;
                        if *kind != crate::ModuleType::JavaScript {
                            // Commit accounting only after fallible cache publication.
                            let next = self
                                .bytes
                                .checked_add(source.len())
                                .filter(|size| *size <= 32 * 1024 * 1024)
                                .ok_or_else(|| Self::limit("module graph source bytes"))?;
                            host.retain_typed_module_fetch(
                                requested,
                                *kind,
                                Some((url.clone(), source.clone())),
                            )?;
                            self.bytes = next;
                        } else {
                            // Reserve graph custody before publishing into the
                            // realm cache. A budget rejection must not leave a
                            // fetched-module entry that bypasses the rejected load.
                            let graph_source = if self.source_only {
                                None
                            } else {
                                let next = self
                                    .bytes
                                    .checked_add(source.len())
                                    .filter(|size| *size <= 32 * 1024 * 1024)
                                    .ok_or_else(|| Self::limit("module graph source bytes"))?;
                                let reservation = ScriptSourceReservation::reserve(
                                    &self.budget,
                                    url.len().saturating_add(source.len()),
                                )
                                .ok_or_else(|| Self::limit("queued module graph source bytes"))?;
                                Some((next, reservation))
                            };
                            host.retain_verified_module_fetch(requested, url, source)?;
                            host.set_module_credentials(requested, *include);
                            host.set_module_referrer_policy(requested, policy, false);
                            host.set_module_credentials(url, *include);
                            host.set_module_referrer_policy(url, policy, false);
                            #[cfg(test)]
                            graph_publication_failure(2)?;
                            if let Some((next, reservation)) = graph_source {
                                self.bytes = next;
                                self.pending.push_back((
                                    url.clone(),
                                    source.clone().into(),
                                    reservation,
                                ));
                            }
                            resources.insert(url.clone(), source.clone());
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = applied {
                // A failed native fetch can be admitted again; a successful body
                // remains owned here until cache/discovery publication commits.
                let retained_body = matches!(result, Err(HostError::LimitExceeded { .. }))
                    && self
                        .active
                        .get(&(requested.clone(), *kind))
                        .is_some_and(|fetch| {
                            fetch
                                .completion
                                .borrow()
                                .as_ref()
                                .is_some_and(|(_, _, result, _)| result.is_ok())
                        });
                if retained_body {
                    return Err(error);
                }
                let retry_fetch = matches!(
                    result,
                    Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(_)))
                )
                .then(|| {
                    self.active
                        .get(&(requested.clone(), *kind))
                        .and_then(|fetch| fetch.request.clone())
                })
                .flatten();
                if let Some(fetch) = retry_fetch {
                    if let Some(failed) = self.active.remove(&(requested.clone(), *kind)) {
                        self.registry.borrow_mut().retain(|_, candidate| {
                            candidate
                                .upgrade()
                                .is_some_and(|candidate| !Rc::ptr_eq(&candidate, &failed))
                        });
                    }
                    self.fetches.push_front(fetch);
                } else if matches!(
                    result,
                    Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(_)))
                ) {
                    // A response-only producer has no request to retry. Retire
                    // its disconnected receipt without poisoning the module map
                    // or keeping the graph permanently runnable.
                    if let Some(failed) = self.active.remove(&(requested.clone(), *kind)) {
                        self.registry.borrow_mut().retain(|_, candidate| {
                            candidate
                                .upgrade()
                                .is_some_and(|candidate| !Rc::ptr_eq(&candidate, &failed))
                        });
                    }
                } else {
                    self.received.push_front(completion);
                }
                return Err(error);
            }
            self.active.remove(&(requested.clone(), *kind));
        }
        // Bound synchronous V8 compilation work in one actor turn. Retained
        // sources keep the graph runnable for the next fair task slice.
        for _ in 0..8 {
            let Some(input) = self.pending.pop_front() else {
                break;
            };
            let (parent, source, _reservation) = &input;
            let processed = (|| -> Result<(), HostError> {
                let key = RawV8Host::module_graph_key(parent, source);
                if self.root_parse_key.is_none() {
                    self.root_parse_key = Some(key.clone());
                }
                let cache = self.cache_root || self.root_parse_key.as_ref() != Some(&key);
                let dependencies =
                    match host.module_dependency_requests_for_graph(parent, source, cache) {
                        Ok(dependencies) => dependencies,
                        Err(HostError::ModuleSyntax(_)) => {
                            // A parse error is a fetched module script with an error
                            // to rethrow, not a failed response. Finish discovering
                            // sibling dependencies before reporting it at execution.
                            let lease = host.retain_module_graph_error(&key, cache)?;
                            self.parse_errors
                                .entry((parent.clone(), crate::ModuleType::JavaScript))
                                .or_insert(lease.token);
                            self.error_leases.push(lease);
                            return Ok(());
                        }
                        Err(error) => return Err(error),
                    };
                for (dependency, kind) in dependencies {
                    let url = match host.resolve_module_specifier(&parent, &dependency) {
                        Ok(Some(url)) => url,
                        // Explicit source tables may supply legacy non-URL names,
                        // but cannot bypass a null or invalid import-map entry.
                        Ok(None) => match backend {
                            ModuleGraphBackend::Sources(sources, _, _) => match host
                                .resolve_explicit_module_name(&parent, &dependency, sources)
                            {
                                Some(url) => url,
                                None => continue,
                            },
                            _ => continue,
                        },
                        // Linking reports the original resolution exception. Do
                        // not fetch through a source-table fallback or misclassify
                        // a module parse/link error as an element fetch failure.
                        Err(_) => continue,
                    };
                    let edges = self
                        .parse_edges
                        .entry((parent.clone(), crate::ModuleType::JavaScript))
                        .or_default();
                    if !edges.contains(&(url.clone(), kind)) {
                        edges.push((url.clone(), kind));
                    }
                    if self.visited.contains(&(url.clone(), kind)) {
                        continue;
                    }
                    if self.dependencies >= 256 {
                        return Err(Self::limit("module dependency graph"));
                    }
                    let key = (url.clone(), kind);
                    #[cfg(test)]
                    graph_publication_failure(3)?;
                    let include = host.module_credentials(&parent);
                    host.set_module_credentials(&url, include);
                    let integrity = host.module_integrity(&url);
                    if kind != crate::ModuleType::JavaScript {
                        match host.retained_typed_module_fetch(&url, kind) {
                            Some(Some((_, source))) => self.account_source_bytes(source.len())?,
                            Some(None) => {
                                self.fetch_failed = true;
                            }
                            None => self.queue_fetch(Fetch {
                                render_blocking: false,
                                parser_inserted: false,
                                url,
                                kind,
                                parent: parent.clone(),
                                include,
                                policy: host.module_referrer_policy(&parent),
                                integrity,
                                nonce: None,
                            })?,
                        }
                        self.visited.insert(key);
                        self.dependencies += 1;
                        continue;
                    }
                    let cached = match host.retained_module_fetch(&url) {
                        Some(None) => {
                            self.fetch_failed = true;
                            self.visited.insert(key);
                            self.dependencies += 1;
                            continue;
                        }
                        Some(Some(value)) => Some(value),
                        // A decoded resource is not an admitted module response.
                        // In particular, a redirect's final URL is a distinct
                        // request identity and must pass native fetch policy.
                        None => None,
                    };
                    if let Some((url, source)) = cached {
                        self.retain_source(url, source, None)?;
                        self.visited.insert(key);
                        self.dependencies += 1;
                        continue;
                    }
                    self.queue_fetch(Fetch {
                        render_blocking: false,
                        parser_inserted: false,
                        url,
                        kind,
                        parent: parent.clone(),
                        include,
                        policy: host.module_referrer_policy(&parent),
                        integrity,
                        nonce: None,
                    })?;
                    self.visited.insert(key);
                    self.dependencies += 1;
                }
                Ok(())
            })();
            if let Err(error) = processed {
                self.pending.push_front(input);
                return Err(error);
            }
        }
        // Visit each queued request once. A request needing a new slot must not
        // hide a later request that can join an existing fetch without a slot.
        for _ in 0..self.fetches.len() {
            let fetch = self.fetches.pop_front().expect("queued module request");
            // A different graph may finish after discovery queued this URL,
            // then release its shared request before this graph obtains a slot.
            // The retained module map is still authoritative at admission.
            let retained = if fetch.kind == crate::ModuleType::JavaScript {
                host.retained_module_fetch(&fetch.url)
            } else {
                host.retained_typed_module_fetch(&fetch.url, fetch.kind)
            };
            if let Some(retained) = retained {
                if let Some((url, source)) = retained {
                    let retained = if fetch.kind == crate::ModuleType::JavaScript {
                        if self.source_only {
                            Ok(())
                        } else {
                            self.retain_source(url, source, None)
                        }
                    } else {
                        self.account_source_bytes(source.len())
                    };
                    if let Err(error) = retained {
                        self.fetches.push_front(fetch);
                        return Err(error);
                    }
                } else {
                    self.fetch_failed = true;
                }
                continue;
            }
            // Input-stream replacement cancels parser ownership, but the
            // document module map (and import() consumers) keeps its authority.
            let fetch_epoch = host.document_authority_epoch(realm).unwrap_or(epoch);
            let key = (
                realm,
                generation,
                fetch_epoch,
                fetch.url.clone(),
                fetch.kind,
                backend.script_policy(),
                fetch.parser_inserted,
            );
            let existing = self
                .registry
                .borrow()
                .get(&key)
                .and_then(std::rc::Weak::upgrade);
            if let Some(existing) = existing {
                if fetch.render_blocking {
                    existing.render_status.promote();
                }
                self.active.insert((fetch.url, fetch.kind), existing);
                continue;
            }
            let Some(permit) = FetchPermit::acquire(&self.slots) else {
                self.fetches.push_back(fetch);
                continue;
            };
            let prepared = (|| -> Result<_, HostError> {
                #[cfg(test)]
                graph_publication_failure(4)?;
                let creator = match backend {
                    #[cfg(unix)]
                    ModuleGraphBackend::Window(_, _, Some((creator, _)), _) => {
                        Some(creator.clone())
                    }
                    #[cfg(unix)]
                    ModuleGraphBackend::Window(..) => Some(host.native_script_creator()?),
                    _ => None,
                };
                let initiator = match backend {
                    #[cfg(unix)]
                    ModuleGraphBackend::Window(_, _, Some((_, initiator)), _) => initiator.clone(),
                    #[cfg(unix)]
                    ModuleGraphBackend::Window(..) => host.request_initiator_url()?,
                    ModuleGraphBackend::Broker { initiator, .. } => initiator.clone(),
                    _ => fetch.parent.clone(),
                };
                let request_id = if matches!(backend, ModuleGraphBackend::Broker { .. }) {
                    Some(host.reserve_module_request_id()?)
                } else {
                    None
                };
                let resource_job = backend.resource_job()?;
                Ok((creator, initiator, request_id, resource_job))
            })();
            let (creator, initiator, request_id, resource_job) = match prepared {
                Ok(value) => value,
                Err(error) => {
                    self.fetches.push_front(fetch);
                    return Err(error);
                }
            };
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let lease: SharedScriptAdmission = Default::default();
            let admission = Arc::downgrade(&lease);
            let (sender, receiver) = mpsc::channel();
            let render_status = crate::render_request_status::Status::new(fetch.render_blocking);
            let shared = Rc::new(SharedFetch {
                request: Some(fetch.clone()),
                render_status: render_status.clone(),
                _lease: lease,
                cancel: cancel.clone(),
                receiver: RefCell::new(receiver),
                completion: RefCell::new(None),
                disconnected: std::cell::Cell::new(false),
            });
            {
                let mut registry = self.registry.borrow_mut();
                registry.retain(|_, fetch| fetch.strong_count() != 0);
                registry.insert(key, Rc::downgrade(&shared));
            }
            self.active
                .insert((fetch.url.clone(), fetch.kind), shared.clone());
            let backend = backend.clone();
            let budget = self.budget.clone();
            let failed_request = fetch.url.clone();
            let failed_kind = fetch.kind;
            let started = super::resource_io_executor::submit(move || {
                let _resource_job = resource_job;
                // Cancellation drops the graph's broker lease, but this slot
                // remains occupied until the admitted network work terminates.
                let _permit = permit;
                let requested = fetch.url.clone();
                let kind = fetch.kind;
                let result = backend.fetch(
                    fetch,
                    render_status.clone(),
                    creator,
                    initiator,
                    request_id,
                    admission,
                    &cancel,
                );
                render_status.finish();
                // Reserve before publishing into an unbounded channel. The
                // envelope owns the reservation even after graph cancellation.
                let (result, reservation) = match result {
                    Ok(value) => {
                        let bytes = value
                            .0
                            .len()
                            .saturating_add(value.1.len())
                            .saturating_add(value.2.len())
                            .saturating_add(value.4.len());
                        match ScriptSourceReservation::reserve(&budget, bytes) {
                            Some(reservation) => (Ok(value), Some(reservation)),
                            None => (Err(Self::limit("queued module graph source bytes")), None),
                        }
                    }
                    Err(error) => (Err(error), None),
                };
                let _ = sender.send((requested, kind, result, reservation));
            });
            if let Err(error) = started {
                // Rejected admission drops the queued closure, returning its permit
                // and sender. Publish a normal failed response for all graphs
                // sharing this admission instead of panicking the page actor.
                shared.fail_start(failed_request, failed_kind, error);
            }
        }
        if let Some((javascript, typed)) = &self.cache_revisions {
            self.seen_cache_revisions = (javascript.get(), typed.get());
        }
        let complete = self.active.is_empty() && self.fetches.is_empty() && self.pending.is_empty();
        if complete && self.fetch_failed {
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(
                "module dependency fetch failed".into(),
            )));
        }
        if complete {
            for (url, kind) in &self.visited {
                if *kind == crate::ModuleType::Json
                    && !self.parse_errors.contains_key(&(url.clone(), *kind))
                {
                    if let Some(lease) = host.prepare_json_graph_error(url)? {
                        self.parse_errors.insert((url.clone(), *kind), lease.token);
                        self.error_leases.push(lease);
                    }
                }
            }
        }
        if complete && !self.parse_errors.is_empty() {
            if let Some(root) = &self.root_parse_key {
                // Choose by requested-module depth-first order, not network
                // response order. Redirect aliases keep their response module's
                // dependency list and native exception identity.
                let mut pending = vec![(root.0.clone(), crate::ModuleType::JavaScript)];
                let mut seen = BTreeSet::new();
                while let Some((name, kind)) = pending.pop() {
                    let name = if kind == crate::ModuleType::JavaScript {
                        host.retained_module_fetch(&name)
                            .flatten()
                            .map_or(name, |(url, _)| url)
                    } else {
                        name
                    };
                    let name = (name, kind);
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    if let Some(failed) = self.parse_errors.get(&name) {
                        self.error_token = Some(*failed);
                        break;
                    }
                    if let Some(children) = self.parse_edges.get(&name) {
                        pending.extend(children.iter().rev().cloned());
                    }
                }
            }
        }
        Ok(complete)
    }
    // Blocking receipt helper exists only in regression fixtures. Production
    // callers retain the graph and resume through pollable actor turns.
    #[cfg(test)]
    pub(crate) fn wait_for_completion(&mut self) -> Result<(), HostError> {
        if let Some(fetch) = self.active.values().find(|fetch| fetch.poll()) {
            return fetch.wait();
        }
        self.active
            .values()
            .next()
            .ok_or_else(|| {
                HostError::Execution("module scheduler has no active fetch to wait for".into())
            })?
            .wait()
    }
}

#[cfg(test)]
thread_local! {static GRAPH_PUBLICATION_FAILURE:std::cell::Cell<Option<u8>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
fn graph_publication_failure(stage: u8) -> Result<(), HostError> {
    if GRAPH_PUBLICATION_FAILURE.with(|failure| {
        if failure.get() == Some(stage) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        Err(infrastructure("injected graph publication failure"))
    } else {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            1,
        )
        .unwrap();
        host
    }
    fn client() -> (
        crate::context_service::SharedContextClient,
        std::os::unix::net::UnixStream,
    ) {
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        (
            crate::context_service::SharedContextClient::new(
                crate::context_service::IpcContextClient::new(worker),
            ),
            parent,
        )
    }
    fn shared_fetch() -> (Rc<SharedFetch>, mpsc::Sender<Completion>) {
        // Response-only fixtures own no native operation token.
        let (sender, receiver) = mpsc::channel();
        (
            Rc::new(SharedFetch {
                request: None,
                render_status: Default::default(),
                _lease: Default::default(),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                receiver: RefCell::new(receiver),
                completion: RefCell::new(None),
                disconnected: std::cell::Cell::new(false),
            }),
            sender,
        )
    }
    #[test]
    fn graph_retry_has_native_wakeup_and_bounded_terminal_deadline() {
        let mut host = host();
        let mut graph = ModuleGraphLoad::new("https://example.test/retry.js", "export {}").unwrap();
        assert!(graph.defer_retry(&mut host));
        assert!(!graph.is_runnable());
        assert!(host.next_timer_delay().is_some());
        graph.retry = Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
        assert!(graph.is_runnable());
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        graph.retry_deadline = std::time::Instant::now() - std::time::Duration::from_millis(1);
        assert!(!graph.defer_retry(&mut host));
    }
    #[test]
    fn successful_graph_response_survives_pre_and_post_cache_publication_failure() {
        for stage in [1, 2] {
            let mut host = host();
            let url = "https://example.test/retained.js";
            let source = "export const value=42";
            let mut graph = ModuleGraphLoad::new(url, "").unwrap();
            graph.pending.clear();
            graph.received.push_back((
                url.into(),
                crate::ModuleType::JavaScript,
                Ok((
                    url.into(),
                    url.into(),
                    source.into(),
                    false,
                    "no-referrer".into(),
                )),
                None,
            ));
            let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
            GRAPH_PUBLICATION_FAILURE.with(|failure| failure.set(Some(stage)));
            assert!(
                graph
                    .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                    .is_err()
            );
            assert_eq!(graph.received.len(), 1);
            assert_eq!(graph.bytes, 0);
            assert_eq!(host.retained_module_fetch(url).is_some(), stage == 2);
            assert!(
                graph
                    .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                    .unwrap()
            );
            assert!(graph.received.is_empty());
            assert_eq!(graph.bytes, source.len());
            assert_eq!(host.retained_module_fetch(url).unwrap().unwrap().1, source);
        }
    }
    #[test]
    fn graph_dependency_admission_retry_keeps_input_and_commits_visit_once() {
        let mut host = host();
        let root = "https://example.test/root.js";
        let child = "https://example.test/child.js";
        host.retain_verified_module_fetch(child, child, "export {}")
            .unwrap();
        let mut graph = ModuleGraphLoad::new(root, "import './child.js'").unwrap();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        GRAPH_PUBLICATION_FAILURE.with(|failure| failure.set(Some(3)));
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .is_err()
        );
        assert_eq!(graph.pending.len(), 1);
        assert!(
            !graph
                .visited
                .contains(&(child.into(), crate::ModuleType::JavaScript))
        );
        assert_eq!(graph.dependencies, 0);
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        assert_eq!(graph.dependencies, 1);
        assert_eq!(
            graph
                .parse_edges
                .get(&(root.into(), crate::ModuleType::JavaScript))
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn graph_request_preflight_failure_preserves_request_and_slot() {
        let mut host = host();
        let url = "https://example.test/request.js";
        let slots = Arc::new(AtomicUsize::new(0));
        let mut graph = ModuleGraphLoad::external_root(
            url,
            url,
            crate::ModuleType::JavaScript,
            false,
            "",
            None,
            Default::default(),
            slots.clone(),
            Default::default(),
        )
        .unwrap();
        let backend = ModuleGraphBackend::Sources(
            Arc::new(BTreeMap::from([(url.into(), "export {}".into())])),
            None,
            Default::default(),
        );
        GRAPH_PUBLICATION_FAILURE.with(|failure| failure.set(Some(4)));
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .is_err()
        );
        assert_eq!(graph.fetches.len(), 1);
        assert!(graph.active.is_empty());
        assert_eq!(slots.load(Ordering::Acquire), 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !graph
            .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
            .unwrap()
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(host.retained_module_fetch(url).unwrap().is_some());
        assert_eq!(slots.load(Ordering::Acquire), 0);
    }
    #[test]
    fn integrity_failure_does_not_install_a_failed_module_map_entry() {
        let mut host = host();
        let url = "https://example.test/integrity.js";
        let mut graph = ModuleGraphLoad::new(url, "").unwrap();
        graph.pending.clear();
        graph.received.push_back((
            url.into(),
            crate::ModuleType::JavaScript,
            Err(HostError::ScriptLoad(ScriptLoadFailure::Integrity(
                "mismatch".into(),
            ))),
            None,
        ));
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .is_err()
        );
        assert!(host.retained_module_fetch(url).is_none());
        host.retain_verified_module_fetch(url, url, "export {};")
            .unwrap();
        assert!(host.retained_module_fetch(url).flatten().is_some());
    }

    #[test]
    fn external_source_request_metadata_is_charged_until_owner_drops() {
        let budget = Arc::new(AtomicUsize::new(0));
        let graph = ModuleGraphLoad::external_source(
            "https://example.test/root.js",
            "https://example.test/",
            false,
            "no-referrer",
            Some("sha256-example".into()),
            Some("nonce".into()),
            budget.clone(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert!(budget.load(Ordering::Acquire) > "https://example.test/root.js".len());
        drop(graph);
        assert_eq!(budget.load(Ordering::Acquire), 0);
        let occupied =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES).unwrap();
        assert!(
            ModuleGraphLoad::external_source(
                "root.js",
                "about:blank",
                false,
                "",
                None,
                None,
                budget.clone(),
                Default::default(),
                Default::default()
            )
            .is_err()
        );
        assert_eq!(
            budget.load(Ordering::Acquire),
            MAX_QUEUED_SCRIPT_SOURCE_BYTES
        );
        drop(occupied);
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }

    #[test]
    fn retired_script_admission_never_contacts_the_broker() {
        let host = host();
        let creator = host.native_script_creator().unwrap();
        let (client, parent) = client();
        let owner: SharedScriptAdmission = Default::default();
        let retired = Arc::downgrade(&owner);
        drop(owner);
        assert!(
            super::super::script_admission::reserve_script_admission(
                &client,
                oryn_common::v3::PageId(1),
                creator,
                &retired
            )
            .is_err()
        );
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn asynchronous_admission_cancels_late_tokens_and_preserves_shared_owner() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ContextTransport,
        };
        struct Service(mpsc::Sender<u64>);
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let ContextServiceRequest::CancelScriptLoad { token, .. } = request else {
                    return Err(crate::RuntimeError::ContextService(
                        "unexpected test request".into(),
                    ));
                };
                self.0.send(token).unwrap();
                Ok(ContextServiceResponse::ScriptLoadCancelled)
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let (sent, received) = mpsc::channel();
        let server = std::thread::spawn(move || {
            crate::context_service::serve_context_transport(parent, Arc::new(Service(sent)))
        });
        let client = crate::context_service::SharedContextClient::new(
            crate::context_service::IpcContextClient::new(worker),
        );
        let lease = |token| {
            crate::context_service::ContextScriptLoad::from_reserved(
                client.clone(),
                oryn_common::v3::PageId(1),
                token,
            )
        };
        let owner: SharedScriptAdmission = Default::default();
        let weak = Arc::downgrade(&owner);
        let follower = owner.clone();
        drop(owner);
        assert_eq!(publish_script_admission(&weak, lease(11)), Some(11));
        assert!(
            received.try_recv().is_err(),
            "a remaining graph owns the admitted request"
        );
        assert_eq!(publish_script_admission(&weak, lease(12)), None);
        assert_eq!(
            received
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            12
        );
        assert_eq!(follower.lock().unwrap().as_ref().unwrap().token, 11);
        drop(follower);
        assert_eq!(
            received
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            11
        );
        assert_eq!(publish_script_admission(&weak, lease(13)), None);
        assert_eq!(
            received
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            13
        );
        client
            .drain_script_cleanup(std::time::Duration::from_secs(5))
            .unwrap();
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }
    #[test]
    fn parser_adapter_reads_shared_response_and_preserves_it_for_followers() {
        let (fetch, sender) = shared_fetch();
        let follower = fetch.clone();
        let mut graph = ModuleGraphLoad::new("root", "").unwrap();
        graph
            .active
            .insert(("child".into(), crate::ModuleType::JavaScript), fetch);
        sender
            .send((
                "child".into(),
                crate::ModuleType::JavaScript,
                Ok((
                    "child".into(),
                    "child".into(),
                    "export {};".into(),
                    false,
                    String::new(),
                )),
                None,
            ))
            .unwrap();
        graph.wait_for_completion().unwrap();
        assert!(follower.poll());
        assert!(follower.completion.borrow().is_some());
        drop(graph);
        follower.wait().unwrap();
        assert_eq!(follower.completion.borrow().as_ref().unwrap().0, "child");
    }
    #[test]
    fn parser_adapter_reports_missing_or_disconnected_fetch_instead_of_waiting_forever() {
        let mut graph = ModuleGraphLoad::new("root", "").unwrap();
        assert!(graph.wait_for_completion().is_err());
        let (fetch, sender) = shared_fetch();
        graph
            .active
            .insert(("child".into(), crate::ModuleType::JavaScript), fetch);
        drop(sender);
        assert!(graph.wait_for_completion().is_err());
    }
    #[test]
    fn disconnected_shared_producer_wakes_graph_without_poisoning_module_map() {
        let mut host = host();
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
        graph.pending.clear();
        let (fetch, sender) = shared_fetch();
        let url = "https://example.test/child.js";
        graph
            .active
            .insert((url.into(), crate::ModuleType::JavaScript), fetch);
        assert!(!graph.is_runnable());
        drop(sender);
        assert!(
            graph.is_runnable(),
            "producer loss must wake an otherwise idle graph"
        );
        let (client, _parent) = client();
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut BTreeMap::new()
            ),
            Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(_)))
        ));
        assert_eq!(host.retained_module_fetch(url), None);
        assert!(graph.active.is_empty());
        assert!(!graph.is_runnable());
    }
    #[test]
    fn unstarted_fetch_returns_capacity_and_shares_its_failure_with_followers() {
        let slots = Arc::new(AtomicUsize::new(0));
        let permit = FetchPermit::acquire(&slots).unwrap();
        let (fetch, sender) = shared_fetch();
        let follower = fetch.clone();
        // Rejected executor admission drops the closure without running it.
        let unstarted = move || {
            drop(permit);
            drop(sender);
        };
        assert_eq!(slots.load(Ordering::Acquire), 1);
        drop(unstarted);
        assert_eq!(slots.load(Ordering::Acquire), 0);
        fetch.fail_start(
            "https://example.test/child.js".into(),
            crate::ModuleType::JavaScript,
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "thread capacity"),
        );
        assert!(fetch.is_runnable());
        assert!(follower.is_runnable());
        follower.wait().unwrap();
        assert!(
            !follower.disconnected.get(),
            "published startup error takes precedence over closed channel"
        );
        let completion = follower.completion.borrow();
        assert!(matches!(&completion.as_ref().unwrap().2,
            Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(message))) if message.contains("thread capacity")));
        assert!(completion.as_ref().unwrap().3.is_none());
    }
    #[test]
    fn matching_registry_request_coalesces_even_when_all_fetch_slots_are_occupied() {
        let mut host = host();
        let realm = host.active_realm();
        let generation = host.realm_descriptor(realm).unwrap().document_generation;
        let epoch = host.document_stream_epoch(realm).unwrap();
        let url = "https://example.test/child.js";
        let (fetch, _sender) = shared_fetch();
        let registry: SharedFetchRegistry = Default::default();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_registry(
            "https://example.test/root.js",
            "import './blocked.js'; import './child.js';",
            Arc::new(AtomicUsize::new(0)),
            slots.clone(),
            registry.clone(),
        )
        .unwrap();
        let (client, _parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 2);
        assert!(!graph.is_runnable());
        registry.borrow_mut().insert(
            (
                realm,
                generation,
                epoch,
                url.into(),
                crate::ModuleType::JavaScript,
                None,
                false,
            ),
            Rc::downgrade(&fetch),
        );
        assert!(
            graph.is_runnable(),
            "a newly shareable request needs no free network slot"
        );
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 1);
        assert_eq!(graph.fetches[0].url, "https://example.test/blocked.js");
        assert!(Rc::ptr_eq(
            graph
                .active
                .get(&(url.into(), crate::ModuleType::JavaScript))
                .unwrap(),
            &fetch
        ));
        assert_eq!(slots.load(Ordering::Acquire), 8);
    }
    #[test]
    fn pending_fetch_registry_separates_captured_policy_and_parser_provenance() {
        let mut host = host();
        let realm = host.active_realm();
        let generation = host.realm_descriptor(realm).unwrap().document_generation;
        let epoch = host.document_authority_epoch(realm).unwrap();
        let url = "https://example.test/child.js";
        let (fetch, _sender) = shared_fetch();
        let registry: SharedFetchRegistry = Default::default();
        registry.borrow_mut().insert(
            (
                realm,
                generation,
                epoch,
                url.into(),
                crate::ModuleType::JavaScript,
                Some("native-policy-earlier".into()),
                true,
            ),
            Rc::downgrade(&fetch),
        );
        assert!(
            registry
                .borrow()
                .get(&(
                    realm,
                    generation,
                    epoch,
                    url.into(),
                    crate::ModuleType::JavaScript,
                    Some("native-policy-later".into()),
                    true
                ))
                .is_none()
        );
        assert!(
            registry
                .borrow()
                .get(&(
                    realm,
                    generation,
                    epoch,
                    url.into(),
                    crate::ModuleType::JavaScript,
                    Some("native-policy-earlier".into()),
                    false
                ))
                .is_none()
        );
        let graph = ModuleGraphLoad::external_source(
            url,
            "https://example.test/",
            false,
            "",
            None,
            None,
            Default::default(),
            Default::default(),
            registry,
        )
        .unwrap()
        .parser_inserted_root();
        assert!(graph.fetches.front().unwrap().parser_inserted);
    }

    #[test]
    fn registry_does_not_reuse_other_document_generations_epochs_or_module_types() {
        let mut host = host();
        let realm = host.active_realm();
        let generation = host.realm_descriptor(realm).unwrap().document_generation;
        let epoch = host.document_stream_epoch(realm).unwrap();
        let url = "https://example.test/child.js";
        let (fetch, _sender) = shared_fetch();
        let registry: SharedFetchRegistry = Default::default();
        for key in [
            (
                realm,
                generation + 1,
                epoch,
                url.into(),
                crate::ModuleType::JavaScript,
                None,
                false,
            ),
            (
                realm,
                generation,
                epoch + 1,
                url.into(),
                crate::ModuleType::JavaScript,
                None,
                false,
            ),
            (
                realm,
                generation,
                epoch,
                url.into(),
                crate::ModuleType::Json,
                None,
                false,
            ),
        ] {
            registry.borrow_mut().insert(key, Rc::downgrade(&fetch));
        }
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_registry(
            "https://example.test/root.js",
            "import './child.js';",
            Arc::new(AtomicUsize::new(0)),
            slots,
            registry,
        )
        .unwrap();
        let (client, _parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert!(graph.active.is_empty());
        assert_eq!(graph.fetches.len(), 1);
        assert!(!graph.is_runnable());
    }
    #[test]
    fn same_url_different_import_types_get_distinct_pending_fetches() {
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_limits(
            "https://example.test/root.js",
            "import a from './same' with {type:'json'}; import b from './same' with {type:'css'};",
            Arc::new(AtomicUsize::new(0)),
            slots,
        )
        .unwrap();
        let (client, _parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host(),
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 2);
        assert_eq!(graph.fetches[0].kind, crate::ModuleType::Json);
        assert_eq!(graph.fetches[1].kind, crate::ModuleType::Css);
        assert_eq!(graph.fetches[0].url, graph.fetches[1].url);
        assert!(!graph.is_runnable());
    }
    #[test]
    fn saturated_shared_fetch_slots_leave_other_graph_pending_without_network() {
        let slots = Arc::new(AtomicUsize::new(0));
        let mut permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_limits(
            "https://example.test/root.js",
            "",
            Arc::new(AtomicUsize::new(0)),
            slots.clone(),
        )
        .unwrap();
        graph.pending.clear();
        graph.fetches.push_back(Fetch {
            render_blocking: false,
            parser_inserted: false,
            url: "https://example.test/waiting.js".into(),
            kind: crate::ModuleType::JavaScript,
            parent: "https://example.test/root.js".into(),
            include: false,
            policy: "no-referrer".into(),
            integrity: None,
            nonce: None,
        });
        let (client, parent) = client();
        let mut host = host();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert!(!graph.is_runnable());
        assert_eq!(graph.fetches.len(), 1);
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(permits.pop());
        assert!(graph.is_runnable());
        drop(graph);
        assert_eq!(
            slots.load(Ordering::Acquire),
            7,
            "other requests retain their slots after graph cancellation"
        );
        drop(permits);
        assert_eq!(slots.load(Ordering::Acquire), 0);
    }
    #[test]
    fn concurrent_graph_admission_and_cancellation_share_the_document_budget() {
        let budget = Arc::new(AtomicUsize::new(0));
        let held = ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES - 100)
            .unwrap();
        let first = ModuleGraphLoad::with_budget("a", &" ".repeat(64), budget.clone()).unwrap();
        assert!(matches!(
            ModuleGraphLoad::with_budget("b", &" ".repeat(64), budget.clone()),
            Err(HostError::LimitExceeded { .. })
        ));
        drop(first);
        let second = ModuleGraphLoad::with_budget("b", &" ".repeat(64), budget.clone()).unwrap();
        drop(second);
        assert_eq!(
            budget.load(Ordering::Acquire),
            MAX_QUEUED_SCRIPT_SOURCE_BYTES - 100
        );
        drop(held);
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }
    #[test]
    fn late_completion_after_graph_drop_releases_queued_source_reservation() {
        let budget = Arc::new(AtomicUsize::new(0));
        let mut graph = ModuleGraphLoad::with_budget("a", "", budget.clone()).unwrap();
        let (fetch, sender) = shared_fetch();
        graph
            .active
            .insert(("late".into(), crate::ModuleType::JavaScript), fetch);
        drop(graph);
        assert_eq!(budget.load(Ordering::Acquire), 0);
        let reservation = ScriptSourceReservation::reserve(&budget, 64).unwrap();
        let result = sender.send((
            "late".into(),
            crate::ModuleType::JavaScript,
            Ok((
                "late".into(),
                "late".into(),
                "export {};".into(),
                false,
                "no-referrer".into(),
            )),
            Some(reservation),
        ));
        assert!(result.is_err());
        drop(result);
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }
    #[test]
    fn shared_response_budget_survives_one_graph_cancellation_until_last_follower_drops() {
        let budget = Arc::new(AtomicUsize::new(0));
        let mut first = ModuleGraphLoad::with_budget("a", "", budget.clone()).unwrap();
        let mut second = ModuleGraphLoad::with_budget("b", "", budget.clone()).unwrap();
        let (fetch, sender) = shared_fetch();
        first.active.insert(
            ("child".into(), crate::ModuleType::JavaScript),
            fetch.clone(),
        );
        second.active.insert(
            ("child".into(), crate::ModuleType::JavaScript),
            fetch.clone(),
        );
        let weak = Rc::downgrade(&fetch);
        sender
            .send((
                "child".into(),
                crate::ModuleType::JavaScript,
                Ok((
                    "child".into(),
                    "child".into(),
                    "export {};".into(),
                    false,
                    String::new(),
                )),
                Some(ScriptSourceReservation::reserve(&budget, 64).unwrap()),
            ))
            .unwrap();
        drop(fetch);
        drop(first);
        second.wait_for_completion().unwrap();
        assert_eq!(budget.load(Ordering::Acquire), 65);
        assert!(weak.upgrade().is_some());
        drop(second);
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }
    #[test]
    fn cached_graph_walks_redirected_descendants_and_cycles_without_network() {
        let mut host = host();
        host.retain_verified_module_fetch(
            "https://example.test/cache.js",
            "https://example.test/redirect/cache.js",
            "import './child.js';",
        )
        .unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/redirect/child.js",
            "https://example.test/redirect/child.js",
            "import '../root.js';",
        )
        .unwrap();
        let mut resources = BTreeMap::new();
        let mut graph =
            ModuleGraphLoad::new("https://example.test/root.js", "import './cache.js';").unwrap();
        let (client, parent) = client();
        assert!(
            graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        assert!(graph.visited.contains(&(
            "https://example.test/redirect/child.js".into(),
            crate::ModuleType::JavaScript
        )));
        assert_eq!(graph.visited.len(), 3);
        assert!(!graph.is_runnable());
        parent.set_nonblocking(true).unwrap();
        let mut byte = [0];
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut byte)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn completion_budget_rejects_before_publishing_module_map_source() {
        let mut host = host();
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
        graph.bytes = 32 * 1024 * 1024;
        graph.received.push_back((
            "https://example.test/large.js".into(),
            crate::ModuleType::JavaScript,
            Ok((
                "https://example.test/large.js".into(),
                "https://example.test/large.js".into(),
                "export {};".into(),
                false,
                "no-referrer".into(),
            )),
            None,
        ));
        let (client, _parent) = client();
        let mut resources = BTreeMap::new();
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::LimitExceeded { .. })
        ));
        assert!(resources.is_empty());
        assert!(!host.module_fetch_was_attempted("https://example.test/large.js"));
    }
    #[test]
    fn decoded_resources_cannot_skip_native_module_graph_admission() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let final_url = "https://example.test/final.js";
        host.retain_verified_module_fetch(
            "https://example.test/original.js",
            final_url,
            "export const value=1;",
        )
        .unwrap();
        let mut resources = BTreeMap::from([(
            final_url.to_owned(),
            "import './unverified-child.js'; export const value=2;".to_owned(),
        )]);
        let mut graph = ModuleGraphLoad::with_limits(
            "https://example.test/root.js",
            "import './original.js'; import './final.js';",
            Default::default(),
            slots,
        )
        .unwrap();
        let (client, parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 1);
        assert_eq!(graph.fetches[0].url, final_url);
        assert_eq!(host.retained_module_fetch(final_url), None);
        assert!(
            !graph.visited.contains(&(
                "https://example.test/unverified-child.js".into(),
                crate::ModuleType::JavaScript
            )),
            "unverified decoded bodies must not drive discovery"
        );
        assert!(graph.active.is_empty());
        assert!(!graph.is_runnable());
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn later_graph_response_cannot_replace_first_success_or_failure() {
        let mut host = host();
        let first = "https://example.test/first.js";
        host.retain_verified_module_fetch(first, first, "export const version=1;")
            .unwrap();
        let failed = "https://example.test/failed.js";
        host.retain_failed_module_fetch(failed).unwrap();
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
        for url in [first, failed] {
            graph.received.push_back((
                url.into(),
                crate::ModuleType::JavaScript,
                Ok((
                    url.into(),
                    url.into(),
                    "invalid late source }".into(),
                    false,
                    "no-referrer".into(),
                )),
                None,
            ));
        }
        let (client, _parent) = client();
        let mut resources = BTreeMap::new();
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_)))
        ));
        assert_eq!(
            host.retained_module_fetch(first),
            Some(Some((first.into(), "export const version=1;".into())))
        );
        assert_eq!(host.retained_module_fetch(failed), Some(None));
        assert!(
            resources.is_empty(),
            "late bodies must not reach the shared resource map"
        );
    }
    #[test]
    fn cached_typed_dependencies_consume_the_same_graph_byte_budget_as_fetched_sources() {
        let mut host = host();
        host.retain_typed_module_fetch(
            "https://example.test/data.json",
            crate::ModuleType::Json,
            Some(("https://example.test/data.json".into(), "{}".into())),
        )
        .unwrap();
        let mut graph = ModuleGraphLoad::new(
            "https://example.test/root.js",
            "import data from './data.json' with {type:'json'};",
        )
        .unwrap();
        graph.bytes = 32 * 1024 * 1024;
        let (client, parent) = client();
        let mut resources = BTreeMap::new();
        assert!(
            matches!(graph.poll(&mut host, &client, oryn_common::v3::PageId(1), &mut resources),
            Err(HostError::LimitExceeded { resource }) if resource == "module graph source bytes")
        );
        assert!(graph.active.is_empty());
        assert!(resources.is_empty());
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn inline_and_external_roots_have_the_same_dependency_count_limit() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let (client, _parent) = client();
        for requested in [None, Some("https://example.test/root.js")] {
            for count in [256, 257] {
                let source = (0..count)
                    .map(|index| format!("import './dependency-{index}.js';"))
                    .collect::<String>();
                let mut graph = ModuleGraphLoad::with_root_request(
                    "https://example.test/root.js",
                    &source,
                    requested,
                    Default::default(),
                    slots.clone(),
                    Default::default(),
                )
                .unwrap();
                let result = graph.poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new(),
                );
                if count == 256 {
                    assert!(matches!(result, Ok(false)));
                    assert_eq!(graph.fetches.len(), 256);
                } else {
                    assert!(
                        matches!(result, Err(HostError::LimitExceeded { resource }) if resource == "module dependency graph")
                    );
                }
                assert!(graph.active.is_empty());
            }
        }
    }
    #[test]
    fn redirected_root_skips_original_request_but_discovers_final_url_as_distinct_module() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_root_request(
            "https://example.test/redirect/root.js",
            "import '../entry.js'; import './root.js';",
            Some("https://example.test/entry.js"),
            Default::default(),
            slots,
            Default::default(),
        )
        .unwrap();
        let (client, _parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 1);
        assert_eq!(
            graph.fetches[0].url,
            "https://example.test/redirect/root.js"
        );
        assert_eq!(
            graph.fetches[0].parent,
            "https://example.test/redirect/root.js"
        );
        assert!(graph.visited.contains(&(
            "https://example.test/entry.js".into(),
            crate::ModuleType::JavaScript
        )));
    }
    #[test]
    fn inline_module_base_does_not_suppress_a_real_module_request() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_root_request(
            "https://example.test/root.js",
            "import './root.js';",
            None,
            Default::default(),
            slots,
            Default::default(),
        )
        .unwrap();
        let (client, _parent) = client();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut BTreeMap::new()
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 1);
        assert_eq!(graph.fetches[0].url, "https://example.test/root.js");
        assert!(!graph.is_runnable());
    }
    #[test]
    fn queued_dependency_uses_result_published_after_discovery_without_refetching() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_limits("https://example.test/root.js",
            "import './child.js'; import './failed.js'; import data from './data.json' with {type:'json'};",
            Default::default(), slots).unwrap();
        let (client, parent) = client();
        let mut resources = BTreeMap::new();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        assert_eq!(graph.fetches.len(), 3);
        assert!(!graph.is_runnable());
        host.retain_failed_module_fetch("https://example.test/unrelated.js")
            .unwrap();
        assert!(graph.is_runnable());
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        assert!(
            !graph.is_runnable(),
            "an unrelated cache update must not cause a busy loop"
        );
        host.retain_verified_module_fetch(
            "https://example.test/child.js",
            "https://example.test/redirect/child.js",
            "import './grandchild.js';",
        )
        .unwrap();
        host.retain_failed_module_fetch("https://example.test/failed.js")
            .unwrap();
        host.retain_typed_module_fetch(
            "https://example.test/data.json",
            crate::ModuleType::Json,
            Some(("https://example.test/data.json".into(), "{}".into())),
        )
        .unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/redirect/grandchild.js",
            "https://example.test/redirect/grandchild.js",
            "export {};",
        )
        .unwrap();
        assert!(
            graph.is_runnable(),
            "cache publication wakes the queued graph despite occupied fetch slots"
        );
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        assert!(graph.fetches.is_empty());
        assert!(graph.active.is_empty());
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_)))
        ));
        assert!(graph.visited.contains(&(
            "https://example.test/redirect/grandchild.js".into(),
            crate::ModuleType::JavaScript
        )));
        assert!(resources.is_empty());
        assert!(!graph.is_runnable());
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn graph_cannot_publish_or_admit_under_a_different_active_realm() {
        let mut host = host();
        let slots = Arc::new(AtomicUsize::new(0));
        let _permits = (0..8)
            .map(|_| FetchPermit::acquire(&slots).unwrap())
            .collect::<Vec<_>>();
        let mut graph = ModuleGraphLoad::with_limits(
            "https://example.test/root.js",
            "import './child.js';",
            Default::default(),
            slots,
        )
        .unwrap();
        let (client, parent) = client();
        let mut resources = BTreeMap::new();
        assert!(
            !graph
                .poll(
                    &mut host,
                    &client,
                    oryn_common::v3::PageId(1),
                    &mut resources
                )
                .unwrap()
        );
        let other = host
            .create_realm("https://example.test", 91, Some(1))
            .unwrap();
        host.activate_realm(other).unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            91,
        )
        .unwrap();
        host.retain_failed_module_fetch("https://example.test/other-realm.js")
            .unwrap();
        assert!(
            !graph.is_runnable(),
            "other realms have separate cache revision identities"
        );
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled))
        ));
        assert!(resources.is_empty());
        assert_eq!(
            host.retained_module_fetch("https://example.test/child.js"),
            None
        );
        parent.set_nonblocking(true).unwrap();
        assert_eq!(
            std::io::Read::read(&mut &parent, &mut [0])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn full_response_cache_rejects_graph_body_before_resource_publication() {
        let mut host = host();
        for index in 0..256 {
            host.retain_failed_module_fetch(&format!("https://example.test/failed-{index}.js"))
                .unwrap();
        }
        let url = "https://example.test/overflow.js";
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
        graph.received.push_back((
            url.into(),
            crate::ModuleType::JavaScript,
            Ok((
                url.into(),
                url.into(),
                "export {};".into(),
                false,
                String::new(),
            )),
            None,
        ));
        let (client, _parent) = client();
        let mut resources = BTreeMap::new();
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::LimitExceeded { .. })
        ));
        assert!(resources.is_empty());
        assert_eq!(host.retained_module_fetch(url), None);
        assert_eq!(
            host.retained_module_fetch("https://example.test/failed-0.js"),
            Some(None)
        );
    }
    #[test]
    fn unclassified_and_transient_graph_errors_never_publish_failed_module_entries() {
        for kind in [
            crate::ModuleType::JavaScript,
            crate::ModuleType::Json,
            crate::ModuleType::Css,
        ] {
            for failure in [
                HostError::Execution("NetworkError HTTP 404".into()),
                HostError::ScriptLoad(ScriptLoadFailure::Cancelled),
                HostError::ScriptLoad(ScriptLoadFailure::Capacity("source bytes".into())),
                HostError::ScriptLoad(ScriptLoadFailure::Infrastructure("admission".into())),
            ] {
                let mut host = host();
                let url = "https://example.test/retry";
                let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
                graph
                    .received
                    .push_back((url.into(), kind, Err(failure), None));
                let (client, _parent) = client();
                let mut resources = BTreeMap::new();
                assert!(
                    graph
                        .poll(
                            &mut host,
                            &client,
                            oryn_common::v3::PageId(1),
                            &mut resources
                        )
                        .is_err()
                );
                assert_eq!(host.retained_module_fetch(url), None);
                if kind != crate::ModuleType::JavaScript {
                    assert_eq!(host.retained_typed_module_fetch(url, kind), None);
                }
                assert!(resources.is_empty());
            }
        }
    }
    #[test]
    fn failed_completion_stays_in_module_map_without_source_publication() {
        let mut host = host();
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", "").unwrap();
        graph.received.push_back((
            "https://example.test/fail.js".into(),
            crate::ModuleType::JavaScript,
            Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(
                "network failure".into(),
            ))),
            None,
        ));
        let (client, _parent) = client();
        let mut resources = BTreeMap::new();
        assert!(matches!(
            graph.poll(
                &mut host,
                &client,
                oryn_common::v3::PageId(1),
                &mut resources
            ),
            Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_)))
        ));
        assert_eq!(
            host.retained_module_fetch("https://example.test/fail.js"),
            Some(None)
        );
        assert!(resources.is_empty());
    }
    #[test]
    fn cancelled_prepared_graph_releases_receipts_and_inline_parse_roots() {
        let mut host = host();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        for _ in 0..16 {
            let mut graph = ModuleGraphLoad::with_root_request(
                "https://example.test/",
                "export const invalid = ;",
                None,
                Default::default(),
                Default::default(),
                Default::default(),
            )
            .unwrap();
            assert!(
                graph
                    .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                    .unwrap()
            );
            assert_eq!(host.module_graph_error_counts(), (0, 1));
            drop(graph);
            assert_eq!(host.module_graph_error_counts(), (0, 0));
        }
        let mut graph = ModuleGraphLoad::new(
            "https://example.test/external.js",
            "export const invalid = ;",
        )
        .unwrap();
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        assert_eq!(host.module_graph_error_counts(), (1, 1));
        drop(graph);
        assert_eq!(host.module_graph_error_counts(), (1, 0));
    }

    #[test]
    fn identical_inline_preparations_have_distinct_parse_error_receipts() {
        let mut host = host();
        let name = "https://example.test/";
        let source = "export const invalid = ;";
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        let mut first = ModuleGraphLoad::with_root_request(
            name,
            source,
            None,
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let mut second = ModuleGraphLoad::with_root_request(
            name,
            source,
            None,
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert!(
            first
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        assert!(
            second
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        assert_ne!(first.error_token, second.error_token);
        host.execute("globalThis.errors=[];onerror=(message,url,line,column,error)=>{errors.push(error);return true;}").unwrap();
        for token in [first.error_token, second.error_token] {
            host.execute_module_script_with_graph_error(
                name,
                source,
                &BTreeMap::new(),
                None,
                token,
                &mut || {},
            )
            .unwrap();
        }
        assert_eq!(host.evaluate_string("String(errors.length===2 && errors[0] instanceof SyntaxError && errors[1] instanceof SyntaxError && errors[0]!==errors[1])").unwrap(),"true");
    }

    #[test]
    fn graph_parse_error_precedes_linking_and_retains_original_exception() {
        let mut host = host();
        let root = "import {missing} from './valid.js';import './bad.js';globalThis.bodyRan=true;";
        host.retain_verified_module_fetch(
            "https://example.test/valid.js",
            "https://example.test/valid.js",
            "export const present=1;",
        )
        .unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/bad.js",
            "https://example.test/bad.js",
            "export const invalid = ;",
        )
        .unwrap();
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", root).unwrap();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        let mut resources = BTreeMap::new();
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut resources)
                .unwrap()
        );
        assert!(graph.parse_errors.contains_key(&(
            "https://example.test/bad.js".into(),
            crate::ModuleType::JavaScript
        )));
        host.execute("globalThis.errors=[];onerror=(message,url,line,column,error)=>{errors.push([url,error]);return true;}").unwrap();
        for _ in 0..2 {
            host.execute_module_script_with_graph_error(
                "https://example.test/root.js",
                root,
                &resources,
                Some("https://example.test/root.js"),
                graph.error_token,
                &mut || {},
            )
            .unwrap();
        }
        assert_eq!(host.evaluate_string("String(errors.length===2 && errors[0][0]==='https://example.test/bad.js' && errors[0][1] instanceof SyntaxError && errors[0][1]===errors[1][1] && typeof bodyRan==='undefined')").unwrap(),"true");
    }

    #[test]
    fn cached_dependency_compilation_yields_between_bounded_source_slices() {
        let mut host = host();
        let mut root = String::new();
        for index in 0..24 {
            let url = format!("https://example.test/child-{index}.js");
            host.retain_verified_module_fetch(
                &url,
                &url,
                "globalThis.mustNotEvaluate=true;export {};",
            )
            .unwrap();
            root.push_str(&format!("import './child-{index}.js';"));
        }
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", &root).unwrap();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        assert!(
            !graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
        );
        assert!(!graph.pending.is_empty());
        assert_eq!(
            host.evaluate_string("typeof mustNotEvaluate").unwrap(),
            "undefined"
        );
        for _ in 0..4 {
            if graph
                .poll_with_backend(&mut host, &backend, &mut BTreeMap::new())
                .unwrap()
            {
                return;
            }
        }
        panic!("cached graph did not finish its bounded slices");
    }

    #[test]
    fn json_parse_error_precedes_javascript_link_error_and_keeps_value_identity() {
        let mut host = host();
        let json = "https://example.test/bad.json";
        host.retain_typed_module_fetch(
            json,
            crate::ModuleType::Json,
            Some((json.into(), "{invalid".into())),
        )
        .unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/valid.js",
            "https://example.test/valid.js",
            "export const exists=1;",
        )
        .unwrap();
        let root = "import {missing} from './valid.js';import data from './bad.json' with {type:'json'};globalThis.ran=true;";
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", root).unwrap();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        let mut resources = BTreeMap::new();
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut resources)
                .unwrap()
        );
        assert!(graph.error_token.is_some());
        host.execute("globalThis.errors=[];onerror=(m,u,l,c,e)=>{errors.push(e);return true;}")
            .unwrap();
        for _ in 0..2 {
            host.execute_module_script_with_graph_error(
                "https://example.test/root.js",
                root,
                &resources,
                None,
                graph.error_token,
                &mut || {},
            )
            .unwrap();
        }
        assert_eq!(host.evaluate_string("String(errors.length===2&&errors[0] instanceof SyntaxError&&errors[0]===errors[1]&&typeof ran==='undefined')").unwrap(),"true");
        drop(graph);
        assert_eq!(host.module_graph_error_counts().1, 0);
    }

    #[test]
    fn json_error_reports_native_response_location_despite_authored_properties() {
        let mut host = host();
        let requested = "https://example.test/request.json";
        let response = "https://example.test/final/data.json";
        host.retain_typed_module_fetch(
            requested,
            crate::ModuleType::Json,
            Some((response.into(), "{".into())),
        )
        .unwrap();
        let root = "import data from './request.json' with {type:'json'};";
        let mut graph = ModuleGraphLoad::new("https://example.test/root.js", root).unwrap();
        let backend = ModuleGraphBackend::Sources(Default::default(), None, Default::default());
        let mut resources = BTreeMap::new();
        assert!(
            graph
                .poll_with_backend(&mut host, &backend, &mut resources)
                .unwrap()
        );
        host.execute("globalThis.urls=[];onerror=(message,url,line,column,error)=>{urls.push(url);error.fileName='forged';error.stack='forged';return true;}").unwrap();
        for _ in 0..2 {
            host.execute_module_script_with_graph_error(
                "https://example.test/root.js",
                root,
                &resources,
                None,
                graph.error_token,
                &mut || {},
            )
            .unwrap();
        }
        assert_eq!(
            host.evaluate_string("urls.join('|')").unwrap(),
            format!("{response}|{response}")
        );
    }
}
