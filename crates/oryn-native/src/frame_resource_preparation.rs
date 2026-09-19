//! Native frame navigation fetch custody. Only the owning page task commits a
//! response; worker completion never enters V8 or publishes a child document.
use super::*;
#[cfg(unix)]
use crate::context_service::{ContextServiceRequest, ContextServiceResponse as ParentResponse};
use crate::frame_resource_response::FrameResourceResponse;
#[cfg(test)]
thread_local! {pub(super) static FAIL_EMBEDDED_ACTIVATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
pub(super) fn fail_embedded_activation() -> bool {
    FAIL_EMBEDDED_ACTIVATION.with(|failure| failure.replace(false))
}

pub(super) struct FrameResourceCompletion<C = FrameNavigationContext> {
    _metadata: Vec<std::sync::Arc<ScriptSourceReservation>>,
    pub(super) timing: Option<oryn_js::navigation_timing::NavigationTiming>,
    pub(super) ancestor_capture: Option<oryn_js::v8_bindings::LocationAncestorCapture>,
    pub(super) broker: Option<SharedNetworkBroker>,
    pub(super) standalone_policy: Option<crate::standalone_document_policy::FramePolicy>,
    #[cfg(unix)]
    pub(super) body: Option<frame_response_stream::FrameResponseBody>,
    pub(super) navigation_context: Option<C>,
    pub(super) response: Result<FrameResourceResponse, String>,
    _bytes: Option<ScriptSourceReservation>,
    authority: Option<(
        (NodeId, u64, u64, bool),
        Option<u64>,
        Vec<Option<String>>,
        std::sync::Arc<()>,
    )>,
}
impl<C> std::fmt::Debug for FrameResourceCompletion<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameResourceCompletion")
            .field("response", &self.response)
            .field("has_broker", &self.broker.is_some())
            .finish()
    }
}
// Each request has its own one-shot channel. Its preparation, held in the
// page's frame_resource_preparations registry, owns the navigation context and
// archived controls. IO can only deliver this context-free payload.
impl FrameResourceCompletion<()> {
    fn into_isolate_completion(self) -> FrameResourceCompletion {
        FrameResourceCompletion {
            _metadata: self._metadata,
            timing: self.timing,
            ancestor_capture: self.ancestor_capture,
            broker: self.broker,
            standalone_policy: self.standalone_policy,
            #[cfg(unix)]
            body: self.body,
            navigation_context: None,
            response: self.response,
            _bytes: self._bytes,
            authority: self.authority,
        }
    }
}
impl FrameResourceCompletion {
    pub(super) fn current(&self, host: &RawV8Host, parent: RealmId, frame: NodeId) -> bool {
        self.authority
            .as_ref()
            .is_some_and(|(identity, connection, attributes, navigation)| {
                host.automation_realm_identity(parent) == Some(*identity)
                    && !host.realm_document_load_stopped(parent)
                    && host.automation_frame_live(parent, frame)
                    && host.automation_frame_generation(parent, frame) == *connection
                    && host.automation_dom(parent).is_some_and(|dom| {
                        FrameResourcePreparation::attributes(&dom, frame) == *attributes
                            && dom
                                .borrow()
                                .frame_navigation_identity_is_current(frame, navigation)
                    })
            })
    }
}
pub(super) struct FrameResourcePreparation {
    _metadata: Vec<std::sync::Arc<ScriptSourceReservation>>,
    timing: oryn_js::navigation_timing::NavigationTiming,
    ancestor_capture: Option<oryn_js::v8_bindings::LocationAncestorCapture>,
    _cancel: Option<LocalParserSourceCancel>,
    _resource_cancel: Option<standalone_stylesheet::Cancel>,
    pub(super) navigation_context: Option<FrameNavigationContext>,
    identity: (NodeId, u64, u64, bool),
    connection: Option<u64>,
    attributes: Vec<Option<String>>,
    navigation: std::sync::Arc<()>,
    receiver: std::sync::mpsc::Receiver<FrameResourceCompletion<()>>,
    ready: Option<FrameResourceCompletion>,
    _lifetime: std::sync::Arc<()>,
    _node: ParserScriptLease,
}
impl FrameResourcePreparation {
    pub(super) fn set_navigation_context(
        &mut self,
        context: Option<FrameNavigationContext>,
        budget: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<(), HostError> {
        if self.navigation_context.is_some() {
            return Err(HostError::Execution(
                "frame navigation context already admitted".into(),
            ));
        }
        if let Some(value) = &context {
            let held = frame_context_bytes(value)
                .ok()
                .and_then(|bytes| ScriptSourceReservation::reserve(budget, bytes));
            let Some(held) = held else {
                if let Some(observer) = &value.execution_observer {
                    observer.complete(oryn_js::navigation::NavigationExecutionOutcome::Failed {
                        message: "frame navigation context bytes exceeded".into(),
                    });
                }
                self._cancel.take();
                self._resource_cancel.take();
                self._lifetime = std::sync::Arc::new(());
                let (_sender, receiver) = std::sync::mpsc::channel();
                self.receiver = receiver;
                self.ready = Some(FrameResourceCompletion {
                    _metadata: Vec::new(),
                    timing: None,
                    ancestor_capture: None,
                    broker: None,
                    standalone_policy: None,
                    #[cfg(unix)]
                    body: None,
                    navigation_context: None,
                    response: Err("frame navigation context bytes exceeded".into()),
                    _bytes: None,
                    authority: None,
                });
                return Ok(());
            };
            self._metadata.push(std::sync::Arc::new(held));
        }
        self.navigation_context = context;
        Ok(())
    }

    fn navigation_identity(dom: &SharedDomRuntime, frame: NodeId) -> std::sync::Arc<()> {
        dom.borrow_mut().frame_navigation_identity(frame)
    }
    fn attributes(dom: &SharedDomRuntime, node: NodeId) -> Vec<Option<String>> {
        ["src", "srcdoc", "data", "type", "sandbox"]
            .iter()
            .map(|name| {
                dom.borrow()
                    .attribute(node, name)
                    .ok()
                    .flatten()
                    .map(str::to_owned)
            })
            .collect()
    }
    fn live(&self, host: &RawV8Host, parent: RealmId, frame: NodeId) -> bool {
        host.automation_realm_identity(parent) == Some(self.identity)
            && !host.realm_document_load_stopped(parent)
            && host.automation_frame_live(parent, frame)
            && host.automation_frame_generation(parent, frame) == self.connection
            && host.automation_dom(parent).is_some_and(|dom| {
                Self::attributes(&dom, frame) == self.attributes
                    && dom
                        .borrow()
                        .frame_navigation_identity_is_current(frame, &self.navigation)
            })
    }
    fn ready(&mut self) -> bool {
        if self.ready.is_none() {
            match self.receiver.try_recv() {
                Ok(completion) => self.ready = Some(completion.into_isolate_completion()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ready = Some(FrameResourceCompletion {
                        _metadata: Vec::new(),
                        timing: None,
                        ancestor_capture: None,
                        broker: None,
                        standalone_policy: None,
                        #[cfg(unix)]
                        body: None,
                        navigation_context: None,
                        response: Err("frame resource producer disconnected".into()),
                        _bytes: None,
                        authority: None,
                    })
                }
            }
        }
        self.ready.is_some()
    }
}
impl RustExecutableDocument {
    pub(super) fn queue_parser_frame_failures(
        &mut self,
        frames: &BTreeMap<NodeId, LoadedFrameResource>,
    ) -> Result<(), HostError> {
        for (&frame, response) in frames {
            if response.terminal_error.is_none() && !response.embedded_unavailable {
                continue;
            }
            let outcome = if response.embedded_unavailable {
                Ok(FrameResourceResponse::EmbeddedResourceUnavailable)
            } else {
                Err(bounded_frame_error(
                    response.terminal_error.clone().expect("terminal error"),
                ))
            };
            let owner = &response.owner;
            let Some(dom) = self.agent.host.automation_dom(owner.identity.realm) else {
                continue;
            };
            if !owner.live(&self.agent.host, &dom) {
                continue;
            }
            if self.frame_resource_preparations.len() >= 1024 {
                return Err(HostError::LimitExceeded {
                    resource: "terminal frame navigation owners".into(),
                });
            }
            self.agent.host.activate_realm(owner.identity.realm)?;
            let metadata = reserve_frame_owner_metadata(
                &self.dynamic_scripts.source_bytes,
                &owner.attributes,
                &owner.ancestor_capture,
            )?;
            self.agent.host.acknowledge_parser_frame_navigation(frame)?;
            let (_sender, receiver) = std::sync::mpsc::channel();
            self.frame_resource_preparations.insert(
                (owner.identity.realm, frame),
                FrameResourcePreparation {
                    _metadata: vec![metadata],
                    timing: oryn_js::navigation_timing::NavigationTiming::new(
                        std::time::Instant::now(),
                        oryn_js::navigation_timing::Kind::Navigate,
                    ),
                    ancestor_capture: Some(owner.ancestor_capture.clone()),
                    _resource_cancel: None,
                    _cancel: None,
                    navigation_context: None,
                    identity: (
                        owner.identity.document,
                        owner.identity.document_generation,
                        owner.identity.stream_epoch,
                        true,
                    ),
                    connection: owner.connection,
                    attributes: owner.attributes.clone(),
                    navigation: owner.navigation.clone(),
                    receiver,
                    ready: Some(FrameResourceCompletion {
                        _metadata: Vec::new(),
                        timing: None,
                        ancestor_capture: None,
                        broker: None,
                        standalone_policy: None,
                        #[cfg(unix)]
                        body: None,
                        navigation_context: None,
                        response: outcome,
                        _bytes: None,
                        authority: None,
                    }),
                    _lifetime: std::sync::Arc::new(()),
                    _node: ParserScriptLease::new(&dom, frame)?,
                },
            );
        }
        Ok(())
    }
    pub(super) fn frame_resources_pending(&self, realm: Option<RealmId>) -> bool {
        self.frame_resource_preparations
            .iter()
            .any(|(&(parent, frame), pending)| {
                realm.is_none_or(|realm| realm == parent)
                    && pending.live(&self.agent.host, parent, frame)
            })
    }
    pub(super) fn frame_resources_runnable(&mut self) -> bool {
        self.frame_resource_preparations
            .iter_mut()
            .any(|(&(parent, frame), pending)| {
                !pending.live(&self.agent.host, parent, frame) || pending.ready()
            })
    }
    pub(super) fn take_ready_frame_resources(
        &mut self,
    ) -> Vec<(RealmId, NodeId, Option<FrameResourceCompletion>)> {
        let mut completed = Vec::new();
        let mut stale = Vec::new();
        for (&(parent, frame), pending) in &mut self.frame_resource_preparations {
            if !pending.live(&self.agent.host, parent, frame) {
                if let Some(observer) = pending
                    .navigation_context
                    .as_ref()
                    .and_then(|context| context.execution_observer.as_ref())
                {
                    observer.complete(oryn_js::navigation::NavigationExecutionOutcome::Canceled);
                }
                stale.push((parent, frame));
                continue;
            }
            if completed.len() < 64 && pending.ready() {
                let mut response = pending.ready.take().expect("ready frame response");
                if let Some(context) = pending.navigation_context.as_ref() {
                    let flags = context.fetch_navigation_flags();
                    pending.timing.set_kind(if flags.reload {
                        oryn_js::navigation_timing::Kind::Reload
                    } else if flags.history {
                        oryn_js::navigation_timing::Kind::BackForward
                    } else {
                        oryn_js::navigation_timing::Kind::Navigate
                    });
                }
                if let Ok(FrameResourceResponse::PageResource {
                    url,
                    response_headers,
                    timing,
                    referrer_policy,
                    stream_id,
                    ..
                }) = &response.response
                {
                    if let Err(error) = crate::network::install_frame_navigation_timing(
                        &pending.timing,
                        url,
                        response_headers,
                        timing.as_ref(),
                        referrer_policy,
                        stream_id.is_none(),
                    ) {
                        response.response = Err(bounded_frame_error(error));
                        response.standalone_policy = None;
                        response.broker = None;
                        #[cfg(unix)]
                        if let Some(body) = response.body.take() {
                            body.lease.cancel();
                        }
                    }
                }
                response.timing = Some(pending.timing.clone());
                response.navigation_context = pending.navigation_context.take();
                response._metadata = std::mem::take(&mut pending._metadata);
                response.ancestor_capture = pending.ancestor_capture.take();
                response.authority = Some((
                    pending.identity,
                    pending.connection,
                    std::mem::take(&mut pending.attributes),
                    pending.navigation.clone(),
                ));
                completed.push((parent, frame, Some(response)));
                stale.push((parent, frame));
            }
        }
        for key in stale {
            self.frame_resource_preparations.remove(&key);
        }
        completed
    }
    pub(super) fn schedule_standalone_frame_resource(
        &mut self,
        parent: RealmId,
        frame: NodeId,
        url: String,
        referrer: Option<String>,
        post: Option<oryn_js::navigation::PostResource>,
        navigation: oryn_js::platform::NavigationRequestFlags,
        request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
        document_navigation: bool,
    ) -> Result<(), HostError> {
        if self.frame_resource_preparations.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "frame resource preparation documents".into(),
            });
        }
        let timing = oryn_js::navigation_timing::NavigationTiming::new(
            std::time::Instant::now(),
            oryn_js::navigation_timing::Kind::Navigate,
        );
        let identity = self
            .agent
            .host
            .automation_realm_identity(parent)
            .filter(|identity| identity.3)
            .ok_or_else(|| HostError::Execution("frame parent retired".into()))?;
        let dom = self
            .agent
            .host
            .automation_dom(parent)
            .ok_or_else(|| HostError::Execution("frame parent DOM absent".into()))?;
        let node = ParserScriptLease::new(&dom, frame)?;
        let ancestor_capture = self
            .agent
            .host
            .capture_frame_location_ancestors(parent, frame)?;
        let attributes = FrameResourcePreparation::attributes(&dom, frame);
        let metadata = reserve_frame_owner_metadata(
            &self.dynamic_scripts.source_bytes,
            &attributes,
            &ancestor_capture,
        )?;
        let mut request_bytes = crate::retained_size::Size(0);
        request_bytes.string(&url).map_err(HostError::Execution)?;
        request_bytes
            .optional(&referrer)
            .map_err(HostError::Execution)?;
        if let Some(post) = &post {
            request_bytes
                .vector(&post.bytes)
                .map_err(HostError::Execution)?;
            request_bytes
                .string(&post.content_type)
                .map_err(HostError::Execution)?;
        }
        let reservation =
            ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes, request_bytes.0)
                .ok_or_else(|| HostError::LimitExceeded {
                resource: "queued standalone frame request bytes".into(),
            })?;
        if !document_navigation {
            self.agent
                .host
                .reserve_frame_deferred_quota(parent, frame, &url)?;
        }
        let mut loader =
            standalone_frame_resource::Loader::capture_for_url(self, parent, frame, Some(&url))?;
        let local_blob = if url.starts_with("blob:") {
            self.agent.host.local_blob_resource(&url)?
        } else {
            None
        };
        loader.local_blob = local_blob;
        loader.navigation = navigation;
        loader.request_state = request_state;
        if document_navigation {
            loader.select_document_navigation();
        }
        let cancel = LocalParserSourceCancel(loader.cancel.clone());
        let loader_bytes = loader.retained_bytes().map_err(HostError::Execution)?;
        let loader_reservation =
            ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes, loader_bytes)
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "standalone frame loader metadata bytes".into(),
                })?;
        let broker = loader.committed_broker();
        let budget = self.dynamic_scripts.source_bytes.clone();
        let lifetime = std::sync::Arc::new(());
        let worker_lifetime = std::sync::Arc::downgrade(&lifetime);
        let (sender, receiver) = std::sync::mpsc::channel();
        let failed_sender = sender.clone();
        if let Err(error) = resource_io_executor::submit(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                loader.load(url, referrer, post)
            }))
            .map_err(|_| "standalone frame producer panicked".to_owned())
            .and_then(|result| result);
            drop(reservation);
            drop(loader_reservation);
            if worker_lifetime.upgrade().is_none() {
                return;
            }
            let (mut response, mut standalone_policy) = match result {
                Ok((response, policy)) => (Ok(response), Some(policy)),
                Err(error) => (Err(error), None),
            };
            let reservation =
                reserve_frame_response(&budget, &mut response, &mut standalone_policy);
            let broker = if response.is_ok() { broker } else { None };
            let _ = sender.send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker,
                standalone_policy,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response,
                _bytes: reservation,
                authority: None,
            });
        }) {
            let _ = failed_sender.send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response: Err(bounded_frame_error(error)),
                _bytes: None,
                authority: None,
            });
        }
        self.frame_resource_preparations.insert(
            (parent, frame),
            FrameResourcePreparation {
                _metadata: vec![metadata],
                timing,
                ancestor_capture: Some(ancestor_capture),
                _resource_cancel: None,
                _cancel: Some(cancel),
                navigation_context: None,
                identity,
                connection: self.agent.host.automation_frame_generation(parent, frame),
                attributes,
                navigation: FrameResourcePreparation::navigation_identity(&dom, frame),
                receiver,
                ready: None,
                _lifetime: lifetime,
                _node: node,
            },
        );
        Ok(())
    }
    #[cfg(unix)]
    pub(super) fn schedule_frame_resource(
        &mut self,
        parent: RealmId,
        frame: NodeId,
        request: ContextServiceRequest,
        parent_driven: bool,
    ) -> Result<(), HostError> {
        if self.frame_resource_preparations.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "frame resource preparation documents".into(),
            });
        }
        let timing = oryn_js::navigation_timing::NavigationTiming::new(
            std::time::Instant::now(),
            oryn_js::navigation_timing::Kind::Navigate,
        );
        let page_id = self
            .context
            .as_ref()
            .ok_or_else(|| HostError::Execution("frame resource context unavailable".into()))?
            .1;
        let client = self
            .agent
            .host
            .child_resource_client(parent)
            .ok_or_else(|| HostError::Execution("frame resource parent endpoint retired".into()))?;
        let identity = self
            .agent
            .host
            .automation_realm_identity(parent)
            .filter(|identity| identity.3)
            .ok_or_else(|| HostError::Execution("frame resource parent retired".into()))?;
        let dom = self
            .agent
            .host
            .automation_dom(parent)
            .ok_or_else(|| HostError::Execution("frame parent DOM unavailable".into()))?;
        let node = ParserScriptLease::new(&dom, frame)?;
        let ancestor_capture = self
            .agent
            .host
            .capture_frame_location_ancestors(parent, frame)?;
        let attributes = FrameResourcePreparation::attributes(&dom, frame);
        let metadata = reserve_frame_owner_metadata(
            &self.dynamic_scripts.source_bytes,
            &attributes,
            &ancestor_capture,
        )?;
        let request_bytes = frame_request_bytes(&request).map_err(HostError::Execution)?;
        let reservation =
            ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes, request_bytes)
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "queued frame request bytes".into(),
                })?;
        if parent_driven {
            if let ContextServiceRequest::LoadPageResource { url, .. } = &request {
                self.agent
                    .host
                    .reserve_frame_deferred_quota(parent, frame, url)?;
            }
        }
        let budget = self.dynamic_scripts.source_bytes.clone();
        let ownership = client
            .track_resource_job()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let response_lease = client
            .reserve_frame_response(page_id)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let lifetime = std::sync::Arc::new(());
        let worker_lifetime = std::sync::Arc::downgrade(&lifetime);
        let (sender, receiver) = std::sync::mpsc::channel();
        let failed_sender = sender.clone();
        let native = std::sync::Arc::new(crate::context_service::ResourceCancellation::default());
        let cancellation = standalone_stylesheet::Cancel::native(native.clone());
        let result = resource_io_executor::submit(move || {
            let _ownership = ownership;
            let response = if worker_lifetime.upgrade().is_some() {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    native.run_resource(&client, request)
                }))
                .map_err(|_| "frame resource producer panicked".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()))
            } else {
                Err("frame resource owner retired".into())
            };
            let mut body = match &response {
                Ok(ParentResponse::PageResource {
                    stream_id: Some(id),
                    worker_policy_token: Some(policy),
                    ..
                }) => {
                    response_lease.attach(*id, policy);
                    Some(frame_response_stream::FrameResponseBody {
                        client: client.clone(),
                        page_id,
                        id: *id,
                        lease: response_lease.clone(),
                    })
                }
                _ => None,
            };
            drop(reservation);
            if worker_lifetime.upgrade().is_none() {
                return;
            }
            let mut response = response.and_then(FrameResourceResponse::try_from);
            let reservation = reserve_frame_response(&budget, &mut response, &mut None);
            if response.is_err() {
                if let Some(body) = body.take() {
                    body.lease.cancel();
                }
            }
            let _ = sender.send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                body,
                navigation_context: None,
                response,
                _bytes: reservation,
                authority: None,
            });
        });
        if let Err(error) = result {
            let _ = failed_sender.send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response: Err(bounded_frame_error(error)),
                _bytes: None,
                authority: None,
            });
        }
        self.frame_resource_preparations.insert(
            (parent, frame),
            FrameResourcePreparation {
                _metadata: vec![metadata],
                timing,
                ancestor_capture: Some(ancestor_capture),
                _resource_cancel: Some(cancellation),
                _cancel: None,
                navigation_context: None,
                identity,
                connection: self.agent.host.automation_frame_generation(parent, frame),
                attributes,
                navigation: FrameResourcePreparation::navigation_identity(&dom, frame),
                receiver,
                ready: None,
                _lifetime: lifetime,
                _node: node,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_download_keeps_source_url_and_provenance_before_dispatch() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body>source</body>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://download.test/source").unwrap()),
        )
        .unwrap();
        let original = page
            .agent
            .host
            .automation_realm_identity(page.top_realm)
            .unwrap();
        for url in [
            "data:application/octet-stream;base64,AP8B",
            "blob:https://download.test/fixture#ignored",
        ] {
            page.agent
                .host
                .schedule_navigation(oryn_js::v8_bindings::LocationNavigation {
                    execution_observer: None,
                    navigation_operation: None,
                    download: Some("binary.bin".into()),
                    navigation_destination: None,
                    script_authority: None,
                    form_authority: None,
                    url: url.into(),
                    replace: false,
                    reload: false,
                    post: None,
                    referrer_policy: None,
                })
                .unwrap();
            assert_eq!(
                page.agent.host.location_url().unwrap(),
                "https://download.test/source"
            );
            assert_eq!(
                page.agent.host.request_initiator_url().unwrap(),
                "https://download.test/source"
            );
            assert_eq!(
                page.agent
                    .host
                    .automation_realm_identity(page.top_realm)
                    .unwrap(),
                original
            );
            let queued = page.agent.host.take_location_navigation().unwrap();
            assert_eq!(queued.url, url);
            assert_eq!(queued.download.as_deref(), Some("binary.bin"));
        }
    }
    fn fixture() -> (RustExecutableDocument, RealmId, NodeId) {
        let page = RustExecutableDocument::load("<body><iframe id=f srcdoc=''></iframe>").unwrap();
        let owner = page.frame_realms.values().next().unwrap().clone();
        (page, owner.parent_realm, owner.frame)
    }
    fn pending(
        page: &mut RustExecutableDocument,
        parent: RealmId,
        frame: NodeId,
    ) -> std::sync::mpsc::Sender<FrameResourceCompletion<()>> {
        let dom = page.agent.host.automation_dom(parent).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        page.frame_resource_preparations.insert(
            (parent, frame),
            FrameResourcePreparation {
                _metadata: Vec::new(),
                timing: oryn_js::navigation_timing::NavigationTiming::new(
                    std::time::Instant::now(),
                    oryn_js::navigation_timing::Kind::Navigate,
                ),
                ancestor_capture: Some(
                    page.agent
                        .host
                        .capture_frame_location_ancestors(parent, frame)
                        .unwrap(),
                ),
                _resource_cancel: None,
                _cancel: None,
                navigation_context: None,
                identity: page.agent.host.automation_realm_identity(parent).unwrap(),
                connection: page.agent.host.automation_frame_generation(parent, frame),
                attributes: FrameResourcePreparation::attributes(&dom, frame),
                navigation: FrameResourcePreparation::navigation_identity(&dom, frame),
                receiver,
                ready: None,
                _lifetime: std::sync::Arc::new(()),
                _node: ParserScriptLease::new(&dom, frame).unwrap(),
            },
        );
        sender
    }
    fn empty_context() -> FrameNavigationContext {
        FrameNavigationContext {
            execution_observer: None,
            download: None,
            session: None,
            archived_source: None,
            destination: None,
            history: None,
            replace: false,
            reload: false,
            script_authority: None,
            form_authority: None,
            initiator_url: None,
            referrer_policy: None,
            post: None,
        }
    }
    fn failed_completion() -> FrameResourceCompletion<()> {
        FrameResourceCompletion {
            _metadata: Vec::new(),
            timing: None,
            ancestor_capture: None,
            broker: None,
            standalone_policy: None,
            #[cfg(unix)]
            body: None,
            navigation_context: None,
            response: Err("fixture".into()),
            _bytes: None,
            authority: None,
        }
    }
    #[test]
    fn response_policy_capacity_is_charged_and_rejected_policy_is_released() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut origin = String::with_capacity(8192);
        origin.push_str("https://example.test");
        let policy = crate::standalone_document_policy::FramePolicy {
            csp: crate::csp::CspPolicySet::new("https://example.test/"),
            document: crate::standalone_document_policy::DocumentPolicy::root(origin, &[]),
        };
        let expected = policy.retained_bytes().unwrap();
        assert!(expected >= 8192);
        let mut policy = Some(policy);
        let mut response = Ok(FrameResourceResponse::PageResourceNoCommit {
            reason: "HTTP 204".into(),
        });
        let response_bytes = frame_response_bytes(&response).unwrap();
        let held = reserve_frame_response(&budget, &mut response, &mut policy).unwrap();
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            response_bytes + expected
        );
        drop(held);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
        let occupied =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES).unwrap();
        assert!(reserve_frame_response(&budget, &mut response, &mut policy).is_none());
        assert!(policy.is_none());
        assert_eq!(
            response.unwrap_err(),
            "queued frame response and policy bytes exceeded"
        );
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            MAX_QUEUED_SCRIPT_SOURCE_BYTES
        );
        drop(occupied);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[test]
    fn context_capacity_failure_cancels_original_producer_and_queues_one_terminal_failure() {
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let occupied =
            ScriptSourceReservation::reserve(&budget, MAX_QUEUED_SCRIPT_SOURCE_BYTES).unwrap();
        let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = outcomes.clone();
        let observer = oryn_js::navigation::NavigationExecutionObserver::new(move |outcome| {
            sink.lock().unwrap().push(outcome)
        });
        observer.loader_owned();
        let mut context = empty_context();
        context.execution_observer = Some(observer);
        context.download = Some(String::with_capacity(16384));
        let pending = page
            .frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap();
        let lifetime = Arc::downgrade(&pending._lifetime);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        pending._cancel = Some(LocalParserSourceCancel(cancel.clone()));
        pending
            .set_navigation_context(Some(context), &budget)
            .unwrap();
        assert!(lifetime.upgrade().is_none());
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(pending.navigation_context.is_none());
        assert!(sender.send(failed_completion()).is_err());
        assert!(pending.ready());
        assert_eq!(
            pending
                .ready
                .as_ref()
                .unwrap()
                .response
                .as_ref()
                .unwrap_err(),
            "frame navigation context bytes exceeded"
        );
        assert_eq!(
            *outcomes.lock().unwrap(),
            vec![oryn_js::navigation::NavigationExecutionOutcome::Failed {
                message: "frame navigation context bytes exceeded".into()
            }]
        );
        let completed = page.take_ready_frame_resources();
        assert_eq!(completed.len(), 1);
        assert!(page.take_ready_frame_resources().is_empty());
        drop(completed);
        drop(occupied);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[test]
    fn accepted_metadata_moves_into_completion_and_releases_with_last_owner() {
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut context = empty_context();
        let mut name = String::with_capacity(8192);
        name.push_str("download");
        context.download = Some(name);
        let expected = frame_context_bytes(&context).unwrap();
        assert!(expected >= 8192);
        let pending = page
            .frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap();
        pending
            .set_navigation_context(Some(context), &budget)
            .unwrap();
        let allocation = pending
            .navigation_context
            .as_ref()
            .unwrap()
            .download
            .as_ref()
            .unwrap()
            .as_ptr();
        let owner = reserve_frame_owner_metadata(
            &budget,
            &pending.attributes,
            pending.ancestor_capture.as_ref().unwrap(),
        )
        .unwrap();
        pending._metadata.push(owner);
        let admitted = budget.load(std::sync::atomic::Ordering::Acquire);
        assert!(admitted > expected);
        sender.send(failed_completion()).unwrap();
        let mut ready = page.take_ready_frame_resources();
        let completion = ready.pop().unwrap().2.unwrap();
        assert_eq!(
            completion
                .navigation_context
                .as_ref()
                .unwrap()
                .download
                .as_ref()
                .unwrap()
                .as_ptr(),
            allocation
        );
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), admitted);
        assert!(completion.current(&page.agent.host, parent, frame));
        drop(completion);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[cfg(unix)]
    #[test]
    fn typed_frame_request_charges_reserved_post_and_policy_metadata() {
        let request = ContextServiceRequest::LoadPageResource {
            download: None,
            script_authority: None,
            initiator: crate::context_service::ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                stylesheet: None,
                prepared_policy: None,
            },
            form_authority: None,
            parent_encoding: None,
            page_id: oryn_common::v3::PageId(1),
            url: "https://example.test/".into(),
            destination: "frame".into(),
            initiator_url: None,
            referrer_policy: None,
            post: None,
        };
        let baseline = frame_request_bytes(&request).unwrap();
        let mut request = request;
        if let ContextServiceRequest::LoadPageResource {
            post,
            initiator,
            referrer_policy,
            ..
        } = &mut request
        {
            *post = Some(oryn_js::navigation::PostResource {
                bytes: Vec::with_capacity(4096),
                content_type: String::with_capacity(2048),
            });
            *referrer_policy = Some(String::with_capacity(1024));
            initiator.prepared_policy = Some(String::with_capacity(8192));
            initiator.stylesheet = Some(crate::context_service::StylesheetRequest {
                integrity: Some(String::with_capacity(16384)),
                ..Default::default()
            });
        }
        assert!(
            frame_request_bytes(&request).unwrap() >= baseline + 4096 + 2048 + 1024 + 8192 + 16384
        );
    }
    #[test]
    fn bounded_error_releases_excess_capacity_and_preserves_utf8() {
        let mut error = String::with_capacity(8192);
        error.push_str("short");
        let error = bounded_frame_error(error);
        assert_eq!(error, "short");
        assert!(error.capacity() < 8192);
        let error = bounded_frame_error("界".repeat(300));
        assert!(error.len() <= 512);
        assert!(error.chars().all(|value| value == '界'));
    }
    #[test]
    fn frame_ancestry_policy_is_frozen_while_response_crosses_redirect_and_policy_edit() {
        let (mut page, parent, frame) = fixture();
        let dom = page.agent.host.automation_dom(parent).unwrap();
        dom.borrow_mut()
            .set_attribute(frame, "referrerpolicy", "no-referrer")
            .unwrap();
        page.agent.host.activate_realm(parent).unwrap();
        page.agent.host.reschedule_dynamic_frame(frame).unwrap();
        let sender = pending(&mut page, parent, frame);
        dom.borrow_mut()
            .set_attribute(frame, "referrerpolicy", "unsafe-url")
            .unwrap();
        sender
            .send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response: Err("completed fetch fixture".into()),
                _bytes: None,
                authority: None,
            })
            .unwrap();
        let ready = page.take_ready_frame_resources();
        assert_eq!(ready.len(), 1);
        let completion = ready.into_iter().next().unwrap().2.unwrap();
        assert!(completion.current(&page.agent.host, parent, frame));
        let ancestors =
            completion
                .ancestor_capture
                .unwrap()
                .resolve(&oryn_js::origin::Origin::from_url(
                    "https://redirected.test/",
                ));
        assert_eq!(ancestors.len(), 1);
        assert!(ancestors[0].is_opaque());
    }
    #[test]
    fn frame_fetch_flags_distinguish_existing_history_entry_from_new_destination() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let history = page.agent.host.navigation_history_snapshot().unwrap();
        let destination = history.entries[history.current].clone();
        let mut context = FrameNavigationContext {
            execution_observer: None,
            download: None,
            session: None,
            archived_source: None,
            destination: Some(destination),
            history: Some(history),
            replace: false,
            reload: false,
            script_authority: None,
            form_authority: None,
            initiator_url: None,
            referrer_policy: None,
            post: None,
        };
        let state = oryn_common::navigation_request::NavigationRequestState {
            initiator_url: Some("https://original.test/private".into()),
            opaque_initiator: false,
            referrer: None,
            referrer_policy: "no-referrer".into(),
        };
        let entry = context.destination.as_ref().unwrap();
        let request = frame_navigation_history::FrameRequest::capture(
            Some(state.clone()),
            None,
            None,
            &page.dynamic_scripts.source_bytes,
        )
        .unwrap();
        context.session = Some(frame_navigation_history::SessionHistory {
            names: None,
            _metadata: None,
            requests: Arc::new(BTreeMap::from([(entry.document_id.clone(), request)])),
            update: Arc::new(context.history.clone().unwrap()),
            origins: Arc::new(BTreeMap::new()),
        });
        assert_eq!(context.saved_request_state(), Some(state.clone()));
        assert_eq!(
            context.fetch_navigation_flags(),
            oryn_js::platform::NavigationRequestFlags {
                reload: false,
                history: true
            }
        );
        context.reload = true;
        assert_eq!(
            context.fetch_navigation_flags(),
            oryn_js::platform::NavigationRequestFlags {
                reload: true,
                history: false
            }
        );
        assert_eq!(context.saved_request_state(), Some(state.clone()));
        let destination = context.destination.take();
        assert_eq!(context.saved_request_state(), Some(state));
        context.destination = destination;
        context.reload = false;
        context.destination.as_mut().unwrap().id = "new-destination".into();
        assert_eq!(context.fetch_navigation_flags(), Default::default());
        assert_eq!(context.saved_request_state(), None);
    }
    #[test]
    fn dropping_standalone_frame_preparation_cancels_io_before_late_publication() {
        let (mut page, parent, frame) = fixture();
        let _sender = pending(&mut page, parent, frame);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        page.frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap()
            ._cancel = Some(LocalParserSourceCancel(cancel.clone()));
        page.frame_resource_preparations.remove(&(parent, frame));
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn frame_worker_reply_preserves_isolate_archived_source_custody() {
        fn assert_send<T: Send>() {}
        assert_send::<FrameResourceCompletion<()>>();
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let archive =
            frame_navigation_history::ArchivedSource::capture("<input value=saved>", None, &budget)
                .unwrap();
        let weak = Arc::downgrade(&archive);
        let mut context = empty_context();
        context.archived_source = Some(archive);
        page.frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap()
            .set_navigation_context(Some(context), &budget)
            .unwrap();
        assert!(
            std::thread::spawn(move || sender.send(failed_completion()).is_ok())
                .join()
                .unwrap()
        );
        assert!(page.frame_resources_runnable());
        assert!(weak.upgrade().is_some());
        let completed = page.take_ready_frame_resources();
        let retained = completed[0]
            .2
            .as_ref()
            .unwrap()
            .navigation_context
            .as_ref()
            .unwrap()
            .archived_source
            .as_ref()
            .unwrap();
        assert!(Arc::ptr_eq(retained, &weak.upgrade().unwrap()));
        assert_eq!(retained.source.as_ref(), "<input value=saved>");
        assert!(page.take_ready_frame_resources().is_empty());
        drop(completed);
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);

        let sender = pending(&mut page, parent, frame);
        let archive =
            frame_navigation_history::ArchivedSource::capture("retired", None, &budget).unwrap();
        let weak = Arc::downgrade(&archive);
        let mut context = empty_context();
        context.archived_source = Some(archive);
        page.frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap()
            .set_navigation_context(Some(context), &budget)
            .unwrap();
        page.frame_resource_preparations.remove(&(parent, frame));
        assert!(weak.upgrade().is_none());
        assert!(sender.send(failed_completion()).is_err());
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn frame_resource_completion_retains_native_navigation_context() {
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        page.frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap()
            .navigation_context = Some(FrameNavigationContext {
            execution_observer: None,
            download: None,
            session: None,
            archived_source: None,
            destination: None,
            history: None,
            replace: true,
            reload: false,
            script_authority: None,
            form_authority: None,
            initiator_url: Some("https://example.test/source".into()),
            referrer_policy: None,
            post: None,
        });
        sender
            .send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response: Err("network terminal".into()),
                _bytes: None,
                authority: None,
            })
            .unwrap();
        let completed = page.take_ready_frame_resources();
        let context = completed[0]
            .2
            .as_ref()
            .unwrap()
            .navigation_context
            .as_ref()
            .unwrap();
        assert!(context.replace);
        assert_eq!(
            context.initiator_url.as_deref(),
            Some("https://example.test/source")
        );
    }

    #[test]
    fn frame_resource_replacement_discards_ready_result_and_shared_byte_custody() {
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        let budget = page.dynamic_scripts.source_bytes.clone();
        let baseline = budget.load(std::sync::atomic::Ordering::Acquire);
        sender
            .send(FrameResourceCompletion {
                _metadata: Vec::new(),
                timing: None,
                ancestor_capture: None,
                broker: None,
                standalone_policy: None,
                #[cfg(unix)]
                body: None,
                navigation_context: None,
                response: Err("old request".into()),
                _bytes: Some(ScriptSourceReservation::reserve(&budget, 4096).unwrap()),
                authority: None,
            })
            .unwrap();
        assert!(page.frame_resources_runnable());
        assert!(budget.load(std::sync::atomic::Ordering::Acquire) >= 4096);
        page.agent.host.activate_realm(parent).unwrap();
        page.agent
            .host
            .execute("document.getElementById('f').srcdoc='<p>replacement</p>'")
            .unwrap();
        assert!(page.take_ready_frame_resources().is_empty());
        assert!(page.frame_resource_preparations.is_empty());
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), baseline);
    }
    #[test]
    fn pending_frame_resource_gates_parent_load_and_stop_cancels_local_publication() {
        let (mut page, parent, frame) = fixture();
        let sender = pending(&mut page, parent, frame);
        assert!(page.frame_resources_pending(Some(parent)));
        assert!(!page.frame_resources_runnable());
        page.agent.host.activate_realm(parent).unwrap();
        page.agent
            .host
            .set_document_readiness(oryn_dom::DocumentReadyState::Loading)
            .unwrap();
        // This fixture starts completed; begin a new test publication receipt.
        page.document_publications.remove(&parent);
        page.queue_document_load(parent).unwrap();
        assert!(page.pending_document_loads.contains_key(&parent));
        page.stop_pending_document_load().unwrap();
        assert!(page.frame_resource_preparations.is_empty());
        assert!(
            sender
                .send(FrameResourceCompletion {
                    _metadata: Vec::new(),
                    timing: None,
                    ancestor_capture: None,
                    broker: None,
                    standalone_policy: None,
                    #[cfg(unix)]
                    body: None,
                    navigation_context: None,
                    response: Err("late request".into()),
                    _bytes: None,
                    authority: None
                })
                .is_err()
        );
    }
    #[test]
    fn frame_resource_disconnect_becomes_one_runnable_terminal_failure() {
        let (mut page, parent, frame) = fixture();
        drop(pending(&mut page, parent, frame));
        assert!(page.frame_resources_runnable());
        let completed = page.take_ready_frame_resources();
        assert_eq!(completed.len(), 1);
        assert!(completed[0].2.as_ref().unwrap().response.is_err());
        assert!(!page.frame_resources_pending(None));
        assert!(!page.frame_resources_runnable());
    }
    #[cfg(unix)]
    #[test]
    fn native_frame_resource_producer_yields_and_retains_post_and_initiator_until_reply() {
        use crate::context_service::ContextTransport;
        struct Policy {
            root: (u64, u64),
            observed: std::sync::mpsc::Sender<ContextServiceRequest>,
            release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl ContextTransport for Policy {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ParentResponse, crate::RuntimeError> {
                if let ContextServiceRequest::MessageWire {
                    source_page_id,
                    request:
                        oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                            document_generation,
                            stream_epoch,
                        },
                } = &request
                {
                    if *source_page_id != oryn_common::v3::PageId(1)
                        || (*document_generation, *stream_epoch) != self.root
                    {
                        return Err(crate::RuntimeError::ContextService(format!(
                            "frame fixture root registration mismatch: page={source_page_id:?}, identity={:?}; expected page=PageId(1), identity={:?}",
                            (*document_generation, *stream_epoch),
                            self.root
                        )));
                    }
                    return Ok(ParentResponse::MessageWire {
                        result: Ok(oryn_js::message_wire::MessageResponse::Done),
                    });
                }
                match request {
                    ContextServiceRequest::MessageWire {
                        request:
                            oryn_js::message_wire::MessageRequest::RetireRootDocument { .. }
                            | oryn_js::message_wire::MessageRequest::RetireWindow { .. },
                        ..
                    } => Ok(ParentResponse::MessageWire {
                        result: Ok(oryn_js::message_wire::MessageResponse::Done),
                    }),
                    ContextServiceRequest::PublishNativeWindowInventory { .. } => {
                        Ok(ParentResponse::NativeWindowInventoryPublished)
                    }
                    request @ ContextServiceRequest::ReserveDeferredContainer { .. } => {
                        self.observed.send(request).map_err(|error| {
                            crate::RuntimeError::ContextService(error.to_string())
                        })?;
                        Ok(ParentResponse::DeferredFetchResult { result: Ok(()) })
                    }
                    ContextServiceRequest::ReserveScriptLoad { page_id, creator } => {
                        let expected = oryn_js::worker_wire::Creator::Window {
                            target: oryn_js::message_wire::WindowTarget::Root,
                            document_generation: self.root.0,
                            stream_epoch: self.root.1,
                        };
                        if page_id != oryn_common::v3::PageId(1) || creator != expected {
                            return Err(crate::RuntimeError::ContextService(
                                "frame fixture reserve authority mismatch".into(),
                            ));
                        }
                        self.observed
                            .send(ContextServiceRequest::ReserveScriptLoad { page_id, creator })
                            .map_err(|error| {
                                crate::RuntimeError::ContextService(error.to_string())
                            })?;
                        Ok(ParentResponse::ScriptLoadReserved { token: 73 })
                    }
                    ContextServiceRequest::RunScriptLoad {
                        page_id,
                        token,
                        render_blocking,
                        request,
                    } => {
                        if page_id != oryn_common::v3::PageId(1) || token != 73 || render_blocking {
                            return Err(crate::RuntimeError::ContextService(
                                "frame fixture run admission mismatch".into(),
                            ));
                        }
                        if !matches!(&*request, ContextServiceRequest::LoadPageResource { .. }) {
                            return Err(crate::RuntimeError::ContextService(
                                "frame fixture wrapped resource mismatch".into(),
                            ));
                        }
                        self.observed.send(*request).map_err(|error| {
                            crate::RuntimeError::ContextService(error.to_string())
                        })?;
                        // Always return a protocol response, including timeout: panicking in
                        // the server worker leaves the client's reply receiver live forever.
                        self.release
                            .lock()
                            .map_err(|error| {
                                crate::RuntimeError::ContextService(error.to_string())
                            })?
                            .recv_timeout(Duration::from_secs(3))
                            .map_err(|error| {
                                crate::RuntimeError::ContextService(error.to_string())
                            })?;
                        Err(crate::RuntimeError::ContextService(
                            "fixture network failure".into(),
                        ))
                    }
                    request @ ContextServiceRequest::CancelScriptLoad { page_id, token } => {
                        if page_id != oryn_common::v3::PageId(1) || token != 73 {
                            return Err(crate::RuntimeError::ContextService(
                                "frame fixture cleanup admission mismatch".into(),
                            ));
                        }
                        self.observed.send(request).map_err(|error| {
                            crate::RuntimeError::ContextService(error.to_string())
                        })?;
                        Ok(ParentResponse::ScriptLoadCancelled)
                    }
                    ContextServiceRequest::ReleaseDeferredContainer { page_id, .. }
                        if page_id == oryn_common::v3::PageId(1) =>
                    {
                        Ok(ParentResponse::DeferredFetchResult { result: Ok(()) })
                    }
                    unexpected => Err(crate::RuntimeError::ContextService(format!(
                        "unexpected frame fixture request: {unexpected:?}"
                    ))),
                }
            }
        }
        let (worker, parent_socket) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent_socket.try_clone().unwrap();
        let (observed, requests) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (mut page, parent, frame) = fixture();
        struct ShutdownOnDrop(std::os::unix::net::UnixStream);
        impl Drop for ShutdownOnDrop {
            fn drop(&mut self) {
                let _ = self.0.shutdown(std::net::Shutdown::Both);
            }
        }
        // Declared after page so a failed assertion closes IPC before page retirement.
        let _shutdown_on_unwind = ShutdownOnDrop(shutdown.try_clone().unwrap());
        let (_, generation, epoch, _) = page.agent.host.automation_realm_identity(parent).unwrap();
        let server = std::thread::spawn(move || {
            crate::context_service::serve_context_transport(
                parent_socket,
                std::sync::Arc::new(Policy {
                    root: (generation, epoch),
                    observed,
                    release: std::sync::Mutex::new(wait),
                }),
            )
        });
        let client = crate::context_service::SharedContextClient::new(
            crate::context_service::IpcContextClient::new(worker),
        );
        let document_bytes = page
            .dynamic_scripts
            .source_bytes
            .load(std::sync::atomic::Ordering::Acquire);
        page.agent.host.install_test_resource_endpoint(
            parent,
            client.clone(),
            oryn_common::v3::PageId(1),
        );
        page.context = Some((client.clone(), oryn_common::v3::PageId(1), None));
        let (_, generation, epoch, _) = page.agent.host.automation_realm_identity(parent).unwrap();
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: generation,
            stream_epoch: epoch,
        };
        let body = vec![7u8; 4096];
        page.schedule_frame_resource(
            parent,
            frame,
            ContextServiceRequest::LoadPageResource {
                download: None,
                script_authority: None,
                initiator: crate::context_service::ResourceInitiator {
                    embedded_type: None,
                    request_state: None,
                    navigation: Default::default(),
                    creator: creator.clone(),
                    stylesheet: None,
                    prepared_policy: None,
                },
                form_authority: None,
                parent_encoding: Some("UTF-8".into()),
                page_id: oryn_common::v3::PageId(1),
                url: "https://example.test/submit".into(),
                destination: "frame".into(),
                initiator_url: Some("https://example.test/source".into()),
                referrer_policy: Some("origin".into()),
                post: Some(oryn_js::navigation::PostResource {
                    bytes: body.clone(),
                    content_type: "application/octet-stream".into(),
                }),
            },
            true,
        )
        .unwrap();
        match requests.recv_timeout(Duration::from_secs(3)).unwrap() {
            ContextServiceRequest::ReserveDeferredContainer {
                creator: reserved_parent,
                frame: token,
                source,
                ..
            } => {
                assert_eq!(reserved_parent, creator);
                assert_eq!(
                    Some(token),
                    page.agent.host.automation_frame_token(parent, frame)
                );
                assert_eq!(source, "https://example.test/submit");
            }
            _ => panic!("iframe quota must be reserved before response IO"),
        }
        match requests.recv_timeout(Duration::from_secs(3)).unwrap() {
            ContextServiceRequest::ReserveScriptLoad {
                page_id,
                creator: reserved_creator,
            } => {
                assert_eq!(page_id, oryn_common::v3::PageId(1));
                assert_eq!(reserved_creator, creator);
            }
            _ => panic!(
                "frame resource must reserve its exact native creator before running payload"
            ),
        }
        page.agent
            .host
            .execute("globalThis.duringFrameFetch=42")
            .unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(duringFrameFetch)")
                .unwrap(),
            "42"
        );
        assert!(page.frame_resources_pending(Some(parent)));
        assert!(!page.frame_resources_runnable());
        assert!(
            page.dynamic_scripts
                .source_bytes
                .load(std::sync::atomic::Ordering::Acquire)
                >= body.len()
        );
        match requests.recv_timeout(Duration::from_secs(3)).unwrap() {
            ContextServiceRequest::LoadPageResource {
                initiator,
                post,
                initiator_url,
                referrer_policy,
                ..
            } => {
                assert_eq!(initiator.creator, creator);
                assert_eq!(post.unwrap().bytes, body);
                assert_eq!(
                    initiator_url.as_deref(),
                    Some("https://example.test/source")
                );
                assert_eq!(referrer_policy.as_deref(), Some("origin"));
            }
            _ => panic!("wrong native frame request"),
        }
        release.send(()).unwrap();
        let pending = page
            .frame_resource_preparations
            .get_mut(&(parent, frame))
            .unwrap();
        pending.ready = Some(
            pending
                .receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .into_isolate_completion(),
        );
        let completed = page.take_ready_frame_resources();
        assert_eq!(completed.len(), 1);
        assert!(
            completed[0]
                .2
                .as_ref()
                .unwrap()
                .current(&page.agent.host, parent, frame)
        );
        assert!(completed[0].2.as_ref().unwrap().response.is_err());
        drop(completed);
        assert_eq!(
            page.dynamic_scripts
                .source_bytes
                .load(std::sync::atomic::Ordering::Acquire),
            document_bytes
        );
        client.drain_script_cleanup(Duration::from_secs(3)).unwrap();
        match requests.recv_timeout(Duration::from_secs(3)).unwrap() {
            ContextServiceRequest::CancelScriptLoad { page_id, token } => {
                assert_eq!(page_id, oryn_common::v3::PageId(1));
                assert_eq!(token, 73);
            }
            _ => panic!("terminal frame response must release its exact native admission"),
        }
        assert!(
            matches!(
                requests.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "one terminal response releases one admission"
        );
        page.context = None;
        drop(page);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
    }

    #[test]
    fn failed_frame_navigation_commits_opaque_document_and_one_load_for_new_and_existing_contexts()
    {
        for existing in [false, true] {
            let mut page = RustExecutableDocument::load_with_environment(
                "<body></body>",
                &BTreeMap::new(),
                None,
                Some(Url::parse("https://example.test/root").unwrap()),
            )
            .unwrap();
            page.agent.host.execute("globalThis.events=[];globalThis.f=document.createElement('iframe');f.id='f';f.src='failed.html';document.body.appendChild(f);globalThis.proxy=f.contentWindow").unwrap();
            let parent = page.top_realm;
            let frame = page.agent.dom.borrow().find_by_id("f").unwrap();
            if existing {
                page.agent
                    .host
                    .execute("f.srcdoc='<p>previous document</p>'")
                    .unwrap();
                for _ in 0..16 {
                    page.drain_tasks().unwrap();
                    if !page.frame_scripts_pending(None) {
                        break;
                    }
                }
                page.agent.host.activate_realm(parent).unwrap();
                page.agent
                    .host
                    .execute("f.removeAttribute('srcdoc');f.src='failed-again.html'")
                    .unwrap();
            }
            page.agent.host.execute("f.onload=e=>events.push('load:'+e.isTrusted);f.onerror=()=>events.push('error')").unwrap();
            page.agent
                .host
                .acknowledge_parser_frame_navigation(frame)
                .unwrap();
            let sender = pending(&mut page, parent, frame);
            sender
                .send(FrameResourceCompletion {
                    _metadata: Vec::new(),
                    timing: None,
                    ancestor_capture: None,
                    broker: None,
                    standalone_policy: None,
                    #[cfg(unix)]
                    body: None,
                    navigation_context: None,
                    response: Err("network failure".into()),
                    _bytes: None,
                    authority: None,
                })
                .unwrap();
            page.drain_dynamic_frames().unwrap();
            for _ in 0..16 {
                page.poll_frame_script_preparations().unwrap();
                page.flush_document_loads().unwrap();
                if !page.frame_scripts_pending(None) {
                    break;
                }
            }
            page.agent.host.activate_realm(parent).unwrap();
            assert_eq!(
                page.agent.host.evaluate_string("events.join(',')").unwrap(),
                "load:true"
            );
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("String(proxy===f.contentWindow&&f.contentDocument===null)")
                    .unwrap(),
                "true"
            );
            assert_eq!(page.agent.host.evaluate_string("(()=>{try{f.contentWindow.document;return 'exposed'}catch(e){return e.name}})()").unwrap(),"SecurityError");
        }
    }

    #[test]
    fn no_commit_frame_response_preserves_current_document_without_load() {
        for reason in ["HTTP 204", "HTTP 205", "download"] {
            let mut page =
                RustExecutableDocument::load("<iframe id=f srcdoc='<p>current</p>'></iframe>")
                    .unwrap();
            let parent = page.top_realm;
            page.agent.host.activate_realm(parent).unwrap();
            let frame = page.node_by_id("f").unwrap();
            let previous = page
                .automation_realms()
                .into_iter()
                .find(|realm| realm.parent_realm == Some(parent) && realm.live)
                .unwrap();
            page.agent.host.execute("globalThis.events=[];globalThis.f=document.getElementById('f');globalThis.original=f.contentDocument;f.onload=()=>events.push('load');f.removeAttribute('srcdoc');f.src='https://example.test/no-commit'").unwrap();
            page.agent
                .host
                .acknowledge_parser_frame_navigation(frame)
                .unwrap();
            let sender = pending(&mut page, parent, frame);
            let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = outcomes.clone();
            let observer = oryn_js::navigation::NavigationExecutionObserver::new(move |outcome| {
                sink.lock().unwrap().push(outcome)
            });
            observer.loader_owned();
            page.frame_resource_preparations
                .get_mut(&(parent, frame))
                .unwrap()
                .navigation_context = Some(FrameNavigationContext {
                execution_observer: Some(observer),
                download: None,
                session: None,
                archived_source: None,
                destination: None,
                history: None,
                replace: false,
                reload: false,
                script_authority: None,
                form_authority: None,
                initiator_url: None,
                referrer_policy: None,
                post: None,
            });
            sender
                .send(FrameResourceCompletion {
                    _metadata: Vec::new(),
                    timing: None,
                    ancestor_capture: None,
                    broker: None,
                    standalone_policy: None,
                    #[cfg(unix)]
                    body: None,
                    navigation_context: None,
                    response: Ok(FrameResourceResponse::PageResourceNoCommit {
                        reason: reason.into(),
                    }),
                    _bytes: None,
                    authority: None,
                })
                .unwrap();
            page.drain_dynamic_frames().unwrap();
            page.agent.host.activate_realm(parent).unwrap();
            assert_eq!(
                page.agent.host.automation_realm_identity(previous.realm),
                Some((
                    previous.document_root,
                    previous.document_generation,
                    previous.stream_epoch,
                    true
                ))
            );
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("String(f.contentDocument===original&&events.length===0)")
                    .unwrap(),
                "true"
            );
            assert_eq!(
                *outcomes.lock().unwrap(),
                vec![oryn_js::navigation::NavigationExecutionOutcome::Completed]
            );
        }
    }
}

