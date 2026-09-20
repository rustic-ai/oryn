// A root response stays with its actor transaction until both departure events
// finish. The staged history identity is the identity exposed to pageswap.
#[cfg(feature="v8-host")]
struct RootDepartureSource {
    identity:Option<(oryn_js::realm::RealmId,u64,u64)>,
    target_name:String,
    forms:Vec<oryn_js::form_state::SavedFormControl>,
    frames:Vec<oryn_js::form_state::SavedFrameFormState>,
}
#[cfg(feature="v8-host")]
struct RootDeparture {
    source_generation:u64,
    source_epoch:Option<u64>,
    source_identity:Option<(oryn_js::realm::RealmId,u64,u64)>,
    url:String,
    cached:bool,
    generation:u64,
    history:Vec<NavigationHistoryEntry>,
    index:usize,
    persisted:bool,
    swap:Option<oryn_js::v8_bindings::NativePageSwap>,
    hide:Option<crate::host::NativePageTransition>,
    unload:Option<crate::host::NativePageTransition>,
    unload_guards:Vec<oryn_js::v8_bindings::UnloadGuard>,
    phase:u8,
    source:Option<RootDepartureSource>,
}
#[cfg(feature="v8-host")]
impl PageState {
    fn prepare_root_departure(&mut self,url:&str,history:HistoryUpdate,cross_origin_redirect:bool,cached:bool)->Result<RootDeparture,RuntimeError>{
        let generation=if cached{self.document_generation}else{advance_document_generation(self.last_issued_document_generation,self.config.document_generation_base)?};
        self.prepare_root_departure_at_generation(url,history,cross_origin_redirect,cached,generation)
    }
    fn prepare_root_departure_at_generation(&mut self,url:&str,history:HistoryUpdate,cross_origin_redirect:bool,cached:bool,generation:u64)->Result<RootDeparture,RuntimeError>{
        self.ensure_cached_retirement_capacity()?;
        let (entries,index)=if cached {
            let HistoryUpdate::Preserve(index)=history else{return Err(RuntimeError::HistoryBoundary)};
            let destination=self.back_forward_cache.get_mut(&index).filter(|entry|entry.reusable).ok_or(RuntimeError::HistoryBoundary)?;
            if destination.history_publications.len()>=128{return Err(RuntimeError::ProtocolViolation("native history publication capacity".into()))}
            destination.history_publications.try_reserve(1).map_err(|_|RuntimeError::ProtocolViolation("native history publication allocation".into()))?;
            (self.history.clone(),index)
        }else{self.prepare_navigation_history(url,history,generation)?};
        let mut job=RootDeparture{source_generation:self.document_generation,source_epoch:self.executable.as_ref().and_then(|page|page.native_navigation_load_epoch()),source_identity:self.executable.as_ref().and_then(|page|page.form_history_root_identity()),url:url.into(),cached,generation,history:entries,index,persisted:false,swap:None,hide:None,unload:None,unload_guards:Vec::new(),phase:0,source:None};
        if let Some(executable)=self.executable.as_mut(){
            Self::check_navigation_operation(executable,&self.script_navigation_operation)?;
            job.persisted=!matches!(history,HistoryUpdate::Reload)&&executable.document_load_complete()&&self.pending_navigation_response.is_none();
            let disclose=!cross_origin_redirect&&executable.can_disclose_navigation_destination(url);
            if disclose{executable.stage_swap_destination(job.history[index].navigation.clone().ok_or(RuntimeError::HistoryBoundary)?).map_err(map_host_error)?;}
            let kind=match history{HistoryUpdate::Push=>"push",HistoryUpdate::Replace=>"replace",HistoryUpdate::Reload=>"reload",HistoryUpdate::Preserve(_)=>"traverse"};
            job.swap=Some(executable.begin_page_swap(url,kind,disclose).map_err(map_host_error)?);
        }else{job.phase=3;}
        Ok(job)
    }
    fn poll_root_departure(&mut self,job:&mut RootDeparture)->Result<bool,RuntimeError>{
        if self.executable.as_ref().and_then(|page|page.form_history_root_identity())!=job.source_identity||self.document_generation!=job.source_generation||self.executable.as_ref().and_then(|page|page.native_navigation_load_epoch())!=job.source_epoch{return Err(RuntimeError::NavigationCancelled)}
        let Some(executable)=self.executable.as_mut()else{return Ok(job.phase==3)};
        Self::check_navigation_operation(executable,&self.script_navigation_operation)?;
        while job.phase<2{
            #[cfg(test)]
            if job.phase==1&&ROOT_DEPARTURE_FAULT.with(|fault|fault.replace(false)){return Err(RuntimeError::EvaluationUnavailable)}
            if job.phase==0{
                let event=job.swap.as_mut().ok_or(RuntimeError::NavigationCancelled)?;
                if !executable.poll_page_swap(event).map_err(map_host_error)?{return Ok(false)}
                if event.canceled(){return Err(RuntimeError::NavigationCancelled)}
            }else{
                if job.hide.is_none(){
                    job.unload_guards=executable.begin_native_unload_tree().map_err(map_host_error)?;
                    job.hide=Some(executable.prepare_native_page_transition("pagehide",job.persisted).map_err(map_host_error)?);
                }
                if !executable.resume_native_page_transition(job.hide.as_mut().unwrap()).map_err(map_host_error)?{return Ok(false)}
            }
            if executable.form_history_root_identity()!=job.source_identity||executable.native_navigation_load_epoch()!=job.source_epoch{return Err(RuntimeError::NavigationCancelled)}
            job.phase+=1;
            Self::check_navigation_operation(executable,&self.script_navigation_operation)?;
        }
        if job.phase==2{
            if job.source.is_none(){
                let target_name=executable.capture_window_name_history().map_err(map_host_error)?;
                let forms=executable.capture_form_history().map_err(map_host_error)?;
                let frames=executable.capture_child_form_history().map_err(map_host_error)?;
                let forms_bytes=oryn_js::form_state::snapshot_bytes(&forms).map_err(|resource|RuntimeError::WorkerLimitExceeded{resource:resource.into()})?;
                let frame_bytes=oryn_js::form_state::frame_snapshot_bytes(&frames).map_err(|resource|RuntimeError::WorkerLimitExceeded{resource:resource.into()})?;
                if forms_bytes.checked_add(frame_bytes).is_none_or(|bytes|bytes>oryn_js::form_state::MAX_FORM_SNAPSHOT_BYTES){return Err(RuntimeError::WorkerLimitExceeded{resource:"form history bytes".into()})}
                job.source=Some(RootDepartureSource{identity:job.source_identity,target_name,forms,frames});
            }
            if !job.persisted{
                if job.unload.is_none(){job.unload=Some(executable.prepare_native_page_transition("unload",false).map_err(map_host_error)?);}
                if !executable.resume_native_page_transition(job.unload.as_mut().unwrap()).map_err(map_host_error)?{return Ok(false)}
                if executable.form_history_root_identity()!=job.source_identity||executable.native_navigation_load_epoch()!=job.source_epoch{return Err(RuntimeError::NavigationCancelled)}
            }
            #[cfg(test)] if ROOT_SOURCE_CAPTURE_FAULT.with(|fault|fault.replace(false)){return Err(RuntimeError::EvaluationUnavailable)}
            self.sync_executable_semantics()?;
            if self.executable.as_ref().and_then(|page|page.form_history_root_identity())!=job.source_identity{return Err(RuntimeError::NavigationCancelled)}
            job.unload_guards.clear();job.phase=3;
        }
        Ok(true)
    }
    // Compatibility entry points are synchronous. Actor callers prepare and
    // poll the same job in NavigationTransactionPhase::Departure instead.
    fn complete_root_departure(&mut self,url:&str,history:HistoryUpdate,redirect:bool,cached:bool)->Result<RootDeparture,RuntimeError>{
        if let Some(job)=self.completed_root_departure.take(){
            if job.url!=url||job.cached!=cached||job.source_generation!=self.document_generation||job.phase!=3||job.source_identity!=self.executable.as_ref().and_then(|page|page.form_history_root_identity()){return Err(RuntimeError::NavigationCancelled)}
            if cached&&self.history.get(job.index).and_then(|entry|entry.navigation.as_ref()).zip(job.history.get(job.index).and_then(|entry|entry.navigation.as_ref())).is_none_or(|(current,expected)|current.id!=expected.id||current.key!=expected.key){return Err(RuntimeError::NavigationCancelled)}
            if self.executable.as_ref().and_then(|page|page.native_navigation_load_epoch())!=job.source_epoch{return Err(RuntimeError::NavigationCancelled)}
            return Ok(job)
        }
        let mut job=self.prepare_root_departure(url,history,redirect,cached)?;
        let deadline=Instant::now().checked_add(self.config.navigation_timeout).ok_or(RuntimeError::NavigationCancelled)?;
        loop{
            match self.poll_root_departure(&mut job){Ok(true)=>return Ok(job),Ok(false)=>{},Err(error)=>{
                if (job.phase==0&&!job.swap.as_ref().is_none_or(|event|event.retry_safe()))||matches!(error,RuntimeError::NavigationCancelled){return Err(error)}
            }}
            if Instant::now()>=deadline{return Err(RuntimeError::WorkerLimitExceeded{resource:"document departure wall time".into()})}
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(all(test,feature="v8-host"))]
thread_local!{static ROOT_DEPARTURE_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static FAILED_INSTALL_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static ROOT_SUSPENSION_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static ROOT_SOURCE_CAPTURE_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static ROOT_RESTORE_PUBLICATION_FAULT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

#[cfg(all(test,feature="v8-host"))]
thread_local!{static LAST_INSTALL_RECOVERY_ERROR:std::cell::RefCell<Option<String>>=const{std::cell::RefCell::new(None)};}

#[cfg(all(test,feature="v8-host"))]
mod retained_root_departure_tests {
    use super::*;
    fn page()->PageState{
        let mut page=PageState::new(PageId(708),None,BrowserConfig::default(),ContextOptions::default(),None,#[cfg(unix)] None);
        page.load_document("https://departure.test/one".into(),"<p>incumbent</p>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
        page.executable.as_mut().unwrap().evaluate("globalThis.departures=[];addEventListener('pageswap',()=>departures.push('swap'));addEventListener('pagehide',()=>departures.push('hide'));'ready'").unwrap();
        page
    }
    #[test]fn nonpersisted_reload_dispatches_unload_after_pagehide_once(){
        let mut page=page();page.executable.as_mut().unwrap().evaluate("addEventListener('unload',()=>departures.push('unload'));'ready'").unwrap();
        let mut job=page.prepare_root_departure("https://departure.test/two",HistoryUpdate::Reload,false,false).unwrap();assert!(!job.persisted);
        assert!(page.poll_root_departure(&mut job).unwrap());let interval=job.unload.as_ref().unwrap().unload_interval().unwrap();assert!(interval.0<=interval.1);
        assert!(page.poll_root_departure(&mut job).unwrap());assert_eq!(job.unload.as_ref().unwrap().unload_interval(),Some(interval));assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap,hide,unload");
    }
    #[test]fn persisted_departure_skips_unload_even_with_registered_handlers(){
        let mut page=page();for _ in 0..32{page.executable.as_mut().unwrap().drain_tasks().unwrap();if page.executable.as_ref().unwrap().document_load_complete(){break;}}
        page.executable.as_mut().unwrap().evaluate("addEventListener('unload',()=>departures.push('unexpected-unload'));'ready'").unwrap();
        let mut job=page.prepare_root_departure("https://departure.test/two",HistoryUpdate::Push,false,false).unwrap();assert!(job.persisted);assert!(page.poll_root_departure(&mut job).unwrap());assert!(job.unload.is_none());assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap,hide");
    }
    #[test]fn pagehide_ignores_destructive_writes_for_persisted_and_nonpersisted_departure(){
        for history in [HistoryUpdate::Push,HistoryUpdate::Reload]{
            let mut page=page();for _ in 0..32{page.executable.as_mut().unwrap().drain_tasks().unwrap();if page.executable.as_ref().unwrap().document_load_complete(){break;}}
            page.executable.as_mut().unwrap().evaluate("onpagehide=()=>{document.open();document.write('<p>incorrect replacement</p>');departures.push(document.querySelector('p').textContent)};true").unwrap();
            let mut job=page.prepare_root_departure("https://departure.test/two",history,false,false).unwrap();assert!(page.poll_root_departure(&mut job).unwrap());
            assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap,hide,incumbent");
            drop(job);assert_eq!(page.executable.as_mut().unwrap().evaluate("document.open();String(document.childNodes.length)").unwrap(),"0","completed departure releases its guards");
        }
    }
    #[test]fn abandoned_post_hide_retry_releases_unload_guard_without_replaying_callbacks(){
        let mut page=page();let mut job=page.prepare_root_departure("https://departure.test/two",HistoryUpdate::Reload,false,false).unwrap();ROOT_SOURCE_CAPTURE_FAULT.with(|fault|fault.set(true));assert!(page.poll_root_departure(&mut job).is_err());
        assert_eq!(page.executable.as_mut().unwrap().evaluate("document.open();document.querySelector('p').textContent").unwrap(),"incumbent");drop(job);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("document.open();String(document.childNodes.length)").unwrap(),"0");
    }
    fn arm(){ROOT_DEPARTURE_FAULT.with(|fault|fault.set(true));}
    fn next(page:&mut PageState,transaction:&mut NavigationTransaction)->std::task::Poll<Result<NavigationResult,RuntimeError>>{
        if let NavigationTransactionPhase::Departure{retry,..}=&mut transaction.phase{*retry=None;}
        page.step_navigation_transaction(transaction)
    }
    #[test]
    fn actor_keeps_buffered_body_and_history_identity_across_departure_fault(){
        let mut page=page();let generation=page.document_generation;
        page.begin_navigation_resource("data:text/html,<p id=destination>retained-body</p>".into(),HistoryUpdate::Push,None,None,None,None).unwrap();
        let mut transaction=page.pending_navigation_transaction.take().unwrap();
        arm();let mut paused=false;
        for _ in 0..16{
            assert!(next(&mut page,&mut transaction).is_pending());
            if let NavigationTransactionPhase::Departure{input:RootNavigationInput::Buffered{bytes,..},job:Some(job),..}=&transaction.phase
                && job.phase==1{assert!(String::from_utf8_lossy(bytes).contains("retained-body"));assert_eq!(page.document_generation,generation);assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap");paused=true;break;}
        }
        assert!(paused);let expected=match &transaction.phase{NavigationTransactionPhase::Departure{job:Some(job),..}=>job.history[job.index].navigation.as_ref().unwrap().id.clone(),_=>unreachable!()};
        let mut complete=false;for _ in 0..16{match next(&mut page,&mut transaction){std::task::Poll::Pending=>{},std::task::Poll::Ready(result)=>{result.unwrap();complete=true;break}}}assert!(complete);
        assert_eq!(page.history[page.history_index.unwrap()].navigation.as_ref().unwrap().id,expected);
        let old=page.back_forward_cache.values_mut().find(|entry|entry.document_generation==generation).unwrap();
        // A standalone cached realm is suspended; inspect only after native restoration.
        assert!(old.reusable);assert_ne!(page.document_generation,generation);
    }
    #[test]
    fn cached_actor_does_not_consume_destination_or_replay_swap_on_retry(){
        let mut page=page();page.load_document("https://departure.test/two".into(),"<p>second</p>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
        page.executable.as_mut().unwrap().evaluate("globalThis.departures=[];addEventListener('pageswap',()=>departures.push('swap'));addEventListener('pagehide',()=>departures.push('hide'));'ready'").unwrap();
        let destination=page.back_forward_cache[&0].document_generation;let source=page.document_generation;
        page.begin_navigation_resource("https://departure.test/one".into(),HistoryUpdate::Preserve(0),None,None,None,None).unwrap();let mut transaction=page.pending_navigation_transaction.take().unwrap();arm();
        for _ in 0..16{assert!(next(&mut page,&mut transaction).is_pending());if matches!(&transaction.phase,NavigationTransactionPhase::Departure{job:Some(job),..} if job.phase==1){break}}
        assert_eq!(page.document_generation,source);assert_eq!(page.back_forward_cache[&0].document_generation,destination);assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap");
        let mut done=false;for _ in 0..16{match next(&mut page,&mut transaction){std::task::Poll::Pending=>{},std::task::Poll::Ready(result)=>{result.unwrap();done=true;break}}}assert!(done);assert_eq!(page.document_generation,destination);
    }
    #[test]
    fn post_hide_capture_failure_preserves_callback_prefix_and_actor_input(){
        let mut page=page();page.begin_navigation_resource("data:text/html,<p>owned</p>".into(),HistoryUpdate::Push,None,None,None,None).unwrap();
        let mut transaction=page.pending_navigation_transaction.take().unwrap();ROOT_SOURCE_CAPTURE_FAULT.with(|fault|fault.set(true));
        let mut retained=false;for _ in 0..16{assert!(next(&mut page,&mut transaction).is_pending());if matches!(&transaction.phase,NavigationTransactionPhase::Departure{input:RootNavigationInput::Buffered{..},job:Some(job),..} if job.phase==2&&job.source.is_some()){retained=true;break}}
        assert!(retained);assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap,hide");
        let NavigationTransactionPhase::Departure{job:Some(job),..}=&mut transaction.phase else{unreachable!()};
        assert!(page.poll_root_departure(job).unwrap());assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"swap,hide");
    }
    #[test]
    fn failed_provisional_executable_stays_owned_until_rollback_can_run(){
        let mut page=page();let index=page.history_index.unwrap();let generation=page.document_generation;page.suspend_document(true).unwrap();
        let failed=crate::executable::RustExecutableDocument::load("<p>failed provisional</p>").unwrap();
        page.failed_install_recovery=Some(FailedInstallRecovery{executable:Some(failed),failed_realm:None,failed_authority_retired:false,#[cfg(unix)]failed_authority_requests:None,source:Some((index,generation,true)),source_published:false,discard_cache:None,retry:None});
        FAILED_INSTALL_FAULT.with(|fault|fault.set(true));page.poll_failed_install_recovery();
        assert!(page.failed_install_recovery.as_ref().unwrap().executable.is_some());assert!(page.executable.is_none());assert!(page.back_forward_cache.contains_key(&index));
        page.failed_install_recovery.as_mut().unwrap().retry=None;page.poll_failed_install_recovery();
        assert!(page.failed_install_recovery.is_none());assert_eq!(page.document_generation,generation);assert!(page.executable.is_some());assert_eq!(page.history_publications.len(),1);
        page.poll_failed_install_recovery();assert_eq!(page.history_publications.len(),1,"rollback event is queued once");
    }
    #[test]
    fn failed_suspension_keeps_original_executable_until_native_resume(){
        let mut page=page();let generation=page.document_generation;let realm=page.executable.as_ref().unwrap().agent_root_realm();
        ROOT_SUSPENSION_FAULT.with(|fault|fault.set(true));assert!(page.suspend_document(true).is_err());
        assert!(page.root_source_resume.is_some());assert_eq!(page.executable.as_ref().unwrap().agent_root_realm(),realm);
        page.poll_source_resume();assert!(page.root_source_resume.is_none());assert_eq!(page.document_generation,generation);
        page.poll_history_publications().unwrap();assert_eq!(page.executable.as_mut().unwrap().evaluate("departures.join(',')").unwrap(),"hide");
    }
    #[test]
    fn rollback_pageshow_preparation_failure_retains_original_publication(){
        let mut page=page();page.executable.as_mut().unwrap().evaluate("globalThis.shows=[];onpageshow=e=>shows.push(e.persisted);'ready'").unwrap();
        page.queue_rollback_pageshow(false).unwrap();
        ROOT_RESTORE_PUBLICATION_FAULT.with(|fault|fault.set(true));
        assert!(page.poll_history_publications().is_err());assert_eq!(page.history_publications.len(),1);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(shows.length)").unwrap(),"0");
        page.history_publications.front_mut().unwrap().retry=None;page.poll_history_publications().unwrap();
        assert!(page.history_publications.is_empty());assert_eq!(page.executable.as_mut().unwrap().evaluate("shows.join(',')").unwrap(),"false");
        page.poll_history_publications().unwrap();assert_eq!(page.executable.as_mut().unwrap().evaluate("String(shows.length)").unwrap(),"1");
    }
    #[test]
    fn document_replacement_cancels_original_departure_suffix(){
        let mut page=page();let mut job=page.prepare_root_departure("https://departure.test/two",HistoryUpdate::Push,false,false).unwrap();arm();
        assert!(page.poll_root_departure(&mut job).is_err());assert_eq!(job.phase,1);
        page.executable.as_mut().unwrap().evaluate("document.open();document.write('<p>replacement</p>');document.close();globalThis.hides=0;onpagehide=()=>hides++;'replaced'").unwrap();
        assert!(matches!(page.poll_root_departure(&mut job),Err(RuntimeError::NavigationCancelled)));
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(hides)").unwrap(),"0");
    }
}

#[cfg(feature="v8-host")]
struct RootSourceResume {realm:oryn_js::realm::RealmId,generation:u64,load_epoch:Option<u64>,persisted:bool,resumed:bool,retry:Option<Instant>}
#[cfg(feature="v8-host")]
impl PageState {
    fn retain_source_resume(&mut self,persisted:bool){
        if let Some(executable)=&self.executable{self.root_source_resume=Some(RootSourceResume{realm:executable.agent_root_realm(),generation:self.document_generation,load_epoch:executable.native_navigation_load_epoch(),persisted,resumed:false,retry:None});}
    }
    fn poll_source_resume(&mut self){
        let Some(mut pending)=self.root_source_resume.take()else{return};
        if pending.retry.is_some_and(|retry|retry>Instant::now()){self.root_source_resume=Some(pending);return}
        if self.document_generation!=pending.generation||self.executable.as_ref().is_none_or(|page|page.agent_root_realm()!=pending.realm||page.native_navigation_load_epoch()!=pending.load_epoch){return}
        let result=(||->Result<(),RuntimeError>{
            if !pending.resumed{
                let executable=self.executable.as_mut().ok_or(RuntimeError::EvaluationUnavailable)?;
                executable.restore_cached_authority().map_err(map_host_error)?;
                executable.finish_cached_authority_restore().map_err(map_host_error)?;
                executable.resume_from_back_forward_cache();pending.resumed=true;
            }
            self.queue_rollback_pageshow(pending.persisted)
        })();
        if matches!(&result,Err(RuntimeError::NavigationCancelled)){
            let executable=self.executable.take();
            // No predecessor will be restored on this cancellation path;
            // retain the existing queued physical-retirement ownership.
            self.failed_install_recovery=Some(FailedInstallRecovery{executable,failed_realm:Some(pending.realm),failed_authority_retired:true,#[cfg(unix)]failed_authority_requests:None,source:None,source_published:false,discard_cache:None,retry:None});
            self.poll_failed_install_recovery();
        }else if result.is_err(){pending.retry=Some(Instant::now()+Duration::from_millis(25));self.root_source_resume=Some(pending);}
    }
    fn source_resume_delay(&self)->Option<Duration>{self.root_source_resume.as_ref().map(|pending|pending.retry.map(|retry|retry.saturating_duration_since(Instant::now())).unwrap_or(Duration::ZERO))}
}

#[cfg(feature="v8-host")]
struct FailedInstallRecovery {
    executable:Option<crate::executable::RustExecutableDocument>,
    failed_realm:Option<oryn_js::realm::RealmId>,
    failed_authority_retired:bool,
    #[cfg(unix)]
    failed_authority_requests:Option<(crate::context_service::SharedContextClient,PageId,std::collections::VecDeque<oryn_js::message_wire::MessageRequest>)>,
    source:Option<(usize,u64,bool)>,source_published:bool,discard_cache:Option<(usize,u64)>,retry:Option<Instant>,
}
#[cfg(feature="v8-host")]
impl PageState {
    fn poll_failed_install_recovery(&mut self){
        let Some(mut pending)=self.failed_install_recovery.take()else{return};
        if pending.retry.is_some_and(|retry|retry>Instant::now()){self.failed_install_recovery=Some(pending);return}
        let result=(||->Result<(),RuntimeError>{
            if let Some(executable)=pending.executable.as_mut(){
                #[cfg(test)] if FAILED_INSTALL_FAULT.with(|fault|fault.replace(false)){return Err(RuntimeError::EvaluationUnavailable)}
                if pending.failed_realm.is_some(){
                    if self.pending_agent_host.is_some(){return Err(RuntimeError::ProtocolViolation("failed installation host slot occupied".into()))}
                    self.pending_agent_host=Some(executable.take_agent_host().map_err(map_host_error)?);
                }
                pending.executable.take();
            }
            if let Some(realm)=pending.failed_realm
                && !pending.failed_authority_retired{
                    #[cfg(unix)]
                    if self.standalone_root.is_none(){
                        self.pending_agent_host.as_mut()
                            .ok_or_else(||RuntimeError::ProtocolViolation("failed root host unavailable".into()))?
                            .revoke_failed_agent_root_authority(realm,&mut pending.failed_authority_requests).map_err(map_host_error)?;
                    }
                    #[cfg(not(unix))] let _=realm;
                    // This ACK phase precedes any predecessor restore. Keep the
                    // realm itself until restore has detached/rebound its proxy.
                    pending.failed_authority_retired=true;
                }
            if let Some((index,generation,persisted))=pending.source{
                if self.executable.is_none(){
                    if self.back_forward_cache.get(&index).is_some_and(|entry|entry.document_generation==generation){
                        if let Some((destination,destination_generation))=pending.discard_cache
                            && self.back_forward_cache.get(&destination).is_some_and(|entry|entry.document_generation==destination_generation){
                                self.repark_failed_cached_destination(destination)?;
                            }
                        match self.restore_parked_incumbent(index){
                            Ok(())=>pending.source_published=true,
                            Err(RuntimeError::NavigationCancelled)=>{self.evict_cached_document(index);pending.source=None;},
                            Err(error)=>return Err(error),
                        }
                    }
                    else{pending.source=None;}
                }
                if self.executable.is_some()&&self.document_generation==generation&&!pending.source_published{self.queue_rollback_pageshow(persisted)?;pending.source_published=true;}
            }
            if let Some((index,generation))=pending.discard_cache.take()&& self.back_forward_cache.get(&index).is_some_and(|entry|entry.document_generation==generation){self.evict_cached_document(index);}
            if let Some(realm)=pending.failed_realm.take(){self.enqueue_retired_agent_root(realm);}
            Ok(())
        })();
        #[cfg(test)] LAST_INSTALL_RECOVERY_ERROR.with(|last|*last.borrow_mut()=result.as_ref().err().map(ToString::to_string));
        if result.is_err(){pending.retry=Some(Instant::now()+Duration::from_millis(25));self.failed_install_recovery=Some(pending);}
    }
    fn failed_install_delay(&self)->Option<Duration>{self.failed_install_recovery.as_ref().map(|pending|pending.retry.map(|retry|retry.saturating_duration_since(Instant::now())).unwrap_or(Duration::ZERO))}
}
