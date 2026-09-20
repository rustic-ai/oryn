
/// Inputs retained together for create prerender root.
pub(crate) struct PrerenderRootInput {
    pub(crate) previous: RealmId,
    pub(crate) broker: Option<SharedNetworkBroker>,
    pub(crate) client: SharedContextClient,
    pub(crate) page_id: PageId,
    pub(crate) token: oryn_js::message_wire::PrerenderToken,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
}
// Same-agent nonactive root preparation. The candidate gets its own context;
// preloading must never detach the incumbent's public WindowProxy.
pub(crate) struct NativePrerenderActivationTree{
    route:NativePageTransition,
    events:std::collections::VecDeque<oryn_js::v8_bindings::NativePrerenderActivation>,
    canceled:bool,
}
impl NativePrerenderActivationTree{
    pub(crate) fn canceled(&self)->bool{self.canceled}
    pub(crate) fn cancel(&mut self){for event in &mut self.events{event.cancel();}self.events.clear();self.route.recipients.clear();self.canceled=true;}
}
impl RawV8Host {
    /// Read an already-owned predecessor lease without parking twice. No
    /// callback or parent message occurs while the activation owner is built.
    pub(crate) fn parked_prerender_predecessor(&self,realm:RealmId)->Result<oryn_js::message_wire::ParkedDocumentToken,HostError>{
        self.parked_documents.get(&realm).map(|record|record.token.clone())
            .ok_or_else(||HostError::Execution("prerender predecessor is not parked".into()))
    }
    pub(crate) fn prerender_source_snapshot(&self,realm:RealmId)->Result<Option<super::PrerenderSourceSnapshot>,HostError>{
        let Some(state)=self.binding_states.get(&realm)else{return Ok(None)};
        if state.execution_retired()||state.execution_suspended()||state.document_prerendering()
            ||self.realm_descriptor(realm).is_none_or(|descriptor|descriptor.frame_id.is_some()){
            return Ok(None)
        }
        let Some(endpoint)=self.context_endpoint_for_realm(realm)else{return Ok(None)};
        if self.navigation_roots.get(&endpoint.page_id).is_some_and(|current|*current!=realm){return Ok(None)}
        let candidates=state.native_prerender_candidates();
        let creator=state.native_worker_creator().map_err(|error|HostError::Execution(error.to_string()))?;
        Ok(Some(super::PrerenderSourceSnapshot{realm,
            identity:(state.document_root(),state.document_generation(),state.document_stream_epoch(),state.document_authority_epoch()),creator,candidates}))
    }

    pub(crate) fn prerender_retirement_documents(&mut self,root:RealmId)->Result<std::collections::VecDeque<oryn_js::message_wire::MessageRequest>,HostError>{
        let snapshot=self.prepare_native_page_transition(root,"pagehide",false)?;
        let mut requests=std::collections::VecDeque::new();
        for recipient in snapshot.recipients.iter().rev(){
            let creator=recipient.owner.native_worker_creator().map_err(|error|HostError::Execution(error.to_string()))?;
            let oryn_js::worker_wire::Creator::Window{target,document_generation,stream_epoch}=creator else{return Err(HostError::Execution("prerender tree contains a non-Window owner".into()))};
            requests.push_back(match target{oryn_js::message_wire::WindowTarget::Root=>oryn_js::message_wire::MessageRequest::RetireRootDocument{document_generation,stream_epoch},oryn_js::message_wire::WindowTarget::Frame(frame)=>oryn_js::message_wire::MessageRequest::RetireWindow{frame,document_generation,stream_epoch}});
        }
        Ok(requests)
    }

