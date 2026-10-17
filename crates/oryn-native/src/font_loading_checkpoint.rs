impl RustExecutableDocument {
    fn checkpoint_public_fonts(&mut self)->Result<(),HostError>{
        self.poll_public_fonts()?;
        self.agent.host.bootstrap_callback_public_fonts()?;
        // Realm IDs are monotonically ordered native records. A scalar cursor
        // avoids an unadmitted realm snapshot before entering owning scopes.
        let mut last=None;
        while let Some(realm)=self.agent.host.next_public_font_loading_realm(last){
            last=Some(realm);
            if !self.agent.host.realm_execution_live(realm){continue;}
            self.agent.host.activate_public_font_loading(realm)?;
            self.synchronize_initial_implicit_fonts(realm)?;
            let state=self.agent.host.retained_binding_state(realm)?;
            let _admission=self.agent.host.native_retained_admission_for_realm(realm)?;
            let _scope=self.agent.host.retained_allocation_scope(realm)?;
            let mut requests=state.take_native_font_requests();
            while let Some(request)=requests.requests.pop_front(){
                if !state.native_font_request_current(request.token){drop(request);continue;}
                let mut slot=Some(request);
                if let Err(error)=self.schedule_public_font(realm,&mut slot){
                    if let Some(request)=slot.take(){
                        if state.native_font_request_current(request.token){requests.requests.push_front(request);}else{drop(request);}
                    }
                    // Swap whole queues; no allocation or deep clone is needed
                    // to preserve the original FIFO on admission/submit errors.
                    state.restore_native_font_requests(&mut requests).map_err(|error|HostError::Execution(error.to_string()))?;
                    return Err(error);
                }
            }
            self.poll_public_fonts()?;
            self.refresh_font_ready_layout(realm)?;
        }
        Ok(())
    }
}
impl RawV8Host {
    fn next_public_font_loading_realm(&self,after:Option<RealmId>)->Option<RealmId>{
        self.binding_states.iter().filter(|(realm,state)|after.is_none_or(|last|**realm>last)&&
            self.realm_execution_live(**realm)&&state.native_font_loading_materialized())
            .map(|(realm,_)|*realm).min()
    }
}
