//! Native profile bridge ownership for transferred MessagePorts. Admission
//! validates two live principals under ordered context snapshots; the pump
//! owns entanglement independently of the original sending document.

/// Inputs for authority.
struct MessageAuthorityInputs<'a> {
    context: &'a BrowserContext,
    owner: &'a AgentOwner,
    pages: &'a BTreeMap<PageId, PageRecord>,
    origins: &'a BTreeMap<PageId, SecurityOrigin>,
    agents: &'a AgentRegistry,
    hub: &'a MessageHub,
    cached: &'a BTreeMap<oryn_js::message_wire::ParkedDocumentToken, CachedDocumentSecurity>,
    allow_retained: bool,
}
use super::*;
use crate::agent_registry::{AgentOwner, AgentRegistry};
use oryn_js::{
    message_wire::{CrossHubPortBridge, MessageHub, Principal, WindowTarget, WireValue},
    worker_wire::WorkerError,
};
struct Route {
    left: WeakBrowserContext,
    right: WeakBrowserContext,
    bridges: Vec<CrossHubPortBridge>,
}
#[derive(Default)]
pub(crate) struct ProfileMessageBridges {
    routes: Mutex<Vec<Route>>,
    changed: std::sync::Condvar,
    running: std::sync::atomic::AtomicBool,
    stopping: std::sync::atomic::AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
fn failure(message: impl Into<String>) -> WorkerError {
    WorkerError::new("InvalidStateError", message)
}
fn principal(owner: &AgentOwner) -> Principal {
    match owner {
        AgentOwner::Window { page, .. } => Principal::Window(page.0),
        AgentOwner::Agent(id) => Principal::Agent(id.0),
    }
}
fn stable(partition: &str) -> Result<String, WorkerError> {
    let parts: serde_json::Value =
        serde_json::from_str(partition).map_err(|_| failure("invalid native partition"))?;
    let parts = parts
        .as_array()
        .filter(|parts| parts.len() >= 3 && parts[0].is_u64())
        .ok_or_else(|| failure("invalid native partition"))?;
    serde_json::to_string(&parts[1..]).map_err(|error| failure(error.to_string()))
}
fn authority(params: MessageAuthorityInputs<'_>) -> Result<(String, String), WorkerError> {
    let MessageAuthorityInputs {
        context,
        owner,
        pages,
        origins,
        agents,
        hub,
        cached,
        allow_retained,
    } = params;

    if context.inner.workers_retired.load(Ordering::Acquire) {
        return Err(failure("cross-context participant retired"));
    }
    match owner {
        AgentOwner::Agent(id) => {
            let (partition, origin) = agents.principal(*id)?;
            if origin.opaque {
                return Err(failure("opaque service message principal"));
            }
            Ok((stable(partition)?, origin.authority.clone()))
        }
        AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } => {
            if !pages.get(page).is_some_and(|record| record.is_live()) {
                return Err(failure("cross-context source page retired"));
            }
            let top = origins
                .get(page)
                .ok_or_else(|| failure("cross-context page origin missing"))?;
            let active = match target {
                WindowTarget::Root => hub
                    .root_document_authority(page.0, *generation, *epoch)
                    .map(|_| (top.serialize(), top.security_token_key())),
                WindowTarget::Frame(frame) => {
                    hub.frame_document_authority(page.0, frame, *generation, *epoch)
                }
            };
            let (serialized, origin) = match active {
                Ok(origin) => origin,
                Err(error) => {
                    if !allow_retained {
                        return Err(failure(error));
                    }
                    hub.retained_document_authority(page.0, target, *generation, *epoch)
                        .map_err(failure)?;
                    let (partition, origin, _) = cached
                        .values()
                        .filter(|snapshot| snapshot.page == *page && !snapshot.evicted)
                        .find_map(|snapshot| snapshot.worker_authorities.get(owner))
                        .ok_or_else(|| failure("cached recipient authority unavailable"))?;
                    if origin.opaque {
                        return Err(failure("opaque cached service recipient"));
                    }
                    return Ok((stable(partition)?, origin.authority.clone()));
                }
            };
            if serialized == "null" {
                return Err(failure("opaque service message source"));
            }
            Ok((
                stable(&native_storage_partition(context.id(), top))?,
                origin,
            ))
        }
    }
}
fn hubs<T>(
    left: &BrowserContext,
    right: &BrowserContext,
    apply: impl FnOnce(&mut MessageHub, &mut MessageHub) -> Result<T, WorkerError>,
) -> Result<T, WorkerError> {
    if left.id() == right.id() {
        return Err(failure("cross-context route requires distinct contexts"));
    }
    if left.id() > right.id() {
        let mut b = right
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("right message hub poisoned"))?;
        let mut a = left
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("left message hub poisoned"))?;
        apply(&mut a, &mut b)
    } else {
        let mut a = left
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("left message hub poisoned"))?;
        let mut b = right
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("right message hub poisoned"))?;
        apply(&mut a, &mut b)
    }
}
impl BrowserContext {
    pub(crate) fn route_service_value_to(
        &self,
        target: &BrowserContext,
        sender: &AgentOwner,
        recipient: &AgentOwner,
        value: &mut WireValue,
    ) -> Result<(), WorkerError> {
        let profile = self
            .context_store()
            .service_coordinator()
            .map_err(|error| failure(error.to_string()))?;
        let target_profile = target
            .context_store()
            .service_coordinator()
            .map_err(|error| failure(error.to_string()))?;
        if !Arc::ptr_eq(&profile, &target_profile) {
            return Err(WorkerError::new(
                "SecurityError",
                "service message crosses profiles",
            ));
        }
        if self.id() == target.id() {
            let pages = self
                .inner
                .group
                .pages
                .lock()
                .map_err(|_| failure("page registry poisoned"))?;
            let origins = self
                .inner
                .security_origins
                .lock()
                .map_err(|_| failure("origin registry poisoned"))?;
            let cached = self
                .inner
                .cached_document_security
                .lock()
                .map_err(|_| failure("cached recipient authority poisoned"))?;
            let agents = self
                .inner
                .agents
                .lock()
                .map_err(|_| failure("agent registry poisoned"))?;
            let mut hub = self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| failure("message hub poisoned"))?;
            if authority(MessageAuthorityInputs {
                context: self,
                owner: sender,
                pages: &pages,
                origins: &origins,
                agents: &agents,
                hub: &hub,
                cached: &cached,
                allow_retained: false,
            })? != authority(MessageAuthorityInputs {
                context: self,
                owner: recipient,
                pages: &pages,
                origins: &origins,
                agents: &agents,
                hub: &hub,
                cached: &cached,
                allow_retained: true,
            })? {
                return Err(WorkerError::new(
                    "SecurityError",
                    "service message crosses native storage or origin authority",
                ));
            }
            return hub
                .route_value(principal(sender), principal(recipient), value)
                .map_err(failure);
        }
        let manager = profile.versions.bridges.clone();
        manager.start()?;
        let mut routes = manager
            .routes
            .lock()
            .map_err(|_| failure("profile bridge ownership poisoned"))?;
        if routes.len() >= 4096 {
            return Err(failure("profile bridge route capacity"));
        }
        let bridges = self.admit_cross_context_value(target, sender, recipient, value)?;
        if !bridges.is_empty() {
            routes.push(Route {
                left: self.downgrade(),
                right: target.downgrade(),
                bridges,
            });
            manager.changed.notify_all();
        }
        Ok(())
    }
    fn admit_cross_context_value(
        &self,
        target: &BrowserContext,
        sender: &AgentOwner,
        recipient: &AgentOwner,
        value: &mut WireValue,
    ) -> Result<Vec<CrossHubPortBridge>, WorkerError> {
        if self.id() > target.id() {
            return target.admit_cross_context_value_reverse(self, recipient, sender, value);
        }
        self.admit_cross_context_value_ordered(target, sender, recipient, value, false)
    }
    fn admit_cross_context_value_reverse(
        &self,
        target: &BrowserContext,
        sender: &AgentOwner,
        recipient: &AgentOwner,
        value: &mut WireValue,
    ) -> Result<Vec<CrossHubPortBridge>, WorkerError> {
        self.admit_cross_context_value_ordered(target, sender, recipient, value, true)
    }
    fn admit_cross_context_value_ordered(
        &self,
        target: &BrowserContext,
        left_owner: &AgentOwner,
        right_owner: &AgentOwner,
        value: &mut WireValue,
        reverse: bool,
    ) -> Result<Vec<CrossHubPortBridge>, WorkerError> {
        let left_pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| failure("left page registry poisoned"))?;
        let right_pages = target
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| failure("right page registry poisoned"))?;
        let left_origins = self
            .inner
            .security_origins
            .lock()
            .map_err(|_| failure("left origin registry poisoned"))?;
        let right_origins = target
            .inner
            .security_origins
            .lock()
            .map_err(|_| failure("right origin registry poisoned"))?;
        let left_cached = self
            .inner
            .cached_document_security
            .lock()
            .map_err(|_| failure("left cached authority poisoned"))?;
        let right_cached = target
            .inner
            .cached_document_security
            .lock()
            .map_err(|_| failure("right cached authority poisoned"))?;
        let left_agents = self
            .inner
            .agents
            .lock()
            .map_err(|_| failure("left agent registry poisoned"))?;
        let right_agents = target
            .inner
            .agents
            .lock()
            .map_err(|_| failure("right agent registry poisoned"))?;
        let mut left_hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("left message hub poisoned"))?;
        let mut right_hub = target
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| failure("right message hub poisoned"))?;
        let left = authority(MessageAuthorityInputs {
            context: self,
            owner: left_owner,
            pages: &left_pages,
            origins: &left_origins,
            agents: &left_agents,
            hub: &left_hub,
            cached: &left_cached,
            allow_retained: reverse,
        })?;
        let right = authority(MessageAuthorityInputs {
            context: target,
            owner: right_owner,
            pages: &right_pages,
            origins: &right_origins,
            agents: &right_agents,
            hub: &right_hub,
            cached: &right_cached,
            allow_retained: !reverse,
        })?;
        if left != right {
            return Err(WorkerError::new(
                "SecurityError",
                "service message crosses native storage or origin authority",
            ));
        }
        if reverse {
            oryn_js::message_wire::route_cross_hub_value(
                &mut right_hub,
                &mut left_hub,
                principal(right_owner),
                principal(left_owner),
                value,
            )
            .map_err(failure)
        } else {
            oryn_js::message_wire::route_cross_hub_value(
                &mut left_hub,
                &mut right_hub,
                principal(left_owner),
                principal(right_owner),
                value,
            )
            .map_err(failure)
        }
    }
}
impl ProfileMessageBridges {
    fn start(self: &Arc<Self>) -> Result<(), WorkerError> {
        let mut thread = self
            .thread
            .lock()
            .map_err(|_| failure("profile bridge supervisor poisoned"))?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(failure("profile bridge supervisor retired"));
        }
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(previous) = thread.take() {
            previous
                .join()
                .map_err(|_| failure("profile bridge supervisor panicked"))?;
        }
        self.running.store(true, Ordering::Release);
        let manager = self.clone();
        match std::thread::Builder::new()
            .name("oryn-profile-message-bridges".into())
            .spawn(move || manager.run())
        {
            Ok(handle) => {
                *thread = Some(handle);
                Ok(())
            }
            Err(error) => {
                self.running.store(false, Ordering::Release);
                Err(failure(error.to_string()))
            }
        }
    }
    pub(crate) fn shutdown(&self) -> Result<(), WorkerError> {
        self.stopping.store(true, Ordering::Release);
        self.changed.notify_all();
        let thread = self
            .thread
            .lock()
            .map_err(|_| failure("profile bridge supervisor poisoned"))?
            .take();
        let mut join_failed = false;
        if let Some(thread) = thread {
            // Last context ownership may be released by a pump wakeup. The
            // running supervisor owns its stack and observes stopping next;
            // attempting to join that same stack would deadlock destruction.
            if thread.thread().id() == std::thread::current().id() {
                drop(thread);
                return Ok(());
            }
            join_failed = thread.join().is_err();
            self.running.store(false, Ordering::Release);
        }
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        routes.retain_mut(|route| {
            match (route.left.upgrade(), route.right.upgrade()) {
                (Some(left), Some(right)) => {
                    let mut retire = |a: &mut MessageHub, b: &mut MessageHub| {
                        route.bridges.retain(|bridge| bridge.retire(a, b).is_err());
                    };
                    // Poison recovery is teardown-only; it never reopens a
                    // principal or admits another message into either hub.
                    if left.id() < right.id() {
                        let mut a = left
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        let mut b = right
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        retire(&mut a, &mut b);
                    } else {
                        let mut b = right
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        let mut a = left
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        retire(&mut a, &mut b);
                    }
                }
                (Some(context), None) => {
                    let mut hub = context
                        .inner
                        .group
                        .messages
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    route
                        .bridges
                        .retain(|bridge| bridge.retire_left(&mut hub).is_err());
                }
                (None, Some(context)) => {
                    let mut hub = context
                        .inner
                        .group
                        .messages
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    route
                        .bridges
                        .retain(|bridge| bridge.retire_right(&mut hub).is_err());
                }
                (None, None) => route.bridges.clear(),
            }
            !route.bridges.is_empty()
        });
        if !routes.is_empty() {
            return Err(failure("profile bridge teardown retained failed routes"));
        }
        if join_failed {
            return Err(failure(
                "profile bridge supervisor panicked; remaining routes retired",
            ));
        }
        Ok(())
    }
    fn run(self: Arc<Self>) {
        loop {
            let mut routes = self
                .routes
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut wake = Vec::new();
            routes.retain_mut(|route| {
                let left = route.left.upgrade();
                let right = route.right.upgrade();
                match (left, right) {
                    (Some(left), Some(right)) => {
                        let retired = self.stopping.load(Ordering::Acquire)
                            || left.inner.workers_retired.load(Ordering::Acquire)
                            || right.inner.workers_retired.load(Ordering::Acquire);
                        let result = hubs(&left, &right, |a, b| {
                            let mut nested = Vec::new();
                            let mut first_error = None;
                            for bridge in &route.bridges {
                                let result = if retired {
                                    bridge.retire(a, b)
                                } else {
                                    bridge.pump(a, b, &mut nested)
                                };
                                if let Err(error) = result {
                                    first_error.get_or_insert(error);
                                }
                            }
                            route.bridges.extend(nested);
                            route
                                .bridges
                                .retain(|bridge| !matches!(bridge.retired(a, b), Ok(true)));
                            // Earlier bridges may have delivered messages before a
                            // later route hits backpressure. Always publish those
                            // wakeups; failed routes and nested ownership remain
                            // retained for the next pump or explicit shutdown.
                            Ok((a.pending_pages(), b.pending_pages(), first_error))
                        });
                        if let Ok((a, b, _retry_error)) = result {
                            wake.push((left, a));
                            wake.push((right, b));
                        }
                        !route.bridges.is_empty()
                    }
                    (Some(context), None) => {
                        let mut hub = context
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        route
                            .bridges
                            .retain(|bridge| bridge.retire_left(&mut hub).is_err());
                        !route.bridges.is_empty()
                    }
                    (None, Some(context)) => {
                        let mut hub = context
                            .inner
                            .group
                            .messages
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        route
                            .bridges
                            .retain(|bridge| bridge.retire_right(&mut hub).is_err());
                        !route.bridges.is_empty()
                    }
                    (None, None) => false,
                }
            });
            let empty = routes.is_empty();
            drop(routes);
            #[cfg(unix)]
            for (context, pages) in wake {
                for page in pages {
                    context.wake_context_events(PageId(page));
                }
            }
            #[cfg(not(unix))]
            let _ = wake;
            let routes = self
                .routes
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if self.stopping.load(Ordering::Acquire)
                || (empty && routes.is_empty() && Arc::strong_count(&self) == 1)
            {
                self.running.store(false, Ordering::Release);
                return;
            }
            drop(
                self.changed
                    .wait_timeout(routes, Duration::from_millis(if empty { 100 } else { 2 })),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, MessageResponse};
    fn payload(ports: Vec<oryn_js::message_wire::PortToken>) -> WireValue {
        WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![3],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports,
        }
    }
    fn bind(context: &BrowserContext, page: PageId) -> AgentOwner {
        context
            .commit_page_url(page, "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page, "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0
    }
    #[test]
    fn panicked_supervisor_is_joined_and_shutdown_can_be_retried() {
        let manager = ProfileMessageBridges::default();
        manager.running.store(true, Ordering::Release);
        *manager.thread.lock().unwrap() = Some(std::thread::spawn(|| {
            panic!("simulated bridge pump failure")
        }));
        assert!(manager.shutdown().is_err());
        assert!(!manager.running.load(Ordering::Acquire));
        assert!(manager.thread.lock().unwrap().is_none());
        manager.shutdown().unwrap();
    }
    #[tokio::test]
    async fn parent_profile_bridge_preserves_ports_and_rejects_foreign_profile_and_stale_root() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-profile-port-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = crate::storage::PersistentProfile::open(&directory).unwrap();
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let left = browser.new_persistent_context(Default::default(), profile.clone());
        let right = browser.new_persistent_context(Default::default(), profile.clone());
        let foreign = browser.new_context();
        let a = left.new_page();
        let b = right.new_page();
        let c = foreign.new_page();
        let source = bind(&left, a.page_id());
        let target = bind(&right, b.page_id());
        let foreign_owner = bind(&foreign, c.page_id());
        let MessageResponse::Channel([send, transfer]) = left
            .handle_message_wire(a.page_id(), MessageRequest::CreateChannel)
            .unwrap()
        else {
            panic!("channel")
        };
        let MessageResponse::Transferred(ports) = left
            .handle_message_wire(
                a.page_id(),
                MessageRequest::Transfer {
                    ports: vec![transfer],
                },
            )
            .unwrap()
        else {
            panic!("transfer")
        };
        let mut message = payload(ports);
        let original = message.ports.clone();
        assert!(
            left.route_service_value_to(&foreign, &source, &foreign_owner, &mut message)
                .is_err()
        );
        assert_eq!(message.ports, original);
        let stale = AgentOwner::Window {
            page: a.page_id(),
            target: WindowTarget::Root,
            generation: 99,
            epoch: 0,
        };
        assert!(
            left.route_service_value_to(&right, &stale, &target, &mut message)
                .is_err()
        );
        assert_eq!(message.ports, original);
        left.route_service_value_to(&right, &source, &target, &mut message)
            .unwrap();
        let received = message.ports.remove(0);
        left.handle_message_wire(
            a.page_id(),
            MessageRequest::PostPort {
                port: send.clone(),
                payload: payload(Vec::new()),
            },
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let MessageResponse::PortMessages { messages, .. } = right
                .handle_message_wire(
                    b.page_id(),
                    MessageRequest::PollPort {
                        port: received.clone(),
                        max: 1,
                    },
                )
                .unwrap()
            else {
                panic!("poll")
            };
            if !messages.is_empty() {
                assert_eq!(messages[0].bytes, vec![3]);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        right
            .handle_message_wire(b.page_id(), MessageRequest::ClosePort { port: received })
            .unwrap();
        loop {
            let MessageResponse::PortMessages { peer_closed, .. } = left
                .handle_message_wire(
                    a.page_id(),
                    MessageRequest::PollPort {
                        port: send.clone(),
                        max: 1,
                    },
                )
                .unwrap()
            else {
                panic!("close poll")
            };
            if peer_closed {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let manager = left
            .context_store()
            .service_coordinator()
            .unwrap()
            .versions
            .bridges
            .clone();
        manager.shutdown().unwrap();
        assert!(!manager.running.load(Ordering::Acquire));
        assert!(manager.thread.lock().unwrap().is_none());
        a.close().await.unwrap();
        b.close().await.unwrap();
        c.close().await.unwrap();
        left.retire_workers().await.unwrap();
        right.retire_workers().await.unwrap();
        foreign.retire_workers().await.unwrap();
        drop(a);
        drop(b);
        drop(c);
        drop(left);
        drop(right);
        drop(foreign);
        drop(browser);
        drop(profile);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
