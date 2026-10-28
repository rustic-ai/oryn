#![cfg(unix)]

type RouteScriptCleanup = (
    oryn_common::v3::PageId,
    crate::script_load_lease::ScriptLoadRegistry,
    Arc<crate::context_service::SocketLifecycle>,
    Option<Arc<crate::navigation_control::NavigationControl>>,
    crate::context_service::NavigationResourceCleanup,
);

#[path = "navigation_network_source.rs"]
mod navigation_network_source;
#[path = "navigation_response_policy.rs"]
mod navigation_response_policy;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::MetadataExt,
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use oryn_common::v3::PageId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BrowserConfig, BrowserContext, ContextOptions, RuntimeBuildInfo,
    context_service::{
        ContextServiceRequest, ContextServiceResponse, ContextTransport, ParentContextService,
        serve_context_transport,
    },
    cookies::CookieRequestKind,
    csp::{CspDestination, CspRequest},
    network::{
        NetworkBroker, NetworkError, NetworkEvent, NetworkOperation, NetworkRequest,
        NetworkResponse, serve_ipc_broker,
    },
    runtime::RuntimeError,
    worker::{
        HostEnvelope, WORKER_PROTOCOL_VERSION, WorkerEnvelope, WorkerLimits, WorkerRequest,
        WorkerResponse,
    },
};

#[cfg(test)]
use crate::cookies::CookieRequestContext;

const EXPECTED_STYLE_ENGINE_VERSION: &str = oryn_common::STYLE_ENGINE_VERSION;
const EXPECTED_BOX_ENGINE_VERSION: &str = "taffy-0.14";
const EXPECTED_INLINE_ENGINE_VERSION: &str = "parley-0.11.1";

#[derive(Default)]
struct RouteAdmissions {
    retired: bool,
    active: usize,
    network_requests: BTreeSet<u64>,
    response_policies: navigation_response_policy::Registry,
    navigation_sources: navigation_network_source::Registry,
}

/// Parent-owned admission barrier. Revocation and admission serialize on the
/// same lock; an accepted operation retains its lease until all its work ends.
#[derive(Default)]
struct RouteAuthority {
    state: Mutex<RouteAdmissions>,
    drained: Condvar,
}

struct RouteAdmission {
    authority: Arc<RouteAuthority>,
    network_request: Option<u64>,
}

/// Owns a response reservation until synchronous completion or streaming
/// header publication transfers custody. Body completion is a separate lifetime.
struct ResponsePolicyReservation {
    authority: Arc<RouteAuthority>,
    token: Option<String>,
    admitted: std::cell::Cell<bool>,
}

impl ResponsePolicyReservation {
    fn headers(
        &self,
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<String, NetworkError> {
        let token = self.token.as_ref().expect("live response reservation");
        self.authority
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
            .response_policies
            .headers(token, status, url, headers)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        self.admitted.set(true);
        Ok(token.clone())
    }

    fn transfer_to_headers(mut self) -> crate::network::ResponsePolicyToken {
        assert!(
            self.admitted.get(),
            "response headers must be admitted before transfer"
        );
        let token = self.token.take().expect("live response reservation");
        let owned = crate::network::ResponsePolicyToken::from(token.clone());
        let authority = self.authority.clone();
        owned.attach_cleanup(move || {
            authority
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .response_policies
                .abandon_reservation(&token);
        });
        owned
    }

    fn retain_for_commit(mut self) {
        if self.admitted.get() {
            self.token.take();
        }
    }
}

impl Drop for ResponsePolicyReservation {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            self.authority
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .response_policies
                .abandon_reservation(&token);
        }
    }
}

impl RouteAuthority {
    fn ensure_live(&self) -> Result<(), NetworkError> {
        let state = self
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?;
        if state.retired {
            return Err(NetworkError::Transport(
                "worker network route revoked".into(),
            ));
        }
        Ok(())
    }

    fn admit(
        self: &Arc<Self>,
        network_request: Option<u64>,
    ) -> Result<RouteAdmission, NetworkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?;
        if state.retired {
            return Err(NetworkError::Transport(
                "worker network route revoked".into(),
            ));
        }
        if state.active >= 64 {
            return Err(NetworkError::Transport(
                "worker route admission capacity exceeded".into(),
            ));
        }
        if let Some(id) = network_request
            && !state.network_requests.insert(id)
        {
            return Err(NetworkError::Transport(
                "duplicate active worker network request".into(),
            ));
        }
        state.active += 1;
        Ok(RouteAdmission {
            authority: self.clone(),
            network_request,
        })
    }

    fn revoke(&self) -> Result<Vec<u64>, RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?;
        state.retired = true;
        state.response_policies.revoke();
        state.navigation_sources.revoke();
        self.drained.notify_all();
        Ok(state.network_requests.iter().copied().collect())
    }

    fn wait_drained(&self, timeout: Duration) -> Result<(), RuntimeError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?;
        if !state.retired {
            return Err(RuntimeError::ProtocolViolation(
                "cannot drain an active worker route".into(),
            ));
        }
        let (state, _) = self
            .drained
            .wait_timeout_while(state, timeout, |state| state.active != 0)
            .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?;
        if state.active != 0 {
            return Err(RuntimeError::ContextService(
                "worker route did not drain before reset deadline".into(),
            ));
        }
        Ok(())
    }
}

impl Drop for RouteAdmission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.authority.state.lock() {
            state.active -= 1;
            if let Some(id) = self.network_request {
                state.network_requests.remove(&id);
            }
            if state.active == 0 {
                self.authority.drained.notify_all();
            }
        }
    }
}

struct NavigationSourceRequest {
    authority: Arc<RouteAuthority>,
    nonce: String,
    request_id: u64,
}
impl NavigationSourceRequest {
    fn headers(&self, status: u16, headers: &[(String, String)]) -> Result<(), NetworkError> {
        self.authority
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
            .navigation_sources
            .headers(&self.nonce, self.request_id, status, headers)
    }
}
impl Drop for NavigationSourceRequest {
    fn drop(&mut self) {
        if let Ok(mut state) = self.authority.state.lock() {
            state
                .navigation_sources
                .finish_request(&self.nonce, self.request_id);
        }
    }
}
#[derive(Clone)]
struct PageNetworkBroker {
    authority: Arc<RouteAuthority>,
    inner: crate::network::SharedNetworkBroker,
    context: crate::runtime::WeakBrowserContext,
    page_id: PageId,
}

impl PageNetworkBroker {
    fn reserve_response_policy(
        &self,
        request: &NetworkRequest,
    ) -> Result<Option<ResponsePolicyReservation>, NetworkError> {
        if request.security.kind != CookieRequestKind::Document {
            return Ok(None);
        }
        let source = self
            .context_owner()?
            .capture_navigation_response_source(self.page_id)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let token = self
            .authority
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
            .response_policies
            .begin(request.request_id, source)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(Some(ResponsePolicyReservation {
            authority: self.authority.clone(),
            token: Some(token),
            admitted: Default::default(),
        }))
    }

    fn context_owner(&self) -> Result<BrowserContext, NetworkError> {
        self.context
            .upgrade()
            .ok_or_else(|| NetworkError::Transport("browser context is closed".into()))
    }

    fn ensure_bound(&self) -> Result<(), NetworkError> {
        self.authority.ensure_live()
    }

    fn prepare(
        &self,
        mut request: NetworkRequest,
    ) -> Result<(NetworkRequest, String, Option<NavigationSourceRequest>), NetworkError> {
        self.ensure_bound()?;
        let requested_url = request.url.clone();
        let source_nonce = request.security.navigation_source.take();
        let page_url = self
            .context_owner()?
            .page_url(self.page_id)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let target = url::Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        if let Some(upgraded) = self
            .context_owner()?
            .upgrade_csp_url(self.page_id, &target)
            .map_err(|error| NetworkError::Transport(error.to_string()))?
        {
            request.url = upgraded.to_string();
        }
        let destination = match request.security.kind {
            CookieRequestKind::Document => None,
            CookieRequestKind::Script => Some(CspDestination::Script),
            CookieRequestKind::Style => Some(CspDestination::Style),
            CookieRequestKind::Image => Some(CspDestination::Image),
            CookieRequestKind::Font => Some(CspDestination::Font),
            CookieRequestKind::Media => Some(CspDestination::Media),
            CookieRequestKind::Object => Some(CspDestination::Object),
            CookieRequestKind::Manifest => Some(CspDestination::Manifest),
            CookieRequestKind::Worker => Some(CspDestination::Worker),
            CookieRequestKind::Frame => Some(CspDestination::Frame),
            CookieRequestKind::Form => Some(CspDestination::Form),
            CookieRequestKind::Fetch | CookieRequestKind::Xhr | CookieRequestKind::WebSocket => {
                Some(CspDestination::Connect)
            }
            CookieRequestKind::Other => Some(CspDestination::Connect),
        };
        if let Some(destination) = destination {
            let decision = self
                .context_owner()?
                .authorize_csp(
                    self.page_id,
                    &CspRequest {
                        destination,
                        url: request.url.clone(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    },
                )
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
            if !decision.allowed {
                return Err(NetworkError::Transport(format!(
                    "Content-Security-Policy blocked {:?} request",
                    destination
                )));
            }
        }
        request.security.dns_timing = None;
        request.security.initiator_url = Some(page_url.clone());
        request.security.top_level_url = Some(page_url.clone());
        let source = if let Some(nonce) = source_nonce {
            let seal = self
                .authority
                .state
                .lock()
                .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
                .navigation_sources
                .seal(&nonce)?;
            // The native target must still be the sealed generation and epoch.
            // Never call into BrowserContext while holding the route mutex.
            seal.authority
                .source(
                    &self.context_owner()?,
                    self.page_id,
                    &seal.target,
                    &seal.initial_url,
                )
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
            self.authority
                .state
                .lock()
                .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
                .navigation_sources
                .consume(&nonce, &requested_url, &mut request)?;
            Some(NavigationSourceRequest {
                authority: self.authority.clone(),
                nonce,
                request_id: request.request_id,
            })
        } else {
            None
        };
        Ok((request, page_url, source))
    }
}

impl NetworkBroker for PageNetworkBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let _admission = self.authority.admit(Some(request.request_id))?;
        let (request, page_url, source) = self.prepare(request)?;
        let reservation = self.reserve_response_policy(&request)?;
        let timing = crate::network::ServiceWorkerTiming::default();
        let response = match self.context_owner()?.intercept_service_navigation(
            self.page_id,
            &request,
            self.inner.clone(),
            &timing,
        )? {
            Some(response) => response,
            None => self.inner.send(request.clone())?,
        };
        let mut response = timing.response(response);
        response.response_policy = None;
        if response.request_id != request.request_id {
            return Err(NetworkError::Transport(
                "navigation response identity mismatch".into(),
            ));
        }
        self.ensure_bound()?;
        if let Some(source) = &source {
            source.headers(response.status, &response.headers)?;
        }
        if request.security.kind == CookieRequestKind::Frame
            && crate::frame_embedding::creates_document(response.status, &response.headers)
        {
            let mut ancestors = request.security.ancestor_urls.clone();
            if !ancestors.contains(&page_url) {
                ancestors.push(page_url.clone());
            }
            crate::frame_embedding::authorize(&request.url, &response.headers, &ancestors)
                .map_err(NetworkError::Transport)?;
        }
        if let Some(reservation) = reservation
            && crate::frame_embedding::creates_document(response.status, &response.headers)
        {
            response.response_policy = Some(
                reservation
                    .headers(response.status, request.url, response.headers.clone())?
                    .into(),
            );
            reservation.retain_for_commit();
        }
        Ok(response)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let admission = self.authority.admit(Some(request.request_id))?;
        let (request, page_url, source) = self.prepare(request)?;
        let mut reservation = self.reserve_response_policy(&request)?;
        let request_id = request.request_id;
        let request_kind = request.security.kind;
        let mut frame_ancestors = request.security.ancestor_urls.clone();
        if !frame_ancestors.contains(&page_url) {
            frame_ancestors.push(page_url.clone());
        }
        let request_url = request.url.clone();
        let context = self.context.clone();
        let route = self.clone();
        let timing = crate::network::ServiceWorkerTiming::default();
        let operation = match self
            .context_owner()?
            .intercept_service_navigation_operation(
                self.page_id,
                &request,
                self.inner.clone(),
                &timing,
            )? {
            Some(operation) => operation,
            None => self.inner.start_operation(request)?,
        };
        let mut operation = timing.operation(operation);
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        std::thread::spawn(move || {
            let _admission = admission;
            let mut response_headers_transferred = false;
            while let Some(mut event) = operation.blocking_event() {
                let Some(context) = context.upgrade() else {
                    let _ = route.inner.cancel_operation(request_id);
                    let _ = sender.blocking_send(NetworkEvent::Failed {
                        request_id,
                        error: "browser context is closed".into(),
                    });
                    return;
                };
                if let Err(error) = route.ensure_bound() {
                    let _ = route.inner.cancel_operation(request_id);
                    drop(context);
                    let _ = sender.blocking_send(NetworkEvent::Failed {
                        request_id,
                        error: error.to_string().into(),
                    });
                    return;
                }
                if let NetworkEvent::Headers {
                    response_policy,
                    status,
                    headers,
                    request_id: response_id,
                    ..
                } = &mut event
                {
                    *response_policy = None;
                    // Moving cleanup custody must not admit a second response
                    // header block after the original native grant was issued.
                    if response_headers_transferred {
                        let _ = route.inner.cancel_operation(request_id);
                        drop(context);
                        let _ = sender.blocking_send(NetworkEvent::Failed {
                            request_id,
                            error: "navigation response headers already admitted".into(),
                        });
                        return;
                    }
                    if *response_id != request_id {
                        let _ = route.inner.cancel_operation(request_id);
                        drop(context);
                        let _ = sender.blocking_send(NetworkEvent::Failed {
                            request_id,
                            error: "navigation response identity mismatch".into(),
                        });
                        return;
                    }
                    let response_headers = headers.clone();
                    if let Some(source) = &source
                        && let Err(error) = source.headers(*status, &response_headers)
                    {
                        let _ = route.inner.cancel_operation(request_id);
                        drop(context);
                        let _ = sender.blocking_send(NetworkEvent::Failed {
                            request_id,
                            error: error.to_string().into(),
                        });
                        return;
                    }
                    if crate::frame_embedding::creates_document(*status, &response_headers) {
                        if let Some(grant) = reservation.take() {
                            match grant.headers(
                                *status,
                                request_url.clone(),
                                response_headers.clone(),
                            ) {
                                Ok(_) => {
                                    *response_policy = Some(grant.transfer_to_headers());
                                    response_headers_transferred = true;
                                }
                                Err(error) => {
                                    let _ = route.inner.cancel_operation(request_id);
                                    drop(context);
                                    let _ = sender.blocking_send(NetworkEvent::Failed {
                                        request_id,
                                        error: error.to_string().into(),
                                    });
                                    return;
                                }
                            }
                        }
                    } else {
                        reservation.take();
                    }
                    if request_kind == CookieRequestKind::Frame
                        && crate::frame_embedding::creates_document(*status, &response_headers)
                        && let Err(error) = crate::frame_embedding::authorize(
                            &request_url,
                            &response_headers,
                            &frame_ancestors,
                        )
                    {
                        let _ = route.inner.cancel_operation(request_id);
                        drop(context);
                        let _ = sender.blocking_send(NetworkEvent::Failed {
                            request_id,
                            error: error.into(),
                        });
                        return;
                    }
                }

                // A full downstream queue must not retain the context that
                // owns this route while waiting for a reader that was dropped.
                drop(context);
                let terminal = matches!(
                    event,
                    NetworkEvent::Complete { .. } | NetworkEvent::Failed { .. }
                );
                if sender.blocking_send(event).is_err() {
                    let _ = route.inner.cancel_operation(request_id);
                    return;
                }
                if terminal {
                    return;
                }
            }
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        let _admission = self.authority.admit(None)?;
        let mut state = self
            .authority
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?;
        state.response_policies.cancel_request(request_id);
        state.navigation_sources.cancel_request(request_id);
        let owned = state.network_requests.contains(&request_id);
        drop(state);
        if owned {
            if let Some(context) = self.context.upgrade() {
                context.cancel_service_navigation(self.page_id, request_id);
            }
            self.inner.cancel_operation(request_id)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
struct BoundWorkerRoute {
    identity: crate::worker_route::WorkerRoute,
    network: PageNetworkBroker,
    context: Arc<ParentContextService>,
}

impl BoundWorkerRoute {
    fn response_policy_error(&self, error: RuntimeError) -> RuntimeError {
        match error {
            RuntimeError::ProtocolViolation(message)
                if message.starts_with("unknown navigation response grant:") =>
            {
                RuntimeError::ProtocolViolation(format!(
                    "{message} route_process_session={} route_environment={} route_generation={} route_page={}",
                    self.identity.process_session,
                    self.identity.environment,
                    self.identity.generation,
                    self.network.page_id.0,
                ))
            }
            other => other,
        }
    }

    fn context_request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        match request {
            ContextServiceRequest::AcquireNavigationNetworkSource {
                page_id,
                nonce,
                target,
                url,
                authority,
            } => {
                self.context.ensure_page_admitted()?;
                if page_id != self.network.page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign navigation source page".into(),
                    ));
                }
                let context = self
                    .network
                    .context_owner()
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let (source, service_get) =
                    authority.network_source(&context, page_id, &target, &url)?;
                let limit = context.navigation_network_redirect_limit();
                let mut state = self.network.authority.state.lock().map_err(|_| {
                    RuntimeError::ContextService("worker route lock poisoned".into())
                })?;
                if state.retired {
                    return Err(RuntimeError::Closed);
                }
                state
                    .navigation_sources
                    .begin(
                        nonce,
                        navigation_network_source::Seal {
                            authority,
                            target,
                            initial_url: url,
                            service_get,
                        },
                        source.clone(),
                        limit,
                    )
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                Ok(ContextServiceResponse::NavigationNetworkSource {
                    source,
                    bound: true,
                })
            }
            ContextServiceRequest::DiscardNavigationNetworkSource { page_id, nonce } => {
                if page_id != self.network.page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign navigation source cleanup".into(),
                    ));
                }
                self.network
                    .authority
                    .state
                    .lock()
                    .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?
                    .navigation_sources
                    .discard(&nonce);
                Ok(ContextServiceResponse::NavigationNetworkSourceDiscarded)
            }
            ContextServiceRequest::CommitPrerenderResponse {
                response_policy: Some(response_policy),
                page_id,
                token,
                generation,
                epoch,
                url,
                headers,
            } => {
                self.context.ensure_page_admitted()?;
                if page_id != self.network.page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign prerender response page".into(),
                    ));
                }
                let context = self
                    .network
                    .context_owner()
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let scope = context.native_prerender_scope(page_id, &token, (generation, epoch))?;
                let mut policy = None;
                self.network
                    .authority
                    .state
                    .lock()
                    .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?
                    .response_policies
                    .commit(
                        page_id,
                        &response_policy,
                        &url,
                        &headers,
                        |source, url, native_headers| {
                            context
                                .native_worker_authority(source)
                                .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                            policy = Some(context.commit_prerender_response_policy(
                                page_id,
                                &scope,
                                &url,
                                native_headers,
                            )?);
                            Ok(())
                        },
                    )
                    .map_err(|error| self.response_policy_error(error))?;
                let committed =
                    context.native_prerender_scope(page_id, &token, (generation, epoch))?;
                Ok(ContextServiceResponse::PrerenderResponseCommitted {
                    policy: policy.expect("successful native response commit"),
                    origin: committed.origin,
                })
            }
            ContextServiceRequest::CommitNavigationResponsePolicy {
                page_id,
                token,
                document_url,
                headers,
            } => {
                self.context.ensure_page_admitted()?;
                if page_id != self.network.page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "worker cannot commit another page's response policy".into(),
                    ));
                }
                let context = self
                    .network
                    .context_owner()
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let mut state = self.network.authority.state.lock().map_err(|_| {
                    RuntimeError::ContextService("worker route lock poisoned".into())
                })?;
                // Keep revocation serialized through native policy publication.
                // An admitted request alone does not prevent route retirement.
                state
                    .response_policies
                    .commit(
                        page_id,
                        &token,
                        &document_url,
                        &headers,
                        |source, url, values| {
                            context.commit_navigation_response_policy(page_id, source, url, values)
                        },
                    )
                    .map_err(|error| self.response_policy_error(error))?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::DiscardNavigationResponsePolicy { page_id, token } => {
                self.context.ensure_page_admitted()?;
                if page_id != self.network.page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "worker cannot discard another page's response policy".into(),
                    ));
                }
                self.network
                    .authority
                    .state
                    .lock()
                    .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?
                    .response_policies
                    .discard(&token);
                Ok(ContextServiceResponse::NavigationResponsePolicyDiscarded)
            }
            other => ContextTransport::request(&*self.context, other),
        }
    }
}

enum WorkerRouteState {
    Bound(BoundWorkerRoute),
    Draining,
    Retiring(BoundWorkerRoute),
}

const MAX_PROCESS_ENVIRONMENTS: usize = 256;

#[derive(Default)]
struct WorkerRoutes {
    process_session: Option<u64>,
    environments: BTreeMap<u64, WorkerRouteState>,
    closing: bool,
    script_cleanup: BTreeMap<crate::worker_route::WorkerRoute, RouteScriptCleanup>,
}

// Killing the child closes its broker pipe, so the broker thread and the
// initiating request can both retire this router. Only one drains at a time;
// followers wait for that owner, rather than treating `closing` as settlement.
#[derive(Default)]
struct RouterDrainOwner {
    busy: Mutex<bool>,
    settled: Condvar,
}
struct RouterDrainGuard<'a>(&'a RouterDrainOwner);
impl RouterDrainOwner {
    fn acquire(&self, timeout: Duration) -> Result<RouterDrainGuard<'_>, RuntimeError> {
        let busy = self
            .busy
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker drain ownership poisoned".into()))?;
        let (mut busy, _) = self
            .settled
            .wait_timeout_while(busy, timeout, |busy| *busy)
            .map_err(|_| RuntimeError::ContextService("worker drain ownership poisoned".into()))?;
        if *busy {
            return Err(RuntimeError::ContextService(
                "worker drain owner remains pending".into(),
            ));
        }
        *busy = true;
        Ok(RouterDrainGuard(self))
    }
}
impl Drop for RouterDrainGuard<'_> {
    fn drop(&mut self) {
        *self
            .0
            .busy
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
        self.0.settled.notify_all();
    }
}

#[derive(Default)]
struct WorkerBrokerRouter {
    route: Mutex<WorkerRoutes>,
    drain_owner: RouterDrainOwner,
}

impl WorkerBrokerRouter {
    fn script_cleanup_registry(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        page_id: oryn_common::v3::PageId,
    ) -> Result<crate::script_load_lease::ScriptLoadRegistry, RuntimeError> {
        let identity = identity.ok_or_else(|| {
            RuntimeError::ProtocolViolation("missing script cleanup route".into())
        })?;
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        let (owner, registry, _, _, _) = routes.script_cleanup.get(identity).ok_or_else(|| {
            RuntimeError::ProtocolViolation("unknown script cleanup route".into())
        })?;
        if *owner != page_id {
            return Err(RuntimeError::ProtocolViolation(
                "script cleanup page owner mismatch".into(),
            ));
        }
        Ok(registry.clone())
    }

    fn network_cleanup_registry(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        page_id: oryn_common::v3::PageId,
    ) -> Result<Arc<crate::context_service::SocketLifecycle>, RuntimeError> {
        let identity = identity.ok_or_else(|| {
            RuntimeError::ProtocolViolation("missing network cleanup route".into())
        })?;
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router poisoned".into()))?;
        let (page, _, scope, _, _) = routes.script_cleanup.get(identity).ok_or_else(|| {
            RuntimeError::ProtocolViolation("unknown network cleanup route".into())
        })?;
        if *page != page_id {
            return Err(RuntimeError::ProtocolViolation(
                "network cleanup page owner mismatch".into(),
            ));
        }
        Ok(scope.clone())
    }

    fn navigation_cleanup(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        page: PageId,
    ) -> Result<Option<Arc<crate::navigation_control::NavigationControl>>, RuntimeError> {
        let identity = identity.ok_or_else(|| {
            RuntimeError::ProtocolViolation("missing navigation cleanup route".into())
        })?;
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router poisoned".into()))?;
        let (owner, _, _, control, _) = routes.script_cleanup.get(identity).ok_or_else(|| {
            RuntimeError::ProtocolViolation("unknown navigation cleanup route".into())
        })?;
        if *owner != page {
            return Err(RuntimeError::ProtocolViolation(
                "navigation cleanup page mismatch".into(),
            ));
        }
        Ok(control.clone())
    }
    fn navigation_resource_cleanup(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        page: PageId,
    ) -> Result<crate::context_service::NavigationResourceCleanup, RuntimeError> {
        let identity = identity.ok_or_else(|| {
            RuntimeError::ProtocolViolation("missing navigation resource cleanup route".into())
        })?;
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router poisoned".into()))?;
        let (owner, _, _, _, scope) = routes.script_cleanup.get(identity).ok_or_else(|| {
            RuntimeError::ProtocolViolation("unknown navigation resource cleanup route".into())
        })?;
        if *owner != page {
            return Err(RuntimeError::ProtocolViolation(
                "navigation resource cleanup page mismatch".into(),
            ));
        }
        Ok(scope.clone())
    }
    fn refresh_navigation_cleanup(
        &self,
        identity: &crate::worker_route::WorkerRoute,
    ) -> Result<(), RuntimeError> {
        let route = self.authorized(Some(identity))?;
        route.context.admit_registered_page()?;
        let control = route.context.navigation_cleanup_control()?;
        let mut routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router poisoned".into()))?;
        let (_, _, _, saved, _) = routes.script_cleanup.get_mut(identity).ok_or_else(|| {
            RuntimeError::ProtocolViolation("navigation cleanup admission absent".into())
        })?;
        match (&*saved, &control) {
            (Some(old), Some(new)) if !Arc::ptr_eq(old, new) => {
                return Err(RuntimeError::ProtocolViolation(
                    "navigation controller replacement on same route".into(),
                ));
            }
            (Some(_), _) => {}
            (None, _) => *saved = control,
        }
        Ok(())
    }

