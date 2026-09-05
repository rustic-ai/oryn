#![cfg(unix)]

use std::{
    fs,
    os::{
        fd::{AsRawFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use oryn_common::v3::PageId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BrowserConfig, BrowserContext, ContextOptions, RuntimeBuildInfo,
    context_service::{ParentContextService, serve_context_service},
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

#[derive(Clone)]
struct PageNetworkBroker {
    inner: crate::network::SharedNetworkBroker,
    context: BrowserContext,
    page_id: PageId,
}

impl PageNetworkBroker {
    fn prepare(
        &self,
        mut request: NetworkRequest,
    ) -> Result<(NetworkRequest, String), NetworkError> {
        let page_url = self
            .context
            .page_url(self.page_id)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let target = url::Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        if let Some(upgraded) = self
            .context
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
                .context
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
        let page_url = self
            .context
            .page_url(self.page_id)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let target = url::Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        if let Some(upgraded) = self
            .context
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
                .context
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
            self.context
                .prepare_document_security(self.page_id, request.url, &response.headers, &html)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
        }
        Ok(response)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (request, page_url) = self.prepare(request)?;
        let request_id = request.request_id;
        let request_kind = request.security.kind;
        let request_url = request.url.clone();
        let context = self.context.clone();
        let page_id = self.page_id;
        let mut operation = self.inner.start_operation(request)?;
        let demand = operation.demand.clone();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        std::thread::spawn(move || {
            let mut response_headers = Vec::new();
            let mut body = Vec::new();
            let mut prepared_meta_count = 0_usize;
            while let Some(event) = operation.events.blocking_recv() {
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
                if sender.blocking_send(event).is_err() {
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
        self.inner.cancel_operation(request_id)
    }
}

const BROKER_FD: RawFd = 3;
const CONTEXT_FD: RawFd = 4;
const EXPECTED_BUNDLE_ID: &str = "ai.rustic.oryn.page-worker";
const EXPECTED_TEAM_ID: &str = "5HVA9VFF8K";
const EXPECTED_SIGNING_CERT_SHA1: &str = "9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90";
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_CERTIFICATE_DIRECTORY_ID: AtomicU64 = AtomicU64::new(1);

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
}

fn request_timeout(request: &WorkerRequest, limits: &WorkerLimits) -> Duration {
    match request {
        WorkerRequest::Goto { .. }
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
        _ => Duration::from_millis(limits.command_wall_ms),
    }
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
        drop(child_broker);
        drop(child_context);

        let parent_network = context_owner.shared_network_broker().ok_or_else(|| {
            RuntimeError::SandboxUnavailable("parent network broker is unavailable".into())
        })?;
        let broker_network = PageNetworkBroker {
            inner: parent_network.clone(),
            context: context_owner.clone(),
            page_id,
        };
        let frame_limit = config.max_ipc_frame_bytes;
        std::thread::Builder::new()
            .name("oryn-worker-network-broker".into())
            .spawn(move || {
                let _ = serve_ipc_broker(parent_broker, broker_network, frame_limit);
            })
            .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
        std::thread::Builder::new()
            .name("oryn-worker-context-service".into())
            .spawn(move || {
                let _ = serve_context_service(
                    parent_context,
                    ParentContextService::new_with_network(
                        context_owner,
                        page_id,
                        Some(parent_network),
                    ),
                );
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
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let limits = WorkerLimits::from(config);
        let nonce = handshake_nonce(session_id);
        let mut client = Self {
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

    fn request_with_timeout(
        &mut self,
        request: WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerResponse, RuntimeError> {
        let message_id = self.next_message_id;
        self.next_message_id += 1;
        let envelope = HostEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            session_id: self.session_id,
            message_id,
            command: request,
        };
        let encoded = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if encoded.len() > self.limits.ipc_frame_bytes {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "IPC frame".into(),
            });
        }
        self.write_frame(&encoded, timeout)?;

        let response = self.read_frame(timeout)?;
        let response: WorkerEnvelope = serde_json::from_slice(&response)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != WORKER_PROTOCOL_VERSION
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
            WorkerResponse::Error { error } => Err(*error),
            WorkerResponse::LimitExceeded { resource } => {
                self.kill();
                Err(RuntimeError::WorkerLimitExceeded { resource })
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
            if resident_bytes(self.child.id()).is_some_and(|rss| rss > self.limits.worker_rss_bytes)
            {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "RSS".into(),
                });
            }
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
            if let Some(newline) = self.response_buffer.iter().position(|byte| *byte == b'\n') {
                return Ok(self.response_buffer.drain(..=newline).collect());
            }
            if self.response_buffer.len() > self.limits.ipc_frame_bytes {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "IPC frame".into(),
                });
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
            if resident_bytes(self.child.id()).is_some_and(|rss| rss > self.limits.worker_rss_bytes)
            {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "RSS".into(),
                });
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.kill();
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let poll_ms = remaining.min(Duration::from_millis(250)).as_millis() as i32;
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(&app)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    if !verify.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            String::from_utf8_lossy(&verify.stderr).trim().to_string(),
        ));
    }
    let details = Command::new("/usr/bin/codesign")
        .args(["-dvvv"])
        .arg(&path)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let details = String::from_utf8_lossy(&details.stderr);
    let bundle_id = detail_value(&details, "Identifier=").unwrap_or_default();
    let team_id = detail_value(&details, "TeamIdentifier=").unwrap_or_default();
    if bundle_id != EXPECTED_BUNDLE_ID || team_id != EXPECTED_TEAM_ID {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker identity mismatch: bundle={bundle_id}, team={team_id}"
        )));
    }
    if !details.contains("Authority=Apple Development:")
        || !details.contains("flags=0x10000(runtime)")
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker lacks the trusted Apple Development chain or hardened runtime".into(),
        ));
    }
    let signing_cert_sha1 = signing_certificate_sha1(&path)?;
    if signing_cert_sha1 != EXPECTED_SIGNING_CERT_SHA1 {
        return Err(RuntimeError::SandboxUnavailable(format!(
            "worker signing certificate mismatch: {signing_cert_sha1}"
        )));
    }
    let entitlements = Command::new("/usr/bin/codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(&path)
        .output()
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    let entitlements = String::from_utf8_lossy(&entitlements.stdout);
    let expected = std::collections::BTreeSet::from([
        "com.apple.security.app-sandbox".to_string(),
        "com.apple.security.cs.allow-jit".to_string(),
    ]);
    if entitlement_keys(&entitlements) != expected
        || expected
            .iter()
            .any(|key| !entitlement_is_true(&entitlements, key))
    {
        return Err(RuntimeError::SandboxUnavailable(
            "worker entitlement set is not exactly app-sandbox plus allow-jit".into(),
        ));
    }

    let manifest_path = app.join("Contents/Resources/worker-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
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
    Ok(WorkerVerification {
        path,
        bundle_id,
        team_id,
        signing_cert_sha1,
        binary_sha256,
        manifest_sha256: format!("{:x}", Sha256::digest(manifest_bytes)),
        app_sandbox: true,
        allow_jit: true,
        hardened_runtime: true,
    })
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

fn detail_value(details: &str, prefix: &str) -> Option<String> {
    details
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn entitlement_keys(xml: &str) -> std::collections::BTreeSet<String> {
    xml.split("<key>")
        .skip(1)
        .filter_map(|part| part.split_once("</key>").map(|(key, _)| key.to_string()))
        .collect()
}

fn entitlement_is_true(xml: &str, key: &str) -> bool {
    xml.split_once(&format!("<key>{key}</key>"))
        .map(|(_, remainder)| {
            let value = remainder.split("<key>").next().unwrap_or(remainder);
            value.contains("<true/>") || value.contains("<true />")
        })
        .unwrap_or(false)
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

fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let bytes =
        fs::read(path).map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn signing_certificate_sha1(path: &Path) -> Result<String, RuntimeError> {
    use sha1::{Digest, Sha1};

    let directory = create_certificate_directory()?;
    let prefix = directory.join("certificate-");
    let extraction = Command::new("/usr/bin/codesign")
        .arg("-d")
        .arg(format!("--extract-certificates={}", prefix.display()))
        .arg(path)
        .output();
    let extraction = match extraction {
        Ok(extraction) => extraction,
        Err(error) => {
            let _ = fs::remove_dir_all(&directory);
            return Err(RuntimeError::SandboxUnavailable(error.to_string()));
        }
    };
    let leaf_path = PathBuf::from(format!("{}0", prefix.display()));
    let leaf = fs::read(&leaf_path);
    let trust = Command::new("/usr/bin/security")
        .args(["verify-cert", "-c"])
        .arg(&leaf_path)
        .args(["-p", "codeSign"])
        .output();
    let _ = fs::remove_dir_all(&directory);
    if !extraction.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            "unable to extract worker signing certificate".into(),
        ));
    }
    let leaf = leaf.map_err(|error| {
        RuntimeError::SandboxUnavailable(format!(
            "unable to read extracted signing certificate: {error}"
        ))
    })?;
    let trust = trust.map_err(|error| RuntimeError::SandboxUnavailable(error.to_string()))?;
    if !trust.status.success() {
        return Err(RuntimeError::SandboxUnavailable(
            "worker signing certificate is not trusted for code signing".into(),
        ));
    }
    Ok(format!("{:X}", Sha1::digest(leaf)))
}

fn create_certificate_directory() -> Result<PathBuf, RuntimeError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let sequence = NEXT_CERTIFICATE_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "oryn-worker-cert-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(RuntimeError::SandboxUnavailable(error.to_string())),
        }
    }
    Err(RuntimeError::SandboxUnavailable(
        "unable to allocate a unique certificate-verification directory".into(),
    ))
}