fn frame_response_bytes(response: &Result<FrameResourceResponse, String>) -> Result<usize, String> {
    match response {
        Ok(response) => response.retained_bytes(),
        Err(error) => std::mem::size_of::<String>()
            .checked_add(error.capacity())
            .ok_or_else(|| "frame response retained byte accounting overflow".to_owned()),
    }
}

fn reserve_frame_owner_metadata(
    budget: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    attributes: &Vec<Option<String>>,
    ancestors: &oryn_js::v8_bindings::LocationAncestorCapture,
) -> Result<std::sync::Arc<ScriptSourceReservation>, HostError> {
    let mut bytes = crate::retained_size::Size(
        std::mem::size_of::<FrameResourcePreparation>()
            + std::mem::size_of::<FrameResourceCompletion>()
            + 1024,
    );
    bytes.vector(attributes).map_err(HostError::Execution)?;
    for value in attributes {
        bytes.optional(value).map_err(HostError::Execution)?;
    }
    bytes
        .add(
            ancestors
                .retained_bytes()
                .ok_or_else(|| HostError::Execution("frame ancestor bytes overflow".into()))?,
        )
        .map_err(HostError::Execution)?;
    ScriptSourceReservation::reserve(budget, bytes.0)
        .map(std::sync::Arc::new)
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "frame owner metadata bytes".into(),
        })
}
fn frame_context_bytes(context: &FrameNavigationContext) -> Result<usize, String> {
    let mut bytes = crate::retained_size::Size(std::mem::size_of::<FrameNavigationContext>());
    for value in [
        &context.download,
        &context.initiator_url,
        &context.referrer_policy,
    ] {
        bytes.optional(value)?;
    }
    if let Some(post) = &context.post {
        bytes.vector(&post.bytes)?;
        bytes.string(&post.content_type)?;
    }
    if let Some(authority) = &context.form_authority {
        bytes.vector(&authority.0)?;
    }
    if let Some(admission) = &context.script_authority {
        bytes.vector(&admission.authority.0)?;
        if let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame(frame),
            ..
        } = &admission.target
        {
            bytes.string(frame)?;
        }
    }
    fn entry(
        bytes: &mut crate::retained_size::Size,
        entry: &oryn_js::navigation_api::NavigationHistorySnapshot,
    ) -> Result<(), String> {
        for value in [&entry.key, &entry.id, &entry.url, &entry.document_id] {
            bytes.string(value)?;
        }
        for state in [&entry.state, &entry.history_state] {
            if let Some(state) = state {
                bytes.add(state.retained_bytes())?;
            }
        }
        Ok(())
    }
    if let Some(destination) = &context.destination {
        entry(&mut bytes, destination)?;
    }
    if let Some(history) = &context.history {
        bytes.vector(&history.entries)?;
        bytes.string(&history.navigation_type)?;
        for value in &history.entries {
            entry(&mut bytes, value)?;
        }
    }
    // SessionHistory and ArchivedSource are shared Arcs carrying their own
    // independent reservations. Their backing storage is not cloned here.
    Ok(bytes.0)
}

