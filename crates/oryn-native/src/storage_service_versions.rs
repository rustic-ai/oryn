//! Profile-scoped logical service-version ownership. Entries name a native
//! registry, never a process ID in the caller's context. A reservation is held
//! before process admission and survives until the owner releases it.
use super::StorageError;
use crate::service_registry::ServiceRegistry;
use oryn_js::service_worker_wire::{ServiceVersionId, WorkerSnapshot};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, Weak},
};
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    partition: String,
    origin: String,
    version: ServiceVersionId,
}
struct Entry {
    serial: u64,
    owner: Weak<ServiceRegistry>,
    scope: String,
    lineage: String,
    snapshot: Option<WorkerSnapshot>,
}
#[derive(Default)]
struct State {
    revision: u64,
    next: u64,
    entries: BTreeMap<Key, Entry>,
    registries: Vec<Weak<ServiceRegistry>>,
    clients: BTreeMap<
        (String, String, String),
        (Weak<ServiceRegistry>, crate::agent_registry::AgentOwner),
    >,
}
#[derive(Default)]
pub(crate) struct ServiceVersionDirectory {
    pub(crate) bridges: Arc<crate::runtime::ProfileMessageBridges>,
    state: Mutex<State>,
    changed: Condvar,
}
pub(crate) struct ServiceVersionReservation {
    directory: Arc<ServiceVersionDirectory>,
    key: Key,
    serial: u64,
}
pub(crate) enum ServiceVersionClaim {
    Reserved(ServiceVersionReservation),
    Existing(Arc<ServiceRegistry>),
}
#[derive(Clone)]
pub(crate) struct ServiceVersionObservation {
    pub owner: Arc<ServiceRegistry>,
    pub scope: String,
    pub lineage: String,
    pub snapshot: WorkerSnapshot,
}
impl ServiceVersionDirectory {
    pub(crate) fn revision(&self) -> Result<u64, StorageError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service directory poisoned".into()))?
            .revision)
    }
    pub(crate) fn wait_changed(
        &self,
        revision: u64,
        timeout: std::time::Duration,
    ) -> Result<(), StorageError> {
        let state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service directory poisoned".into()))?;
        let _guard = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.revision == revision)
            .map_err(|_| StorageError::Persistent("service directory poisoned".into()))?;
        Ok(())
    }

    pub(crate) fn register_registry(
        &self,
        registry: &Arc<ServiceRegistry>,
    ) -> Result<(), StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service directory poisoned".into()))?;
        state.registries.retain(|owner| owner.strong_count() != 0);
        if state
            .registries
            .iter()
            .any(|owner| owner.ptr_eq(&Arc::downgrade(registry)))
        {
            return Ok(());
        }
        if state.registries.len() >= 4096 {
            return Err(StorageError::Persistent(
                "profile service context capacity".into(),
            ));
        }
        state.registries.push(Arc::downgrade(registry));
        Ok(())
    }
    pub(crate) fn registries(&self) -> Result<Vec<Arc<ServiceRegistry>>, StorageError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service directory poisoned".into()))?
            .registries
            .iter()
            .filter_map(Weak::upgrade)
            .collect())
    }
    pub(crate) fn publish_client(
        &self,
        partition: &str,
        origin: &str,
        id: &str,
        registry: &Arc<ServiceRegistry>,
        owner: &crate::agent_registry::AgentOwner,
    ) -> Result<(), StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service client directory poisoned".into()))?;
        state
            .clients
            .retain(|_, (registry, _)| registry.strong_count() != 0);
        let key = (partition.into(), origin.into(), id.into());
        if let Some((previous, previous_owner)) = state.clients.get(&key) {
            if !previous.ptr_eq(&Arc::downgrade(registry)) || previous_owner != owner {
                return Err(StorageError::Persistent(
                    "native service client identity collision".into(),
                ));
            }
            return Ok(());
        }
        if state.clients.len() >= 65536 {
            return Err(StorageError::Persistent(
                "profile service client capacity".into(),
            ));
        }
        state
            .clients
            .insert(key, (Arc::downgrade(registry), owner.clone()));
        Ok(())
    }
    pub(crate) fn client(
        &self,
        partition: &str,
        origin: &str,
        id: &str,
    ) -> Option<(Arc<ServiceRegistry>, crate::agent_registry::AgentOwner)> {
        let state = self.state.lock().ok()?;
        let (registry, owner) = state
            .clients
            .get(&(partition.into(), origin.into(), id.into()))?;
        Some((registry.upgrade()?, owner.clone()))
    }
    pub(crate) fn retire_registry(&self, registry: &Weak<ServiceRegistry>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state
            .clients
            .retain(|_, (owner, _)| !owner.ptr_eq(registry));
        state.registries.retain(|owner| !owner.ptr_eq(registry));
    }
    pub(crate) fn retire_client(&self, id: &str, registry: &Weak<ServiceRegistry>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state
            .clients
            .retain(|(_, _, client), (owner, _)| client != id || !owner.ptr_eq(registry));
    }
    pub(crate) fn claim(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        lineage: &str,
        version: ServiceVersionId,
        owner: &Arc<ServiceRegistry>,
    ) -> Result<ServiceVersionClaim, StorageError> {
        let key = Key {
            partition: partition.into(),
            origin: origin.into(),
            version,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service version directory poisoned".into()))?;
        if let Some(entry) = state.entries.get(&key) {
            if let Some(existing) = entry.owner.upgrade() {
                if entry.scope != scope || entry.lineage != lineage {
                    return Err(StorageError::Persistent(
                        "service version identity conflicts with registration".into(),
                    ));
                }
                return Ok(ServiceVersionClaim::Existing(existing));
            }
        }
        state
            .entries
            .retain(|_, entry| entry.owner.strong_count() != 0);
        if state.entries.len() >= 4096 {
            return Err(StorageError::Persistent(
                "service version directory capacity".into(),
            ));
        }
        state.next = state.next.checked_add(1).ok_or_else(|| {
            StorageError::Persistent("service version directory identities exhausted".into())
        })?;
        let serial = state.next;
        state.entries.insert(
            key.clone(),
            Entry {
                serial,
                owner: Arc::downgrade(owner),
                scope: scope.into(),
                lineage: lineage.into(),
                snapshot: None,
            },
        );
        Ok(ServiceVersionClaim::Reserved(ServiceVersionReservation {
            directory: self.clone(),
            key,
            serial,
        }))
    }
    pub(crate) fn lookup(
        &self,
        partition: &str,
        origin: &str,
        version: ServiceVersionId,
    ) -> Option<Arc<ServiceRegistry>> {
        self.state
            .lock()
            .ok()?
            .entries
            .get(&Key {
                partition: partition.into(),
                origin: origin.into(),
                version,
            })?
            .owner
            .upgrade()
    }
    pub(crate) fn observations(
        &self,
        partition: &str,
        origin: &str,
    ) -> Result<Vec<ServiceVersionObservation>, StorageError> {
        let state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service version directory poisoned".into()))?;
        Ok(state
            .entries
            .iter()
            .filter(|(key, _)| key.partition == partition && key.origin == origin)
            .filter_map(|(_, entry)| {
                Some(ServiceVersionObservation {
                    owner: entry.owner.upgrade()?,
                    scope: entry.scope.clone(),
                    lineage: entry.lineage.clone(),
                    snapshot: entry.snapshot.clone()?,
                })
            })
            .collect())
    }
}
impl ServiceVersionReservation {
    pub(crate) fn publish(&self, snapshot: WorkerSnapshot) -> Result<(), StorageError> {
        if snapshot.version_id != self.key.version {
            return Err(StorageError::Persistent(
                "service version publication identity mismatch".into(),
            ));
        }
        let mut state =
            self.directory.state.lock().map_err(|_| {
                StorageError::Persistent("service version directory poisoned".into())
            })?;
        let entry = state
            .entries
            .get_mut(&self.key)
            .filter(|entry| entry.serial == self.serial)
            .ok_or_else(|| {
                StorageError::Persistent("service version reservation retired".into())
            })?;
        entry.snapshot = Some(snapshot);
        state.revision = state.revision.wrapping_add(1);
        self.directory.changed.notify_all();
        Ok(())
    }
}
impl Drop for ServiceVersionReservation {
    fn drop(&mut self) {
        let mut state = self
            .directory
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state
            .entries
            .get(&self.key)
            .is_some_and(|entry| entry.serial == self.serial)
        {
            state.entries.remove(&self.key);
            state.revision = state.revision.wrapping_add(1);
            self.directory.changed.notify_all();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Browser, BrowserConfig};
    use oryn_js::{service_worker_wire::WorkerState, worker_wire::AgentId};
    #[test]
    fn retirement_notifies_admission_waiters_without_holding_registry_lock() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_context();
        let registry = ServiceRegistry::new(&context);
        let directory = Arc::new(ServiceVersionDirectory::default());
        let ServiceVersionClaim::Reserved(reservation) = directory
            .claim(
                "partition",
                "origin",
                "/",
                "lineage",
                ServiceVersionId([8; 16]),
                &registry,
            )
            .unwrap()
        else {
            panic!("initial owner")
        };
        let revision = directory.revision().unwrap();
        let waiting = directory.clone();
        let waiter = std::thread::spawn(move || {
            waiting
                .wait_changed(revision, std::time::Duration::from_secs(2))
                .unwrap();
            waiting.revision().unwrap()
        });
        drop(reservation);
        assert_ne!(waiter.join().unwrap(), revision);
        assert!(
            directory
                .lookup("partition", "origin", ServiceVersionId([8; 16]))
                .is_none()
        );
        assert!(matches!(
            directory
                .claim(
                    "partition",
                    "origin",
                    "/",
                    "lineage",
                    ServiceVersionId([8; 16]),
                    &registry
                )
                .unwrap(),
            ServiceVersionClaim::Reserved(_)
        ));
    }
    #[test]
    fn reservations_exclude_duplicate_process_owners_and_scope_observations() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let first = browser.new_context();
        let second = browser.new_context();
        let first = ServiceRegistry::new(&first);
        let second = ServiceRegistry::new(&second);
        let directory = Arc::new(ServiceVersionDirectory::default());
        let version = ServiceVersionId([7; 16]);
        let ServiceVersionClaim::Reserved(lease) = directory
            .claim("site", "origin", "/", "lifetime", version, &first)
            .unwrap()
        else {
            panic!("initial reservation")
        };
        let ServiceVersionClaim::Existing(owner) = directory
            .claim("site", "origin", "/", "lifetime", version, &second)
            .unwrap()
        else {
            panic!("duplicate owner admitted")
        };
        assert!(Arc::ptr_eq(&owner, &first));
        assert!(
            directory.observations("site", "origin").unwrap().is_empty(),
            "unpublished admission is not an installing worker"
        );
        let revision = directory.revision().unwrap();
        lease
            .publish(WorkerSnapshot {
                version_id: version,
                id: AgentId(3),
                script_url: "https://example.test/sw.js".into(),
                state: WorkerState::Installing,
            })
            .unwrap();
        assert_ne!(directory.revision().unwrap(), revision);
        directory
            .wait_changed(revision, std::time::Duration::ZERO)
            .unwrap();

