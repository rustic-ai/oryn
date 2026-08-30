//! Rust-owned live DOM primitives.
//!
//! JavaScript wrappers retain generation-checked [`NodeId`] values, never
//! pointers into the arena. Mutations are journaled at their source so style,
//! semantic projection, observers, and tracing share one authoritative tree.

use std::collections::{BTreeMap, BTreeSet};

use markup5ever::{LocalName, QualName, ns};
use serde::{Deserialize, Serialize};

pub mod html;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowMode {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    DocumentType { name: String },
    Element { name: QualName },
    Text { data: String },
    Comment { data: String },
    DocumentFragment,
    ShadowRoot { mode: ShadowMode },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: BTreeMap<String, String>,
    pub value: Option<String>,
    pub sensitive_value: bool,
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
    #[error("wrapper lease underflow for node {0:?}")]
    WrapperLeaseUnderflow(NodeId),
    #[error("native root underflow for node {0:?}")]
    NativeRootUnderflow(NodeId),
    #[error("node {0:?} is not a child of {1:?}")]
    NotAChild(NodeId, NodeId),
    #[error("operation requires an element node, got {0:?}")]
    NotAnElement(NodeId),
}

#[derive(Debug, Default, Clone)]
pub struct DomArena {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl DomArena {
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
        slot.node = Some(new_node(kind));
        Ok(())
    }

