// Unix parent-context authority. Pure scheduling/transport lives in deferred_fetch.
impl BrowserContext {
    pub(crate) fn keepalive_context_active(&self)->bool{!self.inner.workers_retired.load(Ordering::Acquire)}
    pub(crate) fn reserve_keepalive(&self,owner:&AgentOwner,bytes:usize)->Result<crate::keepalive::Lease,String>{self.inner.keepalive_budget.reserve(owner.clone(),bytes)}
    /// Walk exact committed ancestors. A same-origin grandparent separated by
    /// a cross-origin document is never a quota owner for the descendant.
    fn deferred_control_document(&self,owner:&AgentOwner)->Result<AgentOwner,String>{
        let parents=self.inner.worker_policies.lock().map_err(|_|"deferred ancestry poisoned")?.activation_parents.clone();
        let mut current=owner.clone();let mut seen=std::collections::BTreeSet::new();
        loop{if !seen.insert(current.clone())||seen.len()>256{return Err("deferred ancestry cycle".into());}
            let Some(parent)=parents.get(&current)else{return Ok(current);};
            let(_,child_origin,_)=self.native_worker_authority(&current).map_err(|error|error.to_string())?;
            let(_,parent_origin,_)=self.native_worker_authority(parent).map_err(|error|error.to_string())?;
            if child_origin.opaque||parent_origin.opaque||child_origin.serialized!=parent_origin.serialized{return Ok(current);}
            current=parent.clone();
        }
    }
    pub(crate) fn reserve_deferred_frame(&self,parent:&AgentOwner,page:PageId,frame:String,source:&str,allow:&str)->Result<(),String>{
        if source.len()>16384||allow.len()>16384{return Err("deferred container metadata too large".into());}
        if frame.len()!=64||!frame.bytes().all(|byte|byte.is_ascii_hexdigit()){return Err("invalid deferred container token".into());}
        if !matches!(parent,AgentOwner::Window{page:owner,..}if *owner==page){return Err("foreign deferred container parent".into());}
        if matches!(source,"about:blank"|"about:srcdoc"){self.remove_deferred_container(page,&frame);return Ok(());}
        self.reserve_deferred_container(parent,AgentOwner::Window{page,target:oryn_js::message_wire::WindowTarget::Frame(frame),generation:0,epoch:0},source,allow)
    }
    pub(crate) fn remove_deferred_container(&self,page:PageId,frame:&str){self.inner.deferred_fetches.lock().unwrap_or_else(|error|error.into_inner()).reservations.remove(&(page,frame.to_owned()));}
    /// Reserve before a parent-driven navigation; the caller supplies native
    /// parent authority and the intended child identity, never final URL policy.
    pub(crate) fn reserve_deferred_container(&self,parent:&AgentOwner,child:AgentOwner,source:&str,allow:&str)->Result<(),String>{
        let control=self.deferred_control_document(parent)?;
        let(_,origin,policy)=self.native_worker_authority(parent).map_err(|error|error.to_string())?;
        let target=Url::parse(source).map_err(|error|error.to_string())?.origin().ascii_serialization();
        let snapshot=self.native_permissions_policy_snapshot(parent).map_err(|error|error.to_string())?;
        let permits=|feature|snapshot.allows(feature,None)&&oryn_js::permissions_policy::allows_feature(&policy.headers,feature,&origin.serialized,&target)&&oryn_js::permissions_policy::container_allows_feature(allow,feature,&origin.serialized,source,&target);
        let control_policy=self.native_permissions_policy_snapshot(&control).map_err(|error|error.to_string())?;
        let top=matches!(&control,AgentOwner::Window{target:oryn_js::message_wire::WindowTarget::Root,..});
        let mut registry=self.inner.deferred_fetches.lock().map_err(|_|"deferred registry poisoned")?;
        if registry.closed{return Err("deferred context retired".into());}
        registry.reservations.remove(&container_key(&child));
        let quota=if top{if !control_policy.allows("deferred-fetch",None){0}else if control_policy.allows("deferred-fetch-minimal",None){TOP_BYTES-MINIMAL_POOL}else{TOP_BYTES}}else{registry.reservations.get(&container_key(&control)).map_or(0,|value|value.bytes)};
        let used=registry.entries.values().filter(|entry|entry.activation.load(Ordering::Acquire)==0).filter_map(|entry|entry.pending.as_ref()).filter(|plan|plan.control==control).fold(0usize,|sum,plan|sum.saturating_add(plan.bytes));
        let delegated=registry.reservations.values().filter(|value|value.control==control&&!value.minimal).fold(0usize,|sum,value|sum.saturating_add(value.bytes));
        let normal=permits("deferred-fetch")&&quota.saturating_sub(used.saturating_add(delegated))>=ORIGIN_BYTES;
        let minimal=!normal&&top&&permits("deferred-fetch-minimal")&&registry.reservations.values().filter(|value|value.control==control&&value.minimal).count()<MINIMAL_POOL/CHILD_BYTES;
        if registry.reservations.len()>=MAX_CAPABILITIES{return Err("deferred container capacity exceeded".into());}
        registry.reservations.insert(container_key(&child),Reservation{control,bytes:if normal{ORIGIN_BYTES}else if minimal{CHILD_BYTES}else{0},minimal});Ok(())
    }
    /// Called after redirects and native origin installation. Sharing restores
    /// the container reservation; cross-origin redirects retain its charge.
    pub(crate) fn commit_deferred_container(&self,owner:&AgentOwner)->Result<(),String>{
        let control=self.deferred_control_document(owner)?;
        if &control!=owner{self.inner.deferred_fetches.lock().map_err(|_|"deferred registry poisoned")?.reservations.remove(&container_key(owner));}Ok(())
    }
    pub(crate) fn drain_deferred_fetches(&self,budget:Duration)->Result<(),RuntimeError>{
        let scope=self.inner.deferred_fetches.lock().unwrap_or_else(|error|error.into_inner()).scope.clone();scope.drain(budget)
    }
    pub(crate) fn flush_deferred_page(&self,page:PageId)->Result<(),String>{
        let tokens=self.inner.deferred_fetches.lock().map_err(|_|"deferred registry poisoned")?.entries.iter().filter(|(_,entry)|entry.page==page&&entry.pending.is_some()).map(|(token,_)|DeferredFetchToken(token.clone())).collect::<Vec<_>>();
        let mut errors=Vec::new();for token in tokens{if let Err(error)=self.activate_deferred_fetch(page,&token){errors.push(error.to_string());}}if errors.is_empty(){Ok(())}else{Err(errors.join("; "))}
    }
    pub(crate) fn prepare_deferred_fetch(&self,page:PageId,creator:Creator,token:DeferredFetchToken,request:PlatformHttpRequest,activate_after:Option<Duration>)->Result<(),DeferredFetchError>{
        if self.inner.broker.is_none(){return Err(transport("deferred Fetch network unavailable"));}
        let hash=deferred_fingerprint(&request,activate_after)?;
        if self.inner.deferred_fetches.lock().map_err(transport)?.replay(page,&creator,&token,&hash)?{return Ok(());}
        let Creator::Window{target,document_generation,stream_epoch}=&creator else{return Err(invalid("deferred Fetch requires a Window"));};
        let (owner,_,origin)=self.window_worker_creator(page,target,*document_generation,*stream_epoch).map_err(transport)?;
        let native_policy=self.window_worker_policy(page,target,*document_generation,*stream_epoch).map_err(transport)?;
        let policy=crate::csp::CspPolicySet::from_agent_policy(&native_policy);
        let source=Url::parse(&native_policy.url).map_err(transport)?;
        let target_url=Url::parse(&request.url).map_err(|_|invalid("invalid deferred Fetch URL"))?;
        if !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(target_url.as_str()))||!matches!(target_url.scheme(),"http"|"https")||!target_url.username().is_empty()||target_url.password().is_some(){return Err(invalid("deferred Fetch requires an HTTP URL without credentials"));}
        if request.upgrade_mixed_image{return Err(invalid("deferred Fetch cannot use image authority"));}
        let fetch=request.fetch.as_ref().ok_or_else(||invalid("missing deferred Fetch options"))?;fetch.validate().map_err(invalid)?;
        if fetch.mode=="navigate"||fetch.privileged_range{return Err(invalid("invalid deferred Fetch request mode"));}
        let method=tungstenite::http::Method::from_bytes(request.method.as_bytes()).map_err(|_|invalid("invalid deferred Fetch method"))?;
        if matches!(method.as_str().to_ascii_uppercase().as_str(),"CONNECT"|"TRACE"|"TRACK")||(!request.body.is_empty()&&matches!(method.as_str(),"GET"|"HEAD")){return Err(invalid("invalid deferred Fetch method/body"));}
        if !matches!(request.credentials.as_str(),"include"|"omit"|"same-origin"){return Err(invalid("invalid deferred Fetch credentials mode"));}
        if fetch.mode=="no-cors"&&!matches!(method.as_str(),"GET"|"HEAD"|"POST"){return Err(invalid("no-cors deferred Fetch method is not safelisted"));}
        let top=self.page_url(page).map_err(transport)?;let ancestors=self.window_ancestor_urls(&owner).map_err(transport)?;
        let admitted_policy=self.native_permissions_policy_snapshot(&owner).map_err(transport)?;
        if !admitted_policy.allows("deferred-fetch",None)&&!admitted_policy.allows("deferred-fetch-minimal",None){return Err(DeferredFetchError::Quota("deferred Fetch disabled by document policy".into()));}
        let control=self.deferred_control_document(&owner).map_err(transport)?;
        let snapshot=self.native_permissions_policy_snapshot(&control).map_err(transport)?;
        let quota=if matches!(&control,AgentOwner::Window{target:oryn_js::message_wire::WindowTarget::Root,..}){
            if !snapshot.allows("deferred-fetch",None){0}else if snapshot.allows("deferred-fetch-minimal",None){TOP_BYTES-MINIMAL_POOL}else{TOP_BYTES}
        }else{self.inner.deferred_fetches.lock().map_err(transport)?.reservations.get(&container_key(&control)).filter(|reservation|snapshot.allows(if reservation.minimal{"deferred-fetch-minimal"}else{"deferred-fetch"},None)).map_or(0,|reservation|reservation.bytes)};
        let referrer=request.referrer_policy.as_deref().filter(|value|!value.is_empty()).map(crate::referrer::ReferrerPolicy::parse_list).unwrap_or_else(||crate::referrer::ReferrerPolicy::default().after_response(&native_policy.headers));
        let frozen_referrer=if request.initiator_url.as_deref()==Some(""){None}else{request.initiator_url.as_deref().and_then(|value|Url::parse(value).ok()).filter(|url|!origin.opaque&&url.origin().ascii_serialization()==origin.serialized).or_else(||Some(source.clone()))};
        let referrer_bytes=frozen_referrer.as_ref().and_then(|source|referrer.header(source,&target_url)).map_or(0,|value|value.len());
        let bytes=oryn_js::fetch_later::request_bytes(&request).and_then(|bytes|bytes.checked_add(referrer_bytes)).ok_or_else(||DeferredFetchError::Quota("deferred Fetch request size overflow".into()))?;
        let plan=Plan{keepalive:self.inner.keepalive_budget.clone(),page,creator:creator.clone(),source,origin:origin.serialized,opaque:origin.opaque,top,ancestors,policy,embedder:crate::agent_isolation::DocumentIsolationPolicy::from_headers(&native_policy.headers),coep:crate::coep_reporting::Policy::new(&native_policy.url,&native_policy.headers),referrer,request,bytes,control,quota};
        self.window_worker_creator(page,target,*document_generation,*stream_epoch).map_err(transport)?;
        crate::resource_io_executor::ready().map_err(transport)?;crate::http_cleanup_custody::ready().map_err(transport)?;
        let deadline=activate_after.map(|delay|Instant::now().checked_add(delay).ok_or_else(||invalid("deferred Fetch deadline overflow"))).transpose()?;
        let activation=Arc::new(AtomicU8::new(3));
        scheduler()?.reserve(ScheduledOwner::Context(Arc::downgrade(&self.inner)),page,token.clone(),Arc::downgrade(&activation),deadline)?;
        self.inner.deferred_fetches.lock().map_err(transport)?.insert(token.clone(),plan,hash,activation,deadline)?;
        // Retirement may race the final registry insertion. A second native
        // identity lookup closes the gap; retirement after it sees the entry.
        if self.window_worker_creator(page,target,*document_generation,*stream_epoch).is_err(){self.activate_deferred_fetch(page,&token)?;}Ok(())
    }
    pub(crate) fn activate_deferred_fetch(&self,page:PageId,token:&DeferredFetchToken)->Result<DeferredFetchState,DeferredFetchError>{
        let registry=self.inner.deferred_fetches.lock().map_err(transport)?;let _=registry.state(page,token)?;
        if let Some(entry)=registry.entries.get(&token.0){let _=entry.activation.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire);}
        if let Some(Ok(pool))=SCHEDULER.get(){pool.changed.notify_one();}registry.state(page,token)
    }
    pub(crate) fn acknowledge_deferred_fetch(&self,page:PageId,token:&DeferredFetchToken)->Result<(),DeferredFetchError>{self.inner.deferred_fetches.lock().map_err(transport)?.acknowledge(page,token)}
    pub(crate) fn deferred_fetch_state(&self,page:PageId,token:&DeferredFetchToken)->Result<DeferredFetchState,DeferredFetchError>{self.inner.deferred_fetches.lock().map_err(transport)?.state(page,token)}
    fn dispatch_deferred_fetch(&self,page:PageId,token:&DeferredFetchToken)->Result<(),DeferredFetchError>{
        let mut registry=self.inner.deferred_fetches.lock().map_err(transport)?;
        if registry.closed{return Ok(());}
        let Some(entry)=registry.entries.get(&token.0)else{return Ok(());};
        if entry.page!=page{return Err(invalid("foreign deferred Fetch capability"));}
        if entry.activation.load(Ordering::Acquire)!=1{return Ok(());}
        let Some(plan)=entry.pending.clone()else{return Ok(());};
        let broker=self.inner.broker.clone().ok_or_else(||transport("deferred Fetch broker unavailable"))?.for_page(page.0).new_request_scope();
        let scope=registry.scope.clone();let ticket=Ticket::reserve(&scope,0,broker.clone()).map_err(transport)?;
        let context=Arc::downgrade(&self.inner);
        // The executor either accepts the complete closure or drops it before
        // any IO. Keep the plan pending on failure so native lifecycle retries.
        crate::resource_io_executor::submit(move||{
            if ticket.cancelled(){return;}
            let _=deliver(&broker,&scope,&plan,&ticket,|violation|{
                if let Some(inner)=context.upgrade(){let context=BrowserContext{inner};match violation{SecurityReport::Csp(violation)=>for target in plan.policy.report_targets(&violation){let _=context.queue_native_csp_report(Some(page),plan.top.clone(),target,violation.clone());},SecurityReport::Coep(reports)=>{let _=context.deliver_coep_reports(&plan.top,reports);}}}
            });
        }).map_err(transport)?;
        registry.submitted(&token.0);Ok(())
    }
    pub(crate) fn cancel_deferred_fetch(&self,page:PageId,creator:&Creator,token:&DeferredFetchToken)->Result<DeferredFetchState,DeferredFetchError>{
        let known={let registry=self.inner.deferred_fetches.lock().map_err(transport)?;registry.entries.contains_key(&token.0)||registry.cancelled.contains_key(&token.0)};
        if !known{let owner=issuer(page,creator)?;self.native_worker_authority(&owner).map_err(|_|invalid("deferred Fetch issuer retired"))?;}
        let result=self.inner.deferred_fetches.lock().map_err(transport)?.cancel(page,creator,token);
        if !known{let owner=issuer(page,creator)?;if self.native_worker_authority(&owner).is_err(){self.retire_deferred_issuer(&owner);}}
        result
    }
    pub(crate) fn retire_deferred_issuer(&self,owner:&AgentOwner){self.inner.deferred_fetches.lock().unwrap_or_else(|error|error.into_inner()).retire_issuer(owner);}
    pub(crate) fn flush_deferred_document(&self,owner:&crate::agent_registry::AgentOwner)->Result<(),String>{
        let crate::agent_registry::AgentOwner::Window{page,target,generation,epoch}=owner else{return Ok(());};
        let creator=Creator::Window{target:target.clone(),document_generation:*generation,stream_epoch:*epoch};
        let tokens=self.inner.deferred_fetches.lock().map_err(|_|"deferred Fetch registry poisoned")?.entries.iter().filter(|(_,entry)|entry.page==*page&&entry.creator==creator&&entry.pending.is_some()).map(|(token,_)|DeferredFetchToken(token.clone())).collect::<Vec<_>>();
        let mut errors=Vec::new();for token in tokens{if let Err(error)=self.activate_deferred_fetch(*page,&token){errors.push(error.to_string());}}if errors.is_empty(){Ok(())}else{Err(errors.join("; "))}
    }
}
