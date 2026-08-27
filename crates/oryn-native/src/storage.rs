//! BrowserContext-owned storage for G3.
//!
//! Page workers never open this database. They access it only through the
//! typed context service, keeping filesystem access in the privileged parent.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use oryn_common::v3::PageId;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use reqwest::{Url, cookie::CookieStore, header::HeaderValue};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub key_path: String,
    pub unique: bool,
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
    Open {
        name: String,
        version: u64,
    },
    DeleteDatabase {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdbResponse {
    pub version: Option<u64>,
    pub old_version: Option<u64>,
    pub upgrade_needed: bool,
    pub object_stores: Vec<String>,
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
    cookies: Arc<PersistentCookieStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCookie {
    url: String,
    set_cookie: String,
}

struct PersistentCookieStore {
    jar: reqwest::cookie::Jar,
    database: Arc<Database>,
    records: Mutex<Vec<StoredCookie>>,
}

impl PersistentCookieStore {
    fn open(database: Arc<Database>) -> Result<Self, StorageError> {
        let records: Vec<StoredCookie> = {
            let read = database
                .begin_read()
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            let table = read
                .open_table(PERSISTENT_COOKIES)
                .map_err(|error| StorageError::Persistent(error.to_string()))?;
            table
                .get("state")
                .map_err(|error| StorageError::Persistent(error.to_string()))?
                .map(|value| serde_json::from_slice(value.value()))
                .transpose()
                .map_err(|error| StorageError::Persistent(error.to_string()))?
                .unwrap_or_default()
        };
        let jar = reqwest::cookie::Jar::default();
        for record in &records {
            if let Ok(url) = Url::parse(&record.url) {
                jar.add_cookie_str(&record.set_cookie, &url);
            }
        }
        Ok(Self {
            jar,
            database,
            records: Mutex::new(records),
        })
    }

    fn persist(&self) {
        let Ok(records) = self.records.lock() else {
            return;
        };
        let Ok(encoded) = serde_json::to_vec(&*records) else {
            return;
        };
        let Ok(write) = self.database.begin_write() else {
            return;
        };
        if let Ok(mut table) = write.open_table(PERSISTENT_COOKIES) {
            let _ = table.insert("state", encoded.as_slice());
        }
        let _ = write.commit();
    }
}

impl CookieStore for PersistentCookieStore {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let headers = cookie_headers.cloned().collect::<Vec<_>>();
        self.jar.set_cookies(&mut headers.iter(), url);
        if let Ok(mut records) = self.records.lock() {
            records.extend(headers.iter().filter_map(|header| {
                header.to_str().ok().map(|set_cookie| StoredCookie {
                    url: url.to_string(),
                    set_cookie: set_cookie.to_string(),
                })
            }));
        }
        self.persist();
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        self.jar.cookies(url)
    }
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
        let cookies = Arc::new(PersistentCookieStore::open(database.clone())?);
        Ok(Self {
            path: path.to_path_buf(),
            database,
            cookies,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn cookie_store(&self) -> Arc<dyn CookieStore> {
        self.cookies.clone()
    }
}

#[derive(Default)]
struct MemoryState {
    local: BTreeMap<(String, String), String>,
    session: BTreeMap<(PageId, String, String), String>,
    indexed_db: BTreeMap<(String, String), DatabaseData>,
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
                let response = execute_idb(
                    &mut state.indexed_db,
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
    limits: StorageLimits,
}

impl PersistentContextStore {
    fn new(database: Arc<Database>, limits: StorageLimits) -> Self {
        Self {
            database,
            session: Mutex::new(BTreeMap::new()),
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
                let response = execute_idb(
                    &mut databases,
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

fn execute_idb(
    databases: &mut BTreeMap<(String, String), DatabaseData>,
    origin: &str,
    command: IdbCommand,
    limit: usize,
) -> Result<IdbResponse, StorageError> {
    let response = match command {
        IdbCommand::Open { name, version } => {
            let key = (origin.to_string(), name);
            let database = databases.entry(key).or_default();
            let old_version = database.version;
            let requested = if version == 0 {
                old_version.max(1)
            } else {
                version
            };
            let upgrade_needed = requested > old_version;
            if upgrade_needed {
                database.version = requested;
            }
            IdbResponse {
                version: Some(database.version),
                old_version: Some(old_version),
                upgrade_needed,
                object_stores: database.stores.keys().cloned().collect(),
                ..IdbResponse::default()
            }
        }
        IdbCommand::DeleteDatabase { name } => {
            databases.remove(&(origin.to_string(), name));
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
            store_data
                .indexes
                .insert(definition.name.clone(), definition);
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
            IdbResponse {
                values: store_data
                    .records
                    .values()
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
            store_data.records.remove(&encode_key(&key)?);
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
                .filter_map(|record| {
                    let cursor_key = index_definition
                        .and_then(|definition| record.value.get(&definition.key_path))
                        .unwrap_or(&record.key);
                    (query.as_ref().is_none_or(|query| query == cursor_key)).then(|| IdbRecord {
                        key: cursor_key.clone(),
                        primary_key: record.key.clone(),
                        value: record.value.clone(),
                    })
                })
                .collect::<Vec<_>>();
            records.sort_by_key(|record| encode_key(&record.key).unwrap_or_default());
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

fn put_record(
    store: &mut ObjectStoreData,
    write: IdbWrite,
) -> Result<serde_json::Value, StorageError> {
    let definition = store.definition.clone().unwrap_or(ObjectStoreDefinition {
        name: write.store,
        key_path: None,
        auto_increment: false,
    });
    let mut value = write.value;
    let key_from_path = definition
        .key_path
        .as_deref()
        .and_then(|path| value.get(path).cloned());
    let key = write.key.or(key_from_path).or_else(|| {
        definition.auto_increment.then(|| {
            let key = serde_json::Value::from(store.next_key.max(1));
            store.next_key = store.next_key.max(1).saturating_add(1);
            key
        })
    });
    let key = key.ok_or(StorageError::KeyRequired)?;
    if let Some(path) = definition.key_path
        && value.get(&path).is_none()
        && let Some(object) = value.as_object_mut()
    {
        object.insert(path, key.clone());
    }
    let encoded = encode_key(&key)?;
    if write.add_only && store.records.contains_key(&encoded) {
        return Err(StorageError::Constraint);
    }
    for index in store.indexes.values().filter(|index| index.unique) {
        let Some(index_value) = value.get(&index.key_path) else {
            continue;
        };
        if store.records.iter().any(|(record_key, record)| {
            record_key != &encoded && record.value.get(&index.key_path) == Some(index_value)
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

fn encode_key(key: &serde_json::Value) -> Result<String, StorageError> {
    match key {
        serde_json::Value::Null | serde_json::Value::Object(_) => {
            Err(StorageError::DataClone("invalid IndexedDB key".into()))
        }
        _ => serde_json::to_string(key).map_err(|error| StorageError::DataClone(error.to_string())),
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
                origin,
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
            let cookie = HeaderValue::from_static("session=persisted; Path=/; HttpOnly");
            profile.cookie_store().set_cookies(
                &mut std::iter::once(&cookie),
                &Url::parse("https://example.test/login").unwrap(),
            );
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
                    .cookies(&Url::parse("https://example.test/account").unwrap())
                    .and_then(|value| value.to_str().ok().map(str::to_string))
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
}
