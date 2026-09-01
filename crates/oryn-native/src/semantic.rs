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
    referenced_by: BTreeMap<NodeId, BTreeSet<NodeId>>,
    accessibility: BTreeMap<NodeId, AccessibilityNode>,
    structures: BTreeMap<String, SemanticStructure>,
    frame_ids: BTreeMap<NodeId, u64>,
    authored_hidden: BTreeSet<NodeId>,
    geometry_hidden: BTreeSet<NodeId>,
    semantic_revision: u64,
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
            referenced_by: BTreeMap::new(),
            accessibility: BTreeMap::new(),
            structures: BTreeMap::new(),
            frame_ids: BTreeMap::new(),
            authored_hidden: BTreeSet::new(),
            geometry_hidden: BTreeSet::new(),
            semantic_revision: 0,
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
        use crate::dom::Mutation;
        let mut dirty = BTreeSet::new();
        let mut dependency_change = false;
        for mutation in mutations {
            match mutation {
                Mutation::Inserted { node, parent } => {
                    dirty.insert(*node);
                    dirty.extend(parent.iter().copied());
                    dependency_change = true;
                }
                Mutation::Removed {
                    node,
                    former_parent,
                } => {
                    self.remove_subtree(*node);
                    dirty.extend(former_parent.iter().copied());
                    dependency_change = true;
                }
                Mutation::Attribute { node, name, .. } => {
                    dirty.insert(*node);
                    dependency_change |= matches!(
                        name.as_str(),
                        "id" | "for"
                            | "aria-label"
                            | "aria-labelledby"
                            | "aria-describedby"
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
                Mutation::FormState { node } | Mutation::EventListener { node, .. } => {
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
        let initial = dirty.iter().copied().collect::<Vec<_>>();
        for node in initial {
            if let Ok(ancestors) = dom.ancestors(node) {
                dirty.extend(ancestors);
            }
            if let Some(dependents) = self.referenced_by.get(&node) {
                dirty.extend(dependents);
            }
        }
        if dependency_change {
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
            let Some(value) = geometry.get(&node.semantic_ref.node) else {
                continue;
            };
            if value.availability == GeometryAvailability::Hidden {
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
            self.pending_upserts.insert(node.semantic_ref, node.clone());
            if let Some(accessible) = self.accessibility.get_mut(&id) {
                accessible.excluded = hidden;
                accessible.exclusion_reason = hidden.then(|| {
                    if self.geometry_hidden.contains(&id) {
                        "computed_style_hidden".into()
                    } else {
                        "hidden_subtree".into()
                    }
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
        use oryn_common::agent::Target;
        if let Target::Reference { reference } = target {
            let id = unpack_node_ref(reference.node);
            let node = self
                .full
                .get(&id)
                .ok_or(SemanticResolutionError::NotFound)?;
            if node.semantic_ref != *reference || !node.actions.contains(&action) {
                return Err(SemanticResolutionError::NotFound);
            }
            return Ok(oryn_common::v6::ResolutionResult {
                revision,
                selected: Some(*reference),
                candidates: vec![oryn_common::v6::ResolutionCandidate {
                    semantic_ref: *reference,
                    score: 1_000,
                    actionable: true,
                    factors: vec!["exact_reference".into()],
                }],
                diagnostics: Vec::new(),
            });
        }
        let mut candidates = self
            .full
            .values()
            .filter(|node| node.actions.contains(&action))
            .filter_map(|node| {
                self.score_target(node, target, geometry)
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
                .cmp(&left.1)
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
                    score,
                    actionable: true,
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
    ) -> Result<Option<(i32, Vec<String>)>, SemanticResolutionError> {
        use oryn_common::agent::Target;
        let scored = match target {
            Target::Reference { reference } => {
                (node.semantic_ref == *reference).then(|| (1_000, vec!["exact_reference".into()]))
            }
            Target::Alias { alias } => {
                (node.alias == *alias).then(|| (900, vec!["exact_alias".into()]))
            }
            Target::Text { text } => {
                let expected = normalize_text(text).to_ascii_lowercase();
                let actual = normalize_text(&node.name).to_ascii_lowercase();
                if actual == expected {
                    Some((800, vec!["exact_accessible_name".into()]))
                } else if !expected.is_empty() && actual.contains(&expected) {
                    Some((600, vec!["contained_accessible_name".into()]))
                } else {
                    None
                }
            }
            Target::Role { role } => node
                .role
                .eq_ignore_ascii_case(role)
                .then(|| (700, vec!["exact_role".into()])),
            Target::Css { selector } => (node.selector.as_deref() == Some(selector.as_str()))
                .then(|| (850, vec!["exact_css_selector".into()])),
            Target::XPath { expression } => {
                return Err(SemanticResolutionError::UnsupportedSelector(format!(
                    "XPath resolution is not part of the semantic graph: {expression}"
                )));
            }
            Target::Inside { target, container } => {
                let Some((score, mut factors)) = self.score_target(node, target, geometry)? else {
                    return Ok(None);
                };
                let containers = self.matching_refs(container, geometry)?;
                let inside = containers
                    .iter()
                    .any(|container| self.is_descendant(node.semantic_ref, *container));
                inside.then(|| {
                    factors.push("inside".into());
                    (score + 100, factors)
                })
            }
            Target::Contains { target, content } => {
                let Some((score, mut factors)) = self.score_target(node, target, geometry)? else {
                    return Ok(None);
                };
                let content = self.matching_refs(content, geometry)?;
                content
                    .iter()
                    .any(|content| self.is_descendant(*content, node.semantic_ref))
                    .then(|| {
                        factors.push("contains".into());
                        (score + 100, factors)
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
                let Some((score, mut factors)) = self.score_target(node, subject, geometry)? else {
                    return Ok(None);
                };
                let anchors = self.matching_refs(anchor, geometry)?;
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
                        (score + 80, factors)
                    })
            }
            Target::Near { target, anchor } => {
                let Some((score, mut factors)) = self.score_target(node, target, geometry)? else {
                    return Ok(None);
                };
                let geometry = geometry.ok_or(SemanticResolutionError::GeometryUnavailable)?;
                let candidate = geometry
                    .get(&node.semantic_ref.node)
                    .and_then(|value| value.box_model.as_ref())
                    .ok_or(SemanticResolutionError::GeometryUnavailable)?
                    .border_box;
                let anchors = self.matching_refs(anchor, Some(geometry))?;
                let distance = anchors
                    .iter()
                    .filter_map(|anchor| {
                        geometry
                            .get(&anchor.node)
                            .and_then(|value| value.box_model.as_ref())
                    })
                    .map(|model| {
                        let dx = candidate.x - model.border_box.x;
                        let dy = candidate.y - model.border_box.y;
                        (dx * dx + dy * dy).sqrt()
                    })
                    .min_by(f64::total_cmp)
                    .ok_or(SemanticResolutionError::GeometryUnavailable)?;
                factors.push(format!("distance:{distance:.3}"));
                Some((score + 100_000 - distance.min(99_999.0) as i32, factors))
            }
        };
        Ok(scored)
    }

    fn matching_refs(
        &self,
        target: &oryn_common::agent::Target,
        geometry: Option<&BTreeMap<u64, oryn_common::v4::NodeGeometry>>,
    ) -> Result<Vec<SemanticRef>, SemanticResolutionError> {
        self.full
            .values()
            .filter_map(|node| {
                self.score_target(node, target, geometry)
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
        let role = computed_role(name.local.as_ref(), &node.attributes);
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
            properties: accessibility_properties(name.local.as_ref(), &node.attributes),
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
                let cycle = target == owner
                    || dom
                        .ancestors(owner_id)
                        .is_ok_and(|ancestors| ancestors.contains(&target_id));
                if cycle || owner_by_target.contains_key(&target) {
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
                self.compact.remove(id);
                self.pending_upserts.remove(&node.semantic_ref);
                self.pending_removed.insert(node.semantic_ref);
            }
            keep
        });
    }

    fn rebuild_dependencies(&mut self, dom: &DomArena) {
        self.referenced_by.clear();
        let ids = dom
            .iter()
            .filter_map(|(id, node)| node.attributes.get("id").map(|value| (value.clone(), id)))
            .collect::<BTreeMap<_, _>>();
        for (source, node) in dom.iter() {
            for attribute in ["aria-labelledby", "aria-describedby"] {
                if let Some(value) = node.attributes.get(attribute) {
                    for token in value.split_ascii_whitespace() {
                        if let Some(target) = ids.get(token) {
                            self.referenced_by
                                .entry(*target)
                                .or_default()
                                .insert(source);
                        }
                    }
                }
            }
            if matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "label")
                && let Some(target) = node.attributes.get("for").and_then(|value| ids.get(value))
            {
                self.referenced_by
                    .entry(*target)
                    .or_default()
                    .insert(source);
                self.referenced_by
                    .entry(source)
                    .or_default()
                    .insert(*target);
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
    let node = dom.get(id).ok()?;
    let NodeKind::Element { name } = &node.kind else {
        return None;
    };
    let ancestors = dom.ancestors(id).ok()?;
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
                mode: crate::dom::ShadowMode::Closed
            })
        )
    }) {
        return None;
    }
    let local_name = name.local.as_ref();
    let parent = ancestors.into_iter().find_map(|ancestor| {
        matches!(dom.get(ancestor).ok()?.kind, NodeKind::Element { .. }).then_some(SemanticRef {
            page_id,
            document_generation,
            node: (u64::from(ancestor.generation) << 32) | u64::from(ancestor.slot),
        })
    });
    let role = computed_role(local_name, &node.attributes);
    let mut accessible =
        accessible_semantics(dom, id, local_name, &role, page_id, document_generation);
    if accessible.name.is_empty() && matches!(role.as_str(), "status" | "alert" | "log") {
        let text = normalize_text(&dom.text_content(id).unwrap_or_default());
        if !text.is_empty() {
            // Compatibility observations historically surface live-region
            // content as the node's agent-facing name. Contract v5 keeps the
            // accessibility name and live content distinct.
            accessible.name = text;
            accessible.provenance = Provenance::Heuristic { confidence: 0.8 };
        }
    }
    let accessible_name = accessible.name;
    let mut actions = BTreeSet::new();
    let mut states = BTreeSet::new();
    if is_hidden(dom, id) {
        states.insert(NodeState::Hidden);
    } else {
        states.insert(NodeState::Visible);
    }
    for (present, state) in [
        (
            node.attributes.contains_key("disabled")
                || aria_true(&node.attributes, "aria-disabled"),
            NodeState::Disabled,
        ),
        (
            node.attributes.contains_key("required")
                || aria_true(&node.attributes, "aria-required"),
            NodeState::Required,
        ),
        (
            node.attributes.contains_key("readonly")
                || aria_true(&node.attributes, "aria-readonly"),
            NodeState::Readonly,
        ),
        (
            node.attributes.contains_key("data-oryn-focused"),
            NodeState::Focused,
        ),
        (
            node.attributes.contains_key("checked") || aria_true(&node.attributes, "aria-checked"),
            NodeState::Checked,
        ),
        (
            node.attributes.contains_key("selected")
                || aria_true(&node.attributes, "aria-selected"),
            NodeState::Selected,
        ),
        (
            aria_true(&node.attributes, "aria-expanded")
                || (local_name == "details" && node.attributes.contains_key("open")),
            NodeState::Expanded,
        ),
    ] {
        if present {
            states.insert(state);
        }
    }
    if !states.contains(&NodeState::Disabled) && !states.contains(&NodeState::Hidden) {
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
            match node.attributes.get("type").map(String::as_str) {
                Some("checkbox" | "radio") => actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                ]),
                Some("button" | "submit" | "reset") => {
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
                    && input.attributes.get("type").is_some_and(|kind| {
                        kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
                    })
            })
            })
        {
            actions.extend([SemanticAction::Click, SemanticAction::Focus]);
        }
        if local_name == "form"
            || matches!(
                node.attributes.get("type").map(String::as_str),
                Some("submit")
            )
        {
            actions.insert(SemanticAction::Submit);
        }
        if node.attributes.contains_key("onclick") || node.event_listeners.contains("click") {
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
            "combobox" | "listbox" => {
                actions.extend([
                    SemanticAction::Click,
                    SemanticAction::Select,
                    SemanticAction::Focus,
                ]);
            }
            "slider" | "spinbutton" => {
                actions.extend([SemanticAction::Click, SemanticAction::Focus]);
            }
            _ => {}
        }
    }
    let description = accessible.description;
    Some(SemanticNodeView {
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
            .get("id")
            .map(|id| format!("#{id}"))
            .or_else(|| {
                node.attributes
                    .get("data-testid")
                    .map(|value| format!(r#"[data-testid="{value}"]"#))
            }),
        value: if node.sensitive_value
            || (local_name == "input"
                && node
                    .attributes
                    .get("type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("password")))
        {
            (node.value.is_some() || node.sensitive_value).then(|| "[REDACTED]".into())
        } else {
            node.value
                .clone()
                .or_else(|| node.attributes.get("value").cloned())
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

fn aria_true(attributes: &BTreeMap<String, String>, name: &str) -> bool {
    attributes
        .get(name)
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
    let name = if let Some(labelled_by) = node.attributes.get("aria-labelledby") {
        let (value, references) =
            referenced_text(dom, id, labelled_by, page_id, document_generation);
        if !value.is_empty() {
            evidence.push(AccessibilityEvidence {
                kind: AccessibilityEvidenceKind::ReferencedContent,
                source: "aria-labelledby".into(),
                references,
            });
            value
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let name = if !name.is_empty() {
        name
    } else if let Some(label) = node
        .attributes
        .get("aria-label")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::ExplicitAria,
            source: "aria-label".into(),
            references: Vec::new(),
        });
        label
    } else if let Some((label, reference)) = host_label(dom, id) {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::HostLanguage,
            source: "label".into(),
            references: vec![semantic_ref_for(reference, page_id, document_generation)],
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
        .get("title")
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
        .get("placeholder")
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

    let description = if let Some(described_by) = node.attributes.get("aria-describedby") {
        let (value, references) =
            referenced_text(dom, id, described_by, page_id, document_generation);
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
        .attributes
        .get("aria-description")
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
    {
        evidence.push(AccessibilityEvidence {
            kind: AccessibilityEvidenceKind::ExplicitAria,
            source: "aria-description".into(),
            references: Vec::new(),
        });
        Some(value)
    } else {
        node.attributes
            .get("title")
            .map(|value| normalize_text(value))
            .filter(|value| !value.is_empty() && value != &name)
            .or_else(|| {
                node.attributes
                    .get("placeholder")
                    .map(|value| normalize_text(value))
                    .filter(|value| !value.is_empty() && value != &name)
            })
    };

    AccessibleSemantics {
        name,
        description,
        provenance,
        evidence,
    }
}

fn host_label(dom: &DomArena, id: NodeId) -> Option<(String, NodeId)> {
    let node = dom.get(id).ok()?;
    if let Some(control_id) = node.attributes.get("id")
        && let Some((label_id, text)) = dom.iter().find_map(|(candidate, label)| {
            matches!(&label.kind, NodeKind::Element { name } if name.local.as_ref() == "label")
                .then_some(())
                .filter(|_| label.attributes.get("for") == Some(control_id))
                .filter(|_| tree_scope(dom, candidate) == tree_scope(dom, id))
                .and_then(|_| {
                    dom.text_content(candidate)
                        .ok()
                        .map(|text| (candidate, text))
                })
        })
    {
        let text = normalize_text(&text);
        if !text.is_empty() {
            return Some((text, label_id));
        }
    }
    dom.ancestors(id).ok()?.into_iter().find_map(|ancestor| {
        let ancestor_node = dom.get(ancestor).ok()?;
        matches!(&ancestor_node.kind, NodeKind::Element { name } if name.local.as_ref() == "label")
            .then(|| normalize_text(&dom.text_content(ancestor).unwrap_or_default()))
            .filter(|text| !text.is_empty())
            .map(|text| (text, ancestor))
    })
}

fn host_language_name(
    dom: &DomArena,
    id: NodeId,
    local_name: &str,
    role: &str,
) -> Option<(String, String)> {
    let node = dom.get(id).ok()?;
    let attribute = match local_name {
        "img" | "area" => Some(("alt", "alt")),
        "input"
            if node
                .attributes
                .get("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("image")) =>
        {
            Some(("alt", "input-alt"))
        }
        "input"
            if node.attributes.get("type").is_some_and(|kind| {
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
        && let Some(value) = node
            .attributes
            .get(attribute)
            .map(|value| normalize_text(value))
            .filter(|value| !value.is_empty())
    {
        return Some((value, source.into()));
    }
    if local_name == "input"
        && node
            .attributes
            .get("type")
            .is_some_and(|kind| matches!(kind.to_ascii_lowercase().as_str(), "submit" | "reset"))
    {
        return Some((
            if node
                .attributes
                .get("type")
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
    let named_child = match local_name {
        "fieldset" => Some(("legend", "legend")),
        "table" => Some(("caption", "caption")),
        "figure" => Some(("figcaption", "figcaption")),
        _ => None,
    };
    if let Some((child_name, source)) = named_child
        && let Some(child) = node.children.iter().find(|child| {
            dom.get(**child).is_ok_and(|node| {
                matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == child_name)
            })
        })
    {
        let text = normalize_text(&dom.text_content(*child).unwrap_or_default());
        if !text.is_empty() {
            return Some((text, source.into()));
        }
    }
    if name_from_content(local_name, role)
        || node.attributes.contains_key("onclick")
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
    let Ok(node) = dom.get(id) else {
        return String::new();
    };
    if !include_hidden_root
        && node
            .attributes
            .get("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return String::new();
    }
    match &node.kind {
        NodeKind::Text { data } => data.clone(),
        NodeKind::Element { name } if name.local.as_ref() == "img" => {
            node.attributes.get("alt").cloned().unwrap_or_default()
        }
        NodeKind::Element { name }
            if name.local.as_ref() == "input"
                && node.attributes.get("type").is_some_and(|kind| {
                    matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "button" | "submit" | "reset"
                    )
                }) =>
        {
            node.attributes.get("value").cloned().unwrap_or_else(|| {
                if node
                    .attributes
                    .get("type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"))
                {
                    "Reset".into()
                } else {
                    "Submit".into()
                }
            })
        }
        _ => node
            .children
            .iter()
            .map(|child| accessible_text_content(dom, *child, false))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
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
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_element_by_id_scoped(dom: &DomArena, source: NodeId, value: &str) -> Option<NodeId> {
    let scope = tree_scope(dom, source);
    dom.iter()
        .find(|(id, node)| {
            node.attributes.get("id").is_some_and(|id| id == value) && tree_scope(dom, *id) == scope
        })
        .map(|(id, _)| id)
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

fn referenced_text(
    dom: &DomArena,
    source: NodeId,
    idrefs: &str,
    page_id: PageId,
    document_generation: u64,
) -> (String, Vec<SemanticRef>) {
    let mut values = Vec::new();
    let mut references = Vec::new();
    for idref in idrefs.split_whitespace() {
        let Some(id) = find_element_by_id_scoped(dom, source, idref) else {
            continue;
        };
        let text = dom
            .get(id)
            .ok()
            .and_then(|node| node.attributes.get("aria-label"))
            .map_or_else(
                || normalize_text(&accessible_text_content(dom, id, true)),
                |label| normalize_text(label),
            );
        if !text.is_empty() {
            values.push(text);
            references.push(semantic_ref_for(id, page_id, document_generation));
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
        let role = computed_role(name.local.as_ref(), &node.attributes);
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
            properties: accessibility_properties(name.local.as_ref(), &node.attributes),
            owned_by: None,
            content_trust: ContentTrust::PageAuthored,
            provenance: named.provenance,
            evidence: named.evidence,
        });
    }

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
            let creates_cycle = target == owner
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
            .get("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    {
        return Some("hidden_input".into());
    }
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
    if lineage.iter().any(|candidate| {
        dom.get(*candidate)
            .is_ok_and(|node| node.attributes.contains_key("inert"))
    }) {
        return Some("inert_subtree".into());
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
        let Some(idrefs) = node.attributes.get(attribute) else {
            continue;
        };
        let mut targets = Vec::new();
        for idref in idrefs.split_whitespace() {
            if let Some(id) = find_element_by_id_scoped(dom, source, idref) {
                targets.push(semantic_ref_for(id, page_id, document_generation));
            } else {
                diagnostics.push(format!("{attribute} references missing id {idref:?}"));
            }
        }
        if !targets.is_empty() {
            relationships.insert(relation, targets);
        }
    }
    relationships
}

fn accessibility_properties(
    local_name: &str,
    attributes: &BTreeMap<String, String>,
) -> BTreeMap<String, serde_json::Value> {
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
        "aria-rowindex",
        "aria-colindex",
        "aria-rowspan",
        "aria-colspan",
        "aria-live",
        "aria-atomic",
        "aria-relevant",
    ] {
        if let Some(value) = attributes.get(attribute) {
            properties.insert(
                attribute.trim_start_matches("aria-").replace('-', "_"),
                semantic_property_value(value),
            );
        }
    }
    for attribute in [
        "disabled", "required", "readonly", "checked", "selected", "open",
    ] {
        if attributes.contains_key(attribute) {
            properties
                .entry(attribute.into())
                .or_insert(serde_json::Value::Bool(true));
        }
    }
    if matches!(local_name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
        && !properties.contains_key("level")
    {
        properties.insert(
            "level".into(),
            serde_json::json!(local_name[1..].parse::<u8>().unwrap_or(0)),
        );
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
            if let Some(method) = dom_node.attributes.get("method") {
                properties.insert("method".into(), serde_json::json!(method));
            }
            if let Some(action) = dom_node.attributes.get("action") {
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
    attributes: &BTreeMap<String, String>,
) -> Option<SemanticStructureKind> {
    if attributes.contains_key("aria-live")
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
        _ if local_name == "dl" || attributes.contains_key("data-oryn-collection") => {
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

fn computed_role(
    local_name: &str,
    attributes: &std::collections::BTreeMap<String, String>,
) -> String {
    if let Some(role) = attributes.get("role").and_then(|roles| {
        roles
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .find(|role| supported_aria_role(role))
    }) {
        let presentation_conflict = matches!(role.as_str(), "none" | "presentation")
            && (matches!(local_name, "button" | "input" | "select" | "textarea")
                || (local_name == "a" && attributes.contains_key("href"))
                || attributes.contains_key("tabindex"));
        if !presentation_conflict {
            return role;
        }
    }
    match local_name {
        "a" if attributes.contains_key("href") => "link",
        "input" => match attributes.get("type").map(String::as_str) {
            Some("checkbox") => "checkbox",
            Some("radio") => "radio",
            Some("range") => "slider",
            Some("number") => "spinbutton",
            Some("search") => "searchbox",
            Some("button" | "submit" | "reset") => "button",
            _ => "textbox",
        },
        "textarea" => "textbox",
        "select"
            if attributes.contains_key("multiple")
                || attributes
                    .get("size")
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|size| size > 1) =>
        {
            "listbox"
        }
        "select" => "combobox",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "output" => "status",
        "img" if attributes.get("alt").is_some_and(String::is_empty) => "presentation",
        "img" => "img",
        "nav" => "navigation",
        "main" => "main",
        "aside" => "complementary",
        "header" => "header",
        "footer" => "footer",
        "form"
            if attributes.contains_key("aria-label")
                || attributes.contains_key("aria-labelledby") =>
        {
            "form"
        }
        "form" => "generic",
        "ul" | "ol" | "menu" => "list",
        "li" => "listitem",
        "table" => "table",
        "tr" => "row",
        "th" => match attributes.get("scope").map(String::as_str) {
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
        "section"
            if attributes.contains_key("aria-label")
                || attributes.contains_key("aria-labelledby") =>
        {
            "region"
        }
        other => other,
    }
    .into()
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
            | "cell"
            | "checkbox"
            | "columnheader"
            | "combobox"
            | "complementary"
            | "contentinfo"
            | "definition"
            | "dialog"
            | "directory"
            | "document"
            | "feed"
            | "figure"
            | "form"
            | "generic"
            | "grid"
            | "gridcell"
            | "group"
            | "heading"
            | "img"
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
            | "separator"
            | "slider"
            | "spinbutton"
            | "status"
            | "switch"
            | "tab"
            | "table"
            | "tablist"
            | "tabpanel"
            | "term"
            | "textbox"
            | "timer"
            | "toolbar"
            | "tooltip"
            | "tree"
            | "treegrid"
            | "treeitem"
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
    let mut lineage = vec![id];
    lineage.extend(dom.ancestors(id).unwrap_or_default());
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
            if node.attributes.contains_key("hidden")
                || node
                    .attributes
                    .get("aria-hidden")
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
            if let Some(style) = node.attributes.get("style") {
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
        let Some(actual) = node.attributes.get(key) else {
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
        if node.attributes.get("id").is_none_or(|value| value != id) {
            return false;
        }
    }
    simple.split('.').skip(1).all(|class| {
        node.attributes
            .get("class")
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
    use super::*;

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
