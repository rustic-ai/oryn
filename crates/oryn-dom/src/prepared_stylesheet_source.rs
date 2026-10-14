/// Caller owns this through final DOM borrow, including all old allocation
/// custody. Admission and String construction occur before DOM preparation.
pub struct PreparedSheetSourceReplacement {
    source: Option<OwnedNativeStylesheetSource>,
    base_url: Option<String>,
    policy_allowed: bool,
    buffers: Option<PreparedStylesheetJournalBuffers>,
    old_source: Option<OwnedNativeStylesheetSource>,
    old_base_url: Option<String>,
    old_entries: VecDeque<(u64, Mutation)>,
    old_discarded: Vec<(NodeId, MutationRemovalLease)>,
    old_charge: Option<StylesheetJournalCharge>,
    committed: bool,
}
impl PreparedSheetSourceReplacement {
    pub fn new(
        source: OwnedNativeStylesheetSource,
        base_url: Option<String>,
        policy_allowed: bool,
        buffers: PreparedStylesheetJournalBuffers,
    ) -> Self {
        Self {
            source: Some(source),
            base_url,
            policy_allowed,
            buffers: Some(buffers),
            old_source: None,
            old_base_url: None,
            old_entries: VecDeque::new(),
            old_discarded: Vec::new(),
            old_charge: None,
            committed: false,
        }
    }
}
pub struct PreparedSheetSourceTransaction<'a> {
    dom: &'a mut DomRuntime,
    prepared: &'a mut PreparedSheetSourceReplacement,
    node: NodeId,
}
impl DomRuntime {
    pub fn prepare_stylesheet_source_replacement<'a>(
        &'a mut self,
        node: NodeId,
        expected_revision: u64,
        prepared: &'a mut PreparedSheetSourceReplacement,
    ) -> Result<Option<PreparedSheetSourceTransaction<'a>>, StylesheetMetadataPreparationError>
    {
        use StylesheetMetadataPreparationError as Error;
        if prepared.committed {
            return Ok(None);
        }
        let Some(buffers) = prepared.buffers.as_ref() else {
            return Ok(None);
        };
        if self.stylesheet_journal_stamp() != buffers.stamp {
            return Ok(None);
        }
        self.arena.get(node).map_err(Error::Dom)?;
        if !self
            .arena
            .is_css_stylesheet_owner(node)
            .map_err(Error::Dom)?
            || !self.is_connected(node).map_err(Error::Dom)?
        {
            return Ok(None);
        }
        if self.arena.stylesheet_revision(node).map_err(Error::Dom)? != expected_revision {
            return Ok(None);
        }
        Ok(Some(PreparedSheetSourceTransaction {
            dom: self,
            prepared,
            node,
        }))
    }
}
impl PreparedSheetSourceTransaction<'_> {
    /// No fallible work, allocation, author callback, or old-value Drop.
    /// Caller drops prepared custody only AFTER the mutable DOM borrow ends.
    pub fn commit(self) {
        let p = self.prepared;
        let record = self
            .dom
            .arena
            .get_mut(self.node)
            .expect("prepared source node");
        std::mem::swap(&mut record.stylesheet_override, &mut p.source);
        p.old_source = p.source.take();
        std::mem::swap(&mut record.stylesheet_base_url, &mut p.base_url);
        p.old_base_url = p.base_url.take();
        record.stylesheet_policy_allowed = p.policy_allowed;
        let mut buffers = p.buffers.take().expect("prepared source buffers");
        buffers.entries.append(&mut self.dom.mutations.entries);
        buffers
            .discarded
            .append(&mut self.dom.mutations.discarded_removed_roots);
        std::mem::swap(&mut self.dom.mutations.entries, &mut buffers.entries);
        std::mem::swap(
            &mut self.dom.mutations.discarded_removed_roots,
            &mut buffers.discarded,
        );
        p.old_entries = buffers.entries;
        p.old_discarded = buffers.discarded;
        p.old_charge = self.dom.mutations.stylesheet_capacity_charge.take();
        self.dom.mutations.stylesheet_capacity_charge = Some(buffers.charge);
        self.dom
            .mutations
            .push(Mutation::StyleSheets { node: self.node });
        p.committed = true;
    }
}

