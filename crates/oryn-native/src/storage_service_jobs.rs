//! Profile-wide service registration job ordering. A lease owns only a native
//! scope queue slot, never a database, page, broker, or process mutex.
use super::StorageError;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Key = (String, String, String);
#[derive(Default)]
struct Queue {
    active: Option<u64>,
    waiting: VecDeque<u64>,
}
#[derive(Default)]
struct State {
    next: u64,
    queues: BTreeMap<Key, Queue>,
    scopes: BTreeMap<Key, Scope>,
}
#[derive(Default)]
struct Scope {
    clients: BTreeMap<String, usize>,
    events: BTreeMap<String, usize>,
    activating: Option<String>,
    active_work_ready: bool,
    watches: BTreeMap<u64, Arc<dyn Fn() + Send + Sync>>,
}
pub(crate) struct ServiceClientLease {
    coordinator: Arc<ServiceJobCoordinator>,
    key: Key,
    lifetime: String,
}
pub(crate) struct ServiceActivityLease {
    coordinator: Arc<ServiceJobCoordinator>,
    key: Key,
    lifetime: String,
}
pub(crate) struct ServiceActivationLease {
    coordinator: Arc<ServiceJobCoordinator>,
    key: Key,
}
pub(crate) struct ServiceScopeWatch {
    coordinator: Arc<ServiceJobCoordinator>,
    key: Key,
    id: u64,
}
#[derive(Default)]
pub struct ServiceJobCoordinator {
    #[cfg(unix)]
    pub(crate) equivalent: crate::service_registry::ProfileJobs,
    pub(crate) versions: Arc<super::ServiceVersionDirectory>,
    state: Mutex<State>,
    changed: Condvar,
}
pub(crate) struct ServiceJobLease {
    coordinator: Arc<ServiceJobCoordinator>,
    key: Key,
    id: u64,
}
impl ServiceJobCoordinator {
    fn scope_key(partition: &str, origin: &str, scope: &str) -> Key {
        (partition.into(), origin.into(), scope.into())
    }
    pub(crate) fn retain_client(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
    ) -> Result<ServiceClientLease, StorageError> {
        self.retain_client_for_activation(partition, origin, scope, "", false)
    }
    pub(crate) fn retain_client_for_activation(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        lifetime: &str,
        claimed_by_activating_worker: bool,
    ) -> Result<ServiceClientLease, StorageError> {
        let key = Self::scope_key(partition, origin, scope);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service ownership poisoned".into()))?;
        if state.scopes.len() >= 4096 && !state.scopes.contains_key(&key) {
            return Err(StorageError::Persistent(
                "service ownership scope capacity".into(),
            ));
        }
        let entry = state.scopes.entry(key.clone()).or_default();
        if entry.activating.as_deref() == Some(lifetime)
            && !entry.active_work_ready
            && !claimed_by_activating_worker
        {
            return Err(StorageError::Persistent(
                "service activation in progress; retry client selection".into(),
            ));
        }
        if entry.clients.values().sum::<usize>() >= 65536 {
            return Err(StorageError::Persistent("service client capacity".into()));
        }
        *entry.clients.entry(lifetime.into()).or_default() += 1;
        Ok(ServiceClientLease {
            coordinator: self.clone(),
            key,
            lifetime: lifetime.into(),
        })
    }
    pub(crate) fn retain_event(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
    ) -> Result<ServiceActivityLease, StorageError> {
        self.retain_event_for_lifetime(partition, origin, scope, "")
    }
    pub(crate) fn retain_event_for_lifetime(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        lifetime: &str,
    ) -> Result<ServiceActivityLease, StorageError> {
        let key = Self::scope_key(partition, origin, scope);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service ownership poisoned".into()))?;
        if state.scopes.len() >= 4096 && !state.scopes.contains_key(&key) {
            return Err(StorageError::Persistent(
                "service ownership scope capacity".into(),
            ));
        }
        let entry = state.scopes.entry(key.clone()).or_default();
        if entry.activating.as_deref() == Some(lifetime) && !entry.active_work_ready {
            return Err(StorageError::Persistent(
                "service activation in progress".into(),
            ));
        }
        if entry.events.values().sum::<usize>() >= 65536 {
            return Err(StorageError::Persistent("service event capacity".into()));
        }
        *entry.events.entry(lifetime.into()).or_default() += 1;
        Ok(ServiceActivityLease {
            coordinator: self.clone(),
            key,
            lifetime: lifetime.into(),
        })
    }
    pub(crate) fn begin_activation(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        skip_waiting: bool,
    ) -> Result<Option<ServiceActivationLease>, StorageError> {
        self.begin_activation_for_lifetime(partition, origin, scope, "", skip_waiting)
    }
    pub(crate) fn begin_activation_for_lifetime(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        lifetime: &str,
        skip_waiting: bool,
    ) -> Result<Option<ServiceActivationLease>, StorageError> {
        let key = Self::scope_key(partition, origin, scope);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service ownership poisoned".into()))?;
        if state.scopes.len() >= 4096 && !state.scopes.contains_key(&key) {
            return Err(StorageError::Persistent(
                "service ownership scope capacity".into(),
            ));
        }
        if state
            .queues
            .get(&key)
            .is_some_and(|queue| queue.active.is_some())
        {
            return Ok(None);
        }
        let entry = state.scopes.entry(key.clone()).or_default();
        if entry.activating.is_some()
            || entry.events.get(lifetime).copied().unwrap_or(0) != 0
            || (!skip_waiting && entry.clients.get(lifetime).copied().unwrap_or(0) != 0)
        {
            return Ok(None);
        }
        entry.activating = Some(lifetime.into());
        entry.active_work_ready = false;
        Ok(Some(ServiceActivationLease {
            coordinator: self.clone(),
            key,
        }))
    }
    pub(crate) fn watch(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<ServiceScopeWatch, StorageError> {
        let key = Self::scope_key(partition, origin, scope);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service ownership poisoned".into()))?;
        if state.scopes.len() >= 4096 && !state.scopes.contains_key(&key) {
            return Err(StorageError::Persistent(
                "service ownership scope capacity".into(),
            ));
        }
        if state
            .scopes
            .get(&key)
            .is_some_and(|entry| entry.watches.len() >= 256)
        {
            return Err(StorageError::Persistent("service observer capacity".into()));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| StorageError::Persistent("service identities exhausted".into()))?;
        let id = state.next;
        state
            .scopes
            .entry(key.clone())
            .or_default()
            .watches
            .insert(id, wake);
        Ok(ServiceScopeWatch {
            coordinator: self.clone(),
            key,
            id,
        })
    }
    pub(crate) fn notify_changed(&self, partition: &str, origin: &str, scope: &str) {
        let callbacks = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .scopes
                .get(&Self::scope_key(partition, origin, scope))
                .map(|scope| scope.watches.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        for callback in callbacks {
            callback();
        }
    }
    pub(crate) fn acquire(
        self: &Arc<Self>,
        partition: &str,
        origin: &str,
        scope: &str,
        cancel: &AtomicBool,
    ) -> Result<ServiceJobLease, StorageError> {
        let key = (partition.to_owned(), origin.to_owned(), scope.to_owned());
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::Persistent("service job coordinator poisoned".into()))?;
        if state.queues.len() >= 4096 && !state.queues.contains_key(&key) {
            return Err(StorageError::Persistent(
                "service scope queue capacity".into(),
            ));
        }
        if state
            .queues
            .get(&key)
            .is_some_and(|queue| queue.waiting.len() >= 256)
        {
            return Err(StorageError::Persistent(
                "service scope waiter capacity".into(),
            ));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| StorageError::Persistent("service job identities exhausted".into()))?;
        let id = state.next;
        state
            .queues
            .entry(key.clone())
            .or_default()
            .waiting
            .push_back(id);
        loop {
            if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                if let Some(queue) = state.queues.get_mut(&key) {
                    queue.waiting.retain(|ticket| *ticket != id);
                    if queue.active.is_none() && queue.waiting.is_empty() {
                        state.queues.remove(&key);
                    }
                }
                self.changed.notify_all();
                return Err(StorageError::Persistent(
                    "service scope job cancelled or timed out".into(),
                ));
            }
            let activating = state
                .scopes
                .get(&key)
                .is_some_and(|scope| scope.activating.is_some());
            let queue = state.queues.get_mut(&key).expect("admitted service queue");
            if !activating && queue.active.is_none() && queue.waiting.front() == Some(&id) {
                queue.waiting.pop_front();
                queue.active = Some(id);
                return Ok(ServiceJobLease {
                    coordinator: self.clone(),
                    key,
                    id,
                });
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }
}
fn prune_scope(state: &mut State, key: &Key) {
    if state.scopes.get(key).is_some_and(|scope| {
        scope.clients.is_empty()
            && scope.events.is_empty()
            && scope.activating.is_none()
            && scope.watches.is_empty()
    }) {
        state.scopes.remove(key);
    }
}
impl Drop for ServiceClientLease {
    fn drop(&mut self) {
        let wake = {
            let mut state = self
                .coordinator
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let wake = if let Some(scope) = state.scopes.get_mut(&self.key) {
                if let Some(count) = scope.clients.get_mut(&self.lifetime) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        scope.clients.remove(&self.lifetime);
                    }
                }
                if !scope.clients.contains_key(&self.lifetime) {
                    scope.watches.values().cloned().collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            prune_scope(&mut state, &self.key);
            wake
        };
        // Native observers enqueue work only. Never invoke them under the profile lock.
        for callback in wake {
            callback();
        }
    }
}
impl Drop for ServiceActivityLease {
    fn drop(&mut self) {
        let wake = {
            let mut state = self
                .coordinator
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let wake = if let Some(scope) = state.scopes.get_mut(&self.key) {
                if let Some(count) = scope.events.get_mut(&self.lifetime) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        scope.events.remove(&self.lifetime);
                    }
                }
                if !scope.events.contains_key(&self.lifetime) {
                    scope.watches.values().cloned().collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            prune_scope(&mut state, &self.key);
            wake
        };
        for callback in wake {
            callback();
        }
    }
}
impl ServiceActivationLease {
    /// The activate event has completed, so clients and events may observe the
    /// new active worker. Keep the profile job barrier until durable publication
    /// completes and this lease is dropped.
    pub(crate) fn allow_active_work(&self) -> Result<(), StorageError> {
        let mut state = self.coordinator.state.lock().map_err(|_| {
            StorageError::Persistent("service activation ownership poisoned".into())
        })?;
        let scope = state
            .scopes
            .get_mut(&self.key)
            .filter(|scope| scope.activating.is_some())
            .ok_or_else(|| StorageError::Persistent("service activation lease retired".into()))?;
        scope.active_work_ready = true;
        Ok(())
    }
}
impl Drop for ServiceActivationLease {
    fn drop(&mut self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(scope) = state.scopes.get_mut(&self.key) {
            scope.activating = None;
            scope.active_work_ready = false;
        }
        prune_scope(&mut state, &self.key);
        self.coordinator.changed.notify_all();
    }
}
impl Drop for ServiceScopeWatch {
    fn drop(&mut self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(scope) = state.scopes.get_mut(&self.key) {
            scope.watches.remove(&self.id);
        }
        prune_scope(&mut state, &self.key);
    }
}
impl Drop for ServiceJobLease {
    fn drop(&mut self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(queue) = state.queues.get_mut(&self.key) {
            if queue.active == Some(self.id) {
                queue.active = None;
            }
            if queue.active.is_none() && queue.waiting.is_empty() {
                state.queues.remove(&self.key);
            }
        }
        self.coordinator.changed.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activated_work_is_ready_while_competing_profile_jobs_wait_for_publication() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let activation = coordinator
            .begin_activation_for_lifetime("p", "o", "/", "new", true)
            .unwrap()
            .unwrap();
        assert!(
            coordinator
                .retain_client_for_activation("p", "o", "/", "new", false)
                .is_err()
        );
        assert!(
            coordinator
                .retain_event_for_lifetime("p", "o", "/", "new")
                .is_err()
        );
        let other = coordinator
            .retain_event_for_lifetime("p", "o", "/", "other")
            .unwrap();
        let published = Arc::new(AtomicBool::new(false));
        let worker_coordinator = coordinator.clone();
        let worker_published = published.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let job = worker_coordinator
                .acquire("p", "o", "/", &AtomicBool::new(false))
                .unwrap();
            assert!(
                worker_published.load(Ordering::Acquire),
                "competing job entered before durable activation publication"
            );
            send.send(()).unwrap();
            drop(job);
        });
        let key = ServiceJobCoordinator::scope_key("p", "o", "/");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !coordinator
            .state
            .lock()
            .unwrap()
            .queues
            .get(&key)
            .is_some_and(|queue| !queue.waiting.is_empty())
        {
            assert!(
                Instant::now() < deadline,
                "competing profile job did not queue"
            );
            std::thread::yield_now();
        }
        activation.allow_active_work().unwrap();
        let client = coordinator
            .retain_client_for_activation("p", "o", "/", "new", false)
            .unwrap();
        let event = coordinator
            .retain_event_for_lifetime("p", "o", "/", "new")
            .unwrap();
        assert!(matches!(
            receive.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        published.store(true, Ordering::Release);
        drop(activation);
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
        drop(client);
        drop(event);
        drop(other);
        // A subsequent activation starts closed again; readiness never leaks
        // into a later logical registration or retirement lease.
        let next = coordinator
            .begin_activation_for_lifetime("p", "o", "/", "next", true)
            .unwrap()
            .unwrap();
        assert!(
            coordinator
                .retain_event_for_lifetime("p", "o", "/", "next")
                .is_err()
        );
        assert!(
            coordinator
                .retain_client_for_activation("p", "o", "/", "next", false)
                .is_err()
        );
        drop(next);
        assert!(coordinator.state.lock().unwrap().scopes.is_empty());
    }

    #[test]
    fn unregistered_retirement_excludes_sibling_clients_and_events_atomically() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let sibling = coordinator
            .retain_client_for_activation("profile", "origin", "/scope", "lineage", false)
            .unwrap();
        let event = coordinator
            .retain_event_for_lifetime("profile", "origin", "/scope", "lineage")
            .unwrap();
        assert!(
            coordinator
                .begin_activation_for_lifetime("profile", "origin", "/scope", "lineage", false)
                .unwrap()
                .is_none()
        );
        drop(sibling);
        assert!(
            coordinator
                .begin_activation_for_lifetime("profile", "origin", "/scope", "lineage", false)
                .unwrap()
                .is_none()
        );
        drop(event);
        let retirement = coordinator
            .begin_activation_for_lifetime("profile", "origin", "/scope", "lineage", false)
            .unwrap()
            .unwrap();
        assert!(
            coordinator
                .retain_client_for_activation("profile", "origin", "/scope", "lineage", false)
                .is_err()
        );
        assert!(
            coordinator
                .retain_event_for_lifetime("profile", "origin", "/scope", "lineage")
                .is_err()
        );
        // A later registration at the same scope owns a different lifetime.
        let replacement = coordinator
            .retain_client_for_activation("profile", "origin", "/scope", "replacement", false)
            .unwrap();
        drop(replacement);
        drop(retirement);
        assert!(coordinator.state.lock().unwrap().scopes.is_empty());
    }
    #[test]
    fn reregistered_scope_has_distinct_client_and_event_lifetime() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let old_client = coordinator
            .retain_client_for_activation("p", "o", "/", "old", false)
            .unwrap();
        let old_event = coordinator
            .retain_event_for_lifetime("p", "o", "/", "old")
            .unwrap();
        assert!(
            coordinator
                .begin_activation_for_lifetime("p", "o", "/", "old", true)
                .unwrap()
                .is_none()
        );
        let new_activation = coordinator
            .begin_activation_for_lifetime("p", "o", "/", "new", false)
            .unwrap()
            .unwrap();
        let concurrent_old_event = coordinator
            .retain_event_for_lifetime("p", "o", "/", "old")
            .unwrap();
        assert!(
            coordinator
                .retain_event_for_lifetime("p", "o", "/", "new")
                .is_err()
        );
        drop(concurrent_old_event);
        drop(new_activation);
        drop(old_event);
        drop(old_client);
        assert!(coordinator.state.lock().unwrap().scopes.is_empty());
    }
    #[test]
    fn sibling_fetch_lifetime_blocks_skip_waiting_and_profile_jobs_exclude_activation() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let stopped = AtomicBool::new(false);
        let event = coordinator.retain_event("p", "o", "/").unwrap();
        assert!(
            coordinator
                .begin_activation("p", "o", "/", true)
                .unwrap()
                .is_none()
        );
        drop(event);
        let update = coordinator.acquire("p", "o", "/", &stopped).unwrap();
        assert!(
            coordinator
                .begin_activation("p", "o", "/", true)
                .unwrap()
                .is_none()
        );
        drop(update);
        let activation = coordinator
            .begin_activation("p", "o", "/", true)
            .unwrap()
            .unwrap();
        assert!(coordinator.retain_event("p", "o", "/").is_err());
        drop(activation);
        let event = coordinator.retain_event("p", "o", "/").unwrap();
        drop(event);
        assert!(coordinator.state.lock().unwrap().scopes.is_empty());
    }
    #[test]
    fn retained_clients_block_sibling_activation_and_last_release_wakes_outside_lock() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let flag = notifications.clone();
        let weak = Arc::downgrade(&coordinator);
        let watch = coordinator
            .watch(
                "profile-site",
                "native-origin",
                "/",
                Arc::new(move || {
                    assert!(weak.upgrade().unwrap().state.try_lock().is_ok());
                    flag.fetch_add(1, Ordering::AcqRel);
                }),
            )
            .unwrap();
        let first = coordinator
            .retain_client("profile-site", "native-origin", "/")
            .unwrap();
        // A cached document retains its lease without a runnable process.
        let cached = coordinator
            .retain_client("profile-site", "native-origin", "/")
            .unwrap();
        assert!(
            coordinator
                .begin_activation("profile-site", "native-origin", "/", false)
                .unwrap()
                .is_none()
        );
        drop(first);
        assert_eq!(notifications.load(Ordering::Acquire), 0);
        drop(cached);
        assert_eq!(notifications.load(Ordering::Acquire), 1);
        let activation = coordinator
            .begin_activation("profile-site", "native-origin", "/", false)
            .unwrap()
            .unwrap();
        assert!(
            coordinator
                .retain_client("profile-site", "native-origin", "/")
                .is_err()
        );
        assert!(
            coordinator
                .begin_activation("profile-site", "native-origin", "/", true)
                .unwrap()
                .is_none()
        );
        let other = coordinator
            .retain_client("different-site", "native-origin", "/")
            .unwrap();
        drop(other);
        drop(activation);
        let fresh = coordinator
            .retain_client("profile-site", "native-origin", "/")
            .unwrap();
        drop(watch);
        drop(fresh);
        assert!(coordinator.state.lock().unwrap().scopes.is_empty());
    }
    #[test]
    fn skip_waiting_uses_one_exclusive_activation_without_revoking_existing_client_leases() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let client = coordinator.retain_client("p", "o", "/").unwrap();
        let activation = coordinator
            .begin_activation("p", "o", "/", true)
            .unwrap()
            .unwrap();
        assert_eq!(
            coordinator.state.lock().unwrap().scopes[&("p".into(), "o".into(), "/".into())]
                .clients
                .get("")
                .copied(),
            Some(1)
        );
        assert!(
            coordinator
                .begin_activation("p", "o", "/", true)
                .unwrap()
                .is_none()
        );
        drop(activation);
        assert!(
            coordinator
                .begin_activation("p", "o", "/", false)
                .unwrap()
                .is_none()
        );
        drop(client);
        assert!(
            coordinator
                .begin_activation("p", "o", "/", false)
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn same_profile_scope_serializes_while_other_scopes_progress() {
        let coordinator = Arc::new(ServiceJobCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let first = coordinator
            .acquire("site", "origin", "/app", &cancel)
            .unwrap();
        let independent = coordinator
            .acquire("site", "origin", "/other", &cancel)
            .unwrap();
        drop(independent);
        let next = coordinator.clone();
        let stopped = cancel.clone();
        let acquired = Arc::new(AtomicBool::new(false));
        let flag = acquired.clone();
        let waiter = std::thread::spawn(move || {
            let lease = next.acquire("site", "origin", "/app", &stopped).unwrap();
            flag.store(true, Ordering::Release);
            lease
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while coordinator.state.lock().unwrap().queues
            [&("site".into(), "origin".into(), "/app".into())]
            .waiting
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!acquired.load(Ordering::Acquire));
        drop(first);
        drop(waiter.join().unwrap());
        assert!(acquired.load(Ordering::Acquire));
        assert!(coordinator.state.lock().unwrap().queues.is_empty());
    }
}
