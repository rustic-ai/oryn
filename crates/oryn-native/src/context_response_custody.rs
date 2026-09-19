//! Parent ownership of resources granted by a response until wire publication.
use super::*;
use oryn_js::{
    message_wire::{MessageResponse, PortToken, WireValue},
    shared_memory::{SharedMemoryToken, WaitAdmissionId},
    worker_wire::{AgentEvent, AgentServiceResponse, Creator},
};
#[derive(Clone)]
pub(crate) enum ResponseCleanup {
    Message(WireValue),
    Broadcast(PortToken),
    ParkedDocument(oryn_js::message_wire::ParkedDocumentToken),
    Download(crate::download::DownloadId),
    LocalNavigation(String),
    FramePolicy(String),
    Connection(oryn_js::worker_wire::AgentConnection),
    BlankPrerender {
        page: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    Prerender {
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    Memory {
        creator: Creator,
        token: SharedMemoryToken,
    },
    Wait {
        creator: Creator,
        id: WaitAdmissionId,
    },
}
#[derive(Clone, Copy)]
pub(crate) enum ResponseSource {
    Window(PageId),
    Agent(oryn_js::worker_wire::AgentId),
}
impl ResponseSource {
    pub(crate) fn principal(self) -> oryn_js::message_wire::Principal {
        match self {
            Self::Window(page) => oryn_js::message_wire::Principal::Window(page.0),
            Self::Agent(agent) => oryn_js::message_wire::Principal::Agent(agent.0),
        }
    }
}
#[derive(Clone)]
pub(crate) struct NetworkResponseCustody {
    pub(super) _lifecycle: Option<Arc<SocketLifecycle>>,
    pub(super) sockets: Arc<Mutex<BTreeMap<u64, Arc<OwnedWebSocket>>>>,
    pub(super) owners: Arc<Mutex<BTreeMap<u64, Creator>>>,
    pub(super) audits: Arc<Mutex<BTreeMap<u64, crate::resource_audit::WebSocketObservation>>>,
    pub(super) streams: Arc<Mutex<BTreeMap<u64, Arc<HttpStreamEntry>>>>,
}
struct HttpReservation {
    owner: NetworkResponseCustody,
    id: u64,
    armed: bool,
}
impl Drop for HttpReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let stream = self
            .owner
            .streams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        if let Some(stream) = stream {
            if stream.keepalive_drain.is_some() {
                orphan_keepalive_response(stream);
            } else if let Some(cleanup) = &stream.cleanup {
                cleanup.cancel();
            }
        }
    }
}
struct SocketReservation {
    owner: NetworkResponseCustody,
    id: u64,
    armed: bool,
}
impl Drop for SocketReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // No close handshake or broker RPC: an unreceived capability has no
        // recipient to drive it. Release only this exact registry's socket.
        let socket = self
            .owner
            .sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        self.owner
            .owners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        let audit = self
            .owner
            .audits
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        if let Some(socket) = &socket {
            socket.interrupt();
        }
        drop(socket);
        if let Some(mut audit) = audit {
            audit.finish("response_not_received");
        }
    }
}
#[derive(Clone)]
pub(crate) struct ResponseScope {
    pub keepalive: Option<Arc<super::ordinary_keepalive::Registry>>,
    pub context: crate::runtime::WeakBrowserContext,
    pub source: ResponseSource,
    pub scripts: Option<crate::script_load_lease::ScriptLoadRegistry>,
    pub sockets: Option<NetworkResponseCustody>,
}
#[derive(Default)]
pub(crate) struct ResponseRequest {
    creator: Option<Creator>,
    wait: Option<WaitAdmissionId>,
}
impl ResponseRequest {
    pub(crate) fn capture(request: &ContextServiceRequest) -> Self {
        let memory = match request {
            ContextServiceRequest::SharedMemory {
                creator, request, ..
            } => Some((creator, request)),
            ContextServiceRequest::WorkerService {
                request:
                    oryn_js::worker_wire::AgentServiceRequest::SharedMemory { creator, request },
                ..
            } => Some((creator, request)),
            _ => None,
        };
        memory.map_or_else(Self::default, |(creator, request)| Self {
            creator: Some(creator.clone()),
            wait: match request {
                oryn_js::shared_memory::SharedMemoryRequest::WaitAdmission { id, .. } => Some(*id),
                _ => None,
            },
        })
    }
}
fn ports(tokens: Vec<PortToken>) -> ResponseCleanup {
    ResponseCleanup::Message(WireValue {
        shared: Vec::new(),
        shared_queue: None,
        shared_recipient: None,
        wasm_modules: Vec::new(),
        version: 1,
        bytes: Vec::new(),
        hosts: Vec::new(),
        buffers: Vec::new(),
        ports: tokens,
    })
}
fn message(output: &mut Vec<ResponseCleanup>, response: &MessageResponse) {
    match response {
        MessageResponse::PortMessages { messages, .. } => output.extend(
            messages
                .iter()
                .map(|value| ResponseCleanup::Message(value.cleanup_clone())),
        ),
        MessageResponse::WindowMessages(messages) => output.extend(
            messages
                .iter()
                .map(|value| ResponseCleanup::Message(value.payload.cleanup_clone())),
        ),
        MessageResponse::BroadcastMessages(messages) => output.extend(
            messages
                .iter()
                .map(|value| ResponseCleanup::Message(value.payload.cleanup_clone())),
        ),
        MessageResponse::Channel(tokens) => output.push(ports(tokens.to_vec())),
        MessageResponse::Transferred(tokens) => output.push(ports(tokens.clone())),
        MessageResponse::DocumentParked(token) => {
            output.push(ResponseCleanup::ParkedDocument(token.clone()))
        }
        MessageResponse::BroadcastOpened(token) => {
            output.push(ResponseCleanup::Broadcast(token.clone()))
        }
        _ => {}
    }
}
pub(crate) fn agent(output: &mut Vec<ResponseCleanup>, response: &AgentServiceResponse) {
    match response {
        AgentServiceResponse::Message(response) => message(output, response),
        AgentServiceResponse::Connected(connection) => {
            if let Some(port) = &connection.port {
                output.push(ports(vec![port.clone()]));
            }
            output.push(ResponseCleanup::Connection(connection.clone()));
        }
        AgentServiceResponse::Events(events) => {
            for event in events {
                if let AgentEvent::Message { value, .. } = event {
                    output.push(ResponseCleanup::Message(value.cleanup_clone()));
                }
            }
        }
        AgentServiceResponse::Service(oryn_js::service_worker_wire::ServiceResponse::Snapshot(
            snapshot,
        )) => {
            output.extend(
                snapshot
                    .messages
                    .iter()
                    .map(|message| ResponseCleanup::Message(message.value.cleanup_clone())),
            );
        }
        _ => {}
    }
}
struct ScriptReservation {
    registry: crate::script_load_lease::ScriptLoadRegistry,
    token: u64,
    armed: bool,
}
impl Drop for ScriptReservation {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.registry.cancel(self.token);
        }
    }
}
struct KeepaliveReservation {
    registry: Arc<super::ordinary_keepalive::Registry>,
    token: String,
    armed: bool,
}
impl Drop for KeepaliveReservation {
    fn drop(&mut self) {
        if self.armed {
            self.registry.release(&self.token);
        }
    }
}
pub(crate) struct ResponseGuard {
    keepalive: Option<KeepaliveReservation>,
    scope: Option<ResponseScope>,
    actions: Vec<ResponseCleanup>,
    script: Option<ScriptReservation>,
    socket: Option<SocketReservation>,
    http: Option<HttpReservation>,
}
impl ResponseGuard {
    pub(crate) fn new(
        scope: Option<ResponseScope>,
        request: ResponseRequest,
        response: Option<&ContextServiceResponse>,
    ) -> Self {
        let mut actions = Vec::new();
        if let Some(response) = response {
            match response {
                ContextServiceResponse::BlankPrerenderReserved {
                    page_id,
                    token,
                    generation,
                    epoch,
                    ..
                } => actions.push(ResponseCleanup::BlankPrerender {
                    page: *page_id,
                    token: token.clone(),
                    generation: *generation,
                    epoch: *epoch,
                }),
                ContextServiceResponse::PrerenderReserved {
                    token,
                    generation,
                    epoch,
                } => actions.push(ResponseCleanup::Prerender {
                    token: token.clone(),
                    generation: *generation,
                    epoch: *epoch,
                }),
                ContextServiceResponse::InitialDocumentParked { token } => {
                    actions.push(ResponseCleanup::ParkedDocument(token.clone()))
                }
                ContextServiceResponse::PageResource {
                    worker_policy_token: Some(token),
                    ..
                } => actions.push(ResponseCleanup::FramePolicy(token.clone())),
                ContextServiceResponse::BlankNavigationPrepared { token, .. }
                | ContextServiceResponse::BlobNavigationPrepared { token, .. }
                | ContextServiceResponse::LocalDataNavigationPrepared { token, .. } => {
                    actions.push(ResponseCleanup::LocalNavigation(token.clone()))
                }
                ContextServiceResponse::DownloadQuarantined { download } => {
                    actions.push(ResponseCleanup::Download(download.id))
                }
                ContextServiceResponse::MessageWire {
                    result: Ok(response),
                } => message(&mut actions, response),
                ContextServiceResponse::WorkerService {
                    result: Ok(response),
                } => agent(&mut actions, response),
                _ => {}
            }
            let memory = match response {
                ContextServiceResponse::SharedMemory(memory) => Some(memory),
                ContextServiceResponse::WorkerService {
                    result: Ok(AgentServiceResponse::SharedMemory(memory)),
                } => Some(memory),
                _ => None,
            };
            if let (Some(memory), Some(creator)) = (memory, request.creator) {
                if let Some(descriptor) = memory.descriptor {
                    actions.push(ResponseCleanup::Memory {
                        creator: creator.clone(),
                        token: descriptor.token,
                    });
                }
                if let Some(id) = request.wait {
                    actions.push(ResponseCleanup::Wait { creator, id });
                }
            }
        }
        let keepalive = match response {
            Some(ContextServiceResponse::OrdinaryKeepalivePrepared { token }) => scope
                .as_ref()
                .and_then(|scope| scope.keepalive.clone())
                .map(|registry| KeepaliveReservation {
                    registry,
                    token: token.clone(),
                    armed: true,
                }),
            _ => None,
        };
        let script = match response {
            Some(ContextServiceResponse::ScriptLoadReserved { token }) => scope
                .as_ref()
                .and_then(|scope| scope.scripts.clone())
                .map(|registry| ScriptReservation {
                    registry,
                    token: *token,
                    armed: true,
                }),
            _ => None,
        };
        let socket = match response {
            Some(ContextServiceResponse::WebSocketOpened { socket_id, .. }) => scope
                .as_ref()
                .and_then(|scope| scope.sockets.clone())
                .map(|owner| SocketReservation {
                    owner,
                    id: *socket_id,
                    armed: true,
                }),
            _ => None,
        };
        let http = match response {
            Some(ContextServiceResponse::HttpStreamStarted { stream_id, .. })
            | Some(ContextServiceResponse::PageResource {
                stream_id: Some(stream_id),
                ..
            }) => scope
                .as_ref()
                .and_then(|scope| scope.sockets.clone())
                .map(|owner| HttpReservation {
                    owner,
                    id: *stream_id,
                    armed: true,
                }),
            _ => None,
        };
        Self {
            keepalive,
            scope,
            actions,
            script,
            socket,
            http,
        }
    }
    pub(crate) fn commit(&mut self) {
        if let Some(mut keepalive) = self.keepalive.take() {
            keepalive.armed = false;
        }
        self.actions.clear();
        if let Some(mut script) = self.script.take() {
            script.armed = false;
        }
        if let Some(mut socket) = self.socket.take() {
            socket.armed = false;
        }
        if let Some(mut http) = self.http.take() {
            http.armed = false;
        }
    }
}
impl Drop for ResponseGuard {
    fn drop(&mut self) {
        if let Some(scope) = &self.scope {
            if let Some(context) = scope.context.upgrade() {
                for action in self.actions.drain(..) {
                    context.reclaim_context_response(scope.source, action);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::MessageRequest;
    #[test]
    fn service_snapshot_messages_have_the_same_rejected_reply_custody_as_port_events() {
        use oryn_js::service_worker_wire::{
            ClientMessage, ContainerSnapshot, ServiceResponse, ServiceVersionId, WorkerSnapshot,
            WorkerState,
        };
        for accepted in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let MessageResponse::Channel(tokens) = context
                .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
                .unwrap()
            else {
                panic!()
            };
            let ResponseCleanup::Message(value) = ports(tokens.to_vec()) else {
                panic!()
            };
            let response = ContextServiceResponse::WorkerService {
                result: Ok(AgentServiceResponse::Service(ServiceResponse::Snapshot(
                    ContainerSnapshot {
                        controller: None,
                        registrations: Vec::new(),
                        messages: vec![ClientMessage {
                            source: WorkerSnapshot {
                                version_id: ServiceVersionId([1; 16]),
                                id: oryn_js::worker_wire::AgentId(1),
                                script_url: "https://example.test/sw.js".into(),
                                state: WorkerState::Activated,
                            },
                            origin: "https://example.test".into(),
                            value,
                        }],
                    },
                ))),
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(page.page_id()),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if accepted {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert_eq!(
                context
                    .handle_message_wire(
                        page.page_id(),
                        MessageRequest::PollPort {
                            port: tokens[0].clone(),
                            max: 1
                        }
                    )
                    .is_ok(),
                accepted
            );
        }
    }
    #[test]
    fn prerender_reservation_response_owns_unreceived_candidate_cleanup() {
        for received in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page().page_id();
            context
                .install_document_security(page, "https://rules.test/start".into(), &[])
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
            let creator = Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let candidate = context
                .reserve_native_prerender(page, &creator, "https://rules.test/next")
                .unwrap();
            let response = ContextServiceResponse::PrerenderReserved {
                token: candidate.token.clone(),
                generation: candidate.lifetime.0,
                epoch: candidate.lifetime.1,
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(page),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert_eq!(
                context
                    .native_prerender_scope(page, &candidate.token, candidate.lifetime)
                    .is_ok(),
                received
            );
            context
                .cancel_native_prerender(page, &candidate.token, candidate.lifetime)
                .unwrap();
        }
    }
    #[test]
    fn initial_document_park_reply_retains_exact_rollback_custody_until_receipt() {
        for received in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page().page_id();
            let token = context.park_initial_document(page).unwrap();
            let response = ContextServiceResponse::InitialDocumentParked {
                token: token.clone(),
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(page),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert!(context.finish_initial_document(page, token, true).unwrap());
        }
    }

    #[test]
    fn frame_policy_response_requires_delivery_receipt() {
        for received in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .install_document_security(id, "https://source.test/".into(), &[])
                .unwrap();
            let token = context
                .prepare_frame_worker_policy(id, "https://child.test/", &[], "")
                .unwrap();
            let response = ContextServiceResponse::PageResource {
                inherited_encoding: None,
                response_headers: Vec::new(),
                raw_body_base64: None,
                request_state: None,
                content_language: None,
                timing: None,
                stream_id: None,
                native_origin: None,
                effective_post: None,
                worker_policy_token: Some(token.clone()),
                url: "https://child.test/".into(),
                source: String::new(),
                encoding: "UTF-8".into(),
                destination: "frame".into(),
                content_type: Some("text/html".into()),
                modals_allowed: true,
                document_referrer: String::new(),
                referrer_policy: "no-referrer".into(),
                refresh_url: None,
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(id),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert_eq!(
                context.prepared_frame_resource_policy(id, &token).is_ok(),
                received
            );
        }
    }

    #[test]
    fn local_navigation_preparation_requires_delivery_receipt() {
        for received in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let id = page.page_id();
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
            let (token, _, media_type, headers, origin) = context
                .prepare_local_navigation(
                    id,
                    Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    "data:text/plain,hello",
                    false,
                )
                .unwrap();
            let response = ContextServiceResponse::LocalDataNavigationPrepared {
                token: token.clone(),
                body_base64: "aGVsbG8=".into(),
                media_type,
                headers,
                serialized: origin.serialized,
                opaque_key: Some(origin.authority),
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(id),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert_eq!(
                context.commit_prepared_local_navigation(id, &token).is_ok(),
                received
            );
        }
    }

    #[test]
    fn quarantined_download_response_requires_delivery_receipt() {
        for received in [false, true] {
            let mut options = crate::ContextOptions::default();
            options.action_policy.rules.insert(
                crate::security::ConsequentialAction::Download,
                crate::security::PolicyRule::Allow,
            );
            let context = crate::Browser::new().new_context_with_options(options);
            let page = context.new_page();
            let id = page.page_id();
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
            let download = context
                .quarantine_page_download(
                    id,
                    Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    "https://file.test/x".into(),
                    "x".into(),
                    "text/plain".into(),
                    b"x".to_vec(),
                )
                .unwrap();
            let download_id = download.id;
            let response = ContextServiceResponse::DownloadQuarantined { download };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(id),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert_eq!(context.release_download(download_id).is_ok(), received);
        }
    }

    #[test]
    fn cached_document_response_requires_delivery_receipt_to_retain_restore_capability() {
        for received in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .install_document_security(id, "https://example.test/".into(), &[])
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
            let MessageResponse::DocumentParked(token) = context
                .handle_message_wire(
                    id,
                    MessageRequest::ParkDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            let response = ContextServiceResponse::MessageWire {
                result: Ok(MessageResponse::DocumentParked(token.clone())),
            };
            let scope = ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(id),
                scripts: None,
                sockets: None,
            };
            let mut guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
            if received {
                guard.commit();
            }
            drop(guard);
            context.retry_context_response_reclaims().unwrap();
            assert!(
                context
                    .handle_message_wire(id, MessageRequest::RestoreDocument { token })
                    .is_ok()
            );
            assert!(
                context
                    .window_worker_creator(id, &oryn_js::message_wire::WindowTarget::Root, 1, 0)
                    .is_ok(),
                "unreceived park must restore the still-installed source"
            );
        }
    }

    #[test]
    fn discarded_channel_response_closes_only_its_issued_capabilities() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let MessageResponse::Channel(tokens) = context
            .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
            .unwrap()
        else {
            panic!()
        };
        let MessageResponse::Channel(retained) = context
            .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
            .unwrap()
        else {
            panic!()
        };
        let response = ContextServiceResponse::MessageWire {
            result: Ok(MessageResponse::Channel(tokens.clone())),
        };
        let scope = ResponseScope {
            keepalive: None,
            context: context.downgrade(),
            source: ResponseSource::Window(page.page_id()),
            scripts: None,
            sockets: None,
        };
        drop(ResponseGuard::new(
            Some(scope),
            ResponseRequest::default(),
            Some(&response),
        ));
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::PollPort {
                        port: tokens[0].clone(),
                        max: 1
                    }
                )
                .is_err()
        );
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::PollPort {
                        port: retained[0].clone(),
                        max: 1
                    }
                )
                .is_ok()
        );
        context.retry_context_response_reclaims().unwrap();
    }
    #[test]
    fn committed_response_keeps_native_endpoints_and_oversize_replacement_does_not() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let scope = ResponseScope {
            keepalive: None,
            context: context.downgrade(),
            source: ResponseSource::Window(page.page_id()),
            scripts: None,
            sockets: None,
        };
        let MessageResponse::Channel(tokens) = context
            .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
            .unwrap()
        else {
            panic!()
        };
        let response = ContextServiceResponse::MessageWire {
            result: Ok(MessageResponse::Channel(tokens.clone())),
        };
        let mut guard = ResponseGuard::new(
            Some(scope.clone()),
            ResponseRequest::default(),
            Some(&response),
        );
        guard.commit();
        drop(guard);
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::PollPort {
                        port: tokens[0].clone(),
                        max: 1
                    }
                )
                .is_ok()
        );
        let guard = ResponseGuard::new(Some(scope), ResponseRequest::default(), Some(&response));
        let (_, preserved) = encode_context_response_status(
            ResponseEnvelope {
                protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                route: None,
                correlation_id: 7,
                response: Some(response),
                error: None,
            },
            256,
        )
        .unwrap();
        assert!(!preserved);
        drop(guard);
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::PollPort {
                        port: tokens[0].clone(),
                        max: 1
                    }
                )
                .is_err()
        );
    }
}

