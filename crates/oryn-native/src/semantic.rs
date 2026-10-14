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

pub const SEMANTIC_GRAPH_VERSION: u16 = 2;

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

    pub fn reset(&mut self, dom: &DomArena, document_generation: u64) {
        if self.document_generation != document_generation {
            self.generated_content.clear();
            self.computed_visibility.clear();
            self.computed_text_style.clear();
        }
        let projection = self.layout_projection(dom);
        self.reset_projected(projection.as_ref().unwrap_or(dom), document_generation);
    }

    fn reset_projected(&mut self, dom: &DomArena, document_generation: u64) {
        self.native_control_cursor = None;
        for node in self.full.values() {
            self.pending_removed.insert(node.semantic_ref);
        }
        for node in self.accessibility.values() {
            self.pending_accessibility_removed.insert(node.semantic_ref);
        }
        for id in self.structures.keys() {
            self.pending_structure_removed.insert(id.clone());
        }
        self.document_generation = document_generation;
        self.full.clear();
        self.compact.clear();
        self.target_hints.clear();
        self.accessibility.clear();
        self.structures.clear();
        self.frame_ids.clear();
        self.authored_hidden.clear();
        self.geometry_hidden.clear();
        for (id, node) in dom.iter() {
            let NodeKind::Element { name } = &node.kind else {
                continue;
            };
            if matches!(
                name.local.as_ref(),
                "html" | "head" | "body" | "meta" | "title" | "script" | "style"
            ) {
                continue;
            }
            self.refresh_node(dom, id);
        }
        self.rebuild_dependencies(dom);
        self.rebuild_accessibility(dom);
        self.rebuild_structures(dom);
        for node in self.full.values() {
            self.pending_removed.remove(&node.semantic_ref);
            self.pending_upserts.insert(node.semantic_ref, node.clone());
        }
        self.semantic_revision = self.semantic_revision.saturating_add(1);
    }

    pub fn apply_mutations(&mut self, dom: &DomArena, mutations: &[crate::dom::Mutation]) {
        let projection = self.layout_projection(dom);
        self.apply_projected_mutations(projection.as_ref().unwrap_or(dom), mutations);
    }

    fn layout_projection(&self, dom: &DomArena) -> Option<DomArena> {
        if self.generated_content.is_empty()
            && self.computed_visibility.is_empty()
            && self.computed_text_style.is_empty()
        {
            return None;
        }
        let mut projection = dom.with_accessibility_generated_content(&self.generated_content);
        projection.set_accessibility_computed_visibility(&self.computed_visibility);
        projection.set_accessibility_computed_text_style(&self.computed_text_style);
        Some(projection)
    }

    pub fn apply_computed_visibility(
        &mut self,
        dom: &DomArena,
        styles: &oryn_layout::ComputedStyles,
    ) {
        let next = styles
            .iter()
            .filter_map(|(id, style)| {
                let node = unpack_node_ref(*id);
                dom.get(node).ok()?;
                Some((
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
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let text_style = styles
            .iter()
            .filter_map(|(id, style)| {
                let node = unpack_node_ref(*id);
                dom.get(node).ok()?;
                Some((
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
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut changed = self
            .computed_text_style
            .keys()
            .chain(text_style.keys())
            .copied()
            .filter(|id| self.computed_text_style.get(id) != text_style.get(id))
            .collect::<BTreeSet<_>>();
        changed.extend(
            self.computed_visibility
                .keys()
                .chain(next.keys())
                .copied()
                .filter(|id| self.computed_visibility.get(id) != next.get(id)),
        );
        if changed.is_empty() {
            return;
        }
        self.computed_visibility = next;
        self.computed_text_style = text_style;
        let mut affected = changed.clone();
        for node in changed {
            affected.extend(dom.descendants_inclusive(node).unwrap_or_default());
        }
        self.apply_mutations(
            dom,
            &affected
                .into_iter()
                .map(|node| crate::dom::Mutation::FormState { node })
                .collect::<Vec<_>>(),
        );
    }

    /// Native layout owns pseudo strings. A changed pseudo invalidates ordinary
    /// name-from-content ancestors and native ARIA/label dependents through the
    /// same incremental graph as DOM text. Paint-only synthetic IDs never enter.
    pub fn apply_generated_content(
        &mut self,
        dom: &DomArena,
        content: &[oryn_common::v4::GeneratedContent],
    ) {
        let mut next: BTreeMap<NodeId, (String, String)> = BTreeMap::new();
        let mut markers: BTreeMap<NodeId, String> = BTreeMap::new();
        for item in content {
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
        let changed = self
            .generated_content
            .keys()
            .chain(next.keys())
            .copied()
            .filter(|id| self.generated_content.get(id) != next.get(id))
            .collect::<BTreeSet<_>>();
        if changed.is_empty() {
            return;
        }
        self.generated_content = next;
        let mut affected = changed;
        // A generated descendant can name a referenced ancestor (label/span).
        // Include those ancestors before reference dependency expansion.
        for node in affected.iter().copied().collect::<Vec<_>>() {
            affected.extend(dom.ancestors(node).unwrap_or_default());
        }
        let mutations = affected
            .into_iter()
            .map(|node| crate::dom::Mutation::FormState { node })
            .collect::<Vec<_>>();
        self.apply_mutations(dom, &mutations);
    }

    fn apply_projected_mutations(&mut self, dom: &DomArena, mutations: &[crate::dom::Mutation]) {
        if !mutations.is_empty() {
            self.native_control_cursor = None;
        }
        use crate::dom::Mutation;
        let mut dirty = BTreeSet::new();
        let mut dependency_change = false;
        // Modal accessibility scope affects siblings, not only descendants of
        // the changed dialog. Include the prior scope when a modal is removed.
        if !mutations.is_empty()
            && (self
                .accessibility
                .values()
                .any(|node| node.exclusion_reason.as_deref() == Some("aria_modal_scope"))
                || dom.iter().any(|(_, node)| {
                    node.accessibility_attribute("aria-modal")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                }))
        {
            dirty.extend(dom.iter().map(|(id, _)| id));
        }
        for mutation in mutations {
            match mutation {
                Mutation::HostedDocument { frame }
                | Mutation::DocumentOwnership { node: frame, .. } => {
                    // Association changes are not DOM child-list mutations.
                    // Reconcile old retained documents as well as the new tree.
                    dirty.insert(*frame);
                    dirty.extend(self.full.keys().copied());
                    dirty.extend(dom.iter().map(|(id, _)| id));
                    dependency_change = true;
                }
                Mutation::Inserted { node, parent, .. } => {
                    dirty.insert(*node);
                    dirty.extend(dom.descendants_inclusive(*node).unwrap_or_default());
                    dirty.extend(parent.iter().copied());
                    dependency_change = true;
                }
                Mutation::Removed {
                    node,
                    former_parent,
                    ..
                } => {
                    self.remove_subtree(*node);
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
                        dirty.extend(dom.descendants_inclusive(*node).unwrap_or_default());
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
                    dirty.extend(dom.descendants_inclusive(*node).unwrap_or_default());
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
                        dirty.extend(dom.descendants_inclusive(*node).unwrap_or_default());
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
        let mut pending = dirty.iter().copied().collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            let related = dom
                .ancestors(node)
                .unwrap_or_default()
                .into_iter()
                .chain(self.referenced_by.get(&node).into_iter().flatten().copied());
            for target in related {
                if dirty.insert(target) {
                    pending.push(target);
                }
            }
        }
        if dependency_change {
            // Declared IDREF edges can remain identical while document order or
            // visibility changes which competing aria-owns edge is accepted.
            // Refresh both former and new owners before propagating name users.
            let current_owners = accepted_aria_owners(dom);
            let previous_owners = self
                .accessibility
                .iter()
                .filter_map(|(target, node)| {
                    node.owned_by
                        .map(|owner| (*target, unpack_node_ref(owner.node)))
                })
                .collect::<BTreeMap<_, _>>();
            for target in previous_owners.keys().chain(current_owners.keys()) {
                if previous_owners.get(target) != current_owners.get(target) {
                    dirty.insert(*target);
                    dirty.extend(previous_owners.get(target).copied());
                    dirty.extend(current_owners.get(target).copied());
                }
            }
            let previous_dependencies = self.referenced_by.clone();
            self.rebuild_dependencies(dom);
            let targets = previous_dependencies
                .keys()
                .chain(self.referenced_by.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            for target in targets {
                let previous = previous_dependencies.get(&target);
                let current = self.referenced_by.get(&target);
                if previous == current {
                    continue;
                }
                dirty.insert(target);
                if let Some(dependents) = previous {
                    dirty.extend(dependents);
                }
                if let Some(dependents) = current {
                    dirty.extend(dependents);
                }
            }
            // A newly resolved reference may name an ancestor of its owner.
            // Propagate through the rebuilt graph, not only the old edges.
            let mut pending = dirty.iter().copied().collect::<Vec<_>>();
            while let Some(node) = pending.pop() {
                let related = dom
                    .ancestors(node)
                    .unwrap_or_default()
                    .into_iter()
                    .chain(self.referenced_by.get(&node).into_iter().flatten().copied());
                for target in related {
                    if dirty.insert(target) {
                        pending.push(target);
                    }
                }
            }
        }
        for node in dirty {
            self.refresh_node(dom, node);
            self.refresh_accessibility(dom, node);
        }
        if dependency_change {
            self.rebuild_accessibility_ownership(dom);
            self.rebuild_structures(dom);
        }
        if !mutations.is_empty() {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
    }

    /// Returns graph changes accumulated since the previous publication.
    /// Consumers use this journal directly instead of serializing and diffing
    /// complete observations at every revision.
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
        let cursor = dom.mutation_cursor();
        if self.native_control_cursor == Some(cursor) {
            return Ok(());
        }
        let mut changed = false;
        for (&id, node) in &mut self.full {
            let record = dom.arena().get(id)?;
            let custom_control = dom.is_form_associated_custom_element(id);
            let control = custom_control
                || matches!(&record.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && matches!(name.local.as_ref(), "input" | "textarea" | "select" | "button" | "fieldset" | "output" | "object"));
            let validation = if control {
                let numeric = dom.control_numeric_validity(id)?;
                let flag = |name: &str, builtin: bool| {
                    if custom_control {
                        dom.custom_element_validity_flag(id, name)
                    } else {
                        builtin
                    }
                };
                Some(oryn_common::v3::NativeValidationState {
                    bad_input: flag("badInput", dom.control_bad_input(id)?),
                    too_short: flag("tooShort", dom.control_length_validity(id)?.0),
                    too_long: flag("tooLong", dom.control_length_validity(id)?.1),
                    pattern_mismatch: flag("patternMismatch", dom.control_pattern_mismatch(id)?),
                    range_underflow: flag("rangeUnderflow", numeric.range_underflow),
                    range_overflow: flag("rangeOverflow", numeric.range_overflow),
                    step_mismatch: flag("stepMismatch", numeric.step_mismatch),
                    will_validate: dom.control_will_validate(id)?,
                    valid: dom.control_validity(id)?,
                    value_missing: flag("valueMissing", dom.control_value_missing(id)?),
                    type_mismatch: flag("typeMismatch", dom.control_type_mismatch(id)?),
                    custom_error: flag("customError", !dom.custom_validity_message(id)?.is_empty()),
                })
            } else {
                None
            };
            let focused = dom.focused() == Some(id);
            let disabled = if control || record.submit_behavior.is_some() {
                dom.control_disabled(id)?
                    || aria_true(&record.accessibility_attributes(), "aria-disabled")
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
            self.pending_upserts.insert(node.semantic_ref, node.clone());
            changed = true;
        }
        for (&id, accessible) in &mut self.accessibility {
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
                oryn_dom::parse_html_floating_point(&dom.control_value(id)?)
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
                    self.pending_accessibility_upserts
                        .insert(accessible.semantic_ref, accessible.clone());
                    changed = true;
                }
            }

            let invoker = record.accessibility_attribute("aria-expanded").is_none()
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
                    self.pending_accessibility_upserts
                        .insert(accessible.semantic_ref, accessible.clone());
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
                        self.pending_upserts.insert(node.semantic_ref, node.clone());
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
                    self.pending_accessibility_upserts
                        .insert(accessible.semantic_ref, accessible.clone());
                    changed = true;
                }
            }
            if record.submit_behavior.is_some()
                || self
                    .full
                    .get(&id)
                    .is_some_and(|node| node.validation.is_some())
            {
                let disabled = dom.control_disabled(id)?
                    || aria_true(&record.accessibility_attributes(), "aria-disabled");
                let value = serde_json::Value::Bool(disabled);
                if accessible.properties.get("disabled") != Some(&value) {
                    accessible.properties.insert("disabled".into(), value);
                    self.pending_accessibility_upserts
                        .insert(accessible.semantic_ref, accessible.clone());
                    changed = true;
                }
            }
            let inert = dom.is_inert(id)?;
            let was_native = accessible.exclusion_reason.as_deref() == Some("native_inert");
            if inert && !accessible.excluded {
                accessible.excluded = true;
                accessible.exclusion_reason = Some("native_inert".into());
                self.pending_accessibility_upserts
                    .insert(accessible.semantic_ref, accessible.clone());
                changed = true;
            } else if !inert && was_native {
                accessible.excluded = false;
                accessible.exclusion_reason = None;
                self.pending_accessibility_upserts
                    .insert(accessible.semantic_ref, accessible.clone());
                changed = true;
            }
        }
        if changed {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
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
        use oryn_common::v4::GeometryAvailability;
        let mut changed = false;
        for node in self.full.values_mut() {
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
            self.pending_upserts.insert(node.semantic_ref, node.clone());
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
                self.pending_accessibility_upserts
                    .insert(accessible.semantic_ref, accessible.clone());
            }
            changed = true;
        }
        if changed {
            self.semantic_revision = self.semantic_revision.saturating_add(1);
        }
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
        let id = unpack_node_ref(reference.node);
        let accessible = self.accessibility.get(&id);
        let structure_ids: Vec<String> = self
            .structures
            .values()
            .filter(|structure| {
                structure.root == reference || structure.members.contains(&reference)
            })
            .map(|structure| structure.structure_id.clone())
            .collect();
        let deterministic = |field| SemanticFieldEvidence {
            field,
            trust: ContentTrust::PageAuthored,
            provenance: Provenance::BrowserDeterministic,
            evidence: Vec::new(),
        };
        let mut field_evidence = vec![
            deterministic(SemanticField::Role),
            deterministic(SemanticField::State),
            deterministic(SemanticField::Action),
            deterministic(SemanticField::Visibility),
        ];
        if semantic.value.is_some() {
            field_evidence.push(deterministic(SemanticField::Value));
        }
        if !structure_ids.is_empty() {
            field_evidence.push(deterministic(SemanticField::Structure));
        }
        if let Some(accessible) = accessible {
            field_evidence.push(SemanticFieldEvidence {
                field: SemanticField::Name,
                trust: ContentTrust::PageAuthored,
                provenance: accessible.provenance.clone(),
                evidence: accessible.evidence.clone(),
            });
            if accessible.description.is_some() {
                field_evidence.push(SemanticFieldEvidence {
                    field: SemanticField::Description,
                    trust: ContentTrust::PageAuthored,
                    provenance: accessible.provenance.clone(),
                    evidence: accessible.evidence.clone(),
                });
            }
            if !accessible.relationships.is_empty() {
                field_evidence.push(deterministic(SemanticField::Relationship));
            }
        }
        Some(SemanticNodeV6 {
            semantic,
            frame_id: self.frame_ids.get(&id).copied(),
            relationships: accessible
                .map(|node| node.relationships.clone())
                .unwrap_or_default(),
            accessibility_properties: accessible
                .map(|node| node.properties.clone())
                .unwrap_or_default(),
            structure_ids,
            geometry: None,
            field_evidence,
        })
    }

    fn refresh_node(&mut self, dom: &DomArena, id: NodeId) {
        let previous = self.full.get(&id).cloned();
        let projected = dom.get(id).ok().and_then(|node| {
            let NodeKind::Element { name } = &node.kind else {
                return None;
            };
            (!matches!(
                name.local.as_ref(),
                "html" | "head" | "body" | "meta" | "title" | "script" | "style"
            ))
            .then(|| {
                project_element(
                    dom,
                    id,
                    self.page_id,
                    self.document_generation,
                    id.slot.saturating_add(1),
                )
            })
            .flatten()
        });
        match projected {
            Some(node) => {
                let mut hints = Vec::new();
                if let Ok(record) = dom.get(id) {
                    for (attribute, factor) in [
                        ("id", "exact_element_id"),
                        ("placeholder", "exact_placeholder"),
                    ] {
                        if let Some(value) = record
                            .attributes
                            .get_null(attribute)
                            .filter(|value| !value.trim().is_empty())
                        {
                            hints.push((value.clone(), factor));
                        }
                    }
                    if node.actions.contains(&SemanticAction::Click) {
                        // Read content below the root so an aria-label on a
                        // button doesn't erase its visible '+' / '×' label.
                        let text = target_content_text(dom, id);
                        if !text.trim().is_empty() {
                            hints.push((normalize_text(&text), "exact_content_label"));
                        }
                    }
                }
                self.target_hints.insert(id, hints);
                if node.states.contains(&NodeState::Hidden) {
                    self.authored_hidden.insert(id);
                } else {
                    self.authored_hidden.remove(&id);
                }
                let mut node = node;
                if self.geometry_hidden.contains(&id) {
                    node.states.remove(&NodeState::Visible);
                    node.states.insert(NodeState::Hidden);
                }
                if is_compact_semantic(&node) {
                    self.compact.insert(id);
                } else {
                    self.compact.remove(&id);
                }
                if previous.as_ref() != Some(&node) {
                    self.pending_removed.remove(&node.semantic_ref);
                    self.pending_upserts.insert(node.semantic_ref, node.clone());
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
    }

    fn rebuild_accessibility(&mut self, dom: &DomArena) {
        let previous = self.accessibility.keys().copied().collect::<BTreeSet<_>>();
        let current = self.full.keys().copied().collect::<Vec<_>>();
        for id in current {
            self.refresh_accessibility(dom, id);
        }
        for id in previous {
            if !self.full.contains_key(&id)
                && let Some(node) = self.accessibility.remove(&id)
            {
                self.pending_accessibility_upserts
                    .remove(&node.semantic_ref);
                self.pending_accessibility_removed.insert(node.semantic_ref);
            }
        }
        self.rebuild_accessibility_ownership(dom);
    }

    fn refresh_accessibility(&mut self, dom: &DomArena, id: NodeId) {
        let Some(semantic) = self.full.get(&id) else {
            if let Some(previous) = self.accessibility.remove(&id) {
                self.pending_accessibility_upserts
                    .remove(&previous.semantic_ref);
                self.pending_accessibility_removed
                    .insert(previous.semantic_ref);
            }
            return;
        };
        let Ok(node) = dom.get(id) else { return };
        let NodeKind::Element { name } = &node.kind else {
            return;
        };
        let role = computed_role_for_node(dom, id);
        let named = accessible_semantics(
            dom,
            id,
            name.local.as_ref(),
            &role,
            self.page_id,
            self.document_generation,
        );
        let mut diagnostics = Vec::new();
        let accessible = AccessibilityNode {
            semantic_ref: semantic.semantic_ref,
            excluded: tree_exclusion_reason(dom, id, &role).is_some(),
            exclusion_reason: tree_exclusion_reason(dom, id, &role),
            role,
            name: named.name,
            description: named.description,
            relationships: accessibility_relationships(
                dom,
                id,
                node,
                self.page_id,
                self.document_generation,
                &mut diagnostics,
            ),
            properties: accessibility_properties(dom, id, name.local.as_ref(), node),
            owned_by: None,
            content_trust: ContentTrust::PageAuthored,
            provenance: named.provenance,
            evidence: named.evidence,
        };
        self.diagnostics.extend(diagnostics);
        if self.accessibility.get(&id) != Some(&accessible) {
            self.pending_accessibility_removed
                .remove(&accessible.semantic_ref);
            self.pending_accessibility_upserts
                .insert(accessible.semantic_ref, accessible.clone());
            self.accessibility.insert(id, accessible);
        }
    }

    fn rebuild_accessibility_ownership(&mut self, dom: &DomArena) {
        let previous = self.accessibility.clone();
        self.diagnostics.clear();
        let available = self
            .accessibility
            .values()
            .map(|node| node.semantic_ref)
            .collect::<BTreeSet<_>>();
        for node in self.accessibility.values_mut() {
            node.owned_by = None;
            for targets in node.relationships.values_mut() {
                targets.retain(|target| available.contains(target));
            }
            node.relationships.retain(|_, targets| !targets.is_empty());
        }
        let accepted_owners = accepted_aria_owners(dom);
        let mut owner_by_target = BTreeMap::new();
        let ownership = self
            .accessibility
            .values()
            .filter_map(|node| {
                node.relationships
                    .get(&AccessibilityRelation::Owns)
                    .cloned()
                    .map(|targets| (node.semantic_ref, targets))
            })
            .collect::<Vec<_>>();
        for (owner, targets) in ownership {
            let owner_id = unpack_node_ref(owner.node);
            let mut accepted = Vec::new();
            for target in targets {
                let target_id = unpack_node_ref(target.node);
                let cycle = ownership_cycle(&owner_by_target, owner, target)
                    || dom
                        .ancestors(owner_id)
                        .is_ok_and(|ancestors| ancestors.contains(&target_id));
                if cycle
                    || owner_by_target.contains_key(&target)
                    || accepted_owners.get(&target_id) != Some(&owner_id)
                {
                    self.diagnostics.push(format!(
                        "ignored invalid aria-owns edge {} -> {}",
                        owner.node, target.node
                    ));
                    continue;
                }
                owner_by_target.insert(target, owner);
                accepted.push(target);
            }
            if let Some(node) = self.accessibility.get_mut(&owner_id) {
                node.relationships
                    .insert(AccessibilityRelation::Owns, accepted);
            }
        }
        for (target, owner) in owner_by_target {
            if let Some(node) = self.accessibility.get_mut(&unpack_node_ref(target.node)) {
                node.owned_by = Some(owner);
            }
        }
        for (id, node) in &self.accessibility {
            if previous.get(id) != Some(node) {
                self.pending_accessibility_upserts
                    .insert(node.semantic_ref, node.clone());
            }
        }
    }

    fn rebuild_structures(&mut self, dom: &DomArena) {
        let refs_by_dom_id = self
            .full
            .iter()
            .map(|(id, node)| (*id, node.semantic_ref))
            .collect::<BTreeMap<_, _>>();
        let accessibility = self.accessibility.values().cloned().collect::<Vec<_>>();
        let next = project_structures(dom, &accessibility, &refs_by_dom_id)
            .into_iter()
            .map(|structure| (structure.structure_id.clone(), structure))
            .collect::<BTreeMap<_, _>>();
        for id in self.structures.keys() {
            if !next.contains_key(id) {
                self.pending_structure_upserts.remove(id);
                self.pending_structure_removed.insert(id.clone());
            }
        }
        for (id, structure) in &next {
            if self.structures.get(id) != Some(structure) {
                self.pending_structure_removed.remove(id);
                self.pending_structure_upserts
                    .insert(id.clone(), structure.clone());
            }
        }
        self.structures = next;
    }

    fn remove_subtree(&mut self, root: NodeId) {
        let root_ref = (u64::from(root.generation) << 32) | u64::from(root.slot);
        let mut removed = BTreeSet::from([root_ref]);
        loop {
            let before = removed.len();
            for node in self.full.values() {
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
        self.full.retain(|id, node| {
            let keep = !removed.contains(&node.semantic_ref.node);
            if !keep {
                self.target_hints.remove(id);
                self.compact.remove(id);
                self.pending_upserts.remove(&node.semantic_ref);
                self.pending_removed.insert(node.semantic_ref);
            }
            keep
        });
        // Removal must retire both projections in the same publication. The
        // removed root need not be in the dirty set (only its former parent
        // is), so refreshing dirty accessibility nodes cannot do this later.
        self.accessibility.retain(|_, node| {
            let keep = !removed.contains(&node.semantic_ref.node);
            if !keep {
                self.pending_accessibility_upserts
                    .remove(&node.semantic_ref);
                self.pending_accessibility_removed.insert(node.semantic_ref);
            }
            keep
        });
    }

    fn rebuild_dependencies(&mut self, dom: &DomArena) {
        self.referenced_by.clear();
        self.accessibility_state_revisions = dom
            .iter()
            .filter_map(|(id, node)| {
                let revision = node.accessibility_reference_revision();
                (revision != 0).then_some((id, revision))
            })
            .collect();
        for (source, source_node) in dom.iter() {
            if matches!(&source_node.kind,NodeKind::Element{name}if name.local.as_ref()=="th")
                && let Some(parent) = source_node.parent
            {
                self.referenced_by.entry(parent).or_default().insert(source);
            }
            if matches!(&source_node.kind,NodeKind::Element{name}if name.local.as_ref()=="img")
                && let Some(parent)=source_node.parent.filter(|parent|dom.get(*parent).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.local.as_ref()=="figure"))) {self.referenced_by.entry(parent).or_default().insert(source);}
            for attribute in [
                "aria-labelledby",
                "aria-describedby",
                "aria-owns",
                "aria-errormessage",
                "aria-actions",
            ] {
                if let Some(targets) = dom.accessibility_relation(source, attribute) {
                    for target in targets {
                        self.referenced_by.entry(target).or_default().insert(source);
                    }
                }
            }
            if let Some(target) = dom.label_control(source).ok().flatten() {
                self.referenced_by.entry(target).or_default().insert(source);
                self.referenced_by.entry(source).or_default().insert(target);
            }
        }
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
    project_element_with_visibility(dom, id, page_id, document_generation, alias, false)
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
    let node = dom.get(id).ok()?;
    let NodeKind::Element { name } = &node.kind else {
        return None;
    };
    let ancestors = dom.ancestors(id).ok()?;
    let mut owner = id;
    loop {
        let document = dom.ancestors(owner).ok()?.into_iter().find(|ancestor| {
            dom.get(*ancestor)
                .is_ok_and(|node| matches!(node.kind, NodeKind::Document))
        })?;
        if dom.document_is_retired(document) {
            return None;
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
        return None;
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
        return None;
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
        return None;
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
    let role = computed_role_for_node(dom, id);
    let mut accessible =
        accessible_semantics(dom, id, local_name, &role, page_id, document_generation);
    if accessible.name.is_empty() && matches!(role.as_str(), "status" | "alert" | "log") {
        let text = normalize_text(&accessible_text_content(dom, id, false));
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
        .get_null("type")
        .map(|kind| kind.to_ascii_lowercase())
        .unwrap_or_else(|| "text".into());
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
    let mut actions = BTreeSet::new();
    let mut states = BTreeSet::new();
    if is_hidden(dom, id) {
        states.insert(NodeState::Hidden);
    } else {
        states.insert(NodeState::Visible);
    }
    for (present, state) in [
        (
            node.attributes.contains_null("disabled")
                || aria_true(&node.accessibility_attributes(), "aria-disabled"),
            NodeState::Disabled,
        ),
        (
            node.attributes.contains_null("required")
                || aria_true(&node.accessibility_attributes(), "aria-required"),
            NodeState::Required,
        ),
        (
            (native_readonly && node.attributes.contains_null("readonly"))
                || accessibility_readonly(dom, id),
            NodeState::Readonly,
        ),
        (
            node.attributes.contains_null("data-oryn-focused"),
            NodeState::Focused,
        ),
        (
            node.checked() || aria_true(&node.accessibility_attributes(), "aria-checked"),
            NodeState::Checked,
        ),
        (
            node.attributes.contains_null("selected")
                || aria_true(&node.accessibility_attributes(), "aria-selected"),
            NodeState::Selected,
        ),
        (
            aria_true(&node.accessibility_attributes(), "aria-expanded")
                || (local_name == "details" && node.attributes.contains_null("open")),
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
            .get_null("tabindex")
            .is_some_and(|value| value.trim().parse::<i32>().is_ok())
            || node
                .attributes
                .get_null("contenteditable")
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
        if local_name == "label"
            && descendants(dom, id).into_iter().any(|input| {
                dom.get(input).is_ok_and(|input| {
                matches!(&input.kind, NodeKind::Element { name } if name.local.as_ref() == "input")
                    && input.attributes.get_null("type").is_some_and(|kind| {
                        kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
                    })
            })
            })
        {
            actions.extend([SemanticAction::Click, SemanticAction::Focus]);
        }
        if local_name == "form"
            || matches!(
                node.attributes.get_null("type").map(String::as_str),
                Some("submit")
            )
        {
            actions.insert(SemanticAction::Submit);
        }
        if node.attributes.contains_null("onclick") || node.event_listeners.contains("click") {
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
    let description = accessible.description;
    Some(SemanticNodeView {
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
        selector: node
            .attributes
            .get_null("id")
            .map(|id| format!("#{id}"))
            .or_else(|| {
                node.attributes
                    .get_null("data-testid")
                    .map(|value| format!(r#"[data-testid="{value}"]"#))
            }),
        value: if node.sensitive_value
            || (local_name == "input"
                && node
                    .attributes
                    .get_null("type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("password")))
        {
            (node.value.is_some() || node.sensitive_value).then(|| "[REDACTED]".into())
        } else {
            node.value
                .clone()
                .or_else(|| node.attributes.get_null("value").cloned())
        },
        description,
        states,
        actions,
        content_trust: ContentTrust::PageAuthored,
        // This projection currently has node-level provenance. Accessible-name
        // fallback and visibility both use deliberately bounded heuristics, so
        // the whole node must be conservatively labelled until field-level
        // provenance is introduced.
        provenance: accessible.provenance,
    })
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
    let Ok(node) = dom.get(id) else {
        return AccessibleSemantics {
            name: String::new(),
            description: None,
            provenance: Provenance::Heuristic { confidence: 0.0 },
            evidence: Vec::new(),
        };
    };
    let mut evidence = Vec::new();
    let mut provenance = Provenance::BrowserDeterministic;
    let name = if let Some(targets) = dom
        .accessibility_relation(id, "aria-labelledby")
        .filter(|targets| !targets.is_empty())
    {
        let (value, references) =
            referenced_nodes_text(dom, &targets, page_id, document_generation);
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::ReferencedContent,
            source: "aria-labelledby".into(),
            references,
        });
        // A valid reference wins even when its text alternative is empty.
        Some(value)
    } else {
        None
    };
    let name = if let Some(name) = name {
        name
    } else if let Some(label) = node
        .accessibility_attribute("aria-label")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::ExplicitAria,
            source: "aria-label".into(),
            references: Vec::new(),
        });
        label
    } else if let Some((label, references)) = host_label(dom, id) {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::HostLanguage,
            source: "label".into(),
            references: references
                .into_iter()
                .map(|reference| semantic_ref_for(reference, page_id, document_generation))
                .collect(),
        });
        label
    } else if let Some((value, source)) = host_language_name(dom, id, local_name, role) {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::HostLanguage,
            source,
            references: Vec::new(),
        });
        value
    } else if let Some(title) = node
        .attributes
        .get_null("title")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::HostLanguage,
            source: "title".into(),
            references: Vec::new(),
        });
        title
    } else if let Some(placeholder) = node
        .attributes
        .get_null("placeholder")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        provenance = Provenance::Heuristic { confidence: 0.65 };
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::PlaceholderFallback,
            source: "placeholder".into(),
            references: Vec::new(),
        });
        placeholder
    } else {
        String::new()
    };

    let description = if let Some(targets) = dom
        .accessibility_relation(id, "aria-describedby")
        .filter(|targets| !targets.is_empty())
    {
        let (value, references) =
            referenced_nodes_text(dom, &targets, page_id, document_generation);
        if value.is_empty() {
            None
        } else {
            evidence.push(AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::ReferencedContent,
                source: "aria-describedby".into(),
                references,
            });
            Some(value)
        }
    } else if let Some(value) = node
        .accessibility_attribute("aria-description")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::ExplicitAria,
            source: "aria-description".into(),
            references: Vec::new(),
        });
        Some(value)
    } else if let Some(value) = svg_child_alternative(dom, id, "desc") {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::HostLanguage,
            source: "svg-desc".into(),
            references: Vec::new(),
        });
        Some(value)
    } else {
        // Tooltip text is a description only when it did not supply the name.
        // Equal strings from independent naming sources still qualify.
        let title_used_for_name = evidence.iter().any(|item| {
            item.kind == AccessibilityEvidenceKind::HostLanguage && item.source == "title"
        });
        let title = node
            .attributes
            .get_null("title")
            .map(|value| normalize_text(value))
            .filter(|value| !value.is_empty() && !title_used_for_name);
        if title.is_some() {
            evidence.push(AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::HostLanguage,
                source: "title-description".into(),
                references: Vec::new(),
            });
        }
        title
    };

    AccessibleSemantics {
        name,
        description,
        provenance,
        evidence,
    }
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

fn host_label(dom: &DomArena, id: NodeId) -> Option<(String, Vec<NodeId>)> {
    if !dom.is_labelable(id) {
        return None;
    }
    let root = tree_scope(dom, id)?;
    let labels = dom
        .descendants_inclusive(root)
        .ok()?
        .into_iter()
        .filter(|candidate| {
            tree_scope(dom, *candidate) == Some(root)
                && dom.label_control(*candidate).ok().flatten() == Some(id)
        })
        .map(|candidate| {
            let text = normalize_text(&accessible_text_content_excluding(
                dom,
                candidate,
                true,
                Some(id),
            ));
            (candidate, text)
        })
        .collect::<Vec<_>>();
    labels.first()?;
    Some((
        labels
            .iter()
            .map(|(_, text)| text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        labels.iter().map(|(node, _)| *node).collect(),
    ))
}

/// SVG title/desc alternatives are direct children in the SVG namespace;
/// descendant titles and HTML lookalikes cannot name the enclosing graphic.
fn svg_child_alternative(dom: &DomArena, id: NodeId, local: &str) -> Option<String> {
    let node = dom.get(id).ok()?;
    if !matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/2000/svg")
    {
        return None;
    }
    let child=node.children.iter().find(|child|dom.get(**child).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()==local)))?;
    let text = normalize_text(&dom.text_content(*child).ok()?);
    (!text.is_empty()).then_some(text)
}