    fn retain_script_cleanup(
        routes: &mut WorkerRoutes,
        route: &BoundWorkerRoute,
    ) -> Result<(), RuntimeError> {
        // Old grants authorize cancellation only. Retain the original registry,
        // never the context or a network authority, across generation rotation.
        if routes.script_cleanup.contains_key(&route.identity) {
            return Err(RuntimeError::ProtocolViolation(
                "script cleanup grant cannot be reused".into(),
            ));
        }
        if routes.script_cleanup.len() >= 4096 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "retired script cleanup grants".into(),
            });
        }
        routes.script_cleanup.insert(
            route.identity.clone(),
            (
                route.network.page_id,
                route.context.script_cleanup_registry(),
                route.context.network_cleanup_registry(),
                route.context.navigation_cleanup_control()?,
                route.context.navigation_resource_cleanup(),
            ),
        );
        Ok(())
    }
    fn authorized(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
    ) -> Result<BoundWorkerRoute, RuntimeError> {
        let identity = identity.ok_or_else(|| {
            RuntimeError::ProtocolViolation("missing broker route authority".into())
        })?;
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        if !routes.closing
            && let Some(WorkerRouteState::Bound(route)) =
                routes.environments.get(&identity.environment)
            && &route.identity == identity
        {
            return Ok(route.clone());
        }
        Err(RuntimeError::ProtocolViolation(
            "missing or stale broker route authority".into(),
        ))
    }

    fn bind(&self, route: BoundWorkerRoute) -> Result<(), RuntimeError> {
        if route.identity.environment != route.network.page_id.0 {
            return Err(RuntimeError::ProtocolViolation(
                "broker route environment does not match its page owner".into(),
            ));
        }
        let mut routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        if routes.closing
            || routes
                .environments
                .contains_key(&route.identity.environment)
        {
            return Err(RuntimeError::ProtocolViolation(
                "worker environment is bound or draining".into(),
            ));
        }
        if routes.environments.len() >= MAX_PROCESS_ENVIRONMENTS {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "process environments".into(),
            });
        }
        if routes
            .process_session
            .is_some_and(|session| session != route.identity.process_session)
        {
            return Err(RuntimeError::ProtocolViolation(
                "broker route belongs to another process session".into(),
            ));
        }
        route
            .network
            .ensure_bound()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        // Lazy startup and recycled workers bind after the page already exists.
        // Initial/related creation remains staged until the runtime's explicit
        // post-registration refresh. Activation itself rechecks live ownership.
        if route.context.navigation_cleanup_control()?.is_some() {
            route.context.admit_registered_page()?;
        }
        Self::retain_script_cleanup(&mut routes, &route)?;
        routes.process_session = Some(route.identity.process_session);
        routes
            .environments
            .insert(route.identity.environment, WorkerRouteState::Bound(route));
        Ok(())
    }

    fn revoke_route(previous: &BoundWorkerRoute) -> Result<(), RuntimeError> {
        let requests = previous.network.authority.revoke()?;
        previous.context.cancel_all_network_operations();
        for request in requests {
            if let Some(context) = previous.network.context.upgrade() {
                context.cancel_service_navigation(previous.network.page_id, request);
            }
            let _ = previous.network.inner.cancel_operation(request);
        }
        Ok(())
    }

    fn finish_route_drain(
        previous: BoundWorkerRoute,
        timeout: Duration,
    ) -> Result<(), RuntimeError> {
        let started = std::time::Instant::now();
        previous.network.authority.wait_drained(timeout)?;
        // An admitted operation could register resources during first cleanup.
        previous.context.cancel_all_network_operations();
        previous
            .context
            .drain_http_cleanup(timeout.saturating_sub(started.elapsed()))?;
        previous
            .network
            .inner
            .drain_owned_operations(timeout.saturating_sub(started.elapsed()))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        Ok(())
    }

    /// Close one environment without revoking sibling grants. The draining
    /// tombstone prevents reuse until every admitted operation has completed.
    fn unbind_environment(
        &self,
        identity: &crate::worker_route::WorkerRoute,
        timeout: Duration,
    ) -> Result<(), RuntimeError> {
        let previous = {
            let mut routes = self
                .route
                .lock()
                .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
            if routes.closing {
                return Err(RuntimeError::ContextService(
                    "worker process is draining".into(),
                ));
            }
            let slot = routes
                .environments
                .get_mut(&identity.environment)
                .ok_or_else(|| {
                    RuntimeError::ProtocolViolation("unknown environment grant".into())
                })?;
            let previous = match slot {
                WorkerRouteState::Bound(route) | WorkerRouteState::Retiring(route)
                    if &route.identity == identity =>
                {
                    route.clone()
                }
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "stale environment grant".into(),
                    ));
                }
            };
            // Keep exact cleanup custody on timeout. A retry must neither revive
            // the route nor lose the operations still draining under its grant.
            *slot = WorkerRouteState::Retiring(previous.clone());
            previous
        };
        Self::revoke_route(&previous)?;
        Self::finish_route_drain(previous, timeout)?;
        let mut routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        routes.environments.remove(&identity.environment);
        Ok(())
    }

    #[cfg(test)]
    fn rotate_environment(
        &self,
        previous: &crate::worker_route::WorkerRoute,
        replacement: BoundWorkerRoute,
        timeout: Duration,
    ) -> Result<(), RuntimeError> {
        if previous.process_session != replacement.identity.process_session
            || previous.environment != replacement.identity.environment
            || previous.generation >= replacement.identity.generation
        {
            return Err(RuntimeError::ProtocolViolation(
                "invalid environment grant rotation".into(),
            ));
        }
        if replacement.identity.environment != replacement.network.page_id.0 {
            return Err(RuntimeError::ProtocolViolation(
                "replacement environment owner mismatch".into(),
            ));
        }
        replacement
            .network
            .ensure_bound()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let replacement_owner = replacement
            .network
            .context_owner()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let old = {
            let mut routes = self
                .route
                .lock()
                .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
            if routes.closing {
                return Err(RuntimeError::ContextService(
                    "worker process is draining".into(),
                ));
            }
            if routes.script_cleanup.contains_key(&replacement.identity)
                || routes.script_cleanup.len() >= 4096
            {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "retired script cleanup grants".into(),
                });
            }
            let slot = routes
                .environments
                .get_mut(&previous.environment)
                .ok_or_else(|| {
                    RuntimeError::ProtocolViolation("unknown environment grant".into())
                })?;
            let WorkerRouteState::Bound(old) = slot else {
                return Err(RuntimeError::ProtocolViolation(
                    "environment is already draining".into(),
                ));
            };
            if &old.identity != previous
                || old
                    .network
                    .context_owner()
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?
                    .id()
                    != replacement_owner.id()
            {
                return Err(RuntimeError::ProtocolViolation(
                    "environment rotation owner mismatch".into(),
                ));
            }
            let WorkerRouteState::Bound(old) = std::mem::replace(slot, WorkerRouteState::Draining)
            else {
                unreachable!()
            };
            Self::retain_script_cleanup(&mut routes, &replacement)?;
            old
        };
        Self::revoke_route(&old)?;
        Self::finish_route_drain(old, timeout)?;
        let mut routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        if routes.closing
            || !matches!(
                routes.environments.get(&previous.environment),
                Some(WorkerRouteState::Draining)
            )
        {
            return Err(RuntimeError::ContextService(
                "process retired during environment rotation".into(),
            ));
        }
        routes
            .environments
            .insert(previous.environment, WorkerRouteState::Bound(replacement));
        Ok(())
    }

    fn unbind(&self) -> Result<(), RuntimeError> {
        self.unbind_with_timeout(Duration::from_secs(2))
    }

    fn unbind_with_timeout(&self, timeout: Duration) -> Result<(), RuntimeError> {
        let started = Instant::now();
        let _owner = self.drain_owner.acquire(timeout)?;
        let previous = {
            let mut routes = self
                .route
                .lock()
                .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
            if routes.closing {
                return Err(RuntimeError::ContextService(
                    "worker route is still draining".into(),
                ));
            }
            routes.closing = true;
            routes
                .environments
                .values_mut()
                .map(|slot| std::mem::replace(slot, WorkerRouteState::Draining))
                .collect::<Vec<_>>()
        };
        let mut failure = None;
        let mut draining = Vec::new();
        // Revoke every authority before waiting for any admitted operation.
        // Retained route clones cannot admit work in a later sibling while the
        // first sibling consumes the process drain deadline.
        for route in previous {
            match route {
                WorkerRouteState::Bound(route) | WorkerRouteState::Retiring(route) => {
                    if let Err(error) = Self::revoke_route(&route) {
                        failure.get_or_insert(error);
                    }
                    draining.push(route);
                }
                WorkerRouteState::Draining => {
                    failure.get_or_insert_with(|| {
                        RuntimeError::ContextService("environment is still draining".into())
                    });
                }
            }
        }
        for route in draining {
            if let Err(error) =
                Self::finish_route_drain(route, timeout.saturating_sub(started.elapsed()))
            {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let mut routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        let script_cleanup = std::mem::take(&mut routes.script_cleanup);
        *routes = WorkerRoutes {
            script_cleanup,
            ..Default::default()
        };
        Ok(())
    }

    /// Called only after verified ResetReady: the child has destroyed every
    /// realm and acknowledged its cleanup queue. Ordinary revocation cannot
    /// discard these scopes because late cleanup receipts still need them.
    fn finish_acknowledged_reset(&self) -> Result<(), RuntimeError> {
        self.finish_unbound_cleanup(false)
    }

    // The caller must own either ResetReady or a checked, reaped child. Revoking
    // routes alone does not prove that retained script/network work has stopped.
    fn finish_unbound_cleanup(&self, reaped: bool) -> Result<(), RuntimeError> {
        let scopes = {
            let routes = self
                .route
                .lock()
                .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
            if routes.closing || !routes.environments.is_empty() || routes.process_session.is_some()
            {
                return Err(RuntimeError::ProtocolViolation(
                    "terminal cleanup requires fully unbound process".into(),
                ));
            }
            routes.script_cleanup.values().cloned().collect::<Vec<_>>()
        };
        // Cancellation can call the broker and agent registries. Do not hold
        // the router mutex while waiting for their retained work to settle.
        let reaped_pages = scopes.iter().map(|(page, ..)| *page).collect::<Vec<_>>();
        for (page, registry, sockets, navigation, resources) in &scopes {
            if !sockets.is_drained()? {
                return Err(RuntimeError::ProtocolViolation(
                    "terminal cleanup has outstanding network custody".into(),
                ));
            }
            if !registry.is_drained()? {
                return Err(RuntimeError::ProtocolViolation(
                    "terminal cleanup has outstanding script authority".into(),
                ));
            }
            if let Some(control) = navigation
                && !control
                    .cleanup_is_drained()
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?
            {
                return Err(RuntimeError::ProtocolViolation(
                    "terminal cleanup has outstanding navigation authority".into(),
                ));
            }
            if reaped {
                resources.retire_reaped(&reaped_pages)?;
            }
            if !resources.is_drained()? {
                return Err(RuntimeError::ProtocolViolation(format!(
                    "terminal cleanup has outstanding navigation resources for page {}",
                    page.0,
                )));
            }
        }
        let retired = std::mem::take(
            &mut self
                .route
                .lock()
                .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?
                .script_cleanup,
        );
        drop(retired);
        Ok(())
    }

    /// Trusted local compatibility calls can select only an unambiguous root.
    /// IPC always uses `authorized` and a full native grant.
    fn current(&self) -> Result<BoundWorkerRoute, RuntimeError> {
        let routes = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        if !routes.closing
            && routes.environments.len() == 1
            && let Some(WorkerRouteState::Bound(route)) = routes.environments.values().next()
        {
            return Ok(route.clone());
        }
        Err(RuntimeError::ContextService(
            "worker has no unambiguous live page route".into(),
        ))
    }
}

enum ResponsePolicyCleanup {
    ContextLease,
    NetworkToken,
}

impl WorkerBrokerRouter {
    fn discard_navigation_response_policy(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        page_id: PageId,
        token: &str,
        source: ResponsePolicyCleanup,
    ) -> Result<(), RuntimeError> {
        // This exact route remains authorized for cleanup after retirement;
        // revocation has already cleared its grants. Never resolve a token
        // through a replacement environment's authority.
        self.navigation_resource_cleanup(identity, page_id)?;
        if let Ok(route) = self.authorized(identity) {
            let mut state =
                route.network.authority.state.lock().map_err(|_| {
                    RuntimeError::ContextService("worker route lock poisoned".into())
                })?;
            match source {
                ResponsePolicyCleanup::ContextLease => state.response_policies.discard(token),
                ResponsePolicyCleanup::NetworkToken => {
                    state.response_policies.discard_network_token(token)
                }
            }
        }
        Ok(())
    }
}

impl NetworkBroker for WorkerBrokerRouter {
    fn discard_response_policy_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        let identity = identity
            .ok_or_else(|| NetworkError::Transport("missing response policy route".into()))?;
        self.discard_navigation_response_policy(
            Some(identity),
            PageId(identity.environment),
            token,
            ResponsePolicyCleanup::NetworkToken,
        )
        .map_err(|error| NetworkError::Transport(error.to_string()))
    }

    fn start_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        let route = self
            .authorized(identity)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route
            .context
            .ensure_page_admitted()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route.network.start_operation(request)
    }
    fn cancel_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        self.authorized(identity)
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .cancel_operation(request_id)
    }
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let route = self
            .current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route
            .context
            .ensure_page_admitted()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route.network.send(request)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let route = self
            .current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route
            .context
            .ensure_page_admitted()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        route.network.start_operation(request)
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .cancel_operation(request_id)
    }
}

impl NetworkBroker for Arc<WorkerBrokerRouter> {
    fn discard_response_policy_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        NetworkBroker::discard_response_policy_routed(&**self, route, token)
    }
    fn start_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        NetworkBroker::start_routed(&**self, route, request)
    }
    fn cancel_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        NetworkBroker::cancel_routed(&**self, route, request_id)
    }
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        NetworkBroker::send(&**self, request)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        NetworkBroker::start_operation(&**self, request)
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        NetworkBroker::cancel_operation(&**self, request_id)
    }
}

impl ContextTransport for WorkerBrokerRouter {
    fn response_scope(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request: Option<&crate::context_service::ContextServiceRequest>,
    ) -> Result<Option<crate::context_service::ResponseScope>, RuntimeError> {
        if let Some(
            ContextServiceRequest::AcknowledgeBlankPrerender { page_id, .. }
            | ContextServiceRequest::ActivateBlankPrerender { page_id, .. }
            | ContextServiceRequest::PublishBlankPrerender { page_id, .. }
            | ContextServiceRequest::CancelBlankPrerender { page_id, .. }
            | ContextServiceRequest::CancelPrerender { page_id, .. }
            | ContextServiceRequest::ParserEncodingPolicy {
                page_id,
                action: crate::context_service::ParserEncodingPolicyAction::Release,
                ..
            }
            | ContextServiceRequest::ReleaseFrameResponseStream { page_id, .. }
            | ContextServiceRequest::DiscardNavigationNetworkSource { page_id, .. }
            | ContextServiceRequest::DiscardNavigationResponsePolicy { page_id, .. }
            | ContextServiceRequest::DiscardPreparedLocalNavigation { page_id, .. },
        ) = request
        {
            self.navigation_resource_cleanup(identity, *page_id)?;
            return Ok(None);
        }
        if let Some(ContextServiceRequest::NavigationEvent {
            page_id,
            event: crate::navigation_control::NavigationEvent::Stopped(_),
            ..
        }) = request
        {
            self.navigation_cleanup(identity, *page_id)?;
            return Ok(None);
        }
        if let Some(
            ContextServiceRequest::ReleaseOrdinaryKeepalive { page_id, .. }
            | ContextServiceRequest::ReleaseNetworkResult { page_id, .. }
            | ContextServiceRequest::HttpRequestAbort { page_id, .. },
        ) = request
        {
            self.network_cleanup_registry(identity, *page_id)?;
            return Ok(None);
        }
        if let Some(ContextServiceRequest::CancelScriptLoad { page_id, .. }) = request {
            self.script_cleanup_registry(identity, *page_id)?;
            return Ok(None);
        }
        let route = self.authorized(identity)?;
        ContextTransport::response_scope(&*route.context, None, request)
    }
    fn request_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        if let ContextServiceRequest::DiscardNavigationNetworkSource { page_id, nonce } = &request {
            // Cleanup authenticates the retired identity, never a replacement route.
            self.navigation_resource_cleanup(identity, *page_id)?;
            if let Ok(route) = self.authorized(identity) {
                route
                    .network
                    .authority
                    .state
                    .lock()
                    .map_err(|_| RuntimeError::ContextService("worker route lock poisoned".into()))?
                    .navigation_sources
                    .discard(nonce);
            }
            return Ok(ContextServiceResponse::NavigationNetworkSourceDiscarded);
        }
        if let ContextServiceRequest::DiscardNavigationResponsePolicy { page_id, token } = &request
        {
            self.discard_navigation_response_policy(
                identity,
                *page_id,
                token,
                ResponsePolicyCleanup::ContextLease,
            )?;
            return Ok(ContextServiceResponse::NavigationResponsePolicyDiscarded);
        }
        if let ContextServiceRequest::AcknowledgeBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            return self
                .navigation_resource_cleanup(identity, *page_id)?
                .acknowledge_blank_prerender(activation);
        }
        if let ContextServiceRequest::ActivateBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            return self
                .navigation_resource_cleanup(identity, *page_id)?
                .activate_blank_prerender(activation);
        }
        if let ContextServiceRequest::PublishBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            return self
                .navigation_resource_cleanup(identity, *page_id)?
                .publish_blank_prerender(activation);
        }
        if let ContextServiceRequest::CancelBlankPrerender {
            page_id,
            destination,
            token,
            generation,
            epoch,
        } = &request
        {
            self.navigation_resource_cleanup(identity, *page_id)?
                .cancel_blank_prerender(*destination, token, *generation, *epoch)?;
            return Ok(ContextServiceResponse::PrerenderCancelled);
        }
        if let ContextServiceRequest::CancelPrerender {
            page_id,
            token,
            generation,
            epoch,
        } = &request
        {
            self.navigation_resource_cleanup(identity, *page_id)?
                .cancel_prerender(token, *generation, *epoch)?;
            return Ok(ContextServiceResponse::PrerenderCancelled);
        }
        if let ContextServiceRequest::ParserEncodingPolicy {
            page_id,
            creator,
            token,
            action: crate::context_service::ParserEncodingPolicyAction::Release,
        } = &request
        {
            self.navigation_resource_cleanup(identity, *page_id)?
                .release_parser_encoding(creator, token)?;
            return Ok(ContextServiceResponse::DocumentSecurityInstalled);
        }
        if let ContextServiceRequest::ReleaseFrameResponseStream {
            page_id,
            stream_id,
            policy,
        } = &request
        {
            self.navigation_resource_cleanup(identity, *page_id)?
                .release_frame(*stream_id, policy)?;
            return Ok(ContextServiceResponse::NetworkResultReleased);
        }
        if let ContextServiceRequest::DiscardPreparedLocalNavigation { page_id, token } = &request {
            self.navigation_resource_cleanup(identity, *page_id)?
                .discard_local(token)?;
            return Ok(ContextServiceResponse::LocalNavigationDiscarded);
        }
        if let ContextServiceRequest::NavigationEvent {
            page_id,
            id,
            event: crate::navigation_control::NavigationEvent::Stopped(reason),
        } = &request
        {
            let result = match self.navigation_cleanup(identity, *page_id)? {
                Some(control) => control.active(*id).and_then(|job| job.stop(reason.clone())),
                None => Err(crate::navigation_control::NavigationStop::Retired),
            };
            return Ok(ContextServiceResponse::NavigationControl { result });
        }
        if let ContextServiceRequest::ReleaseOrdinaryKeepalive { page_id, .. } = &request {
            self.network_cleanup_registry(identity, *page_id)?;
            if let Ok(route) = self.authorized(identity) {
                return ContextTransport::request(&*route.context, request);
            }
            // Revocation clears the old route's receipt table and marks every
            // queued native response abandoned; never address a replacement.
            return Ok(ContextServiceResponse::OrdinaryKeepaliveReleased);
        }
        if let ContextServiceRequest::HttpRequestAbort {
            page_id,
            request_id,
        } = &request
        {
            self.network_cleanup_registry(identity, *page_id)?
                .abort_request(*request_id)?;
            return Ok(ContextServiceResponse::HttpRequestAborted);
        }
        if let ContextServiceRequest::ReleaseNetworkResult {
            page_id,
            creator,
            stream_id,
            socket_id,
        } = &request
        {
            self.network_cleanup_registry(identity, *page_id)?
                .release_result(creator.as_ref(), *stream_id, *socket_id)?;
            return Ok(ContextServiceResponse::NetworkResultReleased);
        }
        if let ContextServiceRequest::CancelScriptLoad { page_id, token } = &request {
            self.script_cleanup_registry(identity, *page_id)?
                .cancel(*token)?;
            return Ok(ContextServiceResponse::ScriptLoadCancelled);
        }
        let route = self.authorized(identity)?;
        let _admission = route
            .network
            .authority
            .admit(None)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        route.context_request(request)
    }
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let route = self.current()?;
        let _admission = route
            .network
            .authority
            .admit(None)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        route.context_request(request)
    }
}

const BROKER_FD: RawFd = 3;
const CONTEXT_FD: RawFd = 4;
const EXPECTED_BUNDLE_ID: &str = "ai.rustic.oryn.page-worker";
const EXPECTED_TEAM_ID: &str = "5HVA9VFF8K";
const EXPECTED_SIGNING_CERT_SHA1: &str = "9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90";
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_session_id(counter: &AtomicU64) -> Result<u64, RuntimeError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| RuntimeError::ProtocolViolation("worker session identifiers exhausted".into()))
}
static WORKER_VERIFICATION_CACHE: OnceLock<Mutex<HashMap<WorkerFileIdentity, WorkerVerification>>> =
    OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkerFileIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
    manifest_sha256: String,
}