pub(crate) fn agent_actions(
    scope: oryn_js::worker_wire::ServiceResponseScope,
    response: &AgentServiceResponse,
) -> Vec<ResponseCleanup> {
    let mut actions = Vec::new();
    agent(&mut actions, response);
    if let (AgentServiceResponse::SharedMemory(memory), Some(creator)) =
        (response, scope.memory_creator)
    {
        if let Some(descriptor) = memory.descriptor {
            actions.push(ResponseCleanup::Memory {
                creator: creator.clone(),
                token: descriptor.token,
            });
        }
        if let Some(id) = scope.wait_admission {
            actions.push(ResponseCleanup::Wait { creator, id });
        }
    }
    actions
}

/// In-flight batches remain visible to admission, but never hold this mutex
/// while invoking process teardown (which can re-enter the broker).
#[derive(Default)]
pub(crate) struct ResponseReclaims {
    pending: Vec<(ResponseSource, ResponseCleanup)>,
    running: bool,
}
pub(crate) struct ReclaimBatch<'a> {
    queue: &'a std::sync::Mutex<ResponseReclaims>,
    remaining: std::collections::VecDeque<(ResponseSource, ResponseCleanup)>,
    failed: Vec<(ResponseSource, ResponseCleanup)>,
}
impl ResponseReclaims {
    pub(crate) fn enqueue(&mut self, source: ResponseSource, action: ResponseCleanup) {
        self.pending.push((source, action));
    }
    pub(crate) fn ready(&self) -> bool {
        !self.running && self.pending.is_empty()
    }
    pub(crate) fn begin(queue: &std::sync::Mutex<Self>) -> Option<ReclaimBatch<'_>> {
        let mut state = queue.lock().unwrap_or_else(|error| error.into_inner());
        if state.running || state.pending.is_empty() {
            return None;
        }
        state.running = true;
        Some(ReclaimBatch {
            queue,
            remaining: std::mem::take(&mut state.pending).into(),
            failed: Vec::new(),
        })
    }
}
impl ReclaimBatch<'_> {
    pub(crate) fn run(
        &mut self,
        mut dispose: impl FnMut(ResponseSource, &ResponseCleanup) -> bool,
    ) {
        // Keep the current item owned until disposal returns. Unwinding restores
        // it with the remaining tail; completed predecessors are not replayed.
        while let Some((source, action)) = self.remaining.front() {
            let done = dispose(*source, action);
            let item = self.remaining.pop_front().expect("owned response cleanup");
            if !done {
                self.failed.push(item);
            }
        }
    }
}
impl Drop for ReclaimBatch<'_> {
    fn drop(&mut self) {
        let mut state = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        state.pending.append(&mut self.failed);
        state.pending.extend(self.remaining.drain(..));
        state.running = false;
    }
}
#[cfg(test)]
mod reclaim_batch_tests {
    use super::*;
    fn action() -> ResponseCleanup {
        ports(Vec::new())
    }
    #[test]
    fn running_cleanup_remains_visible_and_allows_lock_reentry() {
        let queue = std::sync::Mutex::new(ResponseReclaims::default());
        queue
            .lock()
            .unwrap()
            .enqueue(ResponseSource::Window(PageId(1)), action());
        let mut batch = ResponseReclaims::begin(&queue).unwrap();
        assert!(!queue.lock().unwrap().ready());
        assert!(ResponseReclaims::begin(&queue).is_none());
        batch.run(|_, _| {
            queue
                .try_lock()
                .unwrap()
                .enqueue(ResponseSource::Window(PageId(2)), action());
            true
        });
        drop(batch);
        assert!(!queue.lock().unwrap().ready());
        let mut retry = ResponseReclaims::begin(&queue).unwrap();
        retry.run(|source, _| {
            assert!(matches!(source, ResponseSource::Window(PageId(2))));
            true
        });
        drop(retry);
        assert!(queue.lock().unwrap().ready());
    }
    #[test]
    fn unwinding_restores_current_and_unattempted_capabilities() {
        let queue = std::sync::Mutex::new(ResponseReclaims::default());
        for id in 1..=3 {
            queue
                .lock()
                .unwrap()
                .enqueue(ResponseSource::Window(PageId(id)), action());
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut batch = ResponseReclaims::begin(&queue).unwrap();
            batch.run(|source, _| {
                if matches!(source, ResponseSource::Window(PageId(2))) {
                    panic!("teardown panic")
                };
                true
            });
        }));
        assert!(result.is_err());
        let mut seen = Vec::new();
        let mut retry = ResponseReclaims::begin(&queue).unwrap();
        retry.run(|source, _| {
            if let ResponseSource::Window(page) = source {
                seen.push(page.0)
            };
            true
        });
        drop(retry);
        assert_eq!(seen, vec![2, 3]);
        assert!(queue.lock().unwrap().ready());
    }
}

