// Parent-admitted auxiliary roots created during window.open's native callback.
// The actor claims their existing DOM/realm after the callback returns.
use super::*;

pub(crate) struct CreatedAuxiliaryRoot {
    pub page_id: PageId,
    pub opener_page_id: Option<PageId>,
    pub route: crate::worker_route::WorkerRoute,
    pub actor_route: crate::worker_route::WorkerRoute,
    pub realm: RealmId,
    pub dom: SharedDom,
    pub document: NodeId,
    pub context: SharedContextClient,
    pub broker: Option<SharedNetworkBroker>,
    pub url: Url,
    pub policy: crate::csp::CspPolicySet,
    pub referrer_policy: crate::referrer::ReferrerPolicy,
    pub isolation: crate::agent_isolation::DocumentIsolationPolicy,
}
#[derive(Default)]
struct AuxiliaryRoots {
    pending: Vec<CreatedAuxiliaryRoot>,
    pages: BTreeMap<PageId, RealmId>,
    actors: BTreeMap<crate::worker_route::WorkerRoute, crate::worker_route::WorkerRoute>,
}
struct RootActorRoute(crate::worker_route::WorkerRoute);

struct RootCreationRollback {
    context: SharedContextClient,
    page: PageId,
    expected_generation: Option<u64>,
    armed: bool,
}
impl Drop for RootCreationRollback {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .context
                .request(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: self.page,
                    expected_generation: self.expected_generation,
                });
        }
    }
}

pub(super) fn create_admitted_auxiliary_root(
    scope: &mut v8::PinScope,
    page_id: PageId,
    opener_page_id: Option<PageId>,
    admission: crate::context_service::RelatedRootAdmission,
    initial_navigation: Option<&str>,
) -> Result<(), HostError> {
    let creator = scope.get_current_context();
    let source = current_context_endpoint(scope)
        .ok_or_else(|| HostError::Execution("auxiliary creator endpoint missing".into()))?;
    let source_route = source
        .client
        .environment_route()
        .ok_or_else(|| HostError::Execution("auxiliary creator route missing".into()))?;
    let actor_route = creator
        .get_slot::<RootActorRoute>()
        .map(|key| key.0.clone())
        .or_else(|| {
            scope
                .get_slot::<AuxiliaryRoots>()
                .and_then(|registry| registry.actors.get(&source_route))
                .cloned()
        })
        .or_else(|| source.client.actor_route())
        .unwrap_or_else(|| source_route.clone());
    if admission.actor_route != actor_route
        || admission.route.process_session != source_route.process_session
        || admission.route.environment != page_id.0
        || opener_page_id != Some(source.page_id)
    {
        return Err(HostError::Execution(
            "auxiliary admission does not match native creator".into(),
        ));
    }
    let policy = admission
        .policy
        .clone()
        .ok_or_else(|| HostError::Execution("auxiliary policy admission missing".into()))?;
    if scope.get_slot::<AuxiliaryRoots>().is_none() {
        scope.set_slot(AuxiliaryRoots::default());
    }
    let registry = scope.get_slot::<AuxiliaryRoots>().unwrap();
    if registry.pages.contains_key(&page_id) || registry.pages.len() >= 256 {
        return Err(HostError::Execution(
            "auxiliary root identity unavailable".into(),
        ));
    }
    let creator_state = creator
        .get_slot::<V8BindingState>()
        .ok_or_else(|| HostError::Execution("auxiliary creator bindings missing".into()))?;
    let base = creator_state
        .native_document_base_url(true)
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let referrer = creator_state.document_referrer_policy();
    let context = source
        .client
        .related_endpoint(&admission)
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let mut rollback = RootCreationRollback {
        context: context.clone(),
        page: page_id,
        expected_generation: None,
        armed: true,
    };
    let response = context
        .request(ContextServiceRequest::GetNativeDocumentAuthority {
            page_id,
            document_url: "about:blank".into(),
        })
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let ContextServiceResponse::NativeDocumentAuthority {
        serialized,
        opaque_key,
        secure_context,
        document_generation,
        initial_document,
    } = response
    else {
        return Err(HostError::Execution(
            "auxiliary authority response mismatch".into(),
        ));
    };
    if !initial_document {
        return Err(HostError::Execution(
            "auxiliary initial authority already consumed".into(),
        ));
    }
    rollback.expected_generation = Some(document_generation);
    let origin = Origin::from_native_authority(&serialized, opaque_key.as_deref())
        .map_err(|error| HostError::Execution(error.into()))?;
    let broker = if admission.direct {
        source
            .client
            .related_network_broker(&admission)
            .map_err(|error| HostError::Execution(error.to_string()))?
    } else {
        current_fetch_endpoint(scope).map(|fetch| fetch.broker.for_route(admission.route.clone()))
    };
    let fetch = broker.clone().map(|broker| FetchState {
        broker,
        base_url: Url::parse(&base).unwrap_or_else(|_| Url::parse("about:blank").unwrap()),
        next_script_request: Rc::new(std::cell::Cell::new(900_000)),
    });
    let endpoints = RealmEndpoints {
        fetch,
        context: Some(ContextState {
            client: context.clone(),
            page_id,
        }),
        activity_parent: None,
    };
    let parsed = oryn_dom::html::parse("<!doctype html><html><head></head><body></body></html>");
    let dom = Rc::new(RefCell::new(
        parsed
            .into_runtime()
            .map_err(|error| HostError::Execution(error.to_string()))?,
    ));
    let document = dom.borrow().document();
    dom.borrow_mut()
        .arena_mut()
        .get_mut(document)
        .map_err(|error| HostError::Execution(error.to_string()))?
        .document_metadata
        .as_mut()
        .ok_or_else(|| HostError::Execution("auxiliary metadata missing".into()))?
        .ready_state = oryn_dom::DocumentReadyState::Complete;
    let (realm, global) =
        create_native_realm_context(scope, origin, document_generation, None, None, endpoints)?;
    let local = v8::Local::new(scope, &global);
    let result = (|| {
        local.set_slot(Rc::new(oryn_js::realm::SecureContextState(secure_context)));
        local.set_slot(Rc::new(NativeInitialDocument(true)));
        local.set_slot(Rc::new(RootActorRoute(actor_route.clone())));
        let inherited_policy = creator.get_slot::<DocumentCodePolicy>();
        let state = install_native_document(
            scope,
            local,
            dom.clone(),
            realm,
            document_generation,
            document,
            inherited_policy.as_deref(),
        )?;
        state.set_layout_provider(crate::executable::initial_layout_provider(
            dom.clone(),
            document,
        ));
        state.set_inherited_base_url(Some(base));
        state
            .set_document_referrer_policy(&referrer)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        state.set_navigation_initial_document(true);
        state
            .commit_location("about:blank")
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let opener_window = creator.global(scope);
        let scope = &mut v8::ContextScope::new(scope, local);
        if !begin_runtime_bootstrap(local) {
            return Err(HostError::Execution(
                "auxiliary bootstrap already admitted".into(),
            ));
        }
        for source in [
            crate::executable::RUST_RUNTIME_GLOBALS,
            crate::observable::OBSERVABLE_GLOBALS,
        ] {
            let source = v8::String::new(scope, source)
                .ok_or_else(|| HostError::Execution("auxiliary bootstrap allocation".into()))?;
            v8::Script::compile(scope, source, None)
                .and_then(|script| script.run(scope))
                .ok_or_else(|| HostError::Execution("auxiliary bootstrap failed".into()))?;
        }
        let source = v8::String::new(
            scope,
            &format!(
                "__oryn_configure_context({}, {})",
                page_id.0, source.page_id.0
            ),
        )
        .ok_or_else(|| HostError::Execution("auxiliary configuration allocation".into()))?;
        v8::Script::compile(scope, source, None)
            .and_then(|script| script.run(scope))
            .ok_or_else(|| HostError::Execution("auxiliary configuration failed".into()))?;
        let window = local.global(scope);
        oryn_js::v8_bindings::set_native_window_opener(scope, window, Some(opener_window))
            .map_err(|error| HostError::Execution(error.to_string()))?;
        finish_runtime_bootstrap(scope, local)?;
        Ok(())
    })();
    // Authorize from the opener route, never the child's about:blank route.
    // This runs after native document publication and before committing creation,
    // so stale source/target authority or scheduling failure rolls the root back.
    let result = result.and_then(|()| {
        if let Some(url) = initial_navigation {
            let target = local.get_slot::<V8BindingState>().ok_or_else(|| {
                HostError::Execution("auxiliary navigation target missing".into())
            })?;
            let target_creator = target
                .native_worker_creator()
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let response = source
                .client
                .request(ContextServiceRequest::IssueScriptNavigationAuthority {
                    page_id: source.page_id,
                    target_page_id: Some(page_id),
                    creator: creator_state
                        .native_worker_creator()
                        .map_err(|error| HostError::Execution(error.to_string()))?,
                    target: target_creator.clone(),
                    url: url.into(),
                    source_url: creator_state
                        .request_initiator_url()
                        .map_err(|error| HostError::Execution(error.to_string()))?,
                    referrer_policy: creator_state.document_referrer_policy(),
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let ContextServiceResponse::ScriptNavigationAuthority { authority } = response else {
                return Err(HostError::Execution(
                    "auxiliary initial navigation denied".into(),
                ));
            };
            // Initial about:blank hides Navigation API entries/events, but the
            // actor still requires the native binding/generation/revision token.
            // Admit in the child's realm while retaining the opener as source.
            let opener = creator.global(scope);
            let navigation_source = oryn_js::navigation_api::NativeNavigationSource {
                source_window: Some(v8::Global::new(scope, opener)),
                ..Default::default()
            };
            let admission = {
                let scope = &mut v8::ContextScope::new(scope, local);
                oryn_js::navigation_api::admit_native_navigation(
                    scope,
                    url,
                    true,
                    false,
                    navigation_source,
                )
                .map_err(|error| HostError::Execution(error.to_string()))?
            };
            let oryn_js::navigation_api::NativeNavigationAdmission::Proceed {
                operation: Some(operation),
                destination,
            } = admission
            else {
                return Err(HostError::Execution(
                    "auxiliary initial navigation admission canceled".into(),
                ));
            };
            target.schedule_navigation(oryn_js::v8_bindings::LocationNavigation {
                execution_observer: None,
                navigation_operation: Some(operation),
                navigation_destination: destination.map(|entry| *entry),
                script_authority: Some(oryn_js::navigation::ScriptNavigationAdmission {
                    authority,
                    target: target_creator,
                }),
                form_authority: None,
                url: url.into(),
                replace: true,
                reload: false,
                post: None,
                referrer_policy: None,
                download: None,
            });
        }
        Ok(())
    });
    if let Err(error) = result {
        fail_runtime_bootstrap(local);
        if let Some(state) = local.get_slot::<V8BindingState>() {
            state.retire_execution();
        }
        if let Some(environment) = local.get_slot::<EnvironmentDescriptor>() {
            retire_environment(scope, environment.id);
        }
        let identities = scope.get_slot_mut::<NativeRealmIdentities>().unwrap();
        identities.pending_contexts.remove(&realm);
        identities.contexts.remove(&realm);
        identities.realms.remove(realm);
        return Err(error);
    }
    let registry = scope.get_slot_mut::<AuxiliaryRoots>().unwrap();
    registry.pages.insert(page_id, realm);
    registry
        .actors
        .insert(admission.route.clone(), actor_route.clone());
    registry.pending.push(CreatedAuxiliaryRoot {
        page_id,
        opener_page_id,
        route: admission.route,
        actor_route,
        realm,
        dom,
        document,
        context,
        broker,
        url: Url::parse("about:blank").unwrap(),
        policy,
        referrer_policy: crate::referrer::ReferrerPolicy::parse_list(&referrer),
        isolation: admission.isolation,
    });
    rollback.armed = false;
    Ok(())
}

pub(super) fn local_auxiliary_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    page: PageId,
) -> Option<v8::Local<'s, v8::Object>> {
    let realm = *scope.get_slot::<AuxiliaryRoots>()?.pages.get(&page)?;
    let weak = scope
        .get_slot::<NativeRealmIdentities>()?
        .contexts
        .get(&realm)?
        .clone();
    let context = weak.to_local(scope)?;
    if context.get_slot::<V8BindingState>()?.execution_retired() {
        return None;
    }
    Some(context.global(scope))
}

