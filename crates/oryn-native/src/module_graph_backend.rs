// Native document-owned report collector shared with admitted IO. The host
// drains it only into the exact native document authority that created it.
#[derive(Default)]
pub(crate) struct ScriptCspReports(std::sync::Mutex<Vec<crate::csp::CspViolation>>,Option<Arc<crate::standalone_csp_reporting::Sender>>,std::sync::Mutex<Vec<ExternalTiming>>,pub(crate) Arc<crate::local_resource_preload::Store>,bool);
pub(crate) struct ExternalTiming{pub render_blocking:bool,pub details:Option<oryn_js::platform::ResourceTimingMetadata>,pub name:String,pub initiator:String,pub started:std::time::Instant,pub finished:std::time::Instant}
pub(crate) struct ExternalTimingGuard{queue:Arc<ScriptCspReports>,record:Option<ExternalTiming>,status:Option<crate::render_request_status::Status>}
impl ExternalTimingGuard{pub(crate) fn set_shared_status(&mut self,status:crate::render_request_status::Status){self.status=Some(status);}pub(crate) fn set_render_blocking(&mut self,value:bool){if let Some(record)=&mut self.record{record.render_blocking=value;}}pub(crate) fn set_details(&mut self,details:oryn_js::platform::ResourceTimingMetadata){if let Some(record)=&mut self.record{record.details=Some(details);}}}
impl Drop for ExternalTimingGuard{fn drop(&mut self){if let Some(mut record)=self.record.take(){record.finished=std::time::Instant::now();if let Some(status)=&self.status{record.render_blocking=status.finish();}let mut queue=self.queue.2.lock().unwrap_or_else(|e|e.into_inner());if queue.len()<250{queue.push(record)}}}}
impl ScriptCspReports {
    pub(crate) fn time_resource(self:&Arc<Self>,name:&str,initiator:&str)->ExternalTimingGuard{ExternalTimingGuard{status:None,queue:self.clone(),record:Some(ExternalTiming{render_blocking:false,details:None,name:name.into(),initiator:initiator.into(),started:std::time::Instant::now(),finished:std::time::Instant::now()})}}
    pub(crate) fn take_timings(&self)->Vec<ExternalTiming>{std::mem::take(&mut*self.2.lock().unwrap_or_else(|e|e.into_inner()))}

    pub(crate) fn with_transport(sender:Option<Arc<crate::standalone_csp_reporting::Sender>>,window_client:bool)->Self{Self(Default::default(),sender,Default::default(),Default::default(),window_client)}
    pub(crate) fn push_hashes(&self,reports:Vec<crate::csp::ScriptHashReport>){
        if !self.4{return;}
        if let Some(sender)=&self.1{for report in reports{let _=sender.queue_payload(report.endpoint,report.document_url,"application/reports+json".into(),report.body);}}
    }
    pub(crate) fn push_coep(&self,reports:Vec<crate::coep_reporting::Report>){
        if let Some(sender)=&self.1{for report in reports{let _=sender.queue_payload(report.endpoint,report.document_url,"application/reports+json".into(),report.body);}}
    }
    pub(crate) fn push_with_policy(&self,policy:&crate::csp::CspPolicySet,violations:Vec<crate::csp::CspViolation>)->Result<(),String>{
        self.push(violations.clone())?;
        if let Some(sender)=&self.1{for violation in &violations{sender.queue(policy,violation)?;}}
        Ok(())
    }

