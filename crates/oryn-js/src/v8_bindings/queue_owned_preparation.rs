//! Staged two-phase owned queue admission: all fallible reserve precedes schema publication.
use super::*;

pub(super) fn prepare_schema_admission(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    database_id: u64,
    transaction_id: u64,
    command: IdbHostCommand,
) -> Result<crate::idb_owned_admission::Admission, V8BindingError> {
    let upgrade = {
        let objects = state
            .platform_objects
            .try_borrow()
            .map_err(|_| V8BindingError::Allocation("IDB schema metadata borrow"))?;
        let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get(&transaction_id) else {
            return Err(V8BindingError::Allocation("IDB schema transaction"));
        };
        if tx.database_id != database_id
            || tx.mode != "versionchange"
            || !tx.active
            || tx.finished
            || tx.aborted
        {
            return Err(V8BindingError::Allocation("IDB schema transaction state"));
        }
        let Some(PlatformObjectRecord::IdbDatabase(db)) = objects.get(&database_id) else {
            return Err(V8BindingError::Allocation("IDB schema database"));
        };
        if db.upgrade_transaction_id != Some(transaction_id) {
            return Err(V8BindingError::Allocation("IDB schema upgrade identity"));
        }
        db.connection
            .as_ref()
            .map(|token| {
                Ok::<_, V8BindingError>((
                    owned_database_name::text(scope, state, token)?,
                    owned_database_name::text(scope, state, &db.name)?,
                ))
            })
            .transpose()?
    };
    let mut leases = None;
    let command = if let Some((token, name)) = upgrade {
        let (token, t) = token.into_parts();
        let (name, n) = name.into_parts();
        leases = Some((t, n));
        IdbHostCommand::UpgradeConnection {
            token,
            name,
            inner: Box::new(command),
        }
    } else {
        command
    };
    let heap = scope.get_heap_statistics();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB schema heap"))?;
    let result = crate::idb_owned_admission::Admission::metadata(
        &command,
        &state.idb_payload_ledger,
        heap.heap_size_limit(),
        used,
        || scope.is_execution_terminating() || state.execution_retired(),
    )
    .map_err(|_| {
        if scope.is_execution_terminating() || state.execution_retired() {
            V8BindingError::JavaScriptException
        } else {
            V8BindingError::Allocation("IDB schema admission")
        }
    });
    drop(command);
    drop(leases);
    result
}

pub(super) struct SchemaMutation {
    pub(super) database_id: u64,
    pub(super) definitions: owned_store_map::StoreMap,
    pub(super) names: Option<names_mutation_prepare::NameMutation>,
    pub(super) store: Option<StoreMutation>,
    pub(super) index: Option<IndexMutation>,
    pub(super) extra_stores: Option<crate::idb_owned_admission::Buffer<StoreMutation>>,
    pub(super) extra_indexes: Option<crate::idb_owned_admission::Buffer<IndexMutation>>,
}
pub(super) enum StoreMutation {
    Created(u64),
    Updated {
        id: u64,
        name: Option<crate::idb_owned_admission::Text>,
        indexes: Option<owned_index_map::IndexMap>,
        deleted: bool,
    },
}
pub(super) struct IndexMutation {
    pub(super) id: u64,
    pub(super) name: Option<crate::idb_owned_admission::Text>,
    pub(super) deleted: bool,
}