    pub(crate) fn check_prerender_activation_authority(&self,realm:RealmId,generation:u64,epoch:u64)->Result<(),HostError>{
        let endpoint=self.context_endpoint_for_realm(realm).ok_or_else(||HostError::Execution("prerender activation endpoint missing".into()))?;
        let state=self.binding_states.get(&realm).ok_or_else(||HostError::Execution("prerender activation document missing".into()))?;
        if state.execution_retired()||(state.document_generation(),state.document_authority_epoch())!=(generation,epoch){return Err(HostError::Execution("prerender activation document replaced".into()))}
        match endpoint.client.request(ContextServiceRequest::ReadNativeWindowOrigin{source_page_id:endpoint.page_id,creator:oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Root,document_generation:generation,stream_epoch:epoch},target_page_id:endpoint.page_id,target:oryn_js::message_wire::WindowTarget::Root,expected:None}){
            Ok(ContextServiceResponse::SecurityOrigin{..})=>Ok(()),
            Ok(_)=>Err(HostError::Execution("prerender activation authority acknowledgement mismatch".into())),
            Err(error)=>Err(HostError::Execution(error.to_string())),
        }
    }

    /// Called only after the actor receives the exact parent activation ACK.
    /// A prerender moves its browsing context with its history entry, so its
    /// existing proxy remains attached; this is not BFCache proxy reattachment.
    pub(crate) fn publish_prerender_root(&mut self,previous:RealmId,realm:RealmId,token:&oryn_js::message_wire::PrerenderToken,generation:u64,epoch:u64)->Result<(),HostError>{
        self.check_prerender_activation_authority(realm,generation,epoch)?;
        let endpoint=self.context_endpoint_for_realm(realm).ok_or_else(||HostError::Execution("prerender publication endpoint absent".into()))?;
        let state=self.binding_states.get(&realm).ok_or_else(||HostError::Execution("prerender publication owner absent".into()))?;
        if state.execution_retired()||state.document_generation()!=generation||state.document_authority_epoch()!=epoch{
            return Err(HostError::Execution("prerender publication owner changed".into()))
        }
        let context=self.contexts.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender publication context absent".into()))?;
        {v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,&context);
            if context.get_slot::<NativePrerenderRoot>().is_none_or(|record|record.token!=*token||record.epoch!=epoch){return Err(HostError::Execution("prerender publication capability mismatch".into()))}
        }
        let current=self.navigation_roots.get(&endpoint.page_id).copied().unwrap_or(previous);
        if current==realm{return Ok(())}
        if current!=previous||!self.parked_documents.contains_key(&previous)
            ||self.context_endpoint_for_realm(previous).is_none_or(|source|source.page_id!=endpoint.page_id){
            return Err(HostError::Execution("prerender incumbent departure changed".into()))
        }
        let previous_context=self.contexts.get(&previous).cloned().ok_or_else(||HostError::Execution("prerender predecessor context absent".into()))?;
        {v8::scope!(let scope,&mut self.isolate);let previous_context=v8::Local::new(scope,&previous_context);let context=v8::Local::new(scope,&context);
            if previous_context.get_slot::<NativeRootProxyLineage>().is_none(){previous_context.set_slot(Rc::new(NativeRootProxyLineage(previous)));}
            context.set_slot(Rc::new(NativeRootProxyLineage(realm)));
        }
        self.navigation_roots.insert(endpoint.page_id,realm);
        self.publish_current_auxiliary_root(endpoint.page_id,realm);self.active_realm=realm;Ok(())
    }

    pub(crate) fn prepare_prerender_activation(&mut self,realm:RealmId)->Result<NativePrerenderActivationTree,HostError>{
        let route=self.prepare_native_page_transition(realm,"pageshow",false)?;
        let mut events=std::collections::VecDeque::new();
        for recipient in &route.recipients{events.push_back(self.prepare_prerender_document_activation(recipient.realm)?);}
        Ok(NativePrerenderActivationTree{route,events,canceled:false})
    }
    pub(crate) fn resume_prerender_activation(&mut self,tree:&mut NativePrerenderActivationTree)->Result<bool,HostError>{
        while let Some(recipient)=tree.route.recipients.front(){
            let root=&tree.route;
            if self.automation_realm_identity(root.root)!=Some(root.identity)||root.owner.document_authority_epoch()!=root.authority{
                tree.cancel();return Ok(true)
            }
            let live=self.automation_realm_identity(recipient.realm)==Some(recipient.identity)
                &&recipient.owner.document_authority_epoch()==recipient.authority
                &&recipient.ancestors.iter().all(|(realm,identity,frame,child,owner,authority)|self.automation_realm_identity(*realm)==Some(*identity)&&owner.document_authority_epoch()==*authority&&self.automation_frame_child_live(*realm,*frame,*child));
            if !live{
                if let Some(mut event)=tree.events.pop_front(){event.cancel();}
                tree.route.recipients.pop_front();continue
            }
            let event=tree.events.front_mut().ok_or_else(||HostError::Execution("prerender event route mismatch".into()))?;
            if !self.resume_prerender_document_activation(event)?{return Ok(false)}
            if event.canceled()&&recipient.realm==tree.route.root{tree.cancel();return Ok(true)}
            // Keep the whole activated forest blocked between document events;
            // an already-completed ancestor must not reveal before descendants.
            for(owner,identity,authority)in &tree.route.reveal_targets{
                if !owner.execution_retired()&&(owner.document_root(),owner.document_generation(),owner.document_stream_epoch(),true)==*identity&&owner.document_authority_epoch()==*authority{owner.prepare_history_reveal();}
            }
            tree.events.pop_front();tree.route.recipients.pop_front();
        }
        for(owner,identity,authority)in tree.route.reveal_targets.drain(..){
            if !owner.execution_retired()&&(owner.document_root(),owner.document_generation(),owner.document_stream_epoch(),true)==identity&&owner.document_authority_epoch()==authority{owner.complete_history_reveal();}
        }
        Ok(true)
    }

    fn prepare_prerender_document_activation(&mut self,realm:RealmId)->Result<oryn_js::v8_bindings::NativePrerenderActivation,HostError>{
        let state=self.binding_states.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender activation document missing".into()))?;
        let context=self.contexts.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender activation context missing".into()))?;
        v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,&context);let scope=&mut v8::ContextScope::new(scope,context);
        oryn_js::v8_bindings::NativePrerenderActivation::new(scope,&state).map_err(|error|HostError::Execution(error.to_string()))
    }
    fn resume_prerender_document_activation(&mut self,receipt:&mut oryn_js::v8_bindings::NativePrerenderActivation)->Result<bool,HostError>{
        let owner=receipt.owner();
        let realm=self.binding_states.iter().find(|(_,state)|Rc::ptr_eq(state,&owner)).map(|(realm,_)|*realm);
        let Some(realm)=realm else{receipt.cancel();return Ok(true)};
        if !receipt.retry_safe(){receipt.cancel();return Ok(true)}
        let context=self.contexts.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender activation context missing".into()))?;
        self.active_realm=realm;
        v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,&context);let scope=&mut v8::ContextScope::new(scope,context);
        receipt.poll(scope).map_err(|error|HostError::Execution(error.to_string()))
    }

    pub(crate) fn prerender_activation_available(&self,realm:RealmId)->bool{self.binding_states.get(&realm).is_some_and(|state|!state.native_document_lifecycle_pending())}
    pub(crate) fn active_document_prerendering(&self)->bool{
        self.binding_states.get(&self.active_realm).is_some_and(|state|state.document_prerendering())
    }

    pub(crate) fn create_prerender_root(&mut self, params: PrerenderRootInput)->Result<RealmId,HostError>{
        let PrerenderRootInput { previous, broker, client, page_id, token, generation, epoch } = params;

        let response=client.request(ContextServiceRequest::ReadPrerenderAuthority{page_id,token:token.clone(),generation,epoch})
            .map_err(|error|HostError::Execution(error.to_string()))?;
        let ContextServiceResponse::PrerenderAuthority{initiator_page,url,policy:_,origin}=response else{return Err(HostError::Execution("prerender authority response mismatch".into()))};
        // A blank candidate already owns an authenticated hidden destination.
        // Its accepted load can construct after the source realm was retired.
        // Same-page candidates still require the exact incumbent proxy owner.
        if initiator_page==page_id&&(self.context_endpoint_for_realm(previous).is_none_or(|endpoint|endpoint.page_id!=initiator_page)||self.realm_descriptor(previous).is_none_or(|realm|realm.frame_id.is_some())||self.navigation_roots.get(&initiator_page).is_some_and(|current|*current!=previous)){return Err(HostError::Execution("prerender initiator host mismatch".into()))}
        let origin=Origin::from_native_authority(&origin.serialized,origin.opaque.then_some(origin.authority).as_deref()).map_err(|error|HostError::Execution(error.into()))?;
        let base_url=Url::parse(&url).map_err(|error|HostError::Execution(error.to_string()))?;
        let secure_context=oryn_js::realm::potentially_trustworthy_origin(&Origin::from_url(&url));
        self.isolate.get_slot::<NativeRealmIdentities>().unwrap().realms.ensure_capacity().map_err(|_|HostError::LimitExceeded{resource:"realm identities".into()})?;
        let fetch=broker.map(|broker|FetchState{broker,base_url,next_script_request:Rc::new(std::cell::Cell::new(900_000))});
        let(realm,context)={
            v8::scope!(let scope,&mut self.isolate);
            let(realm,context)=create_native_realm_context(scope,origin,generation,None,None,RealmEndpoints{fetch,context:Some(ContextState{client,page_id}),activity_parent:None})?;
            let local=v8::Local::new(scope,&context);
            local.set_slot(Rc::new(NativeRootGeneration(generation)));
            local.set_slot(Rc::new(NativeInitialDocument(false)));
            local.set_slot(Rc::new(oryn_js::realm::SecureContextState(secure_context)));
            local.set_slot(Rc::new(NativePrerenderRoot{token,epoch}));
            (realm,context)
        };
        self.contexts.insert(realm,context);
        self.isolate.get_slot_mut::<NativeRealmIdentities>().unwrap().pending_contexts.remove(&realm);
        // The construction driver enters this context explicitly. The native
        // target registry continues pointing at the initiating browsing context.
        self.active_realm=realm;Ok(realm)
    }

    pub(crate) fn prerender_root_current(&mut self,realm:RealmId)->Result<bool,HostError>{
        let endpoint=self.context_endpoint_for_realm(realm).ok_or_else(||HostError::Execution("prerender endpoint missing".into()))?;
        let state=self.binding_states.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender document missing".into()))?;
        if state.execution_retired()||!state.document_prerendering(){return Ok(false)}
        let context=self.contexts.get(&realm).cloned().ok_or_else(||HostError::Execution("prerender context missing".into()))?;
        let record={v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,&context);context.get_slot::<NativePrerenderRoot>()};
        let Some(record)=record else{return Ok(false)};
        match endpoint.client.request(ContextServiceRequest::ReadPrerenderAuthority{page_id:endpoint.page_id,token:record.token.clone(),generation:state.document_generation(),epoch:record.epoch}){
            Ok(ContextServiceResponse::PrerenderAuthority{..})=>Ok(true),
            Ok(_)=>Err(HostError::Execution("prerender authority response mismatch".into())),
            Err(crate::runtime::RuntimeError::NavigationCancelled|crate::runtime::RuntimeError::PageNotFound(_))=>Ok(false),
            Err(error)=>Err(HostError::Execution(error.to_string())),
        }
    }
}

