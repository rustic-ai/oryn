//! Document-local snapshots for native layout and semantic projection.
use super::*;

impl DomArena {
    /// Clone every slot and projection index cooperatively, with the same fresh
    /// arena identity and parser-document rebinding as `Clone`.
    pub fn clone_with_checkpoint(
        &self,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Self, DomError> {
        checkpoint()?;
        let id = ArenaId::fresh();
        let mut slots = Vec::new();
        for source in &self.slots {
            checkpoint()?;
            let mut slot = source.clone();
            if let Some(record) = slot.node.as_mut()
                && let Some(parser) = record.script_parser_document.as_mut()
                && parser.arena == self.id
            {
                parser.arena = id;
            }
            slots.push(slot);
        }
        macro_rules! copy_map {
            ($source:expr) => {{
                let mut output = BTreeMap::new();
                for (key, value) in $source {
                    checkpoint()?;
                    output.insert(key.clone(), value.clone());
                }
                output
            }};
        }
        macro_rules! copy_set {
            ($source:expr) => {{
                let mut output = BTreeSet::new();
                for value in $source {
                    checkpoint()?;
                    output.insert(*value);
                }
                output
            }};
        }
        let mut free = Vec::new();
        for &value in &self.free {
            checkpoint()?;
            free.push(value);
        }
        let snapshot = Self {
            registry_index: Default::default(),
            id,
            allocation_document: self.allocation_document,
            slots,
            free,
            attribute_nodes: copy_map!(&self.attribute_nodes),
            stylesheet_revisions: copy_map!(&self.stylesheet_revisions),
            name_attribute_revisions: copy_map!(&self.name_attribute_revisions),
            hosted_documents: copy_map!(&self.hosted_documents),
            retired_documents: copy_set!(&self.retired_documents),
            accessibility_generated_content: copy_map!(&self.accessibility_generated_content),
            accessibility_computed_visibility: copy_map!(&self.accessibility_computed_visibility),
            accessibility_computed_text_style: copy_map!(&self.accessibility_computed_text_style),
            form_associated_custom_elements: copy_set!(&self.form_associated_custom_elements),
        };
        checkpoint()?;
        Ok(snapshot)
    }

    pub fn with_accessibility_generated_content_with_checkpoint(
        &self,
        content: &BTreeMap<NodeId, (String, String)>,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Self, DomError> {
        let mut snapshot = self.clone_with_checkpoint(checkpoint)?;
        let mut output = BTreeMap::new();
        for (&id, value) in content {
            checkpoint()?;
            if self.get(id).is_ok() {
                output.insert(id, value.clone());
            }
        }
        checkpoint()?;
        snapshot.accessibility_generated_content = output;
        Ok(snapshot)
    }

    pub fn set_accessibility_computed_visibility_with_checkpoint(
        &mut self,
        values: &BTreeMap<NodeId, (bool, bool)>,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<(), DomError> {
        checkpoint()?;
        let mut output = BTreeMap::new();
        for (&id, &value) in values {
            checkpoint()?;
            if self.get(id).is_ok() {
                output.insert(id, value);
            }
        }
        checkpoint()?;
        self.accessibility_computed_visibility = output;
        Ok(())
    }

    pub fn set_accessibility_computed_text_style_with_checkpoint(
        &mut self,
        values: &BTreeMap<NodeId, (String, String)>,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<(), DomError> {
        checkpoint()?;
        let mut output = BTreeMap::new();
        for (&id, value) in values {
            checkpoint()?;
            if self.get(id).is_ok() {
                output.insert(id, value.clone());
            }
        }
        checkpoint()?;
        self.accessibility_computed_text_style = output;
        Ok(())
    }

    /// Snapshot exactly one live document and its owned nodes (including detached
    /// nodes, shadow trees and its associated inert template document). Local
    /// NodeIds keep their slots/generations; the snapshot gets a fresh ArenaId and
    /// therefore cannot accept live NodeRefs from the source arena.
    ///
    /// Hosted child documents and embedding parents are omitted. Consumers must
    /// select another document explicitly rather than crossing a frame edge.
    pub fn snapshot_document_scope(&self, root: NodeId) -> Result<Self, DomError> {
        self.snapshot_document_scope_with_checkpoint(root, &mut || Ok(()))
    }

    /// Cooperatively snapshot a document. Errors discard the private draft;
    /// the source arena is never mutated and no partial snapshot is published.
    pub fn snapshot_document_scope_with_checkpoint(
        &self,
        root: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Self, DomError> {
        checkpoint()?;
        self.document_id(root)?;
        if self.document_is_retired(root) {
            return Err(DomError::StaleNode(root));
        }
        self.snapshot_document_roots(root, vec![root], BTreeMap::new(), checkpoint)
    }

    /// Snapshot the selected live root and connected hosted descendants sharing
    /// this arena. Independent documents (for example DOMParser results) are
    /// not authority to publish actionable nodes merely because they share
    /// storage. Detached and retired embedding edges are never followed.
    pub fn snapshot_live_document_tree(&self, root: NodeId) -> Result<Self, DomError> {
        self.snapshot_live_document_tree_with_checkpoint(root, &mut || Ok(()))
    }

    /// Cooperatively snapshot the connected hosted document tree. The caller's
    /// checkpoint remains the sole cancellation/deadline authority.
    pub fn snapshot_live_document_tree_with_checkpoint(
        &self,
        root: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Self, DomError> {
        checkpoint()?;
        self.document_id(root)?;
        if self.document_is_retired(root) {
            return Err(DomError::StaleNode(root));
        }
        let mut documents = BTreeSet::new();
        let mut pending = vec![root];
        let mut hosted = BTreeMap::new();
        while let Some(document) = pending.pop() {
            checkpoint()?;
            if !documents.insert(document) {
                continue;
            }
            for (&frame, &child) in &self.hosted_documents {
                checkpoint()?;
                if self.document_is_retired(child) {
                    continue;
                }
                let mut ancestor = self.get(frame)?.parent;
                let mut connected = false;
                let mut visited = BTreeSet::from([frame]);
                while let Some(id) = ancestor {
                    checkpoint()?;
                    if !visited.insert(id) {
                        return Err(DomError::Cycle);
                    }
                    if id == document {
                        connected = true;
                    }
                    ancestor = self.get(id)?.parent;
                }
                if connected {
                    hosted.insert(frame, child);
                    pending.push(child);
                }
            }
        }
        let mut roots = Vec::new();
        for document in documents {
            checkpoint()?;
            roots.push(document);
        }
        self.snapshot_document_roots(root, roots, hosted, checkpoint)
    }

    fn snapshot_document_roots(
        &self,
        root: NodeId,
        roots: Vec<NodeId>,
        hosted_documents: BTreeMap<NodeId, NodeId>,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Self, DomError> {
        let mut documents = BTreeSet::new();
        let mut pending = roots;
        while let Some(document) = pending.pop() {
            checkpoint()?;
            if !documents.insert(document) {
                continue;
            }
            self.document_id(document)?;
            if let Some(inert) = self.get(document)?.associated_inert_document {
                pending.push(inert);
            }
        }
        let mut retained = BTreeSet::new();
        // Walk slots directly: iter() may skip an unbounded run of tombstones
        // before returning the next live node to its consumer.
        for (index, slot) in self.slots.iter().enumerate() {
            checkpoint()?;
            if let Some(node) = &slot.node {
                let id = NodeId {
                    slot: index as u32,
                    generation: slot.generation,
                };
                if documents.contains(&id)
                    || node
                        .owner_document
                        .is_some_and(|owner| documents.contains(&owner))
                {
                    retained.insert(id);
                }
            }
        }
        let mut free = Vec::new();
        for &index in &self.free {
            checkpoint()?;
            free.push(index);
        }
        let slots = self
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                checkpoint()?;
                let id = NodeId {
                    slot: index as u32,
                    generation: slot.generation,
                };
                if retained.contains(&id) {
                    let mut node = slot.node.clone().expect("retained live slot");
                    node.parent = node.parent.filter(|id| retained.contains(id));
                    node.owner_document = node.owner_document.filter(|id| documents.contains(id));
                    let mut children = Vec::new();
                    for &id in &node.children {
                        checkpoint()?;
                        if retained.contains(&id) {
                            children.push(id);
                        }
                    }
                    node.children = children;
                    let mut sheets = Vec::new();
                    for &id in &node.adopted_stylesheets {
                        checkpoint()?;
                        if retained.contains(&id) {
                            sheets.push(id);
                        }
                    }
                    node.adopted_stylesheets = sheets;
                    node.associated_inert_document = node
                        .associated_inert_document
                        .filter(|id| documents.contains(id));
                    node.template_content =
                        node.template_content.filter(|id| retained.contains(id));
                    node.template_host = node.template_host.filter(|id| retained.contains(id));
                    if let NodeKind::Attribute { owner, .. } = &mut node.kind {
                        *owner = owner.filter(|id| retained.contains(id));
                    }
                    Ok(Slot {
                        generation: slot.generation,
                        node: Some(node),
                    })
                } else if slot.node.is_some() {
                    // Reusing an omitted slot must not revive its previous NodeId in
                    // a mutable snapshot. Exhausted generations remain tombstones.
                    let generation = match slot.generation.checked_add(1) {
                        Some(next) => {
                            free.push(index as u32);
                            next
                        }
                        None => slot.generation,
                    };
                    Ok(Slot {
                        generation,
                        node: None,
                    })
                } else {
                    Ok(slot.clone())
                }
            })
            .collect::<Result<_, DomError>>()?;
        // Do not filter before checkpointing: a wholly omitted index can be
        // just as large as an index retained in the selected document.
        macro_rules! scoped_map {
            ($source:expr, $keep:expr, $copy:expr) => {{
                let mut output = BTreeMap::new();
                for (key, value) in $source {
                    checkpoint()?;
                    if ($keep)(key, value) {
                        let (key, value) = ($copy)(key, value);
                        output.insert(key, value);
                    }
                }
                output
            }};
        }
        let attribute_nodes = scoped_map!(
            &self.attribute_nodes,
            |key: &(NodeId, AttributeKey), attribute: &NodeId| {
                retained.contains(&key.0) && retained.contains(attribute)
            },
            |key: &(NodeId, AttributeKey), value: &NodeId| (key.clone(), *value)
        );
        let stylesheet_revisions = scoped_map!(
            &self.stylesheet_revisions,
            |id, _| retained.contains(id),
            |id: &NodeId, value: &u64| (*id, *value)
        );
        let name_attribute_revisions = scoped_map!(
            &self.name_attribute_revisions,
            |id, _| retained.contains(id),
            |id: &NodeId, value: &u64| (*id, *value)
        );
        let accessibility_generated_content = scoped_map!(
            &self.accessibility_generated_content,
            |id, _| retained.contains(id),
            |id: &NodeId, value: &(String, String)| (*id, value.clone())
        );
        let accessibility_computed_visibility = scoped_map!(
            &self.accessibility_computed_visibility,
            |id, _| retained.contains(id),
            |id: &NodeId, value: &(bool, bool)| (*id, *value)
        );
        let accessibility_computed_text_style = scoped_map!(
            &self.accessibility_computed_text_style,
            |id, _| retained.contains(id),
            |id: &NodeId, value: &(String, String)| (*id, value.clone())
        );
        let mut form_associated_custom_elements = BTreeSet::new();
        for &id in &self.form_associated_custom_elements {
            checkpoint()?;
            if retained.contains(&id) {
                form_associated_custom_elements.insert(id);
            }
        }
        let mut retired_documents = BTreeSet::new();
        for &id in &self.retired_documents {
            checkpoint()?;
            if documents.contains(&id) {
                retired_documents.insert(id);
            }
        }
        let snapshot = Self {
            registry_index: Default::default(),
            id: ArenaId::fresh(),
            allocation_document: Some(root),
            slots,
            free,
            attribute_nodes,
            stylesheet_revisions,
            name_attribute_revisions,
            accessibility_generated_content,
            accessibility_computed_visibility,
            accessibility_computed_text_style,
            form_associated_custom_elements,
            hosted_documents,
            retired_documents,
        };
        checkpoint()?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_scope_keeps_native_ids_but_excludes_embedding_and_nested_documents() {
        let mut dom = DomRuntime::new();
        let top = dom.document();
        let frame = dom.create_element("iframe");
        dom.append_child(top, frame).unwrap();
        let selected = dom.create(NodeKind::Document);
        let button = dom.create_element("button");
        dom.append_child(selected, button).unwrap();
        dom.set_attribute(button, "id", "selected").unwrap();
        let attribute = dom.attribute_node(button, "id").unwrap().unwrap();
        dom.set_attribute(frame, "id", "outside").unwrap();
        let foreign_attribute = dom.attribute_node(frame, "id").unwrap().unwrap();
        dom.set_hosted_document(frame, Some(selected)).unwrap();
        let nested_frame = dom.create_element("iframe");
        dom.append_child(selected, nested_frame).unwrap();
        let nested_document = dom.create(NodeKind::Document);
        let foreign = dom.create_element("input");
        dom.append_child(nested_document, foreign).unwrap();
        dom.set_hosted_document(nested_frame, Some(nested_document))
            .unwrap();
        let reference = dom.arena().node_ref(button).unwrap();
        let snapshot = dom.arena().snapshot_document_scope(selected).unwrap();
        assert_eq!(
            snapshot.get(button).unwrap(),
            dom.arena().get(button).unwrap()
        );
        assert!(snapshot.get(nested_frame).is_ok());
        assert_eq!(
            snapshot.existing_attribute_node(button, "id"),
            Some(attribute)
        );
        assert_eq!(snapshot.existing_attribute_node(frame, "id"), None);
        assert!(snapshot.get(foreign_attribute).is_err());
        for excluded in [top, frame, nested_document, foreign] {
            assert!(snapshot.get(excluded).is_err());
        }
        assert_eq!(snapshot.document_host(selected), None);
        assert_eq!(snapshot.hosted_document(nested_frame), None);
        assert!(matches!(
            snapshot.resolve(reference),
            Err(DomError::WrongArena { .. })
        ));
        assert_eq!(dom.arena().hosted_document(frame), Some(selected));
        assert_eq!(
            dom.arena().hosted_document(nested_frame),
            Some(nested_document)
        );
    }
    #[test]
    fn templates_shadow_styles_and_attribute_indexes_stay_inside_selected_scope() {
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Document);
        let host = arena.insert(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("div"),
            ),
        });
        arena.append_child(root, host).unwrap();
        let shadow = arena.insert(NodeKind::ShadowRoot {
            mode: ShadowMode::Open,
            clonable: false,
        });
        arena.append_child(host, shadow).unwrap();
        let template = arena.insert(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("template"),
            ),
        });
        arena.append_child(shadow, template).unwrap();
        let inert = arena.template_contents_owner_document(root).unwrap();
        let content = arena.insert(NodeKind::DocumentFragment);
        arena.assign_node_document(content, inert).unwrap();
        arena.get_mut(template).unwrap().template_content = Some(content);
        arena.get_mut(content).unwrap().template_host = Some(template);
        let sheet = arena.insert(NodeKind::Text {
            data: "div {color:red}".into(),
        });
        arena
            .get_mut(shadow)
            .unwrap()
            .adopted_stylesheets
            .push(sheet);
        let other = arena.insert(NodeKind::Document);
        let foreign = arena.insert(NodeKind::Text {
            data: "foreign".into(),
        });
        arena.append_child(other, foreign).unwrap();
        let snapshot = arena.snapshot_document_scope(root).unwrap();
        for kept in [root, host, shadow, template, inert, content, sheet] {
            assert!(snapshot.get(kept).is_ok());
        }
        assert_eq!(
            snapshot.get(template).unwrap().template_content,
            Some(content)
        );
        assert_eq!(snapshot.get(content).unwrap().owner_document, Some(inert));
        assert_eq!(
            snapshot.get(shadow).unwrap().adopted_stylesheets,
            vec![sheet]
        );
        assert!(snapshot.get(foreign).is_err());
        assert!(snapshot.get(other).is_err());
    }
    #[test]
    fn scoped_snapshot_preserves_only_retained_accessibility_and_form_projections() {
        let mut dom = DomRuntime::new();
        let outer = dom.document();
        let outside = dom.create_element("x-outside");
        dom.append_child(outer, outside).unwrap();
        let selected = dom.create(NodeKind::Document);
        let kept = dom.create_element("x-kept");
        dom.append_child(selected, kept).unwrap();
        let detached = dom.create_element("x-detached");
        dom.append_child(selected, detached).unwrap();
        dom.remove_child(selected, detached).unwrap();
        let generated = BTreeMap::from([
            (kept, ("before".into(), "after".into())),
            (detached, ("detached".into(), String::new())),
            (outside, ("outside".into(), String::new())),
        ]);
        let mut arena = dom.arena().with_accessibility_generated_content(&generated);
        arena.set_accessibility_computed_visibility(&BTreeMap::from([
            (kept, (false, true)),
            (detached, (true, false)),
            (outside, (false, false)),
        ]));
        arena.set_accessibility_computed_text_style(&BTreeMap::from([
            (kept, ("inline-block".into(), "uppercase".into())),
            (detached, ("none".into(), "none".into())),
            (outside, ("block".into(), "lowercase".into())),
        ]));
        arena
            .form_associated_custom_elements
            .extend([kept, detached, outside]);
        let snapshot = arena.snapshot_document_scope(selected).unwrap();
        assert_eq!(
            snapshot.accessibility_generated_content(kept),
            Some(("before", "after"))
        );
        assert_eq!(
            snapshot.accessibility_computed_visibility(kept),
            Some((false, true))
        );
        assert_eq!(
            snapshot.accessibility_computed_text_style(kept),
            Some(("inline-block", "uppercase"))
        );
        assert_eq!(
            snapshot.accessibility_generated_content(detached),
            Some(("detached", ""))
        );
        assert_eq!(
            snapshot.accessibility_computed_visibility(detached),
            Some((true, false))
        );
        assert_eq!(
            snapshot.accessibility_computed_text_style(detached),
            Some(("none", "none"))
        );
        assert_eq!(
            snapshot.form_associated_custom_elements,
            BTreeSet::from([kept, detached])
        );
        assert!(snapshot.get(outside).is_err());
        assert_eq!(snapshot.accessibility_generated_content(outside), None);
        assert_eq!(snapshot.accessibility_computed_visibility(outside), None);
        assert_eq!(snapshot.accessibility_computed_text_style(outside), None);
        assert_eq!(
            arena.accessibility_generated_content(outside),
            Some(("outside", ""))
        );
        assert!(arena.form_associated_custom_elements.contains(&outside));
    }
    #[test]
    fn omitted_identity_cannot_revive_on_snapshot_allocation_and_bad_roots_reject() {
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Document);
        let other = arena.insert(NodeKind::Document);
        let foreign = arena.insert(NodeKind::Text {
            data: "foreign".into(),
        });
        arena.append_child(other, foreign).unwrap();
        assert_eq!(
            arena.snapshot_document_scope(foreign).unwrap_err(),
            DomError::NotADocument(foreign)
        );
        let mut snapshot = arena.snapshot_document_scope(root).unwrap();
        let allocated = snapshot.insert(NodeKind::Text { data: "new".into() });
        assert_eq!(snapshot.get(allocated).unwrap().owner_document, Some(root));
        assert_ne!(allocated, foreign);
        assert!(snapshot.get(foreign).is_err());
        arena.retired_documents.insert(root);
        assert_eq!(
            arena.snapshot_document_scope(root).unwrap_err(),
            DomError::StaleNode(root)
        );
    }
}

