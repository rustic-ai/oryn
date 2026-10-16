//! Staged preparation of upgrade rollback metadata before native mutation.
use super::*;

pub(super) struct StoreRestore {
    pub(super) id: u64,
    pub(super) name: Option<crate::idb_owned_admission::Text>,
    pub(super) indexes: owned_index_map::IndexMap,
}
pub(super) struct UpgradeRestore {
    pub(super) database_name: crate::idb_owned_admission::Text,
    pub(super) definitions: owned_store_map::StoreMap,
    // Exact admitted storage for the preparation plan; no unbounded Vec.
    pub(super) stores: crate::idb_owned_admission::Buffer<StoreRestore>,
    pub(super) indexes: crate::idb_owned_admission::Buffer<IndexRestore>,
    pub(super) database_names: owned_names::Names,
    pub(super) transaction_names: crate::idb_owned_admission::Scope,
}
pub(super) struct IndexRestore {
    pub(super) id: u64,
    pub(super) name: Option<crate::idb_owned_admission::Text>,
}
pub(super) fn prepare(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    database_id: u64,
    transaction_id: u64,
) -> Result<UpgradeRestore, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB restore heap"))?;
    let limit = heap.heap_size_limit();
    let ledger = &state.idb_payload_ledger;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("IDB restore borrow"))?;
    if !matches!(objects.get(&transaction_id),Some(PlatformObjectRecord::IdbTransaction(tx)) if tx.database_id==database_id)
    {
        return Err(V8BindingError::Allocation(
            "IDB restore transaction identity",
        ));
    }
    let Some(PlatformObjectRecord::IdbDatabase(database)) = objects.get(&database_id) else {
        return Err(V8BindingError::Allocation("IDB restore database"));
    };
    let database_name =
        owned_names::Names::copy_text(&database.name, ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restore database name")
            }
        })?;
    let definitions = database
        .upgrade_base_definitions
        .copy_admitted(ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restore definitions")
            }
        })?;
    let database_names =
        owned_names::Names::from_metadata(definitions.values(), ledger, limit, used, &mut || {
            scope.is_execution_terminating() || state.execution_retired()
        })
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restore database names")
            }
        })?;
    let mut transaction_names = crate::idb_owned_admission::Scope::allocate(
        ledger,
        database_names.as_slice().len(),
        limit,
        used,
    )
    .map_err(|_| {
        if scope.is_execution_terminating() || state.execution_retired() {
            V8BindingError::JavaScriptException
        } else {
            V8BindingError::Allocation("IDB restore transaction names")
        }
    })?;
    for name in database_names.as_slice() {
        transaction_names
            .push(owned_database_name::text(scope, state, name)?)
            .map_err(|_| V8BindingError::Allocation("IDB restored transaction scope"))?;
    }
    let mut count = 0;
    for record in objects.values() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        if matches!(record,PlatformObjectRecord::IdbObjectStore(store) if store.transaction_id==transaction_id)
        {
            count += 1;
        }
    }
    let mut stores = crate::idb_owned_admission::Buffer::allocate(ledger, count, limit, used)
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restore plan")
            }
        })?;
    for (id, record) in objects.iter() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let PlatformObjectRecord::IdbObjectStore(store) = record else {
            continue;
        };
        if store.transaction_id != transaction_id {
            continue;
        }
        let base = store
            .upgrade_base_name
            .as_ref()
            .map(|name| name.as_str())
            .and_then(|name| definitions.get(name));
        let indexes = match base {
            Some(base) => owned_index_map::IndexMap::from_metadata(
                base.indexes(),
                ledger,
                limit,
                used,
                &mut || scope.is_execution_terminating() || state.execution_retired(),
            ),
            None => owned_index_map::IndexMap::empty(ledger, limit, used, &mut || {
                scope.is_execution_terminating() || state.execution_retired()
            }),
        }
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restored indexes")
            }
        })?;
        let name = base
            .map(|base| {
                crate::idb_owned_admission::Text::copy_string_cancelled(
                    ledger,
                    base.name.as_str(),
                    limit,
                    used,
                    &mut || scope.is_execution_terminating() || state.execution_retired(),
                )
            })
            .transpose()
            .map_err(|_| {
                if scope.is_execution_terminating() || state.execution_retired() {
                    V8BindingError::JavaScriptException
                } else {
                    V8BindingError::Allocation("IDB restored name")
                }
            })?;
        stores
            .push(StoreRestore {
                id: *id,
                name,
                indexes,
            })
            .map_err(|_| {
                if scope.is_execution_terminating() || state.execution_retired() {
                    V8BindingError::JavaScriptException
                } else {
                    V8BindingError::Allocation("IDB restore plan")
                }
            })?;
    }
    let mut count = 0;
    for record in objects.values() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        if matches!(record,PlatformObjectRecord::IdbIndex(index) if matches!(objects.get(&index.object_store_id),Some(PlatformObjectRecord::IdbObjectStore(store)) if store.transaction_id==transaction_id))
        {
            count += 1;
        }
    }
    let mut indexes = crate::idb_owned_admission::Buffer::allocate(ledger, count, limit, used)
        .map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB restored index plan")
            }
        })?;
    for (id, record) in objects.iter() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let PlatformObjectRecord::IdbIndex(index) = record else {
            continue;
        };
        if !matches!(objects.get(&index.object_store_id),Some(PlatformObjectRecord::IdbObjectStore(store)) if store.transaction_id==transaction_id)
        {
            continue;
        }
        let name = index
            .upgrade_base_name
            .as_ref()
            .map(|name| {
                crate::idb_owned_admission::Text::copy_string_cancelled(
                    ledger,
                    name.as_str(),
                    limit,
                    used,
                    &mut || scope.is_execution_terminating() || state.execution_retired(),
                )
            })
            .transpose()
            .map_err(|_| {
                if scope.is_execution_terminating() || state.execution_retired() {
                    V8BindingError::JavaScriptException
                } else {
                    V8BindingError::Allocation("IDB restored index name")
                }
            })?;
        indexes.push(IndexRestore { id: *id, name }).map_err(|_| {
            if scope.is_execution_terminating() || state.execution_retired() {
                V8BindingError::JavaScriptException
            } else {
                V8BindingError::Allocation("IDB index restore plan")
            }
        })?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(UpgradeRestore {
        database_name,
        definitions,
        stores,
        indexes,
        database_names,
        transaction_names,
    })
}