pub(super) fn prepare_store_aliases(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    transaction_id: u64,
    old: &str,
    new: Option<&str>,
) -> Result<crate::idb_owned_admission::Buffer<StoreMutation>, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB aliases heap"))?;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("IDB aliases borrow"))?;
    let mut count = 0usize;
    for record in objects.values() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        if matches!(record,PlatformObjectRecord::IdbObjectStore(s) if s.transaction_id==transaction_id && s.name.as_str()==old)
        {
            count = count
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("IDB aliases count"))?;
        }
    }
    let mut changes =
        crate::idb_owned_admission::Buffer::allocate(&state.idb_payload_ledger, count, limit, used)
            .map_err(|_| V8BindingError::Allocation("IDB aliases plan"))?;
    for (id, record) in objects.iter() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let PlatformObjectRecord::IdbObjectStore(store) = record else {
            continue;
        };
        if store.transaction_id != transaction_id || store.name.as_str() != old {
            continue;
        }
        let name = new
            .map(|name| owned_database_name::text(scope, state, name))
            .transpose()?;
        let indexes = if new.is_none() {
            Some(
                owned_index_map::IndexMap::empty(
                    &state.idb_payload_ledger,
                    limit,
                    used,
                    &mut || scope.is_execution_terminating() || state.execution_retired(),
                )
                .map_err(|_| {
                    if scope.is_execution_terminating() || state.execution_retired() {
                        V8BindingError::JavaScriptException
                    } else {
                        V8BindingError::Allocation("IDB deleted aliases")
                    }
                })?,
            )
        } else {
            None
        };
        changes
            .push(StoreMutation::Updated {
                id: *id,
                name,
                indexes,
                deleted: new.is_none(),
            })
            .map_err(|_| V8BindingError::Allocation("IDB aliases plan"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(changes)
}
pub(super) fn prepare_index_aliases(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    store_id: u64,
    old: &str,
    new: Option<&str>,
) -> Result<crate::idb_owned_admission::Buffer<IndexMutation>, V8BindingError> {
    let heap = scope.get_heap_statistics();
    let limit = heap.heap_size_limit();
    let used = heap
        .used_heap_size()
        .checked_add(heap.external_memory())
        .ok_or(V8BindingError::Allocation("IDB aliases heap"))?;
    let objects = state
        .platform_objects
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("IDB aliases borrow"))?;
    let mut count = 0usize;
    for record in objects.values() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        if matches!(record,PlatformObjectRecord::IdbIndex(i) if i.object_store_id==store_id && i.name.as_str()==old)
        {
            count = count
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("IDB aliases count"))?;
        }
    }
    let mut changes =
        crate::idb_owned_admission::Buffer::allocate(&state.idb_payload_ledger, count, limit, used)
            .map_err(|_| V8BindingError::Allocation("IDB aliases plan"))?;
    for (id, record) in objects.iter() {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let PlatformObjectRecord::IdbIndex(index) = record else {
            continue;
        };
        if index.object_store_id != store_id || index.name.as_str() != old {
            continue;
        }
        let name = new
            .map(|name| owned_database_name::text(scope, state, name))
            .transpose()?;
        changes
            .push(IndexMutation {
                id: *id,
                name,
                deleted: new.is_none(),
            })
            .map_err(|_| V8BindingError::Allocation("IDB aliases plan"))?;
    }
    if scope.is_execution_terminating() || state.execution_retired() {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(changes)
}

