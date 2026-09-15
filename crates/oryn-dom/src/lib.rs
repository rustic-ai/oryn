//! Rust-owned live DOM primitives.
//!
//! JavaScript wrappers retain generation-checked [`NodeId`] values, never
//! pointers into the arena. Mutations are journaled at their source so style,
//! semantic projection, observers, and tracing share one authoritative tree.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use markup5ever::{LocalName, Namespace, Prefix, QualName};
use serde::{Deserialize, Serialize};

mod attributes;
pub mod directionality;
pub mod dom_string;
use attributes::AttributeKey;
pub use attributes::{AttributeList, ContentAttribute};
mod form_controls;
mod numeric_controls;
mod pattern_controls;
mod text_controls;
pub use numeric_controls::NumericValidity;
pub use pattern_controls::PatternMatcher;
pub mod html;
mod identity;
pub use identity::{ArenaId, DocumentId, NodeRef};
pub mod opaque_ranges;
mod option_controls;
pub use opaque_ranges::{OpaqueRange, OpaqueRangeError, OpaqueRangeId, OpaqueRangeStore};
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
        /// Shared immutable record, also held by the owning element. Mutations
        /// replace this record centrally; detached Attrs retain their own data.
        data: Arc<ContentAttribute>,
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
    pub url: Option<String>,
    pub encoding: String,
    pub content_type: String,
    pub is_html: bool,
    pub quirks: bool,
}

impl Default for DocumentMetadata {
    fn default() -> Self {
        Self {
            content_type: "application/xml".into(),
            url: None,
            encoding: "UTF-8".into(),
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
            ..Self::default()
        }
    }
}