fn host_language_name(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
) -> Option<(String, String)> {
    let node = dom.get(id).ok()?;
    if let Some(title) = svg_child_alternative(dom, id, "title") {
        return Some((title, "svg-title".into()));
    }
    let html = matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml");
    let attribute = match if html { local_name } else { "" } {
        "img" | "area" => Some(("alt", "alt")),
        "input"
            if node
                .attributes
                .get_null("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("image")) =>
        {
            Some(("alt", "input-alt"))
        }
        "input"
            if node.attributes.get_null("type").is_some_and(|kind| {
                matches!(
                    kind.to_ascii_lowercase().as_str(),
                    "button" | "submit" | "reset"
                )
            }) =>
        {
            Some(("value", "input-value"))
        }
        _ => None,
    };
    if let Some((attribute, source)) = attribute
        && let Some(value) = (if attribute == "value" {
            node.value
                .as_ref()
                .or_else(|| node.attributes.get_null(attribute))
        } else {
            node.attributes.get_null(attribute)
        })
    {
        // An explicitly empty host-language alternative is authoritative and
        // must suppress lower-priority title/placeholder fallbacks.
        return Some((normalize_text(value), source.into()));
    }
    if html
        && local_name == "input"
        && node
            .attributes
            .get_null("type")
            .is_some_and(|kind| matches!(kind.to_ascii_lowercase().as_str(), "submit" | "reset"))
    {
        return Some((
            if node
                .attributes
                .get_null("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"))
            {
                "Reset"
            } else {
                "Submit"
            }
            .into(),
            "input-default-value".into(),
        ));
    }
    if html
        && local_name == "img"
        && node
            .attributes
            .get_null("title")
            .is_none_or(|title| normalize_text(title).is_empty())
        && let Some(parent)=node.parent.and_then(|parent|dom.get(parent).ok()).filter(|parent|matches!(&parent.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="figure")) {
            let mut caption=None;let mut eligible=true;
            for child in &parent.children {if *child==id{continue;}let Ok(child_node)=dom.get(*child)else{eligible=false;break;};match &child_node.kind{
                NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="figcaption"=>{if caption.replace(*child).is_some(){eligible=false;}},
                NodeKind::Text{data}|NodeKind::CDataSection{data} if data.chars().all(|ch|matches!(ch,' '|'\t'|'\r'|'\n'|'\u{c}'))=>{},
                NodeKind::Comment{..}=>{},_=>eligible=false,
            }}
            if eligible&& let Some(caption)=caption {return Some((normalize_text(&accessible_text_content(dom,caption,false)),"figcaption-image".into()));}
        }
    let named_child = match if html { local_name } else { "" } {
        "fieldset" => Some(("legend", "legend")),
        "table" => Some(("caption", "caption")),
        _ => None,
    };
    if let Some((child_name, source)) = named_child
        && let Some(child) = node.children.iter().find(|child| {
            dom.get(**child).is_ok_and(|node| {
                matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref() == child_name)
            })
        })
    {
        let text = normalize_text(&accessible_text_content(dom, *child, false));
        if !text.is_empty() {
            return Some((text, source.into()));
        }
    }
    // Name-from-heading selects the first exposed heading in depth-first
    // flat-tree order. Dialogs and articles share this algorithm; looking only
    // at immediate children would skip headings inside native layout wrappers.
    if matches!(role, "article" | "dialog" | "alertdialog") {
        let mut pending = accessible_children(dom, id);
        pending.reverse();
        let mut seen = BTreeSet::new();
        while let Some(candidate) = pending.pop() {
            if !seen.insert(candidate) || subtree_excluded_at(dom, candidate) {
                continue;
            }
            if !accessibility_hidden_at(dom, candidate)
                && computed_role_for_node(dom, candidate) == "heading"
            {
                let text = normalize_text(&accessible_text_content(dom, candidate, false));
                if !text.is_empty() {
                    return Some((text, "heading".into()));
                }
                break;
            }
            pending.extend(accessible_children(dom, candidate).into_iter().rev());
        }
    }
    if name_from_content(local_name, role)
        || node.attributes.contains_null("onclick")
        || node.event_listeners.contains("click")
    {
        let text = normalize_text(&accessible_text_content(dom, id, true));
        if !text.is_empty() {
            return Some((text, "text-content".into()));
        }
    }
    None
}