#[cfg(all(test,feature="in-process-probe"))]
mod prerender_host_tests {
    use super::*;
    #[test]fn activation_and_cross_group_back_restore_keep_both_original_globals(){
        let context=crate::Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        context.install_document_security(page,"https://prerender.test/start".into(),&[]).unwrap();
        let client=SharedContextClient::direct(context.clone(),page);
        let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,Some(Url::parse("https://prerender.test/start").unwrap()),client.clone(),page,128*1024*1024).unwrap();
        let source=host.active_realm();let generation=host.realm_descriptor(source).unwrap().document_generation;
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>source</body>").into_runtime().unwrap())),generation).unwrap();
        host.execute("globalThis.savedWindow=window;globalThis.savedDocument=document;globalThis.marker='source'").unwrap();
        let mut session=host.navigation_history_snapshot().unwrap();
        host.sync_session_history(session.clone()).unwrap();
        let creator=host.native_script_creator().unwrap();
        let ContextServiceResponse::PrerenderReserved{token,generation,epoch}=client.request(ContextServiceRequest::ReservePrerender{page_id:page,creator,url:"https://prerender.test/next".into()}).unwrap()else{panic!("reserve")};
        client.request(ContextServiceRequest::CommitPrerenderResponse{response_policy:None,page_id:page,token:token.clone(),generation,epoch,url:"https://prerender.test/next".into(),headers:Vec::new()}).unwrap();
        let candidate=host.create_prerender_root(PrerenderRootInput { previous: source, broker: None, client: client.clone(), page_id: page, token: token.clone(), generation, epoch }).unwrap();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>candidate</body>").into_runtime().unwrap())),generation).unwrap();
        host.execute("globalThis.savedWindow=window;globalThis.savedDocument=document;globalThis.marker='candidate';globalThis.order=[];addEventListener('pageshow',()=>order.push('pageshow'));document.addEventListener('prerenderingchange',()=>order.push('prerenderingchange'))").unwrap();
        // Public discovery must retain the incumbent while the hidden candidate
        // has an independently scoped local tree. Neither checkpoint may publish
        // two roots under one page identity or replace the active native root.
        host.run_dom_observer_checkpoint().unwrap();
        {
            v8::scope!(let scope,&mut host.isolate);
            let context=v8::Local::new(scope,host.contexts.get(&candidate).unwrap());
            let scope=&mut v8::ContextScope::new(scope,context);
            let inventory=local_navigation_inventory(scope).unwrap();
            assert_eq!(inventory[&page].iter().map(|(_,entry)|entry.realm).collect::<Vec<_>>(),vec![candidate]);
        }
        host.activate_realm(source).unwrap();
        host.run_dom_observer_checkpoint().unwrap();
        {
            v8::scope!(let scope,&mut host.isolate);
            let context=v8::Local::new(scope,host.contexts.get(&source).unwrap());
            let scope=&mut v8::ContextScope::new(scope,context);
            let inventory=local_navigation_inventory(scope).unwrap();
            assert_eq!(inventory[&page].iter().map(|(_,entry)|entry.realm).collect::<Vec<_>>(),vec![source]);
        }
        assert!(matches!(client.request(ContextServiceRequest::ReadNativeWindowOrigin{
            source_page_id:page,creator:host.native_creator_for_realm(source).unwrap(),
            target_page_id:page,target:oryn_js::message_wire::WindowTarget::Root,expected:None,
        }).unwrap(),ContextServiceResponse::SecurityOrigin{..}));
        host.activate_realm(candidate).unwrap();
        let mut show=host.prepare_native_page_transition(candidate,"pageshow",false).unwrap();assert!(host.resume_native_page_transition(&mut show).unwrap());
        client.request(ContextServiceRequest::MarkPrerenderLoaded{page_id:page,token:token.clone(),generation,epoch}).unwrap();
        let mut activation=host.prepare_prerender_activation(candidate).unwrap();
        let predecessor=host.park_document_authority(source).unwrap().unwrap();
        assert!(matches!(client.request(ContextServiceRequest::ActivatePrerender{page_id:page,token:token.clone(),generation,epoch,predecessor}).unwrap(),ContextServiceResponse::PrerenderActivated));
        host.publish_prerender_root(source,candidate,&token,generation,epoch).unwrap();
        // The real same-page actor supplies the complete source+destination
        // session before activation, independently of Navigation visibility.
        let candidate_history=host.navigation_history_snapshot().unwrap();
        session.entries.extend(candidate_history.entries);session.current=session.entries.len()-1;
        session.navigation_type="push".into();session.truncates_forward=true;
        host.sync_session_history(session).unwrap();
        assert!(host.resume_prerender_activation(&mut activation).unwrap());assert!(!activation.canceled());
        assert_eq!(host.evaluate_string("String(history.length)").unwrap(),"2");
        assert_eq!(host.evaluate_string("String(savedWindow===window&&savedDocument===document&&marker==='candidate'&&!document.prerendering&&order.join(',')==='pageshow,prerenderingchange')").unwrap(),"true");
        host.park_document_authority(candidate).unwrap().unwrap();
        host.restore_document_authority(source).unwrap();host.restore_navigation_root_proxy(source).unwrap();host.activate_realm(source).unwrap();
        assert_eq!(host.evaluate_string("String(savedWindow===window&&savedDocument===document&&marker==='source')").unwrap(),"true");
        host.park_document_authority(source).unwrap().unwrap();
        host.restore_document_authority(candidate).unwrap();host.restore_navigation_root_proxy(candidate).unwrap();host.activate_realm(candidate).unwrap();
        assert_eq!(host.evaluate_string("String(savedWindow===window&&savedDocument===document&&marker==='candidate'&&order.length===2)").unwrap(),"true");
    }
    #[test]
    fn private_http_child_policy_binding_preserves_incumbent_and_rejects_stale_authority() {
        use oryn_js::worker_wire::Creator;
        use oryn_js::message_wire::WindowTarget;
        let context = crate::Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        context.install_document_security(page, "https://prerender.test/source".into(), &[]).unwrap();
        let client = SharedContextClient::direct(context.clone(), page);
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None, Some(Url::parse("https://prerender.test/source").unwrap()), client.clone(), page, 128*1024*1024,
        ).unwrap();
        let source = host.active_realm();
        assert_eq!(host.navigation_roots.get(&page), Some(&source));
        assert_eq!(host.navigation_root_for_page(page), Some(source));
        let source_generation = host.realm_descriptor(source).unwrap().document_generation;
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>source</body>").into_runtime().unwrap())), source_generation).unwrap();
        host.execute("globalThis.originalDocument=document").unwrap();
        let source_creator = host.native_script_creator().unwrap();
        let ContextServiceResponse::PrerenderReserved { token, generation, epoch } = client.request(
            ContextServiceRequest::ReservePrerender { page_id: page, creator: source_creator.clone(), url: "https://prerender.test/candidate".into() },
        ).unwrap() else { panic!("private reservation"); };
        client.request(ContextServiceRequest::CommitPrerenderResponse {
            response_policy: None,            page_id: page, token: token.clone(), generation, epoch, url: "https://prerender.test/candidate".into(), headers: Vec::new(),
        }).unwrap();
        let candidate = host.create_prerender_root(PrerenderRootInput { previous: source, broker: None, client: client.clone(), page_id: page, token: token.clone(), generation, epoch }).unwrap();
        let parent_dom = Rc::new(RefCell::new(oryn_dom::html::parse("<body><iframe id='child' src='https://child.test/response'></iframe></body>").into_runtime().unwrap()));
        host.install_dom(parent_dom.clone(), generation).unwrap();
        host.commit_location("https://prerender.test/candidate").unwrap();
        let frame = parent_dom.borrow().find_by_id("child").unwrap();
        host.ensure_provisional_frame_window(frame).unwrap();
        let candidate_creator = host.native_creator_for_realm(candidate).unwrap();
        let response_url = "https://child.test/response";
        // Exercise the real response-policy ticket and native binding path
        // used after an HTTP response; no local/about:srcdoc inheritance.
        let ticket = context.prepare_frame_worker_policy_for_creator(
            page, response_url, &[("content-security-policy".into(), "img-src 'none'".into())], "", Some(&candidate_creator),
        ).unwrap();
        let child_dom = Rc::new(RefCell::new(oryn_dom::html::parse("<title>HTTP child</title><body>response</body>").into_runtime().unwrap()));
        let child_document = child_dom.borrow().document();
        let child = host.commit_frame_document_with_url(frame, Origin::from_url(response_url), child_dom, child_document, Some(response_url)).unwrap();
        host.activate_realm(child).unwrap();
        host.commit_location(response_url).unwrap();
        // This must already be private before policy binding or any author
        // read; setting the flag after publication would hide the actual bug.
        assert!(host.binding_states[&child].document_prerendering());
        assert!(!host.binding_states[&child].document_rendering_allowed());
        let prepared_before = context.prepared_frame_resource_policy(page, &ticket).unwrap();
        assert!(host.bind_frame_worker_policy("https://child.test/wrong", &ticket).is_err());
        assert_eq!(context.prepared_frame_resource_policy(page, &ticket).unwrap(), prepared_before);
        assert!(host.prerender_root_current(candidate).unwrap());
        assert_eq!(host.navigation_roots.get(&page), Some(&source));
        host.bind_frame_worker_policy(response_url, &ticket).unwrap();
        let child_creator = host.native_creator_for_realm(child).unwrap();
        let Creator::Window { target: WindowTarget::Frame(frame_token), document_generation, stream_epoch } = &child_creator else { panic!("private frame creator"); };
        let (_, _, origin) = context.window_worker_creator(page, &WindowTarget::Frame(frame_token.clone()), *document_generation, *stream_epoch).unwrap();
        assert_eq!(origin.serialized, "https://child.test");
        assert!(context.read_related_window(page, &source_creator, page,
            &WindowTarget::Frame(frame_token.clone()), None).is_err());
        let policy = context.window_worker_policy(page, &WindowTarget::Frame(frame_token.clone()), *document_generation, *stream_epoch).unwrap();
        assert_eq!(policy.url, response_url);
        assert!(policy.headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("content-security-policy") && value == "img-src 'none'"));
        assert!(host.binding_states[&child].document_prerendering());
        assert_eq!(host.evaluate_string("String(document.title==='HTTP child'&&document.prerendering)").unwrap(), "true");
        assert_eq!(host.navigation_roots.get(&page), Some(&source));
        assert!(context.window_worker_creator(page, &WindowTarget::Root, source_generation, 0).is_ok());
        let stale_ticket = context.prepare_frame_worker_policy_for_creator(page, response_url, &[], "", Some(&candidate_creator)).unwrap();
        client.request(ContextServiceRequest::CancelPrerender { page_id: page, token, generation, epoch }).unwrap();
        assert!(host.bind_frame_worker_policy(response_url, &stale_ticket).is_err());
        assert!(context.window_worker_creator(page, &WindowTarget::Frame(frame_token.clone()), *document_generation, *stream_epoch).is_err());
        host.retire_agent_root(candidate).unwrap();
        host.activate_realm(source).unwrap();
        assert_eq!(host.evaluate_string("String(originalDocument===document)").unwrap(), "true");
        assert_eq!(host.native_creator_for_realm(source).unwrap(), source_creator);
    }
    #[test]
    fn http_child_replacement_under_public_parent_keeps_public_discovery() {
        use oryn_js::worker_wire::Creator;
        use oryn_js::message_wire::WindowTarget;
        let context = crate::Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        let parent_url = "https://public-frame.test/source";
        let response_url = "https://child.test/response";
        context.install_document_security(page, parent_url.into(), &[]).unwrap();
        let client = SharedContextClient::direct(context.clone(), page);
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None, Some(Url::parse(parent_url).unwrap()), client, page, 128*1024*1024,
        ).unwrap();
        let parent = host.active_realm();
        assert_eq!(host.navigation_roots.get(&page), Some(&parent));
        assert_eq!(host.navigation_root_for_page(page), Some(parent));
        let generation = host.realm_descriptor(parent).unwrap().document_generation;
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse(
            "<body><iframe id='child' src='https://child.test/response'></iframe></body>",
        ).into_runtime().unwrap()));
        host.install_dom(dom.clone(), generation).unwrap();
        let parent_creator = host.native_creator_for_realm(parent).unwrap();
        let frame = dom.borrow().find_by_id("child").unwrap();
        host.ensure_provisional_frame_window(frame).unwrap();
        let ticket = context.prepare_frame_worker_policy_for_creator(
            page, response_url, &[], "", Some(&parent_creator),
        ).unwrap();
        let child_dom = Rc::new(RefCell::new(oryn_dom::html::parse(
            "<title>public child</title><body>response</body>",
        ).into_runtime().unwrap()));
        let document = child_dom.borrow().document();
        let child = host.commit_frame_document_with_url(
            frame, Origin::from_url(response_url), child_dom, document, Some(response_url),
        ).unwrap();
        assert!(!host.binding_states[&child].document_prerendering());
        host.activate_realm(child).unwrap();
        host.commit_location(response_url).unwrap();
        host.bind_frame_worker_policy(response_url, &ticket).unwrap();
        let creator = host.native_creator_for_realm(child).unwrap();
        let Creator::Window { target, .. } = &creator else { panic!("public frame creator"); };
        assert!(matches!(target, WindowTarget::Frame(_)));
        assert_eq!(context.read_related_window(page, &parent_creator, page, target, None).unwrap().creator, creator);
        assert_eq!(host.evaluate_string("String(document.title==='public child'&&!document.prerendering)").unwrap(), "true");
        assert_eq!(host.navigation_roots.get(&page), Some(&parent));
        host.activate_realm(parent).unwrap();
        assert_eq!(host.native_creator_for_realm(parent).unwrap(), parent_creator);
    }
    #[test]fn private_root_preparation_never_replaces_live_proxy_and_sets_identity_before_script(){
        let context=crate::Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        context.install_document_security(page,"https://prerender.test/start".into(),&[]).unwrap();
        let client=SharedContextClient::direct(context.clone(),page);
        let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,Some(Url::parse("https://prerender.test/start").unwrap()),client.clone(),page,128*1024*1024).unwrap();
        let previous=host.active_realm();let generation=host.realm_descriptor(previous).unwrap().document_generation;
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>source</body>").into_runtime().unwrap())),generation).unwrap();
        host.execute("globalThis.sourceIdentity=window;globalThis.originalDocument=document").unwrap();
        let creator=host.native_script_creator().unwrap();
        let ContextServiceResponse::PrerenderReserved{token,generation,epoch}=client.request(ContextServiceRequest::ReservePrerender{page_id:page,creator,url:"https://prerender.test/next".into()}).unwrap()else{panic!("reserve")};
        client.request(ContextServiceRequest::CommitPrerenderResponse{response_policy:None,page_id:page,token:token.clone(),generation,epoch,url:"https://prerender.test/next".into(),headers:Vec::new()}).unwrap();
        let candidate=host.create_prerender_root(PrerenderRootInput { previous, broker: None, client: client.clone(), page_id: page, token: token.clone(), generation, epoch }).unwrap();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>candidate</body>").into_runtime().unwrap())),generation).unwrap();
        let state=host.binding_states[&candidate].clone();assert!(state.document_prerendering());assert_eq!(state.document_authority_epoch(),epoch);
        assert!(host.prerender_root_current(candidate).unwrap());
        assert_ne!(host.navigation_roots.get(&page),Some(&candidate));
        host.activate_realm(previous).unwrap();host.execute("if(window!==sourceIdentity||document!==originalDocument)throw Error('initiator replaced')").unwrap();
        client.request(ContextServiceRequest::CancelPrerender{page_id:page,token,generation,epoch}).unwrap();
        assert!(!host.prerender_root_current(candidate).unwrap());
        host.retire_agent_root(candidate).unwrap();
    }
}