    pub(crate) fn push(&self,mut incoming:Vec<crate::csp::CspViolation>)->Result<(),String>{
        let mut pending=self.0.lock().map_err(|_|"script CSP report queue poisoned")?;
        let bytes=|items:&[crate::csp::CspViolation]|items.iter().fold(0usize,|sum,item|sum.saturating_add(item.source.len()).saturating_add(item.original_policy.len()).saturating_add(item.document_uri.len()).saturating_add(item.blocked_uri.len()).saturating_add(item.source_file.len()).saturating_add(item.sample.len()).saturating_add(item.referrer.len()).saturating_add(item.effective_directive.len()));
        if pending.len().saturating_add(incoming.len())>256||bytes(&pending).saturating_add(bytes(&incoming))>8*1024*1024{return Err("script CSP report queue capacity".into());}
        pending.append(&mut incoming);Ok(())
    }
    pub(crate) fn pending(&self)->bool{self.0.lock().is_ok_and(|pending|!pending.is_empty())||self.2.lock().is_ok_and(|pending|!pending.is_empty())}
    pub(crate) fn take(&self)->Result<Vec<crate::csp::CspViolation>,String>{Ok(std::mem::take(&mut *self.0.lock().map_err(|_|"script CSP report queue poisoned")?))}
}
// Included in module_graph_load. All methods execute on a bounded IO lane;
// native V8 callbacks capture this immutable backend and never call it directly.
#[derive(Clone)]
pub(crate) enum ModuleGraphBackend {
    #[cfg(unix)]
    Window(crate::context_service::SharedContextClient,oryn_common::v3::PageId,Option<(oryn_js::worker_wire::Creator,String)>,Option<String>),
    Worker {transport:Arc<dyn oryn_js::worker_wire::WorkerTransport>,bootstrap:Arc<oryn_js::worker_wire::AgentBootstrap>},
    Broker {coep:crate::coep_reporting::Policy,broker:SharedNetworkBroker,base:Url,initiator:String,isolation:crate::agent_isolation::DocumentIsolationPolicy,csp:Option<Arc<oryn_js::v8_bindings::ScriptPolicySnapshot>>,reports:Arc<ScriptCspReports>},
    Sources(Arc<BTreeMap<String,String>>,Option<Arc<oryn_js::v8_bindings::ScriptPolicySnapshot>>,Arc<ScriptCspReports>),
}
impl ModuleGraphBackend {
    /// Data modules finish synchronously through the same policy/SRI/MIME loader;
    /// this entry point can never start network IO for a nonlocal URL.
    pub(crate) fn fetch_local_css_module(&self,url:&str,base:&str,policy:&str,integrity:Option<String>,creator:oryn_js::worker_wire::Creator)->Result<(String,String),HostError>{
        let parsed=Url::parse(url).map_err(infrastructure)?;
        if parsed.scheme()!="data"{return Err(infrastructure("synchronous CSS preparation requires data URL"));}
        if let Self::Sources(_,_,reports)=self{
            let base=Url::parse(base).map_err(infrastructure)?;let broker=SharedNetworkBroker::new(crate::network::UnavailableBroker);let mut id=0;
            let csp=self.local_script_policy();let report=|violations|match csp{Some(policy)=>reports.push_with_policy(policy,violations),None=>reports.push(violations)};
            let result=crate::script_fetch::load_module_cancellable(&broker,&mut id,crate::script_fetch::ScriptFetch{
                hash_report:Some(&|items|reports.push_hashes(items)),fallback_encoding:None,coep_report:None,csp,nonce:None,parser_inserted:true,csp_report:Some(&report),embedder_policy:Default::default(),url:parsed,initiator:&base,referrer_source:Some(&base),top_level:&base,module:true,cross_origin:None,integrity:integrity.as_deref(),referrer_policy:Some(policy),
            },crate::ModuleType::Css,&std::sync::atomic::AtomicBool::new(false)).map_err(HostError::ScriptLoad)?;
            return Ok((result.url,result.source));
        }
        let admission:super::script_admission::SharedScriptAdmission=Default::default();
        let(_,final_url,source,_,_)=self.fetch(Fetch{url:url.into(),kind:crate::ModuleType::Css,parent:base.into(),include:false,policy:policy.into(),integrity,nonce:None,parser_inserted:true,render_blocking:false},Default::default(),Some(creator),base.into(),Some(0),Arc::downgrade(&admission),&std::sync::atomic::AtomicBool::new(false))?;
        Ok((final_url,source))
    }
    pub(crate) fn with_script_policy(mut self,authority:Option<String>)->Self{#[cfg(unix)] if let Self::Window(_,_,_,policy)=&mut self{*policy=authority;}#[cfg(not(unix))] let _=authority;self}
    pub(crate) fn with_local_script_policy(mut self,snapshot:Option<&Arc<oryn_js::v8_bindings::ScriptPolicySnapshot>>)->Result<Self,HostError>{
        if snapshot.is_some_and(|snapshot|snapshot.native::<crate::csp::CspPolicySet>().is_none()){return Err(infrastructure("native script policy payload absent"))}
        match &mut self{Self::Broker{csp,..}|Self::Sources(_,csp,_)=>*csp=snapshot.cloned(),_=>{}}Ok(self)
    }

    pub(crate) fn report_violations(&self,policy:&crate::csp::CspPolicySet,violations:Vec<crate::csp::CspViolation>)->Result<(),HostError>{if let Some(reports)=self.reports(){reports.push_with_policy(policy,violations).map_err(infrastructure)?;}Ok(())}
    pub(crate) fn reports(&self)->Option<&Arc<ScriptCspReports>>{match self{Self::Broker{reports,..}|Self::Sources(_,_,reports)=>Some(reports),_=>None}}
    pub(crate) fn local_script_policy(&self)->Option<&crate::csp::CspPolicySet>{match self{Self::Broker{csp,..}|Self::Sources(_,csp,_)=>csp.as_ref().and_then(|snapshot|snapshot.native::<crate::csp::CspPolicySet>()),_=>None}}
    fn script_policy(&self)->Option<String>{match self{#[cfg(unix)] Self::Window(_,_,_,policy)=>policy.clone(),Self::Broker{csp,..}|Self::Sources(_,csp,_)=>csp.as_ref().map(|snapshot|crate::execution_placement::fingerprint(snapshot.as_str().as_bytes())),_=>None}}


    pub(crate) fn resolve_request(&self,base:&str,requested:&str)->Result<String,HostError>{
        self.resolve_html_request(base,requested,"UTF-8")
    }
    pub(crate) fn resolve_html_request(&self,base:&str,requested:&str,encoding:&str)->Result<String,HostError>{
        let resolved=oryn_dom::document_url::parse_with_label(requested,Url::parse(base).ok().as_ref(),encoding).map(|url|url.to_string());
        // Request identity must precede decoded-fixture alias selection:
        // a foreign base can resolve a relative request away from its client,
        // even when the stored response alias ultimately returns home.
        if let Ok(url)=resolved{return Ok(url);}
        if let Self::Sources(sources,_,_)=self{
            if let Some((name,_))=resource_entry(sources,requested){return Ok(name.into());}
        }
        resolved.map_err(infrastructure)
    }
    fn authorize_source_response(&self,original:&str,final_url:&str,destination:crate::csp::CspDestination,nonce:Option<&str>,parser_inserted:bool)->Result<(),HostError>{
        if original==final_url{return Ok(());}
        if let Some(policy)=self.local_script_policy(){
            let decision=policy.authorize_redirect_from(&crate::csp::CspRequest{destination,url:final_url.into(),nonce:nonce.map(str::to_owned),inline_source:None,parser_inserted,initiated_by_trusted_script:!parser_inserted},true,original);
            self.report_violations(policy,decision.violations)?;
            if !decision.allowed{return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch("script alias response blocked by Content-Security-Policy".into())));}
        }
        Ok(())
    }
    fn resource_job(&self)->Result<Option<super::script_admission::ResourceJob>,HostError>{
        match self{#[cfg(unix)] Self::Window(client,..)=>client.track_resource_job().map(Some).map_err(infrastructure),_=>Ok(None)}
    }
    fn fetch(&self,fetch:Fetch,render_status:crate::render_request_status::Status,creator:Option<oryn_js::worker_wire::Creator>,initiator:String,request_id:Option<u64>,
        admission:super::script_admission::WeakScriptAdmission,
        cancel:&std::sync::atomic::AtomicBool)->Result<(String,String,String,bool,String),HostError>{
        if cancel.load(Ordering::Acquire){return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled));}
        // Scripting restrictions apply to both native local backends, while
        // non-executable typed modules retain their own destination policy.
        if fetch.kind==crate::ModuleType::JavaScript&&self.local_script_policy().is_some_and(|policy|policy.sandbox().is_some_and(|sandbox|!sandbox.allow_scripts)){
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch("module blocked by sandbox scripting restriction".into())));
        }
        if let Some(policy)=self.local_script_policy().filter(|_|matches!(self,Self::Sources(..))){
            let decision=policy.authorize(&crate::csp::CspRequest{destination:fetch.kind.csp_destination(),url:fetch.url.clone(),nonce:fetch.nonce.clone(),inline_source:None,parser_inserted:fetch.parser_inserted,initiated_by_trusted_script:!fetch.parser_inserted});
            self.reports().expect("standalone report queue").push_with_policy(policy,decision.violations).map_err(infrastructure)?;
            if !decision.allowed{return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch("module blocked by Content-Security-Policy".into())));}
        }
        match self{
            #[cfg(unix)]
            Self::Window(client,page_id,_,script_policy)=>{
                let page_id=*page_id;
                let creator=creator.ok_or_else(||infrastructure("window module creator unavailable"))?;
                let token=super::script_admission::reserve_script_admission(client,page_id,creator.clone(),&admission).map_err(infrastructure)?;
                let request=if fetch.kind==crate::ModuleType::JavaScript{
                    crate::context_service::ContextServiceRequest::LoadScript{fallback_encoding:None,page_id,creator,script_policy:script_policy.clone(),parser_inserted:fetch.parser_inserted,url:fetch.url.clone(),module:true,nonce:fetch.nonce,
                        integrity:fetch.integrity,cross_origin:fetch.include.then(||"use-credentials".into()),initiator_url:Some(initiator),referrer_policy:Some(fetch.policy),referrer_source:Some(fetch.parent)}
                }else{
                    crate::context_service::ContextServiceRequest::LoadTypedModule{nonce:fetch.nonce,page_id,creator,script_policy:script_policy.clone(),parser_inserted:fetch.parser_inserted,url:fetch.url.clone(),kind:fetch.kind,
                        integrity:fetch.integrity,cross_origin:fetch.include.then(||"use-credentials".into()),initiator_url:Some(initiator),referrer_policy:Some(fetch.policy),referrer_source:Some(fetch.parent)}
                };
                match client.request_with_render_status(crate::context_service::ContextServiceRequest::RunScriptLoad{render_blocking:fetch.render_blocking,page_id,token,request:Box::new(request)},Some(render_status)).map_err(infrastructure)?{
                    crate::context_service::ContextServiceResponse::ScriptLoadFailed{failure}=>Err(HostError::ScriptLoad(failure)),
                    crate::context_service::ContextServiceResponse::ScriptSource{url,source,referrer_policy,..}=>Ok((fetch.url,url,source,fetch.include,referrer_policy)),
                    _=>Err(infrastructure("parent returned an invalid module response")),
                }
            },
            Self::Worker{transport,bootstrap}=>{
                if fetch.kind==crate::ModuleType::JavaScript&&bootstrap.kind!=oryn_js::worker_wire::AgentKind::Service&&let Some(source)=bootstrap.modules.get(&fetch.url){
                    let metadata=bootstrap.import_metadata.get(&fetch.url);
                    let final_url=metadata.map(|metadata|metadata.response_url.clone()).unwrap_or_else(||fetch.url.clone());
                    let policy=metadata.map(|metadata|metadata.referrer_policy.clone()).unwrap_or_else(||bootstrap.referrer_policy.clone());
                    return Ok((fetch.url,final_url,String::from_utf16_lossy(source),fetch.include,policy));
                }
                let kind=match fetch.kind{
                    crate::ModuleType::JavaScript=>oryn_js::worker_wire::ImportedModuleType::JavaScript,
                    crate::ModuleType::Json=>oryn_js::worker_wire::ImportedModuleType::Json,
                    crate::ModuleType::Text=>oryn_js::worker_wire::ImportedModuleType::Text,
                    crate::ModuleType::Css=>return Err(infrastructure("CSS modules are unavailable in worker globals")),
                };
                match transport.request(oryn_js::worker_wire::AgentServiceRequest::ImportModule{url:fetch.url.clone(),kind,referrer_source:fetch.parent,referrer_policy:fetch.policy})
                    .map_err(|error|HostError::ScriptLoad(ScriptLoadFailure::worker(error)))?{
                    oryn_js::worker_wire::AgentServiceResponse::Script{url,source,referrer_policy}=>Ok((fetch.url,url,String::from_utf16_lossy(&source),fetch.include,referrer_policy)),
                    _=>Err(infrastructure("invalid worker module response")),
                }
            },
            Self::Broker{coep,broker,base,isolation,reports,..}=>{
                let mut _timing=reports.time_resource(&fetch.url,"script");_timing.set_shared_status(render_status);
                let report=|violations|match self.local_script_policy(){Some(policy)=>reports.push_with_policy(policy,violations),None=>reports.push(violations)};
                let mut request_id=request_id.ok_or_else(Self::capacity)?;
                let url=Url::parse(&fetch.url).map_err(infrastructure)?;
                let initiator=Url::parse(&initiator).map_err(infrastructure)?;
                let parent=Url::parse(&fetch.parent).map_err(infrastructure)?;
                let coep_report=|target:&Url,cors,credentials,headers:&[(String,String)]|reports.push_coep(coep.response(&initiator.origin().ascii_serialization(),matches!(initiator.origin(),url::Origin::Opaque(_)),target,cors,credentials,headers,"script"));
                let hash_report=|items|reports.push_hashes(items);
                let options=crate::script_fetch::ScriptFetch{hash_report:Some(&hash_report),fallback_encoding:None,coep_report:Some(&coep_report),csp:self.local_script_policy(),nonce:fetch.nonce.as_deref(),parser_inserted:fetch.parser_inserted,csp_report:Some(&report),embedder_policy:*isolation,url,initiator:&initiator,referrer_source:Some(&parent),top_level:base,
                    module:true,cross_origin:fetch.include.then_some("use-credentials"),integrity:fetch.integrity.as_deref(),referrer_policy:Some(&fetch.policy)};
                let result=crate::script_fetch::load_module_cancellable(broker,&mut request_id,options,fetch.kind,cancel).map_err(HostError::ScriptLoad)?;
                if let Some(timing)=result.timing.clone(){_timing.set_details(timing);}
                Ok((fetch.url,result.url,result.source,fetch.include,result.referrer_policy))
            },
            Self::Sources(..) if fetch.kind!=crate::ModuleType::JavaScript=>Err(infrastructure("untyped source map cannot authorize a typed module")),
            Self::Sources(..) if fetch.integrity.as_deref().is_some_and(|value|!value.is_empty())=>Err(infrastructure("decoded source map cannot verify module integrity")),
            Self::Sources(sources,_,_)=>{
                let base=Url::parse(&fetch.parent).ok();
                let entry=resource_entry(sources,&fetch.url).or_else(||sources.keys().find_map(|key|{
                    let requested=key.strip_prefix("oryn:module-alias:").unwrap_or(key);
                    (oryn_dom::document_url::parse_with_label(requested,base.as_ref(),"UTF-8").is_ok_and(|url|url.as_str()==fetch.url))
                        .then(||resource_entry(sources,requested)).flatten()
                }));
                let (url,source)=entry.ok_or_else(||infrastructure("module source unavailable"))?;
                let final_url=Url::parse(url).or_else(|_|base.as_ref().ok_or(url::ParseError::RelativeUrlWithoutBase).and_then(|base|base.join(url))).map(|url|url.to_string()).unwrap_or_else(|_|url.to_owned());
                self.authorize_source_response(&fetch.url,&final_url,fetch.kind.csp_destination(),fetch.nonce.as_deref(),fetch.parser_inserted)?;
                Ok((fetch.url.clone(),final_url,source.to_owned(),fetch.include,fetch.policy))
            },
        }
    }
    pub(crate) fn authorize_preload_reuse(&self,key:&crate::local_resource_preload::Key,entry:&crate::local_resource_preload::Entry)->Result<bool,HostError>{
        let trace=entry.trace();if trace.is_empty()||trace.len()>11{return Ok(false)}
        let policy=key.policy.as_deref().map(serde_json::from_str::<crate::csp::CspPolicySet>).transpose().map_err(infrastructure)?;
        let mut expected=Url::parse(&key.url).map_err(infrastructure)?;if let Some(upgraded)=policy.as_ref().and_then(|policy|policy.upgraded_url(&expected)){expected=upgraded;}expected.set_fragment(None);let mut first=Url::parse(&trace[0]).map_err(infrastructure)?;first.set_fragment(None);if expected!=first{return Ok(false)}
        let Some(policy)=policy else{return Ok(true)};
        let destination=if key.destination=="script"{crate::csp::CspDestination::Script}else{crate::csp::CspDestination::Style};
        if destination==crate::csp::CspDestination::Script&&policy.sandbox().is_some_and(|sandbox|!sandbox.allow_scripts){return Ok(false)}let mut violations=Vec::new();
        for(index,hop)in trace.iter().enumerate(){let url=Url::parse(hop).map_err(infrastructure)?;if policy.upgraded_url(&url).is_some_and(|upgraded|upgraded!=url){return Ok(false)}let decision=policy.authorize_redirect_from(&crate::csp::CspRequest{destination,url:hop.clone(),nonce:key.nonce.clone(),inline_source:None,parser_inserted:key.parser_inserted,initiated_by_trusted_script:destination==crate::csp::CspDestination::Script&&!key.parser_inserted},index!=0,&key.url);if !decision.allowed{return Ok(false)}violations.extend(decision.violations);}
        entry.publish_consumption_once(key,|evidence|{
            self.report_violations(&policy,violations)?;
            if destination==crate::csp::CspDestination::Script{
                if let (Some(reports),Some(evidence))=(self.reports(),evidence){reports.push_hashes(policy.hash_reports_from_evidence(destination,"script",&key.url,evidence));}
            }
            Ok(())
        })?;
        Ok(true)
    }
    pub(crate) fn classic_preload_key(&self,url:&str,integrity:Option<String>,cross_origin:Option<String>,policy:&str,nonce:Option<String>,parser_inserted:bool,encoding:Option<&str>)->Result<crate::local_resource_preload::Key,HostError>{
        let initiator=match self{Self::Broker{initiator,..}=>initiator.clone(),Self::Sources(..)=>String::new(),_=>return Err(infrastructure("local preload backend absent"))};
        Ok(crate::local_resource_preload::Key{destination:"script",url:url.into(),policy:self.local_script_policy().map(serde_json::to_string).transpose().map_err(infrastructure)?,initiator,referrer_policy:policy.into(),encoding:encoding.unwrap_or("UTF-8").into(),nonce,integrity,cross_origin,parser_inserted,quirks:false})
    }
    pub(crate) fn fetch_classic(&self,requested:&str,initiator_url:&str,url:String,integrity:Option<String>,cross_origin:Option<String>,policy:String,nonce:Option<String>,parser_inserted:bool,render_blocking:bool,fallback_encoding:Option<&str>,request_id:Option<u64>,cancel:&std::sync::atomic::AtomicBool)->Result<(String,String,bool,String),HostError>{
        if let Some(reports)=self.reports(){let mut key=self.classic_preload_key(&url,integrity.clone(),cross_origin.clone(),&policy,nonce.clone(),parser_inserted,fallback_encoding)?;if matches!(self,Self::Sources(..)){key.initiator=initiator_url.into();}if let Some(preload)=reports.3.claim(key.clone(),render_blocking){let response=preload.wait(cancel);if cancel.load(Ordering::Acquire){reports.3.finish_claim(&preload,false);return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled))}let authorized=if response.is_ok(){self.authorize_preload_reuse(&key,&preload)}else{Ok(preload.same_request(&key))};match authorized{Ok(false)=>reports.3.finish_claim(&preload,false),Ok(true)=>{reports.3.finish_claim(&preload,true);return match response?{crate::local_resource_preload::Response::Script(url,source,muted,policy)=>Ok((url,source,muted,policy)),_=>Err(infrastructure("classic preload response kind"))}},Err(error)=>{reports.3.finish_claim(&preload,false);return Err(error)}}}}
        self.fetch_classic_uncached(requested,initiator_url,url,integrity,cross_origin,policy,nonce,parser_inserted,render_blocking,fallback_encoding,request_id,cancel,None,None,None)
    }
    pub(crate) fn fetch_classic_uncached(&self,requested:&str,initiator_url:&str,url:String,integrity:Option<String>,cross_origin:Option<String>,policy:String,nonce:Option<String>,parser_inserted:bool,render_blocking:bool,fallback_encoding:Option<&str>,request_id:Option<u64>,cancel:&std::sync::atomic::AtomicBool,preload:Option<crate::render_request_status::Status>,trace:Option<&mut Vec<String>>,hash_evidence:Option<&mut Option<crate::csp::ScriptHashEvidence>>)->Result<(String,String,bool,String),HostError>{
        if matches!(self,Self::Sources(..))&&self.local_script_policy().and_then(|policy|policy.sandbox()).is_some_and(|sandbox|!sandbox.allow_scripts){
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch("script blocked by sandbox".into())));
        }
        if let Some(policy)=self.local_script_policy().filter(|_|matches!(self,Self::Sources(..))&&!Url::parse(&url).is_ok_and(|url|url.scheme()=="data")){
            let decision=policy.authorize(&crate::csp::CspRequest{destination:crate::csp::CspDestination::Script,url:url.clone(),nonce:nonce.clone(),inline_source:None,parser_inserted,initiated_by_trusted_script:!parser_inserted});
            self.reports().expect("standalone report queue").push_with_policy(policy,decision.violations).map_err(infrastructure)?;
            if !decision.allowed||policy.sandbox().is_some_and(|sandbox|!sandbox.allow_scripts){return Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch("script blocked by Content-Security-Policy".into())));}
        }
        match self{
            Self::Broker{coep,broker,base,initiator,isolation,csp,reports}=>{
                let mut _timing=reports.time_resource(&url,if preload.is_some(){"link"}else{"script"});_timing.set_render_blocking(render_blocking);if let Some(status)=preload{_timing.set_shared_status(status);}
                let report=|violations|match self.local_script_policy(){Some(policy)=>reports.push_with_policy(policy,violations),None=>reports.push(violations)};
                let mut request_id=request_id.ok_or_else(Self::capacity)?;
                let url=Url::parse(&url).map_err(infrastructure)?;let initiator=Url::parse(initiator).map_err(infrastructure)?;
                let coep_report=|target:&Url,cors,credentials,headers:&[(String,String)]|reports.push_coep(coep.response(&initiator.origin().ascii_serialization(),matches!(initiator.origin(),url::Origin::Opaque(_)),target,cors,credentials,headers,"script"));
                let loaded=crate::script_fetch::load_with_metadata_cancellable(broker,&mut request_id,crate::script_fetch::ScriptFetch{hash_report:Some(&|items|reports.push_hashes(items)),fallback_encoding,coep_report:Some(&coep_report),
                    csp:csp.as_ref().and_then(|snapshot|snapshot.native::<crate::csp::CspPolicySet>()),nonce:nonce.as_deref(),parser_inserted,csp_report:Some(&report),embedder_policy:*isolation,url,initiator:&initiator,referrer_source:Some(&initiator),top_level:base,module:false,
                    integrity:integrity.as_deref(),cross_origin:cross_origin.as_deref(),referrer_policy:Some(&policy),
                },cancel).map_err(HostError::ScriptLoad)?;
                if let Some(timing)=loaded.timing.clone(){_timing.set_details(timing);}
                if let Some(slot)=hash_evidence{*slot=Some(loaded.hash_evidence);}
                if let Some(trace)=trace{*trace=loaded.response_urls;}
                Ok((loaded.url,loaded.source,loaded.muted_errors,loaded.referrer_policy))
            },
            Self::Sources(_,_,reports) if Url::parse(&url).is_ok_and(|url|url.scheme()=="data")=>{
                let mut timing=reports.time_resource(&url,if preload.is_some(){"link"}else{"script"});
                timing.set_render_blocking(render_blocking);if let Some(status)=preload{timing.set_shared_status(status);}
                let initiator=Url::parse(initiator_url).map_err(infrastructure)?;
                let broker=SharedNetworkBroker::new(crate::network::UnavailableBroker);let mut id=0;
                let csp=self.local_script_policy();
                let report=|violations|match csp{Some(policy)=>reports.push_with_policy(policy,violations),None=>reports.push(violations)};
                let loaded=crate::script_fetch::load_with_metadata_cancellable(&broker,&mut id,crate::script_fetch::ScriptFetch{
                    hash_report:Some(&|items|reports.push_hashes(items)),fallback_encoding,coep_report:None,csp,nonce:nonce.as_deref(),parser_inserted,csp_report:Some(&report),embedder_policy:Default::default(),
                    url:Url::parse(&url).map_err(infrastructure)?,initiator:&initiator,referrer_source:Some(&initiator),top_level:&initiator,module:false,
                    integrity:integrity.as_deref(),cross_origin:cross_origin.as_deref(),referrer_policy:Some(&policy),
                },cancel).map_err(HostError::ScriptLoad)?;
                if let Some(details)=loaded.timing.clone(){timing.set_details(details);}
                if let Some(slot)=hash_evidence{*slot=Some(loaded.hash_evidence);}
                if let Some(trace)=trace{*trace=loaded.response_urls;}
                Ok((loaded.url,loaded.source,loaded.muted_errors,loaded.referrer_policy))
            },
            Self::Sources(sources,_,_) if integrity.as_deref().is_none_or(str::is_empty)&&cross_origin.is_none()=>{
                if cancel.load(Ordering::Acquire){return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled));}
                let (name,source)=resource_entry(sources,&url).or_else(||resource_entry(sources,requested))
                    .ok_or_else(||HostError::ScriptLoad(ScriptLoadFailure::Fetch("standalone script source unavailable".into())))?;
                let final_url=if name==requested||name==url{url.clone()}else{
                    Url::parse(name).or_else(|_|Url::parse(&url).and_then(|base|base.join(name))).map(|url|url.to_string()).unwrap_or_else(|_|name.to_owned())
                };
                self.authorize_source_response(&url,&final_url,crate::csp::CspDestination::Script,nonce.as_deref(),parser_inserted)?;
                if let Some(trace)=trace{trace.push(url.clone());if final_url!=url{trace.push(final_url.clone());}}
                // Both known endpoints contribute to no-CORS response taint;
                // decoded fixtures contain no intermediate redirect metadata.
                let initiator=Url::parse(initiator_url).ok();
                let muted=initiator.as_ref().is_some_and(|initiator|[url.as_str(),final_url.as_str()].into_iter().any(|value|{
                    Url::parse(value).or_else(|_|initiator.join(value)).is_ok_and(|target|target.origin()!=initiator.origin())
                }));
                Ok((final_url,source.to_owned(),muted,policy))
            },
            _=>Err(infrastructure("classic script source lacks an eligible native backend")),
        }
    }
    fn capacity()->HostError{HostError::LimitExceeded{resource:"module request identifiers".into()}}
}