/// Agent-critical Accessible Name text alternative traversal. It preserves
/// referenced hidden roots but excludes aria-hidden descendants from ordinary
/// name-from-content, and includes host-language alternatives such as image
/// `alt` text that raw DOM textContent omits.
fn accessible_text_content(dom: &DomArena, id: NodeId, include_hidden_root: bool) -> String {
    accessible_text_content_excluding(dom, id, include_hidden_root, None)
}

fn accessible_children(dom: &DomArena, id: NodeId) -> Vec<NodeId> {
    let Ok(node) = dom.get(id) else {
        return Vec::new();
    };
    if let Some(root) = node.children.iter().find_map(|child| {
        dom.get(*child)
            .ok()
            .filter(|child| matches!(child.kind, NodeKind::ShadowRoot { .. }))
    }) {
        return root.children.clone();
    }
    if matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="slot")
        && let Some(assigned) = &node.native_resolved_slot_nodes
    {
        return if assigned.is_empty() {
            node.children.clone()
        } else {
            assigned.clone()
        };
    }
    if matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
        && let Some(root_id) = tree_scope(dom, id)
        && let Ok(root) = dom.get(root_id)
        && matches!(root.kind, NodeKind::ShadowRoot { .. })
        && let Some(host) = root.parent.and_then(|host| dom.get(host).ok())
    {
        let slot_name = node
            .attributes
            .get_null("name")
            .map(String::as_str)
            .unwrap_or("");
        // Only the first slot with a given name receives matching light DOM.
        let first_slot = descendants(dom, root_id).into_iter().find(|candidate| {
            tree_scope(dom, *candidate) == Some(root_id)
                && dom.get(*candidate).is_ok_and(|candidate| {
                    matches!(&candidate.kind, NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
                        && candidate.attributes.get_null("name").map(String::as_str).unwrap_or("") == slot_name
                })
        });
        if first_slot == Some(id) {
            let assigned = host
                .children
                .iter()
                .copied()
                .filter(|child| {
                    dom.get(*child).is_ok_and(|child| {
                        matches!(child.kind, NodeKind::Element { .. } | NodeKind::Text { .. })
                            && child
                                .attributes
                                .get_null("slot")
                                .map(String::as_str)
                                .unwrap_or("")
                                == slot_name
                    })
                })
                .collect::<Vec<_>>();
            if !assigned.is_empty() {
                return assigned;
            }
        }
    }
    node.children.clone()
}

