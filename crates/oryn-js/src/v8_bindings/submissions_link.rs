//! Frozen hyperlink activation through selection, admission and publication.
use super::*;
pub(in super::super) struct LinkActivation {
    owner: Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    authority_epoch: u64,
    document: NodeId,
    target: Option<RootedTarget>,
    destination: Option<Rc<V8BindingState>>,
    reservation: Option<Reservation>,
    authority: Option<crate::navigation::ScriptNavigationAdmission>,
    phase: u8,
    terminal: Option<String>,
}
struct Reservation {
    owner: Rc<V8BindingState>,
}
impl Reservation {
    fn new(owner: &Rc<V8BindingState>) -> Result<Self, V8BindingError> {
        check_capacity(owner)?;
        let count = owner.reserved_default_actions.get();
        owner
            .pending_default_actions
            .borrow_mut()
            .try_reserve(count + 1)
            .map_err(|_| V8BindingError::Allocation("accepted hyperlink publication"))?;
        owner.reserved_default_actions.set(count + 1);
        Ok(Self {
            owner: owner.clone(),
        })
    }
    fn publish(self, action: PendingDefaultAction) {
        self.owner
            .pending_default_actions
            .borrow_mut()
            .push_back(action);
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.owner
            .reserved_default_actions
            .set(self.owner.reserved_default_actions.get() - 1);
    }
}
impl LinkActivation {
    pub(in super::super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        node: NodeId,
        user_initiated: bool,
        suffix: Option<&str>,
        target_override: Option<&str>,
    ) -> Result<Option<Self>, V8BindingError> {
        check_capacity(state)?;
        let mut target = RootedTarget::new(state, node)?;
        target.user_initiated = user_initiated;
        let Some(navigation) =
            freeze_link(scope, state, node, user_initiated, suffix, target_override)?
        else {
            return Ok(None);
        };
        let document = owner_document_for_node(state, node)?;
        target.navigation = Some(navigation);
        Ok(Some(Self {
            owner: state.clone(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority_epoch: state.document_authority_epoch(),
            document,
            target: Some(target),
            destination: None,
            reservation: None,
            authority: None,
            phase: 0,
            terminal: None,
        }))
    }
    pub(in super::super) fn retry_safe(&self) -> bool {
        self.terminal.is_none()
    }
    pub(in super::super) fn cancel(&mut self) {
        self.target = None;
        self.destination = None;
        self.authority = None;
        self.reservation = None;
        self.phase = 5;
    }
    pub(in super::super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        if let Some(error) = &self.terminal {
            return Err(V8BindingError::Message(error.clone()));
        }
        loop {
            if self.phase == 5 {
                return Ok(true);
            }
            let state = self.owner.clone();
            // Once local admission returned its token, the reserved native
            // queue handoff cannot yield back to a source-owned receipt. That
            // source may have retired in Navigate; no V8 work remains here.
            if scope.is_execution_terminating() && !(self.phase == 4 && self.reservation.is_some())
            {
                return Ok(false);
            }
            if self.phase < 4 || self.destination.is_none() {
                if state.execution_retired()
                    || state.document_generation() != self.generation
                    || state.document_stream_epoch() != self.epoch
                    || state.document_authority_epoch() != self.authority_epoch
                {
                    self.cancel();
                    return Ok(true);
                }
                if state.execution_suspended() {
                    return Ok(false);
                }
            }
            let target = self.target.as_mut().unwrap();
            let navigation = target.navigation.as_mut().unwrap();
            match self.phase {
                0 => {
                    // A selection service error may follow auxiliary creation.
                    // Preserve it as terminal rather than allocate a second
                    // window with another request. Successful selection is
                    // stored before any later fallible preparation.
                    let name = if navigation.download.is_some() {
                        "_self"
                    } else {
                        &navigation.target
                    };
                    navigation.navigation_target = match select_target(
                        scope,
                        &state,
                        self.document,
                        name,
                        &navigation.url,
                        navigation.noopener,
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            self.terminal = Some(error.to_string());
                            return Err(error);
                        }
                    };
                    if matches!(navigation.navigation_target, SubmissionTarget::Denied) {
                        self.cancel();
                        return Ok(true);
                    }
                    self.phase = 1;
                }
                1 => {
                    self.destination =
                        scope
                            .get_slot::<BindingStateRegistry>()
                            .and_then(|registry| {
                                registry
                                    .realms
                                    .borrow()
                                    .values()
                                    .filter_map(RcWeak::upgrade)
                                    .find(|owner| {
                                        !owner.execution_retired()
                                            && !owner.execution_suspended()
                                            && match &navigation.navigation_target {
                                                SubmissionTarget::Parent(selected) => {
                                                    selected.local_realm == Some(owner.realm())
                                                }
                                                _ => {
                                                    capture_target_state(owner)
                                                        == navigation.navigation_target
                                                        && (matches!(
                                                            navigation.navigation_target,
                                                            SubmissionTarget::Root { .. }
                                                        ) || parent_state(owner).is_some())
                                                }
                                            }
                                    })
                            });
                    if self.destination.is_none()
                        && let SubmissionTarget::Parent(selected) = &navigation.navigation_target
                    {
                        if selected.local_realm.is_some() {
                            self.cancel();
                            return Ok(true);
                        }
                        if let Some(dispatch) = state.selected_link_dispatcher.get() {
                            let source = navigation.document_source.as_deref().ok_or(
                                V8BindingError::Allocation("selected link source provenance"),
                            )?;
                            let audit = navigation.audit.as_ref().map(|audit| {
                                let token = audit.token();
                                audit.transferred();
                                token
                            });
                            match dispatch(
                                scope,
                                selected,
                                source,
                                &navigation.url,
                                navigation.referrer_policy.as_deref(),
                                target.user_initiated,
                                audit,
                            ) {
                                Ok(()) => {
                                    self.cancel();
                                    return Ok(true);
                                }
                                Err(error) => {
                                    self.terminal = Some(error.to_string());
                                    return Err(error);
                                }
                            }
                        }
                    }
                    self.phase = if self.destination.is_some() { 2 } else { 4 };
                }
                2 => {
                    let destination = self.destination.as_ref().unwrap();
                    if destination.execution_retired() {
                        self.cancel();
                        return Ok(true);
                    }
                    if destination.execution_suspended() {
                        return Ok(false);
                    }
                    if let Some(backend) = state.location_navigation_backend.borrow().clone() {
                        if navigation.download.is_some() {
                            let allowed = backend
                                .authorize_download(
                                    state.native_worker_creator()?,
                                    navigation.url.clone(),
                                )
                                .map_err(V8BindingError::Message)?;
                            if !allowed {
                                self.cancel();
                                return Ok(true);
                            }
                        } else {
                            let creator = destination.native_worker_creator()?;
                            let authority = match &navigation.navigation_target {
                                SubmissionTarget::Parent(selected) => backend.authorize_selected(
                                    state.native_worker_creator()?,
                                    selected.page_id,
                                    creator.clone(),
                                    navigation.url.clone(),
                                    state.request_initiator_url()?,
                                    navigation
                                        .referrer_policy
                                        .clone()
                                        .unwrap_or_else(|| state.document_referrer_policy()),
                                ),
                                _ => backend.authorize(
                                    state.native_worker_creator()?,
                                    creator.clone(),
                                    navigation.url.clone(),
                                    state.request_initiator_url()?,
                                    navigation
                                        .referrer_policy
                                        .clone()
                                        .unwrap_or_else(|| state.document_referrer_policy()),
                                ),
                            }
                            .map_err(V8BindingError::Message)?;
                            let Some(authority) = authority else {
                                self.cancel();
                                return Ok(true);
                            };
                            self.authority = Some(crate::navigation::ScriptNavigationAdmission {
                                authority,
                                target: creator,
                            });
                        }
                    }
                    self.phase = 3;
                }
                3 => {
                    let destination = self.destination.as_ref().unwrap();
                    if destination.execution_retired() {
                        self.cancel();
                        return Ok(true);
                    }
                    if destination.execution_suspended() {
                        return Ok(false);
                    }
                    if self.reservation.is_none() {
                        self.reservation = Some(Reservation::new(destination)?);
                    }
                    let context = destination
                        .global_object
                        .to_local(scope)
                        .and_then(|window| window.get_creation_context(scope))
                        .ok_or(V8BindingError::Allocation("link destination context"))?;
                    let scope = &mut v8::ContextScope::new(scope, context);
                    // Navigate admission retains an interrupted event tail and
                    // cancels that exact admission; it never returns a token
                    // for it. Reissuing here would create another navigation.
                    match crate::navigation_api::admit_native_navigation(
                        scope,
                        &navigation.url,
                        false,
                        false,
                        navigation.navigation_source.clone(),
                    ) {
                        Ok(crate::navigation_api::NativeNavigationAdmission::Handled) => {
                            self.cancel();
                            return Ok(true);
                        }
                        Ok(crate::navigation_api::NativeNavigationAdmission::Proceed {
                            operation,
                            destination: entry,
                        }) => {
                            navigation.admission = Some(AcceptedLinkNavigation {
                                realm: destination.realm(),
                                operation,
                                destination: entry.map(|entry| *entry),
                                script_authority: self.authority.take(),
                            });
                            self.phase = 4;
                        }
                        Err(error) => {
                            self.terminal = Some(error.to_string());
                            return Err(error);
                        }
                    }
                }
                4 => {
                    // Local admission reserves the destination queue before
                    // callbacks. Publication therefore cannot be stranded in
                    // a source realm retired by a destination Navigate handler.
                    if self.reservation.is_none() {
                        self.reservation = Some(Reservation::new(&state)?);
                    }
                    let reservation = self.reservation.take().unwrap();
                    let target = self.target.take().unwrap();
                    reservation.publish(PendingDefaultAction::Click(Box::new(target)));
                    self.phase = 5;
                    #[cfg(test)]
                    if FAIL_PUBLICATION.with(|flag| flag.replace(false)) {
                        return Err(V8BindingError::Allocation(
                            "injected after hyperlink publication",
                        ));
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}
impl Drop for LinkActivation {
    fn drop(&mut self) {
        self.cancel()
    }
}
#[cfg(test)]
thread_local! {pub(super) static FAIL_PUBLICATION:Cell<bool>=const{Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    thread_local! {static AUDIT:RefCell<Vec<String>>=const{RefCell::new(Vec::new())};}
    struct MockAudit(std::sync::atomic::AtomicBool);
    impl crate::navigation::HyperlinkAudit for MockAudit {
        fn token(&self) -> crate::navigation::HyperlinkAuditToken {
            crate::navigation::HyperlinkAuditToken {
                page_id: 1,
                capability: vec![1; 32],
            }
        }
        fn accepted(&self) {
            if !self.0.swap(true, std::sync::atomic::Ordering::AcqRel) {
                AUDIT.with(|log| log.borrow_mut().push("accepted".into()));
            }
        }
        fn discarded(&self) {
            if !self.0.swap(true, std::sync::atomic::Ordering::AcqRel) {
                AUDIT.with(|log| log.borrow_mut().push("discarded".into()));
            }
        }
        fn transferred(&self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
            AUDIT.with(|log| log.borrow_mut().push("transferred".into()));
        }
    }
    fn prepare_audit(
        _scope: &mut v8::PinScope,
        _state: &V8BindingState,
        destination: &str,
        urls: &[String],
    ) -> Result<Option<std::sync::Arc<dyn crate::navigation::HyperlinkAudit>>, V8BindingError> {
        AUDIT.with(|log| {
            let mut log = log.borrow_mut();
            log.push(destination.into());
            log.extend_from_slice(urls);
        });
        Ok(Some(std::sync::Arc::new(MockAudit(
            std::sync::atomic::AtomicBool::new(false),
        ))))
    }
    #[test]
    fn audit_freezes_legacy_queries_and_requires_uncanceled_admission() {
        super::super::super::tests::initialize_v8();
        for cancel in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<a id=a href=/b ping='/audit?Я'>go</a>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(259), 1).unwrap();
            history(scope, &state);
            state
                .dom
                .borrow_mut()
                .arena_mut()
                .get_mut(state.document_root())
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = "windows-1251".into();
            state.set_hyperlink_audit_preparer(prepare_audit);
            AUDIT.with(|log| log.borrow_mut().clear());
            eval(
                scope,
                if cancel {
                    "navigation.addEventListener('navigate',e=>{a.setAttribute('ping','/mutated');e.preventDefault()})"
                } else {
                    "navigation.addEventListener('navigate',()=>a.setAttribute('ping','/mutated'))"
                },
            );
            let node = state.dom.borrow().find_by_id("a").unwrap();
            let mut job = LinkActivation::new(scope, &state, node, true, None, None)
                .unwrap()
                .unwrap();
            assert!(job.poll(scope).unwrap());
            drop(job);
            AUDIT.with(|log| {
                assert_eq!(
                    *log.borrow(),
                    vec![
                        "https://example.test/b",
                        "https://example.test/audit?%DF",
                        if cancel { "discarded" } else { "accepted" }
                    ]
                )
            });
            assert_eq!(
                state.pending_default_actions.borrow().len(),
                usize::from(!cancel)
            );
        }
    }
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    fn history(scope: &mut v8::PinScope, state: &V8BindingState) {
        state.commit_location("https://example.test/a").unwrap();
        crate::navigation_api::sync_history(
            scope,
            vec![crate::navigation_api::NavigationHistorySnapshot {
                scroll: Default::default(),
                key: "key-a".into(),
                id: "a".into(),
                url: "https://example.test/a".into(),
                document_id: "retained-document".into(),
                state: None,
                history_state: None,
            }],
            0,
            "push",
        )
        .unwrap();
    }
    #[test]
    fn destination_owns_accepted_link_when_navigate_listener_retires_source() {
        fn retire_source(
            scope: &mut v8::PinScope,
            args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            let object = v8::Local::<v8::Object>::try_from(args.data()).unwrap();
            binding_state_for_receiver(scope, object)
                .unwrap()
                .retire_execution();
        }
        fn select(
            _scope: &mut v8::PinScope,
            _state: &V8BindingState,
            _source: &oryn_dom::DocumentProvenance,
            _name: &str,
            _url: &str,
            _noopener: bool,
        ) -> Result<Option<SelectedNavigationTarget>, V8BindingError> {
            Ok(Some(SelectedNavigationTarget {
                receipt: crate::navigation::NavigationTargetReceipt(vec![1]),
                page_id: 1,
                local_realm: Some(RealmId(258)),
                local_frame: None,
            }))
        }
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_context = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, source_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<a id=a href=/b target=destination ping=/audit>go</a>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, source_context, dom, RealmId(257), 1).unwrap();
            history(scope, &state);
            state.set_document_provenance_capture(Rc::new(|_, _, _| {
                Ok(oryn_dom::DocumentProvenance {
                    capability: vec![1],
                    origin: "https://example.test".into(),
                    origin_key: "tuple:https://example.test".into(),
                    inert: false,
                })
            }));
            state.set_navigation_target_selector(select);
            state.set_hyperlink_audit_preparer(prepare_audit);
            AUDIT.with(|log| log.borrow_mut().clear());
            state
        };
        let destination_context = v8::Context::new(scope, Default::default());
        let destination = {
            let scope = &mut v8::ContextScope::new(scope, destination_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>destination</body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, destination_context, dom, RealmId(258), 1)
                    .unwrap();
            history(scope, &state);
            state.set_document_provenance_capture(Rc::new(|_, _, _| {
                Ok(oryn_dom::DocumentProvenance {
                    capability: vec![2],
                    origin: "https://example.test".into(),
                    origin_key: "tuple:https://example.test".into(),
                    inert: false,
                })
            }));
            let source_global = source.global_object.to_local(scope).unwrap();
            let callback = v8::Function::builder(retire_source)
                .data(source_global.into())
                .build(scope)
                .unwrap();
            let key = v8::String::new(scope, "retireSource").unwrap();
            destination_context
                .global(scope)
                .set(scope, key.into(), callback.into());
            eval(
                scope,
                "navigation.addEventListener('navigate',()=>retireSource())",
            );
            state
        };
        let scope = &mut v8::ContextScope::new(scope, source_context);
        let node = source.dom.borrow().find_by_id("a").unwrap();
        let mut job = LinkActivation::new(scope, &source, node, true, None, None)
            .unwrap()
            .unwrap();
        assert!(job.poll(scope).unwrap());
        assert!(source.execution_retired());
        AUDIT.with(|log| assert_eq!(log.borrow().last().map(String::as_str), Some("accepted")));
        assert!(source.pending_default_actions.borrow().is_empty());
        assert_eq!(destination.reserved_default_actions.get(), 0);
        let actions = destination.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let PendingDefaultAction::Click(target) = actions.front().unwrap() else {
            panic!("missing accepted link")
        };
        let admission = target
            .navigation
            .as_ref()
            .unwrap()
            .admission
            .as_ref()
            .unwrap();
        assert_eq!(admission.realm, RealmId(258));
        assert!(admission.operation.as_ref().is_some_and(|token| {
            crate::navigation_api::native_navigation_matches(&destination, token)
        }));
    }
    #[test]
    fn accepted_admission_is_not_repeated_after_queue_publication_failure() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<a id=a href=/b>go</a>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(255), 1).unwrap();
        history(scope, &state);
        eval(
            scope,
            "globalThis.navigations=0;navigation.addEventListener('navigate',()=>navigations++)",
        );
        let node = state.dom.borrow().find_by_id("a").unwrap();
        let mut job = LinkActivation::new(scope, &state, node, true, None, None)
            .unwrap()
            .unwrap();
        FAIL_PUBLICATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(job.retry_safe());
        assert_eq!(state.pending_default_actions.borrow().len(), 1);
        eval(
            scope,
            "a.href='/changed';a.target='_blank';if(navigations!==1)throw Error('missing original admission')",
        );
        assert!(job.poll(scope).unwrap());
        assert!(job.poll(scope).unwrap());
        eval(scope, "if(navigations!==1)throw Error('admission replay')");
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let PendingDefaultAction::Click(target) = actions.front().unwrap() else {
            panic!("missing link")
        };
        let navigation = target.navigation.as_ref().unwrap();
        assert_eq!(navigation.url, "https://example.test/b");
        assert!(navigation.admission.is_some());
    }
    #[test]
    fn interrupted_navigate_tail_never_reissues_canceled_admission() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<a id=a href=/b>go</a>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(256), 1).unwrap();
        history(scope, &state);
        eval(
            scope,
            "globalThis.trace=[];navigation.addEventListener('navigate',()=>trace.push('first'));navigation.addEventListener('navigate',()=>trace.push('tail'))",
        );
        let node = state.dom.borrow().find_by_id("a").unwrap();
        let mut job = LinkActivation::new(scope, &state, node, true, None, None)
            .unwrap()
            .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(!job.retry_safe());
        assert!(job.poll(scope).is_err());
        std::thread::sleep(
            crate::navigation_api::publication_delay(&state).unwrap_or_default()
                + Duration::from_millis(1),
        );
        crate::navigation_api::poll_publications(scope, &state).unwrap();
        eval(
            scope,
            "if(trace.join()!=='first,tail')throw Error('canceled admission replay')",
        );
        assert!(state.pending_default_actions.borrow().is_empty());
    }
}

