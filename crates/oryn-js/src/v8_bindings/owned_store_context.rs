//! Admitted snapshots for metadata copied out of live database/store records.
use super::*;

/// Physical wire strings must be moved from Text and their leases kept until
/// submit_idb has prepared its independent immutable command owner.
pub(super) struct Custody {
    _database: crate::idb_owned_admission::Lease,
    _store: crate::idb_owned_admission::Lease,
}
pub(super) fn snapshot(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    store_id: u64,
) -> Result<(String,String,u64,Custody),V8BindingError> {
    let heap=scope.get_heap_statistics();
    let limit=heap.heap_size_limit();
    let used=heap.used_heap_size().checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB context metadata heap"))?;
    let objects=state.platform_objects.borrow();
    let Some(PlatformObjectRecord::IdbObjectStore(store))=objects.get(&store_id) else {return Err(V8BindingError::Allocation("IDBObjectStore record"))};
    if store.deleted {return Err(V8BindingError::Unsupported {interface:"IDBObjectStore".into(),member:"InvalidStateError".into()})}
    let transaction_id=store.transaction_id;
    let Some(PlatformObjectRecord::IdbTransaction(transaction))=objects.get(&transaction_id) else {return Err(V8BindingError::Allocation("IDBTransaction record"))};
    if transaction.aborted || !transaction.active {return Err(V8BindingError::Unsupported {interface:"IDBTransaction".into(),member:"TransactionInactiveError".into()})}
    let Some(PlatformObjectRecord::IdbDatabase(database))=objects.get(&transaction.database_id) else {return Err(V8BindingError::Allocation("IDBDatabase record"))};
    let copy=|value:&str| owned_names::Names::copy_text(value,&state.idb_payload_ledger,limit,used,&mut ||scope.is_execution_terminating()||state.execution_retired())
        .map_err(|_| if scope.is_execution_terminating()||state.execution_retired() {V8BindingError::JavaScriptException} else {V8BindingError::Allocation("IDB context metadata")});
    let database=copy(database.name.as_str())?;
    let store=copy(store.name.as_str())?;
    drop(objects);
    let (database,_database)=database.into_parts();
    let (store,_store)=store.into_parts();
    Ok((database,store,transaction_id,Custody{_database,_store}))
}