/// Walk the accessible option ownership tree, preserving all selected values.
/// Nested widgets and selected options terminate descent; cycle protection also
/// bounds malformed ownership graphs without duplicating an owned option.
fn embedded_selected_options(dom: &DomArena, root: NodeId) -> Vec<NodeId> {
    let root_is_combobox = computed_role_for_node(dom, root) == "combobox";
    let owners = accepted_aria_owners(dom);
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Ok(node) = dom.get(id) else {
            continue;
        };
        if id != root
            && let NodeKind::Element { .. } = &node.kind
        {
            let role = computed_role_for_node(dom, id);
            if role == "option" {
                if node
                    .accessibility_attribute("aria-selected")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                {
                    selected.push(id);
                }
                continue;
            }
            if role == "combobox" || (role == "listbox" && !root_is_combobox) {
                continue;
            }
        }
        let children = owned_accessible_children(dom, id, &owners);
        pending.extend(children.into_iter().rev());
    }
    selected
}

fn accessible_text_content_excluding(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    excluded: Option<NodeId>,
) -> String {
    let include_hidden_subtree = include_hidden_root && referenced_root_hidden(dom, id);
    accessible_text_content_walk(
        dom,
        id,
        include_hidden_root,
        excluded,
        include_hidden_subtree,
        &mut NameTraversal::default(),
    )
}

#[derive(Default)]
struct NameTraversal {
    seen: BTreeSet<NodeId>,
    reference_depth: usize,
    // Naming holds an immutable DOM borrow and invokes no author code. Reuse
    // ownership for this walk only; a later mutation starts a fresh traversal.
    owners: Option<BTreeMap<NodeId, NodeId>>,
}
impl NameTraversal {
    fn owner(&mut self, dom: &DomArena, target: NodeId) -> Option<NodeId> {
        self.owners
            .get_or_insert_with(|| accepted_aria_owners(dom))
            .get(&target)
            .copied()
    }
}