impl RawV8Host {
    pub(crate) fn navigation_root_for_page(&self, page: PageId) -> Option<RealmId> {
        self.navigation_roots.get(&page).copied().or_else(|| {
            self.isolate
                .get_slot::<RealmEndpointRegistry>()?
                .0
                .iter()
                .find_map(|(realm, (_, endpoints))| {
                    (endpoints
                        .context
                        .as_ref()
                        .is_some_and(|endpoint| endpoint.page_id == page)
                        && !self.realm_is_parked(*realm)
                        && self
                            .realm_descriptor(*realm)
                            .is_some_and(|entry| entry.frame_id.is_none()))
                    .then_some(*realm)
                })
        })
    }
    pub(crate) fn publish_current_auxiliary_root(&mut self, page: PageId, realm: RealmId) {
        if self.isolate.get_slot::<AuxiliaryRoots>().is_none() {
            self.isolate.set_slot(AuxiliaryRoots::default());
        }
        self.isolate
            .get_slot_mut::<AuxiliaryRoots>()
            .unwrap()
            .pages
            .insert(page, realm);
    }

    pub(crate) fn take_created_auxiliary_roots(&mut self) -> Vec<CreatedAuxiliaryRoot> {
        self.adopt_created_realms();
        self.isolate
            .get_slot_mut::<AuxiliaryRoots>()
            .map(|registry| std::mem::take(&mut registry.pending))
            .unwrap_or_default()
    }
    pub(crate) fn discard_created_auxiliary_root(
        &mut self,
        realm: RealmId,
    ) -> Result<(), HostError> {
        if let Some(endpoint) = self.context_endpoint_for_realm(realm) {
            match endpoint
                .client
                .request(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: endpoint.page_id,
                    expected_generation: self
                        .binding_states
                        .get(&realm)
                        .map(|state| state.document_generation()),
                }) {
                Ok(ContextServiceResponse::PageClosed) => {}
                Ok(_) => {
                    return Err(HostError::Execution(
                        "auxiliary rollback response mismatch".into(),
                    ));
                }
                Err(error) => return Err(HostError::Execution(error.to_string())),
            }
        }
        self.retire_agent_root(realm)
    }
    /// Revoke the exact failed forest without detaching its WindowProxy. The
    /// caller retains the native-only journal through ACK loss and host loans.
    pub(crate) fn revoke_failed_agent_root_authority(
        &mut self,
        realm: RealmId,
        pending: &mut Option<(
            SharedContextClient,
            PageId,
            std::collections::VecDeque<oryn_js::message_wire::MessageRequest>,
        )>,
    ) -> Result<(), HostError> {
        use oryn_js::message_wire::{MessageRequest as Q, MessageResponse as R};
        if pending.is_none() {
            self.adopt_created_realms();
            let root = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("failed root context missing".into()))?;
            let contexts = self.contexts.values().cloned().collect::<Vec<_>>();
            let (endpoint, requests, environments) = {
                v8::scope!(let scope,&mut self.isolate);
                let root = v8::Local::new(scope, &root);
                let descriptor = root
                    .get_slot::<RealmDescriptor>()
                    .ok_or_else(|| HostError::Execution("failed root descriptor missing".into()))?;
                if descriptor.id != realm || descriptor.frame_id.is_some() {
                    return Err(HostError::Execution("failed root identity mismatch".into()));
                }
                let environment = *root.get_slot::<EnvironmentDescriptor>().ok_or_else(|| {
                    HostError::Execution("failed root environment missing".into())
                })?;
                // Cleanup may replay after local quarantine. Read only this
                // retained exact environment's route, never an active fallback.
                let endpoint = scope
                    .get_slot::<RealmEndpointRegistry>()
                    .and_then(|registry| registry.0.get(&realm))
                    .filter(|(owned, _)| *owned == environment)
                    .and_then(|(_, endpoints)| endpoints.context.clone())
                    .ok_or_else(|| HostError::Execution("failed root endpoint missing".into()))?;
                let mut requests = std::collections::VecDeque::new();
                let mut environments = vec![environment];
                for context in &contexts {
                    let context = v8::Local::new(scope, context);
                    let Some(frame) = context.get_slot::<FrameLifetime>() else {
                        continue;
                    };
                    if frame.root_realm != realm {
                        continue;
                    }
                    environments.push(*context.get_slot::<EnvironmentDescriptor>().ok_or_else(
                        || HostError::Execution("failed child environment missing".into()),
                    )?);
                    // Closed frames may still await a failed native close ACK.
                    // Keep their exact old tuple; never substitute a live tree.
                    if let Some(state) = context.get_slot::<V8BindingState>() {
                        requests.push_back(Q::RetireWindow {
                            frame: frame.message_token.clone(),
                            document_generation: state.document_generation(),
                            stream_epoch: state.document_authority_epoch(),
                        });
                    }
                }
                // Partial installation may have populated the context slot but
                // not the host map. No slot means no native registration yet.
                if let Some(state) = root.get_slot::<V8BindingState>() {
                    requests.push_back(Q::RetireRootDocument {
                        document_generation: state.document_generation(),
                        stream_epoch: state.document_authority_epoch(),
                    });
                }
                (endpoint, requests, environments)
            };
            // A sibling can borrow this host during an ACK retry. Quarantine
            // native admission before any I/O so it cannot add descendants to
            // the fixed journal through a retained failed Document reference.
            let registry = self
                .isolate
                .get_slot_mut::<AgentEnvironments>()
                .ok_or_else(|| {
                    HostError::Execution("failed root environment registry missing".into())
                })?;
            for environment in environments {
                registry.quarantine(environment.id);
            }
            *pending = Some((endpoint.client, endpoint.page_id, requests));
        }
        let (client, page, requests) = pending.as_mut().unwrap();
        while let Some(request) = requests.front() {
            match client.request(ContextServiceRequest::MessageWire {
                source_page_id: *page,
                request: request.clone(),
            }) {
                Ok(ContextServiceResponse::MessageWire {
                    result: Ok(R::Done),
                }) => {
                    requests.pop_front();
                }
                Ok(ContextServiceResponse::MessageWire { result: Err(error) }) => {
                    return Err(HostError::Execution(error));
                }
                Ok(_) => {
                    return Err(HostError::Execution(
                        "failed root retirement response mismatch".into(),
                    ));
                }
                Err(error) => return Err(HostError::Execution(error.to_string())),
            }
        }
        Ok(())
    }

    pub(crate) fn retire_agent_root(&mut self, realm: RealmId) -> Result<(), HostError> {
        self.adopt_created_realms();
        self.discard_cached_authority(realm)?;
        let environment = self
            .isolate
            .get_slot::<RealmEndpointRegistry>()
            .and_then(|registry| registry.0.get(&realm))
            .map(|entry| entry.0);
        let mut environments = environment
            .map(|root| self.document_environments(root))
            .unwrap_or_default();
        // A close callback or native ACK can fail after the frame was marked
        // closed. Its retained context still belongs to this exact root.
        for context in self.contexts.values() {
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, context);
            if context
                .get_slot::<FrameLifetime>()
                .is_some_and(|frame| frame.root_realm == realm)
                && let Some(environment) = context.get_slot::<EnvironmentDescriptor>()
                && !environments.contains(&*environment)
            {
                environments.push(*environment);
            }
        }
        for environment in environments.iter().rev() {
            if let Some(context) = self.contexts.get(&environment.realm).cloned() {
                v8::scope!(let scope,&mut self.isolate);
                let context = v8::Local::new(scope, &context);
                if let Some(window) = context.get_slot::<NativeWindowLifecycle>() {
                    window.closed.set(true);
                }
            }
            retire_environment(&mut self.isolate, environment.id);
            if let Some(state) = self.binding_states.remove(&environment.realm) {
                state.retire_execution();
            }
            self.contexts.remove(&environment.realm);
            let identities = self
                .isolate
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap();
            identities.pending_contexts.remove(&environment.realm);
            identities.contexts.remove(&environment.realm);
            identities.realms.remove(environment.realm);
        }
        let retired_realms = environments
            .iter()
            .map(|environment| environment.realm)
            .collect::<BTreeSet<_>>();
        let retired_environments = environments
            .iter()
            .map(|environment| environment.id)
            .collect::<BTreeSet<_>>();
        self.script_isolation
            .retain(|realm, _| !retired_realms.contains(realm));
        if let Some(frames) = self.isolate.get_slot_mut::<FrameLifetimes>() {
            frames.live.retain(|_, frame| {
                let keep = !retired_realms.contains(&frame.root_realm)
                    && !retired_realms.contains(&frame.environment.get().realm);
                if !keep {
                    frame.closed.set(true);
                }
                keep
            });
            frames
                .creating
                .retain(|(parent, _)| !retired_environments.contains(parent));
            frames.discarded_owners.retain(|retired, owner| {
                !retired_realms.contains(retired) && !retired_realms.contains(owner)
            });
            frames
                .discarded_realms
                .retain(|retired| frames.discarded_owners.contains_key(retired));
        }
        self.navigation_roots.retain(|_, current| *current != realm);
        if let Some(registry) = self.isolate.get_slot_mut::<AuxiliaryRoots>() {
            registry.pages.retain(|_, value| *value != realm);
            registry.pending.retain(|root| root.realm != realm);
        }
        if !self.contexts.contains_key(&self.active_realm)
            && let Some(realm) = self.contexts.keys().next().copied()
        {
            self.active_realm = realm;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target_inventory(
        host: &mut RawV8Host,
    ) -> BTreeMap<PageId, Vec<(String, webdriver_script::AutomationRealmSnapshot)>> {
        let context = host.contexts[&host.active_realm].clone();
        v8::scope!(let scope,&mut host.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        local_navigation_inventory(scope).unwrap()
    }
    #[test]
    fn failed_root_retirement_keeps_closed_descendant_custody_until_physical_cleanup() {
        use oryn_js::message_wire::{MessageRequest as Q, MessageResponse as R, WindowTarget};
        let owner = crate::Browser::in_process_with_config(Default::default()).new_context();
        let page = owner.new_page();
        let url = "https://closed-failed-root.test/start";
        owner.commit_page_url(page.page_id(), url.into()).unwrap();
        owner
            .install_document_security(page.page_id(), url.into(), &[])
            .unwrap();
        struct LoseFrameAck {
            inner: SharedContextClient,
            fail: std::sync::atomic::AtomicBool,
        }
        impl crate::context_service::ContextTransport for LoseFrameAck {
            fn request_routed(
                &self,
                _: Option<&crate::worker_route::WorkerRoute>,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                self.request(request)
            }
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let frame = matches!(
                    &request,
                    ContextServiceRequest::MessageWire {
                        request: Q::RetireWindow { .. },
                        ..
                    }
                );
                let response = self.inner.request(request)?;
                if frame
                    && matches!(
                        &response,
                        ContextServiceResponse::MessageWire {
                            result: Ok(R::Done)
                        }
                    )
                    && self.fail.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(crate::RuntimeError::ContextService(
                        "injected child retirement ACK loss".into(),
                    ));
                }
                Ok(response)
            }
        }
        let transport = Arc::new(LoseFrameAck {
            inner: SharedContextClient::direct(owner, page.page_id()),
            fail: std::sync::atomic::AtomicBool::new(true),
        });
        let client = SharedContextClient::for_test_transport(
            transport,
            crate::worker_route::WorkerRoute::issue(781, page.page_id().0, 0).unwrap(),
        );
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(Url::parse(url).unwrap()),
            client.clone(),
            page.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let root = host.active_realm();
        let generation = host.realm_descriptor(root).unwrap().document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        host.execute("const frame=document.createElement('iframe');document.body.append(frame);void frame.contentWindow.document").unwrap();
        host.adopt_created_realms();
        let (child, frame) = {
            let contexts = host
                .contexts
                .iter()
                .map(|(realm, context)| (*realm, context.clone()))
                .collect::<Vec<_>>();
            v8::scope!(let scope,&mut host.isolate);
            contexts
                .into_iter()
                .find_map(|(realm, context)| {
                    v8::Local::new(scope, &context)
                        .get_slot::<FrameLifetime>()
                        .filter(|frame| frame.root_realm == root)
                        .map(|frame| (realm, frame))
                })
                .unwrap()
        };
        let status = || {
            client
                .request(ContextServiceRequest::MessageWire {
                    source_page_id: page.page_id(),
                    request: Q::WindowStatus {
                        page: page.page_id().0,
                        target: WindowTarget::Frame(frame.message_token.clone()),
                    },
                })
                .unwrap()
        };
        assert!(matches!(
            status(),
            ContextServiceResponse::MessageWire {
                result: Ok(R::WindowLive(true))
            }
        ));
        // This is the retained state after close marked the local lifetime but
        // its native retirement suffix failed. No authority is fabricated.
        frame.closed.set(true);
        let sibling = host
            .create_realm("https://closed-failed-root.test", 90, None)
            .unwrap();
        host.activate_realm(sibling).unwrap();
        host.execute("globalThis.siblingKept=81").unwrap();
        {
            let original = host.contexts[&root].clone();
            let other = host.contexts[&sibling].clone();
            v8::scope!(let handles,&mut host.isolate);
            let original = v8::Local::new(handles, &original);
            let scope = &mut v8::ContextScope::new(handles, original);
            let key = v8::String::new(scope, "document").unwrap();
            let document = original.global(scope).get(scope, key.into()).unwrap();
            let other = v8::Local::new(scope, &other);
            let scope = &mut v8::ContextScope::new(scope, other);
            let key = v8::String::new(scope, "failedDoc").unwrap();
            assert_eq!(
                other.global(scope).set(scope, key.into(), document),
                Some(true)
            );
        }
        assert_eq!(
            host.evaluate_string("String(failedDoc.body.tagName==='BODY')")
                .unwrap(),
            "true"
        );
        let mut pending = None;
        assert!(
            host.revoke_failed_agent_root_authority(root, &mut pending)
                .is_err()
        );
        assert!(matches!(
            pending.as_ref().unwrap().2.front(),
            Some(Q::RetireWindow { .. })
        ));
        let count = host.contexts.len();
        // The same-origin sibling keeps the actual failed Document while the
        // child ACK is pending and the original root remains parent-registered.
        host.execute("try{const added=failedDoc.createElement('iframe');failedDoc.body.append(added);void added.contentWindow}catch(error){};globalThis.siblingKept++").unwrap();
        host.adopt_created_realms();
        assert_eq!(
            host.contexts.len(),
            count,
            "quarantined failed document cannot admit another native child"
        );
        host.revoke_failed_agent_root_authority(root, &mut pending)
            .unwrap();
        assert!(pending.as_ref().unwrap().2.is_empty());
        assert!(matches!(
            status(),
            ContextServiceResponse::MessageWire {
                result: Ok(R::WindowLive(false))
            }
        ));
        assert!(host.realm_descriptor(root).is_some() && host.realm_descriptor(child).is_some());
        host.retire_agent_root(root).unwrap();
        assert!(host.realm_descriptor(root).is_none() && host.realm_descriptor(child).is_none());
        assert!(!host.contexts.contains_key(&child) && !host.binding_states.contains_key(&child));
        assert!(host.realm_descriptor(sibling).is_some());
        host.activate_realm(sibling).unwrap();
        assert_eq!(host.evaluate_string("String(siblingKept)").unwrap(), "82");
    }

    #[test]
    fn initial_auxiliary_navigation_is_root_local_and_seals_opener_source() {
        for fail_authorization in [false, true] {
            let browser = crate::Browser::in_process_with_config(Default::default());
            let owner = browser.new_context();
            let first = owner.new_page();
            let url = "https://source.example.test/path/start?query=1";
            owner.commit_page_url(first.page_id(), url.into()).unwrap();
            owner
                .install_document_security(first.page_id(), url.into(), &[])
                .unwrap();
            let first_route =
                crate::worker_route::WorkerRoute::issue(48, first.page_id().0, 0).unwrap();
            let transport = Arc::new(Routes(std::sync::Mutex::new(BTreeMap::from([(
                first_route.clone(),
                SharedContextClient::direct(owner.clone(), first.page_id()),
            )]))));
            let endpoint =
                SharedContextClient::for_test_transport(transport.clone(), first_route.clone());
            let mut host = RawV8Host::new_with_network_context_and_heap_limit(
                None,
                Some(Url::parse(url).unwrap()),
                endpoint,
                first.page_id(),
                256 * 1024 * 1024,
            )
            .unwrap();
            let parent = host.active_realm();
            let generation = host.realm_descriptor(parent).unwrap().document_generation;
            host.install_dom(
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>parent</body>")
                        .into_runtime()
                        .unwrap(),
                )),
                generation,
            )
            .unwrap();
            host.binding_states[&parent]
                .set_document_referrer_policy("origin")
                .unwrap();
            let creator = host.binding_states[&parent]
                .native_worker_creator()
                .unwrap();
            let second = owner
                .new_popup_from_creator(first.page_id(), &creator)
                .unwrap();
            let second_route =
                crate::worker_route::WorkerRoute::issue(48, second.page_id().0, 0).unwrap();
            transport.0.lock().unwrap().insert(
                second_route.clone(),
                SharedContextClient::direct(owner.clone(), second.page_id()),
            );
            let admission = crate::context_service::RelatedRootAdmission {
                direct: false,
                route: second_route,
                actor_route: first_route,
                policy: Some(crate::csp::CspPolicySet::new("about:blank")),
                isolation: Default::default(),
            };
            let destination = if fail_authorization {
                "not a URL"
            } else {
                "https://destination.example.test/ready"
            };
            let parent_context = host.contexts[&parent].clone();
            let result = {
                v8::scope!(let handles,&mut host.isolate);
                let context = v8::Local::new(handles, &parent_context);
                let scope = &mut v8::ContextScope::new(handles, context);
                create_admitted_auxiliary_root(
                    scope,
                    second.page_id(),
                    Some(first.page_id()),
                    admission,
                    Some(destination),
                )
            };
            if fail_authorization {
                assert!(result.is_err());
                assert!(host.take_created_auxiliary_roots().is_empty());
                assert!(
                    owner
                        .native_initial_document_creator(second.page_id())
                        .is_err(),
                    "failed creation must revoke its parent-owned target"
                );
                assert_eq!(
                    host.evaluate_string("document.body.textContent").unwrap(),
                    "parent"
                );
                continue;
            }
            result.unwrap();
            let roots = host.take_created_auxiliary_roots();
            assert_eq!(roots.len(), 1);
            let child = roots[0].realm;
            assert!(
                host.take_location_navigation().is_none(),
                "opener cannot consume child navigation"
            );
            host.activate_realm(child).unwrap();
            assert_eq!(
                host.evaluate_string("location.href").unwrap(),
                "about:blank"
            );
            let navigation = host
                .take_location_navigation()
                .expect("accepted initial navigation belongs to child turn");
            assert_eq!(navigation.url, destination);
            assert!(navigation.replace);
            let operation = navigation
                .navigation_operation
                .as_ref()
                .expect("initial documents still need an actor navigation identity");
            assert!(host.native_navigation_matches(operation));
            assert!(
                navigation.navigation_destination.is_none(),
                "initial about:blank must not expose a Navigation API entry"
            );
            let admission = navigation.script_authority.unwrap();
            let source = owner
                .script_navigation_source(
                    &admission.authority,
                    second.page_id(),
                    &admission.target,
                    destination,
                )
                .unwrap();
            assert_eq!(source.url, url);
            assert_eq!(source.top_level_url, url);
            assert_eq!(source.referrer_policy, "origin");
            assert!(
                owner
                    .script_navigation_source(
                        &admission.authority,
                        first.page_id(),
                        &creator,
                        destination
                    )
                    .is_err()
            );
            assert!(
                owner
                    .script_navigation_source(
                        &admission.authority,
                        second.page_id(),
                        &admission.target,
                        "https://destination.example.test/other"
                    )
                    .is_err()
            );
            let queue = owner.queued_navigation_status(second.page_id()).unwrap();
            assert_eq!(queue.active, 0);
            assert!(queue.outcomes.is_empty());
            assert!(host.abort_native_navigation(operation).unwrap());
            assert!(
                !host.native_navigation_matches(operation),
                "explicit cancellation must retire the initial native revision"
            );
            assert!(
                !host.abort_native_navigation(operation).unwrap(),
                "a canceled initial navigation identity must not be reused"
            );
            host.activate_realm(parent).unwrap();
            assert_eq!(
                host.evaluate_string("document.body.textContent").unwrap(),
                "parent"
            );
        }
    }

    struct Routes(
        std::sync::Mutex<BTreeMap<crate::worker_route::WorkerRoute, SharedContextClient>>,
    );
    impl crate::context_service::ContextTransport for Routes {
        fn request(
            &self,
            _: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, crate::RuntimeError> {
            Err(crate::RuntimeError::ProtocolViolation(
                "test requires exact route".into(),
            ))
        }
        fn request_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            request: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, crate::RuntimeError> {
            let client = self
                .0
                .lock()
                .unwrap()
                .get(route.ok_or_else(|| {
                    crate::RuntimeError::ProtocolViolation("missing route".into())
                })?)
                .cloned()
                .ok_or_else(|| crate::RuntimeError::ProtocolViolation("unknown grant".into()))?;
            client.request(request)
        }
    }
    #[test]
    fn native_container_attribute_updates_reach_parent_policy_before_domain_setter() {
        let browser = crate::Browser::in_process_with_config(Default::default());
        let owner = browser.new_context();
        let page = owner.new_page();
        let url = "https://source.example.test/start";
        owner.commit_page_url(page.page_id(), url.into()).unwrap();
        owner
            .install_document_security(page.page_id(), url.into(), &[])
            .unwrap();
        let client = SharedContextClient::direct(owner.clone(), page.page_id());
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(Url::parse(url).unwrap()),
            client,
            page.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let generation = host
            .realm_descriptor(host.active_realm())
            .unwrap()
            .document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        assert_eq!(host.evaluate_string(r#"(()=>{
            const frame=document.createElement('iframe');document.body.append(frame);const child=frame.contentWindow,original=child.document;
            frame.allow="document-domain 'none'";let denied=false;try{child.document.domain='example.test'}catch(e){denied=e.name==='SecurityError'}
            frame.allow='document-domain *';child.document.domain='example.test';return String(denied&&child.document===original&&child.document.domain==='example.test');
        })()"#).unwrap(),"true");
    }
    #[test]
    fn native_document_cluster_is_admitted_before_first_authored_api_use() {
        for origin_keyed in [false, true] {
            let browser = crate::Browser::in_process_with_config(Default::default());
            let owner = browser.new_context();
            let page = owner.new_page();
            let url = "https://source.example.test/start";
            owner.commit_page_url(page.page_id(), url.into()).unwrap();
            let headers = if origin_keyed {
                vec![("Origin-Agent-Cluster".into(), "?1".into())]
            } else {
                Vec::new()
            };
            owner
                .install_document_security(page.page_id(), url.into(), &headers)
                .unwrap();
            let client = SharedContextClient::direct(owner.clone(), page.page_id());
            let mut host = RawV8Host::new_with_network_context_and_heap_limit(
                None,
                Some(Url::parse(url).unwrap()),
                client,
                page.page_id(),
                256 * 1024 * 1024,
            )
            .unwrap();
            let generation = host
                .realm_descriptor(host.active_realm())
                .unwrap()
                .document_generation;
            host.install_dom(
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>cluster</body>")
                        .into_runtime()
                        .unwrap(),
                )),
                generation,
            )
            .unwrap();
            assert_eq!(
                host.evaluate_string("String(originAgentCluster)").unwrap(),
                origin_keyed.to_string()
            );
            if origin_keyed {
                host.execute("document.domain='not a valid host'").unwrap();
                assert_eq!(
                    host.evaluate_string("document.domain").unwrap(),
                    "source.example.test"
                );
            } else {
                host.execute("document.domain='example.test'").unwrap();
                assert_eq!(
                    host.evaluate_string("document.domain").unwrap(),
                    "example.test"
                );
            }
            let state = &host.binding_states[&host.active_realm()];
            assert_eq!(
                state
                    .document_provenance(state.document_root())
                    .unwrap()
                    .unwrap()
                    .origin,
                "https://source.example.test"
            );
        }
    }
    #[test]
    fn parked_document_refreshes_domain_changed_by_live_inherited_popup() {
        struct MutableRoutes(
            std::sync::Mutex<BTreeMap<crate::worker_route::WorkerRoute, SharedContextClient>>,
        );
        impl crate::context_service::ContextTransport for MutableRoutes {
            fn request(
                &self,
                _: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                Err(crate::RuntimeError::ProtocolViolation(
                    "test requires route".into(),
                ))
            }
            fn request_routed(
                &self,
                route: Option<&crate::worker_route::WorkerRoute>,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let client = self
                    .0
                    .lock()
                    .unwrap()
                    .get(route.ok_or_else(|| {
                        crate::RuntimeError::ProtocolViolation("missing route".into())
                    })?)
                    .cloned()
                    .ok_or_else(|| {
                        crate::RuntimeError::ProtocolViolation("unknown route".into())
                    })?;
                client.request(request)
            }
        }
        let browser = crate::Browser::in_process_with_config(Default::default());
        let owner = browser.new_context();
        let first = owner.new_page();
        let url = "https://a.source.example.test/start";
        owner.commit_page_url(first.page_id(), url.into()).unwrap();
        owner
            .install_document_security(first.page_id(), url.into(), &[])
            .unwrap();
        let first_route =
            crate::worker_route::WorkerRoute::issue(42, first.page_id().0, 0).unwrap();
        // Create the initial popup after the parent cluster has been published,
        // so its inherited domain identity is the parent's live alias group.
        let transport = Arc::new(MutableRoutes(std::sync::Mutex::new(BTreeMap::from([(
            first_route.clone(),
            SharedContextClient::direct(owner.clone(), first.page_id()),
        )]))));
        let client =
            SharedContextClient::for_test_transport(transport.clone(), first_route.clone());
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(Url::parse(url).unwrap()),
            client,
            first.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let parent = host.active_realm();
        let generation = host.realm_descriptor(parent).unwrap().document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>parent</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        let creator = host.binding_states[&parent]
            .native_worker_creator()
            .unwrap();
        let second = owner
            .new_popup_from_creator(first.page_id(), &creator)
            .unwrap();
        let second_route =
            crate::worker_route::WorkerRoute::issue(42, second.page_id().0, 0).unwrap();
        transport.0.lock().unwrap().insert(
            second_route.clone(),
            SharedContextClient::direct(owner.clone(), second.page_id()),
        );
        // Both routed endpoints execute in this actor; no network response is
        // needed to mutate the inherited popup's shared domain identity.
        host.admit_navigation_auxiliary_root(
            second.page_id(),
            Some(first.page_id()),
            crate::context_service::RelatedRootAdmission {
                direct: false,
                route: second_route,
                actor_route: first_route,
                policy: Some(crate::csp::CspPolicySet::new("about:blank")),
                isolation: Default::default(),
            },
        )
        .unwrap();
        let child = host.take_created_auxiliary_roots().pop().unwrap().realm;
        host.execute("globalThis.inertBeforeRelaxation=new Document();globalThis.domainAfterResume='pending';globalThis.resumeDomain=undefined;new Promise(resolve=>resumeDomain=resolve).then(()=>domainAfterResume=document.domain)").unwrap();
        host.activate_realm(child).unwrap();
        host.execute("globalThis.savedInert=opener.inertBeforeRelaxation;globalThis.savedResume=opener.resumeDomain").unwrap();
        host.park_document_authority(parent).unwrap();
        host.activate_realm(child).unwrap();
        host.execute("document.domain='source.example.test';savedResume()")
            .unwrap();
        assert_eq!(
            host.evaluate_string("document.domain").unwrap(),
            "source.example.test"
        );
        host.restore_document_authority(parent).unwrap();
        host.run_microtask_checkpoint().unwrap();
        assert_eq!(
            host.evaluate_string("opener.domainAfterResume+':'+opener.document.domain")
                .unwrap(),
            "source.example.test:source.example.test"
        );
        let state = &host.binding_states[&parent];
        assert_eq!(
            state
                .document_provenance(state.document_root())
                .unwrap()
                .unwrap()
                .origin,
            "https://a.source.example.test"
        );
        assert_eq!(
            host.evaluate_string("savedInert.domain").unwrap(),
            "source.example.test"
        );
        host.retire_agent_root(parent).unwrap();
        host.execute("document.domain='example.test'").unwrap();
        assert_eq!(
            host.evaluate_string("savedInert.domain+':'+savedInert.cloneNode(false).domain")
                .unwrap(),
            "example.test:example.test"
        );
    }
    #[test]
    fn closed_popup_inventory_excludes_retained_descendants_without_admitting_another_group() {
        let browser = crate::Browser::in_process_with_config(Default::default());
        let owner = browser.new_context();
        let first = owner.new_page();
        let url = "https://source.example.test/start";
        owner.commit_page_url(first.page_id(), url.into()).unwrap();
        owner
            .install_document_security(first.page_id(), url.into(), &[])
            .unwrap();
        let first_route =
            crate::worker_route::WorkerRoute::issue(43, first.page_id().0, 0).unwrap();
        let transport = Arc::new(Routes(std::sync::Mutex::new(BTreeMap::from([(
            first_route.clone(),
            SharedContextClient::direct(owner.clone(), first.page_id()),
        )]))));
        let client =
            SharedContextClient::for_test_transport(transport.clone(), first_route.clone());
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(Url::parse(url).unwrap()),
            client,
            first.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let parent = host.active_realm();
        let generation = host.realm_descriptor(parent).unwrap().document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>parent</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        let creator = host.binding_states[&parent]
            .native_worker_creator()
            .unwrap();
        let popup = owner
            .new_popup_from_creator(first.page_id(), &creator)
            .unwrap();
        let popup_route =
            crate::worker_route::WorkerRoute::issue(43, popup.page_id().0, 0).unwrap();
        transport.0.lock().unwrap().insert(
            popup_route.clone(),
            SharedContextClient::direct(owner.clone(), popup.page_id()),
        );
        host.admit_navigation_auxiliary_root(
            popup.page_id(),
            Some(first.page_id()),
            crate::context_service::RelatedRootAdmission {
                direct: false,
                route: popup_route,
                actor_route: first_route,
                policy: Some(crate::csp::CspPolicySet::new("about:blank")),
                isolation: Default::default(),
            },
        )
        .unwrap();
        let child = host.take_created_auxiliary_roots().pop().unwrap();
        host.activate_realm(child.realm).unwrap();
        host.execute("globalThis.frame=document.createElement('iframe');frame.name='closed-descendant';document.body.append(frame);void frame.contentWindow.document").unwrap();
        host.adopt_created_realms();
        host.activate_realm(parent).unwrap();
        let before = target_inventory(&mut host);
        assert_eq!(before[&popup.page_id()].len(), 2);
        let descendant = before[&popup.page_id()]
            .iter()
            .find(|(name, entry)| {
                name == "closed-descendant" && entry.parent_realm == Some(child.realm)
            })
            .unwrap()
            .1
            .realm;
        host.publish_native_window_inventory().unwrap();
        // A different actor can rely on the parent's saved forest instead of
        // supplying the popup in its own local inventory.
        let parent_only = BTreeMap::from([(first.page_id(), before[&first.page_id()].clone())]);
        let saved = owner
            .read_related_window_with_inventory(
                first.page_id(),
                &creator,
                popup.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                None,
                &parent_only,
            )
            .unwrap();
        assert_eq!(saved.children.len(), 1);
        assert_eq!(saved.children[0].0, "closed-descendant");
        host.execute(&format!("globalThis.popup=__oryn_native_page_proxy({});popup.close();globalThis.closedSynchronously=popup.closed",popup.page_id().0)).unwrap();
        assert_eq!(
            host.evaluate_string("String(closedSynchronously)").unwrap(),
            "true"
        );
        assert!(
            !host.binding_states[&child.realm].execution_retired()
                && !host.binding_states[&descendant].execution_retired(),
            "logical close precedes actor cleanup"
        );
        let closed = target_inventory(&mut host);
        assert!(
            closed[&popup.page_id()].is_empty(),
            "closed root and its retained descendant need a negative inventory entry"
        );
        assert_eq!(closed[&first.page_id()].len(), 1);
        host.publish_native_window_inventory().unwrap();
        assert!(
            owner
                .native_window_group(first.page_id(), popup.page_id(), None)
                .unwrap()
                .2,
            "logical closing precedes native page retirement"
        );
        assert!(
            owner
                .read_related_window_with_inventory(
                    first.page_id(),
                    &creator,
                    popup.page_id(),
                    &oryn_js::message_wire::WindowTarget::Root,
                    None,
                    &parent_only
                )
                .unwrap_err()
                .to_string()
                .contains("remote Window tree target is closing"),
            "saved inventories must not revive a logically closed forest"
        );
        assert_eq!(
            owner
                .read_related_window_with_inventory(
                    first.page_id(),
                    &creator,
                    first.page_id(),
                    &oryn_js::message_wire::WindowTarget::Root,
                    None,
                    &parent_only
                )
                .unwrap()
                .length,
            0
        );
        // Reintroducing the old nonempty closed forest still fails at the
        // parent, as does claiming an unrelated live page in this actor.
        assert!(
            owner
                .publish_native_window_inventory(first.page_id(), &creator, &before)
                .unwrap_err()
                .to_string()
                .contains("Window tree publication crosses group")
        );
        let unrelated = owner.new_page();
        let mut forged = closed.clone();
        forged.insert(unrelated.page_id(), closed[&first.page_id()].clone());
        assert!(
            owner
                .publish_native_window_inventory(first.page_id(), &creator, &forged)
                .unwrap_err()
                .to_string()
                .contains("Window tree publication crosses group")
        );
    }
    #[test]
    fn native_popup_preserves_synchronous_global_dom_and_root_retirement() {
        let browser = crate::Browser::in_process_with_config(Default::default());
        let owner = browser.new_context();
        let first = owner.new_page();
        let url = "https://source.example.test/start";
        owner.commit_page_url(first.page_id(), url.into()).unwrap();
        owner
            .install_document_security(first.page_id(), url.into(), &[])
            .unwrap();
        let first_route =
            crate::worker_route::WorkerRoute::issue(41, first.page_id().0, 0).unwrap();
        let transport = Arc::new(Routes(std::sync::Mutex::new(BTreeMap::from([(
            first_route.clone(),
            SharedContextClient::direct(owner.clone(), first.page_id()),
        )]))));
        let context =
            SharedContextClient::for_test_transport(transport.clone(), first_route.clone());
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(Url::parse(url).unwrap()),
            context,
            first.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let parent = host.active_realm();
        let generation = host.realm_descriptor(parent).unwrap().document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>parent</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        // Inherit the published source Document's origin, cluster and policy.
        // The generic popup allocator does not establish those creator edges.
        let creator = host.binding_states[&parent]
            .native_worker_creator()
            .unwrap();
        let second = owner
            .new_popup_from_creator(first.page_id(), &creator)
            .unwrap();
        let second_route =
            crate::worker_route::WorkerRoute::issue(41, second.page_id().0, 0).unwrap();
        transport.0.lock().unwrap().insert(
            second_route.clone(),
            SharedContextClient::direct(owner.clone(), second.page_id()),
        );
        // Exercise the typed link/form popup admission entry as well as the
        // shared root creation machinery used by window.open callbacks.
        host.admit_navigation_auxiliary_root(
            second.page_id(),
            Some(first.page_id()),
            crate::context_service::RelatedRootAdmission {
                direct: false,
                route: second_route.clone(),
                actor_route: first_route.clone(),
                policy: Some(crate::csp::CspPolicySet::new("about:blank")),
                isolation: Default::default(),
            },
        )
        .unwrap();
        let mut roots = host.take_created_auxiliary_roots();
        assert_eq!(roots.len(), 1);
        let child = roots.remove(0);
        assert_eq!(child.route, second_route);
        assert_eq!(child.actor_route, first_route);
        assert_eq!(
            host.evaluate_string(&format!(
                "globalThis.popup=__oryn_native_page_proxy({});popup.document.readyState",
                second.page_id().0
            ))
            .unwrap(),
            "complete"
        );
        assert_eq!(
            host.evaluate_string(
                "popup.opener===window && popup.parent===popup && popup.top===popup"
            )
            .unwrap(),
            "true"
        );
        host.execute("globalThis.navigationOrder=[];globalThis.link=document.createElement('a');link.href='https://destination.test/next';link.target='selected-popup';document.body.append(link);popup.name='selected-popup';popup.navigation.onnavigate=e=>{navigationOrder.push(e.sourceElement===link?'navigate':'wrong-source');e.preventDefault();};navigationOrder.push('before');link.click();navigationOrder.push('after')").unwrap();
        assert_eq!(
            host.evaluate_string("navigationOrder.join(',')").unwrap(),
            "before,after",
            "an initial about:blank document has Navigation entries and events disabled"
        );
        // The reservation publishes into the selected destination's queue.
        // A parent actor turn must not drain its auxiliary root's action.
        assert!(host.take_pending_default_actions().unwrap().is_empty());
        host.activate_realm(child.realm).unwrap();
        let actions = host.take_pending_default_actions().unwrap();
        assert_eq!(actions.len(), 1);
        let (_, oryn_js::v8_bindings::PendingDefaultAction::Click(click)) = &actions[0] else {
            panic!("link action missing")
        };
        let navigation = click.navigation.as_ref().unwrap();
        assert_eq!(navigation.admission.as_ref().unwrap().realm, child.realm);
        assert!(
            navigation
                .admission
                .as_ref()
                .unwrap()
                .script_authority
                .is_some(),
            "cross-page target must receive parent-issued script authority"
        );
        assert_eq!(
            host.resolve_submission_target(&navigation.navigation_target),
            Some(child.realm)
        );
        host.activate_realm(child.realm).unwrap();
        let admission = navigation.admission.as_ref().unwrap();
        host.schedule_navigation(oryn_js::v8_bindings::LocationNavigation {
            execution_observer: None,
            navigation_operation: admission.operation.clone(),
            navigation_destination: admission.destination.clone(),
            script_authority: admission.script_authority.clone(),
            form_authority: None,
            url: navigation.url.clone(),
            replace: false,
            reload: false,
            post: None,
            referrer_policy: navigation.referrer_policy.clone(),
            download: None,
        })
        .unwrap();
        assert!(host.has_pending_root_navigation());
        assert!(host.has_runnable_page_work());
        host.activate_realm(parent).unwrap();
        assert!(
            !host.has_pending_root_navigation(),
            "another root's accepted navigation belongs to that actor turn"
        );
        assert!(host.take_location_navigation().is_none());
        host.activate_realm(child.realm).unwrap();
        let scheduled = host.take_location_navigation().unwrap();
        assert_eq!(scheduled.navigation_operation, admission.operation);
        assert_eq!(scheduled.url, navigation.url);
        assert!(!host.has_pending_root_navigation());
        host.activate_realm(parent).unwrap();
        host.execute("popup.navigation.onnavigate=null").unwrap();
        host.execute(
            "popup.marker={shared:23};popup.document.body.textContent='synchronous child'",
        )
        .unwrap();
        host.activate_realm(child.realm).unwrap();
        assert_eq!(
            host.evaluate_string("marker.shared+':'+document.body.textContent")
                .unwrap(),
            "23:synchronous child"
        );
        host.execute("document.body.style.cssText='margin:0;width:200px;height:200px'")
            .unwrap();
        let child_state = host.binding_states[&child.realm].clone();
        let pointer = oryn_js::v8_bindings::NativePointerInput {
            x: 10.,
            y: 10.,
            ..Default::default()
        };
        host.dispatch_webdriver_pointer(
            child.realm,
            child_state.document_generation(),
            child_state.document_stream_epoch(),
            &pointer,
            oryn_js::v8_bindings::NativePointerAction::Move,
        )
        .unwrap();
        assert!(oryn_js::v8_bindings::native_pointer_has_hover(
            &child_state,
            pointer.id
        ));
        host.activate_realm(parent).unwrap();
        host.reset_webdriver_pointer_sources();
        assert!(oryn_js::v8_bindings::native_pointer_has_hover(
            &child_state,
            pointer.id
        ));
        host.activate_realm(child.realm).unwrap();
        host.reset_webdriver_pointer_sources();
        assert!(!oryn_js::v8_bindings::native_pointer_has_hover(
            &child_state,
            pointer.id
        ));
        host.activate_realm(parent).unwrap();
        host.execute("let frame=document.createElement('iframe');frame.name='parked-target';document.body.append(frame);void frame.contentWindow.document").unwrap();
        host.adopt_created_realms();
        let before_park = target_inventory(&mut host);
        let nested = before_park[&first.page_id()]
            .iter()
            .find(|(name, entry)| name == "parked-target" && entry.parent_realm == Some(parent))
            .unwrap()
            .1
            .clone();
        let root_target = oryn_js::v8_bindings::submissions::SubmissionTarget::Root {
            realm: parent,
            generation: host.binding_states[&parent].document_generation(),
        };
        let frame_target = oryn_js::v8_bindings::submissions::SubmissionTarget::Frame {
            parent,
            parent_generation: host.binding_states[&parent].document_generation(),
            frame: nested.frame.unwrap(),
            connection: nested.frame_generation.unwrap(),
        };
        assert_eq!(
            host.resolve_submission_target(&frame_target),
            Some(nested.realm)
        );
        host.execute("globalThis.deliveries=[];globalThis.finishLater=undefined;new Promise(resolve=>finishLater=resolve).then(()=>deliveries.push('restored'))").unwrap();
        host.activate_realm(child.realm).unwrap();
        host.execute("globalThis.savedOpener=opener;globalThis.savedFinish=opener.finishLater;globalThis.savedDeliveries=opener.deliveries").unwrap();
        host.park_document_authority(parent).unwrap();
        let parked_inventory = target_inventory(&mut host);
        assert!(
            parked_inventory[&first.page_id()].is_empty(),
            "parked actor page must retain a negative entry without exposing its forest"
        );
        assert!(
            !parked_inventory[&second.page_id()].is_empty(),
            "unrelated auxiliary root remains selectable"
        );
        assert!(host.realm_is_parked(parent) && host.realm_is_parked(nested.realm));
        assert!(
            host.binding_states[&parent].execution_suspended()
                && host.binding_states[&nested.realm].execution_suspended()
        );
        assert!(!host.binding_states[&child.realm].execution_suspended());
        assert!(host.resolve_submission_target(&root_target).is_none());
        assert!(host.resolve_submission_target(&frame_target).is_none());
        host.activate_realm(child.realm).unwrap();
        host.execute("savedFinish()").unwrap();
        assert_eq!(host.evaluate_string("savedDeliveries.length").unwrap(), "0");
        // A parent ACK must not expose this resolved promise to the shared
        // queue until native installation commits. Rollback freezes the whole
        // forest with a fresh token while retaining the original queue owner.
        let consumed = host.parked_documents[&parent].token.clone();
        host.restore_document_authority_suspended(parent).unwrap();
        assert!(host.parked_documents[&parent].parent_state == ParkedParentState::Restored);
        assert!(host.realm_is_parked(parent) && host.realm_is_parked(nested.realm));
        host.activate_realm(child.realm).unwrap();
        assert_eq!(host.evaluate_string("savedDeliveries.length").unwrap(), "0");
        host.repark_restored_document_authority(parent).unwrap();
        let replacement = host.parked_documents[&parent].token.clone();
        assert_ne!(consumed, replacement);
        host.repark_restored_document_authority(parent).unwrap();
        assert_eq!(
            host.parked_documents[&parent].token, replacement,
            "retry must not mint another token"
        );
        assert_eq!(host.evaluate_string("savedDeliveries.length").unwrap(), "0");
        // Model a native partial-restoration retry while preserving the queue
        // owner and withholding the shared checkpoint until parent restoration.
        let parked = host.parked_documents.remove(&parent).unwrap();
        host.resume_root_microtasks(&parked.suspended_environments, &parked.microtasks, false)
            .unwrap();
        host.resume_root_microtasks(&parked.suspended_environments, &parked.microtasks, false)
            .unwrap();
        assert!(
            target_inventory(&mut host)[&first.page_id()].is_empty(),
            "partial restore must not release native target admission"
        );
        assert!(
            host.binding_states[&parent].execution_suspended()
                && host.binding_states[&nested.realm].execution_suspended(),
            "partial microtask restore cannot release CE tasks"
        );
        host.parked_documents.insert(parent, parked);
        // Cached binding metadata must not override authenticated parent state
        // when a parked forest resumes, including its inherited descendants.
        for realm in [parent, nested.realm] {
            let state = &host.binding_states[&realm];
            state
                .dom()
                .borrow_mut()
                .arena_mut()
                .get_mut(state.document_root())
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .relaxed_domain
                .set(Some("stale.example".into()));
        }
        host.restore_document_authority(parent).unwrap();
        assert!(
            !host.binding_states[&parent].execution_suspended()
                && !host.binding_states[&nested.realm].execution_suspended()
        );
        for realm in [parent, nested.realm] {
            let state = &host.binding_states[&realm];
            assert_eq!(
                state
                    .dom()
                    .borrow()
                    .arena()
                    .get(state.document_root())
                    .unwrap()
                    .document_metadata
                    .as_ref()
                    .unwrap()
                    .relaxed_domain
                    .get(),
                None,
                "restore must refresh every document before queued authored work resumes"
            );
        }
        assert_eq!(host.resolve_submission_target(&root_target), Some(parent));
        assert_eq!(
            host.resolve_submission_target(&frame_target),
            Some(nested.realm)
        );
        assert!(
            target_inventory(&mut host)[&first.page_id()]
                .iter()
                .any(|(_, entry)| entry.same_navigable_as(&nested))
        );
        host.run_microtask_checkpoint().unwrap();
        assert_eq!(
            host.evaluate_string("savedDeliveries.join(',')").unwrap(),
            "restored"
        );
        host.park_document_authority(parent).unwrap();
        host.discard_cached_authority(parent).unwrap();
        assert!(
            host.binding_states[&parent].execution_retired()
                && host.binding_states[&nested.realm].execution_retired()
        );
        assert!(!host.parked_documents.contains_key(&parent));
        assert!(
            target_inventory(&mut host)[&first.page_id()].is_empty(),
            "discard cannot revive a cached Document before execution retirement"
        );
        assert!(host.resolve_submission_target(&frame_target).is_none());
        host.retire_agent_root(parent).unwrap();
        assert!(!host.contexts.contains_key(&parent) && !host.binding_states.contains_key(&parent));
        assert!(
            !host.contexts.contains_key(&nested.realm)
                && !host.binding_states.contains_key(&nested.realm)
        );
        assert!(
            host.contexts.contains_key(&child.realm),
            "retiring one root preserves its live auxiliary root"
        );
        assert!(!host.realm_is_parked(parent));
        // Test the retained native Window, not a fresh parent-authorized
        // opener lookup: the browser page itself has not been closed here.
        assert_eq!(
            host.evaluate_string("savedOpener.closed+':'+marker.shared")
                .unwrap(),
            "true:23"
        );
        assert!(host.take_created_auxiliary_roots().is_empty());
    }
}
