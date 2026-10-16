//! Database-name snapshots whose physical String lives with its exact lease.
use super::*;
pub(super) fn text(scope:&mut v8::PinScope,state:&V8BindingState,value:&str)
    ->Result<crate::idb_owned_admission::Text,V8BindingError> {
    let heap=scope.get_heap_statistics();
    let used=heap.used_heap_size().checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB database name heap"))?;
    owned_names::Names::copy_text(value,&state.idb_payload_ledger,heap.heap_size_limit(),used,
        &mut ||scope.is_execution_terminating()||state.execution_retired())
        .map_err(|_|if scope.is_execution_terminating()||state.execution_retired(){V8BindingError::JavaScriptException}else{V8BindingError::Allocation("IDB database name")})
}
pub(super) fn snapshot(scope:&mut v8::PinScope,state:&V8BindingState,database_id:u64)
    ->Result<crate::idb_owned_admission::Text,V8BindingError> {
    let record=platform_object(state,database_id)?;
    let PlatformObjectRecord::IdbDatabase(database)=&*record else{return Err(V8BindingError::Allocation("IDB database name identity"))};
    text(scope,state,&database.name)
}