    pub fn insert(&mut self, kind: NodeKind) -> NodeId {
        let node = new_node(kind);
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.node = Some(node);
            NodeId {
                slot: index,
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
        self.detach(child)?;
        self.get_mut(child)?.parent = Some(parent);
        self.get_mut(parent)?.children.push(child);
        Ok(())
    }

    pub fn detach(&mut self, id: NodeId) -> Result<(), DomError> {
        if let Some(parent) = self.get(id)?.parent {
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
        self.detach(id)?;
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

fn new_node(kind: NodeKind) -> Node {
    Node {
        kind,
        parent: None,
        children: Vec::new(),
        attributes: BTreeMap::new(),
        value: None,
        sensitive_value: false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mutation {
    Inserted {
        node: NodeId,
        parent: Option<NodeId>,
    },
    Removed {
        node: NodeId,
        former_parent: Option<NodeId>,
    },
    Attribute {
        node: NodeId,
        name: String,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    CharacterData {
        node: NodeId,
        old_value: String,
        new_value: String,
    },
    FormState {
        node: NodeId,
    },
    Focus {
        previous: Option<NodeId>,
        current: Option<NodeId>,
    },
    ShadowAttached {
        host: NodeId,
        root: NodeId,
        mode: ShadowMode,
    },
}

#[derive(Debug, Default, Clone)]
struct NodeRoots {
    wrappers: u32,
    native: u32,
}

/// Source-order nodes withheld while a parser-blocking script runs. This is
/// deliberately an internal parser state transition: suspending and restoring
/// the not-yet-parsed tail does not publish JavaScript-visible mutations.
#[derive(Debug)]
pub struct ParserSuspension {
    anchor: NodeId,
    tails: Vec<(NodeId, Vec<NodeId>)>,
}

#[derive(Debug, Default, Clone)]
pub struct DomRuntime {
    arena: DomArena,
    document: Option<NodeId>,
    connected: BTreeSet<NodeId>,
    roots: BTreeMap<NodeId, NodeRoots>,
    ids: BTreeMap<String, NodeId>,
    mutations: Vec<Mutation>,
    focused: Option<NodeId>,
}

impl DomRuntime {
    pub fn new() -> Self {
        let mut runtime = Self::default();
        let document = runtime.arena.insert(NodeKind::Document);
        runtime.document = Some(document);
        runtime.connected.insert(document);
        runtime
    }

    /// Adopt an html5ever-built arena as the live document without replaying
    /// parser mutations. Subsequent parser, JavaScript, and agent changes are
    /// journaled normally.
    pub fn from_parsed(arena: DomArena, document: NodeId) -> Result<Self, DomError> {
        arena.get(document)?;
        let mut runtime = Self {
            arena,
            document: Some(document),
            ..Self::default()
        };
        runtime.rebuild_document_indexes()?;
        Ok(runtime)
    }

    pub fn replace_parsed_document(
        &mut self,
        arena: DomArena,
        document: NodeId,
    ) -> Result<(), DomError> {
        arena.get(document)?;
        self.arena = arena;
        self.document = Some(document);
        self.connected.clear();
        self.roots.clear();
        self.ids.clear();
        self.mutations.clear();
        self.focused = None;
        self.rebuild_document_indexes()
    }

    pub fn arena(&self) -> &DomArena {
        &self.arena
    }
    pub fn arena_mut(&mut self) -> &mut DomArena {
        &mut self.arena
    }

    pub fn snapshot(&self) -> (DomArena, NodeId) {
        (self.arena.clone(), self.document())
    }
    pub fn document(&self) -> NodeId {
        self.document.expect("DOM runtime always has a document")
    }

    pub fn create(&mut self, kind: NodeKind) -> NodeId {
        let node = self.arena.insert(kind);
        self.mutations
            .push(Mutation::Inserted { node, parent: None });
        node
    }

    pub fn create_element(&mut self, local_name: &str) -> NodeId {
        self.create(NodeKind::Element {
            name: QualName::new(None, ns!(html), LocalName::from(local_name)),
        })
    }

    pub fn create_text(&mut self, data: impl Into<String>) -> NodeId {
        self.create(NodeKind::Text { data: data.into() })
    }

    pub fn create_comment(&mut self, data: impl Into<String>) -> NodeId {
        self.create(NodeKind::Comment { data: data.into() })
    }

    pub fn create_fragment(&mut self) -> NodeId {
        self.create(NodeKind::DocumentFragment)
    }

    pub fn import_subtree(&mut self, source: &DomArena, node: NodeId) -> Result<NodeId, DomError> {
        let source_node = source.get(node)?;
        let imported = self.create(source_node.kind.clone());
        {
            let target = self.arena.get_mut(imported)?;
            target.attributes.clone_from(&source_node.attributes);
            target.value.clone_from(&source_node.value);
            target.sensitive_value = source_node.sensitive_value;
        }
        for child in &source_node.children {
            let imported_child = self.import_subtree(source, *child)?;
            self.append_child(imported, imported_child)?;
        }
        Ok(imported)
    }

    pub fn set_inner_html(&mut self, node: NodeId, markup: &str) -> Result<(), DomError> {
        let context_name = match &self.arena.get(node)?.kind {
            NodeKind::Element { name } => name.local.to_string(),
            _ => return Err(DomError::NotAnElement(node)),
        };
        let parsed = html::parse_fragment(markup, &context_name);
        for child in self.arena.get(node)?.children.clone() {
            self.remove_child(node, child)?;
        }
        let document_children = parsed.dom.get(parsed.document)?.children.clone();
        let fragment_children = document_children
            .first()
            .and_then(|root| parsed.dom.get(*root).ok())
            .filter(|root| {
                matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html")
            })
            .map(|root| root.children.clone())
            .unwrap_or(document_children);
        for child in fragment_children {
            let imported = self.import_subtree(&parsed.dom, child)?;
            self.append_child(node, imported)?;
        }
        Ok(())
    }

    pub fn set_outer_html(&mut self, node: NodeId, markup: &str) -> Result<(), DomError> {
        let parent = self
            .arena
            .get(node)?
            .parent
            .ok_or(DomError::NotAnElement(node))?;
        let context_name = match &self.arena.get(parent)?.kind {
            NodeKind::Element { name } => name.local.to_string(),
            _ => "body".into(),
        };
        let parsed = html::parse_fragment(markup, &context_name);
        let document_children = parsed.dom.get(parsed.document)?.children.clone();
        let fragment_children = document_children
            .first()
            .and_then(|root| parsed.dom.get(*root).ok())
            .filter(|root| matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html"))
            .map(|root| root.children.clone())
            .unwrap_or(document_children);
        let mut reference = Some(node);
        for child in fragment_children {
            let imported = self.import_subtree(&parsed.dom, child)?;
            self.insert_before(parent, imported, reference)?;
            reference = Some(node);
        }
        self.remove_child(parent, node)
    }

    /// Insert a contextual HTML fragment immediately after a parser-inserted
    /// script. This is the Rust-owned insertion primitive used by
    /// `document.write` while that script is executing.
    pub fn insert_html_after(
        &mut self,
        anchor: NodeId,
        markup: &str,
    ) -> Result<Vec<NodeId>, DomError> {
        let parent = self.parent(anchor)?.ok_or(DomError::StaleNode(anchor))?;
        let context_name = match &self.arena.get(parent)?.kind {
            NodeKind::Element { name } => name.local.to_string(),
            NodeKind::Document => "html".into(),
            _ => "div".into(),
        };
        let parsed = html::parse_fragment(markup, &context_name);
        let document_children = parsed.dom.get(parsed.document)?.children.clone();
        let fragment_children = document_children
            .first()
            .and_then(|root| parsed.dom.get(*root).ok())
            .filter(|root| {
                matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html")
            })
            .map(|root| root.children.clone())
            .unwrap_or(document_children);
        let reference = {
            let children = &self.arena.get(parent)?.children;
            children
                .iter()
                .position(|child| *child == anchor)
                .and_then(|index| children.get(index + 1).copied())
        };
        let mut inserted = Vec::new();
        for child in fragment_children {
            let imported = self.import_subtree(&parsed.dom, child)?;
            self.insert_before(parent, imported, reference)?;
            inserted.push(imported);
        }
        Ok(inserted)
    }

    pub fn replace_document_contents(&mut self, markup: &str) -> Result<(), DomError> {
        let parsed = html::parse(markup);
        let document = self.document();
        for child in self.arena.get(document)?.children.clone() {
            self.remove_child(document, child)?;
        }
        for child in parsed.dom.get(parsed.document)?.children.clone() {
            let imported = self.import_subtree(&parsed.dom, child)?;
            self.append_child(document, imported)?;
        }
        Ok(())
    }

    /// Hide every source-order branch after `anchor` until its parser-blocking
    /// script has completed. html5ever may already have tokenized a later
    /// network chunk, but script-visible traversal must still observe only the
    /// tree constructed up to the insertion point.
    pub fn suspend_parser_tail(&mut self, anchor: NodeId) -> Result<ParserSuspension, DomError> {
        self.arena.get(anchor)?;
        let mut current = anchor;
        let mut tails = Vec::new();
        while let Some(parent) = self.arena.get(current)?.parent {
            let index = self
                .arena
                .get(parent)?
                .children
                .iter()
                .position(|node| *node == current)
                .ok_or(DomError::NotAChild(current, parent))?;
            let tail = self.arena.get_mut(parent)?.children.split_off(index + 1);
            for node in &tail {
                self.arena.get_mut(*node)?.parent = None;
            }
            if !tail.is_empty() {
                tails.push((parent, tail));
            }
            current = parent;
        }
        self.connected.clear();
        self.ids.clear();
        self.rebuild_document_indexes()?;
        Ok(ParserSuspension { anchor, tails })
    }

    /// Resume parsing after a blocking script. If `document.open()` replaced
    /// the document while the script ran, the old parser tail is discarded.
    pub fn resume_parser_tail(&mut self, suspension: ParserSuspension) -> Result<(), DomError> {
        if self.arena.get(suspension.anchor).is_err()
            || !self.connected.contains(&suspension.anchor)
        {
            return Ok(());
        }
        for (parent, tail) in suspension.tails {
            if self.arena.get(parent).is_err() {
                continue;
            }
            for node in tail {
                if self.arena.get(node).is_err() {
                    continue;
                }
                self.arena.get_mut(node)?.parent = Some(parent);
                self.arena.get_mut(parent)?.children.push(node);
            }
        }
        self.connected.clear();
        self.ids.clear();
        self.rebuild_document_indexes()
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        self.arena.append_child(parent, child)?;
        if self.connected.contains(&parent) {
            self.mark_connected(child)?;
        }
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
        });
        Ok(())
    }

    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), DomError> {
        self.arena.get(parent)?;
        self.arena.get(child)?;
        if let Some(reference) = reference {
            if self.arena.get(reference)?.parent != Some(parent) {
                return Err(DomError::NotAChild(reference, parent));
            }
            self.arena.insert_before(reference, child)?;
        } else {
            self.arena.append_child(parent, child)?;
        }
        if self.connected.contains(&parent) {
            self.mark_connected(child)?;
        }
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
        });
        Ok(())
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        if self.arena.get(child)?.parent != Some(parent) {
            return Err(DomError::NotAChild(child, parent));
        }
        self.remove(child)
    }

    pub fn replace_child(
        &mut self,
        parent: NodeId,
        replacement: NodeId,
        child: NodeId,
    ) -> Result<(), DomError> {
        if self.arena.get(child)?.parent != Some(parent) {
            return Err(DomError::NotAChild(child, parent));
        }
        self.insert_before(parent, replacement, Some(child))?;
        self.remove_child(parent, child)
    }

    pub fn set_attribute(
        &mut self,
        node: NodeId,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let name = name.into();
        let value = value.into();
        let old_value = self
            .arena
            .get_mut(node)?
            .attributes
            .insert(name.clone(), value.clone());
        if name == "id" {
            if let Some(old) = &old_value
                && self.ids.get(old) == Some(&node)
            {
                self.ids.remove(old);
            }
            if self.connected.contains(&node) {
                self.ids.insert(value.clone(), node);
            }
        }
        self.mutations.push(Mutation::Attribute {
            node,
            name,
            old_value,
            new_value: Some(value),
        });
        Ok(())
    }

    pub fn remove_attribute(&mut self, node: NodeId, name: &str) -> Result<(), DomError> {
        if !matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        let old_value = self.arena.get_mut(node)?.attributes.remove(name);
        if name == "id"
            && let Some(old) = &old_value
            && self.ids.get(old) == Some(&node)
        {
            self.ids.remove(old);
        }
        if old_value.is_some() {
            self.mutations.push(Mutation::Attribute {
                node,
                name: name.to_string(),
                old_value,
                new_value: None,
            });
        }
        Ok(())
    }

    pub fn attribute(&self, node: NodeId, name: &str) -> Result<Option<&str>, DomError> {
        let record = self.arena.get(node)?;
        if !matches!(record.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        Ok(record.attributes.get(name).map(String::as_str))
    }

    pub fn set_text(&mut self, node: NodeId, value: impl Into<String>) -> Result<(), DomError> {
        let value = value.into();
        let data = match &mut self.arena.get_mut(node)?.kind {
            NodeKind::Text { data } | NodeKind::Comment { data } => data,
            _ => return Err(DomError::StaleNode(node)),
        };
        let old_value = std::mem::replace(data, value.clone());
        self.mutations.push(Mutation::CharacterData {
            node,
            old_value,
            new_value: value,
        });
        Ok(())
    }

    pub fn set_text_content(
        &mut self,
        node: NodeId,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let value = value.into();
        if matches!(self.arena.get(node)?.kind, NodeKind::Text { .. }) {
            return self.set_text(node, value);
        }
        let children = self.arena.get(node)?.children.clone();
        for child in children {
            self.remove_child(node, child)?;
        }
        if !value.is_empty() {
            let text = self.create_text(value);
            self.append_child(node, text)?;
        }
        Ok(())
    }

    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> Result<NodeId, DomError> {
        let source = self.arena.get(node)?.clone();
        let clone = self.create(source.kind);
        {
            let record = self.arena.get_mut(clone)?;
            record.attributes = source.attributes;
            record.value = source.value;
            record.sensitive_value = source.sensitive_value;
        }
        if deep {
            for child in source.children {
                let child_clone = self.clone_node(child, true)?;
                self.append_child(clone, child_clone)?;
            }
        }
        Ok(clone)
    }

    pub fn remove(&mut self, node: NodeId) -> Result<(), DomError> {
        let parent = self.arena.get(node)?.parent;
        self.arena.detach(node)?;
        self.mark_disconnected(node)?;
        self.mutations.push(Mutation::Removed {
            node,
            former_parent: parent,
        });
        self.reclaim_if_unrooted(node)
    }

    pub fn retain_wrapper(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get(node)?;
        self.roots.entry(node).or_default().wrappers += 1;
        Ok(())
    }

    pub fn release_wrapper(&mut self, node: NodeId) -> Result<(), DomError> {
        let roots = self
            .roots
            .get_mut(&node)
            .ok_or(DomError::WrapperLeaseUnderflow(node))?;
        roots.wrappers = roots
            .wrappers
            .checked_sub(1)
            .ok_or(DomError::WrapperLeaseUnderflow(node))?;
        self.reclaim_if_unrooted(node)
    }

    pub fn retain_native(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get(node)?;
        self.roots.entry(node).or_default().native += 1;
        Ok(())
    }

    pub fn release_native(&mut self, node: NodeId) -> Result<(), DomError> {
        let roots = self
            .roots
            .get_mut(&node)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        roots.native = roots
            .native
            .checked_sub(1)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        self.reclaim_if_unrooted(node)
    }

    pub fn focus(&mut self, current: Option<NodeId>) -> Result<(), DomError> {
        if let Some(node) = current {
            self.arena.get(node)?;
        }
        let previous = std::mem::replace(&mut self.focused, current);
        self.mutations.push(Mutation::Focus { previous, current });
        Ok(())
    }

    pub fn value(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        Ok(self.arena.get(node)?.value.as_deref())
    }

    pub fn set_value(
        &mut self,
        node: NodeId,
        value: impl Into<String>,
        sensitive: bool,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        record.value = Some(value.into());
        record.sensitive_value = sensitive;
        self.mutations.push(Mutation::FormState { node });
        Ok(())
    }

    pub fn find_by_id(&self, id: &str) -> Option<NodeId> {
        self.ids
            .get(id)
            .copied()
            .filter(|node| self.connected.contains(node) && self.arena.get(*node).is_ok())
    }

    pub fn parent(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        Ok(self.arena.get(node)?.parent)
    }

    pub fn children(&self, node: NodeId) -> Result<&[NodeId], DomError> {
        Ok(&self.arena.get(node)?.children)
    }

    pub fn first_element_child(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        Ok(self
            .arena
            .get(node)?
            .children
            .iter()
            .copied()
            .find(|child| {
                self.arena
                    .get(*child)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
            }))
    }

    pub fn document_element(&self) -> Option<NodeId> {
        self.first_element_child(self.document()).ok().flatten()
    }

    pub fn node_name(&self, node: NodeId) -> Result<String, DomError> {
        Ok(match &self.arena.get(node)?.kind {
            NodeKind::Document => "#document".into(),
            NodeKind::DocumentType { name } => name.clone(),
            NodeKind::Element { name } => name.local.as_ref().to_ascii_uppercase(),
            NodeKind::Text { .. } => "#text".into(),
            NodeKind::Comment { .. } => "#comment".into(),
            NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => "#document-fragment".into(),
        })
    }

    pub fn query_selector(&self, root: NodeId, selector: &str) -> Result<Option<NodeId>, DomError> {
        Ok(self.query_selector_all(root, selector)?.into_iter().next())
    }

    pub fn query_selector_all(
        &self,
        root: NodeId,
        selector: &str,
    ) -> Result<Vec<NodeId>, DomError> {
        self.arena.get(root)?;
        let selectors = selector
            .split(',')
            .map(str::trim)
            .filter(|selector| !selector.is_empty())
            .collect::<Vec<_>>();
        let mut result = Vec::new();
        let mut pending = self
            .arena
            .get(root)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            pending.extend(record.children.iter().rev().copied());
            if selectors
                .iter()
                .any(|selector| self.matches_selector_chain(node, selector).unwrap_or(false))
            {
                result.push(node);
            }
        }
        Ok(result)
    }

    pub fn matches_selector(&self, node: NodeId, selector: &str) -> Result<bool, DomError> {
        self.arena.get(node)?;
        Ok(selector
            .split(',')
            .map(str::trim)
            .any(|selector| self.matches_selector_chain(node, selector).unwrap_or(false)))
    }

    fn matches_selector_chain(&self, node: NodeId, selector: &str) -> Result<bool, DomError> {
        let compounds = selector
            .split_ascii_whitespace()
            .filter(|part| !part.is_empty() && *part != ">")
            .collect::<Vec<_>>();
        let Some(last) = compounds.last() else {
            return Ok(false);
        };
        if !matches_compound_selector(self.arena.get(node)?, last) {
            return Ok(false);
        }
        let mut ancestor = self.arena.get(node)?.parent;
        for compound in compounds[..compounds.len() - 1].iter().rev() {
            let mut matched = None;
            while let Some(candidate) = ancestor {
                let record = self.arena.get(candidate)?;
                ancestor = record.parent;
                if matches_compound_selector(record, compound) {
                    matched = Some(candidate);
                    break;
                }
            }
            if matched.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn serialize(&self, node: NodeId) -> Result<String, DomError> {
        let record = self.arena.get(node)?;
        match &record.kind {
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => {
                let mut output = String::new();
                for child in &record.children {
                    output.push_str(&self.serialize(*child)?);
                }
                Ok(output)
            }
            NodeKind::DocumentType { name } => Ok(format!("<!DOCTYPE {name}>")),
            NodeKind::Text { data } => Ok(escape_text(data)),
            NodeKind::Comment { data } => Ok(format!("<!--{data}-->")),
            NodeKind::Element { name } => {
                let mut output = format!("<{}", name.local);
                for (key, value) in &record.attributes {
                    output.push(' ');
                    output.push_str(key);
                    output.push_str("=\"");
                    output.push_str(&escape_attribute(value));
                    output.push('"');
                }
                output.push('>');
                for child in &record.children {
                    output.push_str(&self.serialize(*child)?);
                }
                if !is_void_html_element(&name.local) {
                    output.push_str("</");
                    output.push_str(name.local.as_ref());
                    output.push('>');
                }
                Ok(output)
            }
        }
    }

    pub fn drain_mutations(&mut self) -> Vec<Mutation> {
        std::mem::take(&mut self.mutations)
    }

    fn rebuild_document_indexes(&mut self) -> Result<(), DomError> {
        let document = self.document();
        let mut pending = vec![document];
        while let Some(node) = pending.pop() {
            self.connected.insert(node);
            let record = self.arena.get(node)?;
            if let Some(id) = record.attributes.get("id") {
                self.ids.insert(id.clone(), node);
            }
            pending.extend(record.children.iter().rev().copied());
        }
        Ok(())
    }

    fn mark_connected(&mut self, node: NodeId) -> Result<(), DomError> {
        self.connected.insert(node);
        if let Some(id) = self.arena.get(node)?.attributes.get("id") {
            self.ids.insert(id.clone(), node);
        }
        for child in self.arena.get(node)?.children.clone() {
            self.mark_connected(child)?;
        }
        Ok(())
    }

    fn mark_disconnected(&mut self, node: NodeId) -> Result<(), DomError> {
        self.connected.remove(&node);
        if let Some(id) = self.arena.get(node)?.attributes.get("id")
            && self.ids.get(id) == Some(&node)
        {
            self.ids.remove(id);
        }
        for child in self.arena.get(node)?.children.clone() {
            self.mark_disconnected(child)?;
        }
        Ok(())
    }

    fn reclaim_if_unrooted(&mut self, node: NodeId) -> Result<(), DomError> {
        let rooted = self.connected.contains(&node)
            || self
                .roots
                .get(&node)
                .is_some_and(|roots| roots.wrappers > 0 || roots.native > 0);
        if !rooted {
            let children = self.arena.get(node)?.children.clone();
            for child in children {
                let child_rooted = self.connected.contains(&child)
                    || self
                        .roots
                        .get(&child)
                        .is_some_and(|roots| roots.wrappers > 0 || roots.native > 0);
                if child_rooted {
                    self.arena.get_mut(child)?.parent = None;
                } else {
                    self.reclaim_if_unrooted(child)?;
                }
            }
            let _ = self.arena.remove(node)?;
            self.roots.remove(&node);
        }
        Ok(())
    }
}

fn matches_compound_selector(node: &Node, selector: &str) -> bool {
    let NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let bytes = selector.as_bytes();
    let mut index = 0;
    if bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'*')
    {
        let end = selector
            .find(['#', '.', '[', ':'])
            .unwrap_or(selector.len());
        let tag = &selector[..end];
        if tag != "*" && !name.local.as_ref().eq_ignore_ascii_case(tag) {
            return false;
        }
        index = end;
    }
    while index < selector.len() {
        let marker = bytes[index];
        index += 1;
        let end = match marker {
            b'[' => selector[index..]
                .find(']')
                .map(|offset| index + offset)
                .unwrap_or(selector.len()),
            _ => selector[index..]
                .find(['#', '.', '[', ':'])
                .map(|offset| index + offset)
                .unwrap_or(selector.len()),
        };
        let token = &selector[index..end];
        let matched = match marker {
            b'#' => node
                .attributes
                .get("id")
                .is_some_and(|value| value == token),
            b'.' => node
                .attributes
                .get("class")
                .is_some_and(|value| value.split_ascii_whitespace().any(|class| class == token)),
            b'[' => {
                if let Some((attribute, value)) = token.split_once('=') {
                    let value = value.trim().trim_matches(['\'', '"']);
                    node.attributes
                        .get(attribute.trim())
                        .is_some_and(|actual| actual == value)
                } else {
                    node.attributes.contains_key(token.trim())
                }
            }
            b':' => match token {
                "checked" => node.attributes.contains_key("checked"),
                "disabled" => node.attributes.contains_key("disabled"),
                "enabled" => !node.attributes.contains_key("disabled"),
                _ => false,
            },
            _ => false,
        };
        if !matched {
            return false;
        }
        index = if marker == b'[' && end < selector.len() {
            end + 1
        } else {
            end
        };
    }
    true
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}

fn is_void_html_element(name: &LocalName) -> bool {
    matches!(
        name.as_ref(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use markup5ever::{local_name, ns};

    #[test]
    fn stale_ids_do_not_resolve_after_reuse() {
        let mut dom = DomArena::default();
        let old = dom.insert(NodeKind::Text { data: "old".into() });
        dom.remove(old).unwrap();
        let new = dom.insert(NodeKind::Text { data: "new".into() });
        assert_eq!(old.slot, new.slot);
        assert_ne!(old.generation, new.generation);
        assert_eq!(dom.get(old), Err(DomError::StaleNode(old)));
    }

    #[test]
    fn detached_node_lives_until_wrapper_release() {
        let mut dom = DomRuntime::new();
        let root = dom.create(NodeKind::Element {
            name: QualName::new(None, ns!(html), local_name!("main")),
        });
        dom.append_child(dom.document(), root).unwrap();
        dom.retain_wrapper(root).unwrap();
        dom.remove(root).unwrap();
        assert!(dom.arena().get(root).is_ok());
        dom.release_wrapper(root).unwrap();
        assert_eq!(dom.arena().get(root), Err(DomError::StaleNode(root)));
    }

    #[test]
    fn every_live_mutation_is_journaled() {
        let mut dom = DomRuntime::new();
        let node = dom.create(NodeKind::Element {
            name: QualName::new(None, ns!(html), local_name!("button")),
        });
        dom.append_child(dom.document(), node).unwrap();
        dom.set_attribute(node, "id", "go").unwrap();
        assert_eq!(dom.find_by_id("go"), Some(node));
        assert!(dom.drain_mutations().len() >= 3);
    }

    #[test]
    fn detached_subtrees_reclaim_postorder_without_stale_parent_links() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("section");
        let retained_child = dom.create_element("button");
        let reclaimed_child = dom.create_text("label");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(parent, retained_child).unwrap();
        dom.append_child(parent, reclaimed_child).unwrap();
        dom.retain_wrapper(parent).unwrap();
        dom.retain_wrapper(retained_child).unwrap();
        dom.remove(parent).unwrap();
        dom.release_wrapper(parent).unwrap();
        assert_eq!(dom.parent(retained_child).unwrap(), None);
        assert_eq!(dom.arena().get(parent), Err(DomError::StaleNode(parent)));
        assert_eq!(
            dom.arena().get(reclaimed_child),
            Err(DomError::StaleNode(reclaimed_child))
        );
    }

    #[test]
    fn inner_html_uses_contextual_fragment_parsing_in_the_live_dom() {
        let mut dom = DomRuntime::new();
        let table = dom.create_element("table");
        dom.append_child(dom.document(), table).unwrap();
        dom.set_inner_html(table, "<tr id=row><td>cell</td></tr>")
            .unwrap();
        let row = dom.find_by_id("row").unwrap();
        assert_eq!(dom.node_name(row).unwrap(), "TR");
        assert_eq!(dom.arena().text_content(row).unwrap(), "cell");
    }

    #[test]
    fn selectors_support_compounds_and_descendant_relationships() {
        let parsed = html::parse(
            "<main id=app><section class=panel><button class='primary action' data-id=7 checked>Go</button></section></main>",
        );
        let dom = parsed.into_runtime().unwrap();
        let button = dom
            .query_selector(
                dom.document(),
                "#app .panel button.primary[data-id='7']:checked",
            )
            .unwrap()
            .unwrap();
        assert_eq!(dom.node_name(button).unwrap(), "BUTTON");
        assert!(
            dom.matches_selector(button, "button.action, input")
                .unwrap()
        );
    }
}
