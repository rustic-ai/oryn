//! Stored script resource maps restore real dormant native service versions.
//! Process IDs and context IDs are freshly admitted; stored identifiers never
//! authorize a process, client, origin or filesystem route.
use super::*;
use crate::storage::{ServiceStorageKind, ServiceStorageRequest, StorageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
#[derive(Serialize, Deserialize)]
struct StoredRegistration {
    #[serde(default)]
    active_version: Option<ServiceVersionId>,
    #[serde(default)]
    waiting_version: Option<ServiceVersionId>,
    #[serde(default)]
    lineage: String,
    #[serde(default)]
    navigation_preload: NavigationPreloadState,
    #[serde(default)]
    last_update_check_ms: u64,
    scope: String,
    update_via_cache: String,
    active: Option<AgentBootstrap>,
    waiting: Option<AgentBootstrap>,
}
pub(super) fn partition_key(partition: &str) -> Result<String, WorkerError> {
    let value: serde_json::Value =
        serde_json::from_str(partition).map_err(|_| error("invalid native service partition"))?;
    let parts = value
        .as_array()
        .ok_or_else(|| error("invalid native service partition"))?;
    let shape = match parts.get(1).and_then(serde_json::Value::as_str) {
        Some("site") => {
            parts.len() == 4
                && matches!(parts[2].as_str(), Some("http" | "https"))
                && parts[3]
                    .as_str()
                    .is_some_and(|host| !host.is_empty() && host.len() <= 65536)
        }
        Some("origin") => {
            parts.len() == 3
                && parts[2]
                    .as_str()
                    .is_some_and(|origin| !origin.is_empty() && origin.len() <= 65536)
        }
        _ => false,
    };
    if !parts.first().is_some_and(serde_json::Value::is_u64) || !shape {
        return Err(error("invalid native service partition"));
    }
    serde_json::to_string(&parts[1..]).map_err(|failure| error(failure.to_string()))
}
fn record_lineage(record: &StoredRegistration) -> String {
    if record.lineage.is_empty() {
        crate::agent_script::representation_digest(
            format!("oryn-service-legacy:{}", record.scope).as_bytes(),
        )
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
    } else {
        record.lineage.clone()
    }
}
pub(super) fn navigation_preload_snapshot(
    context: &BrowserContext,
    partition: &str,
    origin: &AgentOrigin,
    url: &str,
) -> Result<Option<NavigationPreloadState>, WorkerError> {
    if origin.opaque {
        return Ok(None);
    }
    let current = context
        .context_store()
        .service_storage(ServiceStorageRequest::Read {
            partition: partition_key(partition)?,
            origin: origin.authority.clone(),
            kind: ServiceStorageKind::Registrations,
        })
        .map_err(|failure| error(failure.to_string()))?;
    let records: Vec<StoredRegistration> = if current.bytes.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&current.bytes).map_err(|failure| error(failure.to_string()))?
    };
    validate_stored_records(&records, origin)?;
    Ok(records
        .into_iter()
        .filter(|record| record.active.is_some() && url.starts_with(&record.scope))
        .max_by_key(|record| record.scope.len())
        .map(|record| record.navigation_preload))
}
fn validate_stored_records(
    records: &[StoredRegistration],
    origin: &AgentOrigin,
) -> Result<(), WorkerError> {
    if records.len() > 128 {
        return Err(error("stored service registration count exceeds budget"));
    }
    let mut scopes = BTreeSet::new();
    let mut versions = BTreeSet::new();
    for record in records {
        if record.active.is_none() && record.active_version.is_some()
            || record.waiting.is_none() && record.waiting_version.is_some()
        {
            return Err(error("stored logical version has no resource graph"));
        }
        for version in [record.active_version, record.waiting_version]
            .into_iter()
            .flatten()
        {
            if version.0 == [0; 16] || !versions.insert(version) {
                return Err(error(
                    "invalid or duplicate stored service version identity",
                ));
            }
        }
        if !record.lineage.is_empty()
            && (record.lineage.len() != 64
                || !record.lineage.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(error("invalid stored service registration lifetime"));
        }
        let scope =
            url::Url::parse(&record.scope).map_err(|_| error("invalid stored service scope"))?;
        if record.navigation_preload.header_value.len() > 8192
            || record
                .navigation_preload
                .header_value
                .chars()
                .any(|character| u32::from(character) > 255)
            || record
                .navigation_preload
                .header_value
                .bytes()
                .any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
            || !matches!(record.update_via_cache.as_str(), "imports" | "all" | "none")
            || !scopes.insert(record.scope.clone())
            || !matches!(scope.scheme(), "http" | "https")
            || scope.origin().ascii_serialization() != origin.serialized
            || scope.fragment().is_some()
            || !scope.username().is_empty()
            || scope.password().is_some()
        {
            return Err(error("invalid stored service registration authority"));
        }
        for bootstrap in record.active.iter().chain(record.waiting.iter()) {
            if bootstrap.resource_digests.len() > bootstrap.modules.len().saturating_add(1)
                || bootstrap
                    .resource_digests
                    .keys()
                    .any(|url| url != &bootstrap.script_url && !bootstrap.modules.contains_key(url))
            {
                return Err(error(
                    "stored service representation map has unknown resources",
                ));
            }
            if bootstrap.import_metadata.len() > bootstrap.modules.len()
                || bootstrap
                    .import_metadata
                    .keys()
                    .any(|url| !bootstrap.modules.contains_key(url))
            {
                return Err(error(
                    "stored service import provenance has unknown resources",
                ));
            }
            for metadata in bootstrap.import_metadata.values() {
                crate::agent_script::validate_import_metadata(
                    metadata,
                    crate::agent_script::ScriptLoadKind::from_import_kind(metadata.kind),
                )?;
                for value in std::iter::once(metadata.response_url.as_str())
                    .chain(metadata.request_referrer_source.as_deref())
                {
                    let url = url::Url::parse(value)
                        .map_err(|_| error("invalid stored service import provenance URL"))?;
                    if !matches!(url.scheme(), "http" | "https")
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(error("invalid stored service import provenance authority"));
                    }
                }
                if metadata.response_url.len() > 65536
                    || metadata
                        .request_referrer_source
                        .as_ref()
                        .is_some_and(|value| value.len() > 65536)
                    || metadata
                        .content_type
                        .as_ref()
                        .is_some_and(|value| value.len() > 8192)
                    || metadata.referrer_policy.len() > 128
                    || metadata.request_referrer_policy.len() > 128
                {
                    return Err(error("stored service import provenance exceeds budget"));
                }
            }
            let script = url::Url::parse(&bootstrap.script_url)
                .map_err(|_| error("invalid stored service script URL"))?;
            if bootstrap.kind != AgentKind::Service
                || &bootstrap.origin != origin
                || !matches!(script.scheme(), "http" | "https")
                || script.origin().ascii_serialization() != origin.serialized
                || script.fragment().is_some()
                || !script.username().is_empty()
                || script.password().is_some()
                || bootstrap.policy.url != bootstrap.script_url
                || bootstrap.modules.len() > 1024
                || bootstrap.source.len() > 4 * 1024 * 1024
                || bootstrap.policy.headers.len() > 1024
            {
                return Err(error("stored service script authority or limits mismatch"));
            }
            // Reopening restores resource data, never relaxed worker options
            // or an unbounded/foreign policy-origin authority from old bytes.
            if bootstrap.options.credentials != WorkerCredentials::SameOrigin
                || !bootstrap.options.name.is_empty()
                || bootstrap.cookie_ancestor_urls.len() > 64
                || bootstrap
                    .cookie_ancestor_urls
                    .iter()
                    .try_fold(0usize, |size, url| size.checked_add(url.len()))
                    .is_none_or(|size| size > 65536)
                || bootstrap
                    .top_level_url
                    .as_ref()
                    .is_some_and(|url| url.len() > 65536 || url::Url::parse(url).is_err())
                || bootstrap
                    .cookie_ancestor_urls
                    .iter()
                    .any(|url| url::Url::parse(url).is_err())
                || bootstrap.referrer_policy.len() > 128
            {
                return Err(error("stored service fetch authority or limits mismatch"));
            }
            let policy_count = bootstrap
                .policy
                .headers
                .iter()
                .filter(|(name, _)| {
                    name.eq_ignore_ascii_case("content-security-policy")
                        || name.eq_ignore_ascii_case("content-security-policy-report-only")
                })
                .count();
            if !bootstrap.policy.policy_sources.is_empty()
                && (bootstrap.policy.policy_sources.len() != policy_count
                    || bootstrap
                        .policy
                        .policy_sources
                        .iter()
                        .any(|source| !matches!(source.as_str(), "header" | "meta")))
            {
                return Err(error("stored service policy source mismatch"));
            }
            if !bootstrap.policy.policy_self_urls.is_empty()
                && bootstrap.policy.policy_self_urls.len() != policy_count
            {
                return Err(error("stored service policy origin count mismatch"));
            }
            for source in bootstrap
                .policy
                .policy_self_url
                .iter()
                .chain(bootstrap.policy.policy_self_urls.iter().flatten())
            {
                if source.len() > 65536 {
                    return Err(error("invalid stored service policy origin"));
                }
                let source = url::Url::parse(source)
                    .map_err(|_| error("invalid stored service policy origin"))?;
                if !matches!(source.scheme(), "http" | "https")
                    || source.origin().ascii_serialization() != origin.serialized
                {
                    return Err(error("foreign stored service policy origin"));
                }
            }
            let mut bytes = bootstrap
                .source
                .len()
                .checked_mul(2)
                .ok_or_else(|| error("stored service graph size overflow"))?;
            for (url, source) in &bootstrap.modules {
                let url =
                    url::Url::parse(url).map_err(|_| error("invalid stored service module URL"))?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.fragment().is_some()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(error("invalid stored service resource URL"));
                }
                bytes = bytes
                    .checked_add(
                        source
                            .len()
                            .checked_mul(2)
                            .ok_or_else(|| error("stored service graph size overflow"))?,
                    )
                    .ok_or_else(|| error("stored service graph size overflow"))?;
                if bytes > 32 * 1024 * 1024 {
                    return Err(error("stored service resource graph exceeds budget"));
                }
            }
            if bootstrap.policy.headers.iter().any(|(name, value)| {
                name.len() > 8192
                    || value.len() > 65536
                    || name.contains(['\r', '\n'])
                    || value.contains(['\r', '\n'])
            }) {
                return Err(error("invalid stored service policy headers"));
            }
        }
    }
    Ok(())
}
fn fingerprint(record: &StoredRegistration) -> Result<[u8; 32], WorkerError> {
    Ok(
        Sha256::digest(serde_json::to_vec(record).map_err(|failure| error(failure.to_string()))?)
            .into(),
    )
}
impl ServiceRegistry {
    pub(super) fn persist_registration(&self, id: RegistrationId) -> Result<(), WorkerError> {
        self.persist_registration_with_update(id, |_| {})
    }
    // Serialize a local field update with namespace restoration and the exact
    // existing durable compare-and-replace operation. In particular, test clock
    // advancement must not race a background profile snapshot installation.
    pub(super) fn persist_registration_with_update(
        &self,
        id: RegistrationId,
        update: impl FnOnce(&mut Registration),
    ) -> Result<(), WorkerError> {
        let _serialization = self
            .persistence
            .lock()
            .map_err(|_| error("service persistence lock poisoned"))?;
        {
            let mut state = self.state()?;
            let Some(registration) = state.registrations.get_mut(&id) else {
                state.dirty_registrations.remove(&id);
                return Ok(());
            };
            update(registration);
            state.dirty_registrations.insert(id);
        }
        let (partition, origin, scope, record, baseline) = {
            let state = self.state()?;
            let Some(registration) = state.registrations.get(&id) else {
                return Ok(());
            };
            let record = if registration.unregistered {
                None
            } else {
                Some(StoredRegistration {
                    active_version: registration
                        .snapshot
                        .active
                        .as_ref()
                        .filter(|worker| worker.state == WorkerState::Activated)
                        .map(|worker| worker.version_id),
                    waiting_version: registration
                        .snapshot
                        .waiting
                        .as_ref()
                        .map(|worker| worker.version_id),
                    lineage: registration.lineage.clone(),
                    navigation_preload: registration.snapshot.navigation_preload.clone(),
                    last_update_check_ms: registration.last_update_check_ms,
                    scope: registration.snapshot.scope.clone(),
                    update_via_cache: registration.snapshot.update_via_cache.clone(),
                    active: registration
                        .snapshot
                        .active
                        .as_ref()
                        .filter(|worker| worker.state == WorkerState::Activated)
                        .and_then(|worker| Self::version_bootstrap(&state, worker))
                        .cloned(),
                    waiting: registration
                        .snapshot
                        .waiting
                        .as_ref()
                        .and_then(|worker| Self::version_bootstrap(&state, worker))
                        .cloned(),
                })
            };
            (
                partition_key(&registration.partition)?,
                registration.origin.clone(),
                registration.snapshot.scope.clone(),
                record,
                registration.durable_fingerprint,
            )
        };
        let store = self.context()?.context_store();
        for _ in 0..32 {
            let current = store
                .service_storage(ServiceStorageRequest::Read {
                    partition: partition.clone(),
                    origin: origin.clone(),
                    kind: ServiceStorageKind::Registrations,
                })
                .map_err(|failure| error(failure.to_string()))?;
            let mut records: Vec<StoredRegistration> = if current.bytes.is_empty() {
                Vec::new()
            } else {
                serde_json::from_slice(&current.bytes)
                    .map_err(|failure| error(failure.to_string()))?
            };
            let observed = records
                .iter()
                .find(|record| record.scope == scope)
                .map(fingerprint)
                .transpose()?;
            if observed != baseline {
                self.state()?.dirty_registrations.remove(&id);
                return Err(error(
                    "service registration changed in another context; refresh required",
                ));
            }
            records.retain(|record| record.scope != scope);
            if let Some(record) = &record {
                if record.active.is_some() || record.waiting.is_some() {
                    records.push(StoredRegistration {
                        active_version: record.active_version,
                        waiting_version: record.waiting_version,
                        lineage: record.lineage.clone(),
                        navigation_preload: record.navigation_preload.clone(),
                        last_update_check_ms: record.last_update_check_ms,
                        scope: record.scope.clone(),
                        update_via_cache: record.update_via_cache.clone(),
                        active: record.active.clone(),
                        waiting: record.waiting.clone(),
                    });
                }
            }
            let bytes =
                serde_json::to_vec(&records).map_err(|failure| error(failure.to_string()))?;
            match store.service_storage(ServiceStorageRequest::Replace {
                partition: partition.clone(),
                origin: origin.clone(),
                kind: ServiceStorageKind::Registrations,
                expected_revision: current.revision,
                bytes,
            }) {
                Ok(_) => {
                    let fingerprint = record
                        .as_ref()
                        .filter(|record| record.active.is_some() || record.waiting.is_some())
                        .map(fingerprint)
                        .transpose()?;
                    if let Some(registration) = self.state()?.registrations.get_mut(&id) {
                        registration.durable_fingerprint = fingerprint;
                    }
                    self.state()?.dirty_registrations.remove(&id);
                    drop(_serialization);
                    store
                        .service_coordinator()
                        .map_err(|failure| error(failure.to_string()))?
                        .notify_changed(&partition, &origin, &scope);
                    return Ok(());
                }
                Err(StorageError::Constraint) => {}
                Err(failure) => return Err(error(failure.to_string())),
            }
        }
        Err(error("service persistence contention limit"))
    }
    pub(super) fn registration_fields_current(
        &self,
        partition: &str,
        origin: &str,
        scope: &str,
        baseline: Option<[u8; 32]>,
    ) -> Result<bool, WorkerError> {
        let current = self
            .context()?
            .context_store()
            .service_storage(ServiceStorageRequest::Read {
                partition: partition_key(partition)?,
                origin: origin.into(),
                kind: ServiceStorageKind::Registrations,
            })
            .map_err(|failure| error(failure.to_string()))?;
        let records: Vec<StoredRegistration> = if current.bytes.is_empty() {
            Vec::new()
        } else {
            serde_json::from_slice(&current.bytes).map_err(|failure| error(failure.to_string()))?
        };
        Ok(records
            .iter()
            .find(|record| record.scope == scope)
            .map(fingerprint)
            .transpose()?
            == baseline)
    }
    pub(super) fn registration_current(&self, id: RegistrationId) -> Result<bool, WorkerError> {
        let snapshot = {
            let state = self.state()?;
            let Some(registration) = state.registrations.get(&id) else {
                return Ok(false);
            };
            (
                registration.partition.clone(),
                registration.origin.clone(),
                registration.snapshot.scope.clone(),
                registration.durable_fingerprint,
            )
        };
        self.registration_fields_current(&snapshot.0, &snapshot.1, &snapshot.2, snapshot.3)
    }
    pub(super) fn restore_for_owner(
        &self,
        owner: &AgentOwner,
        network: SharedNetworkBroker,
    ) -> Result<(), WorkerError> {
        let (partition, origin, _) = self.context()?.native_worker_authority(owner)?;
        self.restore_namespace(partition, origin, network)
    }
    pub(super) fn restore_namespace(
        &self,
        partition: String,
        origin: AgentOrigin,
        network: SharedNetworkBroker,
    ) -> Result<(), WorkerError> {
        let directory = self
            .context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?
            .versions
            .clone();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.retired.load(Ordering::Acquire) {
                return Err(error("service registry retired during restoration"));
            }
            let revision = directory
                .revision()
                .map_err(|failure| error(failure.to_string()))?;
            match self.restore_namespace_once(partition.clone(), origin.clone(), network.clone()) {
                Err(failure) if failure.name == "ServiceAdmissionPending" => {
                    if Instant::now() >= deadline {
                        return Err(WorkerError::new(
                            "TimeoutError",
                            "service version admission deadline",
                        ));
                    }
                    // restore_namespace_once released its persistence guard.
                    // Never wait while preventing the owner from publishing.
                    directory
                        .wait_changed(revision, Duration::from_millis(50))
                        .map_err(|failure| error(failure.to_string()))?;
                }
                result => return result,
            }
        }
    }
    fn restore_namespace_once(
        &self,
        partition: String,
        origin: AgentOrigin,
        network: SharedNetworkBroker,
    ) -> Result<(), WorkerError> {
        self.retry_restore_publications()?;
        let registry = self
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service registry retired"))?;
        self.context()?
            .context_store()
            .service_coordinator()
            .map_err(|failure| error(failure.to_string()))?
            .versions
            .register_registry(&registry)
            .map_err(|failure| error(failure.to_string()))?;
        if origin.opaque
            || !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(
                &origin.serialized,
            ))
        {
            return Ok(());
        }
        // Lifecycle callbacks observe the version currently installing/activating;
        // restoring our own last durable snapshot would replace that native
        // transition and recursively dispatch another activation.
        {
            let state = self.state()?;
            if state
                .dirty_registrations
                .iter()
                .filter_map(|id| state.registrations.get(id))
                .any(|registration| {
                    registration.partition == partition && registration.origin == origin.authority
                })
                || state.versions.values().any(|version| {
                    version.bootstrap.partition == partition
                        && version.bootstrap.origin.authority == origin.authority
                        && matches!(
                            version.snapshot.state,
                            WorkerState::Installing | WorkerState::Activating
                        )
                })
            {
                return Ok(());
            }
        }
        let identity = (partition.clone(), origin.authority.clone());
        let serialization = self
            .persistence
            .lock()
            .map_err(|_| error("service persistence lock poisoned"))?;
        let context = self.context()?;
        let stable = partition_key(&partition)?;
        let current = context
            .context_store()
            .service_storage(ServiceStorageRequest::Read {
                partition: stable,
                origin: origin.authority.clone(),
                kind: ServiceStorageKind::Registrations,
            })
            .map_err(|failure| error(failure.to_string()))?;
        let known = {
            let state = self.state()?;
            state.restored_namespaces.get(&identity) == Some(&current.revision)
        };
        if known {
            let remotes = self
                .state()?
                .remote_versions
                .values()
                .filter(|remote| {
                    remote.bootstrap.partition == partition
                        && remote.bootstrap.origin.authority == origin.authority
                })
                .cloned()
                .collect::<Vec<_>>();
            let live = remotes.iter().all(|remote| {
                remote.owner.upgrade().is_some_and(|owner| {
                    !owner.retired.load(Ordering::Acquire)
                        && owner.state().is_ok_and(|state| {
                            state.versions.values().any(|version| {
                                version.snapshot.version_id == remote.snapshot.version_id
                            })
                        })
                })
            });
            if live {
                return Ok(());
            }
        }
        let records: Vec<StoredRegistration> = if current.bytes.is_empty() {
            Vec::new()
        } else {
            serde_json::from_slice(&current.bytes).map_err(|failure| error(failure.to_string()))?
        };
        validate_stored_records(&records, &origin)?;
        {
            let state = self.state()?;
            let additional = records
                .iter()
                .filter(|record| {
                    !state.registrations.values().any(|registration| {
                        !registration.unregistered
                            && registration.lineage == record_lineage(record)
                            && registration.partition == partition
                            && registration.origin == origin.authority
                            && registration.snapshot.scope == record.scope
                    })
                })
                .count();
            if state.registrations.len().saturating_add(additional) > 128 {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "restored service registration capacity",
                ));
            }
        }

        let mut staged = Vec::new();
        let mut remote_versions = Vec::new();
        let mut admissions = Vec::new();
        let mut snapshots = Vec::new();
        let mut scopes = BTreeSet::new();
        let equivalent = |old: &AgentBootstrap, new: &AgentBootstrap| {
            old.script_url == new.script_url
                && old.source == new.source
                && old.modules == new.modules
                && old.resource_digests == new.resource_digests
                && old.import_metadata == new.import_metadata
                && old.service_skip_waiting == new.service_skip_waiting
                && old.options.script_type == new.options.script_type
                && old.options.credentials == new.options.credentials
                && old.policy == new.policy
                && old.referrer_policy == new.referrer_policy
                && old.top_level_url == new.top_level_url
                && old.cookie_ancestor_urls == new.cookie_ancestor_urls
        };
        let result = (|| -> Result<(), WorkerError> {
            for record in records {
                let durable_fingerprint = Some(fingerprint(&record)?);
                let lineage = record_lineage(&record);
                scopes.insert((record.scope.clone(), lineage.clone()));
                let existing = {
                    let state = self.state()?;
                    state
                        .registrations
                        .iter()
                        .find(|(_, registration)| {
                            !registration.unregistered
                                && registration.lineage == lineage
                                && registration.partition == partition
                                && registration.origin == origin.authority
                                && registration.snapshot.scope == record.scope
                        })
                        .map(|(id, _)| *id)
                };
                let registration = match existing {
                    Some(id) => id,
                    None => {
                        self.observed_registration_id(&partition, &origin.authority, &lineage)?
                    }
                };
                let Some(latest) = record.waiting.as_ref().or(record.active.as_ref()) else {
                    continue;
                };
                let mut snapshot = RegistrationSnapshot {
                    navigation_preload: record.navigation_preload.clone(),
                    id: registration,
                    scope: record.scope.clone(),
                    update_via_cache: record.update_via_cache.clone(),
                    installing: None,
                    waiting: None,
                    active: None,
                };
                let script_url = latest.script_url.clone();
                let script_type = latest.options.script_type;
                let source = latest.source.clone();
                for (saved, stored_version, worker_state) in [
                    (record.active, record.active_version, WorkerState::Activated),
                    (
                        record.waiting,
                        record.waiting_version,
                        WorkerState::Installed,
                    ),
                ] {
                    let Some(mut bootstrap) = saved else { continue };
                    let version_id = match stored_version {
                        Some(version) => version,
                        None => {
                            let mut digest = Sha256::new();
                            digest.update(b"oryn-service-legacy-version\0");
                            digest.update(lineage.as_bytes());
                            digest.update(
                                serde_json::to_vec(&bootstrap)
                                    .map_err(|failure| error(failure.to_string()))?,
                            );
                            let hash = digest.finalize();
                            let mut bytes = [0; 16];
                            bytes.copy_from_slice(&hash[..16]);
                            ServiceVersionId(bytes)
                        }
                    };
                    let retained = {
                        let state = self.state()?;
                        state
                            .registrations
                            .get(&registration)
                            .and_then(|registration| {
                                if worker_state == WorkerState::Activated {
                                    registration.snapshot.active.as_ref()
                                } else {
                                    registration.snapshot.waiting.as_ref()
                                }
                            })
                            .filter(|worker| {
                                worker.version_id == version_id
                                    && state
                                        .versions
                                        .get(&worker.id)
                                        .filter(|version| {
                                            version.snapshot.version_id == worker.version_id
                                        })
                                        .is_some_and(|version| {
                                            equivalent(&version.bootstrap, &bootstrap)
                                        })
                            })
                            .cloned()
                    };
                    let worker = if let Some(retained) = retained {
                        retained
                    } else {
                        bootstrap.id = AgentId(0);
                        bootstrap.secure_context = true;
                        bootstrap.partition = partition.clone();
                        bootstrap.service_registration = Some(registration);
                        bootstrap.service_controller = None;
                        bootstrap.limits = context.agent_limits();
                        let coordinator = context
                            .context_store()
                            .service_coordinator()
                            .map_err(|failure| error(failure.to_string()))?;
                        match coordinator
                            .versions
                            .claim(
                                &partition_key(&partition)?,
                                &origin.authority,
                                &record.scope,
                                &lineage,
                                version_id,
                                &registry,
                            )
                            .map_err(|failure| error(failure.to_string()))?
                        {
                            crate::storage::ServiceVersionClaim::Existing(owner) => {
                                if owner.retired.load(Ordering::Acquire) {
                                    return Err(WorkerError::new(
                                        "ServiceAdmissionPending",
                                        "profile service owner is completing retirement",
                                    ));
                                }
                                let worker = {
                                    let state = owner.state()?;
                                    let version = state
                                        .versions
                                        .values()
                                        .find(|version| {
                                            version.snapshot.version_id == version_id
                                                && !version.retiring
                                        })
                                        .ok_or_else(|| {
                                            WorkerError::new(
                                                "ServiceAdmissionPending",
                                                "profile service version admission still pending",
                                            )
                                        })?;
                                    if !equivalent(&version.bootstrap, &bootstrap) {
                                        return Err(error(
                                            "logical service version conflicts with saved resource graph",
                                        ));
                                    }
                                    version.snapshot.clone()
                                };
                                if !Arc::ptr_eq(&owner, &registry) {
                                    remote_versions.push((
                                        version_id,
                                        RemoteVersion {
                                            owner: Arc::downgrade(&owner),
                                            snapshot: worker.clone(),
                                            bootstrap,
                                        },
                                    ));
                                }
                                worker
                            }
                            crate::storage::ServiceVersionClaim::Reserved(directory) => {
                                let (agent, admission) =
                                    crate::agent_service::launch_service_agent(
                                        &context,
                                        bootstrap.clone(),
                                        network.clone(),
                                    )?;
                                bootstrap.id = agent;
                                let skip_waiting = bootstrap.service_skip_waiting;
                                let worker = WorkerSnapshot {
                                    version_id,
                                    id: agent,
                                    script_url: bootstrap.script_url.clone(),
                                    state: worker_state,
                                };
                                staged.push((
                                    agent,
                                    Version {
                                        directory: Some(Arc::new(directory)),
                                        retiring: false,
                                        revalidate_imports: false,
                                        snapshot: worker.clone(),
                                        bootstrap,
                                        serial: Arc::new(Mutex::new(())),
                                        inbox: Arc::new(Mutex::new(Default::default())),
                                        inflight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                                        skip_waiting,
                                    },
                                ));
                                admissions.push((agent, admission));
                                worker
                            }
                        }
                    };
                    if worker_state == WorkerState::Activated {
                        snapshot.active = Some(worker);
                    } else {
                        snapshot.waiting = Some(worker);
                    }
                }
                snapshots.push((
                    registration,
                    Registration {
                        lineage,
                        durable_fingerprint,
                        last_update_check_ms: record.last_update_check_ms,
                        snapshot,
                        partition: partition.clone(),
                        origin: origin.authority.clone(),
                        script_url,
                        script_type,
                        source,
                        unregistered: false,
                    },
                ));
            }
            Ok(())
        })();
        if let Err(failure) = result {
            // Prepared processes have no author execution until publication.
            // Parent registry retains failed teardown batches for retry.
            drop(serialization);
            let agents = staged
                .into_iter()
                .map(|(agent, _)| agent)
                .collect::<Vec<_>>();
            self.state()?
                .orphaned_services
                .extend(agents.iter().copied());
            let mut cleanup = Vec::new();
            for agent in agents {
                if let Err(error) = self.retire_orphan(agent) {
                    cleanup.push(error.to_string());
                }
            }
            if cleanup.is_empty() {
                return Err(failure);
            }
            return Err(error(format!(
                "{failure}; restore teardown: {}",
                cleanup.join("; ")
            )));
        }
        {
            let mut state = self.state()?;
            for registration in state.registrations.values_mut().filter(|registration| {
                registration.partition == partition && registration.origin == origin.authority
            }) {
                if !scopes.contains(&(
                    registration.snapshot.scope.clone(),
                    registration.lineage.clone(),
                )) {
                    registration.unregistered = true;
                }
            }
            state.remote_versions.extend(remote_versions);
            for (id, version) in staged {
                state.remote_versions.remove(&version.snapshot.version_id);
                state.versions.insert(id, version);
            }
            for (id, registration) in snapshots {
                state.registrations.insert(id, registration);
            }
            state.restored_namespaces.remove(&identity);
            state.pending_restore_admissions.extend(admissions);
        }
        let published = self
            .state()?
            .versions
            .iter()
            .filter(|(_, version)| {
                version.bootstrap.partition == partition
                    && version.bootstrap.origin.authority == origin.authority
            })
            .map(|(agent, _)| *agent)
            .collect::<Vec<_>>();
        for agent in published {
            self.publish_version(agent)?;
        }
        drop(serialization);
        self.state()?
            .restored_namespaces
            .insert(identity, current.revision);
        let waiting = {
            let state = self.state()?;
            state
                .registrations
                .iter()
                .filter(|(_, registration)| {
                    !registration.unregistered
                        && registration.partition == partition
                        && registration.origin == origin.authority
                })
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        for id in waiting {
            self.try_activate(id)?;
        }
        self.cleanup_unregistered()?;
        self.wake_clients();
        Ok(())
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use super::*;
    struct NoNetwork;
    impl crate::network::NetworkBroker for NoNetwork {
        fn send(
            &self,
            _: crate::network::NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            Err(crate::network::NetworkError::Transport(
                "restoration unexpectedly fetched stored script".into(),
            ))
        }
    }
    struct UpdateNetwork;
    impl crate::network::NetworkBroker for UpdateNetwork {
        fn send(
            &self,
            request: crate::network::NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            Ok(crate::network::NetworkResponse {
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/javascript".into())],
                body: if request.url.ends_with("/sw.js") {
                    b"onfetch=e=>e.respondWith(new Response('updated from sibling context'));"
                        .to_vec()
                } else {
                    b"globalThis.saved='restored graph'".to_vec()
                },
            })
        }
    }
    #[tokio::test]
    async fn profile_reopen_restores_real_service_and_immutable_import_graph() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-service-restore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let native = oryn_js::origin::Origin::from_url("https://a.test/sw.js");
        let origin = AgentOrigin {
            serialized: native.serialize(),
            authority: native.security_token_key(),
            opaque: false,
        };
        let bootstrap=AgentBootstrap{cookie_ancestor_urls:Vec::new(),import_metadata:BTreeMap::from([("https://a.test/saved.js".into(),oryn_js::worker_wire::AgentImportMetadata{kind:oryn_js::worker_wire::AgentImportKind::Classic,response_url:"https://a.test/saved.js".into(),content_type:Some("text/javascript".into()),referrer_policy:"no-referrer".into(),request_referrer_source:None,request_referrer_policy:"no-referrer".into()})]),service_skip_waiting:false,resource_digests:Default::default(),secure_context:true,service_registration:Some(RegistrationId(999)),service_controller:None,id:AgentId(999),kind:AgentKind::Service,
            partition:"stale process partition".into(),origin:origin.clone(),script_url:"https://a.test/sw.js".into(),
            source:"if(!registration.active)throw Error('version not published');importScripts('/saved.js');onfetch=e=>e.respondWith(new Response(saved));".encode_utf16().collect(),
            modules:BTreeMap::from([("https://a.test/saved.js".into(),"globalThis.saved='restored graph'".encode_utf16().collect())]),options:WorkerOptions{script_type:ScriptType::Classic,..Default::default()},
            policy:AgentPolicy{hide_nonces:false,policy_sources:Vec::new(),policy_self_urls:Vec::new(),policy_self_url:None,url:"https://a.test/sw.js".into(),headers:Vec::new()},referrer_policy:"no-referrer".into(),limits:AgentLimits::default(),top_level_url:Some("https://a.test/".into())};
        // Validate the complete saved namespace before launching any version.
        let record = |bootstrap: AgentBootstrap| StoredRegistration {
            active_version: None,
            waiting_version: None,
            lineage: String::new(),
            navigation_preload: Default::default(),
            last_update_check_ms: 0,
            scope: "https://a.test/".into(),
            update_via_cache: "imports".into(),
            active: Some(bootstrap),
            waiting: None,
        };
        assert!(validate_stored_records(&[record(bootstrap.clone())], &origin).is_ok());
        let mut wrong_mime = bootstrap.clone();
        wrong_mime
            .import_metadata
            .get_mut("https://a.test/saved.js")
            .unwrap()
            .content_type = Some("text/html".into());
        assert!(validate_stored_records(&[record(wrong_mime)], &origin).is_err());
        let mut wrong_type = bootstrap.clone();
        wrong_type
            .import_metadata
            .get_mut("https://a.test/saved.js")
            .unwrap()
            .kind = oryn_js::worker_wire::AgentImportKind::Json;
        assert!(validate_stored_records(&[record(wrong_type)], &origin).is_err());
        let mut duplicate = record(bootstrap.clone());
        duplicate.active_version = Some(ServiceVersionId([9; 16]));
        duplicate.waiting_version = duplicate.active_version;
        duplicate.waiting = Some(bootstrap.clone());
        assert!(validate_stored_records(&[duplicate], &origin).is_err());
        let mut absent = record(bootstrap.clone());
        absent.waiting_version = Some(ServiceVersionId([9; 16]));
        assert!(validate_stored_records(&[absent], &origin).is_err());
        let mut zero = record(bootstrap.clone());
        zero.active_version = Some(ServiceVersionId([0; 16]));
        assert!(validate_stored_records(&[zero], &origin).is_err());
        let mut credentials = bootstrap.clone();
        credentials.options.credentials = WorkerCredentials::Include;
        assert!(validate_stored_records(&[record(credentials)], &origin).is_err());
        let mut ancestry = bootstrap.clone();
        ancestry.cookie_ancestor_urls = vec!["https://a.test/".into(); 65];
        assert!(validate_stored_records(&[record(ancestry)], &origin).is_err());
        let mut ancestry = bootstrap.clone();
        ancestry.cookie_ancestor_urls = vec!["not a native URL".into()];
        assert!(validate_stored_records(&[record(ancestry)], &origin).is_err());
        let mut self_origin = bootstrap.clone();
        self_origin.policy.policy_self_url = Some("https://foreign.test/".into());
        assert!(validate_stored_records(&[record(self_origin)], &origin).is_err());
        let mut self_origin = bootstrap.clone();
        self_origin.policy.policy_self_urls = vec![Some("https://a.test/".into())];
        assert!(validate_stored_records(&[record(self_origin)], &origin).is_err());
        let mut self_origin = bootstrap.clone();
        self_origin.policy.headers.push((
            "content-security-policy".into(),
            "default-src 'self'".into(),
        ));
        self_origin.policy.policy_self_urls = vec![Some("https://a.test/sw.js".into())];
        assert!(validate_stored_records(&[record(self_origin)], &origin).is_ok());
        let mut forged = bootstrap.clone();
        forged.policy.url = "https://foreign.test/sw.js".into();
        assert!(validate_stored_records(&[record(forged)], &origin).is_err());
        let mut forged = bootstrap.clone();
        forged
            .modules
            .insert("file:///private/script.js".into(), vec![0]);
        assert!(validate_stored_records(&[record(forged)], &origin).is_err());
        let mut forged = bootstrap.clone();
        forged.policy.headers.push((
            "content-security-policy".into(),
            "default-src *\r\nx-injected: yes".into(),
        ));
        assert!(validate_stored_records(&[record(forged)], &origin).is_err());
        assert!(
            validate_stored_records(
                &[record(bootstrap.clone()), record(bootstrap.clone())],
                &origin
            )
            .is_err()
        );
        {
            let profile = crate::storage::PersistentProfile::open(&directory).unwrap();
            let store =
                crate::storage::SharedContextStore::persistent(&profile, Default::default());
            let records = vec![StoredRegistration {
                active_version: None,
                waiting_version: None,
                lineage: String::new(),
                navigation_preload: Default::default(),
                last_update_check_ms: 0,
                scope: "https://a.test/".into(),
                update_via_cache: "imports".into(),
                active: Some(bootstrap),
                waiting: None,
            }];
            store
                .service_storage(ServiceStorageRequest::Replace {
                    partition: serde_json::json!(["site", "https", "a.test"]).to_string(),
                    origin: origin.authority.clone(),
                    kind: ServiceStorageKind::Registrations,
                    expected_revision: 0,
                    bytes: serde_json::to_vec(&records).unwrap(),
                })
                .unwrap();
        }
        let profile = crate::storage::PersistentProfile::open(&directory).unwrap();
        let browser = crate::runtime::Browser::in_process_with_config(Default::default());
        let context = browser.new_persistent_context(Default::default(), profile.clone());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://a.test/app".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://a.test/app".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(
                page.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                1,
                0,
            )
            .unwrap()
            .0;
        let registry = context.service_registry().unwrap();
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        fail_next_publication();
        assert!(
            registry
                .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
                .is_err()
        );
        let prepared = {
            let state = registry.state().unwrap();
            assert_eq!(state.pending_restore_admissions.len(), 1);
            assert!(state.restored_namespaces.is_empty());
            *state.pending_restore_admissions.keys().next().unwrap()
        };
        assert!(
            matches!(registry.dispatch(prepared,AgentCommand::Lifecycle{id:919,event:LifecycleEvent::Activate,payload:None},919),Err(failure) if failure.name=="ServiceAdmissionPending")
        );
        registry
            .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
            .unwrap();
        assert!(
            registry
                .state()
                .unwrap()
                .pending_restore_admissions
                .is_empty()
        );
        assert!(
            registry.state().unwrap().versions.contains_key(&prepared),
            "retry publishes the retained process, not a duplicate"
        );
        registry.control_client(owner.clone(), None).unwrap();
        let active = registry
            .state()
            .unwrap()
            .registrations
            .values()
            .next()
            .unwrap()
            .snapshot
            .active
            .clone()
            .unwrap();
        assert_ne!(active.id, AgentId(999));
        let response = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(response.body, b"restored graph");
        let registration_before_restart = registry.controller_registration(&owner).unwrap();
        let version_before_restart = active.version_id;
        registry.retire_version(active.id).unwrap();
        assert!(context.worker_bootstrap(active.id).is_err());
        registry
            .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
            .unwrap();
        assert_eq!(
            registry.controller_registration(&owner),
            Some(registration_before_restart)
        );
        assert_eq!(
            registry.state().unwrap().registrations[&registration_before_restart]
                .snapshot
                .active
                .as_ref()
                .unwrap()
                .version_id,
            version_before_restart,
            "logical version survives native process restart"
        );
        let restarted = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            restarted.body, b"restored graph",
            "terminated native process must not suppress restored active graph at unchanged durable revision"
        );
        let sibling = browser.new_persistent_context(Default::default(), profile.clone());
        let sibling_page = sibling.new_page();
        sibling
            .commit_page_url(sibling_page.page_id(), "https://a.test/other".into())
            .unwrap();
        sibling
            .install_document_security(sibling_page.page_id(), "https://a.test/other".into(), &[])
            .unwrap();
        sibling
            .handle_message_wire(
                sibling_page.page_id(),
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let sibling_owner = sibling
            .window_worker_creator(
                sibling_page.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                1,
                0,
            )
            .unwrap()
            .0;
        sibling
            .publish_worker_secure_context(sibling_owner.clone(), true)
            .unwrap();
        let sibling_registry = sibling.service_registry().unwrap();
        let (partition, origin, policy) = sibling.native_worker_authority(&sibling_owner).unwrap();
        sibling_registry
            .restore_for_owner(&sibling_owner, SharedNetworkBroker::new(NoNetwork))
            .unwrap();
        let claiming_agent = registry
            .state()
            .unwrap()
            .versions
            .values()
            .find(|version| version.snapshot.version_id == version_before_restart)
            .unwrap()
            .snapshot
            .id;
        assert!(matches!(
            registry
                .request(
                    AgentOwner::Agent(claiming_agent),
                    ServiceRequest::Claim,
                    SharedNetworkBroker::new(NoNetwork)
                )
                .unwrap(),
            ServiceResponse::Done
        ));
        assert!(
            sibling_registry
                .controller_registration(&sibling_owner)
                .is_some(),
            "native claim reaches same-profile sibling clients"
        );
        let sibling_active = sibling_registry
            .control_client(sibling_owner.clone(), None)
            .unwrap()
            .unwrap();
        assert_eq!(sibling_active.version_id, version_before_restart);
        assert!(
            sibling_registry.state().unwrap().versions.is_empty(),
            "profile adoption must not launch a duplicate native process"
        );
        assert!(
            sibling_registry
                .state()
                .unwrap()
                .remote_versions
                .contains_key(&version_before_restart)
        );
        let shared = sibling_registry
            .fetch(
                &sibling_owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/shared".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            shared.body, b"restored graph",
            "sibling fetch must execute in the directory owner"
        );
        sibling
            .record_native_activation(sibling_owner.clone())
            .unwrap();
        let current_agent = registry
            .state()
            .unwrap()
            .versions
            .values()
            .find(|version| version.snapshot.version_id == version_before_restart)
            .unwrap()
            .snapshot
            .id;
        registry.consume_profile_interaction(current_agent).unwrap();
        assert!(
            registry.consume_profile_interaction(current_agent).is_err(),
            "remote native activation is consumed once"
        );
        sibling_registry.retire_client(&sibling_owner).unwrap();
        sibling_registry
            .update(
                0,
                sibling_owner,
                SharedNetworkBroker::new(UpdateNetwork),
                partition,
                origin,
                policy,
                "/sw.js".into(),
                Some("/".into()),
                ScriptType::Classic,
                "none".into(),
            )
            .unwrap();
        let original_registration = registry
            .state()
            .unwrap()
            .registrations
            .keys()
            .next()
            .copied()
            .unwrap();
        assert!(
            registry
                .persist_registration(original_registration)
                .is_err(),
            "stale context must not overwrite sibling registration update"
        );
        registry
            .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
            .unwrap();
        assert!(
            sibling_registry
                .state()
                .unwrap()
                .registrations
                .values()
                .next()
                .unwrap()
                .snapshot
                .waiting
                .is_some(),
            "a sibling cannot activate while the original document retains its controller"
        );
        let retained = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(retained.body, b"restored graph");
        registry.retire_client(&owner).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            registry
                .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
                .unwrap();
            if registry
                .state()
                .unwrap()
                .registrations
                .values()
                .next()
                .unwrap()
                .snapshot
                .waiting
                .is_none()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        registry.control_client(owner.clone(), None).unwrap();
        let response = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(response.body, b"updated from sibling context");
        let inherited_version = registry.state().unwrap().registrations[&original_registration]
            .snapshot
            .active
            .as_ref()
            .unwrap()
            .version_id;
        assert!(
            registry
                .state()
                .unwrap()
                .versions
                .values()
                .all(|version| version.snapshot.version_id != inherited_version),
            "updated version executes in sibling owner"
        );
        sibling_registry.retire().unwrap();
        sibling_page.close().await.unwrap();
        sibling.retire_workers().await.unwrap();
        drop(sibling_registry);
        drop(sibling_page);
        drop(sibling);
        registry
            .restore_for_owner(&owner, SharedNetworkBroker::new(NoNetwork))
            .unwrap();
        let resumed = registry
            .state()
            .unwrap()
            .versions
            .values()
            .find(|version| version.snapshot.version_id == inherited_version)
            .unwrap()
            .snapshot
            .clone();
        assert_eq!(
            registry.state().unwrap().registrations[&original_registration]
                .snapshot
                .active
                .as_ref()
                .unwrap()
                .version_id,
            resumed.version_id
        );
        let after_close = registry
            .fetch(
                &owner,
                AgentFetchRequest {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    navigation: Default::default(),
                    integrity: String::new(),
                    navigation_preload: false,
                    streaming: false,
                    id: 0,
                    client_id: String::new(),
                    url: "https://a.test/data".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    destination: String::new(),
                    mode: "cors".into(),
                    credentials: "same-origin".into(),
                    redirect: "follow".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            after_close.body, b"updated from sibling context",
            "owner-context shutdown preserves sibling controller and durable logical version"
        );
        registry.retire().unwrap();
        page.close().await.unwrap();
        context.retire_workers().await.unwrap();
        drop(registry);
        drop(page);
        drop(context);
        drop(browser);
        drop(profile);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
thread_local! {static FAIL_PUBLICATION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)]
pub(super) fn publication_fault() -> Result<(), WorkerError> {
    if FAIL_PUBLICATION.with(|flag| flag.replace(false)) {
        Err(error("injected native service publication failure"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn fail_next_publication() {
    FAIL_PUBLICATION.with(|flag| flag.set(true));
}

#[cfg(test)]
mod partition_shape_tests {
    use super::*;
    #[test]
    fn persistent_partition_reopening_accepts_only_native_exact_shapes() {
        assert_eq!(
            partition_key(r#"[1,"site","https","a.test"]"#).unwrap(),
            r#"["site","https","a.test"]"#
        );
        assert_eq!(
            partition_key(r#"[2,"origin","native-origin"]"#).unwrap(),
            r#"["origin","native-origin"]"#
        );
        for input in [
            "[]",
            r#"[1,"site","https"]"#,
            r#"[1,"site","https","a.test","extra"]"#,
            r#"[1,"origin",4]"#,
            r#"[1,"origin",""]"#,
            r#"[1,"site","file","a.test"]"#,
        ] {
            assert!(partition_key(input).is_err(), "{input}");
        }
    }
}
