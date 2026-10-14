impl ParserResourceScheduler {
    #[cfg(unix)]
    fn scan_native_pending_imports(&mut self, host: &mut RawV8Host) -> Result<(), HostError> {
        let mut retry = self.native_pending_import_retry.take();
        if retry.is_some() && let Err(error) = self.start_native_pending_import(&mut retry, host) {
            self.native_pending_import_retry = retry;
            return Err(error);
        }
        // One owning provider at a time. No registry/graph borrow survives a
        // broker request, allocation receipt, or native scheduler callback.
        for realm in host.realm_ids() {
            let Some(provider) = host.native_layout_provider_in_realm(realm) else {
                continue;
            };
            let Some(backend) = provider
                .native_backend()
                .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
            else {
                continue;
            };
            let Some(shared) = host.native_retained_admission_for_realm(realm)? else {
                continue;
            };
            let receipt = backend
                .native_retained_allocation
                .try_borrow()
                .map_err(|_| HostError::Execution("pending scan receipt borrow conflict".into()))?
                .as_ref()
                .cloned()
                .ok_or_else(|| HostError::Execution("pending scan receipt missing".into()))?;
            let _scope = receipt.scope_guard();
            sweep_retired_stylesheet_records(&backend.retained_stylesheets, shared.as_ref())?;
            let Some(snapshot) = RetainedPublicationSnapshot::capture(
                &backend.retained_stylesheets,
                shared.as_ref(),
            )?
            else {
                continue;
            };
            let mut remaining = MAX_SCRIPT_COMPLETIONS_PER_SLICE;
            for entry in &snapshot.entries {
                if remaining == 0 {
                    break;
                }
                if entry.record.owner_publication_retired()? {
                    continue;
                }
                while remaining != 0 {
                    // Starting the preceding resource job ended its receipt
                    // scope. Refresh before the next controlled extraction.
                    host.refresh_native_retained_allocation(realm, &receipt)?;
                    let Some(request) = entry.record.take_pending_import()? else {
                        break;
                    };
                    remaining -= 1;
                    let mut request_slot = Some(request);
                    if let Err(error) = self.start_native_pending_import(&mut request_slot, host) {
                        // Exact native occurrence remains Pending; return it to
                        // its publication queue after the transport refusal.
                        if let Err(refresh_error) =
                            host.refresh_native_retained_allocation(realm, &receipt)
                        {
                            self.native_pending_import_retry = request_slot;
                            return Err(refresh_error);
                        }
                        if let Err(requeue_error) = request_slot.as_ref().expect("failed dispatch retains request").return_to_pending() {
                            self.native_pending_import_retry = request_slot;
                            return Err(requeue_error);
                        }
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }
}