#[cfg(test)]mod classic_preparation_tests{
 use super::*;
 #[test]fn decoded_classic_sources_keep_response_origin_and_redirect_taint(){
  let sources=BTreeMap::from([
   ("https://home.test/local.js".into(),"void 0".into()),
   ("https://remote.test/remote.js".into(),"void 0".into()),
   ("oryn:module-alias:https://remote.test/back.js".into(),"https://home.test/local.js".into()),
   ("oryn:module-alias:https://home.test/away.js".into(),"https://remote.test/remote.js".into()),
  ]);
  let backend=ModuleGraphBackend::Sources(Arc::new(sources),None,Arc::new(ScriptCspReports::default()));
  for (url,muted) in [("https://home.test/local.js",false),("https://remote.test/remote.js",true),("https://remote.test/back.js",true),("https://home.test/away.js",true)]{
   let resolved=backend.resolve_request("https://home.test/page",url).unwrap();
   let response=backend.fetch_classic(url,"https://home.test/page",resolved,None,None,"origin".into(),None,true,false,None,None,&std::sync::atomic::AtomicBool::new(false)).unwrap();
   assert_eq!(response.2,muted,"{url}");
  }
 }
 #[test]fn decoded_module_alias_preserves_resolved_request_and_final_response_urls(){
  let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::from([
   ("oryn:module-alias:entry.js".into(),"final.js".into()),("final.js".into(),"export const value=42".into()),
  ])),None,Arc::new(ScriptCspReports::default()));
  let requested=backend.resolve_request("https://example.test/dir/page","entry.js").unwrap();assert_eq!(requested,"https://example.test/dir/entry.js");
  let result=backend.fetch(Fetch{url:requested.clone(),kind:crate::ModuleType::JavaScript,parent:"https://example.test/dir/page".into(),include:false,policy:"origin".into(),integrity:None,nonce:None,parser_inserted:true,render_blocking:false},Default::default(),None,"https://example.test/dir/page".into(),None,Default::default(),&std::sync::atomic::AtomicBool::new(false)).unwrap();
  assert_eq!(result.0,requested);assert_eq!(result.1,"https://example.test/dir/final.js");assert_eq!(result.2,"export const value=42");
 }
 #[test]fn decoded_alias_final_response_obeys_csp_and_retains_preload_trace(){
  let original="https://home.test/away.js";let final_url="https://remote.test/final.js";
  for report_only in [false,true]{
   let policy=crate::csp::CspPolicySet::from_headers("https://home.test/page",&[(if report_only{"content-security-policy-report-only"}else{"content-security-policy"}.into(),"script-src https://home.test".into())]);
   let snapshot=oryn_js::v8_bindings::ScriptPolicySnapshot::new(serde_json::to_string(&policy).unwrap(),policy).unwrap();let reports=Arc::new(ScriptCspReports::default());
   let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::from([(format!("oryn:module-alias:{original}"),final_url.into()),(final_url.into(),"export {};".into())])),Some(snapshot),reports.clone());
   let mut trace=Vec::new();let classic=backend.fetch_classic_uncached(original,"https://home.test/page",original.into(),None,None,"origin".into(),None,true,false,None,None,&std::sync::atomic::AtomicBool::new(false),None,Some(&mut trace),None);
   assert_eq!(classic.is_ok(),report_only);if report_only{assert_eq!(trace,vec![original,final_url]);assert!(classic.unwrap().2);}
   let module=backend.fetch(Fetch{url:original.into(),kind:crate::ModuleType::JavaScript,parent:"https://home.test/page".into(),include:false,policy:"origin".into(),integrity:None,nonce:None,parser_inserted:true,render_blocking:false},Default::default(),None,"https://home.test/page".into(),None,Default::default(),&std::sync::atomic::AtomicBool::new(false));
   assert_eq!(module.is_ok(),report_only);let violations=reports.0.lock().unwrap();assert_eq!(violations.len(),2);assert!(violations.iter().all(|violation|violation.blocked_uri.starts_with("https://home.test")),"redirect reports retain original request identity");
  }
 }
 #[test]fn relative_classic_alias_keeps_foreign_base_request_taint(){
  let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::from([
   ("oryn:module-alias:back.js".into(),"https://home.test/local.js".into()),
   ("https://home.test/local.js".into(),"throw 42".into()),
  ])),None,Arc::new(ScriptCspReports::default()));
  let resolved=backend.resolve_request("https://remote.test/","back.js").unwrap();assert_eq!(resolved,"https://remote.test/back.js");
  let response=backend.fetch_classic("back.js","https://home.test/page",resolved,None,None,"origin".into(),None,true,false,None,None,&std::sync::atomic::AtomicBool::new(false)).unwrap();
  assert_eq!(response.0,"https://home.test/local.js");assert!(response.2);
 }
 #[test]fn local_classic_data_retains_sandbox_denial_and_resource_timing(){
  let url="data:text/javascript,void%200";
  let policy=crate::csp::CspPolicySet::from_headers("https://example.test/page",&[("content-security-policy".into(),"sandbox; script-src data:".into())]);
  let snapshot=oryn_js::v8_bindings::ScriptPolicySnapshot::new(serde_json::to_string(&policy).unwrap(),policy).unwrap();
  let denied=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),Some(snapshot),Arc::new(ScriptCspReports::default()));
  assert!(matches!(denied.fetch_classic(url,"https://example.test/page",url.into(),None,None,"no-referrer".into(),None,true,true,None,None,&std::sync::atomic::AtomicBool::new(false)),Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(message))) if message.contains("sandbox")));
  let reports=Arc::new(ScriptCspReports::default());let allowed=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,reports.clone());
  allowed.fetch_classic(url,"https://example.test/page",url.into(),None,None,"no-referrer".into(),None,true,true,None,None,&std::sync::atomic::AtomicBool::new(false)).unwrap();
  let timings=reports.take_timings();assert_eq!(timings.len(),1);assert!(timings[0].render_blocking);
 }
 #[test]fn local_classic_data_obeys_frozen_csp_before_decoding(){
  let policy=crate::csp::CspPolicySet::from_headers("https://example.test/page",&[("content-security-policy".into(),"script-src 'none'".into())]);
  let snapshot=oryn_js::v8_bindings::ScriptPolicySnapshot::new(serde_json::to_string(&policy).unwrap(),policy).unwrap();
  let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),Some(snapshot),Arc::new(ScriptCspReports::default()));
  let url="data:text/javascript,globalThis.mustNotRun%3Dtrue";
  assert!(matches!(backend.fetch_classic(url,"https://example.test/page",url.into(),None,None,"no-referrer".into(),None,true,false,None,None,&std::sync::atomic::AtomicBool::new(false)),Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_)))));
 }
 #[test]fn local_classic_data_uses_native_decoder_integrity_and_cancellation(){
  let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,Arc::new(ScriptCspReports::default()));
  let url="data:text/javascript;charset=windows-1251,%E0";
  let load=|integrity,cancel:&std::sync::atomic::AtomicBool|backend.fetch_classic(url,"https://example.test/page",url.into(),integrity,None,"no-referrer".into(),None,true,false,Some("UTF-8"),None,cancel);
  let cancelled=std::sync::atomic::AtomicBool::new(false);
  let loaded=load(None,&cancelled).unwrap();assert_eq!(loaded.1,"а");assert_eq!(loaded.0,url);
  assert!(matches!(load(Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),&cancelled),Err(HostError::ScriptLoad(ScriptLoadFailure::Integrity(_)))));
  cancelled.store(true,Ordering::Release);
  assert!(matches!(load(None,&cancelled),Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled))));
  let remote="https://example.test/missing.js";
  assert!(matches!(backend.fetch_classic(remote,"https://example.test/page",remote.into(),None,None,"no-referrer".into(),None,true,false,None,None,&std::sync::atomic::AtomicBool::new(false)),Err(HostError::ScriptLoad(ScriptLoadFailure::Fetch(_)))));
 }

 struct Encoded;
 impl crate::network::NetworkBroker for Encoded{
  fn send(&self,request:crate::network::NetworkRequest)->Result<crate::network::NetworkResponse,crate::network::NetworkError>{Ok(crate::network::NetworkResponse{timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/javascript".into())],body:b"globalThis.value='\xe0'".to_vec()})}
 }
 #[test]fn preload_hash_evidence_keeps_original_encoded_bytes(){
  let reports=Arc::new(ScriptCspReports::default());let base=Url::parse("https://example.test/").unwrap();let backend=ModuleGraphBackend::Broker{coep:crate::coep_reporting::Policy::new(base.as_str(),&[]),broker:crate::network::SharedNetworkBroker::new(Encoded),base:base.clone(),initiator:base.to_string(),isolation:Default::default(),csp:None,reports};
  let mut evidence=None;let mut trace=Vec::new();
  let result=backend.fetch_classic_uncached("script.js",base.as_str(),base.join("script.js").unwrap().to_string(),None,None,String::new(),None,true,false,Some("windows-1251"),Some(1),&std::sync::atomic::AtomicBool::new(false),None,Some(&mut trace),Some(&mut evidence)).unwrap();
  let evidence=evidence.unwrap();assert_eq!(evidence,crate::csp::ScriptHashEvidence::from_bytes(b"globalThis.value='\xe0'",true));assert_ne!(evidence,crate::csp::ScriptHashEvidence::from_bytes(result.1.as_bytes(),true));assert_eq!(trace,vec![base.join("script.js").unwrap().to_string()]);
 }
 #[test]fn standalone_classic_consumes_original_encoding_and_timing_status(){
  let reports=Arc::new(ScriptCspReports::default());let base=Url::parse("https://example.test/").unwrap();let backend=ModuleGraphBackend::Broker{coep:crate::coep_reporting::Policy::new(base.as_str(),&[]),broker:crate::network::SharedNetworkBroker::new(Encoded),base:base.clone(),initiator:base.to_string(),isolation:Default::default(),csp:None,reports:reports.clone()};
  let result=backend.fetch_classic("script.js",base.as_str(),base.join("script.js").unwrap().to_string(),None,None,String::new(),None,true,true,Some("windows-1251"),Some(1),&std::sync::atomic::AtomicBool::new(false)).unwrap();assert!(result.1.contains('\u{430}'));let timings=reports.take_timings();assert_eq!(timings.len(),1);assert!(timings[0].render_blocking);assert!(timings[0].details.is_some());
 }
}