fn accessible_text_content_walk(
    dom: &DomArena,
    id: NodeId,
    include_hidden_root: bool,
    excluded: Option<NodeId>,
    include_hidden_subtree: bool,
    traversal: &mut NameTraversal,
) -> String {
    if Some(id) == excluded {
        // A labelled control contributes no value to its own label, while its
        // replaced-element text boundary still separates adjacent pseudo runs.
        return if dom.get(id).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"input"|"select"|"textarea"))){" ".into()}else{String::new()};
    }
    let Ok(node) = dom.get(id) else {
        return String::new();
    };
    // Raw DOM textContent intentionally includes source text. Agent-facing
    // names must not treat executable code, styles or inert template contents
    // as rendered label text, including when traversing a shadow tree.
    if matches!(&node.kind, NodeKind::Element { name }
        if (name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            && matches!(name.local.as_ref(), "script" | "style" | "template"))
        || (name.ns.as_ref() == "http://www.w3.org/2000/svg"
            && matches!(name.local.as_ref(), "script" | "style")))
    {
        return String::new();
    }
    if !include_hidden_root && !include_hidden_subtree && accessibility_hidden_at(dom, id) {
        // `visibility` is inherited but can be overridden by a descendant.
        // Other accessibility exclusions suppress the complete subtree.
        if visibility_hidden_at(dom, id) {
            return accessible_children(dom, id)
                .iter()
                .map(|child| visible_text_below_hidden_visibility(dom, *child, excluded, traversal))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
        return String::new();
    }
    if !traversal.seen.insert(id) {
        return String::new();
    }
    // Recursive name-from-content traversal uses a descendant's explicit
    // label before its children, including descendants inside shadow trees.
    // Referenced roots already use this rule in referenced_nodes_text; applying it
    // here also covers labels reached through their containing element.
    let is_shadow_slot = matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
        && tree_scope(dom, id).is_some_and(|root| {
            dom.get(root)
                .is_ok_and(|root| matches!(root.kind, NodeKind::ShadowRoot { .. }))
        });
    // Embedded text/range/select controls contribute their current value to
    // the containing label, rather than the label of that embedded control.
    let is_embedded_value = matches!(&node.kind, NodeKind::Element { name }
        if matches!(computed_role_for_node(dom, id).as_str(),
            "textbox" | "searchbox" | "combobox" | "listbox" | "menu" | "slider" | "spinbutton"));
    if !is_shadow_slot
        && !is_embedded_value
        && traversal.reference_depth == 0
        && let Some(targets) = dom
            .accessibility_relation(id, "aria-labelledby")
            .filter(|targets| !targets.is_empty())
    {
        traversal.reference_depth += 1;
        let mut parts = Vec::new();
        let mut self_reference = false;
        for target in targets {
            if target == id && !self_reference {
                traversal.seen.remove(&id);
                self_reference = true;
            }
            let text = accessible_text_content_walk(
                dom,
                target,
                true,
                excluded,
                referenced_root_hidden(dom, target),
                traversal,
            );
            if !text.is_empty() {
                parts.push(text);
            }
        }
        traversal.reference_depth -= 1;
        return normalize_text(&parts.join(" "));
    }
    if !is_shadow_slot
        && !is_embedded_value
        && let Some(label) = node
            .accessibility_attribute("aria-label")
            .map(|label| normalize_text(label))
            .filter(|label| !label.is_empty())
    {
        return label;
    }
    if let Some(title) = svg_child_alternative(dom, id, "title") {
        return title;
    }
    match &node.kind {
        NodeKind::Text { data } => accessible_case_text(dom, id, data),
        NodeKind::Element { .. } if computed_role_for_node(dom, id) == "menu" => String::new(),
        NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && name.local.as_ref() == "br" =>
        {
            " ".into()
        }
        NodeKind::Element { name } if name.local.as_ref() == "img" => {
            node.attributes.get_null("alt").cloned().unwrap_or_default()
        }
        NodeKind::Element { name }
            if name.local.as_ref() == "input"
                && node.attributes.get_null("type").is_some_and(|kind| {
                    matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "button" | "submit" | "reset"
                    )
                }) =>
        {
            node.value
                .clone()
                .or_else(|| node.attributes.get_null("value").cloned())
                .unwrap_or_else(|| {
                    if node
                        .attributes
                        .get_null("type")
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"))
                    {
                        "Reset".into()
                    } else if node
                        .attributes
                        .get_null("type")
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("submit"))
                    {
                        "Submit".into()
                    } else {
                        String::new()
                    }
                })
        }
        NodeKind::Element { name } if name.local.as_ref() == "input" => {
            let role = computed_role_for_node(dom, id);
            if matches!(role.as_str(), "slider" | "spinbutton") {
                node.accessibility_attribute("aria-valuetext")
                    .or_else(|| node.accessibility_attribute("aria-valuenow"))
                    .cloned()
                    .or_else(|| node.value.clone())
                    .or_else(|| node.attributes.get_null("value").cloned())
                    .unwrap_or_default()
            } else {
                node.value
                    .clone()
                    .or_else(|| node.attributes.get_null("value").cloned())
                    .unwrap_or_default()
            }
        }
        NodeKind::Element { name } if name.local.as_ref() == "textarea" => {
            if node.dirty_value {
                node.value.clone().unwrap_or_default()
            } else {
                dom.text_content(id).unwrap_or_default()
            }
        }
        NodeKind::Element { name } if name.local.as_ref() == "select" => {
            let options = dom.select_options(id).unwrap_or_default();
            let mut selected = options
                .iter()
                .copied()
                .filter(|option| dom.get(*option).is_ok_and(|option| option.selected()))
                .collect::<Vec<_>>();
            // Parsed-only snapshots can precede native selectedness initialization.
            // Never invent a first selection once native state has been supplied.
            if selected.is_empty()
                && !node.attributes.contains_null("multiple")
                && node
                    .attributes
                    .get_null("size")
                    .and_then(|size| size.parse::<u32>().ok())
                    .unwrap_or(0)
                    <= 1
                && options.iter().all(|option| {
                    dom.get(*option)
                        .is_ok_and(|option| option.selectedness.is_none())
                })
                && let Some(first) = options.iter().find(|option| {
                    dom.get(**option).is_ok_and(|option| !option.attributes.contains_null("disabled"))
                        && !dom.ancestors(**option).unwrap_or_default().into_iter().take_while(|ancestor| *ancestor != id).any(|ancestor| {
                            dom.get(ancestor).is_ok_and(|node| matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "optgroup") && node.attributes.contains_null("disabled"))
                        })
                }) { selected.push(*first); }
            selected
                .into_iter()
                .map(|option| {
                    dom.get(option)
                        .ok()
                        .and_then(|node| {
                            node.accessibility_attribute("aria-label")
                                .map(|label| normalize_text(label))
                                .filter(|label| !label.is_empty())
                                .or_else(|| {
                                    node.attributes
                                        .get_null("label")
                                        .map(|label| normalize_text(label))
                                        .filter(|label| !label.is_empty())
                                })
                        })
                        .unwrap_or_else(|| {
                            accessible_text_content_walk(
                                dom,
                                option,
                                false,
                                excluded,
                                include_hidden_subtree,
                                traversal,
                            )
                        })
                })
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        NodeKind::Element { name }
            if matches!(
                computed_role_for_node(dom, id).as_str(),
                "listbox" | "combobox"
            ) =>
        {
            let selected = embedded_selected_options(dom, id);
            if !selected.is_empty() {
                selected
                    .into_iter()
                    .map(|option| {
                        accessible_text_content_walk(
                            dom,
                            option,
                            false,
                            excluded,
                            include_hidden_subtree,
                            traversal,
                        )
                    })
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else if computed_role_for_node(dom, id) == "combobox" {
                accessible_children(dom, id)
                    .into_iter()
                    .map(|child| {
                        accessible_text_content_walk(
                            dom,
                            child,
                            false,
                            excluded,
                            include_hidden_subtree,
                            traversal,
                        )
                    })
                    .filter(|value| !value.is_empty())
                    .collect::<String>()
            } else {
                String::new()
            }
        }
        NodeKind::Element { name }
            if matches!(
                computed_role_for_node(dom, id).as_str(),
                "slider" | "spinbutton"
            ) =>
        {
            node.accessibility_attribute("aria-valuetext")
                .or_else(|| node.accessibility_attribute("aria-valuenow"))
                .cloned()
                .unwrap_or_default()
        }
        _ => {
            let action_targets = dom
                .accessibility_relation(id, "aria-actions")
                .unwrap_or_default()
                .into_iter()
                .collect::<BTreeSet<_>>();
            let children = accessible_children(dom, id)
                .into_iter()
                .filter(|child| {
                    traversal.owner(dom, *child).is_none() && !action_targets.contains(child)
                })
                .collect::<Vec<_>>();
            let mut value = children
                .iter()
                .map(|child| {
                    let value = accessible_text_content_walk(
                        dom,
                        *child,
                        false,
                        excluded,
                        include_hidden_subtree,
                        traversal,
                    );
                    if !value.is_empty() && accessible_text_run_separator(dom, *child) {
                        format!(" {value} ")
                    } else {
                        value
                    }
                })
                .filter(|value| !value.is_empty())
                .collect::<String>();
            if let Some(owned) = dom.accessibility_relation(id, "aria-owns") {
                for owned_id in owned {
                    if owned_id == id {
                        continue;
                    }
                    if traversal.owner(dom, owned_id) != Some(id) {
                        continue;
                    }
                    let owned_text = accessible_text_content_walk(
                        dom,
                        owned_id,
                        true,
                        excluded,
                        include_hidden_subtree || accessibility_hidden_at(dom, owned_id),
                        traversal,
                    );
                    if !owned_text.is_empty() {
                        if !value.is_empty() {
                            value.push(' ');
                        }
                        value.push_str(&owned_text);
                    }
                }
            }
            if excluded.is_some()
                && normalize_text(&value).is_empty()
                && matches!(&node.kind,NodeKind::Element{name}if name.local.as_ref()=="label")
                && let Some(title) = node
                    .attributes
                    .get_null("title")
                    .map(|value| normalize_text(value))
                    .filter(|value| !value.is_empty())
            {
                value = format!(" {title} ");
            }
            if let Some((before, after)) = dom.accessibility_generated_content(id) {
                // Generated strings retain author whitespace; normalization is
                // performed by the name algorithm, not by a fake DOM text node.
                value = format!("{before}{value}{after}");
            }
            value
        }
    }
}

fn accessible_case_text(dom: &DomArena, id: NodeId, data: &str) -> String {
    let lineage = dom.ancestors(id).unwrap_or_default();
    let transform = lineage
        .iter()
        .find_map(|parent| {
            dom.accessibility_computed_text_style(*parent)
                .map(|(_, transform)| transform.to_owned())
                .or_else(|| {
                    dom.get(*parent).ok().and_then(|node| {
                        inline_style_value(node, "text-transform").map(str::to_owned)
                    })
                })
        })
        .unwrap_or_else(|| "none".into());
    if !transform
        .split_ascii_whitespace()
        .any(|token| matches!(token, "uppercase" | "lowercase" | "capitalize"))
    {
        return data.into();
    }
    let language = lineage
        .iter()
        .find_map(|parent| {
            dom.get(*parent)
                .ok()
                .and_then(|node| node.attributes.get_null("lang").cloned())
        })
        .unwrap_or_default();
    let root = lineage
        .iter()
        .copied()
        .find(|parent| accessible_text_run_separator(dom, *parent))
        .or_else(|| lineage.last().copied())
        .unwrap_or(id);
    let mut pending = vec![(root, false)];
    let mut seen = BTreeSet::new();
    let mut context = String::new();
    let mut offset = 0;
    let mut range = None;
    while let Some((node_id, after)) = pending.pop() {
        if after {
            context.push(' ');
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
            let end = offset + data.chars().count();
            if node_id == id {
                range = Some((offset, end));
            }
            context.push_str(data);
            offset = end;
        } else {
            if node_id != root && accessible_text_run_separator(dom, node_id) {
                context.push(' ');
                offset += 1;
                pending.push((node_id, true));
            }
            pending.extend(
                accessible_children(dom, node_id)
                    .into_iter()
                    .rev()
                    .map(|child| (child, false)),
            );
        }
    }
    let Some((start, end)) = range else {
        return oryn_layout::accessibility_case_fragment(
            data,
            0,
            data.chars().count(),
            &transform,
            &language,
        );
    };
    oryn_layout::accessibility_case_fragment(&context, start, end, &transform, &language)
}

fn accessible_text_run_separator(dom: &DomArena, id: NodeId) -> bool {
    let Ok(node) = dom.get(id) else {
        return false;
    };
    let NodeKind::Element { name } = &node.kind else {
        return false;
    };
    if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
        && matches!(name.local.as_ref(), "br" | "input" | "select" | "textarea")
    {
        return true;
    }
    if let Some((display, _)) = dom.accessibility_computed_text_style(id) {
        return !matches!(display, "inline" | "contents" | "none");
    }
    if let Some(display) = inline_style_value(node, "display") {
        return !matches!(display.trim(), "inline" | "contents" | "none");
    }
    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
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
        )
}