#[derive(Default)]
pub(crate) struct Publications {
    closed: bool,
    pending: BTreeMap<u64, (Option<WorkerRoute>, ResponseGuard, ContextDispatchPermit)>,
}
pub(crate) struct PublicationOwner(pub Arc<Mutex<Publications>>);
impl Drop for PublicationOwner {
    fn drop(&mut self) {
        let entries = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            std::mem::take(&mut state.pending)
        };
        drop(entries); // No broker/process teardown under the publication lock.
    }
}
impl Publications {
    pub(crate) fn insert(
        &mut self,
        id: u64,
        route: Option<WorkerRoute>,
        guard: ResponseGuard,
        permit: ContextDispatchPermit,
    ) -> Result<(), (ResponseGuard, ContextDispatchPermit)> {
        if self.closed
            || self.pending.contains_key(&id)
            || self.pending.len() >= MAX_CONTEXT_OPERATIONS
        {
            return Err((guard, permit));
        }
        self.pending.insert(id, (route, guard, permit));
        Ok(())
    }
    pub(crate) fn receive(
        pool: &Mutex<Self>,
        receipt: &ResponseReceipt,
    ) -> Result<(), RuntimeError> {
        let item = {
            let mut state = pool.lock().map_err(|_| {
                RuntimeError::ContextService("response publication lock poisoned".into())
            })?;
            let Some((route, _, _)) = state.pending.get(&receipt.received_id) else {
                return Err(RuntimeError::ProtocolViolation(
                    "unknown response receipt".into(),
                ));
            };
            if receipt.protocol_version != CONTEXT_SERVICE_PROTOCOL_VERSION
                || *route != receipt.route
            {
                return Err(RuntimeError::ProtocolViolation(
                    "response receipt authority mismatch".into(),
                ));
            }
            state
                .pending
                .remove(&receipt.received_id)
                .expect("validated response receipt")
        };
        let (_, mut guard, _permit) = item;
        if receipt.accepted {
            guard.commit();
        }
        Ok(())
    }
    pub(crate) fn discard(pool: &Mutex<Self>, id: u64) {
        let item = pool
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .remove(&id);
        drop(item);
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use oryn_js::message_wire::MessageRequest;
    #[test]
    fn rejected_keepalive_receipt_transfers_body_instead_of_aborting_transport() {
        let cleanup = http_cleanup_custody::Ticket::reserve_keepalive(
            &Default::default(),
            91,
            SharedNetworkBroker::new(crate::network::UnavailableBroker),
        )
        .unwrap();
        let (sender, events) = tokio::sync::mpsc::channel(2);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 91,
                bytes: bytes::Bytes::from_static(b"accepted"),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                request_id: 91,
                encoded_body_size: Some(8),
            })
            .unwrap();
        let entry = Arc::new(HttpStreamEntry {
            keepalive_context: None,
            keepalive_drain: Some(crate::keepalive_drain::Ticket::reserve().unwrap()),
            page_id: PageId(1),
            creator: Mutex::new(None),
            prepared_frame_policy: None,
            frame_deadline: None,
            request_id: 91,
            cleanup: Some(cleanup),
            state: Mutex::new(HttpStreamState {
                preload: None,
                operation: NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 91,
                    events,
                    demand: None,
                },
                current: bytes::Bytes::new(),
                offset: 0,
                bytes_read: 0,
                content_length: Some(8),
            }),
        });
        let owner = NetworkResponseCustody {
            _lifecycle: None,
            sockets: Default::default(),
            owners: Default::default(),
            audits: Default::default(),
            streams: Arc::new(Mutex::new(BTreeMap::from([(91, entry.clone())]))),
        };
        drop(HttpReservation {
            owner: owner.clone(),
            id: 91,
            armed: true,
        });
        assert!(owner.streams.lock().unwrap().is_empty());
        assert!(!entry.cleanup.as_ref().unwrap().cancelled());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while entry.state.lock().unwrap().bytes_read < 8 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            !entry.cleanup.as_ref().unwrap().cancelled(),
            "wire rejection is not explicit fetch cancellation"
        );
    }
    #[test]
    fn http_receipt_rejection_removes_only_its_original_stream_registry() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let first = ParentContextService::new_with_network(
            context.clone(),
            page.page_id(),
            Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
        );
        let replacement = ParentContextService::new_with_network(
            context.clone(),
            page.page_id(),
            Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
        );
        let request = || ContextServiceRequest::HttpStreamStart {
            page_id: page.page_id(),
            request: HttpStreamRequest {
                upgrade_mixed_image: false,
                fetch: None,
                client: None,
                use_cors_preflight: false,
                request_id: 77,
                method: "GET".into(),
                url: "data:text/plain,hello".into(),
                headers: Vec::new(),
                body_base64: String::new(),
                credentials: false,
                credentials_mode: None,
                initiator_url: None,
                referrer_policy: None,
            },
        };
        let response = first.handle(request()).unwrap();
        let other = replacement.handle(request()).unwrap();
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = &response else {
            panic!()
        };
        let ContextServiceResponse::HttpStreamStarted {
            stream_id: other_id,
            ..
        } = other
        else {
            panic!()
        };
        assert_eq!(*stream_id, other_id);
        let scope = ContextTransport::response_scope(&first, None, None).unwrap();
        let guard = ResponseGuard::new(scope, Default::default(), Some(&response));
        let pool = Mutex::new(Publications::default());
        let limit = ContextDispatchLimit::default();
        assert!(
            pool.lock()
                .unwrap()
                .insert(1, None, guard, limit.admit().unwrap())
                .is_ok()
        );
        Publications::receive(
            &pool,
            &ResponseReceipt {
                protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                route: None,
                received_id: 1,
                accepted: false,
            },
        )
        .unwrap();
        assert!(!first.http_streams.lock().unwrap().contains_key(stream_id));
        assert!(
            replacement
                .http_streams
                .lock()
                .unwrap()
                .contains_key(stream_id)
        );
        let scope = ContextTransport::response_scope(&replacement, None, None).unwrap();
        let mut accepted = ResponseGuard::new(scope, Default::default(), Some(&other));
        accepted.commit();
        drop(accepted);
        assert!(
            replacement
                .http_streams
                .lock()
                .unwrap()
                .contains_key(stream_id)
        );
    }
    fn socket_owner() -> (NetworkResponseCustody, std::net::TcpStream) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let socket = WebSocket::from_raw_socket(
            MaybeTlsStream::Plain(client),
            tungstenite::protocol::Role::Client,
            None,
        );
        (
            NetworkResponseCustody {
                _lifecycle: None,
                owners: Arc::new(Mutex::new(BTreeMap::from([(
                    1,
                    Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )]))),
                sockets: Arc::new(Mutex::new(BTreeMap::from([(
                    1,
                    OwnedWebSocket::new(socket).unwrap(),
                )]))),
                audits: Arc::new(Mutex::new(BTreeMap::new())),
                streams: Arc::new(Mutex::new(BTreeMap::new())),
            },
            server,
        )
    }
    #[test]
    fn websocket_receipts_release_only_unaccepted_original_socket_after_context_retirement() {
        use std::io::Read;
        for accepted in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let (original, mut peer) = socket_owner();
            let (replacement, _replacement_peer) = socket_owner();
            let response = ContextServiceResponse::WebSocketOpened {
                socket_id: 1,
                protocol: String::new(),
            };
            let guard = ResponseGuard::new(
                Some(ResponseScope {
                    keepalive: None,
                    context: context.downgrade(),
                    source: ResponseSource::Window(page.page_id()),
                    scripts: None,
                    sockets: Some(original.clone()),
                }),
                Default::default(),
                Some(&response),
            );
            let limit = ContextDispatchLimit::default();
            let pool = Mutex::new(Publications::default());
            assert!(
                pool.lock()
                    .unwrap()
                    .insert(1, None, guard, limit.admit().unwrap())
                    .is_ok()
            );
            drop(page);
            drop(context);
            Publications::receive(
                &pool,
                &ResponseReceipt {
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    received_id: 1,
                    accepted,
                },
            )
            .unwrap();
            assert_eq!(original.sockets.lock().unwrap().contains_key(&1), accepted);
            assert_eq!(original.owners.lock().unwrap().contains_key(&1), accepted);
            assert!(replacement.sockets.lock().unwrap().contains_key(&1));
            assert_eq!(limit.0.load(Ordering::Acquire), 0);
            if !accepted {
                assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
            }
        }
    }
    #[test]
    fn disconnected_publication_owner_reclaims_unreceived_websocket() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let (sockets, _peer) = socket_owner();
        let response = ContextServiceResponse::WebSocketOpened {
            socket_id: 1,
            protocol: String::new(),
        };
        let guard = ResponseGuard::new(
            Some(ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(page.page_id()),
                scripts: None,
                sockets: Some(sockets.clone()),
            }),
            Default::default(),
            Some(&response),
        );
        let limit = ContextDispatchLimit::default();
        let pool = Arc::new(Mutex::new(Publications::default()));
        assert!(
            pool.lock()
                .unwrap()
                .insert(1, None, guard, limit.admit().unwrap())
                .is_ok()
        );
        drop(PublicationOwner(pool));
        assert!(sockets.sockets.lock().unwrap().is_empty());
        assert_eq!(limit.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn unreceived_grant_keeps_dispatch_permit_and_disconnect_reclaims() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let MessageResponse::Channel(tokens) = context
            .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
            .unwrap()
        else {
            panic!()
        };
        let response = ContextServiceResponse::MessageWire {
            result: Ok(MessageResponse::Channel(tokens.clone())),
        };
        let guard = ResponseGuard::new(
            Some(ResponseScope {
                keepalive: None,
                context: context.downgrade(),
                source: ResponseSource::Window(page.page_id()),
                scripts: None,
                sockets: None,
            }),
            Default::default(),
            Some(&response),
        );
        let limit = ContextDispatchLimit::default();
        let pool = Arc::new(Mutex::new(Publications::default()));
        let owner = PublicationOwner(pool.clone());
        assert!(
            pool.lock()
                .unwrap()
                .insert(1, None, guard, limit.admit().unwrap())
                .is_ok()
        );
        assert_eq!(limit.0.load(Ordering::Acquire), 1);
        assert!(
            Publications::receive(
                &pool,
                &ResponseReceipt {
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    received_id: 2,
                    accepted: true
                }
            )
            .is_err()
        );
        assert_eq!(limit.0.load(Ordering::Acquire), 1);
        drop(owner);
        assert_eq!(limit.0.load(Ordering::Acquire), 0);
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::PollPort {
                        port: tokens[0].clone(),
                        max: 1
                    }
                )
                .is_err()
        );
        let empty = ResponseGuard::new(None, Default::default(), None);
        assert!(
            pool.lock()
                .unwrap()
                .insert(3, None, empty, limit.admit().unwrap())
                .is_err()
        );
    }
    #[test]
    fn accepted_and_rejected_receipts_have_distinct_capability_lifetimes() {
        for accepted in [false, true] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let MessageResponse::Channel(tokens) = context
                .handle_message_wire(page.page_id(), MessageRequest::CreateChannel)
                .unwrap()
            else {
                panic!()
            };
            let response = ContextServiceResponse::MessageWire {
                result: Ok(MessageResponse::Channel(tokens.clone())),
            };
            let guard = ResponseGuard::new(
                Some(ResponseScope {
                    keepalive: None,
                    context: context.downgrade(),
                    source: ResponseSource::Window(page.page_id()),
                    scripts: None,
                    sockets: None,
                }),
                Default::default(),
                Some(&response),
            );
            let limit = ContextDispatchLimit::default();
            let pool = Mutex::new(Publications::default());
            assert!(
                pool.lock()
                    .unwrap()
                    .insert(7, None, guard, limit.admit().unwrap())
                    .is_ok()
            );
            Publications::receive(
                &pool,
                &ResponseReceipt {
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    received_id: 7,
                    accepted,
                },
            )
            .unwrap();
            assert_eq!(limit.0.load(Ordering::Acquire), 0);
            assert_eq!(
                context
                    .handle_message_wire(
                        page.page_id(),
                        MessageRequest::PollPort {
                            port: tokens[0].clone(),
                            max: 1
                        }
                    )
                    .is_ok(),
                accepted
            );
            assert!(
                Publications::receive(
                    &pool,
                    &ResponseReceipt {
                        protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                        route: None,
                        received_id: 7,
                        accepted: true
                    }
                )
                .is_err()
            );
        }
    }
}

