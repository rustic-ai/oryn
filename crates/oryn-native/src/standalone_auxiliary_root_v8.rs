// Native standalone auxiliary roots. Realm/group/opener identity is never
// inferred from author properties and carries no synthetic context-service ID.
#[derive(Clone)]
struct StandaloneAuxiliary {
    script_created: bool,
    group: RealmId,
    opener: Option<RealmId>,
    creator: RealmId,
    window: v8::Global<v8::Object>,
}
#[derive(Default)]
struct StandaloneAuxiliaryRoots {
    roots: BTreeMap<RealmId, StandaloneAuxiliary>,
    pending: Vec<RealmId>,
    current: BTreeMap<RealmId, RealmId>,
    lineages: BTreeMap<RealmId, RealmId>,
}
fn standalone_root(_scope: &mut v8::PinScope, context: v8::Local<v8::Context>) -> Option<RealmId> {
    context
        .get_slot::<FrameLifetime>()
        .map(|frame| frame.root_realm)
        .or_else(|| {
            context
                .get_slot::<EnvironmentDescriptor>()
                .map(|environment| environment.realm)
        })
}
fn standalone_group(scope: &mut v8::PinScope, root: RealmId) -> RealmId {
    scope
        .get_slot::<StandaloneAuxiliaryRoots>()
        .and_then(|roots| roots.roots.get(&root))
        .map_or(root, |root| root.group)
}
fn standalone_auxiliary_windows<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(context) = window.get_creation_context(scope) else {
        return Vec::new();
    };
    let Some(root) = standalone_root(scope, context) else {
        return Vec::new();
    };
    let group = standalone_group(scope, root);
    let candidates = scope
        .get_slot::<StandaloneAuxiliaryRoots>()
        .map(|roots| {
            roots
                .roots
                .iter()
                .filter(|(realm, entry)| {
                    entry.group == group
                        && roots
                            .current
                            .get(roots.lineages.get(realm).unwrap_or(realm))
                            .is_some_and(|current| current == *realm)
                })
                .map(|(realm, entry)| (*realm, entry.window.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    candidates
        .into_iter()
        .filter_map(|(_, window)| {
            let window = v8::Local::new(scope, &window);
            let context = window.get_creation_context(scope)?;
            if context
                .get_slot::<NativeWindowLifecycle>()
                .is_some_and(|lifecycle| lifecycle.closed.get())
            {
                return None;
            }
            context
                .get_slot::<V8BindingState>()
                .filter(|state| !state.execution_retired() && !state.execution_suspended())?;
            Some(window)
        })
        .collect()
}
fn create_standalone_auxiliary_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    creator: v8::Local<'s, v8::Context>,
    name: &str,
    noopener: bool,
    noreferrer: bool,
) -> Result<Option<v8::Local<'s, v8::Object>>, String> {
    let source = creator
        .get_slot::<V8BindingState>()
        .filter(|state| !state.execution_retired() && !state.execution_suspended())
        .ok_or("auxiliary creator inactive")?;
    let policy = creator
        .get_slot::<RefCell<crate::csp::CspPolicySet>>()
        .map(|policy| policy.borrow().clone())
        .unwrap_or_else(|| {
            crate::csp::CspPolicySet::from_headers(
                source.request_initiator_url().unwrap_or_default(),
                &[],
            )
        });
    if policy.sandbox().is_some_and(|flags| !flags.allow_popups) {
        return Ok(None);
    }
    if scope.get_slot::<StandaloneAuxiliaryRoots>().is_none() {
        scope.set_slot(StandaloneAuxiliaryRoots::default());
    }
    if scope
        .get_slot::<StandaloneAuxiliaryRoots>()
        .unwrap()
        .current
        .len()
        >= 256
    {
        return Err("standalone auxiliary root capacity".into());
    }
    let root = standalone_root(scope, creator).ok_or("auxiliary creator root absent")?;
    let group = standalone_group(scope, root);
    let inherited = policy.initial_auxiliary_policy();
    let flags = inherited.sandbox();
    let origin = if flags.as_ref().is_some_and(|flags| !flags.allow_same_origin) {
        Origin::opaque()
    } else {
        creator
            .get_slot::<RealmDescriptor>()
            .ok_or("auxiliary origin absent")?
            .origin
            .clone()
    };
    let base = source
        .native_document_base_url(true)
        .map_err(|error| error.to_string())?;
    let referrer = if noreferrer {
        "no-referrer".into()
    } else {
        source.document_referrer_policy()
    };
    let mut endpoints =
        endpoints_for_context(scope, creator).ok_or("auxiliary native endpoints unavailable")?;
    #[cfg(unix)]
    {
        endpoints.context = None;
    }
    endpoints.activity_parent = None;
    if let Some(fetch) = &mut endpoints.fetch {
        fetch.broker = fetch.broker.clone().new_request_scope();
        fetch.base_url = Url::parse(&base).map_err(|error| error.to_string())?;
    }
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<!doctype html><html><head></head><body></body></html>")
            .into_runtime()
            .map_err(|error| error.to_string())?,
    ));
    let document = dom.borrow().document();
    dom.borrow_mut()
        .arena_mut()
        .get_mut(document)
        .map_err(|error| error.to_string())?
        .document_metadata
        .as_mut()
        .ok_or("auxiliary document metadata absent")?
        .ready_state = oryn_dom::DocumentReadyState::Complete;
    let (realm, global) = create_native_realm_context(scope, origin, 1, None, None, endpoints)
        .map_err(|error| error.to_string())?;
    let context = v8::Local::new(scope, &global);
    let result = (|| -> Result<(), String> {
        context.set_slot(Rc::new(RefCell::new(inherited)));
        context.set_slot(Rc::new(NativeInitialDocument(true)));
        if let Some(isolation) =
            creator.get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
        {
            context.set_slot(isolation);
        }
        context.set_slot(Rc::new(oryn_js::realm::SecureContextState(
            creator
                .get_slot::<oryn_js::realm::SecureContextState>()
                .is_some_and(|secure| secure.0),
        )));
        let code = creator.get_slot::<DocumentCodePolicy>();
        let state = install_native_document(
            scope,
            context,
            dom.clone(),
            realm,
            1,
            document,
            code.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let provider = crate::executable::initial_layout_provider(
            dom.clone(),
            document,
        );
        provider.bind_stylesheet_document_owner(&dom.borrow(),
            oryn_js::v8_bindings::BindingStylesheetDocumentOwner {
                realm,
                document,
                document_generation: state.document_generation(),
                stream_epoch: state.document_stream_epoch(),
            })?;
        state.set_layout_provider(provider);
        state.set_inherited_base_url(Some(base));
        state
            .set_document_referrer_policy(&referrer)
            .map_err(|error| error.to_string())?;
        state.set_navigation_initial_document(true);
        state
            .commit_location("about:blank")
            .map_err(|error| error.to_string())?;
        state.set_modals_allowed(flags.as_ref().is_none_or(|flags| flags.allow_modals));
        state.set_storage_access_allowed(
            flags
                .as_ref()
                .is_none_or(|flags| flags.allow_storage_access_by_user_activation),
        );
        state
            .set_automatic_features_allowed(flags.as_ref().is_none_or(|flags| flags.allow_scripts));
        state.set_native_window_target_name(if name.eq_ignore_ascii_case("_blank") {
            String::new()
        } else {
            name.into()
        });
        let opener = creator.global(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        if !begin_runtime_bootstrap(context) {
            return Err("auxiliary bootstrap already admitted".to_owned());
        }
        for source in [
            crate::executable::RUST_RUNTIME_GLOBALS,
            crate::observable::OBSERVABLE_GLOBALS,
        ] {
            let source = v8::String::new(scope, source).ok_or("auxiliary bootstrap allocation")?;
            v8::Script::compile(scope, source, None)
                .and_then(|script| script.run(scope))
                .ok_or("auxiliary native bootstrap failed")?;
        }
        let window = context.global(scope);
        oryn_js::v8_bindings::set_native_window_opener(
            scope,
            window,
            (!noopener).then_some(opener),
        )
        .map_err(|error| error.to_string())?;
        finish_runtime_bootstrap(scope, context).map_err(|error| error.to_string())?;
        Ok(())
    })();
    if let Err(error) = result {
        fail_runtime_bootstrap(context);
        if let Some(state) = context.get_slot::<V8BindingState>() {
            state.retire_execution()
        }
        if let Some(environment) = context.get_slot::<EnvironmentDescriptor>() {
            retire_environment(scope, environment.id)
        }
        let identities = scope.get_slot_mut::<NativeRealmIdentities>().unwrap();
        identities.pending_contexts.remove(&realm);
        identities.contexts.remove(&realm);
        identities.realms.remove(realm);
        return Err(error);
    }
    let root_context = scope
        .get_slot::<NativeRealmIdentities>()
        .and_then(|identities| identities.contexts.get(&root).cloned())
        .and_then(|weak| weak.to_local(scope));
    let root_window = root_context.map(|context| {
        let window = context.global(scope);
        v8::Global::new(scope, window)
    });
    let window = context.global(scope);
    let retained = v8::Global::new(scope, window);
    let roots = scope.get_slot_mut::<StandaloneAuxiliaryRoots>().unwrap();
    if let Some(window) = root_window {
        roots.roots.entry(root).or_insert(StandaloneAuxiliary {
            script_created: false,
            group,
            opener: None,
            creator: root,
            window,
        });
        roots.current.entry(root).or_insert(root);
        roots.lineages.entry(root).or_insert(root);
    }
    roots.roots.insert(
        realm,
        StandaloneAuxiliary {
            script_created: true,
            group: if noopener { realm } else { group },
            opener: (!noopener).then_some(source.realm()),
            creator: source.realm(),
            window: retained,
        },
    );
    roots.current.insert(realm, realm);
    roots.lineages.insert(realm, realm);
    roots.pending.push(realm);
    Ok(Some(window))
}
impl RawV8Host {
    pub(crate) fn standalone_has_opener(&self, initial: RealmId) -> bool {
        self.standalone_navigation_root(initial)
            .is_some_and(|realm| {
                self.isolate
                    .get_slot::<StandaloneAuxiliaryRoots>()
                    .and_then(|roots| roots.roots.get(&realm))
                    .is_some_and(|entry| entry.opener.is_some())
            })
    }
    pub(crate) fn forget_standalone_lineage(&mut self, initial: RealmId) {
        if let Some(roots) = self.isolate.get_slot_mut::<StandaloneAuxiliaryRoots>() {
            roots.current.remove(&initial);
            let retired = roots
                .lineages
                .iter()
                .filter_map(|(realm, lineage)| (*lineage == initial).then_some(*realm))
                .collect::<Vec<_>>();
            for realm in retired {
                roots.lineages.remove(&realm);
                roots.roots.remove(&realm);
            }
        }
    }
    pub(crate) fn has_unclaimed_standalone_roots(&self) -> bool {
        self.isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .is_some_and(|roots| !roots.pending.is_empty())
    }
    pub(crate) fn take_created_standalone_roots(&mut self) -> Vec<RealmId> {
        self.adopt_created_realms();
        self.isolate
            .get_slot_mut::<StandaloneAuxiliaryRoots>()
            .map(|roots| std::mem::take(&mut roots.pending))
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub(crate) fn standalone_root_realms(&self) -> Vec<RealmId> {
        self.isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .map(|roots| {
                roots
                    .roots
                    .keys()
                    .filter(|realm| {
                        self.binding_states
                            .get(realm)
                            .is_some_and(|state| !state.execution_retired())
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub(crate) struct CreatedStandaloneRoot {
    pub realm: RealmId,
    pub dom: SharedDom,
    pub document: NodeId,
    pub url: Url,
    pub broker: Option<SharedNetworkBroker>,
    pub policy: crate::csp::CspPolicySet,
    pub origin: Origin,
    pub generation: u64,
    pub referrer: crate::referrer::ReferrerPolicy,
    pub isolation: crate::agent_isolation::DocumentIsolationPolicy,
}
impl RawV8Host {
    pub(crate) fn standalone_root_snapshot(
        &mut self,
        realm: RealmId,
    ) -> Result<CreatedStandaloneRoot, HostError> {
        self.adopt_created_realms();
        let global = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("standalone root absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &global);
        let res = {
            let scope = &mut v8::ContextScope::new(scope, context);
            context_has_parent_platform(scope, context)
        };
        if res {
            return Err(HostError::Execution(
                "parent root cannot use standalone driver".into(),
            ));
        }
        let state = context
            .get_slot::<V8BindingState>()
            .ok_or_else(|| HostError::Execution("standalone root bindings absent".into()))?;
        let descriptor = context
            .get_slot::<RealmDescriptor>()
            .ok_or_else(|| HostError::Execution("standalone root descriptor absent".into()))?;
        let dom = state.dom().clone();
        let document = dom.borrow().document();
        let url = Url::parse(
            &state
                .request_initiator_url()
                .unwrap_or_else(|_| "about:blank".into()),
        )
        .map_err(|error| HostError::Execution(error.to_string()))?;
        Ok(CreatedStandaloneRoot {
            realm,
            dom,
            document,
            url,
            broker: {
                let scope = &mut v8::ContextScope::new(scope, context);
                endpoints_for_context(scope, context)
            }
            .and_then(|entry| entry.fetch.as_ref().map(|fetch| fetch.broker.clone())),
            policy: context
                .get_slot::<RefCell<crate::csp::CspPolicySet>>()
                .map(|policy| policy.borrow().clone())
                .unwrap_or_else(|| crate::csp::CspPolicySet::from_headers("about:blank", &[])),
            origin: descriptor.origin.clone(),
            generation: state.document_generation(),
            referrer: crate::referrer::ReferrerPolicy::parse_list(
                &state.document_referrer_policy(),
            ),
            isolation: context
                .get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
                .map(|policy| *policy)
                .unwrap_or_default(),
        })
    }
    pub(crate) fn standalone_navigation_root(&self, initial: RealmId) -> Option<RealmId> {
        self.isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .and_then(|roots| {
                roots
                    .current
                    .get(roots.lineages.get(&initial).unwrap_or(&initial))
                    .copied()
            })
            .or_else(|| self.contexts.contains_key(&initial).then_some(initial))
    }
    pub(crate) fn create_standalone_navigation_root(
        &mut self,
        previous: RealmId,
        broker: Option<SharedNetworkBroker>,
        url: Option<Url>,
        origin: Origin,
        generation: u64,
        isolation: crate::agent_isolation::DocumentIsolationPolicy,
    ) -> Result<RealmId, HostError> {
        let old = self.contexts.get(&previous).cloned().ok_or_else(|| {
            HostError::Execution("standalone navigation predecessor absent".into())
        })?;
        self.isolate
            .get_slot::<NativeRealmIdentities>()
            .unwrap()
            .realms
            .ensure_capacity()
            .map_err(|_| HostError::LimitExceeded {
                resource: "realm identities".into(),
            })?;
        let preserve = self.realm_is_parked(previous);
        let previous_isolation = self
            .script_isolation
            .get(&previous)
            .copied()
            .unwrap_or_default();
        let same_origin = self
            .realm_descriptor(previous)
            .is_some_and(|descriptor| descriptor.origin == origin);
        let sever = previous_isolation.same_origin_opener != isolation.same_origin_opener
            || (isolation.same_origin_opener && !same_origin);
        let fetch = broker
            .zip(url.clone())
            .map(|(broker, base_url)| FetchState {
                broker,
                base_url,
                next_script_request: Rc::new(std::cell::Cell::new(900_000)),
            });
        let created = {
            v8::scope!(let scope,&mut self.isolate);
            let old = v8::Local::new(scope, &old);
            let res = {
                let scope = &mut v8::ContextScope::new(scope, old);
                context_has_parent_platform(scope, old)
            };
            if res {
                return Err(HostError::Execution(
                    "parent root cannot replace through standalone driver".into(),
                ));
            }
            let lifecycle = old.get_slot::<NativeWindowLifecycle>();
            let proxy = v8::Global::new(scope, old.global(scope));
            if !sever {
                if preserve {
                    oryn_js::v8_context_bridge::detach_global_preserving_microtasks(scope, old)
                } else {
                    oryn_js::v8_context_bridge::detach_global(scope, old)
                }
            }
            match create_native_realm_context(
                scope,
                origin.clone(),
                generation,
                None,
                (!sever).then_some(proxy),
                RealmEndpoints {
                    fetch,
                    #[cfg(unix)]
                    context: None,
                    activity_parent: None,
                },
            ) {
                Ok((realm, context)) => {
                    let local = v8::Local::new(scope, &context);
                    if let Some(lifecycle) = lifecycle {
                        if sever {
                            lifecycle.closed.set(true);
                            // A new browsing-context group changes proxy
                            // identity, not this native page's terminal owner.
                            local.set_slot(Rc::new(NativeWindowLifecycle {
                                closed: std::cell::Cell::new(false),
                                discarded: std::cell::Cell::new(false),
                                page_controller: lifecycle.page_controller.clone(),
                            }));
                        } else {
                            local.set_slot(lifecycle);
                        }
                    }
                    local.set_slot(Rc::new(NativeRootGeneration(generation)));
                    local.set_slot(Rc::new(NativeInitialDocument(false)));
                    local.set_slot(Rc::new(oryn_js::realm::SecureContextState(
                        oryn_js::realm::potentially_trustworthy_origin(&origin),
                    )));
                    Ok((realm, context))
                }
                Err(error) => {
                    if !sever && !oryn_js::v8_context_bridge::reattach_global(scope, old) {
                        return Err(HostError::Execution(format!(
                            "{error}; standalone proxy restoration failed"
                        )));
                    }
                    {
                        let scope = &mut v8::ContextScope::new(scope, old);
                        restore_native_context_security_token(scope, old)?;
                    }
                    Err(error)
                }
            }
        }?;
        let (realm, context) = created;
        let window = {
            v8::scope!(let scope,&mut self.isolate);
            let local = v8::Local::new(scope, &context);
            v8::Global::new(scope, local.global(scope))
        };
        self.contexts.insert(realm, context);
        self.isolate
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .pending_contexts
            .remove(&realm);
        if let Some(roots) = self.isolate.get_slot_mut::<StandaloneAuxiliaryRoots>() {
            let initial = roots
                .current
                .iter()
                .find_map(|(initial, current)| (*current == previous).then_some(*initial))
                .unwrap_or(previous);
            roots.current.insert(initial, realm);
            roots.lineages.insert(realm, initial);
            if let Some(mut entry) = roots.roots.get(&previous).cloned() {
                entry.window = window;
                if sever {
                    entry.group = realm;
                    entry.opener = None;
                    entry.creator = realm;
                }
                roots.roots.insert(realm, entry);
            }
        }
        let opener = self
            .isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .and_then(|roots| roots.roots.get(&realm).and_then(|entry| entry.opener));
        if let Some(opener) = opener.and_then(|opener| self.contexts.get(&opener).cloned()) {
            let global = self.contexts[&realm].clone();
            v8::scope!(let handles,&mut self.isolate);
            let context = v8::Local::new(handles, &global);
            let opener = v8::Local::new(handles, &opener);
            let window = context.global(handles);
            let opener = opener.global(handles);
            let scope = &mut v8::ContextScope::new(handles, context);
            oryn_js::v8_bindings::set_native_window_opener(scope, window, Some(opener))
                .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        self.active_realm = realm;
        Ok(realm)
    }
    pub(crate) fn restore_standalone_navigation_proxy(
        &mut self,
        realm: RealmId,
    ) -> Result<(), HostError> {
        let initial = self
            .isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .and_then(|roots| roots.lineages.get(&realm).copied())
            .unwrap_or(realm);
        let Some(current) = self.standalone_navigation_root(initial) else {
            return Err(HostError::Execution("standalone lineage missing".into()));
        };
        if current == realm {
            return Ok(());
        }
        let target = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("standalone cached context missing".into()))?;
        let old = self
            .contexts
            .get(&current)
            .cloned()
            .ok_or_else(|| HostError::Execution("standalone current context missing".into()))?;
        let preserve = self.realm_is_parked(current);
        {
            v8::scope!(let scope,&mut self.isolate);
            let old = v8::Local::new(scope, &old);
            let target = v8::Local::new(scope, &target);
            if target.global(scope).strict_equals(old.global(scope).into()) {
                if preserve {
                    oryn_js::v8_context_bridge::detach_global_preserving_microtasks(scope, old)
                } else {
                    oryn_js::v8_context_bridge::detach_global(scope, old)
                };
                if !oryn_js::v8_context_bridge::reattach_global(scope, target) {
                    let restored = oryn_js::v8_context_bridge::reattach_global(scope, old);
                    if restored {
                        {
                            let scope = &mut v8::ContextScope::new(scope, old);
                            restore_native_context_security_token(scope, old)?;
                        }
                    }
                    return Err(HostError::Execution(
                        "standalone history proxy restoration failed".into(),
                    ));
                }
            } else {
                // COOP retained a different native proxy. Its parked context remains
                // attached; restoring it never joins two groups through an author value.
                if let Some(lifecycle) = old.get_slot::<NativeWindowLifecycle>() {
                    lifecycle.closed.set(true)
                }
            }
            if let Some(lifecycle) = target.get_slot::<NativeWindowLifecycle>() {
                lifecycle.closed.set(false)
            }
            {
                let scope = &mut v8::ContextScope::new(scope, target);
                restore_native_context_security_token(scope, target)?;
            }
        }
        self.isolate
            .get_slot_mut::<StandaloneAuxiliaryRoots>()
            .unwrap()
            .current
            .insert(initial, realm);
        self.active_realm = realm;
        Ok(())
    }
}

struct StandaloneRootDrivers(Rc<RefCell<crate::runtime::standalone_root_driver::Roots>>);
impl RawV8Host {
    pub(crate) fn standalone_root_drivers(
        &mut self,
    ) -> Rc<RefCell<crate::runtime::standalone_root_driver::Roots>> {
        if self.isolate.get_slot::<StandaloneRootDrivers>().is_none() {
            self.isolate
                .set_slot(StandaloneRootDrivers(Rc::new(RefCell::new(
                    Default::default(),
                ))));
        }
        self.isolate
            .get_slot::<StandaloneRootDrivers>()
            .unwrap()
            .0
            .clone()
    }
    pub(crate) fn retain_standalone_root_lineage(&mut self, realm: RealmId) {
        if self
            .isolate
            .get_slot::<StandaloneAuxiliaryRoots>()
            .is_none()
        {
            self.isolate.set_slot(StandaloneAuxiliaryRoots::default());
        }
        let roots = self
            .isolate
            .get_slot_mut::<StandaloneAuxiliaryRoots>()
            .unwrap();
        roots.current.entry(realm).or_insert(realm);
        roots.lineages.entry(realm).or_insert(realm);
    }
}
