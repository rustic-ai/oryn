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
pub mod document_refresh;
pub mod document_url;
mod document_urls;
pub mod event_handlers;
mod namespaces;
pub mod render_blocking;
pub mod sanitizer;
pub use render_blocking::RenderBlocker;
mod embedded_content;
mod frame_policy;
mod hover_state;
mod image_intrinsics;
mod interest;
pub mod language;
mod media_state;
mod selector_states;
pub mod webvtt;
pub use embedded_content::EmbeddedContentKind;
pub use image_intrinsics::ImageIntrinsics;
mod output;
mod submit_behavior;
pub use selector_states::valid_custom_element_name;
pub use submit_behavior::SubmitButtonBehavior;
pub mod boundary_journal;
pub mod directionality;
mod document_scope;
mod registry_index;
use registry_index::RegistryIndex;
pub mod dom_string;
pub mod projection_allocation;
use attributes::AttributeKey;
pub use attributes::{AttributeList, ContentAttribute};
mod control_locale;
pub use control_locale::ControlLocale;
mod color_controls;
pub use color_controls::{input_color_components, input_color_rgba8, serialize_input_color};
mod autocomplete;
mod form_controls;
mod numeric_controls;
mod slots;
pub use numeric_controls::parse_html_floating_point;
mod insertion;
mod selected_content;
mod top_layer;
pub use top_layer::{TopLayerEntry, TopLayerKind};
mod file_picker;
mod pattern_controls;
mod temporal_edit;
mod temporal_picker;
mod text_controls;
pub use file_picker::FilePicker;
mod permission_prompt;
pub use permission_prompt::PermissionPrompt;
pub use temporal_edit::{TemporalAction, TemporalEdit, TemporalEditOutcome};
pub use temporal_picker::{TemporalPicker, TemporalPickerItem};
mod style_sets;
mod xml_stylesheets;
pub use style_sets::StylesheetOwnerStateInput;
include!("owned_stylesheet_source.rs");
pub use numeric_controls::NumericValidity;
pub use pattern_controls::PatternMatcher;
pub use xml_stylesheets::{
    XmlStyleSheet, document_xslt_stylesheet, parse_xml_stylesheet, parse_xslt_stylesheet,
};
pub mod html;
mod identity;
mod node_migration;
mod tracked_lease;
pub use tracked_lease::{TrackedNodeLease, drain_tracked_node_leases};
mod clone_tree;
mod node_iterators;
pub use identity::{ArenaId, DocumentId, NodeRef};
pub use node_iterators::{NodeIteratorId, NodeIteratorState, drain_node_iterator_releases};
pub use node_migration::{
    NodeMigration, NodeMigrationPlan, PreparedNodeMigration, drain_node_migration_pins,
};
pub mod opaque_ranges;
mod option_controls;
pub use opaque_ranges::{OpaqueRange, OpaqueRangeError, OpaqueRangeId, OpaqueRangeStore};
mod serialization;
pub mod xml;
mod xml_serialization;
pub mod xpath;
pub use serialization::{
    HtmlSerializationOptions, SerializationResolver, serialize_html_with_options,
    serialize_html_with_resolver,
};
pub use xml_serialization::{
    WellFormedError, serialize_xml_well_formed_with_resolver, serialize_xml_with_resolver,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentReadyState {
    Loading,
    Interactive,
    #[default]
    Complete,
}

impl DocumentReadyState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Interactive => "interactive",
            Self::Complete => "complete",
        }
    }
}

/// Immutable native provenance. The sealed capability is authenticated by the
/// browser parent; author properties and URL strings cannot grant its authority.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DocumentProvenance {
    pub capability: Vec<u8>,
    pub origin: String,
    pub origin_key: String,
    pub inert: bool,
}
impl std::fmt::Debug for DocumentProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentProvenance")
            .field("origin", &self.origin)
            .field("inert", &self.inert)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisteredCustomProperty {
    pub name: String,
    pub syntax: String,
    pub inherits: bool,
    pub initial_value: Option<String>,
    pub base_url: String,
}

/// Native mutable origin-domain component. Cloned Documents retain this cell;
/// strict origin and frozen navigation capabilities are stored independently.
#[derive(Debug, Clone, Default)]
pub struct DocumentDomainCell(pub std::sync::Arc<std::sync::RwLock<Option<String>>>);
impl DocumentDomainCell {
    pub fn get(&self) -> Option<String> {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub fn set(&self, value: Option<String>) {
        *self.0.write().unwrap_or_else(|error| error.into_inner()) = value;
    }
    pub fn is_some(&self) -> bool {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }
}
impl PartialEq for DocumentDomainCell {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0) || self.get() == other.get()
    }
}
impl Eq for DocumentDomainCell {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    /// Optional DOM-access domain; never replaces the strict origin capability.
    pub relaxed_domain: DocumentDomainCell,
    pub registered_custom_properties: Vec<RegisteredCustomProperty>,
    pub provenance: Option<std::sync::Arc<DocumentProvenance>>,
    /// Header-delivered CSP on an active native browsing-context Document.
    pub hide_nonces: bool,
    pub content_language: Option<String>,
    pub pragma_language: Option<String>,
    pub preferred_style_set: Option<String>,
    pub last_style_set: Option<String>,
    pub design_mode: bool,
    /// Successful native CSS image resources for this Document only.
    pub css_image_intrinsics: BTreeMap<String, (u32, u32)>,
    /// Bounded successful native SVG image responses; not authored resource data.
    pub css_svg_resources: Vec<oryn_common::v4::PaintImageResource>,
    pub ready_state: DocumentReadyState,
    pub url: Option<String>,
    /// Referrer of the request that created this Document, never a Window global.
    pub referrer: String,
    /// Creator base retained by about:blank/srcdoc Documents independently of
    /// whichever Document subsequently occupies their Window.
    pub inherited_base_url: Option<String>,
    pub encoding: String,
    pub content_type: String,
    pub is_html: bool,
    pub quirks: bool,
}

