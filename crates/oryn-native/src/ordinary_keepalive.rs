//! Single-use native prelaunch custody. The caller cannot replace any request
//! field after its Document is retired.
#[cfg(test)]
type DispatchGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

use super::*;
use std::time::Instant;
#[derive(Clone)]
pub(crate) struct Authority {
    pub owner: crate::agent_registry::AgentOwner,
    pub native: (
        String,
        oryn_js::worker_wire::AgentOrigin,
        oryn_js::worker_wire::AgentPolicy,
    ),
    pub top_url: String,
    pub referrer: crate::referrer::ReferrerPolicy,
    pub controller: Option<Arc<crate::service_registry::FrozenFetchController>>,
    pub blob: Option<(
        String,
        Arc<crate::runtime::worker_service::NativeBlobResource>,
    )>,
}
struct ResultState {
    abandoned: bool,
    result: Option<Result<ContextServiceResponse, RuntimeError>>,
}
pub(super) struct Job {
    release_retry: crate::keepalive_drain::Ticket,
    state: Mutex<ResultState>,
    changed: std::sync::Condvar,
    lifecycle: Arc<SocketLifecycle>,
    creator: Option<oryn_js::worker_wire::Creator>,
    expires: Instant,
    bytes: usize,
}
impl Job {
    fn release_result(&self, result: Result<ContextServiceResponse, RuntimeError>) {
        if let Ok(ContextServiceResponse::HttpStreamStarted { stream_id, .. }) = result
            && self
                .lifecycle
                .release_result(self.creator.as_ref(), Some(stream_id), None)
                .is_err()
        {
            let lifecycle = self.lifecycle.clone();
            let creator = self.creator.clone();
            self.release_retry.orphan(move || {
                lifecycle
                    .release_result(creator.as_ref(), Some(stream_id), None)
                    .is_ok()
            });
        }
    }
    fn publish(&self, result: Result<ContextServiceResponse, RuntimeError>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.abandoned {
            drop(state);
            self.release_result(result);
        } else {
            state.result = Some(result);
            self.changed.notify_all();
        }
    }
    fn abandon(&self) {
        let result = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.abandoned = true;
            self.changed.notify_all();
            state.result.take()
        };
        if let Some(result) = result {
            self.release_result(result);
        }
    }
    pub(super) fn wait(&self) -> Result<ContextServiceResponse, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(result) = state.result.take() {
                return result;
            }
            if state.abandoned {
                return Err(RuntimeError::Network("keepalive response abandoned".into()));
            }
            let Some(wait) = self.expires.checked_duration_since(Instant::now()) else {
                drop(state);
                self.abandon();
                return Err(RuntimeError::Network(
                    "keepalive response deadline exceeded".into(),
                ));
            };
            state = self
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
#[derive(Default)]
pub(crate) struct Registry {
    entries: Mutex<BTreeMap<String, Arc<Job>>>,
    #[cfg(test)]
    dispatch_gate: Mutex<Option<DispatchGate>>,
}
impl Registry {
    fn insert(
        &self,
        request: &HttpStreamRequest,
        authority: &Authority,
        lifecycle: Arc<SocketLifecycle>,
    ) -> Result<(String, Arc<Job>), RuntimeError> {
        let bytes = serde_json::to_vec(request)
            .map_err(|e| RuntimeError::ProtocolViolation(e.to_string()))?
            .len()
            .saturating_add(
                serde_json::to_vec(&authority.native)
                    .map_err(|e| RuntimeError::ProtocolViolation(e.to_string()))?
                    .len(),
            )
            .saturating_add(authority.top_url.len())
            .saturating_add(
                authority
                    .blob
                    .as_ref()
                    .map_or(0, |(_, blob)| blob.bytes.len()),
            );
        let mut entries = self.entries.lock().map_err(|_| {
            RuntimeError::ContextService("keepalive receipt registry poisoned".into())
        })?;
        entries.retain(|_, entry| {
            if entry.expires <= Instant::now() {
                entry.abandon();
                false
            } else {
                true
            }
        });
        if entries.len() >= 1024
            || bytes
                > (32 * 1024 * 1024usize)
                    .saturating_sub(entries.values().map(|e| e.bytes).sum::<usize>())
        {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "keepalive prelaunch receipts".into(),
            });
        }
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|e| RuntimeError::ProtocolViolation(e.to_string()))?;
        let token =
            base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, random);
        if entries.contains_key(&token) {
            return Err(RuntimeError::ProtocolViolation(
                "keepalive receipt identity collision".into(),
            ));
        }
        let release_retry =
            crate::keepalive_drain::Ticket::reserve().map_err(RuntimeError::ContextService)?;
        let job = Arc::new(Job {
            release_retry,
            state: Mutex::new(ResultState {
                abandoned: false,
                result: None,
            }),
            changed: Default::default(),
            lifecycle,
            creator: request.client.clone(),
            expires: Instant::now() + Duration::from_secs(60),
            bytes,
        });
        entries.insert(token.clone(), job.clone());
        Ok((token, job))
    }
    pub(super) fn take(&self, token: &str) -> Result<Arc<Job>, RuntimeError> {
        self.entries
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("keepalive receipt registry poisoned".into())
            })?
            .remove(token)
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation("unknown or consumed keepalive receipt".into())
            })
    }
    pub(crate) fn release(&self, token: &str) {
        let job = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token);
        if let Some(job) = job {
            job.abandon();
        }
    }
    pub(super) fn clear(&self) {
        let jobs = std::mem::take(&mut *self.entries.lock().unwrap_or_else(|e| e.into_inner()));
        for job in jobs.values() {
            job.abandon();
        }
    }
}
impl ParentContextService {
    pub(super) fn prepare_ordinary_keepalive(
        &self,
        page: PageId,
        request: HttpStreamRequest,
    ) -> Result<String, RuntimeError> {
        self.validate_socket_owner(page)?;
        if !request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive)
            || request.upgrade_mixed_image
        {
            return Err(RuntimeError::ProtocolViolation(
                "ordinary keepalive receipt requires keepalive fetch".into(),
            ));
        }
        request
            .fetch
            .as_ref()
            .unwrap()
            .validate()
            .map_err(|e| RuntimeError::ProtocolViolation(e.into()))?;
        let creator = request.client.as_ref().ok_or_else(|| {
            RuntimeError::ProtocolViolation("keepalive requires native creator".into())
        })?;
        if request.body_base64.len() > 4 * 65536_usize.div_ceil(3)
            || request.url.len() > 256 * 1024
            || request.method.len() > 32
            || request.headers.len() > 1024
            || request.headers.iter().fold(0usize, |bytes, (name, value)| {
                bytes.saturating_add(name.len()).saturating_add(value.len())
            }) > 256 * 1024
        {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "keepalive request metadata".into(),
            });
        }
        let body = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &request.body_base64,
        )
        .map_err(|e| RuntimeError::ProtocolViolation(e.to_string()))?;
        if body.len() > 65536 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "keepalive body".into(),
            });
        }
        let mut authority = self
            .context_owner()?
            .freeze_ordinary_keepalive_authority(page, creator)?;
        let target = Url::parse(&request.url)
            .or_else(|_| Url::parse(&authority.native.2.url)?.join(&request.url))
            .map_err(|e| RuntimeError::Network(e.to_string()))?;
        if target.scheme() == "blob" {
            let mut lookup = target.clone();
            lookup.set_fragment(None);
            let blob = self
                .context_owner()?
                .resolve_native_blob(&authority.owner, lookup.as_str())
                .map_err(|e| RuntimeError::Network(e.to_string()))?;
            if blob.bytes.len() > 16 * 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "keepalive blob response".into(),
                });
            }
            authority.blob = Some((lookup.to_string(), Arc::new(blob)));
        }
        let service = self
            .weak_self
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                RuntimeError::ContextService("keepalive parent executor is not attached".into())
            })?;
        let (token, job) =
            self.ordinary_keepalive
                .insert(&request, &authority, self.socket_lifecycle.clone())?;
        #[cfg(test)]
        let dispatch_gate = self
            .ordinary_keepalive
            .dispatch_gate
            .lock()
            .unwrap()
            .clone();
        let accepted = crate::resource_io_executor::submit(move || {
            #[cfg(test)]
            if let Some(gate) = dispatch_gate {
                let mut ready = gate.0.lock().unwrap();
                while !*ready {
                    ready = gate.1.wait(ready).unwrap();
                }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id: page,
                    request,
                    destination: crate::csp::CspDestination::Connect,
                    cors_enabled: true,
                    media_destination: None,
                    preload: false,
                    xslt: false,
                    template_nonce: None,
                    frozen: Some(authority),
                })
            }))
            .unwrap_or_else(|_| {
                Err(RuntimeError::Network(
                    "keepalive parent dispatch panicked".into(),
                ))
            });
            job.publish(result);
        });
        if let Err(error) = accepted {
            self.ordinary_keepalive.release(&token);
            return Err(RuntimeError::ContextService(error));
        }
        Ok(token)
    }
}