fn worker_file_identity(
    path: PathBuf,
    manifest_bytes: &[u8],
) -> Result<WorkerFileIdentity, RuntimeError> {
    let metadata =
        fs::metadata(&path).map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    Ok(WorkerFileIdentity {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
        manifest_sha256: format!("{:x}", Sha256::digest(manifest_bytes)),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerVerification {
    pub path: PathBuf,
    pub bundle_id: String,
    pub team_id: String,
    pub signing_cert_sha1: String,
    pub binary_sha256: String,
    pub manifest_sha256: String,
    pub app_sandbox: bool,
    pub allow_jit: bool,
    pub hardened_runtime: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkerManifest {
    render_contract_version: u16,
    local_effects_contract_version: u16,
    schema_version: u32,
    bundle_id: String,
    team_id: String,
    signing_cert_sha1: String,
    protocol_version: u32,
    context_service_protocol_version: u32,
    agent_ipc_protocol_version: u32,
    v8_version: String,
    v8_archive_sha256: String,
    v8_patch_chain_sha256: String,
    v8_profile_sha256: String,
    worker_build_sha256: String,
    worker_build_bytes: u64,
    entitlement_sha256: String,
    policy_sha256: String,
    source_tree_sha256: String,
    limits: WorkerLimits,
    style_engine_version: String,
    box_engine_version: String,
    inline_engine_version: String,
    embedded_font_sha256: String,
}
fn runtime_routing_protocols_match(runtime: &RuntimeBuildInfo) -> bool {
    runtime.context_service_protocol_version
        == Some(crate::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION)
        && runtime.agent_ipc_protocol_version == Some(crate::agent_ipc::VERSION)
}
fn worker_render_contract_matches(version: Option<u16>) -> bool {
    // A V8-free parent validates the worker's wire protocol even though its
    // local RuntimeBuildInfo correctly reports no embedded renderer.
    version == Some(oryn_common::v4::RENDER_CONTRACT_VERSION)
}
impl WorkerManifest {
    fn render_contract_matches(&self) -> bool {
        worker_render_contract_matches(Some(self.render_contract_version))
    }

    fn protocols_match(&self) -> bool {
        self.protocol_version == WORKER_PROTOCOL_VERSION
            && self.context_service_protocol_version
                == crate::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION
            && self.agent_ipc_protocol_version == crate::agent_ipc::VERSION
    }
}

/// A page route holds a reference to the process dispatcher, never its pipes.
/// All exchanges, including initialization and reset, serialize through session.
pub struct WorkerClient {
    pending_initialization: Option<PendingEnvironment>,
    actor_route: crate::worker_route::WorkerRoute,
    attached: bool,
    route_identity: crate::worker_route::WorkerRoute,
    session: Arc<Mutex<WorkerProcessSession>>,
    session_id: u64,
    limits: WorkerLimits,
    runtime: RuntimeBuildInfo,
    binary_sha256: String,
    verification: Option<WorkerVerification>,
    router: Arc<WorkerBrokerRouter>,
    leases: usize,
    code_directory_hash: Option<String>,
}

struct PendingEnvironment {
    config: BrowserConfig,
    context: ContextOptions,
    page_id: PageId,
    opener_page_id: Option<PageId>,
    context_id: u64,
}

/// Published before a command enters the dispatcher. Parent popup admission
/// must not relock that command's WorkerPage client or its process stream.
pub(crate) struct ReservedWorkerRoot {
    pub client: WorkerClient,
    pub attachment: Arc<WorkerAttachment>,
    pub admission: crate::context_service::RelatedRootAdmission,
    pub process_id: u32,
}

pub(crate) struct WorkerAttachment {
    template: WorkerClient,
    process_id: u32,
}
impl WorkerAttachment {
    pub(crate) fn refresh_navigation_cleanup(&self) -> Result<(), RuntimeError> {
        self.template
            .router
            .refresh_navigation_cleanup(&self.template.route_identity)
    }

    pub(crate) fn reserve(
        &self,
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener: PageId,
        owner: BrowserContext,
    ) -> Result<ReservedWorkerRoot, RuntimeError> {
        let source = self
            .template
            .router
            .authorized(Some(&self.template.route_identity))?;
        if source
            .network
            .context_owner()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?
            .id()
            != owner.id()
            || source.network.page_id != opener
            || page_id == opener
            || WorkerLimits::from(config) != self.template.limits
            || self.template.runtime.policy_digest.as_deref()
                != Some(crate::worker::policy_digest(context).as_str())
        {
            return Err(RuntimeError::ProtocolViolation(
                "related root placement does not match native owner".into(),
            ));
        }
        let mut child = self.template.attachment_template();
        let context_id = owner.id();
        child.bind_environment(config, page_id, owner, self.process_id)?;
        child.pending_initialization = Some(PendingEnvironment {
            config: config.clone(),
            context: context.clone(),
            page_id,
            opener_page_id: Some(opener),
            context_id,
        });
        let admission = child.related_root_admission();
        let attachment = Arc::new(WorkerAttachment {
            template: child.attachment_template(),
            process_id: self.process_id,
        });
        Ok(ReservedWorkerRoot {
            client: child,
            attachment,
            admission,
            process_id: self.process_id,
        })
    }
}

/// Sole owner of a worker process and its request/response stream. Page handles
/// cannot consume each other's responses or allocate conflicting message IDs.
struct WorkerProcessSession {
    process_observation: Option<crate::worker_lifecycle_audit::ProcessObservation>,
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    response_buffer: Vec<u8>,
    session_id: u64,
    next_message_id: u64,
    limits: WorkerLimits,
    router: Arc<WorkerBrokerRouter>,
    rss_monitor: Arc<RssMonitor>,
}

/// Own the OS child from spawn until the session takes over. Every `?` during
/// attestation, pipe setup, or broker creation must retain checked cleanup.
struct LaunchingWorker {
    child: Option<Child>,
    observation: Option<crate::worker_lifecycle_audit::ProcessObservation>,
    monitor: Option<Arc<RssMonitor>>,
    router: Option<Arc<WorkerBrokerRouter>>,
}
impl std::ops::Deref for LaunchingWorker {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child
            .as_ref()
            .expect("launch child already transferred")
    }
}
impl std::ops::DerefMut for LaunchingWorker {
    fn deref_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("launch child already transferred")
    }
}
impl LaunchingWorker {
    fn into_parts(
        mut self,
    ) -> (
        Child,
        Option<crate::worker_lifecycle_audit::ProcessObservation>,
    ) {
        (
            self.child.take().expect("launch child already transferred"),
            self.observation.take(),
        )
    }
}
impl Drop for LaunchingWorker {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        if let Some(monitor) = &self.monitor {
            monitor.stop.store(true, Ordering::Release);
        }
        if let Some(observation) = &self.observation {
            observation.sample_retirement();
        }
        let _ = child.kill();
        if let Ok(status) = child.wait()
            && let Some(observation) = &self.observation
        {
            observation.reaped(status);
        }
        if let Some(router) = &self.router {
            let _ = router.unbind();
        }
    }
}

struct RssMonitor {
    resident_bytes: AtomicUsize,
    failed: AtomicBool,
    stop: AtomicBool,
}

impl RssMonitor {
    fn start(pid: u32) -> Result<Arc<Self>, RuntimeError> {
        let resident = resident_bytes(pid)?;
        let monitor = Arc::new(Self {
            resident_bytes: AtomicUsize::new(resident),
            failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let sampler = monitor.clone();
        std::thread::Builder::new()
            .name("oryn-worker-rss-monitor".into())
            .spawn(move || {
                while !sampler.stop.load(Ordering::Acquire) {
                    match resident_bytes(pid) {
                        Ok(bytes) => sampler.resident_bytes.store(bytes, Ordering::Release),
                        Err(_) => {
                            sampler.failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        Ok(monitor)
    }
}

fn request_timeout(request: &WorkerRequest, limits: &WorkerLimits) -> Duration {
    match request {
        WorkerRequest::ControlledNavigate { .. }
        | WorkerRequest::Goto { .. }
        | WorkerRequest::GotoResource { .. }
        | WorkerRequest::Back
        | WorkerRequest::Forward
        | WorkerRequest::Refresh
        | WorkerRequest::LoadHtml { .. } => Duration::from_millis(limits.navigation_wall_ms),
        WorkerRequest::Oil { input } => {
            // OIL is a bounded batch transport. Individual navigation,
            // JavaScript, and action operations retain their worker-side 30s
            // and 10s limits; the outer IPC watchdog must allow each command
            // to reach those checks instead of charging the whole batch as one
            // action. Queue capacity supplies a hard upper bound even when an
            // input contains arbitrarily many lines.
            let commands = input
                .lines()
                .filter(|line| {
                    let line = line.trim();
                    !line.is_empty() && !line.starts_with('#')
                })
                .count()
                .clamp(1, limits.queue_capacity.max(1));
            Duration::from_millis(limits.navigation_wall_ms.saturating_mul(commands as u64))
        }
        WorkerRequest::ExecuteBatch { actions } => Duration::from_millis(
            limits
                .command_wall_ms
                .saturating_mul(actions.len().clamp(1, limits.queue_capacity.max(1)) as u64),
        ),
        _ => Duration::from_millis(limits.command_wall_ms),
    }
}

/// These operations have native cancellation/deadline controllers. Their IPC
/// response can legitimately wait beyond the generic transport budget (including
/// a suspended modal or an explicitly null script/page-load timeout). The worker
/// still has RSS, framing, process-exit and native execution watchdog checks.
fn native_owns_response_deadline(request: &WorkerRequest) -> bool {
    match request {
        WorkerRequest::ControlledNavigate { .. }
        | WorkerRequest::WebdriverScript { .. }
        | WorkerRequest::WebdriverActions { .. } => true,
        WorkerRequest::Scoped { request, .. } => native_owns_response_deadline(request),
        _ => false,
    }
}

/// Fixed labels only: request payloads may contain credentials and must never
/// be included in timeout diagnostics, even for initialization or OIL batches.
fn request_diagnostic_label(request: &WorkerRequest) -> &'static str {
    // A selected frame changes routing, not the operation. Unwrap without
    // formatting its URL/token so diagnostics retain the actual fixed label.
    let mut request = request;
    while let WorkerRequest::Scoped { request: inner, .. } = request {
        request = inner;
    }
    match request {
        WorkerRequest::Initialize { .. } => "initialize",
        WorkerRequest::Ping => "ping",
        WorkerRequest::ControlledNavigate { .. } => "controlled_navigation",
        WorkerRequest::Goto { .. } => "goto",
        WorkerRequest::GotoResource { .. } => "goto_resource",
        WorkerRequest::Back => "back",
        WorkerRequest::Forward => "forward",
        WorkerRequest::Refresh => "refresh",
        WorkerRequest::LoadHtml { .. } => "load_html",
        WorkerRequest::Evaluate { .. } => "evaluate",
        WorkerRequest::WebdriverScript { .. } => "webdriver_script",
        WorkerRequest::PollScript { .. } => "poll_script",
        WorkerRequest::WebdriverActions { .. } => "webdriver_actions",
        WorkerRequest::WebdriverFrames { .. } => "webdriver_frames",
        WorkerRequest::Oil { .. } => "oil",
        WorkerRequest::Reset { .. } => "reset",
        WorkerRequest::Close => "close",
        WorkerRequest::Exit => "exit",
        WorkerRequest::ReleaseEnvironment => "release_environment",
        WorkerRequest::WaitFor { .. } => "wait_for",
        WorkerRequest::WaitForUntil { .. } => "wait_for_until",
        WorkerRequest::Execute { .. } => "execute",
        WorkerRequest::ExecuteBatch { .. } => "execute_batch",
        WorkerRequest::SetFiles { .. } => "set_files",
        WorkerRequest::ScriptCloseHistory => "script_close_history",
        WorkerRequest::PrepareScriptClose { .. } => "prepare_script_close",
        WorkerRequest::Capabilities => "capabilities",
        WorkerRequest::ChallengeAssessment => "challenge_assessment",
        WorkerRequest::ResolveFocusedForm => "resolve_focused_form",
        WorkerRequest::ResolveSelector { .. } => "resolve_selector",
        WorkerRequest::ResolveTarget { .. } => "resolve_target",
        WorkerRequest::ResolveSemantic { .. } => "resolve_semantic",
        WorkerRequest::Extract { .. } => "extract",
        WorkerRequest::Observe { .. } => "observe",
        WorkerRequest::ObserveWith { .. } => "observe_with",
        WorkerRequest::ObserveSemantic { .. } => "observe_semantic",
        WorkerRequest::ObserveSemanticV6 { .. } => "observe_semantic_v6",
        WorkerRequest::ObserveStyles { .. } => "observe_styles",
        WorkerRequest::ComputedStyle { .. } => "computed_style",
        WorkerRequest::AccessibilitySnapshot => "accessibility_snapshot",
        WorkerRequest::AccessibilityText { .. } => "accessibility_text",
        WorkerRequest::BoxModel { .. } => "box_model",
        WorkerRequest::PaintSnapshot => "paint_snapshot",
        WorkerRequest::PrintSnapshot { .. } => "print_snapshot",
        WorkerRequest::SetViewport { .. } => "set_viewport",
        WorkerRequest::SetMedia { .. } => "set_media",
        WorkerRequest::Scroll { .. } => "scroll",
        WorkerRequest::ScrollIntoView { .. } => "scroll_into_view",
        WorkerRequest::ObserveDelta { .. } => "observe_delta",
        WorkerRequest::ObserveSemanticDelta { .. } => "observe_semantic_delta",
        WorkerRequest::Trace => "trace",
        WorkerRequest::Scoped { .. } => unreachable!("scopes were unwrapped above"),
    }
}

fn contextualize_worker_deadline(
    mut error: RuntimeError,
    phase: &'static str,
    command: &'static str,
    message_id: u64,
) -> RuntimeError {
    let mut current = &mut error;
    while let RuntimeError::ActionExecutionFailed { cause, .. }
    | RuntimeError::BatchExecutionFailed { cause, .. }
    | RuntimeError::OilExecutionFailed { cause, .. }
    | RuntimeError::NavigationCleanupFailed { cause, .. } = current
    {
        current = cause.as_mut();
    }
    if let RuntimeError::WorkerLimitExceeded { resource } = current
        && resource.contains("wall time")
    {
        *resource =
            format!("{resource} (phase={phase}, command={command}, message_id={message_id})");
    }
    error
}

/// Extract one newline-delimited frame, checking its size before returning it.
/// The delimiter is part of the wire frame, as in the worker-side reader.
/// Coalesced frames are checked separately rather than charging later frames to
/// the first one. Rejected input remains untouched until the process is killed.
fn take_response_frame(
    buffer: &mut Vec<u8>,
    limit: usize,
) -> Result<Option<Vec<u8>>, RuntimeError> {
    if let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
        if newline >= limit {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "IPC frame".into(),
            });
        }
        return Ok(Some(buffer.drain(..=newline).collect()));
    }
    // Even one more delimiter would exceed the limit at this point.
    if buffer.len() >= limit {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "IPC frame".into(),
        });
    }
    Ok(None)
}

impl WorkerClient {
    /// Cancel a reserved child without ever entering its worker dispatcher.
    /// Keep the pending initialization owner until native route revocation ACKs.
    pub(crate) fn cancel_pending_initialization(&mut self) -> Result<bool, RuntimeError> {
        if self.pending_initialization.is_none() {
            return Ok(false);
        }
        self.router
            .unbind_environment(&self.route_identity, Duration::from_secs(2))?;
        self.pending_initialization = None;
        self.attached = false;
        Ok(true)
    }

    /// Observe only this owned child; never infer exit from a missing route or PID.
    pub(crate) fn observed_process_exit(
        &self,
    ) -> Result<Option<std::process::ExitStatus>, RuntimeError> {
        let mut session = self.session.lock().map_err(|_| {
            RuntimeError::WorkerCrashed("worker process dispatcher lock poisoned".into())
        })?;
        let status = session
            .child
            .try_wait()
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
        if let Some(status) = status
            && let Some(observation) = &session.process_observation
        {
            observation.reaped(status);
        }
        Ok(status)
    }

    pub(crate) fn checked_exit_evidence(&self) -> Result<serde_json::Value, RuntimeError> {
        use std::os::unix::process::ExitStatusExt;
        let mut session = self.session.lock().map_err(|_| {
            RuntimeError::WorkerCrashed("worker process dispatcher lock poisoned".into())
        })?;
        let pid = session.child.id();
        let status = session
            .child
            .try_wait()
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
            .ok_or_else(|| RuntimeError::WorkerCrashed("worker is still running".into()))?;
        if let Some(observation) = &session.process_observation {
            observation.reaped(status);
        }
        Ok(
            serde_json::json!({"worker_pid":pid,"reaped":true,"signal":status.signal(),"exit_code":status.code()}),
        )
    }

    /// Verify physical retirement without claiming a child-issued Closed ACK.
    /// Never kill a live process here or turn an attempted kill into evidence.
    pub(crate) fn checked_terminal_cleanup(&mut self) -> Result<serde_json::Value, RuntimeError> {
        let mut evidence = self.checked_exit_evidence()?;
        self.router.unbind()?;
        self.router.finish_unbound_cleanup(true)?;
        // The exact process is reaped and all retained routes are settled. Drop
        // must not issue another IPC operation to a dead transport.
        self.attached = false;
        self.pending_initialization = None;
        evidence["router_cleanup_complete"] = serde_json::json!(true);
        Ok(evidence)
    }

    pub(crate) fn execution_placement(
        &self,
    ) -> Option<crate::execution_placement::ExecutionPlacement> {
        let session = self.session.try_lock().ok()?;
        Some(crate::execution_placement::ExecutionPlacement {
            mode: "child_process".into(),
            process_id: session.child.id(),
            attested: !self.binary_sha256.is_empty(),
            binary_sha256: Some(self.binary_sha256.clone()),
            bootstrap_sha256: None,
        })
    }

    pub(crate) fn process_id(&self) -> u32 {
        self.session
            .lock()
            .map(|session| session.child.id())
            .unwrap_or(0)
    }

    pub fn spawn(
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        context_owner: BrowserContext,
    ) -> Result<Self, RuntimeError> {
        Self::spawn_with_role(
            config,
            context,
            page_id,
            opener_page_id,
            context_owner,
            "page",
        )
    }

    fn spawn_with_role(
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        context_owner: BrowserContext,
        role: &str,
    ) -> Result<Self, RuntimeError> {
        let session_id = allocate_session_id(&NEXT_SESSION_ID)?;
        let route_identity = crate::worker_route::WorkerRoute::issue(
            session_id,
            page_id.0,
            config.document_generation_base,
        )?;
        let timing_identity = crate::native_timing::worker_identity(
            session_id,
            context_owner.id(),
            page_id.0,
            config.document_generation_base,
            None,
            role,
        );
        let mut launch_timing =
            crate::native_timing::Scope::start("worker_launch_total", timing_identity.clone());
        let mut verification_timing = crate::native_timing::Scope::start(
            "worker_input_verification",
            timing_identity.clone(),
        );
        let worker_path = discover_worker()?;
        let verification = if requires_trusted_worker() {
            Some(verify_worker(&worker_path)?)
        } else {
            None
        };
        let launch_path = verification
            .as_ref()
            .map(|item| item.path.as_path())
            .unwrap_or(worker_path.as_path());
        #[cfg(target_os = "linux")]
        let launch_binary_sha256 = Some(sha256_file(launch_path)?);
        #[cfg(not(target_os = "linux"))]
        let launch_binary_sha256: Option<String> = None;

        verification_timing.finish(true);
        let (parent_broker, child_broker) = UnixStream::pair()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let (parent_context, child_context) = UnixStream::pair()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let child_fd = child_broker.as_raw_fd();
        let child_context_fd = child_context.as_raw_fd();
        let diagnostic_fd = crate::native_timing::inherited_fd();
        let mut command = Command::new(launch_path);
        #[cfg(target_os = "linux")]
        if is_linux_multicall_worker(launch_path) {
            command.arg("__oryn-page-worker");
        }
        if diagnostic_fd.is_some() {
            command.arg("--diagnostic-fd").arg("5");
        }
        command
            .arg("--broker-fd")
            .arg(BROKER_FD.to_string())
            .arg("--context-fd")
            .arg(CONTEXT_FD.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env_clear();
        if let Some(binary_sha256) = verification
            .as_ref()
            .map(|item| item.binary_sha256.as_str())
            .or(launch_binary_sha256.as_deref())
        {
            command.env("ORYN_TRUSTED_WORKER_SHA256", binary_sha256);
        }
        // SAFETY: this closure runs after fork and before exec, and only
        // duplicates an already-open socket descriptor to the agreed slot.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(child_fd, BROKER_FD) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(child_context_fd, CONTEXT_FD) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if let Some(fd) = diagnostic_fd {
                    // Validated retained source is >=64; it cannot alias protocol slots.
                    if libc::dup2(fd, 5) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut spawn_timing =
            crate::native_timing::Scope::start("worker_os_spawn", timing_identity.clone());
        let launch_started_unix_ns = crate::worker_lifecycle_audit::now();
        let child = command
            .spawn()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        spawn_timing.finish(true);
        let timing_identity = crate::native_timing::worker_identity(
            session_id,
            context_owner.id(),
            page_id.0,
            config.document_generation_base,
            Some(child.id()),
            role,
        );
        launch_timing.set_identity(timing_identity.clone());
        crate::native_timing::point("worker_spawned", timing_identity.clone());
        let process_observation = crate::worker_lifecycle_audit::spawned(
            launch_started_unix_ns,
            session_id,
            child.id(),
            context_owner.id(),
            page_id.0,
            config.document_generation_base,
            verification
                .as_ref()
                .map(|value| value.binary_sha256.clone())
                .or(launch_binary_sha256.clone()),
        );
        if let Some(observation) = &process_observation {
            observation.role(role);
        }
        let mut child = LaunchingWorker {
            child: Some(child),
            observation: process_observation,
            monitor: None,
            router: None,
        };
        let mut running_identity_timing = crate::native_timing::Scope::start(
            "worker_running_identity_verification",
            timing_identity.clone(),
        );
        #[cfg(target_os = "macos")]
        verify_running_worker(child.id(), launch_path)?;
        #[cfg(target_os = "macos")]
        let launch_code_directory_hash = Some(running_code_directory_hash(child.id())?);
        #[cfg(not(target_os = "macos"))]
        let launch_code_directory_hash: Option<String> = None;
        running_identity_timing.finish(true);
        let rss_monitor = RssMonitor::start(child.id())?;
        child.monitor = Some(rss_monitor.clone());
        drop(child_broker);
        drop(child_context);

        let parent_network = context_owner.shared_network_broker().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent network broker is unavailable".into())
        })?;
        let parent_network = parent_network.for_worker(child.id()).for_page(page_id.0);
        let broker_network = PageNetworkBroker {
            authority: Arc::new(RouteAuthority::default()),
            inner: parent_network
                .clone()
                .for_page(page_id.0)
                .new_request_scope(),
            context: context_owner.downgrade(),
            page_id,
        };
        let router = Arc::new(WorkerBrokerRouter::default());
        child.router = Some(router.clone());
        router.bind(BoundWorkerRoute {
            identity: route_identity.clone(),
            network: broker_network,
            context: ParentContextService::new_staged_with_network(
                context_owner,
                page_id,
                Some(parent_network),
            )
            .into_shared(),
        })?;
        let frame_limit = config.max_ipc_frame_bytes;
        let network_router = router.clone();
        std::thread::Builder::new()
            .name("oryn-worker-network-broker".into())
            .spawn(move || {
                let _ = serve_ipc_broker(parent_broker, network_router.clone(), frame_limit);
                // Losing the network pipe revokes every response grant from
                // that process, including headers never delivered to a caller.
                let _ = network_router.unbind();
            })
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        std::thread::Builder::new()
            .name("oryn-worker-context-service".into())
            .spawn({
                let context_router = router.clone();
                move || {
                    let transport: Arc<dyn ContextTransport> = context_router;
                    let _ = serve_context_transport(parent_context, transport);
                }
            })
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;

        let input = child.stdin.take().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("worker stdin was not created".into())
        })?;
        set_nonblocking(input.as_raw_fd())
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let output = child.stdout.take().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("worker stdout was not created".into())
        })?;
        let limits = WorkerLimits::from(config);
        let nonce = handshake_nonce(session_id);
        let (child, process_observation) = child.into_parts();
        let mut client = Self {
            pending_initialization: None,
            actor_route: route_identity.clone(),
            attached: true,
            route_identity,
            session: Arc::new(Mutex::new(WorkerProcessSession {
                process_observation,
                child,
                input,
                output,
                response_buffer: Vec::new(),
                session_id,
                next_message_id: 1,
                limits: limits.clone(),
                router: router.clone(),
                rss_monitor,
            })),
            session_id,
            limits: limits.clone(),
            runtime: crate::runtime_build_info(),
            binary_sha256: String::new(),
            verification,
            router,
            leases: 1,
            code_directory_hash: launch_code_directory_hash.clone(),
        };
        let mut ready_timing = crate::native_timing::Scope::start(
            "worker_initialize_and_validate",
            timing_identity.clone(),
        );
        let response = client.request_with_timeout(
            WorkerRequest::Initialize {
                related_actor: None,
                nonce: nonce.clone(),
                limits: limits.clone(),
                context: Box::new(context.clone()),
                page_id,
                opener_page_id,
                document_generation_base: config.document_generation_base,
            },
            config.command_timeout,
        )?;
        let WorkerResponse::Ready {
            nonce: returned_nonce,
            protocol_version,
            runtime,
            binary_sha256,
            policy_digest,
            sandbox_state,
            secret_environment_present,
            limits: effective_limits,
            code_identity,
        } = response
        else {
            client.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker did not return an initialization response".into(),
            ));
        };
        let runtime = *runtime;
        let verified_hash = client
            .verification
            .as_ref()
            .map(|item| item.binary_sha256.as_str())
            .or(launch_binary_sha256.as_deref())
            .unwrap_or(binary_sha256.as_str());
        if returned_nonce != nonce
            || protocol_version != WORKER_PROTOCOL_VERSION
            || !runtime_routing_protocols_match(&runtime)
            || !runtime.native_v8
            || runtime.v8_version.is_none()
            || !worker_render_contract_matches(runtime.render_contract_version)
            || runtime.local_effects_contract_version
                != oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION
            || runtime.contract_version != oryn_common::v4::CONTRACT_VERSION
            || runtime.runtime_profile != config.runtime_profile.capability_name()
            || runtime.binding_abi_version != oryn_js::BINDING_ABI_VERSION
            || runtime.v8_patch_chain_sha256.as_deref() != option_env!("ORYN_V8_PATCH_CHAIN_SHA256")
            || runtime.v8_archive_sha256.as_deref() != option_env!("ORYN_V8_ARCHIVE_SHA256")
            || !runtime.v8_heap_sandbox
            || runtime.historical_test_input != cfg!(feature = "historical-test-input")
            || runtime.transport_mode != "async_streaming_parent_broker"
            || !runtime.streaming_parser
            || runtime.semantic_graph_version != crate::semantic::SEMANTIC_GRAPH_VERSION
            || !runtime.capabilities.iter().any(|item| item == "layout_box")
            || runtime.style_engine_version.as_deref() != Some(EXPECTED_STYLE_ENGINE_VERSION)
            || runtime.box_engine_version.as_deref() != Some(EXPECTED_BOX_ENGINE_VERSION)
            || runtime.inline_engine_version.as_deref() != Some(EXPECTED_INLINE_ENGINE_VERSION)
            || runtime.embedded_font_sha256.as_deref()
                != Some("b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5")
            || effective_limits != limits
            || binary_sha256 != verified_hash
            || code_identity.process_id != client.process_id()
            || code_identity.binary_sha256 != binary_sha256
            || code_identity.code_directory_hash != launch_code_directory_hash
            || policy_digest != crate::worker::policy_digest(context)
            || sandbox_state
                != if cfg!(target_os = "macos") {
                    "app_sandbox"
                } else if cfg!(target_os = "linux") {
                    "linux_namespaces_landlock_seccomp"
                } else {
                    "process_isolated_unsandboxed"
                }
            || secret_environment_present
        {
            client.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker handshake metadata did not match the verified launch".into(),
            ));
        }
        if requires_trusted_worker() && runtime.build_profile != "release" {
            client.kill();
            return Err(RuntimeError::SandboxUnavailable(
                "production worker is not a release build".into(),
            ));
        }
        client.runtime = runtime;
        client.runtime.worker_lifecycle = config.worker_lifecycle.name().into();
        client.runtime.worker_code_directory_hash = client.code_directory_hash.clone();
        client.runtime.policy_digest = Some(policy_digest);
        client.runtime.sandbox_state = sandbox_state;
        if let Ok(session) = client.session.lock()
            && let Some(observation) = &session.process_observation
        {
            observation.binary(&binary_sha256);
        }
        client.binary_sha256 = binary_sha256;
        ready_timing.finish(true);
        crate::native_timing::point("worker_ready_validated", timing_identity);
        launch_timing.finish(true);
        Ok(client)
    }

    pub fn request(&mut self, request: WorkerRequest) -> Result<WorkerResponse, RuntimeError> {
        if let Some(pending) = self.pending_initialization.take() {
            self.complete_environment_initialization(
                &pending.config,
                &pending.context,
                pending.page_id,
                pending.opener_page_id,
                pending.context_id,
            )?;
        }
        let timeout = request_timeout(&request, &self.limits);
        let allows_pending = match &request {
            WorkerRequest::WebdriverScript { request, .. } => request.asynchronous,
            WorkerRequest::Scoped { request, .. } => matches!(request.as_ref(),
                WorkerRequest::WebdriverScript { request, .. } if request.asynchronous),
            _ => false,
        };
        let mut response = self.request_with_timeout(request, timeout)?;
        let mut expected_ticket = None;
        while let WorkerResponse::ScriptPending { ticket } = response {
            if !allows_pending || ticket == 0 || expected_ticket.is_some_and(|expected| expected != ticket) {
                self.kill();
                return Err(RuntimeError::ProtocolViolation("unexpected pending script ticket".into()));
            }
            expected_ticket = Some(ticket);
            // Each correlated exchange releases the shared session lock. The
            // native script deadline remains in the actor, including null.
            std::thread::sleep(Duration::from_millis(10));
            response = self.request_with_timeout(WorkerRequest::PollScript { ticket }, timeout)?;
        }
        Ok(response)
    }

    /// Shorten only the next test caller's transport budgets after real startup.
    /// Child configuration and production defaults remain unchanged.
    #[cfg(test)]
    pub(crate) fn set_response_budget_for_test(&mut self, budget: Duration) {
        let millis = u64::try_from(budget.as_millis()).expect("test response budget");
        assert!(millis > 0);
        self.limits.command_wall_ms = millis;
        self.limits.navigation_wall_ms = millis;
    }

    pub(crate) fn has_sibling_environments(&self) -> bool {
        self.router
            .route
            .lock()
            .map(|routes| {
                routes
                    .environments
                    .values()
                    .filter(|state| matches!(state, WorkerRouteState::Bound(_)))
                    .count()
                    > 1
            })
            .unwrap_or(true)
    }

    pub fn runtime(&self) -> &RuntimeBuildInfo {
        &self.runtime
    }

    pub fn verification(&self) -> Option<&WorkerVerification> {
        self.verification.as_ref()
    }

    pub(crate) fn reset(&mut self) -> Result<(), RuntimeError> {
        if Arc::strong_count(&self.session) != 1 {
            return Err(RuntimeError::ProtocolViolation(
                "cannot reset a process with surviving page handles".into(),
            ));
        }
        // The entire transition is unbound. Teardown cannot acquire fresh
        // authority, even while an old broker thread retains a route clone.
        if let Err(error) = self.router.unbind() {
            self.kill();
            return Err(error);
        }
        let nonce = handshake_nonce(self.session_id);
        let response = self.request(WorkerRequest::Reset {
            nonce: nonce.clone(),
        })?;
        let WorkerResponse::ResetReady {
            nonce: returned_nonce,
            code_identity,
        } = response
        else {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker did not acknowledge reset".into(),
            ));
        };
        if returned_nonce != nonce
            || code_identity.process_id != self.process_id()
            || code_identity.binary_sha256 != self.binary_sha256
            || code_identity.code_directory_hash != self.code_directory_hash
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker identity changed during reset".into(),
            ));
        }
        if let Err(error) = self.router.finish_acknowledged_reset() {
            self.kill();
            return Err(error);
        }
        self.session_id = match allocate_session_id(&NEXT_SESSION_ID) {
            Ok(id) => id,
            Err(error) => {
                self.kill();
                return Err(error);
            }
        };
        let mut session = self.session.lock().map_err(|_| {
            RuntimeError::WorkerCrashed("worker process dispatcher lock poisoned".into())
        })?;
        if !session.response_buffer.is_empty() {
            session.kill();
            return Err(RuntimeError::ProtocolViolation(
                "unexpected worker output after reset".into(),
            ));
        }
        session.session_id = self.session_id;
        session.next_message_id = 1;
        if let Some(observation) = &session.process_observation {
            observation.end_leases(None);
        }
        Ok(())
    }

    pub(crate) fn rebind(
        &mut self,
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        context_owner: BrowserContext,
    ) -> Result<(), RuntimeError> {
        if Arc::strong_count(&self.session) != 1 {
            return Err(RuntimeError::ProtocolViolation(
                "cannot rebind a process with surviving page handles".into(),
            ));
        }
        self.initialize_environment(config, context, page_id, opener_page_id, context_owner)
    }

    fn attachment_template(&self) -> Self {
        Self {
            pending_initialization: None,
            actor_route: self.actor_route.clone(),
            attached: false,
            route_identity: self.route_identity.clone(),
            session: self.session.clone(),
            session_id: self.session_id,
            limits: self.limits.clone(),
            runtime: self.runtime.clone(),
            binary_sha256: self.binary_sha256.clone(),
            verification: self.verification.clone(),
            router: self.router.clone(),
            leases: 0,
            code_directory_hash: self.code_directory_hash.clone(),
        }
    }
    pub(crate) fn attachment(&self) -> WorkerAttachment {
        WorkerAttachment {
            template: self.attachment_template(),
            process_id: self.process_id(),
        }
    }
    pub(crate) fn related_root_admission(&self) -> crate::context_service::RelatedRootAdmission {
        crate::context_service::RelatedRootAdmission {
            direct: false,
            route: self.route_identity.clone(),
            actor_route: self.actor_route.clone(),
            policy: None,
            isolation: Default::default(),
        }
    }

    /// Parent-only attachment for a direct related window in the same context.
    /// Placement selects this explicitly; independent roots cannot enter through
    /// this method merely by supplying a public page identifier.
    #[cfg(test)]
    pub(crate) fn attach_environment(
        &self,
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: PageId,
        context_owner: BrowserContext,
    ) -> Result<Self, RuntimeError> {
        let source = self.router.authorized(Some(&self.route_identity))?;
        let owner = source
            .network
            .context_owner()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if owner.id() != context_owner.id()
            || opener_page_id != source.network.page_id
            || page_id == source.network.page_id
            || WorkerLimits::from(config) != self.limits
            || self.runtime.policy_digest.as_deref()
                != Some(crate::worker::policy_digest(context).as_str())
        {
            return Err(RuntimeError::ProtocolViolation(
                "related environment placement does not match its native owner".into(),
            ));
        }
        let mut attached = Self {
            pending_initialization: None,
            actor_route: self.actor_route.clone(),
            attached: false,
            route_identity: self.route_identity.clone(),
            session: self.session.clone(),
            session_id: self.session_id,
            limits: self.limits.clone(),
            runtime: self.runtime.clone(),
            binary_sha256: self.binary_sha256.clone(),
            verification: self.verification.clone(),
            router: self.router.clone(),
            leases: 0,
            code_directory_hash: self.code_directory_hash.clone(),
        };
        let context_id = context_owner.id();
        attached.bind_environment(config, page_id, context_owner, self.process_id())?;
        attached.complete_environment_initialization(
            config,
            context,
            page_id,
            Some(opener_page_id),
            context_id,
        )?;
        Ok(attached)
    }

    fn initialize_environment(
        &mut self,
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        context_owner: BrowserContext,
    ) -> Result<(), RuntimeError> {
        let context_id = context_owner.id();
        self.bind_environment(config, page_id, context_owner, self.process_id())?;
        self.actor_route = self.route_identity.clone();
        self.complete_environment_initialization(
            config,
            context,
            page_id,
            opener_page_id,
            context_id,
        )
    }

    fn bind_environment(
        &mut self,
        config: &BrowserConfig,
        page_id: PageId,
        context_owner: BrowserContext,
        process_id: u32,
    ) -> Result<(), RuntimeError> {
        let identity = crate::worker_route::WorkerRoute::issue(
            self.session_id,
            page_id.0,
            config.document_generation_base,
        )?;
        let parent_network = context_owner.shared_network_broker().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent network broker is unavailable".into())
        })?;
        let parent_network = parent_network.for_worker(process_id).for_page(page_id.0);
        self.router.bind(BoundWorkerRoute {
            identity: identity.clone(),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner: parent_network
                    .clone()
                    .for_page(page_id.0)
                    .new_request_scope(),
                context: context_owner.downgrade(),
                page_id,
            },
            context: ParentContextService::new_staged_with_network(
                context_owner,
                page_id,
                Some(parent_network),
            )
            .into_shared(),
        })?;
        self.route_identity = identity;
        self.attached = true;
        Ok(())
    }

    fn complete_environment_initialization(
        &mut self,
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        audit_context_id: u64,
    ) -> Result<(), RuntimeError> {
        let timing_identity = crate::native_timing::worker_identity(
            self.session_id,
            audit_context_id,
            page_id.0,
            config.document_generation_base,
            Some(self.process_id()),
            "reused_environment",
        );
        let mut timing = crate::native_timing::Scope::start(
            "worker_environment_initialize_and_validate",
            timing_identity.clone(),
        );
        crate::ipc_progress::bind_context(
            &self.route_identity,
            audit_context_id,
            self.process_id(),
        );
        let nonce = handshake_nonce(self.session_id);
        let result = self.request_with_timeout(
            WorkerRequest::Initialize {
                related_actor: (self.actor_route != self.route_identity)
                    .then(|| self.actor_route.clone()),
                nonce: nonce.clone(),
                limits: self.limits.clone(),
                context: Box::new(context.clone()),
                page_id,
                opener_page_id,
                document_generation_base: config.document_generation_base,
            },
            config.command_timeout,
        );
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                let _ = self.router.unbind();
                self.kill();
                return Err(error);
            }
        };
        let WorkerResponse::Ready {
            nonce: returned_nonce,
            protocol_version,
            runtime,
            binary_sha256,
            policy_digest,
            sandbox_state,
            secret_environment_present,
            limits,
            code_identity,
        } = response
        else {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker environment did not return an initialization response".into(),
            ));
        };
        if returned_nonce != nonce
            || protocol_version != WORKER_PROTOCOL_VERSION
            || !runtime_routing_protocols_match(&runtime)
            || !runtime.native_v8
            || runtime.v8_version != self.runtime.v8_version
            || runtime.render_contract_version != self.runtime.render_contract_version
            || runtime.local_effects_contract_version != self.runtime.local_effects_contract_version
            || runtime.contract_version != self.runtime.contract_version
            || runtime.binding_abi_version != self.runtime.binding_abi_version
            || runtime.v8_archive_sha256 != self.runtime.v8_archive_sha256
            || runtime.v8_patch_chain_sha256 != self.runtime.v8_patch_chain_sha256
            || runtime.v8_heap_sandbox != self.runtime.v8_heap_sandbox
            || runtime.historical_test_input != self.runtime.historical_test_input
            || runtime.semantic_graph_version != self.runtime.semantic_graph_version
            || runtime.style_engine_version != self.runtime.style_engine_version
            || runtime.box_engine_version != self.runtime.box_engine_version
            || runtime.inline_engine_version != self.runtime.inline_engine_version
            || runtime.embedded_font_sha256 != self.runtime.embedded_font_sha256
            || binary_sha256 != self.binary_sha256
            || policy_digest != crate::worker::policy_digest(context)
            || sandbox_state != self.runtime.sandbox_state
            || secret_environment_present
            || limits != self.limits
            || code_identity.process_id != self.process_id()
            || code_identity.binary_sha256 != self.binary_sha256
            || code_identity.code_directory_hash != self.code_directory_hash
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker environment handshake metadata changed".into(),
            ));
        }
        self.runtime.policy_digest = Some(policy_digest);
        self.runtime.worker_lifecycle = config.worker_lifecycle.name().into();
        self.leases = self.leases.saturating_add(1);
        if let Ok(session) = self.session.lock()
            && let Some(observation) = &session.process_observation
        {
            observation.begin_lease(audit_context_id, page_id.0, config.document_generation_base);
        }
        timing.finish(true);
        crate::native_timing::point("worker_environment_ready_validated", timing_identity);
        Ok(())
    }

    pub(crate) fn leases(&self) -> usize {
        self.leases
    }

    pub(crate) fn mark_prepared(&mut self) {
        self.leases = 0;
        if let Ok(session) = self.session.lock()
            && let Some(observation) = &session.process_observation
        {
            observation.role("prewarmed");
            observation.end_leases(None);
        }
    }

    fn request_with_timeout(
        &mut self,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let mut session = self.session.lock().map_err(|_| {
            RuntimeError::WorkerCrashed("worker process dispatcher lock poisoned".into())
        })?;
        session.request_with_timeout(&self.route_identity, request, timeout)
    }

    fn kill(&mut self) {
        // Recover a poisoned owner only to revoke authority and reap it.
        self.session
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .kill();
    }

    /// Explicit qualification fault: retain the owned Child through signal and
    /// wait so the evidence identifies the process actually terminated.
    pub(crate) fn terminate_for_qualification(
        &mut self,
    ) -> Result<serde_json::Value, RuntimeError> {
        use std::os::unix::process::ExitStatusExt;
        let mut session = self.session.lock().map_err(|_| {
            RuntimeError::WorkerCrashed("worker process dispatcher lock poisoned".into())
        })?;
        if let Some(status) = session
            .child
            .try_wait()
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
        {
            if let Some(observation) = &session.process_observation {
                observation.reaped(status);
            }
            return Err(RuntimeError::WorkerCrashed(
                "qualification target already exited".into(),
            ));
        }
        let pid = session.child.id();
        session.rss_monitor.stop.store(true, Ordering::Release);
        if let Some(observation) = &session.process_observation {
            observation.sample_retirement();
        }
        session
            .child
            .kill()
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
        let status = session
            .child
            .wait()
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
        if let Some(observation) = &session.process_observation {
            observation.reaped(status);
        }
        let _ = session.router.unbind();
        if status.signal() != Some(libc::SIGKILL) {
            return Err(RuntimeError::WorkerCrashed(
                "qualification target did not exit from SIGKILL".into(),
            ));
        }
        Ok(serde_json::json!({"worker_pid":pid,"reaped":true,"signal":libc::SIGKILL}))
    }
}