fn handshake_nonce(session_id: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{session_id}-{nanos}", std::process::id())
}

fn resident_bytes(pid: u32) -> Option<usize> {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kib = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .ok()?;
    Some(kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
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
            inner: crate::network::SharedNetworkBroker::new(CaptureBroker {
                requests: requests.clone(),
            }),
            context: context.clone(),
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
    fn extracts_codesign_detail_values() {
        let details = "Identifier=ai.rustic.oryn.page-worker\nTeamIdentifier=5HVA9VFF8K\n";
        assert_eq!(
            detail_value(details, "Identifier=").as_deref(),
            Some(EXPECTED_BUNDLE_ID)
        );
        assert_eq!(
            detail_value(details, "TeamIdentifier=").as_deref(),
            Some(EXPECTED_TEAM_ID)
        );
    }

    #[test]
    fn entitlement_parser_requires_exact_true_keys() {
        let xml = "<dict><key>com.apple.security.app-sandbox</key><true/><key>com.apple.security.cs.allow-jit</key><true/></dict>";
        assert_eq!(entitlement_keys(xml).len(), 2);
        assert!(entitlement_is_true(xml, "com.apple.security.app-sandbox"));
        assert!(!entitlement_is_true(
            xml,
            "com.apple.security.network.client"
        ));
        assert_eq!(expected_entitlement_sha256().len(), 64);
        assert_eq!(compiled_policy_sha256().len(), 64);
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

    #[test]
    fn certificate_verification_directories_are_unique_under_concurrency() {
        let threads = (0..32)
            .map(|_| std::thread::spawn(create_certificate_directory))
            .collect::<Vec<_>>();
        let mut directories = threads
            .into_iter()
            .map(|thread| {
                thread
                    .join()
                    .expect("join allocator")
                    .expect("allocate directory")
            })
            .collect::<Vec<_>>();
        directories.sort();
        directories.dedup();
        assert_eq!(directories.len(), 32);
        for directory in directories {
            fs::remove_dir(directory).expect("remove certificate directory");
        }
    }
}
