//! Document-local snapshots for native layout and semantic projection.
use super::*;

impl DomArena {
    /// Snapshot exactly one live document and its owned nodes (including detached
    /// nodes, shadow trees and its associated inert template document). Local
    /// NodeIds keep their slots/generations; the snapshot gets a fresh ArenaId and
    /// therefore cannot accept live NodeRefs from the source arena.
    ///
    /// Hosted child documents and embedding parents are omitted. Consumers must
    /// select another document explicitly rather than crossing a frame edge.
    pub fn snapshot_document_scope(&self, root: NodeId) -> Result<Self, DomError> {
        self.document_id(root)?;
        if self.document_is_retired(root) {
            return Err(DomError::StaleNode(root));
        }
        self.snapshot_document_roots(root, vec![root], BTreeMap::new())
    }

    /// Snapshot the selected live root and connected hosted descendants sharing
    /// this arena. Independent documents (for example DOMParser results) are
    /// not authority to publish actionable nodes merely because they share
    /// storage. Detached and retired embedding edges are never followed.
    pub fn snapshot_live_document_tree(&self, root: NodeId) -> Result<Self, DomError> {
        self.document_id(root)?;
        if self.document_is_retired(root) {
            return Err(DomError::StaleNode(root));
        }
        let mut documents = BTreeSet::new();
        let mut pending = vec![root];
        let mut hosted = BTreeMap::new();
        while let Some(document) = pending.pop() {
            if !documents.insert(document) {
                continue;
            }
            for (&frame, &child) in &self.hosted_documents {
                if !self.document_is_retired(child) && self.ancestors(frame)?.contains(&document) {
                    hosted.insert(frame, child);
                    pending.push(child);
                }
            }
        }
        self.snapshot_document_roots(root, documents.into_iter().collect(), hosted)
    }

    fn snapshot_document_roots(
        &self,
        root: NodeId,
        roots: Vec<NodeId>,
        hosted_documents: BTreeMap<NodeId, NodeId>,
    ) -> Result<Self, DomError> {
        let mut documents = BTreeSet::new();
        let mut pending = roots;
        while let Some(document) = pending.pop() {
            if !documents.insert(document) {
                continue;
            }
            self.document_id(document)?;
            if let Some(inert) = self.get(document)?.associated_inert_document {
                pending.push(inert);
            }
        }
        let retained = self
            .iter()
            .filter_map(|(id, node)| {
                (documents.contains(&id)
                    || node
                        .owner_document
                        .is_some_and(|owner| documents.contains(&owner)))
                .then_some(id)
            })
            .collect::<BTreeSet<_>>();
        let mut free = self.free.clone();
        let slots = self
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let id = NodeId {
                    slot: index as u32,
                    generation: slot.generation,
                };
                if retained.contains(&id) {
                    let mut node = slot.node.clone().expect("retained live slot");
                    node.parent = node.parent.filter(|id| retained.contains(id));
                    node.owner_document = node.owner_document.filter(|id| documents.contains(id));
                    node.children.retain(|id| retained.contains(id));
                    node.adopted_stylesheets.retain(|id| retained.contains(id));
                    node.associated_inert_document = node
                        .associated_inert_document
                        .filter(|id| documents.contains(id));
                    node.template_content =
                        node.template_content.filter(|id| retained.contains(id));
                    node.template_host = node.template_host.filter(|id| retained.contains(id));
                    if let NodeKind::Attribute { owner, .. } = &mut node.kind {
                        *owner = owner.filter(|id| retained.contains(id));
                    }
                    Slot {
                        generation: slot.generation,
                        node: Some(node),
                    }
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
                    Slot {
                        generation,
                        node: None,
                    }
                } else {
                    slot.clone()
                }
            })
            .collect();
        Ok(Self {
            registry_index: Default::default(),
            id: ArenaId::fresh(),
            allocation_document: Some(root),
            slots,
            free,
            attribute_nodes: self
                .attribute_nodes
                .iter()
                .filter(|((owner, _), attribute)| {
                    retained.contains(owner) && retained.contains(*attribute)
                })
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
            stylesheet_revisions: self
                .stylesheet_revisions
                .iter()
                .filter(|(id, _)| retained.contains(*id))
                .map(|(id, revision)| (*id, *revision))
                .collect(),
            name_attribute_revisions: self
                .name_attribute_revisions
                .iter()
                .filter(|(id, _)| retained.contains(*id))
                .map(|(id, revision)| (*id, *revision))
                .collect(),
            accessibility_generated_content: self
                .accessibility_generated_content
                .iter()
                .filter(|(id, _)| retained.contains(*id))
                .map(|(id, value)| (*id, value.clone()))
                .collect(),
            accessibility_computed_visibility: self
                .accessibility_computed_visibility
                .iter()
                .filter(|(id, _)| retained.contains(*id))
                .map(|(id, value)| (*id, *value))
                .collect(),
            accessibility_computed_text_style: self
                .accessibility_computed_text_style
                .iter()
                .filter(|(id, _)| retained.contains(*id))
                .map(|(id, value)| (*id, value.clone()))
                .collect(),
            form_associated_custom_elements: self
                .form_associated_custom_elements
                .intersection(&retained)
                .copied()
                .collect(),
            hosted_documents,
            retired_documents: self
                .retired_documents
                .intersection(&documents)
                .copied()
                .collect(),
        })
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
