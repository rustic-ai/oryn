//! Staged atomic native metadata preparation for index mutation.
use super::*;

pub(super) struct PreparedRename {
    pub(super) store: owned_index_map::IndexMap,
    pub(super) definitions: owned_store_map::StoreMap,
}
pub(super) fn rename(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    store_id: u64,
    database_id: u64,
    old: &str,
    new: &str,
) -> Result<PreparedRename, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("index rename heap"))?;
    let limit = heap.heap_size_limit();
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("index rename borrow"))?;
    let Some(PlatformObjectRecord::IdbObjectStore(store)) = objects.get(&store_id) else {
        return Err(V8BindingError::Allocation("index rename store"));
    };
    let Some(PlatformObjectRecord::IdbDatabase(database)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("index rename database"));
    };
    let text = |value: &str| {
        crate::idb_owned_admission::Text::copy_string_cancelled(
            ledger,
            value,
            limit,
            used,
            &mut || scope.is_execution_terminating() || state.execution_retired(),
        )
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index rename name")
            }
        })
    };
    let prepared_store = store
        .indexes
        .renamed(old, text(new)?, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index rename metadata")
            }
        })?;
    let definition = database
        .object_store_definitions
        .get(store.name.as_str())
        .ok_or(V8BindingError::Allocation("index rename definition"))?;
    let prepared_definition = definition
        .rename_index(old, text(new)?, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index rename definition")
            }
        })?;
    let definitions = database
        .object_store_definitions
        .replaced(prepared_definition, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index rename definitions")
            }
        })?;
    Ok(PreparedRename {
        store: prepared_store,
        definitions,
    })
}

pub(super) struct PreparedDelete {
    pub(super) store: owned_index_map::IndexMap,
    pub(super) definitions: owned_store_map::StoreMap,
}
pub(super) fn delete(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    store_id: u64,
    database_id: u64,
    name: &str,
) -> Result<PreparedDelete, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("index delete heap"))?;
    let limit = heap.heap_size_limit();
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("index delete borrow"))?;
    let Some(PlatformObjectRecord::IdbObjectStore(store)) = objects.get(&store_id) else {
        return Err(V8BindingError::Allocation("index delete store"));
    };
    let Some(PlatformObjectRecord::IdbDatabase(database)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("index delete database"));
    };
    let prepared_store = store
        .indexes
        .removed(name, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index delete metadata")
            }
        })?;
    let definition = database
        .object_store_definitions
        .get(store.name.as_str())
        .ok_or(V8BindingError::Allocation("index delete definition"))?;
    let prepared_definition = definition
        .without_index(name, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index delete definition")
            }
        })?;
    let definitions = database
        .object_store_definitions
        .replaced(prepared_definition, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("index delete definitions")
            }
        })?;
    Ok(PreparedDelete {
        store: prepared_store,
        definitions,
    })
}
