//! Pointer-owned hover state; author attributes cannot manufacture it.
use crate::*;
impl DomRuntime {
    pub(crate) fn forget_pointer_hover_target(&mut self, node: NodeId) {
        self.pointer_hover_targets
            .retain(|(document, _), target| *target != node && *document != node);
    }
    pub fn set_pointer_hover(
        &mut self,
        document: NodeId,
        pointer: i32,
        target: Option<NodeId>,
    ) -> Result<(), DomError> {
        let target = self.prepare_pointer_hover(document, pointer, target)?;
        self.commit_pointer_hover(document, pointer, target);
        Ok(())
    }
    pub fn prepare_pointer_hover(
        &self,
        document: NodeId,
        pointer: i32,
        target: Option<NodeId>,
    ) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(document)?.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(document));
        }
        let target = match target {
            Some(node) => {
                self.arena.get(node)?;
                if self.node_document(node)? != Some(document) {
                    return Err(DomError::PointerHoverDocumentMismatch);
                }
                self.is_connected(node)?.then_some(node)
            }
            None => None,
        };
        let key = (document, pointer);
        // The existing WebDriver input-state admission permits sixteen sources.
        // Native semantic/OS sources share the admitted registry ceiling; this map retains only
        // one generation-checked target per source, never an ancestry vector.
        if target.is_some()
            && !self.pointer_hover_targets.contains_key(&key)
            && self
                .pointer_hover_targets
                .keys()
                .filter(|(owner, _)| *owner == document)
                .count()
                >= oryn_common::MAX_NATIVE_POINTER_SOURCES
        {
            return Err(DomError::PointerSourceCapacity);
        }
        Ok(target)
    }
    /// Commit only a target prepared in this callback-free native boundary.
    pub fn commit_pointer_hover(&mut self, document: NodeId, pointer: i32, target: Option<NodeId>) {
        let key = (document, pointer);
        let old = self.pointer_hover_targets.get(&key).copied();
        if old == target {
            return;
        }
        if let Some(target) = target {
            self.pointer_hover_targets.insert(key, target);
        } else {
            self.pointer_hover_targets.remove(&key);
        }
        self.mutations.push(Mutation::FormState { node: document });
    }
    pub fn clear_pointer_hover(&mut self, document: NodeId) {
        let old = self.pointer_hover_targets.len();
        self.pointer_hover_targets
            .retain(|(owner, _), _| *owner != document);
        if old != self.pointer_hover_targets.len() {
            self.mutations.push(Mutation::FormState { node: document });
        }
    }
    pub fn element_hovered(&self, node: NodeId) -> Result<bool, DomError> {
        self.arena.get(node)?;
        let Some(document) = self.node_document(node)? else {
            return Ok(false);
        };
        Ok(self.pointer_hover_snapshot(document)?.contains(&node))
    }
    pub fn pointer_hover_snapshot(&self, document: NodeId) -> Result<BTreeSet<NodeId>, DomError> {
        if !matches!(self.arena.get(document)?.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(document));
        }
        let mut hovered = BTreeSet::new();
        for ((owner, _), target) in &self.pointer_hover_targets {
            if *owner != document
                || !self.is_connected(*target).unwrap_or(false)
                || self.node_document(*target).ok().flatten() != Some(document)
            {
                continue;
            }
            let mut current = Some(*target);
            let mut seen = BTreeSet::new();
            while let Some(candidate) = current {
                if !seen.insert(candidate) {
                    return Err(DomError::Cycle);
                }
                hovered.insert(candidate);
                if candidate == *owner {
                    break;
                }
                current = self.flat_tree_parent(candidate)?;
            }
        }
        Ok(hovered)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_sources_clear_independently_and_detached_targets_do_not_hover() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let parent = dom.create_element("div");
        let a = dom.create_element("div");
        let b = dom.create_element("div");
        dom.append_child(document, parent).unwrap();
        dom.append_child(parent, a).unwrap();
        dom.append_child(parent, b).unwrap();
        dom.set_pointer_hover(document, 1, Some(a)).unwrap();
        dom.set_pointer_hover(document, 2, Some(b)).unwrap();
        assert!(dom.element_hovered(parent).unwrap());
        dom.set_pointer_hover(document, 1, None).unwrap();
        assert!(!dom.element_hovered(a).unwrap());
        assert!(dom.element_hovered(b).unwrap());
        dom.remove_child(parent, b).unwrap();
        assert!(!dom.element_hovered(parent).unwrap());
        dom.clear_pointer_hover(document);
        assert!(dom.pointer_hover_targets.is_empty());
    }
    #[test]
    fn atomic_move_preserves_hover_and_flat_shadow_ancestry() {
        let mut dom = html::parse("<div id=host></div><div id=other></div>")
            .into_runtime()
            .unwrap();
        let document = dom.document();
        let host = dom.find_by_id("host").unwrap();
        let other = dom.find_by_id("other").unwrap();
        let shadow = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Open,
            clonable: false,
        });
        dom.append_child(host, shadow).unwrap();
        let child = dom.create_element("span");
        dom.append_child(shadow, child).unwrap();
        dom.set_pointer_hover(document, 1, Some(child)).unwrap();
        assert!(dom.element_hovered(host).unwrap());
        dom.move_before(other, host, None).unwrap();
        assert!(dom.element_hovered(host).unwrap());
        assert!(dom.element_hovered(other).unwrap());
        dom.remove_child(other, host).unwrap();
        dom.append_child(other, host).unwrap();
        assert!(!dom.element_hovered(child).unwrap());
    }
}
