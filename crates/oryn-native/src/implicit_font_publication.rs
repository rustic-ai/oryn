impl RawV8Host {
    fn synchronize_public_font_set(&mut self,realm:RealmId)->Result<(),HostError>{
        if !self.font_publication_live(realm){return Ok(());}
        self.with_public_font_scope(realm,|scope,state|state.synchronize_native_font_set(scope).map_err(|error|HostError::Execution(error.to_string())))
    }
    fn publish_implicit_font_transition(
        &mut self,realm:RealmId,identity:usize,
    )->Result<oryn_js::v8_bindings::font_loading_transport::ImplicitFontTokens,HostError>{
        if !self.font_publication_live(realm){return Ok(oryn_js::v8_bindings::font_loading_transport::ImplicitFontTokens::empty());}
        self.with_public_font_scope(realm,|scope,state|state.start_implicit_font(scope,identity).map_err(|error|HostError::Execution(error.to_string())))
    }
    fn complete_implicit_font_tokens(&mut self,realm:RealmId,
        tokens:&[oryn_js::v8_bindings::font_loading_transport::FontLoadToken],success:bool)
        ->Result<(),HostError>{
        if !self.font_publication_live(realm){return Ok(());}
        self.with_public_font_scope(realm,|scope,state|state.complete_implicit_font(scope,tokens,success).map_err(|error|HostError::Execution(error.to_string())))
    }
}
