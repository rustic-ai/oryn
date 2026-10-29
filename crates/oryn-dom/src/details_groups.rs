//! Synchronous named details groups on canonical attribute writes.
use crate::{DomError, DomRuntime, NodeId, NodeKind};

impl DomRuntime {
    pub(super) fn reconcile_inserted_details(&mut self, root: NodeId) -> Result<(), DomError> {
        let mut node = root;
        for _ in 0..=self.arena.slots.len() {
            self.reconcile_details_attribute(node, false)?;
            if let Some(child) = self.arena.get(node)?.children.iter().copied().find(|child| {
                !matches!(self.arena.get(*child).map(|record| &record.kind), Ok(NodeKind::ShadowRoot { .. }))
            }) {
                node = child;
                continue;
            }
            let mut cursor = node;
            let mut next = None;
            for _ in 0..=self.arena.slots.len() {
                if cursor == root { return Ok(()); }
                let parent = self.arena.get(cursor)?.parent.ok_or(DomError::Cycle)?;
                let children = &self.arena.get(parent)?.children;
                let index = children.iter().position(|child| *child == cursor).ok_or(DomError::Cycle)?;
                next = children[index + 1..].iter().copied().find(|child| {
                    !matches!(self.arena.get(*child).map(|record| &record.kind), Ok(NodeKind::ShadowRoot { .. }))
                });
                if next.is_some() { break; }
                cursor = parent;
            }
            node = next.ok_or(DomError::Cycle)?;
        }
        Err(DomError::Cycle)
    }

    // Bounded parent walking needs no retained scratch allocation. Shadow roots
    // terminate the light-tree scope even though their host has a parent edge.
    fn details_tree_root(&self, mut node: NodeId) -> Result<NodeId, DomError> {
        for _ in 0..=self.arena.slots.len() {
            let record = self.arena.get(node)?;
            if matches!(record.kind, NodeKind::Document | NodeKind::ShadowRoot { .. }) {
                return Ok(node);
            }
            match record.parent {
                Some(parent) => node = parent,
                None => return Ok(node),
            }
        }
        Err(DomError::Cycle)
    }

    pub(super) fn reconcile_details_attribute(
        &mut self, node: NodeId, opening: bool,
    ) -> Result<(), DomError> {
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "details")
            || self.arena.get(node)?.attributes.namespaced(None, "open").is_none()
        { return Ok(()); }
        let root = self.details_tree_root(node)?;
        loop {
            let peer = {
                let Some(name) = self.arena.get(node)?.attributes.namespaced(None, "name")
                    .filter(|name| !name.value.is_empty()) else { return Ok(()); };
                let mut peer = None;
                for (other, record) in self.arena.iter() {
                    if other != node
                        && self.is_element(other, "http://www.w3.org/1999/xhtml", "details")
                        && record.attributes.namespaced(None, "open").is_some()
                        && record.attributes.namespaced(None, "name")
                            .is_some_and(|other_name| other_name.value == name.value)
                        && self.details_tree_root(other)? == root
                    { peer = Some(other); break; }
                }
                peer
            };
            let Some(peer) = peer else { return Ok(()); };
            // Opening wins; changing the name of an open member loses to an
            // already-open destination member. Canonical removal journals closes.
            self.remove_attribute_ns(if opening { peer } else { node }, None, "open")?;
            if !opening { return Ok(()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open(dom: &DomRuntime, node: NodeId) -> bool {
        dom.arena.get(node).unwrap().attributes.namespaced(None, "open").is_some()
    }
    #[test]
    fn details_groups_attribute_opening_and_name_changes_are_synchronous() {
        let mut dom = crate::html::parse("<body><details id=a name=g></details><details id=b name=g></details></body>").into_runtime().unwrap();
        let a = dom.get_element_by_id(dom.document(), "a").unwrap().unwrap();
        let b = dom.get_element_by_id(dom.document(), "b").unwrap().unwrap();
        dom.set_attribute(a, "open", "").unwrap();
        dom.set_attribute(b, "open", "").unwrap();
        assert!(!open(&dom, a)); assert!(open(&dom, b));
        dom.set_attribute(a, "name", "other").unwrap();
        dom.set_attribute(a, "open", "").unwrap();
        assert!(open(&dom, a)); assert!(open(&dom, b));
        dom.set_attribute(a, "name", "g").unwrap();
        assert!(!open(&dom, a)); assert!(open(&dom, b));
    }
    #[test]
    fn details_groups_detached_roots_and_empty_names_are_independent() {
        let mut dom = crate::html::parse("<body></body>").into_runtime().unwrap();
        let a = dom.create_element("details"); let b = dom.create_element("details");
        for node in [a, b] {
            dom.set_attribute(node, "name", "g").unwrap();
            dom.set_attribute(node, "open", "").unwrap();
        }
        assert!(open(&dom, a)); assert!(open(&dom, b));
        let parent = dom.create_element("div");
        dom.remove_attribute(a, "name").unwrap(); dom.set_attribute(b, "name", "").unwrap();
        dom.append_child(parent, a).unwrap(); dom.append_child(parent, b).unwrap();
        dom.set_attribute(a, "open", "new-value").unwrap();
        assert!(open(&dom, a)); assert!(open(&dom, b));
    }
    #[test]
    fn details_groups_inserted_open_member_loses_even_before_existing_member() {
        let mut dom = crate::html::parse("<body><details id=a name=g></details></body>").into_runtime().unwrap();
        let a = dom.get_element_by_id(dom.document(), "a").unwrap().unwrap();
        dom.set_attribute(a, "open", "").unwrap();
        let b = dom.create_element("details");
        dom.set_attribute(b, "name", "g").unwrap(); dom.set_attribute(b, "open", "").unwrap();
        let parent = dom.arena.get(a).unwrap().parent.unwrap();
        dom.insert_before(parent, b, Some(a)).unwrap();
        assert!(open(&dom, a)); assert!(!open(&dom, b));
        let wrapper = dom.create_element("div");
        let c = dom.create_element("details");
        dom.set_attribute(c, "name", "g").unwrap(); dom.set_attribute(c, "open", "").unwrap();
        dom.append_child(wrapper, c).unwrap();
        dom.append_child(parent, wrapper).unwrap();
        assert!(open(&dom, a)); assert!(!open(&dom, c));
        // Re-entering the completed reconciliation cannot duplicate a close.
        let cursor = dom.mutation_cursor();
        dom.reconcile_inserted_details(wrapper).unwrap();
        assert_eq!(dom.mutation_cursor(), cursor);
    }
    #[test]
    fn details_groups_parser_keeps_first_open_independent_of_attribute_order() {
        let dom = crate::html::parse("<body><details name=g id=a open></details><details open name=g id=b></details><details name=g open id=c></details></body>").into_runtime().unwrap();
        let nodes = ["a", "b", "c"].map(|id| dom.get_element_by_id(dom.document(), id).unwrap().unwrap());
        assert!(open(&dom, nodes[0]));
        assert!(!open(&dom, nodes[1])); assert!(!open(&dom, nodes[2]));
    }
}