#[cfg(test)]mod preload_reuse_tests{
 use super::*;
 #[test]fn preload_policy_capacity_failure_keeps_consumer_publication_retryable(){
  let reports=Arc::new(ScriptCspReports::default());let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,reports.clone());
  let policy=crate::csp::CspPolicySet::from_headers("https://root.test/",&[("content-security-policy-report-only".into(),"script-src 'none'".into())]);
  let mut key=backend.classic_preload_key("https://root.test/app.js",None,None,"origin",None,true,Some("UTF-8")).unwrap();key.policy=Some(serde_json::to_string(&policy).unwrap());
  let entry=reports.3.admit(key.clone()).unwrap();entry.set_trace(vec![key.url.clone()]);entry.complete(Ok(crate::local_resource_preload::Response::Script(key.url.clone(),"void 0".into(),false,"origin".into())));
  let decision=policy.authorize(&crate::csp::CspRequest{destination:crate::csp::CspDestination::Script,url:key.url.clone(),nonce:None,inline_source:None,parser_inserted:true,initiated_by_trusted_script:false});assert!(decision.allowed);assert_eq!(decision.violations.len(),1);
  reports.push(vec![decision.violations[0].clone();256]).unwrap();
  assert!(backend.authorize_preload_reuse(&key,&entry).is_err());assert_eq!(reports.take().unwrap().len(),256);
  assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());assert_eq!(reports.take().unwrap().len(),1);
  assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());assert!(reports.take().unwrap().is_empty());
 }
 #[test]fn preload_retry_does_not_duplicate_report_only_events_without_hash_evidence(){
  let reports=Arc::new(ScriptCspReports::default());let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,reports.clone());
  for destination in ["script","stylesheet"]{
   let mut key=backend.classic_preload_key("https://root.test/resource",None,None,"origin",None,true,Some("UTF-8")).unwrap();key.destination=destination;
   let directive=if destination=="script"{"script-src 'none'"}else{"style-src 'none'"};
   key.policy=Some(serde_json::to_string(&crate::csp::CspPolicySet::from_headers("https://root.test/",&[("content-security-policy-report-only".into(),directive.into())])).unwrap());
   let entry=reports.3.admit(key.clone()).unwrap();entry.set_trace(vec![key.url.clone()]);entry.complete(Ok(if destination=="script"{crate::local_resource_preload::Response::Script(key.url.clone(),"void 0".into(),false,"origin".into())}else{crate::local_resource_preload::Response::Style(key.url.clone(),"body{}".into(),"UTF-8".into(),"origin".into())}));
   assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());assert_eq!(reports.take().unwrap().len(),1,"one policy event per consumer response");
  }
 }
 #[test]fn identical_preload_policy_still_commits_one_consumer_hash_receipt(){
  let reports=Arc::new(ScriptCspReports::default());let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,reports.clone());let mut key=backend.classic_preload_key("https://root.test/start.js",None,None,"origin",None,true,Some("UTF-8")).unwrap();
  key.policy=Some(serde_json::to_string(&crate::csp::CspPolicySet::from_headers("https://root.test/",&[("content-security-policy".into(),"script-src https: 'report-sha256'".into())])).unwrap());
  let entry=reports.3.admit(key.clone()).unwrap();entry.set_trace(vec![key.url.clone()]);entry.set_hash_evidence(crate::csp::ScriptHashEvidence::from_bytes(b"void 0",true));entry.complete(Ok(crate::local_resource_preload::Response::Script(key.url.clone(),"void 0".into(),false,"origin".into())));
  assert!(entry.same_request(&key));assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());assert!(backend.authorize_preload_reuse(&key,&entry).unwrap());entry.publish_hash_once(&key,|_|panic!("consumer receipt was not retained")).unwrap();
 }
 #[test]fn local_policy_reuse_checks_intermediate_redirect_and_releases_rejected_candidate(){
  let reports=Arc::new(ScriptCspReports::default());let backend=ModuleGraphBackend::Sources(Arc::new(BTreeMap::new()),None,reports.clone());let original=backend.classic_preload_key("https://root.test/start.js",None,None,"origin",None,true,Some("UTF-8")).unwrap();let producer=reports.3.admit(original.clone()).unwrap();producer.set_trace(vec!["https://root.test/start.js".into(),"https://middle.test/hop.js".into(),"https://final.test/end.js".into()]);producer.complete(Ok(crate::local_resource_preload::Response::Script("https://final.test/end.js".into(),"void 0".into(),true,"origin".into())));
  let mut restricted=original.clone();restricted.parser_inserted=false;restricted.policy=Some(serde_json::to_string(&crate::csp::CspPolicySet::from_headers("https://root.test/",&[("content-security-policy".into(),"script-src https://root.test https://final.test".into())])).unwrap());let candidate=reports.3.claim(restricted.clone(),false).unwrap();assert!(!backend.authorize_preload_reuse(&restricted,&candidate).unwrap());reports.3.finish_claim(&candidate,false);
  let mut allowed=original;allowed.parser_inserted=false;allowed.policy=Some(serde_json::to_string(&crate::csp::CspPolicySet::from_headers("https://root.test/",&[("content-security-policy".into(),"script-src https:".into())])).unwrap());let candidate=reports.3.claim(allowed.clone(),false).unwrap();assert!(backend.authorize_preload_reuse(&allowed,&candidate).unwrap());reports.3.finish_claim(&candidate,true);assert!(reports.3.claim(allowed,false).is_none());
 }
}

