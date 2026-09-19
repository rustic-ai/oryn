// A successful document stays installed while discard retries. Failed initial
// construction retains its exact pre-commit snapshot until restoration ACK.
#[cfg(all(unix,feature="v8-host"))]
struct InitialDocumentSnapshot {
    url:String,generation:u64,revision:u64,lifecycle:Vec<LifecycleState>,
    csp:crate::csp::CspPolicySet,isolation:crate::agent_isolation::DocumentIsolationPolicy,
    headers:Vec<(String,String)>,referrer:crate::referrer::ReferrerPolicy,
    fonts:Vec<oryn_layout::WebFont>,challenge:ChallengeAssessment,scroll:ScrollState,
    quiet:Option<Instant>,broker:Option<SharedNetworkBroker>,
}
#[cfg(all(unix,feature="v8-host"))]
struct InitialDocumentCleanup {
    client:SharedContextClient,token:oryn_js::message_wire::ParkedDocumentToken,
    snapshot:Option<InitialDocumentSnapshot>,expected_generation:u64,retry:Option<Instant>,
}
#[cfg(all(unix,feature="v8-host"))]
impl PageState {
    fn poll_initial_document_cleanup(&mut self){
        let Some(mut pending)=self.initial_document_cleanup.take()else{return};
        if pending.retry.is_some_and(|retry|retry>Instant::now()){self.initial_document_cleanup=Some(pending);return}
        let restore=pending.snapshot.is_some();
        let result=pending.client.request(if restore{crate::context_service::ContextServiceRequest::RestoreInitialDocument{page_id:self.page_id,token:pending.token.clone()}}else{crate::context_service::ContextServiceRequest::DiscardInitialDocument{page_id:self.page_id,token:pending.token.clone()}});
        if restore&&matches!(&result,Ok(crate::context_service::ContextServiceResponse::InitialDocumentRestorationObsolete)){return}
        let accepted=matches!((&result,restore),(Ok(crate::context_service::ContextServiceResponse::InitialDocumentRestored),true)|(Ok(crate::context_service::ContextServiceResponse::InitialDocumentDiscarded),false));
        if !accepted{pending.retry=Some(Instant::now()+Duration::from_millis(25));self.initial_document_cleanup=Some(pending);return}
        if let Some(snapshot)=pending.snapshot{
            // Never overwrite an independently installed document with a late
            // cleanup response. This is original rollback state, not history.
            if self.executable.is_none()&&self.document_generation==pending.expected_generation{
                self.url=snapshot.url;self.document_generation=snapshot.generation;self.revision=snapshot.revision;self.lifecycle=snapshot.lifecycle;
                self.csp=snapshot.csp;self.script_isolation=snapshot.isolation;self.response_policy_headers=snapshot.headers;self.referrer_policy=snapshot.referrer;
                self.web_fonts=snapshot.fonts;self.last_challenge=snapshot.challenge;self.scroll_state=snapshot.scroll;self.network_quiet_since=snapshot.quiet;self.broker=snapshot.broker;
            }
        }
    }
    fn initial_document_cleanup_delay(&self)->Option<Duration>{self.initial_document_cleanup.as_ref().map(|pending|pending.retry.map(|retry|retry.saturating_duration_since(Instant::now())).unwrap_or(Duration::ZERO))}
}

#[cfg(all(unix,feature="v8-host",test))]
mod initial_cleanup_tests {
    use super::*;
    use crate::context_service::{ContextServiceRequest as Q,ContextServiceResponse as R,ContextTransport};
    struct LostReply{inner:SharedContextClient,token:oryn_js::message_wire::ParkedDocumentToken,calls:std::sync::atomic::AtomicUsize}
    impl ContextTransport for LostReply{
        fn request_routed(&self,_:Option<&crate::worker_route::WorkerRoute>,request:Q)->Result<R,RuntimeError>{self.request(request)}
        fn request(&self,request:Q)->Result<R,RuntimeError>{
            let Q::RestoreInitialDocument{token,..}=&request else{return self.inner.request(request)};assert_eq!(token,&self.token);
            let reply=self.inner.request(request)?;
            match self.calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst){0=>Err(RuntimeError::ContextService("injected lost initial restore ACK".into())),1=>Ok(R::PageCommitted),_=>Ok(reply)}
        }
    }
    #[test]
    fn initial_snapshot_survives_lost_and_wrong_restore_ack(){
        let context=Browser::in_process_probe().new_context();let id=context.new_page().page_id();let token=context.park_initial_document(id).unwrap();
        context.install_document_security(id,"https://failed.test/".into(),&[]).unwrap();
        let transport=Arc::new(LostReply{inner:SharedContextClient::direct(context.clone(),id),token:token.clone(),calls:std::sync::atomic::AtomicUsize::new(0)});
        let client=SharedContextClient::for_test_transport(transport.clone(),crate::worker_route::WorkerRoute::issue(778,id.0,0).unwrap());
        let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client.clone()));
        let snapshot=InitialDocumentSnapshot{url:"about:blank".into(),generation:page.document_generation,revision:page.revision,lifecycle:page.lifecycle.clone(),csp:page.csp.clone(),isolation:page.script_isolation,headers:vec![("referrer-policy".into(),"no-referrer".into())],referrer:page.referrer_policy,fonts:Vec::new(),challenge:page.last_challenge.clone(),scroll:page.scroll_state,quiet:page.network_quiet_since,broker:page.broker.clone()};
        page.document_generation=77;page.url="https://failed.test/".into();page.initial_document_cleanup=Some(InitialDocumentCleanup{client,token,snapshot:Some(snapshot),expected_generation:77,retry:None});
        page.poll_initial_document_cleanup();assert!(page.initial_document_cleanup.is_some());assert_eq!(page.document_generation,77);assert!(page.has_pending_page_work());
        page.initial_document_cleanup.as_mut().unwrap().retry=None;page.poll_initial_document_cleanup();assert!(page.initial_document_cleanup.is_some());assert_eq!(page.url,"https://failed.test/");
        page.initial_document_cleanup.as_mut().unwrap().retry=None;page.poll_initial_document_cleanup();assert!(page.initial_document_cleanup.is_none());assert_eq!(page.url,"about:blank");assert_ne!(page.document_generation,77);assert_eq!(page.response_policy_headers,vec![("referrer-policy".into(),"no-referrer".into())]);assert_eq!(transport.calls.load(std::sync::atomic::Ordering::SeqCst),3);
    }
}
