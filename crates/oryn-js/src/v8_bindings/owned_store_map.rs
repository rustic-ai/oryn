//! Staged admitted collection for database current and rollback store graphs.
use super::owned_index_metadata::StoreMetadata;
use crate::idb_owned_admission::{Buffer, Ledger};
use std::sync::Arc;

pub(crate) struct StoreMap {
    entries: Buffer<StoreMetadata>,
}
impl StoreMap {
    #[cfg(test)]
    pub(crate) fn empty(
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let entries = Buffer::allocate(ledger, 0, limit, used)?;
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn get(&self, name: &str) -> Option<&StoreMetadata> {
        self.entries
            .as_slice()
            .binary_search_by(|entry| entry.name.as_str().cmp(name))
            .ok()
            .map(|i| &self.entries.as_slice()[i])
    }
    pub(crate) fn values(&self) -> &[StoreMetadata] {
        self.entries.as_slice()
    }
    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    pub(crate) fn from_wire(
        values: &[crate::platform::IdbObjectStoreDefinition],
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut entries = Buffer::allocate(ledger, values.len(), limit, used)?;
        for value in values {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            entries.push(StoreMetadata::from_wire(
                value,
                ledger,
                limit,
                used,
                &mut *cancelled,
            )?)?;
        }
        super::owned_index_metadata::sort_cancelled(
            entries.as_mut_slice(),
            |a, b, cancel| {
                super::owned_index_metadata::compare_cancelled(
                    a.name.as_str(),
                    b.name.as_str(),
                    cancel,
                )
            },
            cancelled,
        )?;
        for pair in entries.as_slice().windows(2) {
            if super::owned_index_metadata::compare_cancelled(
                pair[0].name.as_str(),
                pair[1].name.as_str(),
                cancelled,
            )?
            .is_eq()
            {
                return Err("duplicate parent store definition");
            }
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn copy_admitted(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut entries = Buffer::allocate(ledger, self.values().len(), limit, used)?;
        for value in self.values() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            entries.push(value.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn replaced(
        &self,
        new: StoreMetadata,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let exists = self.get(new.name.as_str()).is_some();
        let len = self
            .values()
            .len()
            .checked_add(usize::from(!exists))
            .ok_or("store map overflow")?;
        let mut entries = Buffer::allocate(ledger, len, limit, used)?;
        let mut new = Some(new);
        for value in self.values() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if new
                .as_ref()
                .map(|new| {
                    super::owned_index_metadata::compare_cancelled(
                        new.name.as_str(),
                        value.name.as_str(),
                        cancelled,
                    )
                })
                .transpose()?
                .is_some_and(|order| !order.is_gt())
            {
                let same = super::owned_index_metadata::compare_cancelled(
                    new.as_ref().unwrap().name.as_str(),
                    value.name.as_str(),
                    cancelled,
                )?
                .is_eq();
                entries.push(new.take().unwrap())?;
                if same {
                    continue;
                }
            }
            entries.push(value.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        if let Some(new) = new {
            entries.push(new)?
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn removed(
        &self,
        name: &str,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut count = 0;
        for entry in self.values() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if !super::owned_index_metadata::compare_cancelled(
                entry.name.as_str(),
                name,
                cancelled,
            )?
            .is_eq()
            {
                count += 1;
            }
        }
        let mut entries = Buffer::allocate(ledger, count, limit, used)?;
        for value in self.values() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if super::owned_index_metadata::compare_cancelled(value.name.as_str(), name, cancelled)?
                .is_eq()
            {
                continue;
            }
            entries.push(value.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn renamed(
        &self,
        old: &str,
        new: crate::idb_owned_admission::Text,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        if old != new.as_str() && self.get(new.as_str()).is_some() {
            return Err("duplicate object store");
        }
        let mut value = self.get(old).ok_or("missing object store")?.copy_admitted(
            ledger,
            limit,
            used,
            &mut *cancelled,
        )?;
        value.name = new;
        let removed = self.removed(old, ledger, limit, used, &mut *cancelled)?;
        removed.replaced(value, ledger, limit, used, &mut *cancelled)
    }
}
