#![cfg(unix)]

use std::{
    collections::{BTreeSet, HashMap},
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
    cookies::{CookieRequestContext, CookieRequestKind},
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

const EXPECTED_STYLE_ENGINE_VERSION: &str = oryn_common::STYLE_ENGINE_VERSION;
const EXPECTED_BOX_ENGINE_VERSION: &str = "taffy-0.14";
const EXPECTED_INLINE_ENGINE_VERSION: &str = "parley-0.11.1";

#[derive(Default)]
struct RouteAdmissions {
    retired: bool,
    active: usize,
    network_requests: BTreeSet<u64>,
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

#[derive(Clone)]
struct PageNetworkBroker {
    authority: Arc<RouteAuthority>,
    inner: crate::network::SharedNetworkBroker,
    context: crate::runtime::WeakBrowserContext,
    page_id: PageId,
}

impl PageNetworkBroker {
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
    ) -> Result<(NetworkRequest, String), NetworkError> {
        self.ensure_bound()?;
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
        request.security = CookieRequestContext {
            kind: request.security.kind,
            initiator_url: Some(page_url.clone()),
            top_level_url: Some(page_url.clone()),
            ancestor_urls: request.security.ancestor_urls,
            top_level_navigation: request.security.top_level_navigation,
            credentials: request.security.credentials,
            redirect_chain_cross_site: request.security.redirect_chain_cross_site,
        };
        Ok((request, page_url))
    }
}

impl NetworkBroker for PageNetworkBroker {
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let _admission = self.authority.admit(Some(request.request_id))?;
        self.ensure_bound()?;
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
        request.security = CookieRequestContext {
            kind: request.security.kind,
            initiator_url: Some(page_url.clone()),
            top_level_url: Some(page_url.clone()),
            ancestor_urls: request.security.ancestor_urls,
            top_level_navigation: request.security.top_level_navigation,
            credentials: request.security.credentials,
            redirect_chain_cross_site: request.security.redirect_chain_cross_site,
        };
        let response = self.inner.send(request.clone())?;
        self.ensure_bound()?;
        if request.security.kind == CookieRequestKind::Frame {
            let frame_policy =
                crate::csp::CspPolicySet::from_headers(request.url.clone(), &response.headers);
            let decision = frame_policy.authorize_frame_ancestors(&[page_url]);
            if !decision.allowed {
                return Err(NetworkError::Transport(
                    "Content-Security-Policy frame-ancestors blocked the embedded document".into(),
                ));
            }
        }
        if request.security.kind == CookieRequestKind::Document
            && !(300..400).contains(&response.status)
        {
            let content_type = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str());
            let (html, _) = oryn_net::decode_resource_text(
                &response.body,
                content_type,
                oryn_net::TextResourceKind::Html,
            );
            self.context_owner()?
                .prepare_document_security(self.page_id, request.url, &response.headers, &html)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
        }
        Ok(response)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let admission = self.authority.admit(Some(request.request_id))?;
        let (request, page_url) = self.prepare(request)?;
        let request_id = request.request_id;
        let request_kind = request.security.kind;
        let request_url = request.url.clone();
        let context = self.context.clone();
        let page_id = self.page_id;
        let route = self.clone();
        let mut operation = self.inner.start_operation(request)?;
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        std::thread::spawn(move || {
            let _admission = admission;
            let mut response_headers = Vec::new();
            let mut body = Vec::new();
            let mut prepared_meta_count = 0_usize;
            while let Some(event) = operation.events.blocking_recv() {
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
                    let _ = sender.blocking_send(NetworkEvent::Failed {
                        request_id,
                        error: error.to_string(),
                    });
                    return;
                }
                if let NetworkEvent::Headers {
                    status, headers, ..
                } = &event
                {
                    response_headers = headers.clone();
                    if request_kind == CookieRequestKind::Document
                        && !(300..400).contains(status)
                        && let Err(error) = context.prepare_document_security(
                            page_id,
                            request_url.clone(),
                            &response_headers,
                            "",
                        )
                    {
                        let _ = sender.blocking_send(NetworkEvent::Failed {
                            request_id,
                            error: error.to_string(),
                        });
                        return;
                    }
                    if request_kind == CookieRequestKind::Frame {
                        let policy = crate::csp::CspPolicySet::from_headers(
                            request_url.clone(),
                            &response_headers,
                        );
                        if !policy
                            .authorize_frame_ancestors(std::slice::from_ref(&page_url))
                            .allowed
                        {
                            let _ = sender.blocking_send(NetworkEvent::Failed {
                                request_id,
                                error: "Content-Security-Policy frame-ancestors blocked the embedded document".into(),
                            });
                            return;
                        }
                    }
                }
                if let NetworkEvent::Chunk { bytes, .. } = &event
                    && request_kind == CookieRequestKind::Document
                {
                    body.extend_from_slice(bytes);
                    let content_type = response_headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                        .map(|(_, value)| value.as_str());
                    let (html, _) = oryn_net::decode_resource_text(
                        &body,
                        content_type,
                        oryn_net::TextResourceKind::Html,
                    );
                    let meta_policies = crate::csp::CspPolicySet::meta_policies_from_html(&html);
                    for serialized in meta_policies.into_iter().skip(prepared_meta_count) {
                        if let Err(error) =
                            context.append_document_meta_security(page_id, serialized)
                        {
                            let _ = sender.blocking_send(NetworkEvent::Failed {
                                request_id,
                                error: error.to_string(),
                            });
                            return;
                        }
                        prepared_meta_count = prepared_meta_count.saturating_add(1);
                    }
                }
                let terminal = matches!(
                    event,
                    NetworkEvent::Complete { .. } | NetworkEvent::Failed { .. }
                );
                if sender.blocking_send(event).is_err() || terminal {
                    return;
                }
            }
        });
        Ok(NetworkOperation {
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        let _admission = self.authority.admit(None)?;
        let owned = self
            .authority
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("worker route lock poisoned".into()))?
            .network_requests
            .contains(&request_id);
        if owned {
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

#[derive(Default)]
enum WorkerRouteState {
    #[default]
    Unbound,
    Bound(BoundWorkerRoute),
    Draining,
}

#[derive(Default)]
struct WorkerBrokerRouter {
    route: Mutex<WorkerRouteState>,
}

impl WorkerBrokerRouter {
    fn authorized(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
    ) -> Result<BoundWorkerRoute, RuntimeError> {
        let route = self.current()?;
        if identity != Some(&route.identity) {
            return Err(RuntimeError::ProtocolViolation(
                "missing or stale broker route authority".into(),
            ));
        }
        Ok(route)
    }
    fn bind(&self, route: BoundWorkerRoute) -> Result<(), RuntimeError> {
        if route.identity.environment != route.network.page_id.0 {
            return Err(RuntimeError::ProtocolViolation(
                "broker route environment does not match its page owner".into(),
            ));
        }
        let mut current = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        if !matches!(*current, WorkerRouteState::Unbound) {
            return Err(RuntimeError::ProtocolViolation(
                "worker broker router is already bound".into(),
            ));
        }
        route
            .network
            .ensure_bound()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        *current = WorkerRouteState::Bound(route);
        Ok(())
    }

    fn unbind(&self) -> Result<(), RuntimeError> {
        self.unbind_with_timeout(Duration::from_secs(2))
    }

    fn unbind_with_timeout(&self, timeout: Duration) -> Result<(), RuntimeError> {
        let mut current = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        let previous = match std::mem::replace(&mut *current, WorkerRouteState::Draining) {
            WorkerRouteState::Bound(previous) => previous,
            WorkerRouteState::Unbound => {
                *current = WorkerRouteState::Unbound;
                return Ok(());
            }
            WorkerRouteState::Draining => {
                return Err(RuntimeError::ContextService(
                    "worker route is still draining".into(),
                ));
            }
        };
        let requests = previous.network.authority.revoke()?;
        drop(current);
        previous.context.cancel_all_network_operations();
        for request in requests {
            let _ = previous.network.inner.cancel_operation(request);
        }
        // A failed drain leaves the router permanently unavailable. The caller
        // kills this worker rather than issuing Reset or admitting a new lease.
        previous.network.authority.wait_drained(timeout)?;
        // An operation admitted before revocation could have registered a
        // resource while the first cleanup ran. No such operation remains now.
        previous.context.cancel_all_network_operations();
        *self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))? =
            WorkerRouteState::Unbound;
        Ok(())
    }

    fn current(&self) -> Result<BoundWorkerRoute, RuntimeError> {
        let current = self
            .route
            .lock()
            .map_err(|_| RuntimeError::ContextService("worker router lock poisoned".into()))?;
        match &*current {
            WorkerRouteState::Bound(route) => Ok(route.clone()),
            _ => Err(RuntimeError::ContextService(
                "worker is not bound to a page".into(),
            )),
        }
    }
}

