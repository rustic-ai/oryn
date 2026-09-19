// Exact parent history completion custody. A consumed capability is never
// rebound on retry: only its retained cleanup suffix and original ACK remain.
#[cfg(unix)]
#[derive(Default)]
struct CachedAuthorityCompletions {
    initial:BTreeMap<oryn_js::message_wire::ParkedDocumentToken,InitialAuthorityCompletion>,
    initial_admitting:usize,
    unparked_resumes:BTreeMap<crate::agent_registry::AgentOwner,u8>,
    resuming:bool,
    parking:bool,
    records:BTreeMap<oryn_js::message_wire::ParkedDocumentToken,CachedAuthorityCompletion>,
    busy:BTreeMap<oryn_js::message_wire::ParkedDocumentToken,(PageId,Vec<u8>)>,
    settled:VecDeque<oryn_js::message_wire::ParkedDocumentToken>,
}
#[cfg(unix)]
#[derive(Clone,Copy)]
enum InitialAuthorityOutcome{Restored(u64,u64),Discarded}
#[cfg(unix)]
struct InitialAuthorityCompletion{page:PageId,outcome:Option<InitialAuthorityOutcome>}
#[cfg(unix)]
struct CachedAuthorityCompletion {
    page:PageId,
    request:Vec<u8>,
    owners:Vec<crate::agent_registry::AgentOwner>,
    reply:Option<oryn_js::message_wire::MessageResponse>,
    owner:usize,
    stage:u8,
    complete:bool,
    disposing:bool,
}
#[cfg(unix)]
struct CachedCompletionLease<'a>{context:&'a BrowserContext,token:oryn_js::message_wire::ParkedDocumentToken,record:Option<CachedAuthorityCompletion>}
#[cfg(unix)]
impl Drop for CachedCompletionLease<'_>{fn drop(&mut self){
    let mut ledger=self.context.inner.cached_authority_completions.lock().unwrap_or_else(|error|error.into_inner());
    // Page retirement can revoke this lease while native cleanup is outside
    // the registry. Never reinsert a retired page's record afterward.
    if ledger.busy.remove(&self.token).is_none(){return}
    let Some(record)=self.record.take()else{return};
    if record.complete{
        if let Some(initial)=ledger.initial.get_mut(&self.token){initial.outcome=Some(match record.reply.as_ref(){Some(oryn_js::message_wire::MessageResponse::DocumentRestored{document_generation,stream_epoch})=>InitialAuthorityOutcome::Restored(*document_generation,*stream_epoch),_=>InitialAuthorityOutcome::Discarded});}
        ledger.settled.push_back(self.token.clone());
    }
    ledger.records.insert(self.token.clone(),record);
    while ledger.settled.len()>128{if let Some(expired)=ledger.settled.pop_front(){ledger.records.remove(&expired);}}
}}
#[cfg(unix)]
struct NativeParkLease<'a>(&'a BrowserContext);
#[cfg(unix)]
impl Drop for NativeParkLease<'_>{fn drop(&mut self){self.0.inner.cached_authority_completions.lock().unwrap_or_else(|error|error.into_inner()).parking=false;}}
#[cfg(unix)]
struct UnparkedResumeLease<'a>(&'a BrowserContext);
#[cfg(unix)]
impl Drop for UnparkedResumeLease<'_>{fn drop(&mut self){self.0.inner.cached_authority_completions.lock().unwrap_or_else(|error|error.into_inner()).resuming=false;}}
#[cfg(unix)]
impl BrowserContext {
    fn retain_unparked_resumes(&self,owners:&[crate::agent_registry::AgentOwner]){
        let mut ledger=self.inner.cached_authority_completions.lock().unwrap_or_else(|error|error.into_inner());
        for owner in owners{ledger.unparked_resumes.entry(owner.clone()).or_insert(0);}
    }
    // A failed Park has no capability to retry. Retain the exact frozen owners
    // independently, and never hold this registry across worker termination.
    pub(crate) fn retry_unparked_resumes(&self)->Result<(),RuntimeError>{
        let owners={let mut ledger=self.inner.cached_authority_completions.lock().map_err(|_|RuntimeError::ContextService("park recovery registry poisoned".into()))?;
            if ledger.resuming{return Ok(())}ledger.resuming=true;ledger.unparked_resumes.keys().cloned().collect::<Vec<_>>()};
        let _lease=UnparkedResumeLease(self);let mut failed=false;
        for owner in owners{loop{
            let stage=self.inner.cached_authority_completions.lock().map_err(|_|RuntimeError::ContextService("park recovery registry poisoned".into()))?.unparked_resumes.get(&owner).copied();
            let Some(stage)=stage else{break};
            #[cfg(test)] if UNPARKED_RESUME_FAULT.with(|fault|fault.replace(false)){failed=true;break;}
            let result=(||->Result<bool,RuntimeError>{match stage{
                0=>{
                    let current=self.dispatch_worker_registry(|agents,hub|{
                        let current=match &owner{crate::agent_registry::AgentOwner::Window{page,target,generation,epoch}=>hub.registered_documents().iter().any(|(p,t,g,e)|*p==page.0&&t==target&&g==generation&&e==epoch),_=>false};
                        if current{agents.resume_document_owners(std::slice::from_ref(&owner))?;}Ok(current)
                    }).map_err(|error|RuntimeError::ContextService(error.to_string()))?;
                    if current{return Ok(true)}
                },
                1=>self.retire_window_transactions(&owner).map_err(|error|RuntimeError::ContextService(error.to_string()))?,
                2=>self.retire_window_sockets(&owner).map_err(|error|RuntimeError::ContextService(error.to_string()))?,
                3=>{if let Some(services)=self.existing_service_registry(){services.retire_client(&owner).map_err(|error|RuntimeError::ContextService(error.to_string()))?;}},
                4=>self.clear_owner_blob_urls(&owner),
                5=>{self.dispatch_worker_registry(|agents,hub|agents.retire_owner(hub,&owner)).map_err(|error|RuntimeError::ContextService(error.to_string()))?;return Ok(true)},
                _=>return Ok(true),
            }Ok(false)})();
            let complete=match result{Ok(complete)=>complete,Err(_)=>{failed=true;break}};
            let mut ledger=self.inner.cached_authority_completions.lock().unwrap_or_else(|error|error.into_inner());
            if complete{ledger.unparked_resumes.remove(&owner);break}
            if let Some(saved)=ledger.unparked_resumes.get_mut(&owner){*saved=stage+1}else{break}
        }}
        if failed{Err(RuntimeError::ContextService("unparked worker recovery pending".into()))}else{Ok(())}
    }
}
#[cfg(all(unix,test))]
thread_local!{static UNPARKED_RESUME_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(unix)]
impl BrowserContext {
    fn complete_cached_document_authority(&self,page:PageId,request:oryn_js::message_wire::MessageRequest)->Result<oryn_js::message_wire::MessageResponse,String>{
        self.complete_cached_document_authority_guarded(page,request,None)
    }
    fn complete_cached_document_authority_guarded(&self,page:PageId,request:oryn_js::message_wire::MessageRequest,expected_policy_epoch:Option<u64>)->Result<oryn_js::message_wire::MessageResponse,String>{
        use oryn_js::message_wire::{MessageRequest as Q,MessageResponse as R};
        let(token,restore)=match &request{Q::RestoreDocument{token}|Q::RestoreFrameDocument{token,..}=>(token.clone(),true),Q::DiscardDocument{token}|Q::DiscardFrameDocument{token}=>(token.clone(),false),_=>return Err("invalid cached completion request".into())};
        let key=serde_json::to_vec(&request).map_err(|error|error.to_string())?;
        let mut lease={
            let mut ledger=self.inner.cached_authority_completions.lock().map_err(|_|"cached completion registry poisoned")?;
            if let Some((owner,request))=ledger.busy.get(&token){return Err(if *owner==page&&*request==key{"cached completion already in progress"}else{"conflicting or foreign cached completion"}.into())}
            if let Some(record)=ledger.records.get(&token){
                if record.page!=page||record.request!=key{return Err("conflicting or foreign cached completion".into())}
                if record.complete{return record.reply.clone().ok_or_else(||"cached completion omitted response".into())}
            }
            let record=if let Some(record)=ledger.records.remove(&token){record}else{
                if ledger.records.len()+ledger.busy.len()>=4096{return Err("cached completion capacity".into())}
                let owners=self.inner.cached_document_security.lock().map_err(|_|"cached security poisoned")?.get(&token).filter(|snapshot|snapshot.page==page).map(|snapshot|snapshot.worker_owners.clone());
                let owners=match owners{Some(owners)=>owners,None=>{
                    if self.inner.group.messages.lock().map_err(|_|"message hub poisoned")?.cached_token_expired(page.0,&token){return Ok(R::CachedCompletionExpired)}
                    return Err("unknown or foreign cached capability".into())
                }};
                CachedAuthorityCompletion{page,request:key.clone(),owners,reply:None,owner:0,stage:0,complete:false,disposing:false}
            };
            ledger.busy.insert(token.clone(),(page,key));
            CachedCompletionLease{context:self,token:token.clone(),record:Some(record)}
        };
        if lease.record.as_ref().unwrap().reply.is_none(){
            if let Q::RestoreFrameDocument{parent,..}=&request{
                let authority=match parent{
                    oryn_js::worker_wire::Creator::Window{target,document_generation,stream_epoch}=>self.native_worker_authority(&crate::agent_registry::AgentOwner::Window{page,target:target.clone(),generation:*document_generation,epoch:*stream_epoch}).map(|_|()).map_err(|error|error.to_string()),
                    _=>Err("frame restoration requires Window parent".into()),
                };
                if let Err(error)=authority{lease.record=None;return Err(error)}
            }

            let reply=match self.handle_cached_document_security(page,request,BTreeMap::new(),expected_policy_epoch){Ok(reply)=>reply,Err(error)=>{lease.record=None;return Err(error)}};
            if !matches!((&reply,restore),(R::DocumentRestored{..},true)|(R::Done,false)){return Err("cached completion response mismatch".into())}
            lease.record.as_mut().unwrap().reply=Some(reply);
        }
        let record=lease.record.as_mut().unwrap();
        #[cfg(test)] if CACHED_COMPLETION_FAULT.with(|fault|fault.replace(false)){return Err("injected cached completion suffix failure".into())}
        while let Some(owner)=record.owners.get(record.owner){
            if restore&&!record.disposing{
                let resumed=self.dispatch_worker_registry(|agents,hub|{
                    let current=match owner{
                        crate::agent_registry::AgentOwner::Window{page,target,generation,epoch}=>hub.registered_documents().iter().any(|(candidate,candidate_target,candidate_generation,candidate_epoch)|*candidate==page.0&&candidate_target==target&&candidate_generation==generation&&candidate_epoch==epoch),
                        crate::agent_registry::AgentOwner::Agent(_)=>false,
                    };
                    if current{agents.resume_document_owners(std::slice::from_ref(owner))?;}
                    Ok(current)
                }).map_err(|error|error.to_string())?;
                if resumed{record.owner+=1;continue;}
                record.disposing=true;
                // The security commit succeeded, but another navigation
                // replaced this owner before its resume suffix. Dispose its
                // frozen resources; never revive it or replay the old bind.
            }
            match record.stage{
                0=>self.retire_window_transactions(owner).map_err(|error|error.to_string())?,
                1=>self.retire_window_sockets(owner).map_err(|error|error.to_string())?,
                2=>{if let Some(services)=self.existing_service_registry(){services.retire_client(owner).map_err(|error|error.to_string())?;}},
                3=>self.clear_owner_blob_urls(owner),
                4=>self.dispatch_worker_registry(|agents,hub|agents.retire_owner(hub,owner)).map_err(|error|error.to_string())?,
                _=>{record.owner+=1;record.stage=0;record.disposing=false;continue},
            }
            record.stage+=1;
        }
        if !restore{self.inner.cached_document_security.lock().map_err(|_|"cached security poisoned")?.remove(&token);}
        record.complete=true;let reply=record.reply.clone().unwrap();
        Ok(reply)
    }
}

