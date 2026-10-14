#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StylesheetJournalStamp {
    next: u64,
    observer: MutationCursor,
    entries_len: usize,
    entries_capacity: usize,
    discarded_len: usize,
    discarded_capacity: usize,
}
pub enum StylesheetMetadataPreparationError {
    Stale,
    Dom(DomError),
}
impl DomRuntime {
    pub fn stylesheet_journal_stamp(&self) -> StylesheetJournalStamp {
        StylesheetJournalStamp {
            next: self.mutations.next,
            observer: self.mutations.observer,
            entries_len: self.mutations.entries.len(),
            entries_capacity: self.mutations.entries.capacity(),
            discarded_len: self.mutations.discarded_removed_roots.len(),
            discarded_capacity: self.mutations.discarded_removed_roots.capacity(),
        }
    }
}
impl StylesheetJournalStamp {
    pub fn requested_bytes(&self) -> Result<usize, DomError> {
        let entries = self.entries_capacity.max(
            self.entries_len
                .checked_add(1)
                .ok_or(DomError::MigrationCapacity)?,
        );
        let discarded = self.discarded_capacity.max(
            self.discarded_len
                .checked_add(1)
                .ok_or(DomError::MigrationCapacity)?,
        );
        entries
            .checked_mul(std::mem::size_of::<(u64, Mutation)>())
            .and_then(|bytes| {
                discarded
                    .checked_mul(std::mem::size_of::<(NodeId, MutationRemovalLease)>())
                    .and_then(|other| bytes.checked_add(other))
            })
            .ok_or(DomError::MigrationCapacity)
    }
}

// Private buffers leave incumbent untouched on preparation failure.
// Native allocation admission occurs outside the final mutable DOM borrow.
pub struct PreparedStylesheetJournalBuffers {
    stamp: StylesheetJournalStamp,
    entries: VecDeque<(u64, Mutation)>,
    discarded: Vec<(NodeId, MutationRemovalLease)>,
    charge: StylesheetJournalCharge,
}
impl StylesheetJournalStamp {
    pub fn prepare_buffers(
        self,
        lease: Arc<dyn Send + Sync>,
        admitted_bytes: usize,
    ) -> Result<PreparedStylesheetJournalBuffers, DomError> {
        let mut entries = VecDeque::new();
        entries
            .try_reserve_exact(
                self.entries_len
                    .checked_add(1)
                    .ok_or(DomError::MigrationCapacity)?,
            )
            .map_err(|_| DomError::MigrationCapacity)?;
        let mut discarded = Vec::new();
        discarded
            .try_reserve_exact(
                self.discarded_len
                    .checked_add(1)
                    .ok_or(DomError::MigrationCapacity)?,
            )
            .map_err(|_| DomError::MigrationCapacity)?;
        let actual = entries
            .capacity()
            .checked_mul(std::mem::size_of::<(u64, Mutation)>())
            .and_then(|bytes| {
                discarded
                    .capacity()
                    .checked_mul(std::mem::size_of::<(NodeId, MutationRemovalLease)>())
                    .and_then(|other| bytes.checked_add(other))
            })
            .ok_or(DomError::MigrationCapacity)?;
        // Unexpected allocator capacity rejects private replacements only.
        // Incumbent journal state/buffers/charges have not been touched.
        if actual > admitted_bytes {
            return Err(DomError::MigrationCapacity);
        }
        Ok(PreparedStylesheetJournalBuffers {
            stamp: self,
            entries,
            discarded,
            charge: StylesheetJournalCharge {
                bytes: actual,
                _lease: lease,
            },
        })
    }
}
pub struct PreparedSheetMetadataReplacement<'a> {
    dom: &'a mut DomRuntime,
    node: NodeId,
    disabled: Option<bool>,
    media: Option<String>,
    buffers: PreparedStylesheetJournalBuffers,
}
impl DomRuntime {
    pub fn prepare_stylesheet_metadata_replacement(
        &mut self,
        node: NodeId,
        disabled: Option<bool>,
        media: Option<String>,
        buffers: PreparedStylesheetJournalBuffers,
    ) -> Result<PreparedSheetMetadataReplacement<'_>, StylesheetMetadataPreparationError> {
        use StylesheetMetadataPreparationError as Error;
        if self.stylesheet_journal_stamp() != buffers.stamp {
            return Err(Error::Stale);
        }
        self.arena.get(node).map_err(Error::Dom)?;
        Ok(PreparedSheetMetadataReplacement {
            dom: self,
            node,
            disabled,
            media,
            buffers,
        })
    }
}
impl PreparedSheetMetadataReplacement<'_> {
    pub fn commit(mut self) {
        let record = self
            .dom
            .arena
            .get_mut(self.node)
            .expect("exclusive prepared node");
        if let Some(disabled) = self.disabled {
            record.stylesheet_disabled = Some(disabled);
        }
        if let Some(media) = self.media {
            record.stylesheet_media = Some(media);
        }
        // Exact stamp and exclusive borrow ensure replacement capacity fits;
        // append transfers existing custody without cloning removal leases.
        self.buffers.entries.append(&mut self.dom.mutations.entries);
        self.buffers
            .discarded
            .append(&mut self.dom.mutations.discarded_removed_roots);
        self.dom.mutations.entries = self.buffers.entries;
        self.dom.mutations.discarded_removed_roots = self.buffers.discarded;
        self.dom.mutations.stylesheet_capacity_charge = Some(self.buffers.charge);
        self.dom
            .mutations
            .push(Mutation::StyleSheets { node: self.node });
    }
}