pub(super) struct PreparedQueue {
    request_id: u64,
    transaction_id: Option<u64>,
    operation: OwnedIdbOperation,
}
pub(super) fn prepare(
    state: &V8BindingState,
    request_id: u64,
    admission: crate::idb_owned_admission::Admission,
) -> Result<PreparedQueue, V8BindingError> {
    if state
        .queued_owned_idb
        .try_borrow()
        .map_err(|_| V8BindingError::Allocation("IDB owned queue borrow"))?
        .len()
        >= 16_384
    {
        return Err(V8BindingError::Allocation("IDB owned queue limit"));
    }
    state
        .queued_owned_idb
        .try_borrow_mut()
        .map_err(|_| V8BindingError::Allocation("IDB owned queue borrow"))?
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("IDB owned queue"))?;
    let transaction_id = {
        let objects = state
            .platform_objects
            .try_borrow()
            .map_err(|_| V8BindingError::Allocation("IDB request borrow"))?;
        match objects.get(&request_id) {
            Some(PlatformObjectRecord::IdbRequest(request)) => request.transaction_id,
            _ => return Err(V8BindingError::Allocation("IDB owned request identity")),
        }
    };
    if let Some(id) = transaction_id {
        let mut objects = state
            .platform_objects
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("IDB transaction borrow"))?;
        let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get_mut(&id) else {
            return Err(V8BindingError::Allocation("IDB transaction"));
        };
        tx.pending_requests
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("IDB pending count"))?;
        tx.request_queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("IDB request queue"))?;
    }
    // Arc::new remains existing queue-owner allocation policy. This receipt
    // does not assert Arc control-block fallibility/heap accounting coverage.
    let operation = OwnedIdbOperation {
        request_id,
        action: owned_action::OwnedAction::unit(),
        admission: std::sync::Arc::new(admission),
    };
    Ok(PreparedQueue {
        request_id,
        transaction_id,
        operation,
    })
}
impl PreparedQueue {
    pub(super) fn publish_mutation(
        self,
        state: &V8BindingState,
        mutation: SchemaMutation,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(), V8BindingError> {
        if cancelled() {
            return Err(V8BindingError::JavaScriptException);
        }
        let mut objects = state
            .platform_objects
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("IDB schema borrow"))?;
        let mut queue = state
            .queued_owned_idb
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("IDB schema queue borrow"))?;
        let tx_id = self
            .transaction_id
            .ok_or(V8BindingError::Allocation("IDB schema transaction"))?;
        if !matches!(
            objects.get(&mutation.database_id),
            Some(PlatformObjectRecord::IdbDatabase(db)) if db.upgrade_transaction_id==Some(tx_id)
        ) || !matches!(objects.get(&self.request_id),Some(PlatformObjectRecord::IdbRequest(r)) if r.transaction_id==Some(tx_id))
            || !matches!(objects.get(&tx_id),Some(PlatformObjectRecord::IdbTransaction(tx)) if tx.database_id==mutation.database_id && tx.mode=="versionchange" && tx.active && !tx.finished && !tx.aborted)
        {
            return Err(V8BindingError::Allocation("IDB schema identity"));
        }
        for store in mutation.store.iter().chain(
            mutation
                .extra_stores
                .as_ref()
                .into_iter()
                .flat_map(|v| v.as_slice()),
        ) {
            if cancelled() {
                return Err(V8BindingError::JavaScriptException);
            }
            let id = match store {
                StoreMutation::Created(id) | StoreMutation::Updated { id, .. } => *id,
            };
            if !matches!(objects.get(&id),Some(PlatformObjectRecord::IdbObjectStore(s)) if s.transaction_id==tx_id)
            {
                return Err(V8BindingError::Allocation("IDB schema store identity"));
            }
        }
        for index in mutation.index.iter().chain(
            mutation
                .extra_indexes
                .as_ref()
                .into_iter()
                .flat_map(|v| v.as_slice()),
        ) {
            if cancelled() {
                return Err(V8BindingError::JavaScriptException);
            }
            let Some(PlatformObjectRecord::IdbIndex(record)) = objects.get(&index.id) else {
                return Err(V8BindingError::Allocation("IDB schema index identity"));
            };
            if !matches!(objects.get(&record.object_store_id),Some(PlatformObjectRecord::IdbObjectStore(s)) if s.transaction_id==tx_id)
            {
                return Err(V8BindingError::Allocation("IDB schema index owner"));
            }
        }
        if queue.len() >= 16_384 {
            return Err(V8BindingError::Allocation("IDB owned queue limit"));
        }
        queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("IDB schema queue capacity"))?;
        let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get_mut(&tx_id) else {
            unreachable!()
        };
        let pending = tx
            .pending_requests
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("IDB schema pending overflow"))?;
        tx.request_queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("IDB schema request capacity"))?;
        // Last cancellation boundary: every subsequent operation is an
        // infallible native publication with no author callbacks. A mid-commit
        // cancellation cannot return a partially published schema.
        if cancelled() {
            return Err(V8BindingError::JavaScriptException);
        }
        let transaction_names = mutation
            .names
            .map(|names| (names.database, names.transaction.into_parts()));
        if let Some(PlatformObjectRecord::IdbDatabase(db)) = objects.get_mut(&mutation.database_id)
        {
            db.object_store_definitions = mutation.definitions;
        }
        if let Some((database_names, (names, custody))) = transaction_names {
            if let Some(PlatformObjectRecord::IdbDatabase(db)) =
                objects.get_mut(&mutation.database_id)
            {
                db.object_stores = database_names;
            }
            if let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get_mut(&tx_id) {
                tx.object_stores = names;
                tx.scope_custody = Some(custody);
            }
        }
        if let Some(StoreMutation::Updated {
            id,
            name,
            indexes,
            deleted,
        }) = mutation.store
            && let Some(PlatformObjectRecord::IdbObjectStore(store)) = objects.get_mut(&id)
        {
                if let Some(name) = name {
                    store.name = name;
                }
                if let Some(indexes) = indexes {
                    store.indexes = indexes;
                }
                store.deleted = deleted;
        }
        if let Some(index) = mutation.index
            && let Some(PlatformObjectRecord::IdbIndex(record)) = objects.get_mut(&index.id)
        {
                if let Some(name) = index.name {
                    record.name = name;
                }
                record.deleted = index.deleted;
        }
        if let Some(stores) = mutation.extra_stores {
            for store in stores.into_values() {
                if let StoreMutation::Updated {
                    id,
                    name,
                    indexes,
                    deleted,
                } = store
                    && let Some(PlatformObjectRecord::IdbObjectStore(record)) = objects.get_mut(&id)
                {
                        if let Some(name) = name {
                            record.name = name;
                        }
                        if let Some(indexes) = indexes {
                            record.indexes = indexes;
                        }
                        record.deleted = deleted;
                }
            }
        }
        if let Some(indexes) = mutation.extra_indexes {
            for index in indexes.into_values() {
                if let Some(PlatformObjectRecord::IdbIndex(record)) = objects.get_mut(&index.id) {
                    if let Some(name) = index.name {
                        record.name = name;
                    }
                    record.deleted = index.deleted;
                }
            }
        }
        if let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get_mut(&tx_id) {
            tx.pending_requests = pending;
            tx.request_queue.push_back(self.request_id);
        }
        queue.push_back(self.operation);
        Ok(())
    }
    pub(super) fn publish_schema(
        self,
        state: &V8BindingState,
        store_id: u64,
        database_id: u64,
        indexes: owned_index_map::IndexMap,
        definitions: owned_store_map::StoreMap,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(), V8BindingError> {
        self.publish_mutation(
            state,
            SchemaMutation {
                database_id,
                definitions,
                names: None,
                store: Some(StoreMutation::Updated {
                    id: store_id,
                    name: None,
                    indexes: Some(indexes),
                    deleted: false,
                }),
                index: None,
                extra_stores: None,
                extra_indexes: None,
            },
            cancelled,
        )
    }
    #[cfg(test)]
    pub(super) fn publish(
        self,
        state: &V8BindingState,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(), V8BindingError> {
        if cancelled() {
            return Err(V8BindingError::JavaScriptException);
        }
        let mut objects = state
            .platform_objects
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("IDB publication borrow"))?;
        let mut queue = state
            .queued_owned_idb
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("IDB owned queue borrow"))?;
        if queue.len() >= 16_384 {
            return Err(V8BindingError::Allocation("IDB owned queue limit"));
        }
        queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("IDB owned queue capacity"))?;
        if let Some(id) = self.transaction_id {
            if !matches!(objects.get(&self.request_id), Some(PlatformObjectRecord::IdbRequest(request)) if request.transaction_id == Some(id))
            {
                return Err(V8BindingError::Allocation(
                    "IDB publication request identity",
                ));
            }
            let Some(PlatformObjectRecord::IdbTransaction(tx)) = objects.get_mut(&id) else {
                return Err(V8BindingError::Allocation("IDB transaction"));
            };
            if !tx.active || tx.finished || tx.aborted {
                return Err(V8BindingError::Allocation(
                    "IDB publication transaction state",
                ));
            }
            let pending = tx
                .pending_requests
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("IDB pending overflow"))?;
            tx.request_queue
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("IDB request capacity"))?;
            if cancelled() {
                return Err(V8BindingError::JavaScriptException);
            }
            tx.pending_requests = pending;
            tx.request_queue.push_back(self.request_id);
        } else if !matches!(objects.get(&self.request_id), Some(PlatformObjectRecord::IdbRequest(request)) if request.transaction_id.is_none())
        {
            return Err(V8BindingError::Allocation(
                "IDB publication request identity",
            ));
        }
        // No fallible checks follow transaction mutation. Non-transaction
        // publication still checks cancellation before queue insertion.
        if self.transaction_id.is_none() && cancelled() {
            return Err(V8BindingError::JavaScriptException);
        }
        queue.push_back(self.operation);
        Ok(())
    }
}

