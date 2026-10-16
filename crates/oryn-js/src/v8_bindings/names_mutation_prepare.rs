//! Staged admitted name-list plans, prepared before native schema publication.
use super::*;

pub(super) struct NameMutation {
    pub(super) database: owned_names::Names,
    pub(super) transaction: crate::idb_owned_admission::Scope,
}
pub(super) fn inserted(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    database_id: u64,
    transaction_id: u64,
    name: &str,
) -> Result<NameMutation, V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("name mutation heap"))?;
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("name mutation borrow"))?;
    let Some(PlatformObjectRecord::IdbDatabase(db)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("name mutation database"));
    };
    let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get(&transaction_id) else {
        return Err(V8BindingError::Allocation("name mutation transaction"));
    };
    let database = db
        .object_stores
        .inserted(name, ledger, limit, used, || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("database names")
            }
        })?;
    let mut transaction = crate::idb_owned_admission::Scope::allocate(
        ledger,
        tx.object_stores
            .len()
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("transaction names overflow"))?,
        limit,
        used,
    )
    .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    for value in tx
        .object_stores
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(name))
    {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        transaction
            .push(
                owned_names::Names::copy_text(value, ledger, limit, used, &mut || {
                    scope.is_execution_terminating() || state.execution_retired()
                })
                .map_err(|_| {
                    if scope.is_execution_terminating() || state.execution_retired() {
                        V8BindingError::JavaScriptException
                    } else {
                        V8BindingError::Allocation("transaction name")
                    }
                })?,
            )
            .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(NameMutation {
        database,
        transaction,
    })
}
pub(super) fn removed(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    database_id: u64,
    transaction_id: u64,
    name: &str,
) -> Result<NameMutation, V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("name mutation heap"))?;
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("name mutation borrow"))?;
    let Some(PlatformObjectRecord::IdbDatabase(db)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("name mutation database"));
    };
    let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get(&transaction_id) else {
        return Err(V8BindingError::Allocation("name mutation transaction"));
    };
    let database = db
        .object_stores
        .removed(name, ledger, limit, used, || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("database names")
            }
        })?;
    let mut count = 0usize;
    for value in &tx.object_stores {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        if value.as_str() != name {
            count += 1;
        }
    }
    let mut transaction = crate::idb_owned_admission::Scope::allocate(ledger, count, limit, used)
        .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    for value in tx
        .object_stores
        .iter()
        .filter(|value| value.as_str() != name)
    {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        transaction
            .push(
                owned_names::Names::copy_text(value, ledger, limit, used, &mut || {
                    scope.is_execution_terminating() || state.execution_retired()
                })
                .map_err(|_| {
                    if scope.is_execution_terminating() || state.execution_retired() {
                        V8BindingError::JavaScriptException
                    } else {
                        V8BindingError::Allocation("transaction name")
                    }
                })?,
            )
            .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(NameMutation {
        database,
        transaction,
    })
}
pub(super) fn renamed(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    database_id: u64,
    transaction_id: u64,
    old: &str,
    new: &str,
) -> Result<NameMutation, V8BindingError> {
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("name mutation heap"))?;
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("name mutation borrow"))?;
    let Some(PlatformObjectRecord::IdbDatabase(db)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("name mutation database"));
    };
    let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get(&transaction_id) else {
        return Err(V8BindingError::Allocation("name mutation transaction"));
    };
    let database = db
        .object_stores
        .renamed(old, new, ledger, limit, used, || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("database names")
            }
        })?;
    let mut transaction =
        crate::idb_owned_admission::Scope::allocate(ledger, tx.object_stores.len(), limit, used)
            .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    for value in &tx.object_stores {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        transaction
            .push(
                owned_names::Names::copy_text(
                    if value == old { new } else { value },
                    ledger,
                    limit,
                    used,
                    &mut || scope.is_execution_terminating() || state.execution_retired(),
                )
                .map_err(|_| {
                    if scope.is_execution_terminating() || state.execution_retired() {
                        V8BindingError::JavaScriptException
                    } else {
                        V8BindingError::Allocation("transaction name")
                    }
                })?,
            )
            .map_err(|_| V8BindingError::Allocation("transaction names"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(NameMutation {
        database,
        transaction,
    })
}