impl WorkerProcessSession {
    fn request_with_timeout(
        &mut self,
        route: &crate::worker_route::WorkerRoute,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let progress_scope = crate::ipc_progress::request(route, self.next_message_id, &request);
        let progress = progress_scope.identity();
        let mut timing = crate::native_timing::Scope::start(
            "native_parent_request_exchange",
            serde_json::to_value(progress).unwrap_or(serde_json::Value::Null),
        );
        let result = self.exchange_with_timeout(route, request, timeout, progress);
        timing.finish(result.is_ok());
        // Enforce retirement here, not only in WorkerPage. Initialization,
        // reset, and direct callers must also discard malformed or failed
        // transports, including JSON parse errors and unexpected EOF.
        if matches!(
            &result,
            Err(RuntimeError::ProtocolViolation(_)
                | RuntimeError::WorkerCrashed(_)
                | RuntimeError::WorkerLimitExceeded { .. })
        ) {
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::TransportRetirementBegin,
                None,
                None,
            );
            self.kill();
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::TransportRetirementReturned,
                None,
                None,
            );
        }
        result
    }

    fn exchange_with_timeout(
        &mut self,
        route: &crate::worker_route::WorkerRoute,
        request: WorkerRequest,
        timeout: Duration,
        progress: Option<crate::ipc_progress::Identity>,
    ) -> Result<WorkerResponse, RuntimeError> {
        if route.process_session != self.session_id {
            return Err(RuntimeError::ProtocolViolation(
                "route belongs to another process session".into(),
            ));
        }
        let command = request_diagnostic_label(&request);
        let response_timeout = if native_owns_response_deadline(&request) {
            None
        } else {
            Some(timeout)
        };
        let message_id = self.next_message_id;
        self.next_message_id = match self.next_message_id.checked_add(1) {
            Some(id) => id,
            None => {
                self.kill();
                return Err(RuntimeError::ProtocolViolation(
                    "worker message identifiers exhausted".into(),
                ));
            }
        };
        crate::ipc_progress::record(
            progress,
            crate::ipc_progress::Phase::RequestSend,
            None,
            None,
        );
        let envelope = HostEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            route: Some(route.clone()),
            session_id: self.session_id,
            message_id,
            command: request,
        };
        let encoded = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if encoded.len() >= self.limits.ipc_frame_bytes {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "IPC frame".into(),
            });
        }
        self.write_frame(&encoded, timeout).map_err(|error| {
            contextualize_worker_deadline(error, "ipc_write", command, message_id)
        })?;

        crate::ipc_progress::record(
            progress,
            crate::ipc_progress::Phase::RequestWritten,
            None,
            None,
        );
        let response_started = Instant::now();
        let response = self.read_frame(response_timeout).map_err(|error| {
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::ResponseReadFailed,
                None,
                None,
            );
            contextualize_worker_deadline(error, "ipc_response", command, message_id)
        })?;
        crate::ipc_progress::record(
            progress,
            crate::ipc_progress::Phase::ResponseRead,
            None,
            None,
        );
        let response: WorkerEnvelope = serde_json::from_slice(&response)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != WORKER_PROTOCOL_VERSION
            || response.route.as_ref() != Some(route)
            || response.session_id != self.session_id
            || response.correlation_id != message_id
            || response.message_id != message_id
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker response correlation mismatch".into(),
            ));
        }
        match response.response {
            WorkerResponse::Error { error } => Err(contextualize_worker_deadline(
                *error,
                "worker_execution",
                command,
                message_id,
            )),
            WorkerResponse::LimitExceeded { resource } => {
                self.kill();
                Err(contextualize_worker_deadline(
                    RuntimeError::WorkerLimitExceeded { resource },
                    "worker_execution",
                    command,
                    message_id,
                ))
            }
            WorkerResponse::Closed if command == "close" => {
                // The child has released its actor, but HttpRequestAbort only
                // schedules parent cancellation. Keep the exact route's cleanup
                // receipt inside this exchange and its existing response budget.
                let scope = self
                    .router
                    .network_cleanup_registry(Some(route), PageId(route.environment))?;
                let remaining = timeout.saturating_sub(response_started.elapsed());
                if !scope.drain_cancelled_http(remaining)? {
                    return Err(contextualize_worker_deadline(
                        RuntimeError::WorkerLimitExceeded {
                            resource: "IPC response wall time awaiting HTTP cancellation".into(),
                        },
                        "ipc_response",
                        command,
                        message_id,
                    ));
                }
                Ok(WorkerResponse::Closed)
            }
            response => Ok(response),
        }
    }

    fn write_frame(&mut self, encoded: &[u8], timeout: Duration) -> Result<(), RuntimeError> {
        let mut frame = Vec::with_capacity(encoded.len() + 1);
        frame.extend_from_slice(encoded);
        frame.push(b'\n');
        let started = Instant::now();
        let mut written = 0;
        while written < frame.len() {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
            {
                if let Some(observation) = &self.process_observation {
                    observation.reaped(status);
                }
                return Err(RuntimeError::WorkerCrashed(format!(
                    "worker exited with {status}"
                )));
            }
            self.check_rss()?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "IPC write wall time".into(),
                });
            }
            let poll_ms = remaining.min(Duration::from_millis(250)).as_millis() as i32;
            let mut descriptor = libc::pollfd {
                fd: self.input.as_raw_fd(),
                events: libc::POLLOUT | libc::POLLHUP,
                revents: 0,
            };
            // SAFETY: descriptor points to one initialized pollfd and stdin is
            // kept alive by self.input for the duration of the call.
            let result = unsafe { libc::poll(&mut descriptor, 1, poll_ms) };
            if result > 0 {
                // SAFETY: frame is a live byte slice and the descriptor is a
                // nonblocking pipe owned by self.input.
                let count = unsafe {
                    libc::write(
                        self.input.as_raw_fd(),
                        frame[written..].as_ptr().cast(),
                        frame.len() - written,
                    )
                };
                if count > 0 {
                    written += count as usize;
                    continue;
                }
                if count == 0 {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(
                        "worker stopped accepting requests".into(),
                    ));
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted
                    && error.kind() != std::io::ErrorKind::WouldBlock
                {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            } else if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    self.kill();
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
        }
        Ok(())
    }

    fn read_frame(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, RuntimeError> {
        let started = Instant::now();
        loop {
            if let Some(frame) =
                take_response_frame(&mut self.response_buffer, self.limits.ipc_frame_bytes)?
            {
                return Ok(frame);
            }
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?
            {
                if let Some(observation) = &self.process_observation {
                    observation.reaped(status);
                }
                return Err(RuntimeError::WorkerCrashed(format!(
                    "worker exited with {status}"
                )));
            }
            self.check_rss()?;
            let remaining = timeout.map(|budget| budget.saturating_sub(started.elapsed()));
            if remaining.is_some_and(|remaining| remaining.is_zero()) {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let poll_ms = remaining
                .unwrap_or(Duration::from_millis(10))
                .min(Duration::from_millis(10))
                .as_millis() as i32;
            let mut descriptor = libc::pollfd {
                fd: self.output.as_raw_fd(),
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            // SAFETY: descriptor points to one initialized pollfd and the
            // child stdout descriptor remains owned by self.output.
            let result = unsafe { libc::poll(&mut descriptor, 1, poll_ms) };
            if result > 0 {
                let mut chunk = [0_u8; 8192];
                // SAFETY: chunk is a live writable byte buffer and the
                // descriptor remains owned by self.output.
                let read = unsafe {
                    libc::read(
                        self.output.as_raw_fd(),
                        chunk.as_mut_ptr().cast(),
                        chunk.len(),
                    )
                };
                if read > 0 {
                    self.response_buffer
                        .extend_from_slice(&chunk[..read as usize]);
                    continue;
                }
                if read == 0 {
                    return Err(RuntimeError::WorkerCrashed(
                        "worker closed its response stream".into(),
                    ));
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted
                    && error.kind() != std::io::ErrorKind::WouldBlock
                {
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(RuntimeError::WorkerCrashed(error.to_string()));
                }
            }
        }
    }

    fn kill(&mut self) {
        self.rss_monitor.stop.store(true, Ordering::Release);
        if let Some(observation) = &self.process_observation {
            observation.sample_retirement();
        }
        let _ = self.child.kill();
        if let Ok(status) = self.child.wait()
            && let Some(observation) = &self.process_observation
        {
            observation.reaped(status);
        }
        let _ = self.router.unbind();
    }

    fn check_rss(&mut self) -> Result<(), RuntimeError> {
        if self.rss_monitor.failed.load(Ordering::Acquire) {
            self.kill();
            return Err(RuntimeError::WorkerCrashed(
                "worker RSS measurement failed".into(),
            ));
        }
        if self.rss_monitor.resident_bytes.load(Ordering::Acquire) > self.limits.worker_rss_bytes {
            self.kill();
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "RSS".into(),
            });
        }
        Ok(())
    }
}

pub fn runtime_report(
    config: &BrowserConfig,
    context: &ContextOptions,
) -> Result<RuntimeBuildInfo, RuntimeError> {
    let client = WorkerClient::spawn_with_role(
        config,
        context,
        PageId::LEGACY,
        None,
        crate::Browser::in_process_with_config(config.clone())
            .new_context_with_options(context.clone()),
        "verification",
    )?;
    let mut runtime = client.runtime().clone();
    if let Ok(session) = client.session.lock()
        && let Some(observation) = &session.process_observation
    {
        observation.role("verification");
    }
    runtime.execution_mode = if client.verification().is_some() || cfg!(target_os = "linux") {
        "sandboxed_worker".into()
    } else {
        "process_isolated_unsandboxed".into()
    };
    runtime.sandboxed = client.verification().is_some() || cfg!(target_os = "linux");
    runtime.sandbox_state = if cfg!(target_os = "macos") && runtime.sandboxed {
        "app_sandbox".into()
    } else if cfg!(target_os = "linux") {
        "linux_namespaces_landlock_seccomp".into()
    } else {
        "process_isolated_unsandboxed".into()
    };
    runtime.worker_protocol_version = Some(WORKER_PROTOCOL_VERSION);
    runtime.worker_binary_sha256 = Some(client.binary_sha256.clone());
    runtime.worker_code_directory_hash = client.code_directory_hash.clone();
    runtime.worker_bundle_id = client.verification().map(|item| item.bundle_id.clone());
    runtime.worker_team_id = client.verification().map(|item| item.team_id.clone());
    runtime.worker_signing_cert_sha1 = client
        .verification()
        .map(|item| item.signing_cert_sha1.clone());
    runtime.limits = Some(client.limits.clone());
    Ok(runtime)
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        if !self.attached {
            return;
        }
        if self.pending_initialization.take().is_some() {
            // No worker session exists yet. Revoke only this reservation; do
            // not issue a reentrant ReleaseEnvironment against the busy actor.
            let _ = self
                .router
                .unbind_environment(&self.route_identity, Duration::from_secs(2));
            return;
        }
        // A page handle disappearing must not terminate sibling environments.
        // The last process owner attempts orderly exit; session Drop always reaps.
        if !self.has_sibling_environments() {
            let _ = self.request_with_timeout(WorkerRequest::Exit, Duration::from_secs(1));
        } else {
            let released = self
                .request_with_timeout(WorkerRequest::ReleaseEnvironment, Duration::from_secs(1));
            if matches!(released, Ok(WorkerResponse::EnvironmentReleased)) {
                let _ = self
                    .router
                    .unbind_environment(&self.route_identity, Duration::from_secs(2));
                if let Ok(session) = self.session.lock()
                    && let Some(observation) = &session.process_observation
                {
                    observation.end_leases(Some(self.route_identity.environment));
                }
            }
        }
    }
}

impl Drop for WorkerProcessSession {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(all(target_os = "linux", not(debug_assertions)))]
pub fn discover_worker() -> Result<PathBuf, RuntimeError> {
    // A production Linux package is a single immutable multicall ELF. Ignore
    // environment substitution and always re-exec the current executable.
    std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))
}

#[cfg(any(not(target_os = "linux"), debug_assertions))]
pub fn discover_worker() -> Result<PathBuf, RuntimeError> {
    if let Some(path) = std::env::var_os("ORYN_PAGE_WORKER") {
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?
        .canonicalize()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    #[cfg(target_os = "linux")]
    {
        return Ok(executable);
    }
    #[cfg(not(target_os = "linux"))]
    let parent = executable.parent().map(Path::to_path_buf).ok_or_else(|| {
        RuntimeError::SandboxUnavailable("parent executable has no directory".into())
    })?;
    #[cfg(target_os = "macos")]
    {
        let bundled = parent
            .parent()
            .filter(|contents| {
                contents.file_name().and_then(|name| name.to_str()) == Some("Contents")
            })
            .map(|contents| {
                contents.join("Helpers/OrynPageWorker.app/Contents/MacOS/oryn-page-worker")
            });
        if let Some(worker) = bundled.filter(|path| path.is_file()) {
            return Ok(worker);
        }
        Ok(parent.join("OrynPageWorker.app/Contents/MacOS/oryn-page-worker"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(parent.join("oryn-page-worker"))
    }
}

#[cfg(target_os = "linux")]
fn is_linux_multicall_worker(path: &Path) -> bool {
    let Ok(current) = std::env::current_exe().and_then(fs::canonicalize) else {
        return false;
    };
    fs::canonicalize(path).is_ok_and(|candidate| candidate == current)
}

pub fn verify_worker(path: &Path) -> Result<WorkerVerification, RuntimeError> {
    if !path.is_file() {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker does not exist: {}",
            path.display()
        )));
    }
    let path = fs::canonicalize(path)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let app = containing_app(&path)?;
    let manifest_path = app.join("Contents/Resources/worker-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let manifest: WorkerManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    // Verify the sealed profile even on a cached executable verification. Its
    // contents must not be swapped while retaining the manifest/binary pair.
    if !manifest.protocols_match() {
        return Err(RuntimeError::SandboxUnavailable(
            "signed worker/context/agent protocol mismatch".into(),
        ));
    }
    let profile_path = app.join("Contents/Resources/v8-profile-manifest.json");
    if sha256_file(&profile_path)? != manifest.v8_profile_sha256 {
        return Err(RuntimeError::SandboxUnavailable(
            "signed V8 profile digest mismatch".into(),
        ));
    }
    let identity = worker_file_identity(path.clone(), &manifest_bytes)?;
    let cache = WORKER_VERIFICATION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().map_err(|_| {
        RuntimeError::SandboxUnavailable("worker verification cache poisoned".into())
    })?;
    if let Some(verification) = cache.get(&identity) {
        return Ok(verification.clone());
    }
    // Discard stale entries for an earlier file at the same canonical path.
    cache.retain(|key, _| key.path != path);
    #[cfg(target_os = "macos")]
    verify_static_worker(&app)?;
    let bundle_id = EXPECTED_BUNDLE_ID.to_string();
    let team_id = EXPECTED_TEAM_ID.to_string();
    let signing_cert_sha1 = EXPECTED_SIGNING_CERT_SHA1.to_string();

    let binary_sha256 = sha256_file(&path)?;
    if manifest.schema_version != 1
        || manifest.bundle_id != EXPECTED_BUNDLE_ID
        || manifest.team_id != EXPECTED_TEAM_ID
        || manifest.signing_cert_sha1 != EXPECTED_SIGNING_CERT_SHA1
        || !manifest.render_contract_matches()
        || manifest.local_effects_contract_version
            != oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION
        || !manifest.protocols_match()
        || manifest.limits != WorkerLimits::default()
        || manifest.v8_version.is_empty()
        || Some(manifest.v8_archive_sha256.as_str()) != option_env!("ORYN_V8_ARCHIVE_SHA256")
        || Some(manifest.v8_patch_chain_sha256.as_str())
            != option_env!("ORYN_V8_PATCH_CHAIN_SHA256")
        || manifest.v8_profile_sha256.len() != 64
        || manifest.worker_build_sha256.len() != 64
        || manifest.worker_build_bytes == 0
        || manifest.entitlement_sha256 != expected_entitlement_sha256()
        || manifest.policy_sha256 != compiled_policy_sha256()
        || manifest.source_tree_sha256.len() != 64
        || manifest.style_engine_version != EXPECTED_STYLE_ENGINE_VERSION
        || manifest.box_engine_version != EXPECTED_BOX_ENGINE_VERSION
        || manifest.inline_engine_version != EXPECTED_INLINE_ENGINE_VERSION
        || manifest.embedded_font_sha256
            != "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5"
    {
        return Err(RuntimeError::SandboxUnavailable(
            "signed worker manifest does not match the executable".into(),
        ));
    }
    let verification = WorkerVerification {
        path,
        bundle_id,
        team_id,
        signing_cert_sha1,
        binary_sha256,
        manifest_sha256: identity.manifest_sha256.clone(),
        app_sandbox: true,
        allow_jit: true,
        hardened_runtime: true,
    };
    cache.insert(identity, verification.clone());
    Ok(verification)
}

pub(crate) fn prepare_worker_verification() -> Result<(), RuntimeError> {
    let worker_path = discover_worker()?;
    if requires_trusted_worker() {
        verify_worker(&worker_path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn worker_requirement()
-> Result<security_framework::os::macos::code_signing::SecRequirement, RuntimeError> {
    use security_framework::os::macos::code_signing::SecRequirement;
    use std::str::FromStr;

    SecRequirement::from_str(&format!(
        "identifier \"{EXPECTED_BUNDLE_ID}\" and anchor apple generic and certificate leaf = H\"{EXPECTED_SIGNING_CERT_SHA1}\" and certificate leaf[subject.OU] = \"{EXPECTED_TEAM_ID}\" and entitlement[\"com.apple.security.app-sandbox\"] exists and entitlement[\"com.apple.security.cs.allow-jit\"] exists and ! entitlement[\"com.apple.security.network.client\"] exists and ! entitlement[\"com.apple.security.network.server\"] exists and ! entitlement[\"com.apple.security.cs.allow-unsigned-executable-memory\"] exists and ! entitlement[\"com.apple.security.cs.allow-dyld-environment-variables\"] exists and ! entitlement[\"com.apple.security.cs.disable-library-validation\"] exists and ! entitlement[\"com.apple.security.cs.disable-executable-page-protection\"] exists and ! entitlement[\"com.apple.security.get-task-allow\"] exists"
    ))
    .map_err(|error| RuntimeError::SandboxUnavailable(format!(
        "invalid worker code requirement: {error}"
    )))
}

#[cfg(target_os = "macos")]
fn verify_static_worker(app: &Path) -> Result<(), RuntimeError> {
    use core_foundation::url::CFURL;
    use security_framework::os::macos::code_signing::{Flags, SecStaticCode};

    let url = CFURL::from_path(app, true).ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker bundle path is not a valid file URL".into())
    })?;
    let code = SecStaticCode::from_path(&url, Flags::NONE)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    code.check_validity(
        Flags::CHECK_ALL_ARCHITECTURES
            | Flags::CHECK_NESTED_CODE
            | Flags::STRICT_VALIDATE
            | Flags::NO_NETWORK_ACCESS,
        &worker_requirement()?,
    )
    .map_err(|error| {
        RuntimeError::SandboxUnavailable(format!(
            "worker signature or code requirement failed: {error}"
        ))
    })
}

#[cfg(target_os = "macos")]
fn verify_running_worker(pid: u32, expected_path: &Path) -> Result<(), RuntimeError> {
    use security_framework::os::macos::code_signing::{Flags, GuestAttributes, SecCode};

    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid.cast_signed());
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE) {
            Ok(code) => {
                code.check_validity(
                    Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
                    &worker_requirement()?,
                )
                .map_err(|error| {
                    RuntimeError::SandboxUnavailable(format!(
                        "launched worker failed dynamic code validation: {error}"
                    ))
                })?;
                let actual_path = code
                    .path(Flags::NONE)
                    .ok()
                    .and_then(|url| url.to_path())
                    .and_then(|path| fs::canonicalize(path).ok());
                let expected_bundle = containing_app(expected_path).ok();
                if actual_path.as_deref() != Some(expected_path)
                    && actual_path.as_ref() != expected_bundle.as_ref()
                {
                    return Err(RuntimeError::SandboxUnavailable(
                        "launched worker path does not match the verified executable".into(),
                    ));
                }
                return Ok(());
            }
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                std::thread::yield_now();
            }
            Err(error) => {
                return Err(RuntimeError::SandboxUnavailable(format!(
                    "unable to authenticate launched worker PID: {error}"
                )));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn running_code_directory_hash(pid: u32) -> Result<String, RuntimeError> {
    const CS_OPS_CDHASH: u32 = 5;
    unsafe extern "C" {
        fn csops(
            pid: libc::pid_t,
            ops: u32,
            useraddr: *mut libc::c_void,
            usersize: libc::size_t,
        ) -> libc::c_int;
    }
    let mut hash = [0_u8; 20];
    // SAFETY: csops writes at most the supplied fixed-size CDHash buffer for
    // the already authenticated child PID.
    let status = unsafe {
        csops(
            pid.cast_signed(),
            CS_OPS_CDHASH,
            hash.as_mut_ptr().cast(),
            hash.len(),
        )
    };
    if status != 0 {
        return Err(RuntimeError::SandboxUnavailable(
            "unable to read launched worker Code Directory hash".into(),
        ));
    }
    Ok(hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn containing_app(path: &Path) -> Result<PathBuf, RuntimeError> {
    let macos = path.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    let contents = macos.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    let app = contents.parent().ok_or_else(|| {
        RuntimeError::SandboxUnavailable("worker is not inside an app bundle".into())
    })?;
    if path.file_name().and_then(|name| name.to_str()) != Some("oryn-page-worker")
        || macos.file_name().and_then(|name| name.to_str()) != Some("MacOS")
        || contents.file_name().and_then(|name| name.to_str()) != Some("Contents")
        || app.extension().and_then(|value| value.to_str()) != Some("app")
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker is not at OrynPageWorker.app/Contents/MacOS/oryn-page-worker".into(),
        ));
    }
    Ok(app.to_path_buf())
}

fn requires_trusted_worker() -> bool {
    cfg!(target_os = "macos")
}

fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: fd is owned by the live ChildStdin passed by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: F_SETFL updates only descriptor status flags for the same live
    // pipe descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn expected_entitlement_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            br#"{"com.apple.security.app-sandbox":true,"com.apple.security.cs.allow-jit":true}"#
        )
    )
}

