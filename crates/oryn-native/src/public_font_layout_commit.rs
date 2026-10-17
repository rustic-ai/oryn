impl RawV8Host {
    fn demand_public_font_layout(&mut self,realm:RealmId,demand:u64)->Result<bool,HostError>{
        if !self.realm_execution_live(realm){return Ok(false);}
        let state=self.retained_binding_state(realm)?;
        let owner=oryn_js::v8_bindings::font_loading_transport::FontLoadOwner{
            instance:state.native_font_owner_instance(),document:state.document_root(),generation:state.document_generation(),epoch:state.document_stream_epoch(),
        };
        Ok(state.demand_native_font_layout(oryn_js::v8_bindings::font_loading_transport::FontDemandStamp{owner,demand}))
    }
    fn commit_public_font_layout(&mut self,realm:RealmId,demand:u64)->Result<bool,HostError>{
        if !self.realm_execution_live(realm){return Ok(false);}
        let state=self.retained_binding_state(realm)?;
        let context=self.contexts.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)?;
        let owner=oryn_js::v8_bindings::font_loading_transport::FontLoadOwner{
            instance:state.native_font_owner_instance(),document:state.document_root(),generation:state.document_generation(),
            epoch:state.document_stream_epoch(),
        };
        let stamp=oryn_js::v8_bindings::font_loading_transport::FontDemandStamp{owner,demand};
        let deadline=ExecutionDeadline::arm(&self.isolate,self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,context);
        let _admission=state.enter_retained_allocation(scope).map_err(|error|HostError::Execution(error.to_string()))?;
        let committed=state.commit_native_font_layout(scope,stamp).map_err(|error|HostError::Execution(error.to_string()))?;
        if deadline.expired()||scope.is_execution_terminating(){return Err(HostError::Execution("font layout publication interrupted".into()));}
        Ok(committed)
    }
}
