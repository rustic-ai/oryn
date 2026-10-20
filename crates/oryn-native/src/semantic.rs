use std::collections::{BTreeMap, BTreeSet};

use oryn_common::v3::{
    CONTRACT_VERSION, ContentTrust, ExecutionDomain, NodeState, Observation, ObservationSource,
    PageId, PageInfo, ProjectionProfile, Provenance, Revision, SemanticAction, SemanticNodeView,
    SemanticRef,
};
use oryn_common::v5::{
    AccessibilityEvidence, AccessibilityEvidenceKind, AccessibilityNode, AccessibilityRelation,
    SemanticGroup, SemanticObservation, SemanticStructure, SemanticStructureKind,
};
use oryn_common::v6::{
    ProjectionSummary, SemanticField, SemanticFieldEvidence, SemanticGraphChanges as V6Changes,
    SemanticNode as SemanticNodeV6, SemanticObservation as SemanticObservationV6,
    SemanticObservationOptions,
};

use crate::dom::{DomArena, NodeId, NodeKind};

fn accessible_children_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, crate::dom::DomError> {
    shared_accessible_children_with_checkpoint(dom, id, &mut OrdinarySemanticProducer, check)
}

fn tree_scope_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<NodeId>, crate::dom::DomError> {
    let mut cursor = Some(id);
    let mut last = None;
    let mut seen = BTreeSet::new();
    while let Some(candidate) = cursor {
        check()?;
        if !seen.insert(candidate) {
            return Ok(None);
        }
        let Ok(node) = dom.get(candidate) else {
            return Ok(None);
        };
        if matches!(node.kind, NodeKind::ShadowRoot { .. }) {
            return Ok(Some(candidate));
        }
        last = Some(candidate);
        cursor = node.parent;
    }
    Ok(last)
}

#[cfg(test)]
fn accessible_text_content_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    accessible_text_content_excluding_with_checkpoint(dom, id, include_hidden_root, None, check)
}

#[cfg(test)]
fn accessible_text_content_excluding_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    excluded: Option<NodeId>,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    shared_accessible_text(
        dom,
        id,
        include_hidden_root,
        excluded,
        &mut OrdinarySemanticProducer,
        check,
    )
}

#[cfg(test)]
fn embedded_selected_options_with_checkpoint(
    dom: &DomArena,
    root: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, crate::dom::DomError> {
    shared_embedded_selected_options(dom, root, &mut OrdinarySemanticProducer, check)
}

#[cfg(test)]
fn host_label_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<(String, Vec<NodeId>)>, crate::dom::DomError> {
    shared_host_label_with_checkpoint(dom, id, &mut OrdinarySemanticProducer, check)
}

fn accessible_semantics_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
    page_id: PageId,
    document_generation: u64,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<AccessibleSemantics, crate::dom::DomError> {
    shared_accessible_semantics_with_checkpoint(
        dom,
        id,
        local_name,
        role,
        page_id,
        document_generation,
        &mut OrdinarySemanticProducer,
        check,
    )
}

pub(crate) fn computed_role_for_node_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    shared_role_for_node(dom, id, &mut OrdinarySemanticProducer, check)
}

pub fn project_element_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    document_generation: u64,
    alias: u32,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<SemanticNodeView>, crate::dom::DomError> {
    project_element_with_visibility_and_checkpoint(
        dom,
        id,
        page_id,
        document_generation,
        alias,
        false,
        check,
    )
}

fn project_element_with_visibility_and_checkpoint(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    document_generation: u64,
    alias: u32,
    bypass_visibility: bool,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<SemanticNodeView>, crate::dom::DomError> {
    shared_project_element(
        dom,
        id,
        page_id,
        document_generation,
        alias,
        bypass_visibility,
        &mut OrdinarySemanticProducer,
        check,
    )
}

fn aria_owned_target_is_exposable_with_checkpoint(
    dom: &DomArena,
    target: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if dom.get(target).is_err() || accessibility_hidden_at_with_checkpoint(dom, target, check)? {
        return Ok(false);
    }
    for ancestor in cancellation_or_default(ancestors_with_checkpoint(dom, target, check))? {
        check()?;
        let Ok(node) = dom.get(ancestor) else {
            return Ok(false);
        };
        if let Some((display_none, _)) = dom.accessibility_computed_visibility(ancestor) {
            if display_none {
                return Ok(false);
            }
            continue;
        }
        if node
            .attributes
            .contains_null_with_checkpoint("hidden", check)?
        {
            return Ok(false);
        }
        if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
            for declaration in checked_name_tokens(style, |character| character == ';', check)? {
                check()?;
                let tokens = checked_name_tokens(declaration, |character| character == ':', check)?;
                if tokens.len() > 1 {
                    let name = tokens[0];
                    let value = &declaration[name.len() + 1..];
                    let name = naming_ascii_lowercase_with_checkpoint(
                        naming_trim_with_checkpoint(name, check)?,
                        check,
                    )?;
                    let value = naming_ascii_lowercase_with_checkpoint(
                        naming_trim_with_checkpoint(value, check)?,
                        check,
                    )?;
                    if (name == "display" && value == "none")
                        || (name == "visibility" && matches!(value.as_str(), "hidden" | "collapse"))
                    {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}

pub(crate) fn accessibility_readonly_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    shared_accessibility_readonly(dom, id, &mut OrdinarySemanticProducer, check)
}

pub(crate) fn is_hidden_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    shared_is_hidden(dom, id, &mut OrdinarySemanticProducer, check)
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn tree_exclusion_reason_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    role: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<String>, crate::dom::DomError> {
    shared_tree_exclusion(dom, id, role, &mut OrdinarySemanticProducer, check)
}

fn find_element_by_id_scoped_with_checkpoint(
    dom: &DomArena,
    source: NodeId,
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<NodeId>, crate::dom::DomError> {
    let Some(scope) = tree_scope_with_checkpoint(dom, source, check)? else {
        return Ok(None);
    };
    let mut candidates = vec![scope];
    for descendant in descendants_with_checkpoint(dom, scope, check)? {
        check()?;
        candidates.push(descendant);
    }
    for candidate in candidates {
        check()?;
        if tree_scope_with_checkpoint(dom, candidate, check)? == Some(scope)
            && let Ok(node) = dom.get(candidate)
            && matches!(node.kind, NodeKind::Element { .. })
            && node
                .attributes
                .get_null_with_checkpoint("id", check)?
                .map(|id| naming_equal_with_checkpoint(id, value, check))
                .transpose()?
                .unwrap_or(false)
        {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn accessibility_relationships_with_checkpoint(
    dom: &DomArena,
    source: NodeId,
    node: &crate::dom::Node,
    page_id: PageId,
    document_generation: u64,
    diagnostics: &mut Vec<String>,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<BTreeMap<AccessibilityRelation, Vec<SemanticRef>>, crate::dom::DomError> {
    shared_accessibility_relationships(
        dom,
        source,
        node,
        page_id,
        document_generation,
        diagnostics,
        &mut OrdinarySemanticProducer,
        check,
    )
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn accessibility_properties_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    node: &crate::dom::Node,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<BTreeMap<String, serde_json::Value>, crate::dom::DomError> {
    shared_accessibility_properties(
        dom,
        id,
        local_name,
        node,
        &mut OrdinarySemanticProducer,
        check,
    )
}

pub const SEMANTIC_GRAPH_VERSION: u16 = 2;

fn normalize_text_with_checkpoint(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    check()?;
    // Preserve the original single-reservation allocation behavior. Allocation
    // itself is not a cancellation checkpoint and remains an explicit limit.
    let mut output = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        check()?;
        if matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }
    Ok(output)
}

#[cfg(test)]
fn strip_css_comments_with_checkpoint(
    sheet: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    check()?;
    // Keep the original capacity efficiency; cancellation bounds scanning only.
    let mut output = String::with_capacity(sheet.len());
    let mut chars = sheet.chars().peekable();
    let mut comment = false;
    while let Some(character) = chars.next() {
        check()?;
        if comment {
            if character == '*' && chars.peek() == Some(&'/') {
                check()?;
                chars.next();
                comment = false;
            }
        } else if character == '/' && chars.peek() == Some(&'*') {
            check()?;
            chars.next();
            comment = true;
        } else {
            output.push(character);
        }
    }
    Ok(output)
}

fn checked_name_tokens<'a>(
    value: &'a str,
    delimiter: impl Fn(char) -> bool,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<&'a str>, crate::dom::DomError> {
    let mut tokens = Vec::new();
    let mut start = 0;
    for (index, character) in value.char_indices() {
        check()?;
        if delimiter(character) {
            tokens.push(&value[start..index]);
            start = index + character.len_utf8();
        }
    }
    tokens.push(&value[start..]);
    Ok(tokens)
}

#[cfg(test)]
fn matches_simple_selector_with_checkpoint(
    node: &crate::dom::Node,
    selector: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    shared_matches_simple_selector(node, selector, &mut OrdinarySemanticProducer, check)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticResolutionError {
    NotFound,
    Ambiguous(Vec<SemanticRef>),
    GeometryUnavailable,
    UnsupportedSelector(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticGraphChanges {
    pub upserted: Vec<SemanticNodeView>,
    pub removed: Vec<SemanticRef>,
}

/// Persistent semantic projection for one document generation. The graph is
/// updated from the Rust DOM mutation journal and is therefore shared by
/// observations, deltas, action resolution, and causal tracing without
/// repeatedly walking the full document.
#[derive(Debug, Clone)]
pub struct SemanticGraph {
    allocation_receipts: Option<std::sync::Arc<SemanticGraphAllocations>>,
    page_id: PageId,
    document_generation: u64,
    full: BTreeMap<NodeId, SemanticNodeView>,
    compact: BTreeSet<NodeId>,
    // Author-provided targeting hints are separate from accessibility names.
    // Never include control values (especially password/secret values).
    target_hints: BTreeMap<NodeId, Vec<(String, &'static str)>>,
    referenced_by: BTreeMap<NodeId, BTreeSet<NodeId>>,
    accessibility_state_revisions: BTreeMap<NodeId, u64>,
    accessibility: BTreeMap<NodeId, AccessibilityNode>,
    structures: BTreeMap<String, SemanticStructure>,
    frame_ids: BTreeMap<NodeId, u64>,
    authored_hidden: BTreeSet<NodeId>,
    geometry_hidden: BTreeSet<NodeId>,
    semantic_revision: u64,
    native_control_cursor: Option<crate::dom::MutationCursor>,
    generated_content: BTreeMap<NodeId, (String, String)>,
    computed_visibility: BTreeMap<NodeId, (bool, bool)>,
    computed_text_style: BTreeMap<NodeId, (String, String)>,
    pending_upserts: BTreeMap<SemanticRef, SemanticNodeView>,
    pending_removed: BTreeSet<SemanticRef>,
    pending_accessibility_upserts: BTreeMap<SemanticRef, AccessibilityNode>,
    pending_accessibility_removed: BTreeSet<SemanticRef>,
    pending_structure_upserts: BTreeMap<String, SemanticStructure>,
    pending_structure_removed: BTreeSet<String>,
    diagnostics: Vec<String>,
}

impl SemanticGraph {
    pub fn new(page_id: PageId, document_generation: u64) -> Self {
        Self {
            page_id,
            document_generation,
            allocation_receipts: None,
            full: BTreeMap::new(),
            compact: BTreeSet::new(),
            target_hints: BTreeMap::new(),
            referenced_by: BTreeMap::new(),
            accessibility_state_revisions: BTreeMap::new(),
            accessibility: BTreeMap::new(),
            structures: BTreeMap::new(),
            frame_ids: BTreeMap::new(),
            authored_hidden: BTreeSet::new(),
            geometry_hidden: BTreeSet::new(),
            semantic_revision: 0,
            native_control_cursor: None,
            generated_content: BTreeMap::new(),
            computed_visibility: BTreeMap::new(),
            computed_text_style: BTreeMap::new(),
            pending_upserts: BTreeMap::new(),
            pending_removed: BTreeSet::new(),
            pending_accessibility_upserts: BTreeMap::new(),
            pending_accessibility_removed: BTreeSet::new(),
            pending_structure_upserts: BTreeMap::new(),
            pending_structure_removed: BTreeSet::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Operates only on a caller-owned draft; discard that draft on error.
    pub(crate) fn reset_with_checkpoint(
        &mut self,
        dom: &DomArena,
        generation: u64,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        self.shared_reset(dom, generation, &mut OrdinarySemanticProducer, check)
    }

    pub(crate) fn apply_mutations_with_checkpoint(
        &mut self,
        dom: &DomArena,
        mutations: &[crate::dom::Mutation],
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        self.shared_apply_mutations(dom, mutations, &mut OrdinarySemanticProducer, check)
    }

    fn remove_subtree_with_checkpoint(
        &mut self,
        root: NodeId,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        let root_ref = (u64::from(root.generation) << 32) | u64::from(root.slot);
        let mut removed = BTreeSet::from([root_ref]);
        loop {
            check()?;
            let before = removed.len();
            for node in self.full.values() {
                check()?;
                if node
                    .parent
                    .is_some_and(|parent| removed.contains(&parent.node))
                {
                    removed.insert(node.semantic_ref.node);
                }
            }
            if removed.len() == before {
                break;
            }
        }
        let mut ids = Vec::new();
        for (id, node) in &self.full {
            check()?;
            if removed.contains(&node.semantic_ref.node) {
                ids.push(*id);
            }
        }
        for id in ids {
            check()?;
            if let Some(node) = self.full.remove(&id) {
                self.target_hints.remove(&id);
                self.compact.remove(&id);
                self.pending_upserts.remove(&node.semantic_ref);
                self.pending_removed.insert(node.semantic_ref);
            }
        }
        let mut ids = Vec::new();
        for (id, node) in &self.accessibility {
            check()?;
            if removed.contains(&node.semantic_ref.node) {
                ids.push(*id);
            }
        }
        for id in ids {
            check()?;
            if let Some(node) = self.accessibility.remove(&id) {
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts
                        .remove(&node.semantic_ref),
                ));
                self.pending_accessibility_removed.insert(node.semantic_ref);
                drop(AccessibilityOwner(Some(node)));
            }
        }
        check()?;
        Ok(())
    }

    fn rebuild_dependencies_with_checkpoint(
        &mut self,
        dom: &DomArena,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        self.referenced_by.clear();
        self.accessibility_state_revisions.clear();
        {
            let mut cursor = 0usize;
            while let Some((source, record)) = dom.next_with_checkpoint(&mut cursor, check)? {
                check()?;
                let revision = record.accessibility_reference_revision();
                if revision != 0 {
                    self.accessibility_state_revisions.insert(source, revision);
                }
                if matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="th")
                    && let Some(parent) = record.parent
                {
                    self.referenced_by.entry(parent).or_default().insert(source);
                }
                if matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="img")
                && let Some(parent) = record.parent.filter(|parent| dom.get(*parent).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.local.as_ref()=="figure")))
            {
                self.referenced_by.entry(parent).or_default().insert(source);
            }
                for attribute in [
                    "aria-labelledby",
                    "aria-describedby",
                    "aria-owns",
                    "aria-errormessage",
                    "aria-actions",
                ] {
                    check()?;
                    if let Some(targets) =
                        dom.accessibility_relation_with_checkpoint(source, attribute, check)?
                    {
                        for target in targets {
                            check()?;
                            self.referenced_by.entry(target).or_default().insert(source);
                        }
                    }
                }
                check()?;
                if let Some(target) =
                    cancellation_or_default(dom.label_control_with_checkpoint(source, check))?
                {
                    self.referenced_by.entry(target).or_default().insert(source);
                    self.referenced_by.entry(source).or_default().insert(target);
                }
            }
        }
        check()?;
        Ok(())
    }

    /// Clone retained graph collections incrementally. The caller owns the
    /// draft and publishes it only after all fallible projection work succeeds.
    #[cfg(test)]
    pub(crate) fn try_clone_with_checkpoint(
        &self,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<Self, crate::dom::DomError> {
        self.shared_graph_clone(&mut OrdinarySemanticProducer, check)
    }

    fn layout_projection_with_checkpoint(
        &self,
        dom: &DomArena,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<Option<DomArena>, crate::dom::DomError> {
        check()?;
        if self.generated_content.is_empty()
            && self.computed_visibility.is_empty()
            && self.computed_text_style.is_empty()
        {
            return Ok(None);
        }
        let mut projection = dom
            .with_accessibility_generated_content_with_checkpoint(&self.generated_content, check)?;
        projection.set_accessibility_computed_visibility_with_checkpoint(
            &self.computed_visibility,
            check,
        )?;
        projection.set_accessibility_computed_text_style_with_checkpoint(
            &self.computed_text_style,
            check,
        )?;
        check()?;
        Ok(Some(projection))
    }
    pub fn reset(&mut self, dom: &DomArena, document_generation: u64) {
        let _ = self.reset_with_checkpoint(dom, document_generation, &mut || Ok(()));
    }

    pub fn apply_mutations(&mut self, dom: &DomArena, mutations: &[crate::dom::Mutation]) {
        let _ = self.apply_mutations_with_checkpoint(dom, mutations, &mut || Ok(()));
    }

    pub(crate) fn apply_computed_visibility_with_checkpoint(
        &mut self,
        dom: &DomArena,
        styles: &oryn_layout::ComputedStyles,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        let mut next = BTreeMap::new();
        let mut text_style = BTreeMap::new();
        for (id, style) in styles {
            check()?;
            let node = unpack_node_ref(*id);
            if dom.get(node).is_err() {
                continue;
            }
            next.insert(
                node,
                (
                    style.get("display").is_some_and(|value| value == "none")
                        || style
                            .get("-oryn-anchor-hidden")
                            .is_some_and(|value| value == "true"),
                    style
                        .get("visibility")
                        .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse")),
                ),
            );
            text_style.insert(
                node,
                (
                    style
                        .get("display")
                        .cloned()
                        .unwrap_or_else(|| "inline".into()),
                    style
                        .get("text-transform")
                        .cloned()
                        .unwrap_or_else(|| "none".into()),
                ),
            );
        }
        let mut changed = BTreeSet::new();
        for id in self.computed_text_style.keys().chain(text_style.keys()) {
            check()?;
            if self.computed_text_style.get(id) != text_style.get(id) {
                changed.insert(*id);
            }
        }
        for id in self.computed_visibility.keys().chain(next.keys()) {
            check()?;
            if self.computed_visibility.get(id) != next.get(id) {
                changed.insert(*id);
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        self.computed_visibility = next;
        self.computed_text_style = text_style;
        let mut affected = BTreeSet::new();
        for node in changed {
            check()?;
            affected.insert(node);
            for child in cancellation_or_default(descendants_with_checkpoint(dom, node, check))? {
                check()?;
                affected.insert(child);
            }
        }
        let mut mutations = Vec::new();
        for node in affected {
            check()?;
            mutations.push(crate::dom::Mutation::FormState { node });
        }
        self.apply_mutations_with_checkpoint(dom, &mutations, check)
    }
    pub fn apply_computed_visibility(
        &mut self,
        dom: &DomArena,
        styles: &oryn_layout::ComputedStyles,
    ) {
        let _ = self.apply_computed_visibility_with_checkpoint(dom, styles, &mut || Ok(()));
    }

    /// Native layout owns pseudo strings. A changed pseudo invalidates ordinary
    /// name-from-content ancestors and native ARIA/label dependents through the
    /// same incremental graph as DOM text. Paint-only synthetic IDs never enter.
    pub(crate) fn apply_generated_content_with_checkpoint(
        &mut self,
        dom: &DomArena,
        content: &[oryn_common::v4::GeneratedContent],
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        let mut next: BTreeMap<NodeId, (String, String)> = BTreeMap::new();
        let mut markers: BTreeMap<NodeId, String> = BTreeMap::new();
        for item in content {
            check()?;
            let owner = unpack_node_ref(item.owner);
            if dom.get(owner).is_err() {
                continue;
            }
            let value = item.alternative.as_ref().unwrap_or(&item.text);
            let speech = if item.separates_text && !value.is_empty() {
                format!(" {value} ")
            } else {
                value.clone()
            };
            let value = &speech;
            match item.pseudo.as_str() {
                "marker" => {
                    markers.entry(owner).or_default().push_str(value);
                }
                "before" => next.entry(owner).or_default().0.push_str(value),
                "after" => next.entry(owner).or_default().1.push_str(value),
                _ => {}
            }
        }
        // Markers precede ::before independently of publication record order.
        // Empty alternate text suppresses the marker; a nonempty marker forms
        // a separate text run even if authored alternate text omits a space.
        for (owner, mut marker) in markers {
            check()?;
            if marker.is_empty() {
                continue;
            }
            if !marker
                .chars()
                .last()
                .is_some_and(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
            {
                marker.push(' ');
            }
            let before = &mut next.entry(owner).or_default().0;
            marker.push_str(before);
            *before = marker;
        }
        let mut changed = BTreeSet::new();
        for id in self.generated_content.keys().chain(next.keys()) {
            check()?;
            if self.generated_content.get(id) != next.get(id) {
                changed.insert(*id);
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        self.generated_content = next;
        let mut affected = changed;
        // A generated descendant can name a referenced ancestor (label/span).
        // Include those ancestors before reference dependency expansion.
        let mut changed_nodes = Vec::new();
        for node in &affected {
            check()?;
            changed_nodes.push(*node);
        }
        for node in changed_nodes {
            check()?;
            for ancestor in cancellation_or_default(ancestors_with_checkpoint(dom, node, check))? {
                check()?;
                affected.insert(ancestor);
            }
        }
        let mut mutations = Vec::new();
        for node in affected {
            check()?;
            mutations.push(crate::dom::Mutation::FormState { node });
        }
        self.apply_mutations_with_checkpoint(dom, &mutations, check)
    }
    pub fn apply_generated_content(
        &mut self,
        dom: &DomArena,
        content: &[oryn_common::v4::GeneratedContent],
    ) {
        let _ = self.apply_generated_content_with_checkpoint(dom, content, &mut || Ok(()));
    }

    pub fn take_changes(&mut self) -> SemanticGraphChanges {
        let mut upserted = std::mem::take(&mut self.pending_upserts)
            .into_values()
            .collect::<Vec<_>>();
        upserted.sort_by_key(|node| node.document_order);
        SemanticGraphChanges {
            upserted,
            removed: std::mem::take(&mut self.pending_removed)
                .into_iter()
                .collect(),
        }
    }

    pub fn take_v6_changes(&mut self) -> V6Changes {
        let base = self.take_changes();
        let mut upserted_accessibility = std::mem::take(&mut self.pending_accessibility_upserts)
            .into_values()
            .collect::<Vec<_>>();
        upserted_accessibility.sort_by_key(|node| {
            self.full
                .get(&unpack_node_ref(node.semantic_ref.node))
                .map_or(u32::MAX, |semantic| semantic.document_order)
        });
        let mut upserted_structures = std::mem::take(&mut self.pending_structure_upserts)
            .into_values()
            .collect::<Vec<_>>();
        upserted_structures.sort_by(|left, right| left.structure_id.cmp(&right.structure_id));
        V6Changes {
            upserted_nodes: base
                .upserted
                .into_iter()
                .filter_map(|node| self.v6_node(node.semantic_ref, node))
                .collect(),
            removed_nodes: base.removed,
            upserted_accessibility,
            removed_accessibility: std::mem::take(&mut self.pending_accessibility_removed)
                .into_iter()
                .collect(),
            upserted_structures,
            removed_structures: std::mem::take(&mut self.pending_structure_removed)
                .into_iter()
                .collect(),
        }
    }

    pub fn nodes(&self, profile: ProjectionProfile) -> Vec<SemanticNodeView> {
        let mut nodes = self
            .full
            .iter()
            .filter(|(id, _)| profile == ProjectionProfile::Full || self.compact.contains(id))
            .map(|(_, node)| node.clone())
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.document_order);
        nodes
    }

    pub fn semantic_revision(&self) -> u64 {
        self.semantic_revision
    }

    /// Overlay live native state without encoding browser authority as authored
    /// attributes. Recompute group-dependent validity after every checkpoint:
    /// changing one radio, fieldset or form owner can affect other controls.
    #[cfg(feature = "v8-host")]
    pub(crate) fn apply_native_control_state(
        &mut self,
        dom: &crate::dom::DomRuntime,
    ) -> Result<(), crate::dom::DomError> {
        self.apply_native_control_state_with_checkpoint(dom, &mut || Ok(()))
    }

    /// The caller must discard its private graph draft on error.
    #[cfg(feature = "v8-host")]
    pub(crate) fn apply_native_control_state_with_checkpoint(
        &mut self,
        dom: &crate::dom::DomRuntime,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        let cursor = dom.mutation_cursor();
        if self.native_control_cursor == Some(cursor) {
            return Ok(());
        }
        let mut changed = false;
        for (&id, node) in &mut self.full {
            check()?;
            let record = dom.arena().get(id)?;
            let custom_control = dom.is_form_associated_custom_element(id);
            let control = custom_control
                || matches!(&record.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && matches!(name.local.as_ref(), "input" | "textarea" | "select" | "button" | "fieldset" | "output" | "object"));
            let validation = if control {
                let numeric = dom.control_numeric_validity_with_checkpoint(id, check)?;
                let flag = |name: &str, builtin: bool| {
                    if custom_control {
                        dom.custom_element_validity_flag(id, name)
                    } else {
                        builtin
                    }
                };
                Some(oryn_common::v3::NativeValidationState {
                    bad_input: flag(
                        "badInput",
                        dom.control_bad_input_with_checkpoint(id, check)?,
                    ),
                    too_short: flag(
                        "tooShort",
                        dom.control_length_validity_with_checkpoint(id, check)?.0,
                    ),
                    too_long: flag(
                        "tooLong",
                        dom.control_length_validity_with_checkpoint(id, check)?.1,
                    ),
                    pattern_mismatch: flag(
                        "patternMismatch",
                        dom.control_pattern_mismatch_with_checkpoint(id, check)?,
                    ),
                    range_underflow: flag("rangeUnderflow", numeric.range_underflow),
                    range_overflow: flag("rangeOverflow", numeric.range_overflow),
                    step_mismatch: flag("stepMismatch", numeric.step_mismatch),
                    will_validate: dom.control_will_validate_with_checkpoint(id, check)?,
                    valid: dom.control_validity_with_checkpoint(id, check)?,
                    value_missing: flag(
                        "valueMissing",
                        dom.control_value_missing_with_checkpoint(id, check)?,
                    ),
                    type_mismatch: flag(
                        "typeMismatch",
                        dom.control_type_mismatch_with_checkpoint(id, check)?,
                    ),
                    custom_error: flag("customError", !dom.custom_validity_message(id)?.is_empty()),
                })
            } else {
                None
            };
            let focused = dom.focused() == Some(id);
            let disabled = if control || record.submit_behavior.is_some() {
                dom.control_disabled_with_checkpoint(id, check)?
                    || aria_true(
                        record
                            .accessibility_attributes_with_checkpoint(check)?
                            .as_ref(),
                        "aria-disabled",
                    )
            } else {
                node.states.contains(&NodeState::Disabled)
            };
            if node.validation == validation
                && node.states.contains(&NodeState::Focused) == focused
                && node.states.contains(&NodeState::Disabled) == disabled
            {
                continue;
            }
            node.validation = validation;
            if disabled {
                node.states.insert(NodeState::Disabled);
            } else {
                node.states.remove(&NodeState::Disabled);
            }
            if focused {
                node.states.insert(NodeState::Focused);
            } else {
                node.states.remove(&NodeState::Focused);
            }
            self.pending_upserts.insert(
                node.semantic_ref,
                clone_semantic_node_with_checkpoint(node, check)?,
            );
            changed = true;
        }
        for (&id, accessible) in &mut self.accessibility {
            check()?;
            let record = dom.arena().get(id)?;
            let kind = dom
                .attribute_in_null_namespace(id, "type")?
                .unwrap_or("text")
                .to_ascii_lowercase();
            let numeric = dom.is_element(id, "http://www.w3.org/1999/xhtml", "input")
                && matches!(kind.as_str(), "number" | "range");
            let bound = |name: &str| {
                dom.attribute_in_null_namespace(id, name)
                    .ok()
                    .flatten()
                    .and_then(oryn_dom::parse_html_floating_point)
            };
            let minimum = if numeric {
                bound("min").or((kind == "range").then_some(0.))
            } else {
                None
            };
            let maximum = if numeric {
                bound("max")
                    .or((kind == "range").then_some(100.))
                    .map(|max| {
                        if kind == "range" {
                            max.max(minimum.unwrap_or(0.))
                        } else {
                            max
                        }
                    })
            } else {
                None
            };
            let current = if numeric {
                oryn_dom::parse_html_floating_point(&dom.control_value_with_checkpoint(id, check)?)
            } else {
                None
            };
            for (key, value) in [
                ("native_value_min", minimum),
                ("native_value_max", maximum),
                ("native_value_now", current),
            ] {
                let value = value
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number);
                if accessible.properties.get(key) != value.as_ref() {
                    if let Some(value) = value {
                        accessible.properties.insert(key.into(), value);
                    } else {
                        accessible.properties.remove(key);
                    }
                    drop(AccessibilityOwner(
                        self.pending_accessibility_upserts.insert(
                            accessible.semantic_ref,
                            clone_accessibility_with_checkpoint(accessible, check)?,
                        ),
                    ));
                    changed = true;
                }
            }

            let invoker = record
                .accessibility_attribute_with_checkpoint("aria-expanded", check)?
                .is_none()
                && ((dom.is_element(id, "http://www.w3.org/1999/xhtml", "button")
                    || dom.is_element(id, "http://www.w3.org/1999/xhtml", "input"))
                    && dom
                        .attribute_in_null_namespace(id, "popovertarget")?
                        .is_some()
                    || dom.is_element(id, "http://www.w3.org/1999/xhtml", "select")
                    || dom.has_native_input_picker(id));
            if invoker {
                let expanded = dom.top_layer().any(|entry| {
                    (entry.kind == oryn_dom::TopLayerKind::Popover && entry.source == Some(id))
                        || (matches!(
                            entry.kind,
                            oryn_dom::TopLayerKind::SelectPicker
                                | oryn_dom::TopLayerKind::TemporalPicker
                        ) && entry.node == id)
                });
                let value = serde_json::Value::Bool(expanded);
                if accessible.properties.get("expanded") != Some(&value) {
                    accessible.properties.insert("expanded".into(), value);
                    drop(AccessibilityOwner(
                        self.pending_accessibility_upserts.insert(
                            accessible.semantic_ref,
                            clone_accessibility_with_checkpoint(accessible, check)?,
                        ),
                    ));
                    changed = true;
                }
                if let Some(node) = self.full.get_mut(&id) {
                    let was = node.states.contains(&NodeState::Expanded);
                    if was != expanded {
                        if expanded {
                            node.states.insert(NodeState::Expanded);
                        } else {
                            node.states.remove(&NodeState::Expanded);
                        }
                        self.pending_upserts.insert(
                            node.semantic_ref,
                            clone_semantic_node_with_checkpoint(node, check)?,
                        );
                        changed = true;
                    }
                }
            }
            if dom.is_element(id, "http://www.w3.org/1999/xhtml", "select") {
                let active = dom.select_picker_active(id)?;
                let desired = if let Some(active) = active {
                    vec![semantic_ref_for(
                        active,
                        accessible.semantic_ref.page_id,
                        accessible.semantic_ref.document_generation,
                    )]
                } else {
                    dom.arena()
                        .accessibility_relation(id, "aria-activedescendant")
                        .unwrap_or_default()
                        .into_iter()
                        .map(|target| {
                            semantic_ref_for(
                                target,
                                accessible.semantic_ref.page_id,
                                accessible.semantic_ref.document_generation,
                            )
                        })
                        .collect()
                };
                let previous = accessible
                    .relationships
                    .get(&AccessibilityRelation::ActiveDescendant)
                    .cloned()
                    .unwrap_or_default();
                if previous != desired {
                    if desired.is_empty() {
                        accessible
                            .relationships
                            .remove(&AccessibilityRelation::ActiveDescendant);
                    } else {
                        accessible
                            .relationships
                            .insert(AccessibilityRelation::ActiveDescendant, desired);
                    }
                    drop(AccessibilityOwner(
                        self.pending_accessibility_upserts.insert(
                            accessible.semantic_ref,
                            clone_accessibility_with_checkpoint(accessible, check)?,
                        ),
                    ));
                    changed = true;
                }
            }
            if record.submit_behavior.is_some()
                || self
                    .full
                    .get(&id)
                    .is_some_and(|node| node.validation.is_some())
            {
                let disabled = dom.control_disabled_with_checkpoint(id, check)?
                    || aria_true(
                        record
                            .accessibility_attributes_with_checkpoint(check)?
                            .as_ref(),
                        "aria-disabled",
                    );
                let value = serde_json::Value::Bool(disabled);
                if accessible.properties.get("disabled") != Some(&value) {
                    accessible.properties.insert("disabled".into(), value);
                    drop(AccessibilityOwner(
                        self.pending_accessibility_upserts.insert(
                            accessible.semantic_ref,
                            clone_accessibility_with_checkpoint(accessible, check)?,
                        ),
                    ));
                    changed = true;
                }
            }
            let inert = dom.is_inert_with_checkpoint(id, check)?;
            let was_native = accessible.exclusion_reason.as_deref() == Some("native_inert");
            if inert && !accessible.excluded {
                accessible.excluded = true;
                accessible.exclusion_reason = Some("native_inert".into());
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts.insert(
                        accessible.semantic_ref,
                        clone_accessibility_with_checkpoint(accessible, check)?,
                    ),
                ));
                changed = true;
            } else if !inert && was_native {
                accessible.excluded = false;
                accessible.exclusion_reason = None;
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts.insert(
                        accessible.semantic_ref,
                        clone_accessibility_with_checkpoint(accessible, check)?,
                    ),
                ));
                changed = true;
            }
        }
        if changed {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
        check()?;
        self.native_control_cursor = Some(cursor);
        Ok(())
    }

    pub fn set_frame_ids(&mut self, frame_ids: BTreeMap<NodeId, u64>) {
        self.frame_ids = frame_ids;
    }

    pub fn apply_geometry_visibility(
        &mut self,
        geometry: &BTreeMap<u64, oryn_common::v4::NodeGeometry>,
    ) {
        let _ = self.apply_geometry_visibility_with_checkpoint(geometry, &mut || Ok(()));
    }

    pub fn apply_geometry_visibility_with_checkpoint(
        &mut self,
        geometry: &BTreeMap<u64, oryn_common::v4::NodeGeometry>,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        check()?;
        use oryn_common::v4::GeometryAvailability;
        let mut changed = false;
        for node in self.full.values_mut() {
            check()?;
            let id = unpack_node_ref(node.semantic_ref.node);
            // A complete layout snapshot can omit a boxless node (for
            // example display:none -> display:contents). Old geometry is not
            // evidence of current exclusion; native computed/ARIA state below
            // still owns exclusion when there is no current box.
            if geometry
                .get(&node.semantic_ref.node)
                .is_some_and(|value| value.availability == GeometryAvailability::Hidden)
            {
                self.geometry_hidden.insert(id);
            } else {
                self.geometry_hidden.remove(&id);
            }
            let hidden = self.authored_hidden.contains(&id) || self.geometry_hidden.contains(&id);
            let was_hidden = node.states.contains(&NodeState::Hidden);
            if hidden == was_hidden {
                continue;
            }
            if hidden {
                node.states.remove(&NodeState::Visible);
                node.states.insert(NodeState::Hidden);
            } else {
                node.states.remove(&NodeState::Hidden);
                node.states.insert(NodeState::Visible);
            }
            // Layout visibility can change after DOM projection. Keep the
            // compact membership in sync too, otherwise revealed controls
            // remain absent from OIL observations until an unrelated mutation.
            if is_compact_semantic(node) {
                self.compact.insert(id);
            } else {
                self.compact.remove(&id);
            }
            self.pending_upserts.insert(
                node.semantic_ref,
                clone_semantic_node_with_checkpoint(node, check)?,
            );
            if let Some(accessible) = self.accessibility.get_mut(&id) {
                let structural = accessible.exclusion_reason.clone().filter(|reason| {
                    !matches!(reason.as_str(), "computed_style_hidden" | "hidden_subtree")
                });
                accessible.excluded = hidden || structural.is_some();
                accessible.exclusion_reason = structural.or_else(|| {
                    hidden.then(|| {
                        if self.geometry_hidden.contains(&id) {
                            "computed_style_hidden".into()
                        } else {
                            "hidden_subtree".into()
                        }
                    })
                });
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts.insert(
                        accessible.semantic_ref,
                        clone_accessibility_with_checkpoint(accessible, check)?,
                    ),
                ));
            }
            changed = true;
        }
        if changed {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
        check()?;
        Ok(())
    }

    pub fn resolve(
        &self,
        target: &oryn_common::agent::Target,
        action: SemanticAction,
        revision: Revision,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
    ) -> Result<oryn_common::v6::ResolutionResult, SemanticResolutionError> {
        self.resolve_identity(target, Some(action), revision, geometry, &BTreeMap::new())
    }

    pub fn resolve_identity(
        &self,
        target: &oryn_common::agent::Target,
        action: Option<SemanticAction>,
        revision: Revision,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
        selectors: &BTreeMap<(bool, String), BTreeSet<SemanticRef>>,
    ) -> Result<oryn_common::v6::ResolutionResult, SemanticResolutionError> {
        use oryn_common::agent::Target;
        if let Target::Reference { reference } = target {
            let id = unpack_node_ref(reference.node);
            let node = self
                .full
                .get(&id)
                .ok_or(SemanticResolutionError::NotFound)?;
            if node.semantic_ref != *reference
                || action.is_some_and(|action| !node.actions.contains(&action))
            {
                return Err(SemanticResolutionError::NotFound);
            }
            return Ok(oryn_common::v6::ResolutionResult {
                revision,
                selected: Some(*reference),
                candidates: vec![oryn_common::v6::ResolutionCandidate {
                    semantic_ref: *reference,
                    score: 1_000,
                    actionable: action.is_some_and(|action| node.actions.contains(&action)),
                    factors: vec!["exact_reference".into()],
                }],
                diagnostics: Vec::new(),
            });
        }
        let mut candidates = self
            .full
            .values()
            .filter(|node| action.is_none_or(|action| node.actions.contains(&action)))
            .filter_map(|node| {
                self.score_target(node, target, geometry, selectors)
                    .transpose()
                    .map(|result| result.map(|(score, factors)| (node, score, factors)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if candidates.is_empty() {
            return Err(SemanticResolutionError::NotFound);
        }
        candidates.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.document_order.cmp(&right.0.document_order))
        });
        let best_score = candidates[0].1;
        let tied = candidates
            .iter()
            .take_while(|candidate| candidate.1 == best_score)
            .map(|candidate| candidate.0.semantic_ref)
            .collect::<Vec<_>>();
        let result_candidates = candidates
            .into_iter()
            .map(
                |(node, score, factors)| oryn_common::v6::ResolutionCandidate {
                    semantic_ref: node.semantic_ref,
                    // Keep subpixel precision for ranking and ties; the
                    // public integer score is only a diagnostic projection.
                    score: score.round() as i32,
                    actionable: action.is_some_and(|action| node.actions.contains(&action)),
                    factors,
                },
            )
            .collect::<Vec<_>>();
        if tied.len() > 1 {
            return Err(SemanticResolutionError::Ambiguous(tied));
        }
        Ok(oryn_common::v6::ResolutionResult {
            revision,
            selected: tied.first().copied(),
            candidates: result_candidates,
            diagnostics: Vec::new(),
        })
    }

    fn score_target(
        &self,
        node: &SemanticNodeView,
        target: &oryn_common::agent::Target,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
        selectors: &BTreeMap<(bool, String), BTreeSet<SemanticRef>>,
    ) -> Result<Option<(f64, Vec<String>)>, SemanticResolutionError> {
        use oryn_common::agent::Target;
        let scored = match target {
            Target::Reference { reference } => {
                (node.semantic_ref == *reference).then(|| (1_000.0, vec!["exact_reference".into()]))
            }
            Target::Alias { alias } => {
                (node.alias == *alias).then(|| (900.0, vec!["exact_alias".into()]))
            }
            Target::Text { text } => {
                let expected = normalize_text(text).to_ascii_lowercase();
                let actual = normalize_text(&node.name).to_ascii_lowercase();
                if expected.is_empty() {
                    None
                } else if actual == expected {
                    Some((800.0, vec!["exact_accessible_name".into()]))
                } else if !expected.is_empty() && actual.contains(&expected) {
                    Some((600.0, vec!["contained_accessible_name".into()]))
                } else if node.states.contains(&NodeState::Visible) {
                    self.target_hints
                        .get(&unpack_node_ref(node.semantic_ref.node))
                        .and_then(|hints| {
                            hints.iter().find(|(value, _)| {
                                normalize_text(value).to_ascii_lowercase() == expected
                            })
                        })
                        .map(|(_, factor)| (550.0, vec![(*factor).into()]))
                } else {
                    None
                }
            }
            Target::Role { role } => node
                .role
                .eq_ignore_ascii_case(role)
                .then(|| (700.0, vec!["exact_role".into()])),
            Target::Css { selector } => selectors.get(&(false, selector.clone())).map_or_else(
                || {
                    (node.selector.as_deref() == Some(selector.as_str()))
                        .then(|| (850.0, vec!["exact_css_selector".into()]))
                },
                |matches| {
                    matches
                        .contains(&node.semantic_ref)
                        .then(|| (850.0, vec!["native_css_selector".into()]))
                },
            ),
            Target::XPath { expression } => selectors
                .get(&(true, expression.clone()))
                .ok_or_else(|| SemanticResolutionError::UnsupportedSelector(expression.clone()))?
                .contains(&node.semantic_ref)
                .then(|| (850.0, vec!["native_xpath_selector".into()])),
            Target::Inside { target, container } => {
                let Some((score, mut factors)) =
                    self.score_target(node, target, geometry, selectors)?
                else {
                    return Ok(None);
                };
                let containers = self.matching_refs(container, geometry, selectors)?;
                let inside = containers
                    .iter()
                    .any(|container| self.is_descendant(node.semantic_ref, *container));
                inside.then(|| {
                    factors.push("inside".into());
                    (score + 100.0, factors)
                })
            }
            Target::Contains { target, content } => {
                let Some((score, mut factors)) =
                    self.score_target(node, target, geometry, selectors)?
                else {
                    return Ok(None);
                };
                let content = self.matching_refs(content, geometry, selectors)?;
                content
                    .iter()
                    .any(|content| self.is_descendant(*content, node.semantic_ref))
                    .then(|| {
                        factors.push("contains".into());
                        (score + 100.0, factors)
                    })
            }
            Target::Before {
                target: subject,
                anchor,
            }
            | Target::After {
                target: subject,
                anchor,
            } => {
                let Some((score, mut factors)) =
                    self.score_target(node, subject, geometry, selectors)?
                else {
                    return Ok(None);
                };
                let anchors = self.matching_refs(anchor, geometry, selectors)?;
                let is_before = matches!(target, Target::Before { .. });
                anchors
                    .iter()
                    .filter_map(|anchor| self.full.get(&unpack_node_ref(anchor.node)))
                    .any(|anchor| {
                        if is_before {
                            node.document_order < anchor.document_order
                        } else {
                            node.document_order > anchor.document_order
                        }
                    })
                    .then(|| {
                        factors.push(if is_before { "before" } else { "after" }.into());
                        (score + 80.0, factors)
                    })
            }
            Target::Near { target, anchor } => {
                let Some((score, mut factors)) =
                    self.score_target(node, target, geometry, selectors)?
                else {
                    return Ok(None);
                };
                let geometry = geometry.ok_or(SemanticResolutionError::GeometryUnavailable)?;
                let candidate = geometry
                    .get(&node.semantic_ref.node)
                    .and_then(|value| value.box_model.as_ref())
                    .ok_or(SemanticResolutionError::GeometryUnavailable)?
                    .border_box;
                let anchors = self.matching_refs(anchor, Some(geometry), selectors)?;
                let distance = anchors
                    .iter()
                    .filter_map(|anchor| {
                        geometry
                            .get(&anchor.node)
                            .and_then(|value| value.box_model.as_ref())
                    })
                    .map(|model| {
                        // Compare the visual centers, so differing label and
                        // control widths do not bias a nearby row's ranking.
                        let dx = (candidate.x + candidate.width / 2.)
                            - (model.border_box.x + model.border_box.width / 2.);
                        let dy = (candidate.y + candidate.height / 2.)
                            - (model.border_box.y + model.border_box.height / 2.);
                        dx.hypot(dy)
                    })
                    .filter(|distance| distance.is_finite())
                    .min_by(f64::total_cmp)
                    .ok_or(SemanticResolutionError::GeometryUnavailable)?;
                factors.push(format!("distance:{distance:.3}"));
                Some((score + 100_000.0 - distance.min(99_999.0), factors))
            }
        };
        Ok(scored)
    }

    fn matching_refs(
        &self,
        target: &oryn_common::agent::Target,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
        selectors: &BTreeMap<(bool, String), BTreeSet<SemanticRef>>,
    ) -> Result<Vec<SemanticRef>, SemanticResolutionError> {
        self.full
            .values()
            .filter_map(|node| {
                self.score_target(node, target, geometry, selectors)
                    .transpose()
                    .map(|result| result.map(|_| node.semantic_ref))
            })
            .collect()
    }

    fn is_descendant(&self, child: SemanticRef, ancestor: SemanticRef) -> bool {
        let mut current = self
            .full
            .get(&unpack_node_ref(child.node))
            .and_then(|node| node.parent);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self
                .full
                .get(&unpack_node_ref(parent.node))
                .and_then(|node| node.parent);
        }
        false
    }

    pub fn accessibility(&self, profile: ProjectionProfile) -> Vec<AccessibilityNode> {
        let included = self.profile_ids(profile);
        let mut nodes = self
            .accessibility
            .iter()
            .filter(|(id, _)| included.contains(id))
            .map(|(_, node)| node.clone())
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| {
            self.full
                .get(&unpack_node_ref(node.semantic_ref.node))
                .map_or(u32::MAX, |semantic| semantic.document_order)
        });
        nodes
    }

    pub fn structures(&self, profile: ProjectionProfile) -> Vec<SemanticStructure> {
        let included = self.profile_ids(profile);
        self.structures
            .values()
            .filter(|structure| included.contains(&unpack_node_ref(structure.root.node)))
            .cloned()
            .collect()
    }

    pub fn observation_v5(&self, observation: Observation) -> SemanticObservation {
        SemanticObservation {
            contract_version: oryn_common::v5::CONTRACT_VERSION,
            observation,
            // Frozen v5 always exposed the complete accessibility and
            // structure projection even when its embedded v3 view was compact.
            accessibility: self.accessibility(ProjectionProfile::Full),
            structures: self.structures(ProjectionProfile::Full),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub fn observation_v6(
        &self,
        base: Observation,
        options: &SemanticObservationOptions,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
    ) -> Result<SemanticObservationV6, String> {
        use oryn_common::v6::GeometryPolicy;
        let mut included = self.profile_ids(options.profile);
        if let Some(scope) = &options.scope {
            included = self.scope_ids(scope, geometry)?;
        }
        let graph_nodes = self.full.len() as u32;
        let mut nodes = self
            .full
            .iter()
            .filter(|(id, node)| {
                included.contains(id)
                    && (options.include_hidden || !node.states.contains(&NodeState::Hidden))
            })
            .filter_map(|(_, node)| self.v6_node(node.semantic_ref, node.clone()))
            .collect::<Vec<_>>();
        for node in &mut nodes {
            if options.geometry != GeometryPolicy::Omit {
                node.geometry = geometry
                    .and_then(|values| values.get(&node.semantic.semantic_ref.node))
                    .cloned();
                if options.geometry == GeometryPolicy::Require && node.geometry.is_none() {
                    return Err(format!(
                        "geometry unavailable for semantic node {}",
                        node.semantic.semantic_ref.node
                    ));
                }
            }
        }
        nodes.sort_by_key(|node| node.semantic.document_order);
        let included_refs = nodes
            .iter()
            .map(|node| node.semantic.semantic_ref)
            .collect::<BTreeSet<_>>();
        let accessibility = self
            .accessibility(options.profile)
            .into_iter()
            .filter(|node| included_refs.contains(&node.semantic_ref))
            .collect();
        let structures = if options.include_structures {
            self.structures(options.profile)
                .into_iter()
                .filter(|structure| included_refs.contains(&structure.root))
                .collect()
        } else {
            Vec::new()
        };
        let included_nodes = nodes.len() as u32;
        let mut omitted_by_reason = BTreeMap::new();
        if included_nodes < graph_nodes {
            omitted_by_reason.insert(
                "projection_profile_or_scope".into(),
                graph_nodes - included_nodes,
            );
        }
        Ok(SemanticObservationV6 {
            contract_version: oryn_common::v6::CONTRACT_VERSION,
            page: base.page,
            revision: base.revision,
            semantic_revision: self.semantic_revision,
            profile: options.profile,
            source: base.source,
            nodes,
            accessibility,
            structures,
            projection: ProjectionSummary {
                graph_nodes,
                included_nodes,
                omitted_by_reason,
                truncated: false,
            },
            capabilities: base.capabilities,
            diagnostics: base
                .diagnostics
                .into_iter()
                .chain(self.diagnostics.iter().cloned())
                .collect(),
            execution_domain: base.execution_domain,
        })
    }

    fn profile_ids(&self, profile: ProjectionProfile) -> BTreeSet<NodeId> {
        if profile == ProjectionProfile::Full {
            self.full.keys().copied().collect()
        } else {
            let mut included = self.compact.clone();
            included.extend(
                self.structures
                    .values()
                    .map(|structure| unpack_node_ref(structure.root.node)),
            );
            included
        }
    }

    fn scope_ids(
        &self,
        scope: &oryn_common::v6::ObservationScope,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
    ) -> Result<BTreeSet<NodeId>, String> {
        if scope.root.page_id != self.page_id
            || scope.root.document_generation != self.document_generation
        {
            return Err("scope belongs to another page or document generation".into());
        }
        let root = unpack_node_ref(scope.root.node);
        if !self.full.contains_key(&root) {
            return Err("scope root is stale or unavailable".into());
        }
        let mut selected = BTreeSet::from([root]);
        let mut current = Some(root);
        for _ in 0..scope.ancestor_depth {
            current = current.and_then(|id| {
                self.full
                    .get(&id)
                    .and_then(|node| node.parent)
                    .map(|parent| unpack_node_ref(parent.node))
            });
            if let Some(id) = current {
                selected.insert(id);
            } else {
                break;
            }
        }
        let mut frontier = BTreeSet::from([root]);
        for _ in 0..scope.descendant_depth {
            let next = self
                .full
                .iter()
                .filter(|(_, node)| {
                    node.parent
                        .is_some_and(|parent| frontier.contains(&unpack_node_ref(parent.node)))
                })
                .map(|(id, _)| *id)
                .collect::<BTreeSet<_>>();
            selected.extend(next.iter().copied());
            frontier = next;
        }
        frontier = selected.clone();
        for _ in 0..scope.relationship_depth {
            let mut next = BTreeSet::new();
            for id in &frontier {
                if let Some(accessible) = self.accessibility.get(id) {
                    for target in accessible.relationships.values().flatten() {
                        next.insert(unpack_node_ref(target.node));
                    }
                }
                if let Some(dependents) = self.referenced_by.get(id) {
                    next.extend(dependents);
                }
            }
            selected.extend(next.iter().copied());
            frontier = next;
        }
        if scope.include_structural_neighbors {
            for structure in self.structures.values() {
                let members = std::iter::once(structure.root)
                    .chain(structure.members.iter().copied())
                    .map(|reference| unpack_node_ref(reference.node))
                    .collect::<BTreeSet<_>>();
                if !selected.is_disjoint(&members) {
                    selected.extend(members);
                }
            }
        }
        if scope.include_spatial_neighbors {
            let geometry = geometry.ok_or_else(|| {
                "spatial semantic scope requires authoritative geometry".to_string()
            })?;
            let anchors = selected
                .iter()
                .filter_map(|id| self.full.get(id))
                .filter_map(|node| {
                    geometry
                        .get(&node.semantic_ref.node)
                        .and_then(|value| value.box_model.as_ref())
                        .map(|model| model.border_box)
                })
                .collect::<Vec<_>>();
            if anchors.is_empty() {
                return Err("spatial semantic scope root has no authoritative geometry".into());
            }
            let mut nearby = self
                .full
                .iter()
                .filter(|(id, _)| !selected.contains(id))
                .filter_map(|(id, node)| {
                    let rect = geometry
                        .get(&node.semantic_ref.node)?
                        .box_model
                        .as_ref()?
                        .border_box;
                    let distance = anchors
                        .iter()
                        .map(|anchor| {
                            let dx = rect.x - anchor.x;
                            let dy = rect.y - anchor.y;
                            dx * dx + dy * dy
                        })
                        .min_by(f64::total_cmp)?;
                    Some((*id, distance, node.document_order))
                })
                .collect::<Vec<_>>();
            nearby.sort_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.2.cmp(&right.2))
            });
            let remaining = (scope.max_nodes as usize).saturating_sub(selected.len());
            selected.extend(nearby.into_iter().take(remaining).map(|(id, _, _)| id));
        }
        if selected.len() > scope.max_nodes as usize {
            return Err(format!("semantic scope exceeds {} nodes", scope.max_nodes));
        }
        Ok(selected)
    }

    fn v6_node(
        &self,
        reference: SemanticRef,
        semantic: SemanticNodeView,
    ) -> Option<SemanticNodeV6> {
        self.projected_v6_with_checkpoint(reference, &semantic, &mut || Ok(()))
            .ok()
    }

    #[cfg(test)]
    fn refresh_node_with_checkpoint(
        &mut self,
        dom: &DomArena,
        id: NodeId,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), crate::dom::DomError> {
        self.shared_refresh_node(dom, id, &mut OrdinarySemanticProducer, check)
    }
}

fn ownership_cycle_with_checkpoint(
    owners: &BTreeMap<SemanticRef, SemanticRef>,
    owner: SemanticRef,
    target: SemanticRef,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    let mut cursor = Some(owner);
    let mut visited = BTreeSet::new();
    while let Some(node) = cursor {
        check()?;
        if node == target || !visited.insert(node) {
            return Ok(true);
        }
        cursor = owners.get(&node).copied();
    }
    Ok(false)
}

fn accepted_aria_owners_with_checkpoint(
    dom: &DomArena,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<BTreeMap<NodeId, NodeId>, crate::dom::DomError> {
    check()?;
    // DOM order, not arena allocation order, decides competing explicit owners.
    // Use one acceptance algorithm for naming and published accessibility edges.
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    {
        let mut cursor = 0usize;
        while let Some((root, node)) = dom.next_with_checkpoint(&mut cursor, check)? {
            check()?;
            if !matches!(node.kind, NodeKind::Document) {
                continue;
            }
            let mut pending = vec![root];
            while let Some(id) = pending.pop() {
                check()?;
                if !seen.insert(id) {
                    continue;
                }
                order.push(id);
                if let Ok(node) = dom.get(id) {
                    for child in node.children.iter().rev() {
                        check()?;
                        pending.push(*child);
                    }
                }
            }
        }
    }
    let mut accepted = BTreeMap::new();
    for owner in order {
        check()?;
        let Some(targets) =
            dom.accessibility_relation_with_checkpoint(owner, "aria-owns", check)?
        else {
            continue;
        };
        // Most nodes declare no ownership. Avoid computing CSS visibility for
        // every node just to discover that it has no proposed ownership edge.
        if targets.is_empty() || is_hidden_with_checkpoint(dom, owner, check)? {
            continue;
        }
        for target in targets {
            check()?;
            if accepted.contains_key(&target)
                || !seen.contains(&target)
                || !aria_owned_target_is_exposable_with_checkpoint(dom, target, check)?
                || cancellation_or_default(ancestors_with_checkpoint(dom, owner, check))?
                    .contains(&target)
            {
                continue;
            }
            let mut cursor = Some(owner);
            let mut walked = BTreeSet::new();
            let mut cycle = false;
            while let Some(id) = cursor {
                check()?;
                if id == target || !walked.insert(id) {
                    cycle = true;
                    break;
                }
                cursor = accepted
                    .get(&id)
                    .copied()
                    .or_else(|| dom.get(id).ok().and_then(|node| node.parent));
            }
            if !cycle {
                accepted.insert(target, owner);
            }
        }
    }
    check()?;
    Ok(accepted)
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn project_structures_with_checkpoint(
    dom: &DomArena,
    accessibility: &[AccessibilityNode],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<SemanticStructure>, crate::dom::DomError> {
    shared_project_structures(
        dom,
        accessibility,
        refs_by_dom_id,
        &mut OrdinarySemanticProducer,
        check,
    )
}

fn descendants_with_checkpoint(
    dom: &DomArena,
    root: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, crate::dom::DomError> {
    check()?;
    let mut pending = Vec::new();
    if let Ok(node) = dom.get(root) {
        for child in node.children.iter().rev() {
            check()?;
            pending.push(*child);
        }
    }
    let mut output = Vec::new();
    let mut seen = BTreeSet::from([root]);
    while let Some(id) = pending.pop() {
        check()?;
        if !seen.insert(id) {
            return Err(crate::dom::DomError::Cycle);
        }
        output.push(id);
        if let Ok(node) = dom.get(id) {
            for child in node.children.iter().rev() {
                check()?;
                pending.push(*child);
            }
        }
    }
    check()?;
    Ok(output)
}

fn ancestors_with_checkpoint(
    dom: &DomArena,
    node: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, crate::dom::DomError> {
    check()?;
    let mut output = Vec::new();
    let mut seen = BTreeSet::from([node]);
    let mut parent = dom.get(node)?.parent;
    while let Some(id) = parent {
        check()?;
        if !seen.insert(id) {
            return Err(crate::dom::DomError::Cycle);
        }
        output.push(id);
        parent = dom.get(id)?.parent;
    }
    Ok(output)
}

fn cancellation_or_default<T: Default>(
    result: Result<T, crate::dom::DomError>,
) -> Result<T, crate::dom::DomError> {
    match result {
        Err(
            error @ (crate::dom::DomError::ProjectionCancelled
            | crate::dom::DomError::SemanticProjectionCapacity),
        ) => Err(error),
        Err(_) => Ok(T::default()),
        Ok(value) => Ok(value),
    }
}
pub fn unpack_node_ref(node: u64) -> NodeId {
    NodeId {
        slot: node as u32,
        generation: (node >> 32) as u32,
    }
}

pub fn project_element(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    document_generation: u64,
    alias: u32,
) -> Option<SemanticNodeView> {
    project_element_with_checkpoint(dom, id, page_id, document_generation, alias, &mut || Ok(()))
        .ok()
        .flatten()
}

/// Force is an explicit driver geometry override; reuse the native action
/// classification rather than accepting arbitrary connected nodes.
#[cfg(feature = "v8-host")]
pub(crate) fn forced_click_supported(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    generation: u64,
) -> bool {
    project_element_with_visibility(dom, id, page_id, generation, 0, true)
        .is_some_and(|node| node.actions.contains(&SemanticAction::Click))
}

fn project_element_with_visibility(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    document_generation: u64,
    alias: u32,
    bypass_visibility: bool,
) -> Option<SemanticNodeView> {
    project_element_with_visibility_and_checkpoint(
        dom,
        id,
        page_id,
        document_generation,
        alias,
        bypass_visibility,
        &mut || Ok(()),
    )
    .ok()
    .flatten()
}

fn aria_true(attributes: &oryn_dom::AttributeList, name: &str) -> bool {
    attributes
        .get_null(name)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

#[cfg(any(test, feature = "semantic-oracle"))]
pub fn project_document(
    dom: &DomArena,
    page_id: PageId,
    document_generation: u64,
) -> Vec<SemanticNodeView> {
    dom.iter()
        .filter_map(|(id, _)| {
            let alias = id.slot.checked_add(1)?;
            project_element(dom, id, page_id, document_generation, alias)
                .filter(is_compact_semantic)
        })
        .collect()
}

#[cfg(any(test, feature = "semantic-oracle"))]
pub fn project_document_full(
    dom: &DomArena,
    page_id: PageId,
    document_generation: u64,
) -> Vec<SemanticNodeView> {
    dom.iter()
        .filter_map(|(id, node)| {
            let NodeKind::Element { name } = &node.kind else {
                return None;
            };
            if matches!(
                name.local.as_ref(),
                "html" | "head" | "body" | "meta" | "title" | "script" | "style"
            ) {
                return None;
            }
            let alias = id.slot.checked_add(1)?;
            project_element(dom, id, page_id, document_generation, alias)
        })
        .collect()
}

#[derive(Debug)]
struct AccessibleSemantics {
    name: String,
    description: Option<String>,
    provenance: Provenance,
    evidence: Vec<AccessibilityEvidence>,
}

fn accessible_semantics(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
    page_id: PageId,
    document_generation: u64,
) -> AccessibleSemantics {
    accessible_semantics_with_checkpoint(
        dom,
        id,
        local_name,
        role,
        page_id,
        document_generation,
        &mut || Ok(()),
    )
    .unwrap_or_else(|_| AccessibleSemantics {
        name: String::new(),
        description: None,
        provenance: Provenance::Heuristic { confidence: 0.0 },
        evidence: Vec::new(),
    })
}

#[cfg_attr(not(any(feature = "v8-host", test)), allow(dead_code))]
pub(crate) fn accessibility_for_node(dom: &DomArena, id: NodeId) -> Option<(String, String)> {
    let node = dom.get(id).ok()?;
    let NodeKind::Element { name } = &node.kind else {
        return None;
    };
    let role = computed_role_for_node(dom, id);
    let accessible = accessible_semantics(dom, id, name.local.as_ref(), &role, PageId(1), 1);
    Some((role, accessible.name))
}

#[cfg(test)]
fn host_label(dom: &DomArena, id: NodeId) -> Option<(String, Vec<NodeId>)> {
    host_label_with_checkpoint(dom, id, &mut || Ok(()))
        .ok()
        .flatten()
}

/// Agent-critical Accessible Name text alternative traversal. It preserves
/// referenced hidden roots but excludes aria-hidden descendants from ordinary
/// name-from-content, and includes host-language alternatives such as image
/// `alt` text that raw DOM textContent omits.
#[cfg(test)]
fn accessible_text_content(dom: &DomArena, id: NodeId, include_hidden_root: bool) -> String {
    accessible_text_content_with_checkpoint(dom, id, include_hidden_root, &mut || Ok(()))
        .unwrap_or_default()
}

fn accessible_children(dom: &DomArena, id: NodeId) -> Vec<NodeId> {
    accessible_children_with_checkpoint(dom, id, &mut || Ok(())).unwrap_or_default()
}

/// Walk the accessible option ownership tree, preserving all selected values.
/// Nested widgets and selected options terminate descent; cycle protection also
/// bounds malformed ownership graphs without duplicating an owned option.
#[cfg(test)]
fn embedded_selected_options(dom: &DomArena, root: NodeId) -> Vec<NodeId> {
    embedded_selected_options_with_checkpoint(dom, root, &mut || Ok(())).unwrap_or_default()
}

#[derive(Default)]
struct NameTraversal {
    seen: BTreeSet<NodeId>,
    reference_depth: usize,
    // Naming holds an immutable DOM borrow and invokes no author code. Reuse
    // ownership for this walk only; a later mutation starts a fresh traversal.
    owners: Option<BTreeMap<NodeId, NodeId>>,
}

// Follow accepted accessibility-parent edges as well as DOM ancestry. A
// cross-reference cycle can involve siblings and is invisible to DOM ancestry.
#[cfg(any(test, feature = "semantic-oracle"))]
fn ownership_cycle(
    owners: &BTreeMap<SemanticRef, SemanticRef>,
    owner: SemanticRef,
    target: SemanticRef,
) -> bool {
    let mut cursor = Some(owner);
    let mut visited = BTreeSet::new();
    while let Some(node) = cursor {
        if node == target || !visited.insert(node) {
            return true;
        }
        cursor = owners.get(&node).copied();
    }
    false
}

fn accepted_aria_owners(dom: &DomArena) -> BTreeMap<NodeId, NodeId> {
    accepted_aria_owners_with_checkpoint(dom, &mut || Ok(())).unwrap_or_default()
}

fn owned_accessible_children(
    dom: &DomArena,
    id: NodeId,
    owners: &BTreeMap<NodeId, NodeId>,
) -> Vec<NodeId> {
    let mut children = accessible_children(dom, id)
        .into_iter()
        .filter(|child| !owners.contains_key(child))
        .collect::<Vec<_>>();
    if let Some(owned) = dom.accessibility_relation(id, "aria-owns") {
        let mut seen = children.iter().copied().collect::<BTreeSet<_>>();
        children.extend(
            owned
                .into_iter()
                .filter(|child| owners.get(child) == Some(&id) && seen.insert(*child)),
        );
    }
    children
}

#[cfg(test)]
fn aria_owner(dom: &DomArena, target: NodeId) -> Option<NodeId> {
    accepted_aria_owners(dom).get(&target).copied()
}

fn name_from_content(local_name: &str, role: &str) -> bool {
    matches!(
        local_name,
        "button"
            | "a"
            | "label"
            | "summary"
            | "option"
            | "li"
            | "strong"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    ) || matches!(
        role,
        "button"
            | "link"
            | "tab"
            | "menuitem"
            | "checkbox"
            | "radio"
            | "switch"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "treeitem"
            | "option"
            | "listitem"
            | "heading"
            | "cell"
            | "rowheader"
            | "columnheader"
    )
}

fn normalize_text(value: &str) -> String {
    normalize_text_with_checkpoint(value, &mut || Ok(())).unwrap_or_default()
}

#[cfg(test)]
fn find_element_by_id_scoped(dom: &DomArena, source: NodeId, value: &str) -> Option<NodeId> {
    find_element_by_id_scoped_with_checkpoint(dom, source, value, &mut || Ok(()))
        .ok()
        .flatten()
}

fn semantic_ref_for(id: NodeId, page_id: PageId, document_generation: u64) -> SemanticRef {
    SemanticRef {
        page_id,
        document_generation,
        node: (u64::from(id.generation) << 32) | u64::from(id.slot),
    }
}

/// Build contract-v5 semantic structures without changing frozen v2-v4
/// observations. All node references retain the page/document generation of
/// the base observation.
#[cfg(any(test, feature = "semantic-oracle"))]
pub fn project_semantic_observation(
    dom: &DomArena,
    observation: Observation,
) -> SemanticObservation {
    let page_id = observation.page.page_id;
    let document_generation = observation.page.document_generation;
    let projected = project_document_full(dom, page_id, document_generation);
    let mut diagnostics = Vec::new();
    let mut accessibility = Vec::new();
    let mut refs_by_dom_id = BTreeMap::new();
    for node in &projected {
        refs_by_dom_id.insert(unpack_node_ref(node.semantic_ref.node), node.semantic_ref);
    }

    for semantic in &projected {
        let id = unpack_node_ref(semantic.semantic_ref.node);
        let Ok(node) = dom.get(id) else {
            continue;
        };
        let NodeKind::Element { name } = &node.kind else {
            continue;
        };
        let role = computed_role_for_node(dom, id);
        let named = accessible_semantics(
            dom,
            id,
            name.local.as_ref(),
            &role,
            page_id,
            document_generation,
        );
        let exclusion_reason = tree_exclusion_reason(dom, id, &role);
        let relationships = accessibility_relationships(
            dom,
            id,
            node,
            page_id,
            document_generation,
            &mut diagnostics,
        );
        accessibility.push(AccessibilityNode {
            semantic_ref: semantic.semantic_ref,
            excluded: exclusion_reason.is_some(),
            exclusion_reason,
            role,
            name: named.name,
            description: named.description,
            relationships,
            properties: accessibility_properties(dom, id, name.local.as_ref(), node),
            owned_by: None,
            content_trust: ContentTrust::PageAuthored,
            provenance: named.provenance,
            evidence: named.evidence,
        });
    }

    let accepted_owners = accepted_aria_owners(dom);
    let mut owner_by_target = BTreeMap::<SemanticRef, SemanticRef>::new();
    let index_by_ref = accessibility
        .iter()
        .enumerate()
        .map(|(index, node)| (node.semantic_ref, index))
        .collect::<BTreeMap<_, _>>();
    for node in &mut accessibility {
        for targets in node.relationships.values_mut() {
            targets.retain(|target| {
                let available = index_by_ref.contains_key(target);
                if !available {
                    diagnostics.push(format!(
                        "ignored accessibility relationship to unavailable semantic node {}",
                        target.node
                    ));
                }
                available
            });
        }
        node.relationships.retain(|_, targets| !targets.is_empty());
    }
    for owner_node in &mut accessibility {
        let owner = owner_node.semantic_ref;
        let Some(owned) = owner_node
            .relationships
            .get(&AccessibilityRelation::Owns)
            .cloned()
        else {
            continue;
        };
        let owner_id = unpack_node_ref(owner.node);
        let mut valid = Vec::new();
        for target in owned {
            let target_id = unpack_node_ref(target.node);
            let creates_cycle = ownership_cycle(&owner_by_target, owner, target)
                || dom
                    .ancestors(owner_id)
                    .is_ok_and(|ancestors| ancestors.contains(&target_id));
            if creates_cycle {
                diagnostics.push(format!(
                    "ignored aria-owns cycle from semantic node {} to {}",
                    owner.node, target.node
                ));
                continue;
            }
            if accepted_owners.get(&target_id) != Some(&owner_id) {
                diagnostics.push(format!(
                    "ignored unavailable or competing aria-owns target {} from {}",
                    target.node, owner.node
                ));
                continue;
            }
            if let Some(existing) = owner_by_target.insert(target, owner) {
                owner_by_target.insert(target, existing);
                diagnostics.push(format!(
                    "ignored duplicate aria-owns target {} from semantic node {}",
                    target.node, owner.node
                ));
                continue;
            }
            valid.push(target);
        }
        owner_node
            .relationships
            .insert(AccessibilityRelation::Owns, valid);
    }
    for (target, owner) in owner_by_target {
        if let Some(index) = index_by_ref.get(&target).copied() {
            accessibility[index].owned_by = Some(owner);
        }
    }

    let structures = project_structures(dom, &accessibility, &refs_by_dom_id);
    SemanticObservation {
        contract_version: oryn_common::v5::CONTRACT_VERSION,
        observation,
        accessibility,
        structures,
        diagnostics,
    }
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn tree_exclusion_reason(dom: &DomArena, id: NodeId, role: &str) -> Option<String> {
    tree_exclusion_reason_with_checkpoint(dom, id, role, &mut || Ok(()))
        .ok()
        .flatten()
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn accessibility_relationships(
    dom: &DomArena,
    source: NodeId,
    node: &crate::dom::Node,
    page_id: PageId,
    document_generation: u64,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<AccessibilityRelation, Vec<SemanticRef>> {
    accessibility_relationships_with_checkpoint(
        dom,
        source,
        node,
        page_id,
        document_generation,
        diagnostics,
        &mut || Ok(()),
    )
    .unwrap_or_default()
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn accessibility_properties(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    node: &crate::dom::Node,
) -> BTreeMap<String, serde_json::Value> {
    accessibility_properties_with_checkpoint(dom, id, local_name, node, &mut || Ok(()))
        .unwrap_or_default()
}

#[cfg(any(test, feature = "semantic-oracle"))]
fn project_structures(
    dom: &DomArena,
    accessibility: &[AccessibilityNode],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
) -> Vec<SemanticStructure> {
    project_structures_with_checkpoint(dom, accessibility, refs_by_dom_id, &mut || Ok(()))
        .unwrap_or_default()
}

fn structure_kind(
    local_name: &str,
    role: &str,
    attributes: &oryn_dom::AttributeList,
) -> Option<SemanticStructureKind> {
    if attributes.contains_null("aria-live")
        || matches!(role, "alert" | "status" | "log" | "marquee" | "timer")
    {
        return Some(SemanticStructureKind::LiveRegion);
    }
    if local_name == "form" && role == "generic" {
        return Some(SemanticStructureKind::Form);
    }
    match role {
        "form" => Some(SemanticStructureKind::Form),
        "table" => Some(SemanticStructureKind::Table),
        "grid" | "treegrid" => Some(SemanticStructureKind::Grid),
        "list" => Some(SemanticStructureKind::List),
        "navigation" => Some(SemanticStructureKind::Navigation),
        "dialog" | "alertdialog" => Some(SemanticStructureKind::Dialog),
        "menu" | "menubar" => Some(SemanticStructureKind::Menu),
        "tree" => Some(SemanticStructureKind::Tree),
        "tablist" => Some(SemanticStructureKind::TabSet),
        "main" | "banner" | "contentinfo" | "complementary" | "search" | "region" => {
            Some(SemanticStructureKind::Landmark)
        }
        "feed" => Some(SemanticStructureKind::RepeatedCollection),
        _ if local_name == "dl" || attributes.contains_null("data-oryn-collection") => {
            Some(SemanticStructureKind::RepeatedCollection)
        }
        _ => None,
    }
}

fn implicit_live_politeness(role: &str) -> &'static str {
    match role {
        "alert" => "assertive",
        "status" | "log" => "polite",
        _ => "off",
    }
}

pub(crate) fn accessibility_readonly(dom: &DomArena, id: NodeId) -> bool {
    accessibility_readonly_with_checkpoint(dom, id, &mut || Ok(())).unwrap_or(true)
}
#[cfg(any(feature = "v8-host", test))]
pub(crate) fn native_selection_options(dom: &DomArena, root: NodeId) -> Vec<NodeId> {
    let owners = accepted_aria_owners(dom);
    let mut pending = owned_accessible_children(dom, root, &owners);
    pending.reverse();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let role = computed_role_for_node(dom, id);
        if matches!(
            role.as_str(),
            "option" | "radio" | "menuitemradio" | "treeitem"
        ) {
            result.push(id);
            continue;
        }
        if matches!(
            role.as_str(),
            "listbox" | "combobox" | "radiogroup" | "tree"
        ) {
            continue;
        }
        let children = owned_accessible_children(dom, id, &owners);
        pending.extend(children.into_iter().rev());
    }
    result
}

#[cfg(feature = "v8-host")]
pub(crate) fn native_checked_state(
    dom: &crate::dom::DomRuntime,
    id: NodeId,
) -> Result<Option<bool>, String> {
    let node = dom.arena().get(id).map_err(|error| error.to_string())?;
    let role = computed_role_for_node(dom.arena(), id);
    if !matches!(
        role.as_str(),
        "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio"
    ) {
        return Err("check action requires a checkable native accessibility role".into());
    }
    if dom
        .control_disabled(id)
        .map_err(|error| error.to_string())?
        || node
            .accessibility_attribute("aria-disabled")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || accessibility_readonly(dom.arena(), id)
    {
        return Err("checkable accessibility target is disabled or read-only".into());
    }
    if dom.is_element(id, "http://www.w3.org/1999/xhtml", "input")
        && node
            .attributes
            .get_null("type")
            .is_some_and(|kind| matches!(kind.to_ascii_lowercase().as_str(), "checkbox" | "radio"))
    {
        return if node.indeterminate && role != "radio" {
            Ok(None)
        } else {
            dom.checked(id).map(Some).map_err(|error| error.to_string())
        };
    }
    Ok(
        match node
            .accessibility_attribute("aria-checked")
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("true") => Some(true),
            Some("mixed") if matches!(role.as_str(), "checkbox" | "menuitemcheckbox") => None,
            _ => Some(false),
        },
    )
}

thread_local! {static ROLE_NAME_STACK:std::cell::RefCell<BTreeSet<(usize,NodeId)>>=const{std::cell::RefCell::new(BTreeSet::new())};}
pub(crate) fn computed_role_for_node(dom: &DomArena, id: NodeId) -> String {
    computed_role_for_node_with_checkpoint(dom, id, &mut || Ok(()))
        .unwrap_or_else(|_| "generic".into())
}

#[cfg(test)]
fn computed_role(local_name: &str, attributes: &oryn_dom::AttributeList) -> String {
    if let Some(role) = attributes.get_null("role").and_then(|roles| {
        roles
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .find(|role| {
                supported_aria_role(role)
                    && (!matches!(role.as_str(), "form" | "region")
                        || attributes.contains_null("aria-label")
                        || attributes.contains_null("aria-labelledby"))
            })
    }) {
        return explicit_role_value(local_name, attributes, role);
    }
    implicit_role(local_name, attributes)
}

#[cfg(test)]
fn explicit_role_value(
    local_name: &str,
    attributes: &oryn_dom::AttributeList,
    role: String,
) -> String {
    let presentation_conflict = matches!(role.as_str(), "none" | "presentation")
        && (matches!(local_name, "button" | "input" | "select" | "textarea")
            || (local_name == "a" && attributes.contains_null("href"))
            || attributes.contains_null("tabindex")
            || attributes.records().any(|entry| {
                entry.namespace.is_none() && is_global_aria_attribute(&entry.local_name)
            }));
    if presentation_conflict {
        return implicit_role(local_name, attributes);
    }
    match role.as_str() {
        "img" | "image" => "image".into(),
        "directory" => "list".into(),
        "presentation" => "none".into(),
        _ => role,
    }
}

#[cfg(test)]
fn implicit_role(local_name: &str, attributes: &oryn_dom::AttributeList) -> String {
    match local_name {
        "a" | "area" if attributes.contains_null("href") => "link",
        "input" => match attributes
            .get_null("type")
            .map(|kind| kind.to_ascii_lowercase())
            .as_deref()
        {
            Some("checkbox") => "checkbox",
            Some("radio") => "radio",
            Some("range") => "slider",
            Some("number") => "spinbutton",
            Some("search") => "searchbox",
            Some("button" | "submit" | "reset" | "file" | "image") => "button",
            Some("color") => "colorwell",
            _ => "textbox",
        },
        "textarea" => "textbox",
        "select"
            if attributes.contains_null("multiple")
                || attributes
                    .get_null("size")
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|size| size > 1) =>
        {
            "listbox"
        }
        "select" => "combobox",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "output" => "status",
        "img" if attributes.get_null("alt").is_some_and(String::is_empty) => "none",
        "img" => "image",
        "nav" => "navigation",
        "main" => "main",
        "aside" => "complementary",
        "header" => "banner",
        "footer" => "contentinfo",
        "form"
            if attributes.contains_null("aria-label")
                || attributes.contains_null("aria-labelledby") =>
        {
            "form"
        }
        "form" => "generic",
        "ul" | "ol" | "menu" | "dir" => "list",
        "li" => "listitem",
        "table" => "table",
        "tr" => "row",
        "th" => match attributes
            .get_null("scope")
            .map(|scope| scope.to_ascii_lowercase())
            .as_deref()
        {
            Some("row") | Some("rowgroup") => "rowheader",
            _ => "columnheader",
        },
        "td" => "cell",
        "dialog" => "dialog",
        "summary" => "button",
        "option" => "option",
        "progress" => "progressbar",
        "search" => "search",
        "details" => "group",
        "datalist" => "listbox",
        "blockquote" => "blockquote",
        "caption" => "caption",
        "code" => "code",
        "del" | "s" => "deletion",
        "em" => "emphasis",
        "ins" => "insertion",
        "p" => "paragraph",
        "strong" => "strong",
        "sub" => "subscript",
        "sup" => "superscript",
        "time" => "time",
        "address" | "fieldset" | "hgroup" | "optgroup" => "group",
        "cite" => "html-cite",
        "dd" => "definition",
        "dt" | "dfn" => "term",
        "hr" => "separator",
        "thead" | "tbody" | "tfoot" => "rowgroup",
        "button" | "article" | "figure" | "mark" | "meter" => local_name,
        "div" | "span" => "generic",
        "section"
            if attributes.contains_null("aria-label")
                || attributes.contains_null("aria-labelledby") =>
        {
            "region"
        }
        _ => "generic",
    }
    .into()
}

fn is_global_aria_attribute(name: &str) -> bool {
    matches!(
        name,
        "aria-atomic"
            | "aria-braillelabel"
            | "aria-brailleroledescription"
            | "aria-busy"
            | "aria-controls"
            | "aria-current"
            | "aria-describedby"
            | "aria-description"
            | "aria-details"
            | "aria-disabled"
            | "aria-dropeffect"
            | "aria-errormessage"
            | "aria-flowto"
            | "aria-grabbed"
            | "aria-haspopup"
            | "aria-hidden"
            | "aria-invalid"
            | "aria-keyshortcuts"
            | "aria-label"
            | "aria-labelledby"
            | "aria-live"
            | "aria-owns"
            | "aria-relevant"
            | "aria-roledescription"
    )
}

fn supported_aria_role(role: &str) -> bool {
    matches!(
        role,
        "alert"
            | "alertdialog"
            | "application"
            | "article"
            | "banner"
            | "button"
            | "blockquote"
            | "caption"
            | "cell"
            | "checkbox"
            | "columnheader"
            | "combobox"
            | "complementary"
            | "contentinfo"
            | "code"
            | "deletion"
            | "definition"
            | "dialog"
            | "directory"
            | "document"
            | "emphasis"
            | "feed"
            | "figure"
            | "form"
            | "generic"
            | "grid"
            | "gridcell"
            | "group"
            | "heading"
            | "img"
            | "image"
            | "insertion"
            | "link"
            | "list"
            | "listbox"
            | "listitem"
            | "log"
            | "main"
            | "marquee"
            | "math"
            | "menu"
            | "menubar"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "meter"
            | "navigation"
            | "none"
            | "note"
            | "option"
            | "paragraph"
            | "presentation"
            | "progressbar"
            | "radio"
            | "radiogroup"
            | "region"
            | "row"
            | "rowgroup"
            | "rowheader"
            | "scrollbar"
            | "search"
            | "searchbox"
            | "sectionfooter"
            | "sectionheader"
            | "separator"
            | "slider"
            | "spinbutton"
            | "status"
            | "strong"
            | "subscript"
            | "suggestion"
            | "switch"
            | "tab"
            | "table"
            | "tablist"
            | "tabpanel"
            | "term"
            | "textbox"
            | "timer"
            | "time"
            | "toolbar"
            | "tooltip"
            | "tree"
            | "treegrid"
            | "treeitem"
            | "superscript"
    )
}

fn is_compact_semantic(node: &SemanticNodeView) -> bool {
    !node.states.contains(&NodeState::Hidden)
        && (!node.actions.is_empty()
            || matches!(
                node.role.as_str(),
                "heading"
                    | "status"
                    | "button"
                    | "link"
                    | "textbox"
                    | "checkbox"
                    | "radio"
                    | "combobox"
            ))
}

pub(crate) fn is_hidden(dom: &DomArena, id: NodeId) -> bool {
    is_hidden_with_checkpoint(dom, id, &mut || Ok(())).unwrap_or(false)
}

pub fn empty_observation(
    page_id: PageId,
    opener_page_id: Option<PageId>,
    url: String,
    title: String,
    document_generation: u64,
) -> Observation {
    Observation {
        contract_version: CONTRACT_VERSION,
        page: PageInfo {
            page_id,
            opener_page_id,
            url,
            title,
            document_generation,
        },
        revision: Revision(0),
        profile: ProjectionProfile::Compact,
        source: ObservationSource::NativeDom,
        nodes: Vec::new(),
        capabilities: Vec::new(),
        diagnostics: Vec::new(),
        execution_domain: ExecutionDomain::Native,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_text_helpers_keep_explicit_byte_semantics_and_cancel_mid_scan() {
        assert_eq!(
            normalize_text_with_checkpoint("  α\tβ\n\r\u{c}γ\u{a0}δ  ", &mut || Ok(())).unwrap(),
            "α β γ\u{a0}δ"
        );
        assert_eq!(
            strip_css_comments_with_checkpoint("α/* ignored */β/**/γ", &mut || Ok(())).unwrap(),
            "αβγ"
        );
        assert_eq!(
            strip_css_comments_with_checkpoint("a/* unfinished", &mut || Ok(())).unwrap(),
            "a"
        );
        let mut dom = crate::dom::DomRuntime::new();
        let node = dom.create_element("button");
        dom.set_attribute(node, "id", "α").unwrap();
        dom.set_attribute(node, "class", "one two").unwrap();
        dom.set_attribute(node, "data-state", "ready").unwrap();
        let record = dom.arena().get(node).unwrap();
        assert!(
            matches_simple_selector_with_checkpoint(
                record,
                "button#α.one[data-state='ready']",
                &mut || Ok(())
            )
            .unwrap()
        );
        assert!(
            !matches_simple_selector_with_checkpoint(record, "button.missing", &mut || Ok(()))
                .unwrap()
        );
        let input = "α ".repeat(128);
        for comments in [false, true] {
            let mut visits = 0;
            let mut check = || {
                visits += 1;
                if visits == 35 {
                    Err(crate::dom::DomError::ProjectionCancelled)
                } else {
                    Ok(())
                }
            };
            let result = if comments {
                strip_css_comments_with_checkpoint(&input, &mut check)
            } else {
                normalize_text_with_checkpoint(&input, &mut check)
            };
            assert!(matches!(
                result,
                Err(crate::dom::DomError::ProjectionCancelled)
            ));
            assert_eq!(visits, 35);
        }
    }

    #[test]
    fn hosted_document_projection_rejects_cycles_and_preserves_live_nested_owner() {
        let mut dom = crate::dom::DomRuntime::new();
        let outer = dom.document();
        let frame = dom.create_element("iframe");
        dom.append_child(outer, frame).unwrap();
        let child = dom.create(NodeKind::Document);
        let child_frame = dom.create_element("iframe");
        dom.append_child(child, child_frame).unwrap();
        dom.set_hosted_document(frame, Some(child)).unwrap();
        assert!(matches!(
            dom.set_hosted_document(child_frame, Some(outer)),
            Err(crate::dom::DomError::Cycle)
        ));
        let button = dom.create_element("button");
        dom.append_child(child, button).unwrap();
        assert!(
            project_element_with_checkpoint(dom.arena(), button, PageId(31), 9, 1, &mut || Ok(()))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn graph_checkpoint_clone_cancellation_never_changes_published_graph() {
        let dom = crate::html::parse("<button id=one>one</button><button id=two>two</button>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        let before = format!("{graph:?}");
        let mut calls = 0;
        let result = graph.try_clone_with_checkpoint(&mut || {
            calls += 1;
            if calls == 5 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(crate::dom::DomError::ProjectionCancelled)
        ));
        assert_eq!(format!("{graph:?}"), before);
        let copy = graph.try_clone_with_checkpoint(&mut || Ok(())).unwrap();
        assert_eq!(format!("{copy:?}"), before);
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn mid_control_checkpoint_failure_discards_only_private_graph_draft() {
        let dom = crate::html::parse("<input id=one><button id=two>two</button>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        let before = format!("{graph:?}");
        let mut draft = graph.try_clone_with_checkpoint(&mut || Ok(())).unwrap();
        let mut calls = 0;
        let result = draft.apply_native_control_state_with_checkpoint(&dom, &mut || {
            calls += 1;
            if calls == 3 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(crate::dom::DomError::ProjectionCancelled)
        ));
        assert_eq!(format!("{graph:?}"), before);
        assert!(draft.native_control_cursor.is_none());
        let mut normal = graph.clone();
        normal.apply_native_control_state(&dom).unwrap();
        let mut checked = graph.try_clone_with_checkpoint(&mut || Ok(())).unwrap();
        checked
            .apply_native_control_state_with_checkpoint(&dom, &mut || Ok(()))
            .unwrap();
        assert_eq!(format!("{checked:?}"), format!("{normal:?}"));
    }

    #[test]
    fn subtree_removal_retires_accessibility_and_its_pending_upserts() {
        let mut dom = crate::html::parse(
            "<main id=parent><section id=removed><button id=child>Gone</button></section><button id=kept>Kept</button></main>",
        ).into_runtime().unwrap();
        let parent = dom.find_by_id("parent").unwrap();
        let removed = dom.find_by_id("removed").unwrap();
        let child = dom.find_by_id("child").unwrap();
        let kept = dom.find_by_id("kept").unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        let references = [
            graph.full[&removed].semantic_ref,
            graph.full[&child].semantic_ref,
        ];
        let cursor = dom.mutation_cursor();
        dom.remove(removed).unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        for (id, reference) in [removed, child].into_iter().zip(references) {
            assert!(!graph.full.contains_key(&id));
            assert!(!graph.accessibility.contains_key(&id));
            assert!(!graph.pending_accessibility_upserts.contains_key(&reference));
            assert!(graph.pending_accessibility_removed.contains(&reference));
        }
        assert!(graph.accessibility.contains_key(&parent));
        assert!(graph.accessibility.contains_key(&kept));
        #[cfg(feature = "v8-host")]
        graph.apply_native_control_state(&dom).unwrap();
    }

    // Use native publication so previously parsed slot snapshots are refreshed.
    // Direct arena kind/parent edits bypass that production mutation contract.
    fn attach_test_shadow(
        dom: &mut crate::dom::DomRuntime,
        host: NodeId,
        container: NodeId,
    ) -> NodeId {
        let root = dom.create(NodeKind::ShadowRoot {
            mode: crate::dom::ShadowMode::Open,
            clonable: false,
        });
        dom.append_child(host, root).unwrap();
        let children = dom.arena().get(container).unwrap().children.clone();
        for child in children {
            dom.append_child(root, child).unwrap();
        }
        root
    }
    #[test]
    fn unnamed_landmark_role_tokens_continue_to_supported_fallback() {
        let mut dom=crate::html::parse("<nav id=n role='ReGiOn group'></nav><div id=b role='form button' aria-labelledby=missing></div>").into_runtime().unwrap();
        let n = dom.find_by_id("n").unwrap();
        let b = dom.find_by_id("b").unwrap();
        assert_eq!(computed_role_for_node(dom.arena(), n), "group");
        assert_eq!(computed_role_for_node(dom.arena(), b), "button");
        dom.set_attribute(n, "aria-label", "Named").unwrap();
        assert_eq!(computed_role_for_node(dom.arena(), n), "region");
    }

    #[test]
    fn manual_heading_defaults_and_aria_placeholders_reach_shared_properties() {
        let dom=crate::html::parse("<div id=heading role=heading></div><h4 id=native>Heading</h4><div id=textbox role=textbox aria-placeholder='DD/MM/YYYY'></div>").into_runtime().unwrap();
        for (id, tag, level) in [("heading", "div", 2), ("native", "h4", 4)] {
            let node = dom.find_by_id(id).unwrap();
            assert_eq!(
                accessibility_properties(dom.arena(), node, tag, dom.arena().get(node).unwrap())["level"],
                serde_json::json!(level)
            );
        }
        let node = dom.find_by_id("textbox").unwrap();
        assert_eq!(
            accessibility_properties(dom.arena(), node, "div", dom.arena().get(node).unwrap())["placeholder"],
            serde_json::json!("DD/MM/YYYY")
        );
    }

    #[test]
    fn aria_modal_accessibility_scope_reconciles_siblings_without_hiding_native_actions() {
        let mut dom=crate::html::parse("<h1 id=outside>Outside</h1><div id=modal role=dialog aria-modal=true><h2 id=inside>Inside</h2></div>").into_runtime().unwrap();
        let outside = dom.find_by_id("outside").unwrap();
        let inside = dom.find_by_id("inside").unwrap();
        let modal = dom.find_by_id("modal").unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(
            graph.accessibility[&outside].exclusion_reason.as_deref(),
            Some("aria_modal_scope")
        );
        assert!(!graph.accessibility[&inside].excluded);
        assert!(!graph.full[&outside].states.contains(&NodeState::Hidden));
        let cursor = dom.mutation_cursor();
        dom.set_attribute(modal, "aria-modal", "false").unwrap();
        let changes = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &changes.mutations);
        assert!(!graph.accessibility[&outside].excluded);
    }

    #[test]
    fn own_label_omits_control_value_but_preserves_pseudo_boundaries_and_label_tooltip() {
        for (kind, title, expected) in [
            ("text", "title=bar", "foo bar baz"),
            ("password", "title=bar", "foo bar baz"),
            ("checkbox", "", "foo baz"),
            ("radio", "", "foo baz"),
            ("image", "", "foo baz"),
        ] {
            let dom=crate::html::parse(&format!("<label id=label for=target {title}><input id=target type={kind} title=ignored></label>")).into_runtime().unwrap();
            let label = dom.find_by_id("label").unwrap();
            let target = dom.find_by_id("target").unwrap();
            let owner = (u64::from(label.generation) << 32) | u64::from(label.slot);
            let mut graph = SemanticGraph::new(PageId(1), 1);
            graph.reset(dom.arena(), 1);
            graph.apply_generated_content(
                dom.arena(),
                &[
                    oryn_common::v4::GeneratedContent {
                        separates_text: false,
                        owner,
                        pseudo: "before".into(),
                        text: "foo".into(),
                        alternative: None,
                    },
                    oryn_common::v4::GeneratedContent {
                        separates_text: false,
                        owner,
                        pseudo: "after".into(),
                        text: "baz".into(),
                        alternative: None,
                    },
                ],
            );
            assert_eq!(graph.full[&target].name, expected, "{kind}");
        }
    }

    #[test]
    fn manual_label_embedded_value_boundaries_and_referenced_tooltip_fallback() {
        let dom=crate::html::parse("<input id=target type=checkbox><label for=target>foo<input value=bar>baz</label><img id=image alt='' title=t aria-labelledby='image other'><span id=other>peanuts</span><img id=plain alt='' title=t>").into_runtime().unwrap();
        for (id, expected) in [
            ("target", "foo bar baz"),
            ("image", "t peanuts"),
            ("plain", ""),
        ] {
            assert_eq!(
                accessibility_for_node(dom.arena(), dom.find_by_id(id).unwrap())
                    .unwrap()
                    .1,
                expected
            );
        }
    }

    #[test]
    fn manual_embedded_menu_and_owned_combobox_values_share_all_host_control_labels() {
        for kind in ["checkbox", "file", "password", "radio", "text"] {
            let source = format!(
                "<input id=target type={kind}><label for=target>Flash <span role=menu><span role=menuitem aria-selected=true>Wrong</span></span>the screen <span role=combobox aria-owns=popup><span role=textbox></span></span> times.</label><ul id=popup role=listbox><li role=option>1</li><li role=option aria-selected=true>2</li></ul>"
            );
            let dom = crate::html::parse(&source).into_runtime().unwrap();
            assert_eq!(
                accessibility_for_node(dom.arena(), dom.find_by_id("target").unwrap())
                    .unwrap()
                    .1,
                "Flash the screen 2 times.",
                "{kind}"
            );
        }
    }

    #[test]
    fn line_breaks_separate_accessible_text_without_inventing_inline_spacing() {
        let dom = crate::html::parse(
            "<button id=breaks>one<br>two</button><button id=inline>one<span>two</span></button>",
        )
        .into_runtime()
        .unwrap();
        for (id, expected) in [("breaks", "one two"), ("inline", "onetwo")] {
            assert_eq!(
                accessibility_for_node(dom.arena(), dom.find_by_id(id).unwrap())
                    .unwrap()
                    .1,
                expected
            );
        }
    }

    #[test]
    fn descendant_labelledby_shares_visited_nodes_only_within_one_name_traversal() {
        let dom=crate::html::parse("<h3 id=heading><a href='#' aria-labelledby=image>ignored</a> <a id=second href='#'>link2 <img id=image alt=image> link3</a></h3><h3 id=nested>heading <a href='#' aria-labelledby=inside>ignored<img id=inside alt=image>ignored</a> heading</h3><h3 id=selfhost><span id=self aria-label=Self aria-labelledby='self suffix'>ignored</span></h3><span id=suffix>suffix</span>").into_runtime().unwrap();
        for (id, expected) in [
            ("heading", "image link2 link3"),
            ("second", "link2 image link3"),
            ("nested", "heading image heading"),
            ("selfhost", "Self suffix"),
        ] {
            assert_eq!(
                accessibility_for_node(dom.arena(), dom.find_by_id(id).unwrap())
                    .unwrap()
                    .1,
                expected,
                "{id}"
            );
        }
    }

    // The semantic consumer must use complete inline context and the language
    // rules even when no V8 feature or computed layout provider is present.
    #[test]
    fn accessibility_locale_case_crosses_text_nodes_without_mutating_source() {
        for (source, expected) in [
            (
                "<h2 id=target lang=tr style='text-transform:uppercase'>i<span>ı</span></h2>",
                "İI",
            ),
            (
                "<h2 id=target lang=tr style='text-transform:lowercase'>I<span>\u{307}</span></h2>",
                "i",
            ),
            (
                "<h2 id=target lang=el style='text-transform:lowercase'>Ο<span>Σ</span></h2>",
                "ος",
            ),
            (
                "<h2 id=target lang=el style='text-transform:lowercase'>Ο<span>Σ</span>Α</h2>",
                "οσα",
            ),
        ] {
            let dom = crate::html::parse(source).into_runtime().unwrap();
            let target = dom.find_by_id("target").unwrap();
            let text_nodes = || {
                dom.arena()
                    .iter()
                    .filter_map(|(id, node)| {
                        if let NodeKind::Text { data } = &node.kind {
                            Some((id, data.clone()))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let original_text = text_nodes();
            let cursor = dom.mutation_cursor();
            let mut graph = SemanticGraph::new(PageId(73), 9);
            graph.reset(dom.arena(), 9);
            assert_eq!(graph.full[&target].name, expected, "{source}");
            assert_eq!(
                accessibility_for_node(dom.arena(), target).unwrap().1,
                expected,
                "{source}"
            );
            assert_eq!(text_nodes(), original_text);
            assert!(dom.mutations_since(cursor).mutations.is_empty());
        }
    }

    #[test]
    fn native_text_style_projection_updates_spacing_and_preserves_inline_case_context() {
        let dom=crate::html::parse("<button id=b><span id=one>one</span><span id=two>two</span></button><h2 id=cap style='text-transform:capitalize'>hel<span>lo</span> world</h2><h2 id=kana style='text-transform:full-size-kana'>びょういん</h2>").into_runtime().unwrap();
        let b = dom.find_by_id("b").unwrap();
        let one = dom.find_by_id("one").unwrap();
        let two = dom.find_by_id("two").unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(graph.full[&b].name, "onetwo");
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        for (display, expected) in [
            ("block", "one two"),
            ("inline-block", "one two"),
            ("inline", "onetwo"),
        ] {
            let styles = oryn_layout::ComputedStyles::from([one, two].map(|id| {
                (
                    pack(id),
                    std::sync::Arc::new(BTreeMap::from([
                        ("display".into(), display.into()),
                        ("text-transform".into(), "none".into()),
                    ])),
                )
            }));
            graph.apply_computed_visibility(dom.arena(), &styles);
            assert_eq!(graph.full[&b].name, expected);
        }
        assert_eq!(
            accessibility_for_node(dom.arena(), dom.find_by_id("cap").unwrap())
                .unwrap()
                .1,
            "Hello World"
        );
        assert_eq!(
            accessibility_for_node(dom.arena(), dom.find_by_id("kana").unwrap())
                .unwrap()
                .1,
            "びょういん"
        );
    }

    #[test]
    fn aria_range_defaults_share_platform_and_webdriver_numeric_projection() {
        for (role, minimum, maximum, current) in [
            ("slider", 0., 100., 50.),
            ("scrollbar", 0., 100., 50.),
            ("separator", 0., 100., 50.),
            ("spinbutton", -9007199254740992., 9007199254740992., 0.),
        ] {
            let mut dom = crate::html::parse(&format!("<div id=r role={role} tabindex=0></div>"))
                .into_runtime()
                .unwrap();
            let id = dom.find_by_id("r").unwrap();
            let properties =
                accessibility_properties(dom.arena(), id, "div", dom.arena().get(id).unwrap());
            assert_eq!(properties["valuemin"].as_f64(), Some(minimum));
            assert_eq!(properties["valuemax"].as_f64(), Some(maximum));
            assert_eq!(properties["valuenow"].as_f64(), Some(current));
            dom.set_attribute(id, "aria-valuemin", "0").unwrap();
            dom.set_attribute(id, "aria-valuemax", "40").unwrap();
            let properties =
                accessibility_properties(dom.arena(), id, "div", dom.arena().get(id).unwrap());
            assert_eq!(
                properties["valuenow"].as_f64(),
                Some(if role == "spinbutton" { 0. } else { 20. })
            );
        }
    }

    #[test]
    fn generated_marker_alternatives_name_references_in_native_order_and_invalidate() {
        let dom = crate::html::parse(
            "<ul><li id=item>Body</li></ul><button id=reference aria-labelledby=item></button>",
        )
        .into_runtime()
        .unwrap();
        let item = dom.find_by_id("item").unwrap();
        let reference = dom.find_by_id("reference").unwrap();
        let owner = (u64::from(item.generation) << 32) | u64::from(item.slot);
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        for (alternative, expected) in [
            (None, "• Prefix Body After"),
            (Some("Bullet"), "Bullet Prefix Body After"),
            (Some(""), "Prefix Body After"),
        ] {
            let content = vec![
                oryn_common::v4::GeneratedContent {
                    separates_text: false,
                    owner,
                    pseudo: "before".into(),
                    text: "Prefix ".into(),
                    alternative: None,
                },
                oryn_common::v4::GeneratedContent {
                    separates_text: false,
                    owner,
                    pseudo: "after".into(),
                    text: " After".into(),
                    alternative: None,
                },
                oryn_common::v4::GeneratedContent {
                    separates_text: false,
                    owner,
                    pseudo: "marker".into(),
                    text: "• ".into(),
                    alternative: alternative.map(str::to_owned),
                },
            ];
            graph.apply_generated_content(dom.arena(), &content);
            assert_eq!(graph.full[&reference].name, expected);
        }
        graph.apply_generated_content(dom.arena(), &[]);
        assert_eq!(graph.full[&reference].name, "Body");
    }

    #[test]
    fn html_aam_container_naming_precedence_matrix() {
        // The 16 element families and eight precedence combinations are the
        // declared matrix in pinned html-aam/names.html, including details
        // whose summary is not an implicit name for the group itself.
        for tag in [
            "address",
            "aside",
            "blockquote",
            "details",
            "figure",
            "footer",
            "form",
            "hgroup",
            "hr",
            "ol",
            "main",
            "menu",
            "nav",
            "search",
            "section",
            "ul",
        ] {
            for (attributes, expected) in [
                ("", ""),
                ("aria-label='label'", "label"),
                ("aria-labelledby='reference'", "labelledby"),
                ("title='title'", "title"),
                (
                    "aria-labelledby='reference' aria-label='label' title='title'",
                    "labelledby",
                ),
                (
                    "aria-labelledby='reference' aria-label='label'",
                    "labelledby",
                ),
                ("aria-labelledby='reference' title='title'", "labelledby"),
                ("aria-label='label' title='title'", "label"),
            ] {
                let content = if tag == "details" {
                    "<summary>x</summary>"
                } else {
                    "x"
                };
                let source = format!(
                    "<div id=reference>labelledby</div><{tag} id=target {attributes}>{content}</{tag}>"
                );
                let dom = crate::html::parse(&source).into_runtime().unwrap();
                let target = dom.find_by_id("target").unwrap();
                assert_eq!(
                    accessibility_for_node(dom.arena(), target).unwrap().1,
                    expected,
                    "{tag} {attributes}"
                );
            }
        }
    }

    #[test]
    fn native_table_presentation_propagates_required_children_and_reverses_after_mutation() {
        let mut dom=crate::html::parse("<table id=t role=none><tbody id=body><tr id=row><td id=cell aria-describedby=description>Cell</td><td id=explicit role=button>Action</td><td id=focus tabindex=0>Focusable</td></tr></tbody></table><span id=description>Help</span>").into_runtime().unwrap();
        let table = dom.find_by_id("t").unwrap();
        for id in ["body", "row", "cell"] {
            assert_eq!(
                computed_role_for_node(dom.arena(), dom.find_by_id(id).unwrap()),
                "none"
            );
        }
        assert_eq!(
            computed_role_for_node(dom.arena(), dom.find_by_id("explicit").unwrap()),
            "button"
        );
        assert_eq!(
            computed_role_for_node(dom.arena(), dom.find_by_id("focus").unwrap()),
            "cell"
        );
        dom.set_attribute(table, "role", "table").unwrap();
        for (id, role) in [("body", "rowgroup"), ("row", "row"), ("cell", "cell")] {
            assert_eq!(
                computed_role_for_node(dom.arena(), dom.find_by_id(id).unwrap()),
                role
            );
        }
    }

    #[test]
    fn heading_names_share_depth_first_exposed_tree_for_articles_and_dialogs() {
        let dom=crate::html::parse("<article id=a><div><h3>First nested</h3></div><h2>Second</h2></article><div id=d role=dialog><div hidden><h2>Hidden</h2></div><div><span role=heading>Dialog heading</span></div><h2>Later</h2></div><div id=g role=group><h2>No group name</h2></div>").into_runtime().unwrap();
        for (id, expected) in [("a", "First nested"), ("d", "Dialog heading"), ("g", "")] {
            let node = dom.find_by_id(id).unwrap();
            assert_eq!(
                accessibility_for_node(dom.arena(), node).unwrap().1,
                expected
            );
        }
    }

    #[test]
    fn missing_geometry_drops_only_previous_geometry_exclusion() {
        let dom=crate::html::parse("<button id=boxless>Boxless</button><button id=aria aria-hidden=true>ARIA</button><button id=css>CSS</button>").into_runtime().unwrap();
        let boxless = dom.find_by_id("boxless").unwrap();
        let aria = dom.find_by_id("aria").unwrap();
        let css = dom.find_by_id("css").unwrap();
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_geometry_visibility(
            &[boxless, aria, css]
                .into_iter()
                .map(|id| {
                    (
                        pack(id),
                        oryn_common::v4::NodeGeometry {
                            availability: oryn_common::v4::GeometryAvailability::Hidden,
                            box_model: None,
                            client_rects: Vec::new(),
                            in_viewport: false,
                            interactable: false,
                            diagnostic: None,
                        },
                    )
                })
                .collect(),
        );
        assert!(graph.full[&boxless].states.contains(&NodeState::Hidden));
        let styles = oryn_layout::ComputedStyles::from([(
            pack(css),
            std::sync::Arc::new(BTreeMap::from([
                ("display".into(), "none".into()),
                ("visibility".into(), "visible".into()),
            ])),
        )]);
        graph.apply_computed_visibility(dom.arena(), &styles);
        graph.apply_geometry_visibility(&BTreeMap::new());
        assert!(!graph.full[&boxless].states.contains(&NodeState::Hidden));
        assert!(graph.full[&aria].states.contains(&NodeState::Hidden));
        assert!(graph.full[&css].states.contains(&NodeState::Hidden));
        assert!(graph.geometry_hidden.is_empty());
        assert!(!graph.accessibility[&boxless].excluded);
        assert!(graph.accessibility[&aria].excluded);
        assert!(graph.accessibility[&css].excluded);
    }

    #[test]
    fn computed_visibility_drives_names_and_exclusion_without_author_attribute_changes() {
        let dom=crate::html::parse("<button id=b><span id=s>Secret </span>Public</button><button id=r aria-labelledby=s></button><div id=p><button id=c>Child</button></div><button id=h hidden>Overridden</button>").into_runtime().unwrap();
        let id = |name: &str| {
            dom.arena()
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").is_some_and(|id| id == name))
                .unwrap()
                .0
        };
        let (b, s, r, p, c, h) = (id("b"), id("s"), id("r"), id("p"), id("c"), id("h"));
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let style = |display: &str, visibility: &str| {
            std::sync::Arc::new(BTreeMap::from([
                ("display".into(), display.into()),
                ("visibility".into(), visibility.into()),
            ]))
        };
        let mut styles = oryn_layout::ComputedStyles::from([
            (pack(b), style("block", "visible")),
            (pack(s), style("none", "visible")),
            (pack(r), style("block", "visible")),
            (pack(p), style("block", "hidden")),
            (pack(c), style("block", "visible")),
            (pack(h), style("block", "visible")),
        ]);
        let cursor = dom.mutation_cursor();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_computed_visibility(dom.arena(), &styles);
        assert_eq!(graph.full[&b].name, "Public");
        assert_eq!(graph.full[&r].name, "Secret");
        assert!(graph.full[&s].states.contains(&NodeState::Hidden));
        assert!(!graph.full[&c].states.contains(&NodeState::Hidden));
        assert!(!graph.full[&h].states.contains(&NodeState::Hidden));
        styles.insert(pack(s), style("inline", "visible"));
        styles.insert(pack(p), style("none", "visible"));
        graph.apply_computed_visibility(dom.arena(), &styles);
        assert_eq!(graph.full[&b].name, "Secret Public");
        assert!(graph.full[&c].states.contains(&NodeState::Hidden));
        let mut full = SemanticGraph::new(PageId(1), 1);
        full.computed_visibility = graph.computed_visibility.clone();
        full.reset(dom.arena(), 1);
        assert_eq!(
            graph.nodes(ProjectionProfile::Full),
            full.nodes(ProjectionProfile::Full)
        );
        assert!(dom.mutations_since(cursor).mutations.is_empty());
        assert!(
            dom.arena()
                .get(h)
                .unwrap()
                .attributes
                .contains_null("hidden")
        );
        graph.reset(dom.arena(), 2);
        assert!(graph.computed_visibility.is_empty());
    }

    #[test]
    fn empty_painted_counter_alternatives_name_visible_controls_not_hidden_content() {
        let dom = crate::html::parse("<button id=button>label</button><h3 id=heading>label</h3><a id=link href='#'>label</a><button id=parent><span id=hidden hidden>Secret</span>Public</button>").into_runtime().unwrap();
        let id = |name: &str| {
            dom.arena()
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").map(String::as_str) == Some(name))
                .unwrap()
                .0
        };
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        let content = vec![
            oryn_common::v4::GeneratedContent {
                owner: pack(id("button")),
                pseudo: "before".into(),
                text: String::new(),
                alternative: Some("1 ".into()),
                separates_text: false,
            },
            oryn_common::v4::GeneratedContent {
                owner: pack(id("heading")),
                pseudo: "before".into(),
                text: String::new(),
                alternative: Some("2 ".into()),
                separates_text: false,
            },
            oryn_common::v4::GeneratedContent {
                owner: pack(id("link")),
                pseudo: "after".into(),
                text: String::new(),
                alternative: Some(" 3".into()),
                separates_text: false,
            },
            oryn_common::v4::GeneratedContent {
                owner: pack(id("hidden")),
                pseudo: "before".into(),
                text: String::new(),
                alternative: Some("Hidden counter ".into()),
                separates_text: false,
            },
        ];
        graph.apply_generated_content(dom.arena(), &content);
        assert_eq!(graph.full[&id("button")].name, "1 label");
        assert_eq!(graph.full[&id("heading")].name, "2 label");
        assert_eq!(graph.full[&id("link")].name, "label 3");
        assert_eq!(graph.full[&id("parent")].name, "Public");
        assert!(
            graph.full[&id("hidden")]
                .states
                .contains(&NodeState::Hidden)
        );
        graph.apply_generated_content(dom.arena(), &content);
        assert_eq!(graph.full[&id("button")].name, "1 label");
        graph.apply_generated_content(dom.arena(), &[]);
        assert_eq!(graph.full[&id("button")].name, "label");
    }
    #[test]
    fn generated_names_follow_native_owner_labels_and_removal_without_dom_mutation() {
        let parsed = crate::html::parse(
            "<label id=label for=field><span id=source>Amount</span></label><input id=field><button id=ref aria-labelledby=label></button><button id=explicit aria-label=Authored>Text</button>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let id = |name: &str| {
            dom.arena()
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").is_some_and(|id| id == name))
                .unwrap()
                .0
        };
        let source = id("source");
        let field = id("field");
        let reference = id("ref");
        let explicit = id("explicit");
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let before = oryn_common::v4::GeneratedContent {
            separates_text: false,
            owner: pack(source),
            pseudo: "before".into(),
            text: "$ ".into(),
            alternative: None,
        };
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        let cursor = dom.mutation_cursor();
        graph.apply_generated_content(
            dom.arena(),
            &[
                before.clone(),
                oryn_common::v4::GeneratedContent {
                    separates_text: false,
                    owner: pack(explicit),
                    pseudo: "before".into(),
                    text: "Ignored".into(),
                    alternative: None,
                },
            ],
        );
        assert_eq!(graph.full[&field].name, "$ Amount");
        assert_eq!(graph.full[&reference].name, "$ Amount");
        assert_eq!(graph.full[&explicit].name, "Authored");
        assert!(dom.mutations_since(cursor).mutations.is_empty());
        assert!(
            dom.arena()
                .accessibility_generated_content(source)
                .is_none()
        );
        dom.set_text_content(source, "Price").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(graph.full[&field].name, "$ Price");
        graph.apply_generated_content(dom.arena(), &[]);
        assert_eq!(graph.full[&field].name, "Price");
        assert_eq!(graph.full[&reference].name, "Price");
        graph.apply_generated_content(
            dom.arena(),
            &[oryn_common::v4::GeneratedContent {
                alternative: Some("Money ".into()),
                ..before
            }],
        );
        assert_eq!(graph.full[&field].name, "Money Price");
        graph.reset(dom.arena(), 2);
        assert_eq!(graph.full[&field].name, "Price");
    }

    #[test]
    fn targeting_hints_preserve_names_ambiguity_privacy_and_mutation_identity() {
        use oryn_common::agent::Target;
        let parsed = crate::html::parse(
            "<button id=plus aria-label='Increase quantity'>+</button><label for=coupon>Coupon</label><input id=coupon placeholder='Enter code'><button aria-label='Submit'><input type=password value=secret><span hidden>hidden-label</span>Send</button>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        let resolve = |graph: &SemanticGraph, text: &str, action| {
            graph.resolve(
                &Target::Text { text: text.into() },
                action,
                Revision(1),
                None,
            )
        };
        let plus = resolve(&graph, "+", SemanticAction::Click)
            .unwrap()
            .selected
            .unwrap();
        assert_eq!(
            resolve(&graph, "Increase quantity", SemanticAction::Click)
                .unwrap()
                .selected,
            Some(plus)
        );
        assert!(
            graph
                .nodes(ProjectionProfile::Compact)
                .iter()
                .any(|node| node.semantic_ref == plus && node.name == "Increase quantity")
        );
        assert_eq!(
            resolve(&graph, "coupon", SemanticAction::Type)
                .unwrap()
                .selected,
            resolve(&graph, "Enter code", SemanticAction::Type)
                .unwrap()
                .selected
        );
        for text in ["secret", "hidden-label", ""] {
            assert_eq!(
                resolve(&graph, text, SemanticAction::Click),
                Err(SemanticResolutionError::NotFound)
            );
        }
        let cursor = dom.mutation_cursor();
        let node = dom
            .query_selector(dom.document(), "#plus")
            .unwrap()
            .unwrap();
        dom.set_text_content(node, "-").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(
            resolve(&graph, "+", SemanticAction::Click),
            Err(SemanticResolutionError::NotFound)
        );
        assert_eq!(
            resolve(&graph, "-", SemanticAction::Click)
                .unwrap()
                .selected,
            Some(plus)
        );
        let parsed = crate::html::parse(
            "<button aria-label=First>+</button><button aria-label=Second>+</button>",
        );
        graph.reset(&parsed.dom, 2);
        assert!(
            matches!(resolve(&graph, "+", SemanticAction::Click), Err(SemanticResolutionError::Ambiguous(items)) if items.len() == 2)
        );
    }

    #[test]
    fn retained_replaced_frame_document_is_removed_from_projection() {
        let mut dom = crate::dom::DomRuntime::new();
        let frame = dom.create_element("iframe");
        dom.append_child(dom.document(), frame).unwrap();
        let old = dom.create(NodeKind::Document);
        let button = dom.create_element("button");
        dom.append_child(old, button).unwrap();
        dom.set_hosted_document(frame, Some(old)).unwrap();
        dom.retain_native(button).unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        assert!(graph.full.contains_key(&button));
        let cursor = dom.mutation_cursor();
        dom.remove(frame).unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert!(
            !graph.full.contains_key(&button),
            "detached frame must not remain actionable"
        );
        let cursor = dom.mutation_cursor();
        dom.append_child(dom.document(), frame).unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert!(graph.full.contains_key(&button));
        let cursor = dom.mutation_cursor();
        let replacement = dom.create(NodeKind::Document);
        dom.set_hosted_document(frame, Some(replacement)).unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert!(dom.arena().get(button).is_ok(), "retained DOM must survive");
        assert!(
            !graph.full.contains_key(&button),
            "inactive DOM must not be actionable"
        );
        assert!(project_element(dom.arena(), button, PageId(1), 1, 1).is_none());
        dom.release_native(button).unwrap();
    }

    #[test]
    fn incremental_accessible_reference_and_tooltip_precedence_matches_full_projection() {
        let parsed = crate::html::parse(
            "<span id=reference></span><input id=field aria-label=Same title=Same aria-description=Details>",
        );
        let mut dom = parsed.into_runtime().expect("live DOM");
        let field = dom
            .query_selector(dom.document(), "#field")
            .unwrap()
            .unwrap();
        let reference = dom
            .query_selector(dom.document(), "#reference")
            .unwrap()
            .unwrap();
        let page = PageId(19);
        let mut graph = SemanticGraph::new(page, 1);
        graph.reset(dom.arena(), 1);
        for step in 0..7 {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom
                    .set_attribute(field, "aria-labelledby", "reference")
                    .unwrap(),
                1 => dom
                    .set_attribute(field, "aria-describedby", "reference")
                    .unwrap(),
                2 => dom.set_text_content(reference, "Referenced").unwrap(),
                3 => dom
                    .set_attribute(reference, "id", "retired-reference")
                    .unwrap(),
                4 => dom.remove_attribute(field, "aria-description").unwrap(),
                5 => dom.remove_attribute(field, "aria-label").unwrap(),
                _ => dom.set_attribute(field, "aria-label", "Same").unwrap(),
            };
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), page, 1)
            );
            let accessible = accessible_semantics(dom.arena(), field, "input", "textbox", page, 1);
            let expected = match step {
                0 => ("", Some("Details")),
                1 => ("", None),
                2 => ("Referenced", Some("Referenced")),
                3 => ("Same", Some("Details")),
                4 | 6 => ("Same", Some("Same")),
                _ => ("Same", None),
            };
            assert_eq!(
                (accessible.name.as_str(), accessible.description.as_deref()),
                expected
            );
        }
    }

    #[test]
    fn incremental_graph_matches_full_projection_after_mutations() {
        let parsed = crate::html::parse(
            "<label id=label for=field>Account</label><input id=field><button>Save</button>",
        );
        let mut dom = parsed.into_runtime().expect("live DOM");
        let page_id = PageId(9);
        let generation = 4;
        let mut graph = SemanticGraph::new(page_id, generation);
        graph.reset(dom.arena(), generation);
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), page_id, generation)
        );

        let cursor = dom.mutation_cursor();
        let field = dom
            .query_selector(dom.document(), "#field")
            .expect("query")
            .expect("field");
        let label = dom
            .query_selector(dom.document(), "#label")
            .expect("query")
            .expect("label");
        dom.set_attribute(field, "aria-labelledby", "label")
            .expect("attribute");
        dom.set_text_content(label, "Profile").expect("text");
        let batch = dom.mutations_since(cursor);
        assert!(!batch.truncated);
        graph.apply_mutations(dom.arena(), &batch.mutations);
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), page_id, generation)
        );
    }

    #[test]
    fn incremental_graph_matches_the_full_oracle_at_every_mutation_checkpoint() {
        let parsed = crate::html::parse(
            "<main id=root><span id=name>Initial</span><input id=field aria-labelledby=name><button id=save>Save</button></main>",
        );
        let mut dom = parsed.into_runtime().expect("live DOM");
        let page_id = PageId(19);
        let generation = 7;
        let mut graph = SemanticGraph::new(page_id, generation);
        graph.reset(dom.arena(), generation);
        let root = dom.find_by_id("root").expect("root");
        let name = dom.find_by_id("name").expect("name");
        let field = dom.find_by_id("field").expect("field");
        let save = dom.find_by_id("save").expect("save");

        for step in 0..48 {
            let cursor = dom.mutation_cursor();
            match step % 8 {
                0 => dom
                    .set_text_content(name, format!("Account {step}"))
                    .expect("label text"),
                1 => dom
                    .set_attribute(field, "aria-describedby", "name")
                    .expect("description relation"),
                2 => dom
                    .set_attribute(save, "aria-label", format!("Save {step}"))
                    .expect("button name"),
                3 => dom
                    .set_attribute(field, "value", format!("value-{step}"))
                    .expect("form value"),
                4 => dom.set_attribute(save, "hidden", "").expect("hide button"),
                5 => dom.remove_attribute(save, "hidden").expect("show button"),
                6 => {
                    let status = dom.create_element("output");
                    dom.set_attribute(status, "aria-live", "polite")
                        .expect("live region");
                    let text = dom.create_text(format!("Status {step}"));
                    dom.append_child(status, text).expect("status text");
                    dom.append_child(root, status).expect("status node");
                }
                _ => dom
                    .set_attribute(field, "aria-labelledby", "name")
                    .expect("label relation"),
            }
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), page_id, generation),
                "compact projection diverged at step {step}"
            );
            assert_eq!(
                graph.nodes(ProjectionProfile::Full),
                project_document_full(dom.arena(), page_id, generation),
                "full projection diverged at step {step}"
            );
        }
    }

    #[test]
    fn v6_graph_owns_accessibility_structures_and_field_evidence() {
        let parsed =
            crate::html::parse("<nav aria-label=Primary><a id=home href=/home>Home</a></nav>");
        let page_id = PageId(12);
        let generation = 5;
        let mut graph = SemanticGraph::new(page_id, generation);
        graph.reset(&parsed.dom, generation);
        let base = Observation {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id,
                opener_page_id: None,
                url: "https://semantic.example/".into(),
                title: String::new(),
                document_generation: generation,
            },
            revision: Revision(1),
            profile: ProjectionProfile::Compact,
            source: ObservationSource::NativeDom,
            nodes: graph.nodes(ProjectionProfile::Compact),
            capabilities: Vec::new(),
            diagnostics: Vec::new(),
            execution_domain: ExecutionDomain::Native,
        };
        let observed = graph
            .observation_v6(base, &SemanticObservationOptions::default(), None)
            .expect("v6 observation");
        assert_eq!(observed.contract_version, oryn_common::v6::CONTRACT_VERSION);
        assert!(
            observed
                .structures
                .iter()
                .any(|structure| { structure.kind == SemanticStructureKind::Navigation })
        );
        let home = observed
            .nodes
            .iter()
            .find(|node| node.semantic.name == "Home")
            .expect("home semantic node");
        assert!(home.field_evidence.iter().any(|evidence| {
            evidence.field == SemanticField::Name && evidence.trust == ContentTrust::PageAuthored
        }));
    }

    #[test]
    fn graph_resolution_fails_closed_for_ambiguity_and_missing_geometry() {
        use oryn_common::agent::Target;
        let parsed = crate::html::parse(
            "<p id=anchor>Plan</p><button>Continue</button><button>Continue</button>",
        );
        let mut graph = SemanticGraph::new(PageId(4), 2);
        graph.reset(&parsed.dom, 2);
        let ambiguous = graph.resolve(
            &Target::Text {
                text: "Continue".into(),
            },
            SemanticAction::Click,
            Revision(1),
            None,
        );
        assert!(matches!(
            ambiguous,
            Err(SemanticResolutionError::Ambiguous(candidates)) if candidates.len() == 2
        ));
        let near = graph.resolve(
            &Target::Near {
                target: Box::new(Target::Text {
                    text: "Continue".into(),
                }),
                anchor: Box::new(Target::Text {
                    text: "Plan".into(),
                }),
            },
            SemanticAction::Click,
            Revision(1),
            None,
        );
        assert_eq!(near, Err(SemanticResolutionError::GeometryUnavailable));
    }

    #[test]
    fn near_uses_box_centers_and_preserves_equal_distance_ambiguity() {
        use oryn_common::{
            agent::Target,
            v4::{BoxModel, CssRect, GeometryAvailability, NodeGeometry},
        };
        let parsed = crate::html::parse(
            "<p id=anchor>Wireless Mouse</p><button id=first>+</button><button id=second>+</button>",
        );
        let mut graph = SemanticGraph::new(PageId(4), 2);
        graph.reset(&parsed.dom, 2);
        let reference = |id: &str| {
            graph
                .full
                .values()
                .find(|node| node.selector.as_deref() == Some(id))
                .unwrap()
                .semantic_ref
        };
        let anchor = reference("#anchor");
        let first = reference("#first");
        let second = reference("#second");
        let geometry = |x, y, width, height| {
            let rect = CssRect {
                x,
                y,
                width,
                height,
            };
            NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    margin: Default::default(),
                    border: Default::default(),
                    padding: Default::default(),
                }),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            }
        };
        // A slightly shorter second row puts its button's top-left corner
        // closer, even though the first control's center is nearest the label.
        let mut boxes = BTreeMap::from([
            (anchor.node, geometry(288., 266., 117.728, 24.)),
            (first.node, geometry(927.282, 279., 10.975, 30.)),
            (second.node, geometry(918.130, 368., 10.975, 30.)),
        ]);
        let target = Target::Near {
            target: Box::new(Target::Text { text: "+".into() }),
            anchor: Box::new(Target::Reference { reference: anchor }),
        };
        let result = graph
            .resolve(&target, SemanticAction::Click, Revision(1), Some(&boxes))
            .unwrap();
        assert_eq!(result.selected, Some(first));
        boxes.insert(second.node, boxes[&first.node].clone());
        assert!(
            matches!(graph.resolve(&target,SemanticAction::Click,Revision(1),Some(&boxes)),Err(SemanticResolutionError::Ambiguous(candidates)) if candidates.len()==2)
        );
    }

    #[test]
    fn v6_delta_contains_only_changed_accessibility_nodes() {
        let parsed =
            crate::html::parse("<button id=first>First</button><button id=second>Second</button>");
        let mut dom = parsed.into_runtime().expect("live DOM");
        let mut graph = SemanticGraph::new(PageId(8), 3);
        graph.reset(dom.arena(), 3);
        let _ = graph.take_v6_changes();
        let cursor = dom.mutation_cursor();
        let first = dom
            .query_selector(dom.document(), "#first")
            .expect("query")
            .expect("first");
        dom.set_attribute(first, "aria-label", "Updated")
            .expect("attribute");
        let batch = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &batch.mutations);
        let changes = graph.take_v6_changes();
        assert_eq!(changes.upserted_nodes.len(), 1);
        assert_eq!(changes.upserted_accessibility.len(), 1);
        assert_eq!(changes.upserted_accessibility[0].name, "Updated");
    }

    #[test]
    fn computed_geometry_visibility_updates_the_persistent_graph() {
        let parsed = crate::html::parse("<button id=save>Save</button>");
        let mut graph = SemanticGraph::new(PageId(4), 2);
        graph.reset(&parsed.dom, 2);
        let reference = graph
            .nodes(ProjectionProfile::Compact)
            .into_iter()
            .find(|node| node.name == "Save")
            .expect("save node")
            .semantic_ref;
        graph.take_v6_changes();
        graph.apply_geometry_visibility(&BTreeMap::from([(
            reference.node,
            oryn_common::v4::NodeGeometry {
                availability: oryn_common::v4::GeometryAvailability::Hidden,
                box_model: None,
                client_rects: Vec::new(),
                in_viewport: false,
                interactable: false,
                diagnostic: None,
            },
        )]));
        let changes = graph.take_v6_changes();
        let save = changes
            .upserted_nodes
            .iter()
            .find(|node| node.semantic.semantic_ref == reference)
            .expect("visibility upsert");
        assert!(save.semantic.states.contains(&NodeState::Hidden));
        assert!(changes.upserted_accessibility[0].excluded);
        assert!(graph.nodes(ProjectionProfile::Compact).is_empty());
        graph.apply_geometry_visibility(&BTreeMap::from([(
            reference.node,
            oryn_common::v4::NodeGeometry {
                availability: oryn_common::v4::GeometryAvailability::Available,
                box_model: None,
                client_rects: Vec::new(),
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
        )]));
        assert!(
            graph
                .nodes(ProjectionProfile::Compact)
                .iter()
                .any(|node| node.semantic_ref == reference
                    && node.states.contains(&NodeState::Visible))
        );
    }

    #[test]
    fn dependency_reindex_does_not_republish_unrelated_accessible_names() {
        let parsed = crate::html::parse(
            "<span id=one>One</span><button id=a aria-labelledby=one></button>\
             <span id=two>Two</span><button id=b aria-labelledby=two></button>",
        );
        let mut dom = parsed.into_runtime().expect("live DOM");
        let mut graph = SemanticGraph::new(PageId(15), 6);
        graph.reset(dom.arena(), 6);
        let _ = graph.take_v6_changes();
        let label = dom
            .query_selector(dom.document(), "#one")
            .expect("query")
            .expect("first label");
        let unrelated = dom
            .query_selector(dom.document(), "#b")
            .expect("query")
            .expect("unrelated button");
        let unrelated_ref = semantic_ref_for(unrelated, PageId(15), 6);
        let cursor = dom.mutation_cursor();
        dom.set_attribute(label, "id", "renamed")
            .expect("rename id");
        let batch = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &batch.mutations);
        let changes = graph.take_v6_changes();
        assert!(
            changes
                .upserted_accessibility
                .iter()
                .all(|node| node.semantic_ref != unrelated_ref),
            "an unrelated accessible-name dependency was republished"
        );
    }

    fn semantic_observation(html: &str) -> SemanticObservation {
        let document = crate::html::parse(html);
        let page_id = PageId(7);
        let generation = 11;
        let observation = Observation {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id,
                opener_page_id: None,
                url: "https://semantic.example/".into(),
                title: "Semantic fixture".into(),
                document_generation: generation,
            },
            revision: Revision(3),
            profile: ProjectionProfile::Full,
            source: ObservationSource::NativeDom,
            nodes: project_document_full(&document.dom, page_id, generation),
            capabilities: Vec::new(),
            diagnostics: Vec::new(),
            execution_domain: ExecutionDomain::Native,
        };
        project_semantic_observation(&document.dom, observation)
    }

    #[test]
    fn native_projection_marks_page_content_untrusted_and_rule_provenance() {
        let document = crate::html::parse(
            r#"<button title="Continue" style="display:block">Fallback text</button>"#,
        );
        let nodes = project_document(&document.dom, PageId(3), 7);
        let button = nodes
            .iter()
            .find(|node| node.role == "button")
            .expect("button projection");

        assert_eq!(button.content_trust, ContentTrust::PageAuthored);
        assert_eq!(button.provenance, Provenance::BrowserDeterministic);
    }

    #[test]
    fn supported_accessible_name_description_and_relationship_rules_are_explicit() {
        let observation = semantic_observation(
            r#"
            <h1 id=title>Checkout</h1>
            <form aria-labelledby=title>
              <label for=email>Email address</label>
              <input id=email aria-describedby=hint>
              <p id=hint>We send receipts here.</p>
            </form>
            "#,
        );
        let input = observation
            .accessibility
            .iter()
            .find(|node| node.role == "textbox")
            .expect("textbox accessibility node");
        assert_eq!(input.name, "Email address");
        assert_eq!(input.description.as_deref(), Some("We send receipts here."));
        assert!(
            input
                .relationships
                .contains_key(&AccessibilityRelation::DescribedBy)
        );
        assert_eq!(input.semantic_ref.page_id, PageId(7));
        assert!(input.evidence.iter().any(|item| {
            item.kind == AccessibilityEvidenceKind::HostLanguage && item.source == "label"
        }));
        let form = observation
            .structures
            .iter()
            .find(|structure| structure.kind == SemanticStructureKind::Form)
            .expect("form structure");
        assert_eq!(form.name, "Checkout");
        assert!(form.members.contains(&input.semantic_ref));
    }

    #[test]
    fn pinned_agent_critical_accname_and_role_cases_conform() {
        // Curated from the WAI-ARIA 1.2 / Accessible Name 1.2 agent-critical
        // rules. The pin is repository-owned so expectation drift is reviewed.
        let observation = semantic_observation(
            r#"
            <button id=save><img alt=Save><span aria-hidden=true>dangerous hint</span></button>
            <input id=submit type=submit>
            <span id=first aria-label="Account"></span><span id=second>settings</span>
            <button id=refs aria-labelledby="first second">ignored</button>
            <button id=conflict role="unknown presentation">Focusable</button>
            <div id=fallback role="unknown button">Fallback role</div>
            "#,
        );
        let by_name = |id: &str| {
            observation
                .accessibility
                .iter()
                .find(|node| {
                    observation
                        .observation
                        .nodes
                        .iter()
                        .find(|semantic| semantic.semantic_ref == node.semantic_ref)
                        .and_then(|semantic| semantic.selector.as_deref())
                        == Some(&format!("#{id}"))
                })
                .expect("pinned accessibility node")
        };
        assert_eq!(by_name("save").name, "Save");
        assert_eq!(by_name("submit").name, "Submit");
        assert_eq!(by_name("refs").name, "Account settings");
        assert_eq!(by_name("conflict").role, "button");
        assert_eq!(by_name("fallback").role, "button");
    }

    #[test]
    fn accessible_names_follow_shadow_slot_assignment_and_fallback() {
        let document = crate::html::parse(
            r#"<button id=host><span id=assigned slot=x>assigned</span><i>default</i></button>
            <div id=shadow>before <slot name=x aria-label=ignored>fallback</slot> / <slot></slot> / <slot name=x>second fallback</slot> after</div>"#,
        );
        let by_id = |value: &str| {
            document
                .dom
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").is_some_and(|id| id == value))
                .map(|(id, _)| id)
                .unwrap()
        };
        let host = by_id("host");
        let shadow = by_id("shadow");
        let assigned = by_id("assigned");
        let mut runtime = document.into_runtime().unwrap();
        attach_test_shadow(&mut runtime, host, shadow);
        assert_eq!(
            accessibility_for_node(runtime.arena(), host).unwrap().1,
            "before assigned / default / second fallback after"
        );
        runtime
            .set_attribute(assigned, "slot", "unassigned")
            .unwrap();
        assert_eq!(
            accessibility_for_node(runtime.arena(), host).unwrap().1,
            "before fallback / default / second fallback after"
        );
    }

    #[test]
    fn grid_readonly_inheritance_reconciles_writable_actions_after_parent_changes() {
        let mut dom=crate::html::parse("<div id=grid role=grid aria-readonly=true><div role=row><span id=cell role=gridcell>Value</span><span id=override role=rowheader aria-readonly=false>Heading</span></div></div>").into_runtime().unwrap();
        let grid = dom.find_by_id("grid").unwrap();
        let cell = dom.find_by_id("cell").unwrap();
        let explicit = dom.find_by_id("override").unwrap();
        let mut graph = SemanticGraph::new(PageId(21), 1);
        graph.reset(dom.arena(), 1);
        assert!(graph.full[&cell].states.contains(&NodeState::Readonly));
        assert!(!graph.full[&cell].actions.contains(&SemanticAction::Type));
        assert!(
            graph.full[&explicit]
                .actions
                .contains(&SemanticAction::Type)
        );
        let cursor = dom.mutation_cursor();
        dom.set_attribute(grid, "aria-readonly", "false").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert!(!graph.full[&cell].states.contains(&NodeState::Readonly));
        assert!(graph.full[&cell].actions.contains(&SemanticAction::Type));
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(21), 1)
        );
    }

    #[test]
    fn reflected_action_targets_override_idrefs_in_host_name_and_incremental_dependencies() {
        let mut dom=crate::html::parse("<button id=host aria-actions=a>Save <span id=a>First</span><span id=b>Second</span></button>").into_runtime().unwrap();
        let host = dom.find_by_id("host").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let mut graph = SemanticGraph::new(PageId(20), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(
            accessibility_for_node(dom.arena(), host).unwrap().1,
            "Save Second"
        );
        let cursor = dom.mutation_cursor();
        dom.set_accessibility_references(host, "aria-actions", false, Some(vec![b]))
            .unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(
            accessibility_for_node(dom.arena(), host).unwrap().1,
            "Save First"
        );
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(20), 1)
        );
        dom.set_accessibility_references(host, "aria-actions", false, Some(vec![a, b]))
            .unwrap();
        assert_eq!(accessibility_for_node(dom.arena(), host).unwrap().1, "Save");
    }

    #[test]
    fn native_aria_platform_properties_and_error_dependencies_follow_mutation() {
        let mut dom=crate::html::parse("<div id=control role=slider aria-valuemin=-1.5 aria-valuemax=4.5 aria-valuenow=2.5 aria-valuetext=Medium aria-roledescription=Level aria-invalid=true aria-errormessage=error></div><span id=error>Fix value</span>").into_runtime().unwrap();
        let control = dom.find_by_id("control").unwrap();
        let error = dom.find_by_id("error").unwrap();
        let mut graph = SemanticGraph::new(PageId(19), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(
            graph.accessibility[&control].properties["valuemin"],
            serde_json::json!(-1.5)
        );
        assert_eq!(
            graph.accessibility[&control].properties["validation_error"],
            serde_json::json!("Fix value")
        );
        let cursor = dom.mutation_cursor();
        dom.set_text_content(error, "Updated error").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(
            graph.accessibility[&control].properties["validation_error"],
            serde_json::json!("Updated error")
        );
        let cursor = dom.mutation_cursor();
        dom.set_attribute(control, "aria-invalid", "false").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert!(
            !graph.accessibility[&control]
                .properties
                .contains_key("validation_error")
        );
    }

    #[test]
    fn automatic_header_roles_and_minimum_roles_recompute_from_native_context() {
        let mut dom=crate::html::parse("<table><tr id=row><th id=header>Heading</th></tr></table><div id=minimum role=none draggable></div><nav id=form role=form></nav><img id=empty>").into_runtime().unwrap();
        let header = dom.find_by_id("header").unwrap();
        let row = dom.find_by_id("row").unwrap();
        assert_eq!(computed_role_for_node(dom.arena(), header), "columnheader");
        assert_eq!(
            computed_role_for_node(dom.arena(), dom.find_by_id("minimum").unwrap()),
            "group"
        );
        assert_eq!(
            computed_role_for_node(dom.arena(), dom.find_by_id("form").unwrap()),
            "navigation"
        );
        assert_eq!(
            computed_role_for_node(dom.arena(), dom.find_by_id("empty").unwrap()),
            "none"
        );
        let mut graph = SemanticGraph::new(PageId(18), 1);
        graph.reset(dom.arena(), 1);
        let cursor = dom.mutation_cursor();
        let cell = dom.create_element("td");
        dom.append_child(row, cell).unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(computed_role_for_node(dom.arena(), header), "rowheader");
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(18), 1)
        );
    }

    #[test]
    fn html_role_mapping_covers_generic_and_grouping_elements_without_tag_name_fallback() {
        for (tag, role) in [
            ("address", "group"),
            ("fieldset", "group"),
            ("hgroup", "group"),
            ("optgroup", "group"),
            ("dd", "definition"),
            ("dt", "term"),
            ("dfn", "term"),
            ("hr", "separator"),
            ("s", "deletion"),
            ("tbody", "rowgroup"),
            ("b", "generic"),
            ("bdi", "generic"),
            ("q", "generic"),
            ("pre", "generic"),
            ("unknown-element", "generic"),
        ] {
            assert_eq!(
                computed_role(tag, &oryn_dom::AttributeList::default()),
                role
            );
        }
        let dom=crate::html::parse("<ul role=none><li id=orphan>Not a list item</li></ul><div role=list aria-owns=owned></div><li id=owned>Owned</li><ul><div><li id=nested>Nested</li></div></ul>").into_runtime().unwrap();
        for (id, role) in [
            ("orphan", "generic"),
            ("owned", "listitem"),
            ("nested", "listitem"),
        ] {
            assert_eq!(
                computed_role_for_node(dom.arena(), dom.find_by_id(id).unwrap()),
                role
            );
        }
    }

    #[test]
    fn contextual_roles_use_actual_names_and_ancestry_after_reparenting() {
        let parsed = crate::html::parse(
            "<article id=article><div id=container><aside id=aside></aside><header id=header></header></div></article><section id=section aria-labelledby=missing></section><span id=label>Region</span><img id=image alt aria-label=Image><input id=input type=checkbox switch>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let find = |dom: &crate::dom::DomRuntime, id: &str| dom.find_by_id(id).unwrap();
        let aside = find(&dom, "aside");
        let header = find(&dom, "header");
        let section = find(&dom, "section");
        let container = find(&dom, "container");
        let image = find(&dom, "image");
        let input = find(&dom, "input");
        assert_eq!(computed_role_for_node(dom.arena(), aside), "generic");
        assert_eq!(computed_role_for_node(dom.arena(), header), "sectionheader");
        assert_eq!(computed_role_for_node(dom.arena(), section), "generic");
        assert_eq!(computed_role_for_node(dom.arena(), image), "image");
        assert_eq!(computed_role_for_node(dom.arena(), input), "switch");
        let mut graph = SemanticGraph::new(PageId(17), 1);
        graph.reset(dom.arena(), 1);
        for step in 0..4 {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom
                    .set_attribute(section, "aria-labelledby", "label")
                    .unwrap(),
                1 => dom.set_attribute(aside, "title", "Named aside").unwrap(),
                2 => {
                    let document = dom.document();
                    dom.append_child(document, container).unwrap();
                }
                _ => dom.remove_attribute(input, "switch").unwrap(),
            }
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(17), 1)
            );
        }
        assert_eq!(computed_role_for_node(dom.arena(), section), "region");
        assert_eq!(computed_role_for_node(dom.arena(), aside), "complementary");
        assert_eq!(computed_role_for_node(dom.arena(), header), "banner");
        assert_eq!(computed_role_for_node(dom.arena(), input), "checkbox");
    }

    #[test]
    fn figure_caption_names_only_eligible_unnamed_image_and_recomputes_on_sibling_changes() {
        let parsed = crate::html::parse(
            "<figure id=figure><img id=image><figcaption id=caption>Caption</figcaption></figure>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let figure = dom.find_by_id("figure").unwrap();
        let image = dom.find_by_id("image").unwrap();
        let caption = dom.find_by_id("caption").unwrap();
        assert_eq!(accessibility_for_node(dom.arena(), figure).unwrap().1, "");
        assert_eq!(
            accessibility_for_node(dom.arena(), image).unwrap().1,
            "Caption"
        );
        let mut graph = SemanticGraph::new(PageId(16), 1);
        graph.reset(dom.arena(), 1);
        for (step, expected) in ["Updated", "Title", "Updated", ""].into_iter().enumerate() {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.set_text_content(caption, "Updated").unwrap(),
                1 => dom.set_attribute(image, "title", "Title").unwrap(),
                2 => dom.remove_attribute(image, "title").unwrap(),
                _ => {
                    let text = dom.create_text("Other flow content");
                    dom.append_child(figure, text).unwrap();
                }
            }
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(16), 1)
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), image).unwrap().1,
                expected
            );
        }
    }

    #[test]
    fn aria_checked_and_pressed_defaults_follow_role_and_live_mutations() {
        let parsed = crate::html::parse(
            "<div id=control role=checkbox></div><button id=button></button><input id=native type=checkbox checked aria-checked=false>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let control = dom.find_by_id("control").unwrap();
        let button = dom.find_by_id("button").unwrap();
        let native = dom.find_by_id("native").unwrap();
        let properties = |dom: &crate::dom::DomRuntime, id| {
            let node = dom.arena().get(id).unwrap();
            let NodeKind::Element { name } = &node.kind else {
                panic!()
            };
            accessibility_properties(dom.arena(), id, name.local.as_ref(), node)
        };
        assert_eq!(
            properties(&dom, control)["checked"],
            serde_json::json!(false)
        );
        assert_eq!(
            properties(&dom, button)["pressed"],
            serde_json::json!("undefined")
        );
        assert_eq!(properties(&dom, native)["checked"], serde_json::json!(true));
        let mut graph = SemanticGraph::new(PageId(15), 1);
        graph.reset(dom.arena(), 1);
        for (role, value, expected) in [
            ("checkbox", "mixed", serde_json::json!("mixed")),
            ("radio", "mixed", serde_json::json!(false)),
            ("switch", "true", serde_json::json!(true)),
            ("checkbox", "garbage", serde_json::json!(false)),
        ] {
            let cursor = dom.mutation_cursor();
            dom.set_attribute(control, "role", role).unwrap();
            dom.set_attribute(control, "aria-checked", value).unwrap();
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(15), 1)
            );
            assert_eq!(properties(&dom, control)["checked"], expected);
        }
    }

    #[test]
    fn hidden_owner_does_not_reserve_a_visible_owners_target() {
        let parsed = crate::html::parse(
            "<span id=hidden hidden aria-owns=target></span><button id=owner aria-owns=target></button><div aria-hidden=true><span id=target>Visible owned name</span></div>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let find = |dom: &crate::dom::DomRuntime, id: &str| {
            dom.query_selector(dom.document(), &format!("#{id}"))
                .unwrap()
                .unwrap()
        };
        let owner = find(&dom, "owner");
        let target = find(&dom, "target");
        let hidden = find(&dom, "hidden");
        assert_eq!(aria_owner(dom.arena(), target), Some(owner));
        assert_eq!(
            accessibility_for_node(dom.arena(), owner).unwrap().1,
            "Visible owned name"
        );
        let mut graph = SemanticGraph::new(PageId(24), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(
            graph.accessibility[&target].owned_by,
            Some(semantic_ref_for(owner, PageId(24), 1))
        );
        assert!(
            !graph.accessibility[&hidden]
                .relationships
                .get(&AccessibilityRelation::Owns)
                .is_some_and(|targets| targets.contains(&semantic_ref_for(target, PageId(24), 1)))
        );
        dom.set_attribute(hidden, "aria-owns", "").unwrap();
        assert_eq!(aria_owner(dom.arena(), target), Some(owner));
    }

    #[test]
    fn aria_ownership_names_and_published_edges_share_document_order_after_reparenting() {
        let mut dom=crate::html::parse("<main id=root><button id=first aria-owns=target></button><button id=second aria-owns=target></button><span id=target>Owned</span></main>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        let first = dom.find_by_id("first").unwrap();
        let second = dom.find_by_id("second").unwrap();
        let target = dom.find_by_id("target").unwrap();
        let mut graph = SemanticGraph::new(PageId(25), 1);
        graph.reset(dom.arena(), 1);
        for step in 0..3 {
            let expected = if step == 0 {
                first
            } else if step == 1 {
                second
            } else {
                first
            };
            assert_eq!(aria_owner(dom.arena(), target), Some(expected));
            assert_eq!(
                graph.accessibility[&target].owned_by,
                Some(semantic_ref_for(expected, PageId(25), 1))
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), expected).unwrap().1,
                "Owned"
            );
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.append_child(root, first).unwrap(),
                1 => dom.set_attribute(second, "hidden", "").unwrap(),
                _ => continue,
            };
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(25), 1)
            );
        }
        let accepted = accepted_aria_owners(dom.arena());
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[&target], first);
    }

    #[test]
    fn aria_ownership_rejects_cycles_with_interleaved_dom_and_explicit_edges() {
        let mut dom=crate::html::parse("<div id=a role=group aria-owns=b></div><div id=b role=group><div id=c role=group aria-owns=a></div></div>").into_runtime().unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let c = dom.find_by_id("c").unwrap();
        let mut graph = SemanticGraph::new(PageId(26), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(aria_owner(dom.arena(), b), Some(a));
        assert_eq!(aria_owner(dom.arena(), a), None);
        assert_eq!(graph.accessibility[&a].owned_by, None);
        let cursor = dom.mutation_cursor();
        dom.set_attribute(a, "aria-owns", "").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(aria_owner(dom.arena(), a), Some(c));
        assert_eq!(
            graph.accessibility[&a].owned_by,
            Some(semantic_ref_for(c, PageId(26), 1))
        );
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(26), 1)
        );
    }

    #[test]
    fn competing_ownership_changes_refresh_old_new_names_and_external_references() {
        let mut dom = crate::html::parse("<main id=root><button id=first aria-owns=target></button><button id=second aria-owns=target></button><span id=target>Owned</span><button id=reference aria-labelledby=second></button><button id=unrelated>Stable</button></main>").into_runtime().unwrap();
        let root = dom.find_by_id("root").unwrap();
        let first = dom.find_by_id("first").unwrap();
        let second = dom.find_by_id("second").unwrap();
        let reference = dom.find_by_id("reference").unwrap();
        let unrelated = dom.find_by_id("unrelated").unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 1);
        graph.reset(dom.arena(), 1);
        graph.take_changes();
        for hidden in [false, true, false] {
            let cursor = dom.mutation_cursor();
            if hidden {
                dom.set_attribute(second, "hidden", "").unwrap();
            } else if dom
                .arena()
                .get(second)
                .unwrap()
                .attributes
                .contains_null("hidden")
            {
                dom.remove_attribute(second, "hidden").unwrap();
            } else {
                dom.append_child(root, first).unwrap();
            }
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(31), 1)
            );
            assert_eq!(
                graph.full[&reference].name,
                if hidden { "" } else { "Owned" }
            );
            assert_eq!(graph.full[&first].name, if hidden { "Owned" } else { "" });
            assert!(
                !graph
                    .take_changes()
                    .upserted
                    .iter()
                    .any(|node| node.semantic_ref == semantic_ref_for(unrelated, PageId(31), 1))
            );
        }
    }

    #[test]
    fn svg_direct_title_description_and_aria_precedence_share_name_callers() {
        let mut dom=crate::html::parse("<svg id=graphic><title id=title>Graphic name</title><desc>Graphic details</desc><g><title>Nested title</title></g></svg><button id=button aria-labelledby=graphic></button>").into_runtime().unwrap();
        let graphic = dom.find_by_id("graphic").unwrap();
        let title = dom.find_by_id("title").unwrap();
        let button = dom.find_by_id("button").unwrap();
        let mut graph = SemanticGraph::new(PageId(28), 1);
        graph.reset(dom.arena(), 1);
        for (step, expected) in [(0, "Graphic name"), (1, "Changed name"), (2, "ARIA name")] {
            assert_eq!(
                accessibility_for_node(dom.arena(), graphic).unwrap().1,
                expected
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), button).unwrap().1,
                expected
            );
            let semantics =
                accessible_semantics(dom.arena(), graphic, "svg", "generic", PageId(28), 1);
            assert_eq!(semantics.description.as_deref(), Some("Graphic details"));
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.set_text_content(title, "Changed name").unwrap(),
                1 => dom
                    .set_attribute(graphic, "aria-label", "ARIA name")
                    .unwrap(),
                _ => continue,
            };
            graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(28), 1)
            );
        }
        dom.set_attribute(graphic, "aria-description", "ARIA details")
            .unwrap();
        assert_eq!(
            accessible_semantics(dom.arena(), graphic, "svg", "generic", PageId(28), 1)
                .description
                .as_deref(),
            Some("ARIA details")
        );
    }

    #[test]
    fn ownership_is_shared_by_selection_readonly_and_modal_callers() {
        let mut dom=crate::html::parse("<div id=first role=listbox aria-owns=option></div><div id=second role=listbox aria-owns=option></div><div id=option role=option aria-selected=true>Choice</div><div id=grid role=grid aria-readonly=true aria-owns=cell></div><div id=cell role=gridcell>Value</div><div id=modal role=dialog aria-modal=true aria-owns=owned></div><button id=owned>Owned button</button>").into_runtime().unwrap();
        let first = dom.find_by_id("first").unwrap();
        let second = dom.find_by_id("second").unwrap();
        let option = dom.find_by_id("option").unwrap();
        let grid = dom.find_by_id("grid").unwrap();
        let cell = dom.find_by_id("cell").unwrap();
        let modal = dom.find_by_id("modal").unwrap();
        let owned = dom.find_by_id("owned").unwrap();
        assert_eq!(native_selection_options(dom.arena(), first), vec![option]);
        assert!(native_selection_options(dom.arena(), second).is_empty());
        assert_eq!(embedded_selected_options(dom.arena(), first), vec![option]);
        assert!(embedded_selected_options(dom.arena(), second).is_empty());
        assert!(accessibility_readonly(dom.arena(), cell));
        assert_eq!(tree_exclusion_reason(dom.arena(), owned, "button"), None);
        let mut graph = SemanticGraph::new(PageId(29), 1);
        graph.reset(dom.arena(), 1);
        let cursor = dom.mutation_cursor();
        dom.set_attribute(first, "aria-owns", "").unwrap();
        dom.set_attribute(grid, "aria-owns", "").unwrap();
        dom.set_attribute(modal, "aria-owns", "").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(native_selection_options(dom.arena(), second), vec![option]);
        assert!(!accessibility_readonly(dom.arena(), cell));
        assert_eq!(
            tree_exclusion_reason(dom.arena(), owned, "button").as_deref(),
            Some("aria_modal_scope")
        );
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(29), 1)
        );
    }

    #[test]
    fn only_html_slots_distribute_light_content_or_ignore_aria_labels() {
        let parsed = crate::html::parse(
            "<button id=host><span slot=x>Assigned</span></button><div id=shadow><svg><slot id=foreign name=x aria-label='Foreign label'>Foreign fallback</slot></svg><slot name=x>HTML fallback</slot></div>",
        );
        let find = |id: &str| {
            parsed
                .dom
                .iter()
                .find(|(_, node)| {
                    node.attributes
                        .get_null("id")
                        .is_some_and(|value| value == id)
                })
                .map(|(id, _)| id)
                .unwrap()
        };
        let host = find("host");
        let shadow = find("shadow");
        let foreign = find("foreign");
        let mut runtime = parsed.into_runtime().unwrap();
        attach_test_shadow(&mut runtime, host, shadow);
        assert_eq!(
            accessible_text_content(runtime.arena(), foreign, false),
            "Foreign label"
        );
        let children = accessible_children(runtime.arena(), foreign);
        assert!(children.iter().all(|id| {
            runtime
                .arena()
                .get(*id)
                .is_ok_and(|node| !node.attributes.contains_null("slot"))
        }));
        assert!(
            accessibility_for_node(runtime.arena(), host)
                .unwrap()
                .1
                .contains("Assigned")
        );
    }

    #[test]
    fn aria_ownership_rejects_sibling_reference_cycles_after_mutation() {
        let parsed = crate::html::parse(
            "<div id=a role=group aria-owns=b></div><div id=b role=group aria-owns=c></div><div id=c role=group></div>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let c = dom.query_selector(dom.document(), "#c").unwrap().unwrap();
        let mut graph = SemanticGraph::new(PageId(14), 1);
        graph.reset(dom.arena(), 1);
        let cursor = dom.mutation_cursor();
        dom.set_attribute(c, "aria-owns", "a").unwrap();
        graph.apply_mutations(dom.arena(), &dom.mutations_since(cursor).mutations);
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(14), 1)
        );
        let mut owners = BTreeMap::new();
        for node in graph.accessibility.values() {
            if let Some(owner) = node.owned_by {
                owners.insert(node.semantic_ref, owner);
            }
        }
        assert_eq!(owners.len(), 2);
        for (target, owner) in &owners {
            let mut without = owners.clone();
            without.remove(target);
            assert!(!ownership_cycle(&without, *owner, *target));
        }
    }

    #[test]
    fn manual_slot_names_and_reference_dependents_match_full_projection() {
        let mut parsed = crate::html::parse(
            "<button id=host><span id=a>Alpha</span><span id=b>Beta</span></button><div id=shadow><slot id=slot>Fallback</slot></div><button id=ref aria-labelledby=host></button>",
        );
        let find = |value: &str| {
            parsed
                .dom
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").is_some_and(|id| id == value))
                .map(|(id, _)| id)
                .unwrap()
        };
        let host = find("host");
        let shadow = find("shadow");
        let slot = find("slot");
        let a = find("a");
        let b = find("b");
        let referenced = find("ref");
        parsed.dom.get_mut(shadow).unwrap().kind = NodeKind::ShadowRoot {
            mode: crate::dom::ShadowMode::Open,
            clonable: false,
        };
        parsed.dom.append_child(host, shadow).unwrap();
        let mut dom = crate::dom::DomRuntime::from_parsed(parsed.dom, parsed.document).unwrap();
        let mut graph = SemanticGraph::new(PageId(13), 1);
        graph.reset(dom.arena(), 1);
        for (step, expected) in ["Fallback", "BetaAlpha", "Alpha", "Fallback"]
            .into_iter()
            .enumerate()
        {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.set_manual_slot_assignment(shadow).unwrap(),
                1 => dom.assign_slot_nodes(slot, &[b, a, b]).unwrap(),
                2 => dom.assign_slot_nodes(slot, &[a]).unwrap(),
                _ => dom.assign_slot_nodes(slot, &[]).unwrap(),
            }
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(13), 1)
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), host).unwrap().1,
                expected
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), referenced).unwrap().1,
                expected
            );
        }
    }

    #[test]
    fn accessible_names_exclude_source_text_inside_shadow_roots() {
        let document = crate::html::parse(
            r#"<button id=host>light</button><div id=shadow><style aria-label=css-leak>.secret { color:red }</style><script>source_leak()</script><template>template leak</template><span>Save</span><slot></slot></div>"#,
        );
        let mut runtime = document.into_runtime().unwrap();
        let find = |value: &str| {
            runtime
                .arena()
                .iter()
                .find(|(_, node)| node.attributes.get_null("id").is_some_and(|id| id == value))
                .map(|(id, _)| id)
                .unwrap()
        };
        let host = find("host");
        let shadow = find("shadow");
        let shadow = attach_test_shadow(&mut runtime, host, shadow);
        assert_eq!(
            accessibility_for_node(runtime.arena(), host).unwrap().1,
            "Savelight"
        );
        // This is projection filtering, not destructive DOM sanitization.
        assert!(
            runtime
                .arena()
                .text_content(shadow)
                .unwrap()
                .contains("source_leak()")
        );
    }

    #[test]
    fn embedded_textarea_and_owned_listbox_values_update_accessible_names() {
        let parsed = crate::html::parse(
            "<label>Text <input id=text-target type=checkbox><textarea id=edit aria-label=Editor>Default</textarea></label><label>Pick <input id=pick-target type=checkbox><span id=list role=listbox aria-label=Choices aria-owns='external'><span id=local role=option aria-selected=true>Local</span><span role=listbox><span role=option aria-selected=true>Nested</span></span></span></label><span id=external role=option aria-selected=true>External</span>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let node = |dom: &oryn_dom::DomRuntime, id: &str| {
            dom.query_selector(dom.document(), &format!("#{id}"))
                .unwrap()
                .unwrap()
        };
        let edit = node(&dom, "edit");
        let text = node(&dom, "text-target");
        let pick = node(&dom, "pick-target");
        let local = node(&dom, "local");
        let external = node(&dom, "external");
        assert_eq!(
            accessibility_for_node(dom.arena(), text).unwrap().1,
            "Text Default"
        );
        assert_eq!(
            accessibility_for_node(dom.arena(), pick).unwrap().1,
            "Pick Local External"
        );
        let mut graph = SemanticGraph::new(PageId(12), 1);
        graph.reset(dom.arena(), 1);
        for step in 0..4 {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.set_value(edit, "Live\r\nvalue", false).unwrap(),
                1 => dom.set_text_content(edit, "Changed default").unwrap(),
                2 => dom.set_attribute(local, "aria-selected", "false").unwrap(),
                _ => dom
                    .set_attribute(external, "aria-selected", "false")
                    .unwrap(),
            }
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(12), 1)
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), text).unwrap().1,
                "Text Live value"
            );
            assert_eq!(
                accessibility_for_node(dom.arena(), pick).unwrap().1,
                match step {
                    0 | 1 => "Pick Local External",
                    2 => "Pick External",
                    _ => "Pick",
                }
            );
        }
    }

    #[test]
    fn scoped_reference_lookup_follows_tree_order_after_reparenting() {
        let parsed = crate::html::parse(
            "<div id=container><span id=duplicate>First</span><span id=duplicate>Second</span><button id=source aria-labelledby=duplicate></button></div>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let container = dom
            .query_selector(dom.document(), "#container")
            .unwrap()
            .unwrap();
        let source = dom
            .query_selector(dom.document(), "#source")
            .unwrap()
            .unwrap();
        let first = find_element_by_id_scoped(dom.arena(), source, "duplicate").unwrap();
        assert_eq!(
            accessibility_for_node(dom.arena(), source).unwrap().1,
            "First"
        );
        dom.append_child(container, first).unwrap();
        let second = find_element_by_id_scoped(dom.arena(), source, "duplicate").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            accessibility_for_node(dom.arena(), source).unwrap().1,
            "Second"
        );
        assert_eq!(
            dom.arena()
                .accessibility_relation(source, "aria-labelledby"),
            Some(vec![second])
        );
    }

    #[test]
    fn embedded_select_names_follow_native_grouped_and_multiple_selection() {
        let parsed = crate::html::parse(
            "<label>Choose <input id=target type=checkbox><select id=choice><optgroup label=Group><option id=a value=a selected>Alpha</option><option id=b value=b label=Beta>Old text</option></optgroup><option id=c value=c>Gamma</option></select></label>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let target = dom
            .query_selector(dom.document(), "#target")
            .unwrap()
            .unwrap();
        let select = dom
            .query_selector(dom.document(), "#choice")
            .unwrap()
            .unwrap();
        let a = dom.query_selector(dom.document(), "#a").unwrap().unwrap();
        let c = dom.query_selector(dom.document(), "#c").unwrap().unwrap();
        let name =
            |dom: &oryn_dom::DomRuntime| accessibility_for_node(dom.arena(), target).unwrap().1;
        assert_eq!(name(&dom), "Choose Alpha");
        let mut graph = SemanticGraph::new(PageId(10), 1);
        graph.reset(dom.arena(), 1);
        for step in 0..3 {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.set_select_value(select, "b").unwrap(),
                1 => dom.set_select_selected_index(select, -1).unwrap(),
                _ => {
                    dom.set_attribute(select, "multiple", "").unwrap();
                    dom.set_option_selected(a, true).unwrap();
                    dom.set_option_selected(c, true).unwrap();
                }
            }
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(10), 1)
            );
            assert_eq!(
                name(&dom),
                match step {
                    0 => "Choose Beta",
                    1 => "Choose",
                    _ => "Choose Alpha Gamma",
                }
            );
        }
        // Authored defaults still differ from native selectedness.
        assert!(
            dom.arena()
                .get(a)
                .unwrap()
                .attributes
                .contains_null("selected")
        );
        assert!(
            !dom.arena()
                .get(c)
                .unwrap()
                .attributes
                .contains_null("selected")
        );
    }

    #[test]
    fn incremental_label_ownership_tracks_native_upgrade_and_input_type_changes() {
        let parsed =
            crate::html::parse("<label>Owner<x-field id=custom></x-field><input id=field></label>");
        let mut dom = parsed.into_runtime().unwrap();
        let custom = dom
            .query_selector(dom.document(), "#custom")
            .unwrap()
            .unwrap();
        let field = dom
            .query_selector(dom.document(), "#field")
            .unwrap()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(9), 1);
        graph.reset(dom.arena(), 1);
        for step in 0..4 {
            let cursor = dom.mutation_cursor();
            match step {
                0 => dom.register_form_associated_custom_element(custom).unwrap(),
                1 => dom.unregister_form_associated_custom_element(custom),
                2 => dom.set_attribute(field, "type", "hidden").unwrap(),
                _ => dom.set_attribute(field, "type", "text").unwrap(),
            }
            let batch = dom.mutations_since(cursor);
            assert!(!batch.truncated);
            graph.apply_mutations(dom.arena(), &batch.mutations);
            assert_eq!(
                graph.nodes(ProjectionProfile::Compact),
                project_document(dom.arena(), PageId(9), 1)
            );
            let owner = if step == 0 {
                Some(custom)
            } else if step == 2 {
                None
            } else {
                Some(field)
            };
            for node in [custom, field] {
                assert_eq!(
                    host_label(dom.arena(), node).map(|(name, _)| name),
                    (owner == Some(node)).then(|| "Owner".to_string())
                );
            }
        }
    }

    #[test]
    fn native_label_ownership_is_shared_with_semantics_and_snapshot_projection() {
        let parsed = crate::html::parse(
            r#"<label for=field>Explicit</label><label id=implicit>Implicit<input id=field><input id=second></label>
            <label for=other>Wrong<input id=nested></label><input id=other>
            <div id=duplicate></div><input id=duplicate><label for=duplicate>Must not attach</label>
            <label for=empty></label><input id=empty title=Fallback>
            <label id=custom-label>Custom<x-field id=custom></x-field><input id=later></label>"#,
        );
        let mut dom = parsed.into_runtime().unwrap();
        let by_id = |dom: &oryn_dom::DomRuntime, id: &str| {
            dom.query_selector(dom.document(), &format!("#{id}"))
                .unwrap()
                .unwrap()
        };
        let field = by_id(&dom, "field");
        assert_eq!(
            host_label(dom.arena(), field).unwrap().0,
            "Explicit Implicit"
        );
        let label_sources = host_label(dom.arena(), field).unwrap().1;
        assert_eq!(label_sources.len(), 2);
        let accessible = accessible_semantics(dom.arena(), field, "input", "textbox", PageId(1), 1);
        let evidence = accessible
            .evidence
            .iter()
            .find(|item| item.source == "label")
            .unwrap();
        assert_eq!(
            evidence.references,
            label_sources
                .into_iter()
                .map(|id| semantic_ref_for(id, PageId(1), 1))
                .collect::<Vec<_>>()
        );
        assert!(host_label(dom.arena(), by_id(&dom, "second")).is_none());
        assert!(host_label(dom.arena(), by_id(&dom, "nested")).is_none());
        for (id, record) in dom.arena().iter() {
            if record
                .attributes
                .get_null("id")
                .is_some_and(|value| value == "duplicate")
            {
                assert!(host_label(dom.arena(), id).is_none());
            }
        }
        assert_eq!(
            accessibility_for_node(dom.arena(), by_id(&dom, "empty"))
                .unwrap()
                .1,
            ""
        );
        let custom = by_id(&dom, "custom");
        let later = by_id(&dom, "later");
        let label = by_id(&dom, "custom-label");
        assert_eq!(dom.label_control(label).unwrap(), Some(later));
        dom.register_form_associated_custom_element(custom).unwrap();
        assert_eq!(dom.label_control(label).unwrap(), Some(custom));
        let snapshot = dom.arena().clone();
        assert_eq!(snapshot.label_control(label).unwrap(), Some(custom));
        assert_eq!(host_label(&snapshot, custom).unwrap().0, "Custom");
        assert!(host_label(&snapshot, later).is_none());
        dom.unregister_form_associated_custom_element(custom);
        assert_eq!(dom.label_control(label).unwrap(), Some(later));
        assert_eq!(snapshot.label_control(label).unwrap(), Some(custom));
    }

    #[test]
    fn accessible_description_uses_tooltip_source_instead_of_string_inequality() {
        let document = crate::html::parse(
            r#"<input id=aria aria-label=Same title=Same>
            <label for=labelled>Same</label><input id=labelled title=Same>
            <input id=tooltip title=Same placeholder="typing hint">
            <input id=placeholder aria-label=Name placeholder="typing hint">
            <input id=explicit aria-label=Name title=Tooltip aria-description=Explicit>
            <span id=detail>Referenced</span>
            <input id=referenced aria-label=Name title=Tooltip aria-description=Explicit aria-describedby=detail>"#,
        );
        let semantics = |id: &str| {
            let node = document
                .dom
                .iter()
                .find(|(_, node)| {
                    node.attributes
                        .get_null("id")
                        .is_some_and(|value| value == id)
                })
                .map(|(id, _)| id)
                .expect("fixture element");
            accessible_semantics(&document.dom, node, "input", "textbox", PageId(1), 1)
        };
        for id in ["aria", "labelled"] {
            let accessible = semantics(id);
            assert_eq!(accessible.name, "Same");
            assert_eq!(accessible.description.as_deref(), Some("Same"));
            assert!(
                accessible
                    .evidence
                    .iter()
                    .any(|item| item.source == "title-description")
            );
        }
        let tooltip = semantics("tooltip");
        assert_eq!(tooltip.name, "Same");
        assert_eq!(tooltip.description, None);
        assert_eq!(semantics("placeholder").description, None);
        assert_eq!(
            semantics("explicit").description.as_deref(),
            Some("Explicit")
        );
        assert_eq!(
            semantics("referenced").description.as_deref(),
            Some("Referenced")
        );
    }

    #[test]
    fn accessible_reference_precedence_distinguishes_empty_text_from_missing_targets() {
        let document = crate::html::parse(
            r#"<span id=empty></span>
            <button id=valid aria-labelledby="absent empty" aria-label="fallback"
                aria-describedby=empty aria-description="description fallback">content</button>
            <button id=missing aria-labelledby=absent aria-label="fallback"
                aria-describedby=absent aria-description="description fallback">content</button>
            <button id=blank aria-labelledby=" " aria-label="fallback"
                aria-describedby=" " aria-description="description fallback">content</button>
            <span id=whitespace aria-label=" ">referenced content</span>
            <button id=whitespace-reference aria-labelledby=whitespace></button>"#,
        );
        let semantics = |id: &str| {
            let node = document
                .dom
                .iter()
                .find(|(_, node)| {
                    node.attributes
                        .get_null("id")
                        .is_some_and(|value| value == id)
                })
                .map(|(id, _)| id)
                .expect("fixture element");
            accessible_semantics(&document.dom, node, "button", "button", PageId(1), 1)
        };
        let valid = semantics("valid");
        assert_eq!(valid.name, "");
        assert_eq!(valid.description, None);
        let empty = document
            .dom
            .iter()
            .find(|(_, node)| {
                node.attributes
                    .get_null("id")
                    .is_some_and(|value| value == "empty")
            })
            .unwrap()
            .0;
        assert!(
            valid
                .evidence
                .iter()
                .any(|item| item.source == "aria-labelledby"
                    && item.references == vec![semantic_ref_for(empty, PageId(1), 1)])
        );
        assert_eq!(semantics("whitespace-reference").name, "referenced content");
        for id in ["missing", "blank"] {
            let fallback = semantics(id);
            assert_eq!(fallback.name, "fallback");
            assert_eq!(
                fallback.description.as_deref(),
                Some("description fallback")
            );
        }
    }

    #[test]
    fn directly_referenced_hidden_subtree_keeps_hidden_descendants_through_visible_wrappers() {
        let document = crate::html::parse(
            r#"
            <span id=hidden-root hidden><span><span aria-hidden=true>deep</span></span><span> tail</span></span>
            <span id=visible-root><span><span aria-hidden=true>deep</span></span><span> tail</span></span>
            <button id=hidden-name aria-labelledby=hidden-root></button>
            <button id=visible-name aria-labelledby=visible-root></button>
            <button id=ordinary><span hidden><span>deep</span></span>tail</button>
            <div hidden><span id=inherited-root><span><span hidden>inherited</span></span></span></div>
            <button id=inherited-name aria-labelledby=inherited-root></button>
            <div style="visibility:hidden"><span id=override-root style="visibility:visible"><span hidden>skip</span>shown</span></div>
            <button id=override-name aria-labelledby=override-root></button>
        "#,
        );
        for (id, expected) in [
            ("hidden-name", "deep tail"),
            ("visible-name", "tail"),
            ("ordinary", "tail"),
            ("inherited-name", "inherited"),
            ("override-name", "shown"),
        ] {
            let node = document
                .dom
                .iter()
                .find(|(_, node)| {
                    node.attributes
                        .get_null("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap()
                .0;
            assert_eq!(
                accessible_semantics(&document.dom, node, "button", "button", PageId(1), 1).name,
                expected,
                "{id}"
            );
        }
    }

    #[test]
    fn accessible_name_honors_hidden_content_empty_alt_and_inline_boundaries() {
        let document = crate::html::parse(
            r#"
            <button id=hidden><span hidden>skip</span><span>keep</span></button>
            <label id=explicit-descendant><div><span aria-label="spoken">visual</span></div></label>
            <button id=labelled-descendant aria-labelledby=explicit-descendant></button>
            <button id=labelled-content><span aria-label="spoken">visual</span><span aria-hidden=true aria-label="hidden">skip</span></button>
            <label>Flash <input id=embedded-checkbox type=checkbox><input aria-label="number of times" value=3> times</label>
            <h2 id=visibility>visible <span style="visibility:hidden">hidden <span style="visibility:visible">restored</span></span></h2>
            <button id=actions aria-actions="edit close">Save <span id=edit>Edit</span><span id=close>Close</span></button>
            <div id=article role=article><div role=group><h3>First heading</h3></div><h2>Later heading</h2></div>
            <span id=joined role=button>button<span></span>label</span>
            <img id=empty-alt alt="" title="must not win">
            <div id=dialog role=alertdialog><h2>Dialog heading</h2><p>Body copy</p></div>
            "#,
        );
        let named = |id: &str| {
            let node = document
                .dom
                .iter()
                .find(|(_, node)| {
                    node.attributes
                        .get_null("id")
                        .is_some_and(|value| value == id)
                })
                .map(|(node, _)| node)
                .expect("fixture id");
            accessibility_for_node(&document.dom, node).expect("accessibility projection")
        };
        assert_eq!(named("hidden"), ("button".into(), "keep".into()));
        assert_eq!(
            named("embedded-checkbox"),
            ("checkbox".into(), "Flash 3 times".into())
        );
        assert_eq!(
            named("labelled-descendant"),
            ("button".into(), "spoken".into())
        );
        assert_eq!(
            named("labelled-content"),
            ("button".into(), "spoken".into())
        );
        assert_eq!(
            named("visibility"),
            ("heading".into(), "visible restored".into())
        );
        assert_eq!(named("actions"), ("button".into(), "Save".into()));
        assert_eq!(named("article"), ("article".into(), "First heading".into()));
        assert_eq!(named("joined"), ("button".into(), "buttonlabel".into()));
        // The role surface canonicalizes the presentation synonym to `none`.
        assert_eq!(named("empty-alt"), ("none".into(), String::new()));
        assert_eq!(
            named("dialog"),
            ("alertdialog".into(), "Dialog heading".into())
        );
    }

    #[test]
    fn structures_cover_agent_relevant_composites_and_landmarks() {
        let observation = semantic_observation(
            r#"
            <nav aria-label="Primary"><ul><li><a href=/home>Home</a></li></ul></nav>
            <table><caption>Orders</caption><tr><th>Item</th><td>Book</td></tr></table>
            <div role=tablist aria-label="Settings"><button role=tab aria-selected=true aria-controls=panel>Profile</button></div>
            <section id=panel role=tabpanel aria-label="Profile panel"></section>
            <div role=menu aria-label="Actions"><button role=menuitem>Save</button></div>
            <div role=tree aria-label="Files"><div role=treeitem>src</div></div>
            <div role=grid aria-label="Metrics"><div role=row><span role=gridcell>42</span></div></div>
            <dialog aria-label="Confirm"></dialog>
            <dl aria-label="Properties"><dt>Color</dt><dd>Blue</dd></dl>
            <main aria-label="Workspace"></main>
            <div role=status aria-live=polite>Saved</div>
            "#,
        );
        for expected in [
            SemanticStructureKind::Navigation,
            SemanticStructureKind::List,
            SemanticStructureKind::Table,
            SemanticStructureKind::TabSet,
            SemanticStructureKind::Menu,
            SemanticStructureKind::Tree,
            SemanticStructureKind::Grid,
            SemanticStructureKind::Dialog,
            SemanticStructureKind::RepeatedCollection,
            SemanticStructureKind::Landmark,
            SemanticStructureKind::LiveRegion,
        ] {
            assert!(
                observation
                    .structures
                    .iter()
                    .any(|structure| structure.kind == expected),
                "missing {expected:?}"
            );
        }
        let table = observation
            .structures
            .iter()
            .find(|structure| structure.kind == SemanticStructureKind::Table)
            .unwrap();
        assert_eq!(table.name, "Orders");
        assert_eq!(table.groups.len(), 1);
        assert_eq!(table.groups[0].kind, "row");
        assert_eq!(table.groups[0].members.len(), 3);
        let tab = observation
            .accessibility
            .iter()
            .find(|node| node.role == "tab")
            .unwrap();
        assert_eq!(
            tab.properties.get("selected"),
            Some(&serde_json::json!(true))
        );
        assert!(
            tab.relationships
                .contains_key(&AccessibilityRelation::Controls)
        );
    }

    #[test]
    fn tree_exclusion_and_aria_ownership_are_conservative() {
        let observation = semantic_observation(
            r#"
            <div id=owner role=list aria-owns="item missing"><span>Owner</span></div>
            <div id=item role=listitem>Owned item</div>
            <div inert><button>Unavailable</button></div>
            <div id=cycle role=group aria-owns=cycle>Cycle</div>
            "#,
        );
        let owner = observation
            .accessibility
            .iter()
            .find(|node| node.role == "list")
            .unwrap();
        let item = observation
            .accessibility
            .iter()
            .find(|node| node.role == "listitem")
            .unwrap();
        assert_eq!(item.owned_by, Some(owner.semantic_ref));
        let unavailable = observation
            .accessibility
            .iter()
            .find(|node| node.name == "Unavailable")
            .unwrap();
        assert!(unavailable.excluded);
        assert_eq!(
            unavailable.exclusion_reason.as_deref(),
            Some("inert_subtree")
        );
        assert!(
            observation
                .diagnostics
                .iter()
                .any(|item| item.contains("missing id"))
        );
        assert!(
            observation
                .diagnostics
                .iter()
                .any(|item| item.contains("aria-owns cycle"))
        );
    }
}

#[cfg(test)]
mod internals_accessibility_tests {
    use super::*;
    #[test]
    fn default_role_name_and_native_relations_obey_author_precedence() {
        let mut dom=crate::html::parse("<x-ui id=x></x-ui><span id=label>Native label</span><span id=other>Author label</span>").into_runtime().unwrap();
        let document = dom.document();
        let x = dom.get_element_by_id(document, "x").unwrap().unwrap();
        let label = dom.get_element_by_id(document, "label").unwrap().unwrap();
        dom.set_default_accessibility_attribute(x, "role", Some("button".into()))
            .unwrap();
        dom.set_default_accessibility_attribute(x, "aria-label", Some("Fallback".into()))
            .unwrap();
        assert_eq!(
            accessibility_for_node(dom.arena(), x),
            Some(("button".into(), "Fallback".into()))
        );
        assert!(
            dom.attribute_in_null_namespace(x, "aria-label")
                .unwrap()
                .is_none()
        );
        dom.set_accessibility_references(x, "aria-labelledby", true, Some(vec![label]))
            .unwrap();
        assert_eq!(
            accessibility_for_node(dom.arena(), x).unwrap().1,
            "Native label"
        );
        dom.set_attribute(x, "aria-labelledby", "other").unwrap();
        dom.set_attribute(x, "role", "link").unwrap();
        assert_eq!(
            accessibility_for_node(dom.arena(), x),
            Some(("link".into(), "Author label".into()))
        );
        dom.remove_attribute(x, "aria-labelledby").unwrap();
        assert_eq!(
            accessibility_for_node(dom.arena(), x).unwrap().1,
            "Native label"
        );
        let mut graph = SemanticGraph::new(PageId(15), 1);
        graph.reset(dom.arena(), 1);
        let _ = graph.take_v6_changes();
        let cursor = dom.mutation_cursor();
        dom.set_text_content(label, "Changed label").unwrap();
        let batch = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &batch.mutations);
        let changes = graph.take_v6_changes();
        assert!(
            changes
                .upserted_accessibility
                .iter()
                .any(
                    |node| node.semantic_ref == semantic_ref_for(x, PageId(15), 1)
                        && node.name == "Changed label"
                )
        );
    }
    #[test]
    fn native_ownership_cycles_do_not_recurse_and_relations_need_no_ids() {
        let mut dom = crate::html::parse("<div><x-a>A</x-a><x-b>B</x-b></div>")
            .into_runtime()
            .unwrap();
        let a = dom.query_selector(dom.document(), "x-a").unwrap().unwrap();
        let b = dom.query_selector(dom.document(), "x-b").unwrap().unwrap();
        dom.set_default_accessibility_attribute(a, "role", Some("button".into()))
            .unwrap();
        dom.set_default_accessibility_attribute(b, "role", Some("button".into()))
            .unwrap();
        dom.set_accessibility_references(a, "aria-owns", true, Some(vec![b]))
            .unwrap();
        dom.set_accessibility_references(b, "aria-owns", true, Some(vec![a]))
            .unwrap();
        assert!(aria_owner(dom.arena(), a).is_none());
        assert_eq!(aria_owner(dom.arena(), b), Some(a));
        let mut graph = SemanticGraph::new(PageId(27), 1);
        graph.reset(dom.arena(), 1);
        assert_eq!(
            graph.accessibility[&b].owned_by,
            Some(semantic_ref_for(a, PageId(27), 1))
        );
        assert_eq!(
            graph.nodes(ProjectionProfile::Compact),
            project_document(dom.arena(), PageId(27), 1)
        );
        dom.set_accessibility_references(a, "aria-labelledby", true, Some(vec![b]))
            .unwrap();
        assert_eq!(accessibility_for_node(dom.arena(), a).unwrap().1, "B");
    }
    #[cfg(feature = "v8-host")]
    #[test]
    fn native_picker_highlight_updates_active_descendant_without_changing_selection() {
        let mut dom=crate::html::parse("<select id=s aria-activedescendant=a><option id=a selected>A</option><option id=b>B</option></select>").into_runtime().unwrap();
        let select = dom.query_selector(dom.document(), "#s").unwrap().unwrap();
        let a = dom.query_selector(dom.document(), "#a").unwrap().unwrap();
        let b = dom.query_selector(dom.document(), "#b").unwrap().unwrap();
        let mut graph = SemanticGraph::new(PageId(21), 1);
        graph.reset(dom.arena(), 1);
        dom.add_to_top_layer(select, oryn_dom::TopLayerKind::SelectPicker)
            .unwrap();
        dom.set_select_picker_active(select, Some(b)).unwrap();
        graph.apply_native_control_state(&dom).unwrap();
        assert_eq!(
            graph.accessibility[&select].relationships[&AccessibilityRelation::ActiveDescendant],
            vec![semantic_ref_for(b, PageId(21), 1)]
        );
        assert!(
            dom.arena()
                .get(a)
                .unwrap()
                .attributes
                .contains_null("selected")
        );
        assert!(
            !dom.arena()
                .get(b)
                .unwrap()
                .attributes
                .contains_null("selected")
        );
        dom.remove_from_top_layer(select);
        graph.apply_native_control_state(&dom).unwrap();
        assert_eq!(
            graph.accessibility[&select].relationships[&AccessibilityRelation::ActiveDescendant],
            vec![semantic_ref_for(a, PageId(21), 1)]
        );
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod native_custom_control_projection_regressions {
    use super::*;
    #[test]
    fn submit_behavior_projects_role_and_disability_without_authored_attributes() {
        let mut dom = crate::html::parse("<x-submit id=x>Save</x-submit>")
            .into_runtime()
            .unwrap();
        let x = dom.find_by_id("x").unwrap();
        let mut behavior = oryn_dom::SubmitButtonBehavior::default();
        dom.set_submit_behavior(x, behavior.clone()).unwrap();
        let mut graph = SemanticGraph::new(PageId(42), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_native_control_state(&dom).unwrap();
        assert_eq!(graph.full[&x].role, "button");
        assert!(graph.full[&x].validation.is_none());
        assert!(
            dom.attribute_in_null_namespace(x, "role")
                .unwrap()
                .is_none()
        );
        behavior.disabled = true;
        dom.set_submit_behavior(x, behavior).unwrap();
        graph.apply_native_control_state(&dom).unwrap();
        assert!(graph.full[&x].states.contains(&NodeState::Disabled));
        assert_eq!(
            graph.accessibility[&x].properties["disabled"],
            serde_json::json!(true)
        );
        let cursor = dom.mutation_cursor();
        dom.set_default_accessibility_attribute(x, "role", Some("link".into()))
            .unwrap();
        let changes = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &changes.mutations);
        assert_eq!(graph.full[&x].role, "link");
        let cursor = dom.mutation_cursor();
        dom.set_attribute(x, "role", "menuitem").unwrap();
        let changes = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &changes.mutations);
        assert_eq!(graph.full[&x].role, "menuitem");
    }

    #[test]
    fn custom_validity_flags_and_fieldset_disability_reach_oil_and_accessibility() {
        let mut dom = crate::html::parse(
            "<fieldset id=f><x-field id=x role=textbox></x-field><input id=i></fieldset>",
        )
        .into_runtime()
        .unwrap();
        let fieldset = dom.find_by_id("f").unwrap();
        let custom = dom.find_by_id("x").unwrap();
        let input = dom.find_by_id("i").unwrap();
        dom.register_form_associated_custom_element(custom).unwrap();
        dom.set_custom_element_validity(
            custom,
            ["valueMissing".into(), "rangeOverflow".into()]
                .into_iter()
                .collect(),
            "Required".into(),
        )
        .unwrap();
        let mut graph = SemanticGraph::new(PageId(42), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_native_control_state(&dom).unwrap();
        let validation = graph.full[&custom].validation.as_ref().unwrap();
        assert!(validation.will_validate && validation.value_missing && validation.range_overflow);
        assert!(!validation.valid && !validation.custom_error);
        let cursor = dom.mutation_cursor();
        dom.set_attribute(fieldset, "disabled", "").unwrap();
        let mutations = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &mutations.mutations);
        graph.apply_native_control_state(&dom).unwrap();
        for control in [custom, input] {
            assert!(graph.full[&control].states.contains(&NodeState::Disabled));
            assert!(
                !graph.full[&control]
                    .validation
                    .as_ref()
                    .unwrap()
                    .will_validate
            );
            assert_eq!(
                graph.accessibility[&control].properties.get("disabled"),
                Some(&serde_json::Value::Bool(true))
            );
        }
        let cursor = dom.mutation_cursor();
        dom.remove_attribute(fieldset, "disabled").unwrap();
        dom.set_custom_element_validity(custom, BTreeSet::new(), String::new())
            .unwrap();
        let mutations = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &mutations.mutations);
        graph.apply_native_control_state(&dom).unwrap();
        assert!(graph.full[&custom].validation.as_ref().unwrap().valid);
        assert!(!graph.full[&custom].states.contains(&NodeState::Disabled));
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod native_temporal_projection_regressions {
    use super::*;
    #[test]
    fn partial_temporal_edit_updates_oil_bad_input_and_validity() {
        let mut dom = crate::html::parse("<input id=d type=date required>")
            .into_runtime()
            .unwrap();
        let node = dom.find_by_id("d").unwrap();
        let mut graph = SemanticGraph::new(PageId(77), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_native_control_state(&dom).unwrap();
        assert!(!graph.full[&node].validation.as_ref().unwrap().bad_input);
        let cursor = dom.mutation_cursor();
        dom.edit_temporal_control(node, oryn_dom::TemporalAction::Digit(2))
            .unwrap();
        let changes = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &changes.mutations);
        graph.apply_native_control_state(&dom).unwrap();
        let state = graph.full[&node].validation.as_ref().unwrap();
        assert!(state.bad_input && state.value_missing && !state.valid);
        let cursor = dom.mutation_cursor();
        dom.set_user_control_value(node, "2024-02-29", false)
            .unwrap();
        let changes = dom.mutations_since(cursor);
        graph.apply_mutations(dom.arena(), &changes.mutations);
        graph.apply_native_control_state(&dom).unwrap();
        let state = graph.full[&node].validation.as_ref().unwrap();
        assert!(!state.bad_input && !state.value_missing && state.valid);
    }
}

#[cfg(test)]
mod native_control_action_inventory_tests {
    use super::*;
    #[test]
    fn native_type_case_and_readonly_applicability_match_advertised_operations() {
        let dom=crate::html::parse("<input id=r type=RANGE readonly><input id=c type=COLOR readonly><input id=f type=FILE><input id=b type=CHECKBOX>").into_runtime().unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        let r = dom.find_by_id("r").unwrap();
        let c = dom.find_by_id("c").unwrap();
        let f = dom.find_by_id("f").unwrap();
        let b = dom.find_by_id("b").unwrap();
        assert_eq!(graph.full[&r].role, "slider");
        assert_eq!(graph.full[&c].role, "colorwell");
        assert!(!graph.full[&r].states.contains(&NodeState::Readonly));
        assert!(graph.full[&c].actions.contains(&SemanticAction::Type));
        assert!(!graph.full[&f].actions.contains(&SemanticAction::Type));
        assert!(!graph.full[&b].actions.contains(&SemanticAction::Type));
        assert!(graph.full[&b].actions.contains(&SemanticAction::Check));
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod native_numeric_ax_inventory_tests {
    use super::*;
    #[test]
    fn native_ax_numeric_properties_update_and_match_full_recomputation() {
        let mut dom = crate::html::parse("<input id=r type=range min=2 max=8 step=2 value=4>")
            .into_runtime()
            .unwrap();
        let r = dom.find_by_id("r").unwrap();
        let mut graph = SemanticGraph::new(PageId(1), 1);
        graph.reset(dom.arena(), 1);
        graph.apply_native_control_state(&dom).unwrap();
        assert_eq!(
            graph.accessibility[&r].properties["native_value_min"],
            serde_json::json!(2.)
        );
        assert_eq!(
            graph.accessibility[&r].properties["native_value_now"],
            serde_json::json!(4.)
        );
        dom.set_value(r, "7", false).unwrap();
        graph.apply_native_control_state(&dom).unwrap();
        let mut rebuilt = SemanticGraph::new(PageId(1), 1);
        rebuilt.reset(dom.arena(), 1);
        rebuilt.apply_native_control_state(&dom).unwrap();
        assert_eq!(
            graph.accessibility[&r].properties,
            rebuilt.accessibility[&r].properties
        );
        assert_eq!(
            graph.accessibility[&r].properties["native_value_now"],
            serde_json::json!(8.)
        );
    }
}

fn naming_ascii_lowercase_with_checkpoint(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    let mut result = String::new();
    for character in value.chars() {
        check()?;
        result.push(character.to_ascii_lowercase());
    }
    check()?;
    Ok(result)
}

#[cfg(test)]
mod naming_value_controls {
    use super::*;
    #[test]
    fn signed_integer_parity_and_long_zero_cancellation() {
        for text in [
            "",
            "+",
            "-",
            "-0",
            "+0",
            "2147483647",
            "-2147483648",
            "2147483648",
            "-2147483649",
            "  12  ",
            "α",
            "00",
        ] {
            assert_eq!(
                naming_i32_with_checkpoint(text, &mut || Ok(())).unwrap(),
                text.trim().parse::<i32>().ok()
            );
        }
        let text = "0".repeat(8192);
        let mut calls = 0;
        let result = naming_i32_with_checkpoint(&text, &mut || {
            calls += 1;
            if calls == 64 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(crate::dom::DomError::ProjectionCancelled)
        ));
        assert_eq!(calls, 64);
    }
    #[test]
    fn long_slot_value_comparison_cancels() {
        let text = "x".repeat(8192);
        let mut calls = 0;
        let result = naming_equal_with_checkpoint(&text, &text, &mut || {
            calls += 1;
            if calls == 64 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(crate::dom::DomError::ProjectionCancelled)
        ));
        assert_eq!(calls, 64);
    }
}

pub(crate) fn naming_clone_with_checkpoint(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, crate::dom::DomError> {
    OrdinarySemanticProducer.text(value, check)
}

fn naming_trim_with_checkpoint<'a>(
    value: &'a str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<&'a str, crate::dom::DomError> {
    let mut start = value.len();
    let mut end = 0;
    for (index, character) in value.char_indices() {
        check()?;
        if !character.is_whitespace() {
            start = start.min(index);
            end = index + character.len_utf8();
        }
    }
    check()?;
    Ok(if end == 0 {
        &value[value.len()..]
    } else {
        &value[start..end]
    })
}

fn naming_equal_with_checkpoint(
    left: &str,
    right: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.bytes().zip(right.bytes()) {
        check()?;
        if left != right {
            return Ok(false);
        }
    }
    check()?;
    Ok(true)
}
fn naming_i32_with_checkpoint(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<i32>, crate::dom::DomError> {
    let value = naming_trim_with_checkpoint(value, check)?;
    let mut digits = value.bytes();
    let first = digits.next();
    check()?;
    let negative = first == Some(b'-');
    let mut result = 0i32;
    let mut count = 0usize;
    let rest = if matches!(first, Some(b'+' | b'-')) {
        None
    } else {
        first
    };
    for digit in rest.into_iter().chain(digits) {
        check()?;
        if !digit.is_ascii_digit() {
            return Ok(None);
        }
        count += 1;
        let digit = i32::from(digit - b'0');
        result = match result.checked_mul(10).and_then(|n| {
            if negative {
                n.checked_sub(digit)
            } else {
                n.checked_add(digit)
            }
        }) {
            Some(n) => n,
            None => return Ok(None),
        };
    }
    check()?;
    Ok((count > 0).then_some(result))
}

fn naming_span_with_checkpoint(
    node: &crate::dom::Node,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<u64, crate::dom::DomError> {
    let parsed = match node.attributes.get_null_with_checkpoint("span", check)? {
        Some(value) => naming_u64_with_checkpoint(value, check)?,
        None => None,
    };
    Ok(parsed.filter(|span| *span > 0).unwrap_or(1).min(1000))
}
fn naming_u64_with_checkpoint(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<u64>, crate::dom::DomError> {
    let mut result = 0u64;
    let mut saw = false;
    let mut first = true;
    for byte in value.bytes() {
        check()?;
        if first && byte == b'+' {
            first = false;
            continue;
        }
        first = false;
        if !byte.is_ascii_digit() {
            return Ok(None);
        }
        saw = true;
        result = match result
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(byte - b'0')))
        {
            Some(n) => n,
            None => return Ok(None),
        };
    }
    check()?;
    Ok(saw.then_some(result))
}

fn subtree_excluded_at_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(false);
    };
    let aria = node
        .accessibility_attribute_with_checkpoint("aria-hidden", check)?
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if let Some((display_none, _)) = dom.accessibility_computed_visibility(id) {
        return Ok(display_none || aria);
    }
    if node
        .attributes
        .contains_null_with_checkpoint("hidden", check)?
        || aria
    {
        return Ok(true);
    }
    if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
        for declaration in checked_name_tokens(style, |character| character == ';', check)? {
            let tokens = checked_name_tokens(declaration, |character| character == ':', check)?;
            if tokens.len() < 2 {
                continue;
            }
            if naming_trim_with_checkpoint(tokens[0], check)?.eq_ignore_ascii_case("display") {
                return Ok(naming_trim_with_checkpoint(
                    &declaration[tokens[0].len() + 1..],
                    check,
                )?
                .eq_ignore_ascii_case("none"));
            }
        }
    }
    Ok(false)
}

fn naming_visibility_hidden_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if let Some((_, hidden)) = dom.accessibility_computed_visibility(id) {
        return Ok(hidden);
    }
    let Ok(node) = dom.get(id) else {
        return Ok(false);
    };
    if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
        for declaration in checked_name_tokens(style, |character| character == ';', check)? {
            let tokens = checked_name_tokens(declaration, |character| character == ':', check)?;
            if tokens.len() > 1
                && naming_trim_with_checkpoint(tokens[0], check)?.eq_ignore_ascii_case("visibility")
            {
                let value =
                    naming_trim_with_checkpoint(&declaration[tokens[0].len() + 1..], check)?;
                return Ok(
                    value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
                );
            }
        }
    }
    Ok(false)
}
fn accessibility_hidden_at_with_checkpoint(
    dom: &DomArena,
    id: NodeId,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    Ok(subtree_excluded_at_with_checkpoint(dom, id, check)?
        || naming_visibility_hidden_with_checkpoint(dom, id, check)?)
}

fn clone_semantic_node_with_checkpoint(
    node: &SemanticNodeView,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<SemanticNodeView, crate::dom::DomError> {
    shared_semantic_node(node, &mut OrdinarySemanticProducer, check)
}

fn semantic_node_equal_with_checkpoint(
    left: &SemanticNodeView,
    right: &SemanticNodeView,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if left.semantic_ref != right.semantic_ref
        || left.parent != right.parent
        || left.document_order != right.document_order
        || left.alias != right.alias
        || left.validation != right.validation
        || left.states != right.states
        || left.actions != right.actions
        || left.content_trust != right.content_trust
    {
        return Ok(false);
    }
    if !provenance_equal_with_checkpoint(&left.provenance, &right.provenance, check)? {
        return Ok(false);
    }
    for (left, right) in [(&left.role, &right.role), (&left.name, &right.name)] {
        if !naming_equal_with_checkpoint(left, right, check)? {
            return Ok(false);
        }
    }
    for (left, right) in [
        (&left.selector, &right.selector),
        (&left.value, &right.value),
        (&left.description, &right.description),
    ] {
        match (left, right) {
            (None, None) => {}
            (Some(left), Some(right)) => {
                if !naming_equal_with_checkpoint(left, right, check)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    check()?;
    Ok(true)
}
#[cfg(test)]
mod json_depth_controls {
    use super::*;

    #[test]
    fn iterative_json_clone_preserves_order_and_nested_values() {
        let input = serde_json::json!({"first":[1,{"nested":[true,null,"text"]}],"second":2});
        let result = clone_json_iterative_with_checkpoint(&input, &mut || Ok(())).unwrap();
        assert_eq!(result, input);
        assert_eq!(result.to_string(), input.to_string());
    }

    #[test]
    fn deeply_assembled_json_cancellation_drops_iteratively() {
        // Use a dedicated small stack: recursive cleanup of this chain would
        // overflow. Both the source and successful probe use iterative owners.
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut input = serde_json::Value::String("leaf".into());
                for _ in 0..16384 {
                    input = serde_json::Value::Array(vec![input]);
                }
                let source = JsonValuesGuard(vec![input]);
                let mut count = 0usize;
                let probe = clone_json_iterative_with_checkpoint(&source.0[0], &mut || {
                    count += 1;
                    Ok(())
                })
                .unwrap();
                let _probe_owner = JsonValuesGuard(vec![probe]);
                // Near final publication, children are already fully assembled.
                for boundary in [count - 1, count] {
                    let mut calls = 0usize;
                    let result = clone_json_iterative_with_checkpoint(&source.0[0], &mut || {
                        calls += 1;
                        if calls == boundary {
                            Err(crate::dom::DomError::ProjectionCancelled)
                        } else {
                            Ok(())
                        }
                    });
                    assert!(matches!(
                        result,
                        Err(crate::dom::DomError::ProjectionCancelled)
                    ));
                    assert_eq!(calls, boundary);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn cancellation_during_object_and_array_assembly_preserves_source() {
        let input = serde_json::json!({"a":[[1],[2]],"b":{"c":[3],"d":[4]}});
        let before = input.to_string();
        let mut total = 0usize;
        let _ = clone_json_iterative_with_checkpoint(&input, &mut || {
            total += 1;
            Ok(())
        })
        .unwrap();
        for boundary in 1..=total {
            let mut calls = 0usize;
            let result = clone_json_iterative_with_checkpoint(&input, &mut || {
                calls += 1;
                if calls == boundary {
                    Err(crate::dom::DomError::ProjectionCancelled)
                } else {
                    Ok(())
                }
            });
            assert!(matches!(
                result,
                Err(crate::dom::DomError::ProjectionCancelled)
            ));
            assert_eq!(input.to_string(), before);
        }
    }
}

fn clone_provenance_with_checkpoint(
    value: &Provenance,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Provenance, crate::dom::DomError> {
    shared_provenance(value, &mut OrdinarySemanticProducer, check)
}
fn provenance_equal_with_checkpoint(
    left: &Provenance,
    right: &Provenance,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    Ok(match (left, right) {
        (Provenance::BrowserDeterministic, Provenance::BrowserDeterministic) => true,
        (
            Provenance::Heuristic { confidence: left },
            Provenance::Heuristic { confidence: right },
        ) => left == right,
        (
            Provenance::Model {
                confidence: left,
                model_id: a,
            },
            Provenance::Model {
                confidence: right,
                model_id: b,
            },
        ) => left == right && naming_equal_with_checkpoint(a, b, check)?,
        _ => false,
    })
}

fn clone_json_with_checkpoint(
    value: &serde_json::Value,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<serde_json::Value, crate::dom::DomError> {
    clone_json_iterative_with_checkpoint(value, check)
}

#[cfg(test)]
mod deep_cancellation_controls {
    use super::*;
    #[test]
    fn cancelled_node_refresh_preserves_graph() {
        let dom = crate::html::parse("<button id=one>one</button>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        let id = *graph.full.keys().next().unwrap();
        let before = format!("{graph:?}");
        let mut total = 0;
        let mut probe = graph.clone();
        probe
            .refresh_node_with_checkpoint(dom.arena(), id, &mut || {
                total += 1;
                Ok(())
            })
            .unwrap();
        for boundary in 1..=total {
            let mut draft = graph.clone();
            let mut calls = 0;
            let result = draft.refresh_node_with_checkpoint(dom.arena(), id, &mut || {
                calls += 1;
                if calls == boundary {
                    Err(crate::dom::DomError::ProjectionCancelled)
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert_eq!(format!("{draft:?}"), before);
        }
    }
    #[test]
    fn nested_json_long_string_cancels() {
        let value = serde_json::json!({"key":["x".repeat(8192)]});
        let mut calls = 0;
        let result = clone_json_with_checkpoint(&value, &mut || {
            calls += 1;
            if calls == 64 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(crate::dom::DomError::ProjectionCancelled)
        ));
        assert_eq!(calls, 64);
    }
}

pub(crate) fn clone_accessibility_with_checkpoint(
    node: &AccessibilityNode,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<AccessibilityNode, crate::dom::DomError> {
    shared_accessibility(node, &mut OrdinarySemanticProducer, check)
}

fn clone_evidence_with_checkpoint(
    values: &[AccessibilityEvidence],
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<AccessibilityEvidence>, crate::dom::DomError> {
    shared_evidence(values, &mut OrdinarySemanticProducer, check)
}
pub(crate) fn clone_structure_with_checkpoint(
    value: &SemanticStructure,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<SemanticStructure, crate::dom::DomError> {
    shared_structure(value, &mut OrdinarySemanticProducer, check)
}

fn clone_json_iterative_with_checkpoint(
    value: &serde_json::Value,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<serde_json::Value, crate::dom::DomError> {
    shared_json(value, &mut OrdinarySemanticProducer, check)
}

struct JsonValuesGuard(Vec<serde_json::Value>);
impl Drop for JsonValuesGuard {
    fn drop(&mut self) {
        while let Some(value) = self.0.pop() {
            match value {
                serde_json::Value::Array(values) => self.0.extend(values),
                serde_json::Value::Object(values) => self.0.extend(values.into_values()),
                _ => {}
            }
        }
    }
}
/// A graph draft may contain deeply nested property values. Disposal is an
/// ownership operation, not a projection pass: it must finish even after the
/// callback has requested cancellation, without recursive JSON destruction.
impl Drop for SemanticGraph {
    fn drop(&mut self) {
        let mut values = JsonValuesGuard(Vec::new());
        for node in self.accessibility.values_mut() {
            for (_, value) in std::mem::take(&mut node.properties) {
                values.0.push(value);
            }
        }
        for node in self.pending_accessibility_upserts.values_mut() {
            for (_, value) in std::mem::take(&mut node.properties) {
                values.0.push(value);
            }
        }
        for node in self.structures.values_mut() {
            for (_, value) in std::mem::take(&mut node.properties) {
                values.0.push(value);
            }
        }
        for node in self.pending_structure_upserts.values_mut() {
            for (_, value) in std::mem::take(&mut node.properties) {
                values.0.push(value);
            }
        }
        // The guard now owns every JSON property. Other fields have no recursive
        // Value ownership and retain their standard destruction behavior.
    }
}

struct JsonPropertiesGuard(BTreeMap<String, serde_json::Value>);
impl std::ops::Deref for JsonPropertiesGuard {
    type Target = BTreeMap<String, serde_json::Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for JsonPropertiesGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
struct AccessibilityOwner(Option<AccessibilityNode>);
impl Drop for AccessibilityOwner {
    fn drop(&mut self) {
        if let Some(mut node) = self.0.take() {
            drop(JsonPropertiesGuard(std::mem::take(&mut node.properties)));
        }
    }
}
impl Drop for JsonPropertiesGuard {
    fn drop(&mut self) {
        let mut values = JsonValuesGuard(Vec::new());
        for (_, value) in std::mem::take(&mut self.0) {
            values.0.push(value);
        }
    }
}

#[cfg(test)]
mod graph_disposal_controls {
    use super::*;
    #[test]
    fn graph_drop_dismantles_deep_properties_without_recursion() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let dom = crate::html::parse("<button>one</button>")
                    .into_runtime()
                    .unwrap();
                let mut graph = SemanticGraph::new(PageId(31), 9);
                graph.reset(dom.arena(), 9);
                let node = graph.accessibility.values_mut().next().unwrap();
                let mut value = serde_json::Value::Null;
                for _ in 0..16384 {
                    value = serde_json::Value::Array(vec![value]);
                }
                node.properties.insert("deep".into(), value);
                drop(graph);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
fn json_equal_with_checkpoint(
    left: &serde_json::Value,
    right: &serde_json::Value,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    let mut pairs = vec![(left, right)];
    while let Some((left, right)) = pairs.pop() {
        check()?;
        match (left, right) {
            (serde_json::Value::Null, serde_json::Value::Null) => {}
            (serde_json::Value::Bool(left), serde_json::Value::Bool(right)) if left == right => {}
            (serde_json::Value::Number(left), serde_json::Value::Number(right))
                if left == right => {}
            (serde_json::Value::String(left), serde_json::Value::String(right)) => {
                if !naming_equal_with_checkpoint(left, right, check)? {
                    return Ok(false);
                }
            }
            (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
                if left.len() != right.len() {
                    return Ok(false);
                }
                for pair in left.iter().zip(right) {
                    check()?;
                    pairs.push(pair);
                }
            }
            (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
                if left.len() != right.len() {
                    return Ok(false);
                }
                let left = json_ordered_entries_with_checkpoint(left, check)?;
                let right = json_ordered_entries_with_checkpoint(right, check)?;
                for ((key, value), (other_key, other)) in left.into_iter().zip(right) {
                    check()?;
                    if !naming_equal_with_checkpoint(key, other_key, check)? {
                        return Ok(false);
                    }
                    pairs.push((value, other));
                }
            }
            _ => return Ok(false),
        }
    }
    check()?;
    Ok(true)
}

fn accessibility_equal_with_checkpoint(
    left: &AccessibilityNode,
    right: &AccessibilityNode,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if left.semantic_ref != right.semantic_ref
        || left.excluded != right.excluded
        || left.owned_by != right.owned_by
        || left.content_trust != right.content_trust
        || left.relationships.len() != right.relationships.len()
        || left.properties.len() != right.properties.len()
        || left.evidence.len() != right.evidence.len()
    {
        return Ok(false);
    }
    for (left, right) in [(&left.role, &right.role), (&left.name, &right.name)] {
        if !naming_equal_with_checkpoint(left, right, check)? {
            return Ok(false);
        }
    }
    for (left, right) in [
        (&left.description, &right.description),
        (&left.exclusion_reason, &right.exclusion_reason),
    ] {
        match (left, right) {
            (None, None) => {}
            (Some(left), Some(right)) => {
                if !naming_equal_with_checkpoint(left, right, check)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    if !provenance_equal_with_checkpoint(&left.provenance, &right.provenance, check)? {
        return Ok(false);
    }
    for ((lk, lv), (rk, rv)) in left.relationships.iter().zip(&right.relationships) {
        check()?;
        if lk != rk || lv.len() != rv.len() {
            return Ok(false);
        }
        for (left, right) in lv.iter().zip(rv) {
            check()?;
            if left != right {
                return Ok(false);
            }
        }
    }
    for ((lk, lv), (rk, rv)) in left.properties.iter().zip(&right.properties) {
        check()?;
        if !naming_equal_with_checkpoint(lk, rk, check)?
            || !json_equal_with_checkpoint(lv, rv, check)?
        {
            return Ok(false);
        }
    }
    for (left, right) in left.evidence.iter().zip(&right.evidence) {
        check()?;
        if left.kind != right.kind
            || left.references.len() != right.references.len()
            || !naming_equal_with_checkpoint(&left.source, &right.source, check)?
        {
            return Ok(false);
        }
        for (left, right) in left.references.iter().zip(&right.references) {
            check()?;
            if left != right {
                return Ok(false);
            }
        }
    }
    check()?;
    Ok(true)
}
fn structure_equal_with_checkpoint(
    left: &SemanticStructure,
    right: &SemanticStructure,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, crate::dom::DomError> {
    check()?;
    if left.kind != right.kind
        || left.root != right.root
        || left.content_trust != right.content_trust
        || left.members.len() != right.members.len()
        || left.groups.len() != right.groups.len()
        || left.properties.len() != right.properties.len()
        || left.evidence.len() != right.evidence.len()
    {
        return Ok(false);
    }
    for (left, right) in [
        (&left.structure_id, &right.structure_id),
        (&left.name, &right.name),
    ] {
        if !naming_equal_with_checkpoint(left, right, check)? {
            return Ok(false);
        }
    }
    match (&left.parent_structure_id, &right.parent_structure_id) {
        (None, None) => {}
        (Some(left), Some(right)) => {
            if !naming_equal_with_checkpoint(left, right, check)? {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    if !provenance_equal_with_checkpoint(&left.provenance, &right.provenance, check)? {
        return Ok(false);
    }
    for (left, right) in left.members.iter().zip(&right.members) {
        check()?;
        if left != right {
            return Ok(false);
        }
    }
    for (left, right) in left.groups.iter().zip(&right.groups) {
        check()?;
        if left.members.len() != right.members.len()
            || !naming_equal_with_checkpoint(&left.kind, &right.kind, check)?
        {
            return Ok(false);
        }
        match (&left.name, &right.name) {
            (None, None) => {}
            (Some(left), Some(right)) => {
                if !naming_equal_with_checkpoint(left, right, check)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        for (left, right) in left.members.iter().zip(&right.members) {
            check()?;
            if left != right {
                return Ok(false);
            }
        }
    }
    for ((lk, lv), (rk, rv)) in left.properties.iter().zip(&right.properties) {
        check()?;
        if !naming_equal_with_checkpoint(lk, rk, check)?
            || !json_equal_with_checkpoint(lv, rv, check)?
        {
            return Ok(false);
        }
    }
    for (left, right) in left.evidence.iter().zip(&right.evidence) {
        check()?;
        if left.kind != right.kind
            || left.references.len() != right.references.len()
            || !naming_equal_with_checkpoint(&left.source, &right.source, check)?
        {
            return Ok(false);
        }
        for (left, right) in left.references.iter().zip(&right.references) {
            check()?;
            if left != right {
                return Ok(false);
            }
        }
    }
    check()?;
    Ok(true)
}

struct AccessibilityListOwner(Vec<AccessibilityNode>);
impl Drop for AccessibilityListOwner {
    fn drop(&mut self) {
        for mut node in self.0.drain(..) {
            drop(JsonPropertiesGuard(std::mem::take(&mut node.properties)));
        }
    }
}
struct StructureMapOwner(BTreeMap<String, SemanticStructure>);
struct StructureListOwner(Vec<SemanticStructure>);
impl Drop for StructureListOwner {
    fn drop(&mut self) {
        for mut node in self.0.drain(..) {
            drop(JsonPropertiesGuard(std::mem::take(&mut node.properties)));
        }
    }
}
impl Drop for StructureMapOwner {
    fn drop(&mut self) {
        for (_, mut node) in std::mem::take(&mut self.0) {
            drop(JsonPropertiesGuard(std::mem::take(&mut node.properties)));
        }
    }
}
/// Private native custody only. Public common records retain movable fields;
/// serialized representation delegates to the unchanged record.
pub(crate) struct OwnedSemanticChanges(
    Option<oryn_common::v6::SemanticGraphChanges>,
    Option<usize>,
    Vec<SemanticAllocationReceipt>,
);
pub(crate) fn dispose_owned_properties(properties: &mut BTreeMap<String, serde_json::Value>) {
    drop(JsonPropertiesGuard(std::mem::take(properties)));
}
impl OwnedSemanticChanges {
    pub(crate) fn new(changes: oryn_common::v6::SemanticGraphChanges) -> Self {
        Self(Some(changes), None, Vec::new())
    }
    #[cfg(test)]
    pub(crate) fn with_verified_bytes(
        changes: oryn_common::v6::SemanticGraphChanges,
        bytes: usize,
    ) -> Self {
        Self(Some(changes), Some(bytes), Vec::new())
    }
    pub(crate) fn verified_bytes(&self) -> usize {
        self.1
            .expect("cache owner requires verified byte admission")
    }
}
impl std::ops::Deref for OwnedSemanticChanges {
    type Target = oryn_common::v6::SemanticGraphChanges;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}
impl serde::Serialize for OwnedSemanticChanges {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().unwrap().serialize(serializer)
    }
}
impl Drop for OwnedSemanticChanges {
    fn drop(&mut self) {
        if let Some(mut changes) = self.0.take() {
            for mut node in changes.upserted_nodes.drain(..) {
                drop(JsonPropertiesGuard(std::mem::take(
                    &mut node.accessibility_properties,
                )));
            }
            drop(AccessibilityListOwner(std::mem::take(
                &mut changes.upserted_accessibility,
            )));
            for mut structure in changes.upserted_structures.drain(..) {
                drop(JsonPropertiesGuard(std::mem::take(
                    &mut structure.properties,
                )));
            }
        }
    }
}

pub(crate) fn dispose_semantic_observation(mut observation: oryn_common::v6::SemanticObservation) {
    for mut node in observation.nodes.drain(..) {
        drop(JsonPropertiesGuard(std::mem::take(
            &mut node.accessibility_properties,
        )));
    }
    drop(AccessibilityListOwner(std::mem::take(
        &mut observation.accessibility,
    )));
    for mut structure in observation.structures.drain(..) {
        drop(JsonPropertiesGuard(std::mem::take(
            &mut structure.properties,
        )));
    }
}
fn json_ordered_entries_with_checkpoint<'a>(
    values: &'a serde_json::Map<String, serde_json::Value>,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<(&'a str, &'a serde_json::Value)>, crate::dom::DomError> {
    let mut entries = Vec::new();
    for (key, value) in values {
        check()?;
        entries.push((key.as_str(), value));
    }
    // Callback-aware bottom-up merge sort on borrowed entries. Works for both
    // BTreeMap and preserve_order maps without copying keys or changing JSON.
    let mut width = 1usize;
    while width < entries.len() {
        let mut merged = Vec::new();
        let mut start = 0usize;
        while start < entries.len() {
            check()?;
            let mid = start.saturating_add(width).min(entries.len());
            let end = mid.saturating_add(width).min(entries.len());
            let (mut left, mut right) = (start, mid);
            while left < mid || right < end {
                check()?;
                let take_left = if right == end {
                    true
                } else if left == mid {
                    false
                } else {
                    json_key_order_with_checkpoint(entries[left].0, entries[right].0, check)?
                        != std::cmp::Ordering::Greater
                };
                if take_left {
                    merged.push(entries[left]);
                    left += 1;
                } else {
                    merged.push(entries[right]);
                    right += 1;
                }
            }
            start = end;
        }
        entries = merged;
        width = width.saturating_mul(2);
    }
    check()?;
    Ok(entries)
}

fn json_key_order_with_checkpoint(
    left: &str,
    right: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<std::cmp::Ordering, crate::dom::DomError> {
    let mut left = left.bytes();
    let mut right = right.bytes();
    loop {
        check()?;
        match (left.next(), right.next()) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => return Ok(left.cmp(&right)),
            (None, None) => return Ok(std::cmp::Ordering::Equal),
            (None, Some(_)) => return Ok(std::cmp::Ordering::Less),
            (Some(_), None) => return Ok(std::cmp::Ordering::Greater),
        }
    }
}
#[cfg(test)]
mod private_ownership_controls {
    use super::*;
    #[test]
    fn private_changes_preserve_wire_shape_and_deep_disposal() {
        let changes = oryn_common::v6::SemanticGraphChanges::default();
        let before = serde_json::to_vec(&changes).unwrap();
        let owner = OwnedSemanticChanges::new(changes);
        assert_eq!(serde_json::to_vec(&owner).unwrap(), before);
    }
    #[test]
    fn key_sort_is_order_independent_and_cancellable() {
        let value = serde_json::json!({"z":0,"a":1,"中":2});
        let map = value.as_object().unwrap();
        let entries = json_ordered_entries_with_checkpoint(map, &mut || Ok(())).unwrap();
        assert_eq!(
            entries.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            ["a", "z", "中"]
        );
        let mut calls = 0;
        assert!(
            json_ordered_entries_with_checkpoint(map, &mut || {
                calls += 1;
                if calls == 4 {
                    Err(crate::dom::DomError::ProjectionCancelled)
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
    }
}
pub(crate) fn clone_delta_node_with_checkpoint(
    node: &oryn_common::v6::SemanticNode,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<oryn_common::v6::SemanticNode, crate::dom::DomError> {
    let semantic = clone_semantic_node_with_checkpoint(&node.semantic, check)?;
    let mut relationships = BTreeMap::new();
    for (key, values) in &node.relationships {
        let mut copied = Vec::new();
        for value in values {
            check()?;
            copied.push(*value);
        }
        relationships.insert(key.clone(), copied);
    }
    let mut properties = JsonPropertiesGuard(BTreeMap::new());
    for (key, value) in &node.accessibility_properties {
        check()?;
        properties.0.insert(
            naming_clone_with_checkpoint(key, check)?,
            clone_json_with_checkpoint(value, check)?,
        );
    }
    let mut structure_ids = Vec::new();
    for value in &node.structure_ids {
        structure_ids.push(naming_clone_with_checkpoint(value, check)?);
    }
    let mut field_evidence = Vec::new();
    for value in &node.field_evidence {
        check()?;
        field_evidence.push(SemanticFieldEvidence {
            field: value.field,
            trust: value.trust,
            provenance: clone_provenance_with_checkpoint(&value.provenance, check)?,
            evidence: clone_evidence_with_checkpoint(&value.evidence, check)?,
        });
    }
    let geometry = node
        .geometry
        .as_ref()
        .map(|value| {
            let mut client_rects = Vec::new();
            for rect in &value.client_rects {
                check()?;
                client_rects.push(*rect);
            }
            Ok::<_, crate::dom::DomError>(oryn_common::v4::NodeGeometry {
                availability: value.availability,
                box_model: value.box_model.clone(),
                client_rects,
                in_viewport: value.in_viewport,
                interactable: value.interactable,
                diagnostic: value
                    .diagnostic
                    .as_ref()
                    .map(|value| naming_clone_with_checkpoint(value, check))
                    .transpose()?,
            })
        })
        .transpose()?;
    check()?;
    Ok(oryn_common::v6::SemanticNode {
        semantic,
        frame_id: node.frame_id,
        relationships,
        accessibility_properties: std::mem::take(&mut properties.0),
        structure_ids,
        geometry,
        field_evidence,
    })
}

pub(crate) struct DeltaAssembly {
    pub(crate) nodes: BTreeMap<SemanticRef, oryn_common::v6::SemanticNode>,
    pub(crate) accessibility: BTreeMap<SemanticRef, AccessibilityNode>,
    pub(crate) structures: BTreeMap<String, SemanticStructure>,
}
impl DeltaAssembly {
    pub(crate) fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            accessibility: BTreeMap::new(),
            structures: BTreeMap::new(),
        }
    }
}
impl Drop for DeltaAssembly {
    fn drop(&mut self) {
        for node in self.nodes.values_mut() {
            dispose_owned_properties(&mut node.accessibility_properties);
        }
        for node in self.accessibility.values_mut() {
            dispose_owned_properties(&mut node.properties);
        }
        for node in self.structures.values_mut() {
            dispose_owned_properties(&mut node.properties);
        }
    }
}
#[cfg(test)]
pub(crate) fn deep_changes_for_test() -> oryn_common::v6::SemanticGraphChanges {
    let dom = crate::html::parse("<button>deep</button>")
        .into_runtime()
        .unwrap();
    let mut graph = SemanticGraph::new(PageId(31), 9);
    graph.reset(dom.arena(), 9);
    let mut changes = graph.take_v6_changes();
    let mut value = serde_json::Value::Null;
    for _ in 0..16384 {
        value = serde_json::Value::Array(vec![value]);
    }
    changes
        .upserted_accessibility
        .first_mut()
        .unwrap()
        .properties
        .insert("deep".into(), value);
    changes
}

#[cfg(test)]
mod private_depth_controls {
    use super::*;
    #[test]
    fn cache_eviction_drops_deep_private_changes_on_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut cache = BTreeMap::new();
                cache.insert(1, OwnedSemanticChanges::new(deep_changes_for_test()));
                drop(cache.remove(&1));
                cache.insert(2, OwnedSemanticChanges::new(deep_changes_for_test()));
                drop(cache);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn failed_actor_delivery_drops_deep_changes_on_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                drop(receiver);
                let unsent = sender.send(deep_changes_for_test()).unwrap_err();
                drop(OwnedSemanticChanges::new(unsent));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
/// Nonrecursive JSON wire traversal. Scalar serialization delegates to the
/// existing serde_json numeric/string formatter; container depth is iterative.
#[cfg(test)]
fn write_json_value_iterative(
    writer: &mut impl std::io::Write,
    value: &serde_json::Value,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    write_json_cursor(writer, value, check)
}

fn write_json_string_checked(
    writer: &mut impl std::io::Write,
    value: &str,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(b"\"")?;
    for ch in value.chars() {
        check()?;
        match ch {
            '"' => writer.write_all(b"\\\"")?,
            '\\' => writer.write_all(b"\\\\")?,
            '\u{8}' => writer.write_all(b"\\b")?,
            '\u{c}' => writer.write_all(b"\\f")?,
            '\n' => writer.write_all(b"\\n")?,
            '\r' => writer.write_all(b"\\r")?,
            '\t' => writer.write_all(b"\\t")?,
            ch if ch <= '\u{1f}' => {
                let hex = b"0123456789abcdef";
                let byte = ch as u8;
                writer.write_all(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    hex[(byte >> 4) as usize],
                    hex[(byte & 15) as usize],
                ])?;
            }
            ch => {
                let mut bytes = [0; 4];
                writer.write_all(ch.encode_utf8(&mut bytes).as_bytes())?;
            }
        }
    }
    check()?;
    writer.write_all(b"\"")
}

#[cfg(test)]
mod iterative_wire_controls {
    use super::*;
    #[test]
    fn exact_scalar_container_wire_parity() {
        let values = [
            serde_json::json!({"z":[null,true,false,1,-1,1.25],"escape":"\u{0}\u{8}\u{c}\n\r\t\\\"中😀"}),
            serde_json::json!([]),
            serde_json::json!({}),
        ];
        for value in values {
            let mut bytes = Vec::new();
            write_json_value_iterative(&mut bytes, &value, &mut || Ok(())).unwrap();
            assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
        }
    }
    #[test]
    fn successful_deep_wire_on_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut value = serde_json::Value::Null;
                for _ in 0..16384 {
                    value = serde_json::Value::Array(vec![value]);
                }
                let owner = JsonValuesGuard(vec![value]);
                let mut bytes = Vec::new();
                write_json_value_iterative(&mut bytes, &owner.0[0], &mut || Ok(())).unwrap();
                assert_eq!(bytes.len(), 32772);
                assert_eq!(&bytes[16384..16388], b"null");
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn cancellation_stops_actual_wire_loop() {
        let value = serde_json::Value::String("x".repeat(8192));
        let mut calls = 0;
        let mut bytes = Vec::new();
        let error = write_json_value_iterative(&mut bytes, &value, &mut || {
            calls += 1;
            if calls == 64 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(bytes.len() < 64);
    }
}
fn write_properties_iterative(
    writer: &mut impl std::io::Write,
    properties: &BTreeMap<String, serde_json::Value>,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(b"{")?;
    let mut first = true;
    for (key, value) in properties {
        check()?;
        if !first {
            writer.write_all(b",")?;
        }
        first = false;
        write_json_string_checked(writer, key, check)?;
        writer.write_all(b":")?;
        write_json_cursor(writer, value, check)?;
    }
    writer.write_all(b"}")
}

fn write_accessibility_record_iterative(
    writer: &mut impl std::io::Write,
    node: &AccessibilityNode,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(b"{")?;
    let mut first = true;
    macro_rules! field {
        ($name:literal, $value:expr) => {{
            check()?;
            if !first {
                writer.write_all(b",")?;
            }
            first = false;
            writer.write_all(concat!("\"", $name, "\":").as_bytes())?;
            write_checked_serde(writer, $value, check)?;
        }};
    }
    field!("semantic_ref", &node.semantic_ref);
    field!("excluded", &node.excluded);
    if let Some(value) = &node.exclusion_reason {
        field!("exclusion_reason", value);
    }
    field!("role", &node.role);
    field!("name", &node.name);
    if let Some(value) = &node.description {
        field!("description", value);
    }
    if !node.relationships.is_empty() {
        field!("relationships", &node.relationships);
    }
    if !node.properties.is_empty() {
        check()?;
        if !first {
            writer.write_all(b",")?;
        }
        first = false;
        writer.write_all(b"\"properties\":")?;
        write_properties_iterative(writer, &node.properties, check)?;
    }
    if let Some(value) = &node.owned_by {
        field!("owned_by", value);
    }
    field!("content_trust", &node.content_trust);
    field!("provenance", &node.provenance);
    if !node.evidence.is_empty() {
        field!("evidence", &node.evidence);
    }
    let _ = first;
    writer.write_all(b"}")
}

#[cfg(test)]
mod accessibility_wire_controls {
    use super::*;
    #[test]
    fn accessibility_record_exact_wire_parity() {
        let dom = crate::html::parse("<button aria-label=one>two</button>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        for node in graph.accessibility.values() {
            let mut bytes = Vec::new();
            write_accessibility_record_iterative(&mut bytes, node, &mut || Ok(())).unwrap();
            assert_eq!(bytes, serde_json::to_vec(node).unwrap());
        }
    }
}
struct WireObject<'a, W: std::io::Write, C: FnMut() -> std::io::Result<()>> {
    writer: &'a mut W,
    check: &'a mut C,
    first: bool,
}
impl<'a, W: std::io::Write, C: FnMut() -> std::io::Result<()>> WireObject<'a, W, C> {
    fn new(writer: &'a mut W, check: &'a mut C) -> std::io::Result<Self> {
        check()?;
        writer.write_all(b"{")?;
        Ok(Self {
            writer,
            check,
            first: true,
        })
    }
    fn key(&mut self, key: &str) -> std::io::Result<()> {
        (self.check)()?;
        if !self.first {
            self.writer.write_all(b",")?;
        }
        self.first = false;
        write_json_string_checked(self.writer, key, self.check)?;
        self.writer.write_all(b":")
    }
    fn field<T: serde::Serialize + ?Sized>(&mut self, key: &str, value: &T) -> std::io::Result<()> {
        self.key(key)?;
        write_checked_serde(self.writer, value, self.check)?;
        Ok(())
    }
    fn string(&mut self, key: &str, value: &str) -> std::io::Result<()> {
        self.key(key)?;
        write_json_string_checked(self.writer, value, self.check)
    }
    fn properties(
        &mut self,
        key: &str,
        value: &BTreeMap<String, serde_json::Value>,
    ) -> std::io::Result<()> {
        self.key(key)?;
        write_properties_iterative(self.writer, value, self.check)
    }
    fn finish(self) -> std::io::Result<()> {
        (self.check)()?;
        self.writer.write_all(b"}")
    }
}
fn write_structure_wire(
    writer: &mut impl std::io::Write,
    node: &SemanticStructure,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut out = WireObject::new(writer, check)?;
    out.field("structure_id", &node.structure_id)?;
    out.field("kind", &node.kind)?;
    out.field("root", &node.root)?;
    if let Some(value) = &node.parent_structure_id {
        out.field("parent_structure_id", value)?;
    }
    out.field("name", &node.name)?;
    if !node.members.is_empty() {
        out.field("members", &node.members)?;
    }
    if !node.groups.is_empty() {
        out.field("groups", &node.groups)?;
    }
    if !node.properties.is_empty() {
        out.properties("properties", &node.properties)?;
    }
    out.field("content_trust", &node.content_trust)?;
    out.field("provenance", &node.provenance)?;
    out.field("evidence", &node.evidence)?;
    out.finish()
}
fn write_v6_node_wire(
    writer: &mut impl std::io::Write,
    node: &oryn_common::v6::SemanticNode,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    // Flattened v3 semantic has no JSON values or recursive record ownership.
    let mut out = WireObject::new(writer, check)?;
    write_v3_semantic_fields(&mut out, &node.semantic)?;
    if let Some(value) = &node.frame_id {
        out.field("frame_id", value)?;
    }
    if !node.relationships.is_empty() {
        out.field("relationships", &node.relationships)?;
    }
    if !node.accessibility_properties.is_empty() {
        out.properties("accessibility_properties", &node.accessibility_properties)?;
    }
    if !node.structure_ids.is_empty() {
        out.field("structure_ids", &node.structure_ids)?;
    }
    if let Some(value) = &node.geometry {
        out.field("geometry", value)?;
    }
    if !node.field_evidence.is_empty() {
        out.field("field_evidence", &node.field_evidence)?;
    }
    out.finish()
}
fn write_wire_array<T, W: std::io::Write, C: FnMut() -> std::io::Result<()>>(
    writer: &mut W,
    values: &[T],
    check: &mut C,
    mut element: impl FnMut(&mut W, &T, &mut C) -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(b"[")?;
    for (index, value) in values.iter().enumerate() {
        check()?;
        if index != 0 {
            writer.write_all(b",")?;
        }
        element(writer, value, check)?;
    }
    writer.write_all(b"]")
}
pub(crate) fn write_changes_wire(
    writer: &mut impl std::io::Write,
    changes: &oryn_common::v6::SemanticGraphChanges,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut out = WireObject::new(writer, check)?;
    if !changes.upserted_nodes.is_empty() {
        out.key("upserted_nodes")?;
        write_wire_array(
            out.writer,
            &changes.upserted_nodes,
            out.check,
            write_v6_node_wire,
        )?;
    }
    if !changes.removed_nodes.is_empty() {
        out.field("removed_nodes", &changes.removed_nodes)?;
    }
    if !changes.upserted_accessibility.is_empty() {
        out.key("upserted_accessibility")?;
        write_wire_array(
            out.writer,
            &changes.upserted_accessibility,
            out.check,
            write_accessibility_record_iterative,
        )?;
    }
    if !changes.removed_accessibility.is_empty() {
        out.field("removed_accessibility", &changes.removed_accessibility)?;
    }
    if !changes.upserted_structures.is_empty() {
        out.key("upserted_structures")?;
        write_wire_array(
            out.writer,
            &changes.upserted_structures,
            out.check,
            write_structure_wire,
        )?;
    }
    if !changes.removed_structures.is_empty() {
        out.field("removed_structures", &changes.removed_structures)?;
    }
    out.finish()
}
#[cfg(test)]
pub(crate) fn semantic_changes_bytes(
    changes: &oryn_common::v6::SemanticGraphChanges,
) -> std::io::Result<usize> {
    semantic_changes_bytes_with_checkpoint(changes, &mut || Ok(()))
}
pub(crate) fn semantic_changes_bytes_with_checkpoint(
    changes: &oryn_common::v6::SemanticGraphChanges,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("semantic size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    write_changes_wire(&mut counter, changes, check)?;
    Ok(counter.0)
}
#[cfg(test)]
mod changes_wire_controls {
    use super::*;
    #[test]
    fn deep_changes_successful_wire_and_count_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let owner = OwnedSemanticChanges::new(deep_changes_for_test());
                let mut bytes = Vec::new();
                write_changes_wire(&mut bytes, &owner, &mut || Ok(())).unwrap();
                assert_eq!(bytes.len(), semantic_changes_bytes(&owner).unwrap());
                assert!(bytes.len() > 32768);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn actual_changes_wire_parity() {
        let dom = crate::html::parse("<form><button>one</button></form>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        let owner = OwnedSemanticChanges::new(graph.take_v6_changes());
        let mut bytes = Vec::new();
        write_changes_wire(&mut bytes, &owner, &mut || Ok(())).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&owner).unwrap());
    }
}
pub(crate) fn write_observation_v6_wire(
    writer: &mut impl std::io::Write,
    node: &oryn_common::v6::SemanticObservation,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut out = WireObject::new(writer, check)?;
    out.field("contract_version", &node.contract_version)?;
    out.field("page", &node.page)?;
    out.field("revision", &node.revision)?;
    out.field("semantic_revision", &node.semantic_revision)?;
    out.field("profile", &node.profile)?;
    out.field("source", &node.source)?;
    out.key("nodes")?;
    write_wire_array(out.writer, &node.nodes, out.check, write_v6_node_wire)?;
    if !node.accessibility.is_empty() {
        out.key("accessibility")?;
        write_wire_array(
            out.writer,
            &node.accessibility,
            out.check,
            write_accessibility_record_iterative,
        )?;
    }
    if !node.structures.is_empty() {
        out.key("structures")?;
        write_wire_array(
            out.writer,
            &node.structures,
            out.check,
            write_structure_wire,
        )?;
    }
    out.field("projection", &node.projection)?;
    if !node.capabilities.is_empty() {
        out.field("capabilities", &node.capabilities)?;
    }
    if !node.diagnostics.is_empty() {
        out.field("diagnostics", &node.diagnostics)?;
    }
    out.field("execution_domain", &node.execution_domain)?;
    out.finish()
}
pub(crate) fn write_observation_v5_wire(
    writer: &mut impl std::io::Write,
    node: &oryn_common::v5::SemanticObservation,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut out = WireObject::new(writer, check)?;
    out.field("contract_version", &node.contract_version)?;
    out.field("observation", &node.observation)?;
    out.key("accessibility")?;
    write_wire_array(
        out.writer,
        &node.accessibility,
        out.check,
        write_accessibility_record_iterative,
    )?;
    out.key("structures")?;
    write_wire_array(
        out.writer,
        &node.structures,
        out.check,
        write_structure_wire,
    )?;
    if !node.diagnostics.is_empty() {
        out.field("diagnostics", &node.diagnostics)?;
    }
    out.finish()
}
pub(crate) fn write_delta_wire(
    writer: &mut impl std::io::Write,
    node: &oryn_common::v6::SemanticDelta,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut out = WireObject::new(writer, check)?;
    out.field("contract_version", &node.contract_version)?;
    out.field("page_id", &node.page_id)?;
    out.field("from_revision", &node.from_revision)?;
    out.field("to_revision", &node.to_revision)?;
    out.field("semantic_revision", &node.semantic_revision)?;
    out.key("changes")?;
    write_changes_wire(out.writer, &node.changes, out.check)?;
    if !node.diagnostics.is_empty() {
        out.field("diagnostics", &node.diagnostics)?;
    }
    out.finish()
}
fn write_json_cursor(
    writer: &mut impl std::io::Write,
    value: &serde_json::Value,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    enum Cursor<'a> {
        Array(std::slice::Iter<'a, serde_json::Value>, bool),
        Object(serde_json::map::Iter<'a>, bool),
    }
    let mut stack = Vec::new();
    let mut next = Some(value);
    loop {
        check()?;
        if let Some(value) = next.take() {
            match value {
                serde_json::Value::Array(values) => {
                    writer.write_all(b"[")?;
                    stack
                        .try_reserve(1)
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    stack.push(Cursor::Array(values.iter(), true));
                }
                serde_json::Value::Object(values) => {
                    writer.write_all(b"{")?;
                    stack
                        .try_reserve(1)
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    stack.push(Cursor::Object(values.iter(), true));
                }
                serde_json::Value::String(value) => {
                    write_json_string_checked(writer, value, check)?
                }
                value => serde_json::to_writer(&mut *writer, value)?,
            }
        }
        match stack.last_mut() {
            Some(Cursor::Array(values, first)) => {
                if let Some(value) = values.next() {
                    if !*first {
                        writer.write_all(b",")?;
                    }
                    *first = false;
                    next = Some(value);
                } else {
                    writer.write_all(b"]")?;
                    stack.pop();
                }
            }
            Some(Cursor::Object(values, first)) => {
                if let Some((key, value)) = values.next() {
                    if !*first {
                        writer.write_all(b",")?;
                    }
                    *first = false;
                    write_json_string_checked(writer, key, check)?;
                    writer.write_all(b":")?;
                    next = Some(value);
                } else {
                    writer.write_all(b"}")?;
                    stack.pop();
                }
            }
            None => break,
        }
    }
    check()
}
fn write_v3_semantic_fields<W: std::io::Write, C: FnMut() -> std::io::Result<()>>(
    out: &mut WireObject<'_, W, C>,
    node: &SemanticNodeView,
) -> std::io::Result<()> {
    out.field("semantic_ref", &node.semantic_ref)?;
    if let Some(value) = &node.parent {
        out.field("parent", value)?;
    }
    out.field("document_order", &node.document_order)?;
    out.field("alias", &node.alias)?;
    out.string("role", &node.role)?;
    out.string("name", &node.name)?;
    if let Some(value) = &node.selector {
        out.string("selector", value)?;
    }
    if let Some(value) = &node.value {
        out.string("value", value)?;
    }
    if let Some(value) = &node.description {
        out.string("description", value)?;
    }
    if let Some(value) = &node.validation {
        out.field("validation", value)?;
    }
    if !node.states.is_empty() {
        out.field("states", &node.states)?;
    }
    if !node.actions.is_empty() {
        out.field("actions", &node.actions)?;
    }
    out.field("content_trust", &node.content_trust)?;
    out.field("provenance", &node.provenance)?;
    Ok(())
}
impl SemanticGraph {
    pub(crate) fn pending_changes_with_checkpoint(
        &self,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<OwnedSemanticChanges, crate::dom::DomError> {
        self.shared_pending(&mut OrdinarySemanticProducer, check)
    }
}
#[cfg(test)]
mod pending_only_controls {
    use super::*;
    #[test]
    fn pending_builder_matches_take_and_failure_preserves_journal() {
        let dom = crate::html::parse("<form><button>one</button></form>")
            .into_runtime()
            .unwrap();
        let mut graph = SemanticGraph::new(PageId(31), 9);
        graph.reset(dom.arena(), 9);
        let before = format!("{graph:?}");
        let prepared = graph
            .pending_changes_with_checkpoint(&mut || Ok(()))
            .unwrap();
        let mut copy = graph.clone();
        let taken = OwnedSemanticChanges::new(copy.take_v6_changes());
        assert_eq!(
            serde_json::to_vec(&prepared).unwrap(),
            serde_json::to_vec(&taken).unwrap()
        );
        let mut calls = 0;
        let failed = graph.pending_changes_with_checkpoint(&mut || {
            calls += 1;
            if calls == 16 {
                Err(crate::dom::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(failed.is_err());
        assert_eq!(format!("{graph:?}"), before);
    }
}
#[cfg(test)]
mod verified_bytes_controls {
    use super::*;
    #[test]
    fn stored_size_matches_wire_and_avoids_recount_on_eviction() {
        let changes = oryn_common::v6::SemanticGraphChanges::default();
        let size = semantic_changes_bytes_with_checkpoint(&changes, &mut || Ok(())).unwrap();
        let owner = OwnedSemanticChanges::with_verified_bytes(changes, size);
        assert_eq!(owner.verified_bytes(), 2);
        let mut calls = 0;
        let failed = semantic_changes_bytes_with_checkpoint(&owner, &mut || {
            calls += 1;
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "retired",
            ))
        });
        assert!(failed.is_err());
        assert_eq!(calls, 1);
        assert_eq!(owner.verified_bytes(), size);
    }
    #[test]
    fn cursor_serializes_wide_payload_without_sibling_stack() {
        let value = serde_json::Value::Array((0..65536).map(|_| serde_json::Value::Null).collect());
        let mut bytes = Vec::new();
        write_json_cursor(&mut bytes, &value, &mut || Ok(())).unwrap();
        assert_eq!(bytes.len(), 327681);
        assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
    }
}
#[cfg(test)]
mod empty_wire_checkpoint_controls {
    use super::*;
    #[test]
    fn capacity_and_cancellation_are_not_defaulted() {
        for error in [
            crate::dom::DomError::ProjectionCancelled,
            crate::dom::DomError::SemanticProjectionCapacity,
        ] {
            assert!(cancellation_or_default::<String>(Err(error)).is_err());
        }
        assert_eq!(
            cancellation_or_default::<String>(Err(crate::dom::DomError::Cycle)).unwrap(),
            ""
        );
    }
    #[test]
    fn cancelled_empty_changes_never_writes_opening_token() {
        let changes = oryn_common::v6::SemanticGraphChanges::default();
        let mut bytes = Vec::new();
        let result = write_changes_wire(&mut bytes, &changes, &mut || {
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ))
        });
        assert!(result.is_err());
        assert!(bytes.is_empty());
    }
    #[test]
    fn empty_changes_final_checkpoint_is_enforced() {
        let changes = oryn_common::v6::SemanticGraphChanges::default();
        let mut bytes = Vec::new();
        let mut calls = 0;
        let result = write_changes_wire(&mut bytes, &changes, &mut || {
            calls += 1;
            if calls == 2 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "cancelled",
                ))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
        assert_eq!(bytes, b"{");
    }
}
impl SemanticGraph {
    fn projected_v6_with_checkpoint(
        &self,
        reference: SemanticRef,
        node: &SemanticNodeView,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<SemanticNodeV6, crate::dom::DomError> {
        self.shared_projected_v6(reference, node, &mut OrdinarySemanticProducer, check)
    }
}
impl OwnedSemanticChanges {
    pub(crate) fn set_verified_bytes(&mut self, bytes: usize) {
        self.1 = Some(bytes);
    }
    pub(crate) fn legacy_changes_with_checkpoint(
        &self,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<SemanticGraphChanges, crate::dom::DomError> {
        self.shared_legacy(&mut OrdinarySemanticProducer, check)
    }
}
impl SemanticGraph {
    pub(crate) fn commit_prepared_journal(&mut self) {
        // Caller owns exclusive &mut graph and has completed every fallible
        // preparation; clear each journal exactly once, with depth-safe custody.
        self.pending_upserts.clear();
        self.pending_removed.clear();
        for (_, mut node) in std::mem::take(&mut self.pending_accessibility_upserts) {
            dispose_owned_properties(&mut node.properties);
        }
        self.pending_accessibility_removed.clear();
        drop(StructureMapOwner(std::mem::take(
            &mut self.pending_structure_upserts,
        )));
        self.pending_structure_removed.clear();
    }
}
struct CheckedSerdeWriter<'a, W: std::io::Write, C: FnMut() -> std::io::Result<()>> {
    writer: &'a mut W,
    check: &'a mut C,
}
impl<W: std::io::Write, C: FnMut() -> std::io::Result<()>> std::io::Write
    for CheckedSerdeWriter<'_, W, C>
{
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        (self.check)()?;
        self.writer.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (self.check)()?;
        self.writer.flush()
    }
}
fn write_checked_serde<T: serde::Serialize + ?Sized>(
    writer: &mut impl std::io::Write,
    value: &T,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    serde_json::to_writer(CheckedSerdeWriter { writer, check }, value)?;
    Ok(())
}

pub(crate) enum SemanticPreparationError {
    Dom(crate::dom::DomError),
    Host(crate::host::HostError),
    Admission(String),
    Allocation(std::collections::TryReserveError),
    Capacity,
}
impl std::fmt::Debug for SemanticPreparationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dom(error) => f.debug_tuple("Dom").field(error).finish(),
            Self::Host(error) => f.debug_tuple("Host").field(error).finish(),
            Self::Admission(error) => f.debug_tuple("Admission").field(error).finish(),
            Self::Allocation(error) => f.debug_tuple("Allocation").field(error).finish(),
            Self::Capacity => f.write_str("Capacity"),
        }
    }
}
impl From<crate::dom::DomError> for SemanticPreparationError {
    fn from(value: crate::dom::DomError) -> Self {
        Self::Dom(value)
    }
}
struct SemanticLeaseOwner<'a> {
    admission: &'a dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    leases: Vec<SemanticAllocationReceipt>,
}
struct SemanticAllocationReceipt {
    payload: Option<std::sync::Arc<dyn Send + Sync>>,
    _metadata: Option<std::sync::Arc<dyn Send + Sync>>,
}

struct SemanticMetadataAdmission<'a> {
    admission: &'a dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    receipt: Option<std::sync::Arc<dyn Send + Sync>>,
}
impl crate::dom::projection_allocation::ProjectionAllocation for SemanticMetadataAdmission<'_> {
    type Error = SemanticPreparationError;
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
        if bytes >= 1024 * 1024
            && std::env::var("ORYN_NATIVE_ALLOCATION_DIAGNOSTICS").ok().as_deref() == Some("1")
        {
            static EMITTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            if EMITTED.fetch_update(std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed, |n| (n < 16).then_some(n + 1)).is_ok()
            {
                eprintln!("oryn-semantic-receipt-vector-growth bytes={bytes}");
            }
        }
        self.admission
            .checkpoint()
            .map_err(SemanticPreparationError::Admission)?;
        self.receipt = Some(
            self.admission
                .reserve(bytes)
                .map_err(SemanticPreparationError::Admission)?,
        );
        Ok(())
    }
    fn allocation_error(&self, error: std::collections::TryReserveError) -> Self::Error {
        SemanticPreparationError::Allocation(error)
    }
}

impl SemanticLeaseOwner<'_> {
    fn reserve(&mut self, bytes: usize) -> Result<(), SemanticPreparationError> {
        // Developer-only numeric source localization, using the existing
        // allocation diagnostic opt-in and bound; no semantic payload text.
        if bytes >= 1024 * 1024
            && std::env::var("ORYN_NATIVE_ALLOCATION_DIAGNOSTICS").ok().as_deref() == Some("1")
        {
            static EMITTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            if EMITTED.fetch_update(std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed, |n| (n < 16).then_some(n + 1)).is_ok()
            {
                eprintln!("oryn-semantic-reserve bytes={bytes} receipts={} receipt_capacity={}", self.leases.len(), self.leases.capacity());
            }
        }
        // Preserve canonical infrastructure/expired/retired error text as an
        // admission error. Do not manufacture owner cancellation from strings.
        self.admission
            .checkpoint()
            .map_err(SemanticPreparationError::Admission)?;
        let needed = self
            .leases
            .len()
            .checked_add(1)
            .ok_or(SemanticPreparationError::Capacity)?;
        // Admit payload first: a later metadata-allocation error drops this
        // temporary receipt, while a successful vector growth is immediately
        // followed by an infallible push retaining its metadata receipt.
        let receipt = self
            .admission
            .reserve(bytes)
            .map_err(SemanticPreparationError::Admission)?;
        // Reserve receipt-vector metadata through the same geometric engine
        // as every other producer. This adapter must not recurse into self.
        let mut metadata = SemanticMetadataAdmission {
            admission: self.admission,
            receipt: None,
        };
        crate::dom::projection_allocation::reserve_vector(&mut self.leases, needed, &mut metadata)?;
        let metadata_receipt = metadata.receipt;
        self.leases.push(SemanticAllocationReceipt {
            payload: Some(receipt),
            _metadata: metadata_receipt,
        });
        Ok(())
    }
    fn retain_on(
        mut self,
        result: &mut OwnedSemanticChanges,
    ) -> Result<(), SemanticPreparationError> {
        // Charge ownership travels with the returned private cache record.
        // This is a move, not a temporary scope that releases living payloads.
        // This producer creates a fresh unpublished owner. Refuse replacement
        // rather than silently releasing charges for an already-live payload.
        if !result.2.is_empty() {
            return Err(SemanticPreparationError::Admission(
                "allocation receipts already installed".into(),
            ));
        }
        result.2 = std::mem::take(&mut self.leases);
        Ok(())
    }
}

// staging source: shared-context-v40.rs
trait SemanticProducer {
    type Error: From<crate::dom::DomError>;
    fn receipt_mark(&self) -> usize {
        0
    }
    fn release_scratch(&mut self, _: usize) {}
    fn release_receipt_range(&mut self, _: std::ops::Range<usize>) {}
    fn ordinary_dom_error(error: Self::Error) -> Result<crate::dom::DomError, Self::Error>;
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error>;
    fn allocation_error(&self, error: std::collections::TryReserveError) -> Self::Error;
    fn text(
        &mut self,
        value: &str,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<String, Self::Error> {
        check().map_err(Self::Error::from)?;
        self.reserve(value.len())?;
        let mut result = String::new();
        result
            .try_reserve_exact(value.len())
            .map_err(|error| self.allocation_error(error))?;
        for character in value.chars() {
            check().map_err(Self::Error::from)?;
            result.push(character);
        }
        check().map_err(Self::Error::from)?;
        Ok(result)
    }
    fn vector<T>(&mut self, length: usize) -> Result<Vec<T>, Self::Error> {
        let bytes = length
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| Self::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
        self.reserve(bytes)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(length)
            .map_err(|error| self.allocation_error(error))?;
        Ok(result)
    }
}
struct OrdinarySemanticProducer;
impl SemanticProducer for OrdinarySemanticProducer {
    type Error = crate::dom::DomError;
    fn ordinary_dom_error(error: Self::Error) -> Result<crate::dom::DomError, Self::Error> {
        Ok(error)
    }
    fn reserve(&mut self, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
    fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
        crate::dom::DomError::SemanticProjectionCapacity
    }
}
impl SemanticProducer for SemanticLeaseOwner<'_> {
    type Error = SemanticPreparationError;
    fn receipt_mark(&self) -> usize {
        self.leases.len()
    }
    fn release_scratch(&mut self, mark: usize) {
        self.release_receipt_range(mark..self.leases.len())
    }
    fn release_receipt_range(&mut self, range: std::ops::Range<usize>) {
        let end = range.end.min(self.leases.len());
        let start = range.start.min(end);
        for receipt in &mut self.leases[start..end] {
            receipt.payload.take();
        }
        // Reclaim only a dead suffix. Interior indexes remain stable for
        // outstanding range marks and later live payloads. Capacity receipts
        // stay at their original index and continue charging the physical Vec.
        // Thus repeated scratch cycles reuse tail slots without dropping
        // metadata admission or compacting an externally bookmarked range.
        while self.leases.last().is_some_and(|receipt|
            receipt.payload.is_none() && receipt._metadata.is_none())
        {
            self.leases.pop();
        }
    }
    fn ordinary_dom_error(error: Self::Error) -> Result<crate::dom::DomError, Self::Error> {
        match error {
            SemanticPreparationError::Dom(error) => Ok(error),
            error => Err(error),
        }
    }
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
        SemanticLeaseOwner::reserve(self, bytes)
    }
    fn allocation_error(&self, error: std::collections::TryReserveError) -> Self::Error {
        SemanticPreparationError::Allocation(error)
    }
}
fn shared_provenance<P: SemanticProducer>(
    value: &Provenance,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Provenance, P::Error> {
    check().map_err(P::Error::from)?;
    Ok(match value {
        Provenance::BrowserDeterministic => Provenance::BrowserDeterministic,
        Provenance::Heuristic { confidence } => Provenance::Heuristic {
            confidence: *confidence,
        },
        Provenance::Model {
            confidence,
            model_id,
        } => Provenance::Model {
            confidence: *confidence,
            model_id: producer.text(model_id, check)?,
        },
    })
}
fn shared_semantic_node<P: SemanticProducer>(
    node: &SemanticNodeView,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<SemanticNodeView, P::Error> {
    check().map_err(P::Error::from)?;
    let mut states = BTreeSet::new();
    for state in &node.states {
        check().map_err(P::Error::from)?;
        states.insert(*state);
    }
    let mut actions = BTreeSet::new();
    for action in &node.actions {
        check().map_err(P::Error::from)?;
        actions.insert(*action);
    }
    let role = producer.text(&node.role, check)?;
    let name = producer.text(&node.name, check)?;
    let selector = node
        .selector
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let value = node
        .value
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let description = node
        .description
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let provenance = shared_provenance(&node.provenance, producer, check)?;
    check().map_err(P::Error::from)?;
    Ok(SemanticNodeView {
        semantic_ref: node.semantic_ref,
        parent: node.parent,
        document_order: node.document_order,
        alias: node.alias,
        role,
        name,
        selector,
        value,
        description,
        validation: node.validation.clone(),
        states,
        actions,
        content_trust: node.content_trust,
        provenance,
    })
}

fn shared_json<P: SemanticProducer>(
    value: &serde_json::Value,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<serde_json::Value, P::Error> {
    enum Frame<'a> {
        Array(std::slice::Iter<'a, serde_json::Value>),
        Object(serde_json::map::Iter<'a>, Option<String>),
    }
    let mut stack: Vec<(Frame<'_>, JsonValuesGuard)> = producer.vector(1)?;
    let mut next = Some(value);
    let mut completed = JsonValuesGuard(producer.vector(1)?);
    loop {
        check().map_err(P::Error::from)?;
        if let Some(value) = next.take() {
            match value {
                serde_json::Value::Array(values) => {
                    let children = producer.vector(values.len())?;
                    shared_vector_push(
                        &mut stack,
                        (Frame::Array(values.iter()), JsonValuesGuard(children)),
                        producer,
                        check,
                    )?;
                }
                serde_json::Value::Object(values) => {
                    let mut owner = producer.vector(1)?;
                    owner.push(serde_json::Value::Object(serde_json::Map::new()));
                    shared_vector_push(
                        &mut stack,
                        (Frame::Object(values.iter(), None), JsonValuesGuard(owner)),
                        producer,
                        check,
                    )?;
                }
                serde_json::Value::String(value) => completed
                    .0
                    .push(serde_json::Value::String(producer.text(value, check)?)),
                value => completed.0.push(value.clone()),
            }
        }
        if let Some(value) = completed.0.pop() {
            if let Some((frame, owner)) = stack.last_mut() {
                match frame {
                    Frame::Array(_) => owner.0.push(value),
                    Frame::Object(_, key) => {
                        if let serde_json::Value::Object(map) = &mut owner.0[0] {
                            map.insert(key.take().expect("visited object key"), value);
                        }
                    }
                }
            } else {
                return Ok(value);
            }
        }
        let Some((frame, owner)) = stack.last_mut() else {
            continue;
        };
        match frame {
            Frame::Array(values) => {
                if let Some(value) = values.next() {
                    next = Some(value);
                } else {
                    let values = std::mem::take(&mut owner.0);
                    stack.pop();
                    completed.0.push(serde_json::Value::Array(values));
                }
            }
            Frame::Object(values, key) => {
                if let Some((name, value)) = values.next() {
                    *key = Some(producer.text(name, check)?);
                    next = Some(value);
                } else {
                    if let Some(value) = owner.0.pop() {
                        completed.0.push(value);
                    }
                    stack.pop();
                }
            }
        }
    }
}

fn shared_evidence<P: SemanticProducer>(
    values: &[AccessibilityEvidence],
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<AccessibilityEvidence>, P::Error> {
    let mut result = producer.vector(values.len())?;
    for value in values {
        check().map_err(P::Error::from)?;
        let mut references = producer.vector(value.references.len())?;
        for reference in &value.references {
            check().map_err(P::Error::from)?;
            references.push(*reference);
        }
        result.push(AccessibilityEvidence {
            kind: value.kind.clone(),
            source: producer.text(&value.source, check)?,
            references,
        });
    }
    Ok(result)
}
fn shared_accessibility<P: SemanticProducer>(
    node: &AccessibilityNode,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<AccessibilityNode, P::Error> {
    check().map_err(P::Error::from)?;
    let mut relationships = BTreeMap::new();
    for (key, values) in &node.relationships {
        let mut refs = producer.vector(values.len())?;
        for value in values {
            check().map_err(P::Error::from)?;
            refs.push(*value);
        }
        relationships.insert(key.clone(), refs);
    }
    let mut properties = JsonPropertiesGuard(BTreeMap::new());
    for (key, value) in &node.properties {
        check().map_err(P::Error::from)?;
        properties.0.insert(
            producer.text(key, check)?,
            shared_json(value, producer, check)?,
        );
    }
    let exclusion_reason = node
        .exclusion_reason
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let role = producer.text(&node.role, check)?;
    let name = producer.text(&node.name, check)?;
    let description = node
        .description
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let provenance = shared_provenance(&node.provenance, producer, check)?;
    let evidence = shared_evidence(&node.evidence, producer, check)?;
    check().map_err(P::Error::from)?;
    Ok(AccessibilityNode {
        semantic_ref: node.semantic_ref,
        excluded: node.excluded,
        exclusion_reason,
        role,
        name,
        description,
        relationships,
        owned_by: node.owned_by,
        content_trust: node.content_trust,
        provenance,
        evidence,
        properties: std::mem::take(&mut properties.0),
    })
}
fn shared_structure<P: SemanticProducer>(
    node: &SemanticStructure,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<SemanticStructure, P::Error> {
    let structure_id = producer.text(&node.structure_id, check)?;
    let name = producer.text(&node.name, check)?;
    let parent_structure_id = node
        .parent_structure_id
        .as_ref()
        .map(|value| producer.text(value, check))
        .transpose()?;
    let mut members = producer.vector(node.members.len())?;
    for member in &node.members {
        check().map_err(P::Error::from)?;
        members.push(*member);
    }
    let mut groups = producer.vector(node.groups.len())?;
    for group in &node.groups {
        let mut members = producer.vector(group.members.len())?;
        for member in &group.members {
            check().map_err(P::Error::from)?;
            members.push(*member);
        }
        groups.push(SemanticGroup {
            kind: producer.text(&group.kind, check)?,
            name: group
                .name
                .as_ref()
                .map(|value| producer.text(value, check))
                .transpose()?,
            members,
        });
    }
    let mut properties = JsonPropertiesGuard(BTreeMap::new());
    for (key, value) in &node.properties {
        check().map_err(P::Error::from)?;
        properties.0.insert(
            producer.text(key, check)?,
            shared_json(value, producer, check)?,
        );
    }
    let provenance = shared_provenance(&node.provenance, producer, check)?;
    let evidence = shared_evidence(&node.evidence, producer, check)?;
    check().map_err(P::Error::from)?;
    Ok(SemanticStructure {
        structure_id,
        kind: node.kind,
        root: node.root,
        parent_structure_id,
        name,
        members,
        groups,
        content_trust: node.content_trust,
        provenance,
        evidence,
        properties: std::mem::take(&mut properties.0),
    })
}

impl SemanticGraph {
    fn shared_projected_v6<P: SemanticProducer>(
        &self,
        reference: SemanticRef,
        node: &SemanticNodeView,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<SemanticNodeV6, P::Error> {
        let id = unpack_node_ref(reference.node);
        let semantic = shared_semantic_node(node, producer, check)?;
        let mut relationships = BTreeMap::new();
        let mut properties = JsonPropertiesGuard(BTreeMap::new());
        let mut field_evidence = producer.vector(9)?;
        for field in [
            SemanticField::Role,
            SemanticField::State,
            SemanticField::Action,
            SemanticField::Visibility,
        ] {
            field_evidence.push(SemanticFieldEvidence {
                field,
                trust: ContentTrust::PageAuthored,
                provenance: Provenance::BrowserDeterministic,
                evidence: Vec::new(),
            });
        }
        if node.value.is_some() {
            field_evidence.push(SemanticFieldEvidence {
                field: SemanticField::Value,
                trust: ContentTrust::PageAuthored,
                provenance: Provenance::BrowserDeterministic,
                evidence: Vec::new(),
            });
        }
        let mut structure_ids = producer.vector(self.structures.len())?;
        for structure in self.structures.values() {
            check().map_err(P::Error::from)?;
            let mut contains = structure.root == reference;
            if !contains {
                for member in &structure.members {
                    check().map_err(P::Error::from)?;
                    if *member == reference {
                        contains = true;
                        break;
                    }
                }
            }
            if contains {
                structure_ids.push(producer.text(&structure.structure_id, check)?);
            }
        }
        if !structure_ids.is_empty() {
            field_evidence.push(SemanticFieldEvidence {
                field: SemanticField::Structure,
                trust: ContentTrust::PageAuthored,
                provenance: Provenance::BrowserDeterministic,
                evidence: Vec::new(),
            });
        }
        if let Some(accessible) = self.accessibility.get(&id) {
            for (key, values) in &accessible.relationships {
                let mut refs = producer.vector(values.len())?;
                for value in values {
                    check().map_err(P::Error::from)?;
                    refs.push(*value);
                }
                relationships.insert(key.clone(), refs);
            }
            for field in [
                Some(SemanticField::Name),
                accessible
                    .description
                    .as_ref()
                    .map(|_| SemanticField::Description),
            ]
            .into_iter()
            .flatten()
            {
                field_evidence.push(SemanticFieldEvidence {
                    field,
                    trust: ContentTrust::PageAuthored,
                    provenance: shared_provenance(&accessible.provenance, producer, check)?,
                    evidence: shared_evidence(&accessible.evidence, producer, check)?,
                });
            }
            if !accessible.relationships.is_empty() {
                field_evidence.push(SemanticFieldEvidence {
                    field: SemanticField::Relationship,
                    trust: ContentTrust::PageAuthored,
                    provenance: Provenance::BrowserDeterministic,
                    evidence: Vec::new(),
                });
            }
            for (key, value) in &accessible.properties {
                check().map_err(P::Error::from)?;
                properties.0.insert(
                    producer.text(key, check)?,
                    shared_json(value, producer, check)?,
                );
            }
        }
        check().map_err(P::Error::from)?;
        Ok(SemanticNodeV6 {
            semantic,
            frame_id: self.frame_ids.get(&id).copied(),
            relationships,
            structure_ids,
            geometry: None,
            field_evidence,
            accessibility_properties: std::mem::take(&mut properties.0),
        })
    }
}

impl SemanticGraph {
    fn shared_pending<P: SemanticProducer>(
        &self,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<OwnedSemanticChanges, P::Error> {
        let mut result = OwnedSemanticChanges::new(V6Changes::default());
        let changes = result.0.as_mut().unwrap();
        changes.upserted_nodes = producer.vector(self.pending_upserts.len())?;
        changes.removed_nodes = producer.vector(self.pending_removed.len())?;
        changes.upserted_accessibility =
            producer.vector(self.pending_accessibility_upserts.len())?;
        changes.removed_accessibility =
            producer.vector(self.pending_accessibility_removed.len())?;
        changes.upserted_structures = producer.vector(self.pending_structure_upserts.len())?;
        changes.removed_structures = producer.vector(self.pending_structure_removed.len())?;
        for node in self.pending_upserts.values() {
            check().map_err(P::Error::from)?;
            changes.upserted_nodes.push(self.shared_projected_v6(
                node.semantic_ref,
                node,
                producer,
                check,
            )?);
        }
        // Stable projection order matches existing native journal publication.
        let scratch_mark = producer.receipt_mark();
        let order = shared_order(&changes.upserted_nodes, producer, check, |left, right| {
            left.semantic
                .document_order
                .cmp(&right.semantic.document_order)
        })?;
        shared_reorder(&mut changes.upserted_nodes, order, producer, check)?;
        producer.release_scratch(scratch_mark);
        for reference in &self.pending_removed {
            check().map_err(P::Error::from)?;
            changes.removed_nodes.push(*reference);
        }
        for node in self.pending_accessibility_upserts.values() {
            changes
                .upserted_accessibility
                .push(shared_accessibility(node, producer, check)?);
        }
        let scratch_mark = producer.receipt_mark();
        let order = shared_order(
            &changes.upserted_accessibility,
            producer,
            check,
            |left, right| {
                let order = |node: &AccessibilityNode| {
                    self.full
                        .get(&unpack_node_ref(node.semantic_ref.node))
                        .map_or(u32::MAX, |node| node.document_order)
                };
                order(left).cmp(&order(right))
            },
        )?;
        shared_reorder(&mut changes.upserted_accessibility, order, producer, check)?;
        producer.release_scratch(scratch_mark);
        for reference in &self.pending_accessibility_removed {
            check().map_err(P::Error::from)?;
            changes.removed_accessibility.push(*reference);
        }
        for node in self.pending_structure_upserts.values() {
            changes
                .upserted_structures
                .push(shared_structure(node, producer, check)?);
        }
        for key in &self.pending_structure_removed {
            changes.removed_structures.push(producer.text(key, check)?);
        }
        check().map_err(P::Error::from)?;
        Ok(result)
    }
}
fn shared_order<T, P: SemanticProducer>(
    values: &[T],
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    mut compare: impl FnMut(&T, &T) -> std::cmp::Ordering,
) -> Result<Vec<usize>, P::Error> {
    let mut indices = producer.vector(values.len())?;
    for index in 0..values.len() {
        check().map_err(P::Error::from)?;
        indices.push(index);
    }
    let mut merged = producer.vector(indices.len())?;
    let mut width = 1usize;
    while width < indices.len() {
        merged.clear();
        let mut start = 0usize;
        while start < indices.len() {
            let middle = start.saturating_add(width).min(indices.len());
            let end = middle.saturating_add(width).min(indices.len());
            let (mut left, mut right) = (start, middle);
            while left < middle || right < end {
                check().map_err(P::Error::from)?;
                if right == end
                    || (left < middle
                        && compare(&values[indices[left]], &values[indices[right]])
                            != std::cmp::Ordering::Greater)
                {
                    merged.push(indices[left]);
                    left += 1;
                } else {
                    merged.push(indices[right]);
                    right += 1;
                }
            }
            start = end;
        }
        std::mem::swap(&mut indices, &mut merged);
        width = width.saturating_mul(2);
    }
    Ok(indices)
}
fn shared_reorder<T, P: SemanticProducer>(
    values: &mut [T],
    indices: Vec<usize>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<(), P::Error> {
    let mut positions = producer.vector(values.len())?;
    let mut locations = producer.vector(values.len())?;
    for index in 0..values.len() {
        check().map_err(P::Error::from)?;
        positions.push(index);
        locations.push(index);
    }
    for (target, original) in indices.into_iter().enumerate() {
        check().map_err(P::Error::from)?;
        let source = locations[original];
        let displaced = positions[target];
        values.swap(target, source);
        positions.swap(target, source);
        locations[original] = target;
        locations[displaced] = source;
    }
    check().map_err(P::Error::from)
}

impl SemanticGraph {
    pub(crate) fn prepare_admitted_pending(
        &self,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(OwnedSemanticChanges, SemanticGraphChanges), SemanticPreparationError> {
        let mut producer = SemanticLeaseOwner {
            admission,
            leases: Vec::new(),
        };
        let mut result = self.shared_pending(&mut producer, check)?;
        let legacy = result.shared_legacy(&mut producer, check)?;
        check()?;
        producer.retain_on(&mut result)?;
        Ok((result, legacy))
    }
}

impl OwnedSemanticChanges {
    fn shared_legacy<P: SemanticProducer>(
        &self,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<SemanticGraphChanges, P::Error> {
        let mut upserted = producer.vector(self.upserted_nodes.len())?;
        let mut removed = producer.vector(self.removed_nodes.len())?;
        for node in &self.upserted_nodes {
            upserted.push(shared_semantic_node(&node.semantic, producer, check)?);
        }
        for reference in &self.removed_nodes {
            check().map_err(P::Error::from)?;
            removed.push(*reference);
        }
        check().map_err(P::Error::from)?;
        Ok(SemanticGraphChanges { upserted, removed })
    }
}

#[cfg(test)]
mod shared_admission_controls {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Receipt(Arc<AtomicUsize>);
    impl Drop for Receipt {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Admission {
        live: Arc<AtomicUsize>,
        calls: AtomicUsize,
        fail: usize,
    }
    impl oryn_layout::cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail {
                return Err("expired authority control".into());
            }
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Receipt(self.live.clone())))
        }
    }
    fn fixture() -> SemanticGraph {
        let document=crate::html::parse("<form><fieldset><legend>Group</legend><label for=x>Name</label><input id=x value=hello><button>Go</button></fieldset></form>").into_runtime().unwrap();
        let mut graph = SemanticGraph::new(PageId(63), 7);
        graph.reset(document.arena(), 7);
        graph
    }
    #[test]
    fn admitted_projection_preserves_wire_and_receipt_custody() {
        let graph = fixture();
        let admission = Admission {
            live: Arc::new(AtomicUsize::new(0)),
            calls: AtomicUsize::new(0),
            fail: usize::MAX,
        };
        let ordinary = graph
            .pending_changes_with_checkpoint(&mut || Ok(()))
            .unwrap();
        let (admitted, legacy) = graph
            .prepare_admitted_pending(&admission, &mut || Ok(()))
            .unwrap();
        assert_eq!(
            serde_json::to_vec(&*ordinary).unwrap(),
            serde_json::to_vec(&*admitted).unwrap()
        );
        assert_eq!(
            ordinary
                .legacy_changes_with_checkpoint(&mut || Ok(()))
                .unwrap(),
            legacy
        );
        assert!(admission.live.load(Ordering::SeqCst) > 0);
        drop(legacy);
        assert!(admission.live.load(Ordering::SeqCst) > 0);
        drop(admitted);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn admission_failure_preserves_journal_and_releases_every_receipt() {
        let graph = fixture();
        let before = format!("{graph:?}");
        let probe = Admission {
            live: Arc::new(AtomicUsize::new(0)),
            calls: AtomicUsize::new(0),
            fail: usize::MAX,
        };
        drop(
            graph
                .prepare_admitted_pending(&probe, &mut || Ok(()))
                .unwrap(),
        );
        let total = probe.calls.load(Ordering::SeqCst);
        for boundary in 1..=total {
            let admission = Admission {
                live: Arc::new(AtomicUsize::new(0)),
                calls: AtomicUsize::new(0),
                fail: boundary,
            };
            assert!(matches!(
                graph.prepare_admitted_pending(&admission, &mut || Ok(())),
                Err(SemanticPreparationError::Admission(_))
            ));
            assert_eq!(admission.live.load(Ordering::SeqCst), 0);
            assert_eq!(format!("{graph:?}"), before);
        }
    }
}

// staging source: shared-default-error-v94.rs
fn shared_default<T: Default, P: SemanticProducer>(
    result: Result<T, P::Error>,
) -> Result<T, P::Error> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match P::ordinary_dom_error(error) {
            Ok(crate::dom::DomError::ProjectionCancelled) => {
                Err(P::Error::from(crate::dom::DomError::ProjectionCancelled))
            }
            Ok(crate::dom::DomError::SemanticProjectionCapacity) => Err(P::Error::from(
                crate::dom::DomError::SemanticProjectionCapacity,
            )),
            Ok(_) => Ok(T::default()),
            Err(error) => Err(error),
        },
    }
}
// Add to SemanticProducer: fn ordinary_dom_error(error:Self::Error)
//     ->Result<crate::dom::DomError,Self::Error>;
// Ordinary implementation returns Ok(error).
// Admitted implementation matches SemanticPreparationError::Dom(error)=>Ok(error),
// all Host/Admission/Allocation/Capacity variants=>Err(original).

// staging source: native-dom-adapter-v66.rs
struct SemanticDomAllocation<'a, P: SemanticProducer>(&'a mut P);
impl<P: SemanticProducer> crate::dom::projection_allocation::ProjectionAllocation
    for SemanticDomAllocation<'_, P>
{
    type Error = P::Error;
    fn scratch_mark(&self) -> usize {
        self.0.receipt_mark()
    }
    fn release_scratch(&mut self, mark: usize) {
        self.0.release_scratch(mark)
    }
    fn release_scratch_range(&mut self, range: std::ops::Range<usize>) {
        self.0.release_receipt_range(range)
    }
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
        self.0.reserve(bytes)
    }
    fn allocation_error(&self, error: std::collections::TryReserveError) -> Self::Error {
        self.0.allocation_error(error)
    }
}
fn shared_dom_text<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    dom.text_content_with_allocation(id, &mut SemanticDomAllocation(producer), check)
}
fn shared_select_options<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    dom.select_options_with_allocation(id, &mut SemanticDomAllocation(producer), check)
}

// staging source: shared-geometric-producers-v132.rs
fn shared_vector_push<T, P: SemanticProducer>(
    values: &mut Vec<T>,
    value: T,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<(), P::Error> {
    check().map_err(P::Error::from)?;
    let needed = values
        .len()
        .checked_add(1)
        .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
    crate::dom::projection_allocation::reserve_vector(
        values,
        needed,
        &mut SemanticDomAllocation(producer),
    )?;
    values.push(value);
    Ok(())
}

// staging source: shared-css-strip-v46.rs
fn shared_strip_css_comments<P: SemanticProducer>(
    sheet: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check().map_err(P::Error::from)?;
    producer.reserve(sheet.len())?;
    let mut output = String::new();
    output
        .try_reserve_exact(sheet.len())
        .map_err(|error| producer.allocation_error(error))?;
    let mut chars = sheet.chars().peekable();
    let mut comment = false;
    while let Some(character) = chars.next() {
        check().map_err(P::Error::from)?;
        if comment {
            if character == '*' && chars.peek() == Some(&'/') {
                check().map_err(P::Error::from)?;
                chars.next();
                comment = false;
            }
        } else if character == '/' && chars.peek() == Some(&'*') {
            check().map_err(P::Error::from)?;
            chars.next();
            comment = true;
        } else {
            output.push(character);
        }
    }
    Ok(output)
}

// staging source: shared-lowercase-v50.rs
fn shared_ascii_lowercase<P: SemanticProducer>(
    value: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check().map_err(P::Error::from)?;
    producer.reserve(value.len())?;
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|error| producer.allocation_error(error))?;
    for character in value.chars() {
        check().map_err(P::Error::from)?;
        result.push(character.to_ascii_lowercase());
    }
    check().map_err(P::Error::from)?;
    Ok(result)
}

// staging source: shared-surround-v50.rs
fn shared_surround<P: SemanticProducer>(
    prefix: &str,
    value: &str,
    suffix: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let length = prefix
        .len()
        .checked_add(value.len())
        .and_then(|n| n.checked_add(suffix.len()))
        .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
    check().map_err(P::Error::from)?;
    producer.reserve(length)?;
    let mut result = String::new();
    result
        .try_reserve_exact(length)
        .map_err(|error| producer.allocation_error(error))?;
    for part in [prefix, value, suffix] {
        for character in part.chars() {
            check().map_err(P::Error::from)?;
            result.push(character);
        }
    }
    check().map_err(P::Error::from)?;
    Ok(result)
}

// staging source: shared-join-v53.rs
fn shared_join<P: SemanticProducer>(
    values: &[String],
    separator: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let mut size = 0usize;
    for (index, value) in values.iter().enumerate() {
        check().map_err(P::Error::from)?;
        if index > 0 {
            size = size
                .checked_add(separator.len())
                .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
        }
        size = size
            .checked_add(value.len())
            .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
    }
    producer.reserve(size)?;
    let mut result = String::new();
    result
        .try_reserve_exact(size)
        .map_err(|error| producer.allocation_error(error))?;
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            shared_append_text(&mut result, separator, producer, check)?;
        }
        shared_append_text(&mut result, value, producer, check)?;
    }
    check().map_err(P::Error::from)?;
    Ok(result)
}

// staging source: shared-basic-roles-v86.rs
fn shared_first_supported_role_with_checkpoint<P: SemanticProducer>(
    value: Option<&String>,
    exclude_named: bool,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<String>, P::Error> {
    let Some(value) = value else {
        check()?;
        return Ok(None);
    };
    let tokens_start = producer.receipt_mark();
    let tokens = shared_name_tokens(
        value,
        |character| character.is_ascii_whitespace(),
        producer,
        check,
    )?;
    let tokens_end = producer.receipt_mark();
    for token in &tokens {
        check()?;
        if token.is_empty() {
            continue;
        }
        let role_start = producer.receipt_mark();
        let role = shared_ascii_lowercase(token, producer, check)?;
        if supported_aria_role(&role)
            && (!exclude_named || !matches!(role.as_str(), "form" | "region"))
        {
            check()?;
            drop(tokens);
            producer.release_receipt_range(tokens_start..tokens_end);
            return Ok(Some(role));
        }
        drop(role);
        producer.release_scratch(role_start);
    }
    drop(tokens);
    producer.release_receipt_range(tokens_start..tokens_end);
    check()?;
    Ok(None)
}

fn shared_implicit_role_with_checkpoint<P: SemanticProducer>(
    local_name: &str,
    attributes: &oryn_dom::AttributeList,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    let role = match local_name {
        "a" | "area" if attributes.contains_null_with_checkpoint("href", check)? => "link",
        "input" => match attributes
            .get_null_with_checkpoint("type", check)?
            .map(|kind| shared_ascii_lowercase(kind, producer, check))
            .transpose()?
            .as_deref()
        {
            Some("checkbox") => "checkbox",
            Some("radio") => "radio",
            Some("range") => "slider",
            Some("number") => "spinbutton",
            Some("search") => "searchbox",
            Some("button" | "submit" | "reset" | "file" | "image") => "button",
            Some("color") => "colorwell",
            _ => "textbox",
        },
        "textarea" => "textbox",
        "select"
            if attributes.contains_null_with_checkpoint("multiple", check)?
                || attributes
                    .get_null_with_checkpoint("size", check)?
                    .map(|value| {
                        naming_u64_with_checkpoint(value, check)
                            .map(|value| value.and_then(|n| u32::try_from(n).ok()))
                    })
                    .transpose()?
                    .flatten()
                    .is_some_and(|size| size > 1) =>
        {
            "listbox"
        }
        "select" => "combobox",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "output" => "status",
        "img"
            if attributes
                .get_null_with_checkpoint("alt", check)?
                .is_some_and(String::is_empty) =>
        {
            "none"
        }
        "img" => "image",
        "nav" => "navigation",
        "main" => "main",
        "aside" => "complementary",
        "header" => "banner",
        "footer" => "contentinfo",
        "form"
            if attributes.contains_null_with_checkpoint("aria-label", check)?
                || attributes.contains_null_with_checkpoint("aria-labelledby", check)? =>
        {
            "form"
        }
        "form" => "generic",
        "ul" | "ol" | "menu" | "dir" => "list",
        "li" => "listitem",
        "table" => "table",
        "tr" => "row",
        "th" => match attributes
            .get_null_with_checkpoint("scope", check)?
            .map(|scope| shared_ascii_lowercase(scope, producer, check))
            .transpose()?
            .as_deref()
        {
            Some("row") | Some("rowgroup") => "rowheader",
            _ => "columnheader",
        },
        "td" => "cell",
        "dialog" => "dialog",
        "summary" => "button",
        "option" => "option",
        "progress" => "progressbar",
        "search" => "search",
        "details" => "group",
        "datalist" => "listbox",
        "blockquote" => "blockquote",
        "caption" => "caption",
        "code" => "code",
        "del" | "s" => "deletion",
        "em" => "emphasis",
        "ins" => "insertion",
        "p" => "paragraph",
        "strong" => "strong",
        "sub" => "subscript",
        "sup" => "superscript",
        "time" => "time",
        "address" | "fieldset" | "hgroup" | "optgroup" => "group",
        "cite" => "html-cite",
        "dd" => "definition",
        "dt" | "dfn" => "term",
        "hr" => "separator",
        "thead" | "tbody" | "tfoot" => "rowgroup",
        "button" | "article" | "figure" | "mark" | "meter" => local_name,
        "div" | "span" => "generic",
        "section"
            if attributes.contains_null_with_checkpoint("aria-label", check)?
                || attributes.contains_null_with_checkpoint("aria-labelledby", check)? =>
        {
            "region"
        }
        _ => "generic",
    };
    producer.text(role, check)
}

fn shared_explicit_role_value_with_checkpoint<P: SemanticProducer>(
    local_name: &str,
    attributes: &oryn_dom::AttributeList,
    role: String,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    let mut global = false;
    for entry in attributes.records() {
        check()?;
        if entry.namespace.is_none() && is_global_aria_attribute(&entry.local_name) {
            global = true;
            break;
        }
    }
    let conflict = matches!(role.as_str(), "none" | "presentation")
        && (matches!(local_name, "button" | "input" | "select" | "textarea")
            || (local_name == "a" && attributes.contains_null_with_checkpoint("href", check)?)
            || attributes.contains_null_with_checkpoint("tabindex", check)?
            || global);
    if conflict {
        return shared_implicit_role_with_checkpoint(local_name, attributes, producer, check);
    }
    match role.as_str() {
        "img" | "image" => producer.text("image", check),
        "directory" => producer.text("list", check),
        "presentation" => producer.text("none", check),
        _ => Ok(role),
    }
}

fn shared_computed_role_with_checkpoint<P: SemanticProducer>(
    local_name: &str,
    attributes: &oryn_dom::AttributeList,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    if let Some(role) = shared_first_supported_role_with_checkpoint(
        attributes.get_null_with_checkpoint("role", check)?,
        false,
        producer,
        check,
    )? && (!matches!(role.as_str(), "form" | "region")
        || attributes.contains_null_with_checkpoint("aria-label", check)?
        || attributes.contains_null_with_checkpoint("aria-labelledby", check)?)
    {
        return shared_explicit_role_value_with_checkpoint(
            local_name, attributes, role, producer, check,
        );
    }
    shared_implicit_role_with_checkpoint(local_name, attributes, producer, check)
}

// staging source: shared-lineage-v135.rs
fn shared_descendants_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    root: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    check()?;
    let mut pending = producer.vector(0)?;
    if let Ok(node) = dom.get(root) {
        for child in node.children.iter().rev() {
            check()?;
            shared_vector_push(&mut pending, *child, producer, check)?;
        }
    }
    let mut output = producer.vector(0)?;
    let mut seen = BTreeSet::from([root]);
    while let Some(id) = pending.pop() {
        check()?;
        if !seen.insert(id) {
            return Err(P::Error::from(crate::dom::DomError::Cycle));
        }
        shared_vector_push(&mut output, id, producer, check)?;
        if let Ok(node) = dom.get(id) {
            for child in node.children.iter().rev() {
                check()?;
                shared_vector_push(&mut pending, *child, producer, check)?;
            }
        }
    }
    check()?;
    Ok(output)
}

fn shared_ancestors_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    node: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    check()?;
    let mut output = producer.vector(0)?;
    let mut seen = BTreeSet::from([node]);
    let mut parent = dom.get(node)?.parent;
    while let Some(id) = parent {
        check()?;
        if !seen.insert(id) {
            return Err(P::Error::from(crate::dom::DomError::Cycle));
        }
        shared_vector_push(&mut output, id, producer, check)?;
        parent = dom.get(id)?.parent;
    }
    Ok(output)
}

fn shared_copy_name_children<P: SemanticProducer>(
    children: &[NodeId],
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    let mut result = producer.vector(children.len())?;
    for child in children {
        check()?;
        result.push(*child);
    }
    Ok(result)
}

fn shared_accessible_children_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(Vec::new());
    };
    for child in &node.children {
        check()?;
        if let Ok(root) = dom.get(*child)
            && matches!(root.kind, NodeKind::ShadowRoot { .. })
        {
            return shared_copy_name_children(&root.children, producer, check);
        }
    }
    let slot = matches!(&node.kind, NodeKind::Element { name }
        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot");
    if slot && let Some(assigned) = &node.native_resolved_slot_nodes {
        return shared_copy_name_children(
            if assigned.is_empty() {
                &node.children
            } else {
                assigned
            },
            producer,
            check,
        );
    }
    if slot
        && let Some(root_id) = tree_scope_with_checkpoint(dom, id, check)?
        && let Ok(root) = dom.get(root_id)
        && matches!(root.kind, NodeKind::ShadowRoot { .. })
        && let Some(host) = root.parent.and_then(|host| dom.get(host).ok())
    {
        let slot_name = node
            .attributes
            .get_null_with_checkpoint("name", check)?
            .map(String::as_str)
            .unwrap_or("");
        let mut first_slot = None;
        for candidate in shared_descendants_with_checkpoint(dom, root_id, producer, check)? {
            check()?;
            if tree_scope_with_checkpoint(dom, candidate, check)? == Some(root_id)
                && let Ok(candidate_node) = dom.get(candidate)
                && matches!(&candidate_node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="slot")
                && naming_equal_with_checkpoint(
                    candidate_node
                        .attributes
                        .get_null_with_checkpoint("name", check)?
                        .map(String::as_str)
                        .unwrap_or(""),
                    slot_name,
                    check,
                )?
            {
                first_slot = Some(candidate);
                break;
            }
        }
        if first_slot == Some(id) {
            let mut assigned = producer.vector(0)?;
            for child in &host.children {
                check()?;
                if let Ok(child_node) = dom.get(*child)
                    && matches!(
                        child_node.kind,
                        NodeKind::Element { .. } | NodeKind::Text { .. }
                    )
                    && naming_equal_with_checkpoint(
                        child_node
                            .attributes
                            .get_null_with_checkpoint("slot", check)?
                            .map(String::as_str)
                            .unwrap_or(""),
                        slot_name,
                        check,
                    )?
                {
                    shared_vector_push(&mut assigned, *child, producer, check)?;
                }
            }
            if !assigned.is_empty() {
                return Ok(assigned);
            }
        }
    }
    shared_copy_name_children(&node.children, producer, check)
}

// staging source: shared-visibility-v105.rs
fn shared_subtree_excluded_at_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(false);
    };
    let aria = node
        .accessibility_attribute_with_checkpoint("aria-hidden", check)?
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if let Some((display_none, _)) = dom.accessibility_computed_visibility(id) {
        return Ok(display_none || aria);
    }
    if node
        .attributes
        .contains_null_with_checkpoint("hidden", check)?
        || aria
    {
        return Ok(true);
    }
    if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
        for declaration in shared_name_tokens(style, |character| character == ';', producer, check)?
        {
            let tokens =
                shared_name_tokens(declaration, |character| character == ':', producer, check)?;
            if tokens.len() < 2 {
                continue;
            }
            if naming_trim_with_checkpoint(tokens[0], check)?.eq_ignore_ascii_case("display") {
                return Ok(naming_trim_with_checkpoint(
                    &declaration[tokens[0].len() + 1..],
                    check,
                )?
                .eq_ignore_ascii_case("none"));
            }
        }
    }
    Ok(false)
}

fn shared_naming_visibility_hidden_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    check()?;
    if let Some((_, hidden)) = dom.accessibility_computed_visibility(id) {
        return Ok(hidden);
    }
    let Ok(node) = dom.get(id) else {
        return Ok(false);
    };
    if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
        for declaration in shared_name_tokens(style, |character| character == ';', producer, check)?
        {
            let tokens =
                shared_name_tokens(declaration, |character| character == ':', producer, check)?;
            if tokens.len() > 1
                && naming_trim_with_checkpoint(tokens[0], check)?.eq_ignore_ascii_case("visibility")
            {
                let value =
                    naming_trim_with_checkpoint(&declaration[tokens[0].len() + 1..], check)?;
                return Ok(
                    value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
                );
            }
        }
    }
    Ok(false)
}

fn shared_accessibility_hidden_at_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    Ok(
        shared_subtree_excluded_at_with_checkpoint(dom, id, producer, check)?
            || shared_naming_visibility_hidden_with_checkpoint(dom, id, producer, check)?,
    )
}

fn shared_visibility_hidden_at_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    Ok(
        !shared_subtree_excluded_at_with_checkpoint(dom, id, producer, check)?
            && shared_naming_visibility_hidden_with_checkpoint(dom, id, producer, check)?,
    )
}

fn shared_inline_style_value_with_checkpoint<'a, P: SemanticProducer>(
    node: &'a crate::dom::Node,
    property: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<&'a str>, P::Error> {
    check()?;
    let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? else {
        return Ok(None);
    };
    for declaration in shared_name_tokens(style, |character| character == ';', producer, check)? {
        let parts = shared_name_tokens(declaration, |character| character == ':', producer, check)?;
        if parts.len() < 2 {
            continue;
        }
        if naming_trim_with_checkpoint(parts[0], check)?.eq_ignore_ascii_case(property) {
            return Ok(Some(naming_trim_with_checkpoint(
                &declaration[parts[0].len() + 1..],
                check,
            )?));
        }
    }
    check()?;
    Ok(None)
}

fn shared_accessible_text_run_separator_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(false);
    };
    let NodeKind::Element { name } = &node.kind else {
        return Ok(false);
    };
    if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        && matches!(name.local.as_ref(), "br" | "input" | "select" | "textarea")
    {
        return Ok(true);
    }
    if let Some((display, _)) = dom.accessibility_computed_text_style(id) {
        return Ok(!matches!(display, "inline" | "contents" | "none"));
    }
    if let Some(display) =
        shared_inline_style_value_with_checkpoint(node, "display", producer, check)?
    {
        return Ok(!matches!(
            naming_trim_with_checkpoint(display, check)?,
            "inline" | "contents" | "none"
        ));
    }
    Ok(name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        && matches!(
            name.local.as_ref(),
            "div"
                | "p"
                | "section"
                | "article"
                | "aside"
                | "header"
                | "footer"
                | "main"
                | "nav"
                | "li"
                | "ul"
                | "ol"
                | "blockquote"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "table"
                | "tr"
                | "td"
                | "th"
        ))
}

// staging source: shared-root-hidden-v107.rs
fn shared_referenced_root_hidden<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let mut cursor = Some(id);
    let mut seen = BTreeSet::new();
    let mut visibility = None;
    while let Some(id) = cursor {
        check()?;
        if !seen.insert(id) {
            break;
        }
        let Ok(node) = dom.get(id) else {
            break;
        };
        if shared_subtree_excluded_at_with_checkpoint(dom, id, producer, check)? {
            return Ok(true);
        }
        if visibility.is_none() {
            if let Some((_, hidden)) = dom.accessibility_computed_visibility(id) {
                visibility = Some(hidden);
            } else if let Some(value) =
                shared_inline_style_value_with_checkpoint(node, "visibility", producer, check)?
            {
                if value.eq_ignore_ascii_case("visible") {
                    visibility = Some(false);
                } else if value.eq_ignore_ascii_case("hidden")
                    || value.eq_ignore_ascii_case("collapse")
                {
                    visibility = Some(true);
                }
            }
        }
        cursor = node.parent;
    }
    Ok(visibility.unwrap_or(false))
}

// staging source: shared-case-text-v133.rs
fn shared_case_text<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    data: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    let scratch_start = producer.receipt_mark();
    let mut lineage = Vec::new();
    let mut cursor = dom.get(id).ok().and_then(|node| node.parent);
    let mut ancestors_seen = BTreeSet::from([id]);
    while let Some(parent) = cursor {
        check()?;
        if !ancestors_seen.insert(parent) {
            lineage.clear();
            break;
        }
        shared_vector_push(&mut lineage, parent, producer, check)?;
        cursor = dom.get(parent).ok().and_then(|node| node.parent);
    }
    let mut transform = None;
    let mut language = None;
    let mut root = None;
    for parent in &lineage {
        check()?;
        if transform.is_none() {
            if let Some((_, value)) = dom.accessibility_computed_text_style(*parent) {
                transform = Some(producer.text(value, check)?);
            } else if let Ok(node) = dom.get(*parent)
                && let Some(value) = shared_inline_style_value_with_checkpoint(
                    node,
                    "text-transform",
                    producer,
                    check,
                )?
            {
                transform = Some(producer.text(value, check)?);
            }
        }
        if language.is_none()
            && let Ok(node) = dom.get(*parent)
            && let Some(value) = node.attributes.get_null_with_checkpoint("lang", check)?
        {
            language = Some(producer.text(value, check)?);
        }
        if root.is_none()
            && shared_accessible_text_run_separator_with_checkpoint(dom, *parent, producer, check)?
        {
            root = Some(*parent);
        }
    }
    let transform = match transform {
        Some(value) => value,
        None => producer.text("none", check)?,
    };
    let mut transforms = false;
    for token in shared_name_tokens(
        &transform,
        |character| character.is_ascii_whitespace(),
        producer,
        check,
    )? {
        check()?;
        transforms |= matches!(token, "uppercase" | "lowercase" | "capitalize");
    }
    if !transforms {
        let scratch_end = producer.receipt_mark();
        let result = producer.text(data, check)?;
        drop(lineage);
        drop(transform);
        drop(language);
        producer.release_receipt_range(scratch_start..scratch_end);
        return Ok(result);
    }
    let language = language.unwrap_or_default();
    let root = root.or_else(|| lineage.last().copied()).unwrap_or(id);
    let mut pending = producer.vector(1)?;
    pending.push((root, false));
    let mut seen = BTreeSet::new();
    let mut context = String::new();
    let mut offset = 0;
    let mut range = None;
    while let Some((node_id, after)) = pending.pop() {
        check()?;
        if after {
            shared_append_text(&mut context, " ", producer, check)?;
            offset += 1;
            continue;
        }
        if !seen.insert(node_id) {
            continue;
        }
        let Ok(node) = dom.get(node_id) else {
            continue;
        };
        if matches!(&node.kind,NodeKind::Element{name}if matches!(name.local.as_ref(),"script"|"style"|"template"))
        {
            continue;
        }
        if let NodeKind::Text { data } = &node.kind {
            let mut end = offset;
            for ch in data.chars() {
                check()?;
                let mut buffer = [0u8; 4];
                shared_append_text(&mut context, ch.encode_utf8(&mut buffer), producer, check)?;
                end += 1;
            }
            if node_id == id {
                range = Some((offset, end));
            }
            offset = end;
        } else {
            if node_id != root
                && shared_accessible_text_run_separator_with_checkpoint(
                    dom, node_id, producer, check,
                )?
            {
                shared_append_text(&mut context, " ", producer, check)?;
                offset += 1;
                shared_vector_push(&mut pending, (node_id, true), producer, check)?;
            }
            for child in shared_accessible_children_with_checkpoint(dom, node_id, producer, check)?
                .into_iter()
                .rev()
            {
                shared_vector_push(&mut pending, (child, false), producer, check)?;
            }
        }
    }
    let scratch_end = producer.receipt_mark();
    let result = if let Some((start, end)) = range {
        shared_case_fragment(&context, start, end, &transform, &language, producer, check)?
    } else {
        let mut count = 0usize;
        for _ in data.chars() {
            check()?;
            count += 1;
        }
        shared_case_fragment(data, 0, count, &transform, &language, producer, check)?
    };
    drop(context);
    drop(pending);
    drop(lineage);
    drop(transform);
    drop(language);
    producer.release_receipt_range(scratch_start..scratch_end);
    check()?;
    Ok(result)
}

fn shared_case_fragment<P: SemanticProducer>(
    text: &str,
    start: usize,
    end: usize,
    transform: &str,
    language: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    oryn_layout::accessibility_case_fragment_with_allocation(
        text,
        start,
        end,
        transform,
        language,
        &mut SemanticDomAllocation(producer),
        check,
    )
    .map_err(|error| match error {
        oryn_layout::CasePreparationError::Checkpoint(error) => P::Error::from(error),
        oryn_layout::CasePreparationError::Allocation(error) => error,
    })
}

// staging source: shared-role-entry-v134.rs
fn shared_role_for_node<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    let role = shared_unpromoted_role_for_node_with_checkpoint(dom, id, producer, check)?;
    let mut promotion = false;
    if let Ok(node) = dom.get(id) {
        for attribute in ["autofocus", "draggable", "popover"] {
            check()?;
            if node
                .attributes
                .contains_null_with_checkpoint(attribute, check)?
            {
                promotion = true;
                break;
            }
        }
    }
    if matches!(role.as_str(), "generic" | "none") && promotion {
        producer.text("group", check)
    } else {
        Ok(role)
    }
}

// staging source: shared-text-entry-v69-v108.rs
fn shared_accessible_text<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    excluded: Option<NodeId>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let hidden = include_hidden_root && shared_referenced_root_hidden(dom, id, producer, check)?;
    shared_accessible_text_walk(
        dom,
        id,
        include_hidden_root,
        excluded,
        hidden,
        &mut NameTraversal::default(),
        producer,
        check,
    )
}

// staging source: shared-visible-text-v68-v103-v106-v108-v135.rs
fn shared_visible_text<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    excluded: Option<NodeId>,
    traversal: &mut NameTraversal,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    if Some(id) == excluded {
        return Ok(String::new());
    }
    let Ok(node) = dom.get(id) else {
        return Ok(String::new());
    };
    if shared_subtree_excluded_at_with_checkpoint(dom, id, producer, check)? {
        return Ok(String::new());
    }
    if dom
        .accessibility_computed_visibility(id)
        .is_some_and(|(_, hidden)| !hidden)
        || (dom.accessibility_computed_visibility(id).is_none()
            && shared_inline_style_value_with_checkpoint(node, "visibility", producer, check)?
                .is_some_and(|value| value.eq_ignore_ascii_case("visible")))
    {
        return shared_accessible_text_walk(
            dom,
            id,
            true,
            excluded,
            shared_referenced_root_hidden(dom, id, producer, check)?,
            traversal,
            producer,
            check,
        );
    }
    let mut parts = producer.vector(0)?;
    for child in &node.children {
        check()?;
        let value = shared_visible_text(dom, *child, excluded, traversal, producer, check)?;
        if !value.is_empty() {
            shared_vector_push(&mut parts, value, producer, check)?;
        }
    }
    shared_join(&parts, " ", producer, check)
}

// staging source: shared-accessible-walk-v57-v103-v106-v108-v135-v143.rs
#[expect(
    clippy::too_many_arguments,
    reason = "Recursive accname traversal keeps node/exclusion/hidden flags distinct from mutable traversal and producer checkpoints."
)]
fn shared_accessible_text_walk<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    excluded: Option<NodeId>,
    include_hidden_subtree: bool,
    traversal: &mut NameTraversal,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    if Some(id) == excluded {
        return if dom.get(id).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && matches!(name.local.as_ref(), "input" | "select" | "textarea"))) { producer.text(" ",check) } else { Ok(String::new()) };
    }
    let Ok(node) = dom.get(id) else {
        return Ok(String::new());
    };
    if matches!(&node.kind, NodeKind::Element { name }
        if (name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && matches!(name.local.as_ref(), "script" | "style" | "template"))
            || (name.ns.as_ref() == "http://www.w3.org/2000/svg" && matches!(name.local.as_ref(), "script" | "style")))
    {
        return Ok(String::new());
    }
    if !include_hidden_root
        && !include_hidden_subtree
        && shared_accessibility_hidden_at_with_checkpoint(dom, id, producer, check)?
    {
        if shared_visibility_hidden_at_with_checkpoint(dom, id, producer, check)? {
            let mut parts = producer.vector(0)?;
            for child in shared_accessible_children_with_checkpoint(dom, id, producer, check)? {
                let text = shared_visible_text(dom, child, excluded, traversal, producer, check)?;
                if !text.is_empty() {
                    shared_vector_push(&mut parts, text, producer, check)?;
                }
            }
            return shared_join(&parts, " ", producer, check);
        }
        return Ok(String::new());
    }
    if !traversal.seen.insert(id) {
        return Ok(String::new());
    }
    let is_shadow_slot = matches!(&node.kind, NodeKind::Element { name }
        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
        && tree_scope_with_checkpoint(dom, id, check)?.is_some_and(|root| {
            dom.get(root)
                .is_ok_and(|root| matches!(root.kind, NodeKind::ShadowRoot { .. }))
        });
    let role = shared_role_for_node(dom, id, producer, check)?;
    let embedded = matches!(&node.kind, NodeKind::Element { .. })
        && matches!(
            role.as_str(),
            "textbox" | "searchbox" | "combobox" | "listbox" | "menu" | "slider" | "spinbutton"
        );
    if !is_shadow_slot
        && !embedded
        && traversal.reference_depth == 0
        && let Some(targets) = shared_relation(dom, id, "aria-labelledby", producer, check)?
            .filter(|targets| !targets.is_empty())
    {
        traversal.reference_depth += 1;
        let result = (|| {
            let mut parts = producer.vector(0)?;
            let mut self_reference = false;
            for target in targets {
                check()?;
                if target == id && !self_reference {
                    traversal.seen.remove(&id);
                    self_reference = true;
                }
                let text = shared_accessible_text_walk(
                    dom,
                    target,
                    true,
                    excluded,
                    shared_referenced_root_hidden(dom, target, producer, check)?,
                    traversal,
                    producer,
                    check,
                )?;
                if !text.is_empty() {
                    shared_vector_push(&mut parts, text, producer, check)?;
                }
            }
            shared_normalize_text(&shared_join(&parts, " ", producer, check)?, producer, check)
        })();
        traversal.reference_depth -= 1;
        return result;
    }
    if !is_shadow_slot
        && !embedded
        && let Some(label) = node
            .accessibility_attribute_with_checkpoint("aria-label", check)?
            .map(|label| shared_normalize_text(label, producer, check))
            .transpose()?
            .filter(|label| !label.is_empty())
    {
        return Ok(label);
    }
    if let Some(title) =
        shared_svg_child_alternative_with_checkpoint(dom, id, "title", producer, check)?
    {
        return Ok(title);
    }
    match &node.kind {
        NodeKind::Text { data } => {
            return shared_case_text(dom, id, data, producer, check);
        }
        NodeKind::Element { .. } if role == "menu" => return Ok(String::new()),
        NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && name.local.as_ref() == "br" =>
        {
            return producer.text(" ", check);
        }
        NodeKind::Element { name } if name.local.as_ref() == "img" => {
            return match node.attributes.get_null_with_checkpoint("alt", check)? {
                Some(value) => producer.text(value, check),
                None => Ok(String::new()),
            };
        }
        NodeKind::Element { name } if name.local.as_ref() == "input" => {
            let kind = node
                .attributes
                .get_null_with_checkpoint("type", check)?
                .map(|kind| shared_ascii_lowercase(kind, producer, check))
                .transpose()?;
            let value = match node.value.as_ref() {
                Some(value) => Some(value),
                None => node.attributes.get_null_with_checkpoint("value", check)?,
            };
            if matches!(kind.as_deref(), Some("button" | "submit" | "reset")) {
                return match value {
                    Some(value) => producer.text(value, check),
                    None => producer.text(
                        match kind.as_deref() {
                            Some("reset") => "Reset",
                            Some("submit") => "Submit",
                            _ => "",
                        },
                        check,
                    ),
                };
            }
            let value = if matches!(role.as_str(), "slider" | "spinbutton") {
                match node.accessibility_attribute_with_checkpoint("aria-valuetext", check)? {
                    Some(value) => Some(value),
                    None => match node
                        .accessibility_attribute_with_checkpoint("aria-valuenow", check)?
                    {
                        Some(value) => Some(value),
                        None => value,
                    },
                }
            } else {
                value
            };
            return match value {
                Some(value) => producer.text(value, check),
                None => Ok(String::new()),
            };
        }
        NodeKind::Element { name } if name.local.as_ref() == "textarea" => {
            return if node.dirty_value {
                match node.value.as_ref() {
                    Some(value) => producer.text(value, check),
                    None => Ok(String::new()),
                }
            } else {
                shared_default::<String, P>(shared_dom_text(dom, id, producer, check))
            };
        }
        NodeKind::Element { name } if name.local.as_ref() == "select" => {
            let options =
                shared_default::<Vec<NodeId>, P>(shared_select_options(dom, id, producer, check))?;
            let mut selected = producer.vector(0)?;
            let mut initialized = false;
            for option in &options {
                check()?;
                if let Ok(option_node) = dom.get(*option) {
                    if option_node.selected_with_checkpoint(check)? {
                        shared_vector_push(&mut selected, *option, producer, check)?;
                    }
                    initialized |= option_node.selectedness.is_some();
                } else {
                    initialized = true;
                }
            }
            if selected.is_empty()
                && !initialized
                && !node
                    .attributes
                    .contains_null_with_checkpoint("multiple", check)?
                && node
                    .attributes
                    .get_null_with_checkpoint("size", check)?
                    .and_then(|size| size.parse::<u32>().ok())
                    .unwrap_or(0)
                    <= 1
            {
                for option in &options {
                    check()?;
                    let Ok(option_record) = dom.get(*option) else {
                        continue;
                    };
                    if option_record
                        .attributes
                        .contains_null_with_checkpoint("disabled", check)?
                    {
                        continue;
                    }
                    let mut disabled = false;
                    for ancestor in shared_default::<Vec<NodeId>, P>(
                        shared_ancestors_with_checkpoint(dom, *option, producer, check),
                    )? {
                        check()?;
                        if ancestor == id {
                            break;
                        }
                        if let Ok(node) = dom.get(ancestor)
                            && matches!(&node.kind,NodeKind::Element{name}if name.local.as_ref()=="optgroup")
                        {
                            disabled |= node
                                .attributes
                                .contains_null_with_checkpoint("disabled", check)?;
                        }
                    }
                    if !disabled {
                        shared_vector_push(&mut selected, *option, producer, check)?;
                        break;
                    }
                }
            }
            let mut parts = producer.vector(0)?;
            for option in selected {
                check()?;
                let label = if let Ok(node) = dom.get(option) {
                    let explicit = node
                        .accessibility_attribute_with_checkpoint("aria-label", check)?
                        .map(|label| shared_normalize_text(label, producer, check))
                        .transpose()?
                        .filter(|label| !label.is_empty());
                    match explicit {
                        Some(label) => Some(label),
                        None => node
                            .attributes
                            .get_null_with_checkpoint("label", check)?
                            .map(|label| shared_normalize_text(label, producer, check))
                            .transpose()?
                            .filter(|label| !label.is_empty()),
                    }
                } else {
                    None
                };
                let text = match label {
                    Some(label) => label,
                    None => shared_accessible_text_walk(
                        dom,
                        option,
                        false,
                        excluded,
                        include_hidden_subtree,
                        traversal,
                        producer,
                        check,
                    )?,
                };
                if !text.is_empty() {
                    shared_vector_push(&mut parts, text, producer, check)?;
                }
            }
            return shared_join(&parts, " ", producer, check);
        }
        NodeKind::Element { .. } if matches!(role.as_str(), "listbox" | "combobox") => {
            let selected = shared_embedded_selected_options(dom, id, producer, check)?;
            let join = !selected.is_empty();
            let children = if join {
                selected
            } else if role == "combobox" {
                shared_accessible_children_with_checkpoint(dom, id, producer, check)?
            } else {
                Vec::new()
            };
            let mut parts = producer.vector(0)?;
            for child in children {
                check()?;
                let text = shared_accessible_text_walk(
                    dom,
                    child,
                    false,
                    excluded,
                    include_hidden_subtree,
                    traversal,
                    producer,
                    check,
                )?;
                if !text.is_empty() {
                    shared_vector_push(&mut parts, text, producer, check)?;
                }
            }
            return shared_join(&parts, if join { " " } else { "" }, producer, check);
        }
        NodeKind::Element { .. } if matches!(role.as_str(), "slider" | "spinbutton") => {
            let value =
                match node.accessibility_attribute_with_checkpoint("aria-valuetext", check)? {
                    Some(value) => Some(value),
                    None => node.accessibility_attribute_with_checkpoint("aria-valuenow", check)?,
                };
            return match value {
                Some(value) => producer.text(value, check),
                None => Ok(String::new()),
            };
        }
        _ => {}
    }
    let mut action_targets = BTreeSet::new();
    for target in shared_relation(dom, id, "aria-actions", producer, check)?.unwrap_or_default() {
        check()?;
        action_targets.insert(target);
    }
    if traversal.owners.is_none() {
        traversal.owners = Some(accepted_aria_owners_with_checkpoint(dom, check)?);
    }
    let mut value = String::new();
    for child in shared_accessible_children_with_checkpoint(dom, id, producer, check)? {
        check()?;
        if traversal
            .owners
            .as_ref()
            .is_some_and(|owners| owners.contains_key(&child))
            || action_targets.contains(&child)
        {
            continue;
        }
        let text = shared_accessible_text_walk(
            dom,
            child,
            false,
            excluded,
            include_hidden_subtree,
            traversal,
            producer,
            check,
        )?;
        if !text.is_empty() {
            if shared_accessible_text_run_separator_with_checkpoint(dom, child, producer, check)? {
                shared_append_text(&mut value, " ", producer, check)?;
                shared_append_text(&mut value, &text, producer, check)?;
                shared_append_text(&mut value, " ", producer, check)?;
            } else {
                shared_append_text(&mut value, &text, producer, check)?;
            }
        }
    }
    if let Some(owned) = shared_relation(dom, id, "aria-owns", producer, check)? {
        for owned_id in owned {
            check()?;
            if owned_id == id
                || traversal
                    .owners
                    .as_ref()
                    .and_then(|owners| owners.get(&owned_id))
                    .copied()
                    != Some(id)
            {
                continue;
            }
            let text = shared_accessible_text_walk(
                dom,
                owned_id,
                true,
                excluded,
                include_hidden_subtree
                    || shared_accessibility_hidden_at_with_checkpoint(
                        dom, owned_id, producer, check,
                    )?,
                traversal,
                producer,
                check,
            )?;
            if !text.is_empty() {
                if !value.is_empty() {
                    shared_append_text(&mut value, " ", producer, check)?;
                }
                shared_append_text(&mut value, &text, producer, check)?;
            }
        }
    }
    if excluded.is_some()
        && shared_normalize_text(&value, producer, check)?.is_empty()
        && matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "label")
        && let Some(title) = node
            .attributes
            .get_null_with_checkpoint("title", check)?
            .map(|title| shared_normalize_text(title, producer, check))
            .transpose()?
            .filter(|title| !title.is_empty())
    {
        value = shared_surround(" ", &title, " ", producer, check)?;
    }
    if let Some((before, after)) = dom.accessibility_generated_content(id) {
        value = shared_surround(before, &value, after, producer, check)?;
    }
    Ok(value)
}

// staging source: shared-name-closure-v101-v103-v106-v108-v135-v143.rs
fn shared_svg_child_alternative_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    local: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<String>, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(None);
    };
    if !matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/2000/svg")
    {
        return Ok(None);
    }
    for child in &node.children {
        check()?;
        if dom.get(*child).is_ok_and(|node| {
            matches!(&node.kind, NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/2000/svg" && name.local.as_ref() == local)
        }) {
            let text = shared_normalize_text(
                &shared_default::<String, P>(shared_dom_text(dom, *child, producer, check))?,
                producer,
                check,
            )?;
            return Ok((!text.is_empty()).then_some(text));
        }
    }
    Ok(None)
}

fn shared_host_label_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<(String, Vec<NodeId>)>, P::Error> {
    check()?;
    if !dom.is_labelable(id) {
        return Ok(None);
    }
    let Some(root) = tree_scope_with_checkpoint(dom, id, check)? else {
        return Ok(None);
    };
    let mut labels = producer.vector(0)?;
    let mut candidates = producer.vector(1)?;
    candidates.push(root);
    for child in shared_descendants_with_checkpoint(dom, root, producer, check)? {
        check()?;
        shared_vector_push(&mut candidates, child, producer, check)?;
    }
    for candidate in candidates {
        check()?;
        if tree_scope_with_checkpoint(dom, candidate, check)? == Some(root)
            && cancellation_or_default(dom.label_control_with_checkpoint(candidate, check))?
                == Some(id)
        {
            let text = shared_normalize_text(
                &shared_accessible_text(dom, candidate, true, Some(id), producer, check)?,
                producer,
                check,
            )?;
            shared_vector_push(&mut labels, (candidate, text), producer, check)?;
        }
    }
    if labels.is_empty() {
        return Ok(None);
    }
    let mut parts = producer.vector(labels.len())?;
    let mut references = producer.vector(labels.len())?;
    for (label, text) in labels {
        check()?;
        references.push(label);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    Ok(Some((
        shared_join(&parts, " ", producer, check)?,
        references,
    )))
}

fn shared_referenced_nodes_text_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    targets: &[NodeId],
    page_id: PageId,
    document_generation: u64,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<(String, Vec<SemanticRef>), P::Error> {
    let mut values = producer.vector(targets.len())?;
    let mut references = producer.vector(targets.len())?;
    for &id in targets {
        check()?;
        let explicit = match dom.get(id) {
            Ok(node) => node.accessibility_attribute_with_checkpoint("aria-label", check)?,
            Err(_) => None,
        }
        .map(|label| shared_normalize_text(label, producer, check))
        .transpose()?
        .filter(|label| !label.is_empty());
        let mut text = if let Some(label) = explicit {
            label
        } else if let Some((label, _)) =
            shared_host_label_with_checkpoint(dom, id, producer, check)?
        {
            label
        } else {
            shared_normalize_text(
                &shared_accessible_text_walk(
                    dom,
                    id,
                    true,
                    None,
                    shared_referenced_root_hidden(dom, id, producer, check)?,
                    &mut NameTraversal {
                        seen: BTreeSet::new(),
                        reference_depth: 1,
                        ..Default::default()
                    },
                    producer,
                    check,
                )?,
                producer,
                check,
            )?
        };
        if text.is_empty() {
            text = match dom.get(id) {
                Ok(node) => node.attributes.get_null_with_checkpoint("title", check)?,
                Err(_) => None,
            }
            .map(|title| shared_normalize_text(title, producer, check))
            .transpose()?
            .unwrap_or_default();
        }
        if dom.get(id).is_ok() {
            references.push(semantic_ref_for(id, page_id, document_generation));
        }
        if !text.is_empty() {
            values.push(text);
        }
    }
    Ok((shared_join(&values, " ", producer, check)?, references))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Accname inputs include stable page/generation identity alongside separate fallible producer and checkpoint owners."
)]
fn shared_accessible_semantics_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
    page_id: PageId,
    document_generation: u64,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<AccessibleSemantics, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(AccessibleSemantics {
            name: String::new(),
            description: None,
            provenance: Provenance::Heuristic { confidence: 0.0 },
            evidence: Vec::new(),
        });
    };
    let mut evidence = producer.vector(0)?;
    let mut provenance = Provenance::BrowserDeterministic;
    let name = if let Some(targets) = shared_relation(dom, id, "aria-labelledby", producer, check)?
        .filter(|targets| !targets.is_empty())
    {
        let (value, references) = shared_referenced_nodes_text_with_checkpoint(
            dom,
            &targets,
            page_id,
            document_generation,
            producer,
            check,
        )?;
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::ReferencedContent,
                source: producer.text("aria-labelledby", check)?,
                references,
            },
            producer,
            check,
        )?;
        // A valid reference wins even when its text alternative is empty.
        Some(value)
    } else {
        None
    };
    let name = if let Some(name) = name {
        name
    } else if let Some(label) = node
        .accessibility_attribute_with_checkpoint("aria-label", check)?
        .map(|value| shared_normalize_text(value, producer, check))
        .transpose()?
        .filter(|value| !value.is_empty())
    {
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::ExplicitAria,
                source: producer.text("aria-label", check)?,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        label
    } else if let Some((label, references)) =
        shared_host_label_with_checkpoint(dom, id, producer, check)?
    {
        let mut semantic_references = producer.vector(references.len())?;
        for reference in references {
            check()?;
            semantic_references.push(semantic_ref_for(reference, page_id, document_generation));
        }
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::HostLanguage,
                source: producer.text("label", check)?,
                references: semantic_references,
            },
            producer,
            check,
        )?;
        label
    } else if let Some((value, source)) =
        shared_host_language_name_with_checkpoint(dom, id, local_name, role, producer, check)?
    {
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::HostLanguage,
                source,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        value
    } else if let Some(title) = node
        .attributes
        .get_null_with_checkpoint("title", check)?
        .map(|value| shared_normalize_text(value, producer, check))
        .transpose()?
        .filter(|value| !value.is_empty())
    {
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::HostLanguage,
                source: producer.text("title", check)?,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        title
    } else if let Some(placeholder) = node
        .attributes
        .get_null_with_checkpoint("placeholder", check)?
        .map(|value| shared_normalize_text(value, producer, check))
        .transpose()?
        .filter(|value| !value.is_empty())
    {
        provenance = Provenance::Heuristic { confidence: 0.65 };
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::PlaceholderFallback,
                source: producer.text("placeholder", check)?,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        placeholder
    } else {
        String::new()
    };

    let description = if let Some(targets) =
        shared_relation(dom, id, "aria-describedby", producer, check)?
            .filter(|targets| !targets.is_empty())
    {
        let (value, references) = shared_referenced_nodes_text_with_checkpoint(
            dom,
            &targets,
            page_id,
            document_generation,
            producer,
            check,
        )?;
        if value.is_empty() {
            None
        } else {
            shared_vector_push(
                &mut evidence,
                AccessibilityEvidence {
                    kind: AccessibilityEvidenceKind::ReferencedContent,
                    source: producer.text("aria-describedby", check)?,
                    references,
                },
                producer,
                check,
            )?;
            Some(value)
        }
    } else if let Some(value) = node
        .accessibility_attribute_with_checkpoint("aria-description", check)?
        .map(|value| shared_normalize_text(value, producer, check))
        .transpose()?
        .filter(|value| !value.is_empty())
    {
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::ExplicitAria,
                source: producer.text("aria-description", check)?,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        Some(value)
    } else if let Some(value) =
        shared_svg_child_alternative_with_checkpoint(dom, id, "desc", producer, check)?
    {
        shared_vector_push(
            &mut evidence,
            AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::HostLanguage,
                source: producer.text("svg-desc", check)?,
                references: Vec::new(),
            },
            producer,
            check,
        )?;
        Some(value)
    } else {
        // Tooltip text is a description only when it did not supply the name.
        // Equal strings from independent naming sources still qualify.
        let title_used_for_name = evidence.iter().any(|item| {
            item.kind == AccessibilityEvidenceKind::HostLanguage && item.source == "title"
        });
        let title = node
            .attributes
            .get_null_with_checkpoint("title", check)?
            .map(|value| shared_normalize_text(value, producer, check))
            .transpose()?
            .filter(|value| !value.is_empty() && !title_used_for_name);
        if title.is_some() {
            shared_vector_push(
                &mut evidence,
                AccessibilityEvidence {
                    kind: AccessibilityEvidenceKind::HostLanguage,
                    source: producer.text("title-description", check)?,
                    references: Vec::new(),
                },
                producer,
                check,
            )?;
        }
        title
    };

    Ok(AccessibleSemantics {
        name,
        description,
        provenance,
        evidence,
    })
}

fn shared_host_language_name_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<(String, String)>, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(None);
    };
    if let Some(title) =
        shared_svg_child_alternative_with_checkpoint(dom, id, "title", producer, check)?
    {
        return Ok(Some((title, producer.text("svg-title", check)?)));
    }
    let html = matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml");
    let input_type = node
        .attributes
        .get_null_with_checkpoint("type", check)?
        .map(|kind| shared_ascii_lowercase(kind, producer, check))
        .transpose()?;
    let attribute = match if html { local_name } else { "" } {
        "img" | "area" => Some(("alt", "alt")),
        "input" if input_type.as_deref() == Some("image") => Some(("alt", "input-alt")),
        "input" if matches!(input_type.as_deref(), Some("button" | "submit" | "reset")) => {
            Some(("value", "input-value"))
        }
        _ => None,
    };
    if let Some((attribute, source)) = attribute
        && let Some(value) = (if attribute == "value" {
            match node.value.as_ref() {
                Some(value) => Some(value),
                None => node.attributes.get_null_with_checkpoint(attribute, check)?,
            }
        } else {
            node.attributes.get_null_with_checkpoint(attribute, check)?
        })
    {
        // An explicitly empty host-language alternative is authoritative and
        // must suppress lower-priority title/placeholder fallbacks.
        return Ok(Some((
            shared_normalize_text(value, producer, check)?,
            producer.text(source, check)?,
        )));
    }
    if html && local_name == "input" && matches!(input_type.as_deref(), Some("submit" | "reset")) {
        return Ok(Some((
            producer.text(
                if input_type.as_deref() == Some("reset") {
                    "Reset"
                } else {
                    "Submit"
                },
                check,
            )?,
            producer.text("input-default-value", check)?,
        )));
    }
    if html
        && local_name == "img"
        && node
            .attributes
            .get_null_with_checkpoint("title", check)?
            .map(|title| shared_normalize_text(title,producer, check)).transpose()?.is_none_or(|title| title.is_empty())
        && let Some(parent)=node.parent.and_then(|parent|dom.get(parent).ok()).filter(|parent|matches!(&parent.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="figure")) {
            let mut caption=None;let mut eligible=true;
            for child in &parent.children {check()?;if *child==id{continue;}let Ok(child_node)=dom.get(*child)else{eligible=false;break;};match &child_node.kind{
                NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="figcaption"=>{if caption.replace(*child).is_some(){eligible=false;}},
                NodeKind::Text{data}|NodeKind::CDataSection{data} if data.chars().all(|ch|matches!(ch,' '|'\t'|'\r'|'\n'|'\u{c}'))=>{},
                NodeKind::Comment{..}=>{},_=>eligible=false,
            }}
            if eligible&& let Some(caption)=caption {return Ok(Some((shared_normalize_text(&shared_accessible_text(dom,caption,false,None,producer,check)?,producer, check)?,producer.text("figcaption-image",check)?)));}
        }
    let named_child = match if html { local_name } else { "" } {
        "fieldset" => Some(("legend", "legend")),
        "table" => Some(("caption", "caption")),
        _ => None,
    };
    if let Some((child_name, source)) = named_child {
        for child in &node.children {
            check()?;
            if dom.get(*child).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == child_name))
            {
                let text = shared_normalize_text(&shared_accessible_text(dom,*child,false,None,producer,check)?,producer,check)?;
                if !text.is_empty() { return Ok(Some((text, producer.text(source,check)?))); }
                break;
            }
        }
    }
    // Name-from-heading selects the first exposed heading in depth-first
    // flat-tree order. Dialogs and articles share this algorithm; looking only
    // at immediate children would skip headings inside native layout wrappers.
    if matches!(role, "article" | "dialog" | "alertdialog") {
        let mut pending = shared_accessible_children_with_checkpoint(dom, id, producer, check)?;
        pending.reverse();
        let mut seen = BTreeSet::new();
        while let Some(candidate) = pending.pop() {
            check()?;
            if !seen.insert(candidate)
                || shared_subtree_excluded_at_with_checkpoint(dom, candidate, producer, check)?
            {
                continue;
            }
            if !shared_accessibility_hidden_at_with_checkpoint(dom, candidate, producer, check)?
                && shared_role_for_node(dom, candidate, producer, check)? == "heading"
            {
                let text = shared_normalize_text(
                    &shared_accessible_text(dom, candidate, false, None, producer, check)?,
                    producer,
                    check,
                )?;
                if !text.is_empty() {
                    return Ok(Some((text, producer.text("heading", check)?)));
                }
                break;
            }
            for child in
                shared_accessible_children_with_checkpoint(dom, candidate, producer, check)?
                    .into_iter()
                    .rev()
            {
                shared_vector_push(&mut pending, child, producer, check)?;
            }
        }
    }
    if name_from_content(local_name, role)
        || node
            .attributes
            .contains_null_with_checkpoint("onclick", check)?
        || node.event_listeners.contains("click")
    {
        let text = shared_normalize_text(
            &shared_accessible_text(dom, id, true, None, producer, check)?,
            producer,
            check,
        )?;
        if !text.is_empty() {
            return Ok(Some((text, producer.text("text-content", check)?)));
        }
    }
    Ok(None)
}

fn shared_role_has_name_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    local: &str,
    base: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let key = (dom as *const DomArena as usize, id);
    if !ROLE_NAME_STACK.with(|stack| stack.borrow_mut().insert(key)) {
        return Ok(false);
    }
    struct Guard((usize, NodeId));
    impl Drop for Guard {
        fn drop(&mut self) {
            ROLE_NAME_STACK.with(|stack| {
                stack.borrow_mut().remove(&self.0);
            });
        }
    }
    let _guard = Guard(key);
    let mark = producer.receipt_mark();
    let named = shared_accessible_semantics_with_checkpoint(
        dom,
        id,
        local,
        base,
        PageId(1),
        1,
        producer,
        check,
    )?;
    let result = !named.name.is_empty();
    drop(named);
    producer.release_scratch(mark);
    Ok(result)
}

fn shared_unpromoted_role_for_node_with_checkpoint<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return producer.text("generic", check);
    };
    let NodeKind::Element { name } = &node.kind else {
        return producer.text("generic", check);
    };
    let local = name.local.as_ref();
    let attributes =
        node.accessibility_attributes_with_allocation(&mut SemanticDomAllocation(producer), check)?;
    let base = shared_computed_role_with_checkpoint(local, &attributes, producer, check)?;
    let explicit = shared_first_supported_role_with_checkpoint(
        attributes.get_null_with_checkpoint("role", check)?,
        false,
        producer,
        check,
    )?;
    if explicit
        .as_deref()
        .is_some_and(|role| !matches!(role, "none" | "presentation" | "form" | "region"))
    {
        return Ok(base);
    }
    let base = if matches!(explicit.as_deref(), Some("form" | "region")) {
        if shared_role_has_name_with_checkpoint(dom, id, local, &base, producer, check)? {
            return Ok(explicit.unwrap());
        }
        if let Some(fallback) = shared_first_supported_role_with_checkpoint(
            attributes.get_null_with_checkpoint("role", check)?,
            true,
            producer,
            check,
        )? {
            return shared_explicit_role_value_with_checkpoint(
                local,
                &attributes,
                fallback,
                producer,
                check,
            );
        }
        shared_implicit_role_with_checkpoint(local, &attributes, producer, check)?
    } else {
        producer.text(&base, check)?
    };
    if matches!(explicit.as_deref(), Some("none" | "presentation")) && base == "none" {
        return Ok(base);
    }
    if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
        return Ok(base);
    }
    // Native required-owned descendants inherit their container's effective
    // presentation. Global ARIA properties on the descendant do not cancel
    // inherited presentation; an explicit role or focusability does.
    if attributes
        .get_null_with_checkpoint("tabindex", check)?
        .map(|value| naming_i32_with_checkpoint(value, check))
        .transpose()?
        .flatten()
        .is_none()
        && let Some(parent) = node.parent
    {
        let required_parent = dom.get(parent).is_ok_and(|parent| match &parent.kind {
            NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" => {
                match local {
                    "thead" | "tbody" | "tfoot" | "caption" => name.local.as_ref() == "table",
                    "tr" => {
                        matches!(name.local.as_ref(), "table" | "thead" | "tbody" | "tfoot")
                    }
                    "td" | "th" => name.local.as_ref() == "tr",
                    _ => false,
                }
            }
            _ => false,
        });
        if required_parent && shared_role_for_node(dom, parent, producer, check)? == "none" {
            return producer.text("none", check);
        }
    }
    let mut sectioning = false;
    let mut below_main = false;
    for ancestor in shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
        dom, id, producer, check,
    ))? {
        check()?;
        if let Ok(node) = dom.get(ancestor)
            && let NodeKind::Element { name } = &node.kind
            && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        {
            sectioning |= matches!(name.local.as_ref(), "article" | "aside" | "nav" | "section");
            below_main |= name.local.as_ref() == "main";
        }
    }
    let mut row_has_cell = false;
    if local == "th"
        && let Some(row) = node.parent.and_then(|parent| dom.get(parent).ok())
    {
        for child in &row.children {
            check()?;
            if dom.get(*child).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "td")) {
                row_has_cell = true;
                break;
            }
        }
    }
    let explicit_scope = if local == "th" {
        attributes
            .get_null_with_checkpoint("scope", check)?
            .map(|scope| shared_ascii_lowercase(scope, producer, check))
            .transpose()?
    } else {
        None
    };
    Ok(match local {
        "th" if explicit_scope
            .as_deref()
            .is_none_or(|scope| !matches!(scope, "row" | "rowgroup" | "col" | "colgroup")) =>
        {
            if row_has_cell {
                producer.text("rowheader", check)?
            } else {
                producer.text("columnheader", check)?
            }
        }
        "li" => {
            let mut cursor = accepted_aria_owners_with_checkpoint(dom, check)?
                .get(&id)
                .copied()
                .or(node.parent);
            let mut seen = BTreeSet::new();
            while let Some(parent) = cursor {
                check()?;
                if !seen.insert(parent) {
                    break;
                }
                let role = shared_role_for_node(dom, parent, producer, check)?;
                if role == "list" {
                    return producer.text("listitem", check);
                }
                if !matches!(role.as_str(), "generic" | "none") {
                    break;
                }
                cursor = dom.get(parent).ok().and_then(|parent| parent.parent);
            }
            producer.text("generic", check)?
        }
        "aside"
            if sectioning
                && !shared_role_has_name_with_checkpoint(
                    dom, id, local, &base, producer, check,
                )? =>
        {
            producer.text("generic", check)?
        }
        "header" | "footer" if sectioning || below_main => {
            if local == "header" {
                producer.text("sectionheader", check)?
            } else {
                producer.text("sectionfooter", check)?
            }
        }
        "section" => {
            if shared_role_has_name_with_checkpoint(dom, id, local, &base, producer, check)? {
                producer.text("region", check)?
            } else {
                producer.text("generic", check)?
            }
        }
        "form" => {
            if shared_role_has_name_with_checkpoint(dom, id, local, &base, producer, check)? {
                producer.text("form", check)?
            } else {
                producer.text("generic", check)?
            }
        }
        "img"
            if attributes
                .get_null_with_checkpoint("alt", check)?
                .map(|alt| shared_normalize_text(alt, producer, check))
                .transpose()?
                .is_some_and(|alt| alt.is_empty()) =>
        {
            if (attributes.contains_null_with_checkpoint("aria-label", check)?
                || shared_relation(dom, id, "aria-labelledby", producer, check)?
                    .is_some_and(|targets| !targets.is_empty()))
                && shared_role_has_name_with_checkpoint(dom, id, local, &base, producer, check)?
            {
                producer.text("image", check)?
            } else {
                producer.text("none", check)?
            }
        }
        "img"
            if !attributes
                .get_null_with_checkpoint("src", check)?
                .is_some_and(|value| !value.is_empty())
                && !attributes
                    .get_null_with_checkpoint("srcset", check)?
                    .is_some_and(|value| !value.is_empty())
                && !shared_role_has_name_with_checkpoint(
                    dom, id, local, &base, producer, check,
                )? =>
        {
            producer.text("none", check)?
        }
        "input"
            if attributes
                .get_null_with_checkpoint("type", check)?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
                && attributes.contains_null_with_checkpoint("switch", check)? =>
        {
            producer.text("switch", check)?
        }
        _ => base,
    })
}

// staging source: shared-embedded-selected-v138-v143.rs
fn shared_embedded_selected_options<P: SemanticProducer>(
    dom: &DomArena,
    root: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    let root_is_combobox = shared_role_for_node(dom, root, producer, check)? == "combobox";
    let owners = accepted_aria_owners_with_checkpoint(dom, check)?;
    let mut pending = producer.vector(1)?;
    pending.push(root);
    let mut seen = BTreeSet::new();
    let mut selected = producer.vector(0)?;
    while let Some(id) = pending.pop() {
        check()?;
        if !seen.insert(id) {
            continue;
        }
        let Ok(node) = dom.get(id) else {
            continue;
        };
        if id != root && matches!(&node.kind, NodeKind::Element { .. }) {
            let role = shared_role_for_node(dom, id, producer, check)?;
            if role == "option" {
                if node
                    .accessibility_attribute_with_checkpoint("aria-selected", check)?
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                {
                    shared_vector_push(&mut selected, id, producer, check)?;
                }
                continue;
            }
            if role == "combobox" || (role == "listbox" && !root_is_combobox) {
                continue;
            }
        }
        let mut children = producer.vector(0)?;
        for child in shared_accessible_children_with_checkpoint(dom, id, producer, check)? {
            check()?;
            if !owners.contains_key(&child) {
                shared_vector_push(&mut children, child, producer, check)?;
            }
        }
        let mut children_seen = BTreeSet::new();
        for child in &children {
            check()?;
            children_seen.insert(*child);
        }
        if let Some(owned) = shared_relation(dom, id, "aria-owns", producer, check)? {
            for child in owned {
                check()?;
                if owners.get(&child) == Some(&id) && children_seen.insert(child) {
                    shared_vector_push(&mut children, child, producer, check)?;
                }
            }
        }
        for child in children.into_iter().rev() {
            check()?;
            shared_vector_push(&mut pending, child, producer, check)?;
        }
    }
    Ok(selected)
}

// staging source: shared-relation-adapter-v143.rs
fn shared_relation<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    name: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<Vec<NodeId>>, P::Error> {
    dom.accessibility_relation_with_allocation(
        id,
        name,
        &mut SemanticDomAllocation(producer),
        check,
    )
}

// staging source: shared-readonly-v146.rs
fn shared_accessibility_readonly<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let mark = producer.receipt_mark();
    let result = (|| {
        check()?;
        let Ok(node) = dom.get(id) else {
            return Ok(true);
        };
        if let Some(value) = node.accessibility_attribute_with_checkpoint("aria-readonly", check)? {
            return Ok(value.eq_ignore_ascii_case("true"));
        }
        if matches!(
            shared_role_for_node(dom, id, producer, check)?.as_str(),
            "gridcell" | "columnheader" | "rowheader"
        ) {
            let owners = accepted_aria_owners_with_checkpoint(dom, check)?;
            let mut cursor = owners.get(&id).copied().or(node.parent);
            let mut seen = BTreeSet::from([id]);
            while let Some(parent) = cursor {
                check()?;
                if !seen.insert(parent) {
                    break;
                }
                if matches!(
                    shared_role_for_node(dom, parent, producer, check)?.as_str(),
                    "grid" | "treegrid"
                ) {
                    return match dom.get(parent) {
                        Ok(parent) => Ok(parent
                            .accessibility_attribute_with_checkpoint("aria-readonly", check)?
                            .is_some_and(|value| value.eq_ignore_ascii_case("true"))),
                        Err(_) => Ok(false),
                    };
                }
                cursor = owners
                    .get(&parent)
                    .copied()
                    .or_else(|| dom.get(parent).ok().and_then(|node| node.parent));
            }
        }
        Ok(false)
    })();
    producer.release_scratch(mark);
    result
}

// staging source: shared-hidden-v148.rs
fn shared_is_hidden<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let mark = producer.receipt_mark();
    let result = (|| {
        check()?;
        let mut lineage = producer.vector(1)?;
        lineage.push(id);
        for ancestor in shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
            dom, id, producer, check,
        ))? {
            check()?;
            shared_vector_push(&mut lineage, ancestor, producer, check)?;
        }
        if let Some((_, visibility_hidden)) = dom.accessibility_computed_visibility(id) {
            if visibility_hidden {
                return Ok(true);
            }
            for candidate in lineage {
                check()?;
                if shared_subtree_excluded_at_with_checkpoint(dom, candidate, producer, check)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let mut rules = producer.vector(0)?;
        {
            let mut cursor = 0usize;
            while let Some((style_id, node)) = dom.next_with_checkpoint(&mut cursor, check)? {
                check()?;
                if !matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "style")
                {
                    continue;
                }
                let sheet =
                    shared_default::<String, P>(shared_dom_text(dom, style_id, producer, check))?;
                let sheet = shared_strip_css_comments(&sheet, producer, check)?;
                for rule in shared_name_tokens(&sheet, |ch| ch == '}', producer, check)? {
                    check()?;
                    if let Some(split) = shared_first_delimiter::<P>(rule, '{', check)? {
                        let selectors = &rule[..split];
                        let declarations = &rule[split + 1..];
                        for selector in
                            shared_name_tokens(selectors, |ch| ch == ',', producer, check)?
                        {
                            check()?;
                            let selector = producer
                                .text(naming_trim_with_checkpoint(selector, check)?, check)?;
                            let declarations = producer.text(declarations, check)?;
                            shared_vector_push(
                                &mut rules,
                                (selector, declarations),
                                producer,
                                check,
                            )?;
                        }
                    }
                }
            }
        }
        for candidate in lineage {
            check()?;
            let Ok(node) = dom.get(candidate) else {
                continue;
            };
            if node
                .attributes
                .contains_null_with_checkpoint("hidden", check)?
                || node
                    .accessibility_attribute_with_checkpoint("aria-hidden", check)?
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                return Ok(true);
            }
            let mut hidden = None;
            for (selector, declarations) in &rules {
                check()?;
                if shared_matches_selector(dom, candidate, selector, producer, check)? {
                    shared_update_visibility(&mut hidden, declarations, producer, check)?;
                }
            }
            if let Some(style) = node.attributes.get_null_with_checkpoint("style", check)? {
                shared_update_visibility(&mut hidden, style, producer, check)?;
            }
            if hidden.unwrap_or(false) {
                return Ok(true);
            }
        }
        Ok(false)
    })();
    producer.release_scratch(mark);
    result
}

fn shared_first_delimiter<P: SemanticProducer>(
    value: &str,
    delimiter: char,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<usize>, P::Error> {
    for (index, ch) in value.char_indices() {
        check()?;
        if ch == delimiter {
            return Ok(Some(index));
        }
    }
    check()?;
    Ok(None)
}

// staging source: shared-selector-v149.rs
fn shared_matches_selector<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    selector: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let mark = producer.receipt_mark();
    let result = (|| {
        check()?;
        let mut segments = producer.vector(0)?;
        for segment in shared_name_tokens(selector, char::is_whitespace, producer, check)? {
            check()?;
            if !segment.is_empty() && segment != ">" {
                shared_vector_push(&mut segments, segment, producer, check)?;
            }
        }
        let Some(subject) = segments.last() else {
            return Ok(false);
        };
        let Ok(node) = dom.get(id) else {
            return Ok(false);
        };
        if !shared_matches_simple_selector(node, subject, producer, check)? {
            return Ok(false);
        }
        let ancestors = shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
            dom, id, producer, check,
        ))?;
        let mut index = 0;
        for segment in segments[..segments.len() - 1].iter().rev() {
            check()?;
            let mut found = false;
            while index < ancestors.len() {
                check()?;
                let ancestor = ancestors[index];
                index += 1;
                if let Ok(node) = dom.get(ancestor)
                    && shared_matches_simple_selector(node, segment, producer, check)?
                {
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    producer.release_scratch(mark);
    result
}

// staging source: shared-simple-selector-v150.rs
fn shared_matches_simple_selector<P: SemanticProducer>(
    node: &crate::dom::Node,
    selector: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    let mark = producer.receipt_mark();
    let result = (|| {
        check()?;
        let NodeKind::Element { name } = &node.kind else {
            return Ok(false);
        };
        let selector =
            shared_name_tokens(selector, |character| character == ':', producer, check)?[0];
        let mut attribute_start = None;
        for (index, character) in selector.char_indices() {
            check()?;
            if character == '[' {
                attribute_start = Some(index);
                break;
            }
        }
        let simple = &selector[..attribute_start.unwrap_or(selector.len())];
        if let Some(index) = attribute_start
            && let Some(selector) = selector.strip_suffix(']')
        {
            let attribute = &selector[index + 1..];
            let mut equal = None;
            for (index, character) in attribute.char_indices() {
                check()?;
                if character == '=' {
                    equal = Some(index);
                    break;
                }
            }
            let (key, expected) = match equal {
                Some(index) => (
                    &attribute[..index],
                    Some(shared_trim_quotes::<P>(&attribute[index + 1..], check)?),
                ),
                None => (attribute, None),
            };
            let Some(actual) = node.attributes.get_null_with_checkpoint(key, check)? else {
                return Ok(false);
            };
            if let Some(expected) = expected
                && !naming_equal_with_checkpoint(actual, expected, check)?
            {
                return Ok(false);
            }
        }
        let mut tag_end = simple.len();
        let mut id_start = None;
        for (index, character) in simple.char_indices() {
            check()?;
            if matches!(character, '#' | '.') {
                tag_end = tag_end.min(index);
            }
            if character == '#' && id_start.is_none() {
                id_start = Some(index);
            }
        }
        let tag = &simple[..tag_end];
        if !tag.is_empty() && !shared_ascii_equal::<P>(name.local.as_ref(), tag, check)? {
            return Ok(false);
        }
        if let Some(start) = id_start {
            let id = shared_name_tokens(
                &simple[start + 1..],
                |character| character == '.',
                producer,
                check,
            )?[0];
            let actual = node.attributes.get_null_with_checkpoint("id", check)?;
            if match actual {
                Some(value) => !naming_equal_with_checkpoint(value, id, check)?,
                None => true,
            } {
                return Ok(false);
            }
        }
        for class in shared_name_tokens(simple, |character| character == '.', producer, check)?
            .into_iter()
            .skip(1)
        {
            check()?;
            let Some(value) = node.attributes.get_null_with_checkpoint("class", check)? else {
                return Ok(false);
            };
            let mut found = false;
            for token in shared_name_tokens(value, char::is_whitespace, producer, check)? {
                check()?;
                if !token.is_empty() && naming_equal_with_checkpoint(token, class, check)? {
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    producer.release_scratch(mark);
    result
}

fn shared_trim_quotes<'a, P: SemanticProducer>(
    value: &'a str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<&'a str, P::Error> {
    let mut start = 0;
    let mut end = value.len();
    while start < end {
        check()?;
        if !matches!(value.as_bytes()[start], b'"' | b'\'') {
            break;
        }
        start += 1;
    }
    while end > start {
        check()?;
        if !matches!(value.as_bytes()[end - 1], b'"' | b'\'') {
            break;
        }
        end -= 1;
    }
    check()?;
    Ok(&value[start..end])
}
fn shared_ascii_equal<P: SemanticProducer>(
    left: &str,
    right: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<bool, P::Error> {
    check()?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.bytes().zip(right.bytes()) {
        check()?;
        if !left.eq_ignore_ascii_case(&right) {
            return Ok(false);
        }
    }
    check()?;
    Ok(true)
}

// staging source: shared-project-element-v148.rs
#[expect(
    clippy::too_many_arguments,
    reason = "Projection carries node, page/generation, authored/geometry visibility and independent producer/checkpoint ownership."
)]
fn shared_project_element<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    page_id: PageId,
    document_generation: u64,
    alias: u32,
    bypass_visibility: bool,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<SemanticNodeView>, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(None);
    };
    let NodeKind::Element { name } = &node.kind else {
        return Ok(None);
    };
    let ancestors = match shared_ancestors_with_checkpoint(dom, id, producer, check) {
        Ok(value) => value,
        Err(error) => match P::ordinary_dom_error(error) {
            Ok(crate::dom::DomError::ProjectionCancelled) => {
                return Err(P::Error::from(crate::dom::DomError::ProjectionCancelled));
            }
            Ok(crate::dom::DomError::SemanticProjectionCapacity) => {
                return Err(P::Error::from(
                    crate::dom::DomError::SemanticProjectionCapacity,
                ));
            }
            Ok(_) => return Ok(None),
            Err(error) => return Err(error),
        },
    };
    let mut owner = id;
    let mut owner_seen = BTreeSet::new();
    loop {
        check()?;
        if !owner_seen.insert(owner) {
            // Hosted-document cycles have no live browsing-context projection.
            return Ok(None);
        }
        let Some(document) = shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
            dom, owner, producer, check,
        ))?
        .into_iter()
        .find(|ancestor| {
            dom.get(*ancestor)
                .is_ok_and(|node| matches!(node.kind, NodeKind::Document))
        }) else {
            return Ok(None);
        };
        if dom.document_is_retired(document) {
            return Ok(None);
        }
        match dom.document_host(document) {
            Some(frame) => owner = frame,
            None => break,
        }
    }
    if ancestors
        .iter()
        .any(|document| dom.document_is_retired(*document))
    {
        return Ok(None);
    }
    // Wrapper leases deliberately keep detached nodes alive, but detached
    // records are not part of the document's semantic projection. Publishing
    // them allowed OIL to resolve a freshly replaced list item to an old,
    // disconnected NodeId and then correctly reject it as stale at dispatch.
    if !ancestors.iter().any(|ancestor| {
        matches!(
            dom.get(*ancestor).ok().map(|node| &node.kind),
            Some(NodeKind::Document)
        )
    }) {
        return Ok(None);
    }
    if ancestors.iter().any(|ancestor| {
        matches!(
            dom.get(*ancestor).ok().map(|node| &node.kind),
            Some(NodeKind::ShadowRoot {
                mode: crate::dom::ShadowMode::Closed,
                ..
            })
        )
    }) {
        return Ok(None);
    }
    let local_name = name.local.as_ref();
    let parent = ancestors
        .iter()
        .copied()
        .find_map(|ancestor| {
            matches!(dom.get(ancestor).ok()?.kind, NodeKind::Element { .. }).then_some(
                SemanticRef {
                    page_id,
                    document_generation,
                    node: (u64::from(ancestor.generation) << 32) | u64::from(ancestor.slot),
                },
            )
        })
        .or_else(|| {
            ancestors.iter().find_map(|document| {
                let frame = dom.document_host(*document)?;
                Some(SemanticRef {
                    page_id,
                    document_generation,
                    node: (u64::from(frame.generation) << 32) | u64::from(frame.slot),
                })
            })
        });
    let role = shared_role_for_node(dom, id, producer, check)?;
    let mut accessible = shared_accessible_semantics_with_checkpoint(
        dom,
        id,
        local_name,
        &role,
        page_id,
        document_generation,
        producer,
        check,
    )?;
    if accessible.name.is_empty() && matches!(role.as_str(), "status" | "alert" | "log") {
        let text = shared_normalize_text(
            &shared_accessible_text(dom, id, false, None, producer, check)?,
            producer,
            check,
        )?;
        if !text.is_empty() {
            // Compatibility observations historically surface live-region
            // content as the node's agent-facing name. Contract v5 keeps the
            // accessibility name and live content distinct.
            accessible.name = text;
            accessible.provenance = Provenance::Heuristic { confidence: 0.8 };
        }
    }
    let accessible_name = accessible.name;
    let input_kind = node
        .attributes
        .get_null_with_checkpoint("type", check)?
        .map(|kind| shared_ascii_lowercase(kind, producer, check))
        .transpose()?;
    let input_kind = match input_kind {
        Some(value) => value,
        None => producer.text("text", check)?,
    };
    let native_readonly = local_name == "textarea"
        || (local_name == "input"
            && !matches!(
                input_kind.as_str(),
                "hidden"
                    | "range"
                    | "color"
                    | "checkbox"
                    | "radio"
                    | "file"
                    | "button"
                    | "submit"
                    | "reset"
                    | "image"
            ));
    let attribute_mark = producer.receipt_mark();
    let effective_attributes =
        node.accessibility_attributes_with_allocation(&mut SemanticDomAllocation(producer), check)?;
    let attribute_end = producer.receipt_mark();
    let mut actions = BTreeSet::new();
    let mut states = BTreeSet::new();
    if shared_is_hidden(dom, id, producer, check)? {
        states.insert(NodeState::Hidden);
    } else {
        states.insert(NodeState::Visible);
    }
    for (present, state) in [
        (
            node.attributes
                .contains_null_with_checkpoint("disabled", check)?
                || aria_true(effective_attributes.as_ref(), "aria-disabled"),
            NodeState::Disabled,
        ),
        (
            node.attributes
                .contains_null_with_checkpoint("required", check)?
                || aria_true(effective_attributes.as_ref(), "aria-required"),
            NodeState::Required,
        ),
        (
            (native_readonly
                && node
                    .attributes
                    .contains_null_with_checkpoint("readonly", check)?)
                || shared_accessibility_readonly(dom, id, producer, check)?,
            NodeState::Readonly,
        ),
        (
            node.attributes
                .contains_null_with_checkpoint("data-oryn-focused", check)?,
            NodeState::Focused,
        ),
        (
            node.checked_with_checkpoint(check)?
                || aria_true(effective_attributes.as_ref(), "aria-checked"),
            NodeState::Checked,
        ),
        (
            node.attributes
                .contains_null_with_checkpoint("selected", check)?
                || aria_true(effective_attributes.as_ref(), "aria-selected"),
            NodeState::Selected,
        ),
        (
            aria_true(effective_attributes.as_ref(), "aria-expanded")
                || (local_name == "details"
                    && node
                        .attributes
                        .contains_null_with_checkpoint("open", check)?),
            NodeState::Expanded,
        ),
    ] {
        if present {
            states.insert(state);
        }
    }
    if !states.contains(&NodeState::Disabled)
        && (bypass_visibility || !states.contains(&NodeState::Hidden))
    {
        if node
            .attributes
            .get_null_with_checkpoint("tabindex", check)?
            .map(|value| naming_i32_with_checkpoint(value, check))
            .transpose()?
            .flatten()
            .is_some()
            || node
                .attributes
                .get_null_with_checkpoint("contenteditable", check)?
                .is_some_and(|value| {
                    value.is_empty()
                        || value.eq_ignore_ascii_case("true")
                        || value.eq_ignore_ascii_case("plaintext-only")
                })
        {
            actions.insert(SemanticAction::Focus);
        }
        if matches!(local_name, "button" | "a") {
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "textarea" {
            if !states.contains(&NodeState::Readonly) {
                actions.extend([SemanticAction::Type, SemanticAction::Clear]);
            }
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "input" {
            match Some(input_kind.as_str()) {
                Some("checkbox" | "radio") => actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                ]),
                Some("button" | "submit" | "reset" | "image" | "file") => {
                    actions.insert(SemanticAction::Click);
                }
                _ if !states.contains(&NodeState::Readonly) => {
                    actions.extend([SemanticAction::Type, SemanticAction::Clear]);
                }
                _ => {}
            }
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Focus,
                SemanticAction::Hover,
            ]);
        }
        if local_name == "select" {
            actions.extend([
                SemanticAction::Click,
                SemanticAction::Select,
                SemanticAction::Focus,
            ]);
        }
        // A visible label that wraps a visually-hidden checkbox or radio is
        // the actual pointer affordance used by many component libraries.
        // Keep the hidden control out of the compact visual projection, but
        // expose the label as clickable so OIL does not resolve its text-only
        // descendant and then report a non-actionable alias.
        if local_name == "label" {
            let mut label_click = false;
            for input in shared_descendants_with_checkpoint(dom, id, producer, check)? {
                check()?;
                if let Ok(input) = dom.get(input)
                    && matches!(&input.kind,NodeKind::Element{name}if name.local.as_ref()=="input")
                    && input
                        .attributes
                        .get_null_with_checkpoint("type", check)?
                        .is_some_and(|kind| {
                            kind.eq_ignore_ascii_case("checkbox")
                                || kind.eq_ignore_ascii_case("radio")
                        })
                {
                    label_click = true;
                    break;
                }
            }
            if label_click {
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
        }
        if local_name == "form"
            || matches!(
                node.attributes
                    .get_null_with_checkpoint("type", check)?
                    .map(String::as_str),
                Some("submit")
            )
        {
            actions.insert(SemanticAction::Submit);
        }
        if node
            .attributes
            .contains_null_with_checkpoint("onclick", check)?
            || node.event_listeners.contains("click")
        {
            actions.extend([SemanticAction::Click, SemanticAction::Focus]);
        }
        match role.as_str() {
            "button" | "tab" | "menuitem" | "treeitem" | "link" | "option" => {
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
            "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio" => {
                actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                    SemanticAction::Focus,
                ]);
            }
            "textbox" | "searchbox" => {
                if !states.contains(&NodeState::Readonly) {
                    actions.extend([SemanticAction::Type, SemanticAction::Clear]);
                }
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
            "combobox" | "listbox" | "radiogroup" => {
                actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Select,
                    SemanticAction::Focus,
                ]);
            }
            "slider" | "spinbutton" | "gridcell" | "columnheader" | "rowheader" => {
                if !states.contains(&NodeState::Readonly) {
                    actions.insert(SemanticAction::Type);
                }
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
            _ => {}
        }
    }
    drop(effective_attributes);
    producer.release_receipt_range(attribute_mark..attribute_end);
    let description = accessible.description;
    let selector = if let Some(value) = node.attributes.get_null_with_checkpoint("id", check)? {
        Some(shared_surround("#", value, "", producer, check)?)
    } else if let Some(value) = node
        .attributes
        .get_null_with_checkpoint("data-testid", check)?
    {
        Some(shared_surround(
            "[data-testid=\"",
            value,
            "\"]",
            producer,
            check,
        )?)
    } else {
        None
    };
    let redacted = node.sensitive_value
        || (local_name == "input"
            && node
                .attributes
                .get_null_with_checkpoint("type", check)?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("password")));
    let value = if redacted {
        if node.value.is_some() || node.sensitive_value {
            Some(producer.text("[REDACTED]", check)?)
        } else {
            None
        }
    } else {
        match node.value.as_ref() {
            Some(value) => Some(producer.text(value, check)?),
            None => node
                .attributes
                .get_null_with_checkpoint("value", check)?
                .map(|value| producer.text(value, check))
                .transpose()?,
        }
    };
    Ok(Some(SemanticNodeView {
        validation: None,
        semantic_ref: SemanticRef {
            page_id,
            document_generation,
            node: (u64::from(id.generation) << 32) | u64::from(id.slot),
        },
        parent,
        document_order: id.slot,
        alias,
        role,
        name: accessible_name,
        selector,
        value,
        description,
        states,
        actions,
        content_trust: ContentTrust::PageAuthored,
        // This projection currently has node-level provenance. Accessible-name
        // fallback and visibility both use deliberately bounded heuristics, so
        // the whole node must be conservatively labelled until field-level
        // provenance is introduced.
        provenance: accessible.provenance,
    }))
}

fn shared_append_text<P: SemanticProducer>(
    target: &mut String,
    value: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<(), P::Error> {
    check().map_err(P::Error::from)?;
    let needed = target
        .len()
        .checked_add(value.len())
        .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
    crate::dom::projection_allocation::reserve_string(
        target,
        needed,
        &mut SemanticDomAllocation(producer),
    )?;
    for character in value.chars() {
        check().map_err(P::Error::from)?;
        target.push(character);
    }
    check().map_err(P::Error::from)
}

fn shared_name_tokens<'a, P: SemanticProducer>(
    value: &'a str,
    delimiter: impl Fn(char) -> bool,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<&'a str>, P::Error> {
    check().map_err(P::Error::from)?;
    let mut tokens = producer.vector(0)?;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        check().map_err(P::Error::from)?;
        if delimiter(character) {
            shared_vector_push(&mut tokens, &value[start..index], producer, check)?;
            start = index + character.len_utf8();
        }
    }
    shared_vector_push(&mut tokens, &value[start..], producer, check)?;
    Ok(tokens)
}

fn shared_normalize_text<P: SemanticProducer>(
    value: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    check().map_err(P::Error::from)?;
    producer.reserve(value.len())?;
    let mut output = String::new();
    output
        .try_reserve_exact(value.len())
        .map_err(|error| producer.allocation_error(error))?;
    let mut pending_space = false;
    for character in value.chars() {
        check().map_err(P::Error::from)?;
        if matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }
    check().map_err(P::Error::from)?;
    Ok(output)
}

fn shared_update_visibility<P: SemanticProducer>(
    hidden: &mut Option<bool>,
    declarations: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<(), P::Error> {
    for declaration in
        shared_name_tokens(declarations, |character| character == ';', producer, check)?
    {
        let tokens =
            shared_name_tokens(declaration, |character| character == ':', producer, check)?;
        if tokens.len() < 2 {
            continue;
        }
        let property = shared_ascii_lowercase(
            naming_trim_with_checkpoint(tokens[0], check)?,
            producer,
            check,
        )?;
        let value = shared_ascii_lowercase(
            naming_trim_with_checkpoint(&declaration[tokens[0].len() + 1..], check)?,
            producer,
            check,
        )?;
        match property.as_str() {
            "display" => *hidden = Some(value == "none"),
            "visibility" => *hidden = Some(matches!(value.as_str(), "hidden" | "collapse")),
            _ => {}
        }
    }
    check()?;
    Ok(())
}

impl SemanticGraph {
    fn shared_graph_clone<P: SemanticProducer>(
        &self,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<Self, P::Error> {
        fn map<K: Ord + Clone, V: Clone>(
            source: &BTreeMap<K, V>,
            check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
        ) -> Result<BTreeMap<K, V>, crate::dom::DomError> {
            let mut target = BTreeMap::new();
            for (key, value) in source {
                check()?;
                target.insert(key.clone(), value.clone());
            }
            Ok(target)
        }
        fn set<K: Ord + Clone>(
            source: &BTreeSet<K>,
            check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
        ) -> Result<BTreeSet<K>, crate::dom::DomError> {
            let mut target = BTreeSet::new();
            for value in source {
                check()?;
                target.insert(value.clone());
            }
            Ok(target)
        }
        check().map_err(P::Error::from)?;
        let mut target = Self::new(self.page_id, self.document_generation);
        for (id, node) in &self.full {
            check().map_err(P::Error::from)?;
            target
                .full
                .insert(*id, shared_semantic_node(node, producer, check)?);
        }
        target.compact = set(&self.compact, check).map_err(P::Error::from)?;
        for (id, hints) in &self.target_hints {
            let mut copied = producer.vector(hints.len())?;
            for (value, factor) in hints {
                check().map_err(P::Error::from)?;
                copied.push((producer.text(value, check)?, *factor));
            }
            target.target_hints.insert(*id, copied);
        }
        for (id, references) in &self.referenced_by {
            check().map_err(P::Error::from)?;
            target
                .referenced_by
                .insert(*id, set(references, check).map_err(P::Error::from)?);
        }
        target.accessibility_state_revisions =
            map(&self.accessibility_state_revisions, check).map_err(P::Error::from)?;
        for (id, node) in &self.accessibility {
            check().map_err(P::Error::from)?;
            target
                .accessibility
                .insert(*id, shared_accessibility(node, producer, check)?);
        }
        for (key, value) in &self.structures {
            check().map_err(P::Error::from)?;
            target.structures.insert(
                producer.text(key, check)?,
                shared_structure(value, producer, check)?,
            );
        }
        target.frame_ids = map(&self.frame_ids, check).map_err(P::Error::from)?;
        target.authored_hidden = set(&self.authored_hidden, check).map_err(P::Error::from)?;
        target.geometry_hidden = set(&self.geometry_hidden, check).map_err(P::Error::from)?;
        target.semantic_revision = self.semantic_revision;
        target.native_control_cursor = self.native_control_cursor;
        for (id, (before, after)) in &self.generated_content {
            check().map_err(P::Error::from)?;
            target.generated_content.insert(
                *id,
                (producer.text(before, check)?, producer.text(after, check)?),
            );
        }
        target.computed_visibility =
            map(&self.computed_visibility, check).map_err(P::Error::from)?;
        for (id, (display, transform)) in &self.computed_text_style {
            check().map_err(P::Error::from)?;
            target.computed_text_style.insert(
                *id,
                (
                    producer.text(display, check)?,
                    producer.text(transform, check)?,
                ),
            );
        }
        for (id, node) in &self.pending_upserts {
            check().map_err(P::Error::from)?;
            target
                .pending_upserts
                .insert(*id, shared_semantic_node(node, producer, check)?);
        }
        target.pending_removed = set(&self.pending_removed, check).map_err(P::Error::from)?;
        for (id, node) in &self.pending_accessibility_upserts {
            check().map_err(P::Error::from)?;
            target
                .pending_accessibility_upserts
                .insert(*id, shared_accessibility(node, producer, check)?);
        }
        target.pending_accessibility_removed =
            set(&self.pending_accessibility_removed, check).map_err(P::Error::from)?;
        for (key, value) in &self.pending_structure_upserts {
            check().map_err(P::Error::from)?;
            target.pending_structure_upserts.insert(
                producer.text(key, check)?,
                shared_structure(value, producer, check)?,
            );
        }
        for key in &self.pending_structure_removed {
            check().map_err(P::Error::from)?;
            target
                .pending_structure_removed
                .insert(producer.text(key, check)?);
        }
        target.diagnostics = producer.vector(self.diagnostics.len())?;
        for diagnostic in &self.diagnostics {
            check().map_err(P::Error::from)?;
            target.diagnostics.push(producer.text(diagnostic, check)?);
        }
        check().map_err(P::Error::from)?;
        Ok(target)
    }
}

fn shared_target_content_text<P: SemanticProducer>(
    dom: &DomArena,
    root: NodeId,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let mut pending = shared_accessible_children_with_checkpoint(dom, root, producer, check)?;
    pending.reverse();
    let mut seen = BTreeSet::new();
    let mut parts = producer.vector(0)?;
    while let Some(id) = pending.pop() {
        check()?;
        if !seen.insert(id) || shared_is_hidden(dom, id, producer, check)? {
            continue;
        }
        let Ok(node) = dom.get(id) else {
            continue;
        };
        match &node.kind {
            NodeKind::Text { data } => {
                let data = producer.text(data, check)?;
                shared_vector_push(&mut parts, data, producer, check)?;
            }
            NodeKind::Element { name }
                if matches!(
                    name.local.as_ref(),
                    "input" | "textarea" | "select" | "script" | "style" | "template"
                ) => {}
            _ => {
                for child in shared_accessible_children_with_checkpoint(dom, id, producer, check)?
                    .into_iter()
                    .rev()
                {
                    check()?;
                    shared_vector_push(&mut pending, child, producer, check)?;
                }
            }
        }
    }
    shared_normalize_text(&shared_join(&parts, " ", producer, check)?, producer, check)
}

impl SemanticGraph {
    fn shared_refresh_node<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        id: NodeId,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let previous_receipt_start = producer.receipt_mark();
        let previous = self
            .full
            .get(&id)
            .map(|node| shared_semantic_node(node, producer, check))
            .transpose()?;
        let previous_receipt_end = producer.receipt_mark();
        let projected = if dom.get(id).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name }
            if !matches!(name.local.as_ref(), "html" | "head" | "body" | "meta" | "title" | "script" | "style"))) {
            shared_project_element(dom, id, self.page_id, self.document_generation, id.slot.saturating_add(1), false, producer, check)?
        } else { None };
        match projected {
            Some(node) => {
                let mut hints = producer.vector(0)?;
                if let Ok(record) = dom.get(id) {
                    for (attribute, factor) in [
                        ("id", "exact_element_id"),
                        ("placeholder", "exact_placeholder"),
                    ] {
                        if let Some(value) = record
                            .attributes
                            .get_null_with_checkpoint(attribute, check)?
                            .map(|value| {
                                naming_trim_with_checkpoint(value, check)
                                    .map(|trimmed| (!trimmed.is_empty()).then_some(value))
                            })
                            .transpose()?
                            .flatten()
                        {
                            let value = producer.text(value, check)?;
                            shared_vector_push(&mut hints, (value, factor), producer, check)?;
                        }
                    }
                    if node.actions.contains(&SemanticAction::Click) {
                        // Read content below the root so an aria-label on a
                        // button doesn't erase its visible '+' / '×' label.
                        let text = shared_target_content_text(dom, id, producer, check)?;
                        if !naming_trim_with_checkpoint(&text, check)?.is_empty() {
                            let text = shared_normalize_text(&text, producer, check)?;
                            shared_vector_push(
                                &mut hints,
                                (text, "exact_content_label"),
                                producer,
                                check,
                            )?;
                        }
                    }
                }
                let authored_hidden = node.states.contains(&NodeState::Hidden);
                let mut node = node;
                if self.geometry_hidden.contains(&id) {
                    node.states.remove(&NodeState::Visible);
                    node.states.insert(NodeState::Hidden);
                }
                let compact = is_compact_semantic(&node);
                let changed = match previous.as_ref() {
                    Some(previous) => !semantic_node_equal_with_checkpoint(previous, &node, check)?,
                    None => true,
                };
                let upsert = if changed {
                    Some(shared_semantic_node(&node, producer, check)?)
                } else {
                    None
                };
                check()?;
                self.target_hints.insert(id, hints);
                if authored_hidden {
                    self.authored_hidden.insert(id);
                } else {
                    self.authored_hidden.remove(&id);
                }
                if compact {
                    self.compact.insert(id);
                } else {
                    self.compact.remove(&id);
                }
                if let Some(upsert) = upsert {
                    self.pending_removed.remove(&node.semantic_ref);
                    self.pending_upserts.insert(node.semantic_ref, upsert);
                }
                self.full.insert(id, node);
            }
            None => {
                self.target_hints.remove(&id);
                if let Some(previous) = self.full.remove(&id) {
                    self.pending_upserts.remove(&previous.semantic_ref);
                    self.pending_removed.insert(previous.semantic_ref);
                }
                self.compact.remove(&id);
                self.authored_hidden.remove(&id);
                self.geometry_hidden.remove(&id);
            }
        }
        // Equality comparison does not retain the cloned previous node. Drop
        // its buffers before releasing only their admission receipts; later
        // projected/upsert buffers and receipt-vector metadata remain charged.
        drop(previous);
        producer.release_receipt_range(previous_receipt_start..previous_receipt_end);
        Ok(())
    }
}

fn shared_tree_exclusion<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    role: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<String>, P::Error> {
    check()?;
    let Ok(node) = dom.get(id) else {
        return Ok(None);
    };
    if matches!(role, "none" | "presentation") {
        return Ok(Some(producer.text("presentational_role", check)?));
    }
    if matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "input")
        && node
            .attributes
            .get_null_with_checkpoint("type", check)?
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    {
        return Ok(Some(producer.text("hidden_input", check)?));
    }
    let mut lineage = producer.vector(1)?;
    lineage.push(id);
    for ancestor in shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
        dom, id, producer, check,
    ))? {
        check()?;
        shared_vector_push(&mut lineage, ancestor, producer, check)?;
    }
    for candidate in &lineage {
        check()?;
        if let Ok(node) = dom.get(*candidate)
            && node
                .attributes
                .contains_null_with_checkpoint("inert", check)?
        {
            return Ok(Some(producer.text("inert_subtree", check)?));
        }
    }
    let document = lineage.iter().copied().find(|candidate| {
        dom.get(*candidate)
            .is_ok_and(|node| matches!(node.kind, NodeKind::Document))
    });
    if let Some(document) = document {
        let mut candidates = producer.vector(1)?;
        candidates.push(document);
        for descendant in shared_descendants_with_checkpoint(dom, document, producer, check)? {
            check()?;
            shared_vector_push(&mut candidates, descendant, producer, check)?;
        }
        let mut modal = None;
        for candidate in candidates.into_iter().rev() {
            check()?;
            if let Ok(candidate_node) = dom.get(candidate)
                && candidate_node
                    .accessibility_attribute_with_checkpoint("aria-modal", check)?
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                && !shared_is_hidden(dom, candidate, producer, check)?
                && matches!(
                    shared_role_for_node(dom, candidate, producer, check)?.as_str(),
                    "dialog" | "alertdialog"
                )
            {
                modal = Some(candidate);
                break;
            }
        }
        if let Some(modal) = modal {
            let owners = accepted_aria_owners_with_checkpoint(dom, check)?;
            if !shared_accessibility_ancestors(dom, id, &owners, producer, check)?.contains(&modal)
                && id != modal
                && !shared_accessibility_ancestors(dom, modal, &owners, producer, check)?
                    .contains(&id)
            {
                return Ok(Some(producer.text("aria_modal_scope", check)?));
            }
        }
    }
    if shared_is_hidden(dom, id, producer, check)? {
        Ok(Some(producer.text("hidden_subtree", check)?))
    } else {
        Ok(None)
    }
}

fn shared_accessibility_ancestors<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    owners: &BTreeMap<NodeId, NodeId>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<NodeId>, P::Error> {
    let mut result = producer.vector(0)?;
    let mut seen = BTreeSet::from([id]);
    let mut cursor = owners
        .get(&id)
        .copied()
        .or_else(|| dom.get(id).ok().and_then(|node| node.parent));
    while let Some(parent) = cursor {
        check()?;
        if !seen.insert(parent) {
            break;
        }
        shared_vector_push(&mut result, parent, producer, check)?;
        cursor = owners
            .get(&parent)
            .copied()
            .or_else(|| dom.get(parent).ok().and_then(|node| node.parent));
    }
    Ok(result)
}

fn shared_accessibility_properties<P: SemanticProducer>(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    node: &crate::dom::Node,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<BTreeMap<String, serde_json::Value>, P::Error> {
    check()?;
    let attribute_start = producer.receipt_mark();
    let effective_attributes =
        node.accessibility_attributes_with_allocation(&mut SemanticDomAllocation(producer), check)?;
    let attribute_end = producer.receipt_mark();
    let attributes = &effective_attributes;
    let mut properties = JsonPropertiesGuard(BTreeMap::new());
    for attribute in [
        "aria-expanded",
        "aria-checked",
        "aria-selected",
        "aria-disabled",
        "aria-required",
        "aria-readonly",
        "aria-pressed",
        "aria-current",
        "aria-invalid",
        "aria-busy",
        "aria-modal",
        "aria-multiline",
        "aria-orientation",
        "aria-level",
        "aria-posinset",
        "aria-setsize",
        "aria-rowcount",
        "aria-colcount",
        "aria-rowindex",
        "aria-colindex",
        "aria-rowspan",
        "aria-colspan",
        "aria-live",
        "aria-atomic",
        "aria-relevant",
        "aria-roledescription",
        "aria-valuetext",
        "aria-haspopup",
    ] {
        if let Some(value) = attributes.get_null_with_checkpoint(attribute, check)? {
            properties.insert(
                shared_property_key(attribute, producer, check)?,
                shared_property_value(value, producer, check)?,
            );
        }
    }
    for attribute in ["aria-valuetext", "aria-roledescription"] {
        if let Some(value) = attributes.get_null_with_checkpoint(attribute, check)? {
            properties.insert(
                producer.text(attribute.trim_start_matches("aria-"), check)?,
                serde_json::Value::String(producer.text(value, check)?),
            );
        }
    }
    for attribute in ["aria-valuemin", "aria-valuemax", "aria-valuenow"] {
        if let Some(value) = attributes
            .get_null_with_checkpoint(attribute, check)?
            .and_then(|value| oryn_dom::parse_html_floating_point(value))
            .and_then(serde_json::Number::from_f64)
        {
            properties.insert(
                producer.text(attribute.trim_start_matches("aria-"), check)?,
                serde_json::Value::Number(value),
            );
        }
    }
    let placeholder_value = match attributes.get_null_with_checkpoint("placeholder", check)? {
        Some(value) => Some(value),
        None => attributes.get_null_with_checkpoint("aria-placeholder", check)?,
    };
    if let Some(value) = placeholder_value {
        properties.insert(
            producer.text("placeholder", check)?,
            serde_json::Value::String(producer.text(value, check)?),
        );
    }
    if attributes
        .get_null_with_checkpoint("aria-invalid", check)?
        .is_some_and(|value| !value.is_empty() && !value.eq_ignore_ascii_case("false"))
        && let Some(targets) = shared_relation(dom, id, "aria-errormessage", producer, check)?
    {
        let mut visible_targets = producer.vector(0)?;
        for target in targets {
            check()?;
            if !shared_is_hidden(dom, target, producer, check)? {
                shared_vector_push(&mut visible_targets, target, producer, check)?;
            }
        }
        let (message, _) = shared_referenced_nodes_text_with_checkpoint(
            dom,
            &visible_targets,
            PageId(1),
            1,
            producer,
            check,
        )?;
        if !message.is_empty() {
            properties.insert(
                producer.text("validation_error", check)?,
                serde_json::Value::String(message),
            );
        }
    }
    let role = shared_role_for_node(dom, id, producer, check)?;
    if matches!(
        role.as_str(),
        "slider" | "scrollbar" | "separator" | "spinbutton"
    ) {
        let spin = role == "spinbutton";
        let minimum = properties
            .get("valuemin")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(if spin { -9007199254740992. } else { 0. });
        let maximum = properties
            .get("valuemax")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(if spin { 9007199254740992. } else { 100. });
        let current = properties
            .get("valuenow")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_else(|| {
                if spin {
                    0.
                } else if maximum < minimum {
                    minimum
                } else {
                    minimum / 2. + maximum / 2.
                }
            });
        for (key, value) in [
            ("valuemin", minimum),
            ("valuemax", maximum),
            ("valuenow", current),
        ] {
            properties.insert(producer.text(key, check)?, serde_json::json!(value));
        }
    }
    if shared_accessibility_readonly(dom, id, producer, check)? {
        properties.insert(producer.text("readonly", check)?, serde_json::json!(true));
    }
    if matches!(
        role.as_str(),
        "checkbox" | "menuitemcheckbox" | "radio" | "menuitemradio" | "switch"
    ) {
        let value = attributes
            .get_null_with_checkpoint("aria-checked", check)?
            .map(|value| shared_ascii_lowercase(value, producer, check))
            .transpose()?;
        let checked = match value.as_deref() {
            Some("true") => serde_json::Value::Bool(true),
            Some("mixed") if matches!(role.as_str(), "checkbox" | "menuitemcheckbox") => {
                serde_json::Value::String(producer.text("mixed", check)?)
            }
            _ => serde_json::Value::Bool(false),
        };
        properties.insert(producer.text("checked", check)?, checked);
    }
    if role == "button" {
        let pressed = match attributes
            .get_null_with_checkpoint("aria-pressed", check)?
            .map(|value| shared_ascii_lowercase(value, producer, check))
            .transpose()?
            .as_deref()
        {
            Some("true") => serde_json::Value::Bool(true),
            Some("false") => serde_json::Value::Bool(false),
            Some("mixed") => serde_json::Value::String(producer.text("mixed", check)?),
            _ => serde_json::Value::String(producer.text("undefined", check)?),
        };
        properties.insert(producer.text("pressed", check)?, pressed);
    }
    if local_name == "input"
        && attributes
            .get_null_with_checkpoint("type", check)?
            .is_some_and(|kind| {
                kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
            })
    {
        properties.insert(
            producer.text("checked", check)?,
            if node.indeterminate
                && attributes
                    .get_null_with_checkpoint("type", check)?
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
            {
                serde_json::Value::String(producer.text("mixed", check)?)
            } else {
                serde_json::Value::Bool(node.checked_with_checkpoint(check)?)
            },
        );
    }
    for attribute in [
        "disabled", "required", "readonly", "checked", "selected", "open",
    ] {
        let kind = attributes
            .get_null_with_checkpoint("type", check)?
            .map(|kind| shared_ascii_lowercase(kind, producer, check))
            .transpose()?;
        let kind = match kind {
            Some(value) => value,
            None => producer.text("text", check)?,
        };
        let applicable = match attribute {
            "readonly" => {
                local_name == "textarea"
                    || (local_name == "input"
                        && !matches!(
                            kind.as_str(),
                            "hidden"
                                | "range"
                                | "color"
                                | "checkbox"
                                | "radio"
                                | "file"
                                | "button"
                                | "submit"
                                | "reset"
                                | "image"
                        ))
            }
            "required" => {
                matches!(local_name, "textarea" | "select")
                    || (local_name == "input"
                        && !matches!(
                            kind.as_str(),
                            "hidden" | "range" | "color" | "button" | "submit" | "reset" | "image"
                        ))
            }
            "checked" => local_name == "input" && matches!(kind.as_str(), "checkbox" | "radio"),
            "selected" => local_name == "option",
            "open" => matches!(local_name, "details" | "dialog"),
            "disabled" => matches!(
                local_name,
                "input" | "button" | "select" | "textarea" | "fieldset" | "optgroup" | "option"
            ),
            _ => false,
        };
        if applicable && attributes.contains_null_with_checkpoint(attribute, check)? {
            properties
                .entry(producer.text(attribute, check)?)
                .or_insert(serde_json::Value::Bool(true));
        }
    }
    if local_name == "table" {
        properties.insert(
            producer.text("native_table", check)?,
            serde_json::Value::Bool(true),
        );
        let mut groups = producer.vector(0)?;
        let mut start = 0u64;
        for child in &node.children {
            check()?;
            let Ok(group) = dom.get(*child) else {
                continue;
            };
            if !matches!(&group.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="colgroup")
            {
                continue;
            }
            let mut columns = producer.vector(0)?;
            for child in &group.children {
                check()?;
                if let Ok(column) = dom.get(*child)
                    && matches!(&column.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "col")
                {
                    shared_vector_push(&mut columns, column, producer, check)?;
                }
            }
            let width = if columns.is_empty() {
                naming_span_with_checkpoint(group, check)?
            } else {
                let mut width = 0;
                for column in columns {
                    check()?;
                    width += naming_span_with_checkpoint(column, check)?;
                }
                width
            };
            let mut pair = producer.vector(2)?;
            pair.push(serde_json::Value::from(start));
            pair.push(serde_json::Value::from(width));
            shared_vector_push(&mut groups, serde_json::Value::Array(pair), producer, check)?;
            start = start.saturating_add(width);
        }
        if !groups.is_empty() {
            properties.insert(
                producer.text("native_column_groups", check)?,
                serde_json::Value::Array(groups),
            );
        }
    }
    if matches!(local_name, "td" | "th") {
        properties.insert(
            producer.text("native_header", check)?,
            serde_json::Value::Bool(local_name == "th"),
        );
        let mut empty = true;
        for child in &node.children {
            check()?;
            let Ok(child) = dom.get(*child) else {
                empty = false;
                break;
            };
            match &child.kind {
                NodeKind::Element { .. } => {
                    empty = false;
                    break;
                }
                NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                    for ch in data.chars() {
                        check()?;
                        if !matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ') {
                            empty = false;
                            break;
                        }
                    }
                }
                _ => {}
            }
            if !empty {
                break;
            }
        }
        properties.insert(
            producer.text("native_empty_cell", check)?,
            serde_json::Value::Bool(empty),
        );
        if let Some(headers) = attributes.get_null_with_checkpoint("headers", check)? {
            let mut targets = producer.vector(0)?;
            for token in shared_name_tokens(
                headers,
                |character| character.is_ascii_whitespace(),
                producer,
                check,
            )?
            .into_iter()
            .filter(|token| !token.is_empty())
            {
                check()?;
                if let Some(target) = find_element_by_id_scoped_with_checkpoint(dom, id, token, check)?
                    && target != id && dom.get(target).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name }
                        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && matches!(name.local.as_ref(), "td" | "th")))
                { shared_vector_push(&mut targets, serde_json::Value::from((u64::from(target.generation) << 32) | u64::from(target.slot)), producer, check)?; }
            }
            properties.insert(
                producer.text("native_headers", check)?,
                serde_json::Value::Array(targets),
            );
        }
        if local_name == "th"
            && let Some(scope) = attributes
                .get_null_with_checkpoint("scope", check)?
                .map(|scope| shared_ascii_lowercase(scope, producer, check))
                .transpose()?
                .filter(|scope| matches!(scope.as_str(), "row" | "col" | "rowgroup" | "colgroup"))
        {
            properties.insert(
                producer.text("native_scope", check)?,
                serde_json::Value::String(scope),
            );
        }
        for (name, maximum, allow_zero) in
            [("rowspan", 65534u64, true), ("colspan", 1000u64, false)]
        {
            let parsed = attributes
                .get_null_with_checkpoint(name, check)?
                .map(|value| {
                    naming_u64_with_checkpoint(naming_trim_with_checkpoint(value, check)?, check)
                })
                .transpose()?
                .flatten()
                .unwrap_or(1);
            let value = if parsed == 0 && !allow_zero {
                1
            } else {
                parsed.min(maximum)
            };
            properties.insert(
                shared_surround("native_", name, "", producer, check)?,
                serde_json::json!(value),
            );
            properties.insert(
                shared_surround("native_", name, "_explicit", producer, check)?,
                serde_json::json!(attributes.contains_null_with_checkpoint(name, check)?),
            );
        }
    }
    if role == "heading"
        && properties
            .get("level")
            .and_then(serde_json::Value::as_i64)
            .is_none_or(|level| level < 1)
    {
        let level = if matches!(local_name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            local_name[1..].parse::<u8>().unwrap_or(2)
        } else {
            2
        };
        properties.insert(producer.text("level", check)?, serde_json::json!(level));
    }
    check()?;
    drop(effective_attributes);
    producer.release_receipt_range(attribute_start..attribute_end);
    Ok(std::mem::take(&mut properties.0))
}

fn shared_i64<P: SemanticProducer>(
    value: &str,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Option<i64>, P::Error> {
    let mut digits = value.bytes();
    let first = digits.next();
    check()?;
    let negative = first == Some(b'-');
    let mut result = 0i64;
    let mut count = 0usize;
    let rest = if matches!(first, Some(b'+' | b'-')) {
        None
    } else {
        first
    };
    for digit in rest.into_iter().chain(digits) {
        check()?;
        if !digit.is_ascii_digit() {
            return Ok(None);
        }
        count += 1;
        let digit = i64::from(digit - b'0');
        result = match result.checked_mul(10).and_then(|n| {
            if negative {
                n.checked_sub(digit)
            } else {
                n.checked_add(digit)
            }
        }) {
            Some(n) => n,
            None => return Ok(None),
        };
    }
    check()?;
    Ok((count > 0).then_some(result))
}

fn shared_property_value<P: SemanticProducer>(
    value: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<serde_json::Value, P::Error> {
    if shared_ascii_equal::<P>(value, "true", check)? {
        return Ok(serde_json::Value::Bool(true));
    }
    if shared_ascii_equal::<P>(value, "false", check)? {
        return Ok(serde_json::Value::Bool(false));
    }
    match shared_i64::<P>(value, check)? {
        Some(value) => Ok(serde_json::Value::from(value)),
        None => Ok(serde_json::Value::String(producer.text(value, check)?)),
    }
}

fn shared_property_key<P: SemanticProducer>(
    attribute: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let mut output = String::new();
    for ch in attribute.strip_prefix("aria-").unwrap_or(attribute).chars() {
        check()?;
        let ch = if ch == '-' { '_' } else { ch };
        let mut bytes = [0u8; 4];
        shared_append_text(&mut output, ch.encode_utf8(&mut bytes), producer, check)?;
    }
    Ok(output)
}

struct SemanticFormatWriter<'a, P: SemanticProducer, C> {
    output: String,
    producer: &'a mut P,
    check: &'a mut C,
    error: Option<P::Error>,
}
impl<P: SemanticProducer, C: FnMut() -> Result<(), crate::dom::DomError>> std::fmt::Write
    for SemanticFormatWriter<'_, P, C>
{
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        if self.error.is_some() {
            return Err(std::fmt::Error);
        }
        match shared_append_text(&mut self.output, value, self.producer, self.check) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.error = Some(error);
                Err(std::fmt::Error)
            }
        }
    }
}
fn shared_missing_id_diagnostic<P: SemanticProducer>(
    attribute: &str,
    idref: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    use std::fmt::Write;
    let mut writer = SemanticFormatWriter {
        output: String::new(),
        producer,
        check,
        error: None,
    };
    let written = write!(writer, "{attribute} references missing id {idref:?}");
    if let Some(error) = writer.error {
        return Err(error);
    }
    debug_assert!(
        written.is_ok(),
        "only the checked sink can fail String formatting"
    );
    Ok(writer.output)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Relationship resolution carries source/node, page/generation, diagnostic journal and producer/checkpoint ownership."
)]
fn shared_accessibility_relationships<P: SemanticProducer>(
    dom: &DomArena,
    source: NodeId,
    node: &crate::dom::Node,
    page_id: PageId,
    document_generation: u64,
    diagnostics: &mut Vec<String>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<BTreeMap<AccessibilityRelation, Vec<SemanticRef>>, P::Error> {
    let mut relationships = BTreeMap::new();
    for (attribute, relation) in [
        ("aria-labelledby", AccessibilityRelation::LabelledBy),
        ("aria-describedby", AccessibilityRelation::DescribedBy),
        ("aria-controls", AccessibilityRelation::Controls),
        ("aria-owns", AccessibilityRelation::Owns),
        (
            "aria-activedescendant",
            AccessibilityRelation::ActiveDescendant,
        ),
        ("aria-details", AccessibilityRelation::Details),
        ("aria-errormessage", AccessibilityRelation::ErrorMessage),
        ("aria-flowto", AccessibilityRelation::FlowTo),
    ] {
        check()?;
        if node.explicit_accessibility_references(attribute).is_none()
            && let Some(idrefs) = node.attributes.get_null_with_checkpoint(attribute, check)?
        {
            for idref in shared_name_tokens(idrefs, char::is_whitespace, producer, check)?
                .into_iter()
                .filter(|token| !token.is_empty())
            {
                check()?;
                if find_element_by_id_scoped_with_checkpoint(dom, source, idref, check)?.is_none() {
                    let diagnostic =
                        shared_missing_id_diagnostic(attribute, idref, producer, check)?;
                    shared_vector_push(diagnostics, diagnostic, producer, check)?;
                }
            }
        }
        if let Some(nodes) = shared_relation(dom, source, attribute, producer, check)? {
            let mut targets = producer.vector(0)?;
            for id in nodes {
                check()?;
                shared_vector_push(
                    &mut targets,
                    semantic_ref_for(id, page_id, document_generation),
                    producer,
                    check,
                )?;
            }
            if !targets.is_empty() {
                relationships.insert(relation, targets);
            }
        }
    }
    Ok(relationships)
}

impl SemanticGraph {
    fn shared_refresh_accessibility<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        id: NodeId,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let Some(semantic) = self.full.get(&id) else {
            if let Some(previous) = self.accessibility.remove(&id) {
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts
                        .remove(&previous.semantic_ref),
                ));
                self.pending_accessibility_removed
                    .insert(previous.semantic_ref);
                drop(AccessibilityOwner(Some(previous)));
            }
            return Ok(());
        };
        let Ok(node) = dom.get(id) else {
            return Ok(());
        };
        let NodeKind::Element { name } = &node.kind else {
            return Ok(());
        };
        let role = shared_role_for_node(dom, id, producer, check)?;
        let named = shared_accessible_semantics_with_checkpoint(
            dom,
            id,
            name.local.as_ref(),
            &role,
            self.page_id,
            self.document_generation,
            producer,
            check,
        )?;
        let exclusion_reason = shared_tree_exclusion(dom, id, &role, producer, check)?;
        let mut diagnostics = producer.vector(0)?;
        let accessible = AccessibilityNode {
            semantic_ref: semantic.semantic_ref,
            excluded: exclusion_reason.is_some(),
            exclusion_reason,
            role,
            name: named.name,
            description: named.description,
            relationships: shared_accessibility_relationships(
                dom,
                id,
                node,
                self.page_id,
                self.document_generation,
                &mut diagnostics,
                producer,
                check,
            )?,
            properties: shared_accessibility_properties(
                dom,
                id,
                name.local.as_ref(),
                node,
                producer,
                check,
            )?,
            owned_by: None,
            content_trust: ContentTrust::PageAuthored,
            provenance: named.provenance,
            evidence: named.evidence,
        };
        let mut candidate = AccessibilityOwner(Some(accessible));
        let accessible = candidate.0.as_ref().unwrap();
        let changed = match self.accessibility.get(&id) {
            Some(previous) => !accessibility_equal_with_checkpoint(previous, accessible, check)?,
            None => true,
        };
        let mut pending = if changed {
            AccessibilityOwner(Some(shared_accessibility(accessible, producer, check)?))
        } else {
            AccessibilityOwner(None)
        };
        check()?;
        let needed = self
            .diagnostics
            .len()
            .checked_add(diagnostics.len())
            .ok_or_else(|| P::Error::from(crate::dom::DomError::SemanticProjectionCapacity))?;
        crate::dom::projection_allocation::reserve_vector(
            &mut self.diagnostics,
            needed,
            &mut SemanticDomAllocation(producer),
        )?;
        check()?;
        self.diagnostics.extend(diagnostics);
        if changed {
            let accessible = candidate.0.take().unwrap();
            self.pending_accessibility_removed
                .remove(&accessible.semantic_ref);
            drop(AccessibilityOwner(
                self.pending_accessibility_upserts
                    .insert(accessible.semantic_ref, pending.0.take().unwrap()),
            ));
            drop(AccessibilityOwner(
                self.accessibility.insert(id, accessible),
            ));
        }
        Ok(())
    }
}

fn shared_structure_groups<P: SemanticProducer>(
    dom: &DomArena,
    kind: SemanticStructureKind,
    descendant_ids: &[NodeId],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
    accessible_by_ref: &BTreeMap<SemanticRef, &AccessibilityNode>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<SemanticGroup>, P::Error> {
    check()?;
    let group_role = match kind {
        SemanticStructureKind::Table | SemanticStructureKind::Grid => Some("row"),
        SemanticStructureKind::List => Some("listitem"),
        SemanticStructureKind::Menu => Some("menuitem"),
        SemanticStructureKind::Tree => Some("treeitem"),
        SemanticStructureKind::TabSet => Some("tab"),
        _ => None,
    };
    let Some(group_role) = group_role else {
        return Ok(Vec::new());
    };
    let mut groups = producer.vector(0)?;
    for id in descendant_ids {
        check()?;
        let Some(reference) = refs_by_dom_id.get(id).copied() else {
            continue;
        };
        if !accessible_by_ref
            .get(&reference)
            .is_some_and(|node| !node.excluded && node.role == group_role)
        {
            continue;
        }
        let mut members = producer.vector(1)?;
        members.push(reference);
        if group_role == "row" {
            for child in shared_descendants_with_checkpoint(dom, *id, producer, check)? {
                check()?;
                let Some(child) = refs_by_dom_id.get(&child).copied() else {
                    continue;
                };
                if accessible_by_ref.get(&child).is_some_and(|node| {
                    !node.excluded
                        && matches!(
                            node.role.as_str(),
                            "cell" | "gridcell" | "rowheader" | "columnheader"
                        )
                }) {
                    shared_vector_push(&mut members, child, producer, check)?;
                }
            }
        }
        let group = SemanticGroup {
            kind: producer.text(group_role, check)?,
            name: accessible_by_ref
                .get(&reference)
                .map(|node| producer.text(&node.name, check))
                .transpose()?
                .filter(|name| !name.is_empty()),
            members,
        };
        shared_vector_push(&mut groups, group, producer, check)?;
    }
    check()?;
    Ok(groups)
}

fn shared_project_structures<P: SemanticProducer>(
    dom: &DomArena,
    accessibility: &[AccessibilityNode],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<Vec<SemanticStructure>, P::Error> {
    check()?;
    let mut accessible_by_ref = BTreeMap::new();
    for node in accessibility {
        check()?;
        accessible_by_ref.insert(node.semantic_ref, node);
    }
    let mut structures = StructureListOwner(producer.vector(0)?);
    for root in accessibility.iter().filter(|node| !node.excluded) {
        check()?;
        let id = unpack_node_ref(root.semantic_ref.node);
        let Ok(dom_node) = dom.get(id) else {
            continue;
        };
        let NodeKind::Element { name } = &dom_node.kind else {
            continue;
        };
        let Some(kind) = structure_kind(name.local.as_ref(), &root.role, &dom_node.attributes)
        else {
            continue;
        };
        let descendants = shared_descendants_with_checkpoint(dom, id, producer, check)?;
        let mut members = producer.vector(0)?;
        for child in &descendants {
            check()?;
            if let Some(reference) = refs_by_dom_id.get(child).copied()
                && accessible_by_ref
                    .get(&reference)
                    .is_some_and(|node| !node.excluded)
            {
                shared_vector_push(&mut members, reference, producer, check)?;
            }
        }
        let groups = shared_structure_groups(
            dom,
            kind,
            &descendants,
            refs_by_dom_id,
            &accessible_by_ref,
            producer,
            check,
        )?;
        let mut properties = JsonPropertiesGuard(BTreeMap::new());
        properties.insert(
            producer.text("member_count", check)?,
            serde_json::json!(members.len()),
        );
        if kind == SemanticStructureKind::Form {
            if let Some(method) = dom_node
                .attributes
                .get_null_with_checkpoint("method", check)?
            {
                properties.insert(
                    producer.text("method", check)?,
                    serde_json::Value::String(producer.text(method, check)?),
                );
            }
            if let Some(action) = dom_node
                .attributes
                .get_null_with_checkpoint("action", check)?
            {
                properties.insert(
                    producer.text("action", check)?,
                    serde_json::Value::String(producer.text(action, check)?),
                );
            }
        }
        if kind == SemanticStructureKind::List {
            properties.insert(
                producer.text("ordered", check)?,
                serde_json::json!(name.local.as_ref() == "ol"),
            );
        }
        if kind == SemanticStructureKind::LiveRegion {
            properties.insert(
                producer.text("politeness", check)?,
                match root.properties.get("live") {
                    Some(value) => shared_json(value, producer, check)?,
                    None => serde_json::Value::String(
                        producer.text(implicit_live_politeness(&root.role), check)?,
                    ),
                },
            );
        }
        let mut references = producer.vector(1)?;
        references.push(root.semantic_ref);
        let mut evidence = producer.vector(1)?;
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::StructuralInference,
            source: shared_structure_evidence_source(
                name.local.as_ref(),
                &root.role,
                producer,
                check,
            )?,
            references,
        });
        let structure = SemanticStructure {
            structure_id: shared_structure_id(kind, root.semantic_ref.node, producer, check)?,
            kind,
            root: root.semantic_ref,
            parent_structure_id: None,
            name: producer.text(&root.name, check)?,
            members,
            groups,
            content_trust: ContentTrust::PageAuthored,
            provenance: Provenance::BrowserDeterministic,
            evidence,
            properties: std::mem::take(&mut properties.0),
        };
        shared_vector_push(&mut structures.0, structure, producer, check)?;
    }
    let mut roots = BTreeMap::new();
    for structure in &structures.0 {
        check()?;
        roots.insert(
            structure.root,
            producer.text(&structure.structure_id, check)?,
        );
    }
    for structure in &mut structures.0 {
        check()?;
        let id = unpack_node_ref(structure.root.node);
        structure.parent_structure_id = None;
        for ancestor in shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
            dom, id, producer, check,
        ))? {
            check()?;
            if let Some(value) = refs_by_dom_id
                .get(&ancestor)
                .and_then(|reference| roots.get(reference))
            {
                structure.parent_structure_id = Some(producer.text(value, check)?);
                break;
            }
        }
    }
    check()?;
    Ok(std::mem::take(&mut structures.0))
}

fn shared_structure_evidence_source<P: SemanticProducer>(
    local: &str,
    role: &str,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    let mut source = String::new();
    for part in ["element:", local, ";role:", role] {
        shared_append_text(&mut source, part, producer, check)?;
    }
    Ok(source)
}
fn shared_structure_id<P: SemanticProducer>(
    kind: SemanticStructureKind,
    node: u64,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    use std::fmt::Write;
    let mut writer = SemanticFormatWriter {
        output: String::new(),
        producer,
        check,
        error: None,
    };
    let written = write!(writer, "{kind:?}:{node}");
    if let Some(error) = writer.error {
        return Err(error);
    }
    debug_assert!(written.is_ok());
    // SAFETY: ASCII lowercasing changes only ASCII code points and therefore
    // preserves UTF-8; checkpoints never access or expose the buffer.
    for byte in unsafe { writer.output.as_bytes_mut() } {
        (writer.check)()?;
        byte.make_ascii_lowercase();
    }
    Ok(writer.output)
}

impl SemanticGraph {
    fn shared_rebuild_structures<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let mut refs_by_dom_id = BTreeMap::new();
        for (id, node) in &self.full {
            check()?;
            refs_by_dom_id.insert(*id, node.semantic_ref);
        }
        let mut accessibility = AccessibilityListOwner(producer.vector(0)?);
        for node in self.accessibility.values() {
            check()?;
            let node = shared_accessibility(node, producer, check)?;
            shared_vector_push(&mut accessibility.0, node, producer, check)?;
        }
        let mut next = StructureMapOwner(BTreeMap::new());
        for structure in
            shared_project_structures(dom, &accessibility.0, &refs_by_dom_id, producer, check)?
        {
            // This collection is owned before the next fallible checkpoint.
            next.0
                .insert(producer.text(&structure.structure_id, check)?, structure);
        }
        let mut removed = producer.vector(0)?;
        let mut upserts = StructureMapOwner(BTreeMap::new());
        for id in self.structures.keys() {
            check()?;
            if !next.0.contains_key(id) {
                let id = producer.text(id, check)?;
                shared_vector_push(&mut removed, id, producer, check)?;
            }
        }
        for (id, structure) in &next.0 {
            check()?;
            let changed = match self.structures.get(id) {
                Some(previous) => !structure_equal_with_checkpoint(previous, structure, check)?,
                None => true,
            };
            if changed {
                upserts.0.insert(
                    producer.text(id, check)?,
                    shared_structure(structure, producer, check)?,
                );
            }
        }
        check()?;
        for id in removed {
            if let Some(mut old) = self.pending_structure_upserts.remove(&id) {
                drop(JsonPropertiesGuard(std::mem::take(&mut old.properties)));
            }
            self.pending_structure_removed.insert(id);
        }
        for (id, structure) in std::mem::take(&mut upserts.0) {
            self.pending_structure_removed.remove(&id);
            if let Some(mut old) = self.pending_structure_upserts.insert(id, structure) {
                drop(JsonPropertiesGuard(std::mem::take(&mut old.properties)));
            }
        }
        drop(StructureMapOwner(std::mem::replace(
            &mut self.structures,
            std::mem::take(&mut next.0),
        )));
        Ok(())
    }
}

impl SemanticGraph {
    fn shared_rebuild_accessibility<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let mut previous = BTreeSet::new();
        for id in self.accessibility.keys() {
            check()?;
            previous.insert(*id);
        }
        let mut current = producer.vector(0)?;
        for id in self.full.keys() {
            check()?;
            shared_vector_push(&mut current, *id, producer, check)?;
        }
        for id in current {
            check()?;
            self.shared_refresh_accessibility(dom, id, producer, check)?;
        }
        for id in previous {
            check()?;
            if !self.full.contains_key(&id)
                && let Some(node) = self.accessibility.remove(&id)
            {
                drop(AccessibilityOwner(
                    self.pending_accessibility_upserts
                        .remove(&node.semantic_ref),
                ));
                self.pending_accessibility_removed.insert(node.semantic_ref);
                drop(AccessibilityOwner(Some(node)));
            }
        }
        self.shared_rebuild_ownership(dom, producer, check)?;
        check()?;
        Ok(())
    }
}

impl SemanticGraph {
    fn shared_rebuild_ownership<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let mut previous = BTreeMap::new();
        for (id, node) in &self.accessibility {
            check()?;
            previous.insert(*id, shared_accessibility(node, producer, check)?);
        }
        self.diagnostics.clear();
        let mut available = BTreeSet::new();
        for node in self.accessibility.values() {
            check()?;
            available.insert(node.semantic_ref);
        }
        for node in self.accessibility.values_mut() {
            check()?;
            node.owned_by = None;
            for targets in node.relationships.values_mut() {
                check()?;
                let mut kept = producer.vector(0)?;
                for target in targets.iter() {
                    check()?;
                    if available.contains(target) {
                        shared_vector_push(&mut kept, *target, producer, check)?;
                    }
                }
                *targets = kept;
            }
            node.relationships.retain(|_, targets| !targets.is_empty());
        }
        let accepted_owners = accepted_aria_owners_with_checkpoint(dom, check)?;
        let mut owner_by_target = BTreeMap::new();
        let mut ownership = producer.vector(0)?;
        for node in self.accessibility.values() {
            check()?;
            if let Some(targets) = node.relationships.get(&AccessibilityRelation::Owns) {
                let mut copied = producer.vector(0)?;
                for target in targets {
                    check()?;
                    shared_vector_push(&mut copied, *target, producer, check)?;
                }
                shared_vector_push(&mut ownership, (node.semantic_ref, copied), producer, check)?;
            }
        }
        for (owner, targets) in ownership {
            check()?;
            let owner_id = unpack_node_ref(owner.node);
            let mut accepted = producer.vector(0)?;
            for target in targets {
                check()?;
                let target_id = unpack_node_ref(target.node);
                let cycle =
                    ownership_cycle_with_checkpoint(&owner_by_target, owner, target, check)?
                        || shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
                            dom, owner_id, producer, check,
                        ))?
                        .contains(&target_id);
                if cycle
                    || owner_by_target.contains_key(&target)
                    || accepted_owners.get(&target_id) != Some(&owner_id)
                {
                    let diagnostic =
                        shared_invalid_owner_diagnostic(owner.node, target.node, producer, check)?;
                    shared_vector_push(&mut self.diagnostics, diagnostic, producer, check)?;
                    continue;
                }
                owner_by_target.insert(target, owner);
                shared_vector_push(&mut accepted, target, producer, check)?;
            }
            if let Some(node) = self.accessibility.get_mut(&owner_id) {
                node.relationships
                    .insert(AccessibilityRelation::Owns, accepted);
            }
        }
        for (target, owner) in owner_by_target {
            check()?;
            if let Some(node) = self.accessibility.get_mut(&unpack_node_ref(target.node)) {
                node.owned_by = Some(owner);
            }
        }
        for (id, node) in &self.accessibility {
            check()?;
            if match previous.get(id) {
                Some(previous) => !accessibility_equal_with_checkpoint(previous, node, check)?,
                None => true,
            } {
                self.pending_accessibility_upserts.insert(
                    node.semantic_ref,
                    shared_accessibility(node, producer, check)?,
                );
            }
        }
        check()?;
        Ok(())
    }
}

fn shared_invalid_owner_diagnostic<P: SemanticProducer>(
    owner: u64,
    target: u64,
    producer: &mut P,
    check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
) -> Result<String, P::Error> {
    use std::fmt::Write;
    let mut writer = SemanticFormatWriter {
        output: String::new(),
        producer,
        check,
        error: None,
    };
    let written = write!(writer, "ignored invalid aria-owns edge {owner} -> {target}");
    if let Some(error) = writer.error {
        return Err(error);
    }
    debug_assert!(written.is_ok());
    Ok(writer.output)
}

impl SemanticGraph {
    fn shared_reset<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        generation: u64,
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        if self.document_generation != generation {
            self.generated_content.clear();
            self.computed_visibility.clear();
            self.computed_text_style.clear();
        }
        let projection = self.layout_projection_with_checkpoint(dom, check)?;
        let dom = projection.as_ref().unwrap_or(dom);
        self.native_control_cursor = None;
        for node in self.full.values() {
            check()?;
            self.pending_removed.insert(node.semantic_ref);
        }
        for node in self.accessibility.values() {
            check()?;
            self.pending_accessibility_removed.insert(node.semantic_ref);
        }
        for id in self.structures.keys() {
            check()?;
            self.pending_structure_removed
                .insert(producer.text(id, check)?);
        }
        self.document_generation = generation;
        self.full.clear();
        self.compact.clear();
        self.target_hints.clear();
        for (_, node) in std::mem::take(&mut self.accessibility) {
            drop(AccessibilityOwner(Some(node)));
        }
        drop(StructureMapOwner(std::mem::take(&mut self.structures)));
        self.frame_ids.clear();
        self.authored_hidden.clear();
        self.geometry_hidden.clear();
        {
            let mut cursor = 0usize;
            while let Some((id, node)) = dom.next_with_checkpoint(&mut cursor, check)? {
                check()?;
                let NodeKind::Element { name } = &node.kind else {
                    continue;
                };
                if matches!(
                    name.local.as_ref(),
                    "html" | "head" | "body" | "meta" | "title" | "script" | "style"
                ) {
                    continue;
                }
                self.shared_refresh_node(dom, id, producer, check)?;
                check()?;
            }
        }
        check()?;
        self.rebuild_dependencies_with_checkpoint(dom, check)?;
        check()?;
        self.shared_rebuild_accessibility(dom, producer, check)?;
        check()?;
        self.shared_rebuild_structures(dom, producer, check)?;
        check()?;
        for node in self.full.values() {
            check()?;
            self.pending_removed.remove(&node.semantic_ref);
            self.pending_upserts.insert(
                node.semantic_ref,
                shared_semantic_node(node, producer, check)?,
            );
        }
        self.semantic_revision = self.semantic_revision.saturating_add(1);
        Ok(())
    }
}

impl SemanticGraph {
    fn shared_apply_mutations<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        mutations: &[crate::dom::Mutation],
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        let projection = self.layout_projection_with_checkpoint(dom, check)?;
        self.shared_apply_projected_mutations(
            projection.as_ref().unwrap_or(dom),
            mutations,
            producer,
            check,
        )
    }

    fn shared_apply_projected_mutations<P: SemanticProducer>(
        &mut self,
        dom: &DomArena,
        mutations: &[crate::dom::Mutation],
        producer: &mut P,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<(), P::Error> {
        check()?;
        if !mutations.is_empty() {
            self.native_control_cursor = None;
        }
        use crate::dom::Mutation;
        let mut dirty = BTreeSet::new();
        let mut dependency_change = false;
        // Modal accessibility scope affects siblings, not only descendants of
        // the changed dialog. Include the prior scope when a modal is removed.
        let mut modal = false;
        if !mutations.is_empty() {
            for node in self.accessibility.values() {
                check()?;
                if node.exclusion_reason.as_deref() == Some("aria_modal_scope") {
                    modal = true;
                    break;
                }
            }
            if !modal {
                {
                    let mut cursor = 0usize;
                    while let Some((_, node)) = dom.next_with_checkpoint(&mut cursor, check)? {
                        check()?;
                        if node
                            .accessibility_attribute_with_checkpoint("aria-modal", check)?
                            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                        {
                            modal = true;
                            break;
                        }
                    }
                }
            }
        }
        if modal {
            {
                let mut cursor = 0usize;
                while let Some((id, _)) = dom.next_with_checkpoint(&mut cursor, check)? {
                    check()?;
                    dirty.insert(id);
                }
            }
        }
        for mutation in mutations {
            check()?;
            match mutation {
                Mutation::HostedDocument { frame }
                | Mutation::DocumentOwnership { node: frame, .. } => {
                    // Association changes are not DOM child-list mutations.
                    // Reconcile old retained documents as well as the new tree.
                    dirty.insert(*frame);
                    for id in self.full.keys() {
                        check()?;
                        dirty.insert(*id);
                    }
                    {
                        let mut cursor = 0usize;
                        while let Some((id, _)) = dom.next_with_checkpoint(&mut cursor, check)? {
                            check()?;
                            dirty.insert(id);
                        }
                    }
                    dependency_change = true;
                }
                Mutation::Inserted { node, parent, .. } => {
                    dirty.insert(*node);
                    for id in
                        cancellation_or_default(descendants_with_checkpoint(dom, *node, check))?
                    {
                        check()?;
                        dirty.insert(id);
                    }
                    dirty.extend(parent.iter().copied());
                    dependency_change = true;
                }
                Mutation::Removed {
                    node,
                    former_parent,
                    ..
                } => {
                    self.remove_subtree_with_checkpoint(*node, check)?;
                    dirty.extend(former_parent.iter().copied());
                    dependency_change = true;
                }
                // Compound records only describe observer delivery; the native
                // insert/remove entries above already update semantic structure.
                Mutation::ChildrenReplaced { .. } => {}
                Mutation::Attribute { node, name, .. } => {
                    dirty.insert(*node);
                    // These ancestor properties affect descendant visibility,
                    // accessibility and action availability. Refreshing only
                    // the container leaves controls in a stale hidden state.
                    if matches!(
                        name.as_str(),
                        "style"
                            | "class"
                            | "hidden"
                            | "aria-hidden"
                            | "aria-readonly"
                            | "role"
                            | "inert"
                            | "disabled"
                    ) {
                        dirty.insert(*node);
                        for id in
                            cancellation_or_default(descendants_with_checkpoint(dom, *node, check))?
                        {
                            check()?;
                            dirty.insert(id);
                        }
                    }
                    dependency_change |= matches!(
                        name.as_str(),
                        "id" | "for"
                            | "type"
                            | "slot"
                            | "name"
                            | "aria-label"
                            | "aria-labelledby"
                            | "aria-describedby"
                            | "aria-errormessage"
                            | "aria-actions"
                            | "aria-owns"
                            | "role"
                            | "hidden"
                            | "aria-hidden"
                            | "style"
                            | "class"
                    );
                }
                Mutation::CharacterData { node, .. } => {
                    dirty.insert(*node);
                    dependency_change = true;
                }
                Mutation::StyleSheets { node } => {
                    dirty.insert(*node);
                    for id in
                        cancellation_or_default(descendants_with_checkpoint(dom, *node, check))?
                    {
                        check()?;
                        dirty.insert(id);
                    }
                    dependency_change = true;
                }
                Mutation::FormState { node } => {
                    dirty.insert(*node);
                    // Native FACE registration can change a label's owner.
                    dependency_change = true;
                    let revision = dom
                        .get(*node)
                        .map(|node| node.accessibility_reference_revision())
                        .unwrap_or(0);
                    if self
                        .accessibility_state_revisions
                        .get(node)
                        .copied()
                        .unwrap_or(0)
                        != revision
                    {
                        dirty.insert(*node);
                        for id in
                            cancellation_or_default(descendants_with_checkpoint(dom, *node, check))?
                        {
                            check()?;
                            dirty.insert(id);
                        }
                        dependency_change = true;
                    }
                }
                Mutation::ImageResource { node } | Mutation::EventListener { node, .. } => {
                    dirty.insert(*node);
                }
                Mutation::Focus { previous, current } => {
                    dirty.extend(previous.iter().copied());
                    dirty.extend(current.iter().copied());
                }
                Mutation::ShadowAttached { host, root, .. } => {
                    dirty.insert(*host);
                    dirty.insert(*root);
                    dependency_change = true;
                }
            }
        }
        let mut pending = producer.vector(0)?;
        for id in &dirty {
            check()?;
            shared_vector_push(&mut pending, *id, producer, check)?;
        }
        while let Some(node) = pending.pop() {
            check()?;
            let related = shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
                dom, node, producer, check,
            ))?
            .into_iter()
            .chain(self.referenced_by.get(&node).into_iter().flatten().copied());
            for target in related {
                check()?;
                if dirty.insert(target) {
                    shared_vector_push(&mut pending, target, producer, check)?;
                }
            }
        }
        if dependency_change {
            // Declared IDREF edges can remain identical while document order or
            // visibility changes which competing aria-owns edge is accepted.
            // Refresh both former and new owners before propagating name users.
            let current_owners = accepted_aria_owners_with_checkpoint(dom, check)?;
            let mut previous_owners = BTreeMap::new();
            for (target, node) in &self.accessibility {
                check()?;
                if let Some(owner) = node.owned_by {
                    previous_owners.insert(*target, unpack_node_ref(owner.node));
                }
            }
            for target in previous_owners.keys().chain(current_owners.keys()) {
                check()?;
                if previous_owners.get(target) != current_owners.get(target) {
                    dirty.insert(*target);
                    dirty.extend(previous_owners.get(target).copied());
                    dirty.extend(current_owners.get(target).copied());
                }
            }
            let mut previous_dependencies = BTreeMap::new();
            for (id, targets) in &self.referenced_by {
                check()?;
                let mut copied = BTreeSet::new();
                for target in targets {
                    check()?;
                    copied.insert(*target);
                }
                previous_dependencies.insert(*id, copied);
            }
            self.rebuild_dependencies_with_checkpoint(dom, check)?;
            let mut targets = BTreeSet::new();
            for target in previous_dependencies
                .keys()
                .chain(self.referenced_by.keys())
            {
                check()?;
                targets.insert(*target);
            }
            for target in targets {
                check()?;
                let previous = previous_dependencies.get(&target);
                let current = self.referenced_by.get(&target);
                if previous == current {
                    continue;
                }
                dirty.insert(target);
                if let Some(dependents) = previous {
                    for id in dependents {
                        check()?;
                        dirty.insert(*id);
                    }
                }
                if let Some(dependents) = current {
                    for id in dependents {
                        check()?;
                        dirty.insert(*id);
                    }
                }
            }
            // A newly resolved reference may name an ancestor of its owner.
            // Propagate through the rebuilt graph, not only the old edges.
            let mut pending = producer.vector(0)?;
            for id in &dirty {
                check()?;
                shared_vector_push(&mut pending, *id, producer, check)?;
            }
            while let Some(node) = pending.pop() {
                check()?;
                let related = shared_default::<Vec<NodeId>, P>(shared_ancestors_with_checkpoint(
                    dom, node, producer, check,
                ))?
                .into_iter()
                .chain(self.referenced_by.get(&node).into_iter().flatten().copied());
                for target in related {
                    check()?;
                    if dirty.insert(target) {
                        shared_vector_push(&mut pending, target, producer, check)?;
                    }
                }
            }
        }
        for node in dirty {
            check()?;
            self.shared_refresh_node(dom, node, producer, check)?;
            self.shared_refresh_accessibility(dom, node, producer, check)?;
        }
        if dependency_change {
            self.shared_rebuild_ownership(dom, producer, check)?;
            self.shared_rebuild_structures(dom, producer, check)?;
        }
        if !mutations.is_empty() {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
        check()?;
        Ok(())
    }
}

struct SemanticGraphAllocations {
    receipts: Vec<SemanticAllocationReceipt>,
}
impl std::fmt::Debug for SemanticGraphAllocations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticGraphAllocations")
            .field("receipts", &self.receipts.len())
            .finish()
    }
}
impl SemanticGraph {
    pub(crate) fn prepare_admitted_sync(
        &self,
        dom: &DomArena,
        generation: u64,
        mutations: &[crate::dom::Mutation],
        truncated: bool,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
        check: &mut impl FnMut() -> Result<(), crate::dom::DomError>,
    ) -> Result<Self, SemanticPreparationError> {
        let mut producer = SemanticLeaseOwner {
            admission,
            leases: Vec::new(),
        };
        let mut graph = self.shared_graph_clone(&mut producer, check)?;
        if truncated {
            graph.shared_reset(dom, generation, &mut producer, check)?;
        } else if !mutations.is_empty() {
            graph.shared_apply_mutations(dom, mutations, &mut producer, check)?;
        }
        producer.reserve(
            std::mem::size_of::<SemanticGraphAllocations>() + 2 * std::mem::size_of::<usize>(),
        )?;
        check()?;
        // The cloned draft owns independent buffers, so it does not retain the
        // previous graph's payload receipts after the old graph is retired.
        graph.allocation_receipts = Some(std::sync::Arc::new(SemanticGraphAllocations {
            receipts: std::mem::take(&mut producer.leases),
        }));
        Ok(graph)
    }
}

#[cfg(test)]
mod admitted_sync_controls {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Receipt(Arc<AtomicUsize>);
    impl Drop for Receipt {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Admission {
        calls: AtomicUsize,
        live: Arc<AtomicUsize>,
        fail: usize,
    }
    #[test]
    fn receipt_growth_charges_full_geometric_metadata_capacity() {
        use std::sync::Mutex;
        struct Ledger(Mutex<Vec<usize>>);
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for Ledger {
            fn checkpoint(&self) -> Result<(), String> {
                Ok(())
            }
            fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
                self.0.lock().unwrap().push(bytes);
                Ok(Arc::new(()))
            }
        }
        let ledger = Ledger(Mutex::new(Vec::new()));
        let mut owner = SemanticLeaseOwner {
            admission: &ledger,
            leases: Vec::new(),
        };
        for _ in 0..4096 {
            owner.reserve(1).unwrap();
        }
        let charges = ledger.0.lock().unwrap();
        let metadata: Vec<usize> = charges
            .iter()
            .copied()
            .filter(|bytes| *bytes != 1)
            .collect();
        assert!(metadata.len() <= 12);
        assert_eq!(
            metadata.iter().sum::<usize>(),
            owner.leases.capacity() * std::mem::size_of::<SemanticAllocationReceipt>()
        );
        assert_eq!(
            owner
                .leases
                .iter()
                .filter(|receipt| receipt._metadata.is_some())
                .count(),
            metadata.len()
        );
    }
    impl oryn_layout::cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            let boundary = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if boundary == self.fail {
                return Err("expired test owner".into());
            }
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Receipt(self.live.clone())))
        }
    }
    #[test]
    fn admitted_reset_preserves_projection_and_failed_source_journals() {
        let document = crate::html::parse(
            "<form><label for=x>Name</label><input id=x value=hello><button>Go</button></form>",
        )
        .into_runtime()
        .unwrap();
        let mut graph = SemanticGraph::new(PageId(63), 7);
        graph.reset(document.arena(), 7);
        let before = format!("{graph:?}");
        let mut ordinary = graph.try_clone_with_checkpoint(&mut || Ok(())).unwrap();
        ordinary
            .reset_with_checkpoint(document.arena(), 7, &mut || Ok(()))
            .unwrap();
        let probe = Admission {
            calls: AtomicUsize::new(0),
            live: Arc::new(AtomicUsize::new(0)),
            fail: usize::MAX,
        };
        let admitted = graph
            .prepare_admitted_sync(document.arena(), 7, &[], true, &probe, &mut || Ok(()))
            .unwrap();
        assert_eq!(
            admitted.nodes(ProjectionProfile::Full),
            ordinary.nodes(ProjectionProfile::Full)
        );
        assert_eq!(admitted.semantic_revision(), ordinary.semantic_revision());
        assert!(probe.live.load(Ordering::SeqCst) > 0);
        drop(admitted);
        assert_eq!(probe.live.load(Ordering::SeqCst), 0);
        for fail in 1..=probe.calls.load(Ordering::SeqCst) {
            let owner = Admission {
                calls: AtomicUsize::new(0),
                live: Arc::new(AtomicUsize::new(0)),
                fail,
            };
            assert!(matches!(
                graph.prepare_admitted_sync(document.arena(), 7, &[], true, &owner, &mut || Ok(())),
                Err(SemanticPreparationError::Admission(_))
            ));
            assert_eq!(owner.live.load(Ordering::SeqCst), 0);
            assert_eq!(format!("{graph:?}"), before);
        }
    }
    #[test]
    fn comparison_scratch_releases_exact_bytes_and_preserves_projection() {
        struct ByteAdmission { live: Arc<AtomicUsize> }
        struct ByteReceipt { live: Arc<AtomicUsize>, bytes: usize }
        impl Drop for ByteReceipt {
            fn drop(&mut self) { self.live.fetch_sub(self.bytes, Ordering::SeqCst); }
        }
        impl oryn_layout::cssom::native_import_graph::ImportAdmission for ByteAdmission {
            fn checkpoint(&self) -> Result<(), String> { Ok(()) }
            fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
                self.live.fetch_add(bytes, Ordering::SeqCst);
                Ok(Arc::new(ByteReceipt { live: self.live.clone(), bytes }))
            }
        }
        let document = crate::html::parse("<button id=b aria-label=Short>Visible</button>").into_runtime().unwrap();
        let id = document.find_by_id("b").unwrap();
        let mut graph = SemanticGraph::new(PageId(901), 1);
        graph.reset(document.arena(), 1);
        // The previous clone is comparison-only, deliberately distinct from
        // the current DOM so retaining its large allocation is observable.
        let large = "previous".repeat(8192);
        graph.full.get_mut(&id).unwrap().name = large.clone();
        let ledger = ByteAdmission { live: Arc::new(AtomicUsize::new(0)) };
        let mut producer = SemanticLeaseOwner { admission: &ledger, leases: Vec::new() };
        graph.shared_refresh_node(document.arena(), id, &mut producer, &mut || Ok(())).unwrap();
        let retained_large_receipts = producer.leases.iter().filter(|receipt|
            receipt.payload.is_some()).count();
        assert!(retained_large_receipts > 0, "live projected output must retain admission");
        assert!(ledger.live.load(Ordering::SeqCst) < large.len(),
            "dead previous-node comparison buffer must not remain charged");
        let ordinary = {
            let mut graph = SemanticGraph::new(PageId(901), 1);
            graph.reset(document.arena(), 1);
            graph
        };
        assert_eq!(graph.full.get(&id), ordinary.full.get(&id));
        assert!(producer.leases.iter().any(|receipt| receipt.payload.is_none()),
            "comparison receipt was released without removing metadata custody");
        drop(graph);
        drop(producer);
        assert_eq!(ledger.live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn property_signed_integer_matches_original_rust_parser() {
        for value in [
            "",
            "+",
            "-",
            " 1",
            "1 ",
            "+0",
            "-0",
            "00001",
            "9223372036854775807",
            "-9223372036854775808",
            "9223372036854775808",
            "-9223372036854775809",
        ] {
            assert_eq!(
                shared_i64::<OrdinarySemanticProducer>(value, &mut || Ok(())).unwrap(),
                value.parse::<i64>().ok(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn admitted_json_clone_deep_stack_and_cancellation_dispose_iteratively() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut value = serde_json::Value::String("leaf".into());
                for depth in 0..16_384 {
                    value = if depth % 2 == 0 {
                        serde_json::Value::Array(vec![value])
                    } else {
                        let mut map = serde_json::Map::new();
                        map.insert("key".into(), value);
                        serde_json::Value::Object(map)
                    };
                }
                let input = JsonValuesGuard(vec![value]);
                let admission = Admission {
                    calls: AtomicUsize::new(0),
                    live: Arc::new(AtomicUsize::new(0)),
                    fail: usize::MAX,
                };
                let mut producer = SemanticLeaseOwner {
                    admission: &admission,
                    leases: Vec::new(),
                };
                let copied = shared_json(&input.0[0], &mut producer, &mut || Ok(())).unwrap();
                let copied = JsonValuesGuard(vec![copied]);
                let mut cursor = &copied.0[0];
                let mut count = 0;
                loop {
                    cursor = match cursor {
                        serde_json::Value::Array(values) => &values[0],
                        serde_json::Value::Object(values) => values.get("key").unwrap(),
                        serde_json::Value::String(value) => {
                            assert_eq!(value, "leaf");
                            break;
                        }
                        _ => panic!("unexpected copied value"),
                    };
                    count += 1;
                }
                assert_eq!(count, 16_384);
                drop(copied);
                drop(producer);
                assert_eq!(admission.live.load(Ordering::SeqCst), 0);
                let admission = Admission {
                    calls: AtomicUsize::new(0),
                    live: Arc::new(AtomicUsize::new(0)),
                    fail: usize::MAX,
                };
                let mut producer = SemanticLeaseOwner {
                    admission: &admission,
                    leases: Vec::new(),
                };
                let mut calls = 0;
                assert!(
                    shared_json(&input.0[0], &mut producer, &mut || {
                        calls += 1;
                        if calls == 4096 {
                            Err(crate::dom::DomError::ProjectionCancelled)
                        } else {
                            Ok(())
                        }
                    })
                    .is_err()
                );
                drop(producer);
                assert_eq!(admission.live.load(Ordering::SeqCst), 0);
                drop(input);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

#[cfg(test)]
mod scratch_receipt_reuse_controls {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    struct Admission { live: Arc<AtomicUsize> }
    struct Lease { live: Arc<AtomicUsize>, bytes: usize }
    impl Drop for Lease { fn drop(&mut self) { self.live.fetch_sub(self.bytes, Ordering::SeqCst); } }
    impl oryn_layout::cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> { Ok(()) }
        fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
            self.live.fetch_add(bytes, Ordering::SeqCst);
            Ok(Arc::new(Lease { live: self.live.clone(), bytes }))
        }
    }
    #[test]
    fn scratch_tail_reuses_capacity_and_preserves_interior_live_range() {
        let admission = Admission { live: Arc::new(AtomicUsize::new(0)) };
        let mut owner = SemanticLeaseOwner { admission: &admission, leases: Vec::new() };
        owner.reserve(123).unwrap();
        let retained = owner.leases[0].payload.as_ref().unwrap().clone();
        let before = owner.receipt_mark();
        owner.reserve(17).unwrap();
        let middle_end = owner.receipt_mark();
        owner.reserve(31).unwrap();
        let later = owner.leases[middle_end].payload.as_ref().unwrap().clone();
        owner.release_receipt_range(before..middle_end);
        assert!(owner.leases[before].payload.is_none());
        assert!(Arc::ptr_eq(owner.leases[middle_end].payload.as_ref().unwrap(), &later));
        assert!(Arc::ptr_eq(owner.leases[0].payload.as_ref().unwrap(), &retained));
        for _ in 0..2 { let mark = owner.receipt_mark(); owner.reserve(7).unwrap(); owner.reserve(11).unwrap(); owner.release_scratch(mark); }
        let capacity = owner.leases.capacity();
        let count = owner.leases.len();
        let charged = admission.live.load(Ordering::SeqCst);
        for _ in 0..10000 {
            let mark = owner.receipt_mark();
            owner.reserve(7).unwrap();
            owner.reserve(11).unwrap();
            owner.release_scratch(mark);
        }
        assert_eq!(owner.leases.capacity(), capacity);
        assert_eq!(owner.leases.len(), count);
        assert_eq!(admission.live.load(Ordering::SeqCst), charged);
        assert!(Arc::ptr_eq(owner.leases[middle_end].payload.as_ref().unwrap(), &later));
        drop(retained); drop(later); drop(owner);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
    }
}
