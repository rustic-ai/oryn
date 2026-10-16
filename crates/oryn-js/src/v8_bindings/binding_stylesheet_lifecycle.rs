/// Native weak owner capability, not a URL or bare arena NodeId identity.
/// Public trait lets the native graph retain proof without retaining a Window.
pub trait BindingStylesheetLifecycle {
    fn current(&self, owner: BindingStylesheetDocumentOwner) -> bool;
}

struct WeakBindingStylesheetLifecycle {
    state: std::rc::Weak<V8BindingState>,
    _allocation: Option<std::sync::Arc<dyn Send + Sync>>,
}
impl BindingStylesheetLifecycle for WeakBindingStylesheetLifecycle {
    fn current(&self, owner: BindingStylesheetDocumentOwner) -> bool {
        let Some(state) = self.state.upgrade() else { return false };
        !state.execution_retired()
            && state.realm == owner.realm
            && state.document_root() == owner.document
            && state.document_generation() == owner.document_generation
            && state.document_stream_epoch() == owner.stream_epoch
    }
}
impl V8BindingState {
    pub fn stylesheet_lifecycle(&self) -> Rc<dyn BindingStylesheetLifecycle> {
        Rc::new(WeakBindingStylesheetLifecycle { state: self.stylesheet_execution_owner.borrow().clone(), _allocation: None })
    }
    fn stylesheet_lifecycle_admitted(&self, allocation: &BindingRetainedAllocation) -> Result<Rc<dyn BindingStylesheetLifecycle>, String> {
        let bytes = std::mem::size_of::<WeakBindingStylesheetLifecycle>()
            .checked_add(2 * std::mem::size_of::<usize>()).ok_or("inline lifecycle size overflow")?;
        let charge = allocation.reserve(bytes)?;
        Ok(Rc::new(WeakBindingStylesheetLifecycle { state: self.stylesheet_execution_owner.borrow().clone(), _allocation: Some(charge) }))
    }
}