fn compiled_policy_sha256() -> String {
    let mut digest = Sha256::new();
    digest.update(b"security.rs\0");
    digest.update(include_bytes!("security.rs"));
    digest.update(b"\0network.rs\0");
    digest.update(include_bytes!("network.rs"));
    format!("{:x}", digest.finalize())
}

#[cfg(target_os = "macos")]
fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    use std::{fs::File, os::fd::AsRawFd};

    unsafe extern "C" {
        fn CC_SHA256(data: *const libc::c_void, len: u32, digest: *mut u8) -> *mut u8;
    }
    let file =
        File::open(path).map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let length = file
        .metadata()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?
        .len();
    let length_u32 = u32::try_from(length).map_err(|_| {
        RuntimeError::SandboxUnavailable("worker is too large for CommonCrypto SHA-256".into())
    })?;
    if length == 0 {
        return Ok(format!("{:x}", Sha256::digest([])));
    }
    // SAFETY: the read-only private mapping is bounded by the opened file's
    // measured length and remains live until after CommonCrypto returns.
    let mapping = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length as usize,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        )
    };
    if mapping == libc::MAP_FAILED {
        return Err(RuntimeError::SandboxUnavailable(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let mut digest = [0_u8; 32];
    // SAFETY: mapping and digest both point to valid buffers of the lengths
    // supplied to CommonCrypto.
    let result = unsafe { CC_SHA256(mapping.cast_const(), length_u32, digest.as_mut_ptr()) };
    // SAFETY: mapping was returned by mmap with exactly this length.
    unsafe { libc::munmap(mapping, length as usize) };
    if result.is_null() {
        return Err(RuntimeError::SandboxUnavailable(
            "CommonCrypto SHA-256 failed".into(),
        ));
    }
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(not(target_os = "macos"))]
fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let bytes =
        fs::read(path).map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn handshake_nonce(session_id: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{session_id}-{nanos}", std::process::id())
}

#[cfg(target_os = "macos")]
pub fn resident_bytes(pid: u32) -> Result<usize, RuntimeError> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let info_size = i32::try_from(std::mem::size_of::<libc::proc_taskinfo>())
        .expect("proc_taskinfo fits in an i32");
    // SAFETY: proc_pidinfo writes at most the supplied proc_taskinfo-sized
    // buffer. The PID belongs to the live child checked immediately before
    // this call.
    let written = unsafe {
        libc::proc_pidinfo(
            pid.cast_signed(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            info_size,
        )
    };
    if written != info_size {
        return Err(RuntimeError::WorkerCrashed(
            "unable to measure worker RSS with proc_pidinfo".into(),
        ));
    }
    // SAFETY: the kernel reported that it initialized the complete structure.
    let resident = unsafe { info.assume_init() }.pti_resident_size;
    usize::try_from(resident).map_err(|_| RuntimeError::WorkerLimitExceeded {
        resource: "RSS measurement overflow".into(),
    })
}

#[cfg(target_os = "linux")]
pub fn resident_bytes(pid: u32) -> Result<usize, RuntimeError> {
    if pid == 0 {
        return Err(RuntimeError::WorkerCrashed(
            "invalid worker PID for RSS measurement".into(),
        ));
    }
    // Read the kernel's resident-set counter directly. Spawning ps for each
    // measurement perturbs process-tree accounting and creates unrelated load.
    let status = fs::read_to_string(format!("/proc/{pid}/status")).map_err(|error| {
        RuntimeError::WorkerCrashed(format!("unable to read worker RSS: {error}"))
    })?;
    parse_linux_resident_bytes(&status)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_resident_bytes(status: &str) -> Result<usize, RuntimeError> {
    let invalid =
        || RuntimeError::WorkerCrashed("invalid or missing VmRSS in process status".into());
    let mut records = status
        .lines()
        .filter_map(|line| line.strip_prefix("VmRSS:"));
    let line = records.next().ok_or_else(invalid)?;
    if records.next().is_some() {
        return Err(invalid());
    }
    let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.len() != 2
        || fields[1] != "kB"
        || fields[0].is_empty()
        || !fields[0].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid());
    }
    let kib = fields[0].parse::<usize>().map_err(|_| invalid())?;
    kib.checked_mul(1024)
        .ok_or_else(|| RuntimeError::WorkerLimitExceeded {
            resource: "RSS measurement overflow".into(),
        })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn resident_bytes(pid: u32) -> Result<usize, RuntimeError> {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
    if !output.status.success() {
        return Err(RuntimeError::WorkerCrashed(
            "unable to measure worker RSS".into(),
        ));
    }
    let kib = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
    Ok(kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_deadline_ownership_does_not_change_legacy_transport_budgets() {
        use crate::navigation_control::{
            NavigationCommand, NavigationId, NavigationRequest, PageLoadStrategy,
        };
        for timeout_ms in [None, Some(0), Some(300_000)] {
            assert!(native_owns_response_deadline(
                &WorkerRequest::ControlledNavigate {
                    id: NavigationId(1),
                    request: NavigationRequest {
                        command: NavigationCommand::Refresh,
                        strategy: PageLoadStrategy::Normal,
                        timeout_ms,
                    },
                }
            ));
            assert!(native_owns_response_deadline(
                &WorkerRequest::WebdriverScript {
                    request: crate::host::webdriver_script::ScriptRequest {
                        body: Vec::new(),
                        args: Vec::new(),
                        asynchronous: true,
                        timeout_ms,
                    },
                    context: crate::host::webdriver_script::ScriptContext::default(),
                }
            ));
        }
        assert!(!native_owns_response_deadline(&WorkerRequest::Goto {
            url: "about:blank".into()
        }));
        assert!(!native_owns_response_deadline(&WorkerRequest::Evaluate {
            source: "1".into()
        }));
        assert!(!native_owns_response_deadline(&WorkerRequest::Close));
    }

    #[test]
    fn scoped_deadline_diagnostics_retain_operation_without_frame_payload() {
        let dom = oryn_dom::html::parse("<body></body>")
            .into_runtime()
            .unwrap();
        let scope = crate::host::webdriver_script::AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(1),
            parent_realm: Some(oryn_js::realm::RealmId(0)),
            frame: Some(dom.document()),
            frame_generation: Some(1),
            frame_token: Some("private-frame-token".into()),
            document_root: dom.document(),
            document_generation: 1,
            stream_epoch: 0,
            document_authority_epoch: 0,
            url: "https://private.invalid/?credential=secret".into(),
            live: true,
        };
        let request = WorkerRequest::Scoped {
            scope: scope.clone(),
            request: Box::new(WorkerRequest::Scoped {
                scope,
                request: Box::new(WorkerRequest::Evaluate {
                    source: "private-script".into(),
                }),
            }),
        };
        let label = request_diagnostic_label(&request);
        assert_eq!(label, "evaluate");
        let error = contextualize_worker_deadline(
            RuntimeError::WorkerLimitExceeded {
                resource: "wall time".into(),
            },
            "ipc_response",
            label,
            17,
        );
        let RuntimeError::WorkerLimitExceeded { resource } = error else {
            panic!("deadline category changed")
        };
        assert_eq!(
            resource,
            "wall time (phase=ipc_response, command=evaluate, message_id=17)"
        );
    }

    #[test]
    fn navigation_cleanup_wrapper_preserves_worker_deadline_context() {
        let secondary = RuntimeError::Script("secondary cleanup failure".into());
        let error = contextualize_worker_deadline(
            RuntimeError::NavigationCleanupFailed {
                cause: Box::new(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                }),
                cleanup: Box::new(secondary.clone()),
            },
            "worker_execution",
            "controlled_navigation",
            23,
        );
        assert!(
            matches!(error.root_cause(), RuntimeError::WorkerLimitExceeded { resource }
            if resource == "wall time (phase=worker_execution, command=controlled_navigation, message_id=23)")
        );
        let RuntimeError::NavigationCleanupFailed { cleanup, .. } = error else {
            panic!("cleanup custody changed")
        };
        assert_eq!(*cleanup, secondary);
    }

    #[test]
    fn nested_action_deadline_preserves_batch_receipts_and_terminal_cause() {
        use oryn_common::v3::{
            ActionId, ActionResult, CONTRACT_VERSION, ExecutionDomain, Revision,
        };
        let receipt = ActionResult {
            contract_version: CONTRACT_VERSION,
            action_id: ActionId(41),
            page_id: PageId(7),
            revision_before: Revision(12),
            revision_after: Revision(13),
            delta: None,
            effects: Vec::new(),
            diagnostics: vec!["completed before failure".into()],
            execution_domain: ExecutionDomain::Native,
        };
        for leaf in [
            RuntimeError::WorkerLimitExceeded {
                resource: "wall time".into(),
            },
            RuntimeError::WorkerLimitExceeded {
                resource: "RSS".into(),
            },
            RuntimeError::WorkerCrashed("worker exited".into()),
            RuntimeError::ProtocolViolation("invalid response".into()),
        ] {
            let original_leaf = leaf.clone();
            let error = RuntimeError::BatchExecutionFailed {
                completed: vec![receipt.clone()],
                failed_index: 1,
                cause: Box::new(RuntimeError::ActionExecutionFailed {
                    action_id: oryn_common::v3::ActionId(42),
                    cause: Box::new(leaf),
                }),
            };
            let contextual =
                contextualize_worker_deadline(error, "worker_execution", "execute_batch", 19);
            let encoded = serde_json::to_vec(&contextual).unwrap();
            let decoded: RuntimeError = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded, contextual);
            let RuntimeError::BatchExecutionFailed {
                completed,
                failed_index,
                cause,
            } = &decoded
            else {
                panic!("lost batch outcome");
            };
            assert_eq!(completed, std::slice::from_ref(&receipt));
            assert_eq!(*failed_index, 1);
            assert!(matches!(
                cause.as_ref(),
                RuntimeError::ActionExecutionFailed {
                    action_id: oryn_common::v3::ActionId(42),
                    ..
                }
            ));
            if matches!(&original_leaf, RuntimeError::WorkerLimitExceeded { resource } if resource == "wall time")
            {
                assert!(
                    matches!(decoded.root_cause(), RuntimeError::WorkerLimitExceeded { resource }
                    if resource == "wall time (phase=worker_execution, command=execute_batch, message_id=19)")
                );
            } else {
                assert_eq!(decoded.root_cause(), &original_leaf);
            }
        }
    }

    #[test]
    fn oil_deadline_preserves_completed_output_and_typed_cause() {
        let error = RuntimeError::OilExecutionFailed {
            completed: vec![r#"{"kind":"text","value":"completed"}"#.into()],
            failed_index: 1,
            cause: Box::new(RuntimeError::ActionExecutionFailed {
                action_id: oryn_common::v3::ActionId(42),
                cause: Box::new(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                }),
            }),
        };
        let error = contextualize_worker_deadline(error, "worker_execution", "oil", 19);
        assert!(
            matches!(&error, RuntimeError::OilExecutionFailed { completed, failed_index: 1, .. } if completed.len() == 1)
        );
        assert!(
            matches!(error.root_cause(), RuntimeError::WorkerLimitExceeded { resource }
            if resource == "wall time (phase=worker_execution, command=oil, message_id=19)")
        );
        let decoded: RuntimeError =
            serde_json::from_slice(&serde_json::to_vec(&error).unwrap()).unwrap();
        assert_eq!(decoded, error);
    }

    #[test]
    fn deadline_context_never_formats_request_payloads() {
        for (request, expected) in [
            (
                WorkerRequest::ResolveSelector {
                    selector: "[data-secret='private-token']".into(),
                    action: oryn_common::v3::SemanticAction::Click,
                },
                "resolve_selector",
            ),
            (WorkerRequest::Observe { profile: None }, "observe"),
            (
                WorkerRequest::AccessibilitySnapshot,
                "accessibility_snapshot",
            ),
            (WorkerRequest::PaintSnapshot, "paint_snapshot"),
            (WorkerRequest::Trace, "trace"),
            (
                WorkerRequest::Goto {
                    url: "https://private.invalid/?token=secret".into(),
                },
                "goto",
            ),
            (
                WorkerRequest::LoadHtml {
                    policy_token: None,
                    url: "private-url".into(),
                    html: "private-html".into(),
                },
                "load_html",
            ),
            (
                WorkerRequest::Evaluate {
                    source: "private-script".into(),
                },
                "evaluate",
            ),
            (
                WorkerRequest::Oil {
                    input: "private-oil".into(),
                },
                "oil",
            ),
            (
                WorkerRequest::Reset {
                    nonce: "private-nonce".into(),
                },
                "reset",
            ),
        ] {
            let command = request_diagnostic_label(&request);
            assert_eq!(command, expected);
            for phase in ["ipc_write", "ipc_response", "worker_execution"] {
                let error = contextualize_worker_deadline(
                    RuntimeError::WorkerLimitExceeded {
                        resource: "wall time".into(),
                    },
                    phase,
                    command,
                    7,
                );
                let RuntimeError::WorkerLimitExceeded { resource } = error else {
                    panic!("deadline must preserve the error category");
                };
                assert_eq!(
                    resource,
                    format!("wall time (phase={phase}, command={expected}, message_id=7)")
                );
            }
        }
        let error = contextualize_worker_deadline(
            RuntimeError::WorkerLimitExceeded {
                resource: "RSS".into(),
            },
            "ipc_response",
            "ping",
            7,
        );
        assert!(
            matches!(error, RuntimeError::WorkerLimitExceeded { resource } if resource == "RSS")
        );
    }

    #[test]
    fn linux_rss_parser_uses_resident_counter_with_checked_units() {
        assert_eq!(
            parse_linux_resident_bytes(
                "Name:\tworker\nVmSize:\t99999 kB\nVmRSS:\t1234 kB\nRssAnon:\t1200 kB\n"
            )
            .unwrap(),
            1234 * 1024
        );
        assert_eq!(parse_linux_resident_bytes("VmRSS: 0 kB\n").unwrap(), 0);
        for source in [
            "",
            "VmSize: 10 kB",
            "VmRSS: -1 kB",
            "VmRSS: +1 kB",
            "VmRSS: 1 MB",
            "VmRSS: 1 kB extra",
            "VmRSS: 1 kB\nVmRSS: 2 kB",
        ] {
            assert!(parse_linux_resident_bytes(source).is_err(), "{source}");
        }
        assert!(parse_linux_resident_bytes(&format!("VmRSS: {} kB", usize::MAX)).is_err());
    }

    #[test]
    fn incomplete_launch_reaps_child_and_stops_monitor() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r line"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let monitor = Arc::new(RssMonitor {
            resident_bytes: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let launching = LaunchingWorker {
            child: Some(child),
            observation: None,
            monitor: Some(monitor.clone()),
            router: None,
        };
        drop(launching);
        assert!(monitor.stop.load(Ordering::Acquire));
        let mut status = 0;
        // The launch guard has consumed the owned child's wait status already.
        assert_eq!(
            unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn completed_launch_transfers_child_without_terminating_it() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r line"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let launching = LaunchingWorker {
            child: Some(child),
            observation: None,
            monitor: None,
            router: None,
        };
        let (mut child, observation) = launching.into_parts();
        assert!(observation.is_none());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn shared_process_dispatcher_serializes_messages_and_owns_child_lifetime() {
        let route = crate::worker_route::WorkerRoute::issue(7, 1, 0).unwrap();
        let mut script = String::new();
        for message_id in 1..=32 {
            let response = serde_json::to_string(&WorkerEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                route: Some(route.clone()),
                session_id: 7,
                message_id,
                correlation_id: message_id,
                response: WorkerResponse::Pong {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                },
            })
            .unwrap();
            // The serialized response contains only fixed names and numeric
            // values; its generated grant is an array of bytes, not shell text.
            assert!(!response.contains('\''));
            script.push_str(&format!(
                "IFS= read -r request || exit; printf '%s\\n' '{response}';\n"
            ));
        }
        script.push_str("IFS= read -r request");
        let mut child = Command::new("/bin/sh")
            .args(["-c", &script])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let monitor = RssMonitor::start(child.id()).unwrap();
        let session = Arc::new(Mutex::new(WorkerProcessSession {
            process_observation: None,
            child,
            input,
            output,
            response_buffer: Vec::new(),
            session_id: 7,
            next_message_id: 1,
            limits: WorkerLimits::default(),
            router: Arc::new(WorkerBrokerRouter::default()),
            rss_monitor: monitor.clone(),
        }));
        let temporary_handle = session.clone();
        drop(temporary_handle);
        assert!(session.lock().unwrap().child.try_wait().unwrap().is_none());
        let callers = (0..4)
            .map(|_| {
                let session = session.clone();
                let route = route.clone();
                std::thread::spawn(move || {
                    for _ in 0..8 {
                        let response = session
                            .lock()
                            .unwrap()
                            .request_with_timeout(
                                &route,
                                WorkerRequest::Ping,
                                Duration::from_secs(2),
                            )
                            .unwrap();
                        assert!(matches!(response, WorkerResponse::Pong { .. }));
                    }
                })
            })
            .collect::<Vec<_>>();
        for caller in callers {
            caller.join().unwrap();
        }
        assert_eq!(session.lock().unwrap().next_message_id, 33);
        assert!(!monitor.stop.load(Ordering::Acquire));
        drop(session);
        assert!(monitor.stop.load(Ordering::Acquire));
    }

    #[test]
    fn malformed_or_closed_response_retires_process_without_page_wrapper() {
        // A deliberately unauthenticated test peer, constructed directly so
        // this exercises transport retirement independently of WorkerPage.
        for script in [
            "IFS= read -r request; printf 'not-json\\n'; IFS= read -r request",
            "IFS= read -r request; printf '%0600d\\n' 0; IFS= read -r request",
            "IFS= read -r request; exec 1>&-; IFS= read -r request",
            "IFS= read -r request; IFS= read -r request",
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", script])
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let input = child.stdin.take().unwrap();
            let output = child.stdout.take().unwrap();
            let rss_monitor = RssMonitor::start(child.id()).unwrap();
            let mut client = WorkerClient {
                attached: true,
                pending_initialization: None,
                actor_route: crate::worker_route::WorkerRoute::issue(1, 1, 0).unwrap(),
                route_identity: crate::worker_route::WorkerRoute::issue(1, 1, 0).unwrap(),
                session: Arc::new(Mutex::new(WorkerProcessSession {
                    process_observation: None,
                    child,
                    input,
                    output,
                    response_buffer: Vec::new(),
                    session_id: 1,
                    next_message_id: 1,
                    limits: WorkerLimits {
                        ipc_frame_bytes: 512,
                        ..WorkerLimits::default()
                    },
                    router: Arc::new(WorkerBrokerRouter::default()),
                    rss_monitor,
                })),
                session_id: 1,
                limits: WorkerLimits {
                    ipc_frame_bytes: 512,
                    ..WorkerLimits::default()
                },
                runtime: crate::runtime_build_info(),
                binary_sha256: String::new(),
                verification: None,
                router: Arc::new(WorkerBrokerRouter::default()),
                leases: 0,
                code_directory_hash: None,
            };
            let error = client
                .request_with_timeout(WorkerRequest::Ping, Duration::from_secs(2))
                .unwrap_err();
            if script == "IFS= read -r request; IFS= read -r request" {
                assert!(
                    matches!(&error, RuntimeError::WorkerLimitExceeded { resource }
                    if resource == "wall time (phase=ipc_response, command=ping, message_id=1)")
                );
            }
            assert!(matches!(
                error,
                RuntimeError::ProtocolViolation(_)
                    | RuntimeError::WorkerLimitExceeded { .. }
                    | RuntimeError::WorkerCrashed(_)
            ));
            assert!(
                client
                    .session
                    .lock()
                    .unwrap()
                    .child
                    .try_wait()
                    .unwrap()
                    .is_some(),
                "failed peer must be reaped before returning"
            );
            assert!(
                client
                    .session
                    .lock()
                    .unwrap()
                    .rss_monitor
                    .stop
                    .load(Ordering::Acquire)
            );
        }
    }

    #[test]
    fn terminated_response_cannot_bypass_frame_limit() {
        for bytes in [b"12345\n".as_slice(), b"123456789\n"] {
            let mut buffer = bytes.to_vec();
            assert!(matches!(
                super::take_response_frame(&mut buffer, 5),
                Err(crate::RuntimeError::WorkerLimitExceeded { .. })
            ));
            assert_eq!(buffer, bytes, "reject before draining attacker input");
        }
    }

    #[test]
    fn response_frame_limit_includes_delimiter_and_preserves_coalesced_frames() {
        let mut buffer = b"1234\n12\n".to_vec();
        assert_eq!(
            super::take_response_frame(&mut buffer, 5).unwrap(),
            Some(b"1234\n".to_vec())
        );
        assert_eq!(
            super::take_response_frame(&mut buffer, 5).unwrap(),
            Some(b"12\n".to_vec())
        );
        assert!(buffer.is_empty());
        buffer.extend_from_slice(b"1234");
        assert_eq!(super::take_response_frame(&mut buffer, 5).unwrap(), None);
        buffer.push(b'5');
        assert!(super::take_response_frame(&mut buffer, 5).is_err());
        assert!(super::take_response_frame(&mut Vec::new(), 0).is_err());
    }

    #[test]
    fn session_identifier_exhaustion_never_reuses_authority() {
        let counter = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
        assert_eq!(super::allocate_session_id(&counter).unwrap(), u64::MAX - 1);
        assert!(super::allocate_session_id(&counter).is_err());
        assert!(super::allocate_session_id(&counter).is_err());
        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), u64::MAX);
    }

    use std::sync::{Arc, Mutex};

    use crate::Browser;

    use super::*;

    struct CaptureBroker {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }

    impl NetworkBroker for CaptureBroker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/html".into())],
                body: br#"<meta http-equiv="content-security-policy" content="script-src 'none'">"#
                    .to_vec(),
            })
        }
    }

    #[test]
    fn sealed_navigation_source_is_identical_for_send_and_stream_and_private_to_its_lease() {
        struct RedirectBroker {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
        }
        impl NetworkBroker for RedirectBroker {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                let (status, headers) = if request.url.ends_with("/start") {
                    (
                        302,
                        vec![("location".into(), "https://final.test/receipt".into())],
                    )
                } else {
                    (200, vec![("content-type".into(), "text/html".into())])
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status,
                    headers,
                    body: vec![],
                })
            }
        }
        for streaming in [false, true] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            let target = context.native_initial_document_creator(page_id).unwrap();
            let url = "https://initial.test/start";
            let sealed = context
                .issue_script_navigation_authority(crate::runtime::ScriptNavigationInputs {
                    source: page_id,
                    source_creator: target.clone(),
                    destination: page_id,
                    target_creator: target.clone(),
                    destination_url: url,
                    source_url: "about:blank",
                    referrer_policy: "no-referrer",
                })
                .unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let inner = crate::network::SharedNetworkBroker::new(RedirectBroker {
                requests: requests.clone(),
            });
            let route = BoundWorkerRoute {
                identity: crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap(),
                context: ParentContextService::new_with_network(
                    context.clone(),
                    page_id,
                    Some(inner.clone()),
                )
                .into_shared(),
                network: PageNetworkBroker {
                    authority: Arc::new(RouteAuthority::default()),
                    inner,
                    context: context.downgrade(),
                    page_id,
                },
            };
            let nonce = "b".repeat(64);
            let acquire = |nonce: String| ContextServiceRequest::AcquireNavigationNetworkSource {
                page_id,
                nonce,
                target: target.clone(),
                url: url.into(),
                authority: crate::context_service::NavigationNetworkAuthority::Script(
                    sealed.clone(),
                ),
            };
            let ContextServiceResponse::NavigationNetworkSource {
                source,
                bound: true,
            } = route.context_request(acquire(nonce.clone())).unwrap()
            else {
                panic!("missing route binding");
            };
            let request = |id, url: &str, token: Option<String>| NetworkRequest {
                request_id: id,
                method: "GET".into(),
                url: url.into(),
                headers: vec![("referer".into(), "https://forged.test/".into())],
                body: vec![],
                credentials: true,
                security: CookieRequestContext {
                    navigation_source: token,
                    kind: CookieRequestKind::Document,
                    initiator_url: Some("https://forged.test/".into()),
                    top_level_url: Some("https://forged.test/".into()),
                    ancestor_urls: vec!["https://forged.test/".into()],
                    ..Default::default()
                },
                resolved_addrs: vec![],
            };
            let dispatch = |request: NetworkRequest| {
                if streaming {
                    let mut operation = route.network.start_operation(request).unwrap();
                    let mut status = None;
                    while let Some(event) = operation.blocking_event() {
                        match event {
                            NetworkEvent::Headers { status: value, .. } => {
                                status = Some(value);
                                operation.request_body_chunk();
                            }
                            NetworkEvent::Chunk { .. } => operation.request_body_chunk(),
                            NetworkEvent::Complete { .. } => break,
                            NetworkEvent::Failed { error, .. } => panic!("{error}"),
                        }
                    }
                    status.unwrap()
                } else {
                    route.network.send(request).unwrap().status
                }
            };
            // Same URL without the nonce leaves the admitted source lease untouched.
            assert_eq!(dispatch(request(90, url, None)), 302);
            assert!(
                route
                    .network
                    .send(request(91, "https://wrong.test/", Some(nonce.clone())))
                    .is_err()
            );
            assert_eq!(dispatch(request(1, url, Some(nonce.clone()))), 302);
            route.network.cancel_operation(1).unwrap();
            assert_eq!(
                dispatch(request(
                    2,
                    "https://final.test/receipt",
                    Some(nonce.clone())
                )),
                200
            );
            assert!(
                route
                    .network
                    .send(request(3, url, Some(nonce.clone())))
                    .is_err()
            );
            let sent = requests.lock().unwrap();
            assert_eq!(sent.len(), 3);
            for request in sent.iter().skip(1) {
                assert!(
                    request.security.navigation_source.is_none(),
                    "private nonce must stop at the parent broker"
                );
                assert_eq!(
                    request.security.initiator_url.as_deref(),
                    Some(if source.opaque_origin {
                        "about:blank"
                    } else {
                        &source.url
                    })
                );
                assert_eq!(
                    request.security.top_level_url.as_deref(),
                    Some(source.top_level_url.as_str())
                );
                assert_eq!(request.security.ancestor_urls, source.ancestor_urls);
                assert!(
                    !request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
                );
            }
            assert!(sent[2].security.redirect_chain_cross_site);
            drop(sent);
            let stale = "c".repeat(64);
            route.context_request(acquire(stale.clone())).unwrap();
            let _parked = context.park_initial_document(page_id).unwrap();
            assert!(route.network.send(request(4, url, Some(stale))).is_err());
            route.network.authority.revoke().unwrap();
            assert!(route.context_request(acquire("d".repeat(64))).is_err());
        }
    }

    #[test]
    fn parent_derives_request_provenance_and_defers_meta_policy_to_native_parser() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .native_initial_document_creator(page.page_id())
            .unwrap();
        context
            .commit_page_url(page.page_id(), "https://app.example/".into())
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broker = PageNetworkBroker {
            authority: Arc::new(RouteAuthority::default()),
            inner: crate::network::SharedNetworkBroker::new(CaptureBroker {
                requests: requests.clone(),
            }),
            context: context.downgrade(),
            page_id: page.page_id(),
        };
        broker
            .send(NetworkRequest {
                request_id: 1,
                method: "GET".into(),
                url: "https://app.example/next".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: true,
                security: CookieRequestContext {
                    dns_timing: None,
                    kind: CookieRequestKind::Document,
                    initiator_url: Some("https://forged.example/".into()),
                    top_level_url: Some("https://forged.example/".into()),
                    ..CookieRequestContext::default()
                },
                resolved_addrs: Vec::new(),
            })
            .unwrap();
        let sent = requests.lock().unwrap().first().cloned().unwrap();
        assert_eq!(
            sent.security.initiator_url.as_deref(),
            Some("https://app.example/")
        );
        // Raw response text is not an admitted parser mutation. The exact
        // native parser meta-before-script regression lives in
        // frame_script_preparation::html_encoding_restart_regressions.
        assert!(
            context
                .authorize_csp(
                    page.page_id(),
                    &CspRequest {
                        destination: CspDestination::Script,
                        url: "https://app.example/app.js".into(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false
                    }
                )
                .unwrap()
                .allowed
        );
        context
            .install_document_security(
                page.page_id(),
                "https://app.example/next".into(),
                &[("content-security-policy".into(), "script-src 'none'".into())],
            )
            .unwrap();
        assert!(
            !context
                .authorize_csp(
                    page.page_id(),
                    &CspRequest {
                        destination: CspDestination::Script,
                        url: "https://app.example/app.js".into(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    },
                )
                .unwrap()
                .allowed
        );
    }

    #[test]
    fn signed_worker_evidence_hashes_are_stable() {
        assert_eq!(expected_entitlement_sha256().len(), 64);
        assert_eq!(compiled_policy_sha256().len(), 64);
    }

    #[test]
    fn cached_worker_verification_still_rejects_changed_engine_profile() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-profile-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app = directory.join("OrynPageWorker.app");
        let resources = app.join("Contents/Resources");
        let executable = app.join("Contents/MacOS/oryn-page-worker");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(&resources).unwrap();
        fs::write(&executable, b"cache fixture executable").unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let profile = resources.join("v8-profile-manifest.json");
        fs::write(&profile, b"original engine profile").unwrap();
        let manifest = WorkerManifest {
            render_contract_version: 3,
            local_effects_contract_version: oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION,
            schema_version: 1,
            bundle_id: EXPECTED_BUNDLE_ID.into(),
            team_id: EXPECTED_TEAM_ID.into(),
            signing_cert_sha1: EXPECTED_SIGNING_CERT_SHA1.into(),
            protocol_version: WORKER_PROTOCOL_VERSION,
            context_service_protocol_version:
                crate::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION,
            agent_ipc_protocol_version: crate::agent_ipc::VERSION,
            v8_version: "150.4.0".into(),
            v8_archive_sha256: "a".repeat(64),
            v8_patch_chain_sha256: "b".repeat(64),
            v8_profile_sha256: sha256_file(&profile).unwrap(),
            worker_build_sha256: sha256_file(&executable).unwrap(),
            worker_build_bytes: fs::metadata(&executable).unwrap().len(),
            entitlement_sha256: expected_entitlement_sha256(),
            policy_sha256: compiled_policy_sha256(),
            source_tree_sha256: "c".repeat(64),
            limits: WorkerLimits::default(),
            style_engine_version: EXPECTED_STYLE_ENGINE_VERSION.into(),
            box_engine_version: EXPECTED_BOX_ENGINE_VERSION.into(),
            inline_engine_version: EXPECTED_INLINE_ENGINE_VERSION.into(),
            embedded_font_sha256: "d".repeat(64),
        };
        assert!(manifest.protocols_match());
        assert!(manifest.render_contract_matches());
        for version in [0, 2, 4, u16::MAX] {
            let mut changed = manifest.clone();
            changed.render_contract_version = version;
            assert!(!changed.render_contract_matches());
        }
        let mut missing_render = serde_json::to_value(&manifest).unwrap();
        missing_render
            .as_object_mut()
            .unwrap()
            .remove("render_contract_version");
        assert!(serde_json::from_value::<WorkerManifest>(missing_render).is_err());
        for field in [
            "protocol_version",
            "context_service_protocol_version",
            "agent_ipc_protocol_version",
        ] {
            let original = serde_json::to_value(&manifest).unwrap();
            for invalid in [
                serde_json::Value::Null,
                serde_json::json!(true),
                serde_json::json!("16"),
            ] {
                let mut value = original.clone();
                value[field] = invalid;
                assert!(serde_json::from_value::<WorkerManifest>(value).is_err());
            }
            let mut absent = original.clone();
            absent.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<WorkerManifest>(absent).is_err());
            let mut stale = original.clone();
            stale[field] = serde_json::json!(original[field].as_u64().unwrap() + 1);
            assert!(
                !serde_json::from_value::<WorkerManifest>(stale)
                    .unwrap()
                    .protocols_match()
            );
        }
        let bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(resources.join("worker-manifest.json"), &bytes).unwrap();
        let identity = worker_file_identity(executable.clone(), &bytes).unwrap();
        let cached = WorkerVerification {
            path: executable.clone(),
            bundle_id: manifest.bundle_id.clone(),
            team_id: manifest.team_id.clone(),
            signing_cert_sha1: manifest.signing_cert_sha1.clone(),
            binary_sha256: manifest.worker_build_sha256.clone(),
            manifest_sha256: identity.manifest_sha256.clone(),
            app_sandbox: true,
            allow_jit: true,
            hardened_runtime: true,
        };
        // Simulate a prior successful signature verification without requiring
        // a signing identity or executing an external worker in this regression.
        WORKER_VERIFICATION_CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .insert(identity.clone(), cached.clone());
        assert_eq!(verify_worker(&executable).unwrap(), cached);
        fs::write(&profile, b"replaced engine profile").unwrap();
        let error = verify_worker(&executable).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("signed V8 profile digest mismatch")
        );
        WORKER_VERIFICATION_CACHE
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&identity);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn verification_identity_tracks_file_and_manifest_changes() {
        let path =
            std::env::temp_dir().join(format!("oryn-verification-identity-{}", std::process::id()));
        fs::write(&path, b"first").unwrap();
        let first = worker_file_identity(path.clone(), b"manifest-a").unwrap();
        let same = worker_file_identity(path.clone(), b"manifest-a").unwrap();
        let changed_manifest = worker_file_identity(path.clone(), b"manifest-b").unwrap();
        fs::write(&path, b"different-size").unwrap();
        let changed_file = worker_file_identity(path.clone(), b"manifest-a").unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(first, same);
        assert_ne!(first, changed_manifest);
        assert_ne!(first, changed_file);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_rss_measurement_succeeds_and_fails_closed() {
        assert!(resident_bytes(std::process::id()).unwrap() > 0);
        assert!(resident_bytes(u32::MAX).is_err());
    }

    #[test]
    fn asynchronous_network_admission_is_held_until_revocation_cancels_the_operation() {
        let fixture_context = Browser::new().new_context();
        struct HeldOperationBroker {
            sender: Mutex<Option<tokio::sync::mpsc::Sender<NetworkEvent>>>,
            cancelled: Arc<Mutex<Vec<u64>>>,
            started: Arc<Mutex<Vec<u64>>>,
        }
        impl NetworkBroker for HeldOperationBroker {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!("asynchronous operation required")
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                self.started.lock().unwrap().push(request.request_id);
                let (sender, events) = tokio::sync::mpsc::channel(1);
                *self.sender.lock().unwrap() = Some(sender);
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
                self.cancelled.lock().unwrap().push(request_id);
                self.sender.lock().unwrap().take();
                Ok(())
            }
        }
        let cancelled = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Mutex::new(Vec::new()));
        let mut route = fixture_worker_route(&fixture_context);
        route.network.inner = crate::network::SharedNetworkBroker::new(HeldOperationBroker {
            sender: Mutex::new(None),
            cancelled: cancelled.clone(),
            started: started.clone(),
        });
        let authority = route.network.authority.clone();
        let router = WorkerBrokerRouter::default();
        router.bind(route).unwrap();
        let operation = router
            .start_operation(NetworkRequest {
                request_id: 17,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: false,
                security: CookieRequestContext::default(),
                resolved_addrs: Vec::new(),
            })
            .unwrap();
        assert_eq!(authority.state.lock().unwrap().active, 1);
        let native_id = *started.lock().unwrap().first().unwrap();
        // An unknown cancellation must not be forwarded to another route's work.
        router.cancel_operation(18).unwrap();
        assert!(cancelled.lock().unwrap().is_empty());
        router.unbind().unwrap();
        assert_eq!(*cancelled.lock().unwrap(), [native_id]);
        assert_eq!(authority.state.lock().unwrap().active, 0);
        drop(operation);
        router.bind(fixture_worker_route(&fixture_context)).unwrap();
        router.unbind().unwrap();
    }

    struct HeadersThenBodyEnd {
        failed: bool,
        duplicate_headers: bool,
    }

    impl NetworkBroker for HeadersThenBodyEnd {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            panic!("regression must use the streaming parent path")
        }

        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(3);
            sender
                .try_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    content_length: Some(1),
                    upload: None,
                })
                .unwrap();
            if self.duplicate_headers {
                sender
                    .try_send(NetworkEvent::Headers {
                        response_policy: None,
                        timing: None,
                        request_id: request.request_id,
                        status: 200,
                        headers: vec![("content-type".into(), "text/plain".into())],
                        content_length: Some(1),
                        upload: None,
                    })
                    .unwrap();
            }
            if self.failed {
                sender
                    .try_send(NetworkEvent::Failed {
                        request_id: request.request_id,
                        error: "body failed after admitted headers".into(),
                    })
                    .unwrap();
            }
            drop(sender);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
    }

    fn header_transfer_fixture(
        failed: bool,
    ) -> (
        BrowserContext,
        crate::runtime::PageHandle,
        PageNetworkBroker,
    ) {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let broker = PageNetworkBroker {
            authority: Arc::new(RouteAuthority::default()),
            inner: crate::network::SharedNetworkBroker::new(HeadersThenBodyEnd {
                failed,
                duplicate_headers: false,
            }),
            context: context.downgrade(),
            page_id: page.page_id(),
        };
        (context, page, broker)
    }

    fn wait_header_relay(authority: &RouteAuthority) {
        let state = authority.state.lock().unwrap();
        let (state, _) = authority
            .drained
            .wait_timeout_while(state, Duration::from_secs(5), |state| state.active != 0)
            .unwrap();
        assert_eq!(state.active, 0, "parent response relay did not settle");
    }

    fn header_transfer_request() -> NetworkRequest {
        NetworkRequest {
            request_id: 41,
            method: "GET".into(),
            url: "https://headers.test/document".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: true,
            resolved_addrs: Vec::new(),
            security: CookieRequestContext {
                kind: CookieRequestKind::Document,
                ..Default::default()
            },
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streamed_header_transfer_still_rejects_duplicate_header_events() {
        let (_context, page, mut broker) = header_transfer_fixture(false);
        broker.inner = crate::network::SharedNetworkBroker::new(HeadersThenBodyEnd {
            failed: false,
            duplicate_headers: true,
        });
        let mut operation = broker.start_operation(header_transfer_request()).unwrap();
        let token = match operation.next_event().await.unwrap() {
            NetworkEvent::Headers {
                response_policy: Some(token),
                headers,
                ..
            } => {
                assert_eq!(headers, vec![("content-type".into(), "text/html".into())]);
                token
            }
            event => panic!("missing original response: {event:?}"),
        };
        assert!(
            matches!(operation.next_event().await, Some(NetworkEvent::Failed { error, .. }) if error.to_string() == "navigation response headers already admitted")
        );
        assert!(operation.next_event().await.is_none());
        wait_header_relay(&broker.authority);
        drop(token);
        drop(operation);
        broker
            .inner
            .drain_owned_operations(Duration::from_secs(5))
            .unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streamed_header_grant_survives_body_failure_and_eof_before_native_commit() {
        for failed in [false, true] {
            let (context, page, broker) = header_transfer_fixture(failed);
            let mut operation = broker.start_operation(header_transfer_request()).unwrap();
            let (token, headers) = match operation.next_event().await.unwrap() {
                NetworkEvent::Headers {
                    response_policy: Some(token),
                    headers,
                    ..
                } => (token, headers),
                event => panic!("missing admitted response: {event:?}"),
            };
            if failed {
                assert!(
                    matches!(operation.next_event().await, Some(NetworkEvent::Failed { error, .. }) if error.to_string().contains("body failed after admitted headers"))
                );
            }
            assert!(operation.next_event().await.is_none());
            wait_header_relay(&broker.authority);
            // The response body has ended unsuccessfully before native commit.
            // Its retained headers still authenticate exactly one document.
            let parked = context.park_initial_document(page.page_id()).unwrap();
            broker
                .authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .commit(
                    page.page_id(),
                    &token,
                    "https://headers.test/document",
                    &headers,
                    |source, url, values| {
                        context.commit_navigation_response_policy(
                            page.page_id(),
                            source,
                            url,
                            values,
                        )
                    },
                )
                .unwrap();
            token.committed();
            context
                .verify_document_security(page.page_id(), "https://headers.test/document", &headers)
                .unwrap();
            assert!(
                broker
                    .authority
                    .state
                    .lock()
                    .unwrap()
                    .response_policies
                    .commit(
                        page.page_id(),
                        &token,
                        "https://headers.test/document",
                        &headers,
                        |_, _, _| panic!("replayed response installed policy"),
                    )
                    .is_err()
            );
            context
                .finish_initial_document(page.page_id(), parked, false)
                .unwrap();
            drop(token);
            drop(operation);
            broker
                .inner
                .drain_owned_operations(Duration::from_secs(5))
                .unwrap();
            page.close().await.unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streamed_header_custody_reclaims_unread_and_uncommitted_responses() {
        for unread in [false, true] {
            let (context, page, broker) = header_transfer_fixture(true);
            let source = context
                .capture_navigation_response_source(page.page_id())
                .unwrap();
            let mut operation = broker.start_operation(header_transfer_request()).unwrap();
            let token = if unread {
                None
            } else {
                match operation.next_event().await.unwrap() {
                    NetworkEvent::Headers {
                        response_policy: Some(token),
                        ..
                    } => Some(token),
                    event => panic!("missing admitted response: {event:?}"),
                }
            };
            drop(operation);
            wait_header_relay(&broker.authority);
            if let Some(token) = token {
                assert!(
                    broker
                        .authority
                        .state
                        .lock()
                        .unwrap()
                        .response_policies
                        .begin(41, source.clone())
                        .is_err(),
                    "retained header token must still own its grant"
                );
                drop(token);
            }
            // Dropping an unread queue or the last retained header token frees
            // the request ID, independently of body terminal delivery.
            let next = broker
                .authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .begin(41, source)
                .unwrap();
            broker
                .authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .discard(&next);
            broker
                .inner
                .drain_owned_operations(Duration::from_secs(5))
                .unwrap();
            page.close().await.unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streamed_header_custody_remains_revocable_by_cancel_and_retirement() {
        for retired in [false, true] {
            let (_context, page, broker) = header_transfer_fixture(true);
            let mut operation = broker.start_operation(header_transfer_request()).unwrap();
            let (token, headers) = match operation.next_event().await.unwrap() {
                NetworkEvent::Headers {
                    response_policy: Some(token),
                    headers,
                    ..
                } => (token, headers),
                event => panic!("missing admitted response: {event:?}"),
            };
            if retired {
                broker.authority.revoke().unwrap();
            } else {
                broker.cancel_operation(41).unwrap();
            }
            let error = broker
                .authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .commit(
                    page.page_id(),
                    &token,
                    "https://headers.test/document",
                    &headers,
                    |_, _, _| panic!("revoked response installed policy"),
                )
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(if retired {
                    "response policy route retired"
                } else {
                    "reason=request_cancelled"
                }),
                "{error}"
            );
            drop(token);
            drop(operation);
            wait_header_relay(&broker.authority);
            broker
                .inner
                .drain_owned_operations(Duration::from_secs(5))
                .unwrap();
            page.close().await.unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ipc_delivered_header_grant_survives_body_failure_until_native_commit() {
        let (context, page, network) = header_transfer_fixture(true);
        let authority = network.authority.clone();
        let inner = network.inner.clone();
        let page_id = page.page_id();
        let service =
            ParentContextService::new_with_network(context.clone(), page_id, Some(inner.clone()))
                .into_shared();
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let router = Arc::new(WorkerBrokerRouter::default());
        router
            .bind(BoundWorkerRoute {
                identity: identity.clone(),
                network,
                context: service,
            })
            .unwrap();
        let (worker, parent) = UnixStream::pair().unwrap();
        let server_router = router.clone();
        let server = std::thread::spawn(move || {
            crate::network::serve_ipc_broker(parent, server_router, 1024 * 1024)
        });
        let ipc = crate::network::IpcNetworkBroker::new(worker);
        let mut operation = ipc
            .start_routed(Some(&identity), header_transfer_request())
            .unwrap();
        let (token, headers) = match operation.next_event().await.unwrap() {
            NetworkEvent::Headers {
                response_policy: Some(token),
                headers,
                ..
            } => (token, headers),
            event => panic!("missing IPC headers: {event:?}"),
        };
        assert!(
            matches!(operation.next_event().await, Some(NetworkEvent::Failed { error, .. }) if error.to_string().contains("body failed after admitted headers"))
        );
        assert!(operation.next_event().await.is_none());
        wait_header_relay(&authority);
        let parked = context.park_initial_document(page_id).unwrap();
        assert!(matches!(
            router
                .request_routed(
                    Some(&identity),
                    ContextServiceRequest::CommitNavigationResponsePolicy {
                        page_id,
                        token: token.to_string(),
                        document_url: "https://headers.test/document".into(),
                        headers: headers.clone(),
                    }
                )
                .unwrap(),
            ContextServiceResponse::DocumentSecurityInstalled
        ));
        token.committed();
        context
            .verify_document_security(page_id, "https://headers.test/document", &headers)
            .unwrap();
        context
            .finish_initial_document(page_id, parked, false)
            .unwrap();
        drop(token);
        drop(operation);
        drop(ipc);
        server.join().unwrap().unwrap();
        inner
            .drain_owned_operations(Duration::from_secs(5))
            .unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn response_policy_cleanup_diagnostics_distinguish_context_and_network_owners() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let inner = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let service =
            ParentContextService::new_with_network(context.clone(), page_id, Some(inner.clone()))
                .into_shared();
        let authority = Arc::new(RouteAuthority::default());
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let router = WorkerBrokerRouter::default();
        router
            .bind(BoundWorkerRoute {
                identity: identity.clone(),
                context: service,
                network: PageNetworkBroker {
                    authority: authority.clone(),
                    inner,
                    context: context.downgrade(),
                    page_id,
                },
            })
            .unwrap();
        let url = "https://response-policy.test/";
        for (request_id, network_cleanup, reason) in [
            (1, false, "context_token_discarded"),
            (2, true, "network_token_discarded"),
        ] {
            let source = context.capture_navigation_response_source(page_id).unwrap();
            let token = {
                let mut state = authority.state.lock().unwrap();
                let token = state.response_policies.begin(request_id, source).unwrap();
                state
                    .response_policies
                    .headers(&token, 200, url.into(), Vec::new())
                    .unwrap();
                token
            };
            let cleanup = |route: &crate::worker_route::WorkerRoute| {
                if network_cleanup {
                    NetworkBroker::discard_response_policy_routed(&router, Some(route), &token)
                        .map_err(|error| error.to_string())
                } else {
                    router
                        .request_routed(
                            Some(route),
                            ContextServiceRequest::DiscardNavigationResponsePolicy {
                                page_id,
                                token: token.clone(),
                            },
                        )
                        .map(|response| {
                            assert!(matches!(
                                response,
                                ContextServiceResponse::NavigationResponsePolicyDiscarded
                            ));
                        })
                        .map_err(|error| error.to_string())
                }
            };
            let foreign = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
            assert!(cleanup(&foreign).is_err());
            // Failed foreign cleanup cannot consume the original parent's grant.
            assert!(
                authority
                    .state
                    .lock()
                    .unwrap()
                    .response_policies
                    .headers(&token, 200, url.into(), Vec::new())
                    .unwrap_err()
                    .to_string()
                    .contains("headers already admitted")
            );
            cleanup(&identity).unwrap();
            cleanup(&identity).unwrap();
            let error = router
                .request_routed(
                    Some(&identity),
                    ContextServiceRequest::CommitNavigationResponsePolicy {
                        page_id,
                        token: token.clone(),
                        document_url: url.into(),
                        headers: Vec::new(),
                    },
                )
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("reason={reason} request_id={request_id}")),
                "{error}"
            );
            assert!(!error.contains(&token));
        }
        // Revoked routes retain exact-route idempotent cleanup admission.
        authority.revoke().unwrap();
        NetworkBroker::discard_response_policy_routed(&router, Some(&identity), "already-removed")
            .unwrap();
        router
            .request_routed(
                Some(&identity),
                ContextServiceRequest::DiscardNavigationResponsePolicy {
                    page_id,
                    token: "already-removed".into(),
                },
            )
            .unwrap();
        page.close().await.unwrap();
    }

    #[test]
    fn response_reservation_drop_records_abandonment_but_completed_transfer_remains_committable() {
        let authority = Arc::new(RouteAuthority::default());
        let source = crate::agent_registry::AgentOwner::Window {
            page: PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 7,
            epoch: 0,
        };
        for completed in [false, true] {
            let token = authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .begin(1, source.clone())
                .unwrap();
            let reservation = ResponsePolicyReservation {
                authority: authority.clone(),
                token: Some(token.clone()),
                admitted: std::cell::Cell::new(false),
            };
            reservation
                .headers(200, "https://response-policy.test/".into(), Vec::new())
                .unwrap();
            if completed {
                reservation.retain_for_commit();
            } else {
                drop(reservation);
            }
            let mut state = authority.state.lock().unwrap();
            let result = state.response_policies.commit(
                PageId(1),
                &token,
                "https://response-policy.test/",
                &[],
                |_, _, _| Ok(()),
            );
            if completed {
                result.unwrap();
            } else {
                let error = result.unwrap_err().to_string();
                assert!(
                    error.contains("reason=reservation_abandoned request_id=1"),
                    "{error}"
                );
                assert!(!error.contains(&token));
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn response_policy_commit_requires_exact_live_router_and_parent_grant() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let inner = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let service =
            ParentContextService::new_with_network(context.clone(), page_id, Some(inner.clone()))
                .into_shared();
        let authority = Arc::new(RouteAuthority::default());
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let router = WorkerBrokerRouter::default();
        router
            .bind(BoundWorkerRoute {
                identity: identity.clone(),
                context: service.clone(),
                network: PageNetworkBroker {
                    authority: authority.clone(),
                    inner,
                    context: context.downgrade(),
                    page_id,
                },
            })
            .unwrap();
        let url = "https://response-policy.test/";
        let response = router
            .authorized(Some(&identity))
            .unwrap()
            .network
            .send(NetworkRequest {
                request_id: 1,
                method: "GET".into(),
                url: url.into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: true,
                resolved_addrs: Vec::new(),
                security: CookieRequestContext {
                    kind: CookieRequestKind::Document,
                    ..Default::default()
                },
            })
            .unwrap();
        let headers = crate::network::sanitize_parent_response_headers(response.headers);
        let token = response
            .response_policy
            .expect("parent-issued response grant");
        let request = || ContextServiceRequest::CommitNavigationResponsePolicy {
            page_id,
            token: token.to_string(),
            document_url: url.into(),
            headers: headers.clone(),
        };
        assert!(ContextTransport::request(&*service, request()).is_err());
        assert!(router.request_routed(None, request()).is_err());
        let forged = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        assert!(router.request_routed(Some(&forged), request()).is_err());
        let parked = context.park_initial_document(page_id).unwrap();
        assert!(matches!(
            router.request_routed(Some(&identity), request()).unwrap(),
            ContextServiceResponse::DocumentSecurityInstalled
        ));
        context
            .verify_document_security(page_id, url, &headers)
            .unwrap();
        let replay_error = router
            .request_routed(Some(&identity), request())
            .unwrap_err()
            .to_string();
        assert!(
            replay_error.contains("reason=consumed request_id=1"),
            "{replay_error}"
        );
        assert!(
            replay_error.contains(&format!(
                "route_process_session=1 route_environment={} route_generation=0 route_page={}",
                page_id.0, page_id.0,
            )),
            "{replay_error}"
        );
        assert!(!replay_error.contains(token.as_str()));
        context
            .finish_initial_document(page_id, parked, false)
            .unwrap();
        // Even a retained route clone cannot commit after revocation. This
        // bypasses the router lookup to exercise the inner authority barrier.
        let retained = router.authorized(Some(&identity)).unwrap();
        authority.revoke().unwrap();
        assert!(retained.context_request(request()).is_err());
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streaming_document_headers_preserve_initial_policy_and_cancellation_revokes_grant() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let authority = Arc::new(RouteAuthority::default());
        let broker = PageNetworkBroker {
            authority: authority.clone(),
            inner: crate::network::SharedNetworkBroker::new(CaptureBroker {
                requests: Arc::new(Mutex::new(Vec::new())),
            }),
            context: context.downgrade(),
            page_id,
        };
        let mut operation = broker
            .start_operation(NetworkRequest {
                request_id: 41,
                method: "GET".into(),
                url: "https://stream.test/".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: true,
                resolved_addrs: Vec::new(),
                security: CookieRequestContext {
                    kind: CookieRequestKind::Document,
                    ..Default::default()
                },
            })
            .unwrap();
        let (token, headers) = match operation.next_event().await.unwrap() {
            NetworkEvent::Headers {
                response_policy: Some(token),
                headers,
                ..
            } => (token, headers),
            event => panic!("missing parent response grant: {event:?}"),
        };
        assert!(context.native_initial_document_creator(page_id).is_ok());
        let parked = context.park_initial_document(page_id).unwrap();
        broker.cancel_operation(41).unwrap();
        assert!(
            authority
                .state
                .lock()
                .unwrap()
                .response_policies
                .commit(
                    page_id,
                    &token,
                    "https://stream.test/",
                    &headers,
                    |_, _, _| panic!("cancelled stream installed document policy"),
                )
                .is_err()
        );
        drop(operation);
        context
            .finish_initial_document(page_id, parked, true)
            .unwrap();
        assert!(context.native_initial_document_creator(page_id).is_ok());
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prerender_policy_commit_authenticates_response_without_replacing_incumbent() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let source_url = "https://prerender.test/source";
        let source_headers = vec![("Content-Security-Policy".into(), "script-src 'self'".into())];
        context
            .install_document_security(page_id, source_url.into(), &source_headers)
            .unwrap();
        context
            .handle_message_wire(
                page_id,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context.commit_page_url(page_id, source_url.into()).unwrap();
        let creator = context.capture_command_navigation_source(page_id).unwrap();
        let url = "https://prerender.test/next";
        let candidate = context
            .reserve_native_prerender(page_id, &creator, url)
            .unwrap();
        let inner = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let service =
            ParentContextService::new_with_network(context.clone(), page_id, Some(inner.clone()))
                .into_shared();
        assert!(
            ContextTransport::request(
                &*service,
                ContextServiceRequest::InstallDocumentSecurity {
                    page_id,
                    document_url: source_url.into(),
                    headers: source_headers.clone(),
                }
            )
            .is_err(),
            "matching incumbent policy is not authority for a new document"
        );
        let route = BoundWorkerRoute {
            identity: identity.clone(),
            context: service.clone(),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner,
                context: context.downgrade(),
                page_id,
            },
        };
        let response = route
            .network
            .send(NetworkRequest {
                request_id: 81,
                method: "GET".into(),
                url: url.into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: true,
                resolved_addrs: Vec::new(),
                security: CookieRequestContext {
                    kind: CookieRequestKind::Document,
                    ..Default::default()
                },
            })
            .unwrap();
        let response_token = response.response_policy.unwrap();
        let request = |response_policy, headers| ContextServiceRequest::CommitPrerenderResponse {
            response_policy,
            page_id,
            token: candidate.token.clone(),
            generation: candidate.lifetime.0,
            epoch: candidate.lifetime.1,
            url: url.into(),
            headers,
        };
        let router = WorkerBrokerRouter::default();
        router.bind(route).unwrap();
        assert!(
            ContextTransport::request(&*service, request(None, response.headers.clone())).is_err()
        );
        assert!(
            router
                .request_routed(Some(&identity), request(None, response.headers.clone()))
                .is_err()
        );
        assert!(
            router
                .request_routed(
                    Some(&identity),
                    request(
                        Some(response_token.to_string()),
                        vec![("Content-Security-Policy".into(), "default-src *".into())],
                    )
                )
                .is_err()
        );
        assert!(matches!(
            router
                .request_routed(
                    Some(&identity),
                    request(Some(response_token.to_string()), response.headers.clone(),)
                )
                .unwrap(),
            ContextServiceResponse::PrerenderResponseCommitted { .. }
        ));
        context
            .verify_document_security(page_id, source_url, &source_headers)
            .unwrap();
        assert_eq!(
            context.capture_command_navigation_source(page_id).unwrap(),
            creator
        );
        assert!(
            router
                .request_routed(
                    Some(&identity),
                    request(Some(response_token.to_string()), response.headers,)
                )
                .is_err()
        );
        page.close().await.unwrap();
    }

    #[test]
    fn route_revocation_waits_for_admitted_work_and_never_reopens_old_authority() {
        let authority = Arc::new(RouteAuthority::default());
        let admission = authority.admit(Some(7)).unwrap();
        assert!(authority.admit(Some(7)).is_err());
        assert_eq!(authority.revoke().unwrap(), [7]);
        assert!(authority.admit(None).is_err());
        assert!(authority.wait_drained(Duration::ZERO).is_err());
        drop(admission);
        authority.wait_drained(Duration::ZERO).unwrap();
        assert!(authority.admit(Some(8)).is_err());
    }

    #[test]
    fn route_admission_is_bounded_and_releases_capacity() {
        let authority = Arc::new(RouteAuthority::default());
        let admissions = (0..64)
            .map(|_| authority.admit(None).unwrap())
            .collect::<Vec<_>>();
        assert!(authority.admit(None).is_err());
        drop(admissions);
        drop(authority.admit(None).unwrap());
        authority.revoke().unwrap();
        authority.wait_drained(Duration::ZERO).unwrap();
    }

    fn fixture_worker_route(context: &BrowserContext) -> BoundWorkerRoute {
        let page = context.new_page();
        context
            .handle_message_wire(
                page.page_id(),
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        BoundWorkerRoute {
            identity: crate::worker_route::WorkerRoute::issue(1, page.page_id().0, 0).unwrap(),
            context: ParentContextService::new_with_network(
                context.clone(),
                page.page_id(),
                Some(network.clone()),
            )
            .into_shared(),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner: network,
                context: context.downgrade(),
                page_id: page.page_id(),
            },
        }
    }

    #[test]
    fn staged_worker_service_requires_registered_page_and_exact_live_route_activation() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let page_id = PageId((1u64 << 63).checked_add(context.id()).unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: requests.clone(),
        });
        let service = ParentContextService::new_staged_with_network(
            context.clone(),
            page_id,
            Some(network.clone()),
        )
        .into_shared();
        assert!(
            service.admit_registered_page().is_err(),
            "unknown page cannot activate"
        );
        assert!(service.ensure_page_admitted().is_err());
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let route = BoundWorkerRoute {
            identity: identity.clone(),
            context: service.clone(),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner: network,
                context: context.downgrade(),
                page_id,
            },
        };
        let router = WorkerBrokerRouter::default();
        router.bind(route).unwrap();
        let query = || ContextServiceRequest::WebFeatureProfile { page_id };
        assert!(
            router.request_routed(Some(&identity), query()).is_err(),
            "binding alone must not activate service"
        );
        assert!(
            router
                .start_routed(
                    Some(&identity),
                    NetworkRequest {
                        request_id: 1,
                        method: "GET".into(),
                        url: "https://example.test/blocked".into(),
                        headers: Vec::new(),
                        body: Vec::new(),
                        credentials: false,
                        security: CookieRequestContext::default(),
                        resolved_addrs: Vec::new()
                    }
                )
                .is_err()
        );
        assert!(
            requests.lock().unwrap().is_empty(),
            "staging cannot reach network transport"
        );
        let forged = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        assert!(router.refresh_navigation_cleanup(&forged).is_err());
        assert!(service.ensure_page_admitted().is_err());
        let page = context.new_page_with_identity(page_id, None);
        router.refresh_navigation_cleanup(&identity).unwrap();
        router.refresh_navigation_cleanup(&identity).unwrap();
        assert!(service.ensure_page_admitted().is_ok());
        assert!(matches!(
            router.request_routed(Some(&identity), query()).unwrap(),
            ContextServiceResponse::WebFeatureProfile { .. }
        ));
        router.unbind().unwrap();
        assert!(
            service.admit_registered_page().is_err(),
            "revocation cannot be undone by late activation"
        );
        assert!(service.ensure_page_admitted().is_err());
        assert!(router.request_routed(Some(&identity), query()).is_err());
        assert!(requests.lock().unwrap().is_empty());
        drop(page);
    }

    #[test]
    fn lazy_worker_route_activates_staged_service_for_existing_registered_page() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let service = ParentContextService::new_staged_with_network(
            context.clone(),
            page_id,
            Some(network.clone()),
        )
        .into_shared();
        assert!(service.ensure_page_admitted().is_err());
        let identity = crate::worker_route::WorkerRoute::issue(1, page_id.0, 0).unwrap();
        let router = WorkerBrokerRouter::default();
        router
            .bind(BoundWorkerRoute {
                identity: identity.clone(),
                context: service.clone(),
                network: PageNetworkBroker {
                    authority: Arc::new(RouteAuthority::default()),
                    inner: network,
                    context: context.downgrade(),
                    page_id,
                },
            })
            .unwrap();
        assert!(service.ensure_page_admitted().is_ok());
        assert!(matches!(
            router
                .request_routed(
                    Some(&identity),
                    ContextServiceRequest::WebFeatureProfile { page_id }
                )
                .unwrap(),
            ContextServiceResponse::WebFeatureProfile { .. }
        ));
        router.unbind().unwrap();
        assert!(service.admit_registered_page().is_err());
    }

    #[test]
    fn abandoned_staged_worker_service_cannot_activate_after_page_registration() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let page_id = PageId((1u64 << 62).checked_add(context.id()).unwrap());
        let service = ParentContextService::new_staged_with_network(context.clone(), page_id, None);
        service.cancel_all_network_operations();
        let page = context.new_page_with_identity(page_id, None);
        assert!(service.admit_registered_page().is_err());
        assert!(service.ensure_page_admitted().is_err());
        drop(page);
    }

    #[test]
    fn stable_environment_route_does_not_restore_retired_document_authority_without_cache_capability()
     {
        use oryn_js::{
            message_wire::{MessageRequest, MessageResponse, WindowTarget},
            platform_control::{Request, WindowStorageOperation},
            worker_wire::Creator,
        };
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let page = route.network.page_id;
        let router = WorkerBrokerRouter::default();
        router.bind(route.clone()).unwrap();
        context
            .install_document_security(page, "https://first.test/".into(), &[])
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
        let old = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let access = |creator| ContextServiceRequest::NativePlatformQuery {
            page_id: page,
            creator,
            request: Request::Storage {
                local: true,
                operation: WindowStorageOperation::Access,
            },
        };
        let allowed = |creator| {
            matches!(
                router.request_routed(Some(&route.identity), access(creator)),
                Ok(ContextServiceResponse::NativePlatformResult { result: Ok(_) })
            )
        };
        assert!(allowed(old.clone()));
        let MessageResponse::DocumentParked(token) = context
            .handle_message_wire(
                page,
                MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        context
            .install_document_security(page, "https://second.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let new = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(router.authorized(Some(&route.identity)).is_ok());
        assert!(!allowed(old.clone()));
        assert!(allowed(new.clone()));
        assert!(
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0
                    }
                )
                .is_err()
        );
        let MessageResponse::DocumentParked(replacement) = context
            .handle_message_wire(
                page,
                MessageRequest::ParkDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RestoreDocument {
                    token: token.clone(),
                },
            )
            .unwrap();
        context
            .handle_message_wire(page, MessageRequest::DiscardDocument { token: replacement })
            .unwrap();
        assert!(allowed(old.clone()));
        assert!(!allowed(new.clone()));
        assert!(
            matches!(
                context.handle_message_wire(
                    page,
                    MessageRequest::RestoreDocument {
                        token: token.clone()
                    }
                ),
                Ok(MessageResponse::DocumentRestored {
                    document_generation: 1,
                    stream_epoch: 0
                })
            ),
            "an identical completed request returns its retained ACK without consuming authority again"
        );
        assert!(allowed(old.clone()));
        assert!(!allowed(new.clone()));
        assert!(
            context
                .handle_message_wire(
                    page,
                    MessageRequest::DiscardDocument {
                        token: token.clone()
                    }
                )
                .is_err(),
            "a conflicting request cannot reuse the completed capability"
        );
        let foreign_page = context.new_page();
        assert!(
            context
                .handle_message_wire(
                    foreign_page.page_id(),
                    MessageRequest::RestoreDocument {
                        token: token.clone()
                    }
                )
                .is_err(),
            "another page cannot claim a retained ACK"
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .install_document_security(page, "https://third.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 3,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let current = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 3,
            stream_epoch: 0,
        };
        assert!(router.authorized(Some(&route.identity)).is_ok());
        assert!(!allowed(old.clone()));
        assert!(allowed(current.clone()));
        assert!(
            matches!(
                context.handle_message_wire(page, MessageRequest::RestoreDocument { token }),
                Ok(MessageResponse::DocumentRestored {
                    document_generation: 1,
                    stream_epoch: 0
                })
            ),
            "historical ACK replay is transport completion, not a new restoration"
        );
        assert!(!allowed(old));
        assert!(!allowed(new));
        assert!(
            allowed(current),
            "replayed ACK cannot replace the current document authority"
        );
    }

    #[test]
    fn checked_terminal_cleanup_rejects_live_child_without_terminating_it() {
        let (mut client, probe) = protocol_fixture::responder(PageId(41), Vec::new());
        assert!(client.checked_terminal_cleanup().is_err());
        assert!(probe.process_alive());
        assert_eq!(probe.next_message_id(), 1);
    }

    #[test]
    fn checked_terminal_cleanup_requires_reap_and_retained_script_drain() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let mut route = fixture_worker_route(&context);
        let (mut client, probe) = protocol_fixture::responder(
            route.network.page_id,
            vec![(
                WorkerRequest::Close,
                WorkerResponse::LimitExceeded {
                    resource: "RSS".into(),
                },
            )],
        );
        route.identity = client.route_identity.clone();
        let registry = route.context.script_cleanup_registry();
        client.router.bind(route.clone()).unwrap();
        let token = registry.reserve().unwrap();
        let running = registry.claim(token).unwrap();
        assert!(
            matches!(client.request(WorkerRequest::Close), Err(RuntimeError::WorkerLimitExceeded { resource }) if resource == "RSS")
        );
        assert!(!probe.process_alive());
        assert!(client.checked_terminal_cleanup().is_err());
        assert!(
            client
                .router
                .script_cleanup_registry(Some(&route.identity), route.network.page_id)
                .is_ok()
        );
        assert!(running.cancel.load(Ordering::Acquire));
        drop(running);
        let evidence = client.checked_terminal_cleanup().unwrap();
        assert_eq!(evidence["worker_pid"], client.process_id());
        assert_eq!(evidence["reaped"], true);
        assert_eq!(evidence["router_cleanup_complete"], true);
        assert!(
            client
                .router
                .route
                .lock()
                .unwrap()
                .script_cleanup
                .is_empty()
        );
        assert_eq!(probe.next_message_id(), 2);
    }

    #[test]
    fn checked_terminal_cleanup_rejects_incomplete_router_unbind() {
        let (mut client, probe) = protocol_fixture::responder(PageId(43), Vec::new());
        client.kill();
        assert!(!probe.process_alive());
        client.router.route.lock().unwrap().closing = true;
        assert!(client.checked_terminal_cleanup().is_err());
        assert!(client.router.route.lock().unwrap().closing);
        assert_eq!(probe.next_message_id(), 1);
    }

    #[test]
    fn reaped_worker_reclaims_navigation_grants_but_preserves_live_sibling() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let mut route = fixture_worker_route(&context);
        let sibling = context.new_page();
        let dead = route.network.page_id;
        context
            .prepare_frame_worker_policy(dead, "https://dead.test/", &[], "")
            .unwrap();
        let sibling_token = context
            .prepare_frame_worker_policy(sibling.page_id(), "https://live.test/", &[], "")
            .unwrap();
        let (mut client, probe) = protocol_fixture::responder(dead, Vec::new());
        route.identity = client.route_identity.clone();
        client.router.bind(route).unwrap();
        assert!(client.checked_terminal_cleanup().is_err());
        assert!(!context.navigation_resource_grants_drained(dead).unwrap());
        client.kill();
        assert!(!probe.process_alive());
        client.checked_terminal_cleanup().unwrap();
        client.checked_terminal_cleanup().unwrap();
        assert!(context.navigation_resource_grants_drained(dead).unwrap());
        assert!(
            !context
                .navigation_resource_grants_drained(sibling.page_id())
                .unwrap()
        );
        context
            .discard_prepared_frame_policy(sibling.page_id(), &sibling_token)
            .unwrap();
        assert!(
            context
                .navigation_resource_grants_drained(sibling.page_id())
                .unwrap()
        );
    }

    #[test]
    fn reset_ack_does_not_reclaim_unreleased_navigation_grants() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let page = route.network.page_id;
        let token = context
            .prepare_frame_worker_policy(page, "https://pending.test/", &[], "")
            .unwrap();
        let router = WorkerBrokerRouter::default();
        router.bind(route).unwrap();
        router.unbind().unwrap();
        assert!(router.finish_acknowledged_reset().is_err());
        context.discard_prepared_frame_policy(page, &token).unwrap();
        router.finish_acknowledged_reset().unwrap();
    }

    #[test]
    fn reset_cleanup_recycles_only_after_unbind_and_complete_script_custody() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let router = WorkerBrokerRouter::default();
        router.bind(route.clone()).unwrap();
        assert!(router.finish_acknowledged_reset().is_err());
        let registry = route.context.script_cleanup_registry();
        let token = registry.reserve().unwrap();
        let running = registry.claim(token).unwrap();
        router.unbind().unwrap();
        assert!(running.cancel.load(Ordering::Acquire));
        assert!(router.finish_acknowledged_reset().is_err());
        assert!(
            router
                .script_cleanup_registry(Some(&route.identity), route.network.page_id)
                .is_ok()
        );
        registry.cancel(token).unwrap();
        assert!(router.finish_acknowledged_reset().is_err());
        drop(running);
        router.finish_acknowledged_reset().unwrap();
        assert!(router.route.lock().unwrap().script_cleanup.is_empty());
        assert!(
            router
                .script_cleanup_registry(Some(&route.identity), route.network.page_id)
                .is_err()
        );
        let mut next = fixture_worker_route(&context);
        next.identity =
            crate::worker_route::WorkerRoute::issue(2, next.network.page_id.0, 0).unwrap();
        router.bind(next).unwrap();
    }

    #[test]
    fn reset_cleanup_keeps_all_retired_routes_until_last_sibling_releases_custody() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let first = fixture_worker_route(&context);
        let second = fixture_worker_route(&context);
        let router = WorkerBrokerRouter::default();
        router.bind(first.clone()).unwrap();
        router.bind(second.clone()).unwrap();
        let registry = second.context.script_cleanup_registry();
        let token = registry.reserve().unwrap();
        let running = registry.claim(token).unwrap();
        router.unbind().unwrap();
        assert!(router.finish_acknowledged_reset().is_err());
        for route in [&first, &second] {
            assert!(router.authorized(Some(&route.identity)).is_err());
            assert!(
                router
                    .script_cleanup_registry(Some(&route.identity), route.network.page_id)
                    .is_ok()
            );
        }
        assert_eq!(router.route.lock().unwrap().script_cleanup.len(), 2);
        registry.cancel(token).unwrap();
        assert!(running.cancel.load(Ordering::Acquire));
        assert!(router.finish_acknowledged_reset().is_err());
        drop(running);
        router.finish_acknowledged_reset().unwrap();
        for route in [&first, &second] {
            assert!(
                router
                    .script_cleanup_registry(Some(&route.identity), route.network.page_id)
                    .is_err()
            );
        }
        assert!(router.route.lock().unwrap().script_cleanup.is_empty());
    }

    #[test]
    fn retired_navigation_grant_retains_only_original_stop_and_waits_for_terminal_cleanup() {
        use crate::navigation_control::{
            NavigationCommand, NavigationEvent, NavigationRequest, NavigationStop, PageLoadStrategy,
        };
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let page = route.network.page_id;
        let control = route.context.navigation_cleanup_control().unwrap().unwrap();
        let request = NavigationRequest {
            command: NavigationCommand::Navigate {
                url: "https://navigation.test/".into(),
            },
            strategy: PageLoadStrategy::None,
            timeout_ms: None,
        };
        let first = control.begin(request.clone()).unwrap();
        let router = WorkerBrokerRouter::default();
        router.bind(route.clone()).unwrap();
        router.refresh_navigation_cleanup(&route.identity).unwrap();
        let registry = route.context.script_cleanup_registry();
        let token = registry.reserve().unwrap();
        let running = registry.claim(token).unwrap();
        router.unbind().unwrap();
        assert_eq!(
            first.progress().unwrap().stopped,
            Some(NavigationStop::Retired)
        );
        assert!(control.cleanup_is_drained().unwrap());
        assert!(router.finish_acknowledged_reset().is_err());
        let stop = |id, page_id| ContextServiceRequest::NavigationEvent {
            page_id,
            id,
            event: NavigationEvent::Stopped(NavigationStop::Cancelled),
        };
        let forged = crate::worker_route::WorkerRoute::issue(
            route.identity.process_session,
            route.identity.environment,
            route.identity.generation,
        )
        .unwrap();
        assert!(router.request_routed(None, stop(first.id, page)).is_err());
        assert!(
            router
                .request_routed(Some(&forged), stop(first.id, page))
                .is_err()
        );
        assert!(
            router
                .request_routed(Some(&route.identity), stop(first.id, PageId(page.0 + 1)))
                .is_err()
        );
        assert!(
            router
                .request_routed(
                    Some(&route.identity),
                    ContextServiceRequest::NavigationEvent {
                        page_id: page,
                        id: first.id,
                        event: NavigationEvent::Started
                    }
                )
                .is_err()
        );
        assert!(matches!(
            control.begin(request),
            Err(NavigationStop::Retired)
        ));
        let stopping = stop(first.id, page);
        assert!(
            router
                .response_scope(Some(&route.identity), Some(&stopping))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            router.request_routed(Some(&route.identity), stopping),
            Ok(ContextServiceResponse::NavigationControl {
                result: Err(NavigationStop::Retired)
            })
        ));
        assert!(running.cancel.load(Ordering::Acquire));
        assert!(router.finish_acknowledged_reset().is_err());
        drop(running);
        router.finish_acknowledged_reset().unwrap();
        assert!(
            router
                .request_routed(Some(&route.identity), stop(first.id, page))
                .is_err()
        );
    }

    #[test]
    fn retired_route_finishes_only_its_selected_blank_activation() {
        let context = crate::Browser::native_test_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let source = route.network.page_id;
        let source_page = context.active_page().expect("fixture route source page");
        assert_eq!(source_page.page_id(), source);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                source_page.load_html("https://blank.test/source", "<!doctype html><p>source</p>"),
            )
            .unwrap();
        let creator = context.capture_command_navigation_source(source).unwrap();
        let (destination, _, scope) = context
            .reserve_blank_prerender(source, &creator, "https://blank.test/next")
            .unwrap();
        context
            .commit_prerender_response_policy(destination, &scope, &scope.url, &[])
            .unwrap();
        context
            .mark_native_prerender_loaded(destination, &scope.token, scope.lifetime)
            .unwrap();
        let receipt = context
            .claim_blank_prerender(source, &creator, Some(destination), &scope.url)
            .unwrap()
            .unwrap();
        let admission = route.network.authority.admit(None).unwrap();
        let router = WorkerBrokerRouter::default();
        router.bind(route.clone()).unwrap();
        assert!(
            router
                .unbind_environment(&route.identity, Duration::ZERO)
                .is_err()
        );
        assert!(router.authorized(Some(&route.identity)).is_err());
        let forged = crate::worker_route::WorkerRoute::issue(
            route.identity.process_session,
            route.identity.environment,
            route.identity.generation,
        )
        .unwrap();
        for request in [
            ContextServiceRequest::ActivateBlankPrerender {
                page_id: source,
                activation: receipt.clone(),
            },
            ContextServiceRequest::PublishBlankPrerender {
                page_id: source,
                activation: receipt.clone(),
            },
            ContextServiceRequest::AcknowledgeBlankPrerender {
                page_id: source,
                activation: receipt.clone(),
            },
        ] {
            assert!(
                router
                    .response_scope(Some(&route.identity), Some(&request))
                    .unwrap()
                    .is_none()
            );
            assert!(router.request_routed(None, request.clone()).is_err());
            assert!(
                router
                    .request_routed(Some(&forged), request.clone())
                    .is_err()
            );
            let response = router
                .request_routed(Some(&route.identity), request)
                .unwrap();
            assert!(matches!(
                response,
                ContextServiceResponse::PrerenderActivated
                    | ContextServiceResponse::PrerenderPublished
                    | ContextServiceResponse::PrerenderAcknowledged
            ));
        }
        assert!(!context.hidden_prerender_page(destination));
        assert!(context.navigation_resource_grants_drained(source).unwrap());
        drop(admission);
    }

    #[test]
    fn draining_route_accepts_only_exact_script_cleanup_authority() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let route = fixture_worker_route(&context);
        let registry = route.context.script_cleanup_registry();
        let token = registry.reserve().unwrap();
        let lease = registry.claim(token).unwrap();
        let admission = route.network.authority.admit(None).unwrap();
        let router = WorkerBrokerRouter::default();
        router.bind(route.clone()).unwrap();
        assert!(
            router
                .unbind_environment(&route.identity, Duration::ZERO)
                .is_err()
        );
        assert!(router.authorized(Some(&route.identity)).is_err());
        let request = ContextServiceRequest::CancelScriptLoad {
            page_id: route.network.page_id,
            token,
        };
        assert!(
            router
                .response_scope(Some(&route.identity), Some(&request))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            router.request_routed(Some(&route.identity), request),
            Ok(ContextServiceResponse::ScriptLoadCancelled)
        ));
        assert!(lease.cancel.load(Ordering::Acquire));
        let release = ContextServiceRequest::ReleaseNetworkResult {
            page_id: route.network.page_id,
            creator: None,
            stream_id: Some(999),
            socket_id: None,
        };
        assert!(
            router
                .response_scope(Some(&route.identity), Some(&release))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            router.request_routed(Some(&route.identity), release.clone()),
            Ok(ContextServiceResponse::NetworkResultReleased)
        ));
        let forged = crate::worker_route::WorkerRoute::issue(
            route.identity.process_session,
            route.identity.environment,
            route.identity.generation,
        )
        .unwrap();
        assert!(router.request_routed(Some(&forged), release).is_err());
        for cleanup in [
            ContextServiceRequest::ReleaseFrameResponseStream {
                page_id: route.network.page_id,
                stream_id: 999,
                policy: "absent".into(),
            },
            ContextServiceRequest::DiscardPreparedLocalNavigation {
                page_id: route.network.page_id,
                token: "absent".into(),
            },
        ] {
            assert!(
                router
                    .response_scope(Some(&route.identity), Some(&cleanup))
                    .unwrap()
                    .is_none()
            );
            assert!(router.request_routed(None, cleanup.clone()).is_err());
            assert!(router.request_routed(Some(&forged), cleanup).is_err());
        }
        assert!(
            router
                .navigation_resource_cleanup(
                    Some(&route.identity),
                    PageId(route.network.page_id.0 + 1)
                )
                .is_err()
        );

        assert!(
            router
                .request_routed(
                    Some(&forged),
                    ContextServiceRequest::CancelScriptLoad {
                        page_id: route.network.page_id,
                        token
                    }
                )
                .is_err()
        );
        assert!(
            router
                .request_routed(
                    Some(&route.identity),
                    ContextServiceRequest::CancelScriptLoad {
                        page_id: oryn_common::v3::PageId(route.network.page_id.0 + 1),
                        token
                    }
                )
                .is_err()
        );
        drop(admission);
        router
            .unbind_environment(&route.identity, Duration::from_secs(1))
            .unwrap();
        assert!(router.authorized(Some(&route.identity)).is_err());
    }

    #[test]
    fn old_cleanup_grant_cannot_cancel_same_token_in_replacement_registry() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let old = fixture_worker_route(&context);
        let router = WorkerBrokerRouter::default();
        router.bind(old.clone()).unwrap();
        let old_registry = old.context.script_cleanup_registry();
        let token = old_registry.reserve().unwrap();
        router
            .unbind_environment(&old.identity, Duration::from_secs(1))
            .unwrap();
        let mut replacement = old.clone();
        replacement.identity = crate::worker_route::WorkerRoute::issue(
            old.identity.process_session,
            old.identity.environment,
            old.identity.generation + 1,
        )
        .unwrap();
        replacement.network.authority = Arc::new(RouteAuthority::default());
        replacement.context = ParentContextService::new_with_network(
            context.clone(),
            old.network.page_id,
            Some(old.network.inner.clone()),
        )
        .into_shared();
        let registry = replacement.context.script_cleanup_registry();
        assert_eq!(registry.reserve().unwrap(), token);
        let lease = registry.claim(token).unwrap();
        router.bind(replacement.clone()).unwrap();
        assert!(matches!(
            router.request_routed(
                Some(&old.identity),
                ContextServiceRequest::CancelScriptLoad {
                    page_id: old.network.page_id,
                    token
                }
            ),
            Ok(ContextServiceResponse::ScriptLoadCancelled)
        ));
        assert!(!lease.cancel.load(Ordering::Acquire));
        assert!(router.authorized(Some(&old.identity)).is_err());
        assert!(router.authorized(Some(&replacement.identity)).is_ok());
    }

    #[test]
    fn backpressured_response_does_not_retain_the_context() {
        struct Flood;
        impl NetworkBroker for Flood {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!()
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(32);
                for _ in 0..16 {
                    sender
                        .try_send(NetworkEvent::Chunk {
                            request_id: request.request_id,
                            bytes: vec![b'x'].into(),
                        })
                        .unwrap();
                }
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
        }
        let context = Browser::new().new_context();
        let weak = context.downgrade();
        let mut route = fixture_worker_route(&context);
        route.network.inner = crate::network::SharedNetworkBroker::new(Flood);
        let operation = route
            .network
            .start_operation(NetworkRequest {
                request_id: 99,
                method: "GET".into(),
                url: "https://example.test/data".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: false,
                security: CookieRequestContext::default(),
                resolved_addrs: Vec::new(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while operation.events.len() < 8 {
            assert!(
                std::time::Instant::now() < deadline,
                "response did not fill the bounded queue"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(10));
        drop(context);
        while weak.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "blocked forwarding retained the context"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(operation);
    }

    #[test]
    fn retained_worker_route_does_not_keep_its_browser_context_alive() {
        let context = Browser::new().new_context();
        let weak = context.downgrade();
        let route = fixture_worker_route(&context);
        assert!(route.network.context_owner().is_ok());
        drop(context);
        assert!(weak.upgrade().is_none());
        assert!(route.network.context_owner().is_err());
        assert!(matches!(
            route.context.handle(
                crate::context_service::ContextServiceRequest::GetReferrerPolicy {
                    page_id: oryn_common::v3::PageId(route.identity.environment),
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                }
            ),
            Err(RuntimeError::Closed)
        ));
    }

    #[test]
    fn broker_route_rejects_missing_forged_and_stale_authority() {
        let fixture_context = Browser::new().new_context();
        let router = WorkerBrokerRouter::default();
        let mut misbound = fixture_worker_route(&fixture_context);
        misbound.identity.environment = u64::MAX;
        assert!(router.bind(misbound).is_err());
        let route = fixture_worker_route(&fixture_context);
        let identity = route.identity.clone();
        router.bind(route).unwrap();
        assert!(router.authorized(Some(&identity)).is_ok());
        assert!(router.authorized(None).is_err());
        let forged = crate::worker_route::WorkerRoute::issue(
            identity.process_session,
            identity.environment,
            identity.generation,
        )
        .unwrap();
        assert!(router.authorized(Some(&forged)).is_err());
        for field in 0..3 {
            let mut changed = identity.clone();
            match field {
                0 => changed.process_session += 1,
                1 => changed.environment += 1,
                _ => changed.generation += 1,
            }
            assert!(router.authorized(Some(&changed)).is_err());
        }
        router.unbind().unwrap();
        router.bind(fixture_worker_route(&fixture_context)).unwrap();
        assert!(router.authorized(Some(&identity)).is_err());
    }

    #[test]
    fn related_attachment_rejects_other_context_and_unrelated_opener_without_retiring_source() {
        let owner = Browser::new().new_context();
        let config = BrowserConfig::default();
        let options = ContextOptions::default();
        let route = fixture_worker_route(&owner);
        let source_page = route.network.page_id;
        let source_identity = route.identity.clone();
        let router = Arc::new(WorkerBrokerRouter::default());
        router.bind(route).unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r request"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let monitor = RssMonitor::start(child.id()).unwrap();
        let limits = WorkerLimits::from(&config);
        let mut runtime = crate::runtime_build_info();
        runtime.policy_digest = Some(crate::worker::policy_digest(&options));
        let mut client = WorkerClient {
            pending_initialization: None,
            actor_route: source_identity.clone(),
            attached: true,
            route_identity: source_identity.clone(),
            session: Arc::new(Mutex::new(WorkerProcessSession {
                process_observation: None,
                child,
                input,
                output,
                response_buffer: Vec::new(),
                session_id: 1,
                next_message_id: 1,
                limits: limits.clone(),
                router: router.clone(),
                rss_monitor: monitor.clone(),
            })),
            session_id: 1,
            limits,
            runtime,
            binary_sha256: String::new(),
            verification: None,
            router: router.clone(),
            leases: 1,
            code_directory_hash: None,
        };
        let target = owner.new_page().page_id();
        let unrelated = Browser::new().new_context();
        assert!(
            client
                .attach_environment(&config, &options, target, source_page, unrelated)
                .is_err()
        );
        assert!(
            client
                .attach_environment(&config, &options, target, target, owner.clone())
                .is_err()
        );
        assert!(router.authorized(Some(&source_identity)).is_ok());
        assert_eq!(Arc::strong_count(&client.session), 1);
        assert_eq!(client.session.lock().unwrap().next_message_id, 1);
        // Reserving a child while the opener holds its dispatcher must never
        // wait for Initialize or consume a response from that dispatcher.
        let capability = client.attachment();
        let dispatcher = client.session.lock().unwrap();
        let mut reserved = capability
            .reserve(&config, &options, target, source_page, owner.clone())
            .unwrap();
        assert!(reserved.client.pending_initialization.is_some());
        assert_eq!(reserved.admission.actor_route, source_identity);
        assert!(router.authorized(Some(&reserved.admission.route)).is_ok());
        assert_eq!(dispatcher.next_message_id, 1);
        let child_route = reserved.admission.route.clone();
        assert!(reserved.client.cancel_pending_initialization().unwrap());
        assert!(!reserved.client.cancel_pending_initialization().unwrap());
        assert!(!reserved.client.attached);
        assert_eq!(dispatcher.next_message_id, 1);
        drop(reserved);
        assert!(router.authorized(Some(&child_route)).is_err());
        assert!(router.authorized(Some(&source_identity)).is_ok());
        drop(dispatcher);
        drop(capability);
        assert!(
            client
                .session
                .lock()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert!(!monitor.stop.load(Ordering::Acquire));
        client.kill();
    }

    #[test]
    fn environment_rotation_revokes_old_grant_without_revoking_sibling() {
        let context = Browser::new().new_context();
        let router = WorkerBrokerRouter::default();
        let first = fixture_worker_route(&context);
        let first_identity = first.identity.clone();
        let old_authority = first.network.authority.clone();
        let mut replacement = first.clone();
        replacement.identity = crate::worker_route::WorkerRoute::issue(
            first_identity.process_session,
            first_identity.environment,
            first_identity.generation + 1,
        )
        .unwrap();
        replacement.network.authority = Arc::new(RouteAuthority::default());
        replacement.context = ParentContextService::new_with_network(
            context.clone(),
            replacement.network.page_id,
            Some(replacement.network.inner.clone()),
        )
        .into_shared();
        let new_identity = replacement.identity.clone();
        let sibling = fixture_worker_route(&context);
        let sibling_identity = sibling.identity.clone();
        router.bind(first).unwrap();
        router.bind(sibling).unwrap();
        assert!(router.current().is_err());
        assert!(router.authorized(Some(&first_identity)).is_ok());
        assert!(router.authorized(Some(&sibling_identity)).is_ok());
        router
            .rotate_environment(&first_identity, replacement, Duration::from_secs(1))
            .unwrap();
        assert!(old_authority.ensure_live().is_err());
        assert!(router.authorized(Some(&first_identity)).is_err());
        assert!(router.authorized(Some(&new_identity)).is_ok());
        assert!(router.authorized(Some(&sibling_identity)).is_ok());
        router
            .unbind_environment(&sibling_identity, Duration::from_secs(1))
            .unwrap();
        assert!(router.authorized(Some(&sibling_identity)).is_err());
        assert!(router.authorized(Some(&new_identity)).is_ok());
        router.unbind().unwrap();
    }

    #[test]
    fn process_drain_revokes_every_sibling_before_waiting_for_first_admission() {
        let context = Browser::new().new_context();
        let router = Arc::new(WorkerBrokerRouter::default());
        let first = fixture_worker_route(&context);
        let admission = first.network.authority.admit(None).unwrap();
        let second = fixture_worker_route(&context);
        let second_authority = second.network.authority.clone();
        router.bind(first).unwrap();
        router.bind(second).unwrap();
        let retiring = router.clone();
        let task = std::thread::spawn(move || retiring.unbind());
        let state = second_authority.state.lock().unwrap();
        let (state, _) = second_authority
            .drained
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.retired)
            .unwrap();
        assert!(
            state.retired,
            "later sibling must be revoked while first admission is still held"
        );
        drop(state);
        assert!(second_authority.admit(None).is_err());
        drop(admission);
        task.join().unwrap().unwrap();
    }

    #[test]
    fn rotating_environment_reserves_identity_until_replacement_is_published() {
        let context = Browser::new().new_context();
        let router = Arc::new(WorkerBrokerRouter::default());
        let first = fixture_worker_route(&context);
        let identity = first.identity.clone();
        let authority = first.network.authority.clone();
        let admission = authority.admit(None).unwrap();
        let mut replacement = first.clone();
        replacement.identity =
            crate::worker_route::WorkerRoute::issue(1, identity.environment, 1).unwrap();
        replacement.network.authority = Arc::new(RouteAuthority::default());
        let next_identity = replacement.identity.clone();
        let competing = replacement.clone();
        let mut malformed = replacement.clone();
        malformed.network.page_id = PageId(u64::MAX);
        router.bind(first).unwrap();
        assert!(
            router
                .rotate_environment(&identity, malformed, Duration::ZERO)
                .is_err()
        );
        assert!(authority.ensure_live().is_ok());
        let rotating = router.clone();
        let old_identity = identity.clone();
        let task = std::thread::spawn(move || {
            rotating.rotate_environment(&old_identity, replacement, Duration::from_secs(2))
        });
        let state = authority.state.lock().unwrap();
        let (state, _) = authority
            .drained
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.retired)
            .unwrap();
        assert!(state.retired);
        drop(state);
        assert!(router.bind(competing).is_err());
        assert!(router.authorized(Some(&identity)).is_err());
        assert!(router.authorized(Some(&next_identity)).is_err());
        drop(admission);
        task.join().unwrap().unwrap();
        assert!(router.authorized(Some(&next_identity)).is_ok());
        router.unbind().unwrap();
    }

    #[test]
    fn failed_environment_drain_leaves_sibling_live_and_prevents_reuse() {
        let context = Browser::new().new_context();
        let router = WorkerBrokerRouter::default();
        let first = fixture_worker_route(&context);
        let identity = first.identity.clone();
        let admission = first.network.authority.admit(None).unwrap();
        let duplicate = first.clone();
        let sibling = fixture_worker_route(&context);
        let sibling_identity = sibling.identity.clone();
        router.bind(first).unwrap();
        router.bind(sibling).unwrap();
        assert!(
            router
                .unbind_environment(&identity, Duration::ZERO)
                .is_err()
        );
        assert!(router.authorized(Some(&identity)).is_err());
        assert!(router.authorized(Some(&sibling_identity)).is_ok());
        drop(admission);
        assert!(router.bind(duplicate).is_err());
        router.unbind().unwrap();
        assert!(
            router.route.lock().unwrap().environments.is_empty(),
            "completed admission permits terminal process drain"
        );
        assert!(router.authorized(Some(&sibling_identity)).is_err());
    }

    #[test]
    fn environment_registry_bounds_capacity_and_rejects_other_processes() {
        let context = Browser::new().new_context();
        let router = WorkerBrokerRouter::default();
        let first = fixture_worker_route(&context);
        let first_identity = first.identity.clone();
        router.bind(first).unwrap();
        let mut foreign = fixture_worker_route(&context);
        foreign.identity =
            crate::worker_route::WorkerRoute::issue(2, foreign.network.page_id.0, 0).unwrap();
        assert!(router.bind(foreign).is_err());
        for _ in 1..MAX_PROCESS_ENVIRONMENTS {
            router.bind(fixture_worker_route(&context)).unwrap();
        }
        assert!(matches!(
            router.bind(fixture_worker_route(&context)),
            Err(RuntimeError::WorkerLimitExceeded { .. })
        ));
        assert!(router.authorized(Some(&first_identity)).is_ok());
        router.unbind().unwrap();
    }

    #[test]
    fn context_wire_keeps_old_clients_bound_to_revoked_authority() {
        let fixture_context = Browser::new().new_context();
        use crate::context_service::{IpcContextClient, SharedContextClient};
        let router = Arc::new(WorkerBrokerRouter::default());
        let old_route = fixture_worker_route(&fixture_context);
        let old_identity = old_route.identity.clone();
        let old_page = old_route.network.page_id;
        router.bind(old_route).unwrap();
        let (worker, parent) = UnixStream::pair().unwrap();
        let server_router = router.clone();
        let server = std::thread::spawn(move || serve_context_transport(parent, server_router));
        let unbound = SharedContextClient::new(IpcContextClient::new(worker));
        let old_client = unbound.for_route(old_identity);
        assert!(
            old_client
                .request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: old_page,
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                })
                .is_ok()
        );
        assert!(
            unbound
                .request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: old_page,
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                })
                .is_err()
        );
        router.unbind().unwrap();
        let replacement = fixture_worker_route(&fixture_context);
        let new_page = replacement.network.page_id;
        let new_client = unbound.for_route(replacement.identity.clone());
        router.bind(replacement).unwrap();
        // Even supplying the new page ID cannot upgrade a retained old client.
        assert!(
            old_client
                .request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: new_page,
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                })
                .is_err()
        );
        assert!(
            new_client
                .request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: new_page,
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                })
                .is_ok()
        );
        drop(old_client);
        drop(new_client);
        drop(unbound);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn draining_router_rejects_rebinding_until_every_admission_has_ended() {
        let fixture_context = Browser::new().new_context();
        let router = Arc::new(WorkerBrokerRouter::default());
        let route = fixture_worker_route(&fixture_context);
        let authority = route.network.authority.clone();
        let admission = authority.admit(None).unwrap();
        router.bind(route).unwrap();
        let draining = router.clone();
        let thread = std::thread::spawn(move || draining.unbind());
        let state = authority.state.lock().unwrap();
        let (state, _) = authority
            .drained
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.retired)
            .unwrap();
        assert!(state.retired);
        drop(state);
        assert!(router.current().is_err());
        assert!(router.bind(fixture_worker_route(&fixture_context)).is_err());
        drop(admission);
        thread.join().unwrap().unwrap();
        router.bind(fixture_worker_route(&fixture_context)).unwrap();
        assert!(authority.ensure_live().is_err());
        router.unbind().unwrap();
    }

    #[test]
    fn concurrent_router_retirement_waits_for_existing_cleanup_owner() {
        let context = Browser::new().new_context();
        let router = Arc::new(WorkerBrokerRouter::default());
        let route = fixture_worker_route(&context);
        let authority = route.network.authority.clone();
        let admission = authority.admit(None).unwrap();
        router.bind(route).unwrap();
        let first = router.clone();
        let first = std::thread::spawn(move || first.unbind());
        let state = authority.state.lock().unwrap();
        let (state, _) = authority
            .drained
            .wait_timeout_while(state, Duration::from_secs(1), |state| !state.retired)
            .unwrap();
        assert!(state.retired);
        drop(state);
        let (entered, entry) = std::sync::mpsc::channel();
        let (returned, result) = std::sync::mpsc::channel();
        let second = router.clone();
        let second = std::thread::spawn(move || {
            entered.send(()).unwrap();
            returned.send(second.unbind()).unwrap();
        });
        entry.recv_timeout(Duration::from_secs(1)).unwrap();
        let early = result.recv_timeout(Duration::from_millis(50));
        // Release before asserting so failed regressions never leave a drain blocked.
        drop(admission);
        first.join().unwrap().unwrap();
        second.join().unwrap();
        assert!(
            matches!(early, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
            "concurrent unbind returned before original cleanup settled: {early:?}"
        );
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(router.route.lock().unwrap().environments.is_empty());
    }

    #[test]
    fn concurrent_router_drain_budget_expires_without_releasing_owner() {
        let owner = RouterDrainOwner::default();
        let guard = owner.acquire(Duration::ZERO).unwrap();
        assert!(owner.acquire(Duration::ZERO).is_err());
        assert!(*owner.busy.lock().unwrap());
        drop(guard);
        assert!(owner.acquire(Duration::ZERO).is_ok());
    }

    #[test]
    fn failed_drain_permanently_retires_router_even_after_late_completion() {
        let fixture_context = Browser::new().new_context();
        let router = WorkerBrokerRouter::default();
        let route = fixture_worker_route(&fixture_context);
        let admission = route.network.authority.admit(None).unwrap();
        router.bind(route).unwrap();
        assert!(router.unbind_with_timeout(Duration::ZERO).is_err());
        drop(admission);
        assert!(router.bind(fixture_worker_route(&fixture_context)).is_err());
        assert!(router.unbind().is_err());
    }

    #[test]
    fn unbound_worker_router_rejects_page_network_authority() {
        let router = WorkerBrokerRouter::default();
        let error = router
            .send(NetworkRequest {
                request_id: 1,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: false,
                security: CookieRequestContext::default(),
                resolved_addrs: Vec::new(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("no unambiguous live page route"));
    }

    #[test]
    fn unbinding_revokes_context_even_when_a_broker_thread_retains_a_clone() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let service = ParentContextService::new_with_network(
            context.clone(),
            page.page_id(),
            Some(network.clone()),
        )
        .into_shared();
        let router = WorkerBrokerRouter::default();
        router
            .bind(BoundWorkerRoute {
                identity: crate::worker_route::WorkerRoute::issue(1, page.page_id().0, 0).unwrap(),
                network: PageNetworkBroker {
                    authority: Arc::new(RouteAuthority::default()),
                    inner: network,
                    context: context.downgrade(),
                    page_id: page.page_id(),
                },
                context: service.clone(),
            })
            .unwrap();
        let retained = router.current().unwrap();
        router.unbind().unwrap();
        assert!(router.current().is_err());
        assert!(retained.network.ensure_bound().is_err());
        let error = retained
            .context
            .handle(ContextServiceRequest::GetReferrerPolicy {
                page_id: page.page_id(),
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
            })
            .unwrap_err();
        assert!(error.to_string().contains("shutting down"));
    }

    #[test]
    fn worker_path_must_have_the_expected_bundle_layout() {
        let valid = Path::new("/tmp/OrynPageWorker.app/Contents/MacOS/oryn-page-worker");
        assert_eq!(
            containing_app(valid).unwrap(),
            PathBuf::from("/tmp/OrynPageWorker.app")
        );
        assert!(containing_app(Path::new("/tmp/oryn-page-worker")).is_err());
    }

    #[test]
    fn oil_batch_timeout_is_bounded_per_command_without_weakening_action_limits() {
        let limits = WorkerLimits::default();
        let batch = WorkerRequest::Oil {
            input: "# comment\nobserve\nclick \"Save\"\n\nwait visible css(#done)".into(),
        };
        assert_eq!(
            request_timeout(&batch, &limits),
            Duration::from_millis(limits.navigation_wall_ms * 3)
        );
        assert_eq!(
            request_timeout(
                &WorkerRequest::Evaluate {
                    source: "for(;;){}".into()
                },
                &limits
            ),
            Duration::from_millis(limits.command_wall_ms)
        );
        let oversized = WorkerRequest::Oil {
            input: std::iter::repeat_n("observe", limits.queue_capacity + 10)
                .collect::<Vec<_>>()
                .join("\n"),
        };
        assert_eq!(
            request_timeout(&oversized, &limits),
            Duration::from_millis(limits.navigation_wall_ms * limits.queue_capacity as u64)
        );
    }
}

/// Shared attested executable selection for an AgentId-only child role. The
/// caller transfers agent capability sockets, never page-worker descriptors.
pub(crate) fn prepare_agent_command() -> Result<(Command, PathBuf, String), RuntimeError> {
    let discovered = discover_worker()?;
    let verification = if requires_trusted_worker() {
        Some(verify_worker(&discovered)?)
    } else {
        None
    };
    let path = verification
        .as_ref()
        .map(|value| value.path.clone())
        .unwrap_or(discovered);
    let hash = verification
        .as_ref()
        .map(|value| value.binary_sha256.clone())
        .map(Ok)
        .unwrap_or_else(|| sha256_file(&path))?;
    let mut command = Command::new(&path);
    #[cfg(target_os = "linux")]
    if is_linux_multicall_worker(&path) {
        command.arg("__oryn-page-worker");
    }
    command
        .env_clear()
        .env("ORYN_TRUSTED_WORKER_SHA256", &hash)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok((command, path, hash))
}
pub(crate) fn verify_agent_process(pid: u32, path: &Path) -> Result<(), RuntimeError> {
    #[cfg(target_os = "macos")]
    {
        verify_running_worker(pid, path)?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, path);
    }
    Ok(())
}