#[cfg(test)]
mod stylesheet_journal_replacement_tests {
    use super::*;
    #[test]
    fn rejected_private_capacity_leaves_incumbent_journal_unchanged() {
        let dom = DomRuntime::default();
        let stamp = dom.stylesheet_journal_stamp();
        let before = dom.mutations.entries.capacity();
        let replacement = stamp.prepare_buffers(Arc::new(()), 0);
        assert!(matches!(replacement, Err(DomError::MigrationCapacity)));
        assert_eq!(dom.stylesheet_journal_stamp(), stamp);
        assert_eq!(dom.mutations.entries.capacity(), before);
        assert!(dom.mutations.stylesheet_capacity_charge.is_none());
    }
    #[test]
    fn stale_preparation_preserves_metadata_and_journal() {
        let mut dom = DomRuntime::default();
        let node = dom.create_element("style");
        let stamp = dom.stylesheet_journal_stamp();
        let buffers = stamp
            .prepare_buffers(Arc::new(()), stamp.requested_bytes().unwrap())
            .unwrap();
        dom.mutations.push(Mutation::StyleSheets { node });
        let before = dom.stylesheet_journal_stamp();
        assert!(matches!(
            dom.prepare_stylesheet_metadata_replacement(
                node,
                Some(true),
                Some("print".into()),
                buffers
            ),
            Err(StylesheetMetadataPreparationError::Stale)
        ));
        assert_eq!(dom.stylesheet_journal_stamp(), before);
        assert_eq!(dom.arena.get(node).unwrap().stylesheet_disabled, None);
        assert_eq!(dom.arena.get(node).unwrap().stylesheet_media, None);
    }
    #[test]
    fn replacement_commit_moves_existing_entries_and_keeps_charge() {
        let mut dom = DomRuntime::default();
        let node = dom.create_element("style");
        dom.mutations.push(Mutation::StyleSheets { node });
        let stamp = dom.stylesheet_journal_stamp();
        let buffers = stamp
            .prepare_buffers(Arc::new(()), stamp.requested_bytes().unwrap())
            .unwrap();
        dom.prepare_stylesheet_metadata_replacement(
            node,
            Some(true),
            Some("print".into()),
            buffers,
        )
        .unwrap_or_else(|_| panic!("valid preparation"))
        .commit();
        assert_eq!(dom.mutations.entries.len(), stamp.entries_len + 1);
        assert_eq!(dom.mutations.next, stamp.next.saturating_add(1));
        assert_eq!(dom.arena.get(node).unwrap().stylesheet_disabled, Some(true));
        assert_eq!(
            dom.arena.get(node).unwrap().stylesheet_media.as_deref(),
            Some("print")
        );
        assert!(dom.mutations.stylesheet_capacity_charge.is_some());
    }
}
