struct RetainedPublicationSnapshotEntry {
    record: Rc<RetainedStylesheetRecord>,
    mutation_revision: u64,
}
struct RetainedPublicationSnapshot {
    entries: Vec<RetainedPublicationSnapshotEntry>,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl RetainedPublicationSnapshot {
    fn capture(
        registry: &RefCell<RetainedStylesheets>,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<Option<Self>, HostError> {
        let count = registry
            .try_borrow()
            .map_err(|_| HostError::Execution("registry snapshot count borrow conflict".into()))?
            .records
            .len();
        // Count borrow ended. Admission and both physical allocation steps
        // occur before acquiring the final registry read borrow.
        let bytes = count
            .checked_mul(std::mem::size_of::<RetainedPublicationSnapshotEntry>())
            .ok_or_else(|| HostError::Execution("registry snapshot size overflow".into()))?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(count)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let current = registry.try_borrow().map_err(|_| {
            HostError::Execution("registry snapshot capture borrow conflict".into())
        })?;
        if current.records.len() != count {
            return Ok(None);
        }
        for record in &current.records {
            entries.push(RetainedPublicationSnapshotEntry {
                record: record.clone(),
                mutation_revision: record.mutation_revision.get(),
            });
        }
        Ok(Some(Self {
            entries,
            _charge: charge,
        }))
    }
    fn matches(&self, registry: &RetainedStylesheets) -> bool {
        registry.records.len() == self.entries.len()
            && registry
                .records
                .iter()
                .zip(&self.entries)
                .all(|(actual, expected)| {
                    Rc::ptr_eq(actual, &expected.record)
                        && actual.mutation_revision.get() == expected.mutation_revision
                })
    }
}
