//! Trusted native delivery into a WorkerGlobalScope.
use super::*;
pub fn message(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: crate::message_wire::WireValue,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired worker"));
    }
    let source = v8::null(scope);
    window_messages::deliver_external(scope, state, value, source.into(), "")
}
pub fn connect(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    token: crate::message_wire::PortToken,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired worker"));
    }
    window_messages::connect(scope, state, token)
}
