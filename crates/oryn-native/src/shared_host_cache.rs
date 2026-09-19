// Cached drivers cannot release shared-host roots by dropping Rust ownership.
// Retain each exact driver until native retirement (including parent ACK) succeeds.
const MAX_RETAINED_CACHE_ROOTS:usize=32;
enum RetiredAgentDocument {
    Cached(BackForwardCacheEntry),
    Provisional(oryn_js::realm::RealmId),
}


impl PageState {
    pub(crate) fn matching_agent_host_count(&self,expected:&std::rc::Rc<()>)->usize{
        let matches=|identity:Option<std::rc::Rc<()>>|usize::from(identity.is_some_and(|identity|std::rc::Rc::ptr_eq(expected,&identity)));
        matches(self.executable.as_ref().and_then(|page|page.attached_agent_host_identity()))
        +matches(self.pending_agent_host.as_ref().map(|host|host.agent_host_identity()))
        +self.back_forward_cache.values().map(|entry|matches(entry.executable.attached_agent_host_identity())).sum::<usize>()
        +self.retired_agent_documents.iter().map(|entry|match entry{RetiredAgentDocument::Cached(entry)=>matches(entry.executable.attached_agent_host_identity()),_=>0}).sum::<usize>()
    }

    fn has_shared_root_driver(&self)->bool{
        // Ownership was selected when this executable was constructed. A
        // subsequently attached page-service transport does not merge isolates.
        if let Some(executable)=self.executable.as_ref(){return executable.uses_shared_agent_host()}
        if self.standalone_root.is_some(){return true}
        #[cfg(unix)] {self.context_client.is_some()}
        #[cfg(not(unix))] {false}
    }
    /// Recover the actor's exact host after any commit/rollback stage. Preflight
    /// all owners before moving custody, so ambiguity cannot drop an isolate.
    pub(crate) fn extract_agent_host(&mut self,expected:Option<&std::rc::Rc<()>>)->Result<Option<crate::host::RawV8Host>,RuntimeError>{
        enum Owner{Current,Pending,Cached(usize),Retired(usize)}
        let mut owners=Vec::new();
        let mut retain=|owner,identity:Option<std::rc::Rc<()>>|{
            if let Some(identity)=identity{if expected.is_none_or(|expected|std::rc::Rc::ptr_eq(expected,&identity)){owners.push(owner);}}
        };
        retain(Owner::Current,self.executable.as_ref().and_then(|page|page.attached_agent_host_identity()));
        retain(Owner::Pending,self.pending_agent_host.as_ref().map(|host|host.agent_host_identity()));
        for(index,entry)in &self.back_forward_cache{retain(Owner::Cached(*index),entry.executable.attached_agent_host_identity());}
        for(index,entry)in self.retired_agent_documents.iter().enumerate(){if let RetiredAgentDocument::Cached(entry)=entry{retain(Owner::Retired(index),entry.executable.attached_agent_host_identity());}}
        if owners.len()>1{return Err(RuntimeError::ProtocolViolation("ambiguous shared agent host custody".into()));}
        let host=match owners.pop(){
            None=>return Ok(None),
            Some(Owner::Current)=>self.executable.as_mut().unwrap().take_agent_host().map_err(map_host_error)?,
            Some(Owner::Pending)=>self.pending_agent_host.take().unwrap(),
            Some(Owner::Cached(index))=>self.back_forward_cache.get_mut(&index).unwrap().executable.take_agent_host().map_err(map_host_error)?,
            Some(Owner::Retired(index))=>match &mut self.retired_agent_documents[index]{RetiredAgentDocument::Cached(entry)=>entry.executable.take_agent_host().map_err(map_host_error)?,_=>unreachable!("preflight retained owner")},
        };
        Ok(Some(host))
    }

    fn ensure_cached_retirement_capacity(&self)->Result<(),RuntimeError>{
        if self.retired_agent_documents.len().saturating_add(self.back_forward_cache.len())>MAX_RETAINED_CACHE_ROOTS-2 {
            return Err(RuntimeError::WorkerLimitExceeded{resource:"retained cached document retirement".into()});
        }
        Ok(())
    }

    fn queue_cached_retirement(&mut self,entry:BackForwardCacheEntry){
        self.retired_agent_documents.push_back(RetiredAgentDocument::Cached(entry));
    }

