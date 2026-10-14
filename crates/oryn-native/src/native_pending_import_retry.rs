impl NativePendingImportRequest {
    /// Err returns the exact owned request/token to the scheduler. No broker
    /// admission failure may silently drop the only completion capability.
    fn return_to_pending(&self) -> Result<(), HostError> {
        (|| -> Result<(), HostError> {
            if !self.current()? {
                return Ok(());
            }
            let count = self
                .publication
                .pending_imports
                .try_borrow()
                .map_err(|_| HostError::Execution("pending retry count borrow conflict".into()))?
                .len();
            let capacity = count
                .checked_add(1)
                .ok_or_else(|| HostError::Execution("pending retry capacity overflow".into()))?;
            let bytes = capacity
                .checked_mul(std::mem::size_of::<
                    oryn_layout::cssom::native_import_graph::OccurrenceId,
                >())
                .ok_or_else(|| HostError::Execution("pending retry allocation overflow".into()))?;
            let allocation = self
                .publication
                .admission
                .reserve(bytes)
                .map_err(HostError::Execution)?;
            let mut replacement = Vec::new();
            replacement
                .try_reserve_exact(capacity)
                .map_err(|e| HostError::Execution(e.to_string()))?;
            let charge_count = self
                .publication
                .responses
                .try_borrow()
                .map_err(|_| HostError::Execution("pending retry charge count conflict".into()))?
                .allocations
                .len();
            let mut charges = Vec::new();
            let charge_capacity = charge_count
                .checked_add(1)
                .ok_or_else(|| HostError::Execution("pending retry charge overflow".into()))?;
            let charge_bytes = charge_capacity
                .checked_mul(std::mem::size_of::<std::sync::Arc<dyn Send + Sync>>())
                .ok_or_else(|| {
                    HostError::Execution("pending retry charge bytes overflow".into())
                })?;
            let charge_lease = self
                .publication
                .admission
                .reserve(charge_bytes)
                .map_err(HostError::Execution)?;
            charges
                .try_reserve_exact(charge_capacity)
                .map_err(|e| HostError::Execution(e.to_string()))?;
            let allocation: std::sync::Arc<dyn Send + Sync> =
                std::sync::Arc::new((allocation, charge_lease));
            let mut queue = self
                .publication
                .pending_imports
                .try_borrow_mut()
                .map_err(|_| HostError::Execution("pending retry queue borrow conflict".into()))?;
            if queue.len() != count {
                return Err(HostError::Execution("pending retry queue changed".into()));
            }
            if queue.contains(&self.occurrence) {
                return Ok(());
            }
            let mut store =
                self.publication.responses.try_borrow_mut().map_err(|_| {
                    HostError::Execution("pending retry charge commit conflict".into())
                })?;
            if store.allocations.len() != charge_count {
                return Err(HostError::Execution("pending retry charges changed".into()));
            }
            replacement.append(&mut queue);
            replacement.push(self.occurrence);
            std::mem::swap(&mut *queue, &mut replacement);
            charges.append(&mut store.allocations);
            charges.push(allocation);
            std::mem::swap(&mut store.allocations, &mut charges);
            drop(store);
            drop(queue);
            // This queue's allocation owner must remain on the record. Use the
            // existing live response allocation store, pre-reserved separately.
            Ok(())
        })()
    }
}
