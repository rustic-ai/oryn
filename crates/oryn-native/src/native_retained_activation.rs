impl RawV8Host {
    pub(crate) fn retained_allocation_scope(
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

    /// Prepare only for the host's verified same-origin initial Window commit.
    /// The receipt accounts native memory, not script/CSP/document authority.
    /// New provider caches are empty and its document is explicit; old charges
    /// stay in the same ledger until their physical owners release them.
    pub(crate) fn prepare_initial_document_provider(
        &mut self,
        realm: RealmId,
        state: &Rc<oryn_js::v8_bindings::V8BindingState>,
        document: NodeId,
    ) -> Result<Option<Rc<dyn oryn_js::v8_bindings::BindingLayoutProvider>>, HostError> {
        let current = self.retained_binding_state(realm)?;
        if !Rc::ptr_eq(&current, state) || !self.realm_execution_live(realm) {
            return Err(HostError::CachedAuthorityExpired);
        }
        let Some(previous) = self.native_layout_provider_in_realm(realm) else { return Ok(None); };
        let old = previous.native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
            .ok_or_else(|| HostError::Execution("initial provider backend unavailable".into()))?;
        if !Rc::ptr_eq(&old.dom, &state.dom()) || old.document != state.document_root() {
            return Err(HostError::CachedAuthorityExpired);
        }
        state.dom().borrow().arena().document_id(document)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let viewport = old.viewport.get();
        let replacement = previous.fork_document(state.dom(), document, viewport.width, viewport.height)
            .ok_or_else(|| HostError::Execution("initial provider fork unavailable".into()))?;
        let shared = old.shared_retained_admission.try_borrow()
            .map_err(|_| HostError::Execution("initial provider authority borrow conflict".into()))?
            .as_ref().cloned();
        if let Some(shared) = shared {
            let allocation = old.native_retained_allocation.try_borrow()
                .map_err(|_| HostError::Execution("initial provider receipt borrow conflict".into()))?
                .as_ref().cloned().ok_or_else(|| HostError::Execution("initial provider receipt missing".into()))?;
            self.refresh_native_retained_allocation(realm, &allocation)?;
            let new = replacement.native_backend().and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
                .ok_or_else(|| HostError::Execution("initial fork backend unavailable".into()))?;
            self.install_native_provider_allocation(realm, new, shared, allocation)?;
        }
        Ok(Some(replacement))
    }

    /// Provider replacement does not replace the binding's owning receipt.
    /// Prepare and publish the exact current authority before replacing the
    /// provider; an error leaves the old provider installed and serviceable.
    pub(crate) fn transfer_native_provider_allocation(
        &mut self,
        realm: RealmId,
        replacement: &Rc<dyn oryn_js::v8_bindings::BindingLayoutProvider>,
    ) -> Result<(), HostError> {
        let Some(previous) = self.native_layout_provider_in_realm(realm) else {
            return Ok(());
        };
        let Some(old) = previous.native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>()) else {
            return Ok(());
        };
        let existing = old.shared_retained_admission.try_borrow()
            .map_err(|_| HostError::Execution("retained transfer authority borrow conflict".into()))?
            .as_ref().cloned();
        let Some(shared) = existing else { return Ok(()); };
        let new = replacement.native_backend()
            .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
            .ok_or_else(|| HostError::Execution("retained transfer backend unavailable".into()))?;
        let state = self.retained_binding_state(realm)?;
        if !Rc::ptr_eq(&old.dom, &new.dom) || old.document != new.document
            || !Rc::ptr_eq(&new.dom, &state.dom()) || new.document != state.document_root()
        {
            return Err(HostError::CachedAuthorityExpired);
        }
        let allocation = old.native_retained_allocation.try_borrow()
            .map_err(|_| HostError::Execution("retained transfer receipt borrow conflict".into()))?
            .as_ref().cloned()
            .ok_or_else(|| HostError::Execution("retained transfer receipt missing".into()))?;
        self.refresh_native_retained_allocation(realm, &allocation)?;
        self.install_native_provider_allocation(realm, new, shared, allocation)
    }

    pub(crate) fn native_retained_admission_for_realm(
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