fn visible_text_below_hidden_visibility(
    dom: &DomArena,
    id: NodeId,
    excluded: Option<NodeId>,
    traversal: &mut NameTraversal,
) -> String {
    if Some(id) == excluded {
        return String::new();
    }
    let Ok(node) = dom.get(id) else {
        return String::new();
    };
    if subtree_excluded_at(dom, id) {
        return String::new();
    }
    if dom
        .accessibility_computed_visibility(id)
        .is_some_and(|(_, hidden)| !hidden)
        || (dom.accessibility_computed_visibility(id).is_none()
            && inline_style_value(node, "visibility")
                .is_some_and(|value| value.eq_ignore_ascii_case("visible")))
    {
        return accessible_text_content_walk(
            dom,
            id,
            true,
            excluded,
            referenced_root_hidden(dom, id),
            traversal,
        );
    }
    node.children
        .iter()
        .map(|child| visible_text_below_hidden_visibility(dom, *child, excluded, traversal))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn inline_style_value<'a>(node: &'a crate::dom::Node, property: &str) -> Option<&'a str> {
    node.attributes
        .get_null("style")?
        .split(';')
        .find_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(property)
                .then(|| value.trim())
        })
}

fn subtree_excluded_without_visibility(node: &crate::dom::Node) -> bool {
    node.attributes.contains_null("hidden")
        || node
            .accessibility_attribute("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || inline_style_value(node, "display")
            .is_some_and(|value| value.eq_ignore_ascii_case("none"))
}

fn subtree_excluded_at(dom: &DomArena, id: NodeId) -> bool {
    let Ok(node) = dom.get(id) else {
        return false;
    };
    match dom.accessibility_computed_visibility(id) {
        Some((display_none, _)) => {
            display_none
                || node
                    .accessibility_attribute("aria-hidden")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        }
        None => subtree_excluded_without_visibility(node),
    }
}
fn accessibility_hidden_at(dom: &DomArena, id: NodeId) -> bool {
    match dom.accessibility_computed_visibility(id) {
        Some((_, visibility_hidden)) => subtree_excluded_at(dom, id) || visibility_hidden,
        None => dom.get(id).is_ok_and(accessibility_hidden),
    }
}
fn visibility_hidden_at(dom: &DomArena, id: NodeId) -> bool {
    match dom.accessibility_computed_visibility(id) {
        Some((_, hidden)) => hidden && !subtree_excluded_at(dom, id),
        None => dom.get(id).is_ok_and(visibility_hidden_only),
    }
}

/// A directly referenced node inherits subtree exclusion from ancestors.
/// Visibility differs: a nearer explicit visible value overrides inheritance,
/// while it cannot override display:none, hidden, or aria-hidden ancestry.
fn referenced_root_hidden(dom: &DomArena, id: NodeId) -> bool {
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
    let mut visibility = None;
    for id in lineage {
        let Ok(node) = dom.get(id) else {
            continue;
        };
        if subtree_excluded_at(dom, id) {
            return true;
        }
        if visibility.is_none() {
            if let Some((_, hidden)) = dom.accessibility_computed_visibility(id) {
                visibility = Some(hidden);
                continue;
            }
            if let Some(value) = inline_style_value(node, "visibility") {
                if value.eq_ignore_ascii_case("visible") {
                    visibility = Some(false);
                } else if value.eq_ignore_ascii_case("hidden")
                    || value.eq_ignore_ascii_case("collapse")
                {
                    visibility = Some(true);
                }
            }
        }
    }
    visibility.unwrap_or(false)
}

fn visibility_hidden_only(node: &crate::dom::Node) -> bool {
    !subtree_excluded_without_visibility(node)
        && inline_style_value(node, "visibility").is_some_and(|value| {
            value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
        })
}

// Follow accepted accessibility-parent edges as well as DOM ancestry. A
// cross-reference cycle can involve siblings and is invisible to DOM ancestry.
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
    // DOM order, not arena allocation order, decides competing explicit owners.
    // Use one acceptance algorithm for naming and published accessibility edges.
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    for (root, node) in dom.iter() {
        if !matches!(node.kind, NodeKind::Document) {
            continue;
        }
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            order.push(id);
            if let Ok(node) = dom.get(id) {
                pending.extend(node.children.iter().rev().copied());
            }
        }
    }
    let mut accepted = BTreeMap::new();
    for owner in order {
        let Some(targets) = dom.accessibility_relation(owner, "aria-owns") else {
            continue;
        };
        // Most nodes declare no ownership. Avoid computing CSS visibility for
        // every node just to discover that it has no proposed ownership edge.
        if targets.is_empty() || is_hidden(dom, owner) {
            continue;
        }
        for target in targets {
            if accepted.contains_key(&target)
                || !seen.contains(&target)
                || !aria_owned_target_is_exposable(dom, target)
                || dom
                    .ancestors(owner)
                    .is_ok_and(|ancestors| ancestors.contains(&target))
            {
                continue;
            }
            let mut cursor = Some(owner);
            let mut walked = BTreeSet::new();
            let mut cycle = false;
            while let Some(id) = cursor {
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
    accepted
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
fn accessibility_ancestors(
    dom: &DomArena,
    id: NodeId,
    owners: &BTreeMap<NodeId, NodeId>,
) -> Vec<NodeId> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::from([id]);
    let mut cursor = owners
        .get(&id)
        .copied()
        .or_else(|| dom.get(id).ok().and_then(|node| node.parent));
    while let Some(id) = cursor {
        if !seen.insert(id) {
            break;
        }
        result.push(id);
        cursor = owners
            .get(&id)
            .copied()
            .or_else(|| dom.get(id).ok().and_then(|node| node.parent));
    }
    result
}

fn aria_owner(dom: &DomArena, target: NodeId) -> Option<NodeId> {
    accepted_aria_owners(dom).get(&target).copied()
}

/// ARIA ownership reparents a target in the accessibility tree, so an
/// `aria-hidden` state inherited only from the target's DOM ancestors does not
/// exclude it after reparenting. HTML/CSS tree exclusion still applies through
/// the original ancestry, and `aria-hidden` on the target itself remains
/// authoritative.
fn aria_owned_target_is_exposable(dom: &DomArena, target: NodeId) -> bool {
    let Ok(_target_node) = dom.get(target) else {
        return false;
    };
    if accessibility_hidden_at(dom, target) {
        return false;
    }
    dom.ancestors(target)
        .unwrap_or_default()
        .into_iter()
        .all(|id| {
            dom.get(id).is_ok_and(|node| {
                if let Some((display_none, _)) = dom.accessibility_computed_visibility(id) {
                    return !display_none;
                }
                if node.attributes.contains_null("hidden") {
                    return false;
                }
                !node.attributes.get_null("style").is_some_and(|style| {
                    style.split(';').any(|declaration| {
                        declaration.split_once(':').is_some_and(|(name, value)| {
                            let name = name.trim().to_ascii_lowercase();
                            let value = value.trim().to_ascii_lowercase();
                            (name == "display" && value == "none")
                                || (name == "visibility"
                                    && matches!(value.as_str(), "hidden" | "collapse"))
                        })
                    })
                })
            })
        })
}

fn accessibility_hidden(node: &crate::dom::Node) -> bool {
    if node.attributes.contains_null("hidden")
        || node
            .accessibility_attribute("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    node.attributes.get_null("style").is_some_and(|style| {
        style.split(';').any(|declaration| {
            declaration.split_once(':').is_some_and(|(name, value)| {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim().to_ascii_lowercase();
                (name == "display" && value == "none")
                    || (name == "visibility" && matches!(value.as_str(), "hidden" | "collapse"))
            })
        })
    })
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
    let mut output = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
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
    output
}

fn find_element_by_id_scoped(dom: &DomArena, source: NodeId, value: &str) -> Option<NodeId> {
    let scope = tree_scope(dom, source)?;
    dom.descendants_inclusive(scope)
        .ok()?
        .into_iter()
        .find(|id| {
            tree_scope(dom, *id) == Some(scope)
                && dom.get(*id).is_ok_and(|node| {
                    matches!(node.kind, NodeKind::Element { .. })
                        && node.attributes.get_null("id").is_some_and(|id| id == value)
                })
        })
}

fn tree_scope(dom: &DomArena, id: NodeId) -> Option<NodeId> {
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).ok()?);
    lineage
        .iter()
        .copied()
        .find(|candidate| {
            matches!(
                dom.get(*candidate).ok().map(|node| &node.kind),
                Some(NodeKind::ShadowRoot { .. })
            )
        })
        .or_else(|| lineage.last().copied())
}

fn referenced_nodes_text(
    dom: &DomArena,
    targets: &[NodeId],
    page_id: PageId,
    document_generation: u64,
) -> (String, Vec<SemanticRef>) {
    let mut values = Vec::new();
    let mut references = Vec::new();
    for &id in targets {
        let mut text = dom
            .get(id)
            .ok()
            .and_then(|node| node.accessibility_attribute("aria-label"))
            .map(|label| normalize_text(label))
            .filter(|label| !label.is_empty())
            .or_else(|| host_label(dom, id).map(|(label, _)| label))
            .unwrap_or_else(|| {
                normalize_text(&accessible_text_content_walk(
                    dom,
                    id,
                    true,
                    None,
                    referenced_root_hidden(dom, id),
                    &mut NameTraversal {
                        seen: BTreeSet::new(),
                        reference_depth: 1,
                        ..Default::default()
                    },
                ))
            });
        if text.is_empty() {
            text = dom
                .get(id)
                .ok()
                .and_then(|node| node.attributes.get_null("title"))
                .map(|title| normalize_text(title))
                .unwrap_or_default();
        }
        if dom.get(id).is_ok() {
            references.push(semantic_ref_for(id, page_id, document_generation));
        }
        if !text.is_empty() {
            values.push(text);
        }
    }
    (values.join(" "), references)
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

fn tree_exclusion_reason(dom: &DomArena, id: NodeId, role: &str) -> Option<String> {
    let node = dom.get(id).ok()?;
    if role == "none" || role == "presentation" {
        return Some("presentational_role".into());
    }
    if matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "input")
        && node
            .attributes
            .get_null("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    {
        return Some("hidden_input".into());
    }
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
    if lineage.iter().any(|candidate| {
        dom.get(*candidate)
            .is_ok_and(|node| node.attributes.contains_null("inert"))
    }) {
        return Some("inert_subtree".into());
    }
    if let Some(document) = dom.node_document(id).ok().flatten() {
        let modal = dom
            .descendants_inclusive(document)
            .unwrap_or_default()
            .into_iter()
            .rfind(|candidate| {
                dom.get(*candidate).is_ok_and(|node| {
                    node.accessibility_attribute("aria-modal")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                }) && !is_hidden(dom, *candidate)
                    && matches!(
                        computed_role_for_node(dom, *candidate).as_str(),
                        "dialog" | "alertdialog"
                    )
            });
        if let Some(modal) = modal {
            let owners = accepted_aria_owners(dom);
            if !accessibility_ancestors(dom, id, &owners).contains(&modal)
                && id != modal
                && !accessibility_ancestors(dom, modal, &owners).contains(&id)
            {
                return Some("aria_modal_scope".into());
            }
        }
    }
    is_hidden(dom, id).then_some("hidden_subtree".into())
}

fn accessibility_relationships(
    dom: &DomArena,
    source: NodeId,
    node: &crate::dom::Node,
    page_id: PageId,
    document_generation: u64,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<AccessibilityRelation, Vec<SemanticRef>> {
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
        if node.explicit_accessibility_references(attribute).is_none()
            && let Some(idrefs) = node.attributes.get_null(attribute)
        {
            for idref in idrefs.split_whitespace() {
                if find_element_by_id_scoped(dom, source, idref).is_none() {
                    diagnostics.push(format!("{attribute} references missing id {idref:?}"));
                }
            }
        }
        let Some(nodes) = dom.accessibility_relation(source, attribute) else {
            continue;
        };
        let targets = nodes
            .into_iter()
            .map(|id| semantic_ref_for(id, page_id, document_generation))
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            relationships.insert(relation, targets);
        }
    }
    relationships
}

