//! HTML selectedcontent cloning. Selection changes and connected insertion
//! trigger cloning; mutations of an option's descendants do not.
use super::*;

const HTML: &str = "http://www.w3.org/1999/xhtml";

impl DomRuntime {
    fn selected_content_owner(&self, target: NodeId) -> Result<Option<NodeId>, DomError> {
        let mut cursor = self.parent(target)?;
        while let Some(node) = cursor {
            // Nested selectedcontent and option content are disabled. Stop at
            // the nearest select, including when selects themselves nest.
            if ["selectedcontent", "option"]
                .iter()
                .any(|tag| self.is_element(node, HTML, tag))
            {
                return Ok(None);
            }
            if self.is_element(node, HTML, "select") {
                return Ok(Some(node));
            }
            if matches!(
                self.arena.get(node)?.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                return Ok(None);
            }
            cursor = self.parent(node)?;
        }
        Ok(None)
    }

    pub fn update_selected_content(&mut self, select: NodeId) -> Result<(), DomError> {
        let mut update = SelectedContentUpdate::select(None, self, select)?;
        let result = update.resume(self);
        let cleanup = update.abort(self);
        result.and(cleanup)
    }
}

struct SelectedTarget {
    node: NodeId,
    clones: Vec<crate::clone_tree::CloneCursor>,
    clone_index: usize,
    added: Vec<NodeId>,
    removed: Vec<NodeId>,
    removed_committed: Vec<NodeId>,
    remove_index: usize,
    insert_index: usize,
    removal: Option<insertion::TreeRemoval>,
    insertion: Option<insertion::TreeInsertion>,
    published: bool,
}
/// Retains each selectedcontent target's accepted source and exact native
/// replace-all prefix. A later target or clone failure never reruns a completed
/// target, even if authored code changes that target before native recovery.
pub(crate) struct SelectedContentUpdate {
    roots: insertion::CursorRoots,
    targets: Vec<SelectedTarget>,
    index: usize,
    aborted: bool,
}
impl SelectedContentUpdate {
    pub(crate) fn inserted(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        root: NodeId,
    ) -> Result<Self, DomError> {
        Self::new(owner, dom, root, None)
    }
    pub(crate) fn select(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        select: NodeId,
    ) -> Result<Self, DomError> {
        Self::new(owner, dom, select, Some(select))
    }
    fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        root: NodeId,
        selected: Option<NodeId>,
    ) -> Result<Self, DomError> {
        let mut update = Self {
            roots: insertion::CursorRoots::new(owner, dom),
            targets: Vec::new(),
            index: 0,
            aborted: false,
        };
        if dom.selected_content_updating || selected.is_none() && !dom.connected.contains(&root) {
            return Ok(update);
        }
        let prepare = (|| -> Result<(), DomError> {
            for target in dom.arena.descendants_inclusive(root)? {
                if !dom.is_element(target, HTML, "selectedcontent")
                    || selected.is_none() && !dom.connected.contains(&target)
                {
                    continue;
                }
                let Some(select) = dom.selected_content_owner(target)? else {
                    continue;
                };
                if selected.is_some_and(|selected| selected != select) {
                    continue;
                }
                let source = dom
                    .select_options(select)?
                    .into_iter()
                    .find(|option| dom.option_selected(*option).unwrap_or(false));
                let source = source
                    .map(|source| dom.arena.get(source).map(|record| record.children.clone()))
                    .transpose()?
                    .unwrap_or_default();
                let clones = source
                    .into_iter()
                    .map(|source| crate::clone_tree::CloneCursor::new(owner, dom, source, true))
                    .collect::<Result<Vec<_>, _>>()?;
                let removed = dom
                    .arena
                    .get(target)?
                    .children
                    .iter()
                    .copied()
                    .filter(|child| {
                        !matches!(
                            dom.arena.get(*child).map(|node| &node.kind),
                            Ok(NodeKind::ShadowRoot { .. })
                        )
                    })
                    .collect::<Vec<_>>();
                update.roots.retain(dom, target)?;
                update.roots.retain(dom, select)?;
                for child in &removed {
                    update.roots.retain(dom, *child)?;
                }
                update.targets.push(SelectedTarget {
                    node: target,
                    clones,
                    clone_index: 0,
                    added: Vec::new(),
                    removed,
                    removed_committed: Vec::new(),
                    remove_index: 0,
                    insert_index: 0,
                    removal: None,
                    insertion: None,
                    published: false,
                });
            }
            Ok(())
        })();
        if let Err(error) = prepare {
            let _ = update.abort(dom);
            return Err(error);
        }
        Ok(update)
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if self.aborted {
            return Err(DomError::MigrationChanged);
        }
        self.roots.validate(dom)?;
        let previous = std::mem::replace(&mut dom.selected_content_updating, true);
        let result = self.resume_inner(dom);
        dom.selected_content_updating = previous;
        result
    }
    fn resume_inner(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        while self.index < self.targets.len() {
            let target = &mut self.targets[self.index];
            while target.clone_index < target.clones.len() {
                let clone = &mut target.clones[target.clone_index];
                clone.resume(dom)?;
                target
                    .added
                    .push(clone.result().ok_or(DomError::MigrationChanged)?);
                target.clone_index += 1;
                #[cfg(test)]
                selected_checkpoint()?;
            }
            while target.remove_index < target.removed.len() {
                let child = target.removed[target.remove_index];
                if target.removal.is_none() {
                    // A later authored reparent is not authority to detach the
                    // node from its new parent during recovery.
                    if dom.arena.get(child)?.parent != Some(target.node) {
                        target.remove_index += 1;
                        continue;
                    }
                    target.removal =
                        Some(insertion::TreeRemoval::new(self.roots.owner(), dom, child)?);
                }
                let cursor = dom.mutation_cursor();
                let result = target.removal.as_mut().unwrap().resume(dom);
                // Suppress only this uninterrupted native slice, including a
                // failed slice's admitted prefix. Never suppress intervening
                // authored entries from an earlier retry interval.
                dom.mutations
                    .suppress_tree_observers_since(cursor, target.node);
                result?;
                target.removed_committed.push(child);
                target.remove_index += 1;
                target.removal.as_mut().unwrap().abort(dom);
                target.removal = None;
                #[cfg(test)]
                selected_checkpoint()?;
            }
            while target.insert_index < target.added.len() {
                let child = target.added[target.insert_index];
                if target.insertion.is_none() {
                    target.insertion = Some(insertion::TreeInsertion::new(
                        self.roots.owner(),
                        dom,
                        target.node,
                        child,
                        None,
                        false,
                    )?);
                }
                let cursor = dom.mutation_cursor();
                let result = target.insertion.as_mut().unwrap().resume(dom);
                dom.mutations
                    .suppress_tree_observers_since(cursor, target.node);
                result?;
                target.insert_index += 1;
                target.insertion.as_mut().unwrap().abort(dom);
                target.insertion = None;
                #[cfg(test)]
                selected_checkpoint()?;
            }
            if !target.published {
                let added_nodes = target
                    .added
                    .iter()
                    .copied()
                    .filter(|child| {
                        !matches!(
                            dom.arena.get(*child).map(|node| &node.kind),
                            Ok(NodeKind::ShadowRoot { .. })
                        )
                    })
                    .collect::<Vec<_>>();
                if !added_nodes.is_empty() || !target.removed_committed.is_empty() {
                    dom.record_mutation(Mutation::ChildrenReplaced {
                        node: target.node,
                        added_nodes,
                        removed_nodes: target.removed_committed.clone(),
                        siblings: MutationSiblings::default(),
                    });
                }
                target.published = true;
            }
            self.index += 1;
            #[cfg(test)]
            selected_checkpoint()?;
        }
        Ok(())
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.aborted = true;
        let mut error = None;
        for target in &mut self.targets {
            if let Some(removal) = target.removal.as_mut() {
                removal.abort(dom);
            }
            if let Some(insertion) = target.insertion.as_mut() {
                insertion.abort(dom);
            }
            for clone in &mut target.clones {
                if let Err(failure) = clone.abort(dom) {
                    error.get_or_insert(failure);
                }
            }
        }
        if let Err(failure) = self.roots.release(dom) {
            error.get_or_insert(failure);
        }
        error.map_or(Ok(()), Err)
    }
}
#[cfg(test)]
thread_local! {static FAIL_SELECTED_AFTER:std::cell::Cell<Option<usize>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
fn selected_checkpoint() -> Result<(), DomError> {
    FAIL_SELECTED_AFTER.with(|slot| match slot.get() {
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
    fn append(dom: &mut DomRuntime, parent: NodeId, tag: &str) -> NodeId {
        let node = dom.create_element(tag);
        dom.append_child(parent, node).unwrap();
        node
    }
    #[test]
    fn explicit_selection_refreshes_but_option_descendant_mutations_do_not() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        dom.retain_native(select).unwrap();
        let target = append(&mut dom, select, "selectedcontent");
        let option = append(&mut dom, select, "option");
        let span = append(&mut dom, option, "span");
        dom.set_text_content(span, "before").unwrap();
        dom.set_select_selected_index(select, 0).unwrap();
        let old = dom.arena.get(target).unwrap().children[0];
        dom.set_text_content(span, "after").unwrap();
        dom.set_attribute(span, "data-test", "changed").unwrap();
        assert_eq!(
            dom.serialize(target).unwrap(),
            "<selectedcontent><span>before</span></selectedcontent>"
        );
        dom.set_select_selected_index(select, 0).unwrap();
        assert_ne!(dom.arena.get(target).unwrap().children[0], old);
        assert!(dom.serialize(target).unwrap().contains("after"));
        dom.release_native(select).unwrap();
        assert!(dom.roots.get(&select).is_none_or(|roots| roots.native == 0));
    }
    #[test]
    fn nested_targets_are_disabled_and_detached_children_keep_their_content() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        dom.retain_native(select).unwrap();
        let target = append(&mut dom, select, "selectedcontent");
        let nested = append(&mut dom, target, "selectedcontent");
        dom.set_text_content(nested, "retained").unwrap();
        dom.retain_native(nested).unwrap();
        let option = append(&mut dom, select, "option");
        dom.set_text_content(option, "one").unwrap();
        dom.set_select_selected_index(select, 0).unwrap();
        assert_eq!(dom.parent(nested).unwrap(), None);
        assert!(dom.serialize(nested).unwrap().contains("retained"));
        assert!(dom.serialize(target).unwrap().contains("one"));
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(
            dom.roots.get(&nested).unwrap().native,
            1,
            "only the explicit fixture guard remains"
        );
        dom.release_native(select).unwrap();
        assert!(dom.roots.get(&select).is_none_or(|roots| roots.native == 0));
        dom.release_native(nested).unwrap();
        assert!(dom.roots.get(&nested).is_none_or(|roots| roots.native == 0));
    }
    fn two_targets() -> (DomRuntime, NodeId, NodeId, NodeId, NodeId) {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let select = append(&mut dom, document, "select");
        let button = append(&mut dom, select, "button");
        let a = append(&mut dom, button, "selectedcontent");
        let b = append(&mut dom, button, "selectedcontent");
        let option = append(&mut dom, select, "option");
        let source = append(&mut dom, option, "span");
        dom.set_text_content(source, "old").unwrap();
        dom.set_select_selected_index(select, 0).unwrap();
        dom.update_selected_content(select).unwrap();
        dom.set_text_content(source, "accepted").unwrap();
        (dom, select, a, b, source)
    }
    #[test]
    fn selected_target_retry_never_reclones_completed_target_or_rereads_source() {
        let (mut dom, select, a, b, source) = two_targets();
        let mut update = SelectedContentUpdate::select(None, &mut dom, select).unwrap();
        FAIL_SELECTED_AFTER.with(|slot| slot.set(Some(3)));
        assert!(update.resume(&mut dom).is_err());
        assert_eq!(update.index, 1);
        let first = dom.children(a).unwrap()[0];
        dom.set_text_content(first, "author-after-first-target")
            .unwrap();
        dom.set_text_content(source, "later-source").unwrap();
        update.resume(&mut dom).unwrap();
        assert_eq!(dom.children(a).unwrap()[0], first);
        assert_eq!(
            dom.arena.text_content(a).unwrap(),
            "author-after-first-target"
        );
        assert_eq!(dom.arena.text_content(b).unwrap(), "accepted");
        let second = dom.children(b).unwrap()[0];
        update.resume(&mut dom).unwrap();
        assert_eq!(dom.children(b).unwrap()[0], second);
        update.abort(&mut dom).unwrap();
    }
    #[test]
    fn replace_retry_preserves_and_does_not_suppress_intervening_author_insert() {
        let (mut dom, select, a, _, _) = two_targets();
        let mut update = SelectedContentUpdate::select(None, &mut dom, select).unwrap();
        FAIL_SELECTED_AFTER.with(|slot| slot.set(Some(1)));
        assert!(update.resume(&mut dom).is_err());
        let before = dom.mutation_cursor();
        let authored = dom.create_element("em");
        dom.append_child(a, authored).unwrap();
        update.resume(&mut dom).unwrap();
        assert_eq!(dom.parent(authored).unwrap(), Some(a));
        assert!(dom.mutations_since(before).mutations.iter().any(|entry|matches!(entry,Mutation::Inserted{node,parent:Some(parent),observer_suppressed:false,..}if *node==authored&&*parent==a)));
        assert_eq!(
            dom.mutations_since(before)
                .mutations
                .iter()
                .filter(|entry| matches!(entry,Mutation::ChildrenReplaced{node,..}if *node==a))
                .count(),
            1
        );
        update.abort(&mut dom).unwrap();
    }

    #[test]
    fn cancelled_selected_update_never_resumes_released_target_custody() {
        let (mut dom, select, first, _, _) = two_targets();
        let mut update = SelectedContentUpdate::select(None, &mut dom, select).unwrap();
        FAIL_SELECTED_AFTER.with(|slot| slot.set(Some(0)));
        assert!(update.resume(&mut dom).is_err());
        update.abort(&mut dom).unwrap();
        let before = dom.children(first).unwrap().to_vec();
        assert!(matches!(
            update.resume(&mut dom),
            Err(DomError::MigrationChanged)
        ));
        assert_eq!(dom.children(first).unwrap(), before.as_slice());
    }
}
