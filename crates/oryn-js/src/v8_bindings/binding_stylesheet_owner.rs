#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BindingStylesheetDocumentOwner {
    pub realm: crate::realm::RealmId,
    pub document: NodeId,
    pub document_generation: u64,
    pub stream_epoch: u64,
}
