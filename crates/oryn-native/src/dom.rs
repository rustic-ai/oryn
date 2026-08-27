use std::collections::BTreeMap;

use markup5ever::QualName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    DocumentType { name: String },
    Element { name: QualName },
    Text { data: String },
    Comment { data: String },
    DocumentFragment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    node: Option<Node>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomError {
    #[error("stale or unknown node {0:?}")]
    StaleNode(NodeId),
    #[error("node cannot be its own child")]
    SelfParent,
    #[error("tree operation would create a cycle")]
    Cycle,
}

#[derive(Debug, Default, Clone)]
pub struct DomArena {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl DomArena {
    /// Prepare an arena rebuild that preserves only the explicitly retained
    /// identities. Removed slots advance generation before they can be reused.
    pub fn prepare_rebuild(previous: &Self, retained: &[NodeId]) -> Self {
        let mut arena = Self {
            slots: previous
                .slots
                .iter()
                .map(|slot| Slot {
                    generation: if slot.node.is_some() {
                        slot.generation.wrapping_add(1)
                    } else {
                        slot.generation
                    },
                    node: None,
                })
                .collect(),
            free: Vec::new(),
        };
        for id in retained {
            if let Some(slot) = arena.slots.get_mut(id.slot as usize) {
                slot.generation = id.generation;
            }
        }
        arena.free = arena
            .slots
            .iter()
            .enumerate()
            .filter(|(index, _)| !retained.iter().any(|id| id.slot as usize == *index))
            .map(|(index, _)| index as u32)
            .rev()
            .collect();
        arena
    }

    /// Restore a retained identity into an arena created by `prepare_rebuild`.
    pub fn restore(&mut self, id: NodeId, kind: NodeKind) -> Result<(), DomError> {
        while self.slots.len() <= id.slot as usize {
            self.slots.push(Slot {
                generation: if self.slots.len() == id.slot as usize {
                    id.generation
                } else {
                    0
                },
                node: None,
            });
        }
        let slot = self.slot_mut(id).ok_or(DomError::StaleNode(id))?;
        if slot.node.is_some() {
            return Err(DomError::StaleNode(id));
        }
        slot.node = Some(Node {
            kind,
            parent: None,
            children: Vec::new(),
            attributes: BTreeMap::new(),
        });
        Ok(())
    }