        assert_eq!(directory.observations("site", "origin").unwrap().len(), 1);
        assert!(
            directory
                .observations("other-site", "origin")
                .unwrap()
                .is_empty()
        );
        assert!(
            directory
                .lookup("site", "foreign-origin", version)
                .is_none()
        );
        assert!(
            directory
                .claim("site", "origin", "/other", "lifetime", version, &second)
                .is_err()
        );
        let client_owner =
            crate::agent_registry::AgentOwner::Agent(oryn_js::worker_wire::AgentId(9));
        directory.register_registry(&first).unwrap();
        directory
            .publish_client("site", "origin", "client", &first, &client_owner)
            .unwrap();
        assert!(directory.client("site", "other-origin", "client").is_none());
        assert!(
            directory
                .publish_client("site", "origin", "client", &second, &client_owner)
                .is_err()
        );
        assert!(Arc::ptr_eq(
            &directory.client("site", "origin", "client").unwrap().0,
            &first
        ));
        directory.retire_registry(&Arc::downgrade(&first));
        assert!(directory.client("site", "origin", "client").is_none());

        drop(owner);
        drop(first);
        let ServiceVersionClaim::Reserved(replacement) = directory
            .claim("site", "origin", "/", "lifetime", version, &second)
            .unwrap()
        else {
            panic!("dead owner retained")
        };
        drop(lease);
        assert!(
            Arc::ptr_eq(
                &directory.lookup("site", "origin", version).unwrap(),
                &second
            ),
            "old lease must not remove replacement"
        );
        drop(replacement);
        assert!(directory.lookup("site", "origin", version).is_none());
    }
}

impl Drop for ServiceVersionDirectory {
    fn drop(&mut self) {
        let _ = self.bridges.shutdown();
    }
}
