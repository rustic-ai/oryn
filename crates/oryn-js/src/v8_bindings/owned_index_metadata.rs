//! Staged fallible native index metadata. No unadmitted refcount allocation.
//! Wire DTO remains unchanged; this type owns only native retained metadata.
use crate::idb_owned_admission::{Buffer, Ledger, Text};
use std::sync::Arc;

pub(crate) fn sort_cancelled<T>(
    values: &mut [T],
    compare: impl Fn(&T, &T, &mut dyn FnMut() -> bool) -> Result<std::cmp::Ordering, &'static str>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(), &'static str> {
    fn sift<T>(
        values: &mut [T],
        mut root: usize,
        end: usize,
        compare: &impl Fn(&T, &T, &mut dyn FnMut() -> bool) -> Result<std::cmp::Ordering, &'static str>,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(), &'static str> {
        loop {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            let Some(left) = root.checked_mul(2).and_then(|v| v.checked_add(1)) else {
                return Ok(());
            };
            if left >= end {
                return Ok(());
            }
            let right = left + 1;
            let child = if right < end && compare(&values[left], &values[right], cancelled)?.is_lt()
            {
                right
            } else {
                left
            };
            if !compare(&values[root], &values[child], cancelled)?.is_lt() {
                return Ok(());
            }
            values.swap(root, child);
            root = child;
        }
    }
    if cancelled() {
        return Err("IDB metadata cancelled");
    }
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), &compare, cancelled)?;
    }
    for end in (1..values.len()).rev() {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        values.swap(0, end);
        sift(values, 0, end, &compare, cancelled)?;
    }
    if cancelled() {
        return Err("IDB metadata cancelled");
    }
    Ok(())
}

pub(crate) fn compare_cancelled(
    a: &str,
    b: &str,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<std::cmp::Ordering, &'static str> {
    let mut a = a.bytes();
    let mut b = b.bytes();
    let mut offset = 0usize;
    loop {
        if offset.is_multiple_of(1024) && cancelled() {
            return Err("IDB metadata cancelled");
        }
        let order = a.next().cmp(&b.next());
        if !order.is_eq() {
            return Ok(order);
        }
        if a.len() == 0 && b.len() == 0 {
            return Ok(std::cmp::Ordering::Equal);
        }
        offset += 1;
    }
}

pub(crate) struct IndexMetadata {
    name: Text,
    key_path: Text,
    pub(crate) unique: bool,
    pub(crate) multi_entry: bool,
}

