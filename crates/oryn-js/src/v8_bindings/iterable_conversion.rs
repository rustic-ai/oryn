//! Shared Web IDL sequence conversion with IteratorClose on item conversion errors.
use super::*;
type Result<T> = std::result::Result<T, V8BindingError>;
fn type_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Iterator".into(),
        member: "TypeError".into(),
    }
}
fn field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("iterator key"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
// Convert each item while the iterator is active, so conversion side effects,
// abrupt completions, and IteratorClose retain their specified ordering.
pub(super) fn consume<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    method: v8::Local<v8::Function>,
    mut convert: impl FnMut(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Value>) -> Result<()>,
) -> Result<()> {
    let iterator = method
        .call(scope, object.into(), &[])
        .ok_or(V8BindingError::JavaScriptException)?;
    let iterator = v8::Local::<v8::Object>::try_from(iterator).map_err(|_| type_error())?;
    let next = v8::Local::<v8::Function>::try_from(field(scope, iterator, "next")?)
        .map_err(|_| type_error())?;
    loop {
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        let step = next
            .call(scope, iterator.into(), &[])
            .ok_or(V8BindingError::JavaScriptException)?;
        let step = v8::Local::<v8::Object>::try_from(step).map_err(|_| type_error())?;
        if field(scope, step, "done")?.boolean_value(scope) {
            return Ok(());
        }
        let value = field(scope, step, "value")?;
        let (result, exception) = {
            v8::tc_scope!(let tc, scope);
            let result = convert(tc, value);
            if tc.has_terminated() || tc.is_execution_terminating() {
                tc.rethrow();
                return Err(V8BindingError::JavaScriptException);
            }
            let exception = tc.exception().map(|e| v8::Global::new(tc, e));
            tc.reset();
            (result, exception)
        };
        if let Err(error) = result {
            if scope.is_execution_terminating() {
                return Err(V8BindingError::JavaScriptException);
            }
            {
                v8::tc_scope!(let tc, scope);
                if let Ok(value) = field(tc, iterator, "return")
                    && let Ok(close) = v8::Local::<v8::Function>::try_from(value)
                {
                    close.call(tc, iterator.into(), &[]);
                }
                if tc.has_terminated() || tc.is_execution_terminating() {
                    tc.rethrow();
                    return Err(V8BindingError::JavaScriptException);
                }
            }
            if let Some(exception) = exception {
                let exception = v8::Local::new(scope, exception);
                scope.throw_exception(exception);
                return Err(V8BindingError::JavaScriptException);
            }
            return Err(error);
        }
    }
}
