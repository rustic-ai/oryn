struct RetainedResponseStore {
    entries: Vec<RetainedStylesheetResponse>,
    allocations: Vec<std::sync::Arc<dyn Send + Sync>>,
}
impl RetainedResponseStore {
    fn new(entries: Vec<RetainedStylesheetResponse>) -> Self { Self { entries, allocations: Vec::new() } }
}
