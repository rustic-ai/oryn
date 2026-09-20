//! Callback-only DOM event custody shared by retained native algorithms.
//! Activation algorithms own their preactivation and default phases separately.
use super::native_platform::publication;
use super::*;

pub(super) struct Event {
    owner: Rc<V8BindingState>,
    event: v8::Global<v8::Object>,
    dispatch: publication::Notification,
    result: Option<bool>,
}
impl Event {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        name: &str,
        event: v8::Local<v8::Object>,
    ) -> Result<Self, V8BindingError> {
        let wrapper = wrap_node(scope, state, target)?;
        Ok(Self {
            owner: state.clone(),
            event: v8::Global::new(scope, event),
            dispatch: publication::pending_dom_dispatch(scope, state, wrapper, name, event),
            result: None,
        })
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !Rc::ptr_eq(state, &self.owner) {
            return Err(V8BindingError::Allocation("retained event owner"));
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        if !self.dispatch.resume(scope, state)? {
            return Ok(None);
        }
        let event = v8::Local::new(scope, &self.event);
        let id = event_id(scope, event)?;
        let accepted = !state
            .events
            .borrow()
            .get(&id)
            .ok_or(V8BindingError::Allocation("retained event record"))?
            .default_prevented;
        self.dispatch.finish(scope, state);
        self.result = Some(accepted);
        Ok(Some(accepted))
    }
    pub(super) fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    pub(super) fn cancel(&self, _state: &V8BindingState) {
        if self.result.is_none() {
            self.dispatch.cancel(&self.owner)
        }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        self.cancel(&self.owner)
    }
}