fn bounded_frame_error(error: String) -> String {
    if error.len() <= 512 {
        return error.into_boxed_str().into_string();
    }
    let mut end = 512;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_owned()
}
fn reserve_frame_response(
    budget: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    response: &mut Result<FrameResourceResponse, String>,
    policy: &mut Option<crate::standalone_document_policy::FramePolicy>,
) -> Option<ScriptSourceReservation> {
    let bytes = frame_response_bytes(response).and_then(|bytes| {
        let policy = policy
            .as_ref()
            .map(|value| value.retained_bytes())
            .transpose()?
            .unwrap_or(0);
        bytes
            .checked_add(policy)
            .ok_or_else(|| "frame response and policy bytes overflow".into())
    });
    if let Ok(bytes) = bytes {
        if let Some(reservation) = ScriptSourceReservation::reserve(budget, bytes) {
            return Some(reservation);
        }
    }
    // The preparation's admitted owner metadata reserves terminal-error room.
    // Release every rejected response/policy allocation before queueing failure.
    *response = Err("queued frame response and policy bytes exceeded".into());
    *policy = None;
    None
}

#[cfg(unix)]
fn frame_request_bytes(request: &ContextServiceRequest) -> Result<usize, String> {
    let ContextServiceRequest::LoadPageResource {
        download,
        script_authority,
        page_id: _,
        initiator,
        form_authority,
        url,
        destination,
        parent_encoding,
        initiator_url,
        referrer_policy,
        post,
    } = request
    else {
        return Err("frame request has unexpected native operation".into());
    };
    let mut size = crate::retained_size::Size(std::mem::size_of::<ContextServiceRequest>());
    for value in [url, destination] {
        size.string(value)?;
    }
    for value in [
        download,
        parent_encoding,
        initiator_url,
        referrer_policy,
        &initiator.embedded_type,
        &initiator.prepared_policy,
    ] {
        size.optional(value)?;
    }
    fn creator(
        size: &mut crate::retained_size::Size,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<(), String> {
        if let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame(frame),
            ..
        } = creator
        {
            size.string(frame)?;
        }
        Ok(())
    }
    creator(&mut size, &initiator.creator)?;
    if let Some(script) = script_authority {
        size.vector(&script.authority.0)?;
        creator(&mut size, &script.target)?;
    }
    if let Some(form) = form_authority {
        size.vector(&form.0)?;
    }
    if let Some(post) = post {
        size.vector(&post.bytes)?;
        size.string(&post.content_type)?;
    }
    if let Some(state) = &initiator.request_state {
        state.retained_bytes().map_err(str::to_owned)?;
        size.optional(&state.initiator_url)?;
        size.optional(&state.referrer)?;
        size.string(&state.referrer_policy)?;
    }
    if let Some(style) = &initiator.stylesheet {
        for value in [
            &style.policy,
            &style.nonce,
            &style.integrity,
            &style.cross_origin,
        ] {
            size.optional(value)?;
        }
    }
    Ok(size.0)
}
