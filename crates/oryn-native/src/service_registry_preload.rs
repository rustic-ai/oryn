//! Navigation preload operations carry parent-created navigation metadata and
//! are consumed once by the exact service agent/event pair.
use super::*;
use crate::network::{NetworkBroker, NetworkOperation, NetworkRequest};
static NEXT: AtomicU64 = AtomicU64::new(1u64 << 59);
pub(super) struct PreloadOwner {
    network: SharedNetworkBroker,
    admitting: AtomicBool,
}
pub(crate) struct Admission(Arc<PreloadOwner>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.admitting.store(false, Ordering::Release);
    }
}
pub(crate) struct PendingPreload {
    pub referrer_policy: crate::referrer::ReferrerPolicy,
    pub agent: Option<AgentId>,
    pub request: NetworkRequest,
    pub network: SharedNetworkBroker,
    pub custody: Option<Admission>,
    pub operation: Option<Result<NetworkOperation, String>>,
    pub cancelled: Arc<AtomicBool>,
    pub handed_off: bool,
}
impl PendingPreload {
    pub(crate) fn redirect_referrer(&mut self, headers: &[(String, String)], target: &url::Url) {
        self.referrer_policy = self.referrer_policy.after_response(headers);
        // The admitted request's referrer is the upper bound on disclosure.
        // Never substitute the worker script URL or a redirect destination.
        let source = self
            .request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
            .and_then(|(_, value)| url::Url::parse(value).ok());
        self.request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("referer"));
        if let Some(value) = source.and_then(|source| self.referrer_policy.header(&source, target))
        {
            self.request.headers.push(("referer".into(), value));
        }
    }
}
impl Drop for PendingPreload {
    fn drop(&mut self) {
        // Closing the receiver transfers cancellation to the retained native owner.
        // Never wait for broker ACK while unwinding a service registry operation.
        if !self.handed_off {
            self.operation.take();
        }
    }
}
impl ServiceRegistry {
    fn reserve_preload_owner(
        &self,
        network: SharedNetworkBroker,
    ) -> Result<Admission, WorkerError> {
        let mut owners = self
            .preload_owners
            .lock()
            .map_err(|_| error("preload cancellation owners poisoned"))?;
        if self.retired.load(Ordering::Acquire) {
            return Err(WorkerError::new("AbortError", "service registry retired"));
        }
        // An admission remains visible even before the broker publishes its ID.
        // Zero-budget adapter drain inspects local identity ownership only.
        owners.retain(|owner| {
            owner.admitting.load(Ordering::Acquire)
                || owner
                    .network
                    .drain_owned_operations(Duration::ZERO)
                    .is_err()
        });
        if owners.len() >= 256 {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "retained preload cancellation capacity",
            ));
        }
        let owner = Arc::new(PreloadOwner {
            network,
            admitting: AtomicBool::new(true),
        });
        owners.push(owner.clone());
        Ok(Admission(owner))
    }
    pub(super) fn drain_preload_owners(&self, budget: Duration) -> Result<(), WorkerError> {
        let deadline = Instant::now() + budget;
        let owners = self
            .preload_owners
            .lock()
            .map_err(|_| error("preload cancellation owners poisoned"))?
            .clone();
        for owner in owners {
            while owner.admitting.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err(error(
                        "preload admission or body remains owned during retirement",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            owner
                .network
                .drain_owned_operations(deadline.saturating_duration_since(Instant::now()))
                .map_err(|failure| error(failure.to_string()))?;
        }
        // Retirement seals admission before this is called; retain on failure.
        self.preload_owners
            .lock()
            .map_err(|_| error("preload cancellation owners poisoned"))?
            .clear();
        Ok(())
    }

    pub(super) fn prepare_navigation_preload(
        &self,
        partition: &str,
        origin: &AgentOrigin,
        request: &NetworkRequest,
        network: SharedNetworkBroker,
        cancelled: Arc<AtomicBool>,
        referrer_policy: crate::referrer::ReferrerPolicy,
    ) -> Result<Option<PendingPreload>, WorkerError> {
        if request.method != "GET" {
            return Ok(None);
        }
        let Some(value) = persistence::navigation_preload_snapshot(
            &self.context()?,
            partition,
            origin,
            &request.url,
        )?
        else {
            return Ok(None);
        };
        if !value.enabled {
            return Ok(None);
        }
        let network = network.new_request_scope();
        let admission = self.reserve_preload_owner(network.clone())?;
        let mut request = request.clone();
        request.request_id = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| error("navigation preload identities exhausted"))?;
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("service-worker-navigation-preload"));
        request.headers.push((
            "service-worker-navigation-preload".into(),
            value.header_value,
        ));
        let operation = if cancelled.load(Ordering::Acquire) {
            Err("AbortError: navigation cancelled".into())
        } else {
            network
                .start_operation(request.clone())
                .map_err(|failure| failure.to_string())
        };
        if self.retired.load(Ordering::Acquire) {
            drop(operation);
            return Err(WorkerError::new(
                "AbortError",
                "preload retired during admission",
            ));
        }
        Ok(Some(PendingPreload {
            custody: Some(admission),
            referrer_policy,
            agent: None,
            request,
            network,
            operation: Some(operation),
            cancelled,
            handed_off: false,
        }))
    }
    pub(super) fn arm_navigation_preload(
        &self,
        agent: AgentId,
        task: u64,
        mut prepared: PendingPreload,
    ) -> Result<(), WorkerError> {
        let bootstrap = self.context()?.worker_bootstrap(agent)?;
        if bootstrap.kind != AgentKind::Service {
            return Err(error("preload target is not a service agent"));
        }
        let mut entries = self
            .preloads
            .lock()
            .map_err(|_| error("preload ownership poisoned"))?;
        if self.retired.load(Ordering::Acquire) {
            return Err(WorkerError::new("AbortError", "service registry retired"));
        }
        if entries.len() >= 256 || entries.contains_key(&task) {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "navigation preload admission capacity",
            ));
        }
        prepared.agent = Some(agent);
        entries.insert(task, prepared);
        Ok(())
    }
    pub(super) fn prepare_preload(
        &self,
        agent: AgentId,
        task: u64,
        request: &NetworkRequest,
        network: SharedNetworkBroker,
        cancelled: Arc<AtomicBool>,
        referrer_policy: crate::referrer::ReferrerPolicy,
    ) -> Result<bool, WorkerError> {
        let value = {
            let state = self.state()?;
            let registration = state
                .versions
                .get(&agent)
                .and_then(|version| version.bootstrap.service_registration)
                .and_then(|id| state.registrations.get(&id))
                .ok_or_else(|| error("preload service registration retired"))?;
            if !registration.snapshot.navigation_preload.enabled || request.method != "GET" {
                return Ok(false);
            }
            registration
                .snapshot
                .navigation_preload
                .header_value
                .clone()
        };
        let network = network.new_request_scope();
        let admission = self.reserve_preload_owner(network.clone())?;
        let mut request = request.clone();
        request.request_id = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| error("navigation preload identities exhausted"))?;
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("service-worker-navigation-preload"));
        request
            .headers
            .push(("service-worker-navigation-preload".into(), value));
        let mut entries = self
            .preloads
            .lock()
            .map_err(|_| error("preload ownership poisoned"))?;
        if self.retired.load(Ordering::Acquire) {
            return Err(WorkerError::new("AbortError", "service registry retired"));
        }
        if entries.len() >= 256 || entries.contains_key(&task) {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "navigation preload admission capacity",
            ));
        }
        entries.insert(
            task,
            PendingPreload {
                custody: None,
                referrer_policy,
                agent: Some(agent),
                request: request.clone(),
                network: network.clone(),
                operation: None,
                cancelled: cancelled.clone(),
                handed_off: false,
            },
        );
        drop(entries);
        // Broker admission never holds the service ownership table.
        let operation = if cancelled.load(Ordering::Acquire) {
            Err("AbortError: navigation cancelled".into())
        } else {
            network
                .start_operation(request.clone())
                .map_err(|failure| failure.to_string())
        };
        let mut entries = self
            .preloads
            .lock()
            .map_err(|_| error("preload ownership poisoned"))?;
        if let Some(entry) = entries
            .get_mut(&task)
            .filter(|_| !self.retired.load(Ordering::Acquire))
        {
            entry.operation = Some(operation);
            entry.custody = Some(admission);
            Ok(true)
        } else {
            drop(entries);
            drop(operation);
            Err(WorkerError::new(
                "AbortError",
                "preload retired during admission",
            ))
        }
    }
    pub(crate) fn take_preload(
        &self,
        agent: AgentId,
        task: u64,
    ) -> Result<PendingPreload, WorkerError> {
        let context = self.context()?;
        if context.worker_bootstrap(agent)?.kind != AgentKind::Service {
            return Err(WorkerError::new(
                "SecurityError",
                "navigation preload requires service agent",
            ));
        }
        let mut entries = self
            .preloads
            .lock()
            .map_err(|_| error("preload ownership poisoned"))?;
        if !entries
            .get(&task)
            .is_some_and(|entry| entry.agent == Some(agent))
        {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign, consumed or retired preload task",
            ));
        }
        Ok(entries.remove(&task).unwrap())
    }
    pub(super) fn discard_preload(&self, task: u64) {
        let entry = self
            .preloads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&task);
        drop(entry);
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
    struct NoDropIo;
    impl NetworkBroker for NoDropIo {
        fn send(
            &self,
            _: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            panic!("unexpected send")
        }
        fn cancel_operation(&self, _: u64) -> Result<(), crate::network::NetworkError> {
            panic!("Drop must not perform broker IO")
        }
    }
    #[test]
    fn registry_retirement_retains_unattached_preload_admission_until_publication_finishes() {
        let context = crate::Browser::new().new_context();
        let registry = ServiceRegistry::new(&context);
        let admission = registry
            .reserve_preload_owner(SharedNetworkBroker::new(NoDropIo).new_request_scope())
            .unwrap();
        registry.retired.store(true, Ordering::Release);
        assert!(registry.drain_preload_owners(Duration::ZERO).is_err());
        assert_eq!(registry.preload_owners.lock().unwrap().len(), 1);
        assert!(
            registry
                .reserve_preload_owner(SharedNetworkBroker::new(NoDropIo).new_request_scope())
                .is_err()
        );
        drop(admission);
        registry.drain_preload_owners(Duration::ZERO).unwrap();
        assert!(registry.preload_owners.lock().unwrap().is_empty());
    }
    #[test]
    fn pending_preload_drop_preserves_navigation_signal_without_broker_io() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: "https://example.test/".into(),
            headers: vec![],
            body: vec![],
            credentials: true,
            security: Default::default(),
            resolved_addrs: vec![],
        };
        let pending = PendingPreload {
            custody: None,
            referrer_policy: Default::default(),
            agent: None,
            request,
            network: SharedNetworkBroker::new(NoDropIo),
            operation: None,
            cancelled: cancelled.clone(),
            handed_off: false,
        };
        drop(pending);
        assert!(
            !cancelled.load(Ordering::Acquire),
            "discarding unused preload must not cancel its navigation"
        );
    }
}