/// Encoded protocol peer for ownership regressions. It validates complete host
/// envelopes before publishing each response; it never starts the engine.
#[cfg(test)]
pub(crate) mod protocol_fixture {
    use super::*;
    pub(crate) struct Probe {
        session: Arc<Mutex<WorkerProcessSession>>,
    }
    impl Probe {
        pub(crate) fn hold_script_cleanup(&self) -> crate::script_load_lease::ScriptLoadLease {
            let session = self.session.lock().unwrap();
            let routes = session.router.route.lock().unwrap();
            let registry = &routes.script_cleanup.values().next().unwrap().1;
            let token = registry.reserve().unwrap();
            registry.claim(token).unwrap()
        }
        pub(crate) fn enforce_zero_rss_limit(&self) {
            let mut session = self.session.lock().unwrap();
            assert!(session.rss_monitor.resident_bytes.load(Ordering::Acquire) > 0);
            session.limits.worker_rss_bytes = 0;
        }
        pub(crate) fn kill_and_unbind(&self) {
            self.session.lock().unwrap().kill();
        }
        pub(crate) fn unbind_while_alive(&self) {
            self.session.lock().unwrap().router.unbind().unwrap();
        }
        pub(crate) fn next_message_id(&self) -> u64 {
            self.session.lock().unwrap().next_message_id
        }
        pub(crate) fn process_alive(&self) -> bool {
            self.session
                .lock()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_none()
        }
    }
    fn quoted(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
    pub(crate) fn responder_for_context(
        context: BrowserContext,
        page: PageId,
        steps: Vec<(WorkerRequest, WorkerResponse)>,
    ) -> (WorkerClient, Probe) {
        let (mut client, probe) = responder(page, steps);
        client.runtime.policy_digest =
            Some(crate::worker::policy_digest(&ContextOptions::default()));
        client.limits = WorkerLimits::from(&BrowserConfig::default());
        client.session.lock().unwrap().limits = client.limits.clone();
        let network = context
            .shared_network_broker()
            .expect("native test network broker");
        client
            .router
            .bind(BoundWorkerRoute {
                identity: client.route_identity.clone(),
                context: ParentContextService::new_with_network(
                    context.clone(),
                    page,
                    Some(network.clone()),
                )
                .into_shared(),
                network: PageNetworkBroker {
                    authority: Arc::new(RouteAuthority::default()),
                    inner: network.for_page(page.0).new_request_scope(),
                    context: context.downgrade(),
                    page_id: page,
                },
            })
            .unwrap();
        (client, probe)
    }

    pub(crate) fn responder(
        page: PageId,
        steps: Vec<(WorkerRequest, WorkerResponse)>,
    ) -> (WorkerClient, Probe) {
        let route = crate::worker_route::WorkerRoute::issue(7, page.0, 0).unwrap();
        let mut script = String::new();
        for (index, (command, response)) in steps.into_iter().enumerate() {
            let message_id = index as u64 + 1;
            let expected = serde_json::to_string(&HostEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                route: Some(route.clone()),
                session_id: 7,
                message_id,
                command,
            })
            .unwrap();
            let response = serde_json::to_string(&WorkerEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                route: Some(route.clone()),
                session_id: 7,
                message_id,
                correlation_id: message_id,
                response,
            })
            .unwrap();
            script.push_str(&format!("IFS= read -r request || exit 91\n[ \"$request\" = {} ] || exit 92\nprintf '%s\\n' {}\n",quoted(&expected),quoted(&response)));
        }
        // Keep the peer alive until the final test owner drops the session.
        script.push_str("IFS= read -r unexpected\nexit 93\n");
        let mut child = Command::new("/bin/sh")
            .args(["-c", &script])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let monitor = RssMonitor::start(child.id()).unwrap();
        let router = Arc::new(WorkerBrokerRouter::default());
        let limits = WorkerLimits::default();
        let session = Arc::new(Mutex::new(WorkerProcessSession {
            process_observation: None,
            child,
            input,
            output,
            response_buffer: Vec::new(),
            session_id: 7,
            next_message_id: 1,
            limits: limits.clone(),
            router: router.clone(),
            rss_monitor: monitor,
        }));
        let client = WorkerClient {
            pending_initialization: None,
            actor_route: route.clone(),
            attached: false,
            route_identity: route,
            session: session.clone(),
            session_id: 7,
            limits,
            runtime: crate::runtime_build_info(),
            binary_sha256: String::new(),
            verification: None,
            router,
            leases: 1,
            code_directory_hash: None,
        };
        // attached=false suppresses unrelated Drop protocol, but request() uses
        // the real encoder, pipes, deadline/correlation validator and decoder.
        (client, Probe { session })
    }
}