#[derive(Default)]
struct BlankPrerenderActivations(std::collections::VecDeque<(super::PrerenderSourceSnapshot,crate::context_service::BlankPrerenderActivation)>);
#[cfg(unix)]
pub(crate) fn reserve_blank_prerender_activation_slot(scope:&mut v8::PinScope)->Result<(),HostError>{
    if scope.get_slot::<BlankPrerenderActivations>().is_none(){scope.set_slot(BlankPrerenderActivations::default());}
    let queue=&mut scope.get_slot_mut::<BlankPrerenderActivations>().unwrap().0;
    if queue.len()>=16{return Err(HostError::LimitExceeded{resource:"blank prerender activations".into()})}
    queue.try_reserve(1).map_err(|_|HostError::LimitExceeded{resource:"blank prerender activation custody".into()})
}
#[cfg(unix)]
pub(crate) fn queue_blank_prerender_activation(scope:&mut v8::PinScope,receipt:crate::context_service::BlankPrerenderActivation)->Result<(),HostError>{
    let context=scope.get_current_context();
    let state=context.get_slot::<V8BindingState>().ok_or_else(||HostError::Execution("blank activation source bindings missing".into()))?;
    if state.execution_retired(){return Err(HostError::Execution("blank activation source retired".into()))}
    let snapshot=super::PrerenderSourceSnapshot{realm:state.realm(),identity:(state.document_root(),state.document_generation(),state.document_stream_epoch(),state.document_authority_epoch()),creator:state.native_worker_creator().map_err(|error|HostError::Execution(error.to_string()))?,candidates:state.native_prerender_candidates()};
    if scope.get_slot::<BlankPrerenderActivations>().is_some_and(|pending|pending.0.iter().any(|(_,old)|old.page_id==receipt.page_id&&old.token==receipt.token)){return Ok(())}
    reserve_blank_prerender_activation_slot(scope)?;
    let queue=&mut scope.get_slot_mut::<BlankPrerenderActivations>().unwrap().0;
    if queue.len()>=16{return Err(HostError::LimitExceeded{resource:"blank prerender activations".into()})}
    queue.push_back((snapshot,receipt));Ok(())
}
impl RawV8Host{
    pub(crate) fn take_blank_prerender_activations(&mut self)->Vec<(super::PrerenderSourceSnapshot,crate::context_service::BlankPrerenderActivation)>{self.isolate.get_slot_mut::<BlankPrerenderActivations>().map(|queue|queue.0.drain(..).collect()).unwrap_or_default()}
    pub(crate) fn pending_blank_prerender_source(&self,realm:RealmId)->Option<super::PrerenderSourceSnapshot>{self.isolate.get_slot::<BlankPrerenderActivations>()?.0.iter().find(|(source,_)|source.realm==realm).map(|(source,_)|source.clone())}
    pub(crate) fn publish_blank_prerender_root(&mut self,realm:RealmId,receipt:&crate::context_service::BlankPrerenderActivation)->Result<(),HostError>{
        self.check_prerender_activation_authority(realm,receipt.generation,receipt.epoch)?;
        let endpoint=self.context_endpoint_for_realm(realm).ok_or_else(||HostError::Execution("blank destination endpoint absent".into()))?;
        if endpoint.page_id!=receipt.page_id{return Err(HostError::Execution("blank destination page mismatch".into()))}
        let context=self.contexts.get(&realm).cloned().ok_or_else(||HostError::Execution("blank destination context absent".into()))?;
        {v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,&context);
            if context.get_slot::<NativePrerenderRoot>().is_none_or(|record|record.token!=receipt.token||record.epoch!=receipt.epoch){return Err(HostError::Execution("blank destination capability mismatch".into()))}
            context.set_slot(Rc::new(NativeRootProxyLineage(realm)));
        }
        if self.navigation_roots.get(&receipt.page_id).is_some_and(|current|*current!=realm){return Err(HostError::Execution("blank destination already materialized".into()))}
        self.navigation_roots.insert(receipt.page_id,realm);self.publish_current_auxiliary_root(receipt.page_id,realm);self.active_realm=realm;Ok(())
    }
}

