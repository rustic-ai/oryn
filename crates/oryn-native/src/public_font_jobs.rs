// Explicit FontFace work uses the same parent broker destination, native
// document initiator and cancellation scope as CSS font demands.
struct PublicFontJob {
    realm:RealmId,
    token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken,
    request:oryn_js::v8_bindings::font_loading_transport::FontLoadRequest,
    cancellation:Arc<crate::context_service::ResourceCancellation>,
    receiver:std::sync::mpsc::Receiver<PublicFontResult>,
    ready:Option<PublicFontResult>,
    terminal:Option<oryn_js::v8_bindings::font_loading_transport::FontLoadTerminal>,
    _metadata:Arc<dyn Send+Sync>,
    _cancellation_metadata:Arc<dyn Send+Sync>,
}
struct PublicFontResult {
    result:Result<dynamic_stylesheet_preparation::CssBinary,HostError>,
    _bytes:Option<ScriptSourceReservation>,
    _producer_owners:[Arc<dyn Send+Sync>;4],
}
impl PublicFontJob {
    fn current(&self,host:&RawV8Host)->bool {
        host.public_font_request_current(self.realm,self.token)
    }
}
impl Drop for PublicFontJob {
    fn drop(&mut self){self.cancellation.cancel();}
}
impl RustExecutableDocument {
    fn schedule_public_font(&mut self,realm:RealmId,slot:&mut Option<oryn_js::v8_bindings::font_loading_transport::FontLoadRequest>)
        ->Result<(),HostError>{
        // The caller retains the exact request through every fallible prepare
        // step. Only successful executor submission transfers it to a job.
        let request=slot.as_ref().ok_or_else(||HostError::Execution("font request slot empty".into()))?;
        let dom=self.agent.host.automation_dom(realm).ok_or(HostError::CachedAuthorityExpired)?;
        let fetch=ParserFetchContext::capture(&self.agent.host,&dom,request.token.owner.document)?;
        if fetch.identity().document!=request.token.owner.document
            || fetch.identity().document_generation!=request.token.owner.generation
            || request.token.owner.epoch!=0 {return Err(HostError::CachedAuthorityExpired);}
        let creator=fetch.resource_initiator.clone().ok_or_else(||HostError::Execution("font native initiator absent".into()))?;
        let endpoint=self.context.as_ref().map(|(_,page_id,_)|self.agent.host.resource_client_for_realm(realm,*page_id)
            .map(|client|(client,*page_id))).transpose()?;
        let standalone=if endpoint.is_none(){
            let (loader,cancel)=standalone_stylesheet::Loader::capture(&mut self.agent.host,&self.resources,&fetch,self.dynamic_scripts.source_bytes.clone())?;
            Some((loader,cancel))
        }else{None};
        let admission=self.agent.host.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        let scope=self.agent.host.retained_allocation_scope(realm)?;
        let cancellation_layout=std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize;2]>()
            .extend(std::alloc::Layout::new::<crate::context_service::ResourceCancellation>())
            .map_err(|_|HostError::Execution("font cancellation metadata layout".into()))?.0.pad_to_align();
        let cancellation_owner=admission.reserve(cancellation_layout.size()).map_err(HostError::Execution)?;
        let producer_cancellation_owner=cancellation_owner.clone();
        let cancellation=Arc::new(crate::context_service::ResourceCancellation::default());
        let (sender,receiver)=std::sync::mpsc::channel();
        let source_bytes=request.sources.iter().try_fold(0usize,|n,source|{
            let size=match source {
                oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(url)
                |oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(url)=>url.len(),
                oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Bytes(bytes)=>bytes.len(),
            };
            n.checked_add(size)
        }).and_then(|n|n.checked_add(request.sources.len().checked_mul(std::mem::size_of::<oryn_js::v8_bindings::font_loading_transport::FontLoadSource>())?))
            .and_then(|n|n.checked_add(request.base_url.len())).ok_or_else(||HostError::Execution("font work metadata overflow".into()))?;
        let work_allocation=admission.reserve(source_bytes).map_err(HostError::Execution)?;
        let source_owner=request.source_allocation.clone();
        let request_owner=request.allocation.clone();
        let metadata=admission.reserve(std::mem::size_of::<PublicFontJob>()).map_err(HostError::Execution)?;
        let producer_metadata=metadata.clone();
        let capacity=self.public_font_jobs.capacity();
        if self.public_font_jobs.len()==capacity{
            let target=capacity.checked_mul(2).unwrap_or(1024).max(1).min(1024);
            if target<=capacity{return Err(HostError::LimitExceeded{resource:"native layout font capacity".into()})}
            let charge=admission.reserve(target.checked_mul(std::mem::size_of::<PublicFontJob>()).ok_or_else(||HostError::Execution("font job capacity overflow".into()))?).map_err(HostError::Execution)?;
            let mut replacement=Vec::new();replacement.try_reserve_exact(target).map_err(|_|HostError::Execution("public font job allocation".into()))?;
            if replacement.capacity()>target{return Err(HostError::Execution("public font capacity exceeded admission".into()))}
            if !self.agent.host.public_font_request_current(realm,request.token){return Err(HostError::CachedAuthorityExpired)}
            replacement.append(&mut self.public_font_jobs);
            self.public_font_jobs=replacement;
            let old=self.public_font_job_capacity.replace(charge);drop(old);
        }
        let copy_text=|source:&str|->Result<String,HostError>{
            let mut value=String::new();value.try_reserve_exact(source.len()).map_err(|_|HostError::Execution("font source text allocation".into()))?;
            if value.capacity()>source.len(){return Err(HostError::Execution("font source text exceeded admission".into()));}
            value.push_str(source);Ok(value)
        };
        let mut sources=Vec::new();
        sources.try_reserve_exact(request.sources.len()).map_err(|_|HostError::Execution("font work source allocation".into()))?;
        if sources.capacity()>request.sources.len(){return Err(HostError::Execution("font source vector exceeded admission".into()));}
        for source in &request.sources {
            sources.push(match source {
            oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(url)=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(copy_text(url)?),
            oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(name)=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(copy_text(name)?),
            oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Bytes(bytes)=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Bytes({let mut value=Vec::new();value.try_reserve_exact(bytes.len()).map_err(|_|HostError::Execution("font source byte allocation".into()))?;if value.capacity()>bytes.len(){return Err(HostError::Execution("font source bytes exceeded admission".into()));}value.extend_from_slice(bytes);value}),
            });
        }
        let base=copy_text(&request.base_url)?;let cancel=cancellation.clone();
        let body_budget=self.dynamic_scripts.source_bytes.clone();
        if !self.agent.host.public_font_request_current(realm,request.token){return Err(HostError::CachedAuthorityExpired);}
        drop(scope);
        resource_io_executor::submit(move||{
            let _work_allocation=work_allocation;
            let _cancellation_owner=producer_cancellation_owner;
            let _source_owner=source_owner;
            let _request_owner=request_owner;
            let _producer_metadata=producer_metadata;
            let (mut loader,_standalone_cancel)=match standalone{Some((loader,cancel))=>(Some(loader),Some(cancel)),None=>(None,None)};
            let mut prepaid=None;
            let mut result=Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch("font sources exhausted".into())));
            for source in sources {
                drop(prepaid.take());
                if cancel.flag.load(std::sync::atomic::Ordering::Acquire){result=Err(HostError::CachedAuthorityExpired);break;}
                let url=match source {
                    oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Bytes(bytes)=>{
                        prepaid=ScriptSourceReservation::reserve(&body_budget,bytes.len());
                        if prepaid.is_none(){result=Err(HostError::LimitExceeded{resource:"CSS binary retained bytes".into()});break;}
                        if oryn_layout::decode_web_font(&bytes,dynamic_stylesheet_preparation::NATIVE_FONT_DECODE_BYTES).is_ok(){
                            result=Ok(dynamic_stylesheet_preparation::CssBinary{url:String::new(),bytes});break;
                        }continue;
                    },
                    oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(name)=>{
                        if let Some(bytes)=oryn_layout::local_font_bytes(&name){
                            prepaid=ScriptSourceReservation::reserve(&body_budget,bytes.len());
                            if prepaid.is_none(){result=Err(HostError::LimitExceeded{resource:"CSS binary retained bytes".into()});break;}
                            if oryn_layout::decode_web_font(bytes,dynamic_stylesheet_preparation::NATIVE_FONT_DECODE_BYTES).is_ok(){
                                result=Ok(dynamic_stylesheet_preparation::CssBinary{url:name,bytes:bytes.to_vec()});break;
                            }
                        }
                        continue;
                    },
                    oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(url)=>url,
                };
                let reference=oryn_layout::cssom::CssResourceReference{url,destination:oryn_layout::cssom::CssResourceDestination::Font};
                if let Some(loader)=loader.as_mut(){
                    match loader.load_dependency_next(&base,&base,"",reference){
                        Ok(binary)=>{
                            if oryn_layout::decode_web_font(&binary.bytes,dynamic_stylesheet_preparation::NATIVE_FONT_DECODE_BYTES).is_ok(){
                                prepaid=ScriptSourceReservation::reserve(&body_budget,binary.bytes.len());result=Ok(binary);break;
                            }continue;
                        },
                        Err(error)=>{
                            let source_failure=matches!(&error,HostError::ScriptLoad(failure) if failure.is_fetch_failure());
                            result=Err(error);if source_failure{continue;}else{break;}
                        },
                    }
                }
                let (client,page_id)=endpoint.as_ref().expect("captured native font endpoint");
                let reply=cancel.run(client,*page_id,creator.creator.clone(),
                    css_subresource_request(*page_id,&creator,&base,&base,"",reference));
                let binary=match reply {
                    Ok(crate::context_service::ContextServiceResponse::PageBinaryResource{url,body_base64,..})=>{
                        if body_base64.len()>dynamic_stylesheet_preparation::CSS_BINARY_RESPONSE_BYTES {
                            result=Err(HostError::LimitExceeded{resource:"CSS binary response".into()});break;
                        }
                        let decoded=body_base64.len().checked_div(4).and_then(|n|n.checked_add(1))
                            .and_then(|n|n.checked_mul(3));
                        prepaid=decoded.and_then(|bytes|ScriptSourceReservation::reserve(&body_budget,bytes));
                        if prepaid.is_none(){result=Err(HostError::LimitExceeded{resource:"CSS binary retained bytes".into()});break;}
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD,body_base64)
                            .map(|bytes|dynamic_stylesheet_preparation::CssBinary{url,bytes}).map_err(|error|HostError::Execution(error.to_string()))
                    },
                    Ok(_)=>{result=Err(HostError::Execution("invalid font broker response".into()));break;},
                    Err(error)=>{
                        let failure=crate::script_load_failure::ScriptLoadFailure::decoded(error);
                        if failure.is_fetch_failure(){result=Err(HostError::ScriptLoad(failure));continue;}
                        result=Err(HostError::ScriptLoad(failure));break;
                    },
                };
                match binary {
                    Ok(binary)=>{if oryn_layout::decode_web_font(&binary.bytes,dynamic_stylesheet_preparation::NATIVE_FONT_DECODE_BYTES).is_ok(){result=Ok(binary);break;}},
                    Err(error)=>{result=Err(error);break;},
                }
            }
            // The prepaid payload lease transfers into the completion before
            // the work allocation drops, including cancelled receiver paths.
            let response=match result {
                Ok(binary)=>match prepaid {
                    Some(bytes)=>PublicFontResult{result:Ok(binary),_bytes:Some(bytes),_producer_owners:[_work_allocation,_source_owner,_request_owner,_producer_metadata]},
                    None=>PublicFontResult{result:Err(HostError::LimitExceeded{resource:"CSS binary retained bytes".into()}),_bytes:None,_producer_owners:[_work_allocation,_source_owner,_request_owner,_producer_metadata]},
                },
                Err(error)=>PublicFontResult{result:Err(error),_bytes:None,_producer_owners:[_work_allocation,_source_owner,_request_owner,_producer_metadata]},
            };
            let _=sender.send(response);
        }).map_err(HostError::Execution)?;
        let request=slot.take().expect("submitted font request retains its caller slot");
        self.public_font_jobs.push(PublicFontJob{realm,token:request.token,request,cancellation,receiver,ready:None,terminal:None,_metadata:metadata,_cancellation_metadata:cancellation_owner});
        Ok(())
    }
}