#[cfg(all(unix,test))]
thread_local!{static CACHED_COMPLETION_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static PARK_COMPLETION_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(all(unix,test))]
mod cached_completion_tests {
    use super::*;
    #[test]
    fn pruned_completed_ack_is_typed_expiry_and_never_rebinds(){
        use oryn_js::message_wire::{MessageRequest as Q,MessageResponse as R};
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        let oryn_js::worker_wire::Creator::Window{document_generation,stream_epoch,..}=context.native_initial_document_creator(page).unwrap()else{unreachable!()};
        let R::DocumentParked(first)=context.handle_cached_document_authority(page,Q::ParkDocument{document_generation,stream_epoch}).unwrap()else{panic!()};
        context.handle_cached_document_authority(page,Q::DiscardDocument{token:first.clone()}).unwrap();
        for generation in 100..229{
            context.install_document_security(page,"https://prune.test/".into(),&[]).unwrap();
            context.handle_message_wire(page,Q::RegisterRootDocument{document_generation:generation,stream_epoch:0}).unwrap();
            let R::DocumentParked(token)=context.handle_cached_document_authority(page,Q::ParkDocument{document_generation:generation,stream_epoch:0}).unwrap()else{panic!()};
            context.handle_cached_document_authority(page,Q::DiscardDocument{token}).unwrap();
        }
        assert!(context.inner.cached_authority_completions.lock().unwrap().records.len()<=128);
        assert!(matches!(context.handle_cached_document_authority(page,Q::RestoreDocument{token:first.clone()}).unwrap(),R::CachedCompletionExpired));
        assert!(matches!(context.handle_cached_document_authority(page,Q::DiscardDocument{token:first.clone()}).unwrap(),R::CachedCompletionExpired));
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(page.0),None);
        let foreign=context.new_page().page_id();assert!(context.handle_cached_document_authority(foreign,Q::RestoreDocument{token:first}).is_err());
    }
    #[test]
    fn failed_park_keeps_original_error_and_exact_worker_resume_without_token(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        let oryn_js::worker_wire::Creator::Window{target,document_generation,stream_epoch}=context.native_initial_document_creator(page).unwrap()else{unreachable!()};
        let owner=crate::agent_registry::AgentOwner::Window{page,target,generation:document_generation,epoch:stream_epoch};
        UNPARKED_RESUME_FAULT.with(|fault|fault.set(true));
        let error=context.handle_cached_document_authority(page,oryn_js::message_wire::MessageRequest::ParkDocument{document_generation:document_generation+100,stream_epoch}).unwrap_err();
        assert!(!error.contains("recovery pending"));
        assert_eq!(context.inner.cached_authority_completions.lock().unwrap().unparked_resumes.get(&owner),Some(&0));
        assert!(context.inner.cached_document_security.lock().unwrap().is_empty());
        context.retry_context_response_reclaims().unwrap();
        assert!(context.inner.cached_authority_completions.lock().unwrap().unparked_resumes.is_empty());
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(page.0),Some((document_generation,stream_epoch)));
    }
    #[test]
    fn superseded_failed_park_cleanup_never_resumes_or_rebinds_new_owner(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        let oryn_js::worker_wire::Creator::Window{target,document_generation,stream_epoch}=context.native_initial_document_creator(page).unwrap()else{unreachable!()};
        let owner=crate::agent_registry::AgentOwner::Window{page,target,generation:document_generation,epoch:stream_epoch};
        context.retain_unparked_resumes(std::slice::from_ref(&owner));
        context.install_document_security(page,"https://new.test/".into(),&[]).unwrap();
        context.handle_message_wire(page,oryn_js::message_wire::MessageRequest::RegisterRootDocument{document_generation:90,stream_epoch:0}).unwrap();
        context.retry_unparked_resumes().unwrap();
        assert!(context.inner.cached_authority_completions.lock().unwrap().unparked_resumes.is_empty());
        assert_eq!(context.inner.group.messages.lock().unwrap().root_document_identity(page.0),Some((90,0)));
    }
    #[test]
    fn consumed_initial_restore_keeps_exact_reply_and_never_rebinds_after_new_commit(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();
        let token=context.park_initial_document(page).unwrap();
        context.commit_page_url(page,"https://replacement.test/".into()).unwrap();
        context.install_document_security(page,"https://replacement.test/".into(),&[]).unwrap();
        CACHED_COMPLETION_FAULT.with(|fault|fault.set(true));
        assert!(context.finish_initial_document(page,token.clone(),true).is_err());
        assert!(context.inner.cached_authority_completions.lock().unwrap().records[&token].reply.is_some());
        assert!(context.finish_initial_document(page,token.clone(),true).unwrap());
        assert_eq!(context.pages().iter().find(|record|record.page.page_id==page).unwrap().page.url,"about:blank");
        assert!(context.native_document_authority(page,"about:blank").unwrap().4);
        let old=context.page_origin(page).unwrap();
        context.commit_page_url(page,"https://later.test/".into()).unwrap();
        context.install_document_security(page,"https://later.test/".into(),&[]).unwrap();
        assert!(!context.finish_initial_document(page,token.clone(),true).unwrap(),"new policy invalidates rollback even before new root registration");
        assert_eq!(context.pages().iter().find(|record|record.page.page_id==page).unwrap().page.url,"https://later.test/");
        context.handle_message_wire(page,oryn_js::message_wire::MessageRequest::RegisterRootDocument{document_generation:99,stream_epoch:0}).unwrap();
        let later=context.page_origin(page).unwrap();assert_ne!(old,later);
        assert!(!context.finish_initial_document(page,token.clone(),true).unwrap());
        assert_eq!(context.page_origin(page).unwrap(),later);
        assert!(context.finish_initial_document(page,token,false).is_err());
    }
    #[test]
    fn stale_resume_suffix_disposes_original_owner_without_rebinding(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();let token=context.park_initial_document(page).unwrap();
        context.install_document_security(page,"https://first.test/".into(),&[]).unwrap();
        CACHED_COMPLETION_FAULT.with(|fault|fault.set(true));assert!(context.finish_initial_document(page,token.clone(),true).is_err());
        context.install_document_security(page,"https://later.test/".into(),&[]).unwrap();
        context.handle_message_wire(page,oryn_js::message_wire::MessageRequest::RegisterRootDocument{document_generation:100,stream_epoch:0}).unwrap();let later=context.page_origin(page).unwrap();
        assert!(!context.finish_initial_document(page,token,true).unwrap());assert_eq!(context.page_origin(page).unwrap(),later);
        assert!(context.window_worker_creator(page,&oryn_js::message_wire::WindowTarget::Root,100,0).is_ok());
    }
    #[test]
    fn unreceived_park_never_restores_over_a_new_security_commit(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();let token=context.park_initial_document(page).unwrap();
        context.install_document_security(page,"https://new-policy.test/".into(),&[]).unwrap();let expected=context.page_origin(page).unwrap();
        context.rollback_unreceived_document(page,&token).unwrap();assert_eq!(context.page_origin(page).unwrap(),expected);
        assert!(context.finish_initial_document(page,token,true).is_err());
    }
    #[test]
    fn nested_completion_retry_observes_busy_lease_without_reentering_commit(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();let token=context.park_initial_document(page).unwrap();
        let request=serde_json::to_vec(&oryn_js::message_wire::MessageRequest::RestoreDocument{token:token.clone()}).unwrap();
        let owners=context.inner.cached_document_security.lock().unwrap()[&token].worker_owners.clone();
        context.inner.cached_authority_completions.lock().unwrap().busy.insert(token.clone(),(page,request.clone()));
        let lease=CachedCompletionLease{context:&context,token:token.clone(),record:Some(CachedAuthorityCompletion{page,request,owners,reply:None,owner:0,stage:0,complete:false,disposing:false})};
        assert!(context.finish_initial_document(page,token.clone(),true).is_err());assert!(context.inner.cached_document_security.lock().unwrap().contains_key(&token));
        drop(lease);assert!(context.finish_initial_document(page,token,true).unwrap());
    }
    #[test]
    fn completed_discard_is_exact_and_foreign_tokens_cannot_claim_cleanup(){
        let context=Browser::in_process_probe().new_context();let page=context.new_page().page_id();let other=context.new_page().page_id();
        let token=context.park_initial_document(page).unwrap();
        CACHED_COMPLETION_FAULT.with(|fault|fault.set(true));assert!(context.finish_initial_document(page,token.clone(),false).is_err());
        assert!(context.finish_initial_document(other,token.clone(),false).is_err());
        assert!(context.finish_initial_document(page,token.clone(),false).unwrap());assert!(context.finish_initial_document(page,token.clone(),false).unwrap());
        assert!(context.finish_initial_document(page,token,true).is_err());
    }
}

