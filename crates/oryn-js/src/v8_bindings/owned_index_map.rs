//! Staged admitted native index map, preserving native name lookup semantics.
use super::owned_index_metadata::IndexMetadata;
use crate::idb_owned_admission::{Buffer, Ledger};
use std::sync::Arc;

pub(crate) struct IndexMap {
    entries: Buffer<IndexMetadata>,
}
impl IndexMap {
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
    pub(crate) fn entries(&self) -> &[IndexMetadata] {
        self.entries.as_slice()
    }
    pub(crate) fn from_metadata(
        values: &[IndexMetadata],
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut entries = Buffer::allocate(ledger, values.len(), limit, used)?;
        for entry in values {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            entries.push(entry.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        super::owned_index_metadata::sort_cancelled(
            entries.as_mut_slice(),
            |a, b, cancel| {
                super::owned_index_metadata::compare_cancelled(a.name(), b.name(), cancel)
            },
            cancelled,
        )?;
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    pub(crate) fn get(&self, name: &str) -> Option<&IndexMetadata> {
        self.entries()
            .binary_search_by(|index| index.name().cmp(name))
            .ok()
            .map(|index| &self.entries()[index])
    }
    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    #[cfg(test)]
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
        let mut entries = Buffer::allocate(ledger, self.entries().len(), limit, used)?;
        for entry in self.entries() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            entries.push(entry.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { entries })
    }
    /// Preparation is non-mutating. Publish by replacing the map only after all
    /// parallel database/record/request preparation has succeeded.
    pub(crate) fn inserted(
        &self,
        new: IndexMetadata,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        if self.contains_key(new.name()) {
            return Err("duplicate index");
        }
        let count = self
            .entries()
            .len()
            .checked_add(1)
            .ok_or("index map overflow")?;
        let mut entries = Buffer::allocate(ledger, count, limit, used)?;
        let mut new = Some(new);
        for entry in self.entries() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if new
                .as_ref()
                .map(|new| {
                    super::owned_index_metadata::compare_cancelled(
                        new.name(),
                        entry.name(),
                        cancelled,
                    )
                })
                .transpose()?
                .is_some_and(|order| order.is_lt())
            {
                entries.push(new.take().unwrap())?;
            }
            entries.push(entry.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        if let Some(new) = new {
            entries.push(new)?;
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
        if !self.contains_key(name) {
            return Err("missing index");
        }
        let mut entries = Buffer::allocate(ledger, self.entries().len() - 1, limit, used)?;
        for entry in self.entries() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if super::owned_index_metadata::compare_cancelled(entry.name(), name, cancelled)?
                .is_eq()
            {
                continue;
            }
            entries.push(entry.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
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
        let original = self.get(old).ok_or("missing index")?;
        if old != new.as_str() && self.contains_key(new.as_str()) {
            return Err("duplicate index");
        }
        let key_path = original.copied_keypath(ledger, limit, used, &mut *cancelled)?;
        let replacement = IndexMetadata::new(new, key_path, original.unique, original.multi_entry);
        let without = self.removed(old, ledger, limit, used, &mut *cancelled)?;
        without.inserted(replacement, ledger, limit, used, &mut *cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_map_final_cancel_releases() {
        let ledger = Arc::new(Ledger::default());
        let mut calls = 0;
        assert!(
            IndexMap::empty(&ledger, 8192, 0, &mut || {
                calls += 1;
                calls == 2
            })
            .is_err()
        );
        assert_eq!(calls, 2);
        assert_eq!(ledger.live_bytes(), 0);
        let mut calls = 0;
        assert!(
            super::super::owned_store_map::StoreMap::empty(&ledger, 8192, 0, &mut || {
                calls += 1;
                calls == 2
            })
            .is_err()
        );
        assert_eq!(calls, 2);
        assert_eq!(ledger.live_bytes(), 0);
    }
    use crate::idb_owned_admission::Text;
    fn entry(ledger: &Arc<Ledger>, name: &str) -> IndexMetadata {
        IndexMetadata::new(
            Text::copy_string(ledger, name, 8192, 0).unwrap(),
            Text::copy_string(ledger, "valid", 8192, 0).unwrap(),
            false,
            false,
        )
    }
    #[test]
    fn map_preparation_is_sorted_atomic_and_fully_released() {
        let ledger = Arc::new(Ledger::default());
        let original = IndexMap::empty(&ledger, 8192, 0, &mut || false).unwrap();
        let first = original
            .inserted(entry(&ledger, "z"), &ledger, 8192, 0, &mut || false)
            .unwrap();
        let map = first
            .inserted(entry(&ledger, "a"), &ledger, 8192, 0, &mut || false)
            .unwrap();
        assert_eq!(
            map.entries()
                .iter()
                .map(|entry| entry.name())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        let before = ledger.live_bytes();
        assert!(
            map.copy_admitted(&ledger, before, 0, &mut || false)
                .is_err()
        );
        assert_eq!(ledger.live_bytes(), before);
        assert!(map.contains_key("z"));
        let smaller = map.removed("a", &ledger, 8192, 0, &mut || false).unwrap();
        assert!(!smaller.contains_key("a"));
        assert!(map.contains_key("a"));
        drop(smaller);
        drop(map);
        drop(first);
        drop(original);
        assert_eq!(ledger.live_bytes(), 0);
    }
}
