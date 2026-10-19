impl RawV8Host {
    // Prepare the current parent-owned Window before any author callback can
    // synchronously create an auxiliary. Never replace an embedding provider.
    #[cfg(unix)]
    pub(crate) fn prepare_auxiliary_creator_fonts(&mut self) -> Result<(), HostError> {
        let realm = self.active_realm;
        if !self.realm_execution_live(realm) { return Err(HostError::CachedAuthorityExpired); }
        let state = self.retained_binding_state(realm)?;
        if self.context_endpoint_for_realm(realm).is_none() { return Ok(()); }
        if self.native_layout_provider_in_realm(realm).is_none() {
            let provider = crate::executable::initial_layout_provider(state.dom(), state.document_root());
            self.install_layout_provider(provider)?;
        }
        let provider = self.native_layout_provider_in_realm(realm).ok_or(HostError::CachedAuthorityExpired)?;
        if !provider.native_backend().is_some_and(|backend|backend.is::<LiveLayoutProvider>()) {
            return Ok(());
        }
        self.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        Ok(())
    }
    fn bootstrap_callback_public_fonts(&mut self) -> Result<(), HostError> {
        // The executable checkpoint adopts contexts before reaching font work.
        // No V8 scope, binding-state borrow, or provider-slot borrow spans this
        // scalar walk or the allocation factory's owning-context refresh.
        let mut after = None;
        loop {
            let next=self.next_native_font_realm(after,false);
            let Some(realm) = next else { return Ok(()); };
            after = Some(realm);
            let Some(layout) = self.native_layout_provider_in_realm(realm) else {
                continue;
            };
            if !layout.native_backend().is_some_and(|backend|
                backend.is::<LiveLayoutProvider>()) {
                continue;
            }
            let identity = Rc::as_ptr(&layout) as *const () as usize;
            let state = self.retained_binding_state(realm)?;
            if state.native_font_provider_matches(identity) { continue; }
            // Strong layout ownership above prevents pointer ABA during this
            // test. activate_public_font_loading rechecks exact provider and
            // document authority after all admission callbacks before publish.
            self.activate_public_font_loading(realm)?;
        }
    }
}