fn accessibility_properties(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    node: &crate::dom::Node,
) -> BTreeMap<String, serde_json::Value> {
    let effective_attributes = node.accessibility_attributes();
    let attributes = &effective_attributes;
    let mut properties = BTreeMap::new();
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
        if let Some(value) = attributes.get_null(attribute) {
            properties.insert(
                attribute.trim_start_matches("aria-").replace('-', "_"),
                semantic_property_value(value),
            );
        }
    }
    for attribute in ["aria-valuetext", "aria-roledescription"] {
        if let Some(value) = attributes.get_null(attribute) {
            properties.insert(
                attribute.trim_start_matches("aria-").into(),
                serde_json::Value::String(value.clone()),
            );
        }
    }
    for attribute in ["aria-valuemin", "aria-valuemax", "aria-valuenow"] {
        if let Some(value) = attributes
            .get_null(attribute)
            .and_then(|value| oryn_dom::parse_html_floating_point(value))
            .and_then(serde_json::Number::from_f64)
        {
            properties.insert(
                attribute.trim_start_matches("aria-").into(),
                serde_json::Value::Number(value),
            );
        }
    }
    if let Some(value) = attributes
        .get_null("placeholder")
        .or_else(|| attributes.get_null("aria-placeholder"))
    {
        properties.insert("placeholder".into(), serde_json::json!(value));
    }
    if attributes
        .get_null("aria-invalid")
        .is_some_and(|value| !value.is_empty() && !value.eq_ignore_ascii_case("false"))
        && let Some(targets) = dom.accessibility_relation(id, "aria-errormessage")
    {
        let targets = targets
            .into_iter()
            .filter(|target| !is_hidden(dom, *target))
            .collect::<Vec<_>>();
        let (message, _) = referenced_nodes_text(dom, &targets, PageId(1), 1);
        if !message.is_empty() {
            properties.insert("validation_error".into(), serde_json::json!(message));
        }
    }
    let role = computed_role_for_node(dom, id);
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
            properties.insert(key.into(), serde_json::json!(value));
        }
    }
    if accessibility_readonly(dom, id) {
        properties.insert("readonly".into(), serde_json::json!(true));
    }
    if matches!(
        role.as_str(),
        "checkbox" | "menuitemcheckbox" | "radio" | "menuitemradio" | "switch"
    ) {
        let value = attributes
            .get_null("aria-checked")
            .map(|value| value.to_ascii_lowercase());
        let checked = match value.as_deref() {
            Some("true") => serde_json::Value::Bool(true),
            Some("mixed") if matches!(role.as_str(), "checkbox" | "menuitemcheckbox") => {
                serde_json::json!("mixed")
            }
            _ => serde_json::Value::Bool(false),
        };
        properties.insert("checked".into(), checked);
    }
    if role == "button" {
        let pressed = match attributes
            .get_null("aria-pressed")
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("true") => serde_json::Value::Bool(true),
            Some("false") => serde_json::Value::Bool(false),
            Some("mixed") => serde_json::json!("mixed"),
            _ => serde_json::json!("undefined"),
        };
        properties.insert("pressed".into(), pressed);
    }
    if local_name == "input"
        && attributes.get_null("type").is_some_and(|kind| {
            kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
        })
    {
        properties.insert(
            "checked".into(),
            if node.indeterminate
                && attributes
                    .get_null("type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
            {
                serde_json::Value::String("mixed".into())
            } else {
                serde_json::Value::Bool(node.checked())
            },
        );
    }
    for attribute in [
        "disabled", "required", "readonly", "checked", "selected", "open",
    ] {
        let kind = attributes
            .get_null("type")
            .map(|kind| kind.to_ascii_lowercase())
            .unwrap_or_else(|| "text".into());
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
        if applicable && attributes.contains_null(attribute) {
            properties
                .entry(attribute.into())
                .or_insert(serde_json::Value::Bool(true));
        }
    }
    if local_name == "table" {
        properties.insert("native_table".into(), serde_json::Value::Bool(true));
        let span = |node: &crate::dom::Node| {
            node.attributes
                .get_null("span")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|span| *span > 0)
                .unwrap_or(1)
                .min(1000)
        };
        let mut groups = Vec::new();
        let mut start = 0u64;
        for child in &node.children {
            let Ok(group) = dom.get(*child) else {
                continue;
            };
            if !matches!(&group.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="colgroup")
            {
                continue;
            }
            let columns=group.children.iter().filter_map(|child|dom.get(*child).ok()).filter(|column|matches!(&column.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="col")).collect::<Vec<_>>();
            let width = if columns.is_empty() {
                span(group)
            } else {
                columns.into_iter().map(span).sum()
            };
            groups.push(serde_json::json!([start, width]));
            start = start.saturating_add(width);
        }
        if !groups.is_empty() {
            properties.insert(
                "native_column_groups".into(),
                serde_json::Value::Array(groups),
            );
        }
    }
    if matches!(local_name, "td" | "th") {
        properties.insert(
            "native_header".into(),
            serde_json::Value::Bool(local_name == "th"),
        );
        let empty = node.children.iter().all(|child| {
            dom.get(*child).is_ok_and(|child| match &child.kind {
                NodeKind::Element { .. } => false,
                NodeKind::Text { data } | NodeKind::CDataSection { data } => data
                    .chars()
                    .all(|ch| matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ')),
                _ => true,
            })
        });
        properties.insert("native_empty_cell".into(), serde_json::Value::Bool(empty));
        if let Some(headers) = attributes.get_null("headers") {
            let targets=headers.split_ascii_whitespace().filter_map(|token|find_element_by_id_scoped(dom,id,token))
                .filter(|target|*target!=id&&dom.get(*target).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"td"|"th"))))
                .map(|target|(u64::from(target.generation)<<32)|u64::from(target.slot)).collect::<Vec<_>>();
            properties.insert("native_headers".into(), serde_json::json!(targets));
        }
        if local_name == "th"
            && let Some(scope) = attributes
                .get_null("scope")
                .map(|scope| scope.to_ascii_lowercase())
                .filter(|scope| matches!(scope.as_str(), "row" | "col" | "rowgroup" | "colgroup"))
        {
            properties.insert("native_scope".into(), serde_json::json!(scope));
        }
        for (name, maximum, allow_zero) in
            [("rowspan", 65534u64, true), ("colspan", 1000u64, false)]
        {
            let parsed = attributes
                .get_null(name)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(1);
            let value = if parsed == 0 && !allow_zero {
                1
            } else {
                parsed.min(maximum)
            };
            properties.insert(format!("native_{name}"), serde_json::json!(value));
            properties.insert(
                format!("native_{name}_explicit"),
                serde_json::json!(attributes.contains_null(name)),
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
        properties.insert("level".into(), serde_json::json!(level));
    }
    properties
}

fn semantic_property_value(value: &str) -> serde_json::Value {
    match value.to_ascii_lowercase().as_str() {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        _ => value
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::Value::String(value.into())),
    }
}