#[cfg(test)]
mod protocol_metadata_admission_tests {
    #[test]
    fn worker_render_contract_does_not_require_a_local_parent_renderer() {
        let mut worker_report = crate::runtime_build_info();
        #[cfg(not(any(feature = "v8-host", feature = "accessibility-host")))]
        assert_eq!(worker_report.render_contract_version, None);

        // Version 3 is the supported worker display-list protocol even when
        // this test's parent has no V8 or renderer implementation linked.
        worker_report.render_contract_version = Some(3);
        assert!(super::worker_render_contract_matches(
            worker_report.render_contract_version
        ));
        for unsupported in [None, Some(0), Some(2), Some(4), Some(u16::MAX)] {
            worker_report.render_contract_version = unsupported;
            assert!(!super::worker_render_contract_matches(
                worker_report.render_contract_version
            ));
        }
    }

    #[test]
    fn worker_ready_requires_compiled_context_and_agent_versions() {
        let runtime = crate::runtime_build_info();
        assert!(super::runtime_routing_protocols_match(&runtime));
        for version in [
            None,
            Some(crate::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION + 1),
        ] {
            let mut changed = runtime.clone();
            changed.context_service_protocol_version = version;
            assert!(!super::runtime_routing_protocols_match(&changed));
        }
        for version in [None, Some(crate::agent_ipc::VERSION + 1)] {
            let mut changed = runtime.clone();
            changed.agent_ipc_protocol_version = version;
            assert!(!super::runtime_routing_protocols_match(&changed));
        }
    }
}