#[cfg(feature = "v8-host")]
pub(crate) struct Lease {
    ticket: script_cleanup_custody::Ticket,
    page: PageId,
}
#[cfg(feature = "v8-host")]
impl Lease {
    pub(crate) fn attach(&self, token: &str) {
        self.ticket
            .attach_request(ContextServiceRequest::ReleaseOrdinaryKeepalive {
                page_id: self.page,
                token: token.into(),
            });
    }
}
#[cfg(feature = "v8-host")]
impl SharedContextClient {
    pub(crate) fn reserve_ordinary_keepalive_cleanup(
        &self,
        page: PageId,
    ) -> Result<Lease, RuntimeError> {
        Ok(Lease {
            ticket: script_cleanup_custody::Ticket::reserve(self.clone(), page)
                .map_err(RuntimeError::ContextService)?,
            page,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    use std::time::Instant;
    struct Recording(Arc<Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Recording {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.0.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![],
                body: vec![],
            })
        }
    }
    type KeepaliveFixture = (
        BrowserContext,
        crate::PageHandle,
        Arc<ParentContextService>,
        Creator,
        Arc<Mutex<Vec<NetworkRequest>>>,
    );

    fn fixture() -> KeepaliveFixture {
        let context =
            crate::Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://accepted.test/source".into(), &[])
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
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context.clone(),
            id,
            Some(SharedNetworkBroker::new(Recording(seen.clone()))),
        )
        .into_shared();
        (context, page, service, creator, seen)
    }
    fn request(creator: Creator) -> HttpStreamRequest {
        HttpStreamRequest {
            upgrade_mixed_image: false,
            fetch: Some(oryn_js::platform::FetchOptions {
                keepalive: true,
                privileged_range: false,
                cache: Default::default(),
                mode: "cors".into(),
                redirect: "follow".into(),
                integrity: String::new(),
            }),
            client: Some(creator),
            use_cors_preflight: false,
            request_id: 701,
            method: "POST".into(),
            url: "https://accepted.test/target".into(),
            headers: vec![("content-type".into(), "text/plain".into())],
            body_base64: "b3JpZ2luYWw=".into(),
            credentials: true,
            credentials_mode: Some("include".into()),
            initiator_url: Some("https://accepted.test/source".into()),
            referrer_policy: None,
        }
    }
    #[test]
    fn frozen_request_survives_replacement_without_retargeting_or_replay() {
        let (context, page, service, creator, seen) = fixture();
        let id = page.page_id();
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        *service.ordinary_keepalive.dispatch_gate.lock().unwrap() = Some(gate.clone());
        let token = service
            .prepare_ordinary_keepalive(id, request(creator.clone()))
            .unwrap();
        context
            .install_document_security(
                id,
                "https://replacement.test/new".into(),
                &[(
                    "content-security-policy".into(),
                    "connect-src 'none'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 1,
                },
            )
            .unwrap();
        assert!(
            service
                .prepare_ordinary_keepalive(id, request(creator))
                .is_err()
        );
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let result = service
            .handle(ContextServiceRequest::StartOrdinaryKeepalive {
                page_id: id,
                token: token.clone(),
            })
            .unwrap();
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result else {
            panic!("frozen request did not start")
        };
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].body, b"original");
        assert_eq!(calls[0].url, "https://accepted.test/target");
        assert_eq!(
            calls[0].security.top_level_url.as_deref(),
            Some("https://accepted.test/source")
        );
        drop(calls);
        assert!(
            service
                .handle(ContextServiceRequest::StartOrdinaryKeepalive { page_id: id, token })
                .is_err()
        );
        service.abort_http_stream(id, stream_id).unwrap();
    }
    #[test]
    fn prelaunch_receipt_is_full_request_bound_page_bound_and_releasable() {
        let (_context, page, service, creator, seen) = fixture();
        let id = page.page_id();
        let mut input = request(creator.clone());
        let token = service
            .prepare_ordinary_keepalive(id, input.clone())
            .unwrap();
        input.url = "https://attacker.test/".into();
        input.body_base64 = "".into();
        assert!(
            service
                .handle(ContextServiceRequest::StartOrdinaryKeepalive {
                    page_id: PageId(id.0 + 1),
                    token: token.clone()
                })
                .is_err()
        );
        let response = service
            .handle(ContextServiceRequest::StartOrdinaryKeepalive { page_id: id, token })
            .unwrap();
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = response else {
            panic!("start")
        };
        service.abort_http_stream(id, stream_id).unwrap();
        let calls = seen.lock().unwrap();
        assert_eq!(calls[0].url, "https://accepted.test/target");
        assert_eq!(calls[0].body, b"original");
        drop(calls);
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        *service.ordinary_keepalive.dispatch_gate.lock().unwrap() = Some(gate.clone());
        let mut next = request(creator);
        next.request_id = 702;
        let token = service.prepare_ordinary_keepalive(id, next).unwrap();
        for _ in 0..2 {
            assert!(matches!(
                service.handle(ContextServiceRequest::ReleaseOrdinaryKeepalive {
                    page_id: id,
                    token: token.clone()
                }),
                Ok(ContextServiceResponse::OrdinaryKeepaliveReleased)
            ));
        }
        assert!(service.ordinary_keepalive.take(&token).is_err());
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().len() < 2 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "response release does not cancel accepted native dispatch"
        );
    }
    #[test]
    fn local_keepalive_freezes_blob_before_revocation_and_accepts_data() {
        let (context, page, service, creator, seen) = fixture();
        let id = page.page_id();
        let owner = context
            .window_worker_creator(id, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        let blob = context
            .register_native_blob(owner.clone(), b"retained".to_vec(), "text/plain".into())
            .unwrap();
        for (index, url) in [blob.clone(), "data:text/plain,retained".into()]
            .into_iter()
            .enumerate()
        {
            let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            *service.ordinary_keepalive.dispatch_gate.lock().unwrap() = Some(gate.clone());
            let mut input = request(creator.clone());
            input.request_id = 800 + index as u64;
            input.method = "GET".into();
            input.body_base64.clear();
            input.url = url;
            let token = service.prepare_ordinary_keepalive(id, input).unwrap();
            if index == 0 {
                context.revoke_native_blob(&owner, &blob).unwrap();
            }
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
            let response = service
                .handle(ContextServiceRequest::StartOrdinaryKeepalive { page_id: id, token })
                .unwrap();
            let ContextServiceResponse::HttpStreamStarted {
                stream_id, status, ..
            } = response
            else {
                panic!("local response")
            };
            assert_eq!(status, 200);
            service.abort_http_stream(id, stream_id).unwrap();
        }
        assert!(
            seen.lock().unwrap().is_empty(),
            "local keepalive must not be sent to HTTP broker"
        );
    }
    #[test]
    fn failed_prepare_response_publication_releases_receipt_without_canceling_dispatch() {
        for received in [false, true] {
            let (context, page, service, creator, seen) = fixture();
            let id = page.page_id();
            let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            *service.ordinary_keepalive.dispatch_gate.lock().unwrap() = Some(gate.clone());
            let token = service
                .prepare_ordinary_keepalive(id, request(creator))
                .unwrap();
            let response = ContextServiceResponse::OrdinaryKeepalivePrepared {
                token: token.clone(),
            };
            let scope = super::super::response_custody::ResponseScope {
                keepalive: Some(service.ordinary_keepalive.clone()),
                context: context.downgrade(),
                source: super::super::response_custody::ResponseSource::Window(id),
                scripts: None,
                sockets: None,
            };
            let mut guard = super::super::response_custody::ResponseGuard::new(
                Some(scope),
                Default::default(),
                Some(&response),
            );
            if received {
                guard.commit();
            }
            drop(guard);
            assert_eq!(
                service
                    .ordinary_keepalive
                    .entries
                    .lock()
                    .unwrap()
                    .contains_key(&token),
                received
            );
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
            if received {
                let response = service
                    .ordinary_keepalive
                    .take(&token)
                    .unwrap()
                    .wait()
                    .unwrap();
                let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = response else {
                    panic!("response")
                };
                service.abort_http_stream(id, stream_id).unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while seen.lock().unwrap().is_empty() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert_eq!(seen.lock().unwrap().len(), 1);
        }
    }
    #[test]
    fn parent_accepted_dispatch_survives_transport_teardown_without_start_rpc() {
        let (_context, page, service, creator, seen) = fixture();
        let id = page.page_id();
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        *service.ordinary_keepalive.dispatch_gate.lock().unwrap() = Some(gate.clone());
        let token = service
            .prepare_ordinary_keepalive(id, request(creator))
            .unwrap();
        service.cancel_all_network_operations();
        assert!(service.ordinary_keepalive.take(&token).is_err());
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "parent acceptance must not depend on a later worker Start RPC"
        );
    }
    #[test]
    fn prepare_rejects_unscoped_nonkeepalive_and_oversized_requests() {
        let (_context, page, service, creator, _) = fixture();
        let id = page.page_id();
        let mut input = request(creator.clone());
        input.client = None;
        assert!(service.prepare_ordinary_keepalive(id, input).is_err());
        let mut input = request(creator.clone());
        input.fetch.as_mut().unwrap().keepalive = false;
        assert!(service.prepare_ordinary_keepalive(id, input).is_err());
        let mut input = request(creator);
        input.body_base64 = "A".repeat(90000);
        assert!(service.prepare_ordinary_keepalive(id, input).is_err());
    }
}
