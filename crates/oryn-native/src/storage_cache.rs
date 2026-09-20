//! Native cache query/batch algorithms over profile-owned service records.
//! Names preserve DOMString code units; URLs and header lists are native
//! Request/Response snapshots, never JSON extracted from author properties.
use super::{ServiceStorageKind, ServiceStorageRequest, SharedContextStore, StorageError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CacheLeaseOwner([u8; 32]);
impl CacheLeaseOwner {
    pub(crate) fn new() -> Result<Self, StorageError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|error| invalid(error.to_string()))?;
        Ok(Self(bytes))
    }
}
/// Shared by every ContextStore opened from one native PersistentProfile.
/// No page-controlled identifier can allocate or release another realm's lease.
#[derive(Default)]
pub struct CacheCoordinator {
    owners: BTreeMap<CacheLeaseOwner, BTreeMap<(String, String), BTreeSet<u64>>>,
}
pub use oryn_js::cache_wire::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    request: CacheRequest,
    response: CacheResponse,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cache {
    id: u64,
    name: Option<Vec<u16>>,
    entries: Vec<Entry>,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    next: u64,
    caches: Vec<Cache>,
}
fn invalid(message: impl Into<String>) -> StorageError {
    StorageError::DataClone(message.into())
}
fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    let values = headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(", "))
}
fn normalized_url(raw: &str, ignore_search: bool) -> Result<url::Url, StorageError> {
    let mut url = url::Url::parse(raw).map_err(|_| invalid("cache request URL is invalid"))?;
    url.set_fragment(None);
    if ignore_search {
        url.set_query(None);
    }
    Ok(url)
}
fn matches(entry: &Entry, request: &CacheRequest, query: CacheQuery) -> Result<bool, StorageError> {
    if !query.ignore_method && request.method != "GET" {
        return Ok(false);
    }
    if normalized_url(&entry.request.url, query.ignore_search)?
        != normalized_url(&request.url, query.ignore_search)?
    {
        return Ok(false);
    }
    if !query.ignore_vary
        && let Some(vary) = header(&entry.response.headers, "vary")
    {
        for name in vary
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if name == "*" || header(&entry.request.headers, name) != header(&request.headers, name)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn validate_put(request: &CacheRequest, response: &CacheResponse) -> Result<(), StorageError> {
    let url = normalized_url(&request.url, false)?;
    if request.method != "GET" || !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("cache put requires HTTP(S) GET"));
    }
    if response
        .timing
        .as_ref()
        .is_some_and(|timing| !timing.valid_native_snapshot())
    {
        return Err(invalid("invalid private resource timing snapshot"));
    }
    if let Some(internal) = &response.internal_response {
        if !matches!(response.response_type.as_str(), "opaque" | "opaqueredirect")
            || response.status != 0
            || !response.headers.is_empty()
            || !response.url.is_empty()
            || response.redirected
            || !(100..=599).contains(&internal.status)
        {
            return Err(invalid("invalid private filtered response metadata"));
        }
        if internal.url.len() > 65536
            || url::Url::parse(&internal.url).is_err()
            || internal.headers.len() > 1024
            || internal.headers.iter().any(|(name, value)| {
                name.len() > 8192
                    || value.len() > 65536
                    || name.contains(['\r', '\n'])
                    || value.contains(['\r', '\n'])
            })
        {
            return Err(invalid("invalid private response header snapshot"));
        }
    }
    if response.status == 206 || response.response_type == "error" {
        return Err(invalid("cache put rejects partial and error responses"));
    }
    if !response.has_body && !response.body.is_empty() {
        return Err(invalid("null cache body contains bytes"));
    }
    if response.body.len() > 32 * 1024 * 1024 {
        return Err(StorageError::QuotaExceeded {
            origin: "CacheStorage".into(),
            limit: 32 * 1024 * 1024,
        });
    }
    if header(&response.headers, "vary")
        .is_some_and(|vary| vary.split(',').any(|name| name.trim() == "*"))
    {
        return Err(invalid("cache put rejects Vary star"));
    }
    if request
        .headers
        .iter()
        .chain(response.headers.iter())
        .any(|(name, value)| {
            name.len() > 8192
                || value.len() > 65536
                || name.contains(['\r', '\n'])
                || value.contains(['\r', '\n'])
        })
    {
        return Err(invalid("invalid cache header snapshot"));
    }
    Ok(())
}
fn apply(
    state: &mut State,
    operation: &CacheOperation,
) -> Result<(CacheResult, bool), StorageError> {
    match operation {
        CacheOperation::ReleaseHandle { .. } => {
            Err(invalid("cache release requires native lease authority"))
        }
        CacheOperation::Open(name) => {
            if name.len() > 8192 {
                return Err(StorageError::QuotaExceeded {
                    origin: "CacheStorage names".into(),
                    limit: 16384,
                });
            }
            if let Some(cache) = state
                .caches
                .iter()
                .find(|cache| cache.name.as_ref() == Some(name))
            {
                return Ok((CacheResult::Cache(cache.id), false));
            }
            if state.caches.len() >= 1024 {
                return Err(invalid("cache count quota exceeded"));
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or_else(|| invalid("cache identity exhausted"))?;
            state.caches.push(Cache {
                id: state.next,
                name: Some(name.clone()),
                entries: Vec::new(),
            });
            Ok((CacheResult::Cache(state.next), true))
        }
        CacheOperation::Has(name) => Ok((
            CacheResult::Bool(
                state
                    .caches
                    .iter()
                    .any(|cache| cache.name.as_ref() == Some(name)),
            ),
            false,
        )),
        CacheOperation::CacheNames => Ok((
            CacheResult::Names(
                state
                    .caches
                    .iter()
                    .filter_map(|cache| cache.name.clone())
                    .collect(),
            ),
            false,
        )),
        CacheOperation::DeleteCache(name) => {
            let cache = state
                .caches
                .iter_mut()
                .find(|cache| cache.name.as_ref() == Some(name));
            let deleted = cache.is_some();
            if let Some(cache) = cache {
                cache.name = None;
            }
            Ok((CacheResult::Bool(deleted), deleted))
        }
        CacheOperation::MatchNamed {
            name,
            request,
            query,
        } => {
            if let Some(cache) = state
                .caches
                .iter()
                .find(|cache| cache.name.as_ref() == Some(name))
            {
                for entry in &cache.entries {
                    if matches(entry, request, *query)? {
                        return Ok((CacheResult::Responses(vec![entry.response.clone()]), false));
                    }
                }
            }
            Ok((CacheResult::Responses(Vec::new()), false))
        }
        CacheOperation::Match {
            cache,
            request,
            query,
        } => {
            for current in &state.caches {
                if cache.map_or(current.name.is_none(), |id| current.id != id) {
                    continue;
                }
                for entry in &current.entries {
                    if matches(entry, request, *query)? {
                        return Ok((CacheResult::Responses(vec![entry.response.clone()]), false));
                    }
                }
            }
            Ok((CacheResult::Responses(Vec::new()), false))
        }
        CacheOperation::MatchAll {
            cache,
            request,
            query,
        }
        | CacheOperation::Keys {
            cache,
            request,
            query,
        } => {
            let cache = state
                .caches
                .iter()
                .find(|current| current.id == *cache)
                .ok_or_else(|| invalid("cache identity retired"))?;
            let mut matched = Vec::new();
            for entry in &cache.entries {
                if request
                    .as_ref()
                    .map(|request| matches(entry, request, *query))
                    .transpose()?
                    .unwrap_or(true)
                {
                    matched.push(entry);
                }
            }
            Ok((
                if matches!(operation, CacheOperation::Keys { .. }) {
                    CacheResult::Requests(
                        matched.iter().map(|entry| entry.request.clone()).collect(),
                    )
                } else {
                    CacheResult::Responses(
                        matched.iter().map(|entry| entry.response.clone()).collect(),
                    )
                },
                false,
            ))
        }
        CacheOperation::Delete {
            cache,
            request,
            query,
        } => {
            let cache = state
                .caches
                .iter_mut()
                .find(|current| current.id == *cache)
                .ok_or_else(|| invalid("cache identity retired"))?;
            let mut indices = Vec::new();
            for (index, entry) in cache.entries.iter().enumerate() {
                if matches(entry, request, *query)? {
                    indices.push(index);
                }
            }
            let changed = !indices.is_empty();
            for index in indices.into_iter().rev() {
                cache.entries.remove(index);
            }
            Ok((CacheResult::Bool(changed), changed))
        }
        CacheOperation::PutAll { cache, entries } => {
            if entries.len() > 1024 {
                return Err(invalid("cache batch quota exceeded"));
            }
            for (request, response) in entries {
                validate_put(request, response)?;
            }
            // Validate duplicate requests before mutating the transaction's
            // snapshot. One failing item rejects the entire native batch.
            for (index, (request, response)) in entries.iter().enumerate() {
                let incoming = Entry {
                    request: request.clone(),
                    response: response.clone(),
                };
                for (other, _) in &entries[index + 1..] {
                    if matches(&incoming, other, CacheQuery::default())? {
                        return Err(StorageError::Constraint);
                    }
                }
            }
            let cache = state
                .caches
                .iter_mut()
                .find(|current| current.id == *cache)
                .ok_or_else(|| invalid("cache identity retired"))?;
            for (request, response) in entries {
                let mut indices = Vec::new();
                for (index, entry) in cache.entries.iter().enumerate() {
                    if matches(entry, request, CacheQuery::default())? {
                        indices.push(index);
                    }
                }
                for index in indices.into_iter().rev() {
                    cache.entries.remove(index);
                }
                cache.entries.push(Entry {
                    request: request.clone(),
                    response: response.clone(),
                });
            }
            if cache.entries.len() > 4096 {
                return Err(invalid("cache entry quota exceeded"));
            }
            Ok((CacheResult::Done, !entries.is_empty()))
        }
    }
}
pub(crate) fn execute(
    store: &SharedContextStore,
    partition: &str,
    origin: &str,
    operation: CacheOperation,
) -> Result<CacheResult, StorageError> {
    for _ in 0..32 {
        let snapshot = store.service_storage(ServiceStorageRequest::Read {
            partition: partition.into(),
            origin: origin.into(),
            kind: ServiceStorageKind::Caches,
        })?;
        let mut state = if snapshot.bytes.is_empty() {
            State::default()
        } else {
            serde_json::from_slice::<State>(&snapshot.bytes)
                .map_err(|error| invalid(error.to_string()))?
        };
        let (result, write) = apply(&mut state, &operation)?;
        if !write {
            return Ok(result);
        }
        let bytes = serde_json::to_vec(&state).map_err(|error| invalid(error.to_string()))?;
        match store.service_storage(ServiceStorageRequest::Replace {
            partition: partition.into(),
            origin: origin.into(),
            kind: ServiceStorageKind::Caches,
            expected_revision: snapshot.revision,
            bytes,
        }) {
            Ok(_) => return Ok(result),
            Err(StorageError::Constraint) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(invalid("cache transaction contention limit"))
}
fn collect_deleted(
    store: &SharedContextStore,
    partition: &str,
    origin: &str,
    live: &BTreeSet<u64>,
) -> Result<(), StorageError> {
    for _ in 0..32 {
        let snapshot = store.service_storage(ServiceStorageRequest::Read {
            partition: partition.into(),
            origin: origin.into(),
            kind: ServiceStorageKind::Caches,
        })?;
        if snapshot.bytes.is_empty() {
            return Ok(());
        }
        let mut state: State =
            serde_json::from_slice(&snapshot.bytes).map_err(|error| invalid(error.to_string()))?;
        let before = state.caches.len();
        state
            .caches
            .retain(|cache| cache.name.is_some() || live.contains(&cache.id));
        if before == state.caches.len() {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&state).map_err(|error| invalid(error.to_string()))?;
        match store.service_storage(ServiceStorageRequest::Replace {
            partition: partition.into(),
            origin: origin.into(),
            kind: ServiceStorageKind::Caches,
            expected_revision: snapshot.revision,
            bytes,
        }) {
            Ok(_) => return Ok(()),
            Err(StorageError::Constraint) => {}
            Err(error) => return Err(error),
        }
    }
    Err(invalid("cache reclamation contention"))
}
pub(crate) fn execute_owned(
    store: &SharedContextStore,
    coordinator: &Mutex<CacheCoordinator>,
    owner: CacheLeaseOwner,
    partition: &str,
    origin: &str,
    operation: CacheOperation,
) -> Result<CacheResult, StorageError> {
    let mut coordinator = coordinator
        .lock()
        .map_err(|_| invalid("cache ownership lock poisoned"))?;
    let namespace = (partition.to_owned(), origin.to_owned());
    if let CacheOperation::ReleaseHandle { cache } = &operation {
        let cache = *cache;
        let handles = coordinator
            .owners
            .get_mut(&owner)
            .and_then(|namespaces| namespaces.get_mut(&namespace))
            .ok_or_else(|| invalid("cache owner already retired"))?;
        if !handles.remove(&cache) {
            return Err(invalid("foreign or retired cache handle release"));
        }
        let live = coordinator
            .owners
            .values()
            .filter_map(|namespaces| namespaces.get(&namespace))
            .flat_map(|handles| handles.iter().copied())
            .collect();
        if let Err(error) = collect_deleted(store, partition, origin, &live) {
            coordinator
                .owners
                .entry(owner)
                .or_default()
                .entry(namespace)
                .or_default()
                .insert(cache);
            return Err(error);
        }
        return Ok(CacheResult::Done);
    }
    let handle = match &operation {
        CacheOperation::Match { cache, .. } => *cache,
        CacheOperation::MatchAll { cache, .. }
        | CacheOperation::Keys { cache, .. }
        | CacheOperation::Delete { cache, .. }
        | CacheOperation::PutAll { cache, .. } => Some(*cache),
        _ => None,
    };
    if let Some(handle) = handle
        && !coordinator
            .owners
            .get(&owner)
            .and_then(|namespaces| namespaces.get(&namespace))
            .is_some_and(|handles| handles.contains(&handle))
    {
        return Err(invalid("foreign or retired native cache handle"));
    }
    let result = execute(store, partition, origin, operation)?;
    if let CacheResult::Cache(id) = &result {
        coordinator
            .owners
            .entry(owner)
            .or_default()
            .entry(namespace.clone())
            .or_default()
            .insert(*id);
    }
    let live = coordinator
        .owners
        .values()
        .filter_map(|namespaces| namespaces.get(&namespace))
        .flat_map(|handles| handles.iter().copied())
        .collect();
    collect_deleted(store, partition, origin, &live)?;
    Ok(result)
}
pub(crate) fn release_owner(
    store: &SharedContextStore,
    coordinator: &Mutex<CacheCoordinator>,
    owner: CacheLeaseOwner,
) -> Result<(), StorageError> {
    let mut coordinator = coordinator
        .lock()
        .map_err(|_| invalid("cache ownership lock poisoned"))?;
    // Native context authority is revoked before this call. Retain failed
    // namespaces for cleanup retry without losing profile-wide ownership.
    let Some(namespaces) = coordinator.owners.remove(&owner) else {
        return Ok(());
    };
    let mut failed = BTreeMap::new();
    let mut failure = None;
    for (namespace, handles) in namespaces {
        let live = coordinator
            .owners
            .values()
            .filter_map(|namespaces| namespaces.get(&namespace))
            .flat_map(|handles| handles.iter().copied())
            .collect();
        if let Err(error) = collect_deleted(store, &namespace.0, &namespace.1, &live) {
            failed.insert(namespace, handles);
            failure.get_or_insert(error);
        }
    }
    if !failed.is_empty() {
        coordinator.owners.insert(owner, failed);
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(url: &str, language: &str) -> CacheRequest {
        CacheRequest {
            url: url.into(),
            method: "GET".into(),
            headers: vec![("accept-language".into(), language.into())],
        }
    }
    #[test]
    fn released_object_handle_cannot_read_but_other_realm_retains_deleted_cache() {
        let store = SharedContextStore::memory(Default::default());
        let coordinator = Mutex::new(CacheCoordinator::default());
        let first = CacheLeaseOwner::new().unwrap();
        let second = CacheLeaseOwner::new().unwrap();
        let name = vec![65];
        let CacheResult::Cache(id) = execute_owned(
            &store,
            &coordinator,
            first,
            "site",
            "origin",
            CacheOperation::Open(name.clone()),
        )
        .unwrap() else {
            panic!()
        };
        execute_owned(
            &store,
            &coordinator,
            second,
            "site",
            "origin",
            CacheOperation::Open(name.clone()),
        )
        .unwrap();
        execute_owned(
            &store,
            &coordinator,
            first,
            "site",
            "origin",
            CacheOperation::PutAll {
                cache: id,
                entries: vec![(request("https://a.test/data", "en"), response("retained"))],
            },
        )
        .unwrap();
        execute_owned(
            &store,
            &coordinator,
            first,
            "site",
            "origin",
            CacheOperation::DeleteCache(name),
        )
        .unwrap();
        execute_owned(
            &store,
            &coordinator,
            first,
            "site",
            "origin",
            CacheOperation::ReleaseHandle { cache: id },
        )
        .unwrap();
        assert!(
            execute_owned(
                &store,
                &coordinator,
                first,
                "site",
                "origin",
                CacheOperation::Keys {
                    cache: id,
                    request: None,
                    query: Default::default()
                }
            )
            .is_err()
        );
        assert!(
            matches!(execute_owned(&store,&coordinator,second,"site","origin",CacheOperation::Keys{cache:id,request:None,query:Default::default()}).unwrap(),CacheResult::Requests(values)if values.len()==1)
        );
        execute_owned(
            &store,
            &coordinator,
            second,
            "site",
            "origin",
            CacheOperation::ReleaseHandle { cache: id },
        )
        .unwrap();
        assert!(
            execute(
                &store,
                "site",
                "origin",
                CacheOperation::Keys {
                    cache: id,
                    request: None,
                    query: Default::default()
                }
            )
            .is_err()
        );
    }
    fn response(body: &str) -> CacheResponse {
        CacheResponse {
            timing: None,
            internal_response: None,
            status: 200,
            status_text: "OK".into(),
            headers: vec![("vary".into(), "Accept-Language".into())],
            body: body.as_bytes().into(),
            has_body: true,
            redirected: false,
            url: "https://a.test/data".into(),
            response_type: "basic".into(),
        }
    }
    fn run(
        store: &SharedContextStore,
        operation: CacheOperation,
    ) -> Result<CacheResult, StorageError> {
        execute(store, "https://top.test", "https://a.test", operation)
    }
    #[test]
    fn opaque_internal_response_survives_cache_without_public_metadata_leak() {
        let store = SharedContextStore::memory(super::super::StorageLimits::default());
        let CacheResult::Cache(cache) = run(&store, CacheOperation::Open(vec![1])).unwrap() else {
            panic!()
        };
        let mut opaque = response("private bytes");
        opaque.status = 0;
        opaque.headers.clear();
        opaque.url.clear();
        opaque.response_type = "opaque".into();
        opaque.internal_response = Some(oryn_js::platform::InternalResponseMetadata {
            status: 206,
            headers: vec![("content-range".into(), "bytes 0-12/100".into())],
            url: "https://foreign.test/data".into(),
            redirected: false,
            range_requested: true,
        });
        opaque.timing = Some(oryn_js::platform::ResourceTimingMetadata {
            server_timing: Vec::new(),
            response_end_micros: None,
            navigation_phases: None,
            producer_clock_micros: None,
            decoded_body_size: Some(13),
            origin: "https://a.test".into(),
            hops: vec![oryn_js::platform::TimingHop {
                url: "https://foreign.test/data".into(),
                allow_origin: vec!["https://a.test".into()],
            }],
            transport: None,
            allow_details: true,
            response_status: 206,
            response_start_micros: 1,
            encoded_body_size: Some(13),
            content_type: String::new(),
        });
        run(
            &store,
            CacheOperation::PutAll {
                cache,
                entries: vec![(request("https://a.test/opaque", "en"), opaque.clone())],
            },
        )
        .unwrap();
        let CacheResult::Responses(values) = run(
            &store,
            CacheOperation::Match {
                cache: Some(cache),
                request: request("https://a.test/opaque", "en"),
                query: Default::default(),
            },
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(values[0].status, 0);
        assert!(values[0].headers.is_empty());
        assert_eq!(values[0].body, b"private bytes");
        assert_eq!(values[0].internal_response, opaque.internal_response);
        assert_eq!(values[0].timing, opaque.timing);
        opaque.status = 200;
        assert!(
            run(
                &store,
                CacheOperation::PutAll {
                    cache,
                    entries: vec![(request("https://a.test/bad", "en"), opaque)]
                }
            )
            .is_err()
        );
    }
    #[test]
    fn native_cache_vary_search_fragments_and_atomic_batch() {
        let store = SharedContextStore::memory(super::super::StorageLimits::default());
        let CacheResult::Cache(cache) = run(&store, CacheOperation::Open(vec![0xd800])).unwrap()
        else {
            panic!("cache")
        };
        run(
            &store,
            CacheOperation::PutAll {
                cache,
                entries: vec![
                    (
                        request("https://a.test/data?q=1#one", "en"),
                        response("english"),
                    ),
                    (request("https://a.test/data?q=1", "fr"), response("french")),
                ],
            },
        )
        .unwrap();
        let CacheResult::Responses(values) = run(
            &store,
            CacheOperation::Match {
                cache: Some(cache),
                request: request("https://a.test/data?q=2#two", "fr"),
                query: CacheQuery {
                    ignore_search: true,
                    ..Default::default()
                },
            },
        )
        .unwrap() else {
            panic!("response")
        };
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].body, b"french");
        let mut invalid = response("invalid");
        invalid.status = 206;
        assert!(
            run(
                &store,
                CacheOperation::PutAll {
                    cache,
                    entries: vec![
                        (request("https://a.test/new", "en"), response("new")),
                        (request("https://a.test/bad", "en"), invalid)
                    ]
                }
            )
            .is_err()
        );
        let CacheResult::Requests(keys) = run(
            &store,
            CacheOperation::Keys {
                cache,
                request: None,
                query: Default::default(),
            },
        )
        .unwrap() else {
            panic!("keys")
        };
        assert_eq!(keys.len(), 2);
        let CacheResult::Names(names) = run(&store, CacheOperation::CacheNames).unwrap() else {
            panic!("names")
        };
        assert_eq!(names, vec![vec![0xd800]]);
    }
    #[test]
    fn deletion_unlinks_name_but_preserves_old_native_handle() {
        let store = SharedContextStore::memory(super::super::StorageLimits::default());
        let name = vec![97];
        let CacheResult::Cache(old) = run(&store, CacheOperation::Open(name.clone())).unwrap()
        else {
            panic!("cache")
        };
        run(
            &store,
            CacheOperation::PutAll {
                cache: old,
                entries: vec![(request("https://a.test/data", "en"), response("old"))],
            },
        )
        .unwrap();
        run(&store, CacheOperation::DeleteCache(name.clone())).unwrap();
        let CacheResult::Cache(new) = run(&store, CacheOperation::Open(name)).unwrap() else {
            panic!("cache")
        };
        assert_ne!(new, old);
        let CacheResult::Responses(values) = run(
            &store,
            CacheOperation::Match {
                cache: None,
                request: request("https://a.test/data", "en"),
                query: Default::default(),
            },
        )
        .unwrap() else {
            panic!("response")
        };
        assert!(values.is_empty());
        let CacheResult::Responses(values) = run(
            &store,
            CacheOperation::Match {
                cache: Some(old),
                request: request("https://a.test/data", "en"),
                query: Default::default(),
            },
        )
        .unwrap() else {
            panic!("response")
        };
        assert_eq!(values[0].body, b"old");
        let CacheResult::Names(names) = execute(
            &store,
            "https://other.test",
            "https://a.test",
            CacheOperation::CacheNames,
        )
        .unwrap() else {
            panic!("names")
        };
        assert!(names.is_empty());
    }
    #[test]
    fn profile_cache_reclamation_preserves_other_context_handles() {
        let path = std::env::temp_dir().join(format!(
            "oryn-cache-leases-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = super::super::PersistentProfile::open(&path).unwrap();
            let first = SharedContextStore::persistent(&profile, Default::default());
            let second = SharedContextStore::persistent(&profile, Default::default());
            let a = CacheLeaseOwner::new().unwrap();
            let b = CacheLeaseOwner::new().unwrap();
            let name = vec![120];
            let CacheResult::Cache(id) = first
                .cache_operation(a, "site", "origin", CacheOperation::Open(name.clone()))
                .unwrap()
            else {
                panic!("cache")
            };
            let CacheResult::Cache(same) = second
                .cache_operation(b, "site", "origin", CacheOperation::Open(name.clone()))
                .unwrap()
            else {
                panic!("cache")
            };
            assert_eq!(id, same);
            first
                .cache_operation(
                    a,
                    "site",
                    "origin",
                    CacheOperation::PutAll {
                        cache: id,
                        entries: vec![(request("https://a.test/data", "en"), response("retained"))],
                    },
                )
                .unwrap();
            first
                .cache_operation(a, "site", "origin", CacheOperation::DeleteCache(name))
                .unwrap();
            first.release_caches(a).unwrap();
            let CacheResult::Responses(values) = second
                .cache_operation(
                    b,
                    "site",
                    "origin",
                    CacheOperation::Match {
                        cache: Some(id),
                        request: request("https://a.test/data", "en"),
                        query: Default::default(),
                    },
                )
                .unwrap()
            else {
                panic!("response")
            };
            assert_eq!(values[0].body, b"retained");
            assert!(
                first
                    .cache_operation(
                        a,
                        "site",
                        "origin",
                        CacheOperation::Keys {
                            cache: id,
                            request: None,
                            query: Default::default()
                        }
                    )
                    .is_err()
            );
            second.release_caches(b).unwrap();
            assert!(
                execute(
                    &first,
                    "site",
                    "origin",
                    CacheOperation::Keys {
                        cache: id,
                        request: None,
                        query: Default::default()
                    }
                )
                .is_err()
            );
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}