#[cfg(test)]
mod live_tree_snapshot_tests {
    use super::*;
    #[test]
    fn checked_slot_cursor_preserves_order_cancels_retries_and_checks_end() {
        let mut arena = DomArena::default();
        arena.insert(NodeKind::Document);
        for _ in 0..64 {
            arena.insert(NodeKind::Text {
                data: "slot".into(),
            });
        }
        for slot in &mut arena.slots[1..64] {
            slot.node = None;
        }
        let expected = arena.iter().map(|(id, _)| id).collect::<Vec<_>>();
        let mut cursor = 0;
        let mut actual = Vec::new();
        let mut check = || Ok(());
        while let Some((id, _)) = arena.next_with_checkpoint(&mut cursor, &mut check).unwrap() {
            check().unwrap(); // The same callback remains available in the body.
            actual.push(id);
        }
        assert_eq!(actual, expected);
        let mut cursor = 0;
        let mut calls = 0;
        let mut check = || {
            calls += 1;
            if calls == 16 {
                Err(DomError::SelfParent)
            } else {
                Ok(())
            }
        };
        assert!(
            arena
                .next_with_checkpoint(&mut cursor, &mut check)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            arena.next_with_checkpoint(&mut cursor, &mut check),
            Err(DomError::SelfParent)
        );
        assert_eq!(cursor, 15);
        assert_eq!(
            arena
                .next_with_checkpoint(&mut cursor, &mut check)
                .unwrap()
                .unwrap()
                .0,
            expected[1]
        );
        assert_eq!(
            arena.next_with_checkpoint(&mut cursor, &mut || Err(DomError::Cycle)),
            Err(DomError::Cycle)
        );
        assert!(
            arena
                .next_with_checkpoint(&mut cursor, &mut || Ok(()))
                .unwrap()
                .is_none()
        );
        cursor = usize::MAX;
        assert!(
            arena
                .next_with_checkpoint(&mut cursor, &mut || Ok(()))
                .unwrap()
                .is_none()
        );
        assert_eq!(cursor, arena.slots.len());
        assert_eq!(arena.iter().map(|(id, _)| id).collect::<Vec<_>>(), expected);
    }
    #[test]
    fn cooperative_validation_parent_cycles_and_radio_callback_errors_reject() {
        let mut dom = crate::html::parse(
            "<div id=p><input id=a type=radio name=g required><input id=b type=radio name=g></div>",
        )
        .into_runtime()
        .unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        dom.arena_mut().get_mut(parent).unwrap().parent = Some(parent);
        assert_eq!(
            dom.control_tree_root_with_checkpoint(a, &mut || Ok(())),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.has_datalist_ancestor_with_checkpoint(a, &mut || Ok(())),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.control_disabled_with_checkpoint(a, &mut || Ok(())),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.form_owner_with_checkpoint(a, &mut || Ok(())),
            Err(DomError::Cycle)
        );
        let document = dom.document();
        dom.arena_mut().get_mut(parent).unwrap().parent = Some(document);
        // A malformed ordinary peer is ignored as before; cancellation from
        // inside that same predicate must still propagate unchanged.
        dom.arena_mut().get_mut(b).unwrap().parent = Some(b);
        assert_eq!(
            dom.control_value_missing_with_checkpoint(a, &mut || Ok(())),
            Ok(true)
        );
        for stop in 1..32 {
            let mut calls = 0;
            let result = dom.control_value_missing_with_checkpoint(a, &mut || {
                calls += 1;
                if calls == stop {
                    Err(DomError::SelfParent)
                } else {
                    Ok(())
                }
            });
            if calls >= stop {
                assert_eq!(result, Err(DomError::SelfParent));
            }
        }
    }
    #[test]
    fn cooperative_full_clone_retains_foreign_nodes_and_rebinds_parser_identity() {
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Document);
        let foreign = arena.insert(NodeKind::Document);
        let reference = arena.node_ref(foreign).unwrap();
        arena.get_mut(root).unwrap().script_parser_document = Some(reference);
        let copied = arena.clone_with_checkpoint(&mut || Ok(())).unwrap();
        assert!(copied.get(foreign).is_ok());
        let parser = copied.get(root).unwrap().script_parser_document.unwrap();
        assert!(copied.resolve(parser).is_ok());
        assert!(matches!(
            arena.resolve(parser),
            Err(DomError::WrongArena { .. })
        ));
    }

    #[test]
    fn cooperative_projection_setters_do_not_publish_cancelled_maps() {
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Document);
        let old_visibility = BTreeMap::from([(root, (true, false))]);
        let old_style = BTreeMap::from([(root, ("old".into(), "old".into()))]);
        arena.set_accessibility_computed_visibility(&old_visibility);
        arena.set_accessibility_computed_text_style(&old_style);
        let mut calls = 0;
        let mut stop_at_publication = || {
            calls += 1;
            if calls == 3 {
                Err(DomError::Cycle)
            } else {
                Ok(())
            }
        };
        assert_eq!(
            arena.set_accessibility_computed_visibility_with_checkpoint(
                &BTreeMap::from([(root, (false, true))]),
                &mut stop_at_publication
            ),
            Err(DomError::Cycle)
        );
        assert_eq!(
            arena.accessibility_computed_visibility(root),
            Some((true, false))
        );
        let mut calls = 0;
        assert_eq!(
            arena.set_accessibility_computed_text_style_with_checkpoint(
                &BTreeMap::from([(root, ("new".into(), "new".into()))]),
                &mut || {
                    calls += 1;
                    if calls == 3 {
                        Err(DomError::Cycle)
                    } else {
                        Ok(())
                    }
                }
            ),
            Err(DomError::Cycle)
        );
        assert_eq!(
            arena.accessibility_computed_text_style(root),
            Some(("old", "old"))
        );
    }

    #[test]
    fn malformed_host_ancestry_rejects_cycle_even_after_selected_document() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let frame = dom.create_element("iframe");
        dom.append_child(root, frame).unwrap();
        let child = dom.create(NodeKind::Document);
        dom.set_hosted_document(frame, Some(child)).unwrap();
        let arena = dom.arena_mut();
        // Internal corruption is not granted authority by encountering root
        // before discovering the cycle. Normal ancestors excludes frame itself.
        arena.get_mut(root).unwrap().parent = Some(frame);
        let mut calls = 0;
        let error = arena
            .snapshot_live_document_tree_with_checkpoint(root, &mut || {
                calls += 1;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error, DomError::Cycle);
        assert!(calls < 16);
    }

    #[test]
    fn checkpoints_cancel_every_draft_phase_without_changing_source() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        for _ in 0..32 {
            let child = dom.create_element("div");
            dom.append_child(root, child).unwrap();
            dom.set_attribute(child, "id", "kept").unwrap();
            dom.attribute_node(child, "id").unwrap();
        }
        let arena = dom.arena();
        let before = format!("{arena:?}");
        let mut total = 0;
        let success = arena
            .snapshot_live_document_tree_with_checkpoint(root, &mut || {
                total += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(success.get(root).unwrap(), arena.get(root).unwrap());
        assert!(total > 64);
        // Includes cancellation at entry, collection, cloning, index copying,
        // and the final pre-publication checkpoint.
        for stop in 1..=total {
            let mut calls = 0;
            let result = arena.snapshot_live_document_tree_with_checkpoint(root, &mut || {
                calls += 1;
                if calls == stop {
                    Err(DomError::Cycle)
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.unwrap_err(), DomError::Cycle);
            assert_eq!(calls, stop);
            assert_eq!(format!("{arena:?}"), before);
        }
    }

    #[test]
    fn scope_checkpoint_preserves_caller_error_and_unrelated_slot_progress() {
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Document);
        let unrelated = arena.insert(NodeKind::Document);
        for _ in 0..128 {
            let node = arena.insert(NodeKind::Text {
                data: "outside".into(),
            });
            arena.append_child(unrelated, node).unwrap();
        }
        let mut calls = 0;
        let error = arena
            .snapshot_document_scope_with_checkpoint(root, &mut || {
                calls += 1;
                if calls == 64 {
                    Err(DomError::SelfParent)
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error, DomError::SelfParent);
        assert_eq!(calls, 64);
        assert_eq!(arena.get(unrelated).unwrap().children.len(), 128);
    }

    #[test]
    fn cooperative_dom_queries_match_existing_and_preserve_caller_error() {
        let mut dom = crate::html::parse("<label id=l for=s>label</label><select id=s><option>A</option><optgroup><option>B</option></optgroup></select><div id=r aria-labelledby='l s l'></div>").into_runtime().unwrap();
        let label = dom.find_by_id("l").unwrap();
        let select = dom.find_by_id("s").unwrap();
        let relation = dom.find_by_id("r").unwrap();
        let arena = dom.arena();
        assert_eq!(
            arena
                .label_control_with_checkpoint(label, &mut || Ok(()))
                .unwrap(),
            arena.label_control(label).unwrap()
        );
        assert_eq!(
            arena
                .select_options_with_checkpoint(select, &mut || Ok(()))
                .unwrap(),
            arena.select_options(select).unwrap()
        );
        assert_eq!(
            arena
                .text_content_with_checkpoint(select, &mut || Ok(()))
                .unwrap(),
            arena.text_content(select).unwrap()
        );
        assert_eq!(
            arena
                .accessibility_relation_with_checkpoint(relation, "aria-labelledby", &mut || Ok(()))
                .unwrap(),
            arena.accessibility_relation(relation, "aria-labelledby")
        );
        assert_eq!(
            arena.label_control_with_checkpoint(label, &mut || Err(DomError::Cycle)),
            Err(DomError::Cycle)
        );
        assert_eq!(
            arena.select_options_with_checkpoint(select, &mut || Err(DomError::Cycle)),
            Err(DomError::Cycle)
        );
        assert_eq!(
            arena.text_content_with_checkpoint(select, &mut || Err(DomError::Cycle)),
            Err(DomError::Cycle)
        );
        assert_eq!(
            arena.accessibility_relation_with_checkpoint(relation, "aria-labelledby", &mut || Err(
                DomError::Cycle
            )),
            Err(DomError::Cycle)
        );
    }

    #[test]
    fn selected_live_tree_preserves_hosted_edges_and_omits_unrelated_and_retired_documents() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let frame = dom.create_element("iframe");
        dom.append_child(root, frame).unwrap();
        let child = dom.create(NodeKind::Document);
        let button = dom.create_element("button");
        dom.append_child(child, button).unwrap();
        dom.retain_native(child).unwrap();
        dom.set_hosted_document(frame, Some(child)).unwrap();
        let unrelated = dom.create(NodeKind::Document);
        let unrelated_button = dom.create_element("button");
        dom.append_child(unrelated, unrelated_button).unwrap();
        let snapshot = dom.arena().snapshot_live_document_tree(root).unwrap();
        assert_eq!(snapshot.hosted_document(frame), Some(child));
        assert_eq!(snapshot.document_host(child), Some(frame));
        assert!(snapshot.get(button).is_ok());
        assert!(snapshot.get(unrelated).is_err());
        assert!(snapshot.get(unrelated_button).is_err());
        // A selected child does not grant access to its embedding parent.
        let scoped = dom.arena().snapshot_live_document_tree(child).unwrap();
        assert!(scoped.get(button).is_ok());
        assert!(scoped.get(root).is_err());
        assert_eq!(scoped.document_host(child), None);
        // Wrapper retention must not let a retired document re-enter projection.
        dom.set_hosted_document(frame, None).unwrap();
        assert!(dom.arena().get(child).is_ok());
        let snapshot = dom.arena().snapshot_live_document_tree(root).unwrap();
        assert!(snapshot.get(child).is_err());
        assert!(snapshot.get(button).is_err());
        assert!(dom.arena().snapshot_live_document_tree(child).is_err());
        dom.release_native(child).unwrap();
    }

    #[test]
    fn detached_host_is_not_live_document_authority() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let frame = dom.create_element("iframe");
        dom.retain_native(frame).unwrap();
        let child = dom.create(NodeKind::Document);
        dom.set_hosted_document(frame, Some(child)).unwrap();
        let snapshot = dom.arena().snapshot_live_document_tree(root).unwrap();
        assert!(snapshot.get(child).is_err());
        assert_eq!(snapshot.hosted_document(frame), None);
        dom.append_child(root, frame).unwrap();
        assert_eq!(
            dom.arena()
                .snapshot_live_document_tree(root)
                .unwrap()
                .hosted_document(frame),
            Some(child)
        );
        dom.release_native(frame).unwrap();
    }
}
