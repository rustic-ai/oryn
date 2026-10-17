impl RawV8Host {
    pub(crate) fn activate_public_font_loading(&mut self,realm:RealmId)->Result<(),HostError>{
        let state=self.retained_binding_state(realm)?;
        let dom=self.automation_dom(realm).ok_or(HostError::CachedAuthorityExpired)?;
        let identity=ParserFetchContext::capture(self,&dom,state.document_root())?.identity();
        let layout=self.native_layout_provider_in_realm(realm).ok_or(HostError::CachedAuthorityExpired)?;
        let provider_identity=Rc::as_ptr(&layout) as *const () as usize;
        if state.native_font_provider_matches(provider_identity){return Ok(());}
        let backend=layout.native_backend().and_then(|value|value.downcast_ref::<LiveLayoutProvider>())
            .ok_or_else(||HostError::Execution("native font loading requires original layout provider".into()))?;
        let registry=backend.retained_stylesheets.clone();
        let backing_provider=layout.clone();
        let shared=self.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        let _scope=self.retained_allocation_scope(realm)?;
        let charge=shared.reserve(std::mem::size_of::<NativeProviderAdmissionFromShared>()+2*std::mem::size_of::<usize>())
            .map_err(HostError::Execution)?;
        let admission=Rc::new(NativeProviderAdmissionFromShared{shared,_charge:charge});
        let backing=backing_provider.native_backend().and_then(|value|value.downcast_ref::<LiveLayoutProvider>())
            .ok_or(HostError::CachedAuthorityExpired)?.implicit_font_store(admission.as_ref()).map_err(HostError::Execution)?;
        sweep_public_font_face_leases(&registry,admission.as_ref()).map_err(HostError::Execution)?;
        prune_implicit_font_backing(&backing,admission.as_ref())?;
        let metadata=admission.reserve(std::mem::size_of::<NativeFontLoadingProvider>()+2*std::mem::size_of::<usize>())
            .map_err(HostError::Execution)?;
        if !self.realm_execution_live(realm)||ParserFetchContext::capture(self,&dom,state.document_root())?.identity()!=identity{
            return Err(HostError::CachedAuthorityExpired);
        }
        let current=self.native_layout_provider_in_realm(realm).ok_or(HostError::CachedAuthorityExpired)?;
        if !Rc::ptr_eq(&current,&layout){return Err(HostError::CachedAuthorityExpired);}
        let provider=NativeFontLoadingProvider{dom,registry,identity,admission,_metadata:metadata,backing,provider_identity,provider:Rc::downgrade(&layout),state:Rc::downgrade(&state)};
        state.install_native_font_loading(Rc::new(provider));
        Ok(())
    }
}
