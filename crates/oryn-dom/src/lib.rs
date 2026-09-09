//! Rust-owned live DOM primitives.
//!
//! JavaScript wrappers retain generation-checked [`NodeId`] values, never
//! pointers into the arena. Mutations are journaled at their source so style,
//! semantic projection, observers, and tracing share one authoritative tree.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use markup5ever::{LocalName, Namespace, Prefix, QualName};
use serde::{Deserialize, Serialize};

pub mod html;
pub mod xml;

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

/// The four DOM selector operations, evaluated by the worker's Stylo adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorOperation {
    First,
    All,
    Matches,
    Closest,
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

/// Native document identity and parser mode, preserved by cloning and import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub content_type: String,
    pub is_html: bool,
    pub quirks: bool,
}

impl Default for DocumentMetadata {
    fn default() -> Self {
        Self {
            content_type: "application/xml".into(),
            is_html: false,
            quirks: false,
        }
    }
}

impl DocumentMetadata {
    pub fn html() -> Self {
        Self {
            content_type: "text/html".into(),
            is_html: true,
            quirks: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub document_metadata: Option<Box<DocumentMetadata>>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: BTreeMap<String, String>,
    /// Listener types registered through `addEventListener`. This internal
    /// semantic state is never serialized as an HTML attribute.
    pub event_listeners: BTreeSet<String>,
    /// Constructed stylesheets adopted by this document or shadow root, in cascade order.
    pub adopted_stylesheets: Vec<NodeId>,
    /// Native CSSOM source, independent of the owner's DOM text children.
    pub stylesheet_override: Option<String>,
    /// Native CSSOM media text, independent of the owner element attribute.
    pub stylesheet_media: Option<String>,
    /// Inert template contents, outside the light DOM tree.
    pub template_content: Option<NodeId>,
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
    attribute_nodes: BTreeMap<(NodeId, String), NodeId>,
    stylesheet_revisions: BTreeMap<NodeId, u64>,
    name_attribute_revisions: BTreeMap<NodeId, u64>,
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
            attribute_nodes: BTreeMap::new(),
            stylesheet_revisions: BTreeMap::new(),
            name_attribute_revisions: BTreeMap::new(),
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
        if let NodeKind::Attribute {
            owner: Some(owner),
            name,
            ..
        } = &self.get(id)?.kind
        {
            self.attribute_nodes.insert((*owner, name.clone()), id);
        }
        Ok(())
    }

    pub fn insert(&mut self, kind: NodeKind) -> NodeId {
        let attribute_key = match &kind {
            NodeKind::Attribute {
                owner: Some(owner),
                name,
                ..
            } => Some((*owner, name.clone())),
            _ => None,
        };
        let node = new_node(kind);
        let id = if let Some(index) = self.free.pop() {
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
        };
        if let Some(key) = attribute_key {
            self.attribute_nodes.insert(key, id);
        }
        id
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
        self.invalidate_stylesheet_ancestors(Some(parent))?;
        self.detach(child)?;
        self.get_mut(child)?.parent = Some(parent);
        self.get_mut(parent)?.children.push(child);
        self.invalidate_stylesheet_subtree(child)?;
        Ok(())
    }

    pub fn detach(&mut self, id: NodeId) -> Result<(), DomError> {
        if let Some(parent) = self.get(id)?.parent {
            self.invalidate_stylesheet_ancestors(Some(parent))?;
            self.get_mut(parent)?.children.retain(|child| *child != id);
            self.get_mut(id)?.parent = None;
            self.invalidate_stylesheet_subtree(id)?;
        }
        Ok(())
    }

    pub fn insert_before(&mut self, sibling: NodeId, node: NodeId) -> Result<(), DomError> {
        let parent = self
            .get(sibling)?
            .parent
            .ok_or(DomError::StaleNode(sibling))?;
        self.invalidate_stylesheet_ancestors(Some(parent))?;
        self.detach(node)?;
        let index = self
            .get(parent)?
            .children
            .iter()
            .position(|child| *child == sibling)
            .ok_or(DomError::StaleNode(sibling))?;
        self.get_mut(node)?.parent = Some(parent);
        self.invalidate_stylesheet_subtree(node)?;
        self.get_mut(parent)?.children.insert(index, node);
        Ok(())
    }

    pub fn remove(&mut self, id: NodeId) -> Result<Node, DomError> {
        self.detach(id)?;
        let slot = self.slot_mut(id).ok_or(DomError::StaleNode(id))?;
        let node = slot.node.take().ok_or(DomError::StaleNode(id))?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.slot);
        self.stylesheet_revisions.remove(&id);
        self.name_attribute_revisions.remove(&id);
        if let NodeKind::Attribute {
            owner: Some(owner),
            name,
            ..
        } = &node.kind
        {
            let key = (*owner, name.clone());
            if self.attribute_nodes.get(&key) == Some(&id) {
                self.attribute_nodes.remove(&key);
            }
        }
        Ok(node)
    }

    pub fn existing_attribute_node(&self, element: NodeId, name: &str) -> Option<NodeId> {
        self.attribute_nodes
            .get(&(element, name.to_string()))
            .copied()
    }

    pub fn attribute_node_ids(&self, element: NodeId) -> Vec<NodeId> {
        self.attribute_nodes
            .range((element, String::new())..)
            .take_while(|((owner, _), _)| *owner == element)
            .map(|(_, attribute)| *attribute)
            .collect()
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

    /// Effective source consumed by the style engine after CSSOM edits.
    pub fn stylesheet_text(&self, id: NodeId) -> Result<String, DomError> {
        match &self.get(id)?.stylesheet_override {
            Some(source) => Ok(source.clone()),
            None => self.text_content(id),
        }
    }

    pub fn stylesheet_media(&self, id: NodeId) -> Result<String, DomError> {
        let node = self.get(id)?;
        Ok(node
            .stylesheet_media
            .clone()
            .unwrap_or_else(|| node.attributes.get("media").cloned().unwrap_or_default()))
    }

    /// DOM stylesheet generations are arena bookkeeping, not structural node data.
    pub fn stylesheet_revision(&self, node: NodeId) -> Result<u64, DomError> {
        self.get(node)?;
        Ok(self.stylesheet_revisions.get(&node).copied().unwrap_or(0))
    }

    fn bump_stylesheet_revision(&mut self, node: NodeId) {
        let revision = self.stylesheet_revisions.entry(node).or_default();
        *revision = revision.saturating_add(1);
    }

    fn invalidate_stylesheet_subtree(&mut self, root: NodeId) -> Result<(), DomError> {
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            let record = self.get_mut(node)?;
            let sheet = matches!(&record.kind, NodeKind::Element { name } if matches!(name.local.as_ref(), "style" | "link"));
            if sheet {
                record.stylesheet_override = None;
                record.stylesheet_media = None;
            }
            pending.extend(record.children.iter().copied());
            if sheet {
                self.bump_stylesheet_revision(node);
            }
        }
        Ok(())
    }

