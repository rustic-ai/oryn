/// Native weak owner capability, not a URL or bare arena NodeId identity.
/// Public trait lets the native graph retain proof without retaining a Window.
pub trait BindingStylesheetLifecycle {
    fn current(&self, owner: BindingStylesheetDocumentOwner) -> bool;
}

struct WeakBindingStylesheetLifecycle {
    state: std::rc::Weak<V8BindingState>,
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
    pub fn stylesheet_lifecycle(self: &Rc<Self>) -> Rc<dyn BindingStylesheetLifecycle> {
        Rc::new(WeakBindingStylesheetLifecycle { state: Rc::downgrade(self) })
    }
}