#[cfg(unix)]
impl BrowserContext {
    pub(crate) fn rollback_unreceived_document(&self,page:PageId,token:&oryn_js::message_wire::ParkedDocumentToken)->Result<(),String>{
        use oryn_js::message_wire::{MessageRequest as Q,WindowTarget};
        if !self.inner.group.pages.lock().map_err(|_|"page registry poisoned")?.get(&page).is_some_and(|record|record.is_live()){return Ok(())}
        let saved={let ledger=self.inner.cached_authority_completions.lock().map_err(|_|"cached completion registry poisoned")?;ledger.records.get(token).filter(|record|record.page==page&&record.reply.is_some()).map(|record|record.request.clone()).or_else(||ledger.busy.get(token).filter(|(owner,_)|*owner==page).map(|(_,request)|request.clone()))};
        let (request,expected_epoch)=if let Some(saved)=saved{(serde_json::from_slice::<Q>(&saved).map_err(|error|error.to_string())?,None)}else{
            let snapshot={let snapshots=self.inner.cached_document_security.lock().map_err(|_|"cached security poisoned")?;snapshots.get(token).filter(|snapshot|snapshot.page==page).map(|snapshot|(snapshot.frame_target.clone(),snapshot.frame_parent.clone(),snapshot.security_epoch))};
            let (frame,parent,epoch)=match snapshot{Some(snapshot)=>snapshot,None=>{
                return if self.inner.group.messages.lock().map_err(|_|"message hub poisoned")?.cached_token_expired(page.0,token){Ok(())}else{Err("unknown unreceived document capability".into())}
            }};
            let changed=self.inner.worker_policies.lock().map_err(|_|"worker policy poisoned")?.root_lifetimes.get(&page).map_or(0,|life|life.epoch)!=epoch;
            let occupied={let hub=self.inner.group.messages.lock().map_err(|_|"message hub poisoned")?;hub.registered_documents().iter().any(|(owner,target,_,_)|*owner==page.0&&match(&frame,target){(None,WindowTarget::Root)=>true,(Some(frame),WindowTarget::Frame(current))=>frame==current,_=>false})};
            let request=if changed||occupied{Q::DiscardDocument{token:token.clone()}}
            else if let Some(frame)=frame{
                let Some(crate::agent_registry::AgentOwner::Window{page:owner,target,generation,epoch})=parent else{return Err("unreceived frame capability omitted parent".into())};
                if owner!=page{return Err("foreign unreceived frame parent".into())}
                Q::RestoreFrameDocument{token:token.clone(),parent:oryn_js::worker_wire::Creator::Window{target,document_generation:generation,stream_epoch:epoch},frame}
            }else{Q::RestoreDocument{token:token.clone()}};
            (request,Some(epoch))
        };
        self.complete_cached_document_authority_guarded(page,request,expected_epoch).map(|_|())
    }
}