#[cfg(all(test,feature="in-process-probe"))]
mod blank_prerender_host_tests{
    use super::*;
    #[test]
    fn blank_promotion_preserves_candidate_globals_and_keeps_source_active(){
        let context=crate::Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        context.install_document_security(page,"https://prerender.test/source".into(),&[]).unwrap();
        let client=SharedContextClient::direct(context.clone(),page);
        let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,Some(Url::parse("https://prerender.test/source").unwrap()),client.clone(),page,128*1024*1024).unwrap();
        let source=host.active_realm();let source_generation=host.realm_descriptor(source).unwrap().document_generation;
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>source</body>").into_runtime().unwrap())),source_generation).unwrap();
        host.execute("globalThis.saved=document;globalThis.hiddenCount=0;addEventListener('pagehide',()=>hiddenCount++)").unwrap();
        let creator=host.native_script_creator().unwrap();
        let ContextServiceResponse::BlankPrerenderReserved{page_id:destination,admission,token,generation,epoch}=client.request(ContextServiceRequest::ReserveBlankPrerender{page_id:page,creator:creator.clone(),url:"https://prerender.test/next".into()}).unwrap()else{panic!("blank reserve")};
        assert_ne!(destination,page);assert!(context.hidden_prerender_page(destination));
        let destination_client=client.related_endpoint(&admission).unwrap();
        destination_client.request(ContextServiceRequest::CommitPrerenderResponse{response_policy:None,page_id:destination,token:token.clone(),generation,epoch,url:"https://prerender.test/next".into(),headers:Vec::new()}).unwrap();
        let candidate=host.create_prerender_root(PrerenderRootInput { previous: source, broker: None, client: destination_client.clone(), page_id: destination, token: token.clone(), generation, epoch }).unwrap();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body>candidate</body>").into_runtime().unwrap())),generation).unwrap();
        host.execute("globalThis.saved=document;globalThis.savedWindow=window;globalThis.count=0;document.addEventListener('prerenderingchange',()=>count++)").unwrap();
        destination_client.request(ContextServiceRequest::MarkPrerenderReady{page_id:destination,token:token.clone(),generation,epoch}).unwrap();
        let receipt=context.claim_blank_prerender(page,&creator,None,"https://prerender.test/next").unwrap().unwrap();
        for _ in 0..2{assert!(matches!(client.request(ContextServiceRequest::ActivateBlankPrerender{page_id:page,activation:receipt.clone()}).unwrap(),ContextServiceResponse::PrerenderActivated));}
        host.publish_blank_prerender_root(candidate,&receipt).unwrap();host.publish_blank_prerender_root(candidate,&receipt).unwrap();
        let mut events=host.prepare_prerender_activation(candidate).unwrap();
        assert!(matches!(client.request(ContextServiceRequest::PublishBlankPrerender{page_id:page,activation:receipt.clone()}).unwrap(),ContextServiceResponse::PrerenderPublished));
        // A raw host bypasses actor adoption. Missing native full-session
        // state must block activation; it cannot be inferred from author reads.
        let missing=host.resume_prerender_activation(&mut events).unwrap_err();
        assert!(missing.to_string().contains("native full history projection unavailable"));
        assert_eq!(host.evaluate_string("String(count)").unwrap(),"0");
        assert!(!host.binding_states[&candidate].document_rendering_allowed());
        let session=host.navigation_history_snapshot().unwrap();assert_eq!(session.entries.len(),1);
        host.sync_session_history(session).unwrap();
        assert!(host.resume_prerender_activation(&mut events).unwrap());assert!(host.resume_prerender_activation(&mut events).unwrap());
        assert!(host.binding_states[&candidate].native_history_has_single_entry());
        assert_eq!(host.evaluate_string("String(saved===document&&savedWindow===window&&count===1&&!document.prerendering&&opener===null)").unwrap(),"true");
        host.activate_realm(source).unwrap();assert_eq!(host.evaluate_string("String(saved===document&&hiddenCount===0)").unwrap(),"true");
        assert!(!context.hidden_prerender_page(destination));
        client.request(ContextServiceRequest::CancelBlankPrerender{page_id:page,destination,token,generation,epoch}).unwrap();
        host.activate_realm(candidate).unwrap();assert_eq!(host.evaluate_string("String(saved===document)").unwrap(),"true");
    }
}