#[cfg(test)]mod sandbox_typed_module_tests{
 use super::*;
 #[test]fn scripting_disabled_still_prepares_css_module_but_never_executable_javascript(){
  let policy=crate::csp::CspPolicySet::from_headers("https://example.test/",&[("content-security-policy".into(),"sandbox allow-same-origin; default-src 'self'".into())]);
  let snapshot=oryn_js::v8_bindings::ScriptPolicySnapshot::new(serde_json::to_string(&policy).unwrap(),policy).unwrap();
  struct TypedResources;
  impl crate::network::NetworkBroker for TypedResources{
   fn send(&self,request:crate::network::NetworkRequest)->Result<crate::network::NetworkResponse,crate::network::NetworkError>{
    assert_eq!(request.url.as_str(),"https://example.test/style.css","sandboxed JavaScript must not reach the broker");
    Ok(crate::network::NetworkResponse{timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/css".into())],body:b"div{color:red}".to_vec()})
   }
  }
  let base=Url::parse("https://example.test/").unwrap();
  let backend=ModuleGraphBackend::Broker{coep:Default::default(),broker:SharedNetworkBroker::new(TypedResources),base:base.clone(),initiator:base.to_string(),isolation:Default::default(),csp:Some(snapshot.clone()),reports:Arc::new(ScriptCspReports::default())};
  let untyped=ModuleGraphBackend::Sources(Arc::new(BTreeMap::from([("https://example.test/style.css".into(),"div{color:red}".into()),("https://example.test/code.js".into(),"globalThis.ran=true".into())])),Some(snapshot),Arc::new(ScriptCspReports::default()));
  for(kind,url,allowed)in[(crate::ModuleType::Css,"https://example.test/style.css",true),(crate::ModuleType::JavaScript,"https://example.test/code.js",false)]{
   let admission:super::super::script_admission::SharedScriptAdmission=Default::default();
   let result=backend.fetch(Fetch{url:url.into(),kind,parent:"https://example.test/".into(),include:false,policy:"strict-origin-when-cross-origin".into(),integrity:None,nonce:None,parser_inserted:true,render_blocking:false},Default::default(),None,"https://example.test/".into(),Some(1),Arc::downgrade(&admission),&std::sync::atomic::AtomicBool::new(false));
   assert_eq!(result.is_ok(),allowed,"{kind:?}: {result:?}");
   let untyped_result=untyped.fetch(Fetch{url:url.into(),kind,parent:"https://example.test/".into(),include:false,policy:"strict-origin-when-cross-origin".into(),integrity:None,nonce:None,parser_inserted:true,render_blocking:false},Default::default(),None,"https://example.test/".into(),Some(1),Arc::downgrade(&admission),&std::sync::atomic::AtomicBool::new(false));
   assert!(untyped_result.is_err(),"untyped CSS must not acquire MIME authority; sandboxed JavaScript stays blocked");
  }
 }
}