/// Selected bytes are native state, never an authored content attribute or a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedFile {
    pub name: String,
    pub media_type: String,
    pub bytes: Arc<[u8]>,
    pub last_modified: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    /// Native node-document identity, independent of parentage and realms.
    pub owner_document: Option<NodeId>,
    pub document_metadata: Option<Box<DocumentMetadata>>,
    /// Document's associated inert template document. Inert documents point
    /// to themselves, so nested templates reuse that document rather than
    /// allocating another owner. This is a traced edge, not a native lease.
    pub associated_inert_document: Option<NodeId>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: AttributeList,
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
    /// Host-including ancestry for template fragments, never parentNode or
    /// connectivity ancestry. Adoption of the fragment preserves this edge.
    pub template_host: Option<NodeId>,
    pub value: Option<String>,
    /// Script/user assignment differs from native value sanitization.
    pub dirty_value: bool,
    /// Whether the current value was last changed by native user editing.
    pub user_edited_value: bool,
    pub pending_edit_value: Option<String>,
    pub selected_files: Arc<Vec<SelectedFile>>,
    /// Last selected image-button coordinate; not an authored attribute.
    pub image_coordinates: (i32, i32),
    /// Decoded resource dimensions, separate from authored width/height.
    pub image_dimensions: Option<(u32, u32)>,
    /// None means the parser-provided checked attribute is still authoritative.
    /// Radio-group updates may supply an override without making it dirty.
    pub checkedness: Option<bool>,
    pub dirty_checkedness: bool,
    /// Option selectedness is independent of its selected content attribute.
    pub selectedness: Option<bool>,
    pub dirty_selectedness: bool,
    pub custom_validity_message: String,
    pub indeterminate: bool,
    pub sensitive_value: bool,
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    node: Option<Node>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomError {
    #[error("pattern validation failed: {0}")]
    PatternEvaluation(&'static str),
    #[error("node belongs to arena {actual:?}, not {expected:?}")]
    WrongArena { expected: ArenaId, actual: ArenaId },
    #[error("operation requires a Document node, got {0:?}")]
    NotADocument(NodeId),
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
    #[error("attribute is already owned by another element")]
    InUseAttribute,
    #[error("attribute does not belong to this element")]
    AttributeNotFound,
    #[error("attributes cannot participate in DOM child-tree edges")]
    AttributeTreeEdge,
    #[error("Document nodes cannot participate as DOM children")]
    DocumentTreeEdge,
    #[error("invalid boundary offset {offset} for node {node:?} (maximum {maximum})")]
    InvalidBoundaryOffset {
        node: NodeId,
        offset: u32,
        maximum: u32,
    },
    #[error("unknown live range {0:?}")]
    UnknownRange(LiveRangeId),
    #[error(transparent)]
    OpaqueRange(#[from] OpaqueRangeError),
}

#[derive(Debug)]
pub struct DomArena {
    id: ArenaId,
    allocation_document: Option<NodeId>,
    slots: Vec<Slot>,
    free: Vec<u32>,
    attribute_nodes: BTreeMap<(NodeId, AttributeKey), NodeId>,
    stylesheet_revisions: BTreeMap<NodeId, u64>,
    name_attribute_revisions: BTreeMap<NodeId, u64>,
    /// Host associations are projection/lifetime edges, never DOM tree edges.
    hosted_documents: BTreeMap<NodeId, NodeId>,
    retired_documents: BTreeSet<NodeId>,
}

impl Default for DomArena {
    fn default() -> Self {
        Self {
            id: ArenaId::fresh(),
            allocation_document: None,
            slots: Vec::new(),
            free: Vec::new(),
            attribute_nodes: BTreeMap::new(),
            stylesheet_revisions: BTreeMap::new(),
            name_attribute_revisions: BTreeMap::new(),
            hosted_documents: BTreeMap::new(),
            retired_documents: BTreeSet::new(),
        }
    }
}

impl Clone for DomArena {
    fn clone(&self) -> Self {
        // A snapshot can evolve independently. Local NodeIds are preserved for
        // projection, but it must never accept the source arena's live handles.
        Self {
            id: ArenaId::fresh(),
            allocation_document: self.allocation_document,
            slots: self.slots.clone(),
            free: self.free.clone(),
            attribute_nodes: self.attribute_nodes.clone(),
            stylesheet_revisions: self.stylesheet_revisions.clone(),
            name_attribute_revisions: self.name_attribute_revisions.clone(),
            hosted_documents: self.hosted_documents.clone(),
            retired_documents: self.retired_documents.clone(),
        }
    }
}

impl DomArena {
    /// Match one selector chain against this arena without cloning DOM state.
    /// Callers provide the active focused node for the `:focus` pseudo-class.
    pub fn matches_selector_chain(
        &self,
        node: NodeId,
        selector: &str,
        focused: Option<NodeId>,
    ) -> Result<bool, DomError> {
        if let Some((left, combinator, right)) = split_last_selector_combinator(selector) {
            if !self.matches_selector_chain(node, right, focused)? {
                return Ok(false);
            }
            let record = self.get(node)?;
            return match combinator {
                '>' => record
                    .parent
                    .map(|parent| self.matches_selector_chain(parent, left, focused))
                    .transpose()
                    .map(|value| value.unwrap_or(false)),
                '+' | '~' => {
                    let Some(parent) = record.parent else {
                        return Ok(false);
                    };
                    let siblings = &self.get(parent)?.children;
                    let Some(index) = siblings.iter().position(|candidate| *candidate == node)
                    else {
                        return Ok(false);
                    };
                    let matching = siblings[..index]
                        .iter()
                        .rev()
                        .filter(|candidate| {
                            self.get(**candidate)
                                .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
                        })
                        .take(if combinator == '+' { 1 } else { usize::MAX })
                        .any(|candidate| {
                            self.matches_selector_chain(*candidate, left, focused)
                                .unwrap_or(false)
                        });
                    Ok(matching)
                }
                _ => Ok(false),
            };
        }
        let compounds = selector_compounds(selector);
        let Some(last) = compounds.last() else {
            return Ok(false);
        };
        let focus_subject = last.strip_suffix(":focus");
        if focus_subject.is_some() && focused != Some(node) {
            return Ok(false);
        }
        let last = focus_subject.unwrap_or(last);
        if !last.is_empty() && !matches_compound_selector(self.get(node)?, last) {
            return Ok(false);
        }
        let mut ancestor = self.get(node)?.parent;
        for compound in compounds[..compounds.len() - 1].iter().rev() {
            let mut matched = None;
            while let Some(candidate) = ancestor {
                let record = self.get(candidate)?;
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

    pub fn hosted_document(&self, frame: NodeId) -> Option<NodeId> {
        self.hosted_documents.get(&frame).copied()
    }

    pub fn document_host(&self, document: NodeId) -> Option<NodeId> {
        self.hosted_documents
            .iter()
            .find_map(|(frame, root)| (*root == document).then_some(*frame))
    }

    pub fn document_is_retired(&self, document: NodeId) -> bool {
        self.retired_documents.contains(&document)
    }

    /// Native projection/routing ancestry, distinct from Node.parentNode.
    pub fn embedding_parent(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.get(node)?;
        Ok(record.parent.or_else(|| {
            matches!(record.kind, NodeKind::Document)
                .then(|| self.document_host(node))
                .flatten()
        }))
    }

    pub const fn id(&self) -> ArenaId {
        self.id
    }

    pub fn node_ref(&self, node: NodeId) -> Result<NodeRef, DomError> {
        self.get(node)?;
        Ok(NodeRef {
            arena: self.id,
            node,
        })
    }

    pub fn resolve(&self, reference: NodeRef) -> Result<&Node, DomError> {
        if reference.arena != self.id {
            return Err(DomError::WrongArena {
                expected: self.id,
                actual: reference.arena,
            });
        }
        self.get(reference.node)
    }

    pub fn document_id(&self, node: NodeId) -> Result<DocumentId, DomError> {
        if !matches!(self.get(node)?.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(node));
        }
        Ok(DocumentId(self.node_ref(node)?))
    }

    pub fn prepare_rebuild(previous: &Self, retained: &[NodeId]) -> Self {
        let mut arena = Self {
            hosted_documents: BTreeMap::new(),
            retired_documents: BTreeSet::new(),
            id: ArenaId::fresh(),
            allocation_document: None,
            slots: previous
                .slots
                .iter()
                .map(|slot| Slot {
                    generation: if slot.node.is_some() {
                        slot.generation.saturating_add(1)
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
            .filter(|(index, slot)| {
                slot.generation != u32::MAX && !retained.iter().any(|id| id.slot as usize == *index)
            })
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
        self.initialize_node_document(id);
        if let NodeKind::Attribute {
            owner: Some(owner),
            data,
            ..
        } = &self.get(id)?.kind
        {
            self.attribute_nodes.insert((*owner, data.key()), id);
        }
        Ok(())
    }

    pub fn insert(&mut self, kind: NodeKind) -> NodeId {
        let attribute_key = match &kind {
            NodeKind::Attribute {
                owner: Some(owner),
                data,
                ..
            } => Some((*owner, data.key())),
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
            let slot = u32::try_from(self.slots.len()).expect("DOM slot identity space exhausted");
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
        self.initialize_node_document(id);
        id
    }

    fn initialize_node_document(&mut self, node: NodeId) {
        if matches!(
            self.get(node).expect("allocated node").kind,
            NodeKind::Document
        ) {
            self.allocation_document.get_or_insert(node);
        } else {
            let owner = match self.get(node).expect("allocated node").kind {
                NodeKind::Attribute {
                    owner: Some(element),
                    ..
                } => self.get(element).ok().and_then(|n| n.owner_document),
                _ => self.allocation_document,
            };
            self.get_mut(node).expect("allocated node").owner_document = owner;
        }
    }

    /// The internal node document; a Document is its own node document.
    pub fn node_document(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.get(node)?;
        Ok(if matches!(record.kind, NodeKind::Document) {
            Some(node)
        } else {
            record.owner_document
        })
    }

    fn assign_node_document(&mut self, root: NodeId, document: NodeId) -> Result<(), DomError> {
        self.document_id(document)?;
        let mut pending = vec![(root, document)];
        let mut seen = BTreeSet::new();
        while let Some((node, document)) = pending.pop() {
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            if matches!(self.get(node)?.kind, NodeKind::Document) {
                // Never change a Document's identity or cross a host edge.
                continue;
            }
            pending.extend(
                self.attribute_node_ids(node)
                    .into_iter()
                    .map(|id| (id, document)),
            );
            let record = self.get_mut(node)?;
            record.owner_document = Some(document);
            pending.extend(record.children.iter().copied().map(|id| (id, document)));
            if let Some(content) = record.template_content {
                let owner = self.template_contents_owner_document(document)?;
                pending.push((content, owner));
            }
        }
        Ok(())
    }

    fn template_contents_owner_document(&mut self, document: NodeId) -> Result<NodeId, DomError> {
        self.document_id(document)?;
        if let Some(owner) = self.get(document)?.associated_inert_document {
            self.document_id(owner)?;
            return Ok(owner);
        }
        let is_html = self
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|m| m.is_html);
        let owner = self.insert(NodeKind::Document);
        if is_html {
            self.get_mut(owner)?.document_metadata = Some(Box::new(DocumentMetadata::html()));
        }
        self.get_mut(owner)?.associated_inert_document = Some(owner);
        self.get_mut(document)?.associated_inert_document = Some(owner);
        Ok(owner)
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

    /// Validate host-including ancestry without mutating or exposing host edges
    /// through the ordinary DOM ancestor API.
    pub fn validate_tree_edge(&self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        if matches!(self.get(parent)?.kind, NodeKind::Attribute { .. })
            || matches!(self.get(child)?.kind, NodeKind::Attribute { .. })
        {
            return Err(DomError::AttributeTreeEdge);
        }
        if matches!(self.get(child)?.kind, NodeKind::Document) {
            return Err(DomError::DocumentTreeEdge);
        }
        if parent == child {
            return Err(DomError::SelfParent);
        }
        // Hosted documents are not DOM children, but an insertion must not
        // turn their explicit native embedding/retention graph into a cycle.
        // Validate before detach or mutation publication on every entry path.
        let mut ancestor = Some(parent);
        let mut seen = BTreeSet::new();
        while let Some(node) = ancestor {
            if node == child || !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            ancestor = self
                .get(node)?
                .template_host
                .or(self.embedding_parent(node)?);
        }
        Ok(())
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        self.validate_tree_edge(parent, child)?;
        self.invalidate_stylesheet_ancestors(Some(parent))?;
        self.detach(child)?;
        self.get_mut(child)?.parent = Some(parent);
        self.get_mut(parent)?.children.push(child);
        if let Some(document) = self.node_document(parent)? {
            self.assign_node_document(child, document)?;
        }
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
        self.validate_tree_edge(parent, node)?;
        if sibling == node {
            return Ok(());
        }
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
        if let Some(document) = self.node_document(parent)? {
            self.assign_node_document(node, document)?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: NodeId) -> Result<Node, DomError> {
        self.detach(id)?;
        let slot = self.slot_mut(id).ok_or(DomError::StaleNode(id))?;
        let node = slot.node.take().ok_or(DomError::StaleNode(id))?;
        // Retire exhausted slots permanently: wrapping would revive stale IDs.
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(id.slot);
        }
        if let Some(content) = node.template_content
            && let Ok(record) = self.get_mut(content)
        {
            record.template_host = None;
        }
        if let Some(host) = node.template_host
            && let Ok(record) = self.get_mut(host)
            && record.template_content == Some(id)
        {
            record.template_content = None;
        }
        self.stylesheet_revisions.remove(&id);
        self.name_attribute_revisions.remove(&id);
        self.hosted_documents
            .retain(|frame, document| *frame != id && *document != id);
        self.retired_documents.remove(&id);
        if let NodeKind::Attribute {
            owner: Some(owner),
            data,
            ..
        } = &node.kind
        {
            let key = (*owner, data.key());
            if self.attribute_nodes.get(&key) == Some(&id) {
                self.attribute_nodes.remove(&key);
            }
        }
        Ok(node)
    }

    pub fn existing_attribute_node(&self, element: NodeId, name: &str) -> Option<NodeId> {
        let key = self.get(element).ok()?.attributes.named(name)?.key();
        self.attribute_nodes.get(&(element, key)).copied()
    }

    pub fn existing_attribute_node_ns(
        &self,
        element: NodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<NodeId> {
        self.attribute_nodes
            .get(&(element, AttributeKey::new(namespace, local_name)))
            .copied()
    }

    pub fn attribute_node_ids(&self, element: NodeId) -> Vec<NodeId> {
        self.get(element)
            .map(|node| {
                node.attributes
                    .records()
                    .filter_map(|entry| self.attribute_nodes.get(&(element, entry.key())).copied())
                    .collect()
            })
            .unwrap_or_default()
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
        Ok(node.stylesheet_media.clone().unwrap_or_else(|| {
            node.attributes
                .get_null("media")
                .cloned()
                .unwrap_or_default()
        }))
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
        owner_document: None,
        document_metadata,
        associated_inert_document: None,
        kind,
        parent: None,
        children: Vec::new(),
        attributes: AttributeList::default(),
        event_listeners: BTreeSet::new(),
        adopted_stylesheets: Vec::new(),
        stylesheet_override: None,
        stylesheet_media: None,
        template_content: None,
        template_host: None,
        value: None,
        dirty_value: false,
        user_edited_value: false,
        pending_edit_value: None,
        selected_files: Arc::new(Vec::new()),
        image_coordinates: (0, 0),
        image_dimensions: None,
        checkedness: None,
        dirty_checkedness: false,
        selectedness: None,
        dirty_selectedness: false,
        custom_validity_message: String::new(),
        indeterminate: false,
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
        #[serde(default)]
        siblings: MutationSiblings,
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
    ImageResource {
        node: NodeId,
    },
    HostedDocument {
        frame: NodeId,
    },
    DocumentOwnership {
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
    pattern_matcher: Option<Rc<dyn PatternMatcher>>,
    arena: DomArena,
    document: Option<NodeId>,
    connected: BTreeSet<NodeId>,
    frame_context_generations: BTreeMap<NodeId, u64>,
    roots: BTreeMap<NodeId, NodeRoots>,
    /// Conservative targets of internal non-tree edges. Used only to avoid
    /// full graph tracing for ordinary detached tree components.
    internal_targets: BTreeSet<NodeId>,
    mutations: MutationJournal,
    focused: Option<NodeId>,
    focus_ring_suppressed: bool,
    next_range: u64,
    ranges: BTreeMap<LiveRangeId, LiveRange>,
    opaque_ranges: OpaqueRangeStore,
    radio_form_owners: BTreeMap<NodeId, Option<NodeId>>,
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
        runtime.index_internal_targets();
        runtime.rebuild_document_indexes()?;
        runtime.initialize_checkedness()?;
        runtime.initialize_selects()?;
        runtime.mutations.clear();
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
        self.frame_context_generations.clear();
        self.roots.clear();
        self.internal_targets.clear();
        self.index_internal_targets();
        self.mutations.clear();
        self.focused = None;
        self.focus_ring_suppressed = false;
        self.next_range = 0;
        self.ranges.clear();
        self.opaque_ranges = OpaqueRangeStore::default();
        self.radio_form_owners.clear();
        self.rebuild_document_indexes()?;
        self.initialize_checkedness()?;
        self.initialize_selects()?;
        self.mutations.clear();
        Ok(())
    }

    pub fn arena(&self) -> &DomArena {
        &self.arena
    }
    pub fn arena_mut(&mut self) -> &mut DomArena {
        &mut self.arena
    }

    /// Ordinary reparenting discards a browsing context even when removal and
    /// insertion happen in one call; state-preserving moveBefore does not.
    pub fn track_frame_context(&mut self, frame: NodeId) -> u64 {
        *self.frame_context_generations.entry(frame).or_insert(1)
    }

    pub fn frame_context_generation(&self, frame: NodeId) -> Option<u64> {
        self.frame_context_generations.get(&frame).copied()
    }

    pub fn untrack_frame_context(&mut self, frame: NodeId) {
        self.frame_context_generations.remove(&frame);
    }

    fn index_internal_targets(&mut self) {
        for (_, record) in self.arena.iter() {
            self.internal_targets.extend(record.template_content);
            self.internal_targets
                .extend(record.adopted_stylesheets.iter().copied());
        }
    }

    pub fn set_node_document(&mut self, node: NodeId, document: NodeId) -> Result<(), DomError> {
        self.arena.document_id(document)?;
        if self.arena.node_document(node)? == Some(document) {
            return Ok(());
        }
        let previous = self.ownership_before_adoption(node, Some(document))?;
        self.arena.assign_node_document(node, document)?;
        self.publish_ownership_changes(previous)?;
        Ok(())
    }

    fn ownership_before_adoption(
        &self,
        node: NodeId,
        document: Option<NodeId>,
    ) -> Result<Vec<(NodeId, Option<NodeId>)>, DomError> {
        if self.arena.node_document(node)? == document {
            return Ok(Vec::new());
        }
        let mut previous = Vec::new();
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            previous.push((node, self.arena.node_document(node)?));
            pending.extend(record.template_content);
            pending.extend(record.children.iter().rev().copied());
            pending.extend(self.arena.attribute_node_ids(node));
        }
        Ok(previous)
    }

    fn publish_ownership_changes(
        &mut self,
        previous: Vec<(NodeId, Option<NodeId>)>,
    ) -> Result<(), DomError> {
        for (node, document) in previous {
            if self.arena.node_document(node)? != document {
                self.mutations.push(Mutation::DocumentOwnership { node });
            }
        }
        Ok(())
    }

    /// Adopt within this agent arena without cloning or changing NodeId.
    pub fn adopt_node(&mut self, document: DocumentId, node: NodeRef) -> Result<NodeId, DomError> {
        self.arena.resolve(document.root())?;
        self.arena.resolve(node)?;
        let document = document.root().node;
        self.arena.document_id(document)?;
        let node = node.node;
        let previous_document = self.arena.get(node)?.owner_document;
        if matches!(
            self.arena.get(node)?.kind,
            NodeKind::Document | NodeKind::ShadowRoot { .. }
        ) {
            return Err(DomError::DocumentTreeEdge);
        }
        // Keep the original identity live while removal updates ranges and
        // publishes observer records. Attribute ownership is not a tree edge.
        self.retain_native(node)?;
        let result = (|| {
            if let NodeKind::Attribute {
                owner: Some(owner), ..
            } = self.arena.get(node)?.kind
            {
                self.remove_attribute_node(owner, node)?;
            } else if self.parent(node)?.is_some() {
                self.remove(node)?;
            }
            self.set_node_document(node, document)?;
            Ok(node)
        })();
        // The caller must own a wrapper/native lease for an adopted detached
        // node, just as for other native mutations returning a live node.
        self.release_native(node)?;
        if let Some(previous) = previous_document
            && previous != document
            && self.arena.get(previous).is_ok()
        {
            self.reclaim_if_unrooted(previous)?;
        }
        result
    }

    /// Replace a same-arena frame's hosted Document without publishing a DOM
    /// mutation or making the Document an iframe child. Authority stays in the
    /// native coordinator; this API is not exposed as a web-platform operation.
    pub fn set_hosted_document(
        &mut self,
        frame: NodeId,
        document: Option<NodeId>,
    ) -> Result<(), DomError> {
        if !self.is_element(frame, "http://www.w3.org/1999/xhtml", "iframe") {
            return Err(DomError::NotAnElement(frame));
        }
        if self.arena.hosted_document(frame) == document {
            return Ok(());
        }
        if let Some(document) = document {
            self.arena.document_id(document)?;
            if self.arena.get(document)?.parent.is_some()
                || self.arena.document_host(document).is_some()
            {
                return Err(DomError::Cycle);
            }
            let mut ancestor = Some(frame);
            while let Some(node) = ancestor {
                if node == document {
                    return Err(DomError::Cycle);
                }
                ancestor = self.retention_parent(node)?;
            }
        }
        if let Some(old) = self.arena.hosted_documents.remove(&frame) {
            self.arena.retired_documents.insert(old);
            self.mark_disconnected(old)?;
            self.reclaim_if_unrooted(old)?;
        }
        if let Some(document) = document {
            self.arena.retired_documents.remove(&document);
            self.arena.hosted_documents.insert(frame, document);
            if self.connected.contains(&frame) {
                self.mark_connected(document)?;
            }
        }
        self.mutations.push(Mutation::HostedDocument { frame });
        Ok(())
    }

    pub fn set_image_dimensions(
        &mut self,
        node: NodeId,
        dimensions: Option<(u32, u32)>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.image_dimensions != dimensions {
            record.image_dimensions = dimensions;
            self.mutations.push(Mutation::ImageResource { node });
        }
        Ok(())
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
        self.arena.get_mut(template)?.template_content = Some(content);
        self.arena.get_mut(content)?.template_host = Some(template);
        self.internal_targets.insert(content);
        if let Some(document) = self.arena.node_document(template)? {
            let owner = self.arena.template_contents_owner_document(document)?;
            self.arena.assign_node_document(content, owner)?;
        }
        Ok(content)
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
            target
                .selected_files
                .clone_from(&source_node.selected_files);
            target.checkedness = source_node.checkedness;
            target.dirty_checkedness = source_node.dirty_checkedness;
            target.selectedness = source_node.selectedness;
            target.dirty_selectedness = source_node.dirty_selectedness;
            target.indeterminate = source_node.indeterminate;
            target.sensitive_value = source_node.sensitive_value;
        }
        for attribute in source.attribute_node_ids(node) {
            if let NodeKind::Attribute { data, .. } = &source.get(attribute)?.kind {
                self.set_attribute_ns(
                    imported,
                    data.namespace.clone(),
                    data.name.clone(),
                    data.value.clone(),
                )?;
            }
        }
        self.arena.get_mut(imported)?.checkedness = source_node.checkedness;
        self.arena.get_mut(imported)?.dirty_checkedness = source_node.dirty_checkedness;
        self.arena.get_mut(imported)?.selectedness = source_node.selectedness;
        self.arena.get_mut(imported)?.dirty_selectedness = source_node.dirty_selectedness;
        if let Some(content) = source_node.template_content {
            let imported_content = self.import_subtree(source, content)?;
            self.arena.get_mut(imported)?.template_content = Some(imported_content);
            self.arena.get_mut(imported_content)?.template_host = Some(imported);
            self.internal_targets.insert(imported_content);
            if let Some(document) = self.arena.node_document(imported)? {
                let owner = self.arena.template_contents_owner_document(document)?;
                self.arena.assign_node_document(imported_content, owner)?;
            }
        }
        for child in &source_node.children {
            // A hosted frame Document is not DOM content. Until frame hosting
            // finishes migrating away from tree edges, never import its live
            // document when importing the embedding element.
            if matches!(source.get(*child)?.kind, NodeKind::Document) {
                continue;
            }
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
        self.replace_document_contents_in(self.document(), markup)
    }

    /// Replace only the addressed Document in this arena, preserving its
    /// identity and host association. The primary document is not implicit.
    pub fn replace_document_contents_in(
        &mut self,
        document: NodeId,
        markup: &str,
    ) -> Result<(), DomError> {
        self.arena.document_id(document)?;
        let parsed = html::parse(markup);
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
        self.arena.validate_tree_edge(parent, child)?;
        let previous_ownership =
            self.ownership_before_adoption(child, self.arena.node_document(parent)?)?;
        let option_owners = self.option_owners_in_subtree(child)?;
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
            self.disconnect_opaque_subtree(child)?;
            self.mutations.push(Mutation::Removed {
                node: child,
                former_parent,
                siblings: removed_siblings,
                observer_suppressed: false,
            });
        }
        self.publish_ownership_changes(previous_ownership)?;
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
            siblings: inserted_siblings,
            observer_suppressed: false,
        });
        self.reconcile_inserted_radios(child)?;
        self.reconcile_option_owners(option_owners)?;
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
        self.arena.validate_tree_edge(parent, child)?;
        if let Some(reference) = reference
            && self.arena.get(reference)?.parent != Some(parent)
        {
            return Err(DomError::NotAChild(reference, parent));
        }
        // The DOM insertion algorithms define inserting a node immediately
        // before itself as a no-op. Resolve this before detaching the node or
        // the reference ceases to be a child and its stable identity can be
        // reclaimed as stale.
        if reference == Some(child) {
            return Ok(());
        }
        let previous_ownership =
            self.ownership_before_adoption(child, self.arena.node_document(parent)?)?;
        let option_owners = self.option_owners_in_subtree(child)?;
        let former_parent = self.arena.get(child)?.parent;
        let mut removed_siblings = MutationSiblings::default();
        if let Some(former_parent) = former_parent {
            let former_index = self.child_index(former_parent, child)?;
            removed_siblings = self.mutation_siblings_at(former_parent, former_index)?;
            self.adjust_ranges_for_removal(former_parent, former_index, child)?;
        }
        let was_connected = self.connected.contains(&child);
        if let Some(reference) = reference {
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
            if !preserve_focus {
                self.disconnect_opaque_subtree(child)?;
            }
            self.mutations.push(Mutation::Removed {
                node: child,
                former_parent,
                siblings: removed_siblings,
                observer_suppressed: false,
            });
        }
        self.publish_ownership_changes(previous_ownership)?;
        self.mutations.push(Mutation::Inserted {
            node: child,
            parent: Some(parent),
            siblings: inserted_siblings,
            observer_suppressed: false,
        });
        self.reconcile_inserted_radios(child)?;
        self.reconcile_option_owners(option_owners)?;
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
        self.arena.validate_tree_edge(parent, replacement)?;
        if self.arena.get(child)?.parent != Some(parent) {
            return Err(DomError::NotAChild(child, parent));
        }
        let index = self.child_index(parent, child)? as usize;
        let previous = self.mutation_siblings_at(parent, index as u32)?.previous;
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
        // Adoption removes the incoming node first and its removal remains
        // observable, including self-replacement. Only the actual replacement
        // removal/insertion are suppressed under the compound target record.
        if self.arena.get(replacement)?.parent.is_some() {
            self.remove(replacement)?;
        }
        let cursor = self.mutation_cursor();
        let mut removed_nodes = Vec::new();
        if self.arena.get(child)?.parent.is_some() {
            removed_nodes.push(child);
            self.remove_child(parent, child)?;
        }
        self.insert_before(parent, replacement, reference)?;
        self.mutations.suppress_tree_observers_since(cursor, parent);
        self.mutations.push(Mutation::ChildrenReplaced {
            node: parent,
            added_nodes: vec![replacement],
            removed_nodes,
            siblings: MutationSiblings {
                previous,
                next: reference,
            },
        });
        Ok(())
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
        self.arena.get_mut(node)?.adopted_stylesheets = sheets;
        self.internal_targets.extend(new.iter().copied());
        for sheet in old.difference(&new) {
            self.reclaim_if_unrooted(*sheet)?;
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
            .get(node)?
            .attributes
            .named(&name)
            .and_then(|record| record.namespace.clone());
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
        if !matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        let namespace = namespace.filter(|value| !value.is_empty());
        let (prefix, local_name) = if namespace.is_some() {
            name.split_once(':')
                .map_or((None, name.clone()), |(prefix, local)| {
                    (Some(prefix.to_string()), local.to_string())
                })
        } else {
            (None, name.clone())
        };
        let value = value.into();
        // Setting an existing expanded name changes only its value. Replacing
        // an Attr node is a separate operation and may supply a new prefix.
        let (name, prefix) = self
            .arena
            .get(node)?
            .attributes
            .namespaced(namespace.as_deref(), &local_name)
            .map_or((name, prefix), |existing| {
                (existing.name.clone(), existing.prefix.clone())
            });
        let data = Arc::new(ContentAttribute {
            namespace: namespace.clone(),
            local_name: local_name.clone(),
            name: name.clone(),
            prefix,
            value: value.clone(),
        });
        self.write_attribute_record(node, data)
    }

    fn write_attribute_record(
        &mut self,
        node: NodeId,
        data: Arc<ContentAttribute>,
    ) -> Result<(), DomError> {
        let key = data.key();
        let name = data.name.clone();
        let namespace = data.namespace.clone();
        let value = data.value.clone();
        let old_email_value = self.text_input_attribute_transition_value(node, &name, &value)?;
        let old_value = self
            .arena
            .get_mut(node)?
            .attributes
            .insert(data.clone())
            .map(|entry| entry.value.clone());
        if name == "type"
            && namespace.is_none()
            && self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && (old_value
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("file"))
                != value.eq_ignore_ascii_case("file"))
        {
            self.arena.get_mut(node)?.selected_files = Arc::new(Vec::new());
            self.arena.get_mut(node)?.value = None;
        }
        if name == "media" && namespace.is_none() {
            self.arena.get_mut(node)?.stylesheet_media = None;
        }
        if namespace.is_none() {
            self.sanitize_after_input_attribute(node, &name, old_email_value)?;
        }
        if let Some(attribute) = self
            .arena
            .attribute_nodes
            .get(&(node, key.clone()))
            .copied()
            && let NodeKind::Attribute {
                data: attribute_data,
                ..
            } = &mut self.arena.get_mut(attribute)?.kind
        {
            *attribute_data = data.clone();
        }
        if namespace.is_some()
            && !self
                .arena
                .attribute_nodes
                .contains_key(&(node, key.clone()))
        {
            let attribute = self.create(NodeKind::Attribute {
                data,
                owner: Some(node),
            });
            self.arena
                .attribute_nodes
                .insert((node, key.clone()), attribute);
        }
        if name == "name" && namespace.is_none() {
            let revision = self.arena.name_attribute_revisions.entry(node).or_default();
            *revision = revision.saturating_add(1);
        }
        if namespace.is_none() {
            self.checked_attribute_changed(node, &name)?;
            self.option_attribute_changed(node, &name)?;
        }
        self.mutations.push(Mutation::Attribute {
            node,
            name: key.local_name,
            namespace,
            old_value,
            new_value: Some(value),
        });
        Ok(())
    }

    pub fn remove_attribute(&mut self, node: NodeId, name: &str) -> Result<(), DomError> {
        let Some(attribute) = self.arena.get(node)?.attributes.named(name) else {
            if !matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
                return Err(DomError::NotAnElement(node));
            }
            return Ok(());
        };
        let key = attribute.key();
        self.remove_attribute_key(node, key)
    }

    pub fn remove_attribute_ns(
        &mut self,
        node: NodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Result<(), DomError> {
        self.remove_attribute_key(node, AttributeKey::new(namespace, local_name))
    }

    fn remove_attribute_key(&mut self, node: NodeId, key: AttributeKey) -> Result<(), DomError> {
        if !matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        let old_email_value =
            self.text_input_attribute_transition_value(node, &key.local_name, "")?;
        let Some(entry) = self.arena.get_mut(node)?.attributes.remove(&key) else {
            return Ok(());
        };
        let name = entry.name.as_str();
        let old_value = Some(entry.value.clone());
        let namespace = entry.namespace.clone();
        if namespace.is_none() {
            self.sanitize_after_input_attribute(node, name, old_email_value)?;
        }
        if entry.local_name == "type"
            && namespace.is_none()
            && entry.value.eq_ignore_ascii_case("file")
            && self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
        {
            self.arena.get_mut(node)?.selected_files = Arc::new(Vec::new());
            self.arena.get_mut(node)?.value = None;
        }
        if let Some(attribute) = self.arena.attribute_nodes.remove(&(node, key))
            && let NodeKind::Attribute { owner, .. } = &mut self.arena.get_mut(attribute)?.kind
        {
            *owner = None;
        }
        if name == "media" && namespace.is_none() && old_value.is_some() {
            self.arena.get_mut(node)?.stylesheet_media = None;
        }
        if old_value.is_some() {
            if namespace.is_none() {
                self.checked_attribute_changed(node, name)?;
                self.option_attribute_changed(node, name)?;
            }
            if name == "name" && namespace.is_none() {
                let revision = self.arena.name_attribute_revisions.entry(node).or_default();
                *revision = revision.saturating_add(1);
            }
            self.mutations.push(Mutation::Attribute {
                node,
                name: entry.local_name.clone(),
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

    /// HTML algorithms consume null-namespace content attributes, not a
    /// namespaced Attr that happens to have the same qualified name.
    pub fn attribute_in_null_namespace(
        &self,
        node: NodeId,
        name: &str,
    ) -> Result<Option<&str>, DomError> {
        self.attribute_ns(node, None, name)
    }

    pub fn attribute_ns(
        &self,
        node: NodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Result<Option<&str>, DomError> {
        let record = self.arena.get(node)?;
        if !matches!(record.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        Ok(record
            .attributes
            .namespaced(namespace, local_name)
            .map(|entry| entry.value.as_str()))
    }

    pub fn attribute_node(
        &mut self,
        element: NodeId,
        name: &str,
    ) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(element)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(element));
        }
        let Some(entry) = self.arena.get(element)?.attributes.named(name).cloned() else {
            return Ok(None);
        };
        self.attribute_node_ns(element, entry.namespace.as_deref(), &entry.local_name)
    }

    pub fn attribute_node_ns(
        &mut self,
        element: NodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(element)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(element));
        }
        let Some(entry) = self
            .arena
            .get(element)?
            .attributes
            .shared(namespace, local_name)
        else {
            return Ok(None);
        };
        let key = entry.key();
        if let Some(attribute) = self
            .arena
            .attribute_nodes
            .get(&(element, key.clone()))
            .copied()
        {
            return Ok(Some(attribute));
        }
        let attribute = self.create(NodeKind::Attribute {
            data: entry,
            owner: Some(element),
        });
        self.arena.attribute_nodes.insert((element, key), attribute);
        Ok(Some(attribute))
    }

    pub fn set_attribute_node_value(
        &mut self,
        attribute: NodeId,
        value: impl Into<String>,
    ) -> Result<(), DomError> {
        let value = value.into();
        let (name, namespace, owner) = match &self.arena.get(attribute)?.kind {
            NodeKind::Attribute { data, owner, .. } => {
                (data.name.clone(), data.namespace.clone(), *owner)
            }
            _ => return Err(DomError::StaleNode(attribute)),
        };
        if let Some(owner) = owner {
            return self.set_attribute_ns(owner, namespace, name, value);
        }
        if let NodeKind::Attribute { data, .. } = &mut self.arena.get_mut(attribute)?.kind {
            let mut updated = data.as_ref().clone();
            updated.value = value;
            *data = Arc::new(updated);
        }
        Ok(())
    }

    /// Attach the supplied Attr by expanded name, retaining its native identity.
    pub fn set_attribute_node(
        &mut self,
        element: NodeId,
        attribute: NodeId,
    ) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(element)?.kind, NodeKind::Element { .. }) {
            return Err(DomError::NotAnElement(element));
        }
        let NodeKind::Attribute { data, owner } = self.arena.get(attribute)?.kind.clone() else {
            return Err(DomError::StaleNode(attribute));
        };
        if owner.is_some_and(|owner| owner != element) {
            return Err(DomError::InUseAttribute);
        }
        let previous =
            self.attribute_node_ns(element, data.namespace.as_deref(), &data.local_name)?;
        if previous == Some(attribute) {
            return Ok(previous);
        }
        let key = data.key();
        // Remove only the identity index before writing: the old content record
        // supplies the old value and keeps its insertion position.
        self.arena.attribute_nodes.remove(&(element, key.clone()));
        if let Some(previous) = previous
            && let NodeKind::Attribute { owner, .. } = &mut self.arena.get_mut(previous)?.kind
        {
            *owner = None;
        }
        if let NodeKind::Attribute { owner, .. } = &mut self.arena.get_mut(attribute)?.kind {
            *owner = Some(element);
        }
        if let Some(document) = self.arena.node_document(element)? {
            self.arena.assign_node_document(attribute, document)?;
        }
        self.arena.attribute_nodes.insert((element, key), attribute);
        self.write_attribute_record(element, data)?;
        Ok(previous)
    }

    pub fn remove_attribute_node(
        &mut self,
        element: NodeId,
        attribute: NodeId,
    ) -> Result<(), DomError> {
        let NodeKind::Attribute { data, owner } = &self.arena.get(attribute)?.kind else {
            return Err(DomError::AttributeNotFound);
        };
        if *owner != Some(element) {
            return Err(DomError::AttributeNotFound);
        }
        let key = data.key();
        self.remove_attribute_key(element, key)
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
                siblings: MutationSiblings::default(),
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
        self.arena.get_mut(clone)?.owner_document = source.owner_document;
        {
            let record = self.arena.get_mut(clone)?;
            record.document_metadata = source.document_metadata;
            record.attributes = source.attributes;
            record.value = source.value;
            record.dirty_value = source.dirty_value;
            record.user_edited_value = source.user_edited_value;
            record.selected_files = source.selected_files;
            record.checkedness = source.checkedness;
            record.dirty_checkedness = source.dirty_checkedness;
            record.selectedness = source.selectedness;
            record.dirty_selectedness = source.dirty_selectedness;
            record.indeterminate = source.indeterminate;
            record.sensitive_value = source.sensitive_value;
        }
        for attribute in attributes {
            if let NodeKind::Attribute { data, .. } = attribute {
                self.set_attribute_ns(
                    clone,
                    data.namespace.clone(),
                    data.name.clone(),
                    data.value.clone(),
                )?;
            }
        }
        self.arena.get_mut(clone)?.checkedness = source.checkedness;
        self.arena.get_mut(clone)?.dirty_checkedness = source.dirty_checkedness;
        self.arena.get_mut(clone)?.selectedness = source.selectedness;
        self.arena.get_mut(clone)?.dirty_selectedness = source.dirty_selectedness;
        if let Some(content) = source.template_content {
            let cloned_content = self.clone_node(content, deep)?;
            self.arena.get_mut(clone)?.template_content = Some(cloned_content);
            self.arena.get_mut(cloned_content)?.template_host = Some(clone);
            self.internal_targets.insert(cloned_content);
            if let Some(document) = self.arena.node_document(clone)? {
                let owner = self.arena.template_contents_owner_document(document)?;
                self.arena.assign_node_document(cloned_content, owner)?;
            }
        }
        if deep {
            for child in source.children {
                // Clone the embedding element's content, not its hosted
                // browsing context or the active Document in that context.
                if matches!(self.arena.get(child)?.kind, NodeKind::Document) {
                    continue;
                }
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

    /// DOM equality compares structure and expanded attribute names, not markup
    /// serialization (which loses namespaces and exposes attribute order).
    pub fn is_equal_node(&self, left: NodeId, right: NodeId) -> Result<bool, DomError> {
        let mut pending = vec![(left, right)];
        while let Some((left, right)) = pending.pop() {
            let left = self.arena.get(left)?;
            let right = self.arena.get(right)?;
            let kind_equal = match (&left.kind, &right.kind) {
                (NodeKind::Attribute { data: a, .. }, NodeKind::Attribute { data: b, .. }) => {
                    (&a.namespace, &a.local_name, &a.value)
                        == (&b.namespace, &b.local_name, &b.value)
                }
                (a, b) => a == b,
            };
            if !kind_equal || left.attributes.len() != right.attributes.len() {
                return Ok(false);
            }
            for attribute in left.attributes.records() {
                if right
                    .attributes
                    .namespaced(attribute.namespace.as_deref(), &attribute.local_name)
                    .is_none_or(|other| other.value != attribute.value)
                {
                    return Ok(false);
                }
            }
            let children = |node: &Node| {
                node.children
                    .iter()
                    .copied()
                    .filter(|child| {
                        self.arena
                            .get(*child)
                            .is_ok_and(|child| !matches!(child.kind, NodeKind::ShadowRoot { .. }))
                    })
                    .collect::<Vec<_>>()
            };
            let a = children(left);
            let b = children(right);
            if a.len() != b.len() {
                return Ok(false);
            }
            pending.extend(a.into_iter().zip(b));
        }
        Ok(true)
    }

    pub fn remove(&mut self, node: NodeId) -> Result<(), DomError> {
        let option_owners = self.option_owners_in_subtree(node)?;
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
        self.refresh_radio_owners()?;
        self.reconcile_option_owners(option_owners)?;
        if parent.is_some() {
            self.disconnect_opaque_subtree(node)?;
        }
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
        let owner = self.arena.get(node)?.owner_document;
        let roots = self
            .roots
            .get_mut(&node)
            .ok_or(DomError::WrapperLeaseUnderflow(node))?;
        roots.wrappers = roots
            .wrappers
            .checked_sub(1)
            .ok_or(DomError::WrapperLeaseUnderflow(node))?;
        self.reclaim_if_unrooted(node)?;
        if let Some(owner) = owner
            && self.arena.get(owner).is_ok()
        {
            self.reclaim_if_unrooted(owner)?;
        }
        Ok(())
    }

    pub fn retain_native(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get(node)?;
        self.roots.entry(node).or_default().native += 1;
        Ok(())
    }

    pub fn release_native(&mut self, node: NodeId) -> Result<(), DomError> {
        let owner = self.arena.get(node)?.owner_document;
        let roots = self
            .roots
            .get_mut(&node)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        roots.native = roots
            .native
            .checked_sub(1)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        self.reclaim_if_unrooted(node)?;
        if let Some(owner) = owner
            && self.arena.get(owner).is_ok()
        {
            self.reclaim_if_unrooted(owner)?;
        }
        Ok(())
    }

    /// Explicit inertness follows the flat tree, including assignment into
    /// named/default slots. This is intentionally not modal-dialog blocking.
    pub fn is_inert(&self, node: NodeId) -> Result<bool, DomError> {
        let mut current = Some(node);
        while let Some(node) = current {
            let record = self.arena.get(node)?;
            if matches!(&record.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
                && record.attributes.contains_null("inert")
            {
                return Ok(true);
            }
            current = self.inert_flat_parent(node)?;
        }
        Ok(false)
    }

    fn inert_flat_parent(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.arena.get(node)?;
        let Some(parent) = record.parent else {
            return Ok(None);
        };
        if !matches!(
            record.kind,
            NodeKind::Element { .. } | NodeKind::Text { .. }
        ) {
            return Ok(Some(parent));
        }
        let shadow = self
            .arena
            .get(parent)?
            .children
            .iter()
            .copied()
            .find(|child| {
                self.arena
                    .get(*child)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
            });
        if let Some(shadow) = shadow {
            let name = record
                .attributes
                .get_null("slot")
                .map_or("", String::as_str);
            let mut pending = self.arena.get(shadow)?.children.clone();
            pending.reverse();
            while let Some(candidate) = pending.pop() {
                let record = self.arena.get(candidate)?;
                if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                    continue;
                }
                if matches!(&record.kind, NodeKind::Element { name }
                    if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
                    && record
                        .attributes
                        .get_null("name")
                        .map_or("", String::as_str)
                        == name
                {
                    return Ok(Some(candidate));
                }
                pending.extend(record.children.iter().rev().copied());
            }
        }
        Ok(Some(parent))
    }

    pub fn focus(&mut self, current: Option<NodeId>) -> Result<(), DomError> {
        if let Some(node) = current {
            self.arena.get(node)?;
            if !self.connected.contains(&node) || self.is_inert(node)? {
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

    /// Retarget the deepest focus for one document or shadow root. Embedding
    /// ancestry is used only for focus propagation; it is not DOM ancestry.
    pub fn focused_element_in(&self, root: NodeId) -> Result<Option<NodeId>, DomError> {
        self.arena.get(root)?;
        let Some(mut target) = self.focused else {
            return Ok(None);
        };
        let mut current = Some(target);
        while let Some(node) = current {
            if node == root {
                return Ok(Some(target));
            }
            let record = self.arena.get(node)?;
            current = match record.kind {
                NodeKind::Document => self.arena.document_host(node),
                _ => record.parent,
            };
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) && let Some(host) = current
            {
                target = host;
            }
        }
        Ok(None)
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
                return !record.attributes.contains_null("readonly");
            }
            if name.local.as_ref() == "input" {
                let kind = record
                    .attributes
                    .get_null("type")
                    .map(String::as_str)
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                return !record.attributes.contains_null("readonly")
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
            if let Some(value) = record.attributes.get_null("contenteditable") {
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

    /// Native value before a control has acquired a dirty value override.
    pub fn is_file_input(&self, node: NodeId) -> bool {
        self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && self
                .attribute_in_null_namespace(node, "type")
                .ok()
                .flatten()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
    }

    pub fn set_selected_files(
        &mut self,
        node: NodeId,
        files: Arc<Vec<SelectedFile>>,
    ) -> Result<(), DomError> {
        if !self.is_file_input(node) {
            return Err(DomError::NotAnElement(node));
        }
        self.arena.get_mut(node)?.selected_files = files;
        self.arena.get_mut(node)?.value = None;
        self.mutations.push(Mutation::FormState { node });
        Ok(())
    }

    pub fn control_value(&self, node: NodeId) -> Result<String, DomError> {
        if self.is_file_input(node) {
            return Ok(self
                .arena
                .get(node)?
                .selected_files
                .first()
                .map(|file| format!("C:\\fakepath\\{}", file.name))
                .unwrap_or_default());
        }
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "select") {
            return self.select_value(node);
        }
        let record = self.arena.get(node)?;
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && record.attributes.get_null("type").is_some_and(|kind| {
                kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
            })
        {
            return Ok(record
                .attributes
                .get_null("value")
                .cloned()
                .unwrap_or_else(|| "on".into()));
        }
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") && !record.dirty_value
        {
            return Ok(dom_string::from_units(&self.normalized_control_edit_units(
                node,
                &self.textarea_default_value(node)?,
            )));
        }
        if let Some(value) = &record.value {
            return self.sanitized_input_value(node, value);
        }
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") {
            return self.arena.text_content(node);
        }
        self.sanitized_input_value(
            node,
            &record
                .attributes
                .get_null("value")
                .cloned()
                .unwrap_or_default(),
        )
    }

    pub fn create_opaque_range(
        &mut self,
        owner: NodeId,
        start: u32,
        end: u32,
    ) -> Result<OpaqueRangeId, DomError> {
        let length = u32::try_from(dom_string::len(&self.control_value(owner)?))
            .map_err(|_| OpaqueRangeError::InvalidOffset)?;
        let id = self.opaque_ranges.create(owner, start, end, length)?;
        self.retain_native(owner)?;
        Ok(id)
    }

    pub fn opaque_range(&self, id: OpaqueRangeId) -> Result<OpaqueRange, DomError> {
        Ok(self.opaque_ranges.get(id)?)
    }

    pub fn disconnect_opaque_range(&mut self, id: OpaqueRangeId) -> Result<(), DomError> {
        let owner = self.opaque_ranges.get(id)?.owner;
        self.opaque_ranges.disconnect(id)?;
        if let Some(owner) = owner {
            self.release_native(owner)?;
        }
        Ok(())
    }

    /// Called after a successful removal, with the mutation's node lease held.
    /// Include shadow descendants: value ranges disconnect when their control
    /// leaves the document even though ordinary Range endpoints stay in a
    /// removed shadow tree.
    pub fn disconnect_opaque_subtree(&mut self, root: NodeId) -> Result<(), DomError> {
        if !self.opaque_ranges.has_live_ranges() {
            return Ok(());
        }
        self.retain_native(root)?;
        let result = (|| {
            let mut pending = vec![root];
            while let Some(node) = pending.pop() {
                pending.extend(self.arena.get(node)?.children.iter().copied());
                for id in self.opaque_ranges.owner_ids(node) {
                    self.disconnect_opaque_range(id)?;
                }
            }
            Ok(())
        })();
        self.release_native(root)?;
        result
    }

    pub fn drop_opaque_range(&mut self, id: OpaqueRangeId) -> Result<(), DomError> {
        self.disconnect_opaque_range(id)?;
        self.opaque_ranges.drop_range(id)?;
        Ok(())
    }

    pub fn value(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        Ok(self.arena.get(node)?.value.as_deref())
    }

    /// Replace a control-value interval without losing the edit boundaries.
    pub fn replace_control_value(
        &mut self,
        node: NodeId,
        start: u32,
        end: u32,
        replacement: &str,
    ) -> Result<(), DomError> {
        let previous = self.control_value(node)?;
        let mut units = dom_string::units(&previous);
        let old_length = u32::try_from(units.len()).map_err(|_| OpaqueRangeError::InvalidOffset)?;
        if start > end || end > old_length {
            return Err(OpaqueRangeError::InvalidOffset.into());
        }
        let inserted = u32::try_from(dom_string::len(replacement))
            .map_err(|_| OpaqueRangeError::InvalidOffset)?;
        units.splice(start as usize..end as usize, dom_string::units(replacement));
        let value = self.sanitized_input_value(node, &dom_string::from_units(&units))?;
        self.opaque_ranges
            .replace(node, start, end - start, inserted, old_length)?;
        self.arena.get_mut(node)?.value = Some(value);
        self.arena.get_mut(node)?.dirty_value = true;
        self.arena.get_mut(node)?.user_edited_value = false;
        self.mutations.push(Mutation::FormState { node });
        Ok(())
    }

    pub fn set_value(
        &mut self,
        node: NodeId,
        value: impl Into<String>,
        sensitive: bool,
    ) -> Result<(), DomError> {
        let value = value.into();
        if self.is_file_input(node) {
            return self.set_selected_files(node, Arc::new(Vec::new()));
        }
        let mut units = dom_string::units(&value);
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") {
            let mut normalized = Vec::with_capacity(units.len());
            let mut previous_cr = false;
            for unit in units {
                if unit != 10 || !previous_cr {
                    normalized.push(if unit == 13 { 10 } else { unit });
                }
                previous_cr = unit == 13;
            }
            units = normalized;
        } else if self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && matches!(
                self.attribute_in_null_namespace(node, "type")?
                    .unwrap_or("text")
                    .to_ascii_lowercase()
                    .as_str(),
                "text" | "search" | "tel" | "url" | "password"
            )
        {
            units.retain(|unit| !matches!(unit, 10 | 13));
        }
        let value = dom_string::from_units(&units);
        if self.opaque_ranges.has_owner(node) {
            let previous = self.control_value(node)?;
            if previous != value {
                let old_length = u32::try_from(dom_string::len(&previous))
                    .map_err(|_| OpaqueRangeError::InvalidOffset)?;
                let new_length = u32::try_from(dom_string::len(&value))
                    .map_err(|_| OpaqueRangeError::InvalidOffset)?;
                self.opaque_ranges
                    .replace(node, 0, old_length, new_length, old_length)?;
            }
        }
        let record = self.arena.get_mut(node)?;
        record.value = Some(value);
        record.dirty_value = true;
        record.user_edited_value = false;
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
                && record
                    .attributes
                    .get_null("id")
                    .is_some_and(|value| value == id)
            {
                return Ok(Some(node));
            }
            pending.extend(record.children.iter().rev().copied());
        }
        Ok(None)
    }

    pub fn find_by_id(&self, id: &str) -> Option<NodeId> {
        // This convenience API targets the primary document, not every live
        // document allocated in the arena. Tree order is authoritative for
        // duplicate IDs, including after moves and attribute changes.
        self.get_element_by_id(self.document(), id).ok().flatten()
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

    /// DOM connectivity includes inert documents, unlike the active tree used
    /// for rendering and script preparation. Arena parent links include shadow
    /// hosts, so this walks the shadow-including ancestry.
    pub fn is_document_connected(&self, node: NodeId) -> Result<bool, DomError> {
        let mut current = Some(node);
        while let Some(node) = current {
            let record = self.arena.get(node)?;
            if matches!(record.kind, NodeKind::Document) {
                return Ok(true);
            }
            current = record.parent;
        }
        Ok(false)
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
            NodeKind::Attribute { data, .. } => data.name.clone(),
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
        self.arena
            .matches_selector_chain(node, selector, self.focused)
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
            NodeKind::Text { data } => {
                // HTML fragment serialization preserves text under raw-text
                // elements. The namespace guard matters for SVG script/style.
                let raw = record.parent.and_then(|parent| self.arena.get(parent).ok())
                    .is_some_and(|parent| matches!(&parent.kind,
                        NodeKind::Element { name }
                        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                            && matches!(name.local.as_ref(), "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext")));
                Ok(if raw { data.clone() } else { escape_text(data) })
            }
            NodeKind::CDataSection { data } => Ok(format!("<![CDATA[{data}]]>")),
            NodeKind::ProcessingInstruction { target, data } => Ok(format!("<?{target} {data}?>")),
            NodeKind::Comment { data } => Ok(format!("<!--{data}-->")),
            NodeKind::Attribute { data, .. } => Ok(escape_attribute(&data.value)),
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
            pending.extend(record.children.iter().rev().copied());
            pending.extend(self.arena.hosted_document(node));
        }
        Ok(())
    }

    fn mark_connected(&mut self, node: NodeId) -> Result<(), DomError> {
        self.connected.insert(node);
        for child in self.arena.get(node)?.children.clone() {
            self.mark_connected(child)?;
        }
        if let Some(document) = self.arena.hosted_document(node) {
            self.mark_connected(document)?;
            self.mutations
                .push(Mutation::HostedDocument { frame: node });
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
        let was_connected = self.connected.remove(&node);
        if was_connected
            && !preserve_focus
            && let Some(generation) = self.frame_context_generations.get_mut(&node)
        {
            *generation = generation.saturating_add(1);
        }
        if !preserve_focus && self.focused == Some(node) {
            self.focus(None)?;
        }
        for child in self.arena.get(node)?.children.clone() {
            self.mark_disconnected_with_focus(child, preserve_focus)?;
        }
        if let Some(document) = self.arena.hosted_document(node) {
            self.mark_disconnected_with_focus(document, preserve_focus)?;
            self.mutations
                .push(Mutation::HostedDocument { frame: node });
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
            NodeKind::Document => self.arena.document_host(node),
            _ => record.parent,
        })
    }

    fn retention_children(&self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        let mut children = self.arena.get(node)?.children.clone();
        children.extend(self.arena.attribute_node_ids(node));
        children.extend(self.arena.hosted_document(node));
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
        let mut graph_target = matches!(self.arena.get(root)?.kind, NodeKind::Document)
            || self.arena.get(root)?.template_content.is_some()
            || self.internal_targets.contains(&root);
        let mut pending = self.retention_children(root)?;
        while let Some(child) = pending.pop() {
            if self.is_gc_root(child) {
                return Ok(());
            }
            graph_target |= matches!(self.arena.get(child)?.kind, NodeKind::Document)
                || self.arena.get(child)?.template_content.is_some()
                || self.internal_targets.contains(&child);
            pending.extend(self.retention_children(child)?);
        }
        if !graph_target {
            return self.reclaim_unrooted_tree(root);
        }
        // Ownership and internal template/stylesheet relationships are directed
        // graph edges, not permanent native roots. Trace them from real leases
        // so a detached node keeps its Document, but unreachable cycles collect.
        let mut reachable = BTreeSet::new();
        let mut pending = self
            .connected
            .iter()
            .copied()
            .chain(
                self.roots
                    .iter()
                    .filter(|(_, roots)| roots.wrappers > 0 || roots.native > 0)
                    .map(|(node, _)| *node),
            )
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            if !reachable.insert(node) {
                continue;
            }
            let record = self.arena.get(node)?;
            pending.extend(self.retention_parent(node)?);
            pending.extend(self.retention_children(node)?);
            pending.extend(record.owner_document);
            pending.extend(record.associated_inert_document);
            pending.extend(record.template_content);
            pending.extend(record.template_host);
            pending.extend(record.adopted_stylesheets.iter().copied());
        }
        if reachable.contains(&root) {
            return Ok(());
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
        let template_host = self.arena.get(node)?.template_host;
        let inert_document = self.arena.get(node)?.associated_inert_document;
        let _ = self.arena.remove(node)?;
        if let Some(content) = template_content {
            self.reclaim_if_unrooted(content)?;
        }
        for sheet in sheets {
            if self.arena.get(sheet).is_ok() {
                self.reclaim_if_unrooted(sheet)?;
            }
        }
        self.roots.remove(&node);
        self.frame_context_generations.remove(&node);
        self.internal_targets.remove(&node);
        if let Some(host) = template_host
            && self.arena.get(host).is_ok()
        {
            self.reclaim_if_unrooted(host)?;
        }
        if let Some(inert) = inert_document
            && inert != node
            && self.arena.get(inert).is_ok()
        {
            self.reclaim_if_unrooted(inert)?;
        }
        Ok(())
    }
}

fn utf16_len(value: &str) -> u32 {
    value.encode_utf16().count().try_into().unwrap_or(u32::MAX)
}

/// Split descendant compounds without splitting quoted attribute values or
/// functional pseudo-class arguments.
fn selector_compounds(selector: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in selector.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            c if c.is_ascii_whitespace() && depth == 0 => {
                if start < index {
                    parts.push(&selector[start..index]);
                }
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < selector.len() {
        parts.push(&selector[start..]);
    }
    parts
}

/// Find the last explicit combinator outside attributes and pseudo-classes.
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
        if selector[index..].starts_with(":not(") {
            let start = index + 5;
            let Some(end) = selector[start..].find(')') else {
                return false;
            };
            let excluded = &selector[start..start + end];
            if excluded.is_empty() || matches_compound_selector(node, excluded) {
                return false;
            }
            index = start + end + 1;
            continue;
        }
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
                .get_null("id")
                .is_some_and(|value| value == token),
            b'.' => node
                .attributes
                .get_null("class")
                .is_some_and(|value| value.split_ascii_whitespace().any(|class| class == token)),
            b'[' => {
                if let Some((attribute, value)) = token.split_once("*=") {
                    let value = value.trim().trim_matches(['\'', '"']);
                    !value.is_empty()
                        && node
                            .attributes
                            .get_null(attribute.trim())
                            .is_some_and(|actual| actual.contains(value))
                } else if let Some((attribute, value)) = token.split_once('=') {
                    let value = value.trim().trim_matches(['\'', '"']);
                    node.attributes
                        .get_null(attribute.trim())
                        .is_some_and(|actual| actual == value)
                } else {
                    node.attributes.contains_null(token.trim())
                }
            }
            b':' => match token {
                "checked" => matches!(&node.kind, NodeKind::Element { name }
                    if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && ((name.local.as_ref() == "input"
                        && node.attributes.get_null("type").is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio"))
                        && node.checked())
                        || (name.local.as_ref() == "option" && node.selected()))),
                "disabled" => node.attributes.contains_null("disabled"),
                "enabled" => !node.attributes.contains_null("disabled"),
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

    #[test]
    fn frame_context_generation_observes_atomic_moves_and_preserves_move_before() {
        let mut dom = html::parse("<div id=a><iframe id=f></iframe></div><div id=b></div>")
            .into_runtime()
            .unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let frame = dom.find_by_id("f").unwrap();
        assert_eq!(dom.track_frame_context(frame), 1);
        dom.move_before(b, frame, None).unwrap();
        assert_eq!(dom.frame_context_generation(frame), Some(1));
        dom.append_child(a, frame).unwrap();
        assert_eq!(dom.frame_context_generation(frame), Some(2));
        dom.remove(a).unwrap();
        assert_eq!(dom.frame_context_generation(frame), Some(3));
        dom.untrack_frame_context(frame);
        assert_eq!(dom.frame_context_generation(frame), None);
    }

    #[test]
    fn template_ownership_is_inert_shared_nested_and_adopted_without_cloning() {
        let mut dom = html::parse("<template id=a><template id=b><span>kept</span></template></template><template id=c></template>").into_runtime().unwrap();
        let document = dom.document();
        let a = dom.find_by_id("a").unwrap();
        let c = dom.find_by_id("c").unwrap();
        let content = dom.template_content(a).unwrap();
        let inert = dom.arena.node_document(content).unwrap().unwrap();
        assert_ne!(inert, document);
        assert!(
            dom.arena
                .get(inert)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .is_html
        );
        let sibling_content = dom.template_content(c).unwrap();
        assert_eq!(
            dom.arena.node_document(sibling_content).unwrap(),
            Some(inert)
        );
        let nested = dom.arena.get(content).unwrap().children[0];
        let nested_content = dom.template_content(nested).unwrap();
        assert_eq!(dom.arena.node_document(nested).unwrap(), Some(inert));
        assert_eq!(
            dom.arena.node_document(nested_content).unwrap(),
            Some(inert)
        );
        let destination = dom.create(NodeKind::Document);
        dom.retain_wrapper(destination).unwrap();
        dom.retain_wrapper(a).unwrap();
        dom.adopt_node(
            dom.arena.document_id(destination).unwrap(),
            dom.arena.node_ref(a).unwrap(),
        )
        .unwrap();
        let next_inert = dom.arena.node_document(content).unwrap().unwrap();
        assert_ne!(next_inert, inert);
        assert_ne!(next_inert, destination);
        assert_eq!(dom.template_content(a).unwrap(), content);
        assert_eq!(
            dom.arena.node_document(nested_content).unwrap(),
            Some(next_inert)
        );
        assert!(
            !dom.arena
                .get(next_inert)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .is_html
        );
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.release_wrapper(a).unwrap();
        dom.release_wrapper(destination).unwrap();
        for node in [a, content, nested, nested_content, destination, next_inert] {
            assert!(
                dom.arena.get(node).is_err(),
                "unreachable template ownership graph: {node:?}"
            );
        }
        assert!(
            dom.arena.get(inert).is_ok(),
            "live source document retains its associated inert document"
        );
    }

    #[test]
    fn retained_template_content_preserves_host_edge_and_unreachable_cycle_collects() {
        let mut dom = DomRuntime::new();
        let template = dom.create_element("template");
        let content = dom.template_content(template).unwrap();
        dom.retain_wrapper(template).unwrap();
        dom.retain_wrapper(content).unwrap();
        dom.release_wrapper(template).unwrap();
        assert_eq!(
            dom.arena.get(content).unwrap().template_host,
            Some(template)
        );
        assert_eq!(dom.append_child(content, template), Err(DomError::Cycle));
        dom.release_wrapper(content).unwrap();
        assert!(dom.arena.get(content).is_err());
        assert!(dom.arena.get(template).is_err());
    }

    #[test]
    fn template_host_including_cycles_are_rejected_before_mutation_even_after_adoption() {
        let mut dom = DomRuntime::new();
        let outer = dom.create_element("template");
        let inner = dom.create_element("template");
        let outer_content = dom.template_content(outer).unwrap();
        let inner_content = dom.template_content(inner).unwrap();
        let span = dom.create_element("span");
        dom.append_child(outer_content, inner).unwrap();
        dom.append_child(inner_content, span).unwrap();
        dom.retain_wrapper(outer).unwrap();
        let destination = dom.create(NodeKind::Document);
        dom.retain_wrapper(destination).unwrap();
        for adopted in [false, true] {
            if adopted {
                dom.retain_wrapper(outer_content).unwrap();
                dom.adopt_node(
                    dom.arena.document_id(destination).unwrap(),
                    dom.arena.node_ref(outer_content).unwrap(),
                )
                .unwrap();
            }
            for parent in [outer_content, inner_content, span] {
                let cursor = dom.mutation_cursor();
                assert_eq!(dom.append_child(parent, outer), Err(DomError::Cycle));
                assert_eq!(dom.mutation_cursor(), cursor);
                assert_eq!(dom.parent(outer).unwrap(), None);
            }
            assert_eq!(dom.append_child(span, inner), Err(DomError::Cycle));
            assert_eq!(dom.parent(inner).unwrap(), Some(outer_content));
            assert!(
                !dom.arena.ancestors(span).unwrap().contains(&outer),
                "host edge must not become a DOM ancestry edge"
            );
        }
    }

    #[test]
    fn native_adoption_preserves_identity_and_updates_descendant_and_attr_ownership() {
        let mut dom = DomRuntime::new();
        let source_document = dom.document();
        let destination = dom.create(NodeKind::Document);
        dom.retain_native(destination).unwrap();
        let element = dom.create_element("section");
        let child = dom.create_text("kept");
        dom.set_attribute(element, "data-kept", "yes").unwrap();
        let attribute = dom.attribute_node(element, "data-kept").unwrap().unwrap();
        dom.append_child(element, child).unwrap();
        dom.append_child(source_document, element).unwrap();
        dom.set_event_listener_presence(element, "test", true)
            .unwrap();
        dom.retain_wrapper(element).unwrap();
        let point = BoundaryPoint {
            container: child,
            offset: 1,
        };
        let range = dom.create_live_range(point, point).unwrap();
        let identity = dom.arena().node_ref(element).unwrap();
        let document = dom.arena().document_id(destination).unwrap();
        assert_eq!(dom.adopt_node(document, identity).unwrap(), element);
        assert_eq!(dom.arena().node_ref(element).unwrap(), identity);
        assert_eq!(dom.parent(element).unwrap(), None);
        for node in [element, child, attribute] {
            assert_eq!(dom.arena().node_document(node).unwrap(), Some(destination));
        }
        assert!(
            dom.arena()
                .get(element)
                .unwrap()
                .event_listeners
                .contains("test")
        );
        assert_eq!(
            dom.live_range(range).unwrap().start.container,
            source_document
        );
        let foreign = DomRuntime::new();
        assert!(matches!(
            dom.adopt_node(
                document,
                foreign.arena().node_ref(foreign.document()).unwrap()
            ),
            Err(DomError::WrongArena { .. })
        ));
        dom.drop_live_range(range).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.release_native(destination).unwrap();
        assert!(
            dom.arena().get(destination).is_ok(),
            "detached wrapper retains owner document"
        );
        dom.release_wrapper(element).unwrap();
        assert!(dom.arena().get(destination).is_err());
        assert!(dom.arena().get(element).is_err());
    }

    #[test]
    fn owner_document_edges_retain_reachable_documents_without_internal_cycles() {
        let mut dom = DomRuntime::new();
        let document = dom.create(NodeKind::Document);
        let template = dom.create_element("template");
        dom.append_child(document, template).unwrap();
        let content = dom.template_content(template).unwrap();
        let sheet = dom.create_element("style");
        dom.set_node_document(sheet, document).unwrap();
        dom.set_adopted_stylesheets(document, vec![sheet]).unwrap();
        dom.retain_wrapper(document).unwrap();
        dom.release_wrapper(document).unwrap();
        for node in [document, template, content, sheet] {
            assert!(
                dom.arena().get(node).is_err(),
                "unreachable internal cycle: {node:?}"
            );
        }
        let owner = dom.create(NodeKind::Document);
        let detached = dom.create_element("div");
        dom.set_node_document(detached, owner).unwrap();
        dom.retain_native(owner).unwrap();
        dom.retain_wrapper(detached).unwrap();
        dom.release_native(owner).unwrap();
        assert!(dom.arena().get(owner).is_ok());
        dom.release_wrapper(detached).unwrap();
        assert!(dom.arena().get(owner).is_err());
    }

    #[test]
    fn replacing_hosted_document_contents_preserves_other_documents() {
        let mut dom = html::parse("<main id=parent></main><iframe></iframe>")
            .into_runtime()
            .unwrap();
        let parent = dom.find_by_id("parent").unwrap();
        let frame = dom
            .query_selector(dom.document(), "iframe")
            .unwrap()
            .unwrap();
        let child = dom.create(NodeKind::Document);
        dom.set_hosted_document(frame, Some(child)).unwrap();
        dom.replace_document_contents_in(child, "<!doctype html><p id=child>new</p>")
            .unwrap();
        assert_eq!(dom.find_by_id("parent"), Some(parent));
        assert_eq!(dom.find_by_id("child"), None);
        assert!(dom.get_element_by_id(child, "child").unwrap().is_some());
        assert_eq!(dom.arena().hosted_document(frame), Some(child));
        assert_eq!(dom.parent(child).unwrap(), None);
        let cursor = dom.mutation_cursor();
        assert_eq!(
            dom.replace_document_contents_in(parent, "bad"),
            Err(DomError::NotADocument(parent))
        );
        assert_eq!(dom.mutation_cursor(), cursor);
    }

    #[test]
    fn focus_retargets_across_hosted_documents_without_dom_ancestry() {
        let mut dom = DomRuntime::new();
        let outer = dom.create_element("iframe");
        dom.append_child(dom.document(), outer).unwrap();
        let outer_document = dom.create(NodeKind::Document);
        let inner = dom.create_element("iframe");
        dom.append_child(outer_document, inner).unwrap();
        dom.set_hosted_document(outer, Some(outer_document))
            .unwrap();
        let inner_document = dom.create(NodeKind::Document);
        let input = dom.create_element("input");
        dom.append_child(inner_document, input).unwrap();
        dom.set_hosted_document(inner, Some(inner_document))
            .unwrap();
        dom.focus(Some(input)).unwrap();
        assert_eq!(dom.focused_element_in(inner_document).unwrap(), Some(input));
        assert_eq!(dom.focused_element_in(outer_document).unwrap(), Some(inner));
        assert_eq!(dom.focused_element_in(dom.document()).unwrap(), Some(outer));
        assert_eq!(dom.parent(inner_document).unwrap(), None);
        dom.retain_native(input).unwrap();
        dom.set_hosted_document(inner, None).unwrap();
        assert_eq!(dom.focused(), None);
        assert_eq!(dom.focused_element_in(dom.document()).unwrap(), None);
        assert_eq!(dom.focused_element_in(inner_document).unwrap(), None);
        dom.release_native(input).unwrap();
    }

    #[test]
    fn rejected_insertions_preserve_range_endpoints_and_tree_state() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("main");
        let other = dom.create_element("aside");
        let child = dom.create_text("text");
        let foreign = dom.create_element("span");
        dom.append_child(dom.document(), parent).unwrap();
        dom.append_child(dom.document(), other).unwrap();
        dom.append_child(parent, child).unwrap();
        dom.append_child(other, foreign).unwrap();
        let point = BoundaryPoint {
            container: child,
            offset: 2,
        };
        let range = dom.create_live_range(point, point).unwrap();
        let cursor = dom.mutation_cursor();
        assert_eq!(
            dom.insert_before(parent, child, Some(foreign)),
            Err(DomError::NotAChild(foreign, parent))
        );
        assert_eq!(
            dom.insert_before(other, child, Some(child)),
            Err(DomError::NotAChild(child, other))
        );
        assert_eq!(dom.append_child(child, parent), Err(DomError::Cycle));
        assert_eq!(
            dom.live_range(range).unwrap(),
            LiveRange {
                start: point,
                end: point
            }
        );
        assert_eq!(dom.mutation_cursor(), cursor);
        assert_eq!(dom.children(parent).unwrap(), &[child]);
        assert_eq!(dom.children(other).unwrap(), &[foreign]);
        dom.arena_mut().insert_before(child, child).unwrap();
        assert_eq!(dom.children(parent).unwrap(), &[child]);
        dom.drop_live_range(range).unwrap();
    }

    #[test]
    fn hosted_document_cycles_are_rejected_before_mutation() {
        let mut dom = DomRuntime::new();
        let frame = dom.create_element("iframe");
        dom.append_child(dom.document(), frame).unwrap();
        let document = dom.create(NodeKind::Document);
        let child = dom.create_element("main");
        dom.append_child(document, child).unwrap();
        dom.set_hosted_document(frame, Some(document)).unwrap();
        let cursor = dom.mutation_cursor();
        assert_eq!(dom.append_child(child, frame), Err(DomError::Cycle));
        assert_eq!(
            dom.insert_before(document, frame, Some(child)),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.replace_child(document, frame, child),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.arena_mut().append_child(child, frame),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.arena_mut().insert_before(child, frame),
            Err(DomError::Cycle)
        );
        assert_eq!(
            dom.append_child(frame, document),
            Err(DomError::DocumentTreeEdge)
        );
        assert_eq!(dom.mutation_cursor(), cursor);
        assert_eq!(dom.parent(frame).unwrap(), Some(dom.document()));
        assert_eq!(dom.parent(document).unwrap(), None);
        assert_eq!(dom.children(document).unwrap(), &[child]);
        assert_eq!(dom.arena().hosted_document(frame), Some(document));
        assert!(dom.is_connected(child).unwrap());
    }

    #[test]
    fn id_lookup_is_document_scoped_and_tracks_duplicate_tree_order() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("main");
        let second = dom.create_element("aside");
        let frame = dom.create_element("iframe");
        for node in [first, second, frame] {
            dom.append_child(dom.document(), node).unwrap();
        }
        for node in [first, second] {
            dom.set_attribute(node, "id", "shared").unwrap();
        }
        let child_document = dom.create(NodeKind::Document);
        let child = dom.create_element("button");
        dom.set_attribute(child, "id", "shared").unwrap();
        dom.append_child(child_document, child).unwrap();
        dom.set_hosted_document(frame, Some(child_document))
            .unwrap();
        assert_eq!(dom.find_by_id("shared"), Some(first));
        assert_eq!(
            dom.get_element_by_id(child_document, "shared").unwrap(),
            Some(child)
        );
        dom.insert_before(dom.document(), second, Some(first))
            .unwrap();
        assert_eq!(dom.find_by_id("shared"), Some(second));
        dom.set_attribute(second, "id", "other").unwrap();
        assert_eq!(dom.find_by_id("shared"), Some(first));
        dom.remove_attribute(first, "id").unwrap();
        assert_eq!(dom.find_by_id("shared"), None);
        assert_eq!(
            dom.get_element_by_id(child_document, "shared").unwrap(),
            Some(child)
        );
        dom.retain_native(child).unwrap();
        dom.set_hosted_document(frame, None).unwrap();
        // Document-local queries are valid even for a retained retired document.
        assert_eq!(
            dom.get_element_by_id(child_document, "shared").unwrap(),
            Some(child)
        );
        assert_eq!(dom.find_by_id("shared"), None);
        dom.release_native(child).unwrap();
        assert_eq!(dom.find_by_id("other"), Some(second));
        dom.set_attribute(second, "id", "").unwrap();
        assert_eq!(dom.find_by_id(""), None);
    }

    #[test]
    fn hosted_documents_have_separate_tree_and_retention_edges() {
        let mut dom = DomRuntime::new();
        let frame = dom.create_element("iframe");
        dom.append_child(dom.document(), frame).unwrap();
        let document = dom.create(NodeKind::Document);
        let child = dom.create_element("button");
        dom.append_child(document, child).unwrap();
        dom.set_hosted_document(frame, Some(document)).unwrap();
        assert!(dom.children(frame).unwrap().is_empty());
        assert_eq!(dom.parent(document).unwrap(), None);
        assert_eq!(dom.retention_parent(document).unwrap(), Some(frame));
        assert!(dom.is_connected(child).unwrap());
        let snapshot = dom.snapshot().0;
        assert_eq!(snapshot.hosted_document(frame), Some(document));
        dom.retain_native(child).unwrap();
        dom.set_hosted_document(frame, None).unwrap();
        assert!(!dom.is_connected(child).unwrap());
        assert!(dom.arena().get(document).is_ok());
        assert_eq!(dom.retention_parent(document).unwrap(), None);
        dom.release_native(child).unwrap();
        assert!(dom.arena().get(document).is_err());
        assert!(dom.arena().get(child).is_err());
        assert!(dom.arena().get(frame).is_ok());
    }

    #[test]
    fn cloning_and_importing_frames_do_not_copy_hosted_documents() {
        let mut dom = DomRuntime::new();
        let frame = dom.create_element("iframe");
        let fallback = dom.create_text("fallback");
        let document = dom.create(NodeKind::Document);
        let live = dom.create_element("main");
        dom.append_child(dom.document(), frame).unwrap();
        dom.append_child(frame, fallback).unwrap();
        dom.set_hosted_document(frame, Some(document)).unwrap();
        dom.append_child(document, live).unwrap();
        let clone = dom.clone_node(frame, true).unwrap();
        assert_eq!(dom.children(clone).unwrap().len(), 1);
        assert!(matches!(
            dom.arena()
                .get(dom.children(clone).unwrap()[0])
                .unwrap()
                .kind,
            NodeKind::Text { .. }
        ));
        assert_eq!(dom.children(frame).unwrap(), &[fallback]);
        let mut destination = DomRuntime::new();
        let imported = destination.import_subtree(dom.arena(), frame).unwrap();
        assert_eq!(destination.children(imported).unwrap().len(), 1);
        // Importing the Document itself must still copy its actual DOM children.
        let imported_document = destination.import_subtree(dom.arena(), document).unwrap();
        assert_eq!(destination.children(imported_document).unwrap().len(), 1);
        assert!(destination.is_element(
            destination.children(imported_document).unwrap()[0],
            "http://www.w3.org/1999/xhtml",
            "main"
        ));
    }
    use markup5ever::{local_name, ns};

    #[test]
    fn attribute_storage_preserves_insertion_order_and_expanded_names() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("div");
        dom.set_attribute(element, "z", "first").unwrap();
        dom.set_attribute(element, "a", "second").unwrap();
        assert_eq!(
            dom.arena()
                .get(element)
                .unwrap()
                .attributes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["z", "a"]
        );
        dom.set_attribute_ns(element, Some("urn:first".into()), "p:item", "one")
            .unwrap();
        let first = dom.attribute_node(element, "p:item").unwrap().unwrap();
        dom.set_attribute_ns(element, Some("urn:second".into()), "p:item", "two")
            .unwrap();
        assert_eq!(dom.arena().get(element).unwrap().attributes.len(), 4);
        assert!(
            matches!(&dom.arena().get(first).unwrap().kind, NodeKind::Attribute { data, .. } if data.value == "one" && data.namespace.as_deref() == Some("urn:first"))
        );
        dom.set_attribute_ns(element, Some("urn:first".into()), "q:item", "updated")
            .unwrap();
        assert_eq!(dom.attribute_node(element, "p:item").unwrap(), Some(first));
        assert_eq!(dom.attribute_node(element, "q:item").unwrap(), None);
        assert_eq!(
            dom.append_child(element, first),
            Err(DomError::AttributeTreeEdge)
        );
        assert_eq!(
            dom.insert_before(element, first, None),
            Err(DomError::AttributeTreeEdge)
        );
        assert_eq!(
            dom.arena_mut().append_child(first, element),
            Err(DomError::AttributeTreeEdge)
        );
        assert!(dom.arena().get(first).unwrap().parent.is_none());
        let child = dom.create_text("unchanged");
        dom.append_child(element, child).unwrap();
        assert_eq!(
            dom.replace_child(element, first, child),
            Err(DomError::AttributeTreeEdge)
        );
        assert_eq!(dom.arena().get(child).unwrap().parent, Some(element));
        assert!(matches!(&dom.arena().get(first).unwrap().kind,
            NodeKind::Attribute { data, .. } if data.prefix.as_deref() == Some("p") && data.value == "updated"));
        assert_eq!(dom.arena().get(element).unwrap().attributes.len(), 4);
    }

    #[test]
    fn qualified_identity_rejects_other_arenas_snapshots_and_stale_nodes() {
        let mut dom = DomRuntime::new();
        let node = dom.create_text("original");
        let reference = dom.arena().node_ref(node).unwrap();
        let snapshot = dom.arena().clone();
        assert!(dom.arena().resolve(reference).is_ok());
        assert!(matches!(
            snapshot.resolve(reference),
            Err(DomError::WrongArena { .. })
        ));
        assert_ne!(
            dom.arena().document_id(dom.document()).unwrap(),
            snapshot.document_id(dom.document()).unwrap()
        );
        assert!(matches!(
            dom.arena().document_id(node),
            Err(DomError::NotADocument(_))
        ));
        dom.arena_mut().remove(node).unwrap();
        let replacement = dom.create_text("replacement");
        assert_eq!(node.slot, replacement.slot);
        assert!(matches!(
            dom.arena().resolve(reference),
            Err(DomError::StaleNode(_))
        ));
    }

    #[test]
    fn exhausted_node_generations_retire_slots_instead_of_reviving_ids() {
        let mut arena = DomArena::default();
        let id = arena.insert(NodeKind::Text {
            data: "last".into(),
        });
        arena.slots[id.slot as usize].generation = u32::MAX;
        let exhausted = NodeId {
            slot: id.slot,
            generation: u32::MAX,
        };
        arena.remove(exhausted).unwrap();
        let new = arena.insert(NodeKind::Text { data: "new".into() });
        assert_ne!(new.slot, exhausted.slot);
        assert!(arena.get(exhausted).is_err());
        let mut rebuilt = DomArena::prepare_rebuild(&arena, &[]);
        let after_rebuild = rebuilt.insert(NodeKind::Text {
            data: "rebuilt".into(),
        });
        assert_ne!(after_rebuild.slot, exhausted.slot);
    }

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
    fn selected_files_are_native_snapshot_state_across_clone_and_import() {
        let mut dom = html::parse("<input id=f type=file><textarea id=t></textarea>")
            .into_runtime()
            .unwrap();
        let input = dom.find_by_id("f").unwrap();
        let selected = Arc::new(vec![SelectedFile {
            name: "proof.txt".into(),
            media_type: "text/plain".into(),
            bytes: Arc::from(&b"proof"[..]),
            last_modified: 7,
        }]);
        dom.set_selected_files(input, selected.clone()).unwrap();
        let cloned = dom.clone_node(input, true).unwrap();
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(dom.arena(), input).unwrap();
        for files in [
            &dom.arena().get(cloned).unwrap().selected_files,
            &other.arena().get(imported).unwrap().selected_files,
        ] {
            assert!(Arc::ptr_eq(files, &selected));
        }
        dom.set_attribute(input, "type", "FILE").unwrap();
        assert!(Arc::ptr_eq(
            &dom.arena().get(input).unwrap().selected_files,
            &selected
        ));
        dom.remove_attribute(input, "type").unwrap();
        assert!(dom.arena().get(input).unwrap().selected_files.is_empty());
        assert_eq!(
            &*other.arena().get(imported).unwrap().selected_files[0].bytes,
            b"proof"
        );
        let textarea = dom.find_by_id("t").unwrap();
        dom.set_value(textarea, "keep", false).unwrap();
        dom.set_attribute(textarea, "type", "file").unwrap();
        dom.remove_attribute(textarea, "type").unwrap();
        assert_eq!(dom.control_value(textarea).unwrap(), "keep");
        assert!(dom.set_selected_files(textarea, selected).is_err());
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
            matches!(changes.last(), Some(Mutation::ChildrenReplaced { node, added_nodes, removed_nodes, .. }) if *node == root && added_nodes == &[added] && removed_nodes == &[a,b])
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
                    "item".into(),
                    Some("urn:test".into()),
                    None,
                    Some("one".into())
                ),
                (
                    "item".into(),
                    Some("urn:test".into()),
                    Some("one".into()),
                    Some("two".into())
                ),
                (
                    "item".into(),
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
            NodeKind::Attribute { owner: Some(element), data }
                if *element == imported && data.namespace.as_deref() == Some("urn:test") && data.prefix.as_deref() == Some("p") && data.local_name == "FLAG" && data.value == "value")
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
    fn attributes_share_records_and_snapshots_detach_on_write() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("div");
        dom.append_child(dom.document(), element).unwrap();
        dom.set_attribute_ns(element, Some("urn:test".into()), "p:item", "old")
            .unwrap();
        let attribute = dom
            .attribute_node_ns(element, Some("urn:test"), "item")
            .unwrap()
            .unwrap();
        dom.retain_wrapper(attribute).unwrap();
        let shared = dom
            .arena()
            .get(element)
            .unwrap()
            .attributes
            .shared(Some("urn:test"), "item")
            .unwrap();
        assert!(
            matches!(&dom.arena().get(attribute).unwrap().kind, NodeKind::Attribute {data, ..} if Arc::ptr_eq(data, &shared))
        );
        let snapshot = dom.arena().clone();
        dom.set_attribute_node_value(attribute, "new").unwrap();
        assert_eq!(
            snapshot
                .get(element)
                .unwrap()
                .attributes
                .namespaced(Some("urn:test"), "item")
                .unwrap()
                .value,
            "old"
        );
        let updated = dom
            .arena()
            .get(element)
            .unwrap()
            .attributes
            .shared(Some("urn:test"), "item")
            .unwrap();
        assert!(!Arc::ptr_eq(&shared, &updated));
        assert!(
            matches!(&dom.arena().get(attribute).unwrap().kind, NodeKind::Attribute {data, ..} if Arc::ptr_eq(data, &updated))
        );
        dom.remove_attribute_node(element, attribute).unwrap();
        dom.set_attribute_node_value(attribute, "detached").unwrap();
        assert!(
            dom.attribute_ns(element, Some("urn:test"), "item")
                .unwrap()
                .is_none()
        );
        assert_eq!(updated.value, "new");
    }

    #[test]
    fn attribute_nodes_preserve_literal_names_case_and_namespace_updates() {
        let mut dom = DomRuntime::new();
        let element = dom.create_element("root");
        dom.set_attribute(element, "P:FLAG", "plain").unwrap();
        let literal = dom.attribute_node(element, "P:FLAG").unwrap().unwrap();
        assert!(dom.attribute_node(element, "p:flag").unwrap().is_none());
        assert!(matches!(&dom.arena().get(literal).unwrap().kind,
            NodeKind::Attribute { data, .. }
                if data.name == "P:FLAG" && data.local_name == "P:FLAG" && data.prefix.is_none() && data.namespace.is_none()));
        dom.set_attribute_ns(element, Some("urn:test".into()), "n:Value", "first")
            .unwrap();
        let namespaced = dom.attribute_node(element, "n:Value").unwrap().unwrap();
        dom.set_attribute(element, "n:Value", "second").unwrap();
        assert_eq!(
            dom.attribute_node(element, "n:Value").unwrap(),
            Some(namespaced)
        );
        assert!(matches!(&dom.arena().get(namespaced).unwrap().kind,
            NodeKind::Attribute { data, .. }
                if data.local_name == "Value" && data.prefix.as_deref() == Some("n") && data.namespace.as_deref() == Some("urn:test") && data.value == "second"));
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
    fn serialization_preserves_html_raw_text_and_escapes_other_namespaces() {
        let text = "a < b && c > d\u{a0}";
        for tag in [
            "script",
            "style",
            "xmp",
            "iframe",
            "noembed",
            "noframes",
            "plaintext",
        ] {
            let mut dom = DomRuntime::new();
            let element = dom.create_element(tag);
            let child = dom.create_text(text);
            dom.append_child(element, child).unwrap();
            assert_eq!(dom.serialize(child).unwrap(), text);
            assert_eq!(
                dom.serialize(element).unwrap(),
                format!("<{tag}>{text}</{tag}>")
            );
        }
        for (namespace, tag) in [
            ("http://www.w3.org/2000/svg", "script"),
            ("http://www.w3.org/2000/svg", "style"),
            ("http://www.w3.org/1999/xhtml", "p"),
        ] {
            let mut dom = DomRuntime::new();
            let element = dom.create_element_ns(namespace, tag);
            let child = dom.create_text(text);
            dom.append_child(element, child).unwrap();
            assert_eq!(
                dom.serialize(child).unwrap(),
                "a &lt; b &amp;&amp; c &gt; d&nbsp;"
            );
        }
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
                "#app .panel button.primary[data-id='7'][checked]",
            )
            .unwrap()
            .unwrap();
        assert_eq!(dom.node_name(button).unwrap(), "BUTTON");
        assert!(!dom.matches_selector(button, ":checked").unwrap());
        assert!(
            dom.matches_selector(button, "button.action, input")
                .unwrap()
        );
    }

    #[test]
    fn selector_negation_checks_ancestors() {
        let dom = html::parse("<div id=container><button id=target>Try</button></div>")
            .into_runtime()
            .unwrap();
        let target = dom
            .query_selector(dom.document(), "#target")
            .unwrap()
            .unwrap();
        assert!(
            dom.matches_selector(target, "#container:not(.disabled) #target")
                .unwrap()
        );
        assert!(
            !dom.matches_selector(target, "#container:not(#container) #target")
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

#[cfg(test)]
mod opaque_runtime_tests {
    use super::*;

    #[test]
    fn assigned_value_and_ancestor_removal_update_native_ranges() {
        let mut dom = DomRuntime::new();
        let host = dom.create_element("div");
        let control = dom.create_element("textarea");
        dom.append_child(dom.document(), host).unwrap();
        dom.append_child(host, control).unwrap();
        dom.set_value(control, "Hello", false).unwrap();
        let range = dom.create_opaque_range(control, 1, 4).unwrap();
        dom.set_value(control, "Hello", false).unwrap();
        assert_eq!(dom.opaque_range(range).unwrap().start, 1);
        dom.set_value(control, "World", false).unwrap();
        assert_eq!(
            dom.opaque_range(range).unwrap(),
            OpaqueRange {
                owner: Some(control),
                start: 0,
                end: 0
            }
        );
        dom.remove(host).unwrap();
        assert_eq!(
            dom.opaque_range(range).unwrap(),
            OpaqueRange {
                owner: None,
                start: 0,
                end: 0
            }
        );
        dom.drop_opaque_range(range).unwrap();
    }

    #[test]
    fn failed_reparent_does_not_disconnect_a_control_range() {
        let mut dom = DomRuntime::new();
        let host = dom.create_element("div");
        let control = dom.create_element("input");
        let unrelated = dom.create_element("span");
        dom.append_child(dom.document(), host).unwrap();
        dom.append_child(host, control).unwrap();
        dom.set_value(control, "Hello", false).unwrap();
        let range = dom.create_opaque_range(control, 1, 4).unwrap();
        assert!(dom.insert_before(host, control, Some(unrelated)).is_err());
        assert_eq!(
            dom.opaque_range(range).unwrap(),
            OpaqueRange {
                owner: Some(control),
                start: 1,
                end: 4
            }
        );
    }
}