    fn evict_cached_document(&mut self,index:usize){
        if let Some(entry)=self.back_forward_cache.remove(&index){
            self.queue_cached_retirement(entry);
        }
    }

    fn clear_cached_documents(&mut self){
        let entries=std::mem::take(&mut self.back_forward_cache);
        for entry in entries.into_values(){self.queue_cached_retirement(entry);}
    }

    fn retain_cached_documents_through(&mut self,index:usize){
        let removed=self.back_forward_cache.range((std::ops::Bound::Excluded(index),std::ops::Bound::Unbounded))
            .map(|(index,_)|*index).collect::<Vec<_>>();
        for index in removed{self.evict_cached_document(index);}
    }

    fn insert_cached_document(&mut self,index:usize,entry:BackForwardCacheEntry){
        if let Some(previous)=self.back_forward_cache.insert(index,entry){
            self.queue_cached_retirement(previous);
        }
    }

    pub(crate) fn enqueue_retired_agent_root(&mut self,realm:oryn_js::realm::RealmId){
        if !self.retired_agent_document_realms().contains(&realm){
            self.retired_agent_documents.push_back(RetiredAgentDocument::Provisional(realm));
        }
    }

    pub(crate) fn retired_agent_document_realms(&self)->Vec<oryn_js::realm::RealmId>{
        self.retired_agent_documents.iter().filter_map(|entry|match entry{
            RetiredAgentDocument::Cached(entry) if !entry.executable.has_agent_host()=>Some(entry.executable.agent_root_realm()),
            RetiredAgentDocument::Provisional(realm)=>Some(*realm),
            _=>None,
        }).collect()
    }

    pub(crate) fn has_retired_agent_documents(&self)->bool{!self.retired_agent_documents.is_empty()}

    pub(crate) fn drain_retired_agent_documents(&mut self,host:&mut crate::host::RawV8Host)->Result<(),RuntimeError>{
        let mut failure=None;
        // A failing oldest token must not starve other independent retirements.
        let count=self.retired_agent_documents.len().min(4);
        for _ in 0..count{
            let Some(mut entry)=self.retired_agent_documents.pop_front()else{break;};
            let result=match &mut entry{
                RetiredAgentDocument::Cached(entry) if entry.executable.has_agent_host()=>entry.executable.retire_owned_cached_document(),
                RetiredAgentDocument::Cached(entry)=>if self.standalone_root.is_some(){host.retire_standalone_root(entry.executable.agent_root_realm())}else{
                    #[cfg(unix)] {host.retire_agent_root(entry.executable.agent_root_realm())}
                    #[cfg(not(unix))] {host.retire_standalone_root(entry.executable.agent_root_realm())}
                },
                RetiredAgentDocument::Provisional(realm)=>if self.standalone_root.is_some(){host.retire_standalone_root(*realm)}else{
                    #[cfg(unix)] {host.retire_agent_root(*realm)}
                    #[cfg(not(unix))] {host.retire_standalone_root(*realm)}
                },
            };
            if let Err(error)=result{
                self.retired_agent_documents.push_back(entry);
                if failure.is_none(){failure=Some(map_host_error(error));}
            }
        }
        failure.map_or(Ok(()),Err)
    }
}

#[cfg(all(test,unix,feature="in-process-probe"))]
mod shared_cache_retirement_tests {
    use super::*;
    use crate::host::JavaScriptHost;

    fn load(page:&mut PageState,url:&str,source:&str,resources:&BTreeMap<String,String>)->Result<(),RuntimeError>{
        page.load_document_with_redirect_history(url.into(),source,None,None,HistoryUpdate::Push,resources,Vec::new(),false,Some(Vec::new()))
    }

    fn drain(page:&mut PageState){
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        page.drain_retired_agent_documents(&mut host).unwrap();
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
    }

