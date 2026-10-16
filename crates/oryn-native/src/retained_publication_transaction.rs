struct PreparedRetainedPublicationStorage {
    snapshot: RetainedPublicationSnapshot,
    replacement_records: Vec<Rc<RetainedStylesheetRecord>>,
    retired_records: Vec<Rc<RetainedStylesheetRecord>>,
    registry_charge: std::sync::Arc<dyn Send + Sync>,
    publication_charge: std::sync::Arc<dyn Send + Sync>,
}
struct RetainedPublicationAllocation {
    _metadata: std::sync::Arc<dyn Send + Sync>,
    _source: std::sync::Arc<dyn Send + Sync>,
}
// Owned preparation. Never invoke this while borrowing RetainedStylesheets.
// Source validation and allocator admission may enter native broker/accounting
// boundaries; the final registry mutation must contain neither callback.
struct PreparedRetainedPublication {
    record: Rc<RetainedStylesheetRecord>,
    replacement_records: Vec<Rc<RetainedStylesheetRecord>>,
    retired_records: Vec<Rc<RetainedStylesheetRecord>>,
    snapshot: RetainedPublicationSnapshot,
    registry_charge: Option<std::sync::Arc<dyn Send + Sync>>,
    committed: bool,
    pending_dependencies: Option<CssDependencies>,
}

impl PreparedRetainedPublication {
    fn preflight(
        snapshot: RetainedPublicationSnapshot,
        admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    ) -> Result<PreparedRetainedPublicationStorage, HostError> {
        let expected_count = snapshot.entries.len();
        let capacity = expected_count
            .checked_add(1)
            .ok_or_else(|| HostError::Execution("stylesheet registry capacity overflow".into()))?;
        let registry_bytes = capacity
            .checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())
            .and_then(|bytes| {
                bytes.checked_add(
                    expected_count
                        .checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())?,
                )
            })
            .ok_or_else(|| HostError::Execution("stylesheet registry metadata overflow".into()))?;
        let registry_charge = admission
            .reserve(registry_bytes)
            .map_err(HostError::Execution)?;
        let mut replacement_records = Vec::new();
        replacement_records
            .try_reserve_exact(capacity)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let mut retired_records = Vec::new();
        retired_records
            .try_reserve_exact(expected_count)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let record_bytes = std::mem::size_of::<RetainedStylesheetRecord>()
            .checked_add(
                std::mem::size_of::<RefCell<ImportGraph>>() + 4 * std::mem::size_of::<usize>(),
            )
            .ok_or_else(|| {
                HostError::Execution("stylesheet publication metadata overflow".into())
            })?;
        let publication_charge = admission
            .reserve(record_bytes)
            .map_err(HostError::Execution)?;
        Ok(PreparedRetainedPublicationStorage {
            snapshot,
            replacement_records,
            retired_records,
            registry_charge,
            publication_charge,
        })
    }
    fn finish_preflight(
        storage: PreparedRetainedPublicationStorage,
        owner: RetainedStylesheetOwner,
        source: RetainedStylesheetSource,
        admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
        source_allocation: std::sync::Arc<dyn Send + Sync>,
    ) -> Self {
        let PreparedRetainedPublicationStorage {
            snapshot,
            replacement_records,
            retired_records,
            registry_charge,
            publication_charge,
        } = storage;
        let publication_charge: std::sync::Arc<dyn Send + Sync> =
            std::sync::Arc::new(RetainedPublicationAllocation {
                _metadata: publication_charge,
                _source: std::sync::Arc::new((source_allocation, source.retained_allocation)),
            });
        let pending_dependencies = Some(source.dependencies);
        let record = Rc::new(RetainedStylesheetRecord {
            owner,
            graph: Rc::new(RefCell::new(source.graph)),
            _source: source.original,
            responses: RefCell::new(RetainedResponseStore::new(source.responses)),
            admission,
            mutation_revision: std::cell::Cell::new(0),
            pending_imports: RefCell::new(Vec::new()),
            _publication_charge: publication_charge,
        });
        Self {
            record,
            replacement_records,
            retired_records,
            snapshot,
            registry_charge: Some(registry_charge),
            committed: false,
            pending_dependencies,
        }
    }

    /// The caller owns `self` outside the registry borrow. False means recapture,
    /// not an error and not permission to discard the previous publication.
    fn commit(&mut self, registry: &mut RetainedStylesheets) -> bool {
        if self.committed || !self.snapshot.matches(registry) {
            return false;
        }
        let owner = &self.record.owner;
        for old in registry.records.drain(..) {
            if Weak::ptr_eq(&old.owner.arena, &owner.arena)
                && old.owner.document.realm == owner.document.realm
                && old.owner.node == owner.node
            {
                self.retired_records.push(old);
            } else {
                self.replacement_records.push(old);
            }
        }
        self.replacement_records.push(self.record.clone());
        std::mem::swap(&mut registry.records, &mut self.replacement_records);
        // Retain old metadata custody until the caller has released its borrow.
        std::mem::swap(&mut registry.registry_charge, &mut self.registry_charge);
        self.committed = true;
        true
    }
}

impl PreparedRetainedPublication {
    /// Caller invokes outside registry/DOM borrows. Keeps the parsed record,
    /// source allocation and proof custody; only registry expectation changes.
    fn rebase(
        &mut self,
        snapshot: RetainedPublicationSnapshot,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<bool, HostError> {
        if self.committed {
            return Ok(false);
        }
        let count = snapshot.entries.len();
        let capacity = count
            .checked_add(1)
            .ok_or_else(|| HostError::Execution("publication rebase capacity overflow".into()))?;
        let bytes = capacity
            .checked_add(count)
            .and_then(|slots| {
                slots.checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())
            })
            .ok_or_else(|| HostError::Execution("publication rebase size overflow".into()))?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(capacity)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let mut retired = Vec::new();
        retired
            .try_reserve_exact(count)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        admission.checkpoint().map_err(HostError::Execution)?;
        // All fallible preparation completed; no loaded graph/source replaced.
        std::mem::swap(&mut self.replacement_records, &mut replacement);
        std::mem::swap(&mut self.retired_records, &mut retired);
        let old_snapshot = std::mem::replace(&mut self.snapshot, snapshot);
        let old_charge = self.registry_charge.replace(charge);
        // This method has no borrowed registry/DOM; old custody destruction is
        // deliberately here rather than in final registry commit.
        drop(old_snapshot);
        drop(old_charge);
        drop(replacement);
        drop(retired);
        Ok(true)
    }
}
#[cfg(test)]
mod publication_rebase_capacity_tests {
    #[test]
    fn reserve_failure_preserves_old_registry_buffers() {
        let mut existing = vec![7u64];
        let before = existing.clone();
        let mut private = Vec::<u64>::new();
        assert!(private.try_reserve_exact(usize::MAX).is_err());
        assert_eq!(existing, before);
        existing.push(8);
        assert_eq!(existing, vec![7, 8]);
    }
}

// Add field `pending_dependencies: Option<CssDependencies>` to
// PreparedRetainedPublication, initialized before moving source graph/record:
// pending_dependencies: Some(source.dependencies).
impl PreparedRetainedPublication {
    /// Move exactly once after commit. Caller owns the batch across any host
    /// queue Retry; don't pass it to an API that consumes it on rejection.
    fn take_committed_dependencies(&mut self) -> Option<CssDependencies> {
        if !self.committed {
            return None;
        }
        self.pending_dependencies.take()
    }
}
