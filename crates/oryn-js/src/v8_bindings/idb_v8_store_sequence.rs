//! Native heap-backed conversion of the IndexedDB store-name union.
use super::*;

pub(super) fn native_scope(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    strings: v8::Local<v8::Array>,
) -> Result<crate::idb_owned_admission::Scope, V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
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
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let value = strings
            .get_index(scope, index)
            .ok_or(V8BindingError::JavaScriptException)?;
        // Array element access can run an author getter. Do not allocate a
        // scratch buffer on its retired receiver or replace its exception.
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let value = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| V8BindingError::Allocation("IDB converted scope string"))?;
        let mut scratch = crate::idb_owned_admission::Buffer::<u16>::allocate(
            ledger,
            value.length(),
            limit,
            used,
        )
        .map_err(|_| V8BindingError::Allocation("IDB scope UTF16 scratch"))?;
        for offset in 0..value.length() {
            if offset % 1024 == 0
                && (scope.is_execution_terminating() || state.execution_retired())
            {
                return Err(V8BindingError::JavaScriptException);
            }
            scratch.push(0).map_err(|_| V8BindingError::Allocation("IDB scope UTF16 scratch"))?;
        }
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        for (chunk, units) in scratch.as_mut_slice().chunks_mut(1024).enumerate() {
            if scope.is_execution_terminating() || state.execution_retired() {
                return Err(V8BindingError::JavaScriptException);
            }
            // V8 String length is u32; every offset is within that length.
            let offset = u32::try_from(chunk * 1024)
                .map_err(|_| V8BindingError::Allocation("IDB UTF16 offset"))?;
            value.write_v2(scope, offset, units, v8::WriteFlags::default());
        }
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let text = crate::idb_owned_admission::Text::from_units_cancelled(
            ledger, scratch.as_slice(), limit, used,
            || scope.is_execution_terminating() || state.execution_retired(),
        ).map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB scope text allocation")
            }
        })?;
        native
            .push(text)
            .map_err(|_| V8BindingError::Allocation("IDB scope allocation"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(native)
}

pub(super) fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    convert_union(scope, value).map(|(output, _compound)| output)
}

/// WebIDL DOMString-or-sequence selection probes @@iterator only for objects.
/// Conversion retains the original iterator/getter exception and closes on abrupt items.
pub(super) fn convert_union<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(v8::Local<'s, v8::Array>, bool), V8BindingError> {
    if scope.is_execution_terminating() {
        return Err(V8BindingError::JavaScriptException);
    }
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
            return Ok((output, true));
        }
    }
    append(scope, value)?;
    Ok((output, false))
}

#[cfg(test)]
mod native_scope_cancellation_tests {
    use super::*;
    fn retire(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
        scope.get_current_context().get_slot::<V8BindingState>().unwrap().retire_execution();
    }
    #[test]
    fn already_retired_empty_scope_rejects_before_allocation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let array = v8::Array::new(scope, 0);
        state.retire_execution();
        let before = state.idb_payload_ledger.live_bytes();
        assert!(matches!(native_scope(scope, &state, array), Err(V8BindingError::JavaScriptException)));
        assert_eq!(state.idb_payload_ledger.live_bytes(), before);
    }
    #[test]
    fn union_iterator_retirement_rejects_owned_scope_after_author_conversion() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let function = v8::Function::new(scope, retire).unwrap();
        let key = v8::String::new(scope, "retireNativeOwner").unwrap();
        context.global(scope).set(scope, key.into(), function.into()).unwrap();
        let source = v8::String::new(scope, "({[Symbol.iterator](){let first=true;return {next(){if(!first)return {done:true};first=false;retireNativeOwner();return {done:false,value:'x'.repeat(4097)}}}}})").unwrap();
        let value = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        let before = state.idb_payload_ledger.live_bytes();
        let (strings, compound) = convert_union(scope, value).unwrap();
        assert!(compound);
        assert!(state.execution_retired());
        assert_eq!(strings.length(), 1);
        assert!(matches!(native_scope(scope, &state, strings), Err(V8BindingError::JavaScriptException)));
        assert_eq!(state.idb_payload_ledger.live_bytes(), before);
    }
    #[test]
    fn author_array_getter_retirement_releases_native_scope_before_scratch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let function = v8::Function::new(scope, retire).unwrap();
        let key = v8::String::new(scope, "retireNativeOwner").unwrap();
        context.global(scope).set(scope, key.into(), function.into()).unwrap();
        let script = v8::String::new(scope, "(()=>{const a=[];a.length=1;Object.defineProperty(a,'0',{get(){retireNativeOwner();return 'x'.repeat(4097)}});return a})()").unwrap();
        let value = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        let array = v8::Local::<v8::Array>::try_from(value).unwrap();
        let before = state.idb_payload_ledger.live_bytes();
        assert!(matches!(native_scope(scope, &state, array), Err(V8BindingError::JavaScriptException)));
        assert!(state.execution_retired());
        assert_eq!(state.idb_payload_ledger.live_bytes(), before);
    }
    #[test]
    fn chunked_utf16_scope_preserves_surrogate_pair_and_empty_string() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let expected = format!("{}😀{}", "a".repeat(1023), "b".repeat(1025));
        let array = v8::Array::new(scope, 2);
        let value = v8::String::new(scope, &expected).unwrap();
        assert_eq!(array.set_index(scope, 0, value.into()), Some(true));
        let empty = v8::String::new(scope, "").unwrap();
        assert_eq!(array.set_index(scope, 1, empty.into()), Some(true));
        let before = state.idb_payload_ledger.live_bytes();
        let native = native_scope(scope, &state, array).unwrap();
        assert_eq!(native.names(), &[expected, String::new()]);
        assert!(state.idb_payload_ledger.live_bytes() > before);
        drop(native);
        assert_eq!(state.idb_payload_ledger.live_bytes(), before);
    }
}
