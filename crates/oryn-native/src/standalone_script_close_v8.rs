// Standalone close is a retained native task. Admission publishes logical
// closed synchronously; the task selects the then-current target Document.
#[derive(Default)]
struct StandaloneScriptCloses {
    jobs: BTreeMap<RealmId, Rc<RefCell<StandaloneScriptClose>>>,
    discarded: BTreeSet<RealmId>,
    microtasks: Vec<(
        Vec<v8::Global<v8::Context>>,
        v8::UniqueRef<v8::MicrotaskQueue>,
    )>,
}
struct StandaloneScriptClose {
    root: RealmId,
    window: Rc<NativeWindowLifecycle>,
    departure: Option<NativeScriptClose>,
    selected: Option<RealmId>,
}
fn admit_standalone_script_close(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    window: &Rc<NativeWindowLifecycle>,
) -> Result<(), String> {
    let target = context
        .get_slot::<V8BindingState>()
        .filter(|state| !state.execution_retired() && !state.execution_suspended())
        .ok_or("close target Document inactive")?;
    let auxiliary = scope
        .get_slot::<StandaloneAuxiliaryRoots>()
        .map(|roots| roots.roots.clone())
        .unwrap_or_default();
    if !auxiliary
        .get(&target.realm())
        .is_some_and(|entry| entry.script_created)
        && !target.native_history_has_single_entry()
    {
        return Ok(());
    }
    let source = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .filter(|state| !state.execution_retired() && !state.execution_suspended())
        .ok_or("close incumbent inactive")?;
    // Validate the incumbent's native tree before evaluating related-root
    // familiarity. Neither document.domain nor author opener values add edges.
    let contexts = scope
        .get_slot::<NativeRealmIdentities>()
        .ok_or("close native registry absent")?
        .contexts
        .clone();
    let mut cursor = source.realm();
    let mut seen = BTreeSet::new();
    let mut source_origin = None;
    loop {
        if !seen.insert(cursor) || seen.len() > 4096 {
            return Err("close native ancestry invalid".into());
        }
        let Some(local) = contexts.get(&cursor).and_then(|weak| weak.to_local(scope)) else {
            return Ok(());
        };
        let Some(current) = local
            .get_slot::<V8BindingState>()
            .filter(|state| !state.execution_retired() && !state.execution_suspended())
        else {
            return Ok(());
        };
        if cursor == source.realm() {
            if !Rc::ptr_eq(&source, &current) {
                return Ok(());
            }
            source_origin = local
                .get_slot::<RealmDescriptor>()
                .map(|realm| realm.origin.clone());
        }
        let Some(frame) = local.get_slot::<FrameLifetime>() else {
            break;
        };
        if frame.closed.get() {
            return Ok(());
        }
        let Some(parent) = frame
            .parent
            .upgrade()
            .filter(|state| !state.execution_retired() && !state.execution_suspended())
        else {
            return Ok(());
        };
        if parent.dom().borrow().frame_context_generation(frame.frame)
            != Some(frame.connection_generation)
        {
            return Ok(());
        }
        cursor = parent.realm();
    }
    let source_root = cursor;
    let target_root = target.realm();
    let group = |root| auxiliary.get(&root).map_or(root, |entry| entry.group);
    if group(source_root) != group(target_root) {
        return Ok(());
    }
    let mut familiar = source_root == target_root;
    let mut cursor = Some(target_root);
    let mut examined = BTreeSet::new();
    while let Some(candidate) = cursor {
        if !examined.insert(candidate) || examined.len() > 4096 {
            return Err("close native opener ancestry invalid".into());
        }
        let Some(local) = contexts
            .get(&candidate)
            .and_then(|weak| weak.to_local(scope))
        else {
            break;
        };
        let Some(current) = local
            .get_slot::<V8BindingState>()
            .filter(|state| !state.execution_retired() && !state.execution_suspended())
        else {
            break;
        };
        if source_origin.as_ref().is_some_and(|origin| {
            local
                .get_slot::<RealmDescriptor>()
                .is_some_and(|realm| &realm.origin == origin)
        }) || candidate == source_root
        {
            familiar = true;
            break;
        }
        cursor = auxiliary
            .get(&candidate)
            .and_then(|entry| entry.opener)
            .and_then(|opener| contexts.get(&opener))
            .and_then(|weak| weak.to_local(scope))
            .and_then(|opener| standalone_root(scope, opener));
        let _ = current;
    }
    if !familiar {
        return Ok(());
    }
    let flags = context
        .get_slot::<RefCell<crate::csp::CspPolicySet>>()
        .and_then(|policy| policy.borrow().sandbox());
    let allowed = if Rc::ptr_eq(&source, &target) {
        true
    } else if source_root == target_root {
        flags.as_ref().is_none_or(|flags| {
            flags.allow_top_navigation
                || (target.has_transient_user_activation()
                    && flags.allow_top_navigation_by_user_activation)
        })
    } else {
        auxiliary
            .get(&target_root)
            .is_some_and(|entry| entry.creator == source.realm())
            || flags.is_none()
    };
    if !allowed {
        return Ok(());
    }
    if scope.get_slot::<StandaloneScriptCloses>().is_none() {
        scope.set_slot(StandaloneScriptCloses::default());
    }
    let jobs = scope.get_slot_mut::<StandaloneScriptCloses>().unwrap();
    if jobs.jobs.contains_key(&target.realm()) {
        return Ok(());
    }
    if jobs.jobs.len() >= 64 {
        return Err("standalone close task capacity".into());
    }
    jobs.jobs.insert(
        target.realm(),
        Rc::new(RefCell::new(StandaloneScriptClose {
            root: target.realm(),
            window: window.clone(),
            departure: None,
            selected: None,
        })),
    );
    window.closed.set(true);
    Ok(())
}
impl RawV8Host {
    pub(crate) fn standalone_close_pending(&self, realm: RealmId) -> bool {
        self.isolate
            .get_slot::<StandaloneScriptCloses>()
            .is_some_and(|queue| {
                queue
                    .jobs
                    .keys()
                    .any(|root| self.standalone_navigation_root(*root).unwrap_or(*root) == realm)
            })
    }
    fn has_standalone_script_close(&self) -> bool {
        self.isolate
            .get_slot::<StandaloneScriptCloses>()
            .is_some_and(|queue| {
                queue.jobs.keys().any(|realm| {
                    !self.realm_is_parked(self.standalone_navigation_root(*realm).unwrap_or(*realm))
                })
            })
    }
    fn poll_standalone_script_closes(&mut self) -> Result<usize, HostError> {
        self.adopt_created_realms();
        let jobs = self
            .isolate
            .get_slot::<StandaloneScriptCloses>()
            .map(|queue| queue.jobs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut completed = 0;
        for job in jobs {
            let mut job = job.borrow_mut();
            let key = job.root;
            let root = job
                .selected
                .unwrap_or_else(|| self.standalone_navigation_root(key).unwrap_or(key));
            if self.realm_is_parked(root) {
                continue;
            }
            if job.departure.is_none() {
                if !self
                    .binding_states
                    .get(&root)
                    .is_some_and(|state| !state.execution_retired() && !state.execution_suspended())
                {
                    self.isolate
                        .get_slot_mut::<StandaloneScriptCloses>()
                        .unwrap()
                        .jobs
                        .remove(&key);
                    continue;
                }
                let creator = self.native_creator_for_realm(root)?;
                job.departure = Some(self.prepare_native_script_close(root, creator)?);
                job.selected = Some(root);
            }
            match self.poll_native_script_close(job.departure.as_mut().unwrap())? {
                None => {}
                Some(false) => {
                    self.isolate
                        .get_slot_mut::<StandaloneScriptCloses>()
                        .unwrap()
                        .jobs
                        .remove(&key);
                    completed += 1;
                }
                Some(true) => {
                    self.discard_standalone_closed_root(root)?;
                    job.window.discarded.set(true);
                    self.isolate
                        .get_slot_mut::<StandaloneScriptCloses>()
                        .unwrap()
                        .jobs
                        .remove(&key);
                    completed += 1;
                }
            }
        }
        Ok(completed)
    }
    fn ensure_standalone_author_task(&mut self) -> Result<(), HostError> {
        if self
            .contexts
            .get(&self.active_realm)
            .is_some_and(|context| {
                context
                    .open(&mut self.isolate)
                    .get_slot::<RuntimeBootstrapped>()
                    .is_some_and(|bootstrap| bootstrap.phase.get() == RuntimeBootstrapPhase::Failed)
            })
        {
            return Err(HostError::Execution(
                "runtime bootstrap previously failed".into(),
            ));
        }
        if self
            .isolate
            .get_slot::<StandaloneScriptCloses>()
            .is_some_and(|queue| queue.discarded.contains(&self.active_realm))
        {
            Err(HostError::Execution(
                "standalone Window has completed departure".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn release_standalone_closed_microtasks(&mut self) {
        let queues = self
            .isolate
            .get_slot_mut::<StandaloneScriptCloses>()
            .map(|queue| std::mem::take(&mut queue.microtasks))
            .unwrap_or_default();
        v8::scope!(let scope,&mut self.isolate);
        for (contexts, queue) in queues {
            for context in contexts {
                let context = v8::Local::new(scope, &context);
                if oryn_js::v8_context_bridge::has_microtask_queue(context, &queue)
                    && !oryn_js::v8_context_bridge::resume_microtasks(context, &queue, true)
                {
                    std::process::abort();
                }
            }
        }
    }
    fn discard_standalone_closed_root(&mut self, root: RealmId) -> Result<(), HostError> {
        // Keep inert WindowProxy shells so existing references still observe closed.
        // Retire every native environment and binding; their own owners cancel IO,
        // timers, observers and child tasks with retained cleanup acknowledgement.
        let context = self
            .contexts
            .get(&root)
            .cloned()
            .ok_or_else(|| HostError::Execution("closed root context absent".into()))?;
        let environment = {
            v8::scope!(let scope,&mut self.isolate);
            v8::Local::new(scope, &context)
                .get_slot::<EnvironmentDescriptor>()
                .map(|environment| *environment)
                .ok_or_else(|| HostError::Execution("closed root environment absent".into()))?
        };
        let environments = self.document_environments(environment);
        let globals = environments
            .iter()
            .filter_map(|environment| self.contexts.get(&environment.realm).cloned())
            .collect::<Vec<_>>();
        let microtasks = self.suspend_root_microtasks(&environments)?;
        {
            let queue = self
                .isolate
                .get_slot_mut::<StandaloneScriptCloses>()
                .expect("admitted close queue");
            queue
                .discarded
                .extend(environments.iter().map(|environment| environment.realm));
            queue.microtasks.push((globals, microtasks));
        }
        for environment in &environments {
            if let Some(state) = self.binding_states.get(&environment.realm) {
                state.retire_execution();
            }
        }
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        if let Some(environment) = context.get_slot::<EnvironmentDescriptor>() {
            retire_environment(scope, environment.id);
        }
        if let Some(frames) = scope.get_slot::<FrameLifetimes>() {
            for frame in frames
                .live
                .values()
                .filter(|frame| frame.root_realm == root)
            {
                frame.closed.set(true);
                frame.opener.borrow_mut().take();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod standalone_script_close_tests {
    use super::*;
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>original</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            1,
        )
        .unwrap();
        host.install_runtime_bootstrap(
            crate::executable::RUST_RUNTIME_GLOBALS,
            crate::observable::OBSERVABLE_GLOBALS,
        )
        .unwrap();
        let history = host.navigation_history_snapshot().unwrap();
        host.sync_session_history(history).unwrap();
        host
    }
    fn retained_trace(host: &mut RawV8Host) -> String {
        v8::scope!(let scope,&mut host.isolate);
        let context = v8::Local::new(scope, &host.contexts[&RealmId(0)]);
        let scope = &mut v8::ContextScope::new(scope, context);
        let key = v8::String::new(scope, "trace").unwrap();
        let value = context.global(scope).get(scope, key.into()).unwrap();
        let array = v8::Local::<v8::Array>::try_from(value).unwrap();
        (0..array.length())
            .map(|index| {
                array
                    .get_index(scope, index)
                    .unwrap()
                    .to_rust_string_lossy(scope)
            })
            .collect::<Vec<_>>()
            .join(",")
    }
    #[test]
    fn logical_closed_is_synchronous_but_departure_is_a_later_native_task() {
        let mut host = host();
        host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('before');onpagehide=e=>trace.push('hide:'+e.persisted);onunload=()=>trace.push('unload');close();trace.push(String(closed));queueMicrotask(()=>trace.push('micro'));close()").unwrap();
        host.run_microtask_checkpoint().unwrap();
        assert_eq!(retained_trace(&mut host), "true,micro");
        assert!(host.has_standalone_script_close());
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 1);
        assert_eq!(
            retained_trace(&mut host),
            "true,micro,before,hide:false,unload"
        );
        assert!(host.binding_states[&RealmId(0)].execution_retired());
        assert!(host.execute("trace.push('late')").is_err());
        assert!(host.evaluate_string("1").is_err());
        assert!(!host.has_standalone_script_close());
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 0);
    }
    #[test]
    fn full_history_and_nested_targets_deny_close_without_logical_change() {
        let mut host = host();
        host.execute("history.pushState({},'', '#second');close();globalThis.rootClosed=closed;globalThis.child=document.createElement('iframe');document.body.append(child);child.contentWindow.close();globalThis.childClosed=child.contentWindow.closed").unwrap();
        assert_eq!(
            host.evaluate_string("rootClosed+':'+childClosed").unwrap(),
            "false:false"
        );
        assert!(!host.has_standalone_script_close());
    }
    #[test]
    fn canceled_beforeunload_keeps_logical_closed_and_live_document() {
        struct Dismiss;
        impl oryn_js::prompts::PromptBackend for Dismiss {
            fn request(
                &self,
                _: oryn_js::prompts::PromptRequest,
            ) -> Result<oryn_js::prompts::PromptResponse, oryn_js::prompts::PromptError>
            {
                Ok(oryn_js::prompts::PromptResponse::Dismissed)
            }
            fn cancel(&self, _: oryn_js::prompts::PromptToken) {}
        }
        let mut host = host();
        host.binding_states[&RealmId(0)]
            .prompt_controller()
            .set_backend(Arc::new(Dismiss))
            .unwrap();
        host.notify_user_activation().unwrap();
        host.execute("globalThis.trace=[];onbeforeunload=e=>{trace.push('before');e.preventDefault()};onpagehide=()=>trace.push('hide');close()").unwrap();
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 1);
        assert_eq!(
            host.evaluate_string("closed+':'+trace.join(',')+':'+document.body.textContent")
                .unwrap(),
            "true:before:original"
        );
        assert!(!host.binding_states[&RealmId(0)].execution_retired());
        host.execute("close()").unwrap();
        assert!(!host.has_standalone_script_close());
    }
    #[test]
    fn descendant_close_checks_destination_sandbox_flags() {
        for allow in [false, true] {
            let mut host = host();
            host.execute(
                "globalThis.child=document.createElement('iframe');document.body.append(child)",
            )
            .unwrap();
            host.adopt_created_realms();
            let child = host
                .binding_states
                .keys()
                .copied()
                .find(|realm| *realm != RealmId(0))
                .unwrap();
            {
                v8::scope!(let scope,&mut host.isolate);
                let context = v8::Local::new(scope, &host.contexts[&RealmId(0)]);
                context.set_slot(Rc::new(RefCell::new(
                    crate::csp::CspPolicySet::from_headers(
                        "https://example.test/",
                        &[(
                            "content-security-policy".into(),
                            if allow {
                                "sandbox allow-scripts allow-same-origin allow-top-navigation"
                            } else {
                                "sandbox allow-scripts allow-same-origin"
                            }
                            .into(),
                        )],
                    ),
                )));
            }
            host.activate_realm(child).unwrap();
            host.execute("top.close()").unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(
                host.evaluate_string("String(closed)").unwrap(),
                allow.to_string()
            );
            assert_eq!(host.has_standalone_script_close(), allow);
        }
    }
    #[test]
    fn queued_close_selects_replacement_document_and_survives_admitting_source() {
        let mut host = host();
        host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('old');close()")
            .unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>replacement</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            2,
        )
        .unwrap();
        host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('new-before');onpagehide=()=>trace.push('new-hide');onunload=()=>trace.push('new-unload')").unwrap();
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 1);
        assert_eq!(retained_trace(&mut host), "new-before,new-hide,new-unload");
        assert!(host.binding_states[&RealmId(0)].execution_retired());
    }
    #[test]
    fn retained_departure_selects_current_document_after_permission_and_retires_descendants() {
        let mut host = host();
        host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('before');close()")
            .unwrap();
        CLOSE_PAUSE_AFTER_BEFORE.with(|pause| pause.set(true));
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 0);
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>next</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            2,
        )
        .unwrap();
        host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('wrong-before');onpagehide=()=>trace.push('hide');onunload=()=>trace.push('unload');globalThis.child=document.createElement('iframe');document.body.append(child);child.contentWindow.onpagehide=()=>trace.push('child-hide');child.contentWindow.onunload=()=>trace.push('child-unload')").unwrap();
        assert_eq!(host.poll_standalone_script_closes().unwrap(), 1);
        assert_eq!(
            retained_trace(&mut host),
            "hide,child-hide,unload,child-unload"
        );
        assert!(
            host.binding_states
                .values()
                .all(|state| state.execution_retired())
        );
    }
}
