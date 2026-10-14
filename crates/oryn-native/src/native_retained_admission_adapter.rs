/// Known allocations use a native owning-scope receipt; opaque Stylo parser
/// allocations are NOT represented as covered by a source-length estimate.
struct NativeRetainedAdmission {
    allocation: Arc<oryn_js::v8_bindings::BindingRetainedAllocation>,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
}
impl oryn_layout::cssom::native_import_graph::ImportAdmission for NativeRetainedAdmission {
    fn checkpoint(&self) -> Result<(), String> {
        if self
            .cancellation
            .flag
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("retained stylesheet owner canceled".into());
        }
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
        self.checkpoint()?;
        self.allocation.reserve(bytes)
    }
}
