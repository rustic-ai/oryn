//! Browser-context-owned storage for modern application compatibility.
//!
//! Page workers never open this database. They access it only through the
//! typed context service, keeping filesystem access in the privileged parent.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use oryn_common::v3::PageId;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::cookies::ContextCookieStore;

const LOCAL_STORAGE: TableDefinition<&str, &[u8]> = TableDefinition::new("local_storage_v1");
const INDEXED_DATABASES: TableDefinition<&str, &[u8]> =
    TableDefinition::new("indexed_databases_v1");
const PERSISTENT_COOKIES: TableDefinition<&str, &[u8]> =
    TableDefinition::new("persistent_cookies_v1");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageLimits {
    pub web_storage_bytes_per_origin: usize,
    pub indexed_db_bytes_per_origin: usize,
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            web_storage_bytes_per_origin: 10 * 1024 * 1024,
            indexed_db_bytes_per_origin: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageArea {
    Local,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageScope {
    pub origin: String,
    pub top_level_page_id: PageId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageChange {
    pub area: StorageArea,
    pub scope: StorageScope,
    pub key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum StorageRequest {
    Get {
        area: StorageArea,
        scope: StorageScope,
        key: String,
    },
    Set {
        area: StorageArea,
        scope: StorageScope,
        key: String,
        value: String,
    },
    Remove {
        area: StorageArea,
        scope: StorageScope,
        key: String,
    },
    Clear {
        area: StorageArea,
        scope: StorageScope,
    },
    Entries {
        area: StorageArea,
        scope: StorageScope,
    },
    IndexedDb {
        origin: String,
        command: IdbCommand,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum StorageResponse {
    Value { value: Option<String> },
    Entries { entries: Vec<(String, String)> },
    Changed { change: StorageChange },
    IndexedDb { response: IdbResponse },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectStoreDefinition {
    pub name: String,
    pub key_path: Option<String>,
    pub auto_increment: bool,
    #[serde(default)]
    pub indexes: Vec<IndexDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub key_path: String,
    pub unique: bool,
    #[serde(default)]
    pub multi_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdbWrite {
    pub store: String,
    pub key: Option<serde_json::Value>,
    pub value: serde_json::Value,
    pub add_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IdbCommand {
    Transaction {
        token: String,
        database: String,
        inner: Box<IdbCommand>,
    },
    CommitTransaction {
        token: String,
        database: String,
    },
    AbortTransaction {
        token: String,
        database: String,
    },
    ListDatabases,
    Open {
        name: String,
        version: u64,
    },
    DeleteDatabase {
        name: String,
    },
    AbortUpgrade {
        name: String,
        old_version: u64,
    },
    CommitUpgrade {
        name: String,
    },
    CreateObjectStore {
        database: String,
        definition: ObjectStoreDefinition,
    },
    DeleteObjectStore {
        database: String,
        store: String,
    },
    CreateIndex {
        database: String,
        store: String,
        definition: IndexDefinition,
    },
    DeleteIndex {
        database: String,
        store: String,
        index: String,
    },
    RenameIndex {
        database: String,
        store: String,
        index: String,
        new_name: String,
    },
    Batch {
        database: String,
        writes: Vec<IdbWrite>,
    },
    Get {
        database: String,
        store: String,
        key: serde_json::Value,
    },
    GetAll {
        database: String,
        store: String,
    },
    Delete {
        database: String,
        store: String,
        key: serde_json::Value,
    },
    Clear {
        database: String,
        store: String,
    },
    Count {
        database: String,
        store: String,
    },
    Cursor {
        database: String,
        store: String,
        index: Option<String>,
        query: Option<serde_json::Value>,
        reverse: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdbRecord {
    pub key: serde_json::Value,
    pub primary_key: serde_json::Value,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub version: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdbResponse {
    pub databases: Vec<DatabaseInfo>,
    pub version: Option<u64>,
    pub old_version: Option<u64>,
    pub upgrade_needed: bool,
    pub object_stores: Vec<String>,
    pub object_store_definitions: Vec<ObjectStoreDefinition>,
    pub key: Option<serde_json::Value>,
    pub value: Option<serde_json::Value>,
    pub values: Vec<serde_json::Value>,
    pub records: Vec<IdbRecord>,
    pub count: Option<u64>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageError {
    #[error("storage quota exceeded for {origin}; limit is {limit} bytes")]
    QuotaExceeded { origin: String, limit: usize },
    #[error("persistent profile failed: {0}")]
    Persistent(String),
    #[error("IndexedDB database {0:?} does not exist")]
    DatabaseNotFound(String),
    #[error("IndexedDB object store {store:?} does not exist in {database:?}")]
    ObjectStoreNotFound { database: String, store: String },
    #[error("IndexedDB object store {0:?} already exists")]
    ObjectStoreExists(String),
    #[error("IndexedDB index {index:?} does not exist in object store {store:?}")]
    IndexNotFound { store: String, index: String },
    #[error("IndexedDB key is required")]
    KeyRequired,
    #[error("IndexedDB key already exists")]
    Constraint,
    #[error("IndexedDB structured value cannot be cloned: {0}")]
    DataClone(String),
    #[error(
        "VersionError: requested IndexedDB version {requested} is below current version {current}"
    )]
    Version { requested: u64, current: u64 },
}

pub trait ContextStore: Send + Sync {
    fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError>;
}

#[derive(Clone)]
pub struct SharedContextStore(Arc<dyn ContextStore>);

impl SharedContextStore {
    pub fn memory(limits: StorageLimits) -> Self {
        Self(Arc::new(MemoryContextStore::new(limits)))
    }

    pub fn persistent(profile: &PersistentProfile, limits: StorageLimits) -> Self {
        Self(Arc::new(PersistentContextStore::new(
            profile.database.clone(),
            limits,
        )))
    }

    pub fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError> {
        self.0.execute(request)
    }
}

#[derive(Clone)]
pub struct PersistentProfile {
    path: PathBuf,
    database: Arc<Database>,
    cookies: Arc<ContextCookieStore>,
}

impl std::fmt::Debug for PersistentProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistentProfile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PersistentProfile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let database_path = path.join("oryn-profile.redb");
        let database = Database::create(&database_path)
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        {
            let write = database
                .begin_write()
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            write
                .open_table(LOCAL_STORAGE)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            write
                .open_table(INDEXED_DATABASES)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            write
                .open_table(PERSISTENT_COOKIES)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            write
                .commit()
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
        }
        let database = Arc::new(database);
        let cookies = ContextCookieStore::persistent(database.clone())
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        Ok(Self {
            path: path.to_path_buf(),
            database,
            cookies,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn cookie_store(&self) -> Arc<ContextCookieStore> {
        self.cookies.clone()
    }
}

#[derive(Default)]
struct MemoryState {
    local: BTreeMap<(String, String), String>,
    session: BTreeMap<(PageId, String, String), String>,
    indexed_db: BTreeMap<(String, String), DatabaseData>,
    indexed_db_transactions: BTreeMap<String, TransactionSnapshot>,
}

struct MemoryContextStore {
    state: Mutex<MemoryState>,
    limits: StorageLimits,
}

impl MemoryContextStore {
    fn new(limits: StorageLimits) -> Self {
        Self {
            state: Mutex::new(MemoryState::default()),
            limits,
        }
    }
}

impl ContextStore for MemoryContextStore {
    fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("memory store lock poisoned".into()))?;
        match request {
            StorageRequest::Get { area, scope, key } => Ok(StorageResponse::Value {
                value: memory_get(&state, area, &scope, &key),
            }),
            StorageRequest::Set {
                area,
                scope,
                key,
                value,
            } => {
                let old_value = memory_get(&state, area, &scope, &key);
                memory_set(&mut state, area, &scope, key.clone(), value.clone());
                if memory_area_bytes(&state, area, &scope)
                    > self.limits.web_storage_bytes_per_origin
                {
                    match old_value.clone() {
                        Some(old) => memory_set(&mut state, area, &scope, key.clone(), old),
                        None => memory_remove(&mut state, area, &scope, &key),
                    }
                    return Err(StorageError::QuotaExceeded {
                        origin: scope.origin,
                        limit: self.limits.web_storage_bytes_per_origin,
                    });
                }
                Ok(StorageResponse::Changed {
                    change: StorageChange {
                        area,
                        scope,
                        key: Some(key),
                        old_value,
                        new_value: Some(value),
                    },
                })
            }
            StorageRequest::Remove { area, scope, key } => {
                let old_value = memory_get(&state, area, &scope, &key);
                memory_remove(&mut state, area, &scope, &key);
                Ok(StorageResponse::Changed {
                    change: StorageChange {
                        area,
                        scope,
                        key: Some(key),
                        old_value,
                        new_value: None,
                    },
                })
            }
            StorageRequest::Clear { area, scope } => {
                memory_clear(&mut state, area, &scope);
                Ok(StorageResponse::Changed {
                    change: StorageChange {
                        area,
                        scope,
                        key: None,
                        old_value: None,
                        new_value: None,
                    },
                })
            }
            StorageRequest::Entries { area, scope } => Ok(StorageResponse::Entries {
                entries: memory_entries(&state, area, &scope),
            }),
            StorageRequest::IndexedDb { origin, command } => {
                let MemoryState {
                    indexed_db,
                    indexed_db_transactions,
                    ..
                } = &mut *state;
                let response = execute_idb_transactional(
                    indexed_db,
                    indexed_db_transactions,
                    &origin,
                    command,
                    self.limits.indexed_db_bytes_per_origin,
                )?;
                Ok(StorageResponse::IndexedDb { response })
            }
        }
    }
}

struct PersistentContextStore {
    database: Arc<Database>,
    session: Mutex<BTreeMap<(PageId, String, String), String>>,
    indexed_db_transactions: Mutex<BTreeMap<String, TransactionSnapshot>>,
    limits: StorageLimits,
}

impl PersistentContextStore {
    fn new(database: Arc<Database>, limits: StorageLimits) -> Self {
        Self {
            database,
            session: Mutex::new(BTreeMap::new()),
            indexed_db_transactions: Mutex::new(BTreeMap::new()),
            limits,
        }
    }

    fn local_entries(&self, origin: &str) -> Result<Vec<(String, String)>, StorageError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let table = read
            .open_table(LOCAL_STORAGE)
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let prefix = format!("{origin}\0");
        let mut entries = Vec::new();
        let iterator = table
            .iter()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        for item in iterator {
            let (key, value) = item.map_err(|error| StorageError::Persistent(error.to_string()))?;
            if let Some(key) = key.value().strip_prefix(&prefix) {
                entries.push((
                    key.to_string(),
                    String::from_utf8(value.value().to_vec())
                        .map_err(|error| StorageError::Persistent(error.to_string()))?,
                ));
            }
        }
        Ok(entries)
    }

    fn update_local(
        &self,
        scope: StorageScope,
        key: Option<String>,
        value: Option<String>,
        clear: bool,
    ) -> Result<StorageResponse, StorageError> {
        let old_entries = self.local_entries(&scope.origin)?;
        let old_value = key
            .as_ref()
            .and_then(|key| old_entries.iter().find(|(item, _)| item == key))
            .map(|(_, value)| value.clone());
        let mut next = old_entries.into_iter().collect::<BTreeMap<_, _>>();
        if clear {
            next.clear();
        } else if let Some(key) = &key {
            if let Some(value) = &value {
                next.insert(key.clone(), value.clone());
            } else {
                next.remove(key);
            }
        }
        let size = next
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>();
        if size > self.limits.web_storage_bytes_per_origin {
            return Err(StorageError::QuotaExceeded {
                origin: scope.origin,
                limit: self.limits.web_storage_bytes_per_origin,
            });
        }
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        {
            let mut table = write
                .open_table(LOCAL_STORAGE)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            let prefix = format!("{}\0", scope.origin);
            let removals = table
                .iter()
                .map_err(|error| StorageError::Persistent(error.to_string()))?
                .filter_map(|item| item.ok().map(|(key, _)| key.value().to_string()))
                .filter(|key| key.starts_with(&prefix))
                .collect::<Vec<_>>();
            for key in removals {
                table
                    .remove(key.as_str())
                    .map_err(|error| StorageError::Persistent(error.to_string()))?;
            }
            for (key, value) in next {
                let key = format!("{prefix}{key}");
                table
                    .insert(key.as_str(), value.as_bytes())
                    .map_err(|error| StorageError::Persistent(error.to_string()))?;
            }
        }
        write
            .commit()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        Ok(StorageResponse::Changed {
            change: StorageChange {
                area: StorageArea::Local,
                scope,
                key,
                old_value,
                new_value: value,
            },
        })
    }

    fn load_idb(
        &self,
        origin: &str,
    ) -> Result<BTreeMap<(String, String), DatabaseData>, StorageError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let table = read
            .open_table(INDEXED_DATABASES)
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let prefix = format!("{origin}\0");
        let mut databases = BTreeMap::new();
        let iterator = table
            .iter()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        for item in iterator {
            let (key, value) = item.map_err(|error| StorageError::Persistent(error.to_string()))?;
            let Some(name) = key.value().strip_prefix(&prefix) else {
                continue;
            };
            let database = serde_json::from_slice(value.value())
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            databases.insert((origin.to_string(), name.to_string()), database);
        }
        Ok(databases)
    }

    fn save_idb(
        &self,
        origin: &str,
        databases: BTreeMap<(String, String), DatabaseData>,
    ) -> Result<(), StorageError> {
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        {
            let mut table = write
                .open_table(INDEXED_DATABASES)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            let prefix = format!("{origin}\0");
            let removals = table
                .iter()
                .map_err(|error| StorageError::Persistent(error.to_string()))?
                .filter_map(|item| item.ok().map(|(key, _)| key.value().to_string()))
                .filter(|key| key.starts_with(&prefix))
                .collect::<Vec<_>>();
            for key in removals {
                table
                    .remove(key.as_str())
                    .map_err(|error| StorageError::Persistent(error.to_string()))?;
            }
            for ((_, name), database) in databases {
                let key = format!("{prefix}{name}");
                let value = serde_json::to_vec(&database)
                    .map_err(|error| StorageError::Persistent(error.to_string()))?;
                table
                    .insert(key.as_str(), value.as_slice())
                    .map_err(|error| StorageError::Persistent(error.to_string()))?;
            }
        }
        write
            .commit()
            .map_err(|error| StorageError::Persistent(error.to_string()))
    }
}

impl ContextStore for PersistentContextStore {
    fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError> {
        match request {
            StorageRequest::Get {
                area: StorageArea::Local,
                scope,
                key,
            } => Ok(StorageResponse::Value {
                value: self
                    .local_entries(&scope.origin)?
                    .into_iter()
                    .find(|(item, _)| item == &key)
                    .map(|(_, value)| value),
            }),
            StorageRequest::Set {
                area: StorageArea::Local,
                scope,
                key,
                value,
            } => self.update_local(scope, Some(key), Some(value), false),
            StorageRequest::Remove {
                area: StorageArea::Local,
                scope,
                key,
            } => self.update_local(scope, Some(key), None, false),
            StorageRequest::Clear {
                area: StorageArea::Local,
                scope,
            } => self.update_local(scope, None, None, true),
            StorageRequest::Entries {
                area: StorageArea::Local,
                scope,
            } => Ok(StorageResponse::Entries {
                entries: self.local_entries(&scope.origin)?,
            }),
            StorageRequest::IndexedDb { origin, command } => {
                let mut databases = self.load_idb(&origin)?;
                let mut transactions = self.indexed_db_transactions.lock().map_err(|_| {
                    StorageError::Persistent("IndexedDB transaction lock poisoned".into())
                })?;
                let response = execute_idb_transactional(
                    &mut databases,
                    &mut transactions,
                    &origin,
                    command,
                    self.limits.indexed_db_bytes_per_origin,
                )?;
                self.save_idb(&origin, databases)?;
                Ok(StorageResponse::IndexedDb { response })
            }
            request => {
                let mut session = self
                    .session
                    .lock()
                    .map_err(|_| StorageError::Persistent("session store lock poisoned".into()))?;
                execute_session(&mut session, request, self.limits)
            }
        }
    }
}

fn execute_session(
    session: &mut BTreeMap<(PageId, String, String), String>,
    request: StorageRequest,
    limits: StorageLimits,
) -> Result<StorageResponse, StorageError> {
    let mut state = MemoryState {
        session: std::mem::take(session),
        ..MemoryState::default()
    };
    let store = MemoryContextStore::new(limits);
    *store.state.lock().expect("fresh memory store") = state;
    let response = store.execute(request)?;
    state = store.state.into_inner().expect("fresh memory store");
    *session = state.session;
    Ok(response)
}

fn memory_get(
    state: &MemoryState,
    area: StorageArea,
    scope: &StorageScope,
    key: &str,
) -> Option<String> {
    match area {
        StorageArea::Local => state
            .local
            .get(&(scope.origin.clone(), key.to_string()))
            .cloned(),
        StorageArea::Session => state
            .session
            .get(&(
                scope.top_level_page_id,
                scope.origin.clone(),
                key.to_string(),
            ))
            .cloned(),
    }
}

fn memory_set(
    state: &mut MemoryState,
    area: StorageArea,
    scope: &StorageScope,
    key: String,
    value: String,
) {
    match area {
        StorageArea::Local => {
            state.local.insert((scope.origin.clone(), key), value);
        }
        StorageArea::Session => {
            state
                .session
                .insert((scope.top_level_page_id, scope.origin.clone(), key), value);
        }
    }
}

fn memory_remove(state: &mut MemoryState, area: StorageArea, scope: &StorageScope, key: &str) {
    match area {
        StorageArea::Local => {
            state.local.remove(&(scope.origin.clone(), key.to_string()));
        }
        StorageArea::Session => {
            state.session.remove(&(
                scope.top_level_page_id,
                scope.origin.clone(),
                key.to_string(),
            ));
        }
    }
}

fn memory_clear(state: &mut MemoryState, area: StorageArea, scope: &StorageScope) {
    match area {
        StorageArea::Local => state.local.retain(|(origin, _), _| origin != &scope.origin),
        StorageArea::Session => state.session.retain(|(page_id, origin, _), _| {
            *page_id != scope.top_level_page_id || origin != &scope.origin
        }),
    }
}

fn memory_entries(
    state: &MemoryState,
    area: StorageArea,
    scope: &StorageScope,
) -> Vec<(String, String)> {
    match area {
        StorageArea::Local => state
            .local
            .iter()
            .filter(|((origin, _), _)| origin == &scope.origin)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect(),
        StorageArea::Session => state
            .session
            .iter()
            .filter(|((page_id, origin, _), _)| {
                *page_id == scope.top_level_page_id && origin == &scope.origin
            })
            .map(|((_, _, key), value)| (key.clone(), value.clone()))
            .collect(),
    }
}

fn memory_area_bytes(state: &MemoryState, area: StorageArea, scope: &StorageScope) -> usize {
    memory_entries(state, area, scope)
        .iter()
        .map(|(key, value)| key.len() + value.len())
        .sum()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DatabaseData {
    version: u64,
    stores: BTreeMap<String, ObjectStoreData>,
    #[serde(skip)]
    upgrade_backup: Option<Box<DatabaseData>>,
}

#[derive(Debug, Clone)]
struct TransactionSnapshot {
    origin: String,
    database: String,
    before: Option<DatabaseData>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ObjectStoreData {
    definition: Option<ObjectStoreDefinition>,
    indexes: BTreeMap<String, IndexDefinition>,
    next_key: u64,
    records: BTreeMap<String, RecordData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordData {
    key: serde_json::Value,
    value: serde_json::Value,
}

fn execute_idb_transactional(
    databases: &mut BTreeMap<(String, String), DatabaseData>,
    transactions: &mut BTreeMap<String, TransactionSnapshot>,
    origin: &str,
    command: IdbCommand,
    limit: usize,
) -> Result<IdbResponse, StorageError> {
    match command {
        IdbCommand::Transaction {
            token,
            database,
            inner,
        } => {
            let key = (origin.to_string(), database.clone());
            transactions
                .entry(token)
                .or_insert_with(|| TransactionSnapshot {
                    origin: origin.to_string(),
                    database,
                    before: databases.get(&key).cloned(),
                });
            execute_idb(databases, origin, *inner, limit)
        }
        IdbCommand::CommitTransaction { token, database } => {
            if let Some(snapshot) = transactions.remove(&token)
                && (snapshot.origin != origin || snapshot.database != database)
            {
                return Err(StorageError::Persistent(
                    "IndexedDB transaction identity mismatch".into(),
                ));
            }
            Ok(IdbResponse::default())
        }
        IdbCommand::AbortTransaction { token, database } => {
            if let Some(snapshot) = transactions.remove(&token) {
                if snapshot.origin != origin || snapshot.database != database {
                    return Err(StorageError::Persistent(
                        "IndexedDB transaction identity mismatch".into(),
                    ));
                }
                let key = (origin.to_string(), database);
                match snapshot.before {
                    Some(before) => {
                        databases.insert(key, before);
                    }
                    None => {
                        databases.remove(&key);
                    }
                }
            }
            Ok(IdbResponse::default())
        }
        command => execute_idb(databases, origin, command, limit),
    }
}

fn execute_idb(
    databases: &mut BTreeMap<(String, String), DatabaseData>,
    origin: &str,
    command: IdbCommand,
    limit: usize,
) -> Result<IdbResponse, StorageError> {
    let response = match command {
        IdbCommand::Transaction { .. }
        | IdbCommand::CommitTransaction { .. }
        | IdbCommand::AbortTransaction { .. } => {
            return Err(StorageError::Persistent(
                "nested IndexedDB transaction command".into(),
            ));
        }
        IdbCommand::ListDatabases => IdbResponse {
            databases: databases
                .iter()
                .filter(|((item_origin, _), _)| item_origin == origin)
                .filter_map(|((_, name), database)| {
                    let visible = database.upgrade_backup.as_deref().unwrap_or(database);
                    (visible.version > 0).then(|| DatabaseInfo {
                        name: name.clone(),
                        version: visible.version,
                    })
                })
                .collect(),
            ..IdbResponse::default()
        },
        IdbCommand::Open { name, version } => {
            let key = (origin.to_string(), name);
            let database = databases.entry(key).or_default();
            let old_version = database.version;
            let requested = if version == 0 {
                old_version.max(1)
            } else {
                version
            };
            if old_version > 0 && requested < old_version {
                return Err(StorageError::Version {
                    requested,
                    current: old_version,
                });
            }
            let upgrade_needed = requested > old_version;
            if upgrade_needed {
                let mut backup = database.clone();
                backup.upgrade_backup = None;
                database.upgrade_backup = Some(Box::new(backup));
                database.version = requested;
            }
            IdbResponse {
                version: Some(database.version),
                old_version: Some(old_version),
                upgrade_needed,
                object_stores: database.stores.keys().cloned().collect(),
                object_store_definitions: database
                    .stores
                    .values()
                    .filter_map(|store| {
                        let mut definition = store.definition.clone()?;
                        definition.indexes = store.indexes.values().cloned().collect();
                        Some(definition)
                    })
                    .collect(),
                ..IdbResponse::default()
            }
        }
        IdbCommand::DeleteDatabase { name } => {
            let old_version = databases
                .remove(&(origin.to_string(), name))
                .map(|database| database.version)
                .unwrap_or(0);
            IdbResponse {
                old_version: Some(old_version),
                ..IdbResponse::default()
            }
        }
        IdbCommand::AbortUpgrade { name, old_version } => {
            let key = (origin.to_string(), name);
            if let Some(backup) = databases
                .get_mut(&key)
                .and_then(|database| database.upgrade_backup.take())
            {
                databases.insert(key, *backup);
            } else if old_version == 0 {
                databases.remove(&key);
            } else if let Some(database) = databases.get_mut(&key) {
                database.version = old_version;
            }
            IdbResponse::default()
        }
        IdbCommand::CommitUpgrade { name } => {
            if let Some(database) = databases.get_mut(&(origin.to_string(), name)) {
                database.upgrade_backup = None;
            }
            IdbResponse::default()
        }
        IdbCommand::CreateObjectStore {
            database,
            definition,
        } => {
            let data = database_mut(databases, origin, &database)?;
            if data.stores.contains_key(&definition.name) {
                return Err(StorageError::ObjectStoreExists(definition.name));
            }
            data.stores.insert(
                definition.name.clone(),
                ObjectStoreData {
                    definition: Some(definition),
                    next_key: 1,
                    ..ObjectStoreData::default()
                },
            );
            IdbResponse {
                version: Some(data.version),
                object_stores: data.stores.keys().cloned().collect(),
                ..IdbResponse::default()
            }
        }
        IdbCommand::DeleteObjectStore { database, store } => {
            let data = database_mut(databases, origin, &database)?;
            if data.stores.remove(&store).is_none() {
                return Err(StorageError::ObjectStoreNotFound { database, store });
            }
            IdbResponse::default()
        }
        IdbCommand::CreateIndex {
            database,
            store,
            definition,
        } => {
            let store_data = store_mut(databases, origin, &database, &store)?;
            if definition.unique {
                let mut seen = BTreeSet::new();
                for record in store_data.records.values() {
                    for key in indexed_record_keys(&record.value, &definition) {
                        if !seen.insert(encode_key(&key)?) {
                            return Err(StorageError::Constraint);
                        }
                    }
                }
            }
            store_data
                .indexes
                .insert(definition.name.clone(), definition);
            IdbResponse::default()
        }
        IdbCommand::DeleteIndex {
            database,
            store,
            index,
        } => {
            let store_data = store_mut(databases, origin, &database, &store)?;
            if store_data.indexes.remove(&index).is_none() {
                return Err(StorageError::IndexNotFound { store, index });
            }
            IdbResponse::default()
        }
        IdbCommand::RenameIndex {
            database,
            store,
            index,
            new_name,
        } => {
            let store_data = store_mut(databases, origin, &database, &store)?;
            if new_name != index && store_data.indexes.contains_key(&new_name) {
                return Err(StorageError::Constraint);
            }
            let mut definition =
                store_data
                    .indexes
                    .remove(&index)
                    .ok_or_else(|| StorageError::IndexNotFound {
                        store: store.clone(),
                        index: index.clone(),
                    })?;
            definition.name = new_name.clone();
            store_data.indexes.insert(new_name, definition);
            IdbResponse::default()
        }
        IdbCommand::Batch { database, writes } => {
            let before = databases.clone();
            let mut last_key = None;
            for write in writes {
                let store_data = store_mut(databases, origin, &database, &write.store)?;
                last_key = Some(put_record(store_data, write)?);
            }
            if indexed_db_origin_bytes(databases, origin)? > limit {
                *databases = before;
                return Err(StorageError::QuotaExceeded {
                    origin: origin.to_string(),
                    limit,
                });
            }
            IdbResponse {
                key: last_key,
                ..IdbResponse::default()
            }
        }
        IdbCommand::Get {
            database,
            store,
            key,
        } => {
            let store_data = store_ref(databases, origin, &database, &store)?;
            let encoded = encode_key(&key)?;
            IdbResponse {
                value: store_data
                    .records
                    .get(&encoded)
                    .map(|record| record.value.clone()),
                ..IdbResponse::default()
            }
        }
        IdbCommand::GetAll { database, store } => {
            let store_data = store_ref(databases, origin, &database, &store)?;
            let mut records = store_data.records.values().collect::<Vec<_>>();
            records.sort_by(|left, right| compare_indexed_db_keys(&left.key, &right.key));
            IdbResponse {
                values: records
                    .into_iter()
                    .map(|record| record.value.clone())
                    .collect(),
                ..IdbResponse::default()
            }
        }
        IdbCommand::Delete {
            database,
            store,
            key,
        } => {
            let store_data = store_mut(databases, origin, &database, &store)?;
            if key
                .get("__oryn_key_range")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                store_data
                    .records
                    .retain(|_, record| !indexed_db_key_matches(&key, &record.key));
            } else {
                store_data.records.remove(&encode_key(&key)?);
            }
            IdbResponse::default()
        }
        IdbCommand::Clear { database, store } => {
            store_mut(databases, origin, &database, &store)?
                .records
                .clear();
            IdbResponse::default()
        }
        IdbCommand::Count { database, store } => {
            let count = store_ref(databases, origin, &database, &store)?
                .records
                .len() as u64;
            IdbResponse {
                count: Some(count),
                ..IdbResponse::default()
            }
        }
        IdbCommand::Cursor {
            database,
            store,
            index,
            query,
            reverse,
        } => {
            let store_data = store_ref(databases, origin, &database, &store)?;
            let index_definition = index
                .as_ref()
                .map(|name| {
                    store_data
                        .indexes
                        .get(name)
                        .ok_or_else(|| StorageError::IndexNotFound {
                            store: store.clone(),
                            index: name.clone(),
                        })
                })
                .transpose()?;
            let mut records = store_data
                .records
                .values()
                .flat_map(|record| {
                    let indexed_key = index_definition.and_then(|definition| {
                        json_value_at_key_path(&record.value, &definition.key_path)
                    });
                    let mut cursor_keys = match (index_definition, indexed_key) {
                        (Some(definition), Some(serde_json::Value::Array(values)))
                            if definition.multi_entry =>
                        {
                            values.clone()
                        }
                        (_, Some(value)) => vec![value.clone()],
                        (Some(_), None) => Vec::new(),
                        (None, None) => vec![record.key.clone()],
                    };
                    cursor_keys.retain(|key| encode_key(key).is_ok());
                    cursor_keys.sort_by(compare_indexed_db_keys);
                    cursor_keys
                        .dedup_by(|left, right| compare_indexed_db_keys(left, right).is_eq());
                    cursor_keys.into_iter().filter_map(|cursor_key| {
                        query
                            .as_ref()
                            .is_none_or(|query| indexed_db_key_matches(query, &cursor_key))
                            .then(|| IdbRecord {
                                key: cursor_key,
                                primary_key: record.key.clone(),
                                value: record.value.clone(),
                            })
                    })
                })
                .collect::<Vec<_>>();
            records.sort_by(|left, right| {
                compare_indexed_db_keys(&left.key, &right.key)
                    .then_with(|| compare_indexed_db_keys(&left.primary_key, &right.primary_key))
            });
            if reverse {
                records.reverse();
            }
            IdbResponse {
                records,
                ..IdbResponse::default()
            }
        }
    };
    if indexed_db_origin_bytes(databases, origin)? > limit {
        return Err(StorageError::QuotaExceeded {
            origin: origin.to_string(),
            limit,
        });
    }
    Ok(response)
}

fn indexed_db_key_matches(query: &serde_json::Value, key: &serde_json::Value) -> bool {
    let Some(range) = query.as_object().filter(|range| {
        range
            .get("__oryn_key_range")
            .and_then(|value| value.as_bool())
            == Some(true)
    }) else {
        return query == key;
    };
    let lower_matches = range
        .get("lower")
        .filter(|value| !value.is_null())
        .is_none_or(|lower| {
            let ordering = compare_indexed_db_keys(key, lower);
            if range
                .get("lower_open")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                ordering.is_gt()
            } else {
                ordering.is_ge()
            }
        });
    let upper_matches = range
        .get("upper")
        .filter(|value| !value.is_null())
        .is_none_or(|upper| {
            let ordering = compare_indexed_db_keys(key, upper);
            if range
                .get("upper_open")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                ordering.is_lt()
            } else {
                ordering.is_le()
            }
        });
    lower_matches && upper_matches
}

fn compare_indexed_db_keys(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (serde_json::Value::Number(left), serde_json::Value::Number(right)) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(Ordering::Equal),
        (serde_json::Value::String(left), serde_json::Value::String(right)) => left.cmp(right),
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            for (left, right) in left.iter().zip(right) {
                let ordering = compare_indexed_db_keys(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        (serde_json::Value::Object(left), serde_json::Value::Object(right))
            if left
                .get("__oryn_idb_key_type")
                .and_then(|value| value.as_str())
                == Some("binary")
                && right
                    .get("__oryn_idb_key_type")
                    .and_then(|value| value.as_str())
                    == Some("binary") =>
        {
            let bytes = |value: &serde_json::Map<String, serde_json::Value>| {
                value
                    .get("bytes")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_u64().map(|value| value as u8))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            bytes(left).cmp(&bytes(right))
        }
        _ => encode_key(left)
            .unwrap_or_default()
            .cmp(&encode_key(right).unwrap_or_default()),
    }
}

fn database_mut<'a>(
    databases: &'a mut BTreeMap<(String, String), DatabaseData>,
    origin: &str,
    database: &str,
) -> Result<&'a mut DatabaseData, StorageError> {
    databases
        .get_mut(&(origin.to_string(), database.to_string()))
        .ok_or_else(|| StorageError::DatabaseNotFound(database.to_string()))
}

fn store_mut<'a>(
    databases: &'a mut BTreeMap<(String, String), DatabaseData>,
    origin: &str,
    database: &str,
    store: &str,
) -> Result<&'a mut ObjectStoreData, StorageError> {
    database_mut(databases, origin, database)?
        .stores
        .get_mut(store)
        .ok_or_else(|| StorageError::ObjectStoreNotFound {
            database: database.to_string(),
            store: store.to_string(),
        })
}

fn store_ref<'a>(
    databases: &'a BTreeMap<(String, String), DatabaseData>,
    origin: &str,
    database: &str,
    store: &str,
) -> Result<&'a ObjectStoreData, StorageError> {
    databases
        .get(&(origin.to_string(), database.to_string()))
        .ok_or_else(|| StorageError::DatabaseNotFound(database.to_string()))?
        .stores
        .get(store)
        .ok_or_else(|| StorageError::ObjectStoreNotFound {
            database: database.to_string(),
            store: store.to_string(),
        })
}

fn indexed_record_keys(
    value: &serde_json::Value,
    definition: &IndexDefinition,
) -> Vec<serde_json::Value> {
    match json_value_at_key_path(value, &definition.key_path) {
        Some(serde_json::Value::Array(values)) if definition.multi_entry => values
            .into_iter()
            .filter(|key| encode_key(key).is_ok())
            .collect(),
        Some(key) if encode_key(&key).is_ok() => vec![key],
        _ => Vec::new(),
    }
}

fn put_record(
    store: &mut ObjectStoreData,
    write: IdbWrite,
) -> Result<serde_json::Value, StorageError> {
    let definition = store.definition.clone().unwrap_or(ObjectStoreDefinition {
        name: write.store,
        key_path: None,
        auto_increment: false,
        indexes: Vec::new(),
    });
    let mut value = write.value;
    let key_from_path = definition
        .key_path
        .as_deref()
        .and_then(|path| json_value_at_key_path(&value, path));
    let key = write.key.or(key_from_path).or_else(|| {
        definition.auto_increment.then(|| {
            let key = serde_json::Value::from(store.next_key.max(1));
            store.next_key = store.next_key.max(1).saturating_add(1);
            key
        })
    });
    let key = key.ok_or(StorageError::KeyRequired)?;
    if definition.auto_increment
        && let Some(numeric) = key.as_f64()
        && numeric.is_finite()
        && numeric >= 1.0
    {
        let next = numeric.floor().min(u64::MAX as f64) as u64;
        store.next_key = store.next_key.max(next.saturating_add(1));
    }
    if let Some(path) = definition.key_path
        && json_value_at_key_path(&value, &path).is_none()
    {
        inject_json_key_path(&mut value, &path, key.clone())?;
    }
    let encoded = encode_key(&key)?;
    if write.add_only && store.records.contains_key(&encoded) {
        return Err(StorageError::Constraint);
    }
    for index in store.indexes.values().filter(|index| index.unique) {
        let Some(index_value) = json_value_at_key_path(&value, &index.key_path) else {
            continue;
        };
        if store.records.iter().any(|(record_key, record)| {
            record_key != &encoded
                && json_value_at_key_path(&record.value, &index.key_path).as_ref()
                    == Some(&index_value)
        }) {
            return Err(StorageError::Constraint);
        }
    }
    store.records.insert(
        encoded,
        RecordData {
            key: key.clone(),
            value,
        },
    );
    Ok(key)
}

const COMPOUND_KEY_PATH_PREFIX: &str = "__oryn_compound_key_path__:";

fn json_value_at_key_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    if let Some(encoded) = path.strip_prefix(COMPOUND_KEY_PATH_PREFIX) {
        let paths = serde_json::from_str::<Vec<String>>(encoded).ok()?;
        return paths
            .iter()
            .map(|path| json_value_at_key_path(value, path))
            .collect::<Option<Vec<_>>>()
            .map(serde_json::Value::Array);
    }
    if path.is_empty() {
        return Some(value.clone());
    }
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
        .cloned()
}

fn inject_json_key_path(
    value: &mut serde_json::Value,
    path: &str,
    key: serde_json::Value,
) -> Result<(), StorageError> {
    let mut segments = path.split('.').peekable();
    let mut current = value;
    while let Some(segment) = segments.next() {
        let object = current.as_object_mut().ok_or(StorageError::KeyRequired)?;
        if segments.peek().is_none() {
            object.insert(segment.to_string(), key);
            return Ok(());
        }
        current = object
            .entry(segment.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !current.is_object() {
            return Err(StorageError::KeyRequired);
        }
    }
    Err(StorageError::KeyRequired)
}

fn encode_key(key: &serde_json::Value) -> Result<String, StorageError> {
    match key {
        serde_json::Value::Number(value) => {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| StorageError::DataClone("invalid IndexedDB number key".into()))?;
            Ok(format!("n:{:016x}", value.to_bits()))
        }
        serde_json::Value::String(value) => serde_json::to_string(value)
            .map(|value| format!("s:{value}"))
            .map_err(|error| StorageError::DataClone(error.to_string())),
        serde_json::Value::Array(values) => {
            let encoded = values
                .iter()
                .map(encode_key)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_string(&encoded)
                .map(|encoded| format!("a:{encoded}"))
                .map_err(|error| StorageError::DataClone(error.to_string()))
        }
        serde_json::Value::Object(value)
            if value
                .get("__oryn_idb_key_type")
                .and_then(|value| value.as_str())
                == Some("binary") =>
        {
            let bytes = value
                .get("bytes")
                .and_then(|value| value.as_array())
                .ok_or_else(|| StorageError::DataClone("invalid binary IndexedDB key".into()))?;
            let mut encoded = String::from("b:");
            for byte in bytes {
                let byte = byte.as_u64().filter(|value| *value <= 255).ok_or_else(|| {
                    StorageError::DataClone("invalid binary IndexedDB key".into())
                })?;
                use std::fmt::Write as _;
                write!(&mut encoded, "{byte:02x}")
                    .map_err(|error| StorageError::DataClone(error.to_string()))?;
            }
            Ok(encoded)
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Object(_) => {
            Err(StorageError::DataClone("invalid IndexedDB key".into()))
        }
    }
}

fn indexed_db_origin_bytes(
    databases: &BTreeMap<(String, String), DatabaseData>,
    origin: &str,
) -> Result<usize, StorageError> {
    databases
        .iter()
        .filter(|((item_origin, _), _)| item_origin == origin)
        .try_fold(0usize, |total, (_, database)| {
            serde_json::to_vec(database)
                .map(|value| total.saturating_add(value.len()))
                .map_err(|error| StorageError::DataClone(error.to_string()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(page: u64) -> StorageScope {
        StorageScope {
            origin: "https://example.test".into(),
            top_level_page_id: PageId(page),
        }
    }

    #[test]
    fn local_is_shared_and_session_is_page_scoped() {
        let store = SharedContextStore::memory(StorageLimits::default());
        store
            .execute(StorageRequest::Set {
                area: StorageArea::Local,
                scope: scope(1),
                key: "theme".into(),
                value: "dark".into(),
            })
            .unwrap();
        let value = store
            .execute(StorageRequest::Get {
                area: StorageArea::Local,
                scope: scope(2),
                key: "theme".into(),
            })
            .unwrap();
        assert!(matches!(value, StorageResponse::Value { value: Some(value) } if value == "dark"));

        store
            .execute(StorageRequest::Set {
                area: StorageArea::Session,
                scope: scope(1),
                key: "step".into(),
                value: "shipping".into(),
            })
            .unwrap();
        assert!(matches!(
            store
                .execute(StorageRequest::Get {
                    area: StorageArea::Session,
                    scope: scope(2),
                    key: "step".into(),
                })
                .unwrap(),
            StorageResponse::Value { value: None }
        ));
    }

    #[test]
    fn indexed_db_batches_are_atomic_and_enforce_add() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Open {
                    name: "app".into(),
                    version: 1,
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateObjectStore {
                    database: "app".into(),
                    definition: ObjectStoreDefinition {
                        name: "items".into(),
                        key_path: Some("id".into()),
                        auto_increment: true,
                        indexes: Vec::new(),
                    },
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Batch {
                    database: "app".into(),
                    writes: vec![IdbWrite {
                        store: "items".into(),
                        key: None,
                        value: serde_json::json!({"name":"first"}),
                        add_only: true,
                    }],
                },
            })
            .unwrap();
        let values = store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::GetAll {
                    database: "app".into(),
                    store: "items".into(),
                },
            })
            .unwrap();
        assert!(
            matches!(values, StorageResponse::IndexedDb { response } if response.values.len() == 1)
        );
    }

    #[test]
    fn indexed_db_transactions_commit_or_restore_the_parent_snapshot() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        for command in [
            IdbCommand::Open {
                name: "transactions".into(),
                version: 1,
            },
            IdbCommand::CreateObjectStore {
                database: "transactions".into(),
                definition: ObjectStoreDefinition {
                    name: "items".into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            },
            IdbCommand::CommitUpgrade {
                name: "transactions".into(),
            },
        ] {
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command,
                })
                .unwrap();
        }
        let write = |token: &str, value: &str| IdbCommand::Transaction {
            token: token.into(),
            database: "transactions".into(),
            inner: Box::new(IdbCommand::Batch {
                database: "transactions".into(),
                writes: vec![IdbWrite {
                    store: "items".into(),
                    key: Some(serde_json::json!(1)),
                    value: serde_json::json!(value),
                    add_only: false,
                }],
            }),
        };
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: write("abort", "discarded"),
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::AbortTransaction {
                    token: "abort".into(),
                    database: "transactions".into(),
                },
            })
            .unwrap();
        let read = || {
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command: IdbCommand::Get {
                        database: "transactions".into(),
                        store: "items".into(),
                        key: serde_json::json!(1),
                    },
                })
                .unwrap()
        };
        assert!(matches!(
            read(),
            StorageResponse::IndexedDb { response } if response.value.is_none()
        ));

        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: write("commit", "retained"),
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CommitTransaction {
                    token: "commit".into(),
                    database: "transactions".into(),
                },
            })
            .unwrap();
        assert!(matches!(
            read(),
            StorageResponse::IndexedDb { response }
                if response.value == Some(serde_json::json!("retained"))
        ));
    }

    #[test]
    fn indexed_db_explicit_numeric_keys_advance_the_auto_increment_generator() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Open {
                    name: "keys".into(),
                    version: 1,
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateObjectStore {
                    database: "keys".into(),
                    definition: ObjectStoreDefinition {
                        name: "items".into(),
                        key_path: Some("id".into()),
                        auto_increment: true,
                        indexes: Vec::new(),
                    },
                },
            })
            .unwrap();
        let write = |value| StorageRequest::IndexedDb {
            origin: origin.clone(),
            command: IdbCommand::Batch {
                database: "keys".into(),
                writes: vec![IdbWrite {
                    store: "items".into(),
                    key: None,
                    value,
                    add_only: false,
                }],
            },
        };
        store.execute(write(serde_json::json!({"id": 8}))).unwrap();
        let generated = store
            .execute(write(serde_json::json!({"name": "generated"})))
            .unwrap();
        assert!(matches!(
            generated,
            StorageResponse::IndexedDb { response }
                if response.key == Some(serde_json::json!(9))
        ));
    }

    #[test]
    fn indexed_db_open_restores_object_store_definitions() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Open {
                    name: "metadata".into(),
                    version: 1,
                },
            })
            .unwrap();
        let mut definition = ObjectStoreDefinition {
            name: "items".into(),
            key_path: Some("id".into()),
            auto_increment: true,
            indexes: Vec::new(),
        };
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateObjectStore {
                    database: "metadata".into(),
                    definition: definition.clone(),
                },
            })
            .unwrap();
        let index = IndexDefinition {
            name: "by_name".into(),
            key_path: "name".into(),
            unique: false,
            multi_entry: false,
        };
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateIndex {
                    database: "metadata".into(),
                    store: "items".into(),
                    definition: index.clone(),
                },
            })
            .unwrap();
        definition.indexes.push(index);
        let reopened = store
            .execute(StorageRequest::IndexedDb {
                origin,
                command: IdbCommand::Open {
                    name: "metadata".into(),
                    version: 1,
                },
            })
            .unwrap();
        assert!(matches!(
            reopened,
            StorageResponse::IndexedDb { response }
                if response.object_store_definitions == vec![definition]
        ));
        store
            .execute(StorageRequest::IndexedDb {
                origin: "https://example.test".into(),
                command: IdbCommand::DeleteIndex {
                    database: "metadata".into(),
                    store: "items".into(),
                    index: "by_name".into(),
                },
            })
            .unwrap();
        let reopened = store
            .execute(StorageRequest::IndexedDb {
                origin: "https://example.test".into(),
                command: IdbCommand::Open {
                    name: "metadata".into(),
                    version: 1,
                },
            })
            .unwrap();
        assert!(matches!(
            reopened,
            StorageResponse::IndexedDb { response }
                if response.object_store_definitions[0].indexes.is_empty()
        ));
    }

    #[test]
    fn indexed_db_aborted_upgrade_restores_schema_and_version() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        for version in [1, 2] {
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command: IdbCommand::Open {
                        name: "rollback".into(),
                        version,
                    },
                })
                .unwrap();
            if version == 1 {
                store
                    .execute(StorageRequest::IndexedDb {
                        origin: origin.clone(),
                        command: IdbCommand::CommitUpgrade {
                            name: "rollback".into(),
                        },
                    })
                    .unwrap();
            }
        }
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateObjectStore {
                    database: "rollback".into(),
                    definition: ObjectStoreDefinition {
                        name: "transient".into(),
                        key_path: None,
                        auto_increment: false,
                        indexes: Vec::new(),
                    },
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::AbortUpgrade {
                    name: "rollback".into(),
                    old_version: 1,
                },
            })
            .unwrap();
        let reopened = store
            .execute(StorageRequest::IndexedDb {
                origin,
                command: IdbCommand::Open {
                    name: "rollback".into(),
                    version: 1,
                },
            })
            .unwrap();
        assert!(matches!(
            reopened,
            StorageResponse::IndexedDb { response }
                if response.version == Some(1) && response.object_stores.is_empty()
        ));
    }

    #[test]
    fn indexed_db_database_listing_is_partitioned_by_origin() {
        let store = SharedContextStore::memory(StorageLimits::default());
        for (origin, name, version) in [
            ("https://one.test", "first", 1),
            ("https://one.test", "second", 2),
            ("https://two.test", "private", 4),
        ] {
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.into(),
                    command: IdbCommand::Open {
                        name: name.into(),
                        version,
                    },
                })
                .unwrap();
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.into(),
                    command: IdbCommand::CommitUpgrade { name: name.into() },
                })
                .unwrap();
        }
        let listed = store
            .execute(StorageRequest::IndexedDb {
                origin: "https://one.test".into(),
                command: IdbCommand::ListDatabases,
            })
            .unwrap();
        assert!(matches!(
            listed,
            StorageResponse::IndexedDb { response }
                if response.databases == vec![
                    DatabaseInfo { name: "first".into(), version: 1 },
                    DatabaseInfo { name: "second".into(), version: 2 },
                ]
        ));
    }

    #[test]
    fn indexed_db_auto_increment_injects_nested_key_paths() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = "https://example.test".to_string();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Open {
                    name: "nested".into(),
                    version: 1,
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CreateObjectStore {
                    database: "nested".into(),
                    definition: ObjectStoreDefinition {
                        name: "items".into(),
                        key_path: Some("a.b.c".into()),
                        auto_increment: true,
                        indexes: Vec::new(),
                    },
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Batch {
                    database: "nested".into(),
                    writes: vec![IdbWrite {
                        store: "items".into(),
                        key: None,
                        value: serde_json::json!({}),
                        add_only: false,
                    }],
                },
            })
            .unwrap();
        let value = store
            .execute(StorageRequest::IndexedDb {
                origin,
                command: IdbCommand::Get {
                    database: "nested".into(),
                    store: "items".into(),
                    key: serde_json::json!(1),
                },
            })
            .unwrap();
        assert!(matches!(
            value,
            StorageResponse::IndexedDb { response }
                if response.value == Some(serde_json::json!({"a":{"b":{"c":1}}}))
        ));
    }

    #[test]
    fn storage_quotas_are_origin_scoped_and_failed_writes_are_atomic() {
        let store = SharedContextStore::memory(StorageLimits {
            web_storage_bytes_per_origin: 16,
            indexed_db_bytes_per_origin: 128,
        });
        store
            .execute(StorageRequest::Set {
                area: StorageArea::Local,
                scope: scope(1),
                key: "ok".into(),
                value: "small".into(),
            })
            .expect("small local value");
        let error = store
            .execute(StorageRequest::Set {
                area: StorageArea::Local,
                scope: scope(1),
                key: "large".into(),
                value: "x".repeat(64),
            })
            .expect_err("oversized local value");
        assert!(matches!(
            error,
            StorageError::QuotaExceeded { limit: 16, .. }
        ));
        assert!(matches!(
            store
                .execute(StorageRequest::Get {
                    area: StorageArea::Local,
                    scope: scope(1),
                    key: "ok".into(),
                })
                .unwrap(),
            StorageResponse::Value { value: Some(value) } if value == "small"
        ));
        assert!(matches!(
            store
                .execute(StorageRequest::Get {
                    area: StorageArea::Local,
                    scope: StorageScope {
                        origin: "https://other.example.test".into(),
                        top_level_page_id: PageId(1),
                    },
                    key: "ok".into(),
                })
                .unwrap(),
            StorageResponse::Value { value: None }
        ));
    }

    #[test]
    fn persistent_profiles_are_isolated_and_session_storage_is_not_persisted() {
        let root = std::env::temp_dir().join(format!(
            "oryn-profile-isolation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let first_path = root.join("first");
        let second_path = root.join("second");
        {
            let first = PersistentProfile::open(&first_path).unwrap();
            let store = SharedContextStore::persistent(&first, StorageLimits::default());
            store
                .execute(StorageRequest::Set {
                    area: StorageArea::Local,
                    scope: scope(1),
                    key: "profile".into(),
                    value: "first".into(),
                })
                .unwrap();
            store
                .execute(StorageRequest::Set {
                    area: StorageArea::Session,
                    scope: scope(1),
                    key: "transient".into(),
                    value: "discard".into(),
                })
                .unwrap();
        }
        {
            let first = PersistentProfile::open(&first_path).unwrap();
            let store = SharedContextStore::persistent(&first, StorageLimits::default());
            assert!(matches!(
                store.execute(StorageRequest::Get {
                    area: StorageArea::Local,
                    scope: scope(9),
                    key: "profile".into(),
                }).unwrap(),
                StorageResponse::Value { value: Some(value) } if value == "first"
            ));
            assert!(matches!(
                store
                    .execute(StorageRequest::Get {
                        area: StorageArea::Session,
                        scope: scope(1),
                        key: "transient".into(),
                    })
                    .unwrap(),
                StorageResponse::Value { value: None }
            ));
            let second = PersistentProfile::open(&second_path).unwrap();
            let other = SharedContextStore::persistent(&second, StorageLimits::default());
            assert!(matches!(
                other
                    .execute(StorageRequest::Get {
                        area: StorageArea::Local,
                        scope: scope(1),
                        key: "profile".into(),
                    })
                    .unwrap(),
                StorageResponse::Value { value: None }
            ));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_local_storage_survives_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-storage-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("thread")
        ));
        let _ = std::fs::remove_dir_all(&directory);
        {
            let profile = PersistentProfile::open(&directory).unwrap();
            let store = SharedContextStore::persistent(&profile, StorageLimits::default());
            store
                .execute(StorageRequest::Set {
                    area: StorageArea::Local,
                    scope: scope(1),
                    key: "token".into(),
                    value: "persisted".into(),
                })
                .unwrap();
            store
                .execute(StorageRequest::IndexedDb {
                    origin: "https://example.test".into(),
                    command: IdbCommand::Open {
                        name: "profile".into(),
                        version: 1,
                    },
                })
                .unwrap();
            store
                .execute(StorageRequest::IndexedDb {
                    origin: "https://example.test".into(),
                    command: IdbCommand::CreateObjectStore {
                        database: "profile".into(),
                        definition: ObjectStoreDefinition {
                            name: "tokens".into(),
                            key_path: None,
                            auto_increment: false,
                            indexes: Vec::new(),
                        },
                    },
                })
                .unwrap();
            store
                .execute(StorageRequest::IndexedDb {
                    origin: "https://example.test".into(),
                    command: IdbCommand::Batch {
                        database: "profile".into(),
                        writes: vec![IdbWrite {
                            store: "tokens".into(),
                            key: Some(serde_json::json!("oauth")),
                            value: serde_json::json!({"state":"persisted"}),
                            add_only: false,
                        }],
                    },
                })
                .unwrap();
            let url = url::Url::parse("https://example.test/login").unwrap();
            profile
                .cookie_store()
                .store_response(
                    &url,
                    &crate::cookies::CookieRequestContext::navigation(Some(url.to_string())),
                    ["session=persisted; Path=/; HttpOnly; Secure; Max-Age=3600".into()],
                )
                .unwrap();
        }
        {
            let profile = PersistentProfile::open(&directory).unwrap();
            let store = SharedContextStore::persistent(&profile, StorageLimits::default());
            assert!(matches!(
                store
                    .execute(StorageRequest::Get {
                        area: StorageArea::Local,
                        scope: scope(9),
                        key: "token".into(),
                    })
                    .unwrap(),
                StorageResponse::Value { value: Some(value) } if value == "persisted"
            ));
            assert_eq!(
                profile
                    .cookie_store()
                    .header(
                        &url::Url::parse("https://example.test/account").unwrap(),
                        &crate::cookies::CookieRequestContext::navigation(Some(
                            "https://example.test/account".into(),
                        )),
                    )
                    .0
                    .as_deref(),
                Some("session=persisted")
            );
            assert!(matches!(
                store
                    .execute(StorageRequest::IndexedDb {
                        origin: "https://example.test".into(),
                        command: IdbCommand::Get {
                            database: "profile".into(),
                            store: "tokens".into(),
                            key: serde_json::json!("oauth"),
                        },
                    })
                    .unwrap(),
                StorageResponse::IndexedDb { response }
                    if response.value == Some(serde_json::json!({"state":"persisted"}))
            ));
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn indexed_db_key_ranges_cover_numeric_string_and_compound_keys() {
        assert_eq!(
            encode_key(&serde_json::json!(1)).unwrap(),
            encode_key(&serde_json::Value::Number(
                serde_json::Number::from_f64(1.0).unwrap()
            ))
            .unwrap()
        );
        let range = serde_json::json!({
            "__oryn_key_range": true,
            "lower": 2,
            "upper": 5,
            "lower_open": true,
            "upper_open": false
        });
        assert!(!indexed_db_key_matches(&range, &serde_json::json!(2)));
        assert!(indexed_db_key_matches(&range, &serde_json::json!(3)));
        assert!(indexed_db_key_matches(&range, &serde_json::json!(5)));
        assert!(!indexed_db_key_matches(&range, &serde_json::json!(6)));

        let compound = serde_json::json!({
            "__oryn_key_range": true,
            "lower": ["a", 1],
            "upper": ["a", 3]
        });
        assert!(indexed_db_key_matches(
            &compound,
            &serde_json::json!(["a", 2])
        ));
        assert!(!indexed_db_key_matches(
            &compound,
            &serde_json::json!(["b", 1])
        ));
    }
}