pub(crate) struct StoreMetadata {
    pub(crate) name: Text,
    pub(crate) key_path: Option<Text>,
    pub(crate) auto_increment: bool,
    indexes: Buffer<IndexMetadata>,
}
impl StoreMetadata {
    pub(crate) fn from_wire(
        value: &crate::platform::IdbObjectStoreDefinition,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut indexes = Buffer::allocate(ledger, value.indexes.len(), limit, used)?;
        for index in &value.indexes {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            indexes.push(IndexMetadata::new(
                Text::copy_string_cancelled(ledger, &index.name, limit, used, &mut *cancelled)?,
                Text::copy_string_cancelled(ledger, &index.key_path, limit, used, &mut *cancelled)?,
                index.unique,
                index.multi_entry,
            ))?;
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self {
            name: Text::copy_string_cancelled(ledger, &value.name, limit, used, &mut *cancelled)?,
            key_path: value
                .key_path
                .as_deref()
                .map(|path| Text::copy_string_cancelled(ledger, path, limit, used, &mut *cancelled))
                .transpose()?,
            auto_increment: value.auto_increment,
            indexes,
        })
    }
    pub(crate) fn indexes(&self) -> &[IndexMetadata] {
        self.indexes.as_slice()
    }
    pub(crate) fn empty(
        name: Text,
        key_path: Option<Text>,
        auto_increment: bool,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let indexes = Buffer::allocate(ledger, 0, limit, used)?;
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self {
            name,
            key_path,
            auto_increment,
            indexes,
        })
    }
    pub(crate) fn with_index(
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
        let len = self
            .indexes()
            .len()
            .checked_add(1)
            .ok_or("index metadata overflow")?;
        let mut indexes = Buffer::allocate(ledger, len, limit, used)?;
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            indexes.push(index.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        indexes.push(new)?;
        self.with_prepared_indexes(indexes, ledger, limit, used, &mut *cancelled)
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
        let mut indexes = Buffer::allocate(ledger, self.indexes().len(), limit, used)?;
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            indexes.push(index.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        self.with_prepared_indexes(indexes, ledger, limit, used, &mut *cancelled)
    }
    pub(crate) fn without_index(
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
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if !compare_cancelled(index.name(), name, cancelled)?.is_eq() {
                count += 1;
            }
        }
        let mut indexes = Buffer::allocate(ledger, count, limit, used)?;
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if compare_cancelled(index.name(), name, cancelled)?.is_eq() {
                continue;
            }
            indexes.push(index.copy_admitted(ledger, limit, used, &mut *cancelled)?)?;
        }
        self.with_prepared_indexes(indexes, ledger, limit, used, &mut *cancelled)
    }
    pub(crate) fn rename_index(
        &self,
        old: &str,
        new: Text,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            if compare_cancelled(index.name(), new.as_str(), cancelled)?.is_eq()
                && !compare_cancelled(index.name(), old, cancelled)?.is_eq()
            {
                return Err("duplicate index");
            }
        }
        let mut indexes = Buffer::allocate(ledger, self.indexes().len(), limit, used)?;
        let mut new = Some(new);
        for index in self.indexes() {
            if cancelled() {
                return Err("IDB metadata cancelled");
            }
            let mut copy = index.copy_admitted(ledger, limit, used, &mut *cancelled)?;
            if compare_cancelled(index.name(), old, cancelled)?.is_eq() {
                copy.rename(new.take().ok_or("duplicate old index")?);
            }
            indexes.push(copy)?;
        }
        if new.is_some() {
            return Err("missing index");
        }
        self.with_prepared_indexes(indexes, ledger, limit, used, &mut *cancelled)
    }
    fn with_prepared_indexes(
        &self,
        indexes: Buffer<IndexMetadata>,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self {
            name: Text::copy_string_cancelled(
                ledger,
                self.name.as_str(),
                limit,
                used,
                &mut *cancelled,
            )?,
            key_path: self
                .key_path
                .as_ref()
                .map(|path| {
                    Text::copy_string_cancelled(ledger, path.as_str(), limit, used, &mut *cancelled)
                })
                .transpose()?,
            auto_increment: self.auto_increment,
            indexes,
        })
    }
}
impl IndexMetadata {
    pub(crate) fn new(name: Text, key_path: Text, unique: bool, multi_entry: bool) -> Self {
        Self {
            name,
            key_path,
            unique,
            multi_entry,
        }
    }
    pub(crate) fn name(&self) -> &str {
        self.name.as_str()
    }
    pub(crate) fn key_path(&self) -> &str {
        self.key_path.as_str()
    }
    /// Every native graph copy must call this, never `.clone()` a String.
    /// Preparation completes before mutating any native metadata container.
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
        let name = Text::copy_string_cancelled(ledger, self.name(), limit, used, &mut *cancelled)?;
        let key_path =
            Text::copy_string_cancelled(ledger, self.key_path(), limit, used, &mut *cancelled)?;
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self::new(name, key_path, self.unique, self.multi_entry))
    }
    pub(crate) fn rename(&mut self, name: Text) {
        self.name = name;
    }
    pub(crate) fn copied_keypath(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Text, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Text::copy_string_cancelled(ledger, self.key_path(), limit, used, &mut *cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_empty_long_copy_and_sort_releases() {
        let ledger = Arc::new(Ledger::default());
        let wire = crate::platform::IdbObjectStoreDefinition {
            name: "x".repeat(8192),
            key_path: None,
            auto_increment: false,
            indexes: vec![],
        };
        assert!(StoreMetadata::from_wire(&wire, &ledger, 1_000_000, 0, &mut || true).is_err());
        assert_eq!(ledger.live_bytes(), 0);
        let mut calls = 0;
        assert!(
            StoreMetadata::from_wire(&wire, &ledger, 1_000_000, 0, &mut || {
                calls += 1;
                calls >= 4
            })
            .is_err()
        );
        assert_eq!(ledger.live_bytes(), 0);
        let mut values = ["z", "a", "b", "a"];
        sort_cancelled(
            &mut values,
            |a, b, c| compare_cancelled(a, b, c),
            &mut || false,
        )
        .unwrap();
        assert_eq!(values, ["a", "a", "b", "z"]);
        assert!(
            sort_cancelled(
                &mut values,
                |a, b, c| compare_cancelled(a, b, c),
                &mut || true
            )
            .is_err()
        );
    }
    #[test]
    fn native_copy_admits_every_payload_and_failure_is_atomic() {
        let ledger = Arc::new(Ledger::default());
        let value = IndexMetadata::new(
            Text::copy_string(&ledger, "name", 1024, 0).unwrap(),
            Text::copy_string(&ledger, "account.name", 1024, 0).unwrap(),
            false,
            false,
        );
        let original = ledger.live_bytes();
        assert!(
            value
                .copy_admitted(&ledger, original + 4, 0, &mut || false)
                .is_err()
        );
        assert_eq!(ledger.live_bytes(), original);
        assert_eq!(value.name(), "name");
        let copy = value
            .copy_admitted(&ledger, 1024, 0, &mut || false)
            .unwrap();
        assert_eq!(ledger.live_bytes(), original * 2);
        assert_eq!(copy.key_path(), value.key_path());
        drop(copy);
        assert_eq!(ledger.live_bytes(), original);
        drop(value);
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn store_name_keypath_and_rollback_index_copies_are_all_charged() {
        let ledger = Arc::new(Ledger::default());
        let wire = crate::platform::IdbObjectStoreDefinition {
            name: "store".into(),
            key_path: Some("nested.id".into()),
            auto_increment: true,
            indexes: vec![crate::platform::IdbIndexDefinition {
                name: "index".into(),
                key_path: "account.name".into(),
                unique: false,
                multi_entry: false,
            }],
        };
        let store = StoreMetadata::from_wire(&wire, &ledger, 8192, 0, &mut || false).unwrap();
        let original = ledger.live_bytes();
        let copy = store
            .copy_admitted(&ledger, 8192, 0, &mut || false)
            .unwrap();
        assert_eq!(ledger.live_bytes(), original * 2);
        assert_eq!(copy.name.as_str(), "store");
        assert_eq!(copy.key_path.as_ref().unwrap().as_str(), "nested.id");
        assert!(
            store
                .copy_admitted(&ledger, original * 2, 0, &mut || false)
                .is_err()
        );
        assert_eq!(ledger.live_bytes(), original * 2);
        drop(copy);
        drop(store);
        assert_eq!(ledger.live_bytes(), 0);
    }
}
