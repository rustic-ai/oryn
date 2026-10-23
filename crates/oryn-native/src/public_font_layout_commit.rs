impl RawV8Host {
    fn demand_public_font_layout(&mut self,realm:RealmId,demand:u64)->Result<bool,HostError>{
        if !self.font_publication_live(realm){return Ok(false);}
        let state=self.retained_binding_state(realm)?;
        let owner=oryn_js::v8_bindings::font_loading_transport::FontLoadOwner{
            instance:state.native_font_owner_instance(),document:state.document_root(),generation:state.document_generation(),epoch:0,
        };
        Ok(state.demand_native_font_layout(oryn_js::v8_bindings::font_loading_transport::FontDemandStamp{owner,demand}))
    }
    fn commit_public_font_layout(&mut self,realm:RealmId,demand:u64)->Result<bool,HostError>{
        if !self.font_publication_live(realm){return Ok(false);}
        let state=self.retained_binding_state(realm)?;

        let owner=oryn_js::v8_bindings::font_loading_transport::FontLoadOwner{
            instance:state.native_font_owner_instance(),document:state.document_root(),generation:state.document_generation(),
            epoch:0,
        };
        let stamp=oryn_js::v8_bindings::font_loading_transport::FontDemandStamp{owner,demand};
        self.with_public_font_scope(realm,|scope,state|state.commit_native_font_layout(scope,stamp).map_err(|error|HostError::Execution(error.to_string())))
    }
}
