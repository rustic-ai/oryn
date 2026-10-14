impl RawV8Host {
    pub(crate) fn retained_binding_state(&self, realm: RealmId) -> Result<Rc<oryn_js::v8_bindings::V8BindingState>, HostError> {
        self.binding_states.get(&realm).cloned().ok_or(HostError::CachedAuthorityExpired)
    }
    #[cfg(unix)]
    pub(crate) fn retained_stylesheet_resource_endpoint(
        &self,
        realm: RealmId,
    ) -> Result<(crate::context_service::SharedContextClient, PageId), HostError> {
        let endpoint = self
            .context_endpoint_for_realm(realm)
            .ok_or(HostError::CachedAuthorityExpired)?;
        Ok((endpoint.client, endpoint.page_id))
    }

    pub(crate) fn refresh_native_retained_allocation(
        &mut self,
        realm: RealmId,
        allocation: &oryn_js::v8_bindings::BindingRetainedAllocation,
    ) -> Result<(), HostError> {
        if !self.realm_execution_live(realm) {
            return Err(HostError::CachedAuthorityExpired);
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or(HostError::CachedAuthorityExpired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        allocation
            .refresh(scope)
            .map_err(|error| HostError::Execution(error.to_string()))
    }

    pub(crate) fn capture_native_retained_allocation(
        &mut self,
        realm: RealmId,
        canceled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<oryn_js::v8_bindings::BindingRetainedAllocation, HostError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("retained allocation realm retired".into()))?;
        if state.execution_retired() || !self.realm_execution_live(realm) {
            return Err(HostError::CachedAuthorityExpired);
        }
        let context =
            self.contexts.get(&realm).cloned().ok_or_else(|| {
                HostError::Execution("retained allocation context missing".into())
            })?;
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        state
            .capture_retained_allocation(scope, canceled)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
}
