//! Snapshot IDB name lists before allocating V8 wrappers or running collection.
use super::*;

pub(super) fn snapshot(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    interface: &str,
    member: &str,
) -> Result<Option<crate::idb_owned_admission::Scope>, V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB name getter heap"))?;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("IDB name getter borrow"))?;
    let Some(record) = objects.get(&id) else {
        return Err(V8BindingError::Allocation("IDB name getter identity"));
    };
    let ledger = &state.idb_payload_ledger;
    let mut names = match (record, interface, member) {
        (PlatformObjectRecord::IdbDatabase(db), "IDBDatabase", "objectStoreNames") => {
            let mut names = crate::idb_owned_admission::Scope::allocate(
                ledger,
                db.object_stores.as_slice().len(),
                limit,
                used,
            )
            .map_err(|_| V8BindingError::Allocation("IDB database names"))?;
            for value in db.object_stores.as_slice() {
                names
                    .push(
                        owned_names::Names::copy_text(value, ledger, limit, used, &mut || {
                            scope.is_execution_terminating() || state.execution_retired()
                        })
                        .map_err(|_| {
                            if scope.is_execution_terminating() || state.execution_retired() {
                                V8BindingError::JavaScriptException
                            } else {
                                V8BindingError::Allocation("IDB database name")
                            }
                        })?,
                    )
                    .map_err(|_| V8BindingError::Allocation("IDB database name capacity"))?;
            }
            names
        }
        (PlatformObjectRecord::IdbTransaction(tx), "IDBTransaction", "objectStoreNames") => {
            let mut names = crate::idb_owned_admission::Scope::allocate(
                ledger,
                tx.object_stores.len(),
                limit,
                used,
            )
            .map_err(|_| V8BindingError::Allocation("IDB transaction names"))?;
            for value in &tx.object_stores {
                let text = owned_names::Names::copy_text(value, ledger, limit, used, &mut || {
                    scope.is_execution_terminating() || state.execution_retired()
                })
                .map_err(|_| {
                    if scope.is_execution_terminating() || state.execution_retired() {
                        V8BindingError::JavaScriptException
                    } else {
                        V8BindingError::Allocation("IDB transaction name")
                    }
                })?;
                names
                    .push(text)
                    .map_err(|_| V8BindingError::Allocation("IDB transaction name capacity"))?;
            }
            names
        }
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "indexNames") => {
            let mut names = crate::idb_owned_admission::Scope::allocate(
                ledger,
                store.indexes.entries().len(),
                limit,
                used,
            )
            .map_err(|_| V8BindingError::Allocation("IDB index names"))?;
            for index in store.indexes.entries() {
                let text =
                    owned_names::Names::copy_text(index.name(), ledger, limit, used, &mut || {
                        scope.is_execution_terminating() || state.execution_retired()
                    })
                    .map_err(|_| {
                        if scope.is_execution_terminating() || state.execution_retired() {
                            V8BindingError::JavaScriptException
                        } else {
                            V8BindingError::Allocation("IDB index name")
                        }
                    })?;
                names
                    .push(text)
                    .map_err(|_| V8BindingError::Allocation("IDB index name capacity"))?;
            }
            names
        }
        _ => return Ok(None),
    };
    drop(objects);
    // Scope's allocation-free comparator preserves DOMString UTF16 ordering.
    names = names
        .normalize_cancelled(|| scope.is_execution_terminating() || state.execution_retired())
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB name ordering")
            }
        })?;
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(Some(names))
}
