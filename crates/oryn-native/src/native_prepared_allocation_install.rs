struct PreparedProviderAllocationInstall {
    shared: SharedRetainedAdmission,
    allocation: std::sync::Arc<oryn_js::v8_bindings::BindingRetainedAllocation>,
    adapter: Option<Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>>,
}
impl LiveLayoutProvider {
    fn prepare_shared_retained_admission(
        &self,
        shared: SharedRetainedAdmission,
        allocation: std::sync::Arc<oryn_js::v8_bindings::BindingRetainedAllocation>,
    ) -> Result<PreparedProviderAllocationInstall, String> {
        if std::thread::current().id() != self.owner_thread {
            return Err("foreign allocation installation thread".into());
        }
        {
            let existing = self
                .shared_retained_admission
                .try_borrow()
                .map_err(|_| "shared identity borrow conflict")?;
            let native = self
                .retained_admission
                .try_borrow()
                .map_err(|_| "native identity borrow conflict")?;
            let receipt = self
                .native_retained_allocation
                .try_borrow()
                .map_err(|_| "allocation identity borrow conflict")?;
            if let Some(existing) = existing.as_ref() {
                if std::sync::Arc::ptr_eq(existing, &shared)
                    && native.is_some()
                    && receipt
                        .as_ref()
                        .is_some_and(|old| std::sync::Arc::ptr_eq(old, &allocation))
                {
                    return Ok(PreparedProviderAllocationInstall {
                        shared,
                        allocation,
                        adapter: None,
                    });
                }
                return Err("retained authority replacement".into());
            }
            if native.is_some() || receipt.is_some() {
                return Err("partial retained allocation identity".into());
            }
        }
        let charge = shared.reserve(
            std::mem::size_of::<NativeProviderAdmissionFromShared>()
                + 2 * std::mem::size_of::<usize>(),
        )?;
        let adapter = Rc::new(NativeProviderAdmissionFromShared {
            shared: shared.clone(),
            _charge: charge,
        });
        Ok(PreparedProviderAllocationInstall {
            shared,
            allocation,
            adapter: Some(adapter),
        })
    }
    /// Caller holds prepared outside binding/provider borrows. On every Err
    /// prepared retains adapter and its cleanup lease for disposal afterwards.
    fn finish_shared_retained_admission(
        &self,
        prepared: &mut PreparedProviderAllocationInstall,
    ) -> Result<(), String> {
        let mut shared = self
            .shared_retained_admission
            .try_borrow_mut()
            .map_err(|_| "shared publication borrow conflict")?;
        let mut native = self
            .retained_admission
            .try_borrow_mut()
            .map_err(|_| "native publication borrow conflict")?;
        let mut allocation = self
            .native_retained_allocation
            .try_borrow_mut()
            .map_err(|_| "allocation publication borrow conflict")?;
        if let Some(existing) = shared.as_ref() {
            if std::sync::Arc::ptr_eq(existing, &prepared.shared)
                && native.is_some()
                && allocation
                    .as_ref()
                    .is_some_and(|old| std::sync::Arc::ptr_eq(old, &prepared.allocation))
            {
                return Ok(());
            }
            return Err("retained authority publication replacement".into());
        }
        if native.is_some() || allocation.is_some() || prepared.adapter.is_none() {
            return Err("partial retained authority publication".into());
        }
        *native = prepared.adapter.take();
        *shared = Some(prepared.shared.clone());
        *allocation = Some(prepared.allocation.clone());
        Ok(())
    }
}
impl RawV8Host {
    fn install_native_provider_allocation(
        &self,
        realm: RealmId,
        provider: &LiveLayoutProvider,
        shared: SharedRetainedAdmission,
        allocation: std::sync::Arc<oryn_js::v8_bindings::BindingRetainedAllocation>,
    ) -> Result<(), HostError> {
        let state = self.retained_binding_state(realm)?;
        let mut prepared = provider
            .prepare_shared_retained_admission(shared, allocation.clone())
            .map_err(HostError::Execution)?;
        state
            .install_retained_allocation_with(allocation, || {
                provider.finish_shared_retained_admission(&mut prepared)
            })
            .map_err(|error| HostError::Execution(error.to_string()))
    }
}
