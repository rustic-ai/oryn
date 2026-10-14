// Inline stylesheet authority is native DOM/CSP state, never a fetched URL.
// Imported descendants still require parent-issued response proof custody.
struct NativeInlineStylesheetAuthority {
    document: ParserDocumentIdentity,
    node: NodeId,
    source_revision: u64,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
}
impl NativeInlineStylesheetAuthority {
    fn current(&self, host: &RawV8Host, arena: &SharedDomRuntime) -> Result<bool, HostError> {
        if self
            .cancellation
            .flag
            .load(std::sync::atomic::Ordering::Acquire)
            || host.realm_for_document(arena, self.document.document) != Some(self.document.realm)
            || !host.realm_execution_live(self.document.realm)
            || host.document_stream_epoch(self.document.realm) != Some(self.document.stream_epoch)
            || !host
                .realm_descriptor(self.document.realm)
                .is_some_and(|descriptor| {
                    descriptor.document_generation == self.document.document_generation
                })
        {
            return Ok(false);
        }
        let dom = arena
            .try_borrow()
            .map_err(|_| HostError::Execution("inline stylesheet policy borrow conflict".into()))?;
        let node = dom
            .arena()
            .get(self.node)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        // No URI, response redirect or origin-clean heuristic participates.
        // This is exclusively an authored HTML style owner with the existing
        // native inline-policy authorization captured for this exact revision.
        Ok(matches!(&node.kind, oryn_dom::NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "style")
            && node.stylesheet_policy_allowed
            && dom.node_document(self.node).ok().flatten() == Some(self.document.document)
            && dom.is_connected(self.node).unwrap_or(false)
            && dom.arena().stylesheet_revision(self.node).ok() == Some(self.source_revision))
    }
}
