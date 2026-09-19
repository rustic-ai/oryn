//! Native document transaction identity and retained rollback custody.
use super::*;
use crate::{
    agent_registry::AgentOwner,
    storage::{IdbCommand, StorageRequest, StorageResponse},
};
use std::sync::atomic::AtomicBool;
struct Entry {
    origin: String,
    database: String,
    token: String,
    serial: Mutex<()>,
    retired: AtomicBool,
}
struct Upgrade {
    origin: String,
    database: String,
    old_version: Mutex<Option<u64>>,
    serial: Mutex<()>,
    retired: AtomicBool,
}
#[derive(Default)]
pub(super) struct Ledger {
    connections: Mutex<BTreeMap<(AgentOwner, String), Arc<Entry>>>,
    entries: Mutex<BTreeMap<(AgentOwner, String), Arc<Entry>>>,
    upgrades: Mutex<BTreeMap<(AgentOwner, String), Arc<Upgrade>>>,
}
fn failure(message: impl Into<String>) -> RuntimeError {
    RuntimeError::ContextService(message.into())
}
impl Ledger {
    fn execute(
        &self,
        context: &BrowserContext,
        owner: &AgentOwner,
        origin: String,
        mut command: IdbCommand,
    ) -> Result<StorageResponse, RuntimeError> {
        if let IdbCommand::UpgradeConnection { token, name, .. } = &command {
            let key = (owner.clone(), token.clone());
            let entry = self
                .connections
                .lock()
                .map_err(|_| failure("Window IDB connection custody poisoned"))?
                .get(&key)
                .cloned()
                .ok_or_else(|| failure("foreign private upgrade connection"))?;
            if entry.origin != origin || entry.database != *name {
                return Err(failure("private upgrade connection mismatch"));
            }
            let _serial = entry
                .serial
                .lock()
                .map_err(|_| failure("private upgrade execution poisoned"))?;
            if entry.retired.load(Ordering::Acquire) {
                return Err(failure("private upgrade retired"));
            }
            context
                .native_worker_authority(owner)
                .map_err(|error| failure(error.to_string()))?;
            let finishing = matches!(&command,IdbCommand::UpgradeConnection{inner,..}if matches!(inner.as_ref(),IdbCommand::CommitUpgrade{..}|IdbCommand::AbortUpgrade{..}));
            let name = name.clone();
            crate::storage::scope_idb_transaction(
                &mut command,
                &format!("window-document:{owner:?}"),
            );
            let result = context
                .context_store()
                .execute(StorageRequest::IndexedDb { origin, command })
                .map_err(|error| failure(error.to_string()));
            if finishing && result.is_ok() {
                self.upgrades
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&(owner.clone(), name));
            }
            return result;
        }
        if matches!(
            &command,
            IdbCommand::OpenConnection { .. }
                | IdbCommand::DeleteConnection { .. }
                | IdbCommand::CloseConnection { .. }
                | IdbCommand::RetireConnection { .. }
                | IdbCommand::PollConnection { .. }
                | IdbCommand::AcknowledgeConnection { .. }
        ) {
            return self.connection(context, owner, origin, command);
        }
        if matches!(
            &command,
            IdbCommand::Open { .. }
                | IdbCommand::CommitUpgrade { .. }
                | IdbCommand::AbortUpgrade { .. }
        ) {
            return self.upgrade(context, owner, origin, command);
        }
        let schema_database = match &command {
            IdbCommand::CreateObjectStore { database, .. }
            | IdbCommand::DeleteObjectStore { database, .. }
            | IdbCommand::CreateIndex { database, .. }
            | IdbCommand::DeleteIndex { database, .. }
            | IdbCommand::RenameObjectStore { database, .. }
            | IdbCommand::RenameIndex { database, .. } => Some(database),
            _ => None,
        };
        if let Some(database) = schema_database {
            let upgrade = self
                .upgrades
                .lock()
                .map_err(|_| failure("Window upgrade custody poisoned"))?
                .get(&(owner.clone(), database.clone()))
                .cloned()
                .ok_or_else(|| failure("schema mutation has no native upgrade owner"))?;
            let _serial = upgrade
                .serial
                .lock()
                .map_err(|_| failure("Window upgrade execution poisoned"))?;
            if upgrade.origin != origin
                || upgrade.retired.load(Ordering::Acquire)
                || upgrade
                    .old_version
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_none()
            {
                return Err(failure("retired or foreign schema upgrade"));
            }
            context
                .native_worker_authority(owner)
                .map_err(|error| failure(error.to_string()))?;
            return context
                .context_store()
                .execute(StorageRequest::IndexedDb { origin, command })
                .map_err(|error| failure(error.to_string()));
        }
        let transaction = match &command {
            IdbCommand::BeginTransaction {
                token, database, ..
            }
            | IdbCommand::Transaction {
                token, database, ..
            } => Some((token.clone(), database.clone(), true)),
            IdbCommand::CommitTransaction { token, database }
            | IdbCommand::AbortTransaction { token, database } => {
                Some((token.clone(), database.clone(), false))
            }
            _ => None,
        };
        crate::storage::scope_idb_transaction(&mut command, &format!("window-document:{owner:?}"));
        let entry = if let Some((token, database, write)) = &transaction {
            let key = (owner.clone(), token.clone());
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| failure("window transaction custody poisoned"))?;
            if let Some(entry) = entries.get(&key) {
                if entry.origin != origin || entry.database != *database {
                    return Err(failure("window transaction identity mismatch"));
                }
                Some(entry.clone())
            } else if *write {
                if entries.len() >= 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "retained Window IndexedDB transactions".into(),
                    });
                }
                let (IdbCommand::BeginTransaction { token: scoped, .. }
                | IdbCommand::Transaction { token: scoped, .. }) = &command
                else {
                    unreachable!()
                };
                let entry = Arc::new(Entry {
                    origin: origin.clone(),
                    database: database.clone(),
                    token: scoped.clone(),
                    serial: Mutex::new(()),
                    retired: AtomicBool::new(false),
                });
                entries.insert(key, entry.clone());
                Some(entry)
            } else {
                None
            }
        } else {
            None
        };
        let _serial = entry
            .as_ref()
            .map(|entry| {
                entry
                    .serial
                    .lock()
                    .map_err(|_| failure("window transaction execution poisoned"))
            })
            .transpose()?;
        if entry
            .as_ref()
            .is_some_and(|entry| entry.retired.load(Ordering::Acquire))
        {
            return Err(failure("retired Window transaction"));
        }
        context
            .native_worker_authority(owner)
            .map_err(|error| failure(error.to_string()))?;
        let result = context
            .context_store()
            .execute(StorageRequest::IndexedDb { origin, command })
            .map_err(|error| failure(error.to_string()));
        if matches!(&result,Ok(StorageResponse::IndexedDb{response})if !response.transaction_blocked)
        {
            if let (Some((token, _, false)), Some(entry)) = (&transaction, &entry) {
                self.remove(&(owner.clone(), token.clone()), entry);
            }
        }
        result
    }
    fn connection(
        &self,
        context: &BrowserContext,
        owner: &AgentOwner,
        origin: String,
        mut command: IdbCommand,
    ) -> Result<StorageResponse, RuntimeError> {
        let (token, name, admission, closing) = match &command {
            IdbCommand::OpenConnection { token, name, .. }
            | IdbCommand::DeleteConnection { token, name } => {
                (token.clone(), name.clone(), true, false)
            }
            IdbCommand::CloseConnection { token, name }
            | IdbCommand::RetireConnection { token, name } => {
                (token.clone(), name.clone(), false, true)
            }
            IdbCommand::PollConnection { token, name }
            | IdbCommand::AcknowledgeConnection { token, name, .. } => {
                (token.clone(), name.clone(), false, false)
            }
            _ => unreachable!(),
        };
        crate::storage::scope_idb_transaction(&mut command, &format!("window-document:{owner:?}"));
        let key = (owner.clone(), token);
        if matches!(&command, IdbCommand::AcknowledgeConnection { .. })
            && !self
                .connections
                .lock()
                .map_err(|_| failure("Window IDB connection custody poisoned"))?
                .contains_key(&key)
        {
            context
                .native_worker_authority(owner)
                .map_err(|error| failure(error.to_string()))?;
            return context
                .context_store()
                .execute(StorageRequest::IndexedDb { origin, command })
                .map_err(|error| failure(error.to_string()));
        }
        let entry = {
            let mut entries = self
                .connections
                .lock()
                .map_err(|_| failure("Window IDB connection custody poisoned"))?;
            if let Some(entry) = entries.get(&key) {
                if entry.origin != origin || entry.database != name {
                    return Err(failure("foreign IDB connection"));
                }
                entry.clone()
            } else {
                if !admission {
                    return Err(failure("IDB connection is not owned by this document"));
                }
                if entries.len() >= 1024 {
                    return Err(failure("Window IDB connection capacity"));
                }
                let scoped = match &command {
                    IdbCommand::OpenConnection { token, .. }
                    | IdbCommand::DeleteConnection { token, .. } => token.clone(),
                    _ => unreachable!(),
                };
                let entry = Arc::new(Entry {
                    origin: origin.clone(),
                    database: name,
                    token: scoped,
                    serial: Mutex::new(()),
                    retired: AtomicBool::new(false),
                });
                entries.insert(key.clone(), entry.clone());
                entry
            }
        };
        let _serial = entry
            .serial
            .lock()
            .map_err(|_| failure("IDB connection execution poisoned"))?;
        if entry.retired.load(Ordering::Acquire) {
            return Err(failure("retired IDB connection"));
        }
        context
            .native_worker_authority(owner)
            .map_err(|error| failure(error.to_string()))?;
        let deleting = matches!(&command, IdbCommand::DeleteConnection { .. });
        let result = context
            .context_store()
            .execute(StorageRequest::IndexedDb { origin, command })
            .map_err(|error| failure(error.to_string()));
        if matches!(&result,Ok(StorageResponse::IndexedDb{response})if !response.transaction_blocked)
            && (closing || deleting)
        {
            self.connections
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&key);
        }
        result
    }
    fn upgrade(
        &self,
        context: &BrowserContext,
        owner: &AgentOwner,
        origin: String,
        command: IdbCommand,
    ) -> Result<StorageResponse, RuntimeError> {
        let (name, opening) = match &command {
            IdbCommand::Open { name, .. } | IdbCommand::OpenConnection { name, .. } => {
                (name.clone(), true)
            }
            IdbCommand::CommitUpgrade { name } | IdbCommand::AbortUpgrade { name, .. } => {
                (name.clone(), false)
            }
            _ => unreachable!(),
        };
        let key = (owner.clone(), name.clone());
        let entry = {
            let mut upgrades = self
                .upgrades
                .lock()
                .map_err(|_| failure("Window upgrade custody poisoned"))?;
            if let Some(entry) = upgrades.get(&key) {
                if opening {
                    return Err(failure("database upgrade already owned"));
                }
                if entry.origin != origin {
                    return Err(failure("foreign database upgrade"));
                }
                entry.clone()
            } else if opening {
                if upgrades
                    .values()
                    .any(|entry| entry.origin == origin && entry.database == name)
                {
                    return Err(failure("database upgrade is owned by another document"));
                }
                if upgrades.len() >= 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "retained Window database upgrades".into(),
                    });
                }
                let entry = Arc::new(Upgrade {
                    origin: origin.clone(),
                    database: name,
                    old_version: Mutex::new(None),
                    serial: Mutex::new(()),
                    retired: AtomicBool::new(false),
                });
                upgrades.insert(key.clone(), entry.clone());
                entry
            } else {
                return Err(failure("database upgrade has no matching document owner"));
            }
        };
        let _serial = entry
            .serial
            .lock()
            .map_err(|_| failure("Window upgrade execution poisoned"))?;
        if entry.retired.load(Ordering::Acquire) {
            return Err(failure("retired Window upgrade"));
        }
        context
            .native_worker_authority(owner)
            .map_err(|error| failure(error.to_string()))?;
        // Abort metadata comes from the native Open result, never author JSON.
        let command = if matches!(&command, IdbCommand::AbortUpgrade { .. }) {
            IdbCommand::AbortUpgrade {
                name: entry.database.clone(),
                old_version: entry
                    .old_version
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .ok_or_else(|| failure("upgrade rollback version unavailable"))?,
            }
        } else {
            command
        };
        let result = context
            .context_store()
            .execute(StorageRequest::IndexedDb { origin, command })
            .map_err(|error| failure(error.to_string()));
        let retain = if opening {
            match &result {
                Ok(StorageResponse::IndexedDb { response }) if response.upgrade_needed => {
                    *entry
                        .old_version
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some(response.old_version.unwrap_or(0));
                    true
                }
                _ => false,
            }
        } else {
            result.is_err()
        };
        if !retain {
            let mut upgrades = self
                .upgrades
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if upgrades
                .get(&key)
                .is_some_and(|old| Arc::ptr_eq(old, &entry))
            {
                upgrades.remove(&key);
            }
        }
        result
    }
    fn remove(&self, key: &(AgentOwner, String), entry: &Arc<Entry>) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if entries.get(key).is_some_and(|old| Arc::ptr_eq(old, entry)) {
            entries.remove(key);
        }
    }
    fn retire(
        &self,
        context: &BrowserContext,
        select: impl Fn(&AgentOwner) -> bool,
    ) -> Result<(), RuntimeError> {
        let pending = {
            let entries = self
                .entries
                .lock()
                .map_err(|_| failure("window transaction custody poisoned"))?;
            entries
                .iter()
                .filter(|((owner, _), _)| select(owner))
                .map(|(key, entry)| {
                    entry.retired.store(true, Ordering::Release);
                    (key.clone(), entry.clone())
                })
                .collect::<Vec<_>>()
        };
        let mut errors = Vec::new();
        for (key, entry) in pending {
            let _serial = entry
                .serial
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match context.context_store().execute(StorageRequest::IndexedDb {
                origin: entry.origin.clone(),
                command: IdbCommand::AbortTransaction {
                    token: entry.token.clone(),
                    database: entry.database.clone(),
                },
            }) {
                Ok(_) => self.remove(&key, &entry),
                Err(error) => errors.push(error.to_string()),
            }
        }
        let upgrades = {
            let upgrades = self
                .upgrades
                .lock()
                .map_err(|_| failure("Window upgrade custody poisoned"))?;
            upgrades
                .iter()
                .filter(|((owner, _), _)| select(owner))
                .map(|(key, entry)| {
                    entry.retired.store(true, Ordering::Release);
                    (key.clone(), entry.clone())
                })
                .collect::<Vec<_>>()
        };
        for (key, entry) in upgrades {
            let _serial = entry
                .serial
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let old = *entry
                .old_version
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let result = old
                .map(|old_version| {
                    context.context_store().execute(StorageRequest::IndexedDb {
                        origin: entry.origin.clone(),
                        command: IdbCommand::AbortUpgrade {
                            name: entry.database.clone(),
                            old_version,
                        },
                    })
                })
                .transpose();
            match result {
                Ok(_) => {
                    let mut upgrades = self
                        .upgrades
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if upgrades
                        .get(&key)
                        .is_some_and(|old| Arc::ptr_eq(old, &entry))
                    {
                        upgrades.remove(&key);
                    }
                }
                Err(error) => errors.push(error.to_string()),
            }
        }
        let connections = self
            .connections
            .lock()
            .map_err(|_| failure("Window IDB connection custody poisoned"))?
            .iter()
            .filter(|((owner, _), _)| select(owner))
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect::<Vec<_>>();
        for (key, entry) in connections {
            entry.retired.store(true, Ordering::Release);
            let _serial = entry
                .serial
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match context.context_store().execute(StorageRequest::IndexedDb {
                origin: entry.origin.clone(),
                command: IdbCommand::RetireConnection {
                    token: entry.token.clone(),
                    name: entry.database.clone(),
                },
            }) {
                Ok(_) => {
                    self.connections
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .remove(&key);
                }
                Err(error) => errors.push(error.to_string()),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(failure(errors.join("; ")))
        }
    }
}
impl BrowserContext {
    pub(crate) fn execute_window_indexed_db(
        &self,
        owner: &AgentOwner,
        partition: &str,
        origin: &oryn_js::worker_wire::AgentOrigin,
        command: IdbCommand,
    ) -> Result<StorageResponse, RuntimeError> {
        if origin.opaque {
            return Err(failure(
                "SecurityError: opaque document cannot access IndexedDB",
            ));
        }
        self.inner.window_idb.execute(
            self,
            owner,
            crate::storage::partitioned_idb_origin(partition, &origin.authority),
            command,
        )
    }
    pub(crate) fn retire_window_transactions(
        &self,
        owner: &AgentOwner,
    ) -> Result<(), RuntimeError> {
        self.inner
            .window_idb
            .retire(self, |candidate| candidate == owner)
    }
    pub(super) fn retire_page_transactions(&self, page: PageId) -> Result<(), RuntimeError> {
        self.inner.window_idb.retire(
            self,
            |owner| matches!(owner,AgentOwner::Window{page:owned,..}if *owned==page),
        )
    }
    pub(super) fn retire_all_window_transactions(&self) -> Result<(), RuntimeError> {
        self.inner.window_idb.retire(self, |_| true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn document_replacement_rolls_back_only_its_exact_transaction_owner() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://db.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (owner, partition, origin) = context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap();
        let execute = |owner: &AgentOwner, command| {
            context
                .execute_window_indexed_db(owner, &partition, &origin, command)
                .unwrap()
        };
        for command in [
            IdbCommand::Open {
                name: "db".into(),
                version: 1,
            },
            IdbCommand::CreateObjectStore {
                database: "db".into(),
                definition: crate::storage::ObjectStoreDefinition {
                    name: "records".into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            },
            IdbCommand::CommitUpgrade { name: "db".into() },
        ] {
            execute(&owner, command);
        }
        let write = |value: &str| IdbCommand::Batch {
            database: "db".into(),
            writes: vec![crate::storage::IdbWrite {
                store: "records".into(),
                key: Some(serde_json::json!(1)),
                value: serde_json::json!(value),
                add_only: false,
            }],
        };
        execute(&owner, write("baseline"));
        let open = || IdbCommand::OpenConnection {
            token: "connection".into(),
            name: "db".into(),
            version: 1,
        };
        let begin = |token: &str, mode: &str| IdbCommand::BeginTransaction {
            token: token.into(),
            database: "db".into(),
            scope: vec!["records".into()],
            mode: mode.into(),
            connection: Some("connection".into()),
        };
        execute(&owner, open());
        execute(&owner, begin("same-token", "readwrite"));
        execute(
            &owner,
            IdbCommand::Transaction {
                token: "same-token".into(),
                database: "db".into(),
                inner: Box::new(write("old-uncommitted")),
            },
        );
        assert_eq!(context.inner.window_idb.entries.lock().unwrap().len(), 1);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(context.inner.window_idb.entries.lock().unwrap().is_empty());
        let (next, _, _) = context
            .window_worker_creator(page, &WindowTarget::Root, 2, 0)
            .unwrap();
        let get = || IdbCommand::Get {
            database: "db".into(),
            store: "records".into(),
            key: serde_json::json!(1),
        };
        let StorageResponse::IndexedDb { response } = execute(&next, get()) else {
            panic!()
        };
        assert_eq!(response.value, Some(serde_json::json!("baseline")));
        execute(&next, open());
        execute(&next, begin("same-token", "readwrite"));
        execute(
            &next,
            IdbCommand::Transaction {
                token: "same-token".into(),
                database: "db".into(),
                inner: Box::new(write("new-uncommitted")),
            },
        );
        context.retire_window_transactions(&owner).unwrap();
        assert_eq!(
            context.inner.window_idb.entries.lock().unwrap().len(),
            1,
            "old cleanup must not consume replacement transaction"
        );
        execute(
            &next,
            IdbCommand::AbortTransaction {
                token: "same-token".into(),
                database: "db".into(),
            },
        );
        assert!(context.inner.window_idb.entries.lock().unwrap().is_empty());
        let StorageResponse::IndexedDb { response } = execute(&next, get()) else {
            panic!()
        };
        assert_eq!(response.value, Some(serde_json::json!("baseline")));
        execute(&next, begin("read-only", "readonly"));
        for _ in 0..3 {
            execute(
                &next,
                IdbCommand::Transaction {
                    token: "read-only".into(),
                    database: "db".into(),
                    inner: Box::new(get()),
                },
            );
        }
        assert_eq!(
            context.inner.window_idb.entries.lock().unwrap().len(),
            1,
            "read-only snapshot retains exact cleanup custody"
        );
        execute(
            &next,
            IdbCommand::CommitTransaction {
                token: "read-only".into(),
                database: "db".into(),
            },
        );
        assert!(context.inner.window_idb.entries.lock().unwrap().is_empty());
        execute(
            &next,
            IdbCommand::CloseConnection {
                token: "connection".into(),
                name: "db".into(),
            },
        );
        execute(
            &next,
            IdbCommand::Open {
                name: "db".into(),
                version: 2,
            },
        );
        execute(
            &next,
            IdbCommand::CreateObjectStore {
                database: "db".into(),
                definition: crate::storage::ObjectStoreDefinition {
                    name: "transient".into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            },
        );
        assert_eq!(context.inner.window_idb.upgrades.lock().unwrap().len(), 1);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 3,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(context.inner.window_idb.upgrades.lock().unwrap().is_empty());
        let (latest, _, _) = context
            .window_worker_creator(page, &WindowTarget::Root, 3, 0)
            .unwrap();
        let StorageResponse::IndexedDb { response } = execute(
            &latest,
            IdbCommand::Open {
                name: "db".into(),
                version: 1,
            },
        ) else {
            panic!()
        };
        assert_eq!(response.version, Some(1));
        assert!(!response.object_stores.contains(&"transient".into()));
        assert!(
            context
                .execute_window_indexed_db(
                    &latest,
                    &partition,
                    &origin,
                    IdbCommand::CommitUpgrade { name: "db".into() }
                )
                .is_err(),
            "replacement cannot commit a retired upgrade"
        );
    }
}

#[cfg(all(test, unix))]
mod retirement_fanout_tests {
    use super::*;
    #[tokio::test]
    async fn failed_deferred_flush_still_retires_page_transactions_and_preserves_sibling() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page().page_id();
        let sibling = context.new_page().page_id();
        let owner = |page| AgentOwner::Window {
            page,
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let entry = || {
            Arc::new(Entry {
                origin: "https://db.test".into(),
                database: "db".into(),
                token: "native-retirement-token".into(),
                serial: Mutex::new(()),
                retired: AtomicBool::new(false),
            })
        };
        let target = entry();
        let survivor = entry();
        context
            .inner
            .window_idb
            .entries
            .lock()
            .unwrap()
            .insert((owner(page), "token".into()), target.clone());
        context
            .inner
            .window_idb
            .entries
            .lock()
            .unwrap()
            .insert((owner(sibling), "token".into()), survivor.clone());
        let injected = context.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = injected.inner.deferred_fetches.lock().unwrap();
                panic!("injected deferred registry failure")
            })
            .join()
            .is_err()
        );
        let error = context.retire_page_agents(page).await.unwrap_err();
        assert!(error.to_string().contains("deferred registry poisoned"));
        assert!(target.retired.load(Ordering::Acquire));
        assert!(!survivor.retired.load(Ordering::Acquire));
        let entries = context.inner.window_idb.entries.lock().unwrap();
        assert!(!entries.contains_key(&(owner(page), "token".into())));
        assert!(Arc::ptr_eq(
            entries.get(&(owner(sibling), "token".into())).unwrap(),
            &survivor
        ));
        drop(entries);
        // Retry reports the unresolved first owner, without resurrecting the
        // released transaction or consuming another page's remaining custody.
        assert!(context.retire_page_agents(page).await.is_err());
        assert_eq!(context.inner.window_idb.entries.lock().unwrap().len(), 1);
    }
}
