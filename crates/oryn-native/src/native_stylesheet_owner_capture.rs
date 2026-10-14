// Add inside RawV8Host's native implementation. Call before parsing/publication;
// ordinary retirement is cancellation, not an ambient realm fallback.
impl RawV8Host {
    pub(crate) fn stylesheet_owner_lifecycle(
        &self,
        owner: oryn_js::v8_bindings::BindingStylesheetDocumentOwner,
        arena: &oryn_dom::SharedDomRuntime,
    ) -> Result<Option<Rc<dyn oryn_js::v8_bindings::BindingStylesheetLifecycle>>, HostError> {
        let Some(state) = self.binding_states.get(&owner.realm) else {
            return Ok(None);
        };
        if state.execution_retired()
            || state.document_root() != owner.document
            || state.document_generation() != owner.document_generation
            || state.document_stream_epoch() != owner.stream_epoch
            || !Rc::ptr_eq(&state.dom(), arena)
        {
            return Ok(None);
        }
        Ok(Some(state.stylesheet_lifecycle()))
    }
}
