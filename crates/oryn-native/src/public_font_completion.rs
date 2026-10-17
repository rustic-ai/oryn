impl RawV8Host {
    fn public_font_request_current(&self,realm:RealmId,token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken)->bool{
        self.realm_execution_live(realm) && self.retained_binding_state(realm)
            .is_ok_and(|state|state.native_font_request_current(token))
    }
    fn complete_public_font(&mut self,realm:RealmId,completion:oryn_js::v8_bindings::font_loading_transport::FontLoadCompletion)
        ->Result<bool,HostError>{
        if !self.public_font_request_current(realm,completion.token){return Ok(false);}
        let state=self.retained_binding_state(realm)?;
        let context=self.contexts.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)?;
        v8::scope!(let handles,&mut self.isolate);
        let context=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,context);
        let _admission=state.enter_retained_allocation(scope).map_err(|error|HostError::Execution(error.to_string()))?;
        state.complete_native_font_request(scope,completion).map_err(|error|HostError::Execution(error.to_string()))
    }
}
impl RustExecutableDocument {
    fn install_public_font_result(&mut self,index:usize)->Result<(),HostError>{
        use sha2::Digest;
        let job=&self.public_font_jobs[index];
        let binary=job.ready.as_ref().ok_or_else(||HostError::Execution("public font result absent".into()))?
            .result.as_ref().map_err(|error|HostError::Execution(error.to_string()))?;
        if !self.agent.host.public_font_request_current(job.realm,job.token){return Ok(());}
        let state=self.agent.host.retained_binding_state(job.realm)?;
        let current=state.native_font_request_face(job.token).ok_or(HostError::CachedAuthorityExpired)?;
        let provider=self.agent.host.native_layout_provider_in_realm(job.realm).ok_or(HostError::CachedAuthorityExpired)?;
        let backend=provider.native_backend().and_then(|value|value.downcast_ref::<LiveLayoutProvider>())
            .ok_or_else(||HostError::Execution("public font native provider absent".into()))?;
        let bytes=binary.bytes.len().checked_add(job.request.family.len())
            .and_then(|n|n.checked_add(job.request.descriptors.len()))
            .and_then(|n|n.checked_add(binary.url.len())).and_then(|n|n.checked_add(64))
            .ok_or_else(||HostError::Execution("public font owned bytes overflow".into()))?;
        let custody=ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes,bytes)
            .ok_or_else(||HostError::LimitExceeded{resource:"native layout font bytes".into()})?;
        let shared=self.agent.host.native_retained_admission_for_realm(job.realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        let _scope=self.agent.host.retained_allocation_scope(job.realm)?;
        let allocation=shared.reserve(bytes.checked_add(std::mem::size_of::<PublicLayoutFont>()).ok_or_else(||HostError::Execution("public font publication metadata overflow".into()))?)
            .map_err(HostError::Execution)?;
        if !self.agent.host.public_font_request_current(job.realm,job.token){return Ok(());}
        let font=oryn_layout::WebFont{pending:false,blocked:false,order:0,
            family:job.request.family.clone(),descriptors:job.request.descriptors.clone(),
            source_url:binary.url.clone(),sha256:format!("{:x}",sha2::Sha256::digest(&binary.bytes)),
            bytes:binary.bytes.clone()};
        backend.install_public_font(job.token,current,font,Arc::new((custody,allocation))).map_err(HostError::Execution)
    }
    fn poll_public_fonts(&mut self)->Result<(),HostError>{
        use oryn_js::v8_bindings::font_loading_transport::{FontLoadCompletion,FontLoadTerminal};
        let mut index=0;
        while index<self.public_font_jobs.len(){
            let current={let job=&self.public_font_jobs[index];self.agent.host.public_font_request_current(job.realm,job.token)};
            if !current {
                self.public_font_jobs[index].cancellation.cancel();
                // Cancellation is not an IO completion receipt. Retain job
                // metadata and receiver until the actual producer replies or
                // disconnects, then release outside any owner state borrow.
                if self.public_font_jobs[index].ready.is_some()
                    || !matches!(self.public_font_jobs[index].receiver.try_recv(),Err(std::sync::mpsc::TryRecvError::Empty)) {
                    drop(self.public_font_jobs.remove(index));
                }else{index+=1;}
                continue;
            }
            if let Some(terminal)=self.public_font_jobs[index].terminal {
                let job=&self.public_font_jobs[index];
                let completion=FontLoadCompletion{token:job.token,terminal};
                if self.agent.host.complete_public_font(job.realm,completion)? {
                    drop(self.public_font_jobs.remove(index));
                }else{index+=1;}
                continue;
            }
            if let Some(error)=self.public_font_jobs[index].ready.as_ref().and_then(|reply|reply.result.as_ref().err()) {
                return Err(match error {
                    HostError::ScriptLoad(failure)=>HostError::ScriptLoad(failure.clone()),
                    HostError::LimitExceeded{resource}=>HostError::LimitExceeded{resource:resource.clone()},
                    HostError::CachedAuthorityExpired=>HostError::CachedAuthorityExpired,
                    error=>HostError::Execution(error.to_string()),
                });
            }
            if self.public_font_jobs[index].ready.as_ref().is_some_and(|reply|reply.result.is_ok()){
                self.install_public_font_result(index)?;
                self.public_font_jobs[index].terminal=Some(FontLoadTerminal::Decoded);
                let job=&self.public_font_jobs[index];
                let completion=FontLoadCompletion{token:job.token,terminal:FontLoadTerminal::Decoded};
                if self.agent.host.complete_public_font(job.realm,completion)? {
                    drop(self.public_font_jobs.remove(index));
                }else{index+=1;}
                continue;
            }
            let reply=self.public_font_jobs[index].receiver.try_recv();
            let terminal=match reply {
                Err(std::sync::mpsc::TryRecvError::Empty)=>{index+=1;continue;},
                Err(std::sync::mpsc::TryRecvError::Disconnected)=>return Err(HostError::Execution("public font producer disconnected without completion".into())),
                Ok(response)=>{
                    let failed=response.result.is_err();
                    // Installation must commit to the actual native font store
                    // before promises receive a Decoded terminal publication.
                    self.public_font_jobs[index].ready=Some(response);
                    if failed {
                        // Keep the exact producer diagnostic in ready custody;
                        // the owning coordinator classifies source failures
                        // versus infrastructure before terminal settlement.
                        let error=self.public_font_jobs[index].ready.as_ref().unwrap().result.as_ref().err().unwrap();
                        if matches!(error,HostError::ScriptLoad(failure) if failure.is_fetch_failure()) {
                            FontLoadTerminal::Exhausted
                        }else{return Err(match error {
                            HostError::ScriptLoad(failure)=>HostError::ScriptLoad(failure.clone()),
                            HostError::LimitExceeded{resource}=>HostError::LimitExceeded{resource:resource.clone()},
                            HostError::CachedAuthorityExpired=>HostError::CachedAuthorityExpired,
                            error=>HostError::Execution(error.to_string()),
                        });}
                    }else{
                        self.install_public_font_result(index)?;
                        FontLoadTerminal::Decoded
                    }
                },
            };
            self.public_font_jobs[index].terminal=Some(terminal);
            let job=&self.public_font_jobs[index];
            let completion=FontLoadCompletion{token:job.token,terminal};
            let realm=job.realm;
            if self.agent.host.complete_public_font(realm,completion)? {
                let job=self.public_font_jobs.remove(index);
                drop(job);
            }else{index+=1;}
        }
        Ok(())
    }
}