#[cfg(test)]
mod download_reflection_tests {
    use super::*;
    #[test]
    fn download_reflects_dom_string_for_anchor_and_area_without_expando_or_partial_conversion() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(super::super::super::test_eval::eval(scope,r#"(()=>{
            for(const tag of ['a','area']){
                const element=document.createElement(tag);
                if(element.download!==''||element.hasAttribute('download'))throw Error('missing default');
                for(const value of ['report.txt','','\ud800','\udc00','\u0000']){
                    element.download=value;
                    if(element.download!==value||element.getAttribute('download')!==value||Object.hasOwn(element,'download'))throw Error('reflection');
                }
                let calls=0;element.download={toString(){calls++;return 'converted'}};
                if(calls!==1||element.getAttribute('download')!=='converted')throw Error('conversion once');
                const reason={};try{element.download={toString(){throw reason}};throw Error('accepted')}catch(error){if(error!==reason)throw error}
                if(element.download!=='converted')throw Error('partial mutation');
                let rejected=false;try{element.download=Symbol()}catch(error){rejected=error instanceof TypeError}
                if(!rejected||element.download!=='converted')throw Error('symbol conversion');
                element.download=null;if(element.download!=='null')throw Error('DOMString null');
                element.removeAttribute('download');if(element.download!=='')throw Error('removed default');
            }
            return true;
        })()"#).is_true());
    }
}