    pub fn insert(&mut self, kind: NodeKind) -> NodeId {
        let node = Node {
            kind,
            parent: None,
            children: Vec::new(),
            attributes: BTreeMap::new(),
        };
        if let Some(slot_index) = self.free.pop() {
            let slot = &mut self.slots[slot_index as usize];
            slot.node = Some(node);
            NodeId {
                slot: slot_index,
                generation: slot.generation,
            }
        } else {
            let slot = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                node: Some(node),
            });
            NodeId {
                slot,
                generation: 0,
            }
        }
    }

    pub fn get(&self, id: NodeId) -> Result<&Node, DomError> {
        self.slot(id)
            .and_then(|slot| slot.node.as_ref())
            .ok_or(DomError::StaleNode(id))
    }

    pub fn get_mut(&mut self, id: NodeId) -> Result<&mut Node, DomError> {
        self.slot_mut(id)
            .and_then(|slot| slot.node.as_mut())
            .ok_or(DomError::StaleNode(id))
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        if parent == child {
            return Err(DomError::SelfParent);
        }
        self.get(parent)?;
        self.get(child)?;
        if self.ancestors(parent)?.contains(&child) {
            return Err(DomError::Cycle);
        }

        if let Some(old_parent) = self.get(child)?.parent {
            self.get_mut(old_parent)?.children.retain(|id| *id != child);
        }
        self.get_mut(child)?.parent = Some(parent);
        self.get_mut(parent)?.children.push(child);
        Ok(())
    }

    pub fn detach(&mut self, id: NodeId) -> Result<(), DomError> {
        let parent = self.get(id)?.parent;
        if let Some(parent) = parent {
            self.get_mut(parent)?.children.retain(|child| *child != id);
            self.get_mut(id)?.parent = None;
        }
        Ok(())
    }

    pub fn insert_before(&mut self, sibling: NodeId, node: NodeId) -> Result<(), DomError> {
        let parent = self
            .get(sibling)?
            .parent
            .ok_or(DomError::StaleNode(sibling))?;
        self.detach(node)?;
        let index = self
            .get(parent)?
            .children
            .iter()
            .position(|child| *child == sibling)
            .ok_or(DomError::StaleNode(sibling))?;
        self.get_mut(node)?.parent = Some(parent);
        self.get_mut(parent)?.children.insert(index, node);
        Ok(())
    }

    pub fn remove(&mut self, id: NodeId) -> Result<Node, DomError> {
        let parent = self.get(id)?.parent;
        if let Some(parent) = parent {
            self.get_mut(parent)?.children.retain(|child| *child != id);
        }
        let slot = self.slot_mut(id).ok_or(DomError::StaleNode(id))?;
        let node = slot.node.take().ok_or(DomError::StaleNode(id))?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.slot);
        Ok(node)
    }

    pub fn ancestors(&self, id: NodeId) -> Result<Vec<NodeId>, DomError> {
        let mut result = Vec::new();
        let mut current = self.get(id)?.parent;
        while let Some(node) = current {
            result.push(node);
            current = self.get(node)?.parent;
        }
        Ok(result)
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.node.as_ref().map(|node| {
                (
                    NodeId {
                        slot: index as u32,
                        generation: slot.generation,
                    },
                    node,
                )
            })
        })
    }

    pub fn text_content(&self, id: NodeId) -> Result<String, DomError> {
        let node = self.get(id)?;
        if let NodeKind::Text { data } = &node.kind {
            return Ok(data.clone());
        }
        let mut result = String::new();
        for child in &node.children {
            result.push_str(&self.text_content(*child)?);
        }
        Ok(result)
    }

    fn slot(&self, id: NodeId) -> Option<&Slot> {
        self.slots
            .get(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)
    }

    fn slot_mut(&mut self, id: NodeId) -> Option<&mut Slot> {
        self.slots
            .get_mut(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)
    }
}

#[cfg(test)]
mod tests {
    use markup5ever::{local_name, ns};

    use super::*;

    #[test]
    fn removed_ids_never_resolve_to_reused_slots() {
        let mut dom = DomArena::default();
        let old = dom.insert(NodeKind::Text { data: "old".into() });
        dom.remove(old).expect("remove old node");
        let new = dom.insert(NodeKind::Text { data: "new".into() });

        assert_eq!(old.slot, new.slot);
        assert_ne!(old.generation, new.generation);
        assert_eq!(dom.get(old), Err(DomError::StaleNode(old)));
        assert!(dom.get(new).is_ok());
    }

    #[test]
    fn tree_operations_prevent_cycles() {
        let mut dom = DomArena::default();
        let parent = dom.insert(NodeKind::Element {
            name: markup5ever::QualName::new(None, ns!(html), local_name!("main")),
        });
        let child = dom.insert(NodeKind::Element {
            name: markup5ever::QualName::new(None, ns!(html), local_name!("button")),
        });
        dom.append_child(parent, child).expect("append child");
        assert_eq!(dom.append_child(child, parent), Err(DomError::Cycle));
    }

    #[test]
    fn rebuild_preserves_retained_ids_and_retires_removed_ids() {
        let mut old = DomArena::default();
        let retained = old.insert(NodeKind::Text {
            data: "retained".into(),
        });
        let removed = old.insert(NodeKind::Text {
            data: "removed".into(),
        });
        let mut rebuilt = DomArena::prepare_rebuild(&old, &[retained]);
        rebuilt
            .restore(
                retained,
                NodeKind::Text {
                    data: "retained".into(),
                },
            )
            .expect("restore retained");
        let replacement = rebuilt.insert(NodeKind::Text {
            data: "replacement".into(),
        });
        assert_eq!(replacement.slot, removed.slot);
        assert_ne!(replacement.generation, removed.generation);
        assert!(rebuilt.get(retained).is_ok());
        assert_eq!(rebuilt.get(removed), Err(DomError::StaleNode(removed)));
    }
}
