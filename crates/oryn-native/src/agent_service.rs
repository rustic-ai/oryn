//! Parent dispatch for native Window worker clients and independent AgentId
//! services. Every operation resolves immutable authority from the registry.

/// Inputs for load blob script.
struct BlobScriptLoad<'a> {
    context: &'a BrowserContext,
    owner: &'a AgentOwner,
    url: &'a url::Url,
    policy: &'a AgentPolicy,
    kind: crate::agent_script::ScriptLoadKind,
    referrer: &'a str,
    cancel: &'a AtomicBool,
    request_destination: &'a str,
}
use crate::{
    agent_platform_service::AgentPlatformService,
    agent_registry::AgentOwner,
    network::SharedNetworkBroker,
    runtime::{BrowserContext, WeakBrowserContext},
};
use oryn_common::v3::PageId;
use oryn_js::{message_wire::WindowTarget, worker_wire::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
fn error(message: impl std::fmt::Display) -> WorkerError {
    WorkerError::new("InvalidStateError", message.to_string())
}

pub(crate) struct NativeAgentBackend {
    context: WeakBrowserContext,
    bootstrap: AgentBootstrap,
    network: SharedNetworkBroker,
    retired: AtomicBool,
    platform: AgentPlatformService,
    worklet_credentials: std::sync::Mutex<std::collections::BTreeMap<String, WorkerCredentials>>,
}
impl NativeAgentBackend {
    fn new(
        context: &BrowserContext,
        bootstrap: AgentBootstrap,
        network: SharedNetworkBroker,
    ) -> Self {
        let network = network.new_request_scope();
        let driver = Arc::new(crate::agent_io::AgentIo::new(
            context.downgrade(),
            bootstrap.clone(),
            network.clone(),
        ));
        Self {
            worklet_credentials: std::sync::Mutex::new(std::collections::BTreeMap::from([(
                bootstrap.script_url.clone(),
                bootstrap.options.credentials,
            )])),
            context: context.downgrade(),
            bootstrap,
            network,
            retired: AtomicBool::new(false),
            platform: AgentPlatformService::new(driver),
        }
    }
}
impl WorkerTransport for NativeAgentBackend {
    fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
        if let AgentServiceRequest::PlatformReleaseResult {
            stream_id,
            socket_id,
        } = &request
        {
            let result = match (stream_id, socket_id) {
                (Some(stream_id), None) => oryn_js::platform::PlatformHostResult::HttpStarted {
                    timing: None,
                    internal_response: None,
                    response_type: None,
                    stream_id: *stream_id,
                    upload: None,
                    cors_filtered: false,
                    redirected: false,
                    status: 0,
                    url: String::new(),
                    headers: Vec::new(),
                },
                (None, Some(socket_id)) => oryn_js::platform::PlatformHostResult::WebSocketOpened {
                    socket_id: *socket_id,
                    protocol: String::new(),
                },
                _ => return Err(error("invalid platform resource release")),
            };
            self.platform
                .reclaim_completions(&[oryn_js::platform::PlatformHostCompletion {
                    operation_id: oryn_js::platform::HostOperationId(0),
                    result: Ok(result),
                }]);
            return Ok(AgentServiceResponse::Done);
        }
        if matches!(&request, AgentServiceRequest::PlatformRetire) {
            self.platform.retire();
            return Ok(AgentServiceResponse::Done);
        }
        // Cleanup retains the bound native principal after live execution is
        // revoked; it never consults an author-supplied owner or re-admits work.
        if let AgentServiceRequest::SharedMemory { creator, request } = &request
            && matches!(
                request,
                oryn_js::shared_memory::SharedMemoryRequest::CancelAdmission { .. }
                    | oryn_js::shared_memory::SharedMemoryRequest::Release { .. }
            )
        {
            if !matches!(creator,Creator::Agent(id) if *id==self.bootstrap.id) {
                return Err(error("foreign cleanup principal"));
            }
            return match self.context.upgrade() {
                Some(context) => context
                    .shared_memory_request(&AgentOwner::Agent(self.bootstrap.id), request.clone())
                    .map(AgentServiceResponse::SharedMemory),
                None => Ok(AgentServiceResponse::SharedMemory(
                    oryn_js::shared_memory::SharedMemoryResponse {
                        isolated: false,
                        outcome: Default::default(),
                        descriptor: None,
                        #[cfg(unix)]
                        native_descriptor: None,
                    },
                )),
            };
        }
        if self.retired.load(Ordering::Acquire) {
            return Err(error("agent service retired"));
        }
        let context = self
            .context
            .upgrade()
            .ok_or_else(|| error("agent context retired"))?;
        context.admit_context_response().map_err(error)?;
        context.worker_bootstrap(self.bootstrap.id)?;
        match request {
            AgentServiceRequest::SharedMemory { creator, request } => {
                if !matches!(creator,Creator::Agent(id) if id==self.bootstrap.id) {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "foreign shared memory principal",
                    ));
                }
                context
                    .shared_memory_request(&AgentOwner::Agent(self.bootstrap.id), request)
                    .map(AgentServiceResponse::SharedMemory)
            }
            AgentServiceRequest::PublishSecureContext { creator, secure } => {
                publish_secure_context(
                    &context,
                    AgentOwner::Agent(self.bootstrap.id),
                    creator,
                    secure,
                )
            }
            AgentServiceRequest::Service(request) => service_request(
                &context,
                AgentOwner::Agent(self.bootstrap.id),
                request,
                self.network.clone(),
            )
            .map(AgentServiceResponse::Service),
            request @ (AgentServiceRequest::BlobRegister { .. }
            | AgentServiceRequest::BlobResolve { .. }
            | AgentServiceRequest::BlobRevoke { .. }) => {
                blob_request(&context, AgentOwner::Agent(self.bootstrap.id), request)
            }
            AgentServiceRequest::Create(creation) => {
                if !matches!(&creation.creator,Creator::Agent(id) if *id==self.bootstrap.id) {
                    return Err(WorkerError::new("SecurityError", "foreign agent creator"));
                }
                create(
                    &context,
                    AgentOwner::Agent(self.bootstrap.id),
                    creation,
                    self.network.clone(),
                    &self.retired,
                )
                .map(AgentServiceResponse::Connected)
            }
            AgentServiceRequest::TrustedTypes(check) => {
                let bytes = check.payload_bytes();
                if bytes > 8 * 1024 * 1024 {
                    return Err(error("Trusted Types check exceeds source capacity"));
                }
                let policy = crate::csp::CspPolicySet::from_agent_policy(&self.bootstrap.policy);
                let decision = policy.check_trusted_types(&check);
                context.report_worker_csp(
                    &AgentOwner::Agent(self.bootstrap.id),
                    &policy,
                    decision.violations,
                    self.bootstrap
                        .top_level_url
                        .as_deref()
                        .unwrap_or(&self.bootstrap.script_url),
                    false,
                )?;
                context.worker_bootstrap(self.bootstrap.id)?;
                Ok(AgentServiceResponse::TrustedTypes {
                    allowed: decision.allowed,
                })
            }
            AgentServiceRequest::CspEvents { after } => context
                .poll_agent_csp_events(self.bootstrap.id, after)
                .map(AgentServiceResponse::CspEvents),
            request @ (AgentServiceRequest::ImportScript { .. }
            | AgentServiceRequest::ImportModule { .. }
            | AgentServiceRequest::ImportWorkletModule { .. }) => {
                let credentials = match &request {
                    AgentServiceRequest::ImportWorkletModule { credentials, .. } => {
                        if self.bootstrap.kind != AgentKind::AudioWorklet {
                            return Err(error("worklet module request on non-worklet route"));
                        }
                        *credentials
                    }
                    AgentServiceRequest::ImportModule {
                        referrer_source, ..
                    } if self.bootstrap.kind == AgentKind::AudioWorklet => self
                        .worklet_credentials
                        .lock()
                        .map_err(error)?
                        .get(referrer_source)
                        .copied()
                        .unwrap_or(self.bootstrap.options.credentials),
                    _ => self.bootstrap.options.credentials,
                };
                let (url, kind, referrer_source, referrer_policy) = match request {
                    AgentServiceRequest::ImportScript { url, module } => (
                        url,
                        if module {
                            crate::agent_script::ScriptLoadKind::ModuleImport
                        } else {
                            crate::agent_script::ScriptLoadKind::ClassicImport
                        },
                        None,
                        self.bootstrap.referrer_policy.clone(),
                    ),
                    AgentServiceRequest::ImportModule {
                        url,
                        kind,
                        referrer_source,
                        referrer_policy,
                    } => (
                        url,
                        match kind {
                            ImportedModuleType::JavaScript => {
                                crate::agent_script::ScriptLoadKind::ModuleImport
                            }
                            ImportedModuleType::Json => {
                                crate::agent_script::ScriptLoadKind::JsonModuleImport
                            }
                            ImportedModuleType::Text => {
                                crate::agent_script::ScriptLoadKind::TextModuleImport
                            }
                        },
                        Some(referrer_source),
                        referrer_policy,
                    ),
                    AgentServiceRequest::ImportWorkletModule {
                        url,
                        referrer_source,
                        referrer_policy,
                        ..
                    } => (
                        url,
                        crate::agent_script::ScriptLoadKind::ModuleImport,
                        Some(referrer_source),
                        referrer_policy,
                    ),
                    _ => unreachable!(),
                };
                let resolved = url::Url::parse(&self.bootstrap.script_url)
                    .and_then(|base| base.join(&url))
                    .map_err(error)?;
                let services = context.existing_service_registry();
                if let Some(services) = &services {
                    services.ensure_agent_controller(&self.bootstrap)?;
                }
                let owner = AgentOwner::Agent(self.bootstrap.id);
                let report =
                    |policy: &crate::csp::CspPolicySet,
                     violations: Vec<crate::csp::CspViolation>| {
                        context.report_worker_csp(
                            &owner,
                            policy,
                            violations,
                            self.bootstrap
                                .top_level_url
                                .as_deref()
                                .unwrap_or(&self.bootstrap.script_url),
                            false,
                        )
                    };
                let coep_report = |reports| {
                    let _ = context.deliver_coep_reports(
                        self.bootstrap
                            .top_level_url
                            .as_deref()
                            .unwrap_or(&self.bootstrap.script_url),
                        reports,
                    );
                };
                if self.bootstrap.kind == AgentKind::Service
                    && let Some(services) = &services
                    && let Some((source, metadata)) =
                        services.imported_script(self.bootstrap.id, resolved.as_str(), kind)?
                {
                    let policy =
                        crate::csp::CspPolicySet::from_agent_policy(&self.bootstrap.policy);
                    let decision = policy.authorize(&crate::csp::CspRequest {
                        destination: crate::csp::CspDestination::Script,
                        url: metadata.response_url.clone(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: false,
                        initiated_by_trusted_script: true,
                    });
                    report(&policy, decision.violations)?;
                    if !decision.allowed {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "stored service import blocked by worker CSP",
                        )
                        .with_script_failure(ScriptFailureKind::Fetch));
                    }
                    return Ok(AgentServiceResponse::Script {
                        url: metadata.response_url,
                        source,
                        referrer_policy: metadata.referrer_policy,
                    });
                }
                let controller = if self.bootstrap.kind == AgentKind::Service {
                    None
                } else {
                    services.as_deref().map(|services| (services, &owner))
                };
                let loaded = if self.bootstrap.kind == AgentKind::Service {
                    let services = services
                        .as_ref()
                        .ok_or_else(|| error("service registry unavailable for import"))?;
                    crate::agent_script::load_service(
                        &self.network,
                        crate::agent_script::ScriptAuthority {
                            hash_report: None,
                            worker_destination: if self.bootstrap.kind == AgentKind::AudioWorklet {
                                "audioworklet"
                            } else {
                                "worker"
                            },
                            coep_report: Some(&coep_report),
                            report: Some(&report),
                            cookie_ancestor_urls: &self.bootstrap.cookie_ancestor_urls,
                            referrer_source: referrer_source.as_deref(),
                            origin: &self.bootstrap.origin,
                            policy: &self.bootstrap.policy,
                            script_url: &self.bootstrap.script_url,
                            referrer_policy: &referrer_policy,
                            top_level_url: self.bootstrap.top_level_url.as_deref(),
                            credentials,
                        },
                        &url,
                        kind,
                        &self.retired,
                        services.service_import_revalidation(self.bootstrap.id)?,
                    )?
                } else if resolved.scheme() == "blob" {
                    load_blob_script(BlobScriptLoad {
                        context: &context,
                        owner: &AgentOwner::Agent(self.bootstrap.id),
                        url: &resolved,
                        policy: &self.bootstrap.policy,
                        kind,
                        referrer: &referrer_policy,
                        cancel: &self.retired,
                        request_destination: if self.bootstrap.kind == AgentKind::AudioWorklet {
                            "audioworklet"
                        } else {
                            "script"
                        },
                    })?
                } else {
                    crate::agent_script::load_controlled(
                        &self.network,
                        crate::agent_script::ScriptAuthority {
                            hash_report: None,
                            worker_destination: if self.bootstrap.kind == AgentKind::AudioWorklet {
                                "audioworklet"
                            } else {
                                "worker"
                            },
                            coep_report: Some(&coep_report),
                            report: Some(&report),
                            cookie_ancestor_urls: &self.bootstrap.cookie_ancestor_urls,
                            referrer_source: referrer_source.as_deref(),
                            origin: &self.bootstrap.origin,
                            policy: &self.bootstrap.policy,
                            script_url: &self.bootstrap.script_url,
                            referrer_policy: &referrer_policy,
                            top_level_url: self.bootstrap.top_level_url.as_deref(),
                            credentials,
                        },
                        &url,
                        kind,
                        &self.retired,
                        controller,
                    )?
                };
                context.worker_bootstrap(self.bootstrap.id)?;
                if self.bootstrap.kind == AgentKind::AudioWorklet {
                    let mut entries = self.worklet_credentials.lock().map_err(error)?;
                    if entries.len() >= 4096 && !entries.contains_key(&loaded.url) {
                        return Err(error("worklet module credential registry bound"));
                    }
                    entries.insert(resolved.to_string(), credentials);
                    entries.insert(loaded.url.clone(), credentials);
                }
                if self.bootstrap.kind == AgentKind::Service
                    && let Some(services) = &services
                {
                    services.record_import(
                        self.bootstrap.id,
                        resolved.to_string(),
                        loaded.source.clone(),
                        loaded.digest,
                        loaded.import_metadata(kind, referrer_source.as_deref(), &referrer_policy),
                    )?;
                }
                Ok(AgentServiceResponse::Script {
                    url: loaded.url,
                    source: loaded.source,
                    referrer_policy: loaded.referrer_policy,
                })
            }
            AgentServiceRequest::Message(request) => context
                .agent_message_request(self.bootstrap.id, request)
                .map(AgentServiceResponse::Message),
            AgentServiceRequest::PlatformSubmit {
                operation_id,
                request,
            } => {
                self.platform.submit(operation_id, request)?;
                Ok(AgentServiceResponse::Done)
            }
            AgentServiceRequest::PlatformPoll { max } => self
                .platform
                .poll(max)
                .map(AgentServiceResponse::PlatformCompletions),
            AgentServiceRequest::PlatformCancel { operation_id } => {
                self.platform.cancel(operation_id)?;
                Ok(AgentServiceResponse::Done)
            }
            AgentServiceRequest::PlatformRetire
            | AgentServiceRequest::PlatformReleaseResult { .. } => unreachable!(),
            request => client_request(&context, AgentOwner::Agent(self.bootstrap.id), request),
        }
    }
    fn reclaim_response(
        &self,
        scope: ServiceResponseScope,
        response: &AgentServiceResponse,
    ) -> Result<(), WorkerError> {
        if let AgentServiceResponse::PlatformCompletions(completions) = response {
            self.platform.reclaim_completions(completions);
        }
        if let Some(context) = self.context.upgrade() {
            for action in crate::context_service::agent_actions(scope, response) {
                context.reclaim_context_response(
                    crate::context_service::ResponseSource::Agent(self.bootstrap.id),
                    action,
                );
            }
        }
        Ok(())
    }
    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.platform.retire();
        if let Some(context) = self.context.upgrade() {
            context.clear_owner_blob_urls(&AgentOwner::Agent(self.bootstrap.id));
            context.retire_worker_secure_context(&AgentOwner::Agent(self.bootstrap.id));
        }
    }
    fn drain(&self, timeout: std::time::Duration) -> Result<(), WorkerError> {
        let platform = self.platform.drain(timeout);
        let responses = self
            .context
            .upgrade()
            .map(|context| context.retry_context_response_reclaims().map_err(error))
            .transpose();
        platform.and(responses.map(|_| ()))
    }
}

