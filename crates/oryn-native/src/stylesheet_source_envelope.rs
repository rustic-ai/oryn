/// The source queue owns this payload across IO completion/main-realm retry.
/// Reservations cover original source retention until parsed publication; they
/// do not become a permanent live graph quota. Parsed graphs have separate
/// allocation-owned admission leases.
struct RetainedStylesheetSourceEnvelope {
    tree: Option<StylesheetSourceTree>,
    reservations: Vec<ScriptSourceReservation>,
    _allocation: Option<Arc<dyn Send + Sync>>,
}

impl RetainedStylesheetSourceEnvelope {
    fn new(tree: StylesheetSourceTree, reservations: Vec<ScriptSourceReservation>) -> Self {
        Self { tree: Some(tree), reservations, _allocation: None }
    }

    /// One-shot move, not a source/response-guard clone. The caller must stage
    /// the native graph before consuming the ready job at final publication.
    fn take_tree(&mut self) -> Result<StylesheetSourceTree, HostError> {
        self.tree.take().ok_or_else(|| HostError::Execution(
            "retained stylesheet completion already consumed".into()))
    }

}
