impl RawV8Host {
    fn retained_allocation_scope(
        &self,
        realm: RealmId,
    ) -> Result<oryn_js::v8_bindings::BindingRetainedAllocationScope, HostError> {
        let provider = self
            .native_layout_provider_in_realm(realm)
            .ok_or(HostError::CachedAuthorityExpired)?;
        let backend = provider
            .native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
            .ok_or_else(|| HostError::Execution("retained scope provider unavailable".into()))?;
        let allocation = backend
            .native_retained_allocation
            .try_borrow()
            .map_err(|_| HostError::Execution("retained scope allocation borrow conflict".into()))?
            .as_ref()
            .cloned()
            .ok_or_else(|| HostError::Execution("retained scope receipt absent".into()))?;
        Ok(allocation.scope_guard())
    }

    fn invalidate_native_retained_allocation(&self, realm: RealmId) -> Result<(), HostError> {
        let Some(provider) = self.native_layout_provider_in_realm(realm) else {
            return Ok(());
        };
        let Some(backend) = provider
            .native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
        else {
            return Ok(());
        };
        let allocation = backend
            .native_retained_allocation
            .try_borrow()
            .map_err(|_| {
                HostError::Execution("retained receipt invalidation borrow conflict".into())
            })?
            .as_ref()
            .cloned();
        if let Some(allocation) = allocation {
            allocation.invalidate().map_err(HostError::Execution)?;
        }
        Ok(())
    }

    fn native_retained_admission_for_realm(
        &mut self,
        realm: RealmId,
    ) -> Result<Option<SharedRetainedAdmission>, HostError> {
        let Some(provider) = self.native_layout_provider_in_realm(realm) else {
            return Ok(None);
        };
        let Some(backend) = provider
            .native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
        else {
            return Ok(None);
        };
        let existing = {
            backend
                .shared_retained_admission
                .try_borrow()
                .map_err(|_| {
                    HostError::Execution("retained authority read borrow conflict".into())
                })?
                .as_ref()
                .cloned()
        };
        if let Some(shared) = existing {
            let allocation = backend
                .native_retained_allocation
                .try_borrow()
                .map_err(|_| {
                    HostError::Execution("retained allocation receipt borrow conflict".into())
                })?
                .as_ref()
                .cloned()
                .ok_or_else(|| {
                    HostError::Execution("retained allocation receipt missing".into())
                })?;
            self.refresh_native_retained_allocation(realm, &allocation)?;
            return Ok(Some(shared));
        }
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let canceled = cancellation.flag.clone();
        let allocation =
            Arc::new(self.capture_native_retained_allocation(realm, canceled.clone())?);
        let admission: SharedRetainedAdmission = Arc::new(NativeRetainedAdmission {
            allocation: allocation.clone(),
            cancellation,
        });
        self.install_native_provider_allocation(realm, backend, admission.clone(), allocation)?;
        Ok(Some(admission))
    }
}