    #[test]
    fn shared_navigation_cache_restore_and_eviction_keep_sibling_realm(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let client=crate::context_service::SharedContextClient::direct(context.clone(),id);
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p id=first>first</p><script>globalThis.kept=document</script>",&BTreeMap::new()).unwrap();
        assert!(page.executable.as_ref().unwrap().uses_shared_agent_host());
        assert!(page.has_shared_root_driver());
        let first=page.executable.as_ref().unwrap().agent_root_realm();
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        let sibling=host.create_realm("https://shared-cache.test",90,None).unwrap();host.activate_realm(sibling).unwrap();
        host.execute("let retainedSibling={value:81};globalThis.siblingIdentity=retainedSibling").unwrap();
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        let second=page.executable.as_ref().unwrap().agent_root_realm();assert_ne!(first,second);
        assert!(!page.back_forward_cache[&0].executable.has_agent_host());
        page.restore_cached_document(0,"https://shared-cache.test/first".into()).unwrap().unwrap();
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),first);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("kept===document").unwrap(),"true");
        page.evict_cached_document(1);assert!(page.retired_agent_document_realms().contains(&second));
        drain(&mut page);assert!(!page.has_retired_agent_documents());
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        assert!(host.realm_descriptor(second).is_none());assert!(host.realm_descriptor(first).is_some());
        host.activate_realm(sibling).unwrap();
        assert_eq!(host.evaluate_string("String(siblingIdentity===retainedSibling&&retainedSibling.value===81)").unwrap(),"true");
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
    }

    #[test]
    fn shared_constructor_failure_retains_incumbent_and_queues_provisional_root(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let client=crate::context_service::SharedContextClient::direct(context.clone(),id);
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p><script>globalThis.kept=document</script>",&BTreeMap::new()).unwrap();
        let first=page.executable.as_ref().unwrap().agent_root_realm();
        let resources=BTreeMap::from([("oryn:frame-history".into(),"invalid native snapshot".into())]);
        assert!(load(&mut page,"https://shared-cache.test/rejected","<p>rejected</p>",&resources).is_err());
        let deadline=Instant::now()+Duration::from_secs(3);
        while page.failed_install_recovery.is_some(){
            page.poll_failed_install_recovery();
            assert!(Instant::now()<deadline,"retained constructor recovery: {:?}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().as_ref().map(|error|error.chars().take(512).collect::<String>())));
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),first);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("kept===document").unwrap(),"true");
        assert_eq!(page.history.len(),1);
        let retired=page.retired_agent_document_realms();assert_eq!(retired.len(),1);assert_ne!(retired[0],first);
        drain(&mut page);assert!(!page.has_retired_agent_documents());
        load(&mut page,"https://shared-cache.test/retry","<p>retry</p>",&BTreeMap::new()).unwrap();
        assert_eq!(page.url,"https://shared-cache.test/retry");
    }
    struct FailOneDiscard {
        inner:crate::context_service::SharedContextClient,
        fail:std::sync::atomic::AtomicBool,
    }
    impl crate::context_service::ContextTransport for FailOneDiscard {
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:crate::context_service::ContextServiceRequest)
            ->Result<crate::context_service::ContextServiceResponse,RuntimeError>{self.request(request)}
        fn request(&self,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{
            if matches!(&request,crate::context_service::ContextServiceRequest::MessageWire{request:oryn_js::message_wire::MessageRequest::DiscardDocument{..},..})
                &&self.fail.swap(false,Ordering::SeqCst){return Err(RuntimeError::ContextService("injected retirement ACK failure".into()));}
            self.inner.request(request)
        }
    }

    #[test]
    fn failed_parent_retirement_keeps_cached_driver_and_native_root_until_retry(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let transport=Arc::new(FailOneDiscard{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),fail:std::sync::atomic::AtomicBool::new(true)});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport,crate::worker_route::WorkerRoute::issue(771,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p>",&BTreeMap::new()).unwrap();
        let first=page.executable.as_ref().unwrap().agent_root_realm();
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        page.evict_cached_document(0);
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        assert!(page.drain_retired_agent_documents(&mut host).is_err());
        assert_eq!(page.retired_agent_document_realms(),vec![first]);assert!(host.realm_descriptor(first).is_some());
        page.drain_retired_agent_documents(&mut host).unwrap();
        assert!(!page.has_retired_agent_documents());assert!(host.realm_descriptor(first).is_none());
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
    }

    struct PermitMalformedCommit {
        inner:crate::context_service::SharedContextClient,
        restored:Arc<std::sync::atomic::AtomicUsize>,
        reject_rollback:std::sync::atomic::AtomicBool,
    }
    impl crate::context_service::ContextTransport for PermitMalformedCommit {
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:crate::context_service::ContextServiceRequest)
            ->Result<crate::context_service::ContextServiceResponse,RuntimeError>{self.request(request)}
        fn request(&self,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{
            use crate::context_service::{ContextServiceRequest as Request,ContextServiceResponse as Response};
            if matches!(&request,Request::CommitPage{url,..} if url.ends_with("/second"))
                &&self.restored.load(Ordering::SeqCst)>0&&self.reject_rollback.swap(false,Ordering::SeqCst){
                return Err(RuntimeError::ContextService("injected incumbent rollback failure".into()));
            }
            if matches!(&request,Request::CommitPage{url,..} if url=="invalid URL"){
                return Ok(Response::PageCommitted);
            }
            let restoring=matches!(&request,Request::MessageWire{request:oryn_js::message_wire::MessageRequest::RestoreDocument{..},..});
            let response=self.inner.request(request)?;
            if restoring&&matches!(&response,Response::MessageWire{result:Ok(oryn_js::message_wire::MessageResponse::DocumentRestored{..})}){
                self.restored.fetch_add(1,Ordering::SeqCst);
            }
            Ok(response)
        }
    }

    #[test]
    fn parent_failed_suspension_finishes_native_resume_before_publication(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let client=crate::context_service::SharedContextClient::direct(context.clone(),id);
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/source","<p>source</p>",&BTreeMap::new()).unwrap();
        let realm=page.executable.as_ref().unwrap().agent_root_realm();let generation=page.document_generation;
        ROOT_SUSPENSION_FAULT.with(|fault|fault.set(true));assert!(page.suspend_document(true).is_err());
        assert!(page.root_source_resume.is_some());
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        assert!(host.realm_is_parked(realm));assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
        page.poll_source_resume();assert!(page.root_source_resume.is_none());assert_eq!(page.document_generation,generation);
        host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        assert!(!host.realm_is_parked(realm),"source publication requires final native queue release");
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
        assert_eq!(page.executable.as_mut().unwrap().evaluate("document.body.textContent").unwrap(),"source");
    }

    struct FailReparkCompletion {
        inner:PermitMalformedCommit,context:BrowserContext,fail:std::sync::atomic::AtomicBool,
    }
    impl crate::context_service::ContextTransport for FailReparkCompletion{
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{self.request(request)}
        fn request(&self,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{
            use crate::context_service::{ContextServiceRequest as Q,ContextServiceResponse as R};
            use oryn_js::message_wire::MessageRequest as M;
            if let Q::MessageWire{source_page_id,request:park@M::ParkDocument{..}}=&request{
                if self.inner.restored.load(Ordering::SeqCst)>0&&self.fail.swap(false,Ordering::SeqCst){
                    PARK_COMPLETION_FAULT.with(|fault|fault.set(true));
                    CACHED_COMPLETION_FAULT.with(|fault|fault.set(true));
                    let result=self.context.handle_cached_document_authority(*source_page_id,park.clone());
                    PARK_COMPLETION_FAULT.with(|fault|fault.set(false));CACHED_COMPLETION_FAULT.with(|fault|fault.set(false));
                    assert!(result.as_ref().is_err_and(|error|error.contains("injected park completion suffix failure")));
                    return Ok(R::MessageWire{result});
                }
            }
            self.inner.request(request)
        }
    }
    #[test]
    fn failed_repark_suffix_retains_unreceived_token_until_exact_rollback_completes(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let restored=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let transport=Arc::new(FailReparkCompletion{inner:PermitMalformedCommit{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),restored:restored.clone(),reject_rollback:std::sync::atomic::AtomicBool::new(false)},context:context.clone(),fail:std::sync::atomic::AtomicBool::new(true)});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport,crate::worker_route::WorkerRoute::issue(775,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p><iframe srcdoc='<p>cached child</p>'></iframe>",&BTreeMap::new()).unwrap();
        assert!(context.inner.group.messages.lock().unwrap().registered_documents().iter().any(|(owner,target,_,_)|*owner==id.0&&matches!(target,oryn_js::message_wire::WindowTarget::Frame(_))),"destination child must have native parent authority");
        let identity=page.executable.as_ref().unwrap().agent_host_identity();let root=page.executable.as_ref().unwrap().automation_realms().into_iter().find(|realm|realm.parent_realm.is_none()&&realm.live).unwrap();
        let (rejected_generation,rejected_epoch)=(root.document_generation,root.document_authority_epoch);
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0),Some((rejected_generation,rejected_epoch)));
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        let incumbent=page.executable.as_ref().unwrap().agent_root_realm();
        page.back_forward_cache.get_mut(&0).unwrap().url="invalid URL".into();
        assert!(page.restore_cached_document(0,"https://shared-cache.test/first".into()).is_err());
        assert!(page.failed_install_recovery.is_some()&&page.executable.is_none());
        assert!(!context.inner.response_reclaims.lock().unwrap().ready(),"fresh park token survives failed pre-response rollback suffix");
        assert!(context.inner.cached_authority_completions.lock().unwrap().records.values().any(|record|record.page==id&&record.reply.is_some()&&!record.complete));
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0).unwrap().0,rejected_generation);
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0).unwrap().1,rejected_epoch);
        assert_eq!(page.matching_agent_host_count(&identity),1);
        page.failed_install_recovery.as_mut().unwrap().retry=None;page.poll_failed_install_recovery();
        assert!(page.failed_install_recovery.is_none(),"{:?}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().clone()));
        assert!(context.inner.response_reclaims.lock().unwrap().ready());
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),incumbent);
        assert_eq!(page.matching_agent_host_count(&identity),1);assert_eq!(restored.load(Ordering::SeqCst),2);
        assert!(!page.back_forward_cache.contains_key(&0));drain(&mut page);assert!(!page.has_retired_agent_documents());
        assert!(context.inner.cached_document_security.lock().unwrap().is_empty(),"no failed park snapshot is orphaned");
    }

    struct FailRestoreCompletion {
        inner:crate::context_service::SharedContextClient,context:BrowserContext,
        first:std::sync::atomic::AtomicBool,hold_retry:std::sync::atomic::AtomicBool,
        restores:Mutex<Vec<oryn_js::message_wire::ParkedDocumentToken>>,
        parks:Mutex<Vec<oryn_js::message_wire::ParkedDocumentToken>>,
        discards:Mutex<Vec<oryn_js::message_wire::ParkedDocumentToken>>,
    }
    impl crate::context_service::ContextTransport for FailRestoreCompletion {
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{self.request(request)}
        fn request(&self,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{
            use crate::context_service::{ContextServiceRequest as Q,ContextServiceResponse as R};
            use oryn_js::message_wire::{MessageRequest as M,MessageResponse as A};
            if let Q::MessageWire{source_page_id,request:M::RestoreDocument{token}}=&request{
                self.restores.lock().unwrap().push(token.clone());
                if self.first.swap(false,Ordering::SeqCst){
                    CACHED_COMPLETION_FAULT.with(|fault|fault.set(true));
                    let result=self.context.handle_cached_document_authority(*source_page_id,M::RestoreDocument{token:token.clone()});
                    CACHED_COMPLETION_FAULT.with(|fault|fault.set(false));
                    assert!(result.as_ref().is_err_and(|error|error.contains("injected cached completion suffix failure")));
                    let ledger=self.context.inner.cached_authority_completions.lock().unwrap();
                    let record=&ledger.records[token];assert!(record.reply.is_some()&&!record.complete,"native restore committed before failing its suffix");
                    return Ok(R::MessageWire{result});
                }
                if self.hold_retry.swap(false,Ordering::SeqCst){return Err(RuntimeError::ContextService("injected restore retry delay".into()))}
            }
            if let Q::MessageWire{request:M::DiscardDocument{token},..}=&request{self.discards.lock().unwrap().push(token.clone());}
            let parking=matches!(&request,Q::MessageWire{request:M::ParkDocument{..},..});
            let response=self.inner.request(request)?;
            if parking&&let R::MessageWire{result:Ok(A::DocumentParked(token))}=&response{self.parks.lock().unwrap().push(token.clone());}
            Ok(response)
        }
    }
    #[test]
    fn failed_restore_completion_reconciles_original_token_before_incumbent_rollback(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let transport=Arc::new(FailRestoreCompletion{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),context:context.clone(),first:std::sync::atomic::AtomicBool::new(true),hold_retry:std::sync::atomic::AtomicBool::new(true),restores:Mutex::new(Vec::new()),parks:Mutex::new(Vec::new()),discards:Mutex::new(Vec::new())});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport.clone(),crate::worker_route::WorkerRoute::issue(774,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p><iframe srcdoc='<p>cached child</p>'></iframe>",&BTreeMap::new()).unwrap();
        assert!(context.inner.group.messages.lock().unwrap().registered_documents().iter().any(|(owner,target,_,_)|*owner==id.0&&matches!(target,oryn_js::message_wire::WindowTarget::Frame(_))),"destination child must have native parent authority");
        let identity=page.executable.as_ref().unwrap().agent_host_identity();
        let root=page.executable.as_ref().unwrap().automation_realms().into_iter().find(|realm|realm.parent_realm.is_none()&&realm.live).unwrap();
        let (rejected_generation,rejected_epoch)=(root.document_generation,root.document_authority_epoch);
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0),Some((rejected_generation,rejected_epoch)));
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        let incumbent=page.executable.as_ref().unwrap().agent_root_realm();
        let failure=page.restore_cached_document(0,"https://shared-cache.test/first".into()).unwrap_err();
        assert!(failure.to_string().contains("injected cached completion suffix failure"),"original failure must remain observable: {failure}");
        assert!(page.failed_install_recovery.is_some()&&page.executable.is_none());
        assert!(!page.back_forward_cache[&0].reusable);
        assert_eq!(page.matching_agent_host_count(&identity),1);
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0).unwrap().0,rejected_generation);
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(id.0).unwrap().1,rejected_epoch);
        let original=transport.parks.lock().unwrap()[0].clone();
        assert_eq!(&*transport.restores.lock().unwrap(),&[original.clone(),original.clone()]);
        assert!(transport.discards.lock().unwrap().is_empty(),"unacknowledged Restore cannot accept Discard");
        page.failed_install_recovery.as_mut().unwrap().retry=None;page.poll_failed_install_recovery();
        assert!(page.failed_install_recovery.is_none(),"{:?}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().clone()));
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),incumbent);
        assert_eq!(page.url,"https://shared-cache.test/second");assert!(!page.back_forward_cache.contains_key(&0));
        assert_eq!(page.matching_agent_host_count(&identity),1);
        let parks=transport.parks.lock().unwrap().clone();assert_eq!(parks.len(),3);assert_ne!(parks[2],original);
        assert_eq!(&*transport.restores.lock().unwrap(),&[original.clone(),original.clone(),original.clone(),parks[1].clone()]);
        drain(&mut page);assert!(!page.has_retired_agent_documents());
        assert_eq!(&*transport.discards.lock().unwrap(),&[parks[2].clone()]);
    }

    struct ExpireFirstRestore{inner:crate::context_service::SharedContextClient,context:BrowserContext,expire:std::sync::atomic::AtomicBool}
    impl crate::context_service::ContextTransport for ExpireFirstRestore{
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{self.request(request)}
        fn request(&self,request:crate::context_service::ContextServiceRequest)->Result<crate::context_service::ContextServiceResponse,RuntimeError>{
            use crate::context_service::{ContextServiceRequest as Q,ContextServiceResponse as R};
            let token=match &request{Q::MessageWire{request:oryn_js::message_wire::MessageRequest::RestoreDocument{token},..}=>Some(token.clone()),_=>None};
            let response=self.inner.request(request)?;
            if let Some(token)=token{if self.expire.swap(false,Ordering::SeqCst){
                assert!(matches!(&response,R::MessageWire{result:Ok(oryn_js::message_wire::MessageResponse::DocumentRestored{..})}));
                let mut ledger=self.context.inner.cached_authority_completions.lock().unwrap();ledger.records.remove(&token);ledger.settled.retain(|value|value!=&token);
                return Ok(R::MessageWire{result:Ok(oryn_js::message_wire::MessageResponse::CachedCompletionExpired)})
            }}Ok(response)
        }
    }
    #[test]
    fn expired_restore_completion_disposes_local_cache_and_recovers_incumbent(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let transport=Arc::new(ExpireFirstRestore{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),context:context.clone(),expire:std::sync::atomic::AtomicBool::new(true)});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport,crate::worker_route::WorkerRoute::issue(773,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p><iframe srcdoc='<p>expired cached child</p>'></iframe>",&BTreeMap::new()).unwrap();
        assert!(context.inner.group.messages.lock().unwrap().registered_documents().iter().any(|(owner,target,_,_)|*owner==id.0&&matches!(target,oryn_js::message_wire::WindowTarget::Frame(_))),"destination child must have native parent authority");
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        let incumbent=page.executable.as_ref().unwrap().agent_root_realm();
        assert!(page.restore_cached_document(0,"https://shared-cache.test/first".into()).is_err());
        drain(&mut page);
        assert!(!page.back_forward_cache.contains_key(&0));assert!(page.failed_install_recovery.is_none(),"{}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().clone().unwrap_or_default()));
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),incumbent);
    }

    #[test]
    fn location_failure_after_native_restore_never_reuses_consumed_cache_token(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let restored=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let transport=Arc::new(PermitMalformedCommit{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),restored:restored.clone(),reject_rollback:std::sync::atomic::AtomicBool::new(false)});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport,crate::worker_route::WorkerRoute::issue(772,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p><iframe srcdoc='<p>cached child</p>'></iframe>",&BTreeMap::new()).unwrap();
        assert!(context.inner.group.messages.lock().unwrap().registered_documents().iter().any(|(owner,target,_,_)|*owner==id.0&&matches!(target,oryn_js::message_wire::WindowTarget::Frame(_))),"destination child must have native parent authority");
        let rejected=page.executable.as_ref().unwrap().agent_root_realm();
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        let incumbent=page.executable.as_ref().unwrap().agent_root_realm();
        page.back_forward_cache.get_mut(&0).unwrap().url="invalid URL".into();
        assert!(page.restore_cached_document(0,"https://shared-cache.test/first".into()).is_err());
        assert_eq!(restored.load(Ordering::SeqCst),2,"destination grant and rollback grant both consumed; recovery={:?}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().clone()));
        assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),incumbent);
        assert_eq!(page.url,"https://shared-cache.test/second");
        assert!(!page.back_forward_cache.contains_key(&0));
        assert!(page.retired_agent_document_realms().contains(&rejected));
        assert!(page.restore_cached_document(0,"https://shared-cache.test/first".into()).unwrap().is_none());
        drain(&mut page);assert!(!page.has_retired_agent_documents());
    }

    #[test]
    fn failed_restore_and_rollback_recover_exact_host_from_nonreusable_cached_driver(){
        let context=Browser::in_process_probe().new_context();let handle=context.new_page();let id=handle.page_id();
        let restored=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let transport=Arc::new(PermitMalformedCommit{inner:crate::context_service::SharedContextClient::direct(context.clone(),id),restored:restored.clone(),reject_rollback:std::sync::atomic::AtomicBool::new(true)});
        let client=crate::context_service::SharedContextClient::for_test_transport(transport,crate::worker_route::WorkerRoute::issue(773,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
        load(&mut page,"https://shared-cache.test/first","<p>first</p>",&BTreeMap::new()).unwrap();
        let identity=page.executable.as_ref().unwrap().attached_agent_host_identity().unwrap();
        let mut host=page.executable.as_mut().unwrap().take_agent_host().unwrap();
        let sibling=host.create_realm("https://shared-cache.test",91,None).unwrap();host.activate_realm(sibling).unwrap();host.execute("let siblingMarker=67").unwrap();
        assert!(page.executable.as_mut().unwrap().install_agent_host(host).is_ok());
        load(&mut page,"https://shared-cache.test/second","<p>second</p>",&BTreeMap::new()).unwrap();
        page.back_forward_cache.get_mut(&0).unwrap().url="invalid URL".into();
        assert!(page.restore_cached_document(0,"https://shared-cache.test/first".into()).is_err());
        assert!(page.executable.is_none());assert!(!page.back_forward_cache[&0].reusable);
        assert!(page.back_forward_cache[&0].executable.has_agent_host());
        assert!(page.back_forward_cache.contains_key(&1));assert_eq!(restored.load(Ordering::SeqCst),1);
        let mut host=page.extract_agent_host(Some(&identity)).unwrap().expect("actor recovers cached host donor");
        assert!(std::rc::Rc::ptr_eq(&identity,&host.agent_host_identity()));
        assert!(page.extract_agent_host(Some(&identity)).unwrap().is_none());
        host.activate_realm(sibling).unwrap();assert_eq!(host.evaluate_string("String(siblingMarker)").unwrap(),"67");
        page.pending_agent_host=Some(host);assert!(page.failed_install_recovery.is_some());page.failed_install_recovery.as_mut().unwrap().retry=None;page.poll_failed_install_recovery();
        assert!(page.failed_install_recovery.is_none(),"{}",LAST_INSTALL_RECOVERY_ERROR.with(|last|last.borrow().clone().unwrap_or_default()));assert_eq!(page.url,"https://shared-cache.test/second");
        assert!(!page.back_forward_cache.contains_key(&0));drain(&mut page);assert!(!page.has_retired_agent_documents());
    }

}