#[cfg(test)]
mod source_replacement_tests {
    use super::*;
    #[test]
    fn stale_source_preparation_keeps_both_custodies() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("style");
        dom.append_child(dom.document(), node).unwrap();
        dom.set_stylesheet_text(node, "old".into()).unwrap();
        let stamp = dom.stylesheet_journal_stamp();
        let buffers = stamp
            .prepare_buffers(Arc::new(()), stamp.requested_bytes().unwrap())
            .unwrap();
        let mut p = PreparedSheetSourceReplacement::new("new".into(), None, true, buffers);
        dom.set_stylesheet_disabled(node, false).unwrap();
        let revision = dom.arena.stylesheet_revision(node).unwrap();
        assert!(
            dom.prepare_stylesheet_source_replacement(node, revision, &mut p)
                .unwrap_or_else(|_| panic!("node exists"))
                .is_none()
        );
        assert_eq!(
            dom.arena.get(node).unwrap().stylesheet_override.as_deref(),
            Some("old")
        );
        assert_eq!(p.source.as_deref(), Some("new"));
    }
    #[test]
    fn source_commit_returns_old_strings_to_caller() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("style");
        dom.append_child(dom.document(), node).unwrap();
        dom.set_stylesheet_text(node, "old".into()).unwrap();
        dom.arena.get_mut(node).unwrap().stylesheet_base_url = Some("old-url".into());
        let stamp = dom.stylesheet_journal_stamp();
        let buffers = stamp
            .prepare_buffers(Arc::new(()), stamp.requested_bytes().unwrap())
            .unwrap();
        let revision = dom.arena.stylesheet_revision(node).unwrap();
        let mut p = PreparedSheetSourceReplacement::new(
            "new".into(),
            Some("new-url".into()),
            true,
            buffers,
        );
        dom.prepare_stylesheet_source_replacement(node, revision, &mut p)
            .unwrap_or_else(|_| panic!("valid node"))
            .unwrap()
            .commit();
        assert_eq!(p.old_source.as_deref(), Some("old"));
        assert_eq!(p.old_base_url.as_deref(), Some("old-url"));
        let record = dom.arena.get(node).unwrap();
        assert_eq!(record.stylesheet_override.as_deref(), Some("new"));
        assert!(record.stylesheet_policy_allowed);
    }
}

impl PreparedSheetSourceReplacement {
    /// Caller owns self and calls outside DOM borrow. Swap rather than drop
    /// incoming custody on refusal; caller releases returned buffers later.
    pub fn rebase_buffers(
        &mut self,
        incoming: &mut Option<PreparedStylesheetJournalBuffers>,
    ) -> bool {
        if self.committed || incoming.is_none() {
            return false;
        }
        std::mem::swap(&mut self.buffers, incoming);
        true
    }
}

#[cfg(test)]
mod source_rebase_tests {
    use super::*;
    #[test]
    fn stale_journal_can_rebase_without_consuming_source() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("style");
        dom.append_child(dom.document(), node).unwrap();
        let first = dom.stylesheet_journal_stamp();
        let buffers = first
            .prepare_buffers(Arc::new(()), first.requested_bytes().unwrap())
            .unwrap();
        let mut prepared = PreparedSheetSourceReplacement::new("p{}".into(), None, true, buffers);
        dom.set_stylesheet_disabled(node, false).unwrap();
        let revision = dom.arena.stylesheet_revision(node).unwrap();
        assert!(
            dom.prepare_stylesheet_source_replacement(node, revision, &mut prepared)
                .unwrap_or_else(|_| panic!("valid node"))
                .is_none()
        );
        let next = dom.stylesheet_journal_stamp();
        let mut replacement = Some(
            next.prepare_buffers(Arc::new(()), next.requested_bytes().unwrap())
                .unwrap(),
        );
        assert!(prepared.rebase_buffers(&mut replacement));
        drop(replacement);
        dom.prepare_stylesheet_source_replacement(node, revision, &mut prepared)
            .unwrap_or_else(|_| panic!("valid node"))
            .unwrap()
            .commit();
        assert_eq!(
            dom.arena.get(node).unwrap().stylesheet_override.as_deref(),
            Some("p{}")
        );
    }
}
