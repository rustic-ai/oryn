// Native parser-resource completion ownership. Included in rust_executable.rs;
// this module is not exposed through WebIDL or deserializable worker input.
use std::rc::Weak;

#[derive(Clone)]
struct StylesheetPublicationAuthority {
    document: ParserDocumentIdentity,
    arena: Weak<RefCell<oryn_dom::DomRuntime>>,
    owner: NodeId,
    source_revision: u64,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
}
impl StylesheetPublicationAuthority {
    fn current(
        &self,
        host: &RawV8Host,
        arena: &Rc<RefCell<oryn_dom::DomRuntime>>,
    ) -> Result<bool, HostError> {
        use std::sync::atomic::Ordering;
        let Some(captured) = self.arena.upgrade() else {
            return Ok(false);
        };
        if !Rc::ptr_eq(&captured, arena)
            || self.cancellation.flag.load(Ordering::Acquire)
            || !host.realm_execution_live(self.document.realm)
            || host.document_stream_epoch(self.document.realm) != Some(self.document.stream_epoch)
            || !host
                .realm_descriptor(self.document.realm)
                .is_some_and(|descriptor| {
                    descriptor.document_generation == self.document.document_generation
                })
            || host.realm_for_document(arena, self.document.document) != Some(self.document.realm)
        {
            return Ok(false);
        }
        let dom = arena.try_borrow().map_err(|_| {
            HostError::Execution("stylesheet publication ownership borrow conflict".into())
        })?;
        Ok(
            dom.node_document(self.owner).ok().flatten() == Some(self.document.document)
                && dom.arena().stylesheet_revision(self.owner).ok() == Some(self.source_revision)
                && dom.is_connected(self.owner).unwrap_or(false),
        )
    }
}