#[cfg(test)]
mod atomic_tests {
    use super::*;
    #[test]
    fn borrowed_queue_rejects_before_identity_lookup_or_publication() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let request = new_internal_idb_request(&state, None);
        let heap = scope.get_heap_statistics();
        let used = heap
            .used_heap_size()
            .checked_add(heap.external_memory())
            .unwrap();
        let admission = crate::idb_owned_admission::Admission::metadata(
            &IdbHostCommand::ListDatabases,
            &state.idb_payload_ledger,
            heap.heap_size_limit(),
            used,
            || false,
        )
        .unwrap();
        let guard = state.queued_owned_idb.borrow_mut();
        assert!(prepare(&state, request, admission).is_err());
        assert!(guard.is_empty());
        drop(guard);
        let admission = crate::idb_owned_admission::Admission::metadata(
            &IdbHostCommand::ListDatabases,
            &state.idb_payload_ledger,
            heap.heap_size_limit(),
            used,
            || false,
        )
        .unwrap();
        let prepared = prepare(&state, request, admission).unwrap();
        let guard = state.platform_objects.borrow_mut();
        assert!(prepared.publish(&state, &mut || false).is_err());
        assert!(state.queued_owned_idb.borrow().is_empty());
        drop(guard);
        owned_unpublished_cleanup::discard(&state, request);
    }
}
