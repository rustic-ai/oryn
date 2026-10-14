impl NativePendingImportRequest {
    /// Broker has already classified and reported the failed child. This is a
    /// terminal native association change, not publication of an empty sheet.
    fn fail_current(
        &self,
        _host: &RawV8Host,
        _arena: &SharedDomRuntime,
    ) -> Result<bool, HostError> {
        if !self.current()? {
            return Ok(false);
        }
        let revision = self
            .publication
            .mutation_revision
            .get()
            .checked_add(1)
            .ok_or_else(|| HostError::Execution("terminal import revision exhausted".into()))?;
        let mut graph =
            self.publication.graph.try_borrow_mut().map_err(|_| {
                HostError::Execution("terminal import graph borrow conflict".into())
            })?;
        let occurrence = graph
            .occurrence(self.occurrence)
            .ok_or_else(|| HostError::Execution("terminal import occurrence absent".into()))?;
        if !occurrence.parent.same_sheet(&self.parent) {
            return Ok(false);
        }
        // cancel preserves ImportRule Arc and parent liveness; native sheet
        // association remains null and stale completions become ineligible.
        if !graph
            .cancel(self.occurrence)
            .map_err(HostError::Execution)?
        {
            return Ok(false);
        }
        self.publication.mutation_revision.set(revision);
        Ok(true)
    }
}
