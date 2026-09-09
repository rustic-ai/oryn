//! Native control-value ranges with UTF-16 offsets and owner indexing.
//!
//! The caller owns value storage and node rooting. It must apply edits here
//! before dispatching input events and disconnect removed/adopted controls.
use crate::NodeId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OpaqueRangeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpaqueRange {
    pub owner: Option<NodeId>,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OpaqueRangeError {
    #[error("invalid control value range offset")]
    InvalidOffset,
    #[error("control value range identity exhausted")]
    IdentityExhausted,
    #[error("unknown control value range")]
    UnknownRange,
}

#[derive(Debug, Default, Clone)]
pub struct OpaqueRangeStore {
    next_id: u64,
    ranges: BTreeMap<OpaqueRangeId, OpaqueRange>,
    owners: BTreeMap<NodeId, BTreeSet<OpaqueRangeId>>,
}

impl OpaqueRangeStore {
    pub fn create(
        &mut self,
        owner: NodeId,
        start: u32,
        end: u32,
        value_length: u32,
    ) -> Result<OpaqueRangeId, OpaqueRangeError> {
        if start > value_length || end > value_length {
            return Err(OpaqueRangeError::InvalidOffset);
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(OpaqueRangeError::IdentityExhausted)?;
        let id = OpaqueRangeId(next);
        self.next_id = next;
        self.ranges.insert(
            id,
            OpaqueRange {
                owner: Some(owner),
                start,
                end: end.max(start),
            },
        );
        self.owners.entry(owner).or_default().insert(id);
        Ok(id)
    }

    pub fn has_live_ranges(&self) -> bool {
        !self.owners.is_empty()
    }

    pub fn owner_ids(&self, owner: NodeId) -> Vec<OpaqueRangeId> {
        self.owners
            .get(&owner)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn has_owner(&self, owner: NodeId) -> bool {
        self.owners.contains_key(&owner)
    }

    pub fn get(&self, id: OpaqueRangeId) -> Result<OpaqueRange, OpaqueRangeError> {
        self.ranges
            .get(&id)
            .copied()
            .ok_or(OpaqueRangeError::UnknownRange)
    }

    pub fn disconnect(&mut self, id: OpaqueRangeId) -> Result<(), OpaqueRangeError> {
        let range = self
            .ranges
            .get_mut(&id)
            .ok_or(OpaqueRangeError::UnknownRange)?;
        if let Some(owner) = range.owner.take()
            && let Some(ids) = self.owners.get_mut(&owner)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.owners.remove(&owner);
            }
        }
        range.start = 0;
        range.end = 0;
        Ok(())
    }

    pub fn disconnect_owner(&mut self, owner: NodeId) {
        if let Some(ids) = self.owners.remove(&owner) {
            for id in ids {
                if let Some(range) = self.ranges.get_mut(&id) {
                    *range = OpaqueRange {
                        owner: None,
                        start: 0,
                        end: 0,
                    };
                }
            }
        }
    }

    pub fn drop_range(&mut self, id: OpaqueRangeId) -> Result<(), OpaqueRangeError> {
        self.disconnect(id)?;
        self.ranges.remove(&id);
        Ok(())
    }

    /// Apply an actual value splice in UTF-16 code units. A `.value` assignment
    /// with unchanged text must skip this operation, whereas setRangeText still
    /// performs its requested replacement even if the resulting text is equal.
    pub fn replace(
        &mut self,
        owner: NodeId,
        offset: u32,
        removed: u32,
        inserted: u32,
        old_length: u32,
    ) -> Result<(), OpaqueRangeError> {
        let end = offset
            .checked_add(removed)
            .ok_or(OpaqueRangeError::InvalidOffset)?;
        if end > old_length || (old_length - removed).checked_add(inserted).is_none() {
            return Err(OpaqueRangeError::InvalidOffset);
        }
        let update = |point: u32| {
            if point > end {
                point - removed + inserted
            } else if point > offset {
                offset
            } else {
                point
            }
        };
        if let Some(ids) = self.owners.get(&owner) {
            for id in ids {
                if let Some(range) = self.ranges.get_mut(id) {
                    range.start = update(range.start);
                    range.end = update(range.end);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_value_splices_preserve_boundary_affinity() {
        let mut dom = crate::DomRuntime::new();
        let owner = dom.create_element("textarea");
        // old length, start, end, edit offset, removed, inserted, new endpoints.
        for (length, start, end, offset, removed, inserted, expected) in [
            (10, 2, 8, 3, 4, 2, (2, 6)),
            (5, 2, 4, 1, 0, 1, (3, 5)),
            (5, 2, 4, 2, 0, 2, (2, 6)),
            (5, 2, 4, 4, 0, 2, (2, 4)),
            (5, 1, 4, 0, 5, 5, (0, 0)),
            (5, 1, 4, 2, 3, 1, (1, 2)),
        ] {
            let mut store = OpaqueRangeStore::default();
            let id = store.create(owner, start, end, length).unwrap();
            store
                .replace(owner, offset, removed, inserted, length)
                .unwrap();
            let actual = store.get(id).unwrap();
            assert_eq!((actual.start, actual.end), expected);
        }
    }

    #[test]
    fn disconnect_is_per_range_and_owner_removal_is_per_control() {
        let mut dom = crate::DomRuntime::new();
        let first = dom.create_element("textarea");
        let second = dom.create_element("input");
        let mut store = OpaqueRangeStore::default();
        let a = store.create(first, 1, 4, 5).unwrap();
        let b = store.create(first, 2, 5, 5).unwrap();
        let c = store.create(second, 0, 3, 3).unwrap();
        store.disconnect(a).unwrap();
        store.disconnect(a).unwrap();
        store.replace(first, 0, 5, 7, 5).unwrap();
        assert_eq!(
            store.get(a).unwrap(),
            OpaqueRange {
                owner: None,
                start: 0,
                end: 0
            }
        );
        assert_eq!(
            store.get(b).unwrap(),
            OpaqueRange {
                owner: Some(first),
                start: 0,
                end: 0
            }
        );
        store.disconnect_owner(first);
        assert_eq!(store.get(b).unwrap().owner, None);
        assert_eq!(
            store.get(c).unwrap(),
            OpaqueRange {
                owner: Some(second),
                start: 0,
                end: 3
            }
        );
        store.drop_range(a).unwrap();
        assert_eq!(store.get(a), Err(OpaqueRangeError::UnknownRange));
        let fresh = store.create(first, 0, 7, 7).unwrap();
        assert_ne!(fresh, a);
    }

    #[test]
    fn reversed_offsets_collapse_and_invalid_edits_are_atomic() {
        let mut dom = crate::DomRuntime::new();
        let owner = dom.create_element("input");
        let mut store = OpaqueRangeStore::default();
        assert_eq!(
            store.create(owner, 0, 6, 5),
            Err(OpaqueRangeError::InvalidOffset)
        );
        let id = store.create(owner, 4, 1, 5).unwrap();
        assert_eq!(
            store.get(id).unwrap(),
            OpaqueRange {
                owner: Some(owner),
                start: 4,
                end: 4
            }
        );
        let before = store.get(id).unwrap();
        assert_eq!(
            store.replace(owner, 4, 2, 0, 5),
            Err(OpaqueRangeError::InvalidOffset)
        );
        assert_eq!(store.get(id).unwrap(), before);
        assert_eq!(
            store.replace(owner, 0, 0, u32::MAX, 5),
            Err(OpaqueRangeError::InvalidOffset)
        );
        assert_eq!(store.get(id).unwrap(), before);
    }
}