impl NetworkBroker for WorkerBrokerRouter {
    fn start_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        self.authorized(identity)
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .start_operation(request)
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
        self.current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .send(request)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        self.current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .start_operation(request)
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.current()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .network
            .cancel_operation(request_id)
    }
}

impl NetworkBroker for Arc<WorkerBrokerRouter> {
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
    fn request_routed(
        &self,
        identity: Option<&crate::worker_route::WorkerRoute>,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let route = self.authorized(identity)?;
        let _admission = route
            .network
            .authority
            .admit(None)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        ContextTransport::request(&*route.context, request)
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
        ContextTransport::request(&*route.context, request)
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
    schema_version: u32,
    bundle_id: String,
    team_id: String,
    signing_cert_sha1: String,
    protocol_version: u32,
    v8_version: String,
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

pub struct WorkerClient {
    route_identity: crate::worker_route::WorkerRoute,
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    response_buffer: Vec<u8>,
    session_id: u64,
    next_message_id: u64,
    limits: WorkerLimits,
    runtime: RuntimeBuildInfo,
    binary_sha256: String,
    verification: Option<WorkerVerification>,
    router: Arc<WorkerBrokerRouter>,
    leases: usize,
    rss_monitor: Arc<RssMonitor>,
    code_directory_hash: Option<String>,
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
        WorkerRequest::Goto { .. }
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

/// Fixed labels only: request payloads may contain credentials and must never
/// be included in timeout diagnostics, even for initialization or OIL batches.
fn request_diagnostic_label(request: &WorkerRequest) -> &'static str {
    match request {
        WorkerRequest::Initialize { .. } => "initialize",
        WorkerRequest::Ping => "ping",
        WorkerRequest::Goto { .. } => "goto",
        WorkerRequest::GotoResource { .. } => "goto_resource",
        WorkerRequest::Back => "back",
        WorkerRequest::Forward => "forward",
        WorkerRequest::Refresh => "refresh",
        WorkerRequest::LoadHtml { .. } => "load_html",
        WorkerRequest::Evaluate { .. } => "evaluate",
        WorkerRequest::Oil { .. } => "oil",
        WorkerRequest::Reset { .. } => "reset",
        WorkerRequest::Close => "close",
        WorkerRequest::Exit => "exit",
        WorkerRequest::WaitFor { .. } => "wait_for",
        WorkerRequest::Execute { .. } => "execute",
        WorkerRequest::ExecuteBatch { .. } => "execute_batch",
        WorkerRequest::SetFiles { .. } => "set_files",
        _ => "query_or_update",
    }
}

fn contextualize_worker_deadline(
    error: RuntimeError,
    phase: &'static str,
    command: &'static str,
    message_id: u64,
) -> RuntimeError {
    match error {
        RuntimeError::WorkerLimitExceeded { resource } if resource.contains("wall time") => {
            RuntimeError::WorkerLimitExceeded {
                resource: format!(
                    "{resource} (phase={phase}, command={command}, message_id={message_id})"
                ),
            }
        }
        error => error,
    }
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
    pub(crate) fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn spawn(
        config: &BrowserConfig,
        context: &ContextOptions,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        context_owner: BrowserContext,
    ) -> Result<Self, RuntimeError> {
        let session_id = allocate_session_id(&NEXT_SESSION_ID)?;
        let route_identity = crate::worker_route::WorkerRoute::issue(
            session_id,
            page_id.0,
            config.document_generation_base,
        )?;
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

        let (parent_broker, child_broker) = UnixStream::pair()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let (parent_context, child_context) = UnixStream::pair()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        let child_fd = child_broker.as_raw_fd();
        let child_context_fd = child_context.as_raw_fd();
        let mut command = Command::new(launch_path);
        #[cfg(target_os = "linux")]
        if is_linux_multicall_worker(launch_path) {
            command.arg("__oryn-page-worker");
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
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        #[cfg(target_os = "macos")]
        if let Err(error) = verify_running_worker(child.id(), launch_path) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        #[cfg(target_os = "macos")]
        let launch_code_directory_hash = Some(running_code_directory_hash(child.id())?);
        #[cfg(not(target_os = "macos"))]
        let launch_code_directory_hash: Option<String> = None;
        let rss_monitor = match RssMonitor::start(child.id()) {
            Ok(monitor) => monitor,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        drop(child_broker);
        drop(child_context);

        let parent_network = context_owner.shared_network_broker().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent network broker is unavailable".into())
        })?;
        let parent_network = parent_network.for_worker(child.id()).for_page(page_id.0);
        let broker_network = PageNetworkBroker {
            authority: Arc::new(RouteAuthority::default()),
            inner: parent_network.clone().for_page(page_id.0),
            context: context_owner.downgrade(),
            page_id,
        };
        let router = Arc::new(WorkerBrokerRouter::default());
        router.bind(BoundWorkerRoute {
            identity: route_identity.clone(),
            network: broker_network,
            context: Arc::new(ParentContextService::new_with_network(
                context_owner,
                page_id,
                Some(parent_network),
            )),
        })?;
        let frame_limit = config.max_ipc_frame_bytes;
        let network_router = router.clone();
        std::thread::Builder::new()
            .name("oryn-worker-network-broker".into())
            .spawn(move || {
                let _ = serve_ipc_broker(parent_broker, network_router, frame_limit);
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
        let mut client = Self {
            route_identity,
            child,
            input,
            output,
            response_buffer: Vec::new(),
            session_id,
            next_message_id: 1,
            limits: limits.clone(),
            runtime: crate::runtime_build_info(),
            binary_sha256: String::new(),
            verification,
            router,
            leases: 1,
            rss_monitor,
            code_directory_hash: launch_code_directory_hash.clone(),
        };
        let response = client.request_with_timeout(
            WorkerRequest::Initialize {
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
            || !runtime.native_v8
            || runtime.v8_version.is_none()
            || runtime.contract_version != oryn_common::v4::CONTRACT_VERSION
            || runtime.runtime_profile != config.runtime_profile.capability_name()
            || runtime.binding_abi_version != oryn_js::BINDING_ABI_VERSION
            || !runtime.v8_heap_sandbox
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
            || code_identity.process_id != client.child.id()
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
        client.binary_sha256 = binary_sha256;
        Ok(client)
    }

    pub fn request(&mut self, request: WorkerRequest) -> Result<WorkerResponse, RuntimeError> {
        let timeout = request_timeout(&request, &self.limits);
        self.request_with_timeout(request, timeout)
    }

    pub fn runtime(&self) -> &RuntimeBuildInfo {
        &self.runtime
    }

    pub fn verification(&self) -> Option<&WorkerVerification> {
        self.verification.as_ref()
    }

    pub(crate) fn reset(&mut self) -> Result<(), RuntimeError> {
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
            || code_identity.process_id != self.child.id()
            || code_identity.binary_sha256 != self.binary_sha256
            || code_identity.code_directory_hash != self.code_directory_hash
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "worker identity changed during reset".into(),
            ));
        }
        self.session_id = match allocate_session_id(&NEXT_SESSION_ID) {
            Ok(id) => id,
            Err(error) => {
                self.kill();
                return Err(error);
            }
        };
        self.next_message_id = 1;
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
        let identity = crate::worker_route::WorkerRoute::issue(
            self.session_id,
            page_id.0,
            config.document_generation_base,
        )?;
        let parent_network = context_owner.shared_network_broker().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent network broker is unavailable".into())
        })?;
        let parent_network = parent_network
            .for_worker(self.child.id())
            .for_page(page_id.0);
        self.router.bind(BoundWorkerRoute {
            identity: identity.clone(),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner: parent_network.clone().for_page(page_id.0),
                context: context_owner.downgrade(),
                page_id,
            },
            context: Arc::new(ParentContextService::new_with_network(
                context_owner,
                page_id,
                Some(parent_network),
            )),
        })?;
        self.route_identity = identity;
        let nonce = handshake_nonce(self.session_id);
        let result = self.request_with_timeout(
            WorkerRequest::Initialize {
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
                "recycled worker did not return an initialization response".into(),
            ));
        };
        if returned_nonce != nonce
            || protocol_version != WORKER_PROTOCOL_VERSION
            || !runtime.native_v8
            || runtime.v8_version != self.runtime.v8_version
            || runtime.contract_version != self.runtime.contract_version
            || runtime.binding_abi_version != self.runtime.binding_abi_version
            || runtime.v8_heap_sandbox != self.runtime.v8_heap_sandbox
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
            || code_identity.process_id != self.child.id()
            || code_identity.binary_sha256 != self.binary_sha256
            || code_identity.code_directory_hash != self.code_directory_hash
        {
            self.kill();
            return Err(RuntimeError::ProtocolViolation(
                "recycled worker handshake metadata changed".into(),
            ));
        }
        self.runtime.policy_digest = Some(policy_digest);
        self.runtime.worker_lifecycle = config.worker_lifecycle.name().into();
        self.leases = self.leases.saturating_add(1);
        Ok(())
    }

    pub(crate) fn leases(&self) -> usize {
        self.leases
    }

    pub(crate) fn mark_prepared(&mut self) {
        self.leases = 0;
    }

    fn request_with_timeout(
        &mut self,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let result = self.exchange_with_timeout(request, timeout);
        // Enforce retirement here, not only in WorkerPage. Initialization,
        // reset, and direct callers must also discard malformed or failed
        // transports, including JSON parse errors and unexpected EOF.
        if matches!(
            &result,
            Err(RuntimeError::ProtocolViolation(_)
                | RuntimeError::WorkerCrashed(_)
                | RuntimeError::WorkerLimitExceeded { .. })
        ) {
            self.kill();
        }
        result
    }

    fn exchange_with_timeout(
        &mut self,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let command = request_diagnostic_label(&request);
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
        let envelope = HostEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            route: Some(self.route_identity.clone()),
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

        let response = self.read_frame(timeout).map_err(|error| {
            contextualize_worker_deadline(error, "ipc_response", command, message_id)
        })?;
        let response: WorkerEnvelope = serde_json::from_slice(&response)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != WORKER_PROTOCOL_VERSION
            || response.route.as_ref() != Some(&self.route_identity)
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

    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, RuntimeError> {
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
                return Err(RuntimeError::WorkerCrashed(format!(
                    "worker exited with {status}"
                )));
            }
            self.check_rss()?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let poll_ms = remaining.min(Duration::from_millis(10)).as_millis() as i32;
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    let client = WorkerClient::spawn(
        config,
        context,
        PageId::LEGACY,
        None,
        crate::Browser::in_process_with_config(config.clone())
            .new_context_with_options(context.clone()),
    )?;
    let mut runtime = client.runtime().clone();
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
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.request_with_timeout(WorkerRequest::Exit, Duration::from_secs(1));
        }
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

    let manifest: WorkerManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let binary_sha256 = sha256_file(&path)?;
    if manifest.schema_version != 1
        || manifest.bundle_id != EXPECTED_BUNDLE_ID
        || manifest.team_id != EXPECTED_TEAM_ID
        || manifest.signing_cert_sha1 != EXPECTED_SIGNING_CERT_SHA1
        || manifest.protocol_version != WORKER_PROTOCOL_VERSION
        || manifest.limits != WorkerLimits::default()
        || manifest.v8_version.is_empty()
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

