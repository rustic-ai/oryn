//! Native OIL replacement editing; author constructors and value setters are
//! never used to perform the operation.
use super::*;
mod job;
pub use job::Replacement;

/// Synchronous compatibility entrypoint. Actor commands retain Replacement
/// themselves; a caller of this adapter must treat interruption as failure.
pub fn replace(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    value: &str,
    sensitive: bool,
) -> Result<(), V8BindingError> {
    let mut job = Replacement::new(state, target, value, sensitive)?;
    if job.poll(scope)? {
        Ok(())
    } else {
        Err(V8BindingError::Allocation("semantic replacement suspended"))
    }
}
