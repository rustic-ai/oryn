//! History traversal tasks retain FIFO admission through host preflight.
use super::*;
pub(super) struct Entry {
    owner: RcWeak<V8BindingState>,
    document: navigation_api::AdmissionDocument,
    delta: i64,
    claimed: Cell<bool>,
}
pub struct HistoryTraversalTask {
    entry: Rc<Entry>,
    completed: bool,
}
impl HistoryTraversalTask {
    pub fn delta(&self) -> i64 {
        self.entry.delta
    }
    pub fn is_current_for(&self, state: &V8BindingState) -> bool {
        self.entry
            .owner
            .upgrade()
            .is_some_and(|owner| std::ptr::eq(owner.as_ref(), state))
            && self.is_current()
    }
    pub fn is_current(&self) -> bool {
        self.entry
            .owner
            .upgrade()
            .is_some_and(|owner| self.entry.document.current(&owner))
    }
    pub fn is_runnable(&self) -> bool {
        self.is_current()
            && self
                .entry
                .owner
                .upgrade()
                .is_some_and(|owner| !owner.execution_suspended())
    }
    /// The receiving algorithm now owns terminal execution. Do this immediately
    /// before its first callback, after all callback-free preflight succeeds.
    pub fn acknowledge(&mut self) {
        if self.completed {
            return;
        }
        self.completed = true;
        if let Some(owner) = self.entry.owner.upgrade() {
            let mut queue = owner.pending_history_traversal.borrow_mut();
            if queue
                .front()
                .is_some_and(|entry| Rc::ptr_eq(entry, &self.entry))
            {
                queue.pop_front();
            }
        }
    }
}
impl Drop for HistoryTraversalTask {
    fn drop(&mut self) {
        if !self.completed {
            self.entry.claimed.set(false);
        }
    }
}
pub(super) fn enqueue(state: &Rc<V8BindingState>, delta: i64) -> Result<(), V8BindingError> {
    if state.execution_retired() || state.execution_suspended() {
        return Err(V8BindingError::Unsupported {
            interface: "History".into(),
            member: "SecurityError".into(),
        });
    }
    let mut queue = state.pending_history_traversal.borrow_mut();
    queue.retain(|entry| entry.document.current(state));
    if queue.len() >= 1024 {
        return Err(V8BindingError::Allocation(
            "history traversal task capacity",
        ));
    }
    queue.push_back(Rc::new(Entry {
        owner: Rc::downgrade(state),
        document: navigation_api::AdmissionDocument::capture(state),
        delta,
        claimed: Cell::new(false),
    }));
    Ok(())
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    !state.execution_retired()
        && !state.execution_suspended()
        && state
            .pending_history_traversal
            .borrow()
            .iter()
            .find(|entry| entry.document.current(state))
            .is_some_and(|entry| !entry.claimed.get())
}
pub(super) fn take(state: &V8BindingState) -> Option<HistoryTraversalTask> {
    if state.execution_suspended() {
        return None;
    }
    let mut queue = state.pending_history_traversal.borrow_mut();
    while queue
        .front()
        .is_some_and(|entry| !entry.document.current(state))
    {
        queue.pop_front();
    }
    let entry = queue.front()?.clone();
    if entry.claimed.replace(true) {
        return None;
    }
    Some(HistoryTraversalTask {
        entry,
        completed: false,
    })
}
