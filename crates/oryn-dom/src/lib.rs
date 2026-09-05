//! Rust-owned live DOM primitives.
//!
//! JavaScript wrappers retain generation-checked [`NodeId`] values, never
//! pointers into the arena. Mutations are journaled at their source so style,
//! semantic projection, observers, and tracing share one authoritative tree.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use markup5ever::{LocalName, Namespace, QualName, ns};
use serde::{Deserialize, Serialize};

pub mod html;

/// Thread-affine ownership shared by html5ever and the page's V8 isolate.
///
/// The page worker deliberately keeps parsing and JavaScript on one thread.
/// `RefCell` therefore provides an executable invariant: V8 cannot enter the
/// DOM while html5ever holds a tree-builder mutation borrow, and neither side
/// ever receives or stores a raw Rust pointer.
pub type SharedDomRuntime = Rc<RefCell<DomRuntime>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LiveRangeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryPoint {
    pub container: NodeId,
    /// A DOM offset: UTF-16 code units for character data and child index for
    /// container nodes.
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRange {
    pub start: BoundaryPoint,
    pub end: BoundaryPoint,
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
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element {
        name: QualName,
    },
    Attribute {
        name: String,
        local_name: String,
        namespace: Option<String>,
        prefix: Option<String>,
        value: String,
        owner: Option<NodeId>,
    },
    Text {
        data: String,
    },
    CDataSection {
        data: String,
    },
    ProcessingInstruction {
        target: String,
        data: String,
    },
    Comment {
        data: String,
    },
    DocumentFragment,
    ShadowRoot {
        mode: ShadowMode,
        clonable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: BTreeMap<String, String>,
    /// Listener types registered through `addEventListener`. This internal
    /// semantic state is never serialized as an HTML attribute.
    pub event_listeners: BTreeSet<String>,
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
    #[error("invalid boundary offset {offset} for node {node:?} (maximum {maximum})")]
    InvalidBoundaryOffset {
        node: NodeId,
        offset: u32,
        maximum: u32,
    },
    #[error("unknown live range {0:?}")]
    UnknownRange(LiveRangeId),
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

    pub fn descendants_inclusive(&self, id: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.get(id)?;
        let mut result = Vec::new();
        let mut pending = vec![id];
        while let Some(node) = pending.pop() {
            result.push(node);
            pending.extend(self.get(node)?.children.iter().rev().copied());
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
        if let NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::ProcessingInstruction { data, .. }
        | NodeKind::Comment { data } = &node.kind
        {
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
        event_listeners: BTreeSet::new(),
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
    EventListener {
        node: NodeId,
        event_type: String,
        present: bool,
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

/// A non-destructive cursor into the live DOM mutation journal. Independent
/// consumers (observers, style, semantics, and tracing) keep their own cursor
/// so one checkpoint cannot hide mutations from another subsystem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutationCursor(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationBatch {
    pub next: MutationCursor,
    pub truncated: bool,
    pub mutations: Vec<Mutation>,
}

#[derive(Debug, Clone)]
struct MutationJournal {
    entries: VecDeque<(u64, Mutation)>,
    next: u64,
    observer: MutationCursor,
}

impl Default for MutationJournal {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next: 1,
            observer: MutationCursor::default(),
        }
    }
}

impl MutationJournal {
    const MAX_ENTRIES: usize = 16_384;

    fn push(&mut self, mutation: Mutation) {
        let sequence = self.next;
        self.next = self.next.saturating_add(1);
        self.entries.push_back((sequence, mutation));
        while self.entries.len() > Self::MAX_ENTRIES {
            self.entries.pop_front();
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn read_since(&self, cursor: MutationCursor) -> MutationBatch {
        let oldest = self
            .entries
            .front()
            .map_or(self.next, |(sequence, _)| *sequence);
        let truncated = cursor.0.saturating_add(1) < oldest;
        let after = if truncated {
            oldest.saturating_sub(1)
        } else {
            cursor.0
        };
        MutationBatch {
            next: MutationCursor(self.next.saturating_sub(1)),
            truncated,
            mutations: self
                .entries
                .iter()
                .filter(|(sequence, _)| *sequence > after)
                .map(|(_, mutation)| mutation.clone())
                .collect(),
        }
    }

    fn drain_observer(&mut self) -> Vec<Mutation> {
        let batch = self.read_since(self.observer);
        self.observer = batch.next;
        batch.mutations
    }
}

#[derive(Debug, Default, Clone)]
struct NodeRoots {
    wrappers: u32,
    native: u32,
}

#[derive(Debug, Default, Clone)]
pub struct DomRuntime {
    arena: DomArena,
    document: Option<NodeId>,
    connected: BTreeSet<NodeId>,
    roots: BTreeMap<NodeId, NodeRoots>,
    ids: BTreeMap<String, NodeId>,
    attribute_nodes: BTreeMap<(NodeId, String), NodeId>,
    mutations: MutationJournal,
    focused: Option<NodeId>,
    next_range: u64,
    ranges: BTreeMap<LiveRangeId, LiveRange>,
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
        self.attribute_nodes.clear();
        self.mutations.clear();
        self.focused = None;
        self.next_range = 0;
        self.ranges.clear();
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

    pub fn create_element_ns(&mut self, namespace: &str, local_name: &str) -> NodeId {
        self.create(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from(namespace),
                LocalName::from(local_name),
            ),
        })
    }

    pub fn create_text(&mut self, data: impl Into<String>) -> NodeId {
        self.create(NodeKind::Text { data: data.into() })
    }

    pub fn create_cdata_section(&mut self, data: impl Into<String>) -> NodeId {
        self.create(NodeKind::CDataSection { data: data.into() })
    }

    pub fn create_processing_instruction(
        &mut self,
        target: impl Into<String>,
        data: impl Into<String>,
    ) -> NodeId {
        self.create(NodeKind::ProcessingInstruction {
            target: target.into(),
            data: data.into(),
        })
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
            NodeKind::ShadowRoot { .. } => self
                .arena
                .get(node)?
                .parent
                .and_then(|host| self.arena.get(host).ok())
                .and_then(|host| match &host.kind {
                    NodeKind::Element { name } => Some(name.local.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "div".into()),
            NodeKind::DocumentFragment => "div".into(),
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

    /// Clear the connected document while retaining its stable `Document`
    /// node. A subsequent html5ever driver can then rebuild that same live
    /// document for `document.open()` without importing a second tree.
    pub fn clear_document(&mut self) -> Result<(), DomError> {
        let document = self.document();
        for child in self.arena.get(document)?.children.clone() {
            self.remove_child(document, child)?;
        }
        let previous = self.focused.take();
        if previous.is_some() {
            self.mutations.push(Mutation::Focus {
                previous,
                current: None,
            });
        }
        Ok(())
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        let former_parent = self.arena.get(child)?.parent;
        if let Some(former_parent) = former_parent {
            let former_index = self.child_index(former_parent, child)?;
            self.adjust_ranges_for_removal(former_parent, former_index, child)?;
        }
        let was_connected = self.connected.contains(&child);
        self.arena.append_child(parent, child)?;
        let inserted_index = self.child_index(parent, child)?;
        self.adjust_ranges_for_insertion(parent, inserted_index, 1)?;
        if was_connected {
            self.mark_disconnected(child)?;
        }
        if self.connected.contains(&parent) {
            self.mark_connected(child)?;
        }
        if former_parent.is_some() {
            self.retain_native(child)?;
            self.mutations.push(Mutation::Removed {
                node: child,
                former_parent,
            });
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
        let former_parent = self.arena.get(child)?.parent;
        if let Some(former_parent) = former_parent {
            let former_index = self.child_index(former_parent, child)?;
            self.adjust_ranges_for_removal(former_parent, former_index, child)?;
        }
        let was_connected = self.connected.contains(&child);
        if let Some(reference) = reference {
            if self.arena.get(reference)?.parent != Some(parent) {
                return Err(DomError::NotAChild(reference, parent));
            }
            self.arena.insert_before(reference, child)?;
        } else {
            self.arena.append_child(parent, child)?;
        }
        let inserted_index = self.child_index(parent, child)?;
        self.adjust_ranges_for_insertion(parent, inserted_index, 1)?;
        if was_connected {
            self.mark_disconnected(child)?;
        }
        if self.connected.contains(&parent) {
            self.mark_connected(child)?;
        }
        if former_parent.is_some() {
            self.retain_native(child)?;
            self.mutations.push(Mutation::Removed {
                node: child,
                former_parent,
            });
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
        self.set_attribute_ns(node, None, name, value)
    }

    pub fn set_attribute_ns(
        &mut self,
        node: NodeId,
        namespace: Option<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let name = name.into();
        let (prefix, local_name) = name
            .split_once(':')
            .map_or((None, name.clone()), |(prefix, local)| {
                (Some(prefix.to_string()), local.to_string())
            });
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
        if let Some(attribute) = self.attribute_nodes.get(&(node, name.clone())).copied()
            && let NodeKind::Attribute {
                local_name: attribute_local_name,
                namespace: attribute_namespace,
                prefix: attribute_prefix,
                value: attribute_value,
                ..
            } = &mut self.arena.get_mut(attribute)?.kind
        {
            attribute_local_name.clone_from(&local_name);
            attribute_namespace.clone_from(&namespace);
            attribute_prefix.clone_from(&prefix);
            attribute_value.clone_from(&value);
        }
        if namespace.is_some() && !self.attribute_nodes.contains_key(&(node, name.clone())) {
            let attribute = self.create(NodeKind::Attribute {
                name: name.clone(),
                local_name,
                namespace,
                prefix,
                value: value.clone(),
                owner: Some(node),
            });
            self.attribute_nodes.insert((node, name.clone()), attribute);
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
        if let Some(attribute) = self.attribute_nodes.remove(&(node, name.to_string()))
            && let NodeKind::Attribute { owner, .. } = &mut self.arena.get_mut(attribute)?.kind
        {
            *owner = None;
        }
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

    pub fn attribute_node(
        &mut self,
        element: NodeId,
        name: &str,
    ) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(element)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(element));
        }
        let normalized = name.to_ascii_lowercase();
        let Some(value) = self
            .arena
            .get(element)?
            .attributes
            .get(&normalized)
            .cloned()
        else {
            return Ok(None);
        };
        if let Some(attribute) = self
            .attribute_nodes
            .get(&(element, normalized.clone()))
            .copied()
        {
            return Ok(Some(attribute));
        }
        let attribute = self.create(NodeKind::Attribute {
            name: normalized.clone(),
            local_name: normalized
                .rsplit_once(':')
                .map_or_else(|| normalized.clone(), |(_, local)| local.to_string()),
            namespace: None,
            prefix: normalized
                .split_once(':')
                .map(|(prefix, _)| prefix.to_string()),
            value,
            owner: Some(element),
        });
        self.attribute_nodes
            .insert((element, normalized), attribute);
        Ok(Some(attribute))
    }

    pub fn set_attribute_node_value(
        &mut self,
        attribute: NodeId,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let value = value.into();
        let (name, owner, old_value) = match &self.arena.get(attribute)?.kind {
            NodeKind::Attribute {
                name, value, owner, ..
            } => (name.clone(), *owner, value.clone()),
            _ => return Err(DomError::StaleNode(attribute)),
        };
        if let NodeKind::Attribute {
            value: attribute_value,
            ..
        } = &mut self.arena.get_mut(attribute)?.kind
        {
            attribute_value.clone_from(&value);
        }
        if let Some(owner) = owner {
            self.arena
                .get_mut(owner)?
                .attributes
                .insert(name.clone(), value.clone());
            self.mutations.push(Mutation::Attribute {
                node: owner,
                name,
                old_value: Some(old_value),
                new_value: Some(value),
            });
        }
        Ok(())
    }

    pub fn set_text(&mut self, node: NodeId, value: impl Into<String>) -> Result<(), DomError> {
        let value = value.into();
        let old_length = match &self.arena.get(node)?.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => utf16_len(data),
            _ => return Err(DomError::StaleNode(node)),
        };
        self.replace_character_data(node, 0, old_length, value)
    }

    /// Replace a UTF-16 range in a character-data node and update every live
    /// Range using the DOM replace-data boundary rules.  Editing, Selection,
    /// and script-visible CharacterData all share this operation so a caret is
    /// never merely clamped after an insertion or deletion.
    pub fn replace_character_data(
        &mut self,
        node: NodeId,
        offset: u32,
        count: u32,
        replacement: impl Into<String>,
    ) -> Result<(), DomError> {
        let replacement = replacement.into();
        let old_value = match &self.arena.get(node)?.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => data.clone(),
            _ => return Err(DomError::StaleNode(node)),
        };
        let mut units = old_value.encode_utf16().collect::<Vec<_>>();
        let start = (offset as usize).min(units.len());
        let end = start.saturating_add(count as usize).min(units.len());
        let inserted = replacement.encode_utf16().collect::<Vec<_>>();
        let inserted_length = inserted.len() as u32;
        let removed_length = (end - start) as u32;
        units.splice(start..end, inserted);
        let value = String::from_utf16_lossy(&units);
        let data = match &mut self.arena.get_mut(node)?.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => data,
            _ => return Err(DomError::StaleNode(node)),
        };
        *data = value.clone();
        let removed_end = offset.saturating_add(removed_length);
        for range in self.ranges.values_mut() {
            for boundary in [&mut range.start, &mut range.end] {
                if boundary.container != node || boundary.offset <= offset {
                    continue;
                }
                if boundary.offset <= removed_end {
                    boundary.offset = offset;
                } else {
                    boundary.offset = boundary
                        .offset
                        .saturating_sub(removed_length)
                        .saturating_add(inserted_length);
                }
            }
        }
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
        if matches!(
            self.arena.get(node)?.kind,
            NodeKind::Text { .. }
                | NodeKind::CDataSection { .. }
                | NodeKind::ProcessingInstruction { .. }
        ) {
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
                // Shadow roots are not light-DOM children and cloneNode does
                // not copy an ordinary (non-clonable) attached shadow tree.
                if let NodeKind::ShadowRoot { clonable, .. } = self.arena.get(child)?.kind
                    && !clonable
                {
                    continue;
                }
                let child_clone = self.clone_node(child, true)?;
                self.append_child(clone, child_clone)?;
            }
        }
        Ok(clone)
    }

    pub fn remove(&mut self, node: NodeId) -> Result<(), DomError> {
        let parent = self.arena.get(node)?.parent;
        if let Some(parent) = parent {
            let index = self.child_index(parent, node)?;
            self.adjust_ranges_for_removal(parent, index, node)?;
        }
        // Mutation delivery may expose the removed subtree to JavaScript.
        // Keep one native lease until every realm has consumed this journal
        // entry; wrapper leases then decide its ordinary detached lifetime.
        self.retain_native(node)?;
        self.arena.detach(node)?;
        self.mark_disconnected(node)?;
        self.mutations.push(Mutation::Removed {
            node,
            former_parent: parent,
        });
        self.reclaim_if_unrooted(node)
    }

    pub fn create_live_range(
        &mut self,
        start: BoundaryPoint,
        end: BoundaryPoint,
    ) -> Result<LiveRangeId, DomError> {
        self.validate_boundary(start)?;
        self.validate_boundary(end)?;
        let id = LiveRangeId(self.next_range.max(1));
        self.next_range = id.0.saturating_add(1);
        self.retain_native(start.container)?;
        self.retain_native(end.container)?;
        self.ranges.insert(id, LiveRange { start, end });
        Ok(id)
    }

    pub fn live_range(&self, id: LiveRangeId) -> Result<LiveRange, DomError> {
        self.ranges
            .get(&id)
            .copied()
            .ok_or(DomError::UnknownRange(id))
    }

    pub fn set_live_range(
        &mut self,
        id: LiveRangeId,
        start: BoundaryPoint,
        end: BoundaryPoint,
    ) -> Result<(), DomError> {
        self.validate_boundary(start)?;
        self.validate_boundary(end)?;
        let previous = self.live_range(id)?;
        self.retain_native(start.container)?;
        self.retain_native(end.container)?;
        self.ranges.insert(id, LiveRange { start, end });
        self.release_native(previous.start.container)?;
        self.release_native(previous.end.container)
    }

    pub fn drop_live_range(&mut self, id: LiveRangeId) -> Result<(), DomError> {
        let range = self.ranges.remove(&id).ok_or(DomError::UnknownRange(id))?;
        self.release_native(range.start.container)?;
        self.release_native(range.end.container)
    }

    fn validate_boundary(&self, boundary: BoundaryPoint) -> Result<(), DomError> {
        let node = self.arena.get(boundary.container)?;
        let maximum = match &node.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => utf16_len(data),
            _ => node.children.len().try_into().unwrap_or(u32::MAX),
        };
        if boundary.offset > maximum {
            return Err(DomError::InvalidBoundaryOffset {
                node: boundary.container,
                offset: boundary.offset,
                maximum,
            });
        }
        Ok(())
    }

    fn child_index(&self, parent: NodeId, child: NodeId) -> Result<u32, DomError> {
        self.arena
            .get(parent)?
            .children
            .iter()
            .position(|candidate| *candidate == child)
            .map(|index| index as u32)
            .ok_or(DomError::NotAChild(child, parent))
    }

    fn adjust_ranges_for_insertion(
        &mut self,
        parent: NodeId,
        index: u32,
        count: u32,
    ) -> Result<(), DomError> {
        self.arena.get(parent)?;
        for range in self.ranges.values_mut() {
            for boundary in [&mut range.start, &mut range.end] {
                if boundary.container == parent && boundary.offset > index {
                    boundary.offset = boundary.offset.saturating_add(count);
                }
            }
        }
        Ok(())
    }

    fn adjust_ranges_for_removal(
        &mut self,
        parent: NodeId,
        index: u32,
        removed: NodeId,
    ) -> Result<(), DomError> {
        self.arena.get(parent)?;
        self.arena.get(removed)?;
        // Shadow-tree boundary points belong to a different tree and remain
        // anchored there when the host (or one of its light-tree ancestors)
        // is removed. Walk only the removed light subtree for Range migration.
        let mut descendant_set = BTreeSet::new();
        let mut pending = vec![removed];
        while let Some(candidate) = pending.pop() {
            let record = self.arena.get(candidate)?;
            if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                continue;
            }
            descendant_set.insert(candidate);
            pending.extend(record.children.iter().copied());
        }
        let mut root_changes = Vec::new();
        for range in self.ranges.values_mut() {
            for boundary in [&mut range.start, &mut range.end] {
                if descendant_set.contains(&boundary.container) {
                    root_changes.push((boundary.container, parent));
                    *boundary = BoundaryPoint {
                        container: parent,
                        offset: index,
                    };
                } else if boundary.container == parent && boundary.offset > index {
                    boundary.offset -= 1;
                }
            }
        }
        for (old, new) in root_changes {
            self.retain_native(new)?;
            self.release_native(old)?;
        }
        Ok(())
    }

    /// Detach a node for an html5ever tree-builder reparenting operation.
    ///
    /// Unlike a script-visible removal, the parser continues to own the
    /// handle and may immediately insert it elsewhere, so reclamation and a
    /// temporary native lease would both be incorrect.
    pub(crate) fn parser_detach(&mut self, node: NodeId) -> Result<(), DomError> {
        let parent = self.arena.get(node)?.parent;
        if parent.is_none() {
            return Ok(());
        }
        self.arena.detach(node)?;
        self.mark_disconnected(node)?;
        self.retain_native(node)?;
        self.mutations.push(Mutation::Removed {
            node,
            former_parent: parent,
        });
        Ok(())
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

    pub const fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    pub fn set_event_listener_presence(
        &mut self,
        node: NodeId,
        event_type: &str,
        present: bool,
    ) -> Result<(), DomError> {
        let event_type = event_type.to_ascii_lowercase();
        let listeners = &mut self.arena.get_mut(node)?.event_listeners;
        let changed = if present {
            listeners.insert(event_type.clone())
        } else {
            listeners.remove(&event_type)
        };
        if changed {
            self.mutations.push(Mutation::EventListener {
                node,
                event_type,
                present,
            });
        }
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

    /// Whether `node` currently participates in this runtime's connected
    /// document tree. Script preparation, custom-element reactions, focus and
    /// semantic invalidation use this instead of guessing from parent links;
    /// detached subtrees may still have complete internal ancestry.
    pub fn is_connected(&self, node: NodeId) -> Result<bool, DomError> {
        self.arena.get(node)?;
        Ok(self.connected.contains(&node))
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
            NodeKind::DocumentType { name, .. } => name.clone(),
            NodeKind::Element { name } => name.local.as_ref().to_ascii_uppercase(),
            NodeKind::Attribute { name, .. } => name.clone(),
            NodeKind::Text { .. } => "#text".into(),
            NodeKind::CDataSection { .. } => "#cdata-section".into(),
            NodeKind::ProcessingInstruction { target, .. } => target.clone(),
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
        if let Some((left, combinator, right)) = split_last_selector_combinator(selector) {
            if !self.matches_selector_chain(node, right)? {
                return Ok(false);
            }
            let record = self.arena.get(node)?;
            return match combinator {
                '>' => record
                    .parent
                    .map(|parent| self.matches_selector_chain(parent, left))
                    .transpose()
                    .map(|value| value.unwrap_or(false)),
                '+' | '~' => {
                    let Some(parent) = record.parent else {
                        return Ok(false);
                    };
                    let siblings = &self.arena.get(parent)?.children;
                    let Some(index) = siblings.iter().position(|candidate| *candidate == node)
                    else {
                        return Ok(false);
                    };
                    let matching = siblings[..index]
                        .iter()
                        .rev()
                        .filter(|candidate| {
                            self.arena
                                .get(**candidate)
                                .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
                        })
                        .take(if combinator == '+' { 1 } else { usize::MAX })
                        .any(|candidate| {
                            self.matches_selector_chain(*candidate, left)
                                .unwrap_or(false)
                        });
                    Ok(matching)
                }
                _ => Ok(false),
            };
        }
        let compounds = selector
            .split_ascii_whitespace()
            .filter(|part| !part.is_empty() && *part != ">")
            .collect::<Vec<_>>();
        let Some(last) = compounds.last() else {
            return Ok(false);
        };
        let focused = last.strip_suffix(":focus");
        if focused.is_some() && self.focused != Some(node) {
            return Ok(false);
        }
        let last = focused.unwrap_or(last);
        if !last.is_empty() && !matches_compound_selector(self.arena.get(node)?, last) {
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
            NodeKind::DocumentType { name, .. } => Ok(format!("<!DOCTYPE {name}>")),
            NodeKind::Text { data } => Ok(escape_text(data)),
            NodeKind::CDataSection { data } => Ok(format!("<![CDATA[{data}]]>")),
            NodeKind::ProcessingInstruction { target, data } => Ok(format!("<?{target} {data}?>")),
            NodeKind::Comment { data } => Ok(format!("<!--{data}-->")),
            NodeKind::Attribute { value, .. } => Ok(escape_attribute(value)),
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
                    if matches!(self.arena.get(*child)?.kind, NodeKind::ShadowRoot { .. }) {
                        continue;
                    }
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
        self.mutations.drain_observer()
    }

    pub fn mutation_cursor(&self) -> MutationCursor {
        MutationCursor(self.mutations.next.saturating_sub(1))
    }

    pub fn mutations_since(&self, cursor: MutationCursor) -> MutationBatch {
        self.mutations.read_since(cursor)
    }

    pub fn release_mutation_roots(&mut self, mutations: &[Mutation]) -> Result<(), DomError> {
        for mutation in mutations {
            if let Mutation::Removed { node, .. } = mutation {
                self.release_native(*node)?;
            }
        }
        Ok(())
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

fn utf16_len(value: &str) -> u32 {
    value.encode_utf16().count().try_into().unwrap_or(u32::MAX)
}

/// Returns the last explicit combinator outside attribute selectors and
/// functional pseudo classes. Descendant whitespace remains handled by the
/// existing ancestor walk below, while explicit child/sibling relationships
/// must retain their exact topology.
fn split_last_selector_combinator(selector: &str) -> Option<(&str, char, &str)> {
    let mut bracket_depth = 0_u32;
    let mut paren_depth = 0_u32;
    let mut quote = None;
    let mut candidate = None;
    for (index, character) in selector.char_indices() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '[' => bracket_depth = bracket_depth.saturating_add(1),
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth = paren_depth.saturating_add(1),
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '>' | '+' | '~' if bracket_depth == 0 && paren_depth == 0 => {
                candidate = Some((index, character));
            }
            _ => {}
        }
    }
    let (index, combinator) = candidate?;
    let left = selector[..index].trim();
    let right = selector[index + combinator.len_utf8()..].trim();
    (!left.is_empty() && !right.is_empty()).then_some((left, combinator, right))
}

fn matches_compound_selector(node: &Node, selector: &str) -> bool {
    let NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let bytes = selector.as_bytes();
    let mut index = 0;
    if bytes
        .first()
        .is_some_and(|byte| !matches!(*byte, b'#' | b'.' | b'[' | b':'))
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
        if !matches!(marker, b'#' | b'.' | b'[' | b':') {
            // This bounded selector matcher intentionally leaves escaped and
            // otherwise unsupported syntax to the Stylo selector path. Never
            // advance through an arbitrary UTF-8 byte and then slice at a
            // non-character boundary.
            return false;
        }
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
        assert!(dom.arena().get(root).is_ok());
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
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
    fn reparenting_preserves_identity_connectivity_and_both_mutation_edges() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("section");
        let second = dom.create_element("section");
        let child = dom.create_element("button");
        dom.set_attribute(child, "id", "move-me").unwrap();
        dom.append_child(dom.document(), first).unwrap();
        dom.append_child(dom.document(), second).unwrap();
        dom.append_child(first, child).unwrap();
        let _ = dom.drain_mutations();

        dom.append_child(second, child).unwrap();
        assert_eq!(dom.parent(child).unwrap(), Some(second));
        assert_eq!(dom.find_by_id("move-me"), Some(child));
        let mutations = dom.drain_mutations();
        assert!(mutations.iter().any(|mutation| matches!(
            mutation,
            Mutation::Removed { node, former_parent: Some(parent) }
                if *node == child && *parent == first
        )));
        assert!(mutations.iter().any(|mutation| matches!(
            mutation,
            Mutation::Inserted { node, parent: Some(parent) }
                if *node == child && *parent == second
        )));
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(dom.parent(child).unwrap(), Some(second));
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
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
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
    fn serialization_preserves_untouched_style_attribute_text() {
        let parsed = html::parse("<select style='visibility:hidden'></select>");
        let dom = parsed.into_runtime().unwrap();
        let select = dom
            .query_selector(dom.document(), "select")
            .unwrap()
            .unwrap();
        assert_eq!(
            dom.serialize(select).unwrap(),
            "<select style=\"visibility:hidden\"></select>"
        );
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

    #[test]
    fn selectors_handle_unicode_without_invalid_utf8_slices() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("élément");
        dom.append_child(dom.document(), element).unwrap();
        assert!(
            dom.query_selector(dom.document(), "élément")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            dom.query_selector(dom.document(), "💥invalid").unwrap(),
            None
        );
    }

    #[test]
    fn live_ranges_follow_insertions_removals_and_character_data() {
        let mut dom = DomRuntime::new();
        let container = dom.create_element("section");
        let before = dom.create_text("before");
        let removed = dom.create_element("span");
        let nested = dom.create_text("nested");
        dom.append_child(dom.document(), container).unwrap();
        dom.append_child(container, before).unwrap();
        dom.append_child(container, removed).unwrap();
        dom.append_child(removed, nested).unwrap();

        let range = dom
            .create_live_range(
                BoundaryPoint {
                    container: nested,
                    offset: 6,
                },
                BoundaryPoint {
                    container,
                    offset: 2,
                },
            )
            .unwrap();
        dom.set_text(nested, "x").unwrap();
        assert_eq!(dom.live_range(range).unwrap().start.offset, 0);

        dom.remove_child(container, removed).unwrap();
        assert_eq!(
            dom.live_range(range).unwrap(),
            LiveRange {
                start: BoundaryPoint {
                    container,
                    offset: 1,
                },
                end: BoundaryPoint {
                    container,
                    offset: 1,
                },
            }
        );

        let inserted = dom.create_element("em");
        dom.insert_before(container, inserted, Some(before))
            .unwrap();
        assert_eq!(dom.live_range(range).unwrap().start.offset, 2);
        dom.drop_live_range(range).unwrap();
    }

    #[test]
    fn character_data_replacement_preserves_live_range_boundaries() {
        let mut dom = DomRuntime::new();
        let text = dom.create_text("abcdef");
        dom.append_child(dom.document(), text).unwrap();
        let before = dom
            .create_live_range(
                BoundaryPoint {
                    container: text,
                    offset: 1,
                },
                BoundaryPoint {
                    container: text,
                    offset: 1,
                },
            )
            .unwrap();
        let inside = dom
            .create_live_range(
                BoundaryPoint {
                    container: text,
                    offset: 3,
                },
                BoundaryPoint {
                    container: text,
                    offset: 3,
                },
            )
            .unwrap();
        let after = dom
            .create_live_range(
                BoundaryPoint {
                    container: text,
                    offset: 6,
                },
                BoundaryPoint {
                    container: text,
                    offset: 6,
                },
            )
            .unwrap();

        dom.replace_character_data(text, 2, 3, "XY").unwrap();
        assert_eq!(dom.arena().text_content(text).unwrap(), "abXYf");
        assert_eq!(dom.live_range(before).unwrap().start.offset, 1);
        assert_eq!(dom.live_range(inside).unwrap().start.offset, 2);
        assert_eq!(dom.live_range(after).unwrap().start.offset, 5);
    }
}
