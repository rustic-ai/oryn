//! Native IndexedDB key-path conversion and owned payload admission.
use super::*;

pub(super) fn convert<'s>(scope:&mut v8::PinScope<'s, '_>,state:&V8BindingState,value:v8::Local<'s, v8::Value>)
    ->Result<(crate::idb_owned_admission::Text,bool,bool),V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {return Err(V8BindingError::JavaScriptException);}
    let (strings,compound)=idb_v8_store_sequence::convert_union(scope,value)?;
    let native=idb_v8_store_sequence::native_scope(scope,state,strings)?;
    let (names,custody)=native.into_parts();
    let mut valid=!compound || !names.is_empty();
    for name in &names {
        let mut component_start=true;
        for (offset,character) in name.chars().enumerate() {
            if offset%1024==0 && (scope.is_execution_terminating() || state.execution_retired()) {
                return Err(V8BindingError::JavaScriptException);
            }
            if character=='.' {
                if component_start {valid=false;}
                component_start=true;
            } else {
                let permitted=character=='_' || character=='$' || if component_start {character.is_alphabetic()} else {character.is_alphanumeric()};
                valid &= permitted;
                component_start=false;
            }
        }
        if !name.is_empty() && component_start {valid=false;}
    }
    let heap=scope.get_heap_statistics();
    let used=heap.used_heap_size().checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("keypath heap accounting"))?;
    let result=keypath_encoding::encode(&state.idb_payload_ledger,&names,compound,
        heap.heap_size_limit(),used,||scope.is_execution_terminating() || state.execution_retired());
    // Source Vec/String buffers drop before their allocation custodian.
    drop(names);drop(custody);
    result.map(|value|(value,compound,valid)).map_err(|_|{
        if scope.is_execution_terminating() || state.execution_retired() {V8BindingError::JavaScriptException}
        else {V8BindingError::Allocation("keypath encoding")}
    })
}