#[cfg(not(target_os = "macos"))]
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
    fn deadline_context_never_formats_request_payloads() {
        for (request, expected) in [
            (
                WorkerRequest::Goto {
                    url: "https://private.invalid/?token=secret".into(),
                },
                "goto",
            ),
            (
                WorkerRequest::LoadHtml {
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
                route_identity: crate::worker_route::WorkerRoute::issue(1, 1, 0).unwrap(),
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
                runtime: crate::runtime_build_info(),
                binary_sha256: String::new(),
                verification: None,
                router: Arc::new(WorkerBrokerRouter::default()),
                leases: 0,
                rss_monitor,
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
                client.child.try_wait().unwrap().is_some(),
                "failed peer must be reaped before returning"
            );
            assert!(client.rss_monitor.stop.load(Ordering::Acquire));
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
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/html".into())],
                body: br#"<meta http-equiv="content-security-policy" content="script-src 'none'">"#
                    .to_vec(),
            })
        }
    }

    #[test]
    fn parent_derives_request_provenance_and_fails_closed_on_omitted_meta_policy() {
        let context = Browser::new().new_context();
        let page = context.new_page();
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
        }
        impl NetworkBroker for HeldOperationBroker {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!("asynchronous operation required")
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(1);
                *self.sender.lock().unwrap() = Some(sender);
                Ok(NetworkOperation {
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
        let mut route = fixture_worker_route(&fixture_context);
        route.network.inner = crate::network::SharedNetworkBroker::new(HeldOperationBroker {
            sender: Mutex::new(None),
            cancelled: cancelled.clone(),
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
        // An unknown cancellation must not be forwarded to another route's work.
        router.cancel_operation(18).unwrap();
        assert!(cancelled.lock().unwrap().is_empty());
        router.unbind().unwrap();
        assert_eq!(*cancelled.lock().unwrap(), [17]);
        assert_eq!(authority.state.lock().unwrap().active, 0);
        drop(operation);
        router.bind(fixture_worker_route(&fixture_context)).unwrap();
        router.unbind().unwrap();
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
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        BoundWorkerRoute {
            identity: crate::worker_route::WorkerRoute::issue(1, page.page_id().0, 0).unwrap(),
            context: Arc::new(ParentContextService::new_with_network(
                context.clone(),
                page.page_id(),
                Some(network.clone()),
            )),
            network: PageNetworkBroker {
                authority: Arc::new(RouteAuthority::default()),
                inner: network,
                context: context.downgrade(),
                page_id: page.page_id(),
            },
        }
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
                .request(ContextServiceRequest::GetReferrerPolicy { page_id: old_page })
                .is_ok()
        );
        assert!(
            unbound
                .request(ContextServiceRequest::GetReferrerPolicy { page_id: old_page })
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
                .request(ContextServiceRequest::GetReferrerPolicy { page_id: new_page })
                .is_err()
        );
        assert!(
            new_client
                .request(ContextServiceRequest::GetReferrerPolicy { page_id: new_page })
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
        assert!(error.to_string().contains("not bound to a page"));
    }

    #[test]
    fn unbinding_revokes_context_even_when_a_broker_thread_retains_a_clone() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let network = crate::network::SharedNetworkBroker::new(CaptureBroker {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let service = Arc::new(ParentContextService::new_with_network(
            context.clone(),
            page.page_id(),
            Some(network.clone()),
        ));
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