#[cfg(test)]
mod script_reservation_tests {
    use super::*;
    #[test]
    fn failed_reserve_publication_cancels_original_registry_after_route_replacement() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let original = crate::script_load_lease::ScriptLoadRegistry::default();
        let replacement = crate::script_load_lease::ScriptLoadRegistry::default();
        let token = original.reserve().unwrap();
        assert_eq!(replacement.reserve().unwrap(), token);
        let response = ContextServiceResponse::ScriptLoadReserved { token };
        let scope = ResponseScope {
            keepalive: None,
            context: context.downgrade(),
            source: ResponseSource::Window(page.page_id()),
            scripts: Some(original.clone()),
            sockets: None,
        };
        let guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
        drop(page);
        drop(context);
        drop(guard);
        assert!(original.claim(token).is_err());
        assert!(replacement.claim(token).is_ok());
    }
    #[test]
    fn script_receipt_commit_preserves_reservation_but_disconnect_cancels_claimed_fetch() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let registry = crate::script_load_lease::ScriptLoadRegistry::default();
        let scope = ResponseScope {
            keepalive: None,
            context: context.downgrade(),
            source: ResponseSource::Window(page.page_id()),
            scripts: Some(registry.clone()),
            sockets: None,
        };
        let token = registry.reserve().unwrap();
        let response = ContextServiceResponse::ScriptLoadReserved { token };
        let mut guard =
            ResponseGuard::new(Some(scope.clone()), Default::default(), Some(&response));
        guard.commit();
        drop(guard);
        let lease = registry.claim(token).unwrap();
        assert!(!lease.cancel.load(Ordering::Acquire));
        drop(lease);
        let token = registry.reserve().unwrap();
        let response = ContextServiceResponse::ScriptLoadReserved { token };
        let guard = ResponseGuard::new(Some(scope), Default::default(), Some(&response));
        let lease = registry.claim(token).unwrap();
        drop(guard);
        assert!(lease.cancel.load(Ordering::Acquire));
        assert!(registry.claim(token).is_err());
    }
}