    fn invalidate_stylesheet_ancestors(
        &mut self,
        mut node: Option<NodeId>,
    ) -> Result<(), DomError> {
        while let Some(id) = node {
            let record = self.get_mut(id)?;
            record.stylesheet_override = None;
            let sheet = matches!(&record.kind, NodeKind::Element { name } if matches!(name.local.as_ref(), "style" | "link"));
            node = record.parent;
            if sheet {
                self.get_mut(id)?.stylesheet_media = None;
                self.bump_stylesheet_revision(id);
            }
        }
        Ok(())
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
    let document_metadata =
        matches!(kind, NodeKind::Document).then(|| Box::new(DocumentMetadata::default()));
    Node {
        document_metadata,
        kind,
        parent: None,
        children: Vec::new(),
        attributes: BTreeMap::new(),
        event_listeners: BTreeSet::new(),
        adopted_stylesheets: Vec::new(),
        stylesheet_override: None,
        stylesheet_media: None,
        template_content: None,
        value: None,
        sensitive_value: false,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationSiblings {
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mutation {
    Inserted {
        node: NodeId,
        parent: Option<NodeId>,
        #[serde(default)]
        siblings: MutationSiblings,
        #[serde(default)]
        observer_suppressed: bool,
    },
    Removed {
        node: NodeId,
        former_parent: Option<NodeId>,
        #[serde(default)]
        siblings: MutationSiblings,
        #[serde(default)]
        observer_suppressed: bool,
    },
    /// Observer-only compound record; structural consumers still receive the
    /// individual Inserted/Removed entries, marked observer_suppressed.
    ChildrenReplaced {
        node: NodeId,
        added_nodes: Vec<NodeId>,
        removed_nodes: Vec<NodeId>,
    },
    Attribute {
        node: NodeId,
        name: String,
        #[serde(default)]
        namespace: Option<String>,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    CharacterData {
        node: NodeId,
        old_value: String,
        new_value: String,
    },
    StyleSheets {
        node: NodeId,
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
    discarded_removed_roots: Vec<NodeId>,
}

impl Default for MutationJournal {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next: 1,
            observer: MutationCursor::default(),
            discarded_removed_roots: Vec::new(),
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
            if let Some((sequence, Mutation::Removed { node, .. })) = self.entries.pop_front()
                && sequence > self.observer.0
            {
                // The bounded journal no longer exposes this entry, but its
                // lease must still be released at a safe delivery checkpoint.
                // Entries already drained transfer that lease to their batch.
                self.discarded_removed_roots.push(node);
            }
        }
    }

    fn suppress_tree_observers_since(&mut self, cursor: MutationCursor, target: NodeId) {
        let start = self
            .entries
            .partition_point(|(sequence, _)| *sequence <= cursor.0);
        for (_, mutation) in self.entries.range_mut(start..) {
            match mutation {
                Mutation::Inserted {
                    parent: Some(parent),
                    observer_suppressed,
                    ..
                }
                | Mutation::Removed {
                    former_parent: Some(parent),
                    observer_suppressed,
                    ..
                } if *parent == target => *observer_suppressed = true,
                _ => {}
            }
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
        // Sequence numbers are ordered even after the deque wraps. Seek to
        // this consumer's cursor without scanning unrelated older entries.
        MutationBatch {
            next: MutationCursor(self.next.saturating_sub(1)),
            truncated,
            mutations: self
                .entries
                .range(
                    self.entries
                        .partition_point(|(sequence, _)| *sequence <= after)..,
                )
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
    mutations: MutationJournal,
    focused: Option<NodeId>,
    focus_ring_suppressed: bool,
    next_range: u64,
    ranges: BTreeMap<LiveRangeId, LiveRange>,
}

impl DomRuntime {
    pub fn new() -> Self {
        let mut runtime = Self::default();
        let document = runtime.arena.insert(NodeKind::Document);
        runtime
            .arena
            .get_mut(document)
            .expect("new document")
            .document_metadata = Some(Box::new(DocumentMetadata::html()));
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
        runtime.restore_template_roots()?;
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
        self.focus_ring_suppressed = false;
        self.next_range = 0;
        self.ranges.clear();
        self.restore_template_roots()?;
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
        self.mutations.push(Mutation::Inserted {
            node,
            parent: None,
            siblings: MutationSiblings::default(),
            observer_suppressed: false,
        });
        node
    }

    pub fn create_element(&mut self, local_name: &str) -> NodeId {
        self.create_element_with_local_name("http://www.w3.org/1999/xhtml", local_name)
    }

    /// Create with a literal local name, as Document.createElement requires.
    /// Unlike qualified-name creation, a colon here does not introduce a prefix.
    pub fn create_element_with_local_name(&mut self, namespace: &str, local_name: &str) -> NodeId {
        self.create(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from(namespace),
                LocalName::from(local_name),
            ),
        })
    }

    pub fn create_element_ns(&mut self, namespace: &str, qualified_name: &str) -> NodeId {
        let (prefix, local_name) = qualified_name
            .split_once(':')
            .map_or((None, qualified_name), |(prefix, local)| {
                (Some(Prefix::from(prefix)), local)
            });
        self.create(NodeKind::Element {
            name: QualName::new(
                prefix,
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

    /// Keep the inert fragment attached to its template owner without making
    /// its descendants connected or visible to document selector queries.
    pub fn template_content(&mut self, template: NodeId) -> Result<NodeId, DomError> {
        let record = self.arena.get(template)?;
        if !matches!(&record.kind, NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "template")
        {
            return Err(DomError::NotAnElement(template));
        }
        if let Some(content) = record.template_content {
            return Ok(content);
        }
        let content = self.create_fragment();
        self.retain_native(content)?;
        self.arena.get_mut(template)?.template_content = Some(content);
        Ok(content)
    }

    fn restore_template_roots(&mut self) -> Result<(), DomError> {
        let contents = self
            .arena
            .iter()
            .filter_map(|(_, node)| node.template_content)
            .collect::<Vec<_>>();
        for content in contents {
            self.retain_native(content)?;
        }
        Ok(())
    }

    pub fn import_subtree(&mut self, source: &DomArena, node: NodeId) -> Result<NodeId, DomError> {
        let source_node = source.get(node)?;
        let mut kind = source_node.kind.clone();
        if let NodeKind::Attribute { owner, .. } = &mut kind {
            *owner = None;
        }
        let imported = self.create(kind);
        {
            let target = self.arena.get_mut(imported)?;
            target
                .document_metadata
                .clone_from(&source_node.document_metadata);
            target.attributes.clone_from(&source_node.attributes);
            target.value.clone_from(&source_node.value);
            target.sensitive_value = source_node.sensitive_value;
        }
        for attribute in source.attribute_node_ids(node) {
            if let NodeKind::Attribute {
                namespace,
                name,
                value,
                ..
            } = &source.get(attribute)?.kind
            {
                self.set_attribute_ns(imported, namespace.clone(), name.clone(), value.clone())?;
            }
        }
        if let Some(content) = source_node.template_content {
            let imported_content = self.import_subtree(source, content)?;
            self.retain_native(imported_content)?;
            self.arena.get_mut(imported)?.template_content = Some(imported_content);
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
        let document_children = parsed.dom.get(parsed.document)?.children.clone();
        let fragment_children = document_children
            .first()
            .and_then(|root| parsed.dom.get(*root).ok())
            .filter(|root| {
                matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html")
            })
            .map(|root| root.children.clone())
            .unwrap_or(document_children);
        let mut children = Vec::new();
        for child in fragment_children {
            children.push(self.import_subtree(&parsed.dom, child)?);
        }
        self.replace_with_new_children(node, children)
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
        self.arena.get_mut(document)?.document_metadata =
            parsed.dom.get(parsed.document)?.document_metadata.clone();
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

    fn mutation_siblings_at(
        &self,
        parent: NodeId,
        index: u32,
    ) -> Result<MutationSiblings, DomError> {
        let children = &self.arena.get(parent)?.children;
        let index = index as usize;
        let light = |node: &&NodeId| {
            !matches!(
                self.arena.get(**node).map(|record| &record.kind),
                Ok(NodeKind::ShadowRoot { .. })
            )
        };
        Ok(MutationSiblings {
            previous: children[..index].iter().rev().find(light).copied(),
            next: children[index + 1..].iter().find(light).copied(),
        })
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        let former_parent = self.arena.get(child)?.parent;
        let mut removed_siblings = MutationSiblings::default();
        if let Some(former_parent) = former_parent {
            let former_index = self.child_index(former_parent, child)?;
            removed_siblings = self.mutation_siblings_at(former_parent, former_index)?;
            self.adjust_ranges_for_removal(former_parent, former_index, child)?;
        }
        let was_connected = self.connected.contains(&child);
        self.arena.append_child(parent, child)?;
        let inserted_index = self.child_index(parent, child)?;
        let inserted_siblings = self.mutation_siblings_at(parent, inserted_index)?;
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
                siblings: removed_siblings,
                observer_suppressed: false,
            });
        }
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
            siblings: inserted_siblings,
            observer_suppressed: false,
        });
        Ok(())
    }

    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), DomError> {
        self.insert_before_with_focus(parent, child, reference, false)
    }

    /// Reparent through the state-preserving moveBefore path. Bindings validate
    /// its document/connectedness constraints before calling this operation.
    pub fn move_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), DomError> {
        self.insert_before_with_focus(parent, child, reference, true)
    }

    fn insert_before_with_focus(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
        preserve_focus: bool,
    ) -> Result<(), DomError> {
        self.arena.get(parent)?;
        self.arena.get(child)?;
        // The DOM insertion algorithms define inserting a node immediately
        // before itself as a no-op. Resolve this before detaching the node or
        // the reference ceases to be a child and its stable identity can be
        // reclaimed as stale.
        if reference == Some(child) {
            return Ok(());
        }
        let former_parent = self.arena.get(child)?.parent;
        let mut removed_siblings = MutationSiblings::default();
        if let Some(former_parent) = former_parent {
            let former_index = self.child_index(former_parent, child)?;
            removed_siblings = self.mutation_siblings_at(former_parent, former_index)?;
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
        let inserted_siblings = self.mutation_siblings_at(parent, inserted_index)?;
        self.adjust_ranges_for_insertion(parent, inserted_index, 1)?;
        if was_connected {
            self.mark_disconnected_with_focus(
                child,
                preserve_focus && self.connected.contains(&parent),
            )?;
        }
        if self.connected.contains(&parent) {
            self.mark_connected(child)?;
        }
        if former_parent.is_some() {
            self.retain_native(child)?;
            self.mutations.push(Mutation::Removed {
                node: child,
                former_parent,
                siblings: removed_siblings,
                observer_suppressed: false,
            });
        }
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
            siblings: inserted_siblings,
            observer_suppressed: false,
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
        self.arena.get(replacement)?;
        if replacement == parent {
            return Err(DomError::SelfParent);
        }
        if self.arena.ancestors(parent)?.contains(&replacement) {
            return Err(DomError::Cycle);
        }
        let index = self.child_index(parent, child)? as usize;
        let reference = self.arena.get(parent)?.children[index + 1..]
            .iter()
            .copied()
            .find(|candidate| {
                *candidate != replacement
                    && !matches!(
                        self.arena.get(*candidate).map(|node| &node.kind),
                        Ok(NodeKind::ShadowRoot { .. })
                    )
            });
        // Range offsets must see removal before insertion at the vacated slot.
        // The removal journal retains the child, including self-replacement.
        self.remove_child(parent, child)?;
        self.insert_before(parent, replacement, reference)
    }

    /// Publish CSSOM media without changing HTML attributes or DOM records.
    pub fn set_stylesheet_media(&mut self, node: NodeId, media: String) -> Result<(), DomError> {
        self.arena.get_mut(node)?.stylesheet_media = Some(media);
        self.mutations.push(Mutation::StyleSheets { node });
        Ok(())
    }

    /// Publish CSSOM source without rewriting DOM text or emitting DOM records.
    pub fn set_stylesheet_text(&mut self, node: NodeId, source: String) -> Result<(), DomError> {
        self.arena.get_mut(node)?.stylesheet_override = Some(source);
        self.mutations.push(Mutation::StyleSheets { node });
        Ok(())
    }

    /// Update internal CSSOM state without exposing a synthetic HTML attribute
    /// or a DOM MutationObserver record. Native roots preserve adopted sheets
    /// independently of wrapper collection, including repeated adoption.
    pub fn set_adopted_stylesheets(
        &mut self,
        node: NodeId,
        sheets: Vec<NodeId>,
    ) -> Result<(), DomError> {
        let previous = self.arena.get(node)?.adopted_stylesheets.clone();
        if previous == sheets {
            return Ok(());
        }
        for sheet in &sheets {
            self.arena.get(*sheet)?;
        }
        let old = previous.into_iter().collect::<BTreeSet<_>>();
        let new = sheets.iter().copied().collect::<BTreeSet<_>>();
        for sheet in new.difference(&old) {
            self.retain_native(*sheet)?;
        }
        self.arena.get_mut(node)?.adopted_stylesheets = sheets;
        for sheet in old.difference(&new) {
            self.release_native(*sheet)?;
        }
        self.mutations.push(Mutation::StyleSheets { node });
        Ok(())
    }

    pub fn set_attribute(
        &mut self,
        node: NodeId,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let name = name.into();
        // A qualified-name update changes the value of an existing attribute;
        // it must not replace that attribute's expanded name.
        let namespace = self
            .arena
            .attribute_nodes
            .get(&(node, name.clone()))
            .and_then(|attribute| self.arena.get(*attribute).ok())
            .and_then(|record| match &record.kind {
                NodeKind::Attribute { namespace, .. } => namespace.clone(),
                _ => None,
            });
        self.set_attribute_ns(node, namespace, name, value)
    }

    pub fn set_attribute_ns(
        &mut self,
        node: NodeId,
        namespace: Option<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let name = name.into();
        let (prefix, local_name) = if namespace.is_some() {
            name.split_once(':')
                .map_or((None, name.clone()), |(prefix, local)| {
                    (Some(prefix.to_string()), local.to_string())
                })
        } else {
            (None, name.clone())
        };
        let value = value.into();
        let replaced_unqualified_name = name == "name"
            && self.arena.get(node)?.attributes.contains_key("name")
            && self.arena.existing_attribute_node(node, "name").is_none_or(|attribute| {
                self.arena.get(attribute).is_ok_and(|record| matches!(&record.kind, NodeKind::Attribute { namespace, .. } if namespace.is_none()))
            });
        let old_value = self
            .arena
            .get_mut(node)?
            .attributes
            .insert(name.clone(), value.clone());
        if name == "media" && namespace.is_none() {
            self.arena.get_mut(node)?.stylesheet_media = None;
        }
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
        if let Some(attribute) = self
            .arena
            .attribute_nodes
            .get(&(node, name.clone()))
            .copied()
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
        if namespace.is_some()
            && !self
                .arena
                .attribute_nodes
                .contains_key(&(node, name.clone()))
        {
            let attribute = self.create(NodeKind::Attribute {
                name: name.clone(),
                local_name,
                namespace: namespace.clone(),
                prefix,
                value: value.clone(),
                owner: Some(node),
            });
            self.arena
                .attribute_nodes
                .insert((node, name.clone()), attribute);
        }
        if name == "name" && (namespace.is_none() || replaced_unqualified_name) {
            let revision = self.arena.name_attribute_revisions.entry(node).or_default();
            *revision = revision.saturating_add(1);
        }
        self.mutations.push(Mutation::Attribute {
            node,
            name,
            namespace,
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
        let mut namespace = None;
        if let Some(attribute) = self.arena.attribute_nodes.remove(&(node, name.to_string()))
            && let NodeKind::Attribute {
                owner,
                namespace: attribute_namespace,
                ..
            } = &mut self.arena.get_mut(attribute)?.kind
        {
            namespace.clone_from(attribute_namespace);
            *owner = None;
        }
        if name == "id"
            && let Some(old) = &old_value
            && self.ids.get(old) == Some(&node)
        {
            self.ids.remove(old);
        }
        if name == "media" && namespace.is_none() && old_value.is_some() {
            self.arena.get_mut(node)?.stylesheet_media = None;
        }
        if old_value.is_some() {
            if name == "name" && namespace.is_none() {
                let revision = self.arena.name_attribute_revisions.entry(node).or_default();
                *revision = revision.saturating_add(1);
            }
            self.mutations.push(Mutation::Attribute {
                node,
                name: name.to_string(),
                namespace,
                old_value,
                new_value: None,
            });
        }
        Ok(())
    }

    /// Revision of unqualified name writes, including assignments of the same
    /// value. Browsing-context target names can diverge from this attribute.
    pub fn name_attribute_revision(&self, node: NodeId) -> Result<u64, DomError> {
        self.arena.get(node)?;
        Ok(self
            .arena
            .name_attribute_revisions
            .get(&node)
            .copied()
            .unwrap_or(0))
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
        let normalized = name.to_string();
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
            .arena
            .attribute_nodes
            .get(&(element, normalized.clone()))
            .copied()
        {
            return Ok(Some(attribute));
        }
        let attribute = self.create(NodeKind::Attribute {
            name: normalized.clone(),
            local_name: normalized.clone(),
            namespace: None,
            prefix: None,
            value,
            owner: Some(element),
        });
        self.arena
            .attribute_nodes
            .insert((element, normalized), attribute);
        Ok(Some(attribute))
    }

    pub fn set_attribute_node_value(
        &mut self,
        attribute: NodeId,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let value = value.into();
        let (name, namespace, owner, old_value) = match &self.arena.get(attribute)?.kind {
            NodeKind::Attribute {
                name,
                namespace,
                value,
                owner,
                ..
            } => (name.clone(), namespace.clone(), *owner, value.clone()),
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
                namespace,
                old_value: Some(old_value),
                new_value: Some(value),
            });
        }
        Ok(())
    }

    /// Normalize light-tree Text descendants using native live-Range updates.
    pub fn normalize(&mut self, root: NodeId) -> Result<(), DomError> {
        let mut pending = self
            .arena
            .get(root)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?.clone();
            let Some(parent) = record.parent else {
                continue;
            };
            let NodeKind::Text { data } = record.kind else {
                if !matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                    pending.extend(record.children.iter().rev().copied());
                }
                continue;
            };
            let mut length = utf16_len(&data);
            if length == 0 {
                self.remove(node)?;
                continue;
            }
            let index = self.child_index(parent, node)? as usize;
            let siblings = self.arena.get(parent)?.children[index + 1..]
                .iter()
                .copied()
                .take_while(|sibling| {
                    matches!(
                        self.arena.get(*sibling).map(|r| &r.kind),
                        Ok(NodeKind::Text { .. })
                    )
                })
                .collect::<Vec<_>>();
            let mut appended = String::new();
            for sibling in &siblings {
                if let NodeKind::Text { data } = &self.arena.get(*sibling)?.kind {
                    appended.push_str(data);
                }
            }
            self.replace_character_data(node, length, 0, appended)?;
            for sibling in &siblings {
                let index = self.child_index(parent, *sibling)?;
                let mut root_changes = Vec::new();
                for range in self.ranges.values_mut() {
                    for boundary in [&mut range.start, &mut range.end] {
                        if boundary.container == *sibling {
                            root_changes.push((boundary.container, node));
                            boundary.container = node;
                            boundary.offset = boundary.offset.saturating_add(length);
                        } else if boundary.container == parent && boundary.offset == index {
                            root_changes.push((boundary.container, node));
                            boundary.container = node;
                            boundary.offset = length;
                        }
                    }
                }
                for (old, new) in root_changes {
                    self.retain_native(new)?;
                    self.release_native(old)?;
                }
                if let NodeKind::Text { data } = &self.arena.get(*sibling)?.kind {
                    length = length.saturating_add(utf16_len(data));
                }
            }
            for sibling in siblings {
                self.remove(sibling)?;
            }
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
        self.arena.invalidate_stylesheet_ancestors(Some(node))?;
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

    fn replace_with_new_children(
        &mut self,
        node: NodeId,
        added_nodes: Vec<NodeId>,
    ) -> Result<(), DomError> {
        let removed_nodes = self
            .arena
            .get(node)?
            .children
            .iter()
            .copied()
            .filter(|child| {
                !matches!(
                    self.arena.get(*child).map(|record| &record.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            })
            .collect::<Vec<_>>();
        let cursor = self.mutation_cursor();
        for child in &removed_nodes {
            self.remove_child(node, *child)?;
        }
        for child in &added_nodes {
            self.append_child(node, *child)?;
        }
        self.mutations.suppress_tree_observers_since(cursor, node);
        if !added_nodes.is_empty() || !removed_nodes.is_empty() {
            self.mutations.push(Mutation::ChildrenReplaced {
                node,
                added_nodes,
                removed_nodes,
            });
        }
        Ok(())
    }

    pub fn set_text_content(
        &mut self,
        node: NodeId,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let value = value.into();
        match self.arena.get(node)?.kind {
            NodeKind::Text { .. }
            | NodeKind::CDataSection { .. }
            | NodeKind::ProcessingInstruction { .. }
            | NodeKind::Comment { .. } => return self.set_text(node, value),
            NodeKind::Attribute { .. } => return self.set_attribute_node_value(node, value),
            NodeKind::Document | NodeKind::DocumentType { .. } => return Ok(()),
            _ => {}
        }
        let children = if value.is_empty() {
            Vec::new()
        } else {
            vec![self.create_text(value)]
        };
        self.replace_with_new_children(node, children)
    }

    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> Result<NodeId, DomError> {
        let source = self.arena.get(node)?.clone();
        let attributes = self
            .arena
            .attribute_node_ids(node)
            .into_iter()
            .map(|attribute| self.arena.get(attribute).map(|record| record.kind.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut kind = source.kind;
        if let NodeKind::Attribute { owner, .. } = &mut kind {
            *owner = None;
        }
        let clone = self.create(kind);
        {
            let record = self.arena.get_mut(clone)?;
            record.document_metadata = source.document_metadata;
            record.attributes = source.attributes;
            record.value = source.value;
            record.sensitive_value = source.sensitive_value;
        }
        for attribute in attributes {
            if let NodeKind::Attribute {
                namespace,
                name,
                value,
                ..
            } = attribute
            {
                self.set_attribute_ns(clone, namespace, name, value)?;
            }
        }
        if let Some(content) = source.template_content {
            let cloned_content = self.clone_node(content, deep)?;
            self.retain_native(cloned_content)?;
            self.arena.get_mut(clone)?.template_content = Some(cloned_content);
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
        let mut siblings = MutationSiblings::default();
        if let Some(parent) = parent {
            let index = self.child_index(parent, node)?;
            siblings = self.mutation_siblings_at(parent, index)?;
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
            siblings,
            observer_suppressed: false,
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
    /// The parser may immediately insert the handle elsewhere. Preserve it
    /// with the removal journal's lease until the normal delivery checkpoint,
    /// without attempting reclamation during the tree-builder operation.
    pub(crate) fn parser_detach(&mut self, node: NodeId) -> Result<(), DomError> {
        let parent = self.arena.get(node)?.parent;
        if parent.is_none() {
            return Ok(());
        }
        let siblings =
            self.mutation_siblings_at(parent.unwrap(), self.child_index(parent.unwrap(), node)?)?;
        self.arena.detach(node)?;
        self.mark_disconnected(node)?;
        self.retain_native(node)?;
        self.mutations.push(Mutation::Removed {
            node,
            former_parent: parent,
            siblings,
            observer_suppressed: false,
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
            if !self.connected.contains(&node) {
                return Ok(());
            }
        }
        let previous = std::mem::replace(&mut self.focused, current);
        self.mutations.push(Mutation::Focus { previous, current });
        Ok(())
    }

    pub const fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Whether the focused element should match `:focus-visible`. Blurring
    /// preserves this state so a subsequent script focus inherits it.
    pub fn focus_visible(&self) -> bool {
        self.focused.is_some_and(|node| {
            !self.focus_ring_suppressed || self.focus_accepts_keyboard_input(node)
        })
    }

    fn focus_accepts_keyboard_input(&self, node: NodeId) -> bool {
        let Ok(record) = self.arena.get(node) else {
            return false;
        };
        if let NodeKind::Element { name } = &record.kind
            && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        {
            if name.local.as_ref() == "textarea" {
                return !record.attributes.contains_key("readonly");
            }
            if name.local.as_ref() == "input" {
                let kind = record
                    .attributes
                    .get("type")
                    .map(String::as_str)
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                return !record.attributes.contains_key("readonly")
                    && !matches!(
                        kind.as_str(),
                        "hidden"
                            | "button"
                            | "submit"
                            | "reset"
                            | "image"
                            | "checkbox"
                            | "radio"
                            | "file"
                            | "color"
                            | "range"
                    );
            }
        }
        let mut current = Some(node);
        while let Some(id) = current {
            let Ok(record) = self.arena.get(id) else {
                break;
            };
            if let Some(value) = record.attributes.get("contenteditable") {
                if value.is_empty()
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("plaintext-only")
                {
                    return true;
                }
                if value.eq_ignore_ascii_case("false") {
                    return false;
                }
            }
            current = record.parent;
        }
        false
    }

    /// Update native input state without dispatching synthetic focus events.
    /// Journal changes even when focus stays put so style consumers recascade.
    pub fn set_focus_visible(&mut self, visible: bool) {
        let suppressed = !visible;
        if self.focus_ring_suppressed == suppressed {
            return;
        }
        self.focus_ring_suppressed = suppressed;
        if self.focused.is_some() {
            self.mutations.push(Mutation::Focus {
                previous: self.focused,
                current: self.focused,
            });
        }
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

    /// Exact expanded-name match, independent of document case folding.
    pub fn is_element(&self, node: NodeId, namespace: &str, local_name: &str) -> bool {
        self.arena.get(node).is_ok_and(|node| matches!(&node.kind,
            NodeKind::Element { name } if name.ns.as_ref() == namespace && name.local.as_ref() == local_name))
    }

    /// Finds the first matching descendant in this tree, excluding shadow trees.
    pub fn get_element_by_id(&self, root: NodeId, id: &str) -> Result<Option<NodeId>, DomError> {
        let mut pending: Vec<_> = self
            .arena
            .get(root)?
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        if id.is_empty() {
            return Ok(None);
        }
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                continue;
            }
            if matches!(record.kind, NodeKind::Element { .. })
                && record.attributes.get("id").is_some_and(|value| value == id)
            {
                let namespaced = self
                    .arena
                    .existing_attribute_node(node, "id")
                    .and_then(|attr| self.arena.get(attr).ok())
                    .is_some_and(|attr| {
                        matches!(
                            &attr.kind,
                            NodeKind::Attribute {
                                namespace: Some(_),
                                ..
                            }
                        )
                    });
                if !namespaced {
                    return Ok(Some(node));
                }
            }
            pending.extend(record.children.iter().rev().copied());
        }
        Ok(None)
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
                let children = match record.template_content {
                    Some(content) => &self.arena.get(content)?.children,
                    None => &record.children,
                };
                for child in children {
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
        for node in std::mem::take(&mut self.mutations.discarded_removed_roots) {
            self.release_native(node)?;
        }
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
        self.mark_disconnected_with_focus(node, false)
    }

    fn mark_disconnected_with_focus(
        &mut self,
        node: NodeId,
        preserve_focus: bool,
    ) -> Result<(), DomError> {
        self.connected.remove(&node);
        if !preserve_focus && self.focused == Some(node) {
            self.focus(None)?;
        }
        if let Some(id) = self.arena.get(node)?.attributes.get("id")
            && self.ids.get(id) == Some(&node)
        {
            self.ids.remove(id);
        }
        for child in self.arena.get(node)?.children.clone() {
            self.mark_disconnected_with_focus(child, preserve_focus)?;
        }
        Ok(())
    }

    fn is_gc_root(&self, node: NodeId) -> bool {
        self.connected.contains(&node)
            || self
                .roots
                .get(&node)
                .is_some_and(|roots| roots.wrappers > 0 || roots.native > 0)
    }

    pub fn retention_parent(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.arena.get(node)?;
        Ok(match record.kind {
            NodeKind::Attribute { owner, .. } => owner,
            _ => record.parent,
        })
    }

    fn retention_children(&self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        let mut children = self.arena.get(node)?.children.clone();
        children.extend(self.arena.attribute_node_ids(node));
        Ok(children)
    }

    fn reclaim_if_unrooted(&mut self, node: NodeId) -> Result<(), DomError> {
        // A retained DOM node exposes both its ancestors and descendants.
        // Collection must not detach a retained child or erase an unwrapped
        // child from a retained parent. Reclaim the whole detached tree only
        // when none of its nodes has a wrapper or native lease.
        let mut root = node;
        loop {
            if self.is_gc_root(root) {
                return Ok(());
            }
            match self.retention_parent(root)? {
                Some(parent) => root = parent,
                None => break,
            }
        }
        let mut pending = self.retention_children(root)?;
        while let Some(child) = pending.pop() {
            if self.is_gc_root(child) {
                return Ok(());
            }
            pending.extend(self.retention_children(child)?);
        }
        self.reclaim_unrooted_tree(root)
    }

    fn reclaim_unrooted_tree(&mut self, node: NodeId) -> Result<(), DomError> {
        for child in self.retention_children(node)? {
            self.reclaim_unrooted_tree(child)?;
        }
        let sheets = self
            .arena
            .get(node)?
            .adopted_stylesheets
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let template_content = self.arena.get(node)?.template_content;
        let _ = self.arena.remove(node)?;
        if let Some(content) = template_content {
            self.release_native(content)?;
        }
        for sheet in sheets {
            self.release_native(sheet)?;
        }
        self.roots.remove(&node);
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
        .replace('\u{a0}', "&nbsp;")
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
    fn document_metadata_survives_import_clone_and_replacement() {
        let mut dom = html::parse("<html><body></body></html>")
            .into_runtime()
            .unwrap();
        let document = dom.document();
        let metadata = dom
            .arena()
            .get(document)
            .unwrap()
            .document_metadata
            .as_ref()
            .unwrap();
        assert!(metadata.is_html && metadata.quirks);
        assert_eq!(metadata.content_type, "text/html");
        let clone = dom.clone_node(document, true).unwrap();
        assert_eq!(
            dom.arena().get(clone).unwrap().document_metadata,
            dom.arena().get(document).unwrap().document_metadata
        );
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(dom.arena(), clone).unwrap();
        assert_eq!(
            other.arena().get(imported).unwrap().document_metadata,
            dom.arena().get(document).unwrap().document_metadata
        );
        dom.replace_document_contents("<!doctype html><title>standards</title>")
            .unwrap();
        assert!(
            !dom.arena()
                .get(document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .quirks
        );
        assert!(
            dom.arena()
                .get(clone)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .quirks
        );
    }

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
    fn parsed_templates_preserve_inert_contents_across_clone_import_and_collection() {
        let mut dom = html::parse(
            "<template id='t'><b id='inert'>kept</b><template><i>nested</i></template></template>",
        )
        .into_runtime()
        .unwrap();
        let template = dom.find_by_id("t").unwrap();
        let content = dom.template_content(template).unwrap();
        assert!(dom.arena().get(template).unwrap().children.is_empty());
        assert_eq!(dom.find_by_id("inert"), None);
        assert!(!dom.is_connected(content).unwrap());
        assert_eq!(dom.arena().get(content).unwrap().parent, None);
        dom.retain_wrapper(content).unwrap();
        dom.release_wrapper(content).unwrap();
        assert_eq!(dom.arena().get(content).unwrap().children.len(), 2);
        let clone = dom.clone_node(template, true).unwrap();
        assert_eq!(
            dom.serialize(clone).unwrap(),
            dom.serialize(template).unwrap()
        );
        assert_ne!(dom.template_content(clone).unwrap(), content);
        let shallow = dom.clone_node(template, false).unwrap();
        let empty = dom.template_content(shallow).unwrap();
        assert!(dom.arena().get(empty).unwrap().children.is_empty());
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(dom.arena(), template).unwrap();
        assert_eq!(
            other.serialize(imported).unwrap(),
            dom.serialize(template).unwrap()
        );
        dom.retain_wrapper(template).unwrap();
        dom.remove_child(dom.arena().get(template).unwrap().parent.unwrap(), template)
            .unwrap();
        dom.release_wrapper(template).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert!(dom.arena().get(content).is_err());
        assert!(dom.serialize(clone).unwrap().contains("nested"));
    }

    #[test]
    fn cssom_stylesheet_source_is_separate_from_dom_text() {
        let mut dom = html::parse("<style id='sheet'>div { color: red; }</style>")
            .into_runtime()
            .unwrap();
        let sheet = dom.find_by_id("sheet").unwrap();
        let original = dom.arena().text_content(sheet).unwrap();
        dom.drain_mutations();
        dom.set_stylesheet_text(sheet, "div { color: blue; }".into())
            .unwrap();
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "div { color: blue; }"
        );
        assert_eq!(dom.arena().text_content(sheet).unwrap(), original);
        assert!(
            matches!(dom.drain_mutations().as_slice(), [Mutation::StyleSheets { node }] if *node == sheet)
        );
        let clone = dom.clone_node(sheet, true).unwrap();
        assert_eq!(dom.arena().stylesheet_text(clone).unwrap(), original);
        let text = dom.arena().get(sheet).unwrap().children[0];
        dom.set_text(text, "div { color: green; }").unwrap();
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "div { color: green; }"
        );
        dom.set_stylesheet_text(sheet, "div { color: blue; }".into())
            .unwrap();
        let extra = dom.create(NodeKind::Text {
            data: "p { color: black; }".into(),
        });
        dom.append_child(sheet, extra).unwrap();
        assert!(
            dom.arena()
                .get(sheet)
                .unwrap()
                .stylesheet_override
                .is_none()
        );
        assert!(
            dom.arena()
                .stylesheet_text(sheet)
                .unwrap()
                .contains("color: black")
        );
    }

    #[test]
    fn stylesheet_revisions_track_dom_changes_but_not_cssom_publication() {
        let mut dom =
            html::parse("<div id='host'><style id='sheet'>a { color: red; }</style></div>")
                .into_runtime()
                .unwrap();
        let sheet = dom.find_by_id("sheet").unwrap();
        let host = dom.find_by_id("host").unwrap();
        let before = dom.arena().stylesheet_revision(sheet).unwrap();
        dom.set_stylesheet_text(sheet, "a { color: blue; }".into())
            .unwrap();
        assert_eq!(dom.arena().stylesheet_revision(sheet).unwrap(), before);
        let text = dom.create(NodeKind::Text {
            data: "b { color: green; }".into(),
        });
        dom.append_child(sheet, text).unwrap();
        let changed = dom.arena().stylesheet_revision(sheet).unwrap();
        assert!(changed > before);
        assert!(
            dom.arena()
                .get(sheet)
                .unwrap()
                .stylesheet_override
                .is_none()
        );
        let sibling = dom.create_element("aside");
        dom.append_child(host, sibling).unwrap();
        assert_eq!(dom.arena().stylesheet_revision(sheet).unwrap(), changed);
        let parent = dom.arena().get(host).unwrap().parent.unwrap();
        dom.remove_child(parent, host).unwrap();
        assert!(dom.arena().stylesheet_revision(sheet).unwrap() > changed);
    }

    #[test]
    fn adopted_sheets_are_native_roots_and_not_dom_children() {
        let mut dom = DomRuntime::new();
        let owner = dom.create(NodeKind::Document);
        dom.retain_wrapper(owner).unwrap();
        let sheet = dom.create_element("style");
        dom.retain_wrapper(sheet).unwrap();
        dom.set_adopted_stylesheets(owner, vec![sheet, sheet])
            .unwrap();
        assert!(dom.arena().get(owner).unwrap().children.is_empty());
        assert_eq!(dom.serialize(owner).unwrap(), "");
        dom.release_wrapper(sheet).unwrap();
        assert!(dom.arena().get(sheet).is_ok());
        dom.set_adopted_stylesheets(owner, vec![sheet]).unwrap();
        assert!(dom.arena().get(sheet).is_ok());
        let mutations = dom.drain_mutations();
        assert_eq!(
            mutations
                .iter()
                .filter(|m| matches!(m, Mutation::StyleSheets { .. }))
                .count(),
            2
        );
        assert!(
            !mutations
                .iter()
                .any(|m| matches!(m, Mutation::Attribute { .. }))
        );
        dom.release_wrapper(owner).unwrap();
        assert!(matches!(
            dom.arena().get(owner),
            Err(DomError::StaleNode(_))
        ));
        assert!(matches!(
            dom.arena().get(sheet),
            Err(DomError::StaleNode(_))
        ));
    }

    #[test]
    fn text_replacement_groups_observers_without_hiding_structural_changes() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let a = dom.create_text("a");
        let b = dom.create_element("b");
        dom.append_child(root, a).unwrap();
        dom.append_child(root, b).unwrap();
        let cursor = dom.mutation_cursor();
        dom.set_text_content(root, "new").unwrap();
        let added = dom.children(root).unwrap()[0];
        let changes = dom.mutations_since(cursor).mutations;
        assert_eq!(
            changes
                .iter()
                .filter(|m| matches!(
                    m,
                    Mutation::Removed {
                        observer_suppressed: true,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert_eq!(
            changes
                .iter()
                .filter(|m| matches!(
                    m,
                    Mutation::Inserted {
                        observer_suppressed: true,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(
            matches!(changes.last(), Some(Mutation::ChildrenReplaced { node, added_nodes, removed_nodes }) if *node == root && added_nodes == &[added] && removed_nodes == &[a,b])
        );
        dom.set_text_content(root, "").unwrap();
        let cursor = dom.mutation_cursor();
        dom.set_text_content(root, "").unwrap();
        assert!(dom.mutations_since(cursor).mutations.is_empty());
        let document = dom.document();
        let cursor = dom.mutation_cursor();
        dom.set_text_content(document, "ignored").unwrap();
        assert!(dom.mutations_since(cursor).mutations.is_empty());
        let comment = dom.create(NodeKind::Comment {
            data: "before".into(),
        });
        dom.set_text_content(comment, "after").unwrap();
        assert!(
            matches!(&dom.arena().get(comment).unwrap().kind, NodeKind::Comment { data } if data == "after")
        );
        assert!(dom.children(comment).unwrap().is_empty());
    }

    #[test]
    fn normalization_moves_live_range_boundaries_and_preserves_record_order() {
        let mut dom = DomRuntime::new();
        let root = dom.create_fragment();
        let empty = dom.create_text("");
        let first = dom.create_text("ab");
        let middle = dom.create_text("😀");
        let last = dom.create_text("cd");
        let comment = dom.create(NodeKind::Comment {
            data: "barrier".into(),
        });
        for node in [empty, first, middle, last, comment] {
            dom.append_child(root, node).unwrap();
        }
        let inside = dom
            .create_live_range(
                BoundaryPoint {
                    container: middle,
                    offset: 1,
                },
                BoundaryPoint {
                    container: last,
                    offset: 1,
                },
            )
            .unwrap();
        let between = dom
            .create_live_range(
                BoundaryPoint {
                    container: root,
                    offset: 2,
                },
                BoundaryPoint {
                    container: root,
                    offset: 4,
                },
            )
            .unwrap();
        let cursor = dom.mutation_cursor();
        dom.normalize(root).unwrap();
        assert_eq!(dom.children(root).unwrap(), vec![first, comment]);
        assert_eq!(dom.arena().text_content(first).unwrap(), "ab😀cd");
        let range = dom.live_range(inside).unwrap();
        assert_eq!(
            range.start,
            BoundaryPoint {
                container: first,
                offset: 3
            }
        );
        assert_eq!(
            range.end,
            BoundaryPoint {
                container: first,
                offset: 5
            }
        );
        let range = dom.live_range(between).unwrap();
        assert_eq!(
            range.start,
            BoundaryPoint {
                container: first,
                offset: 2
            }
        );
        assert_eq!(
            range.end,
            BoundaryPoint {
                container: root,
                offset: 1
            }
        );
        let changes = dom.mutations_since(cursor).mutations;
        assert!(
            matches!(changes.as_slice(), [Mutation::Removed { node: a, .. }, Mutation::CharacterData { node: b, .. }, Mutation::Removed { node: c, .. }, Mutation::Removed { node: d, .. }] if *a == empty && *b == first && *c == middle && *d == last)
        );
    }

    #[test]
    fn focus_is_cleared_on_removal_and_ordinary_moves_and_ignores_detached_nodes() {
        let mut dom = html::parse("<body><div><input></div><section></section></body>")
            .into_runtime()
            .unwrap();
        let root = dom.document();
        let input = dom.query_selector(root, "input").unwrap().unwrap();
        let parent = dom.query_selector(root, "div").unwrap().unwrap();
        let other = dom.query_selector(root, "section").unwrap().unwrap();
        dom.focus(Some(input)).unwrap();
        dom.remove(parent).unwrap();
        assert_eq!(dom.focused(), None);
        dom.focus(Some(input)).unwrap();
        assert_eq!(dom.focused(), None);
        dom.append_child(other, parent).unwrap();
        dom.focus(Some(input)).unwrap();
        assert_eq!(dom.focused(), Some(input));
        dom.append_child(other, input).unwrap();
        assert_eq!(dom.focused(), None);
    }

    #[test]
    fn focus_visibility_survives_script_transfers_and_invalidates_existing_focus() {
        let mut dom = html::parse("<body><button id='a'></button><button id='b'></button></body>")
            .into_runtime()
            .unwrap();
        let a = dom.query_selector(dom.document(), "#a").unwrap().unwrap();
        let b = dom.query_selector(dom.document(), "#b").unwrap().unwrap();
        dom.focus(Some(a)).unwrap();
        assert!(dom.focus_visible());
        dom.set_focus_visible(false);
        assert!(!dom.focus_visible());
        dom.focus(None).unwrap();
        dom.focus(Some(b)).unwrap();
        assert!(!dom.focus_visible());
        let cursor = dom.mutation_cursor();
        dom.set_focus_visible(true);
        assert!(dom.focus_visible());
        assert_eq!(dom.focused(), Some(b));
        assert!(matches!(dom.mutations_since(cursor).mutations.as_slice(),
            [Mutation::Focus { previous, current }] if *previous == Some(b) && *current == Some(b)));
        dom.focus(None).unwrap();
        assert!(!dom.focus_visible());
        dom.focus(Some(a)).unwrap();
        assert!(dom.focus_visible());
    }

    #[test]
    fn text_input_focus_exception_does_not_change_script_focus_modality() {
        let mut dom = html::parse("<body><input id='input'><button id='button'></button></body>")
            .into_runtime()
            .unwrap();
        let input = dom
            .query_selector(dom.document(), "#input")
            .unwrap()
            .unwrap();
        let button = dom
            .query_selector(dom.document(), "#button")
            .unwrap()
            .unwrap();
        dom.set_focus_visible(false);
        dom.focus(Some(input)).unwrap();
        assert!(dom.focus_visible());
        dom.focus(Some(button)).unwrap();
        assert!(!dom.focus_visible());
        dom.set_attribute(input, "type", "button").unwrap();
        dom.focus(Some(input)).unwrap();
        assert!(!dom.focus_visible());
        dom.set_attribute(input, "type", "text").unwrap();
        assert!(dom.focus_visible());
    }

    #[test]
    fn state_preserving_moves_keep_focus_without_focus_mutations() {
        let mut dom = html::parse("<body><div><input></div><section></section></body>")
            .into_runtime()
            .unwrap();
        let root = dom.document();
        let input = dom.query_selector(root, "input").unwrap().unwrap();
        let parent = dom.query_selector(root, "div").unwrap().unwrap();
        let other = dom.query_selector(root, "section").unwrap().unwrap();
        dom.focus(Some(input)).unwrap();
        let cursor = dom.mutation_cursor();
        dom.move_before(other, parent, None).unwrap();
        dom.move_before(parent, input, None).unwrap();
        dom.move_before(other, input, Some(parent)).unwrap();
        assert_eq!(dom.focused(), Some(input));
        assert!(
            !dom.mutations_since(cursor)
                .mutations
                .iter()
                .any(|mutation| matches!(mutation, Mutation::Focus { .. }))
        );
        dom.append_child(parent, input).unwrap();
        assert_eq!(dom.focused(), None);
    }

    #[test]
    fn mutation_siblings_preserve_insertion_removal_and_move_positions() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_fragment();
        let a = dom.create_element("a");
        let b = dom.create_element("b");
        let c = dom.create_element("c");
        dom.append_child(parent, a).unwrap();
        let cursor = dom.mutation_cursor();
        dom.append_child(parent, b).unwrap();
        dom.insert_before(parent, c, Some(b)).unwrap();
        dom.remove(a).unwrap();
        dom.insert_before(parent, b, Some(c)).unwrap();
        let snapshots = dom
            .mutations_since(cursor)
            .mutations
            .into_iter()
            .map(|mutation| match mutation {
                Mutation::Inserted { node, siblings, .. } => {
                    (true, node, siblings.previous, siblings.next)
                }
                Mutation::Removed { node, siblings, .. } => {
                    (false, node, siblings.previous, siblings.next)
                }
                _ => panic!("unexpected mutation"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            snapshots,
            vec![
                (true, b, Some(a), None),
                (true, c, Some(a), Some(b)),
                (false, a, None, Some(c)),
                (false, b, Some(c), None),
                (true, b, None, Some(c)),
            ]
        );
    }

    #[test]
    fn attribute_mutations_snapshot_namespaces_before_detaching() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("div");
        let cursor = dom.mutation_cursor();
        dom.set_attribute_ns(node, Some("urn:test".into()), "p:item", "one")
            .unwrap();
        let attribute = dom.attribute_node(node, "p:item").unwrap().unwrap();
        dom.set_attribute_node_value(attribute, "two").unwrap();
        dom.remove_attribute(node, "p:item").unwrap();
        dom.set_attribute(node, "p:item", "plain").unwrap();
        let attributes = dom
            .mutations_since(cursor)
            .mutations
            .into_iter()
            .filter_map(|mutation| {
                if let Mutation::Attribute {
                    name,
                    namespace,
                    old_value,
                    new_value,
                    ..
                } = mutation
                {
                    Some((name, namespace, old_value, new_value))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            attributes,
            vec![
                (
                    "p:item".into(),
                    Some("urn:test".into()),
                    None,
                    Some("one".into())
                ),
                (
                    "p:item".into(),
                    Some("urn:test".into()),
                    Some("one".into()),
                    Some("two".into())
                ),
                (
                    "p:item".into(),
                    Some("urn:test".into()),
                    Some("two".into()),
                    None
                ),
                ("p:item".into(), None, None, Some("plain".into())),
            ]
        );
    }

    #[test]
    fn discarded_mutation_entries_release_their_native_roots_at_checkpoint() {
        let mut dom = DomRuntime::new();
        let removed = dom.create_element("section");
        dom.append_child(dom.document(), removed).unwrap();
        dom.remove(removed).unwrap();
        for _ in 0..MutationJournal::MAX_ENTRIES {
            dom.focus(None).unwrap();
        }
        assert!(dom.arena().get(removed).is_ok());
        assert_eq!(dom.mutations.discarded_removed_roots, vec![removed]);
        let batch = dom.drain_mutations();
        dom.release_mutation_roots(&batch).unwrap();
        assert!(matches!(
            dom.arena().get(removed),
            Err(DomError::StaleNode(_))
        ));

        let drained = dom.create_element("section");
        dom.append_child(dom.document(), drained).unwrap();
        dom.remove(drained).unwrap();
        let batch = dom.drain_mutations();
        for _ in 0..MutationJournal::MAX_ENTRIES {
            dom.focus(None).unwrap();
        }
        assert!(dom.mutations.discarded_removed_roots.is_empty());
        assert!(dom.arena().get(drained).is_ok());
        dom.release_mutation_roots(&batch).unwrap();
        assert!(matches!(
            dom.arena().get(drained),
            Err(DomError::StaleNode(_))
        ));
        let batch = dom.drain_mutations();
        dom.release_mutation_roots(&batch).unwrap();
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
    fn mutation_cursors_preserve_order_across_journal_wraparound() {
        let mut journal = MutationJournal::default();
        for index in 0..MutationJournal::MAX_ENTRIES + 7 {
            journal.push(Mutation::Inserted {
                node: NodeId {
                    slot: index as u32,
                    generation: 0,
                },
                parent: None,
                siblings: MutationSiblings::default(),
                observer_suppressed: false,
            });
        }
        for cursor in [
            0,
            6,
            7,
            8,
            100,
            MutationJournal::MAX_ENTRIES as u64 + 6,
            MutationJournal::MAX_ENTRIES as u64 + 7,
            u64::MAX,
        ] {
            let batch = journal.read_since(MutationCursor(cursor));
            let expected = journal
                .entries
                .iter()
                .filter(|(sequence, _)| *sequence > cursor)
                .map(|(_, mutation)| mutation.clone())
                .collect::<Vec<_>>();
            assert_eq!(batch.mutations, expected, "cursor {cursor}");
            assert_eq!(batch.truncated, cursor < 7, "cursor {cursor}");
            assert_eq!(
                batch.next,
                MutationCursor(MutationJournal::MAX_ENTRIES as u64 + 7)
            );
        }
        journal.clear();
        assert!(journal.read_since(MutationCursor(0)).mutations.is_empty());
        assert!(!journal.read_since(MutationCursor(0)).truncated);
    }

    #[test]
    fn retained_attribute_keeps_its_detached_owner_and_import_preserves_namespaces() {
        let mut dom = DomRuntime::new();
        let owner = dom.create_element("owner");
        dom.set_attribute_ns(owner, Some("urn:test".into()), "p:FLAG", "value")
            .unwrap();
        let attribute = dom.attribute_node(owner, "p:FLAG").unwrap().unwrap();
        dom.retain_wrapper(attribute).unwrap();
        dom.retain_wrapper(owner).unwrap();
        dom.release_wrapper(owner).unwrap();
        assert!(dom.arena().get(owner).is_ok());
        let cloned_owner = dom.clone_node(owner, true).unwrap();
        let cloned_attribute = dom.attribute_node(cloned_owner, "p:FLAG").unwrap().unwrap();
        assert_ne!(cloned_attribute, attribute);
        let copied_attribute = dom.clone_node(attribute, true).unwrap();
        assert!(matches!(
            dom.arena().get(copied_attribute).unwrap().kind,
            NodeKind::Attribute { owner: None, .. }
        ));
        let (snapshot, document) = dom.snapshot();
        let mut restored = DomRuntime::from_parsed(snapshot.clone(), document).unwrap();
        assert_eq!(
            restored.attribute_node(owner, "p:FLAG").unwrap(),
            Some(attribute)
        );
        let mut target = DomRuntime::new();
        let imported = target.import_subtree(&snapshot, owner).unwrap();
        let imported_attribute = target.attribute_node(imported, "p:FLAG").unwrap().unwrap();
        assert!(
            matches!(&target.arena().get(imported_attribute).unwrap().kind,
            NodeKind::Attribute { owner: Some(element), namespace: Some(namespace), prefix: Some(prefix), local_name, value, .. }
                if *element == imported && namespace == "urn:test" && prefix == "p" && local_name == "FLAG" && value == "value")
        );
        dom.release_wrapper(attribute).unwrap();
        assert!(dom.arena().get(owner).is_err());
        assert!(dom.arena().get(attribute).is_err());
        assert!(
            dom.arena()
                .existing_attribute_node(owner, "p:FLAG")
                .is_none()
        );
        assert!(dom.arena().get(cloned_owner).is_ok());
    }

    #[test]
    fn connected_attribute_cache_survives_wrapper_collection() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("root");
        dom.append_child(dom.document(), element).unwrap();
        dom.set_attribute_ns(element, Some("urn:test".into()), "p:x", "value")
            .unwrap();
        let attribute = dom.attribute_node(element, "p:x").unwrap().unwrap();
        dom.retain_wrapper(attribute).unwrap();
        dom.release_wrapper(attribute).unwrap();
        assert!(dom.arena().get(attribute).is_ok());
        assert_eq!(dom.attribute_node(element, "p:x").unwrap(), Some(attribute));
    }

    #[test]
    fn attribute_nodes_preserve_literal_names_case_and_namespace_updates() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("root");
        dom.set_attribute(element, "P:FLAG", "plain").unwrap();
        let literal = dom.attribute_node(element, "P:FLAG").unwrap().unwrap();
        assert!(dom.attribute_node(element, "p:flag").unwrap().is_none());
        assert!(matches!(&dom.arena().get(literal).unwrap().kind,
            NodeKind::Attribute { name, local_name, prefix: None, namespace: None, .. }
                if name == "P:FLAG" && local_name == "P:FLAG"));
        dom.set_attribute_ns(element, Some("urn:test".into()), "n:Value", "first")
            .unwrap();
        let namespaced = dom.attribute_node(element, "n:Value").unwrap().unwrap();
        dom.set_attribute(element, "n:Value", "second").unwrap();
        assert_eq!(
            dom.attribute_node(element, "n:Value").unwrap(),
            Some(namespaced)
        );
        assert!(matches!(&dom.arena().get(namespaced).unwrap().kind,
            NodeKind::Attribute { local_name, prefix: Some(prefix), namespace: Some(namespace), value, .. }
                if local_name == "Value" && prefix == "n" && namespace == "urn:test" && value == "second"));
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
            Mutation::Removed { node, former_parent: Some(parent), .. }
                if *node == child && *parent == first
        )));
        assert!(mutations.iter().any(|mutation| matches!(
            mutation,
            Mutation::Inserted { node, parent: Some(parent), .. }
                if *node == child && *parent == second
        )));
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(dom.parent(child).unwrap(), Some(second));
    }

    #[test]
    fn retained_detached_descendant_keeps_its_parent_and_siblings() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("section");
        let child = dom.create_element("button");
        let sibling = dom.create_text("label");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(parent, child).unwrap();
        dom.append_child(parent, sibling).unwrap();
        dom.retain_wrapper(child).unwrap();
        dom.remove(parent).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(dom.parent(child).unwrap(), Some(parent));
        assert_eq!(dom.children(parent).unwrap(), &[child, sibling]);
        assert_eq!(dom.arena().text_content(parent).unwrap(), "label");
        dom.release_wrapper(child).unwrap();
        for node in [parent, child, sibling] {
            assert!(matches!(dom.arena().get(node), Err(DomError::StaleNode(_))));
        }
    }

    #[test]
    fn retained_detached_parent_keeps_children_after_wrapper_collection() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("section");
        let child = dom.create_element("button");
        let sibling = dom.create_text("label");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(parent, child).unwrap();
        dom.append_child(parent, sibling).unwrap();
        dom.retain_wrapper(parent).unwrap();
        dom.retain_wrapper(child).unwrap();
        dom.remove(parent).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.release_wrapper(child).unwrap();
        assert_eq!(dom.parent(child).unwrap(), Some(parent));
        assert_eq!(dom.children(parent).unwrap(), &[child, sibling]);
        dom.release_wrapper(parent).unwrap();
        for node in [parent, child, sibling] {
            assert!(matches!(dom.arena().get(node), Err(DomError::StaleNode(_))));
        }
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
    fn serialization_escapes_nonbreaking_spaces_without_changing_dom_text() {
        let parsed = html::parse("<p title='A&nbsp;B'>A&nbsp; &amp; B</p>");
        let dom = parsed.into_runtime().unwrap();
        let paragraph = dom.query_selector(dom.document(), "p").unwrap().unwrap();
        assert_eq!(dom.arena().text_content(paragraph).unwrap(), "A\u{a0} & B");
        assert_eq!(
            dom.serialize(paragraph).unwrap(),
            "<p title=\"A&nbsp;B\">A&nbsp; &amp; B</p>"
        );
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
    fn child_replacement_updates_ranges_before_inserting_at_the_vacated_slot() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("div");
        let child = dom.create_text("old");
        let replacement = dom.create_element("span");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(parent, child).unwrap();
        dom.append_child(parent, replacement).unwrap();
        let point = BoundaryPoint {
            container: parent,
            offset: 1,
        };
        let range = dom.create_live_range(point, point).unwrap();
        dom.replace_child(parent, replacement, child).unwrap();
        assert_eq!(dom.children(parent).unwrap(), &[replacement]);
        assert_eq!(dom.live_range(range).unwrap().start.offset, 0);
        assert_eq!(dom.live_range(range).unwrap().end.offset, 0);
        let inside = BoundaryPoint {
            container: replacement,
            offset: 0,
        };
        let nested = dom.create_live_range(inside, inside).unwrap();
        dom.replace_child(parent, replacement, replacement).unwrap();
        assert_eq!(dom.children(parent).unwrap(), &[replacement]);
        assert_eq!(
            dom.live_range(nested).unwrap().start,
            BoundaryPoint {
                container: parent,
                offset: 0
            }
        );
        assert!(matches!(
            dom.replace_child(parent, parent, replacement),
            Err(DomError::SelfParent)
        ));
        assert_eq!(dom.children(parent).unwrap(), &[replacement]);
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

    #[test]
    fn inserting_a_child_before_itself_is_a_true_no_op() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("div");
        let child = dom.create_element("span");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(parent, child).unwrap();
        let cursor = dom.mutation_cursor();

        dom.insert_before(parent, child, Some(child)).unwrap();

        assert_eq!(dom.children(parent).unwrap(), &[child]);
        assert_eq!(dom.parent(child).unwrap(), Some(parent));
        assert!(dom.mutations_since(cursor).mutations.is_empty());
    }
}
