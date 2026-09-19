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
    Unchanged,
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

/// A bounded cursor request. Position pairs use index key then primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbCursorStep {
    pub after: Option<(serde_json::Value, serde_json::Value)>,
    pub target: Option<(serde_json::Value, Option<serde_json::Value>)>,
    pub advance: u32,
    pub unique: bool,
    pub with_value: bool,
}

#[path = "storage_idb_connections.rs"]
mod idb_connections;
/// Transaction correlations are scoped to the native caller, independently of
/// the shared database bucket. Never trust a script-provided token as ownership.
pub(crate) fn scope_idb_transaction(command: &mut IdbCommand, principal: &str) {
    let connection = match command {
        IdbCommand::OpenConnection { token, .. }
        | IdbCommand::UpgradeConnection { token, .. }
        | IdbCommand::DeleteConnection { token, .. }
        | IdbCommand::CloseConnection { token, .. }
        | IdbCommand::RetireConnection { token, .. }
        | IdbCommand::PollConnection { token, .. }
        | IdbCommand::AcknowledgeConnection { token, .. } => Some(token),
        IdbCommand::BeginTransaction { connection, .. } => connection.as_mut(),
        _ => None,
    };
    if let Some(token) = connection {
        *token = serde_json::to_string(&("oryn:idb-connection:v1", principal, token.as_str()))
            .expect("string tuple serialization");
    }
    match command {
        IdbCommand::BeginTransaction { token, .. }
        | IdbCommand::Transaction { token, .. }
        | IdbCommand::CommitTransaction { token, .. }
        | IdbCommand::AbortTransaction { token, .. } => {
            *token = serde_json::to_string(&("oryn:idb-transaction:v1", principal, token.as_str()))
                .expect("string tuple serialization");
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IdbCommand {
    BeginTransaction {
        token: String,
        database: String,
        scope: Vec<String>,
        mode: String,
        #[serde(default)]
        connection: Option<String>,
    },
    OpenConnection {
        token: String,
        name: String,
        version: u64,
    },
    UpgradeConnection {
        token: String,
        name: String,
        inner: Box<IdbCommand>,
    },
    DeleteConnection {
        token: String,
        name: String,
    },
    CloseConnection {
        token: String,
        name: String,
    },
    RetireConnection {
        token: String,
        name: String,
    },
    PollConnection {
        token: String,
        name: String,
    },
    AcknowledgeConnection {
        token: String,
        name: String,
        event: u64,
    },
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
    RenameObjectStore {
        database: String,
        store: String,
        new_name: String,
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
    CursorStep {
        database: String,
        store: String,
        index: Option<String>,
        query: Option<serde_json::Value>,
        reverse: bool,
        step: IdbCursorStep,
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
    #[serde(default)]
    pub transaction_blocked: bool,
    #[serde(default)]
    pub connection_blocked: bool,
    #[serde(default)]
    pub connection_managed: bool,
    #[serde(default)]
    pub connection_events: Vec<oryn_js::platform::IdbConnectionEvent>,
    pub databases: Vec<DatabaseInfo>,
    pub version: Option<u64>,
    pub old_version: Option<u64>,
    pub upgrade_needed: bool,
    pub object_stores: Vec<String>,
    pub object_store_definitions: Vec<ObjectStoreDefinition>,
    pub key: Option<serde_json::Value>,
    pub value: Option<serde_json::Value>,
    /// Distinguishes a stored JSON null from an absent record across serde,
    /// where `Option<Value>` otherwise maps both states to JSON null.
    pub value_present: bool,
    pub values: Vec<serde_json::Value>,
    pub records: Vec<IdbRecord>,
    pub count: Option<u64>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageError {
    #[error("storage notification admission failed: {0}")]
    NotificationCapacity(String),
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

#[path = "storage_cache.rs"]
pub(crate) mod cache;
#[path = "storage_service.rs"]
mod service;
pub use cache::CacheCoordinator;
pub use service::{ServiceStorageKind, ServiceStorageRequest, ServiceStorageSnapshot};
#[path = "storage_service_versions.rs"]
mod service_versions;
pub(crate) use service_versions::{
    ServiceVersionClaim, ServiceVersionDirectory, ServiceVersionObservation,
    ServiceVersionReservation,
};
#[path = "storage_service_jobs.rs"]
mod service_jobs;
pub use service_jobs::ServiceJobCoordinator;
pub(crate) use service_jobs::{
    ServiceActivityLease, ServiceClientLease, ServiceJobLease, ServiceScopeWatch,
};

#[cfg(test)]
thread_local! {static FAIL_WEB_STORAGE_COMMIT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

pub trait ContextStore: Send + Sync {
    fn service_job_coordinator(&self) -> Option<Arc<ServiceJobCoordinator>> {
        None
    }
    fn cache_coordinator(&self) -> Option<Arc<Mutex<CacheCoordinator>>> {
        None
    }
    /// Parent-only native service storage. Never routed by StorageRequest.
    fn service_storage(
        &self,
        _request: ServiceStorageRequest,
    ) -> Result<ServiceStorageSnapshot, StorageError> {
        Err(StorageError::Persistent(
            "store does not support service persistence".into(),
        ))
    }
    fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError> {
        self.execute_with_admission(request, &mut |_| Ok(()))
    }
    /// Admit the exact notification while the storage transaction is private.
    /// Rejection must leave its old state intact; no callback runs for reads or
    /// unchanged writes. The caller publishes admitted custody after success.
    fn execute_with_admission(
        &self,
        request: StorageRequest,
        admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
    ) -> Result<StorageResponse, StorageError>;
}

#[derive(Clone)]
pub struct SharedContextStore(Arc<dyn ContextStore>);

impl SharedContextStore {
    pub(crate) fn service_coordinator(&self) -> Result<Arc<ServiceJobCoordinator>, StorageError> {
        self.0.service_job_coordinator().ok_or_else(|| {
            StorageError::Persistent("store lacks service ownership coordinator".into())
        })
    }

    pub(crate) fn service_job(
        &self,
        partition: &str,
        origin: &str,
        scope: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<ServiceJobLease, StorageError> {
        self.0
            .service_job_coordinator()
            .ok_or_else(|| StorageError::Persistent("store lacks service job coordinator".into()))?
            .acquire(partition, origin, scope, cancel)
    }

    pub(crate) fn service_storage(
        &self,
        request: ServiceStorageRequest,
    ) -> Result<ServiceStorageSnapshot, StorageError> {
        self.0.service_storage(request)
    }
    pub(crate) fn cache_operation(
        &self,
        owner: cache::CacheLeaseOwner,
        partition: &str,
        origin: &str,
        operation: cache::CacheOperation,
    ) -> Result<cache::CacheResult, StorageError> {
        let coordinator = self.0.cache_coordinator().ok_or_else(|| {
            StorageError::Persistent("store has no cache ownership coordinator".into())
        })?;
        cache::execute_owned(self, &coordinator, owner, partition, origin, operation)
    }
    pub(crate) fn release_caches(&self, owner: cache::CacheLeaseOwner) -> Result<(), StorageError> {
        let coordinator = self.0.cache_coordinator().ok_or_else(|| {
            StorageError::Persistent("store has no cache ownership coordinator".into())
        })?;
        cache::release_owner(self, &coordinator, owner)
    }
    pub fn memory(limits: StorageLimits) -> Self {
        Self(Arc::new(MemoryContextStore::new(limits)))
    }

    pub fn persistent(profile: &PersistentProfile, limits: StorageLimits) -> Self {
        Self(Arc::new(PersistentContextStore::new(
            profile.database.clone(),
            profile.cache_coordinator.clone(),
            profile.service_jobs.clone(),
            profile.idb_mutations.clone(),
            profile.idb_transactions.clone(),
            profile.idb_connections.clone(),
            limits,
        )))
    }

    pub fn execute(&self, request: StorageRequest) -> Result<StorageResponse, StorageError> {
        self.0.execute(request)
    }
    pub(crate) fn execute_with_admission(
        &self,
        request: StorageRequest,
        admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
    ) -> Result<StorageResponse, StorageError> {
        self.0.execute_with_admission(request, admit)
    }
}

#[derive(Clone)]
pub struct PersistentProfile {
    path: PathBuf,
    database: Arc<Database>,
    cookies: Arc<ContextCookieStore>,
    cache_coordinator: Arc<Mutex<cache::CacheCoordinator>>,
    service_jobs: Arc<ServiceJobCoordinator>,
    idb_mutations: Arc<Mutex<()>>,
    idb_transactions: Arc<Mutex<BTreeMap<String, TransactionSnapshot>>>,
    idb_connections: Arc<Mutex<idb_connections::Connections>>,
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
            cache_coordinator: Arc::new(Mutex::new(Default::default())),
            service_jobs: Arc::new(ServiceJobCoordinator::default()),
            idb_mutations: Arc::new(Mutex::new(())),
            idb_transactions: Arc::new(Mutex::new(BTreeMap::new())),
            idb_connections: Arc::new(Mutex::new(Default::default())),
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
    service_records: BTreeMap<String, ServiceStorageSnapshot>,
    local: BTreeMap<(String, String), String>,
    session: BTreeMap<(PageId, String, String), String>,
    indexed_db: BTreeMap<(String, String), DatabaseData>,
    indexed_db_transactions: BTreeMap<String, TransactionSnapshot>,
    idb_connections: idb_connections::Connections,
}

struct MemoryContextStore {
    cache_coordinator: Arc<Mutex<cache::CacheCoordinator>>,
    service_jobs: Arc<ServiceJobCoordinator>,
    state: Mutex<MemoryState>,
    limits: StorageLimits,
}

impl MemoryContextStore {
    fn new(limits: StorageLimits) -> Self {
        Self {
            state: Mutex::new(MemoryState::default()),
            cache_coordinator: Arc::new(Mutex::new(Default::default())),
            service_jobs: Arc::new(ServiceJobCoordinator::default()),
            limits,
        }
    }
}

impl ContextStore for MemoryContextStore {
    fn service_job_coordinator(&self) -> Option<Arc<ServiceJobCoordinator>> {
        Some(self.service_jobs.clone())
    }
    fn cache_coordinator(&self) -> Option<Arc<Mutex<CacheCoordinator>>> {
        Some(self.cache_coordinator.clone())
    }
    fn service_storage(
        &self,
        request: ServiceStorageRequest,
    ) -> Result<ServiceStorageSnapshot, StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("memory store lock poisoned".into()))?;
        service::memory(&mut state.service_records, request)
    }
    fn execute_with_admission(
        &self,
        request: StorageRequest,
        admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
    ) -> Result<StorageResponse, StorageError> {
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
                if old_value.as_ref() == Some(&value) {
                    return Ok(StorageResponse::Unchanged);
                }
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
                let change = StorageChange {
                    area,
                    scope: scope.clone(),
                    key: Some(key.clone()),
                    old_value: old_value.clone(),
                    new_value: Some(value),
                };
                if let Err(error) = admit(&change) {
                    match old_value {
                        Some(old) => memory_set(&mut state, area, &scope, key, old),
                        None => memory_remove(&mut state, area, &scope, &key),
                    };
                    return Err(error);
                }
                Ok(StorageResponse::Changed { change })
            }
            StorageRequest::Remove { area, scope, key } => {
                let old_value = memory_get(&state, area, &scope, &key);
                if old_value.is_none() {
                    return Ok(StorageResponse::Unchanged);
                }
                admit(&StorageChange {
                    area,
                    scope: scope.clone(),
                    key: Some(key.clone()),
                    old_value: old_value.clone(),
                    new_value: None,
                })?;
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
                if memory_entries(&state, area, &scope).is_empty() {
                    return Ok(StorageResponse::Unchanged);
                }
                admit(&StorageChange {
                    area,
                    scope: scope.clone(),
                    key: None,
                    old_value: None,
                    new_value: None,
                })?;
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
                    idb_connections,
                    ..
                } = &mut *state;
                let response = execute_idb_coordinated(
                    indexed_db,
                    indexed_db_transactions,
                    idb_connections,
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
    cache_coordinator: Arc<Mutex<cache::CacheCoordinator>>,
    service_jobs: Arc<ServiceJobCoordinator>,
    database: Arc<Database>,
    session: Mutex<BTreeMap<(PageId, String, String), String>>,
    indexed_db_transactions: Arc<Mutex<BTreeMap<String, TransactionSnapshot>>>,
    limits: StorageLimits,
    idb_mutations: Arc<Mutex<()>>,
    #[cfg(test)]
    fail_next_idb_commit: std::sync::atomic::AtomicBool,
    idb_connections: Arc<Mutex<idb_connections::Connections>>,
}

impl PersistentContextStore {
    fn new(
        database: Arc<Database>,
        cache_coordinator: Arc<Mutex<cache::CacheCoordinator>>,
        service_jobs: Arc<ServiceJobCoordinator>,
        idb_mutations: Arc<Mutex<()>>,
        idb_transactions: Arc<Mutex<BTreeMap<String, TransactionSnapshot>>>,
        idb_connections: Arc<Mutex<idb_connections::Connections>>,
        limits: StorageLimits,
    ) -> Self {
        Self {
            database,
            cache_coordinator,
            service_jobs,
            session: Mutex::new(BTreeMap::new()),
            indexed_db_transactions: idb_transactions,
            idb_connections,
            idb_mutations,
            #[cfg(test)]
            fail_next_idb_commit: std::sync::atomic::AtomicBool::new(false),
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
        admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
    ) -> Result<StorageResponse, StorageError> {
        // Read and mutate under the same database writer. Separate read/write
        // transactions lose an overlapping context's update to another key.
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        let old_value;
        {
            let mut table = write
                .open_table(LOCAL_STORAGE)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            let prefix = format!("{}\0", scope.origin);
            let mut current = BTreeMap::new();
            for item in table
                .iter()
                .map_err(|error| StorageError::Persistent(error.to_string()))?
            {
                let (stored_key, stored_value) =
                    item.map_err(|error| StorageError::Persistent(error.to_string()))?;
                if let Some(key) = stored_key.value().strip_prefix(&prefix) {
                    current.insert(
                        key.to_owned(),
                        String::from_utf8(stored_value.value().to_vec())
                            .map_err(|error| StorageError::Persistent(error.to_string()))?,
                    );
                }
            }
            old_value = key.as_ref().and_then(|key| current.get(key)).cloned();
            if (clear && current.is_empty()) || (!clear && old_value == value) {
                return Ok(StorageResponse::Unchanged);
            }
            if clear {
                for key in current.keys() {
                    let key = format!("{prefix}{key}");
                    table
                        .remove(key.as_str())
                        .map_err(|error| StorageError::Persistent(error.to_string()))?;
                }
            } else if let Some(key) = &key {
                let mut next = current;
                match &value {
                    Some(value) => {
                        next.insert(key.clone(), value.clone());
                    }
                    None => {
                        next.remove(key);
                    }
                }
                let size = next.iter().fold(0usize, |bytes, (key, value)| {
                    bytes.saturating_add(key.len()).saturating_add(value.len())
                });
                if size > self.limits.web_storage_bytes_per_origin {
                    return Err(StorageError::QuotaExceeded {
                        origin: scope.origin,
                        limit: self.limits.web_storage_bytes_per_origin,
                    });
                }
                let key = format!("{prefix}{key}");
                match &value {
                    Some(value) => {
                        table
                            .insert(key.as_str(), value.as_bytes())
                            .map_err(|error| StorageError::Persistent(error.to_string()))?;
                    }
                    None => {
                        table
                            .remove(key.as_str())
                            .map_err(|error| StorageError::Persistent(error.to_string()))?;
                    }
                }
            }
        }
        let change = StorageChange {
            area: StorageArea::Local,
            scope,
            key,
            old_value,
            new_value: value,
        };
        admit(&change)?;
        #[cfg(test)]
        if FAIL_WEB_STORAGE_COMMIT.with(|failure| failure.replace(false)) {
            return Err(StorageError::Persistent(
                "injected commit after notification admission".into(),
            ));
        }
        write
            .commit()
            .map_err(|error| StorageError::Persistent(error.to_string()))?;
        Ok(StorageResponse::Changed { change })
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
        #[cfg(test)]
        if self
            .fail_next_idb_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Persistent(
                "injected IDB commit failure".into(),
            ));
        }
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
    fn service_job_coordinator(&self) -> Option<Arc<ServiceJobCoordinator>> {
        Some(self.service_jobs.clone())
    }
    fn cache_coordinator(&self) -> Option<Arc<Mutex<CacheCoordinator>>> {
        Some(self.cache_coordinator.clone())
    }
    fn service_storage(
        &self,
        request: ServiceStorageRequest,
    ) -> Result<ServiceStorageSnapshot, StorageError> {
        service::persistent(&self.database, request)
    }
    fn execute_with_admission(
        &self,
        request: StorageRequest,
        admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
    ) -> Result<StorageResponse, StorageError> {
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
            } => self.update_local(scope, Some(key), Some(value), false, admit),
            StorageRequest::Remove {
                area: StorageArea::Local,
                scope,
                key,
            } => self.update_local(scope, Some(key), None, false, admit),
            StorageRequest::Clear {
                area: StorageArea::Local,
                scope,
            } => self.update_local(scope, None, None, true, admit),
            StorageRequest::Entries {
                area: StorageArea::Local,
                scope,
            } => Ok(StorageResponse::Entries {
                entries: self.local_entries(&scope.origin)?,
            }),
            StorageRequest::IndexedDb { origin, command } => {
                let changes_storage = idb_command_changes_storage(&command);
                // Serialize the complete read/modify/commit across every context
                // sharing this profile, not just each context's undo ledger.
                let _mutation = self.idb_mutations.lock().map_err(|_| {
                    StorageError::Persistent("IndexedDB mutation lock poisoned".into())
                })?;
                let mut databases = self.load_idb(&origin)?;
                let mut transactions = self.indexed_db_transactions.lock().map_err(|_| {
                    StorageError::Persistent("IndexedDB transaction lock poisoned".into())
                })?;
                let mut connections = self.idb_connections.lock().map_err(|_| {
                    StorageError::Persistent("IndexedDB connection lock poisoned".into())
                })?;
                let mut next_connections = connections.clone();
                let mut next_transactions = transactions.clone();
                let response = execute_idb_coordinated(
                    &mut databases,
                    &mut next_transactions,
                    &mut next_connections,
                    &origin,
                    command,
                    self.limits.indexed_db_bytes_per_origin,
                )?;
                if changes_storage && !response.transaction_blocked {
                    self.save_idb(&origin, databases)?;
                }
                *transactions = next_transactions;
                *connections = next_connections;
                Ok(StorageResponse::IndexedDb { response })
            }
            request => {
                let mut session = self
                    .session
                    .lock()
                    .map_err(|_| StorageError::Persistent("session store lock poisoned".into()))?;
                execute_session_with_admission(&mut session, request, self.limits, admit)
            }
        }
    }
}

fn execute_session(
    session: &mut BTreeMap<(PageId, String, String), String>,
    request: StorageRequest,
    limits: StorageLimits,
) -> Result<StorageResponse, StorageError> {
    execute_session_with_admission(session, request, limits, &mut |_| Ok(()))
}

fn execute_session_with_admission(
    session: &mut BTreeMap<(PageId, String, String), String>,
    request: StorageRequest,
    limits: StorageLimits,
    admit: &mut dyn FnMut(&StorageChange) -> Result<(), StorageError>,
) -> Result<StorageResponse, StorageError> {
    let mut state = MemoryState {
        session: std::mem::take(session),
        ..MemoryState::default()
    };
    let store = MemoryContextStore::new(limits);
    *store.state.lock().expect("fresh memory store") = state;
    let response = store.execute_with_admission(request, admit);
    state = store.state.into_inner().expect("fresh memory store");
    *session = state.session;
    response
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
    #[serde(skip)]
    serialized_bytes: std::cell::Cell<Option<usize>>,
}

#[derive(Debug, Clone)]
struct TransactionSnapshot {
    origin: String,
    database: String,
    scope: Vec<String>,
    mode: String,
    order: u64,
    connection: Option<String>,
    working: Option<DatabaseData>,
    dirty: BTreeSet<String>,
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

/// The script protocol uses connection/transaction capabilities. Raw storage
/// commands remain a trusted in-process API for profile tools and fixtures.
pub(crate) fn validate_script_idb_command(command: &IdbCommand) -> Result<(), StorageError> {
    match command{
        IdbCommand::ListDatabases|IdbCommand::OpenConnection{..}|IdbCommand::DeleteConnection{..}|IdbCommand::CloseConnection{..}|IdbCommand::RetireConnection{..}|IdbCommand::PollConnection{..}|IdbCommand::AcknowledgeConnection{..}|IdbCommand::UpgradeConnection{..}|IdbCommand::BeginTransaction{connection:Some(_),..}|IdbCommand::Transaction{..}|IdbCommand::CommitTransaction{..}|IdbCommand::AbortTransaction{..}=>Ok(()),
        _=>Err(StorageError::Persistent("SecurityError: script IndexedDB operation requires connection or transaction authority".into())),
    }
}

fn execute_idb_coordinated(
    databases: &mut BTreeMap<(String, String), DatabaseData>,
    transactions: &mut BTreeMap<String, TransactionSnapshot>,
    connections: &mut idb_connections::Connections,
    origin: &str,
    command: IdbCommand,
    limit: usize,
) -> Result<IdbResponse, StorageError> {
    connections.reap_closed(transactions);
    let response = match command {
        IdbCommand::OpenConnection {
            token,
            name,
            version,
        } => {
            connections.reserve(&token, origin, &name, Some(version))?;
            if !connections.job_ready(&token) {
                return Ok(IdbResponse {
                    transaction_blocked: true,
                    connection_managed: true,
                    ..Default::default()
                });
            }
            let old = databases
                .get(&(origin.into(), name.clone()))
                .map_or(0, |data| data.version);
            let wanted = if version == 0 { old.max(1) } else { version };
            if wanted > old {
                if let Some(blocked) = connections.changing(&token, origin, &name, Some(wanted))? {
                    return Ok(IdbResponse {
                        transaction_blocked: true,
                        connection_blocked: blocked,
                        connection_managed: true,
                        old_version: Some(old),
                        version: Some(wanted),
                        ..Default::default()
                    });
                }
            }
            let mut view = databases.clone();
            let key = (origin.into(), name.clone());
            let mut response = match execute_idb_transactional(
                &mut view,
                transactions,
                origin,
                IdbCommand::Open { name, version },
                limit,
            ) {
                Ok(response) => response,
                Err(error) => {
                    connections.remove(&token);
                    return Err(error);
                }
            };
            response.connection_managed = true;
            if !response.transaction_blocked {
                let private = response
                    .upgrade_needed
                    .then(|| view.remove(&key).expect("opened upgrade snapshot"));
                connections.opened(
                    &token,
                    response.version.unwrap_or(wanted),
                    response.old_version.unwrap_or(0),
                    private,
                )?;
            }
            response
        }
        IdbCommand::UpgradeConnection { token, name, inner } => {
            let mut view = databases.clone();
            let key = (origin.into(), name.clone());
            view.insert(key.clone(), connections.upgrade(&token, origin, &name)?);
            let commit = matches!(inner.as_ref(), IdbCommand::CommitUpgrade { .. });
            let abort = matches!(inner.as_ref(), IdbCommand::AbortUpgrade { .. });
            let database = match inner.as_ref() {
                IdbCommand::CommitUpgrade { name } | IdbCommand::AbortUpgrade { name, .. } => name,
                IdbCommand::CreateObjectStore { database, .. }
                | IdbCommand::DeleteObjectStore { database, .. }
                | IdbCommand::CreateIndex { database, .. }
                | IdbCommand::DeleteIndex { database, .. }
                | IdbCommand::RenameObjectStore { database, .. }
                | IdbCommand::RenameIndex { database, .. }
                | IdbCommand::Batch { database, .. }
                | IdbCommand::Delete { database, .. }
                | IdbCommand::Clear { database, .. }
                | IdbCommand::Get { database, .. }
                | IdbCommand::GetAll { database, .. }
                | IdbCommand::Count { database, .. }
                | IdbCommand::Cursor { database, .. }
                | IdbCommand::CursorStep { database, .. } => database,
                _ => {
                    return Err(StorageError::Persistent(
                        "invalid private upgrade command".into(),
                    ));
                }
            };
            if database != &name {
                return Err(StorageError::Persistent(
                    "private upgrade database mismatch".into(),
                ));
            }
            let response = execute_idb(&mut view, origin, *inner, limit)?;
            if commit {
                *databases = view;
                connections.finish_upgrade(origin, &name);
            } else if abort {
                connections.finish_upgrade(origin, &name);
            } else {
                connections
                    .stage_upgrade(&token, view.remove(&key).expect("private upgrade snapshot"))?;
            }
            response
        }
        IdbCommand::DeleteConnection { token, name } => {
            connections.reserve(&token, origin, &name, None)?;
            if !connections.job_ready(&token) {
                return Ok(IdbResponse {
                    transaction_blocked: true,
                    connection_managed: true,
                    ..Default::default()
                });
            }
            let old = databases
                .get(&(origin.into(), name.clone()))
                .map_or(0, |data| data.version);
            if let Some(blocked) = connections.changing(&token, origin, &name, None)? {
                return Ok(IdbResponse {
                    transaction_blocked: true,
                    connection_blocked: blocked,
                    connection_managed: true,
                    old_version: Some(old),
                    ..Default::default()
                });
            }
            let mut response = execute_idb_transactional(
                databases,
                transactions,
                origin,
                IdbCommand::DeleteDatabase { name },
                limit,
            )?;
            response.connection_managed = true;
            if !response.transaction_blocked {
                connections.remove(&token);
            }
            response
        }
        IdbCommand::RetireConnection { token, name } => {
            // Forced closure aborts every transaction belonging to this exact
            // native connection. Merely dropping the lease strands snapshots
            // and leaves later writers/version changes blocked indefinitely.
            connections.close(&token, origin, &name)?;
            transactions.retain(|_, transaction| {
                !(transaction.origin == origin
                    && transaction.database == name
                    && transaction.connection.as_deref() == Some(token.as_str()))
            });
            connections.remove(&token);
            IdbResponse::default()
        }
        IdbCommand::CloseConnection { token, name } => {
            connections.close(&token, origin, &name)?;
            IdbResponse::default()
        }
        IdbCommand::PollConnection { token, name } => IdbResponse {
            connection_events: connections.poll(&token, origin, &name)?,
            ..Default::default()
        },
        IdbCommand::AcknowledgeConnection { token, name, event } => {
            connections.acknowledge(&token, origin, &name, event)?;
            IdbResponse::default()
        }
        command => {
            if let IdbCommand::BeginTransaction {
                token,
                database,
                connection: Some(connection),
                ..
            } = &command
            {
                if !transactions.contains_key(token) {
                    connections.validate_live(connection, origin, database)?;
                }
            }
            let finished = match &command {
                IdbCommand::CommitUpgrade { name } | IdbCommand::AbortUpgrade { name, .. } => {
                    Some(name.clone())
                }
                _ => None,
            };
            let response =
                execute_idb_transactional(databases, transactions, origin, command, limit)?;
            if let Some(name) = finished {
                connections.finish_upgrade(origin, &name);
            }
            response
        }
    };
    connections.reap_closed(transactions);
    Ok(response)
}
fn admit_idb_transaction(
    databases: &BTreeMap<(String, String), DatabaseData>,
    transactions: &mut BTreeMap<String, TransactionSnapshot>,
    origin: &str,
    token: &str,
    database: &str,
    mut scope: Vec<String>,
    mode: String,
    connection: Option<String>,
) -> Result<bool, StorageError> {
    scope.sort();
    scope.dedup();
    if scope.is_empty() || scope.len() > 1024 || !matches!(mode.as_str(), "readonly" | "readwrite")
    {
        return Err(StorageError::Persistent(
            "Invalid transaction scope or mode".into(),
        ));
    }
    if let Some(existing) = transactions.get(token) {
        if existing.origin != origin
            || existing.database != database
            || existing.scope != scope
            || existing.mode != mode
            || existing.connection != connection
        {
            return Err(StorageError::Persistent(
                "IndexedDB transaction identity mismatch".into(),
            ));
        }
    } else {
        if transactions.len() >= 1024 {
            return Err(StorageError::Persistent(
                "IndexedDB transaction capacity".into(),
            ));
        }
        let data = databases
            .get(&(origin.into(), database.into()))
            .ok_or_else(|| StorageError::Persistent("IndexedDB database missing".into()))?;
        if scope.iter().any(|name| !data.stores.contains_key(name)) {
            return Err(StorageError::Persistent(
                "NotFoundError: transaction object store missing".into(),
            ));
        }
        let order = transactions
            .values()
            .map(|entry| entry.order)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                StorageError::Persistent("IndexedDB transaction order exhausted".into())
            })?;
        transactions.insert(
            token.into(),
            TransactionSnapshot {
                origin: origin.into(),
                database: database.into(),
                scope,
                mode,
                order,
                connection,
                working: None,
                dirty: BTreeSet::new(),
            },
        );
    }
    let current = &transactions[token];
    if current.working.is_some() {
        return Ok(true);
    }
    if transactions.values().any(|earlier| {
        earlier.origin == origin
            && earlier.database == database
            && earlier.order < current.order
            && (earlier.mode == "readwrite" || current.mode == "readwrite")
            && earlier
                .scope
                .iter()
                .any(|name| current.scope.contains(name))
    }) {
        return Ok(false);
    }
    let mut working = databases
        .get(&(origin.into(), database.into()))
        .cloned()
        .ok_or_else(|| StorageError::Persistent("IndexedDB database missing".into()))?;
    if working.upgrade_backup.is_some() {
        return Ok(false);
    }
    working
        .stores
        .retain(|name, _| current.scope.contains(name));
    working.serialized_bytes.set(None);
    let bytes = serde_json::to_vec(&working)
        .map_err(|error| StorageError::Persistent(error.to_string()))?
        .len();
    let retained = transactions
        .values()
        .filter_map(|entry| entry.working.as_ref())
        .try_fold(bytes, |total, working| {
            serde_json::to_vec(working)
                .map(|bytes| total.saturating_add(bytes.len()))
                .map_err(|error| StorageError::Persistent(error.to_string()))
        })?;
    if retained > 256 * 1024 * 1024 {
        return Err(StorageError::Persistent(
            "IndexedDB transaction snapshot capacity".into(),
        ));
    }
    transactions
        .get_mut(token)
        .expect("admitted transaction")
        .working = Some(working);
    Ok(true)
}
fn execute_idb_transactional(
    databases: &mut BTreeMap<(String, String), DatabaseData>,
    transactions: &mut BTreeMap<String, TransactionSnapshot>,
    origin: &str,
    command: IdbCommand,
    limit: usize,
) -> Result<IdbResponse, StorageError> {
    let connection_name = match &command {
        IdbCommand::Open { name, .. } | IdbCommand::DeleteDatabase { name } => Some(name),
        _ => None,
    };
    if let Some(name) = connection_name {
        let data = databases.get(&(origin.into(), name.clone()));
        let changing = matches!(&command, IdbCommand::DeleteDatabase { .. })
            || matches!(&command,IdbCommand::Open{version,..}if *version>data.map_or(0,|data|data.version));
        if data.is_some_and(|data| data.upgrade_backup.is_some())
            || changing
                && transactions
                    .values()
                    .any(|entry| entry.origin == origin && entry.database == *name)
        {
            return Ok(IdbResponse {
                transaction_blocked: true,
                ..Default::default()
            });
        }
    }
    match command {
        IdbCommand::BeginTransaction {
            token,
            database,
            scope,
            mode,
            connection,
        } => {
            let ready = admit_idb_transaction(
                databases,
                transactions,
                origin,
                &token,
                &database,
                scope,
                mode,
                connection,
            )?;
            Ok(IdbResponse {
                transaction_blocked: !ready,
                ..Default::default()
            })
        }
        IdbCommand::Transaction {
            token,
            database,
            inner,
        } => {
            let (inner_database, stores, write) = match inner.as_ref() {
                IdbCommand::Batch { database, writes } => (
                    database,
                    writes
                        .iter()
                        .map(|write| write.store.clone())
                        .collect::<BTreeSet<_>>(),
                    true,
                ),
                IdbCommand::Delete {
                    database, store, ..
                }
                | IdbCommand::Clear { database, store } => {
                    (database, BTreeSet::from([store.clone()]), true)
                }
                IdbCommand::Get {
                    database, store, ..
                }
                | IdbCommand::GetAll {
                    database, store, ..
                }
                | IdbCommand::Count {
                    database, store, ..
                }
                | IdbCommand::Cursor {
                    database, store, ..
                }
                | IdbCommand::CursorStep {
                    database, store, ..
                } => (database, BTreeSet::from([store.clone()]), false),
                _ => {
                    return Err(StorageError::Persistent(
                        "Invalid transactional command".into(),
                    ));
                }
            };
            if inner_database != &database {
                return Err(StorageError::Persistent(
                    "IndexedDB transaction database mismatch".into(),
                ));
            }
            // Script ingress puts tokens into this native namespace only after
            // authenticating the principal. Such a request must use a snapshot
            // admitted by BeginTransaction with a live connection; it cannot
            // create or revive a transaction by presenting a correlation alone.
            let scoped = serde_json::from_str::<(String, String, String)>(&token)
                .is_ok_and(|(domain, _, _)| domain == "oryn:idb-transaction:v1");
            if scoped
                && !transactions
                    .get(&token)
                    .is_some_and(|entry| entry.connection.is_some())
            {
                return Err(StorageError::Persistent(
                    "TransactionInactiveError: no admitted native connection transaction".into(),
                ));
            }
            // Trusted internal callers may submit an unscoped first command
            // directly; they enter the same conflict queue with its exact scope.
            let (scope, mode) = transactions
                .get(&token)
                .map(|entry| (entry.scope.clone(), entry.mode.clone()))
                .unwrap_or_else(|| {
                    (
                        stores.iter().cloned().collect(),
                        if write { "readwrite" } else { "readonly" }.into(),
                    )
                });
            let connection = transactions
                .get(&token)
                .and_then(|entry| entry.connection.clone());
            if !admit_idb_transaction(
                databases,
                transactions,
                origin,
                &token,
                &database,
                scope,
                mode,
                connection,
            )? {
                return Ok(IdbResponse {
                    transaction_blocked: true,
                    ..Default::default()
                });
            }
            let entry = transactions.get(&token).expect("admitted transaction");
            if stores.iter().any(|name| !entry.scope.contains(name))
                || write && entry.mode != "readwrite"
            {
                return Err(StorageError::Persistent(
                    "ReadOnlyError: request outside transaction authority".into(),
                ));
            }
            let key = (origin.to_string(), database.clone());
            let mut view = databases.clone();
            let target = view
                .get_mut(&key)
                .ok_or_else(|| StorageError::Persistent("IndexedDB database missing".into()))?;
            let working = entry.working.as_ref().expect("active snapshot");
            for name in &entry.scope {
                if let Some(store) = working.stores.get(name) {
                    target.stores.insert(name.clone(), store.clone());
                } else {
                    target.stores.remove(name);
                }
            }
            target.serialized_bytes.set(None);
            let response = execute_idb(&mut view, origin, *inner, limit)?;
            let mut working = view.remove(&key).expect("transaction view");
            working.stores.retain(|name, _| entry.scope.contains(name));
            working.serialized_bytes.set(None);
            let retained = transactions
                .iter()
                .filter(|(existing, _)| *existing != &token)
                .filter_map(|(_, entry)| entry.working.as_ref())
                .chain(std::iter::once(&working))
                .try_fold(0usize, |total, working| {
                    serde_json::to_vec(working)
                        .map(|bytes| total.saturating_add(bytes.len()))
                        .map_err(|error| StorageError::Persistent(error.to_string()))
                })?;
            if retained > 256 * 1024 * 1024 {
                return Err(StorageError::Persistent(
                    "IndexedDB transaction snapshot capacity".into(),
                ));
            }
            let entry = transactions.get_mut(&token).expect("admitted transaction");
            entry.working = Some(working);
            if write {
                entry.dirty.extend(stores);
            }
            Ok(response)
        }
        IdbCommand::CommitTransaction { token, database } => {
            let Some(entry) = transactions.get(&token) else {
                return Ok(IdbResponse::default());
            };
            if entry.origin != origin || entry.database != database {
                return Err(StorageError::Persistent(
                    "IndexedDB transaction identity mismatch".into(),
                ));
            }
            let (scope, mode) = (entry.scope.clone(), entry.mode.clone());
            let connection = transactions
                .get(&token)
                .and_then(|entry| entry.connection.clone());
            if !admit_idb_transaction(
                databases,
                transactions,
                origin,
                &token,
                &database,
                scope,
                mode,
                connection,
            )? {
                return Ok(IdbResponse {
                    transaction_blocked: true,
                    ..Default::default()
                });
            }
            let entry = &transactions[&token];
            let mut next = databases.clone();
            let key = (origin.into(), database);
            let target = next
                .get_mut(&key)
                .ok_or_else(|| StorageError::Persistent("IndexedDB database missing".into()))?;
            let working = entry.working.as_ref().expect("active snapshot");
            for name in &entry.dirty {
                if let Some(store) = working.stores.get(name) {
                    target.stores.insert(name.clone(), store.clone());
                } else {
                    target.stores.remove(name);
                }
            }
            target.serialized_bytes.set(None);
            if indexed_db_origin_bytes(&next, origin)? > limit {
                return Err(StorageError::QuotaExceeded {
                    origin: origin.into(),
                    limit,
                });
            }
            *databases = next;
            transactions.remove(&token);
            Ok(IdbResponse::default())
        }
        IdbCommand::AbortTransaction { token, database } => {
            if transactions
                .get(&token)
                .is_some_and(|entry| entry.origin != origin || entry.database != database)
            {
                return Err(StorageError::Persistent(
                    "IndexedDB transaction identity mismatch".into(),
                ));
            }
            transactions.remove(&token);
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
    let (command, cursor_step) = match command {
        IdbCommand::CursorStep {
            database,
            store,
            index,
            query,
            reverse,
            step,
        } => {
            if step.advance == 0 {
                return Err(StorageError::Persistent(
                    "Cursor advance must be positive".into(),
                ));
            }
            (
                IdbCommand::Cursor {
                    database,
                    store,
                    index,
                    query,
                    reverse,
                },
                Some(step),
            )
        }
        command => (command, None),
    };
    // Quota is affected only by schema/data mutations. Re-serializing an
    // origin's complete database after every get/count/cursor request made
    // high-volume index scans quadratic even though reads cannot increase
    // usage. Mutation commands still perform the exact post-operation check.
    let changes_usage = idb_command_changes_storage(&command);
    if changes_usage && !matches!(&command, IdbCommand::Batch { .. }) {
        // Schema changes and transaction restoration can change several stores.
        // Batch writes below maintain the exact serialized size incrementally.
        for ((item_origin, _), database) in databases.iter() {
            if item_origin == origin {
                database.serialized_bytes.set(None);
            }
        }
    }
    // A versionchange transaction already owns an exact pre-upgrade backup.
    // Recomputing the complete origin size after every schema/write operation
    // makes large upgrades quadratic. Defer their quota check to the atomic
    // CommitUpgrade boundary, which can restore that backup on failure.
    let upgrade_mutation = match &command {
        IdbCommand::CreateObjectStore { database, .. }
        | IdbCommand::DeleteObjectStore { database, .. }
        | IdbCommand::CreateIndex { database, .. }
        | IdbCommand::DeleteIndex { database, .. }
        | IdbCommand::RenameObjectStore { database, .. }
        | IdbCommand::RenameIndex { database, .. }
        | IdbCommand::Batch { database, .. }
        | IdbCommand::Delete { database, .. }
        | IdbCommand::Clear { database, .. } => databases
            .get(&(origin.to_string(), database.clone()))
            .is_some_and(|database| database.upgrade_backup.is_some()),
        _ => false,
    };
    let response = match command {
        IdbCommand::UpgradeConnection { .. }
        | IdbCommand::RetireConnection { .. }
        | IdbCommand::OpenConnection { .. }
        | IdbCommand::DeleteConnection { .. }
        | IdbCommand::CloseConnection { .. }
        | IdbCommand::PollConnection { .. }
        | IdbCommand::AcknowledgeConnection { .. }
        | IdbCommand::BeginTransaction { .. }
        | IdbCommand::Transaction { .. }
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
            let key = (origin.to_string(), name);
            if indexed_db_origin_bytes(databases, origin)? > limit {
                if let Some(backup) = databases
                    .get_mut(&key)
                    .and_then(|database| database.upgrade_backup.take())
                {
                    databases.insert(key, *backup);
                }
                return Err(StorageError::QuotaExceeded {
                    origin: origin.to_string(),
                    limit,
                });
            }
            if let Some(database) = databases.get_mut(&key) {
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
        IdbCommand::RenameObjectStore {
            database,
            store,
            new_name,
        } => {
            let data = database_mut(databases, origin, &database)?;
            if store != new_name && data.stores.contains_key(&new_name) {
                return Err(StorageError::Constraint);
            }
            let mut contents = data
                .stores
                .remove(&store)
                .ok_or(StorageError::ObjectStoreNotFound { database, store })?;
            if let Some(definition) = &mut contents.definition {
                definition.name = new_name.clone();
            }
            data.stores.insert(new_name, contents);
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
            let initial_usage = if upgrade_mutation {
                None
            } else {
                Some(indexed_db_origin_bytes(databases, origin)?)
            };
            let database_key = (origin.to_string(), database.clone());
            let initial_database_bytes = databases
                .get(&database_key)
                .and_then(|database| database.serialized_bytes.get());
            let mut undo = Vec::new();
            let mut byte_delta = 0i128;
            let result = (|| {
                let mut last_key = None;
                for write in writes {
                    let name = write.store.clone();
                    let store = store_mut(databases, origin, &database, &name)?;
                    let (key, previous) = put_record_with_undo(store, write)?;
                    byte_delta += previous.byte_delta;
                    undo.push((name, previous));
                    last_key = Some(key);
                }
                if initial_usage.is_some_and(|usage| usage as i128 + byte_delta > limit as i128) {
                    return Err(StorageError::QuotaExceeded {
                        origin: origin.to_string(),
                        limit,
                    });
                }
                Ok(IdbResponse {
                    key: last_key,
                    ..IdbResponse::default()
                })
            })();
            if result.is_err() {
                // A later write can fail after earlier writes changed the same
                // key. Reverse order restores both records and key generators.
                for (name, previous) in undo.into_iter().rev() {
                    let store = store_mut(databases, origin, &database, &name)?;
                    store.next_key = previous.next_key;
                    match previous.record {
                        Some(record) => {
                            store.records.insert(previous.encoded_key, record);
                        }
                        None => {
                            store.records.remove(&previous.encoded_key);
                        }
                    }
                }
            } else if let Some(database) = databases.get(&database_key) {
                database.serialized_bytes.set(
                    initial_database_bytes
                        .and_then(|bytes| usize::try_from(bytes as i128 + byte_delta).ok()),
                );
            }
            // The batch has already checked exact origin usage. Avoid a
            // second complete serialization in the generic mutation path.
            return result;
        }
        IdbCommand::Get {
            database,
            store,
            key,
        } => {
            let store_data = store_ref(databases, origin, &database, &store)?;
            let stored = if key
                .get("__oryn_key_range")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                store_data
                    .records
                    .values()
                    .filter(|record| indexed_db_key_matches(&key, &record.key))
                    .min_by(|left, right| compare_indexed_db_keys(&left.key, &right.key))
            } else {
                store_data.records.get(&encode_key(&key)?)
            };
            IdbResponse {
                value: stored.map(|record| record.value.clone()),
                value_present: stored.is_some(),
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
                    let mut cursor_keys = index_definition.map_or_else(
                        || vec![record.key.clone()],
                        |definition| indexed_record_keys(&record.value, definition),
                    );
                    cursor_keys.retain(|key| oryn_js::indexed_db::Key::from_json(key).is_some());
                    cursor_keys.sort_by(compare_indexed_db_keys);
                    cursor_keys
                        .dedup_by(|left, right| compare_indexed_db_keys(left, right).is_eq());
                    let query = query.as_ref();
                    cursor_keys.into_iter().filter_map(move |cursor_key| {
                        query
                            .as_ref()
                            .is_none_or(|query| indexed_db_key_matches(query, &cursor_key))
                            .then_some((cursor_key, record))
                    })
                })
                .collect::<Vec<_>>();
            let compare = |left: &(serde_json::Value, &RecordData),
                           right: &(serde_json::Value, &RecordData)| {
                compare_indexed_db_keys(&left.0, &right.0)
                    .then_with(|| compare_indexed_db_keys(&left.1.key, &right.1.key))
            };
            // Choose the lowest primary key for unique traversal before
            // reversing; prevunique reverses keys, not the representative.
            if cursor_step.as_ref().is_some_and(|step| step.unique) {
                records.sort_by(compare);
                records.dedup_by(|right, left| compare_indexed_db_keys(&left.0, &right.0).is_eq());
            }
            let selected = if let Some(step) = &cursor_step {
                records.retain(|(key, record)| {
                    let after =
                        step.after
                            .as_ref()
                            .is_none_or(|(previous_key, previous_primary)| {
                                let order = compare_indexed_db_keys(key, previous_key);
                                let order = if step.unique {
                                    order
                                } else {
                                    order.then_with(|| {
                                        compare_indexed_db_keys(&record.key, previous_primary)
                                    })
                                };
                                if reverse {
                                    order.is_lt()
                                } else {
                                    order.is_gt()
                                }
                            });
                    let target = step
                        .target
                        .as_ref()
                        .is_none_or(|(target_key, target_primary)| {
                            let order = compare_indexed_db_keys(key, target_key);
                            let order = order.then_with(|| {
                                target_primary
                                    .as_ref()
                                    .map_or(std::cmp::Ordering::Equal, |primary| {
                                        compare_indexed_db_keys(&record.key, primary)
                                    })
                            });
                            if reverse {
                                order.is_le()
                            } else {
                                order.is_ge()
                            }
                        });
                    after && target
                });
                let position = step.advance.saturating_sub(1) as usize;
                if position >= records.len() {
                    Vec::new()
                } else {
                    // A step needs one position, not a sorted copy of every
                    // remaining key. Select in linear time rather than sorting
                    // the store again on each continue()/advance() request.
                    records.select_nth_unstable_by(position, |left, right| {
                        if reverse {
                            compare(right, left)
                        } else {
                            compare(left, right)
                        }
                    });
                    vec![records.swap_remove(position)]
                }
            } else {
                records.sort_by(compare);
                if reverse {
                    records.reverse();
                }
                records
            };
            // Only returned values are cloned. A cursor response never owns
            // copies of all the values it skipped while locating its position.
            let records = selected
                .into_iter()
                .map(|(key, record)| IdbRecord {
                    key,
                    primary_key: record.key.clone(),
                    value: if cursor_step.as_ref().is_none_or(|step| step.with_value) {
                        record.value.clone()
                    } else {
                        serde_json::Value::Null
                    },
                })
                .collect();
            IdbResponse {
                records,
                ..IdbResponse::default()
            }
        }
        IdbCommand::CursorStep { .. } => unreachable!("cursor step normalized above"),
    };
    if changes_usage && !upgrade_mutation && indexed_db_origin_bytes(databases, origin)? > limit {
        return Err(StorageError::QuotaExceeded {
            origin: origin.to_string(),
            limit,
        });
    }
    Ok(response)
}

fn idb_command_changes_storage(command: &IdbCommand) -> bool {
    match command {
        IdbCommand::UpgradeConnection { inner, .. } => {
            matches!(inner.as_ref(), IdbCommand::CommitUpgrade { .. })
        }
        IdbCommand::CloseConnection { .. }
        | IdbCommand::PollConnection { .. }
        | IdbCommand::AcknowledgeConnection { .. }
        | IdbCommand::BeginTransaction { .. }
        | IdbCommand::Transaction { .. }
        | IdbCommand::AbortTransaction { .. } => false,
        IdbCommand::DeleteConnection { .. } | IdbCommand::CommitTransaction { .. } => true,
        IdbCommand::RetireConnection { .. } | IdbCommand::OpenConnection { .. } => false,
        IdbCommand::Open { .. }
        | IdbCommand::DeleteDatabase { .. }
        | IdbCommand::AbortUpgrade { .. }
        | IdbCommand::CommitUpgrade { .. }
        | IdbCommand::CreateObjectStore { .. }
        | IdbCommand::DeleteObjectStore { .. }
        | IdbCommand::CreateIndex { .. }
        | IdbCommand::DeleteIndex { .. }
        | IdbCommand::RenameObjectStore { .. }
        | IdbCommand::RenameIndex { .. }
        | IdbCommand::Batch { .. }
        | IdbCommand::Delete { .. }
        | IdbCommand::Clear { .. } => true,
        IdbCommand::ListDatabases
        | IdbCommand::Get { .. }
        | IdbCommand::GetAll { .. }
        | IdbCommand::Count { .. }
        | IdbCommand::Cursor { .. }
        | IdbCommand::CursorStep { .. } => false,
    }
}

fn indexed_db_key_matches(query: &serde_json::Value, key: &serde_json::Value) -> bool {
    let Some(range) = query.as_object().filter(|range| {
        range
            .get("__oryn_key_range")
            .and_then(|value| value.as_bool())
            == Some(true)
    }) else {
        // JSON preserves whether a number was encoded as an integer or a
        // floating-point literal, while IndexedDB has one numeric key domain.
        // Compare keys by the IndexedDB ordering algorithm so `1` and `1.0`
        // address the same index entry.
        return encode_key(query).is_ok()
            && encode_key(key).is_ok()
            && compare_indexed_db_keys(query, key).is_eq();
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
    oryn_js::indexed_db::compare_json(left, right).unwrap_or(std::cmp::Ordering::Equal)
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
    oryn_js::indexed_db::index_keys(value, &definition.key_path, definition.multi_entry)
}

fn stored_index_key(value: &serde_json::Value) -> Option<serde_json::Value> {
    oryn_js::indexed_db::normalized_stored_key(value)
}

#[cfg(test)]
fn put_record(
    store: &mut ObjectStoreData,
    write: IdbWrite,
) -> Result<serde_json::Value, StorageError> {
    put_record_with_undo(store, write).map(|(key, _)| key)
}

struct RecordUndo {
    encoded_key: String,
    record: Option<RecordData>,
    next_key: u64,
    byte_delta: i128,
}

fn put_record_with_undo(
    store: &mut ObjectStoreData,
    write: IdbWrite,
) -> Result<(serde_json::Value, RecordUndo), StorageError> {
    let definition = store.definition.clone().unwrap_or(ObjectStoreDefinition {
        name: write.store,
        key_path: None,
        auto_increment: false,
        indexes: Vec::new(),
    });
    let mut value = write.value;
    let mut next_key = store.next_key;
    let key_from_path = definition
        .key_path
        .as_deref()
        .and_then(|path| json_value_at_key_path(&value, path));
    let key_from_path = key_from_path
        .map(|key| stored_index_key(&key).ok_or(StorageError::KeyRequired))
        .transpose()?;
    const MAX_GENERATED_KEY: u64 = 1_u64 << 53;
    let key = match write.key.or(key_from_path) {
        Some(key) => key,
        None if definition.auto_increment => {
            let next = next_key.max(1);
            if next > MAX_GENERATED_KEY {
                return Err(StorageError::Constraint);
            }
            next_key = next + 1;
            serde_json::Value::from(next)
        }
        None => return Err(StorageError::KeyRequired),
    };
    if definition.auto_increment
        && let Some(oryn_js::indexed_db::Key::Number(numeric)) =
            oryn_js::indexed_db::Key::from_json(&key)
        && numeric >= 1.0
    {
        let next = if numeric >= MAX_GENERATED_KEY as f64 {
            MAX_GENERATED_KEY + 1
        } else {
            numeric.floor() as u64 + 1
        };
        next_key = next_key.max(next);
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
        let keys = indexed_record_keys(&value, index);
        if store.records.iter().any(|(record_key, record)| {
            record_key != &encoded
                && indexed_record_keys(&record.value, index)
                    .iter()
                    .any(|existing| {
                        keys.iter()
                            .any(|key| compare_indexed_db_keys(existing, key).is_eq())
                    })
        }) {
            return Err(StorageError::Constraint);
        }
    }
    let next_record = RecordData {
        key: key.clone(),
        value,
    };
    let old_record = store.records.get(&encoded);
    let mut byte_delta = serialized_size(&next_record)? as i128;
    if let Some(old) = old_record {
        byte_delta -= serialized_size(old)? as i128;
    } else {
        // JSON object entries contain an escaped string key, a colon, and
        // (except for the first entry) one comma.
        byte_delta += serialized_size(&encoded)? as i128 + 1;
        byte_delta += i128::from(!store.records.is_empty());
    }
    byte_delta += next_key.to_string().len() as i128 - store.next_key.to_string().len() as i128;
    let previous_next_key = store.next_key;
    store.next_key = next_key;
    let record = store.records.insert(encoded.clone(), next_record);
    Ok((
        key,
        RecordUndo {
            encoded_key: encoded,
            record,
            next_key: previous_next_key,
            byte_delta,
        },
    ))
}

fn json_value_at_key_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    oryn_js::indexed_db::key_path(value, path)
}

fn inject_json_key_path(
    value: &mut serde_json::Value,
    path: &str,
    key: serde_json::Value,
) -> Result<(), StorageError> {
    if value
        .get("__oryn_clone_type")
        .and_then(serde_json::Value::as_str)
        == Some("storage")
    {
        return oryn_js::indexed_db::inject_storage_key(value, path, key)
            .then_some(())
            .ok_or(StorageError::KeyRequired);
    }
    let mut segments = path.split('.').peekable();
    let mut current = value;
    while let Some(segment) = segments.next() {
        let object = current.as_object_mut().ok_or(StorageError::KeyRequired)?;
        let object = if object
            .get("__oryn_clone_type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| matches!(kind, "array" | "object"))
        {
            object
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or(StorageError::KeyRequired)?
        } else {
            object
        };
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
    oryn_js::indexed_db::Key::from_json(key)
        .map(|key| key.storage_id())
        .ok_or_else(|| StorageError::DataClone("invalid IndexedDB key".into()))
}

fn serialized_size(value: &impl Serialize) -> Result<usize, StorageError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| StorageError::DataClone(error.to_string()))
}

fn indexed_db_origin_bytes(
    databases: &BTreeMap<(String, String), DatabaseData>,
    origin: &str,
) -> Result<usize, StorageError> {
    databases
        .iter()
        .filter(|((item_origin, _), _)| item_origin == origin)
        .try_fold(0usize, |total, (_, database)| {
            let bytes = match database.serialized_bytes.get() {
                Some(bytes) => bytes,
                None => {
                    let bytes = serialized_size(database)?;
                    database.serialized_bytes.set(Some(bytes));
                    bytes
                }
            };
            Ok(total.saturating_add(bytes))
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn web_storage_noop_mutations_do_not_produce_broadcast_changes() {
        let path = std::env::temp_dir().join(format!(
            "oryn-storage-noop-{}",
            format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            for store in [
                SharedContextStore::memory(StorageLimits::default()),
                SharedContextStore::persistent(&profile, StorageLimits::default()),
            ] {
                for area in [StorageArea::Local, StorageArea::Session] {
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Clear {
                                area,
                                scope: scope(1)
                            })
                            .unwrap(),
                        StorageResponse::Unchanged
                    ));
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Remove {
                                area,
                                scope: scope(1),
                                key: "missing".into()
                            })
                            .unwrap(),
                        StorageResponse::Unchanged
                    ));
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Set {
                                area,
                                scope: scope(1),
                                key: "key".into(),
                                value: "value".into()
                            })
                            .unwrap(),
                        StorageResponse::Changed { .. }
                    ));
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Set {
                                area,
                                scope: scope(1),
                                key: "key".into(),
                                value: "value".into()
                            })
                            .unwrap(),
                        StorageResponse::Unchanged
                    ));
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Clear {
                                area,
                                scope: scope(1)
                            })
                            .unwrap(),
                        StorageResponse::Changed { .. }
                    ));
                    assert!(matches!(
                        store
                            .execute(StorageRequest::Clear {
                                area,
                                scope: scope(1)
                            })
                            .unwrap(),
                        StorageResponse::Unchanged
                    ));
                }
            }
        }
        let _ = std::fs::remove_dir_all(path);
    }
    #[test]
    fn persistent_web_storage_overlapping_context_writes_preserve_both_keys() {
        let path = std::env::temp_dir().join(format!(
            "oryn-storage-overlap-{}",
            format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let stores = [
                SharedContextStore::persistent(&profile, StorageLimits::default()),
                SharedContextStore::persistent(&profile, StorageLimits::default()),
            ];
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let mut jobs = Vec::new();
            for (writer, store) in stores.iter().cloned().enumerate() {
                let barrier = barrier.clone();
                jobs.push(std::thread::spawn(move || {
                    for round in 0..64 {
                        barrier.wait();
                        store
                            .execute(StorageRequest::Set {
                                area: StorageArea::Local,
                                scope: scope(writer as u64 + 1),
                                key: format!("{writer}-{round}"),
                                value: "retained".into(),
                            })
                            .unwrap();
                    }
                }));
            }
            for job in jobs {
                job.join().unwrap();
            }
            let StorageResponse::Entries { entries } = stores[0]
                .execute(StorageRequest::Entries {
                    area: StorageArea::Local,
                    scope: scope(1),
                })
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(entries.len(), 128);
            for (writer, round) in
                (0..2).flat_map(|writer| (0..64).map(move |round| (writer, round)))
            {
                assert!(
                    entries
                        .iter()
                        .any(|(key, value)| key == &format!("{writer}-{round}")
                            && value == "retained")
                );
            }
        }
        let _ = std::fs::remove_dir_all(path);
    }
    use super::*;

    #[test]
    fn indexed_db_cursor_steps_bound_values_and_preserve_positions() {
        use serde_json::json;
        let mut store = ObjectStoreData::default();
        store.indexes.insert(
            "group".into(),
            IndexDefinition {
                name: "group".into(),
                key_path: "group".into(),
                unique: false,
                multi_entry: false,
            },
        );
        for (key, group) in [(1, "a"), (2, "a"), (3, "b")] {
            store.records.insert(
                encode_key(&json!(key)).unwrap(),
                RecordData {
                    key: json!(key),
                    value: json!({"group":group,"payload":"x".repeat(256*1024)}),
                },
            );
        }
        let mut databases = BTreeMap::from([(
            ("origin".into(), "db".into()),
            DatabaseData {
                stores: BTreeMap::from([("records".into(), store)]),
                ..Default::default()
            },
        )]);
        let read = |databases: &mut BTreeMap<(String, String), DatabaseData>, reverse, step| {
            execute_idb(
                databases,
                "origin",
                IdbCommand::CursorStep {
                    database: "db".into(),
                    store: "records".into(),
                    index: Some("group".into()),
                    query: None,
                    reverse,
                    step,
                },
                64 * 1024 * 1024,
            )
            .unwrap()
        };
        let first = IdbCursorStep {
            after: None,
            target: None,
            advance: 1,
            unique: false,
            with_value: true,
        };
        let response = read(&mut databases, false, first.clone());
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].primary_key, json!(1));
        assert!(serde_json::to_vec(&response).unwrap().len() < 270 * 1024);
        let second = IdbCursorStep {
            after: Some((json!("a"), json!(1))),
            ..first.clone()
        };
        assert_eq!(
            read(&mut databases, false, second.clone()).records[0].primary_key,
            json!(2)
        );
        assert_eq!(
            read(
                &mut databases,
                false,
                IdbCursorStep {
                    advance: 2,
                    ..first.clone()
                }
            )
            .records[0]
                .primary_key,
            json!(2)
        );
        assert_eq!(
            read(
                &mut databases,
                false,
                IdbCursorStep {
                    target: Some((json!("a"), Some(json!(2)))),
                    ..first.clone()
                }
            )
            .records[0]
                .primary_key,
            json!(2)
        );
        assert_eq!(
            read(
                &mut databases,
                false,
                IdbCursorStep {
                    unique: true,
                    ..second.clone()
                }
            )
            .records[0]
                .primary_key,
            json!(3)
        );
        let backwards = IdbCursorStep {
            after: Some((json!("b"), json!(3))),
            ..first.clone()
        };
        assert_eq!(
            read(&mut databases, true, backwards.clone()).records[0].primary_key,
            json!(2)
        );
        assert_eq!(
            read(
                &mut databases,
                true,
                IdbCursorStep {
                    unique: true,
                    ..backwards
                }
            )
            .records[0]
                .primary_key,
            json!(1)
        );
        assert!(
            read(
                &mut databases,
                false,
                IdbCursorStep {
                    after: Some((json!("b"), json!(3))),
                    ..first.clone()
                }
            )
            .records
            .is_empty()
        );
        let keys = read(
            &mut databases,
            false,
            IdbCursorStep {
                with_value: false,
                ..first.clone()
            },
        );
        assert!(keys.records[0].value.is_null());
        assert!(serde_json::to_vec(&keys).unwrap().len() < 1024);
        let all = execute_idb(
            &mut databases,
            "origin",
            IdbCommand::Cursor {
                database: "db".into(),
                store: "records".into(),
                index: None,
                query: None,
                reverse: false,
            },
            64 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(all.records.len(), 3);
        assert!(serde_json::to_vec(&all).unwrap().len() > 768 * 1024);
        // Steps see writes between requests, without an old cursor snapshot.
        let store = databases
            .get_mut(&("origin".into(), "db".into()))
            .unwrap()
            .stores
            .get_mut("records")
            .unwrap();
        store.records.remove(&encode_key(&json!(2)).unwrap());
        store.records.insert(
            encode_key(&json!(1.5)).unwrap(),
            RecordData {
                key: json!(1.5),
                value: json!({"group":"a"}),
            },
        );
        assert_eq!(
            read(&mut databases, false, second).records[0].primary_key,
            json!(1.5)
        );
    }

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
    fn indexed_db_failed_batch_restores_prior_writes_and_generator() {
        let origin = "https://example.test";
        let mut store = ObjectStoreData {
            definition: Some(ObjectStoreDefinition {
                name: "items".into(),
                key_path: None,
                auto_increment: true,
                indexes: Vec::new(),
            }),
            next_key: 1,
            ..Default::default()
        };
        put_record(
            &mut store,
            IdbWrite {
                store: "items".into(),
                key: Some(serde_json::json!(1)),
                value: serde_json::json!("existing"),
                add_only: true,
            },
        )
        .unwrap();
        let mut databases = BTreeMap::from([(
            (origin.into(), "app".into()),
            DatabaseData {
                version: 1,
                stores: BTreeMap::from([("items".into(), store)]),
                ..Default::default()
            },
        )]);
        let before = serde_json::to_value(&databases[&(origin.into(), "app".into())]).unwrap();
        let result = execute_idb(
            &mut databases,
            origin,
            IdbCommand::Batch {
                database: "app".into(),
                writes: vec![
                    IdbWrite {
                        store: "items".into(),
                        key: None,
                        value: serde_json::json!("new"),
                        add_only: true,
                    },
                    IdbWrite {
                        store: "items".into(),
                        key: Some(serde_json::json!(1)),
                        value: serde_json::json!("duplicate"),
                        add_only: true,
                    },
                ],
            },
            usize::MAX,
        );
        assert!(matches!(result, Err(StorageError::Constraint)));
        assert_eq!(
            serde_json::to_value(&databases[&(origin.into(), "app".into())]).unwrap(),
            before
        );
        let limit = indexed_db_origin_bytes(&databases, origin).unwrap();
        let result = execute_idb(
            &mut databases,
            origin,
            IdbCommand::Batch {
                database: "app".into(),
                writes: ["first replacement", "second replacement"]
                    .into_iter()
                    .map(|value| IdbWrite {
                        store: "items".into(),
                        key: Some(serde_json::json!(1)),
                        value: serde_json::json!(value),
                        add_only: false,
                    })
                    .collect(),
            },
            limit,
        );
        assert!(matches!(result, Err(StorageError::QuotaExceeded { .. })));
        assert_eq!(
            serde_json::to_value(&databases[&(origin.into(), "app".into())]).unwrap(),
            before
        );
    }

    #[test]
    fn indexed_db_incremental_usage_matches_serialization() {
        let origin = "https://example.test";
        let mut databases = BTreeMap::from([(
            (origin.into(), "app".into()),
            DatabaseData {
                stores: BTreeMap::from([(
                    "items".into(),
                    ObjectStoreData {
                        definition: Some(ObjectStoreDefinition {
                            name: "items".into(),
                            key_path: None,
                            auto_increment: true,
                            indexes: Vec::new(),
                        }),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        )]);
        for (key, value) in [
            (serde_json::json!(9), serde_json::json!("a")),
            (
                serde_json::json!(99),
                serde_json::json!({"escaped": "\"\\\n", "unicode": "é"}),
            ),
            (serde_json::json!(9), serde_json::json!([1, 2, 3])),
            (serde_json::json!("quoted\"key"), serde_json::json!(null)),
        ] {
            execute_idb(
                &mut databases,
                origin,
                IdbCommand::Batch {
                    database: "app".into(),
                    writes: vec![IdbWrite {
                        store: "items".into(),
                        key: Some(key),
                        value,
                        add_only: false,
                    }],
                },
                usize::MAX,
            )
            .unwrap();
            let exact = serialized_size(&databases[&(origin.into(), "app".into())]).unwrap();
            assert_eq!(indexed_db_origin_bytes(&databases, origin).unwrap(), exact);
        }
    }

    #[test]
    fn indexed_db_reads_release_native_snapshots_on_finalization() {
        let mut databases = BTreeMap::new();
        let mut transactions = BTreeMap::new();
        let origin = "https://example.test";
        for command in [
            IdbCommand::Open {
                name: "reads".into(),
                version: 1,
            },
            IdbCommand::CreateObjectStore {
                database: "reads".into(),
                definition: ObjectStoreDefinition {
                    name: "store".into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            },
            IdbCommand::CommitUpgrade {
                name: "reads".into(),
            },
        ] {
            execute_idb(&mut databases, origin, command, usize::MAX).unwrap();
        }
        for index in 0..100 {
            execute_idb_transactional(
                &mut databases,
                &mut transactions,
                origin,
                IdbCommand::Transaction {
                    token: format!("reader-{index}"),
                    database: "reads".into(),
                    inner: Box::new(IdbCommand::Get {
                        database: "reads".into(),
                        store: "store".into(),
                        key: serde_json::json!(1),
                    }),
                },
                usize::MAX,
            )
            .unwrap();
            execute_idb_transactional(
                &mut databases,
                &mut transactions,
                origin,
                IdbCommand::CommitTransaction {
                    token: format!("reader-{index}"),
                    database: "reads".into(),
                },
                usize::MAX,
            )
            .unwrap();
        }
        assert!(transactions.is_empty());
    }

    #[test]
    fn indexed_db_abort_preserves_commits_in_other_object_stores() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let execute = |command| {
            store.execute(StorageRequest::IndexedDb {
                origin: "https://example.test".into(),
                command,
            })
        };
        execute(IdbCommand::Open {
            name: "scopes".into(),
            version: 1,
        })
        .unwrap();
        for name in ["left", "right", "observer"] {
            execute(IdbCommand::CreateObjectStore {
                database: "scopes".into(),
                definition: ObjectStoreDefinition {
                    name: name.into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            })
            .unwrap();
        }
        execute(IdbCommand::CommitUpgrade {
            name: "scopes".into(),
        })
        .unwrap();
        execute(IdbCommand::Transaction {
            token: "reader".into(),
            database: "scopes".into(),
            inner: Box::new(IdbCommand::Get {
                database: "scopes".into(),
                store: "observer".into(),
                key: serde_json::json!(1),
            }),
        })
        .unwrap();
        for (token, name, value) in [
            ("aborting", "left", "discard"),
            ("committing", "right", "retain"),
        ] {
            execute(IdbCommand::Transaction {
                token: token.into(),
                database: "scopes".into(),
                inner: Box::new(IdbCommand::Batch {
                    database: "scopes".into(),
                    writes: vec![IdbWrite {
                        store: name.into(),
                        key: Some(serde_json::json!(1)),
                        value: serde_json::json!(value),
                        add_only: false,
                    }],
                }),
            })
            .unwrap();
        }
        assert!(
            execute(IdbCommand::AbortTransaction {
                token: "aborting".into(),
                database: "wrong-database".into(),
            })
            .is_err()
        );
        assert!(
            execute(IdbCommand::CommitTransaction {
                token: "aborting".into(),
                database: "wrong-database".into(),
            })
            .is_err()
        );
        execute(IdbCommand::CommitTransaction {
            token: "committing".into(),
            database: "scopes".into(),
        })
        .unwrap();
        execute(IdbCommand::AbortTransaction {
            token: "aborting".into(),
            database: "scopes".into(),
        })
        .unwrap();
        execute(IdbCommand::AbortTransaction {
            token: "reader".into(),
            database: "scopes".into(),
        })
        .unwrap();
        let right = execute(IdbCommand::Get {
            database: "scopes".into(),
            store: "right".into(),
            key: serde_json::json!(1),
        })
        .unwrap();
        assert!(
            matches!(right, StorageResponse::IndexedDb { response } if response.value == Some(serde_json::json!("retain")))
        );
        let left = execute(IdbCommand::Get {
            database: "scopes".into(),
            store: "left".into(),
            key: serde_json::json!(1),
        })
        .unwrap();
        assert!(
            matches!(left, StorageResponse::IndexedDb { response } if response.value.is_none())
        );
    }

    #[test]
    fn indexed_db_transactions_publish_or_discard_private_snapshots() {
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
        let origin = partitioned_idb_origin("context/site-a", "oryn:origin:https://example.test");
        let execute = |command| {
            let StorageResponse::IndexedDb { response } = store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command,
                })
                .unwrap()
            else {
                panic!("expected IndexedDB response");
            };
            response
        };
        let open = |version| IdbCommand::Open {
            name: "metadata".into(),
            version,
        };
        assert!(execute(open(1)).upgrade_needed);
        let mut definition = ObjectStoreDefinition {
            name: "items".into(),
            key_path: Some("id".into()),
            auto_increment: true,
            indexes: Vec::new(),
        };
        execute(IdbCommand::CreateObjectStore {
            database: "metadata".into(),
            definition: definition.clone(),
        });
        let index = IndexDefinition {
            name: "by_name".into(),
            key_path: "name".into(),
            unique: false,
            multi_entry: false,
        };
        execute(IdbCommand::CreateIndex {
            database: "metadata".into(),
            store: "items".into(),
            definition: index.clone(),
        });
        definition.indexes.push(index);

        // These are trusted native storage commands. A second open still must
        // wait for the existing versionchange transaction to finish.
        let pending = execute(open(1));
        assert!(pending.transaction_blocked, "{pending:?}");
        assert!(pending.object_store_definitions.is_empty());
        execute(IdbCommand::CommitUpgrade {
            name: "metadata".into(),
        });
        let reopened = execute(open(1));
        assert!(
            !reopened.transaction_blocked && !reopened.upgrade_needed,
            "{reopened:?}"
        );
        assert_eq!(reopened.version, Some(1));
        assert_eq!(reopened.object_stores, vec!["items"]);
        assert_eq!(reopened.object_store_definitions, vec![definition.clone()]);

        for (partition, authority) in [
            ("context/site-b", "oryn:origin:https://example.test"),
            ("context/site-a", "oryn:origin:https://other.test"),
        ] {
            let StorageResponse::IndexedDb { response } = store
                .execute(StorageRequest::IndexedDb {
                    origin: partitioned_idb_origin(partition, authority),
                    command: IdbCommand::ListDatabases,
                })
                .unwrap()
            else {
                panic!("expected IndexedDB response");
            };
            assert!(
                response.databases.is_empty(),
                "foreign bucket: {response:?}"
            );
        }

        assert!(execute(open(2)).upgrade_needed);
        execute(IdbCommand::DeleteIndex {
            database: "metadata".into(),
            store: "items".into(),
            index: "by_name".into(),
        });
        execute(IdbCommand::CommitUpgrade {
            name: "metadata".into(),
        });
        definition.indexes.clear();
        let reopened = execute(open(2));
        assert!(
            !reopened.transaction_blocked && !reopened.upgrade_needed,
            "{reopened:?}"
        );
        assert_eq!(reopened.version, Some(2));
        assert_eq!(reopened.object_store_definitions, vec![definition]);
    }

    #[test]
    fn indexed_db_store_rename_preserves_metadata_and_rolls_back() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let origin = partitioned_idb_origin("context/site-a", "oryn:origin:https://example.test");
        let execute = |command| {
            let StorageResponse::IndexedDb { response } = store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command,
                })
                .unwrap()
            else {
                panic!("expected IndexedDB response");
            };
            response
        };
        let open = |version| IdbCommand::Open {
            name: "rename".into(),
            version,
        };
        let rename = || IdbCommand::RenameObjectStore {
            database: "rename".into(),
            store: "books".into(),
            new_name: "renamed".into(),
        };
        let write = |store: &str, title: &str| IdbCommand::Batch {
            database: "rename".into(),
            writes: vec![IdbWrite {
                store: store.into(),
                key: None,
                value: serde_json::json!({"title": title}),
                add_only: true,
            }],
        };
        let mut definition = ObjectStoreDefinition {
            name: "books".into(),
            key_path: None,
            auto_increment: true,
            indexes: Vec::new(),
        };
        assert!(execute(open(1)).upgrade_needed);
        execute(IdbCommand::CreateObjectStore {
            database: "rename".into(),
            definition: definition.clone(),
        });
        let index = IndexDefinition {
            name: "by_title".into(),
            key_path: "title".into(),
            unique: true,
            multi_entry: false,
        };
        execute(IdbCommand::CreateIndex {
            database: "rename".into(),
            store: "books".into(),
            definition: index.clone(),
        });
        definition.indexes.push(index);
        assert_eq!(
            execute(write("books", "original")).key,
            Some(serde_json::json!(1))
        );
        execute(IdbCommand::CommitUpgrade {
            name: "rename".into(),
        });

        assert!(execute(open(2)).upgrade_needed);
        execute(rename());
        assert_eq!(
            execute(write("renamed", "discarded")).key,
            Some(serde_json::json!(2))
        );
        let pending = execute(open(2));
        assert!(pending.transaction_blocked, "{pending:?}");
        assert!(pending.object_store_definitions.is_empty());
        execute(IdbCommand::AbortUpgrade {
            name: "rename".into(),
            old_version: 1,
        });
        let restored = execute(open(1));
        assert!(
            !restored.transaction_blocked && !restored.upgrade_needed,
            "{restored:?}"
        );
        assert_eq!(restored.version, Some(1));
        assert_eq!(restored.object_stores, vec!["books"]);
        assert_eq!(restored.object_store_definitions, vec![definition.clone()]);
        assert_eq!(
            execute(IdbCommand::Get {
                database: "rename".into(),
                store: "books".into(),
                key: serde_json::json!(1),
            })
            .value,
            Some(serde_json::json!({"title": "original"}))
        );
        assert_eq!(
            execute(IdbCommand::Get {
                database: "rename".into(),
                store: "books".into(),
                key: serde_json::json!(2),
            })
            .value,
            None
        );

        // Observe renamed metadata only after a separate upgrade commits;
        // aborting the previous attempt must also have restored its generator.
        assert!(execute(open(2)).upgrade_needed);
        execute(rename());
        execute(IdbCommand::CommitUpgrade {
            name: "rename".into(),
        });
        definition.name = "renamed".into();
        let reopened = execute(open(2));
        assert!(
            !reopened.transaction_blocked && !reopened.upgrade_needed,
            "{reopened:?}"
        );
        assert_eq!(reopened.version, Some(2));
        assert_eq!(reopened.object_stores, vec!["renamed"]);
        assert_eq!(reopened.object_store_definitions, vec![definition]);
        assert_eq!(
            execute(write("renamed", "committed")).key,
            Some(serde_json::json!(2))
        );
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
    fn indexed_db_large_upgrade_checks_quota_once_and_rolls_back_atomically() {
        let store = SharedContextStore::memory(StorageLimits {
            indexed_db_bytes_per_origin: 1_024,
            ..StorageLimits::default()
        });
        let origin = "https://example.test".to_string();
        store
            .execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::Open {
                    name: "large-upgrade".into(),
                    version: 1,
                },
            })
            .unwrap();
        for index in 0..100 {
            store
                .execute(StorageRequest::IndexedDb {
                    origin: origin.clone(),
                    command: IdbCommand::CreateObjectStore {
                        database: "large-upgrade".into(),
                        definition: ObjectStoreDefinition {
                            name: format!("store-{index}"),
                            key_path: None,
                            auto_increment: false,
                            indexes: Vec::new(),
                        },
                    },
                })
                .expect("schema mutation remains staged until upgrade commit");
        }
        assert!(matches!(
            store.execute(StorageRequest::IndexedDb {
                origin: origin.clone(),
                command: IdbCommand::CommitUpgrade {
                    name: "large-upgrade".into(),
                },
            }),
            Err(StorageError::QuotaExceeded { .. })
        ));
        assert!(matches!(
            store
                .execute(StorageRequest::IndexedDb {
                    origin,
                    command: IdbCommand::ListDatabases,
                })
                .unwrap(),
            StorageResponse::IndexedDb { response } if response.databases.is_empty()
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
    fn indexed_db_failed_unique_write_does_not_advance_generator() {
        let index = IndexDefinition {
            name: "unique_value".into(),
            key_path: "value".into(),
            unique: true,
            multi_entry: false,
        };
        let mut store = ObjectStoreData {
            definition: Some(ObjectStoreDefinition {
                name: "items".into(),
                key_path: None,
                auto_increment: true,
                indexes: vec![index.clone()],
            }),
            indexes: BTreeMap::from([(index.name.clone(), index)]),
            ..Default::default()
        };
        let write = |key, value| IdbWrite {
            store: "items".into(),
            key,
            value: serde_json::json!({"value":value}),
            add_only: true,
        };
        assert_eq!(
            put_record(&mut store, write(None, "a")).unwrap(),
            serde_json::json!(1)
        );
        assert!(matches!(
            put_record(&mut store, write(None, "a")),
            Err(StorageError::Constraint)
        ));
        assert!(matches!(
            put_record(&mut store, write(Some(serde_json::json!(100)), "a")),
            Err(StorageError::Constraint)
        ));
        assert_eq!(
            put_record(&mut store, write(None, "b")).unwrap(),
            serde_json::json!(2)
        );
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
    fn multi_entry_indexes_extract_structured_clone_array_elements() {
        let definition = IndexDefinition {
            name: "tags".into(),
            key_path: "tags".into(),
            unique: false,
            multi_entry: true,
        };
        let value = serde_json::json!({"tags": {
            "__oryn_clone_type": "array",
            "values": ["vowel", null, 3, {"__oryn_clone_type":"array", "values":["nested", 1], "properties":{}}],
            "properties": {"extra":"not an array element"}
        }});
        assert_eq!(
            indexed_record_keys(&value, &definition),
            vec![
                serde_json::json!("vowel"),
                serde_json::json!(3),
                serde_json::json!(["nested", 1])
            ]
        );
        let array_key = serde_json::json!({"tags": {
            "__oryn_clone_type": "array", "values": ["a", 2], "properties": {}
        }});
        assert_eq!(
            indexed_record_keys(
                &array_key,
                &IndexDefinition {
                    multi_entry: false,
                    ..definition
                }
            ),
            vec![serde_json::json!(["a", 2])]
        );
    }

    #[test]
    fn indexed_db_unique_multi_entry_backfill_matches_later_writes() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let run = |command| {
            store.execute(StorageRequest::IndexedDb {
                origin: "https://example.test".into(),
                command,
            })
        };
        run(IdbCommand::Open {
            name: "unique".into(),
            version: 1,
        })
        .unwrap();
        run(IdbCommand::CreateObjectStore {
            database: "unique".into(),
            definition: ObjectStoreDefinition {
                name: "items".into(),
                key_path: None,
                auto_increment: false,
                indexes: Vec::new(),
            },
        })
        .unwrap();
        let put = |key, value| {
            run(IdbCommand::Batch {
                database: "unique".into(),
                writes: vec![IdbWrite {
                    store: "items".into(),
                    key: Some(key),
                    value,
                    add_only: false,
                }],
            })
        };
        put(serde_json::json!(1), serde_json::json!({"tags":[1,1.0]})).unwrap();
        run(IdbCommand::CreateIndex {
            database: "unique".into(),
            store: "items".into(),
            definition: IndexDefinition {
                name: "tags".into(),
                key_path: "tags".into(),
                unique: true,
                multi_entry: true,
            },
        })
        .unwrap();
        put(serde_json::json!(1), serde_json::json!({"tags":[1,1]})).unwrap();
        assert!(matches!(
            put(serde_json::json!(2), serde_json::json!({"tags":[1]})),
            Err(StorageError::Constraint)
        ));
        put(
            serde_json::json!(2),
            serde_json::json!({"tags":[{"__oryn_clone_type":"date","value":1}]}),
        )
        .unwrap();
    }

    #[test]
    fn indexed_db_generator_ignores_date_keys_and_exhausts_at_infinity() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let run = |command| {
            store.execute(StorageRequest::IndexedDb {
                origin: "https://example.test".into(),
                command,
            })
        };
        run(IdbCommand::Open {
            name: "generator".into(),
            version: 1,
        })
        .unwrap();
        run(IdbCommand::CreateObjectStore {
            database: "generator".into(),
            definition: ObjectStoreDefinition {
                name: "items".into(),
                key_path: None,
                auto_increment: true,
                indexes: Vec::new(),
            },
        })
        .unwrap();
        let put = |key| {
            run(IdbCommand::Batch {
                database: "generator".into(),
                writes: vec![IdbWrite {
                    store: "items".into(),
                    key,
                    value: serde_json::json!("value"),
                    add_only: true,
                }],
            })
        };
        put(Some(
            serde_json::json!({"__oryn_idb_key_type":"date","value":500}),
        ))
        .unwrap();
        assert!(
            matches!(put(None).unwrap(),StorageResponse::IndexedDb {response} if response.key==Some(serde_json::json!(1)))
        );
        put(Some(
            serde_json::json!({"__oryn_idb_key_type":"number","value":"positive_infinity"}),
        ))
        .unwrap();
        assert!(matches!(put(None), Err(StorageError::Constraint)));
    }

    #[test]
    fn indexed_db_key_transport_preserves_types_order_and_zero_identity() {
        let date = serde_json::json!({"__oryn_idb_key_type":"date","value":1});
        assert!(
            encode_key(&date).is_ok(),
            "Date keys must survive storage transport"
        );
        assert_ne!(
            encode_key(&date).unwrap(),
            encode_key(&serde_json::json!(1)).unwrap()
        );
        assert_eq!(
            encode_key(&serde_json::json!(-0.0)).unwrap(),
            encode_key(&serde_json::json!(0)).unwrap()
        );
        let keys = [
            serde_json::json!({"__oryn_idb_key_type":"number","value":"negative_infinity"}),
            serde_json::json!(-2),
            serde_json::json!(1),
            serde_json::json!({"__oryn_idb_key_type":"number","value":"positive_infinity"}),
            date,
            serde_json::json!("a"),
            serde_json::json!({"__oryn_idb_key_type":"binary","bytes":[0]}),
            serde_json::json!([]),
        ];
        for pair in keys.windows(2) {
            assert!(
                compare_indexed_db_keys(&pair[0], &pair[1]).is_lt(),
                "{pair:?}"
            );
        }
        assert_eq!(
            json_value_at_key_path(&serde_json::json!({"text":"💡"}), "text.length"),
            Some(serde_json::json!(2))
        );
        assert_eq!(
            json_value_at_key_path(
                &serde_json::json!({"__oryn_clone_type":"array","values":[1,2],"properties":{}}),
                "length"
            ),
            Some(serde_json::json!(2))
        );
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
        assert!(indexed_db_key_matches(
            &serde_json::json!(1),
            &serde_json::Value::Number(serde_json::Number::from_f64(1.0).unwrap())
        ));
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

/// Internal IndexedDB bucket key. Inputs must come from the parent authority
/// registry, after opaque-origin rejection; author-facing origin strings never
/// directly select this key. JSON tuple framing prevents delimiter collisions.
pub(crate) fn partitioned_idb_origin(partition: &str, authority: &str) -> String {
    format!(
        "oryn:idb-partition:v1:{}",
        serde_json::to_string(&(partition, authority)).expect("string tuple serialization")
    )
}

#[cfg(test)]
mod partitioned_idb_tests {
    use super::*;
    #[test]
    fn indexeddb_bucket_shares_matching_native_partition_and_separates_site_and_origin() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let key = partitioned_idb_origin("context/site-a", "oryn:origin:https://frame.test");
        store
            .execute(StorageRequest::IndexedDb {
                origin: key.clone(),
                command: IdbCommand::Open {
                    name: "shared".into(),
                    version: 1,
                },
            })
            .unwrap();
        store
            .execute(StorageRequest::IndexedDb {
                origin: key.clone(),
                command: IdbCommand::CommitUpgrade {
                    name: "shared".into(),
                },
            })
            .unwrap();
        for (partition, origin, expected) in [
            ("context/site-a", "oryn:origin:https://frame.test", 1),
            ("context/site-b", "oryn:origin:https://frame.test", 0),
            ("context/site-a", "oryn:origin:https://other.test", 0),
            ("context/site-a/oryn:origin:", "https://frame.test", 0),
        ] {
            let StorageResponse::IndexedDb { response } = store
                .execute(StorageRequest::IndexedDb {
                    origin: partitioned_idb_origin(partition, origin),
                    command: IdbCommand::ListDatabases,
                })
                .unwrap()
            else {
                panic!("IndexedDB response")
            };
            assert_eq!(response.databases.len(), expected);
        }
    }
}

#[cfg(test)]
mod native_transaction_authority_tests {
    use super::*;
    fn scoped(principal: &str, token: &str) -> String {
        let mut command = IdbCommand::AbortTransaction {
            token: token.into(),
            database: "shared".into(),
        };
        scope_idb_transaction(&mut command, principal);
        let IdbCommand::AbortTransaction { token, .. } = command else {
            unreachable!()
        };
        token
    }
    #[test]
    fn native_transaction_identity_cannot_be_forged_by_matching_author_tokens() {
        let agent = scoped("agent:1", "same");
        assert_ne!(agent, scoped("agent:2", "same"));
        assert_ne!(agent, scoped("window:1", "same"));
        assert_ne!(agent, scoped("window:1", &agent));
        assert_ne!(scoped("a:b", "c"), scoped("a", "b:c"));
        assert_eq!(agent, scoped("agent:1", "same"));
    }
}

#[cfg(test)]
mod storage_commit_fault_regressions {
    use super::*;
    #[test]
    fn rejected_session_write_preserves_all_existing_pages() {
        let scope = StorageScope {
            origin: "https://a.example".into(),
            top_level_page_id: PageId(1),
        };
        let mut session = BTreeMap::from([
            (
                (PageId(1), scope.origin.clone(), "old".into()),
                "value".into(),
            ),
            (
                (PageId(2), scope.origin.clone(), "other".into()),
                "page".into(),
            ),
        ]);
        let before = session.clone();
        let limits = StorageLimits {
            web_storage_bytes_per_origin: 1,
            ..Default::default()
        };
        assert!(
            execute_session(
                &mut session,
                StorageRequest::Set {
                    area: StorageArea::Session,
                    scope,
                    key: "new".into(),
                    value: "too-large".into()
                },
                limits
            )
            .is_err()
        );
        assert_eq!(session, before);
    }
    #[test]
    fn persistent_commit_failure_retains_private_transaction_until_retry_commits() {
        let path = std::env::temp_dir().join(format!(
            "oryn-idb-commit-fault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let store = PersistentContextStore::new(
                profile.database.clone(),
                profile.cache_coordinator.clone(),
                profile.service_jobs.clone(),
                profile.idb_mutations.clone(),
                profile.idb_transactions.clone(),
                profile.idb_connections.clone(),
                StorageLimits::default(),
            );
            let origin = "https://a.example";
            let key = (origin.to_string(), "db".to_string());
            let mut database = DatabaseData::default();
            database.stores.insert(
                "store".into(),
                ObjectStoreData {
                    next_key: 42,
                    ..Default::default()
                },
            );
            store
                .save_idb(origin, BTreeMap::from([(key.clone(), database)]))
                .unwrap();
            let mut working = store.load_idb(origin).unwrap().remove(&key).unwrap();
            working.stores.get_mut("store").unwrap().next_key = 7;
            store.indexed_db_transactions.lock().unwrap().insert(
                "owner-token".into(),
                TransactionSnapshot {
                    origin: origin.into(),
                    database: "db".into(),
                    scope: vec!["store".into()],
                    mode: "readwrite".into(),
                    order: 1,
                    connection: None,
                    working: Some(working),
                    dirty: BTreeSet::from(["store".into()]),
                },
            );
            let commit = || StorageRequest::IndexedDb {
                origin: origin.into(),
                command: IdbCommand::CommitTransaction {
                    token: "owner-token".into(),
                    database: "db".into(),
                },
            };
            store
                .fail_next_idb_commit
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(store.execute(commit()).is_err());
            assert!(
                store
                    .indexed_db_transactions
                    .lock()
                    .unwrap()
                    .contains_key("owner-token")
            );
            assert_eq!(
                store.load_idb(origin).unwrap()[&key].stores["store"].next_key,
                42
            );
            store.execute(commit()).unwrap();
            assert!(
                !store
                    .indexed_db_transactions
                    .lock()
                    .unwrap()
                    .contains_key("owner-token")
            );
            assert_eq!(
                store.load_idb(origin).unwrap()[&key].stores["store"].next_key,
                7
            );
            // Separate contexts use the same profile mutation lock, including
            // through a cloned profile handle.
            let cloned = profile.clone();
            let sibling = PersistentContextStore::new(
                cloned.database.clone(),
                cloned.cache_coordinator.clone(),
                cloned.service_jobs.clone(),
                cloned.idb_mutations.clone(),
                cloned.idb_transactions.clone(),
                cloned.idb_connections.clone(),
                StorageLimits::default(),
            );
            assert!(Arc::ptr_eq(&store.idb_mutations, &sibling.idb_mutations));
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod idb_scope_admission_regressions {
    use super::*;
    fn execute(store: &SharedContextStore, command: IdbCommand) -> IdbResponse {
        let StorageResponse::IndexedDb { response } = store
            .execute(StorageRequest::IndexedDb {
                origin: "https://a.example".into(),
                command,
            })
            .unwrap()
        else {
            panic!("IDB response")
        };
        response
    }
    fn begin(token: &str, scope: &str, mode: &str) -> IdbCommand {
        IdbCommand::BeginTransaction {
            token: token.into(),
            database: "db".into(),
            scope: vec![scope.into()],
            mode: mode.into(),
            connection: None,
        }
    }
    fn write(token: &str, store: &str, value: &str) -> IdbCommand {
        IdbCommand::Transaction {
            token: token.into(),
            database: "db".into(),
            inner: Box::new(IdbCommand::Batch {
                database: "db".into(),
                writes: vec![IdbWrite {
                    store: store.into(),
                    key: Some(serde_json::json!(1)),
                    value: serde_json::json!(value),
                    add_only: false,
                }],
            }),
        }
    }
    fn read(token: &str, store: &str) -> IdbCommand {
        IdbCommand::Transaction {
            token: token.into(),
            database: "db".into(),
            inner: Box::new(IdbCommand::Get {
                database: "db".into(),
                store: store.into(),
                key: serde_json::json!(1),
            }),
        }
    }
    fn finish(token: &str, commit: bool) -> IdbCommand {
        if commit {
            IdbCommand::CommitTransaction {
                token: token.into(),
                database: "db".into(),
            }
        } else {
            IdbCommand::AbortTransaction {
                token: token.into(),
                database: "db".into(),
            }
        }
    }
    #[test]
    fn profile_transactions_stage_privately_and_admit_conflicts_in_order() {
        let path = std::env::temp_dir().join(format!(
            "oryn-idb-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let first = SharedContextStore::persistent(&profile, StorageLimits::default());
            let second = SharedContextStore::persistent(&profile.clone(), StorageLimits::default());
            execute(
                &first,
                IdbCommand::Open {
                    name: "db".into(),
                    version: 1,
                },
            );
            for name in ["left", "right"] {
                execute(
                    &first,
                    IdbCommand::CreateObjectStore {
                        database: "db".into(),
                        definition: ObjectStoreDefinition {
                            name: name.into(),
                            key_path: None,
                            auto_increment: false,
                            indexes: Vec::new(),
                        },
                    },
                );
            }
            execute(&first, IdbCommand::CommitUpgrade { name: "db".into() });
            assert!(!execute(&first, begin("writer", "left", "readwrite")).transaction_blocked);
            execute(&first, write("writer", "left", "uncommitted"));
            assert!(
                execute(
                    &second,
                    IdbCommand::Open {
                        name: "db".into(),
                        version: 2
                    }
                )
                .transaction_blocked
            );
            assert!(
                first
                    .execute(StorageRequest::IndexedDb {
                        origin: "https://a.example".into(),
                        command: begin("writer", "right", "readwrite")
                    })
                    .is_err(),
                "token cannot widen its declared scope"
            );
            assert!(execute(&second, begin("reader", "left", "readonly")).transaction_blocked);
            assert!(
                !execute(&second, begin("independent", "right", "readwrite")).transaction_blocked
            );
            execute(&second, write("independent", "right", "survives"));
            execute(&second, finish("independent", true));
            let observed = execute(
                &second,
                IdbCommand::Get {
                    database: "db".into(),
                    store: "left".into(),
                    key: serde_json::json!(1),
                },
            );
            assert!(
                observed.value.is_none(),
                "private transaction writes must never reach persistent storage"
            );
            execute(&first, finish("writer", false));
            assert!(!execute(&second, begin("reader", "left", "readonly")).transaction_blocked);
            assert!(execute(&second, read("reader", "left")).value.is_none());
            assert!(execute(&first, begin("later", "left", "readwrite")).transaction_blocked);
            let denied = second.execute(StorageRequest::IndexedDb {
                origin: "https://a.example".into(),
                command: write("reader", "left", "forbidden"),
            });
            assert!(denied.is_err());
            execute(&second, finish("reader", true));
            assert!(!execute(&first, begin("later", "left", "readwrite")).transaction_blocked);
            execute(&first, write("later", "left", "committed"));
            execute(&first, finish("later", true));
            assert_eq!(
                execute(
                    &second,
                    IdbCommand::Get {
                        database: "db".into(),
                        store: "left".into(),
                        key: serde_json::json!(1)
                    }
                )
                .value,
                Some(serde_json::json!("committed"))
            );
            assert_eq!(
                execute(
                    &second,
                    IdbCommand::Get {
                        database: "db".into(),
                        store: "right".into(),
                        key: serde_json::json!(1)
                    }
                )
                .value,
                Some(serde_json::json!("survives"))
            );
            assert!(profile.idb_transactions.lock().unwrap().is_empty());
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod idb_connection_delivery_regressions {
    use super::*;
    fn execute(store: &SharedContextStore, command: IdbCommand) -> IdbResponse {
        let StorageResponse::IndexedDb { response } = store
            .execute(StorageRequest::IndexedDb {
                origin: "https://a.example".into(),
                command,
            })
            .unwrap()
        else {
            panic!()
        };
        response
    }
    fn open(token: &str, version: u64) -> IdbCommand {
        IdbCommand::OpenConnection {
            token: token.into(),
            name: "db".into(),
            version,
        }
    }
    fn upgrade(token: &str, inner: IdbCommand) -> IdbCommand {
        IdbCommand::UpgradeConnection {
            token: token.into(),
            name: "db".into(),
            inner: Box::new(inner),
        }
    }
    fn schema(name: &str) -> IdbCommand {
        IdbCommand::CreateObjectStore {
            database: "db".into(),
            definition: ObjectStoreDefinition {
                name: name.into(),
                key_path: None,
                auto_increment: false,
                indexes: Vec::new(),
            },
        }
    }
    fn close_pending_and_forced_retirement(store: &SharedContextStore) {
        execute(store, open("first", 1));
        execute(store, upgrade("first", schema("records")));
        execute(
            store,
            upgrade("first", IdbCommand::CommitUpgrade { name: "db".into() }),
        );
        execute(store, open("sibling", 1));
        let begin = |token: &str, connection: &str| IdbCommand::BeginTransaction {
            token: token.into(),
            database: "db".into(),
            scope: vec!["records".into()],
            mode: "readwrite".into(),
            connection: Some(connection.into()),
        };
        assert!(!execute(store, begin("writer", "first")).transaction_blocked);
        execute(
            store,
            IdbCommand::Transaction {
                token: "writer".into(),
                database: "db".into(),
                inner: Box::new(IdbCommand::Batch {
                    database: "db".into(),
                    writes: vec![IdbWrite {
                        store: "records".into(),
                        key: Some(serde_json::json!(1)),
                        value: serde_json::json!("uncommitted"),
                        add_only: false,
                    }],
                }),
            },
        );
        assert!(execute(store, begin("queued", "sibling")).transaction_blocked);
        execute(
            store,
            IdbCommand::CloseConnection {
                token: "first".into(),
                name: "db".into(),
            },
        );
        let waiting = execute(store, open("upgrade", 2));
        assert!(waiting.transaction_blocked);
        assert!(
            execute(
                store,
                IdbCommand::PollConnection {
                    token: "first".into(),
                    name: "db".into()
                }
            )
            .connection_events
            .is_empty(),
            "close-pending connections receive no versionchange task"
        );
        let notice = execute(
            store,
            IdbCommand::PollConnection {
                token: "sibling".into(),
                name: "db".into(),
            },
        )
        .connection_events
        .remove(0);
        execute(
            store,
            IdbCommand::AcknowledgeConnection {
                token: "sibling".into(),
                name: "db".into(),
                event: notice.event,
            },
        );
        assert!(
            execute(store, open("upgrade", 2)).connection_blocked,
            "a closing connection needs no impossible notification acknowledgement"
        );
        let foreign = store.execute(StorageRequest::IndexedDb {
            origin: "https://foreign.example".into(),
            command: IdbCommand::RetireConnection {
                token: "first".into(),
                name: "db".into(),
            },
        });
        assert!(foreign.is_err());
        assert!(
            execute(store, begin("queued", "sibling")).transaction_blocked,
            "foreign cleanup preserves the original lock"
        );
        execute(
            store,
            IdbCommand::RetireConnection {
                token: "first".into(),
                name: "db".into(),
            },
        );
        assert!(
            !execute(store, begin("queued", "sibling")).transaction_blocked,
            "forced retirement aborts original snapshots and unblocks the sibling"
        );
        execute(
            store,
            IdbCommand::RetireConnection {
                token: "first".into(),
                name: "db".into(),
            },
        );
        let visible = execute(
            store,
            IdbCommand::Transaction {
                token: "queued".into(),
                database: "db".into(),
                inner: Box::new(IdbCommand::Get {
                    database: "db".into(),
                    store: "records".into(),
                    key: serde_json::json!(1),
                }),
            },
        );
        assert_eq!(
            visible.value, None,
            "retirement cannot commit uncommitted writes"
        );
        execute(
            store,
            IdbCommand::CommitTransaction {
                token: "queued".into(),
                database: "db".into(),
            },
        );
        execute(
            store,
            IdbCommand::CloseConnection {
                token: "sibling".into(),
                name: "db".into(),
            },
        );
        assert!(execute(store, open("upgrade", 2)).upgrade_needed);
        execute(
            store,
            IdbCommand::RetireConnection {
                token: "upgrade".into(),
                name: "db".into(),
            },
        );
    }
    #[test]
    fn close_pending_notifications_and_forced_retirement_preserve_exact_connection_ownership() {
        close_pending_and_forced_retirement(&SharedContextStore::memory(StorageLimits::default()));
        let path = std::env::temp_dir().join(format!(
            "oryn-idb-forced-close-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            close_pending_and_forced_retirement(&SharedContextStore::persistent(
                &profile,
                StorageLimits::default(),
            ));
        }
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let store = SharedContextStore::persistent(&profile, StorageLimits::default());
            let restored = execute(&store, open("restart", 1));
            assert!(!restored.upgrade_needed);
            assert_eq!(restored.object_stores, vec!["records"]);
            assert_eq!(
                execute(
                    &store,
                    IdbCommand::Get {
                        database: "db".into(),
                        store: "records".into(),
                        key: serde_json::json!(1)
                    }
                )
                .value,
                None
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn profile_versionchange_waits_for_delivery_ack_and_close_pending_transactions() {
        let path = std::env::temp_dir().join(format!(
            "oryn-idb-connections-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let first = SharedContextStore::persistent(&profile, StorageLimits::default());
            let second = SharedContextStore::persistent(&profile.clone(), StorageLimits::default());
            assert!(execute(&first, open("first", 1)).upgrade_needed);
            execute(&first, upgrade("first", schema("records")));
            assert!(
                execute(&second, IdbCommand::ListDatabases)
                    .databases
                    .is_empty(),
                "uncommitted schema is private"
            );
            execute(
                &first,
                upgrade("first", IdbCommand::CommitUpgrade { name: "db".into() }),
            );
            let waiting = execute(&second, open("next", 2));
            assert!(waiting.transaction_blocked);
            assert!(!waiting.connection_blocked);
            let poll = || IdbCommand::PollConnection {
                token: "first".into(),
                name: "db".into(),
            };
            let notices = execute(&first, poll()).connection_events;
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].old_version, 1);
            assert_eq!(notices[0].new_version, Some(2));
            assert_eq!(
                execute(&first, poll()).connection_events,
                notices,
                "unacknowledged notice retains custody"
            );
            execute(
                &first,
                IdbCommand::AcknowledgeConnection {
                    token: "first".into(),
                    name: "db".into(),
                    event: notices[0].event,
                },
            );
            assert!(execute(&second, open("next", 2)).connection_blocked);
            execute(
                &first,
                IdbCommand::BeginTransaction {
                    token: "reader".into(),
                    database: "db".into(),
                    scope: vec!["records".into()],
                    mode: "readonly".into(),
                    connection: Some("first".into()),
                },
            );
            execute(
                &first,
                IdbCommand::CloseConnection {
                    token: "first".into(),
                    name: "db".into(),
                },
            );
            assert!(execute(&second, open("next", 2)).transaction_blocked);
            execute(
                &first,
                IdbCommand::CommitTransaction {
                    token: "reader".into(),
                    database: "db".into(),
                },
            );
            assert!(execute(&second, open("next", 2)).upgrade_needed);
            execute(&second, upgrade("next", schema("discarded")));
            assert_eq!(
                execute(&first, IdbCommand::ListDatabases).databases[0].version,
                1
            );
            execute(
                &second,
                IdbCommand::RetireConnection {
                    token: "next".into(),
                    name: "db".into(),
                },
            );
            let restored = execute(&first, open("restored", 1));
            assert!(!restored.upgrade_needed);
            assert_eq!(restored.object_stores, vec!["records"]);
            let foreign = first.execute(StorageRequest::IndexedDb {
                origin: "https://foreign.example".into(),
                command: IdbCommand::PollConnection {
                    token: "restored".into(),
                    name: "db".into(),
                },
            });
            assert!(foreign.is_err());
            execute(
                &first,
                IdbCommand::CloseConnection {
                    token: "restored".into(),
                    name: "db".into(),
                },
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod private_upgrade_commit_fault_regressions {
    use super::*;
    #[test]
    fn private_upgrade_commit_failure_keeps_schema_out_of_disk_until_retry() {
        let path = std::env::temp_dir().join(format!(
            "oryn-idb-upgrade-fault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&path).unwrap();
            let store = PersistentContextStore::new(
                profile.database.clone(),
                profile.cache_coordinator.clone(),
                profile.service_jobs.clone(),
                profile.idb_mutations.clone(),
                profile.idb_transactions.clone(),
                profile.idb_connections.clone(),
                StorageLimits::default(),
            );
            let execute = |command| {
                store.execute(StorageRequest::IndexedDb {
                    origin: "https://a.example".into(),
                    command,
                })
            };
            execute(IdbCommand::OpenConnection {
                token: "owner".into(),
                name: "db".into(),
                version: 1,
            })
            .unwrap();
            execute(IdbCommand::UpgradeConnection {
                token: "owner".into(),
                name: "db".into(),
                inner: Box::new(IdbCommand::CreateObjectStore {
                    database: "db".into(),
                    definition: ObjectStoreDefinition {
                        name: "private".into(),
                        key_path: None,
                        auto_increment: false,
                        indexes: Vec::new(),
                    },
                }),
            })
            .unwrap();
            let commit = || IdbCommand::UpgradeConnection {
                token: "owner".into(),
                name: "db".into(),
                inner: Box::new(IdbCommand::CommitUpgrade { name: "db".into() }),
            };
            assert!(store.load_idb("https://a.example").unwrap().is_empty());
            store
                .fail_next_idb_commit
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(execute(commit()).is_err());
            assert!(store.load_idb("https://a.example").unwrap().is_empty());
            execute(commit()).unwrap();
            let data = store.load_idb("https://a.example").unwrap();
            assert!(
                data[&("https://a.example".into(), "db".into())]
                    .stores
                    .contains_key("private")
            );
            execute(IdbCommand::RetireConnection {
                token: "owner".into(),
                name: "db".into(),
            })
            .unwrap();
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod script_idb_protocol_regressions {
    use super::*;
    #[test]
    fn scoped_script_requests_require_admission_and_cannot_revive_committed_transactions() {
        let store = SharedContextStore::memory(StorageLimits::default());
        let execute = |mut command: IdbCommand| {
            scope_idb_transaction(&mut command, "window-document:original");
            store.execute(StorageRequest::IndexedDb {
                origin: "https://a.example".into(),
                command,
            })
        };
        execute(IdbCommand::OpenConnection {
            token: "c".into(),
            name: "db".into(),
            version: 1,
        })
        .unwrap();
        execute(IdbCommand::UpgradeConnection {
            token: "c".into(),
            name: "db".into(),
            inner: Box::new(IdbCommand::CreateObjectStore {
                database: "db".into(),
                definition: ObjectStoreDefinition {
                    name: "records".into(),
                    key_path: None,
                    auto_increment: false,
                    indexes: Vec::new(),
                },
            }),
        })
        .unwrap();
        execute(IdbCommand::UpgradeConnection {
            token: "c".into(),
            name: "db".into(),
            inner: Box::new(IdbCommand::CommitUpgrade { name: "db".into() }),
        })
        .unwrap();
        let read = || IdbCommand::Transaction {
            token: "t".into(),
            database: "db".into(),
            inner: Box::new(IdbCommand::Get {
                database: "db".into(),
                store: "records".into(),
                key: serde_json::json!(1),
            }),
        };
        assert!(
            execute(read())
                .unwrap_err()
                .to_string()
                .contains("TransactionInactiveError")
        );
        execute(IdbCommand::BeginTransaction {
            token: "t".into(),
            database: "db".into(),
            scope: vec!["records".into()],
            mode: "readonly".into(),
            connection: Some("c".into()),
        })
        .unwrap();
        execute(read()).unwrap();
        execute(IdbCommand::CommitTransaction {
            token: "t".into(),
            database: "db".into(),
        })
        .unwrap();
        assert!(
            execute(read())
                .unwrap_err()
                .to_string()
                .contains("TransactionInactiveError")
        );
        execute(IdbCommand::RetireConnection {
            token: "c".into(),
            name: "db".into(),
        })
        .unwrap();
        assert!(
            execute(IdbCommand::BeginTransaction {
                token: "t".into(),
                database: "db".into(),
                scope: vec!["records".into()],
                mode: "readonly".into(),
                connection: Some("c".into())
            })
            .is_err()
        );
    }
    #[test]
    fn script_routes_reject_legacy_unowned_database_mutations() {
        for command in [
            IdbCommand::Open {
                name: "db".into(),
                version: 1,
            },
            IdbCommand::DeleteDatabase { name: "db".into() },
            IdbCommand::CommitUpgrade { name: "db".into() },
            IdbCommand::Clear {
                database: "db".into(),
                store: "s".into(),
            },
            IdbCommand::BeginTransaction {
                token: "t".into(),
                database: "db".into(),
                scope: vec!["s".into()],
                mode: "readonly".into(),
                connection: None,
            },
        ] {
            assert!(validate_script_idb_command(&command).is_err());
        }
        assert!(
            validate_script_idb_command(&IdbCommand::OpenConnection {
                token: "c".into(),
                name: "db".into(),
                version: 1
            })
            .is_ok()
        );
        assert!(
            validate_script_idb_command(&IdbCommand::BeginTransaction {
                token: "t".into(),
                database: "db".into(),
                scope: vec!["s".into()],
                mode: "readonly".into(),
                connection: Some("c".into())
            })
            .is_ok()
        );
    }
}

#[cfg(test)]
mod event_admission_tests {
    use super::*;
    fn scope() -> StorageScope {
        StorageScope {
            origin: "https://events.example".into(),
            top_level_page_id: PageId(7),
        }
    }
    fn verify(store: &SharedContextStore) {
        for area in [StorageArea::Local, StorageArea::Session] {
            store
                .execute(StorageRequest::Set {
                    area,
                    scope: scope(),
                    key: "key".into(),
                    value: "before".into(),
                })
                .unwrap();
            for request in [
                StorageRequest::Set {
                    area,
                    scope: scope(),
                    key: "key".into(),
                    value: "after".into(),
                },
                StorageRequest::Remove {
                    area,
                    scope: scope(),
                    key: "key".into(),
                },
                StorageRequest::Clear {
                    area,
                    scope: scope(),
                },
            ] {
                let mut admitted = 0;
                let result = store.execute_with_admission(request, &mut |change| {
                    admitted += 1;
                    assert_eq!(change.area, area);
                    Err(StorageError::Persistent(
                        "injected notification backpressure".into(),
                    ))
                });
                assert!(result.is_err());
                assert_eq!(admitted, 1);
                assert!(
                    matches!(store.execute(StorageRequest::Get{area,scope:scope(),key:"key".into()}).unwrap(),StorageResponse::Value{value:Some(value)}if value=="before")
                );
            }
            let response = store
                .execute_with_admission(
                    StorageRequest::Set {
                        area,
                        scope: scope(),
                        key: "key".into(),
                        value: "before".into(),
                    },
                    &mut |_| panic!("unchanged write cannot reserve an event"),
                )
                .unwrap();
            assert!(matches!(response, StorageResponse::Unchanged));
            let mut accepted = None;
            store
                .execute_with_admission(
                    StorageRequest::Set {
                        area,
                        scope: scope(),
                        key: "key".into(),
                        value: "after".into(),
                    },
                    &mut |change| {
                        accepted = Some(change.clone());
                        Ok(())
                    },
                )
                .unwrap();
            let change = accepted.unwrap();
            assert_eq!(change.old_value.as_deref(), Some("before"));
            assert_eq!(change.new_value.as_deref(), Some("after"));
        }
    }
    #[test]
    fn memory_backpressure_preserves_all_web_storage_mutations() {
        verify(&SharedContextStore::memory(StorageLimits::default()));
    }
    #[cfg(unix)]
    #[test]
    fn failed_persistent_commit_cancels_accepted_event_without_publication() {
        let path = std::env::temp_dir().join(format!(
            "oryn-event-commit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = PersistentProfile::open(&path).unwrap();
        let store = SharedContextStore::persistent(&profile, StorageLimits::default());
        store
            .execute(StorageRequest::Set {
                area: StorageArea::Local,
                scope: scope(),
                key: "key".into(),
                value: "old".into(),
            })
            .unwrap();
        let mut journal = crate::context_event_journal::Journal::default();
        let mut admitted = None;
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        FAIL_WEB_STORAGE_COMMIT.with(|failure| failure.set(true));
        let result = store.execute_with_admission(
            StorageRequest::Set {
                area: StorageArea::Local,
                scope: scope(),
                key: "key".into(),
                value: "new".into(),
            },
            &mut |change| {
                admitted = Some(
                    journal
                        .prepare(vec![crate::context_service::ContextEvent::Storage {
                            source_page_id: PageId(1),
                            source_creator: creator.clone(),
                            source_url: "https://events.example/writer".into(),
                            targets: vec![(PageId(2), creator.clone())],
                            change: change.clone(),
                        }])
                        .map_err(StorageError::NotificationCapacity)?,
                );
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(admitted.is_some());
        drop(admitted);
        assert!(journal.is_empty());
        assert_eq!(journal.evidence().last_published_id, 0);
        assert!(
            matches!(store.execute(StorageRequest::Get{area:StorageArea::Local,scope:scope(),key:"key".into()}).unwrap(),StorageResponse::Value{value:Some(value)}if value=="old")
        );
        drop(store);
        drop(profile);
        let _ = std::fs::remove_dir_all(path);
    }
    #[test]
    fn persistent_backpressure_preserves_all_web_storage_mutations() {
        let path = std::env::temp_dir().join(format!(
            "oryn-event-admission-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile = PersistentProfile::open(&path).unwrap();
        verify(&SharedContextStore::persistent(
            &profile,
            StorageLimits::default(),
        ));
        drop(profile);
        let _ = std::fs::remove_dir_all(path);
    }
}
