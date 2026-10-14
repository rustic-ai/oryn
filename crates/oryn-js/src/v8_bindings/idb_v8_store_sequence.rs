//! Native heap-backed conversion of the IndexedDB store-name union.
use super::*;

pub(super) fn native_scope(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    strings: v8::Local<v8::Array>,
) -> Result<crate::idb_owned_admission::Scope, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB payload heap accounting"))?;
    let limit = heap.heap_size_limit();
    let ledger = &state.idb_payload_ledger;
    let mut native =
        crate::idb_owned_admission::Scope::allocate(ledger, strings.length() as usize, limit, used)
            .map_err(|_| V8BindingError::Allocation("IDB scope allocation"))?;
    for index in 0..strings.length() {
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        let value = strings
            .get_index(scope, index)
            .ok_or(V8BindingError::JavaScriptException)?;
        let value = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| V8BindingError::Allocation("IDB converted scope string"))?;
        let mut scratch = crate::idb_owned_admission::Buffer::<u16>::allocate(
            ledger,
            value.length(),
            limit,
            used,
        )
        .map_err(|_| V8BindingError::Allocation("IDB scope UTF16 scratch"))?;
        scratch.initialize_zeroed();
        value.write_v2(scope, 0, scratch.as_mut_slice(), v8::WriteFlags::default());
        let text =
            crate::idb_owned_admission::Text::from_units(ledger, scratch.as_slice(), limit, used)
                .map_err(|_| V8BindingError::Allocation("IDB scope text allocation"))?;
        native
            .push(text)
            .map_err(|_| V8BindingError::Allocation("IDB scope allocation"))?;
    }
    Ok(native)
}

pub(super) fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let output = v8::Array::new(scope, 0);
    let mut count = 0u32;
    let mut append = |scope: &mut v8::PinScope<'s, '_>, item: v8::Local<'s, v8::Value>| {
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        let item = item
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let next = count
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("IDB sequence index overflow"))?;
        if output.set_index(scope, count, item.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        count = next;
        Ok(())
    };
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let key = v8::Symbol::get_iterator(scope);
        let method = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !method.is_null_or_undefined() {
            let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| {
                V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "TypeError".into(),
                }
            })?;
            iterable_conversion::consume(scope, object, method, &mut append)?;
            return Ok(output);
        }
    }
    append(scope, value)?;
    Ok(output)
}
