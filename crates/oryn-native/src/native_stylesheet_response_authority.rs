/// Native-only response authority. Inline roots have no network receipt, and
/// fetched descendants cannot acquire clean status from a base URL.
enum NativeStylesheetResponseAuthority {
    Fetched(crate::context_service::StylesheetResponseProof),
    Inline(NativeInlineStylesheetAuthority),
    Standalone(StandaloneStylesheetAuthority),
}
impl NativeStylesheetResponseAuthority {
    fn origin_clean(&self) -> bool {
        match self {
            Self::Fetched(proof) => proof.origin_clean,
            Self::Inline(_) => true,
            Self::Standalone(authority) => authority.origin_clean,
        }
    }
    fn validate_native_owner(
        &self,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<bool, HostError> {
        match self {
            Self::Inline(authority) => authority.current(host, arena),
            Self::Standalone(authority) => authority.current(host, arena),
            Self::Fetched(_) => Ok(true), // Parent guard validation is separate.
        }
    }
}
// Issued exclusively by the captured native Loader after its existing policy
// checks. It is not a parent network receipt and cannot be sent as one.
struct StandaloneStylesheetAuthority {
    document: ParserDocumentIdentity,
    node: NodeId,
    revision: u64,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    origin_clean: bool,
}
impl StandaloneStylesheetAuthority {
    fn current(&self, host: &RawV8Host, arena: &SharedDomRuntime) -> Result<bool, HostError> {
        let document = self.document;
        if self.cancellation.flag.load(std::sync::atomic::Ordering::Acquire)
            || !host.realm_execution_live(document.realm)
            || host.realm_for_document(arena, document.document) != Some(document.realm)
            || host.document_stream_epoch(document.realm) != Some(document.stream_epoch)
            || !host.realm_descriptor(document.realm).is_some_and(|descriptor|
                descriptor.document_generation == document.document_generation)
        { return Ok(false); }
        let dom = arena.try_borrow().map_err(|_| HostError::Execution(
            "standalone stylesheet authority borrow conflict".into()))?;
        Ok(dom.node_document(self.node).ok().flatten() == Some(document.document)
            && dom.is_connected(self.node).unwrap_or(false)
            && dom.arena().stylesheet_revision(self.node).ok() == Some(self.revision)
            && dom.arena().get(self.node).is_ok_and(|node| node.stylesheet_policy_allowed))
    }
}
