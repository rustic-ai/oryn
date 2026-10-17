impl RawV8Host {
    fn synchronize_public_font_set(&mut self,realm:RealmId)->Result<(),HostError>{
        if !self.realm_execution_live(realm){return Ok(());}
        let state=self.retained_binding_state(realm)?;
        let context=self.contexts.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)?;
        v8::scope!(let handles,&mut self.isolate);
        let context=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,context);
        let _admission=state.enter_retained_allocation(scope).map_err(|error|HostError::Execution(error.to_string()))?;
        state.synchronize_native_font_set(scope).map_err(|error|HostError::Execution(error.to_string()))
    }
    fn publish_implicit_font_transition(
        &mut self,realm:RealmId,identity:usize,
    )->Result<oryn_js::v8_bindings::font_loading_transport::ImplicitFontTokens,HostError>{
        if !self.realm_execution_live(realm){return Ok(oryn_js::v8_bindings::font_loading_transport::ImplicitFontTokens::empty());}
        let state=self.retained_binding_state(realm)?;
        let context=self.contexts.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)?;
        v8::scope!(let handles,&mut self.isolate);
        let context=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,context);
        let _admission=state.enter_retained_allocation(scope).map_err(|error|HostError::Execution(error.to_string()))?;
        state.start_implicit_font(scope,identity).map_err(|error|HostError::Execution(error.to_string()))
    }
    fn complete_implicit_font_tokens(&mut self,realm:RealmId,
        tokens:&[oryn_js::v8_bindings::font_loading_transport::FontLoadToken],success:bool)
        ->Result<(),HostError>{
        if !self.realm_execution_live(realm){return Ok(());}
        let state=self.retained_binding_state(realm)?;
        let context=self.contexts.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)?;
        v8::scope!(let handles,&mut self.isolate);
        let context=v8::Local::new(handles,&context);
        let scope=&mut v8::ContextScope::new(handles,context);
        let _admission=state.enter_retained_allocation(scope).map_err(|error|HostError::Execution(error.to_string()))?;
        state.complete_implicit_font(scope,tokens,success).map_err(|error|HostError::Execution(error.to_string()))
    }
}