fn project_structures(
    dom: &DomArena,
    accessibility: &[AccessibilityNode],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
) -> Vec<SemanticStructure> {
    let accessible_by_ref = accessibility
        .iter()
        .map(|node| (node.semantic_ref, node))
        .collect::<BTreeMap<_, _>>();
    let mut structures = Vec::new();
    for root in accessibility.iter().filter(|node| !node.excluded) {
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
        let descendants = descendants(dom, id);
        let members = descendants
            .iter()
            .filter_map(|id| refs_by_dom_id.get(id).copied())
            .filter(|reference| {
                accessible_by_ref
                    .get(reference)
                    .is_some_and(|node| !node.excluded)
            })
            .collect::<Vec<_>>();
        let groups = structure_groups(dom, kind, &descendants, refs_by_dom_id, &accessible_by_ref);
        let mut properties = BTreeMap::new();
        properties.insert("member_count".into(), serde_json::json!(members.len()));
        if kind == SemanticStructureKind::Form {
            if let Some(method) = dom_node.attributes.get_null("method") {
                properties.insert("method".into(), serde_json::json!(method));
            }
            if let Some(action) = dom_node.attributes.get_null("action") {
                properties.insert("action".into(), serde_json::json!(action));
            }
        }
        if kind == SemanticStructureKind::List {
            properties.insert(
                "ordered".into(),
                serde_json::json!(name.local.as_ref() == "ol"),
            );
        }
        if kind == SemanticStructureKind::LiveRegion {
            properties.insert(
                "politeness".into(),
                root.properties
                    .get("live")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(implicit_live_politeness(&root.role))),
            );
        }
        structures.push(SemanticStructure {
            structure_id: format!("{:?}:{}", kind, root.semantic_ref.node).to_ascii_lowercase(),
            kind,
            root: root.semantic_ref,
            parent_structure_id: None,
            name: root.name.clone(),
            members,
            groups,
            properties,
            content_trust: ContentTrust::PageAuthored,
            provenance: Provenance::BrowserDeterministic,
            evidence: vec![AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::StructuralInference,
                source: format!("element:{};role:{}", name.local, root.role),
                references: vec![root.semantic_ref],
            }],
        });
    }
    let roots = structures
        .iter()
        .map(|structure| (structure.root, structure.structure_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for structure in &mut structures {
        let id = unpack_node_ref(structure.root.node);
        structure.parent_structure_id = dom
            .ancestors(id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|ancestor| refs_by_dom_id.get(&ancestor))
            .find_map(|reference| roots.get(reference).cloned());
    }
    structures
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

fn structure_groups(
    dom: &DomArena,
    kind: SemanticStructureKind,
    descendant_ids: &[NodeId],
    refs_by_dom_id: &BTreeMap<NodeId, SemanticRef>,
    accessible_by_ref: &BTreeMap<SemanticRef, &AccessibilityNode>,
) -> Vec<SemanticGroup> {
    let group_role = match kind {
        SemanticStructureKind::Table | SemanticStructureKind::Grid => Some("row"),
        SemanticStructureKind::List => Some("listitem"),
        SemanticStructureKind::Menu => Some("menuitem"),
        SemanticStructureKind::Tree => Some("treeitem"),
        SemanticStructureKind::TabSet => Some("tab"),
        _ => None,
    };
    let Some(group_role) = group_role else {
        return Vec::new();
    };
    descendant_ids
        .iter()
        .filter_map(|id| {
            refs_by_dom_id
                .get(id)
                .copied()
                .map(|reference| (*id, reference))
        })
        .filter(|(_, reference)| {
            accessible_by_ref
                .get(reference)
                .is_some_and(|node| !node.excluded && node.role == group_role)
        })
        .map(|(id, reference)| {
            let mut members = vec![reference];
            if group_role == "row" {
                members.extend(
                    descendants(dom, id)
                        .into_iter()
                        .filter_map(|child| refs_by_dom_id.get(&child).copied())
                        .filter(|child| {
                            accessible_by_ref.get(child).is_some_and(|node| {
                                !node.excluded
                                    && matches!(
                                        node.role.as_str(),
                                        "cell" | "gridcell" | "rowheader" | "columnheader"
                                    )
                            })
                        }),
                );
            }
            SemanticGroup {
                kind: group_role.into(),
                name: accessible_by_ref
                    .get(&reference)
                    .map(|node| node.name.clone())
                    .filter(|name| !name.is_empty()),
                members,
            }
        })
        .collect()
}

fn descendants(dom: &DomArena, root: NodeId) -> Vec<NodeId> {
    let mut pending = dom
        .get(root)
        .map(|node| node.children.clone())
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    while let Some(id) = pending.pop() {
        output.push(id);
        if let Ok(node) = dom.get(id) {
            for child in node.children.iter().rev() {
                pending.push(*child);
            }
        }
    }
    output
}

fn implicit_live_politeness(role: &str) -> &'static str {
    match role {
        "alert" => "assertive",
        "status" | "log" => "polite",
        _ => "off",
    }
}

pub(crate) fn accessibility_readonly(dom: &DomArena, id: NodeId) -> bool {
    let Ok(node) = dom.get(id) else {
        return true;
    };
    if let Some(value) = node.accessibility_attribute("aria-readonly") {
        return value.eq_ignore_ascii_case("true");
    }
    if matches!(
        computed_role_for_node(dom, id).as_str(),
        "gridcell" | "columnheader" | "rowheader"
    ) {
        for parent in accessibility_ancestors(dom, id, &accepted_aria_owners(dom)) {
            if matches!(
                computed_role_for_node(dom, parent).as_str(),
                "grid" | "treegrid"
            ) {
                return dom.get(parent).is_ok_and(|parent| {
                    parent
                        .accessibility_attribute("aria-readonly")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                });
            }
        }
    }
    false
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
    let role = unpromoted_role_for_node(dom, id);
    if matches!(role.as_str(), "generic" | "none")
        && dom.get(id).is_ok_and(|node| {
            ["autofocus", "draggable", "popover"]
                .iter()
                .any(|attribute| node.attributes.contains_null(attribute))
        })
    {
        "group".into()
    } else {
        role
    }
}
fn unpromoted_role_for_node(dom: &DomArena, id: NodeId) -> String {
    let Ok(node) = dom.get(id) else {
        return "generic".into();
    };
    let NodeKind::Element { name } = &node.kind else {
        return "generic".into();
    };
    let local = name.local.as_ref();
    let attributes = node.accessibility_attributes();
    let base = computed_role(local, &attributes);
    let explicit = attributes.get_null("role").and_then(|value| {
        value
            .split_ascii_whitespace()
            .map(str::to_ascii_lowercase)
            .find(|role| supported_aria_role(role))
    });
    let named = || {
        let key = (dom as *const DomArena as usize, id);
        if !ROLE_NAME_STACK.with(|stack| stack.borrow_mut().insert(key)) {
            return false;
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
        !accessible_semantics(dom, id, local, &base, PageId(1), 1)
            .name
            .is_empty()
    };
    if explicit
        .as_deref()
        .is_some_and(|role| !matches!(role, "none" | "presentation" | "form" | "region"))
    {
        return base;
    }
    let base = if matches!(explicit.as_deref(), Some("form" | "region")) {
        if named() {
            return explicit.unwrap();
        }
        if let Some(fallback) = attributes.get_null("role").and_then(|roles| {
            roles
                .split_ascii_whitespace()
                .map(str::to_ascii_lowercase)
                .find(|role| {
                    supported_aria_role(role) && !matches!(role.as_str(), "form" | "region")
                })
        }) {
            return explicit_role_value(local, &attributes, fallback);
        }
        implicit_role(local, &attributes)
    } else {
        base.clone()
    };
    if matches!(explicit.as_deref(), Some("none" | "presentation")) && base == "none" {
        return base;
    }
    if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
        return base;
    }
    // Native required-owned descendants inherit their container's effective
    // presentation. Global ARIA properties on the descendant do not cancel
    // inherited presentation; an explicit role or focusability does.
    if attributes
        .get_null("tabindex")
        .is_none_or(|value| value.trim().parse::<i32>().is_err())
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
        if required_parent && computed_role_for_node(dom, parent) == "none" {
            return "none".into();
        }
    }
    let sectioning=dom.ancestors(id).unwrap_or_default().into_iter().any(|ancestor|dom.get(ancestor).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"article"|"aside"|"nav"|"section"))));
    match local {
        "th" if attributes.get_null("scope").is_none_or(|scope|!matches!(scope.to_ascii_lowercase().as_str(),"row"|"rowgroup"|"col"|"colgroup"))=>{
            if node.parent.and_then(|parent|dom.get(parent).ok()).is_some_and(|row|row.children.iter().any(|child|dom.get(*child).is_ok_and(|child|matches!(&child.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="td")))){"rowheader".into()}else{"columnheader".into()}
        },
        "li"=>{
            let mut cursor=aria_owner(dom,id).or(node.parent);let mut seen=BTreeSet::new();
            while let Some(parent)=cursor {if !seen.insert(parent){break;}let role=computed_role_for_node(dom,parent);if role=="list"{return "listitem".into();}
            if !matches!(role.as_str(),"generic"|"none"){break;}cursor=dom.get(parent).ok().and_then(|parent|parent.parent);}
            "generic".into()
        },
        "aside" if sectioning&&!named()=>"generic".into(),
        "header"|"footer" if sectioning||dom.ancestors(id).unwrap_or_default().into_iter().any(|ancestor|dom.get(ancestor).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="main")))=>if local=="header"{"sectionheader".into()}else{"sectionfooter".into()},
        "section"=>if named(){"region".into()}else{"generic".into()},
        "form"=>if named(){"form".into()}else{"generic".into()},
        "img" if attributes.get_null("alt").is_some_and(|alt|normalize_text(alt).is_empty())=>if (attributes.contains_null("aria-label")||dom.accessibility_relation(id,"aria-labelledby").is_some_and(|targets|!targets.is_empty()))&&named(){"image".into()}else{"none".into()},
        "img" if !["src","srcset"].iter().any(|attribute|attributes.get_null(attribute).is_some_and(|value|!value.is_empty()))&&!named()=>"none".into(),
        "input" if attributes.get_null("type").is_some_and(|kind|kind.eq_ignore_ascii_case("checkbox"))&&attributes.contains_null("switch")=>"switch".into(),
        _=>base,
    }
}

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

fn target_content_text(dom: &DomArena, root: NodeId) -> String {
    let mut pending = accessible_children(dom, root);
    pending.reverse();
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) || is_hidden(dom, id) {
            continue;
        }
        let Ok(node) = dom.get(id) else { continue };
        match &node.kind {
            NodeKind::Text { data } => parts.push(data.clone()),
            NodeKind::Element { name }
                if matches!(
                    name.local.as_ref(),
                    "input" | "textarea" | "select" | "script" | "style" | "template"
                ) => {}
            _ => pending.extend(accessible_children(dom, id).into_iter().rev()),
        }
    }
    normalize_text(&parts.join(" "))
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
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
    if let Some((_, visibility_hidden)) = dom.accessibility_computed_visibility(id) {
        return visibility_hidden || lineage.iter().any(|id| subtree_excluded_at(dom, *id));
    }
    let rules = dom
        .iter()
        .filter_map(|(style_id, node)| match &node.kind {
            NodeKind::Element { name } if name.local.as_ref() == "style" => {
                dom.text_content(style_id).ok()
            }
            _ => None,
        })
        .flat_map(|sheet| {
            strip_css_comments(&sheet)
                .split('}')
                .filter_map(|rule| rule.split_once('{'))
                .flat_map(|(selectors, declarations)| {
                    selectors
                        .split(',')
                        .map(str::trim)
                        .map(|selector| (selector.to_string(), declarations.to_string()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    lineage.iter().any(|candidate| {
        dom.get(*candidate).is_ok_and(|node| {
            if node.attributes.contains_null("hidden")
                || node
                    .accessibility_attribute("aria-hidden")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                return true;
            }
            let mut hidden = None;
            for (selector, declarations) in &rules {
                if matches_selector(dom, *candidate, selector) {
                    update_visibility(&mut hidden, declarations);
                }
            }
            if let Some(style) = node.attributes.get_null("style") {
                update_visibility(&mut hidden, style);
            }
            hidden.unwrap_or(false)
        })
    })
}

fn strip_css_comments(sheet: &str) -> String {
    let mut output = String::with_capacity(sheet.len());
    let mut remainder = sheet;
    while let Some(start) = remainder.find("/*") {
        output.push_str(&remainder[..start]);
        let Some(end) = remainder[start + 2..].find("*/") else {
            return output;
        };
        remainder = &remainder[start + 2 + end + 2..];
    }
    output.push_str(remainder);
    output
}

fn matches_selector(dom: &DomArena, id: NodeId, selector: &str) -> bool {
    let segments = selector
        .split_whitespace()
        .filter(|segment| *segment != ">")
        .collect::<Vec<_>>();
    let Some(subject) = segments.last() else {
        return false;
    };
    let Ok(node) = dom.get(id) else {
        return false;
    };
    if !matches_simple_selector(node, subject) {
        return false;
    }
    let ancestors = dom.ancestors(id).unwrap_or_default();
    let mut ancestor_index = 0;
    for segment in segments[..segments.len() - 1].iter().rev() {
        let Some(found) = ancestors[ancestor_index..].iter().position(|ancestor| {
            dom.get(*ancestor)
                .is_ok_and(|node| matches_simple_selector(node, segment))
        }) else {
            return false;
        };
        ancestor_index += found + 1;
    }
    true
}

fn update_visibility(hidden: &mut Option<bool>, declarations: &str) {
    for declaration in declarations.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        match property.trim().to_ascii_lowercase().as_str() {
            "display" => *hidden = Some(value.trim().eq_ignore_ascii_case("none")),
            "visibility" => {
                *hidden = Some(matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "hidden" | "collapse"
                ))
            }
            _ => {}
        }
    }
}

fn matches_simple_selector(node: &crate::dom::Node, selector: &str) -> bool {
    let NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let selector = selector.split(':').next().unwrap_or_default();
    let attribute_start = selector.find('[');
    let simple = &selector[..attribute_start.unwrap_or(selector.len())];
    if let Some(attribute) = attribute_start.and_then(|index| {
        selector
            .strip_suffix(']')
            .map(|selector| &selector[index + 1..])
    }) {
        let (key, expected) = attribute
            .split_once('=')
            .map(|(key, value)| (key, Some(value.trim_matches(['"', '\'']))))
            .unwrap_or((attribute, None));
        let Some(actual) = node.attributes.get_null(key) else {
            return false;
        };
        if expected.is_some_and(|expected| actual != expected) {
            return false;
        }
    }
    let mut tag_end = simple.len();
    for marker in ['#', '.'] {
        if let Some(index) = simple.find(marker) {
            tag_end = tag_end.min(index);
        }
    }
    let tag = &simple[..tag_end];
    if !tag.is_empty() && !name.local.as_ref().eq_ignore_ascii_case(tag) {
        return false;
    }
    if let Some(start) = simple.find('#') {
        let id = simple[start + 1..].split('.').next().unwrap_or_default();
        if node
            .attributes
            .get_null("id")
            .is_none_or(|value| value != id)
        {
            return false;
        }
    }
    simple.split('.').skip(1).all(|class| {
        node.attributes
            .get_null("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class))
    })
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

    use super::*;

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
