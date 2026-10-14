// Add to StylesheetExpansionBudget; move only after the source tree is complete.
// The envelope then retains every acquired source reservation through the
// owning realm publication transaction. Drop on failure releases all of them.
#[cfg(unix)]
impl StylesheetExpansionBudget {
    fn finish_retained_tree(
        &mut self,
        tree: StylesheetSourceTree,
    ) -> Result<RetainedStylesheetSourceEnvelope, HostError> {
        self.check()?;
        let actual = tree.queued_source_bytes()?;
        // Metadata absent from the retained body count is added before transfer;
        // do not count original bodies twice merely to change ownership.
        if actual > self.bytes {
            self.retain(actual - self.bytes)?;
        }
        Ok(RetainedStylesheetSourceEnvelope::new(
            tree,
            std::mem::take(&mut self.held),
        ))
    }
}