impl Default for DocumentMetadata {
    fn default() -> Self {
        Self {
            relaxed_domain: DocumentDomainCell::default(),
            provenance: None,
            hide_nonces: false,
            content_language: None,
            pragma_language: None,
            preferred_style_set: None,
            last_style_set: None,
            design_mode: false,
            css_image_intrinsics: BTreeMap::new(),
            css_svg_resources: Vec::new(),
            registered_custom_properties: Vec::new(),
            ready_state: DocumentReadyState::Complete,
            content_type: "application/xml".into(),
            url: None,
            referrer: String::new(),
            inherited_base_url: None,
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
    /// HTML script scheduling state; never an authored attribute.
    pub script_force_async: bool,
    pub script_already_started: bool,
    /// Last script text accepted by a script-specific setter or the parser.
    /// Compare actual child text at preparation; splitting/normalizing text
    /// does not discard trust when the resulting source is unchanged.
    pub script_trusted_text: String,
    pub script_parser_document: Option<NodeRef>,
    pub parser_open_document: Option<NodeRef>,
    pub cryptographic_nonce: Option<String>,
    pub parser_duplicate_attributes: bool,
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
    pub stylesheet_override: Option<OwnedNativeStylesheetSource>,
    /// Native CSSOM media text, independent of the owner element attribute.
    pub stylesheet_media: Option<String>,
    /// Explicit CSSOM disabled override, separate from owner attributes.
    pub stylesheet_disabled: Option<bool>,
    /// Native CSP state; author stylesheet enablement cannot override this.
    pub stylesheet_policy_allowed: bool,
    pub stylesheet_parser_ready: bool,
    pub style_attribute_policy_allowed: bool,
    pub style_attribute_cssom: bool,
    pub style_attribute_revision: u64,
    /// Native response/construction URL used by CSS parsing and resource resolution.
    pub stylesheet_base_url: Option<String>,
    /// Inert template contents, outside the light DOM tree.
    pub template_content: Option<NodeId>,
    /// Host-including ancestry for template fragments, never parentNode or
    /// connectivity ancestry. Adoption of the fragment preserves this edge.
    pub template_host: Option<NodeId>,
    pub value: Option<String>,
    /// Output value-mode default; None keeps descendant text authoritative.
    pub(crate) output_default: Option<String>,
    pub submit_behavior: Option<SubmitButtonBehavior>,
    /// Native custom-element registry identity: None is not initialized,
    /// Some(None) is an explicitly null scoped association.
    pub custom_element_registry: Option<Option<u64>>,
    /// Declarative shadow construction keeps a null registry through adoption.
    pub custom_element_registry_keep_null: bool,
    /// Creation-time is value, independent of subsequent attribute mutation.
    pub custom_element_is: Option<String>,
    /// Synchronous autonomous construction fallback; authored clones construct afresh.
    pub custom_element_creation_failed: bool,
    /// Captured definition policy; None permits legacy native parser policy.
    pub custom_element_shadow_disabled: Option<bool>,
    /// Script/user assignment differs from native value sanitization.
    pub dirty_value: bool,
    /// Whether the current value was last changed by native user editing.
    pub user_edited_value: bool,
    pub temporal_edit: Option<TemporalEdit>,
    pub temporal_picker: Option<TemporalPicker>,
    pub file_picker: Option<FilePicker>,
    pub permission_prompt: Option<PermissionPrompt>,
    /// HTML user validity is independent of dirty value and survives script assignment.
    pub user_validity: bool,
    pub pending_edit_value: Option<String>,
    pub selected_files: Arc<Vec<SelectedFile>>,
    /// Last selected image-button coordinate; not an authored attribute.
    pub image_coordinates: (i32, i32),
    /// Decoded resource dimensions, separate from authored width/height.
    pub image_dimensions: Option<(u32, u32)>,
    pub image_intrinsics: Option<ImageIntrinsics>,
    /// Selected picture source presentation hints; never authored img attributes.
    pub image_source_size: Option<(Option<u32>, Option<u32>)>,
    /// Native MediaStream frame PNG; never an authored attribute.
    pub media_frame: Option<Vec<u8>>,
    pub media_poster: Option<Vec<u8>>,
    pub media_poster_is_svg: bool,
    /// Native object/embed Document representation; independent of image pixels.
    pub embedded_document_active: bool,
    pub embedded_load_failed: bool,
    pub media_poster_dimensions: Option<(u32, u32)>,
    pub showing_poster: bool,
    /// None means the parser-provided checked attribute is still authoritative.
    /// Radio-group updates may supply an override without making it dirty.
    pub checkedness: Option<bool>,
    pub dirty_checkedness: bool,
    /// Option selectedness is independent of its selected content attribute.
    pub selectedness: Option<bool>,
    pub dirty_selectedness: bool,
    pub custom_validity_message: String,
    pub dialog_return_value: String,
    default_accessibility: BTreeMap<String, String>,
    default_accessibility_references: BTreeMap<String, Vec<NodeId>>,
    explicit_accessibility_references: BTreeMap<String, Vec<NodeId>>,
    accessibility_reference_revision: u64,
    pub indeterminate: bool,
    pub custom_element_defined: bool,
    pub permission_granted: bool,
    pub(crate) interest_source: bool,
    pub(crate) media_selector_state: Option<u8>,
    pub(crate) interest_target: bool,
    pub sensitive_value: bool,
    pub manual_slot_assignment: bool,
    /// Resolved native assignment for immutable semantic/layout snapshots.
    pub native_resolved_slot_nodes: Option<Vec<NodeId>>,
    pub delegates_focus: bool,
    pub shadow_serializable: bool,
    /// Original declarative stylesheet specifiers; absent and authored empty differ.
    pub shadow_adopted_stylesheet_specifiers: Option<String>,
    pub(crate) shadow_reference_target: Option<String>,
    pub declarative_shadow_root: bool,
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    node: Option<Node>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomError {
    #[error("native semantic projection allocation capacity exceeded")]
    SemanticProjectionCapacity,
    #[error("native document projection cancelled by its owner")]
    ProjectionCancelled,
    #[error("native pointer source capacity exceeded")]
    PointerSourceCapacity,
    #[error("hover target belongs to another document")]
    PointerHoverDocumentMismatch,
    #[error("native top-layer capacity exceeded")]
    TopLayerCapacity,
    #[error(
        "state-preserving move requires an eligible node in the same document and connection state"
    )]
    InvalidStatePreservingMove,
    #[error("child text source exceeds native capacity")]
    ChildTextCapacity,
    #[error("DOM migration destination capacity exceeded")]
    MigrationCapacity,
    #[error("DOM migration inputs changed after preparation")]
    MigrationChanged,
    #[error("invalid native UI locale")]
    InvalidUiLocale,
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
    #[error("unknown node iterator {0:?}")]
    UnknownIterator(NodeIteratorId),
    #[error(transparent)]
    OpaqueRange(#[from] OpaqueRangeError),
}

#[derive(Debug)]
pub struct DomArena {
    id: ArenaId,
    allocation_document: Option<NodeId>,
    slots: Vec<Slot>,
    registry_index: std::sync::Mutex<RegistryIndex>,
    free: Vec<u32>,
    attribute_nodes: BTreeMap<(NodeId, AttributeKey), NodeId>,
    stylesheet_revisions: BTreeMap<NodeId, u64>,
    name_attribute_revisions: BTreeMap<NodeId, u64>,
    /// Host associations are projection/lifetime edges, never DOM tree edges.
    hosted_documents: BTreeMap<NodeId, NodeId>,
    retired_documents: BTreeSet<NodeId>,
    // Projection-only generated speech; never authored DOM text or attributes.
    accessibility_generated_content: BTreeMap<NodeId, (String, String)>,
    accessibility_computed_visibility: BTreeMap<NodeId, (bool, bool)>,
    accessibility_computed_text_style: BTreeMap<NodeId, (String, String)>,
    /// Native successful FACE definitions, retained in semantic snapshots.
    form_associated_custom_elements: BTreeSet<NodeId>,
}

impl Default for DomArena {
    fn default() -> Self {
        Self {
            registry_index: Default::default(),
            id: ArenaId::fresh(),
            allocation_document: None,
            slots: Vec::new(),
            free: Vec::new(),
            attribute_nodes: BTreeMap::new(),
            stylesheet_revisions: BTreeMap::new(),
            name_attribute_revisions: BTreeMap::new(),
            hosted_documents: BTreeMap::new(),
            retired_documents: BTreeSet::new(),
            accessibility_generated_content: BTreeMap::new(),
            accessibility_computed_visibility: BTreeMap::new(),
            accessibility_computed_text_style: BTreeMap::new(),
            form_associated_custom_elements: BTreeSet::new(),
        }
    }
}

impl Clone for DomArena {
    fn clone(&self) -> Self {
        // A snapshot can evolve independently. Local NodeIds are preserved for
        // projection, but it must never accept the source arena's live handles.
        let id = ArenaId::fresh();
        let mut slots = self.slots.clone();
        for slot in &mut slots {
            if let Some(record) = slot.node.as_mut()
                && let Some(parser) = record.script_parser_document.as_mut()
                && parser.arena == self.id
            {
                parser.arena = id;
            }
        }
        Self {
            registry_index: Default::default(),
            id,
            allocation_document: self.allocation_document,
            slots,
            free: self.free.clone(),
            attribute_nodes: self.attribute_nodes.clone(),
            stylesheet_revisions: self.stylesheet_revisions.clone(),
            name_attribute_revisions: self.name_attribute_revisions.clone(),
            hosted_documents: self.hosted_documents.clone(),
            retired_documents: self.retired_documents.clone(),
            accessibility_generated_content: self.accessibility_generated_content.clone(),
            accessibility_computed_visibility: self.accessibility_computed_visibility.clone(),
            accessibility_computed_text_style: self.accessibility_computed_text_style.clone(),
            form_associated_custom_elements: self.form_associated_custom_elements.clone(),
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
            accessibility_generated_content: BTreeMap::new(),
            accessibility_computed_visibility: BTreeMap::new(),
            accessibility_computed_text_style: BTreeMap::new(),
            form_associated_custom_elements: BTreeSet::new(),
            registry_index: Default::default(),
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
        insertion::DocumentAssignment::new(self.id(), root, document).resume(self)
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
        self.initialize_preferred_style_set(child)?;
        self.process_language_pragmas(child)?;
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
        self.initialize_preferred_style_set(node)?;
        self.process_language_pragmas(node)?;
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
        self.form_associated_custom_elements.remove(&id);
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
        self.ancestors_with_checkpoint(id, &mut || Ok(()))
    }
    pub fn ancestors_with_checkpoint(
        &self,
        id: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Vec<NodeId>, DomError> {
        checkpoint()?;
        let mut result = Vec::new();
        let mut current = self.get(id)?.parent;
        let mut seen = BTreeSet::from([id]);
        while let Some(node) = current {
            checkpoint()?;
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
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

    /// Advance a caller-owned native-slot cursor, checking omitted slots too.
    /// No checkpoint borrow is retained, so nested work can use the same
    /// caller authority. A failed check leaves that slot pending for retry.
    pub fn next_with_checkpoint<'a>(
        &'a self,
        cursor: &mut usize,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<(NodeId, &'a Node)>, DomError> {
        *cursor = (*cursor).min(self.slots.len());
        loop {
            checkpoint()?;
            let index = *cursor;
            let Some(slot) = self.slots.get(index) else {
                return Ok(None);
            };
            // A valid slice index is strictly below len, hence increment
            // cannot overflow usize even for an externally supplied cursor.
            *cursor = index + 1;
            if let Some(node) = &slot.node {
                return Ok(Some((
                    NodeId {
                        slot: index as u32,
                        generation: slot.generation,
                    },
                    node,
                )));
            }
        }
    }

    /// Source eligibility, independent of connection, loading and disabled state.
    pub fn is_css_stylesheet_owner(&self, id: NodeId) -> Result<bool, DomError> {
        let node = self.get(id)?;
        if matches!(node.kind, NodeKind::ProcessingInstruction { .. }) {
            return Ok(self.xml_stylesheet(id)?.is_some());
        }
        let NodeKind::Element { name } = &node.kind else {
            return Ok(false);
        };
        let html = name.ns.as_ref() == "http://www.w3.org/1999/xhtml";
        let svg = name.ns.as_ref() == "http://www.w3.org/2000/svg";
        if !node
            .attributes
            .get_null("type")
            .is_none_or(|value| value.is_empty() || value.eq_ignore_ascii_case("text/css"))
        {
            return Ok(false);
        }
        Ok(((html || svg) && name.local.as_ref() == "style")
            || (html
                && name.local.as_ref() == "link"
                && node.attributes.get_null("rel").is_some_and(|value| {
                    value
                        .split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                })
                && node
                    .attributes
                    .get_null("href")
                    .is_some_and(|value| !value.trim().is_empty())))
    }

    /// Effective source consumed by the style engine after CSSOM edits.
    pub fn stylesheet_text(&self, id: NodeId) -> Result<String, DomError> {
        match &self.get(id)?.stylesheet_override {
            Some(source) => Ok(source.as_ref().to_owned()),
            None => self.text_content(id),
        }
    }

    pub fn stylesheet_media(&self, id: NodeId) -> Result<String, DomError> {
        let node = self.get(id)?;
        if node.stylesheet_media.is_none()
            && let Some(sheet) = self.xml_stylesheet(id)?
        {
            return Ok(sheet.media);
        }
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
        insertion::StyleInvalidation::subtree(self, root)?.resume(self)
    }

    fn invalidate_stylesheet_ancestors(&mut self, node: Option<NodeId>) -> Result<(), DomError> {
        insertion::StyleInvalidation::ancestors(self, node)?.resume(self)
    }

    /// Character data for a character-data node; otherwise descendant text
    /// in light-tree order. Document callers use this internal extraction
    /// helper; the public Document.textContent getter returns null.
    pub fn text_content(&self, id: NodeId) -> Result<String, DomError> {
        self.text_content_with_checkpoint(id, &mut || Ok(()))
    }
    pub fn text_content_with_checkpoint(
        &self,
        id: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<String, DomError> {
        self.text_content_with_allocation(
            id,
            &mut crate::projection_allocation::OrdinaryProjectionAllocation,
            checkpoint,
        )
    }

    fn slot(&self, id: NodeId) -> Option<&Slot> {
        self.slots
            .get(id.slot as usize)
            .filter(|slot| slot.generation == id.generation)
    }
    fn slot_mut(&mut self, id: NodeId) -> Option<&mut Slot> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        // Node fields are public: every mutable borrow can change a registry,
        // even when it did not originate in set_custom_element_registry.
        self.registry_index
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner())
            .mark_dirty(id);
        Some(slot)
    }
}

fn new_node(kind: NodeKind) -> Node {
    let document_metadata =
        matches!(kind, NodeKind::Document).then(|| Box::new(DocumentMetadata::default()));
    let script_force_async = matches!(&kind, NodeKind::Element { name }
        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "script");
    Node {
        script_force_async,
        script_already_started: false,
        script_trusted_text: String::new(),
        script_parser_document: None,
        parser_open_document: None,
        cryptographic_nonce: None,
        parser_duplicate_attributes: false,
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
        stylesheet_disabled: None,
        stylesheet_policy_allowed: true,
        stylesheet_parser_ready: true,
        style_attribute_policy_allowed: true,
        style_attribute_cssom: false,
        style_attribute_revision: 0,
        stylesheet_base_url: None,
        template_content: None,
        template_host: None,
        value: None,
        output_default: None,
        submit_behavior: None,
        custom_element_registry: None,
        custom_element_registry_keep_null: false,
        custom_element_is: None,
        custom_element_creation_failed: false,
        custom_element_shadow_disabled: None,
        dirty_value: false,
        user_edited_value: false,
        temporal_edit: None,
        temporal_picker: None,
        file_picker: None,
        permission_prompt: None,
        user_validity: false,
        pending_edit_value: None,
        selected_files: Arc::new(Vec::new()),
        image_coordinates: (0, 0),
        image_dimensions: None,
        image_intrinsics: None,
        image_source_size: None,
        media_frame: None,
        media_poster: None,
        media_poster_is_svg: false,
        embedded_document_active: false,
        embedded_load_failed: false,
        media_poster_dimensions: None,
        showing_poster: true,
        checkedness: None,
        dirty_checkedness: false,
        selectedness: None,
        dirty_selectedness: false,
        custom_validity_message: String::new(),
        dialog_return_value: String::new(),
        default_accessibility: BTreeMap::new(),
        default_accessibility_references: BTreeMap::new(),
        explicit_accessibility_references: BTreeMap::new(),
        accessibility_reference_revision: 0,
        indeterminate: false,
        custom_element_defined: false,
        permission_granted: false,
        media_selector_state: None,
        interest_source: false,
        interest_target: false,
        sensitive_value: false,
        manual_slot_assignment: false,
        native_resolved_slot_nodes: None,
        delegates_focus: false,
        shadow_serializable: false,
        shadow_adopted_stylesheet_specifiers: None,
        shadow_reference_target: None,
        declarative_shadow_root: false,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationSiblings {
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    /// Native moveBefore provenance; serialized observations cannot forge it.
    #[serde(skip)]
    pub state_preserving_move: bool,
}

/// Native custody for exactly one removal. Serialization intentionally drops
/// authority; cloned in-process batches share the consumed bit.
#[derive(Debug, Clone, Default)]
pub struct MutationRemovalLease(Option<Arc<std::sync::atomic::AtomicBool>>);
impl PartialEq for MutationRemovalLease {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for MutationRemovalLease {}
impl MutationRemovalLease {
    fn new() -> Self {
        Self(Some(Arc::new(std::sync::atomic::AtomicBool::new(false))))
    }
    fn consumed(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|state| state.load(std::sync::atomic::Ordering::Relaxed))
    }
    fn consume(&self) {
        if let Some(state) = &self.0 {
            state.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn same(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
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
        #[serde(skip)]
        lease: MutationRemovalLease,
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
        /// Native adoption provenance; initial owner assignment must not rebind
        /// an explicitly null custom-element registry.
        #[serde(skip)]
        previous_document: Option<NodeId>,
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

include!("prepared_stylesheet_metadata.rs");
include!("prepared_stylesheet_source.rs");

struct StylesheetJournalCharge {
    bytes: usize,
    _lease: Arc<dyn Send + Sync>,
}
impl std::fmt::Debug for StylesheetJournalCharge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StylesheetJournalCharge")
            .field("bytes", &self.bytes)
            .finish()
    }
}
#[derive(Debug)]
struct MutationJournal {
    stylesheet_capacity_charge: Option<StylesheetJournalCharge>,
    entries: VecDeque<(u64, Mutation)>,
    next: u64,
    observer: MutationCursor,
    discarded_removed_roots: Vec<(NodeId, MutationRemovalLease)>,
    // Custody survives observer draining and bounded entry eviction.
    outstanding_removed_roots: BTreeMap<NodeId, u32>,
    removed_leases: BTreeMap<NodeId, Vec<MutationRemovalLease>>,
}

impl Default for MutationJournal {
    fn default() -> Self {
        Self {
            stylesheet_capacity_charge: None,
            entries: VecDeque::new(),
            next: 1,
            observer: MutationCursor::default(),
            discarded_removed_roots: Vec::new(),
            outstanding_removed_roots: BTreeMap::new(),
            removed_leases: BTreeMap::new(),
        }
    }
}

impl MutationJournal {
    const MAX_ENTRIES: usize = 16_384;

    fn push(&mut self, mut mutation: Mutation) {
        if let Mutation::Removed { node, lease, .. } = &mut mutation {
            *lease = MutationRemovalLease::new();
            self.removed_leases
                .entry(*node)
                .or_default()
                .push(lease.clone());
            *self.outstanding_removed_roots.entry(*node).or_default() += 1;
        }
        let sequence = self.next;
        self.next = self.next.saturating_add(1);
        self.entries.push_back((sequence, mutation));
        while self.entries.len() > Self::MAX_ENTRIES {
            if let Some((sequence, Mutation::Removed { node, lease, .. })) =
                self.entries.pop_front()
                && sequence > self.observer.0
            {
                // The bounded journal no longer exposes this entry, but its
                // lease must still be released at a safe delivery checkpoint.
                // Entries already drained transfer that lease to their batch.
                self.discarded_removed_roots.push((node, lease));
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

/// A native, callback-free registry admission checkpoint. It must be restored
/// only before exposing the staged associations to author code.
pub struct CustomRegistryAdmission {
    arena: ArenaId,
    associations: Vec<(NodeId, Option<Option<u64>>)>,
    documents: BTreeMap<(u64, NodeId), u64>,
}

/// Numeric diagnostic counters only; constructing this DTO allocates nothing.
#[derive(Debug, serde::Serialize)]
pub struct DomMemorySnapshot {
    pub arena_slots: usize,
    pub arena_slot_capacity: usize,
    pub arena_occupied_nodes: usize,
    pub character_capacity_bytes: usize,
    pub live_ranges: usize,
    pub mutation_revision: u64,
    pub journal_entries: usize,
    pub discarded_removal_roots: usize,
    pub outstanding_removal_roots: usize,
    pub removal_lease_count: usize,
}

/// Live leases, iterators and mutation consumers belong to one runtime.
/// Use `snapshot` and `from_parsed` to create independent document state.
#[derive(Debug, Default)]
pub struct DomRuntime {
    render_blocking: render_blocking::State,
    parser_shadow_disabled: BTreeMap<NodeId, BTreeSet<String>>,
    global_custom_element_registries: BTreeSet<u64>,
    custom_registry_documents: BTreeMap<(u64, NodeId), u64>,
    native_ui_locale: Option<String>,
    control_locales: BTreeMap<String, ControlLocale>,
    top_layer: Vec<TopLayerEntry>,
    popover_transitions: BTreeSet<NodeId>,
    popover_pointer_down: Option<Option<NodeId>>,
    selected_content_updating: bool,
    slot_elements: BTreeSet<NodeId>,
    slot_assignment_snapshots: BTreeMap<NodeId, Vec<NodeId>>,
    pending_slot_changes: Vec<NodeId>,
    manual_slot_nodes: BTreeMap<NodeId, Vec<NodeId>>,
    manual_node_slots: BTreeMap<NodeId, NodeId>,
    pattern_matcher: Option<Rc<dyn PatternMatcher>>,
    arena: DomArena,
    document: Option<NodeId>,
    connected: BTreeSet<NodeId>,
    frame_context_generations: BTreeMap<NodeId, u64>,
    frame_navigation_identities: BTreeMap<NodeId, std::sync::Arc<()>>,
    roots: BTreeMap<NodeId, NodeRoots>,
    pending_cursor_releases: Vec<NodeId>,
    pending_tree_tails: VecDeque<crate::insertion::CommittedTreeTail>,
    draining_tree_tails: bool,
    pending_stylesheet_reclaims: BTreeSet<NodeId>,
    /// Conservative targets of internal non-tree edges. Used only to avoid
    /// full graph tracing for ordinary detached tree components.
    internal_targets: BTreeSet<NodeId>,
    mutations: MutationJournal,
    xpath_revisions: BTreeMap<NodeId, Rc<()>>,
    pointer_hover_targets: BTreeMap<(NodeId, i32), NodeId>,
    focused: Option<NodeId>,
    focus_ring_suppressed: bool,
    next_range: u64,
    ranges: BTreeMap<LiveRangeId, LiveRange>,
    range_endpoint_revision: u64,
    composed_selection_ranges: BTreeSet<LiveRangeId>,
    range_shadow_removals: BTreeMap<LiveRangeId, u64>,
    next_iterator: u64,
    node_iterators: BTreeMap<NodeIteratorId, Rc<node_iterators::NodeIteratorRecord>>,
    boundary_journal: boundary_journal::Journal,
    opaque_ranges: OpaqueRangeStore,
    radio_form_owners: BTreeMap<NodeId, Option<NodeId>>,
    /// Native validity flags for registered form-associated custom elements.
    custom_element_validity: BTreeMap<NodeId, BTreeSet<String>>,
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
        self.slot_assignment_snapshots.clear();
        self.pending_slot_changes.clear();
        self.top_layer.clear();
        self.popover_transitions.clear();
        self.popover_pointer_down = None;
        self.selected_content_updating = false;
        self.manual_slot_nodes.clear();
        self.manual_node_slots.clear();
        self.xpath_revisions.clear();
        self.render_blocking = Default::default();
        self.retire_node_iterators();
        self.arena = arena;
        self.document = Some(document);
        self.connected.clear();
        self.frame_context_generations.clear();
        self.frame_navigation_identities.clear();
        self.roots.clear();
        self.pending_tree_tails.clear();
        self.draining_tree_tails = false;
        self.pending_cursor_releases.clear();
        self.pending_stylesheet_reclaims.clear();
        self.internal_targets.clear();
        self.index_internal_targets();
        self.mutations.clear();
        self.pointer_hover_targets.clear();
        self.focused = None;
        self.focus_ring_suppressed = false;
        self.next_range = 0;
        self.note_range_endpoint_change();
        self.ranges.clear();
        self.composed_selection_ranges.clear();
        self.range_shadow_removals.clear();
        self.next_iterator = 0;
        self.boundary_journal = boundary_journal::Journal::default();
        self.opaque_ranges = OpaqueRangeStore::default();
        self.radio_form_owners.clear();
        self.arena.form_associated_custom_elements.clear();
        self.custom_element_validity.clear();
        self.rebuild_document_indexes()?;
        self.initialize_checkedness()?;
        self.initialize_selects()?;
        self.mutations.clear();
        Ok(())
    }

    pub fn arena(&self) -> &DomArena {
        &self.arena
    }
    /// Payload-free native diagnostics. Scans slots without cloning node data;
    /// embeddings must sample sparsely, never use this as execution authority.
    pub fn diagnostic_memory_snapshot(&self) -> DomMemorySnapshot {
        let mut occupied = 0usize;
        let mut character_capacity = 0usize;
        for slot in &self.arena.slots {
            let Some(node) = &slot.node else { continue };
            occupied += 1;
            let bytes = match &node.kind {
                NodeKind::Text { data }
                | NodeKind::CDataSection { data }
                | NodeKind::Comment { data } => data.capacity(),
                NodeKind::ProcessingInstruction { target, data } => {
                    target.capacity().saturating_add(data.capacity())
                }
                _ => 0,
            };
            character_capacity = character_capacity.saturating_add(bytes);
        }
        DomMemorySnapshot {
            arena_slots: self.arena.slots.len(),
            arena_slot_capacity: self.arena.slots.capacity(),
            arena_occupied_nodes: occupied,
            character_capacity_bytes: character_capacity,
            live_ranges: self.ranges.len(),
            mutation_revision: self.mutation_cursor().0,
            journal_entries: self.mutations.entries.len(),
            discarded_removal_roots: self.mutations.discarded_removed_roots.len(),
            outstanding_removal_roots: self.mutations.outstanding_removed_roots.len(),
            removal_lease_count: self.mutations.removed_leases.values()
                .fold(0usize, |total, leases| total.saturating_add(leases.len())),
        }
    }
    pub fn arena_mut(&mut self) -> &mut DomArena {
        &mut self.arena
    }

    pub fn node_document(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        self.arena.node_document(node)
    }

    /// Ordinary reparenting discards a browsing context even when removal and
    /// insertion happen in one call; state-preserving moveBefore does not.
    pub fn track_frame_context(&mut self, frame: NodeId) -> u64 {
        *self.frame_context_generations.entry(frame).or_insert(1)
    }

    pub fn frame_navigation_identity(&mut self, frame: NodeId) -> std::sync::Arc<()> {
        self.frame_navigation_identities
            .entry(frame)
            .or_default()
            .clone()
    }
    pub fn frame_navigation_identity_is_current(
        &self,
        frame: NodeId,
        identity: &std::sync::Arc<()>,
    ) -> bool {
        self.frame_navigation_identities
            .get(&frame)
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, identity))
    }
    pub fn invalidate_frame_navigation_identity(&mut self, frame: NodeId) {
        if let Some(identity) = self.frame_navigation_identities.get_mut(&frame) {
            *identity = std::sync::Arc::new(());
        }
    }
    pub fn frame_context_generation(&self, frame: NodeId) -> Option<u64> {
        self.frame_context_generations.get(&frame).copied()
    }

    pub fn untrack_frame_context(&mut self, frame: NodeId) {
        self.frame_context_generations.remove(&frame);
    }

    fn index_internal_targets(&mut self) {
        self.index_slot_elements();
        for (_, record) in self.arena.iter() {
            self.internal_targets.extend(record.template_content);
            self.internal_targets.extend(
                record
                    .default_accessibility_references
                    .values()
                    .flatten()
                    .copied(),
            );
            self.internal_targets.extend(
                record
                    .explicit_accessibility_references
                    .values()
                    .flatten()
                    .copied(),
            );
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

    /// Fresh clone/import ownership is initialization, not adoption. Preserve
    /// copied registry slots until the caller applies cloning's fallback rules.
    pub fn initialize_cloned_node_document(
        &mut self,
        node: NodeId,
        document: NodeId,
    ) -> Result<(), DomError> {
        self.arena.document_id(document)?;
        let previous = self.ownership_before_adoption(node, Some(document))?;
        self.arena.assign_node_document(node, document)?;
        for (node, _) in previous {
            self.note_custom_registry_document(node)?;
            self.record_mutation(Mutation::DocumentOwnership {
                node,
                previous_document: None,
            });
        }
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
        insertion::OwnershipPublication::new(self.arena.id(), previous).resume(self)
    }

    /// Registry classification is installed by the native binding realm; no
    /// authored property can turn a scoped registry into a global registry.
    pub fn register_global_custom_element_registry(&mut self, id: u64) {
        self.global_custom_element_registries.insert(id);
    }
    pub fn has_non_global_shadow_registry(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(match self.arena.get(node)?.custom_element_registry {
            None => false,
            Some(None) => true,
            Some(Some(id)) => !self.global_custom_element_registries.contains(&id),
        })
    }

    /// First association order is shared across native arenas in this process.
    /// Entries do not root Documents; stale native identities are ignored.
    pub fn note_custom_registry_document(&mut self, node: NodeId) -> Result<(), DomError> {
        let record = self.arena.get(node)?;
        if !matches!(
            record.kind,
            NodeKind::Document | NodeKind::Element { .. } | NodeKind::ShadowRoot { .. }
        ) {
            return Ok(());
        }
        let Some(registry) = record.custom_element_registry.flatten() else {
            return Ok(());
        };
        let Some(document) = self.arena.node_document(node)? else {
            return Ok(());
        };
        if self
            .custom_registry_documents
            .contains_key(&(registry, document))
        {
            return Ok(());
        }
        static NEXT_ASSOCIATION: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let sequence = NEXT_ASSOCIATION
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| value.checked_add(1),
            )
            .map_err(|_| DomError::MigrationCapacity)?;
        self.custom_registry_documents
            .insert((registry, document), sequence);
        Ok(())
    }
    pub fn checkpoint_custom_registry_admission(
        &self,
        nodes: &[NodeId],
    ) -> Result<CustomRegistryAdmission, DomError> {
        let associations = nodes
            .iter()
            .map(|node| Ok((*node, self.arena.get(*node)?.custom_element_registry)))
            .collect::<Result<Vec<_>, DomError>>()?;
        Ok(CustomRegistryAdmission {
            arena: self.arena.id(),
            associations,
            documents: self.custom_registry_documents.clone(),
        })
    }
    pub fn rollback_custom_registry_admission(&mut self, checkpoint: CustomRegistryAdmission) {
        assert_eq!(
            self.arena.id(),
            checkpoint.arena,
            "registry admission arena"
        );
        // No native tree mutation or author callback occurs during admission.
        for (node, association) in checkpoint.associations {
            self.arena
                .get_mut(node)
                .expect("retained registry admission node")
                .custom_element_registry = association;
        }
        self.custom_registry_documents = checkpoint.documents;
    }
    pub fn set_custom_element_registry(
        &mut self,
        node: NodeId,
        registry: Option<u64>,
    ) -> Result<(), DomError> {
        let previous = self.arena.get(node)?.custom_element_registry;
        self.arena.get_mut(node)?.custom_element_registry = Some(registry);
        if let Err(error) = self.note_custom_registry_document(node) {
            self.arena.get_mut(node)?.custom_element_registry = previous;
            return Err(error);
        }
        Ok(())
    }
    pub fn custom_registry_documents(&self, registry: u64) -> Vec<(u64, NodeId)> {
        self.custom_registry_documents
            .iter()
            .filter_map(|((id, document), sequence)| {
                (*id == registry
                    && matches!(
                        self.arena.get(*document).map(|node| &node.kind),
                        Ok(NodeKind::Document)
                    ))
                .then_some((*sequence, *document))
            })
            .collect()
    }

    fn rebind_adopted_custom_registry(
        &mut self,
        node: NodeId,
        root: Option<NodeId>,
    ) -> Result<(), DomError> {
        let record = self.arena.get(node)?;
        let association = record.custom_element_registry.flatten();
        let global =
            association.is_some_and(|id| self.global_custom_element_registries.contains(&id));
        if association.is_some() && !global {
            return Ok(());
        }
        let document = self.arena.node_document(node)?;
        let document_registry_slot = document
            .and_then(|document| self.arena.get(document).ok())
            .and_then(|record| record.custom_element_registry);
        if record.custom_element_registry.is_none()
            && document_registry_slot.is_none()
            && record
                .parent
                .filter(|_| Some(node) != root)
                .and_then(|parent| self.arena.get(parent).ok())
                .and_then(|record| record.custom_element_registry)
                .is_none()
        {
            // Parser/import initialization before a binding realm is installed
            // must not manufacture an explicit null registry association.
            return Ok(());
        }
        let document_registry = document_registry_slot.flatten();
        let registry = match record.kind {
            NodeKind::ShadowRoot { .. } => {
                if association.is_none() && record.custom_element_registry_keep_null {
                    return Ok(());
                }
                document_registry
            }
            NodeKind::Element { .. } => {
                let parent = record.parent.filter(|_| Some(node) != root);
                if association.is_some()
                    || parent.is_none()
                    || parent.is_some_and(|parent| {
                        matches!(
                            self.arena.get(parent).map(|record| &record.kind),
                            Ok(NodeKind::DocumentFragment)
                        )
                    })
                {
                    document_registry
                } else {
                    parent
                        .and_then(|parent| self.arena.get(parent).ok())
                        .and_then(|record| record.custom_element_registry.flatten())
                }
            }
            _ => return Ok(()),
        };
        self.arena.get_mut(node)?.custom_element_registry =
            Some(registry.filter(|id| self.global_custom_element_registries.contains(id)));
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
        if self.embedded_content_kind(frame).is_none() {
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
        self.record_mutation(Mutation::HostedDocument { frame });
        Ok(())
    }

    pub fn set_embedded_document_active(
        &mut self,
        node: NodeId,
        active: bool,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.embedded_document_active != active {
            record.embedded_document_active = active;
            self.record_mutation(Mutation::ImageResource { node });
        }
        Ok(())
    }
    pub fn set_media_poster(
        &mut self,
        node: NodeId,
        png: Option<Vec<u8>>,
        svg: bool,
        dimensions: Option<(u32, u32)>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        record.media_poster = png;
        record.media_poster_is_svg = svg;
        record.media_poster_dimensions = dimensions;
        self.record_mutation(Mutation::ImageResource { node });
        Ok(())
    }
    pub fn set_showing_poster(&mut self, node: NodeId, showing: bool) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.showing_poster != showing {
            record.showing_poster = showing;
            self.record_mutation(Mutation::ImageResource { node });
        }
        Ok(())
    }
    pub fn set_media_frame(&mut self, node: NodeId, png: Option<Vec<u8>>) -> Result<(), DomError> {
        self.arena.get_mut(node)?.media_frame = png;
        self.record_mutation(Mutation::ImageResource { node });
        Ok(())
    }

    pub fn set_image_source_size(
        &mut self,
        node: NodeId,
        size: Option<(Option<u32>, Option<u32>)>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.image_source_size != size {
            record.image_source_size = size;
            self.record_mutation(Mutation::ImageResource { node });
        }
        Ok(())
    }

    pub fn set_image_intrinsics(
        &mut self,
        node: NodeId,
        intrinsics: Option<ImageIntrinsics>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.image_intrinsics != intrinsics {
            record.image_intrinsics = intrinsics;
            self.record_mutation(Mutation::ImageResource { node });
        }
        Ok(())
    }

    pub fn set_image_dimensions(
        &mut self,
        node: NodeId,
        dimensions: Option<(u32, u32)>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.image_dimensions != dimensions
            || dimensions.is_none() && record.image_intrinsics.is_some()
        {
            if dimensions.is_none() {
                record.image_intrinsics = None;
            }
            record.image_dimensions = dimensions;
            self.record_mutation(Mutation::ImageResource { node });
        }
        Ok(())
    }

    /// Native image completion publishes dimensions independently of author
    /// attributes. The owning request service validates URLs, lifetime and size.
    /// Script registrations are document-owned, survive style-sheet replacement,
    /// and invalidate the same consumers as a native stylesheet mutation.
    pub fn register_custom_property(
        &mut self,
        document: NodeId,
        registration: RegisteredCustomProperty,
    ) -> Result<bool, DomError> {
        let record = self.arena.get_mut(document)?;
        let metadata = record
            .document_metadata
            .get_or_insert_with(|| Box::new(DocumentMetadata::default()));
        if metadata
            .registered_custom_properties
            .iter()
            .any(|entry| entry.name == registration.name)
        {
            return Ok(false);
        }
        metadata.registered_custom_properties.push(registration);
        self.record_mutation(Mutation::StyleSheets { node: document });
        Ok(true)
    }

    pub fn set_css_svg_resources(
        &mut self,
        document: NodeId,
        resources: Vec<oryn_common::v4::PaintImageResource>,
    ) -> Result<(), DomError> {
        let metadata = self
            .arena
            .get_mut(document)?
            .document_metadata
            .get_or_insert_with(|| Box::new(DocumentMetadata::default()));
        if metadata.css_svg_resources != resources {
            metadata.css_svg_resources = resources;
            self.record_mutation(Mutation::ImageResource { node: document });
        }
        Ok(())
    }

    pub fn set_css_image_intrinsics(
        &mut self,
        document: NodeId,
        images: BTreeMap<String, (u32, u32)>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(document)?;
        let metadata = record
            .document_metadata
            .get_or_insert_with(|| Box::new(DocumentMetadata::default()));
        if metadata.css_image_intrinsics != images {
            metadata.css_image_intrinsics = images;
            self.record_mutation(Mutation::ImageResource { node: document });
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
        self.register_slot_element(node);
        self.record_mutation(Mutation::Inserted {
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

    pub fn script_already_started(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(self.arena.get(node)?.script_already_started)
    }

    pub fn mark_script_started(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get_mut(node)?.script_already_started = true;
        Ok(())
    }

    pub fn mark_parser_script(&mut self, node: NodeId) -> Result<(), DomError> {
        let document = self.arena.node_document(node)?.unwrap_or(self.document());
        self.mark_parser_script_in(node, document)
    }
    pub fn mark_parser_script_in(
        &mut self,
        node: NodeId,
        document: NodeId,
    ) -> Result<(), DomError> {
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "script")
            && !self.is_element(node, "http://www.w3.org/2000/svg", "script")
        {
            return Ok(());
        }
        self.arena.get(document)?;
        let identity = NodeRef {
            arena: self.arena.id(),
            node: document,
        };
        let record = self.arena.get_mut(node)?;
        record.script_force_async = false;
        record.script_parser_document = Some(identity);
        Ok(())
    }
    /// A parser script moved before preparation is consumed without fetch,
    /// evaluation or load/error publication, including if it later moves back.
    pub fn reject_moved_parser_script(&mut self, node: NodeId) -> Result<bool, DomError> {
        if self.arena.get(node)?.script_parser_document.is_none() {
            return Ok(false);
        }
        let record = self.arena.get(node)?;
        let kind = crate::html::script_block_type(
            record.attributes.get_null("type").map(String::as_str),
            record.attributes.get_null("language").map(String::as_str),
        );
        let has_source = self.script_external_source(node)?.is_some()
            || !self
                .arena
                .child_text_source(node, 8 * 1024 * 1024)?
                .is_empty();
        if !has_source
            || !self.is_document_connected(node)?
            || !(kind == "module"
                || kind == "importmap"
                || crate::html::is_javascript_mime_type(&kind))
        {
            let record = self.arena.get_mut(node)?;
            record.script_parser_document = None;
            if !record.attributes.contains_null("async") {
                record.script_force_async = true;
            }
            return Ok(false);
        }
        let document = self.arena.node_document(node)?;
        if document.is_some_and(|document| {
            self.script_parser_document_matches(node, document)
                .unwrap_or(false)
        }) {
            return Ok(false);
        }
        self.mark_script_started(node)?;
        Ok(true)
    }
    pub fn script_parser_document_matches(
        &self,
        node: NodeId,
        document: NodeId,
    ) -> Result<bool, DomError> {
        Ok(self
            .arena
            .get(node)?
            .script_parser_document
            .is_none_or(|identity| {
                identity
                    == NodeRef {
                        arena: self.arena.id(),
                        node: document,
                    }
            }))
    }

    /// SVG href takes precedence over the legacy XLink href, including an
    /// explicitly empty href. HTML scripts use only their null-namespace src.
    pub fn script_external_source(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        if self.is_element(node, "http://www.w3.org/2000/svg", "script") {
            if let Some(value) = self.attribute_in_null_namespace(node, "href")? {
                return Ok(Some(value));
            }
            self.attribute_ns(node, Some("http://www.w3.org/1999/xlink"), "href")
        } else {
            self.attribute_in_null_namespace(node, "src")
        }
    }

    pub fn script_async(&self, node: NodeId) -> Result<bool, DomError> {
        let node = self.arena.get(node)?;
        Ok(node.script_force_async || node.attributes.get_null("async").is_some())
    }

    pub fn set_script_async(&mut self, node: NodeId, asynchronous: bool) -> Result<(), DomError> {
        self.arena.get_mut(node)?.script_force_async = false;
        if asynchronous {
            self.set_attribute_ns(node, None, "async", "")
        } else {
            self.remove_attribute_ns(node, None, "async").map(|_| ())
        }
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
        self.import_graph_from(source, node, true, true)
    }

    pub fn import_node_from(
        &mut self,
        source: &DomArena,
        node: NodeId,
        deep: bool,
    ) -> Result<NodeId, DomError> {
        self.import_graph_from(source, node, deep, false)
    }

    fn import_graph_from(
        &mut self,
        source: &DomArena,
        node: NodeId,
        deep: bool,
        all_shadow: bool,
    ) -> Result<NodeId, DomError> {
        // Validate the complete imported graph before allocating destination nodes.
        // Template contents are separate tree edges; hosted Documents are not
        // imported DOM children. Reject malformed cycles without native recursion.
        let mut pending = vec![(node, deep)];
        let mut seen = BTreeSet::new();
        while let Some((current, deep)) = pending.pop() {
            if !seen.insert(current) {
                return Err(DomError::Cycle);
            }
            let record = source.get(current)?;
            for attribute in source.attribute_node_ids(current) {
                source.get(attribute)?;
            }
            if let Some(content) = record.template_content {
                pending.push((content, deep));
            }
            for child in &record.children {
                if matches!(source.get(*child)?.kind, NodeKind::ShadowRoot { clonable, .. } if clonable||all_shadow)
                    || (deep
                        && !matches!(
                            source.get(*child)?.kind,
                            NodeKind::Document | NodeKind::ShadowRoot { .. }
                        ))
                {
                    pending.push((*child,deep || matches!(source.get(*child)?.kind,NodeKind::ShadowRoot{clonable,..}if clonable||all_shadow)));
                }
            }
        }

        enum ImportTask {
            Expand(NodeId, NodeId, bool),
            Copy(NodeId, NodeId, bool, bool),
            Attach(NodeId, NodeId, bool),
        }
        let mut allocations = crate::insertion::CursorRoots::new(None, self);
        let result = (|| {
            let imported = self.import_shallow_node(source, node)?;
            allocations.retain(self, imported)?;
            if !all_shadow {
                let record = self.arena.get_mut(imported)?;
                record.declarative_shadow_root = false;
                record.output_default = None;
                record.submit_behavior = None;
            }
            let mut tasks = vec![ImportTask::Expand(node, imported, deep)];
            while let Some(task) = tasks.pop() {
                match task {
                    ImportTask::Expand(original, target, deep) => {
                        let record = source.get(original)?;
                        // LIFO order preserves template-first copying and sibling order.
                        for child in record.children.iter().rev() {
                            if matches!(source.get(*child)?.kind, NodeKind::ShadowRoot { clonable, .. } if clonable||all_shadow)
                                || (deep
                                    && !matches!(
                                        source.get(*child)?.kind,
                                        NodeKind::Document | NodeKind::ShadowRoot { .. }
                                    ))
                            {
                                tasks.push(ImportTask::Copy(*child, target, false, deep || matches!(source.get(*child)?.kind,NodeKind::ShadowRoot{clonable,..}if clonable||all_shadow)));
                            }
                        }
                        if let Some(content) = record.template_content {
                            tasks.push(ImportTask::Copy(content, target, true, deep));
                        }
                    }
                    ImportTask::Copy(original, parent, template, deep) => {
                        let target = self.import_shallow_node(source, original)?;
                        allocations.retain(self, target)?;
                        if !all_shadow {
                            let record = self.arena.get_mut(target)?;
                            record.declarative_shadow_root = false;
                            record.output_default = None;
                            record.submit_behavior = None;
                        }
                        // Attach only after descendants, preserving adoption and form
                        // reconciliation behavior of the former recursive import.
                        tasks.push(ImportTask::Attach(target, parent, template));
                        tasks.push(ImportTask::Expand(original, target, deep));
                    }
                    ImportTask::Attach(target, parent, false) => {
                        self.append_child(parent, target)?;
                    }
                    ImportTask::Attach(target, parent, true) => {
                        self.arena.get_mut(parent)?.template_content = Some(target);
                        self.arena.get_mut(target)?.template_host = Some(parent);
                        self.internal_targets.insert(target);
                        if let Some(document) = self.arena.node_document(parent)? {
                            let owner = self.arena.template_contents_owner_document(document)?;
                            self.arena.assign_node_document(target, owner)?;
                        }
                    }
                }
            }
            Ok(imported)
        })();
        match result {
            Ok(imported) => {
                let handoff = self.with_native_node_handoff([imported], |dom| {
                    allocations.release(dom)?;
                    #[cfg(test)]
                    crate::insertion::checkpoint()?;
                    Ok(imported)
                });
                if handoff.is_err() {
                    self.reclaim_if_unrooted(imported)?;
                }
                handoff
            }
            Err(error) => {
                let _ = allocations.release(self);
                Err(error)
            }
        }
    }

    pub fn import_shallow_node(
        &mut self,
        source: &DomArena,
        node: NodeId,
    ) -> Result<NodeId, DomError> {
        let source_node = source.get(node)?;
        let mut kind = source_node.kind.clone();
        if let NodeKind::Attribute { owner, .. } = &mut kind {
            *owner = None;
        }
        let imported = self.create(kind);
        let result = self.with_native_node_handoff([imported], |dom| {
            #[cfg(test)]
            crate::insertion::checkpoint()?;
            {
                let target = dom.arena.get_mut(imported)?;
                target
                    .document_metadata
                    .clone_from(&source_node.document_metadata);
                target.script_already_started = source_node.script_already_started;
                target
                    .cryptographic_nonce
                    .clone_from(&source_node.cryptographic_nonce);
                target.parser_duplicate_attributes = source_node.parser_duplicate_attributes;
                if matches!(target.kind, NodeKind::Document)
                    && let Some(metadata) = target.document_metadata.as_mut()
                {
                    metadata.hide_nonces = false;
                    metadata.registered_custom_properties.clear();
                }
                target.attributes.clone_from(&source_node.attributes);
                target.value.clone_from(&source_node.value);
                target
                    .output_default
                    .clone_from(&source_node.output_default);
                target
                    .submit_behavior
                    .clone_from(&source_node.submit_behavior);
                target.custom_element_registry = source_node.custom_element_registry;
                target.custom_element_registry_keep_null =
                    source_node.custom_element_registry_keep_null;
                target
                    .custom_element_is
                    .clone_from(&source_node.custom_element_is);
                target.dirty_value = source_node.dirty_value;
                target.user_edited_value = source_node.user_edited_value;
                target
                    .selected_files
                    .clone_from(&source_node.selected_files);
                target.checkedness = source_node.checkedness;
                target.dirty_checkedness = source_node.dirty_checkedness;
                target.selectedness = source_node.selectedness;
                target.dirty_selectedness = source_node.dirty_selectedness;
                target.indeterminate = source_node.indeterminate;
                target.sensitive_value = source_node.sensitive_value;
                target.manual_slot_assignment = source_node.manual_slot_assignment;
                target.delegates_focus = source_node.delegates_focus;
                target.shadow_serializable = source_node.shadow_serializable;
                target
                    .shadow_adopted_stylesheet_specifiers
                    .clone_from(&source_node.shadow_adopted_stylesheet_specifiers);
                target.shadow_reference_target = source_node.shadow_reference_target.clone();
                target.declarative_shadow_root = source_node.declarative_shadow_root;
            }
            for attribute in source.attribute_node_ids(node) {
                if let NodeKind::Attribute { data, .. } = &source.get(attribute)?.kind {
                    dom.set_attribute_ns(
                        imported,
                        data.namespace.clone(),
                        data.name.clone(),
                        data.value.clone(),
                    )?;
                    #[cfg(test)]
                    crate::insertion::checkpoint()?;
                }
            }
            dom.arena.get_mut(imported)?.checkedness = source_node.checkedness;
            dom.arena.get_mut(imported)?.dirty_checkedness = source_node.dirty_checkedness;
            dom.arena.get_mut(imported)?.selectedness = source_node.selectedness;
            dom.arena.get_mut(imported)?.dirty_selectedness = source_node.dirty_selectedness;
            // Input cloning copies the exposed sanitized value, not an unfinished
            // native number/date edit that is not representable by the value IDL.
            if dom.control_bad_input(imported)? {
                let value = dom.control_value(imported)?;
                dom.arena.get_mut(imported)?.value = Some(value);
            }
            Ok(imported)
        });
        // A failed initialization has no caller to receive this allocation.
        // Relinquish the protected handoff and collect its partial Attr graph.
        if result.is_err() {
            self.reclaim_if_unrooted(imported)?;
        }
        result
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
        self.set_inner_html_with_context(node, markup, &context_name)
    }

    pub fn set_inner_html_with_context(
        &mut self,
        node: NodeId,
        markup: &str,
        context_name: &str,
    ) -> Result<(), DomError> {
        if !matches!(
            self.arena.get(node)?.kind,
            NodeKind::Element { .. } | NodeKind::ShadowRoot { .. } | NodeKind::DocumentFragment
        ) {
            return Err(DomError::NotAnElement(node));
        }
        let parsed = html::parse_fragment(markup, context_name);
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
        let previous_metadata = self.arena.get(document)?.document_metadata.clone();
        let mut metadata = parsed.dom.get(parsed.document)?.document_metadata.clone();
        if let (Some(previous), Some(current)) = (previous_metadata, metadata.as_mut()) {
            // Replacing children parses a new tree inside the same Document.
            // URL/base identity and loading state belong to that Document;
            // the temporary parser's defaults are not a navigation commit.
            current.url = previous.url;
            current.referrer = previous.referrer;
            current.inherited_base_url = previous.inherited_base_url;
            current.encoding = previous.encoding;
            current.ready_state = previous.ready_state;
            current.registered_custom_properties = previous.registered_custom_properties;
        }
        self.arena.get_mut(document)?.document_metadata = metadata;
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
        self.clear_document_in(self.document())
    }

    /// Clear only the selected native Document, including an imported frame.
    pub fn clear_document_in(&mut self, document: NodeId) -> Result<(), DomError> {
        if !matches!(self.arena.get(document)?.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(document));
        }
        let clear_focus = self
            .focused
            .is_some_and(|node| self.arena.node_document(node).ok().flatten() == Some(document));
        for child in self.arena.get(document)?.children.clone() {
            self.remove_child(document, child)?;
        }
        if clear_focus {
            self.focus(None)?;
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
            state_preserving_move: false,
        })
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        self.insert_before_with_focus(parent, child, None, false)
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
        if self.arena.node_document(parent)? != self.arena.node_document(child)?
            || self.is_connected(parent)? != self.is_connected(child)?
            || !matches!(
                self.arena.get(child)?.kind,
                NodeKind::Element { .. }
                    | NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::ProcessingInstruction { .. }
                    | NodeKind::Comment { .. }
            )
        {
            return Err(DomError::InvalidStatePreservingMove);
        }
        self.insert_before_with_focus(parent, child, reference, true)
    }

    fn insert_before_with_focus(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
        preserve_focus: bool,
    ) -> Result<(), DomError> {
        self.with_native_node_handoff(
            std::iter::once(parent)
                .chain(std::iter::once(child))
                .chain(reference),
            |dom| {
                dom.prepare_tree_tail_capacity()?;
                let mut cursor = crate::insertion::TreeInsertion::new(
                    None,
                    dom,
                    parent,
                    child,
                    reference,
                    preserve_focus,
                )?;
                let result = cursor.resume(dom);
                if result.is_err() && cursor.committed_tail() {
                    dom.pending_tree_tails
                        .push_back(crate::insertion::CommittedTreeTail::Insert(Box::new(
                            cursor,
                        )));
                } else {
                    cursor.abort(dom);
                }
                result
            },
        )
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), DomError> {
        if self.arena.get(child)?.parent != Some(parent) {
            return Err(DomError::NotAChild(child, parent));
        }
        self.remove(child)
    }

    /// Expand a fragment using the DOM removal algorithm. Structural consumers
    /// retain every primitive journal entry; observers receive one fragment row.
    fn take_dom_insertion_nodes(&mut self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        if !matches!(self.arena.get(node)?.kind, NodeKind::DocumentFragment) {
            if self.arena.get(node)?.parent.is_some() {
                self.remove(node)?;
            }
            return Ok(vec![node]);
        }
        let nodes = self
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
        for child in &nodes {
            self.remove_child(node, *child)?;
        }
        self.mutations.suppress_tree_observers_since(cursor, node);
        if !nodes.is_empty() {
            self.record_mutation(Mutation::ChildrenReplaced {
                node,
                added_nodes: Vec::new(),
                removed_nodes: nodes.clone(),
                siblings: MutationSiblings::default(),
            });
        }
        Ok(nodes)
    }

    /// The caller performs DOM pre-insertion validity before this mutation.
    pub fn insert_dom_node(
        &mut self,
        parent: NodeId,
        node: NodeId,
        mut reference: Option<NodeId>,
    ) -> Result<Vec<NodeId>, DomError> {
        if reference == Some(node) {
            let siblings = &self.arena.get(parent)?.children;
            reference = siblings
                .iter()
                .position(|child| *child == node)
                .and_then(|i| {
                    siblings[i + 1..].iter().copied().find(|child| {
                        !matches!(
                            self.arena.get(*child).map(|r| &r.kind),
                            Ok(NodeKind::ShadowRoot { .. })
                        )
                    })
                });
        }
        // Let a single-node TreeInsertion observe both old and new owners.
        // Eager removal would normalize select controls in an intermediate tree.
        let nodes = if matches!(self.arena.get(node)?.kind, NodeKind::DocumentFragment) {
            self.take_dom_insertion_nodes(node)?
        } else {
            vec![node]
        };
        let index = match reference {
            Some(reference) => self.child_index(parent, reference)?,
            None => self.arena.get(parent)?.children.len() as u32,
        };
        let previous = self.arena.get(parent)?.children[..index as usize]
            .iter()
            .rev()
            .copied()
            .find(|child| {
                !nodes.contains(child)
                    && !matches!(
                        self.arena.get(*child).map(|r| &r.kind),
                        Ok(NodeKind::ShadowRoot { .. })
                    )
            });
        let siblings = MutationSiblings {
            previous,
            next: reference,
            state_preserving_move: false,
        };
        let cursor = self.mutation_cursor();
        for child in &nodes {
            self.insert_before(parent, *child, reference)?;
        }
        // The grouped insertion replaces only insertion rows. A same-parent
        // move still exposes the preceding removal with its original siblings.
        for (sequence, mutation) in &mut self.mutations.entries {
            if *sequence > cursor.0
                && let Mutation::Inserted {
                    parent: Some(owner),
                    observer_suppressed,
                    ..
                } = mutation
                && *owner == parent
            {
                *observer_suppressed = true;
            }
        }
        if !nodes.is_empty() {
            self.record_mutation(Mutation::ChildrenReplaced {
                node: parent,
                added_nodes: nodes.clone(),
                removed_nodes: Vec::new(),
                siblings,
            });
        }
        Ok(nodes)
    }

    /// Replace all light children while preserving attached shadow roots and
    /// recording incoming adoption removals before the grouped target mutation.
    pub fn replace_dom_children(
        &mut self,
        parent: NodeId,
        node: NodeId,
    ) -> Result<Vec<NodeId>, DomError> {
        let nodes = self.take_dom_insertion_nodes(node)?;
        let removed = self
            .arena
            .get(parent)?
            .children
            .iter()
            .copied()
            .filter(|child| {
                !matches!(
                    self.arena.get(*child).map(|r| &r.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            })
            .collect::<Vec<_>>();
        let cursor = self.mutation_cursor();
        for child in &removed {
            self.remove_child(parent, *child)?;
        }
        for child in &nodes {
            self.append_child(parent, *child)?;
        }
        self.mutations.suppress_tree_observers_since(cursor, parent);
        if !nodes.is_empty() || !removed.is_empty() {
            self.record_mutation(Mutation::ChildrenReplaced {
                node: parent,
                added_nodes: nodes.clone(),
                removed_nodes: removed,
                siblings: MutationSiblings::default(),
            });
        }
        Ok(nodes)
    }

    pub fn replace_dom_child(
        &mut self,
        parent: NodeId,
        node: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<Vec<NodeId>, DomError> {
        let nodes = self.take_dom_insertion_nodes(node)?;
        let removed = if self.arena.get(child)?.parent == Some(parent) {
            vec![child]
        } else {
            Vec::new()
        };
        let index = if !removed.is_empty() {
            self.child_index(parent, child)?
        } else {
            match reference {
                Some(reference) => self.child_index(parent, reference)?,
                None => self.arena.get(parent)?.children.len() as u32,
            }
        };
        let previous = self.arena.get(parent)?.children[..index as usize]
            .iter()
            .rev()
            .copied()
            .find(|child| {
                !matches!(
                    self.arena.get(*child).map(|r| &r.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            });
        let cursor = self.mutation_cursor();
        for child in &removed {
            self.remove_child(parent, *child)?;
        }
        for child in &nodes {
            self.insert_before(parent, *child, reference)?;
        }
        self.mutations.suppress_tree_observers_since(cursor, parent);
        if !nodes.is_empty() || !removed.is_empty() {
            self.record_mutation(Mutation::ChildrenReplaced {
                node: parent,
                added_nodes: nodes.clone(),
                removed_nodes: removed,
                siblings: MutationSiblings {
                    previous,
                    next: reference,
                    state_preserving_move: false,
                },
            });
        }
        Ok(nodes)
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
        self.record_mutation(Mutation::ChildrenReplaced {
            node: parent,
            added_nodes: vec![replacement],
            removed_nodes,
            siblings: MutationSiblings {
                previous,
                next: reference,
                state_preserving_move: false,
            },
        });
        Ok(())
    }

    /// Publish CSSOM media without changing HTML attributes or DOM records.
    pub fn set_stylesheet_disabled(
        &mut self,
        node: NodeId,
        disabled: bool,
    ) -> Result<(), DomError> {
        self.arena.get_mut(node)?.stylesheet_disabled = Some(disabled);
        self.record_mutation(Mutation::StyleSheets { node });
        Ok(())
    }

    pub fn set_stylesheet_media(&mut self, node: NodeId, media: String) -> Result<(), DomError> {
        self.arena.get_mut(node)?.stylesheet_media = Some(media);
        self.record_mutation(Mutation::StyleSheets { node });
        Ok(())
    }

    /// Publish CSSOM source without rewriting DOM text or emitting DOM records.
    pub fn set_stylesheet_text(&mut self, node: NodeId, source: String) -> Result<(), DomError> {
        self.arena.get_mut(node)?.stylesheet_override = Some(source.into());
        self.record_mutation(Mutation::StyleSheets { node });
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
        if previous != sheets {
            for sheet in &sheets {
                self.arena.get(*sheet)?;
            }
            let old = previous.into_iter().collect::<BTreeSet<_>>();
            let new = sheets.iter().copied().collect::<BTreeSet<_>>();
            self.pending_stylesheet_reclaims
                .extend(old.difference(&new).copied());
            self.arena.get_mut(node)?.adopted_stylesheets = sheets;
            self.internal_targets.extend(new);
            // Observable style invalidation belongs to the list commit, even
            // if reclaiming an independent old sheet subsequently fails.
            self.record_mutation(Mutation::StyleSheets { node });
        }
        self.retry_pending_stylesheet_reclaims()
    }
    /// Reclamation has no root decrement to replay. Keep only failures for the
    /// next native checkpoint and attempt every independent suffix each time.
    pub fn retry_pending_stylesheet_reclaims(&mut self) -> Result<(), DomError> {
        let pending = std::mem::take(&mut self.pending_stylesheet_reclaims);
        let mut first = None;
        for node in pending {
            if self.arena.get(node).is_err() {
                continue;
            }
            if let Err(error) = self.reclaim_if_unrooted(node) {
                self.pending_stylesheet_reclaims.insert(node);
                if first.is_none() {
                    first = Some(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
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
        if namespace.is_none() && local_name == "style" {
            let record = self.arena.get_mut(node)?;
            record.style_attribute_cssom = false;
            record.style_attribute_revision = record
                .style_attribute_revision
                .checked_add(1)
                .expect("style attribute revision exhausted");
        }
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

    fn invalidate_stylesheet_attribute(
        &mut self,
        node: NodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Result<(), DomError> {
        if namespace.is_some() {
            return Ok(());
        }
        if local_name == "disabled" {
            self.arena.get_mut(node)?.stylesheet_disabled = None;
        }
        let invalidates = match &self.arena.get(node)?.kind {
            NodeKind::Element { name } => {
                let html = name.ns.as_ref() == "http://www.w3.org/1999/xhtml";
                let svg = name.ns.as_ref() == "http://www.w3.org/2000/svg";
                (html
                    && name.local.as_ref() == "link"
                    && matches!(local_name, "href" | "rel" | "type"))
                    || ((html || svg) && name.local.as_ref() == "style" && local_name == "type")
            }
            _ => false,
        };
        if invalidates {
            let record = self.arena.get_mut(node)?;
            record.stylesheet_override = None;
            record.stylesheet_disabled = None;
            record.stylesheet_media = None;
            self.arena.bump_stylesheet_revision(node);
        }
        Ok(())
    }

    fn write_attribute_record(
        &mut self,
        node: NodeId,
        data: Arc<ContentAttribute>,
    ) -> Result<(), DomError> {
        let key = data.key();
        if key.namespace.is_none() && key.local_name == "nonce" {
            self.arena.get_mut(node)?.cryptographic_nonce = Some(data.value.clone());
        }
        if key.namespace.is_none() {
            self.clear_explicit_accessibility_reference(node, &key.local_name)?;
        }
        if key.namespace.is_none()
            && key.local_name == "async"
            && self.is_element(node, "http://www.w3.org/1999/xhtml", "script")
        {
            self.arena.get_mut(node)?.script_force_async = false;
        }
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
        if old_value.as_deref() != Some(value.as_str()) {
            self.invalidate_stylesheet_attribute(node, namespace.as_deref(), &data.local_name)?;
        }
        if name == "type" && namespace.is_none() {
            self.close_file_picker(node)?;
            self.arena.get_mut(node)?.temporal_edit = None;
            if self.arena.get(node)?.temporal_picker.is_some() {
                self.remove_from_top_layer(node);
            }
        }
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
        if namespace.is_none() && name == "value" && !self.arena.get(node)?.dirty_value {
            self.arena.get_mut(node)?.temporal_edit = None;
        }
        if namespace.is_none() {
            self.checked_attribute_changed(node, &name)?;
            self.option_attribute_changed(node, &name)?;
        }
        self.record_mutation(Mutation::Attribute {
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
        if key.namespace.is_none() {
            self.clear_explicit_accessibility_reference(node, &key.local_name)?;
        }
        let old_email_value =
            self.text_input_attribute_transition_value(node, &key.local_name, "")?;
        let Some(entry) = self.arena.get_mut(node)?.attributes.remove(&key) else {
            return Ok(());
        };
        if entry.namespace.is_none() && entry.local_name == "nonce" {
            self.arena.get_mut(node)?.cryptographic_nonce = None;
        }
        let name = entry.name.as_str();
        if entry.namespace.is_none() && name == "value" && !self.arena.get(node)?.dirty_value {
            self.arena.get_mut(node)?.temporal_edit = None;
        }
        let old_value = Some(entry.value.clone());
        let namespace = entry.namespace.clone();
        if namespace.is_none() && matches!(entry.local_name.as_str(), "open" | "popover") {
            self.remove_from_top_layer(node);
        }
        self.invalidate_stylesheet_attribute(node, namespace.as_deref(), &entry.local_name)?;
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
            self.record_mutation(Mutation::Attribute {
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
            let mut appended = Vec::new();
            for sibling in &siblings {
                if let NodeKind::Text { data } = &self.arena.get(*sibling)?.kind {
                    appended.extend(dom_string::units(data));
                }
            }
            self.replace_character_data(node, length, 0, dom_string::from_units(&appended))?;
            for sibling in &siblings {
                let index = self.child_index(parent, *sibling)?;
                self.boundary_journal
                    .push(boundary_journal::Change::Normalize {
                        from: *sibling,
                        to: node,
                        parent,
                        index,
                        target_offset: length,
                    });
                let mut root_changes = Vec::new();
                self.note_range_endpoint_change();
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

    /// Normalize's live-range remapping phase, after concatenation and its
    /// children-changed callbacks. Re-read current siblings before remapping.
    pub fn normalize_text_boundaries(
        &mut self,
        node: NodeId,
        mut length: u32,
    ) -> Result<Vec<NodeId>, DomError> {
        let Some(parent) = self.arena.get(node)?.parent else {
            return Ok(Vec::new());
        };
        let index = self.child_index(parent, node)? as usize;
        let siblings = self.arena.get(parent)?.children[index + 1..]
            .iter()
            .copied()
            .take_while(|sibling| {
                matches!(
                    self.arena.get(*sibling).map(|record| &record.kind),
                    Ok(NodeKind::Text { .. })
                )
            })
            .collect::<Vec<_>>();
        for sibling in &siblings {
            let index = self.child_index(parent, *sibling)?;
            self.boundary_journal
                .push(boundary_journal::Change::Normalize {
                    from: *sibling,
                    to: node,
                    parent,
                    index,
                    target_offset: length,
                });
            let mut roots = Vec::new();
            self.note_range_endpoint_change();
            for range in self.ranges.values_mut() {
                for boundary in [&mut range.start, &mut range.end] {
                    if boundary.container == *sibling {
                        roots.push((boundary.container, node));
                        boundary.container = node;
                        boundary.offset = boundary.offset.saturating_add(length);
                    } else if boundary.container == parent && boundary.offset == index {
                        roots.push((boundary.container, node));
                        boundary.container = node;
                        boundary.offset = length;
                    }
                }
            }
            for (old, new) in roots {
                self.retain_native(new)?;
                self.release_native(old)?;
            }
            if let NodeKind::Text { data } = &self.arena.get(*sibling)?.kind {
                length = length.saturating_add(utf16_len(data));
            }
        }
        Ok(siblings)
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
        let mut units = dom_string::units(&old_value);
        let start = (offset as usize).min(units.len());
        let end = start.saturating_add(count as usize).min(units.len());
        let inserted = dom_string::units(&replacement);
        let inserted_length = inserted.len() as u32;
        let removed_length = (end - start) as u32;
        units.splice(start..end, inserted);
        let value = dom_string::from_units(&units);
        let data = match &mut self.arena.get_mut(node)?.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => data,
            _ => return Err(DomError::StaleNode(node)),
        };
        *data = value.clone();
        self.boundary_journal
            .push(boundary_journal::Change::Replace {
                node,
                offset,
                removed: removed_length,
                inserted: inserted_length,
            });
        let removed_end = offset.saturating_add(removed_length);
        self.note_range_endpoint_change();
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
        self.record_mutation(Mutation::CharacterData {
            node,
            old_value,
            new_value: value,
        });
        Ok(())
    }

    /// Split Text/CDATA and migrate every native live range, including ranges
    /// owned by assistive-technology consumers rather than JS wrappers.
    pub fn split_text(&mut self, node: NodeId, offset: u32) -> Result<NodeId, DomError> {
        let record = self.arena.get(node)?;
        let (parent, cdata, data) = match &record.kind {
            NodeKind::Text { data } => (record.parent, false, data.clone()),
            NodeKind::CDataSection { data } => (record.parent, true, data.clone()),
            _ => return Err(DomError::StaleNode(node)),
        };
        let units = dom_string::units(&data);
        let length = units.len().try_into().unwrap_or(u32::MAX);
        if offset > length {
            return Err(DomError::InvalidBoundaryOffset {
                node,
                offset,
                maximum: length,
            });
        }
        let tail = dom_string::from_units(&units[offset as usize..]);
        let originals = self.ranges.clone();
        let revision = self.boundary_revision();
        self.replace_character_data(node, offset, length - offset, "")?;
        let created = if cdata {
            self.create_cdata_section(tail)
        } else {
            self.create_text(tail)
        };
        let parent_boundary = if let Some(parent) = parent {
            let siblings = self.children(parent)?.to_vec();
            let index = siblings
                .iter()
                .position(|candidate| *candidate == node)
                .ok_or(DomError::StaleNode(node))?;
            self.insert_before(parent, created, siblings.get(index + 1).copied())?;
            Some((parent, index as u32 + 1))
        } else {
            None
        };
        let change = boundary_journal::Change::Split {
            node,
            created,
            offset,
            parent: parent_boundary,
        };
        for (id, original) in originals {
            let mut current = self.live_range(id)?;
            for (before, after) in [
                (original.start, &mut current.start),
                (original.end, &mut current.end),
            ] {
                let mut corrected = before;
                change.apply(&mut corrected);
                if corrected != before {
                    *after = corrected;
                }
            }
            self.set_live_range(id, current.start, current.end)?;
        }
        self.boundary_journal
            .finish_split(revision, node, created, offset, parent_boundary);
        Ok(created)
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
        // Native storage includes attached shadow roots alongside light
        // children. They are not DOM children and never belong in a host's
        // childList record, even when a declarative fragment imports them.
        let added_nodes = added_nodes
            .into_iter()
            .filter(|child| {
                !matches!(
                    self.arena.get(*child).map(|record| &record.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            })
            .collect::<Vec<_>>();
        if !added_nodes.is_empty() || !removed_nodes.is_empty() {
            self.record_mutation(Mutation::ChildrenReplaced {
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

    /// DOM equality compares structure and expanded attribute names, not markup
    /// serialization (which loses namespaces and exposes attribute order).
    pub fn is_equal_node(&self, left: NodeId, right: NodeId) -> Result<bool, DomError> {
        self.is_equal_node_from(left, self, right)
    }

    /// Compare native nodes in independent document arenas without importing or
    /// mutating either tree. The ordinary equality entry point shares this walk.
    pub fn is_equal_node_from(
        &self,
        left: NodeId,
        other: &DomRuntime,
        right: NodeId,
    ) -> Result<bool, DomError> {
        let mut pending = vec![(left, right)];
        while let Some((left, right)) = pending.pop() {
            let left = self.arena.get(left)?;
            let right = other.arena.get(right)?;
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
            let children = |arena: &DomArena, node: &Node| {
                node.children
                    .iter()
                    .copied()
                    .filter(|child| {
                        arena
                            .get(*child)
                            .is_ok_and(|child| !matches!(child.kind, NodeKind::ShadowRoot { .. }))
                    })
                    .collect::<Vec<_>>()
            };
            let a = children(&self.arena, left);
            let b = children(&other.arena, right);
            if a.len() != b.len() {
                return Ok(false);
            }
            pending.extend(a.into_iter().zip(b));
        }
        Ok(true)
    }

    pub fn remove(&mut self, node: NodeId) -> Result<(), DomError> {
        let parent = self.arena.get(node)?.parent;
        self.with_native_node_handoff(std::iter::once(node).chain(parent), |dom| {
            dom.prepare_tree_tail_capacity()?;
            let mut cursor = crate::insertion::TreeRemoval::new(None, dom, node)?;
            let result = cursor.resume(dom);
            if result.is_err() && cursor.committed_tail() {
                dom.pending_tree_tails
                    .push_back(crate::insertion::CommittedTreeTail::Remove(Box::new(
                        cursor,
                    )));
            } else {
                cursor.abort(dom);
            }
            result
        })
    }

    pub fn boundary_revision(&self) -> u64 {
        self.boundary_journal.revision()
    }
    pub fn boundary_snapshot(&self) -> boundary_journal::Snapshot {
        self.boundary_journal.snapshot()
    }

    pub fn create_live_range(
        &mut self,
        start: BoundaryPoint,
        end: BoundaryPoint,
    ) -> Result<LiveRangeId, DomError> {
        self.validate_boundary(start)?;
        self.validate_boundary(end)?;
        let id = LiveRangeId(self.next_range.max(1));
        if id.0 > (1_u64 << 53) - 1 {
            return Err(DomError::MigrationCapacity);
        }
        let next = id.0.checked_add(1).ok_or(DomError::MigrationCapacity)?;
        let start_roots = self
            .roots
            .get(&start.container)
            .map_or(0, |roots| roots.native);
        let end_roots = self
            .roots
            .get(&end.container)
            .map_or(0, |roots| roots.native);
        let start_increment = if start.container == end.container {
            2
        } else {
            1
        };
        start_roots
            .checked_add(start_increment)
            .ok_or(DomError::MigrationCapacity)?;
        if start.container != end.container {
            end_roots
                .checked_add(1)
                .ok_or(DomError::MigrationCapacity)?;
        }
        self.retain_native(start.container)?;
        self.retain_native(end.container)?;
        self.note_range_endpoint_change();
        self.ranges.insert(id, LiveRange { start, end });
        self.next_range = next;
        Ok(id)
    }

    pub fn mark_composed_selection_range(&mut self, id: LiveRangeId) -> Result<(), DomError> {
        self.live_range(id)?;
        self.composed_selection_ranges.insert(id);
        Ok(())
    }
    pub fn range_shadow_removal_revision(&self, id: LiveRangeId) -> u64 {
        self.range_shadow_removals.get(&id).copied().unwrap_or(0)
    }
    // An exhausted revision disables caching instead of reusing an old token.
    pub fn range_endpoint_revision(&self) -> Option<u64> {
        (self.range_endpoint_revision != u64::MAX).then_some(self.range_endpoint_revision)
    }
    fn note_range_endpoint_change(&mut self) {
        self.range_endpoint_revision = self.range_endpoint_revision.saturating_add(1);
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
        self.note_range_endpoint_change();
        self.ranges.insert(id, LiveRange { start, end });
        self.release_native(previous.start.container)?;
        self.release_native(previous.end.container)
    }

    pub fn drop_live_range(&mut self, id: LiveRangeId) -> Result<(), DomError> {
        let range = self.ranges.remove(&id).ok_or(DomError::UnknownRange(id))?;
        self.note_range_endpoint_change();
        self.composed_selection_ranges.remove(&id);
        self.range_shadow_removals.remove(&id);
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
        self.boundary_journal
            .push(boundary_journal::Change::Insert {
                parent,
                index,
                count,
            });
        self.note_range_endpoint_change();
        for range in self.ranges.values_mut() {
            for boundary in [&mut range.start, &mut range.end] {
                if boundary.container == parent && boundary.offset > index {
                    boundary.offset = boundary.offset.saturating_add(count);
                }
            }
        }
        Ok(())
    }

    /// Detach a node for an html5ever tree-builder reparenting operation.
    ///
    /// The parser may immediately insert the handle elsewhere. Preserve it
    /// with the removal journal's lease until the normal delivery checkpoint,
    /// without attempting reclamation during the tree-builder operation.
    #[cfg(test)]
    pub(crate) fn parser_detach(&mut self, node: NodeId) -> Result<(), DomError> {
        if self.arena.get(node)?.parent.is_none() {
            return Ok(());
        }
        self.remove(node)
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

    /// Whether this node itself has native custody, excluding wrapper leases.
    /// Callers may memoize ancestry within one immutable DOM observation.
    pub fn is_native_root(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(self.native_root_count(node)? != 0)
    }

    /// Exact native lease count, excluding wrapper custody. Read-only embedders
    /// can distinguish traced object leases from independent native owners.
    pub fn native_root_count(&self, node: NodeId) -> Result<u32, DomError> {
        self.arena.get(node)?;
        Ok(self.roots.get(&node).map_or(0, |roots| roots.native))
    }

    /// Native task/parser/document custody, excluding JavaScript wrapper
    /// leases so a callback closing over a detached wrapper is collectible.
    pub fn retained_by_native_root(&self, node: NodeId) -> Result<bool, DomError> {
        let mut current = Some(node);
        let mut visited = BTreeSet::new();
        while let Some(node) = current {
            if !visited.insert(node) {
                return Ok(false);
            }
            if self.roots.get(&node).is_some_and(|roots| roots.native != 0) {
                return Ok(true);
            }
            current = self.retention_parent(node)?;
        }
        Ok(false)
    }

    // Synchronous native APIs return their bare NodeId arguments to their
    // caller. Cursor cleanup must not collect them during that handoff. These
    // temporary counts are removed without a collection point, just like the
    // fresh-root handoff in clone_node; no permanent native lease is created.
    fn with_native_node_handoff<T>(
        &mut self,
        nodes: impl IntoIterator<Item = NodeId>,
        action: impl FnOnce(&mut Self) -> Result<T, DomError>,
    ) -> Result<T, DomError> {
        let mut retained = Vec::new();
        let result = (|| {
            for node in nodes {
                self.retain_native(node)?;
                retained.push(node);
            }
            action(self)
        })();
        let mut cleanup = Ok(());
        for node in retained.into_iter().rev() {
            let released = (|| {
                let roots = self
                    .roots
                    .get_mut(&node)
                    .ok_or(DomError::NativeRootUnderflow(node))?;
                roots.native = roots
                    .native
                    .checked_sub(1)
                    .ok_or(DomError::NativeRootUnderflow(node))?;
                Ok(())
            })();
            if cleanup.is_ok() {
                cleanup = released;
            }
        }
        result.and_then(|value| cleanup.map(|_| value))
    }

    pub fn retain_native(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get(node)?;
        let roots = self.roots.entry(node).or_default();
        roots.native = roots
            .native
            .checked_add(1)
            .ok_or(DomError::MigrationCapacity)?;
        Ok(())
    }

    fn prepare_tree_tail_capacity(&mut self) -> Result<(), DomError> {
        self.drain_committed_tree_tails()?;
        if self.pending_tree_tails.len() >= 256 {
            return Err(DomError::MigrationCapacity);
        }
        self.pending_tree_tails
            .try_reserve(1)
            .map_err(|_| DomError::MigrationCapacity)
    }
    pub fn drain_committed_tree_tails(&mut self) -> Result<(), DomError> {
        if self.draining_tree_tails {
            return Ok(());
        }
        self.draining_tree_tails = true;
        let result = (|| {
            while let Some(mut tail) = self.pending_tree_tails.pop_front() {
                match tail.resume(self) {
                    Ok(()) => tail.abort(self),
                    Err(error) => {
                        self.pending_tree_tails.push_front(tail);
                        return Err(error);
                    }
                }
            }
            Ok(())
        })();
        self.draining_tree_tails = false;
        result
    }

    /// Aborted synchronous native cursors keep pre-decrement obligations in
    /// their runtime; mutation checkpoints and later operations retry them.
    pub(crate) fn drain_cursor_releases(&mut self) -> Result<(), DomError> {
        while let Some(node) = self.pending_cursor_releases.last().copied() {
            let mut committed = false;
            let result = self.release_native_with_commit(node, || committed = true);
            if committed {
                self.pending_cursor_releases.pop();
            }
            result?;
        }
        Ok(())
    }

    pub fn release_native(&mut self, node: NodeId) -> Result<(), DomError> {
        self.release_native_with_commit(node, || {})
    }

    // Accounting commits before fallible graph reclamation. Scoped custodians
    // must observe that commit even if reclamation subsequently reports an error.
    fn release_native_with_commit(
        &mut self,
        node: NodeId,
        committed: impl FnOnce(),
    ) -> Result<(), DomError> {
        let owner = self.arena.get(node)?.owner_document;
        let roots = self
            .roots
            .get_mut(&node)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        roots.native = roots
            .native
            .checked_sub(1)
            .ok_or(DomError::NativeRootUnderflow(node))?;
        committed();
        self.reclaim_if_unrooted(node)?;
        if let Some(owner) = owner
            && self.arena.get(owner).is_ok()
        {
            self.reclaim_if_unrooted(owner)?;
        }
        Ok(())
    }

    /// Explicit and modal inertness follow the native flat tree. A modal
    /// dialog escapes ancestor inertness but its own inert attribute applies.
    pub fn is_inert(&self, node: NodeId) -> Result<bool, DomError> {
        self.is_inert_with_checkpoint(node, &mut || Ok(()))
    }
    pub fn is_inert_with_checkpoint(
        &self,
        node: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<bool, DomError> {
        checkpoint()?;
        let document = self.arena.node_document(node)?;
        let mut modal = None;
        for entry in self.top_layer() {
            checkpoint()?;
            if entry.kind == TopLayerKind::ModalDialog
                && self.arena.node_document(entry.node).ok() == Some(document)
            {
                modal = Some(entry.node);
            }
        }
        if let Some(modal) = modal {
            let mut ancestor = Some(node);
            let mut inside = false;
            while let Some(current) = ancestor {
                checkpoint()?;
                if current == modal {
                    inside = true;
                    break;
                }
                ancestor = self.flat_tree_parent_with_checkpoint(current, checkpoint)?;
            }
            if !inside {
                return Ok(true);
            }
        }
        let mut current = Some(node);
        while let Some(node) = current {
            checkpoint()?;
            let record = self.arena.get(node)?;
            if matches!(&record.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
                && record.attributes.contains_null("inert")
            {
                return Ok(true);
            }
            if Some(node) == modal {
                break;
            }
            current = self.flat_tree_parent_with_checkpoint(node, checkpoint)?;
        }
        Ok(false)
    }

    /// Children in the rendered flat tree. Shadow contents replace light
    /// children; slots expose assigned nodes, or fallback children when empty.
    pub fn flat_tree_children(&self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        if let Some(root) = self.attached_shadow_root(node, false)? {
            return Ok(self.arena.get(root)?.children.clone());
        }
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "slot") {
            let assigned = self.assigned_nodes(node, false)?;
            if !assigned.is_empty() {
                return Ok(assigned);
            }
        }
        Ok(self.arena.get(node)?.children.clone())
    }

    /// Element paint participants before CSS order/stacking, paired with their
    /// nearest rendered element parent. Slot assignments retain native identity.
    pub fn flat_tree_elements(
        &self,
        root: NodeId,
    ) -> Result<Vec<(NodeId, Option<NodeId>)>, DomError> {
        let mut result = Vec::new();
        let mut pending = vec![(root, None)];
        let mut seen = BTreeSet::new();
        while let Some((id, parent)) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let record = self.arena.get(id)?;
            let element = matches!(record.kind, NodeKind::Element { .. });
            if element {
                result.push((id, parent));
            }
            let next_parent = if element { Some(id) } else { parent };
            pending.extend(
                self.flat_tree_children(id)?
                    .into_iter()
                    .rev()
                    .map(|child| (child, next_parent)),
            );
        }
        Ok(result)
    }

    /// Parent in the rendered flat tree. Unassigned light children and fallback
    /// children hidden by an assignment have no flat-tree parent.
    pub fn flat_tree_parent(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        self.flat_tree_parent_with_checkpoint(node, &mut || Ok(()))
    }
    pub fn flat_tree_parent_with_checkpoint(
        &self,
        node: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        checkpoint()?;
        let record = self.arena.get(node)?;
        if matches!(record.kind, NodeKind::Document) {
            return Ok(None);
        }
        if let Some(slot) = self.assigned_slot_with_checkpoint(node, false, checkpoint)? {
            return Ok(Some(slot));
        }
        let Some(parent) = record.parent else {
            return Ok(None);
        };
        if matches!(self.arena.get(parent)?.kind, NodeKind::ShadowRoot { .. }) {
            return Ok(self.arena.get(parent)?.parent);
        }
        if self
            .attached_shadow_root_with_checkpoint(parent, false, checkpoint)?
            .is_some()
            && !matches!(record.kind, NodeKind::ShadowRoot { .. })
        {
            return Ok(None);
        }
        if self.is_element(parent, "http://www.w3.org/1999/xhtml", "slot")
            && !self
                .assigned_nodes_with_checkpoint(parent, false, checkpoint)?
                .is_empty()
        {
            return Ok(None);
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
        let previous = self.focused;
        let close_picker = previous
            .filter(|previous| Some(*previous) != current)
            .map(|node| {
                self.arena
                    .get(node)
                    .map(|record| record.temporal_picker.is_some())
            })
            .transpose()?
            .unwrap_or(false);
        self.focused = current;
        if close_picker {
            self.remove_from_top_layer(previous.unwrap());
        }
        self.record_mutation(Mutation::Focus { previous, current });
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
            self.record_mutation(Mutation::Focus {
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
            self.record_mutation(Mutation::EventListener {
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
        self.record_mutation(Mutation::FormState { node });
        Ok(())
    }

    pub fn control_value(&self, node: NodeId) -> Result<String, DomError> {
        self.control_value_with_checkpoint(node, &mut || Ok(()))
    }
    pub fn control_value_with_checkpoint(
        &self,
        node: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<String, DomError> {
        checkpoint()?;
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
            return self.select_value_with_checkpoint(node, checkpoint);
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
                &self.textarea_default_value_with_checkpoint(node, checkpoint)?,
            )));
        }
        if let Some(value) = &record.value {
            return self.sanitized_input_value(node, value);
        }
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") {
            return self.arena.text_content_with_checkpoint(node, checkpoint);
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
        let final_length =
            u32::try_from(dom_string::len(&value)).map_err(|_| OpaqueRangeError::InvalidOffset)?;
        self.opaque_ranges
            .replace(node, start, end - start, inserted, old_length)?;
        let unfiltered = dom_string::from_units(&units);
        if self.arena.get(node)?.sensitive_value
            || self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
        {
            self.boundary_journal
                .push(boundary_journal::Change::ControlValueReset { node });
        } else if value == unfiltered {
            self.boundary_journal
                .push(boundary_journal::Change::ControlValueReplace {
                    node,
                    offset: start,
                    removed: end - start,
                    inserted,
                });
        } else {
            self.boundary_journal
                .push(boundary_journal::Change::ControlValueReplace {
                    node,
                    offset: 0,
                    removed: old_length,
                    inserted: final_length,
                });
        }
        self.arena.get_mut(node)?.value = Some(value);
        self.arena.get_mut(node)?.dirty_value = true;
        self.arena.get_mut(node)?.user_edited_value = false;
        self.record_mutation(Mutation::FormState { node });
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
        let value = if self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("color"))
        {
            self.sanitize_color_value(node, &value)?
        } else {
            value
        };
        let previous = self.control_value(node)?;
        let marker_change = if previous == value {
            None
        } else if sensitive
            || self.arena.get(node)?.sensitive_value
            || self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
        {
            Some(boundary_journal::Change::ControlValueReset { node })
        } else {
            Some(boundary_journal::Change::ControlValueReplace {
                node,
                offset: 0,
                removed: u32::try_from(dom_string::len(&previous))
                    .map_err(|_| OpaqueRangeError::InvalidOffset)?,
                inserted: u32::try_from(dom_string::len(&value))
                    .map_err(|_| OpaqueRangeError::InvalidOffset)?,
            })
        };
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
        if let Some(change) = marker_change {
            self.boundary_journal.push(change);
        }
        let record = self.arena.get_mut(node)?;
        record.value = Some(value);
        record.dirty_value = true;
        record.user_edited_value = false;
        record.temporal_edit = None;
        record.sensitive_value = sensitive;
        self.record_mutation(Mutation::FormState { node });
        Ok(())
    }

    /// Exact expanded-name match, independent of document case folding.
    pub fn is_element(&self, node: NodeId, namespace: &str, local_name: &str) -> bool {
        self.arena.get(node).is_ok_and(|node| matches!(&node.kind,
            NodeKind::Element { name } if name.ns.as_ref() == namespace && name.local.as_ref() == local_name))
    }

    /// Finds the first matching descendant in this tree, excluding shadow trees.
    pub fn get_element_by_id(&self, root: NodeId, id: &str) -> Result<Option<NodeId>, DomError> {
        self.get_element_by_id_with_checkpoint(root, id, &mut || Ok(()))
    }
    pub fn get_element_by_id_with_checkpoint(
        &self,
        root: NodeId,
        id: &str,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        checkpoint()?;
        let mut pending = Vec::new();
        for &child in self.arena.get(root)?.children.iter().rev() {
            checkpoint()?;
            pending.push(child);
        }
        if id.is_empty() {
            return Ok(None);
        }
        while let Some(node) = pending.pop() {
            checkpoint()?;
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
            for &child in record.children.iter().rev() {
                checkpoint()?;
                pending.push(child);
            }
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

    /// The HTML document head/body are direct children of its HTML root,
    /// not the first matching descendant returned by a selector.
    pub fn document_head(&self, document: NodeId) -> Result<Option<NodeId>, DomError> {
        self.document_html_child(document, false)
    }

    pub fn document_body(&self, document: NodeId) -> Result<Option<NodeId>, DomError> {
        self.document_html_child(document, true)
    }

    fn document_html_child(&self, document: NodeId, body: bool) -> Result<Option<NodeId>, DomError> {
        if !matches!(self.arena.get(document)?.kind, NodeKind::Document) {
            return Ok(None);
        }
        let mut root = None;
        for child in self.children(document)? {
            if matches!(self.arena.get(*child)?.kind, NodeKind::Element { .. }) {
                root = Some(*child);
                break;
            }
        }
        let Some(root) = root else { return Ok(None) };
        if !self.is_element(root, "http://www.w3.org/1999/xhtml", "html") {
            return Ok(None);
        }
        for child in self.children(root)? {
            let record = self.arena.get(*child)?;
            if let NodeKind::Element { name } = &record.kind
                && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && if body { matches!(name.local.as_ref(), "body" | "frameset") }
                    else { name.local.as_ref() == "head" }
            {
                return Ok(Some(*child));
            }
        }
        Ok(None)
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
        serialize_html_with_resolver(
            NodeRef {
                arena: self.arena.id(),
                node,
            },
            &serialization::LocalResolver(self),
        )
    }

    fn record_mutation(&mut self, mutation: Mutation) {
        self.render_mutation(&mutation);
        // Default-value/type changes have no exact edit interval. Invalidate
        // only markers in that control, never ordinary DOM source boundaries.
        if let Mutation::Attribute {
            node,
            namespace: None,
            name,
            old_value,
            new_value,
        } = &mutation
            && old_value != new_value
            && self.is_element(*node, "http://www.w3.org/1999/xhtml", "input")
            && (name == "type"
                || matches!(name.as_str(), "alpha" | "colorspace")
                || (name == "value"
                    && self
                        .arena
                        .get(*node)
                        .is_ok_and(|record| !record.dirty_value)))
        {
            self.boundary_journal
                .push(boundary_journal::Change::ControlValueReset { node: *node });
        }
        let value_ancestor = match &mutation {
            Mutation::CharacterData { node, .. } | Mutation::ChildrenReplaced { node, .. } => {
                Some(*node)
            }
            Mutation::Inserted { parent, .. } => *parent,
            Mutation::Removed { former_parent, .. } => *former_parent,
            _ => None,
        };
        let mut ancestor = value_ancestor;
        while let Some(node) = ancestor {
            let Ok(record) = self.arena.get(node) else {
                break;
            };
            if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") {
                if !record.dirty_value {
                    self.boundary_journal
                        .push(boundary_journal::Change::ControlValueReset { node });
                }
                break;
            }
            ancestor = record.parent;
        }
        if let Mutation::Attribute {
            node,
            namespace: None,
            name,
            ..
        } = &mutation
            && matches!(name.as_str(), "http-equiv" | "content")
        {
            let _ = self.arena.process_language_pragmas(*node);
        }
        if let Mutation::Attribute {
            node,
            namespace: None,
            name,
            ..
        } = &mutation
        {
            let embedded_kind = self.embedded_content_kind(*node);
            let embedded_selection_changed = match name.as_str() {
                "data" => embedded_kind == Some(EmbeddedContentKind::Object),
                "type" => {
                    embedded_kind.is_some_and(EmbeddedContentKind::selects_response_representation)
                }
                _ => false,
            };
            if matches!(name.as_str(), "src" | "srcdoc" | "sandbox") || embedded_selection_changed {
                self.invalidate_frame_navigation_identity(*node);
            }
        }
        // XPath iterators invalidate on structural mutations in their document,
        // including mutations whose nodes are later reclaimed. Identity tokens
        // cannot wrap and do not depend on the bounded journal retaining nodes.
        let changed: Vec<NodeId> = match &mutation {
            Mutation::Inserted {
                node,
                parent: Some(parent),
                ..
            } => vec![*node, *parent],
            // Allocation alone does not mutate a document tree. Fresh nodes
            // receive their actual owner after allocation (e.g. inert Documents).
            Mutation::Inserted { parent: None, .. } => Vec::new(),
            Mutation::Removed {
                node,
                former_parent,
                ..
            } => std::iter::once(*node).chain(*former_parent).collect(),
            Mutation::ChildrenReplaced { node, .. }
            | Mutation::Attribute { node, .. }
            | Mutation::CharacterData { node, .. }
            | Mutation::DocumentOwnership { node, .. } => vec![*node],
            _ => Vec::new(),
        };
        if matches!(
            &mutation,
            Mutation::Attribute { .. }
                | Mutation::CharacterData { .. }
                | Mutation::DocumentOwnership { .. }
        ) {
            for node in &changed {
                let _ = self.arena.initialize_preferred_style_set(*node);
            }
        }
        let documents = changed
            .into_iter()
            .filter_map(|node| {
                self.arena
                    .get(node)
                    .ok()
                    .map(|record| record.owner_document.unwrap_or(node))
            })
            .collect::<BTreeSet<_>>();
        for document in documents {
            if let Some(revision) = self.xpath_revisions.get_mut(&document) {
                *revision = Rc::new(());
            }
        }

        let affects_assignment = match &mutation {
            Mutation::Inserted { .. }
            | Mutation::Removed { .. }
            | Mutation::ChildrenReplaced { .. } => true,
            Mutation::Attribute {
                namespace: None,
                name,
                ..
            } => name == "slot" || name == "name",
            _ => false,
        };
        let fallback_parent = match &mutation {
            Mutation::Inserted { parent, .. } => *parent,
            Mutation::Removed { former_parent, .. } => *former_parent,
            Mutation::ChildrenReplaced { node, .. } => Some(*node),
            _ => None,
        };
        self.mutations.push(mutation);
        if affects_assignment {
            self.refresh_slot_assignments()
                .expect("live slot assignment state");
            if let Some(parent) = fallback_parent {
                self.signal_slot_fallback_change(parent)
                    .expect("live fallback slot state");
            }
        }
    }

    pub fn drain_mutations(&mut self) -> Vec<Mutation> {
        self.mutations.drain_observer()
    }

    pub fn xpath_document_revision(&mut self, document: NodeId) -> Result<Rc<()>, DomError> {
        self.arena.get(document)?;
        if !self.xpath_revisions.contains_key(&document) {
            self.xpath_revisions
                .retain(|node, _| self.arena.get(*node).is_ok());
        }
        Ok(self
            .xpath_revisions
            .entry(document)
            .or_insert_with(|| Rc::new(()))
            .clone())
    }
    pub fn xpath_document_revision_matches(&self, document: NodeId, revision: &Rc<()>) -> bool {
        self.arena.get(document).is_ok()
            && self
                .xpath_revisions
                .get(&document)
                .is_some_and(|current| Rc::ptr_eq(current, revision))
    }

    pub fn mutation_cursor(&self) -> MutationCursor {
        MutationCursor(self.mutations.next.saturating_sub(1))
    }

    pub fn mutations_since(&self, cursor: MutationCursor) -> MutationBatch {
        self.mutations.read_since(cursor)
    }

    pub fn release_mutation_roots(&mut self, mutations: &[Mutation]) -> Result<(), DomError> {
        self.drain_committed_tree_tails()?;
        self.drain_cursor_releases()?;
        while let Some((node, lease)) = self.mutations.discarded_removed_roots.last().cloned() {
            let result = self.release_removed_root(node, &lease);
            if lease.consumed() {
                self.mutations.discarded_removed_roots.pop();
            }
            result?;
        }
        for mutation in mutations {
            if let Mutation::Removed { node, lease, .. } = mutation {
                self.release_removed_root(*node, lease)?;
            }
        }
        Ok(())
    }

    fn release_removed_root(
        &mut self,
        node: NodeId,
        lease: &MutationRemovalLease,
    ) -> Result<(), DomError> {
        if lease.consumed()
            || !self
                .mutations
                .removed_leases
                .get(&node)
                .is_some_and(|leases| leases.iter().any(|candidate| candidate.same(lease)))
        {
            return Ok(());
        }
        let result = self.release_native_with_commit(node, || lease.consume());
        // Reclamation can fail after the native decrement. Retire that exact
        // token even on error; retry must not consume a later removal's lease.
        if lease.consumed() {
            let leases = self
                .mutations
                .removed_leases
                .get_mut(&node)
                .expect("validated removal lease");
            leases.retain(|candidate| !candidate.same(lease));
            if leases.is_empty() {
                self.mutations.removed_leases.remove(&node);
                self.mutations.outstanding_removed_roots.remove(&node);
            } else {
                *self
                    .mutations
                    .outstanding_removed_roots
                    .get_mut(&node)
                    .expect("removal count") -= 1;
            }
        }
        result
    }

    fn rebuild_document_indexes(&mut self) -> Result<(), DomError> {
        let document = self.document();
        let mut pending = vec![document];
        let mut connected = BTreeSet::new();
        while let Some(node) = pending.pop() {
            // Validate a parsed/snapshot graph before publishing its index.
            // A repeated retention edge is invalid, including a hosted cycle.
            if !connected.insert(node) {
                return Err(DomError::Cycle);
            }
            let record = self.arena.get(node)?;
            pending.extend(record.children.iter().rev().copied());
            pending.extend(self.arena.hosted_document(node));
        }
        self.connected.extend(connected);
        Ok(())
    }

    fn mark_connected(&mut self, node: NodeId) -> Result<(), DomError> {
        insertion::ConnectionUpdate::new(self, node, true, false)?.resume(self)
    }

    fn mark_disconnected(&mut self, node: NodeId) -> Result<(), DomError> {
        self.mark_disconnected_with_focus(node, false)
    }

    fn mark_disconnected_with_focus(
        &mut self,
        node: NodeId,
        preserve_focus: bool,
    ) -> Result<(), DomError> {
        insertion::ConnectionUpdate::new(self, node, false, preserve_focus)?.resume(self)
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
        enum ReclaimTask {
            Check(NodeId),
            Expand(NodeId),
            Remove(NodeId),
        }
        let mut tasks = vec![ReclaimTask::Check(node)];
        while let Some(task) = tasks.pop() {
            let node = match &task {
                ReclaimTask::Check(node)
                | ReclaimTask::Expand(node)
                | ReclaimTask::Remove(node) => *node,
            };
            // Releasing an internal edge can collect another pending target.
            if self.arena.get(node).is_err() {
                continue;
            }
            match task {
                ReclaimTask::Check(node) => {
                    if let Some(root) = self.unrooted_tree_root(node)? {
                        tasks.push(ReclaimTask::Expand(root));
                    }
                }
                ReclaimTask::Expand(node) => {
                    tasks.push(ReclaimTask::Remove(node));
                    tasks.extend(
                        self.retention_children(node)?
                            .into_iter()
                            .rev()
                            .map(ReclaimTask::Expand),
                    );
                }
                ReclaimTask::Remove(node) => {
                    let targets = self.reclaim_unrooted_node(node)?;
                    tasks.extend(targets.into_iter().rev().map(ReclaimTask::Check));
                }
            }
        }
        Ok(())
    }

    fn unrooted_tree_root(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        // A retained DOM node exposes both its ancestors and descendants.
        // Collection must not detach a retained child or erase an unwrapped
        // child from a retained parent. Reclaim the whole detached tree only
        // when none of its nodes has a wrapper or native lease.
        let mut root = node;
        let mut ancestors = BTreeSet::new();
        loop {
            if !ancestors.insert(root) {
                return Err(DomError::Cycle);
            }
            if self.is_gc_root(root) {
                return Ok(None);
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
        let mut descendants = BTreeSet::from([root]);
        while let Some(child) = pending.pop() {
            if !descendants.insert(child) {
                return Err(DomError::Cycle);
            }
            if self.is_gc_root(child) {
                return Ok(None);
            }
            graph_target |= matches!(self.arena.get(child)?.kind, NodeKind::Document)
                || self.arena.get(child)?.template_content.is_some()
                || self.internal_targets.contains(&child);
            pending.extend(self.retention_children(child)?);
        }
        if !graph_target {
            return Ok(Some(root));
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
            pending.extend(
                record
                    .default_accessibility_references
                    .values()
                    .flatten()
                    .copied(),
            );
            pending.extend(
                record
                    .explicit_accessibility_references
                    .values()
                    .flatten()
                    .copied(),
            );
        }
        if reachable.contains(&root) {
            return Ok(None);
        }
        Ok(Some(root))
    }

    // Descendants have already been removed by the explicit task stack.
    // Return internal-edge targets for later reachability checks, without
    // recursively entering reclamation through templates or accessibility links.
    fn reclaim_unrooted_node(&mut self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        let sheets = self
            .arena
            .get(node)?
            .adopted_stylesheets
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let accessibility_targets = self
            .arena
            .get(node)?
            .default_accessibility_references
            .values()
            .chain(
                self.arena
                    .get(node)?
                    .explicit_accessibility_references
                    .values(),
            )
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        let template_content = self.arena.get(node)?.template_content;
        let template_host = self.arena.get(node)?.template_host;
        let inert_document = self.arena.get(node)?.associated_inert_document;
        self.slot_elements.remove(&node);
        self.slot_assignment_snapshots.remove(&node);
        if let Some(nodes) = self.manual_slot_nodes.remove(&node) {
            for child in nodes {
                self.manual_node_slots.remove(&child);
            }
        }
        if let Some(slot) = self.manual_node_slots.remove(&node)
            && let Some(nodes) = self.manual_slot_nodes.get_mut(&slot)
        {
            nodes.retain(|child| *child != node);
        }
        self.arena.form_associated_custom_elements.remove(&node);
        self.custom_element_validity.remove(&node);
        self.parser_shadow_disabled.remove(&node);
        if matches!(self.arena.get(node)?.kind, NodeKind::Document) {
            self.custom_registry_documents
                .retain(|(_, document), _| *document != node);
        }
        let _ = self.arena.remove(node)?;
        let mut targets = accessibility_targets.into_iter().collect::<Vec<_>>();
        targets.extend(template_content);
        targets.extend(sheets);
        self.roots.remove(&node);
        self.frame_context_generations.remove(&node);
        self.frame_navigation_identities.remove(&node);
        self.internal_targets.remove(&node);
        targets.extend(template_host);
        targets.extend(inert_document.filter(|inert| *inert != node));
        Ok(targets)
    }
}

fn utf16_len(value: &str) -> u32 {
    dom_string::len(value).try_into().unwrap_or(u32::MAX)
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
                "paused" => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "audio" | "video")
                        && (node.media_selector_state.unwrap_or(2) & 2) != 0
                }
                "playing" => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "audio" | "video")
                        && (node.media_selector_state.unwrap_or(2) & 4) != 0
                }
                "seeking" => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "audio" | "video")
                        && (node.media_selector_state.unwrap_or(2) & 8) != 0
                }
                "buffering" => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "audio" | "video")
                        && (node.media_selector_state.unwrap_or(2) & 16) != 0
                }
                "stalled" => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "audio" | "video")
                        && (node.media_selector_state.unwrap_or(2) & 32) != 0
                }
                "interest-source" => node.interest_source,
                "interest-target" => node.interest_target,
                "granted" => {
                    node.permission_granted
                        && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(
                            name.local.as_ref(),
                            "geolocation" | "usermedia" | "camera" | "microphone"
                        )
                }
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
    fn memory_snapshot_is_numeric_and_tracks_actual_removal_custody() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("section");
        dom.retain_wrapper(node).unwrap();
        dom.append_child(dom.document(), node).unwrap();
        dom.remove(node).unwrap();
        let before = dom.diagnostic_memory_snapshot();
        assert_eq!(before.outstanding_removal_roots, 1);
        assert_eq!(before.removal_lease_count, 1);
        assert!(before.arena_occupied_nodes <= before.arena_slots);
        assert!(before.arena_slots <= before.arena_slot_capacity);
        let value = serde_json::to_value(&before).unwrap();
        assert!(value.as_object().unwrap().values().all(|value| value.is_u64()));
        let changes = dom.drain_mutations();
        assert_eq!(dom.diagnostic_memory_snapshot().removal_lease_count, 1);
        dom.release_mutation_roots(&changes).unwrap();
        let after = dom.diagnostic_memory_snapshot();
        assert_eq!(after.removal_lease_count, 0);
        assert_eq!(after.outstanding_removal_roots, 0);
        dom.release_wrapper(node).unwrap();
    }

    #[test]
    fn state_preserving_move_rejects_cross_document_before_mutation() {
        let mut dom = html::parse("<main id=target></main><iframe id=frame></iframe>")
            .into_runtime()
            .unwrap();
        let parent = dom.find_by_id("target").unwrap();
        let frame = dom.find_by_id("frame").unwrap();
        let document = dom.create(NodeKind::Document);
        let child = dom.create_element("div");
        dom.append_child(document, child).unwrap();
        dom.set_hosted_document(frame, Some(document)).unwrap();
        assert!(dom.is_connected(child).unwrap());
        let cursor = dom.mutation_cursor();
        assert_eq!(
            dom.move_before(parent, child, None),
            Err(DomError::InvalidStatePreservingMove)
        );
        assert_eq!(dom.parent(child).unwrap(), Some(document));
        assert_eq!(dom.mutation_cursor(), cursor);
        assert_eq!(dom.arena().node_document(child).unwrap(), Some(document));
    }

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
fn document_head_body_are_direct_html_children_with_frameset_precedence() {
    let mut dom = DomRuntime::new();
    let document = dom.document();
    let html = dom.create_element("html");
    let div = dom.create_element("div");
    let nested_body = dom.create_element("body");
    let nested_head = dom.create_element("head");
    let frameset = dom.create_element("frameset");
    let body = dom.create_element("body");
    let head = dom.create_element("head");
    dom.append_child(document, html).unwrap();
    dom.append_child(html, div).unwrap();
    dom.append_child(div, nested_body).unwrap();
    dom.append_child(div, nested_head).unwrap();
    assert_eq!(dom.document_body(document).unwrap(), None);
    assert_eq!(dom.document_head(document).unwrap(), None);
    dom.append_child(html, frameset).unwrap();
    dom.append_child(html, body).unwrap();
    dom.append_child(html, head).unwrap();
    assert_eq!(dom.document_body(document).unwrap(), Some(frameset));
    assert_eq!(dom.document_head(document).unwrap(), Some(head));
    dom.remove(frameset).unwrap();
    assert_eq!(dom.document_body(document).unwrap(), Some(body));
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
    fn native_adoption_commits_effective_registry_before_any_journal_consumer() {
        let mut dom = DomRuntime::new();
        let source = dom.document();
        let destination = dom.create(NodeKind::Document);
        dom.retain_native(destination).unwrap();
        dom.register_global_custom_element_registry(1);
        dom.register_global_custom_element_registry(2);
        dom.arena_mut()
            .get_mut(source)
            .unwrap()
            .custom_element_registry = Some(Some(1));
        dom.arena_mut()
            .get_mut(destination)
            .unwrap()
            .custom_element_registry = Some(Some(2));
        let source_body = dom.create_element("body");
        dom.append_child(source, source_body).unwrap();
        let destination_body = dom.create_element("body");
        dom.append_child(destination, destination_body).unwrap();
        for parent_registry in [None, Some(1), Some(3)] {
            for child_registry in [None, Some(1), Some(3)] {
                let parent = dom.create_element("div");
                let child = dom.create_element("x-child");
                dom.append_child(source_body, parent).unwrap();
                dom.append_child(parent, child).unwrap();
                dom.arena_mut()
                    .get_mut(parent)
                    .unwrap()
                    .custom_element_registry = Some(parent_registry);
                dom.arena_mut()
                    .get_mut(child)
                    .unwrap()
                    .custom_element_registry = Some(child_registry);
                dom.retain_native(parent).unwrap();
                let identity = dom.arena().node_ref(parent).unwrap();
                let document = dom.arena().document_id(destination).unwrap();
                dom.adopt_node(document, identity).unwrap();
                assert_eq!(
                    dom.arena().get(parent).unwrap().custom_element_registry,
                    Some(if parent_registry == Some(3) {
                        Some(3)
                    } else {
                        Some(2)
                    })
                );
                let expected = if child_registry == Some(3) {
                    Some(3)
                } else if child_registry.is_none() && parent_registry == Some(3) {
                    None
                } else {
                    Some(2)
                };
                assert_eq!(
                    dom.arena().get(child).unwrap().custom_element_registry,
                    Some(expected)
                );
                // Later mutations cannot change the already-committed adoption result.
                dom.append_child(destination_body, child).unwrap();
                assert_eq!(
                    dom.arena().get(child).unwrap().custom_element_registry,
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn custom_registry_document_association_retains_first_use_and_adoption_order() {
        let mut dom = DomRuntime::new();
        let first = dom.document();
        let second = dom.create(NodeKind::Document);
        let third = dom.create(NodeKind::Document);
        let a = dom.create_element("div");
        dom.set_node_document(a, second).unwrap();
        dom.set_custom_element_registry(a, Some(41)).unwrap();
        let b = dom.create_element("div");
        dom.set_node_document(b, first).unwrap();
        dom.set_custom_element_registry(b, Some(41)).unwrap();
        dom.set_custom_element_registry(a, None).unwrap();
        dom.set_custom_element_registry(a, Some(41)).unwrap();
        dom.set_node_document(a, third).unwrap();
        let mut associations = dom.custom_registry_documents(41);
        associations.sort_by_key(|(sequence, _)| *sequence);
        assert_eq!(
            associations
                .iter()
                .map(|(_, document)| *document)
                .collect::<Vec<_>>(),
            vec![second, first, third]
        );
        assert_eq!(
            dom.arena().get(a).unwrap().custom_element_registry,
            Some(Some(41))
        );
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
    fn replacing_document_tree_preserves_identity_encoding_and_readiness() {
        let mut dom = html::parse("<p>old</p>").into_runtime().unwrap();
        let document = dom.document();
        {
            let metadata = dom
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap();
            metadata.url = Some("about:srcdoc".into());
            metadata.inherited_base_url = Some("https://example.test/directory/".into());
            metadata.encoding = "windows-1252".into();
            metadata.ready_state = DocumentReadyState::Loading;
        }
        dom.replace_document_contents_in(document, "<!doctype html><p id=new>new</p>")
            .unwrap();
        let metadata = dom
            .arena()
            .get(document)
            .unwrap()
            .document_metadata
            .as_ref()
            .unwrap();
        assert_eq!(metadata.url.as_deref(), Some("about:srcdoc"));
        assert_eq!(
            metadata.inherited_base_url.as_deref(),
            Some("https://example.test/directory/")
        );
        assert_eq!(metadata.encoding, "windows-1252");
        assert_eq!(metadata.ready_state, DocumentReadyState::Loading);
        assert!(!metadata.quirks);
        assert!(dom.get_element_by_id(document, "new").unwrap().is_some());
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
    fn every_native_embedding_kind_preserves_document_custody() {
        for tag in ["iframe", "frame", "object", "embed"] {
            let mut dom = DomRuntime::new();
            let owner = dom.create_element(tag);
            let other = dom.create_element(tag);
            dom.append_child(dom.document(), owner).unwrap();
            dom.append_child(dom.document(), other).unwrap();
            let first = dom.create(NodeKind::Document);
            let child = dom.create_element("button");
            dom.append_child(first, child).unwrap();
            dom.set_hosted_document(owner, Some(first)).unwrap();
            assert_eq!(dom.arena().hosted_document(owner), Some(first));
            assert!(dom.children(owner).unwrap().is_empty());
            assert_eq!(dom.parent(first).unwrap(), None);
            assert_eq!(dom.retention_parent(first).unwrap(), Some(owner));
            assert!(dom.is_connected(child).unwrap());
            let cursor = dom.mutation_cursor();
            dom.set_hosted_document(owner, Some(first)).unwrap();
            assert_eq!(dom.mutation_cursor(), cursor);
            assert_eq!(
                dom.set_hosted_document(other, Some(first)),
                Err(DomError::Cycle)
            );
            assert_eq!(
                dom.set_hosted_document(owner, Some(dom.document())),
                Err(DomError::Cycle)
            );
            assert_eq!(dom.mutation_cursor(), cursor);
            dom.retain_native(child).unwrap();
            let second = dom.create(NodeKind::Document);
            dom.set_hosted_document(owner, Some(second)).unwrap();
            assert_eq!(dom.arena().hosted_document(owner), Some(second));
            assert_eq!(dom.retention_parent(first).unwrap(), None);
            assert!(!dom.is_connected(child).unwrap());
            dom.release_native(child).unwrap();
            assert!(dom.arena().get(first).is_err());
            assert!(dom.arena().get(child).is_err());
            dom.set_hosted_document(owner, None).unwrap();
            assert_eq!(dom.arena().hosted_document(owner), None);
            assert!(dom.arena().get(second).is_err());
            assert!(dom.arena().get(owner).is_ok());
        }
        let mut dom = DomRuntime::new();
        let ordinary = dom.create_element("div");
        let foreign = dom.create_element_ns("http://www.w3.org/2000/svg", "object");
        let document = dom.create(NodeKind::Document);
        for owner in [ordinary, foreign, dom.document()] {
            assert_eq!(
                dom.set_hosted_document(owner, Some(document)),
                Err(DomError::NotAnElement(owner))
            );
            assert_eq!(dom.arena().hosted_document(owner), None);
        }
        assert!(dom.arena().get(document).is_ok());
    }

    #[test]
    fn importing_deep_subtrees_uses_an_explicit_traversal_stack() {
        let mut source = DomRuntime::new();
        let root = source.create_element("section");
        let mut parent = root;
        // Construct a deep arena without recursive parsing or serialization.
        for _ in 0..4096 {
            let child = source.create_element("div");
            source.arena.get_mut(parent).unwrap().children.push(child);
            source.arena.get_mut(child).unwrap().parent = Some(parent);
            parent = child;
        }
        let leaf = source.create_text("deep leaf");
        source.arena.get_mut(parent).unwrap().children.push(leaf);
        source.arena.get_mut(leaf).unwrap().parent = Some(parent);
        let mut destination = DomRuntime::new();
        let imported = destination.import_subtree(source.arena(), root).unwrap();
        let mut current = imported;
        for _ in 0..4096 {
            let children = destination.children(current).unwrap();
            assert_eq!(children.len(), 1);
            current = children[0];
            assert!(destination.is_element(current, "http://www.w3.org/1999/xhtml", "div"));
        }
        let children = destination.children(current).unwrap();
        assert_eq!(children.len(), 1);
        assert!(
            matches!(&destination.arena().get(children[0]).unwrap().kind,
            NodeKind::Text { data } if data == "deep leaf")
        );
        destination.retain_native(imported).unwrap();
        let mutations = destination.drain_mutations();
        destination.release_mutation_roots(&mutations).unwrap();
        destination.release_native(imported).unwrap();
        assert!(destination.arena().get(imported).is_err());
        assert!(destination.arena().get(current).is_err());
    }

    #[test]
    fn deep_template_reclamation_preserves_live_leaf_then_releases_internal_edges() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("template");
        let mut template = root;
        let mut allocated = vec![root];
        for _ in 0..512 {
            let content = dom.template_content(template).unwrap();
            let next = dom.create_element("template");
            dom.append_child(content, next).unwrap();
            allocated.extend([content, next]);
            template = next;
        }
        dom.retain_native(root).unwrap();
        dom.retain_wrapper(template).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.release_native(root).unwrap();
        assert!(
            dom.arena().get(root).is_ok(),
            "leaf wrapper retains template hosts"
        );
        dom.release_wrapper(template).unwrap();
        for node in allocated {
            assert!(
                dom.arena().get(node).is_err(),
                "unreachable template edge {node:?}"
            );
        }
        assert!(dom.arena().get(dom.document()).is_ok());
    }

    #[test]
    fn importing_cyclic_template_graph_rejects_before_destination_mutation() {
        let mut source = DomRuntime::new();
        let template = source.create_element("template");
        let content = source.template_content(template).unwrap();
        // Deliberately corrupt the external arena; public DOM insertion rejects it.
        source
            .arena
            .get_mut(content)
            .unwrap()
            .children
            .push(template);
        let mut destination = DomRuntime::new();
        let retained = destination.create_element("main");
        destination
            .append_child(destination.document(), retained)
            .unwrap();
        let before = destination.arena.slots.len();
        assert_eq!(
            destination.import_subtree(source.arena(), template),
            Err(DomError::Cycle)
        );
        assert_eq!(destination.arena.slots.len(), before);
        assert_eq!(
            destination.children(destination.document()).unwrap(),
            &[retained]
        );
        assert!(destination.is_connected(retained).unwrap());
    }

    #[test]
    fn cloning_and_importing_media_do_not_copy_live_capture_pixels() {
        let mut dom = DomRuntime::new();
        let video = dom.create_element("video");
        dom.set_attribute(video, "width", "320").unwrap();
        dom.arena.get_mut(video).unwrap().media_frame = Some(vec![1, 2, 3]);
        dom.set_media_poster(video, Some(vec![4, 5]), true, Some((123, 123)))
            .unwrap();
        dom.set_showing_poster(video, false).unwrap();
        dom.arena.get_mut(video).unwrap().image_dimensions = Some((640, 480));
        for deep in [false, true] {
            let clone = dom.clone_node(video, deep).unwrap();
            let record = dom.arena.get(clone).unwrap();
            assert!(record.media_frame.is_none());
            assert!(
                record.media_poster.is_none()
                    && record.media_poster_dimensions.is_none()
                    && record.showing_poster
            );
            assert!(record.image_dimensions.is_none());
            assert_eq!(dom.attribute(clone, "width").unwrap(), Some("320"));
        }
        let mut destination = DomRuntime::new();
        let imported = destination.import_subtree(dom.arena(), video).unwrap();
        assert!(
            destination
                .arena
                .get(imported)
                .unwrap()
                .media_frame
                .is_none()
        );
        assert!(
            destination
                .arena
                .get(imported)
                .unwrap()
                .image_dimensions
                .is_none()
        );
        assert_eq!(
            destination.attribute(imported, "width").unwrap(),
            Some("320")
        );
        // Rendering snapshots project the original live node, unlike cloning.
        let (snapshot, _) = dom.snapshot();
        assert_eq!(
            snapshot.get(video).unwrap().media_frame.as_deref(),
            Some(&[1, 2, 3][..])
        );
        assert_eq!(
            snapshot.get(video).unwrap().media_poster_dimensions,
            Some((123, 123))
        );
        assert!(!snapshot.get(video).unwrap().showing_poster);
        assert_eq!(
            snapshot.get(video).unwrap().image_dimensions,
            Some((640, 480))
        );
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
    fn custom_property_registry_is_document_owned_and_not_cloned() {
        let mut dom = html::parse("<p>one</p>").into_runtime().unwrap();
        let document = dom.document();
        let registration = RegisteredCustomProperty {
            name: "--one".into(),
            syntax: "*".into(),
            inherits: false,
            initial_value: Some("value".into()),
            base_url: "https://example.test/".into(),
        };
        assert!(
            dom.register_custom_property(document, registration.clone())
                .unwrap()
        );
        assert!(
            !dom.register_custom_property(document, registration)
                .unwrap()
        );
        let clone = dom.clone_node(document, true).unwrap();
        assert!(
            dom.arena()
                .get(clone)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .registered_custom_properties
                .is_empty()
        );
        let mut other = DomRuntime::new();
        let imported = other.import_subtree(dom.arena(), document).unwrap();
        assert!(
            other
                .arena()
                .get(imported)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .registered_custom_properties
                .is_empty()
        );
        dom.replace_document_contents_in(document, "<p>two</p>")
            .unwrap();
        assert_eq!(
            dom.arena()
                .get(document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .registered_custom_properties
                .len(),
            1
        );
    }

    #[test]
    fn replacing_document_children_preserves_its_request_referrer() {
        let mut dom = html::parse("<body>old</body>").into_runtime().unwrap();
        let document = dom.document();
        dom.arena_mut()
            .get_mut(document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .referrer = "https://source.test/".into();
        dom.replace_document_contents_in(document, "<!doctype html><p>new</p>")
            .unwrap();
        assert_eq!(
            dom.arena()
                .get(document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .referrer,
            "https://source.test/"
        );
        dom.clear_document_in(document).unwrap();
        assert_eq!(
            dom.arena()
                .get(document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .referrer,
            "https://source.test/"
        );
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
    fn link_source_attribute_changes_retire_stylesheet_generations() {
        let mut dom = html::parse("<link id='sheet' rel='stylesheet' href='/first.css'>")
            .into_runtime()
            .unwrap();
        let sheet = dom.find_by_id("sheet").unwrap();
        dom.set_stylesheet_text(sheet, "a { color: red }".into())
            .unwrap();
        let first = dom.arena().stylesheet_revision(sheet).unwrap();
        dom.set_attribute(sheet, "href", "/first.css").unwrap();
        assert_eq!(dom.arena().stylesheet_revision(sheet).unwrap(), first);
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "a { color: red }"
        );
        dom.set_attribute_ns(sheet, Some("urn:other".into()), "p:href", "/ignored.css")
            .unwrap();
        assert_eq!(dom.arena().stylesheet_revision(sheet).unwrap(), first);
        let attribute = dom.attribute_node(sheet, "href").unwrap().unwrap();
        dom.set_attribute_node_value(attribute, "/next.css")
            .unwrap();
        let next = dom.arena().stylesheet_revision(sheet).unwrap();
        assert!(next > first);
        assert!(
            dom.arena()
                .get(sheet)
                .unwrap()
                .stylesheet_override
                .is_none()
        );
        dom.set_stylesheet_text(sheet, "a { color: blue }".into())
            .unwrap();
        dom.remove_attribute(sheet, "rel").unwrap();
        assert!(dom.arena().stylesheet_revision(sheet).unwrap() > next);
        assert!(
            dom.arena()
                .get(sheet)
                .unwrap()
                .stylesheet_override
                .is_none()
        );
    }

    #[test]
    fn stylesheet_media_changes_preserve_sheet_identity_but_type_changes_do_not() {
        let mut dom = html::parse("<style id='sheet'>a { color: red }</style>")
            .into_runtime()
            .unwrap();
        let sheet = dom.find_by_id("sheet").unwrap();
        dom.set_stylesheet_text(sheet, "a { color: blue }".into())
            .unwrap();
        let first = dom.arena().stylesheet_revision(sheet).unwrap();
        dom.set_attribute(sheet, "media", "print").unwrap();
        assert_eq!(dom.arena().stylesheet_revision(sheet).unwrap(), first);
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "a { color: blue }"
        );
        dom.set_attribute(sheet, "type", "text/plain").unwrap();
        assert!(dom.arena().stylesheet_revision(sheet).unwrap() > first);
        assert_eq!(
            dom.arena().stylesheet_text(sheet).unwrap(),
            "a { color: red }"
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
    fn removal_batch_replay_never_consumes_a_later_same_node_lease() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("section");
        dom.retain_wrapper(node).unwrap();
        dom.append_child(dom.document(), node).unwrap();
        dom.remove(node).unwrap();
        let first = dom.drain_mutations();
        let clone = first.clone();
        dom.release_mutation_roots(&first).unwrap();
        dom.append_child(dom.document(), node).unwrap();
        dom.remove(node).unwrap();
        let second = dom.drain_mutations();
        dom.release_mutation_roots(&clone).unwrap();
        assert_eq!(dom.roots[&node].native, 1);
        // The actual wire round trip preserves structural data but never
        // conveys native cleanup authority, even with a forged lease field.
        let mut wire = serde_json::to_value(&second).unwrap();
        for mutation in wire.as_array_mut().unwrap() {
            assert!(mutation.get("lease").is_none());
            if mutation["kind"] == "removed" {
                mutation
                    .as_object_mut()
                    .unwrap()
                    .insert("lease".into(), serde_json::json!({"consumed":false}));
            }
        }
        let reconstructed: Vec<Mutation> = serde_json::from_value(wire).unwrap();
        assert_eq!(reconstructed, second);
        dom.release_mutation_roots(&reconstructed).unwrap();
        assert_eq!(dom.roots[&node].native, 1);
        dom.release_mutation_roots(&second).unwrap();
        assert_eq!(dom.roots[&node].native, 0);
        dom.release_wrapper(node).unwrap();
    }

    #[test]
    fn partially_failed_removal_batch_retires_committed_tokens_before_retry() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("section");
        let broken = dom.create_element("section");
        dom.retain_wrapper(first).unwrap();
        for node in [first, broken] {
            dom.append_child(dom.document(), node).unwrap();
            dom.remove(node).unwrap();
        }
        let batch = dom.drain_mutations();
        dom.arena.get_mut(broken).unwrap().parent = Some(broken);
        assert!(matches!(
            dom.release_mutation_roots(&batch),
            Err(DomError::Cycle)
        ));
        assert_eq!(dom.roots[&first].native, 0);
        assert_eq!(dom.roots[&broken].native, 0);
        dom.arena.get_mut(broken).unwrap().parent = None;
        dom.append_child(dom.document(), first).unwrap();
        dom.remove(first).unwrap();
        let later = dom.drain_mutations();
        dom.release_mutation_roots(&batch).unwrap();
        assert_eq!(dom.roots[&first].native, 1);
        dom.release_mutation_roots(&later).unwrap();
        dom.release_wrapper(first).unwrap();
        dom.reclaim_if_unrooted(broken).unwrap();
    }

    #[test]
    fn discarded_removal_error_consumes_only_its_exact_token() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("section");
        dom.append_child(dom.document(), node).unwrap();
        dom.remove(node).unwrap();
        for _ in 0..MutationJournal::MAX_ENTRIES {
            dom.focus(None).unwrap();
        }
        dom.arena.get_mut(node).unwrap().parent = Some(node);
        assert!(matches!(
            dom.release_mutation_roots(&[]),
            Err(DomError::Cycle)
        ));
        assert!(dom.mutations.discarded_removed_roots.is_empty());
        assert!(!dom.mutations.removed_leases.contains_key(&node));
        dom.release_mutation_roots(&[]).unwrap();
        assert_eq!(dom.roots[&node].native, 0);
        dom.arena.get_mut(node).unwrap().parent = None;
        dom.reclaim_if_unrooted(node).unwrap();
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
        assert_eq!(
            dom.mutations
                .discarded_removed_roots
                .iter()
                .map(|(node, _)| *node)
                .collect::<Vec<_>>(),
            vec![removed]
        );
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
    fn html_serialization_uses_namespace_specific_names_and_void_rules() {
        let mut dom = DomRuntime::new();
        for (namespace, name, expected) in [
            ("http://www.w3.org/1999/xhtml", "br", "<br>"),
            ("http://www.w3.org/2000/svg", "br", "<br>child</br>"),
            (
                "http://www.w3.org/1998/Math/MathML",
                "input",
                "<input>child</input>",
            ),
            ("urn:example", "p:img", "<p:img>child</p:img>"),
        ] {
            let element = dom.create_element_ns(namespace, name);
            let child = dom.create_text("child");
            dom.append_child(element, child).unwrap();
            assert_eq!(dom.serialize(element).unwrap(), expected);
            assert_eq!(dom.arena().text_content(element).unwrap(), "child");
        }
        let element = dom.create_element_ns("http://www.w3.org/2000/svg", "svg");
        for (namespace, name, value) in [
            (
                "http://www.w3.org/XML/1998/namespace",
                "alternate:lang",
                "en",
            ),
            ("http://www.w3.org/1999/xlink", "alternate:href", "#target"),
            ("http://www.w3.org/2000/xmlns/", "xmlns:x", "urn:x"),
            ("urn:custom", "p:data", "a&b"),
        ] {
            dom.set_attribute_ns(element, Some(namespace.into()), name, value)
                .unwrap();
        }
        assert_eq!(
            dom.serialize(element).unwrap(),
            "<svg xml:lang=\"en\" xlink:href=\"#target\" xmlns:x=\"urn:x\" p:data=\"a&amp;b\"></svg>"
        );
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
    fn character_data_and_normalize_preserve_lone_surrogates_and_native_range_offsets() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("div");
        dom.append_child(dom.document(), parent).unwrap();
        let first = dom.create_text(dom_string::from_units(&[97, 0xd800, 98]));
        let second = dom.create_text(dom_string::from_units(&[0xdc00, 99]));
        dom.append_child(parent, first).unwrap();
        dom.append_child(parent, second).unwrap();
        let range = dom
            .create_live_range(
                BoundaryPoint {
                    container: first,
                    offset: 3,
                },
                BoundaryPoint {
                    container: second,
                    offset: 2,
                },
            )
            .unwrap();
        dom.replace_character_data(first, 1, 1, dom_string::from_units(&[0xdc01, 0xd802]))
            .unwrap();
        let NodeKind::Text { data } = &dom.arena().get(first).unwrap().kind else {
            panic!("text missing")
        };
        assert_eq!(dom_string::units(data), vec![97, 0xdc01, 0xd802, 98]);
        assert_eq!(dom.live_range(range).unwrap().start.offset, 4);
        dom.normalize(parent).unwrap();
        let NodeKind::Text { data } = &dom.arena().get(first).unwrap().kind else {
            panic!("text missing")
        };
        assert_eq!(
            dom_string::units(data),
            vec![97, 0xdc01, 0xd802, 98, 0xdc00, 99]
        );
        assert_eq!(
            dom.live_range(range).unwrap().end,
            BoundaryPoint {
                container: first,
                offset: 6
            }
        );
    }

    #[test]
    fn split_text_migrates_native_ranges_and_numeric_boundary_history_losslessly() {
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("div");
        dom.append_child(dom.document(), parent).unwrap();
        let text = dom.create_text(dom_string::from_units(&[97, 0xd800, 98, 99]));
        dom.append_child(parent, text).unwrap();
        let initial = BoundaryPoint {
            container: text,
            offset: 3,
        };
        let range = dom.create_live_range(initial, initial).unwrap();
        let revision = dom.boundary_revision();
        let tail = dom.split_text(text, 2).unwrap();
        let expected = BoundaryPoint {
            container: tail,
            offset: 1,
        };
        assert_eq!(dom.live_range(range).unwrap().start, expected);
        assert_eq!(
            dom.boundary_snapshot().rebase(initial, revision),
            Some(expected)
        );
        let NodeKind::Text { data } = &dom.arena().get(text).unwrap().kind else {
            panic!("text missing")
        };
        assert_eq!(dom_string::units(data), vec![97, 0xd800]);
        dom.normalize(parent).unwrap();
        assert_eq!(
            dom.boundary_snapshot().rebase(initial, revision),
            Some(BoundaryPoint {
                container: text,
                offset: 3
            })
        );
        assert_eq!(
            dom.live_range(range).unwrap().start,
            BoundaryPoint {
                container: text,
                offset: 3
            }
        );
    }

    #[test]
    fn composed_selection_range_adjusts_shadow_removal_without_mutating_script_range() {
        let mut dom =
            html::parse("<body><div id=container><span id=host></span><i></i></div></body>")
                .into_runtime()
                .unwrap();
        let container = dom.find_by_id("container").unwrap();
        let host = dom.find_by_id("host").unwrap();
        let root = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: false,
        });
        dom.append_child(host, root).unwrap();
        let text = dom.create_text("abcdef");
        dom.append_child(root, text).unwrap();
        let point = BoundaryPoint {
            container: text,
            offset: 2,
        };
        let script = dom.create_live_range(point, point).unwrap();
        let composed = dom
            .create_live_range(
                point,
                BoundaryPoint {
                    container,
                    offset: 1,
                },
            )
            .unwrap();
        dom.mark_composed_selection_range(composed).unwrap();
        dom.remove_child(container, host).unwrap();
        assert_eq!(dom.live_range(script).unwrap().start, point);
        assert_eq!(dom.range_shadow_removal_revision(script), 1);
        let collapsed = BoundaryPoint {
            container,
            offset: 0,
        };
        assert_eq!(
            dom.live_range(composed).unwrap(),
            LiveRange {
                start: collapsed,
                end: collapsed
            }
        );
        dom.append_child(container, host).unwrap();
        assert_eq!(dom.range_shadow_removal_revision(script), 1);
        dom.drop_live_range(composed).unwrap();
        assert!(!dom.composed_selection_ranges.contains(&composed));
    }
    #[test]
    fn live_range_id_exhaustion_preserves_existing_range_and_endpoint_roots() {
        let mut dom = DomRuntime::new();
        let text = dom.create_text("range");
        let boundary = BoundaryPoint {
            container: text,
            offset: 2,
        };
        dom.next_range = (1_u64 << 53) - 1;
        let last = dom.create_live_range(boundary, boundary).unwrap();
        assert_eq!(last.0, (1_u64 << 53) - 1);
        let roots = dom.roots.get(&text).unwrap().native;
        assert!(matches!(
            dom.create_live_range(boundary, boundary),
            Err(DomError::MigrationCapacity)
        ));
        assert_eq!(dom.roots.get(&text).unwrap().native, roots);
        assert_eq!(
            dom.live_range(last).unwrap(),
            LiveRange {
                start: boundary,
                end: boundary
            }
        );
        assert_eq!(dom.ranges.len(), 1);
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

mod accessibility;

mod child_text;
mod document_head;
mod nonces;
mod stylesheet_adoption;

#[cfg(test)]
mod document_domain_cell_tests {
    #[test]
    fn inherited_domain_cells_outlive_creator_and_release_with_last_document() {
        let creator = super::DocumentMetadata::default();
        let inert = creator.clone();
        let live_alias = creator.clone();
        let weak = std::sync::Arc::downgrade(&creator.relaxed_domain.0);
        live_alias
            .relaxed_domain
            .set(Some("source.example.test".into()));
        assert_eq!(
            inert.relaxed_domain.get().as_deref(),
            Some("source.example.test")
        );
        drop(creator);
        live_alias.relaxed_domain.set(Some("example.test".into()));
        assert_eq!(inert.relaxed_domain.get().as_deref(), Some("example.test"));
        assert!(
            super::DocumentMetadata::default()
                .relaxed_domain
                .get()
                .is_none()
        );
        drop(live_alias);
        assert!(weak.upgrade().is_some());
        drop(inert);
        assert!(weak.upgrade().is_none());
    }
}

#[cfg(test)]
mod accessibility_control_marker_journal_tests {
    use super::*;
    #[test]
    fn live_control_markers_follow_exact_edits_without_text_matching() {
        let mut dom = html::parse("<textarea id=t>aaaa</textarea><p id=p>other</p>")
            .into_runtime()
            .unwrap();
        let t = dom.find_by_id("t").unwrap();
        let p = dom.find_by_id("p").unwrap();
        let revision = dom.boundary_revision();
        let point = BoundaryPoint {
            container: t,
            offset: 2,
        };
        dom.replace_control_value(t, 0, 0, "a").unwrap();
        let unrelated_text = dom.arena().get(p).unwrap().children[0];
        dom.set_text(unrelated_text, "changed").unwrap();
        let text = dom.arena().get(t).unwrap().children[0];
        dom.set_text(text, "new default").unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "aaaaa");
        assert_eq!(
            dom.boundary_snapshot().rebase_control(point, revision),
            Some(BoundaryPoint {
                container: t,
                offset: 3
            })
        );
        // DOM boundaries in the same control do not use value offsets.
        assert_eq!(
            dom.boundary_snapshot().rebase(
                BoundaryPoint {
                    container: t,
                    offset: 0
                },
                revision
            ),
            Some(BoundaryPoint {
                container: t,
                offset: 0
            })
        );
        let revision = dom.boundary_revision();
        dom.set_value(t, "replacement", false).unwrap();
        assert_eq!(
            dom.boundary_snapshot().rebase_control(
                BoundaryPoint {
                    container: t,
                    offset: 3
                },
                revision
            ),
            Some(BoundaryPoint {
                container: t,
                offset: 0
            })
        );
    }
    #[test]
    fn default_changes_invalidate_only_clean_control_markers_and_passwords_hide_lengths() {
        let mut dom = html::parse("<textarea id=t>default</textarea><input id=s type=password>")
            .into_runtime()
            .unwrap();
        let t = dom.find_by_id("t").unwrap();
        let s = dom.find_by_id("s").unwrap();
        let revision = dom.boundary_revision();
        let text = dom.arena().get(t).unwrap().children[0];
        dom.set_text(text, "changed").unwrap();
        assert!(
            dom.boundary_snapshot()
                .rebase_control(
                    BoundaryPoint {
                        container: t,
                        offset: 2
                    },
                    revision
                )
                .is_none()
        );
        let revision = dom.boundary_revision();
        dom.set_value(s, "secret", true).unwrap();
        dom.replace_control_value(s, 1, 2, "longer secret").unwrap();
        let snapshot = dom.boundary_snapshot();
        assert!(snapshot.records.iter().filter(|record|record.sequence>revision).all(|record|!matches!(record.change,boundary_journal::Change::ControlValueReplace{node,..}if node==s)));
    }
}

#[cfg(test)]
mod deep_connection_regressions {
    use super::*;
    #[test]
    fn deep_connection_walks_do_not_use_the_native_call_stack() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let mut last = root;
        for _ in 0..20_000 {
            let child = dom.create_element("div");
            dom.arena.get_mut(last).unwrap().children.push(child);
            dom.arena.get_mut(child).unwrap().parent = Some(last);
            last = child;
        }
        dom.mark_connected(root).unwrap();
        assert!(dom.connected.contains(&root));
        assert!(dom.connected.contains(&last));
        dom.frame_context_generations.insert(last, 7);
        dom.mark_disconnected_with_focus(root, true).unwrap();
        assert!(!dom.connected.contains(&last));
        assert_eq!(dom.frame_context_generations[&last], 7);
        dom.mark_connected(root).unwrap();
        dom.mark_disconnected(root).unwrap();
        assert!(!dom.connected.contains(&root));
        assert!(!dom.connected.contains(&last));
        assert_eq!(dom.frame_context_generations[&last], 8);
    }
    #[test]
    fn connection_walk_rejects_native_cycles_and_root_overflow_is_non_mutating() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        dom.arena.get_mut(root).unwrap().children.push(root);
        assert_eq!(dom.mark_connected(root), Err(DomError::Cycle));
        assert_eq!(dom.mark_disconnected(root), Err(DomError::Cycle));
        dom.arena.get_mut(root).unwrap().children.clear();
        dom.roots.entry(root).or_default().native = u32::MAX;
        assert_eq!(dom.retain_native(root), Err(DomError::MigrationCapacity));
        assert_eq!(dom.roots[&root].native, u32::MAX);
    }
}

#[cfg(test)]
mod connection_graph_order_regressions {
    use super::*;

    #[test]
    fn hosted_notifications_follow_descendants_and_focus_survives_preserving_moves() {
        let mut dom = DomRuntime::new();
        let outer = dom.create_element("iframe");
        let fallback = dom.create_element("iframe");
        let inner = dom.create_element("iframe");
        let outer_document = dom.create(NodeKind::Document);
        let fallback_document = dom.create(NodeKind::Document);
        let inner_document = dom.create(NodeKind::Document);
        let input = dom.create_element("input");
        dom.append_child(outer, fallback).unwrap();
        dom.append_child(outer_document, inner).unwrap();
        dom.append_child(inner_document, input).unwrap();
        dom.set_hosted_document(fallback, Some(fallback_document))
            .unwrap();
        dom.set_hosted_document(inner, Some(inner_document))
            .unwrap();
        dom.set_hosted_document(outer, Some(outer_document))
            .unwrap();
        let cursor = dom.mutation_cursor();
        dom.mark_connected(outer).unwrap();
        let frames: Vec<_> = dom
            .mutations_since(cursor)
            .mutations
            .iter()
            .filter_map(|mutation| {
                if let Mutation::HostedDocument { frame } = mutation {
                    Some(*frame)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(frames, vec![fallback, inner, outer]);
        for node in [
            outer,
            fallback,
            inner,
            outer_document,
            fallback_document,
            inner_document,
            input,
        ] {
            assert!(dom.is_connected(node).unwrap());
        }
        dom.focus(Some(input)).unwrap();
        dom.frame_context_generations.insert(inner, 3);
        dom.mark_disconnected_with_focus(outer, true).unwrap();
        assert_eq!(dom.focused(), Some(input));
        assert_eq!(dom.frame_context_generations[&inner], 3);
        dom.mark_connected(outer).unwrap();
        let cursor = dom.mutation_cursor();
        dom.mark_disconnected(outer).unwrap();
        assert_eq!(dom.focused(), None);
        assert_eq!(dom.frame_context_generations[&inner], 4);
        let frames: Vec<_> = dom
            .mutations_since(cursor)
            .mutations
            .iter()
            .filter_map(|mutation| {
                if let Mutation::HostedDocument { frame } = mutation {
                    Some(*frame)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(frames, vec![fallback, inner, outer]);
        assert_eq!(dom.parent(inner_document).unwrap(), None);
    }

    #[test]
    fn malformed_snapshot_connectivity_does_not_publish_a_partial_index() {
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let child = dom.create_element("div");
        dom.arena.get_mut(document).unwrap().children.push(child);
        dom.arena.get_mut(child).unwrap().children.push(document);
        let before = dom.connected.clone();
        assert_eq!(dom.rebuild_document_indexes(), Err(DomError::Cycle));
        assert_eq!(dom.connected, before);
        dom.arena.get_mut(child).unwrap().children.clear();
        dom.rebuild_document_indexes().unwrap();
        assert!(dom.connected.contains(&child));
    }
}

#[cfg(test)]
mod synchronous_native_handoff_regressions {
    use super::*;

    #[test]
    fn detached_native_tree_arguments_survive_cursor_cleanup_without_permanent_roots() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let first = dom.create_element("section");
        let second = dom.create_text("tail");
        dom.append_child(root, first).unwrap();
        let text = dom.create_text("nested");
        dom.append_child(first, text).unwrap();
        dom.append_child(root, second).unwrap();
        assert_eq!(dom.arena.text_content(root).unwrap(), "nestedtail");
        dom.remove(first).unwrap();
        assert_eq!(dom.arena.text_content(first).unwrap(), "nested");
        assert_eq!(dom.arena.text_content(root).unwrap(), "tail");
        for node in [root, second, text] {
            assert_eq!(dom.roots.get(&node).map_or(0, |roots| roots.native), 0);
        }
        assert_eq!(
            dom.roots.get(&first).map_or(0, |roots| roots.native),
            1,
            "removal journal retains exactly its own lease"
        );
        dom.retain_native(first).unwrap();
        dom.retain_native(root).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(
            dom.roots.get(&first).unwrap().native,
            1,
            "only the explicit test guard remains"
        );
        dom.release_native(first).unwrap();
        assert!(dom.arena.get(first).is_err());
        assert!(dom.arena.get(text).is_err());
        assert!(dom.arena.get(root).is_ok());
        dom.release_native(root).unwrap();
        assert!(dom.arena.get(root).is_err());
        assert!(dom.arena.get(second).is_err());
    }

    #[test]
    fn imported_document_frontier_retains_template_graph_until_native_handoff() {
        let source = html::parse(
            "<body><template id=t><style>b{color:red}</style><b>inside</b></template><p>after</p></body>",
        );
        let mut dom = DomRuntime::new();
        let imported = dom.import_subtree(&source.dom, source.document).unwrap();
        let template = dom.get_element_by_id(imported, "t").unwrap().unwrap();
        let content = dom.template_content(template).unwrap();
        assert_eq!(
            dom.arena.text_content(content).unwrap(),
            "b{color:red}inside"
        );
        assert_eq!(dom.query_selector_all(imported, "p").unwrap().len(), 1);
        assert!(dom.roots.values().all(|roots| roots.native == 0));
        dom.retain_native(imported).unwrap();
        dom.release_native(imported).unwrap();
        assert!(dom.arena.get(imported).is_err());
        assert!(dom.arena.get(content).is_err());
    }

    #[test]
    fn failed_import_releases_allocation_frontier_after_committed_tail_cleanup() {
        let source = html::parse("<body><div><span>child</span></div><p>sibling</p></body>");
        for failure in 0..32 {
            let mut dom = DomRuntime::new();
            let original = dom.document();
            insertion::set_failure_after(Some(failure));
            let result = dom.import_subtree(&source.dom, source.document);
            insertion::set_failure_after(None);
            dom.drain_committed_tree_tails().unwrap();
            dom.drain_cursor_releases().unwrap();
            if let Ok(imported) = result {
                dom.retain_native(imported).unwrap();
                dom.release_native(imported).unwrap();
            }
            let mutations = dom.drain_mutations();
            dom.release_mutation_roots(&mutations).unwrap();
            assert!(
                dom.roots.values().all(|roots| roots.native == 0),
                "failure {failure} leaked a native root"
            );
            assert_eq!(
                dom.arena.iter().map(|(id, _)| id).collect::<Vec<_>>(),
                vec![original],
                "failure {failure} leaked an imported allocation"
            );
        }
    }
    #[test]
    fn failed_import_final_handoff_reclaims_unpublished_text_root() {
        let mut source = DomArena::default();
        let text = source.insert(NodeKind::Text {
            data: "unpublished".into(),
        });
        let mut dom = DomRuntime::new();
        let original = dom.document();
        // Text has no attributes or children: shallow allocation is checkpoint
        // zero, and the next checkpoint is after releasing the import frontier.
        insertion::set_failure_after(Some(1));
        let result = dom.import_subtree(&source, text);
        insertion::set_failure_after(None);
        assert!(result.is_err());
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(
            dom.arena.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            vec![original]
        );
        assert!(dom.roots.values().all(|roots| roots.native == 0));
    }

    #[test]
    fn failed_shallow_import_reclaims_unpublished_node_and_attribute_initialization() {
        let mut source = DomRuntime::new();
        let element = source.create_element("input");
        source.set_attribute(element, "type", "text").unwrap();
        source.set_attribute(element, "value", "saved").unwrap();
        source.attribute_node(element, "type").unwrap();
        source.attribute_node(element, "value").unwrap();
        let (arena, _) = source.snapshot();
        for failure in 0..3 {
            let mut dom = DomRuntime::new();
            let original = dom.document();
            insertion::set_failure_after(Some(failure));
            assert!(dom.import_shallow_node(&arena, element).is_err());
            insertion::set_failure_after(None);
            let mutations = dom.drain_mutations();
            dom.release_mutation_roots(&mutations).unwrap();
            assert_eq!(
                dom.arena.iter().map(|(id, _)| id).collect::<Vec<_>>(),
                vec![original]
            );
            assert!(dom.roots.values().all(|roots| roots.native == 0));
        }
    }
}

#[cfg(test)]
mod uninitialized_registry_tests {
    use super::*;
    #[test]
    fn parser_and_import_preserve_absent_registry_until_destination_is_initialized() {
        let parsed = html::parse("<body><div><span>text</span></div></body>");
        assert!(
            parsed
                .dom
                .iter()
                .filter(|(_, record)| matches!(record.kind, NodeKind::Element { .. }))
                .all(|(_, record)| record.custom_element_registry.is_none())
        );
        let mut dom = DomRuntime::new();
        let imported = dom.import_subtree(&parsed.dom, parsed.document).unwrap();
        let span = dom.query_selector(imported, "span").unwrap().unwrap();
        assert_eq!(dom.arena().get(span).unwrap().custom_element_registry, None);
        let destination = dom.document();
        dom.register_global_custom_element_registry(941);
        dom.set_custom_element_registry(destination, Some(941))
            .unwrap();
        dom.set_node_document(span, destination).unwrap();
        assert_eq!(
            dom.arena().get(span).unwrap().custom_element_registry,
            Some(Some(941))
        );
    }
    #[test]
    fn explicit_null_and_scoped_associations_are_not_uninitialized() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("span");
        dom.set_custom_element_registry(node, None).unwrap();
        dom.set_node_document(node, dom.document()).unwrap();
        assert_eq!(
            dom.arena().get(node).unwrap().custom_element_registry,
            Some(None)
        );
        dom.set_custom_element_registry(node, Some(942)).unwrap();
        let other = dom.create(NodeKind::Document);
        dom.set_node_document(node, other).unwrap();
        assert_eq!(
            dom.arena().get(node).unwrap().custom_element_registry,
            Some(Some(942))
        );
    }
}

impl DomArena {
    pub fn text_content_with_allocation<A: crate::projection_allocation::ProjectionAllocation>(
        &self,
        id: NodeId,
        allocation: &mut A,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<String, A::Error> {
        use crate::projection_allocation::{admitted_append, admitted_push};
        checkpoint().map_err(A::Error::from)?;
        let node = self.get(id).map_err(A::Error::from)?;
        let raw = match &node.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => Some(data.as_str()),
            NodeKind::Attribute { data, .. } => Some(data.value.as_str()),
            _ => None,
        };
        if let Some(raw) = raw {
            let mut output = String::new();
            admitted_append(&mut output, raw, allocation, checkpoint)?;
            checkpoint().map_err(A::Error::from)?;
            return Ok(output);
        }
        let mut result = Vec::new();
        let mut pending = Vec::new();
        for &child in node.children.iter().rev() {
            admitted_push(&mut pending, child, allocation, checkpoint)?;
        }
        let mut seen = BTreeSet::from([id]);
        while let Some(child) = pending.pop() {
            checkpoint().map_err(A::Error::from)?;
            if !seen.insert(child) {
                return Err(A::Error::from(DomError::Cycle));
            }
            let record = self.get(child).map_err(A::Error::from)?;
            match &record.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                    let units = dom_string::units_with_allocation(data, allocation, checkpoint)?;
                    for unit in units {
                        admitted_push(&mut result, unit, allocation, checkpoint)?;
                    }
                }
                NodeKind::Element { .. } => {
                    for &child in record.children.iter().rev() {
                        admitted_push(&mut pending, child, allocation, checkpoint)?;
                    }
                }
                _ => {}
            }
        }
        dom_string::from_units_with_allocation(&result, allocation, checkpoint)
    }
}
