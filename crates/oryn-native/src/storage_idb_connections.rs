//! Profile-shared IndexedDB connection ownership and version-change delivery.
//! Leases and notification acknowledgements are separate from transaction data.
use super::{DatabaseData, StorageError, TransactionSnapshot};
use oryn_js::platform::IdbConnectionEvent;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Default)]
pub(super) struct Connections {
    entries: BTreeMap<String, Connection>,
    next_event: u64,
    next_order: u64,
}
#[derive(Clone)]
struct Connection {
    order: u64,
    origin: String,
    name: String,
    version: u64,
    old_version: u64,
    open: bool,
    closing: bool,
    upgrading: bool,
    upgrade_data: Option<DatabaseData>,
    requested: Option<Option<u64>>,
    notified: BTreeSet<String>,
    acknowledged: BTreeSet<String>,
    events: BTreeMap<u64, (String, IdbConnectionEvent)>,
}
impl Connections {
    fn check(&self, token: &str, origin: &str, name: &str) -> Result<(), StorageError> {
        if self
            .entries
            .get(token)
            .is_some_and(|entry| entry.origin != origin || entry.name != name)
        {
            return Err(StorageError::Persistent(
                "IndexedDB connection identity mismatch".into(),
            ));
        }
        Ok(())
    }
    pub(super) fn reserve(
        &mut self,
        token: &str,
        origin: &str,
        name: &str,
        requested: Option<u64>,
    ) -> Result<(), StorageError> {
        self.check(token, origin, name)?;
        if token.len() > 65536
            || name.len() > 65536
            || self
                .entries
                .iter()
                .map(|(token, entry)| token.len().saturating_add(entry.name.len()))
                .sum::<usize>()
                .saturating_add(token.len())
                .saturating_add(name.len())
                > 8 * 1024 * 1024
        {
            return Err(StorageError::Persistent(
                "IndexedDB connection metadata capacity".into(),
            ));
        }
        if let Some(entry) = self.entries.get(token) {
            if entry.requested != Some(requested) {
                return Err(StorageError::Persistent(
                    "IndexedDB connection request changed".into(),
                ));
            }
            return Ok(());
        }
        if self.entries.len() >= 1024 {
            return Err(StorageError::Persistent(
                "IndexedDB connection capacity".into(),
            ));
        }
        self.next_order = self.next_order.checked_add(1).ok_or_else(|| {
            StorageError::Persistent("IndexedDB connection job order exhausted".into())
        })?;
        self.entries.insert(
            token.into(),
            Connection {
                order: self.next_order,
                origin: origin.into(),
                name: name.into(),
                version: 0,
                old_version: 0,
                open: false,
                closing: false,
                upgrading: false,
                upgrade_data: None,
                requested: Some(requested),
                notified: BTreeSet::new(),
                acknowledged: BTreeSet::new(),
                events: BTreeMap::new(),
            },
        );
        Ok(())
    }
    pub(super) fn job_ready(&self, token: &str) -> bool {
        let Some(current) = self.entries.get(token) else {
            return false;
        };
        !self.entries.values().any(|entry| {
            entry.origin == current.origin
                && entry.name == current.name
                && (entry.requested.is_some() || entry.upgrading)
                && entry.order < current.order
        })
    }
    /// None means eligible. Some(false) is waiting for notification ACK;
    /// Some(true) means all notifications ran but connections remain open.
    pub(super) fn changing(
        &mut self,
        token: &str,
        origin: &str,
        name: &str,
        new_version: Option<u64>,
    ) -> Result<Option<bool>, StorageError> {
        self.check(token, origin, name)?;
        let blockers = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                id.as_str() != token && entry.origin == origin && entry.name == name && entry.open
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for blocker in &blockers {
            // A close-pending connection cannot run another versionchange event.
            // Its existing transactions still block the upgrade until drained.
            if self.entries[blocker].closing || self.entries[token].notified.contains(blocker) {
                continue;
            }
            self.next_event = self.next_event.checked_add(1).ok_or_else(|| {
                StorageError::Persistent("IndexedDB notification identity exhausted".into())
            })?;
            let entry = self.entries.get_mut(blocker).expect("connection blocker");
            if entry.events.len() >= 1024 {
                return Err(StorageError::Persistent(
                    "IndexedDB notification capacity".into(),
                ));
            }
            entry.events.insert(
                self.next_event,
                (
                    token.into(),
                    IdbConnectionEvent {
                        event: self.next_event,
                        old_version: entry.version,
                        new_version,
                    },
                ),
            );
            self.entries
                .get_mut(token)
                .expect("connection job")
                .notified
                .insert(blocker.clone());
        }
        if blockers.is_empty() {
            return Ok(None);
        }
        let job = &self.entries[token];
        Ok(Some(blockers.iter().all(|blocker| {
            self.entries[blocker].closing || job.acknowledged.contains(blocker)
        })))
    }
    pub(super) fn opened(
        &mut self,
        token: &str,
        version: u64,
        old_version: u64,
        upgrade_data: Option<DatabaseData>,
    ) -> Result<(), StorageError> {
        self.check_upgrade_capacity(token, upgrade_data.as_ref())?;
        if let Some(entry) = self.entries.get_mut(token) {
            entry.open = true;
            entry.version = version;
            entry.old_version = old_version;
            entry.requested = None;
            entry.upgrading = upgrade_data.is_some();
            entry.upgrade_data = upgrade_data;
        }
        Ok(())
    }
    pub(super) fn finish_upgrade(&mut self, origin: &str, name: &str) {
        for entry in self.entries.values_mut() {
            if entry.origin == origin && entry.name == name {
                entry.upgrading = false;
                entry.upgrade_data = None;
            }
        }
    }
    pub(super) fn validate_live(
        &self,
        token: &str,
        origin: &str,
        name: &str,
    ) -> Result<(), StorageError> {
        self.check(token, origin, name)?;
        if !self
            .entries
            .get(token)
            .is_some_and(|entry| entry.open && !entry.closing)
        {
            return Err(StorageError::Persistent(
                "InvalidStateError: IndexedDB connection closed".into(),
            ));
        }
        Ok(())
    }
    pub(super) fn poll(
        &self,
        token: &str,
        origin: &str,
        name: &str,
    ) -> Result<Vec<IdbConnectionEvent>, StorageError> {
        self.check(token, origin, name)?;
        Ok(self
            .entries
            .get(token)
            .map(|entry| {
                entry
                    .events
                    .values()
                    .take(32)
                    .map(|(_, event)| event.clone())
                    .collect()
            })
            .unwrap_or_default())
    }
    pub(super) fn acknowledge(
        &mut self,
        token: &str,
        origin: &str,
        name: &str,
        event: u64,
    ) -> Result<(), StorageError> {
        self.check(token, origin, name)?;
        if let Some((job, _)) = self
            .entries
            .get_mut(token)
            .and_then(|entry| entry.events.remove(&event))
            && let Some(job) = self.entries.get_mut(&job)
        {
            job.acknowledged.insert(token.into());
        }
        Ok(())
    }
    pub(super) fn close(
        &mut self,
        token: &str,
        origin: &str,
        name: &str,
    ) -> Result<(), StorageError> {
        self.check(token, origin, name)?;
        if let Some(entry) = self.entries.get_mut(token) {
            entry.closing = true;
        }
        Ok(())
    }
    pub(super) fn upgrade(
        &self,
        token: &str,
        origin: &str,
        name: &str,
    ) -> Result<DatabaseData, StorageError> {
        self.check(token, origin, name)?;
        self.entries
            .get(token)
            .and_then(|entry| entry.upgrade_data.clone())
            .ok_or_else(|| {
                StorageError::Persistent(
                    "InvalidStateError: connection has no private upgrade".into(),
                )
            })
    }
    pub(super) fn stage_upgrade(
        &mut self,
        token: &str,
        data: DatabaseData,
    ) -> Result<(), StorageError> {
        self.check_upgrade_capacity(token, Some(&data))?;
        if let Some(entry) = self.entries.get_mut(token) {
            entry.upgrade_data = Some(data);
        }
        Ok(())
    }
    fn check_upgrade_capacity(
        &self,
        token: &str,
        replacement: Option<&DatabaseData>,
    ) -> Result<(), StorageError> {
        fn bytes(data: &DatabaseData) -> Result<usize, StorageError> {
            let own = serde_json::to_vec(data)
                .map_err(|error| StorageError::Persistent(error.to_string()))?
                .len();
            Ok(own.saturating_add(
                data.upgrade_backup
                    .as_deref()
                    .map(bytes)
                    .transpose()?
                    .unwrap_or(0),
            ))
        }
        let retained = self
            .entries
            .iter()
            .filter(|(id, _)| id.as_str() != token)
            .filter_map(|(_, entry)| entry.upgrade_data.as_ref())
            .chain(replacement)
            .try_fold(0usize, |total, data| {
                bytes(data).map(|bytes| total.saturating_add(bytes))
            })?;
        if retained > 256 * 1024 * 1024 {
            return Err(StorageError::Persistent(
                "IndexedDB private upgrade capacity".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, token: &str) {
        self.entries.remove(token);
        for entry in self.entries.values_mut() {
            entry.events.retain(|_, (job, _)| job != token);
        }
    }
    pub(super) fn reap_closed(&mut self, transactions: &BTreeMap<String, TransactionSnapshot>) {
        let closed = self
            .entries
            .iter()
            .filter(|(token, entry)| {
                entry.closing
                    && !entry.upgrading
                    && !transactions
                        .values()
                        .any(|transaction| transaction.connection.as_ref() == Some(*token))
            })
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in closed {
            self.remove(&token);
        }
    }
}
