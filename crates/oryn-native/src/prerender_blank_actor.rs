// Accepted blank activations leave source-document cancellation custody and
// retain the original parser/executable until their related actor is installed.
struct BlankTransfer {
    source:crate::host::PrerenderSourceSnapshot,
    receipt:crate::context_service::BlankPrerenderActivation,
    roster:prerender_roster::Roster,
    client:SharedContextClient,
    source_page:PageId,
    source_url:String,
    broker:SharedNetworkBroker,
    policy:crate::referrer::ReferrerPolicy,
    config:BrowserConfig,
    options:ContextOptions,
    context:WeakBrowserContext,
    candidate:Option<Box<prerender_load::CandidateDocument>>,
    reservation:Option<prerender_load::Reservation>,
    pending:Option<navigation_fetch::PendingPolicy>,
    tree:Option<crate::host::NativePrerenderActivationTree>,
    realm:Option<oryn_js::realm::RealmId>,
    phase:u8,
    retry:Option<Instant>,
}
impl BlankTransfer{
    fn take(state:&mut PageState,context:WeakBrowserContext,mut source:crate::host::PrerenderSourceSnapshot,receipt:crate::context_service::BlankPrerenderActivation)->Option<Self>{
        let client=state.context_client.clone()?;let broker=state.broker.clone()?;
        let roster=state.prerender_candidates.detach_blank(&mut source,&receipt)?;
        Some(Self{source,receipt,roster,client,source_page:state.page_id,source_url:state.url.clone(),broker,policy:state.referrer_policy,config:state.config.clone(),options:context.upgrade().map(|context|context.inner.options.clone()).unwrap_or_default(),context,candidate:None,reservation:None,pending:None,tree:None,realm:None,phase:0,retry:None})
    }
    fn cancel(&mut self,host:&mut Option<crate::host::RawV8Host>)->Result<bool,RuntimeError>{
        self.roster.poll(prerender_roster::PrerenderPollInput { snapshot: None, host, client: self.client.clone(), page: self.source_page, broker: self.broker.clone(), source_url: &self.source_url, policy: self.policy, config: &self.config });
        if self.roster.delay().is_some(){return Ok(false)}
        if self.candidate.is_some()&& let Some(realm)=self.realm{host.as_mut().ok_or(RuntimeError::EvaluationUnavailable)?.retire_agent_root(realm).map_err(map_host_error)?;}
        self.candidate.take();self.reservation.take();Ok(true)
    }
    fn poll(&mut self,host:&mut Option<crate::host::RawV8Host>,entries:&mut Vec<Entry>,actor:&WorkerRoute,control:&std::sync::mpsc::SyncSender<Attach>)->Result<bool,RuntimeError>{
        use crate::context_service::{ContextServiceRequest as Q,ContextServiceResponse as R};
        if self.retry.is_some_and(|time|time>Instant::now()){return Ok(false)}
        self.retry=None;
        if self.phase==255{return self.cancel(host)}
        if self.phase==0{
            self.roster.poll(prerender_roster::PrerenderPollInput { snapshot: Some(self.source.clone()), host, client: self.client.clone(), page: self.source_page, broker: self.broker.clone(), source_url: &self.source_url, policy: self.policy, config: &self.config });
            let Some(candidate)=self.roster.take_for_blank_activation(&self.source,&self.receipt.url)else{if self.roster.delay().is_none(){return Err(RuntimeError::NavigationCancelled)}return Ok(false)};
            let matches=candidate.reservation.page_id==self.receipt.page_id&&candidate.reservation.token==self.receipt.token&&candidate.reservation.generation==self.receipt.generation&&candidate.reservation.epoch==self.receipt.epoch;
            self.realm=Some(candidate.executable.agent_root_realm());self.candidate=Some(candidate);
            if !matches{self.phase=255;return self.cancel(host)}
            self.phase=1;
        }
        if self.phase==1{
            if self.pending.is_none(){self.pending=Some(navigation_fetch::PendingPolicy::start_prerender_activation(self.client.clone(),Q::ActivateBlankPrerender{page_id:self.source_page,activation:self.receipt.clone()})?);}
            match self.pending.as_mut().unwrap().poll(){std::task::Poll::Pending=>return Ok(false),std::task::Poll::Ready(result)=>{self.pending=None;match result?{R::PrerenderActivated=>self.phase=2,R::PrerenderActivationObsolete=>return Err(RuntimeError::NavigationCancelled),_=>return Err(RuntimeError::ProtocolViolation("blank activation ACK mismatch".into()))}}}
        }
        #[cfg(test)] if self.phase==2&&BLANK_TRANSFER_FAULTS.with(|faults|{let value=faults.get();faults.set(value&!1);value&1!=0}){return Ok(false)}
        if self.phase==2{
            let owned=host.as_mut().ok_or(RuntimeError::EvaluationUnavailable)?;
            owned.publish_blank_prerender_root(self.realm.unwrap(),&self.receipt).map_err(map_host_error)?;
            if self.tree.is_none(){self.tree=Some(owned.prepare_prerender_activation(self.realm.unwrap()).map_err(map_host_error)?);}
            let candidate=self.candidate.as_ref().unwrap();
            let admission=candidate.reservation.destination.as_ref().ok_or_else(||RuntimeError::ProtocolViolation("blank destination admission missing".into()))?;
            if admission.actor_route!=*actor||entries.len()>=256{return Err(RuntimeError::ProtocolViolation("blank actor admission unavailable".into()))}
            let route=admission.route.clone();let direct=admission.direct;
            if registry().lock().map_err(|_|RuntimeError::ContextService("related actor registry poisoned".into()))?.contains_key(&route){return Err(RuntimeError::ProtocolViolation("blank actor route already installed".into()))}
            // The candidate driver has lent its host to this actor turn.
            // Read its live identity from that owner before consuming custody.
            let realm=self.realm.unwrap();
            let (_,generation,epoch,true)=owned.automation_realm_identity(realm).ok_or(RuntimeError::NavigationCancelled)? else{return Err(RuntimeError::NavigationCancelled)};
            let root_identity=Some((u64::from(realm.0),generation,epoch));
            let history=owned.navigation_history_snapshot().map_err(map_host_error)?;
            // This snapshot becomes the new blank actor's entire initial
            // session below; it has no predecessor from the source Window.
            // Install that same native session before activation publishes its
            // full count. Author access to History must not initialize it for us.
            owned.sync_session_history(history.clone()).map_err(map_host_error)?;
            // All fallible preflight precedes consuming the retained driver or
            // its direct receiver. A failed publication resumes this same state.
            let weak_direct=if direct{Some(self.context.upgrade().and_then(|context|context.inner.group.handle(self.receipt.page_id)).and_then(|page|page.sender).ok_or(RuntimeError::PageNotFound(self.receipt.page_id))?.downgrade())}else{None};
            let mut reservations=direct_reservations().lock().map_err(|_|RuntimeError::ContextService("blank actor reservations poisoned".into()))?;
            if direct&&!reservations.get(&route).is_some_and(|entry|entry.actor==*actor){return Err(RuntimeError::ProtocolViolation("blank direct receiver missing".into()))}
            let (sender,receiver,context)=if direct{let reservation=reservations.remove(&route).unwrap();(None,reservation.receiver,reservation.context)}else{let(sender,receiver)=mpsc::channel(self.config.worker_queue_capacity);(Some(sender),receiver,self.context.clone())};drop(reservations);
            let weak=sender.as_ref().map(|sender|sender.downgrade()).or(weak_direct).unwrap();
            let candidate=self.candidate.take().unwrap();
            let prerender_load::CandidateDocument{reservation,executable,url,status,content_type,headers,broker,csp,origin,request_state,capture,..}=*candidate;
            let mut state=PageState::new(self.receipt.page_id,None,self.config.clone(),self.options.clone(),Some(broker),Some(reservation.client.clone()));
            state.pending_navigation_response=Some(PendingNavigationResponse{generation:reservation.generation,root_identity,current:Url::parse(&url).expect("admitted prerender URL"),status,response_headers:headers.clone(),essence:content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase(),capture});
            state.response_policy_headers=crate::standalone_document_policy::retained_headers(&headers);
            state.adopt_agent_document(crate::runtime::AgentDocumentInputs { executable, url, generation: reservation.generation, origin, history, policy: csp, referrer: crate::referrer::ReferrerPolicy::default().after_response(&headers), isolation: crate::agent_isolation::DocumentIsolationPolicy::from_headers(&headers) });
            if let Some(index)=state.history_index{state.history[index].request_state=Some(request_state);}
            state.lifecycle=vec![LifecycleState::Commit];
            if state.executable.as_ref().is_some_and(|page|page.document_interactive()){state.lifecycle.push(LifecycleState::DomContentLoaded);}
            if state.executable.as_ref().is_some_and(|page|page.document_load_complete()){state.lifecycle.push(LifecycleState::Load);}
            registry().lock().unwrap_or_else(|error|error.into_inner()).insert(route.clone(),Published{actor:actor.clone(),sender:weak.clone(),control:control.clone(),claimed:direct});published().notify_all();
            entries.push(Entry{retiring:false,close_reply:None,retire_realms:Vec::new(),root:Root::new(state,receiver,weak),context:context.clone(),context_guard:if direct{None}else{context.upgrade()},route:Some(route),reserved_sender:sender,next_poll:std::time::Instant::now()});
            self.reservation=Some(reservation);self.phase=3;
        }
        #[cfg(test)] if self.phase==3&&BLANK_TRANSFER_FAULTS.with(|faults|{let value=faults.get();faults.set(value&!2);value&2!=0}){return Ok(false)}
        if self.phase==3{
            if self.pending.is_none(){self.pending=Some(navigation_fetch::PendingPolicy::start_prerender_activation(self.client.clone(),Q::PublishBlankPrerender{page_id:self.source_page,activation:self.receipt.clone()})?);}
            match self.pending.as_mut().unwrap().poll(){std::task::Poll::Pending=>return Ok(false),std::task::Poll::Ready(result)=>{self.pending=None;if !matches!(result?,R::PrerenderPublished){return Err(RuntimeError::ProtocolViolation("blank publication ACK mismatch".into()))}self.phase=4;}}
        }
        if self.phase==4{
            let owned=host.as_mut().ok_or(RuntimeError::EvaluationUnavailable)?;
            if !owned.resume_prerender_activation(self.tree.as_mut().unwrap()).map_err(map_host_error)?{return Ok(false)}
            self.phase=5;
        }
        if self.phase==5{
            if self.pending.is_none(){self.pending=Some(navigation_fetch::PendingPolicy::start_prerender_activation(self.client.clone(),Q::AcknowledgeBlankPrerender{page_id:self.source_page,activation:self.receipt.clone()})?);}
            match self.pending.as_mut().unwrap().poll(){std::task::Poll::Pending=>return Ok(false),std::task::Poll::Ready(result)=>{self.pending=None;if !matches!(result?,R::PrerenderAcknowledged){return Err(RuntimeError::ProtocolViolation("blank completion ACK mismatch".into()))}self.phase=6;}}
        }
        Ok(self.phase==6)
    }
}

