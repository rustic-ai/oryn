//! Iterative native cloning, with graph validation before destination allocation.
use super::*;

impl DomRuntime {
    fn clone_edges(&self, node: NodeId, deep: bool) -> Result<Vec<(NodeId, bool, bool)>, DomError> {
        let record = self.arena.get(node)?;
        let mut edges = Vec::new();
        if let Some(content) = record.template_content {
            edges.push((content, true, deep));
        }
        for child in &record.children {
            match self.arena.get(*child)?.kind {
                NodeKind::Document
                | NodeKind::ShadowRoot {
                    clonable: false, ..
                } => {}
                // Clonable shadow trees are copied deeply even for shallow hosts.
                NodeKind::ShadowRoot { clonable: true, .. } => edges.push((*child, false, true)),
                _ if deep => edges.push((*child, false, true)),
                _ => {}
            }
        }
        Ok(edges)
    }

    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> Result<NodeId, DomError> {
        let mut cursor = CloneCursor::new(None, self, node, deep)?;
        let result = cursor.resume(self).and_then(|_| cursor.handoff(self));
        if result.is_err() {
            let _ = cursor.abort(self);
        }
        result
    }
}

struct CloneEntry {
    snapshot: Node,
    attributes: Vec<Arc<ContentAttribute>>,
    parent: Option<(usize, bool)>,
    target: Option<NodeId>,
    rooted: bool,
    attribute_index: usize,
    pending_attribute: Option<NodeId>,
    assignment: Option<insertion::DocumentAssignment>,
    ownership: Option<insertion::OwnershipPublication>,
    insertion: Option<insertion::TreeInsertion>,
    attachment_stage: u8,
}
/// A clone freezes its source graph before allocation. Every allocation,
/// materialized attribute and child attachment has independent retained custody.
pub(crate) struct CloneCursor {
    roots: insertion::CursorRoots,
    entries: Vec<CloneEntry>,
    allocation: usize,
    attachment: usize,
    handoff_root: bool,
    complete: bool,
    aborted: bool,
}
impl CloneCursor {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        node: NodeId,
        deep: bool,
    ) -> Result<Self, DomError> {
        let mut pending = vec![(node, deep, None)];
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        while let Some((original, deep, parent)) = pending.pop() {
            if !seen.insert(original) {
                return Err(DomError::Cycle);
            }
            let mut snapshot = dom.arena.get(original)?.clone();
            if dom.control_bad_input(original)? {
                snapshot.value = Some(dom.control_value(original)?);
            }
            let attributes = dom
                .arena
                .attribute_node_ids(original)
                .into_iter()
                .map(|node| match &dom.arena.get(node)?.kind {
                    NodeKind::Attribute { data, .. } => Ok(data.clone()),
                    _ => Err(DomError::AttributeNotFound),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let index = entries.len();
            let edges = dom.clone_edges(original, deep)?;
            entries.push(CloneEntry {
                snapshot,
                attributes,
                parent,
                target: None,
                rooted: false,
                attribute_index: 0,
                pending_attribute: None,
                assignment: None,
                ownership: None,
                insertion: None,
                attachment_stage: 0,
            });
            pending.extend(
                edges
                    .into_iter()
                    .rev()
                    .map(|(child, template, deep)| (child, deep, Some((index, template)))),
            );
        }
        let attachment = entries.len();
        Ok(Self {
            roots: insertion::CursorRoots::new(owner, dom),
            entries,
            allocation: 0,
            attachment,
            handoff_root: false,
            complete: false,
            aborted: false,
        })
    }
    pub(crate) fn result(&self) -> Option<NodeId> {
        self.complete.then(|| self.entries[0].target).flatten()
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if self.aborted {
            return Err(DomError::MigrationChanged);
        }
        self.roots.validate(dom)?;
        while self.allocation < self.entries.len() {
            let entry = &mut self.entries[self.allocation];
            if entry.target.is_none() {
                let mut kind = entry.snapshot.kind.clone();
                if let NodeKind::Attribute { owner, .. } = &mut kind {
                    *owner = None;
                }
                let target = dom.create(kind.clone());
                entry.target = Some(target);
                // This is initialization of an unexposed allocation, not an
                // authored setAttribute operation. Copying immutable attributes
                // must not rerun input setters or publish attribute reactions.
                let source = &entry.snapshot;
                let mut record = new_node(kind);
                record.owner_document = source.owner_document;
                record.document_metadata = source.document_metadata.clone();
                record.script_already_started = source.script_already_started;
                record.cryptographic_nonce = source.cryptographic_nonce.clone();
                record.parser_duplicate_attributes = source.parser_duplicate_attributes;
                record.attributes = source.attributes.clone();
                if matches!(record.kind, NodeKind::Document)
                    && let Some(metadata) = record.document_metadata.as_mut()
                {
                    metadata.hide_nonces = false;
                    metadata.registered_custom_properties.clear();
                }
                record.custom_element_registry = source.custom_element_registry;
                record.custom_element_registry_keep_null = source.custom_element_registry_keep_null;
                record.custom_element_is = source.custom_element_is.clone();
                record.declarative_shadow_root = source.declarative_shadow_root;
                record.value = source.value.clone();
                record.dirty_value = source.dirty_value;
                record.user_edited_value = source.user_edited_value;
                record.selected_files = source.selected_files.clone();
                record.checkedness = source.checkedness;
                record.dirty_checkedness = source.dirty_checkedness;
                record.selectedness = source.selectedness;
                record.dirty_selectedness = source.dirty_selectedness;
                record.indeterminate = source.indeterminate;
                record.sensitive_value = source.sensitive_value;
                record.manual_slot_assignment = source.manual_slot_assignment;
                record.delegates_focus = source.delegates_focus;
                record.shadow_serializable = source.shadow_serializable;
                record.shadow_reference_target = source.shadow_reference_target.clone();
                record.shadow_adopted_stylesheet_specifiers =
                    source.shadow_adopted_stylesheet_specifiers.clone();
                *dom.arena.get_mut(target)? = record;
            }
            let target = entry.target.unwrap();
            if !entry.rooted {
                self.roots.retain(dom, target)?;
                entry.rooted = true;
            }
            #[cfg(test)]
            clone_checkpoint()?;
            while entry.attribute_index < entry.attributes.len() {
                let data = entry.attributes[entry.attribute_index].clone();
                let attribute = match entry.pending_attribute {
                    Some(node) => node,
                    None => {
                        let node = dom.create(NodeKind::Attribute {
                            data: data.clone(),
                            owner: Some(target),
                        });
                        entry.pending_attribute = Some(node);
                        node
                    }
                };
                // The parent owns this Attr as soon as the lookup edge is
                // committed; retry does not allocate a second attribute object.
                dom.arena.get_mut(attribute)?.owner_document = entry.snapshot.owner_document;
                dom.arena
                    .attribute_nodes
                    .insert((target, data.key()), attribute);
                entry.attribute_index += 1;
                entry.pending_attribute = None;
                #[cfg(test)]
                clone_checkpoint()?;
            }
            self.allocation += 1;
        }
        while self.attachment > 1 {
            let index = self.attachment - 1;
            let (parent_index, template) = self.entries[index].parent.expect("non-root clone edge");
            let parent = self.entries[parent_index].target.unwrap();
            let entry = &mut self.entries[index];
            let target = entry.target.unwrap();
            if entry.attachment_stage == 0 {
                if let Some(document) = dom.arena.node_document(parent)? {
                    let document = if template {
                        dom.arena.template_contents_owner_document(document)?
                    } else {
                        document
                    };
                    let previous = dom
                        .ownership_before_adoption(target, Some(document))?
                        .into_iter()
                        .map(|(node, _)| (node, None))
                        .collect();
                    entry.assignment = Some(insertion::DocumentAssignment::new(
                        dom.arena.id(),
                        target,
                        document,
                    ));
                    entry.ownership = Some(insertion::OwnershipPublication::new(
                        dom.arena.id(),
                        previous,
                    ));
                }
                entry.attachment_stage = 1;
            }
            if entry.attachment_stage == 1 {
                if let Some(assignment) = entry.assignment.as_mut() {
                    assignment.resume(&mut dom.arena)?;
                }
                entry.attachment_stage = 2;
            }
            if entry.attachment_stage == 2 {
                if let Some(ownership) = entry.ownership.as_mut() {
                    ownership.resume(dom)?;
                }
                entry.attachment_stage = 3;
            }
            if entry.attachment_stage == 3 {
                if template {
                    dom.arena.get_mut(parent)?.template_content = Some(target);
                    dom.arena.get_mut(target)?.template_host = Some(parent);
                    dom.internal_targets.insert(target);
                    entry.attachment_stage = 4;
                } else {
                    if entry.insertion.is_none() {
                        // Reverse postorder completes descendants first; prepend each
                        // sibling to preserve the captured source order.
                        let before = dom.arena.get(parent)?.children.first().copied();
                        entry.insertion = Some(insertion::TreeInsertion::new(
                            self.roots.owner(),
                            dom,
                            parent,
                            target,
                            before,
                            false,
                        )?);
                    }
                    entry.insertion.as_mut().unwrap().resume(dom)?;
                    entry.attachment_stage = 4;
                }
            }
            self.attachment -= 1;
            #[cfg(test)]
            clone_checkpoint()?;
        }
        self.complete = true;
        Ok(())
    }
    fn release_allocations(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        for entry in &mut self.entries {
            if let Some(insertion) = entry.insertion.as_mut() {
                insertion.abort(dom);
            }
        }
        self.roots.release(dom)
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.aborted = true;
        let result = self.release_allocations(dom);
        if self.handoff_root {
            let root = self.entries[0].target.unwrap();
            let released = dom.release_native_with_commit(root, || self.handoff_root = false);
            result.and(released)
        } else {
            result
        }
    }
    fn handoff(&mut self, dom: &mut DomRuntime) -> Result<NodeId, DomError> {
        let root = self.result().ok_or(DomError::MigrationChanged)?;
        if !self.handoff_root {
            dom.retain_native(root)?;
            self.handoff_root = true;
        }
        self.release_allocations(dom)?;
        // clone_node's existing bare-NodeId return convention transfers this
        // fresh allocation to its immediate native caller before any GC point.
        // Do not collect that allocation while relinquishing cursor ownership.
        dom.roots
            .get_mut(&root)
            .ok_or(DomError::NativeRootUnderflow(root))?
            .native -= 1;
        self.handoff_root = false;
        Ok(root)
    }
}
#[cfg(test)]
thread_local! {static FAIL_CLONE_AFTER:std::cell::Cell<Option<usize>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
fn clone_checkpoint() -> Result<(), DomError> {
    FAIL_CLONE_AFTER.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            Err(DomError::MigrationCapacity)
        }
        Some(left) => {
            slot.set(Some(left - 1));
            Ok(())
        }
        None => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_controls_preserve_dirty_values_without_copying_bad_input() {
        let mut source = html::parse("<textarea id=t>default</textarea><input id=n type=number>")
            .into_runtime()
            .unwrap();
        let textarea = source.find_by_id("t").unwrap();
        source.set_value(textarea, "edited", false).unwrap();
        let number = source.find_by_id("n").unwrap();
        {
            let record = source.arena.get_mut(number).unwrap();
            record.value = Some("-".into());
            record.dirty_value = true;
            record.user_edited_value = true;
        }
        assert!(source.control_bad_input(number).unwrap());
        let mut target = DomRuntime::new();
        let imported = target
            .import_node_from(source.arena(), textarea, true)
            .unwrap();
        assert_eq!(target.control_value(imported).unwrap(), "edited");
        target.set_text_content(imported, "new default").unwrap();
        assert_eq!(target.control_value(imported).unwrap(), "edited");
        let imported_number = target
            .import_node_from(source.arena(), number, false)
            .unwrap();
        assert_eq!(target.control_value(imported_number).unwrap(), "");
        assert!(!target.control_bad_input(imported_number).unwrap());
        assert!(source.control_bad_input(number).unwrap());
    }
    #[test]
    fn clone_rejects_malformed_graph_before_allocating_records() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let child = dom.create_element("span");
        dom.append_child(root, child).unwrap();
        dom.arena.get_mut(child).unwrap().children.push(root);
        let before = dom.arena.slots.len();
        assert!(matches!(dom.clone_node(root, true), Err(DomError::Cycle)));
        assert_eq!(dom.arena.slots.len(), before);
    }
    #[test]
    fn clone_preserves_template_and_shadow_depth_without_recursive_copy() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("template");
        let content = dom.template_content(root).unwrap();
        let host = dom.create_element("div");
        dom.append_child(content, host).unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: true,
        });
        dom.append_child(host, shadow).unwrap();
        let text = dom.create_text("retained");
        dom.append_child(shadow, text).unwrap();
        let cloned = dom.clone_node(root, true).unwrap();
        let copied_content = dom.template_content(cloned).unwrap();
        let copied_host = dom.children(copied_content).unwrap()[0];
        let copied_shadow = dom.children(copied_host).unwrap()[0];
        assert_eq!(dom.arena.text_content(copied_shadow).unwrap(), "retained");
        assert_ne!(copied_content, content);
        assert_ne!(copied_shadow, shadow);
        let shallow = dom.clone_node(root, false).unwrap();
        let empty = dom.template_content(shallow).unwrap();
        assert!(dom.children(empty).unwrap().is_empty());
    }
    #[test]
    fn resumable_clone_keeps_allocated_identity_and_frozen_source_after_each_fault() {
        for failure in 0..10 {
            let mut dom = html::parse(
                "<div id=root xmlns:q='urn:q' q:value='old'><span>original</span><b>tail</b></div>",
            )
            .into_runtime()
            .unwrap();
            let root = dom.find_by_id("root").unwrap();
            dom.set_attribute_ns(root, Some("urn:q".into()), "q:value", "old")
                .unwrap();
            let mut cursor = CloneCursor::new(None, &mut dom, root, true).unwrap();
            FAIL_CLONE_AFTER.with(|slot| slot.set(Some(failure)));
            let first = cursor.resume(&mut dom);
            FAIL_CLONE_AFTER.with(|slot| slot.set(None));
            let allocated = cursor
                .entries
                .iter()
                .map(|entry| entry.target)
                .collect::<Vec<_>>();
            if first.is_err() {
                dom.set_attribute(root, "data-after", "author").unwrap();
                cursor.resume(&mut dom).unwrap();
            }
            for (index, previous) in allocated.iter().enumerate() {
                if let Some(previous) = previous {
                    assert_eq!(cursor.entries[index].target, Some(*previous));
                }
            }
            let clone = cursor.handoff(&mut dom).unwrap();
            assert_eq!(dom.arena.text_content(clone).unwrap(), "originaltail");
            assert!(
                dom.attribute_in_null_namespace(clone, "data-after")
                    .unwrap()
                    .is_none()
            );
        }
    }
    #[test]
    fn clone_abort_releases_allocations_and_refuses_later_resume() {
        let mut dom = html::parse("<div id=root><span>source</span></div>")
            .into_runtime()
            .unwrap();
        let root = dom.find_by_id("root").unwrap();
        let mut cursor = CloneCursor::new(None, &mut dom, root, true).unwrap();
        FAIL_CLONE_AFTER.with(|slot| slot.set(Some(0)));
        assert!(cursor.resume(&mut dom).is_err());
        let clone = cursor.entries[0].target.unwrap();
        cursor.abort(&mut dom).unwrap();
        assert!(dom.arena.get(clone).is_err());
        assert!(matches!(
            cursor.resume(&mut dom),
            Err(DomError::MigrationChanged)
        ));
        assert!(dom.arena.get(root).is_ok());
    }

    #[test]
    fn migrated_partial_clone_is_not_replayed_and_lease_release_follows_owner() {
        let owner = Rc::new(RefCell::new(
            html::parse("<div id=root><span>source</span></div>")
                .into_runtime()
                .unwrap(),
        ));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow().find_by_id("root").unwrap();
        let mut cursor =
            CloneCursor::new(Some(&owner), &mut owner.borrow_mut(), root, true).unwrap();
        FAIL_CLONE_AFTER.with(|slot| slot.set(Some(0)));
        assert!(cursor.resume(&mut owner.borrow_mut()).is_err());
        let clone = cursor.entries[0].target.unwrap();
        let document = destination.borrow().document();
        let mut migration =
            DomRuntime::prepare_subtree_migration(&owner, &destination, clone, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&owner, &destination, &mut migration).unwrap();
        assert!(matches!(
            cursor.resume(&mut owner.borrow_mut()),
            Err(DomError::MigrationChanged)
        ));
        drop(cursor);
        drop(migration);
        drain_node_migration_pins().unwrap();
        drain_tracked_node_leases().unwrap();
        assert!(destination.borrow().arena().get(moved.root).is_err());
        assert!(owner.borrow().arena().get(root).is_ok());
    }
}