pub(crate) fn window_request(
    context: &BrowserContext,
    page: PageId,
    request: AgentServiceRequest,
    network: SharedNetworkBroker,
    cancel: &AtomicBool,
) -> Result<AgentServiceResponse, WorkerError> {
    match request {
        AgentServiceRequest::PublishSecureContext { creator, secure } => publish_secure_context(
            context,
            AgentOwner::Window {
                page,
                target: WindowTarget::Root,
                generation: 0,
                epoch: 0,
            },
            creator,
            secure,
        ),
        AgentServiceRequest::Service(request) => service_request(
            context,
            AgentOwner::Window {
                page,
                target: WindowTarget::Root,
                generation: 0,
                epoch: 0,
            },
            request,
            network,
        )
        .map(AgentServiceResponse::Service),
        request @ (AgentServiceRequest::BlobRegister { .. }
        | AgentServiceRequest::BlobResolve { .. }
        | AgentServiceRequest::BlobRevoke { .. }) => blob_request(
            context,
            AgentOwner::Window {
                page,
                target: WindowTarget::Root,
                generation: 0,
                epoch: 0,
            },
            request,
        ),
        AgentServiceRequest::Create(creation) => {
            let Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = &creation.creator
            else {
                return Err(WorkerError::new(
                    "SecurityError",
                    "Window route cannot impersonate an agent",
                ));
            };
            let (owner, _, _) = context
                .window_worker_creator(page, target, *document_generation, *stream_epoch)
                .map_err(error)?;
            create(context, owner, creation, network, cancel).map(AgentServiceResponse::Connected)
        }
        AgentServiceRequest::Message(request) => {
            if request.requires_window_document_scope() {
                return Err(WorkerError::new(
                    "SecurityError",
                    "Window message operation requires native document scope",
                ));
            }
            context
                .handle_message_wire(page, request)
                .map(AgentServiceResponse::Message)
                .map_err(error)
        }
        // Only the page identity is used from this value. connection_owner
        // resolves and validates the original generation/realm from capability.
        request => client_request(
            context,
            AgentOwner::Window {
                page,
                target: WindowTarget::Root,
                generation: 0,
                epoch: 0,
            },
            request,
        ),
    }
}
fn publish_secure_context(
    context: &BrowserContext,
    source: AgentOwner,
    creator: Creator,
    secure: bool,
) -> Result<AgentServiceResponse, WorkerError> {
    let owner = match (source, creator) {
        (
            AgentOwner::Window { page, .. },
            Creator::Window {
                target,
                document_generation,
                stream_epoch,
            },
        ) => {
            context
                .window_worker_creator(page, &target, document_generation, stream_epoch)
                .map_err(error)?
                .0
        }
        (AgentOwner::Agent(source), Creator::Agent(target)) if source == target => {
            AgentOwner::Agent(source)
        }
        _ => {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign secure-context publication",
            ));
        }
    };
    context.publish_worker_secure_context(owner, secure)?;
    Ok(AgentServiceResponse::Done)
}
fn service_request(
    context: &BrowserContext,
    source: AgentOwner,
    request: oryn_js::service_worker_wire::ServiceRequest,
    network: SharedNetworkBroker,
) -> Result<oryn_js::service_worker_wire::ServiceResponse, WorkerError> {
    use oryn_js::service_worker_wire::ServiceRequest;
    let creator = match &request {
        ServiceRequest::NavigationPreload { creator, .. }
        | ServiceRequest::Cache { creator, .. }
        | ServiceRequest::PostMessage { creator, .. }
        | ServiceRequest::Register { creator, .. }
        | ServiceRequest::Update { creator, .. }
        | ServiceRequest::Unregister { creator, .. }
        | ServiceRequest::GetRegistration { creator, .. }
        | ServiceRequest::GetRegistrations { creator }
        | ServiceRequest::Ready { creator }
        | ServiceRequest::Poll { creator, .. }
        | ServiceRequest::Acknowledge { creator, .. }
        | ServiceRequest::Snapshot { creator, .. }
        | ServiceRequest::Controller { creator } => Some(creator),
        ServiceRequest::ClientFocus { .. }
        | ServiceRequest::ClientNavigate { .. }
        | ServiceRequest::ClientsOpenWindow { .. }
        | ServiceRequest::ClientsGet { .. }
        | ServiceRequest::ClientsMatchAll { .. }
        | ServiceRequest::ClientPostMessage { .. }
        | ServiceRequest::WorkerRegistration
        | ServiceRequest::SkipWaiting
        | ServiceRequest::Claim => None,
    };
    let owner = match (&source, creator) {
        (
            AgentOwner::Window { page, .. },
            Some(Creator::Window {
                target,
                document_generation,
                stream_epoch,
            }),
        ) => {
            context
                .window_worker_creator(*page, target, *document_generation, *stream_epoch)
                .map_err(error)?
                .0
        }
        (AgentOwner::Agent(source), Some(Creator::Agent(target))) if source == target => {
            AgentOwner::Agent(*source)
        }
        (AgentOwner::Agent(source), None) => AgentOwner::Agent(*source),
        _ => {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign service worker creator route",
            ));
        }
    };
    context.service_registry()?.request(owner, request, network)
}
fn blob_request(
    context: &BrowserContext,
    source: AgentOwner,
    request: AgentServiceRequest,
) -> Result<AgentServiceResponse, WorkerError> {
    let creator = match &request {
        AgentServiceRequest::BlobRegister { creator, .. }
        | AgentServiceRequest::BlobResolve { creator, .. }
        | AgentServiceRequest::BlobRevoke { creator, .. } => creator,
        _ => return Err(error("invalid blob operation")),
    };
    let owner = match (&source, creator) {
        (
            AgentOwner::Window { page, .. },
            Creator::Window {
                target,
                document_generation,
                stream_epoch,
            },
        ) => {
            context
                .window_worker_creator(*page, target, *document_generation, *stream_epoch)
                .map_err(error)?
                .0
        }
        (AgentOwner::Agent(source), Creator::Agent(target)) if source == target => {
            AgentOwner::Agent(*source)
        }
        _ => {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign blob creator route",
            ));
        }
    };
    match request {
        AgentServiceRequest::BlobRegister {
            bytes, media_type, ..
        } => context
            .register_native_blob(owner, bytes, media_type)
            .map(AgentServiceResponse::BlobUrl),
        AgentServiceRequest::BlobResolve { url, .. } => context
            .resolve_native_blob(&owner, &url)
            .map(|blob| AgentServiceResponse::Blob {
                bytes: blob.bytes,
                media_type: blob.media_type,
            }),
        AgentServiceRequest::BlobRevoke { url, .. } => {
            context.revoke_native_blob(&owner, &url)?;
            Ok(AgentServiceResponse::Done)
        }
        _ => Err(error("invalid blob request")),
    }
}
fn load_blob_script(
    params: BlobScriptLoad<'_>,
) -> Result<crate::agent_script::LoadedAgentScript, WorkerError> {
    let BlobScriptLoad {
        context,
        owner,
        url,
        policy,
        kind,
        referrer,
        cancel,
        request_destination,
    } = params;

    let worklet = request_destination == "audioworklet";
    if cancel.load(Ordering::Acquire) {
        return Err(crate::script_load_failure::ScriptLoadFailure::Cancelled.into_worker());
    }
    let destination = if !worklet
        && matches!(
            kind,
            crate::agent_script::ScriptLoadKind::EntryClassic
                | crate::agent_script::ScriptLoadKind::EntryModule
        ) {
        crate::csp::CspDestination::Worker
    } else {
        crate::csp::CspDestination::Script
    };
    let native_policy = crate::csp::CspPolicySet::from_agent_policy(policy);
    let decision = native_policy.authorize(&crate::csp::CspRequest {
        destination,
        url: url.to_string(),
        nonce: None,
        inline_source: None,
        parser_inserted: false,
        initiated_by_trusted_script: true,
    });
    context.report_worker_csp(
        owner,
        &native_policy,
        decision.violations,
        &policy.url,
        false,
    )?;
    if !decision.allowed {
        return Err(
            WorkerError::new("SecurityError", "blob script blocked by creator CSP")
                .with_script_failure(ScriptFailureKind::Fetch),
        );
    }
    let blob = context.resolve_native_blob(owner, url.as_str())?;
    let response = crate::network::NetworkResponse {
        response_policy: None,
        timing: None,
        request_id: 0,
        status: 200,
        headers: vec![("content-type".into(), blob.media_type)],
        body: blob.bytes,
    };
    let source = crate::agent_script::decode_imported_source(&response, kind)?;
    if cancel.load(Ordering::Acquire) {
        return Err(crate::script_load_failure::ScriptLoadFailure::Cancelled.into_worker());
    }
    if let AgentOwner::Window { page, .. } = owner {
        context.retained_worker_authority(owner)?;
        let top = context.page_url(*page).map_err(error)?;
        context.deliver_script_hash_reports(
            Some(*page),
            &top,
            native_policy.hash_reports(
                destination,
                request_destination,
                url.as_str(),
                &response.body,
                true,
            ),
        );
    }
    Ok(crate::agent_script::LoadedAgentScript {
        content_type: oryn_common::mime_type::MimeType::from_headers(&response.headers)
            .map(|mime| mime.serialize()),
        digest: crate::agent_script::representation_digest(&response.body),
        service_worker_allowed: None,
        url: url.to_string(),
        source: source.encode_utf16().collect(),
        policy: blob.policy.with_document_url(url.to_string()),
        referrer_policy: referrer.into(),
    })
}
fn client_request(
    context: &BrowserContext,
    source: AgentOwner,
    request: AgentServiceRequest,
) -> Result<AgentServiceResponse, WorkerError> {
    if let AgentServiceRequest::Release { connection } = &request {
        let Some(owner) = context.worker_release_owner(source, connection)? else {
            return Ok(AgentServiceResponse::Done);
        };
        let connection = connection.clone();
        return context.dispatch_worker_registry(move |registry, hub| {
            registry.release(hub, &owner, &connection)?;
            Ok(AgentServiceResponse::Done)
        });
    }

    let connection = match &request {
        AgentServiceRequest::Worklet { connection, .. }
        | AgentServiceRequest::Post { connection, .. }
        | AgentServiceRequest::Poll { connection, .. }
        | AgentServiceRequest::Release { connection }
        | AgentServiceRequest::Terminate { connection } => connection,
        _ => {
            return Err(WorkerError::new(
                "SecurityError",
                "agent-only operation on a Window client route",
            ));
        }
    };
    let owner = context.worker_connection_owner(source, connection)?;
    context.dispatch_worker_registry(move |registry, hub| match request {
        AgentServiceRequest::Post { connection, value } => {
            registry.post(hub, &owner, &connection, value)?;
            Ok(AgentServiceResponse::Done)
        }
        AgentServiceRequest::Poll { connection, max } => registry
            .poll(hub, &owner, &connection, max.min(32))
            .map(AgentServiceResponse::Events),
        AgentServiceRequest::Release { connection } => {
            registry.release(hub, &owner, &connection)?;
            Ok(AgentServiceResponse::Done)
        }
        AgentServiceRequest::Worklet {
            connection,
            command,
        } => {
            registry.worklet(hub, &owner, &connection, command)?;
            Ok(AgentServiceResponse::Done)
        }
        AgentServiceRequest::Terminate { connection } => {
            registry.terminate(hub, &owner, &connection)?;
            Ok(AgentServiceResponse::Done)
        }
        _ => Err(error("invalid worker client operation")),
    })
}
fn create(
    context: &BrowserContext,
    owner: AgentOwner,
    creation: AgentCreation,
    network: SharedNetworkBroker,
    cancel: &AtomicBool,
) -> Result<AgentConnection, WorkerError> {
    if creation.kind == AgentKind::Service {
        return Err(WorkerError::new(
            "SecurityError",
            "service agents require parent registration lifecycle authority",
        ));
    }
    let (partition, origin, policy, base, referrer, top) = match &owner {
        AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } => {
            let (_, partition, origin) = context
                .window_worker_creator(*page, target, *generation, *epoch)
                .map_err(error)?;
            let policy = context
                .window_worker_policy(*page, target, *generation, *epoch)
                .map_err(error)?;
            let base = policy.url.clone();
            let referrer = context
                .page_referrer_policy(*page)
                .map_err(error)?
                .as_str()
                .to_owned();
            (
                partition,
                origin,
                policy,
                base,
                referrer,
                Some(context.page_url(*page).map_err(error)?),
            )
        }
        AgentOwner::Agent(id) => {
            let bootstrap = context.worker_bootstrap(*id)?;
            (
                bootstrap.partition,
                bootstrap.origin,
                bootstrap.policy,
                bootstrap.script_url,
                bootstrap.referrer_policy,
                bootstrap.top_level_url,
            )
        }
    };
    if creation.kind == AgentKind::AudioWorklet
        && creation
            .options
            .audio_sample_rate
            .is_none_or(|rate| !rate.is_finite() || !(8000.0..=96000.0).contains(&rate))
    {
        return Err(WorkerError::new(
            "NotSupportedError",
            "invalid worklet sample rate",
        ));
    }
    if creation.kind == AgentKind::AudioWorklet
        && (!matches!(owner, AgentOwner::Window { .. })
            || creation.options.script_type != ScriptType::Module)
    {
        return Err(WorkerError::new(
            "SecurityError",
            "AudioWorklet requires a Window-owned module context",
        ));
    }
    let cookie_ancestor_urls = context.worker_cookie_ancestor_urls(&owner)?;
    let kind = if creation.options.script_type == ScriptType::Module {
        crate::agent_script::ScriptLoadKind::EntryModule
    } else {
        crate::agent_script::ScriptLoadKind::EntryClassic
    };
    let resolved = url::Url::parse(&base)
        .and_then(|base| base.join(&creation.url))
        .map_err(error)?;
    let services = context.existing_service_registry();
    if let (Some(services), AgentOwner::Agent(id)) = (&services, &owner) {
        services.ensure_agent_controller(&context.worker_bootstrap(*id)?)?;
    }
    let controller = services.as_deref().map(|services| (services, &owner));
    let report = |policy: &crate::csp::CspPolicySet, violations: Vec<crate::csp::CspViolation>| {
        context.report_worker_csp(
            &owner,
            policy,
            violations,
            top.as_deref().unwrap_or(&base),
            false,
        )
    };
    let coep_report = |reports| {
        let _ = context.deliver_coep_reports(top.as_deref().unwrap_or(&base), reports);
    };
    let hash_report = |reports| {
        if let AgentOwner::Window { page, .. } = &owner
            && context.retained_worker_authority(&owner).is_ok()
        {
            context.deliver_script_hash_reports(
                Some(*page),
                top.as_deref().unwrap_or(&base),
                reports,
            );
        }
    };
    let loaded = if resolved.scheme() == "blob" {
        load_blob_script(BlobScriptLoad {
            context,
            owner: &owner,
            url: &resolved,
            policy: &policy,
            kind,
            referrer: &referrer,
            cancel,
            request_destination: match creation.kind {
                AgentKind::Shared => "sharedworker",
                AgentKind::AudioWorklet => "audioworklet",
                _ => "worker",
            },
        })?
    } else {
        crate::agent_script::load_controlled(
            &network,
            crate::agent_script::ScriptAuthority {
                hash_report: matches!(&owner, AgentOwner::Window { .. })
                    .then_some(&hash_report as &dyn Fn(Vec<crate::csp::ScriptHashReport>)),
                worker_destination: match creation.kind {
                    AgentKind::Shared => "sharedworker",
                    AgentKind::Service => "serviceworker",
                    AgentKind::AudioWorklet => "audioworklet",
                    _ => "worker",
                },
                coep_report: Some(&coep_report),
                report: Some(&report),
                cookie_ancestor_urls: &cookie_ancestor_urls,
                referrer_source: None,
                origin: &origin,
                policy: &policy,
                script_url: &base,
                referrer_policy: &referrer,
                top_level_url: top.as_deref(),
                credentials: creation.options.credentials,
            },
            &creation.url,
            kind,
            cancel,
            controller,
        )?
    };
    match &owner {
        AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } => {
            let (_, current_partition, current_origin) = context
                .window_worker_creator(*page, target, *generation, *epoch)
                .map_err(error)?;
            if current_partition != partition || current_origin != origin {
                return Err(error("worker creator changed while its script loaded"));
            }
        }
        AgentOwner::Agent(id) => {
            context.worker_bootstrap(*id)?;
        }
    }
    let origin = if creation.kind != AgentKind::AudioWorklet && loaded.url.starts_with("data:") {
        let native = oryn_js::origin::Origin::opaque();
        AgentOrigin {
            serialized: native.serialize(),
            authority: native.security_token_key(),
            opaque: true,
        }
    } else {
        origin
    };
    let service_controller = context
        .existing_service_registry()
        .and_then(|registry| registry.controller_registration(&owner));
    let secure_context = context.worker_secure_context(&owner)?;
    let bootstrap = AgentBootstrap {
        cookie_ancestor_urls,
        import_metadata: Default::default(),
        service_skip_waiting: false,
        resource_digests: std::collections::BTreeMap::from([(loaded.url.clone(), loaded.digest)]),
        secure_context,
        service_registration: None,
        service_controller,
        id: AgentId(0),
        kind: creation.kind,
        partition,
        origin,
        script_url: loaded.url.clone(),
        source: loaded.source,
        modules: Default::default(),
        options: creation.options,
        referrer_policy: loaded.referrer_policy,
        limits: context.agent_limits(),
        policy: if creation.kind == AgentKind::AudioWorklet {
            policy.with_document_url(loaded.url.clone())
        } else {
            loaded.policy
        },
        top_level_url: top,
    };
    let child_mode = context.agent_child_mode();
    let factory_context = context.clone();
    context.dispatch_worker_registry(move |registry, hub| {
        match &owner {
            AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } => match target {
                WindowTarget::Root => hub
                    .root_document_authority(page.0, *generation, *epoch)
                    .map_err(error)?,
                WindowTarget::Frame(frame) => {
                    hub.frame_document_authority(page.0, frame, *generation, *epoch)
                        .map_err(error)?;
                }
            },
            AgentOwner::Agent(id) => {
                registry.principal(*id)?;
            }
        }
        let cluster_parent = owner.clone();
        registry.connect_with_ports(hub, owner, bootstrap, move |bootstrap| {
            factory_context.admit_worker_shared_memory(bootstrap, Some(&cluster_parent))?;
            let backend: Arc<dyn WorkerTransport> = Arc::new(NativeAgentBackend::new(
                &factory_context,
                bootstrap.clone(),
                network,
            ));
            let result =
                (|| -> Result<Box<dyn crate::agent_registry::AgentProcess>, WorkerError> {
                    if child_mode {
                        return crate::agent_process::AgentChildProcess::prepare(
                            bootstrap.clone(),
                            backend,
                        )
                        .map(|process| {
                            Box::new(process) as Box<dyn crate::agent_registry::AgentProcess>
                        });
                    }
                    #[cfg(feature = "v8-host")]
                    {
                        crate::agent_thread::AgentThreadProcess::prepare(
                            bootstrap.clone(),
                            move |bootstrap| {
                                crate::host::NativeAgentRuntime::create(bootstrap, backend)
                            },
                        )
                        .map(|process| {
                            Box::new(process) as Box<dyn crate::agent_registry::AgentProcess>
                        })
                    }
                    #[cfg(not(feature = "v8-host"))]
                    {
                        let _ = backend;
                        Err(error("in-process agent runtime is not compiled"))
                    }
                })();
            if result.is_err() {
                factory_context.retire_shared_memory(&AgentOwner::Agent(bootstrap.id));
            }
            result
        })
    })
}

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use super::*;
    use crate::{
        network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse},
        runtime::{Browser, BrowserConfig},
    };
    use oryn_js::message_wire::MessageRequest;
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    #[test]
    fn cancelled_blob_module_load_keeps_native_cancellation_before_authority_lookup() {
        let context = Browser::new().new_context();
        let policy =
            crate::csp::CspPolicySet::from_headers("https://example.test/", &[]).worker_snapshot();
        let url = url::Url::parse("blob:https://example.test/unregistered").unwrap();
        let error = load_blob_script(BlobScriptLoad {
            context: &context,
            owner: &AgentOwner::Agent(AgentId(999)),
            url: &url,
            policy: &policy,
            kind: crate::agent_script::ScriptLoadKind::ModuleImport,
            referrer: "",
            cancel: &AtomicBool::new(true),
            request_destination: "script",
        })
        .err()
        .unwrap();
        assert_eq!(error.script_failure, Some(ScriptFailureKind::Cancelled));
    }
    struct Fixture {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for Fixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let body = if request.url.ends_with("/worker.js") {
                br#"fetch('./api').then(r=>r.text()).then(text=>{if(text!=='native-body')throw Error('incorrect fetch body');postMessage({ok:true});close()}).catch(e=>{throw e})"#.to_vec()
            } else {
                b"native-body".to_vec()
            };
            self.requests.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![(
                    "content-type".into(),
                    if request.url.ends_with("/worker.js") {
                        "text/javascript"
                    } else {
                        "text/plain"
                    }
                    .into(),
                )],
                body,
            })
        }
    }
    struct TrustedTypesFixture;
    impl NetworkBroker for TrustedTypesFixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![
                ("content-type".into(),"text/javascript".into()),
                ("content-security-policy".into(),"trusted-types allowed".into()),
                ("content-security-policy-report-only".into(),"trusted-types 'none'".into())],body:br#"
                const reports=[];
                onsecuritypolicyviolation=event=>{
                    if(!event.isTrusted||event.target!==globalThis||event.effectiveDirective!=='trusted-types'||event.blockedURI!=='trusted-types-policy')throw Error('incorrect native worker violation');
                    reports.push([event.disposition,event.sample]);
                    if(reports.length===3){
                        if(JSON.stringify(reports)!==JSON.stringify([['report','allowed'],['enforce','blocked'],['report','blocked']]))throw Error('worker policy report order');
                        postMessage('verified');close();
                    }
                };
                const policy=trustedTypes.createPolicy('allowed',{createHTML:x=>x});
                if(policy.createHTML('ok').toString()!=='ok')throw Error('report-only blocked admission');
                let rejected=false;try{trustedTypes.createPolicy('blocked',{})}catch(error){rejected=error instanceof TypeError}
                if(!rejected)throw Error('enforcing policy did not reject');
                "#.to_vec()})
        }
    }
    #[test]
    fn native_worker_trusted_types_reports_follow_agent_authority() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page_id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        // Match Window worker construction: publish the exact document's secure
        // context before the native registry freezes the worker bootstrap.
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let owner = context
            .window_worker_creator(page_id, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        publish_secure_context(&context, owner, creator, true).unwrap();
        let network = SharedNetworkBroker::new(TrustedTypesFixture);
        let response = window_request(
            &context,
            page_id,
            AgentServiceRequest::Create(AgentCreation {
                kind: AgentKind::Dedicated,
                url: "/worker.js".into(),
                options: Default::default(),
                creator: Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
            }),
            network.clone(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let AgentServiceResponse::Connected(connection) = response else {
            panic!("worker connection")
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut verified = false;
        let mut closed = false;
        let mut failure = None;
        while Instant::now() < deadline && !closed {
            match window_request(
                &context,
                page_id,
                AgentServiceRequest::Poll {
                    connection: connection.clone(),
                    max: 8,
                },
                network.clone(),
                &AtomicBool::new(false),
            ) {
                Ok(AgentServiceResponse::Events(events)) => {
                    for event in events {
                        match event {
                            AgentEvent::Message { value, .. } => {
                                value.validate().unwrap();
                                verified = true;
                            }
                            AgentEvent::Closed => closed = true,
                            AgentEvent::Error { message, .. } => {
                                failure = Some(String::from_utf16_lossy(&message));
                                closed = true;
                            }
                            _ => {}
                        }
                    }
                }
                other => {
                    failure = Some(format!("unexpected worker result: {other:?}"));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let released = window_request(
            &context,
            page_id,
            AgentServiceRequest::Release {
                connection: connection.clone(),
            },
            network,
            &AtomicBool::new(false),
        );
        assert!(released.is_ok(), "{released:?}");
        assert!(failure.is_none(), "{failure:?}");
        assert!(
            verified && closed,
            "worker reports were not delivered before close"
        );
        assert!(context.worker_bootstrap(connection.agent).is_err());
    }

    #[test]
    fn admitted_native_worker_fetches_with_agent_authority_and_drains_before_close() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/app".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://example.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page_id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        // Match Window worker construction: publish the exact document's secure
        // context before the native registry freezes the worker bootstrap.
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let owner = context
            .window_worker_creator(page_id, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        publish_secure_context(&context, owner, creator, true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let network = SharedNetworkBroker::new(Fixture {
            requests: requests.clone(),
        });
        let response = window_request(
            &context,
            page_id,
            AgentServiceRequest::Create(AgentCreation {
                kind: AgentKind::Dedicated,
                url: "/worker.js".into(),
                options: Default::default(),
                creator: Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
            }),
            network.clone(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let AgentServiceResponse::Connected(connection) = response else {
            panic!("worker connection response")
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut message = false;
        let mut closed = false;
        let mut failure = None;
        while Instant::now() < deadline && !closed {
            match window_request(
                &context,
                page_id,
                AgentServiceRequest::Poll {
                    connection: connection.clone(),
                    max: 1,
                },
                network.clone(),
                &AtomicBool::new(false),
            ) {
                Ok(AgentServiceResponse::Events(events)) => {
                    for event in events {
                        match event {
                            AgentEvent::Message { value, .. } => {
                                value.validate().unwrap();
                                message = true;
                            }
                            AgentEvent::Closed => {
                                closed = true;
                                assert!(message, "close must follow previously queued message");
                            }
                            AgentEvent::Error { message, .. } => {
                                failure = Some(String::from_utf16_lossy(&message))
                            }
                            _ => {}
                        }
                    }
                }
                other => {
                    failure = Some(format!("unexpected poll: {other:?}"));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let released = window_request(
            &context,
            page_id,
            AgentServiceRequest::Release {
                connection: connection.clone(),
            },
            network.clone(),
            &AtomicBool::new(false),
        );
        assert!(released.is_ok(), "{released:?}");
        assert!(failure.is_none(), "{failure:?}");
        assert!(
            message && closed,
            "native fetch/message/close did not complete"
        );
        assert!(context.worker_bootstrap(connection.agent).is_err());
        assert!(
            matches!(
                window_request(
                    &context,
                    page_id,
                    AgentServiceRequest::Release {
                        connection: connection.clone()
                    },
                    network,
                    &AtomicBool::new(false)
                ),
                Ok(AgentServiceResponse::Done)
            ),
            "lost terminal acknowledgement retries without retaining a dead capability"
        );
        let requests = requests.lock().unwrap();
        let api = requests
            .iter()
            .find(|request| request.url.ends_with("/api"))
            .expect("native broker fetched worker API");
        assert_eq!(
            api.security.initiator_url.as_deref(),
            Some("https://example.test/worker.js")
        );
        assert_eq!(
            api.security.top_level_url.as_deref(),
            Some("https://example.test/app")
        );
        assert_ne!(requests[0].request_id, api.request_id);
    }
}

/// Registry admission precedes publication in ServiceRegistry, and author code
/// starts only after that publication releases its state lock.
pub(crate) struct ServiceAdmission(Arc<AtomicBool>);
impl ServiceAdmission {
    pub(crate) fn publish(self) {
        self.0.store(true, Ordering::Release);
    }
}
struct DeferredServiceProcess {
    inner: Box<dyn crate::agent_registry::AgentProcess>,
    published: Arc<AtomicBool>,
    admitted: bool,
    started: bool,
}
impl DeferredServiceProcess {
    fn ensure_started(&mut self) -> Result<(), WorkerError> {
        if self.admitted && !self.started && self.published.load(Ordering::Acquire) {
            self.inner.start()?;
            self.started = true;
        }
        Ok(())
    }
}
impl crate::agent_registry::AgentProcess for DeferredServiceProcess {
    fn start(&mut self) -> Result<(), WorkerError> {
        self.admitted = true;
        self.ensure_started()
    }
    fn send(&mut self, command: AgentCommand) -> Result<(), WorkerError> {
        self.ensure_started()?;
        self.inner.send(command)
    }
    fn poll(&mut self, max: usize) -> Result<Vec<AgentEvent>, WorkerError> {
        self.ensure_started()?;
        if self.started {
            self.inner.poll(max)
        } else {
            Ok(Vec::new())
        }
    }
    fn set_suspended(&mut self, suspended: bool) -> Result<(), WorkerError> {
        self.inner.set_suspended(suspended)
    }
    fn suspension_status(&mut self) -> Result<bool, WorkerError> {
        self.inner.suspension_status()
    }
    fn terminate(&mut self) -> Result<(), WorkerError> {
        self.inner.terminate()
    }
}
pub(crate) fn launch_service_agent(
    context: &BrowserContext,
    bootstrap: AgentBootstrap,
    network: SharedNetworkBroker,
) -> Result<(AgentId, ServiceAdmission), WorkerError> {
    let published = Arc::new(AtomicBool::new(false));
    let gate = published.clone();
    let child_mode = context.agent_child_mode();
    let factory_context = context.clone();
    let agent = context.dispatch_worker_registry(move |registry, _| {
        registry.launch_service(bootstrap, move |bootstrap| {
            factory_context.admit_worker_shared_memory(bootstrap, None)?;
            let backend: Arc<dyn WorkerTransport> = Arc::new(NativeAgentBackend::new(
                &factory_context,
                bootstrap.clone(),
                network,
            ));
            let process =
                (|| -> Result<Box<dyn crate::agent_registry::AgentProcess>, WorkerError> {
                    if child_mode {
                        return crate::agent_process::AgentChildProcess::prepare(
                            bootstrap.clone(),
                            backend,
                        )
                        .map(|process| {
                            Box::new(process) as Box<dyn crate::agent_registry::AgentProcess>
                        });
                    }
                    #[cfg(feature = "v8-host")]
                    {
                        crate::agent_thread::AgentThreadProcess::prepare(
                            bootstrap.clone(),
                            move |bootstrap| {
                                crate::host::NativeAgentRuntime::create(bootstrap, backend)
                            },
                        )
                        .map(|process| {
                            Box::new(process) as Box<dyn crate::agent_registry::AgentProcess>
                        })
                    }
                    #[cfg(not(feature = "v8-host"))]
                    {
                        let _ = backend;
                        Err(error("in-process service runtime unavailable"))
                    }
                })();
            if process.is_err() {
                factory_context.retire_shared_memory(&AgentOwner::Agent(bootstrap.id));
            }
            let process = process?;
            Ok(Box::new(DeferredServiceProcess {
                inner: process,
                published: gate,
                admitted: false,
                started: false,
            })
                as Box<dyn crate::agent_registry::AgentProcess>)
        })
    })?;
    Ok((agent, ServiceAdmission(published)))
}

#[cfg(test)]
mod service_admission_tests {
    use super::*;
    use crate::agent_registry::AgentProcess;
    struct Process {
        starts: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl AgentProcess for Process {
        fn start(&mut self) -> Result<(), WorkerError> {
            self.starts.fetch_add(1, Ordering::AcqRel);
            Ok(())
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
    #[test]
    fn service_author_execution_requires_registry_admission_and_version_publication() {
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gate = Arc::new(AtomicBool::new(false));
        let mut process = DeferredServiceProcess {
            inner: Box::new(Process {
                starts: starts.clone(),
            }),
            published: gate.clone(),
            admitted: false,
            started: false,
        };
        process.start().unwrap();
        process.poll(1).unwrap();
        assert_eq!(starts.load(Ordering::Acquire), 0);
        ServiceAdmission(gate).publish();
        process.poll(1).unwrap();
        process.poll(1).unwrap();
        assert_eq!(starts.load(Ordering::Acquire), 1);
        process.terminate().unwrap();
    }
}