#[cfg(test)]
thread_local!{static BLANK_TRANSFER_FAULTS:std::cell::Cell<u8>=const{std::cell::Cell::new(0)};}

#[cfg(all(test,feature="in-process-probe"))]
mod blank_transfer_tests{
    use super::*;
    use crate::host::JavaScriptHost;
    struct CandidateResponse(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::network::NetworkBroker for CandidateResponse{
        fn send(&self,request:crate::network::NetworkRequest)->Result<crate::network::NetworkResponse,crate::network::NetworkError>{
            self.0.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/html".into())],body:br#"<script>globalThis.saved=document;globalThis.savedWindow=window;globalThis.activations=0;document.addEventListener('prerenderingchange',()=>activations++)</script><body>candidate"#.to_vec()})
        }
    }
    #[test]
    fn accepted_blank_handoff_moves_existing_driver_once_and_retains_response_capture(){
        let count=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broker=SharedNetworkBroker::new(CandidateResponse(count.clone()));
        let context=Browser::in_process_probe().new_context_with_broker(ContextOptions::default(),broker.clone());let id=context.new_page().page_id();
        let client=SharedContextClient::direct(context.clone(),id);
        let mut source=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),Some(broker),Some(client));
        source.load_document_with_redirect_history("https://prerender.test/source".into(),r#"<script>globalThis.sourceDocument=document</script><script type="speculationrules">{"prerender":[{"source":"list","urls":["/next"],"target_hint":"_blank","eagerness":"immediate"}]}</script><body>source"#,None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new(),false,Some(Vec::new())).unwrap();
        let deadline=Instant::now()+Duration::from_secs(5);
        while !source.prerender_candidates.loaded_for_test(){source.poll_page_tasks().unwrap();assert!(Instant::now()<deadline,
            "blank candidate load deadline: {}; source_generation={} pending_response={} pending_transaction={} requests={}",
            source.prerender_candidates.diagnostic_for_test(), source.document_generation,
            source.pending_navigation_response.is_some(), source.pending_navigation_transaction.is_some(),
            count.load(std::sync::atomic::Ordering::SeqCst));std::thread::park_timeout(Duration::from_millis(1));}
        let mut host=source.extract_agent_host(None).unwrap();let realm=source.executable.as_ref().unwrap().agent_root_realm();
        let snapshot=host.as_ref().unwrap().prerender_source_snapshot(realm).unwrap().unwrap();
        let receipt=context.claim_blank_prerender(id,&snapshot.creator,None,"https://prerender.test/next").unwrap().unwrap();
        let actor=context.related_root_admission(receipt.page_id).unwrap().actor_route;
        let mut transfer=BlankTransfer::take(&mut source,context.downgrade(),snapshot,receipt.clone()).unwrap();
        let(control,_updates)=std::sync::mpsc::sync_channel(256);let mut entries=Vec::new();
        BLANK_TRANSFER_FAULTS.with(|faults|faults.set(3));
        while !transfer.poll(&mut host,&mut entries,&actor,&control).unwrap(){assert!(Instant::now()<deadline);std::thread::park_timeout(Duration::from_millis(1));}
        assert_eq!(entries.len(),1);assert_eq!(entries[0].root.state.page_id,receipt.page_id);
        assert_eq!(entries[0].root.state.history.len(),1);assert_eq!(entries[0].root.state.history_index,Some(0));
        assert!(entries[0].root.state.pending_navigation_response.is_some());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst),1);
        let native=host.as_mut().unwrap();native.activate_realm(entries[0].root.state.executable.as_ref().unwrap().agent_root_realm()).unwrap();
        assert_eq!(native.evaluate_string("String(document===saved&&window===savedWindow&&activations===1)").unwrap(),"true");
        assert!(native.native_script_close_history(native.active_realm()).unwrap().single_entry);
        native.activate_realm(realm).unwrap();assert_eq!(native.evaluate_string("String(document===sourceDocument)").unwrap(),"true");
        drop(transfer);
        assert!(!context.hidden_prerender_page(receipt.page_id));
        registry().lock().unwrap().remove(&context.related_root_admission(receipt.page_id).unwrap().route);
    }
}
