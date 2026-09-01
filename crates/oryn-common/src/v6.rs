//! Incremental semantic-state contract.
//!
//! Contracts v2-v5 remain frozen. This contract exposes the persistent graph
//! used by native observations, resolution, actions, and revision deltas.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use crate::v3::{
    ContentTrust, ExecutionDomain, NodeState, ObservationSource, PageId, PageInfo,
    ProjectionProfile, Provenance, Revision, SemanticAction, SemanticRef,
};
pub use crate::v4::{NodeGeometry, ViewportState};
pub use crate::v5::{
    AccessibilityEvidence, AccessibilityNode, AccessibilityRelation, SemanticStructure,
};

pub const CONTRACT_VERSION: u16 = 6;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryPolicy {
    #[default]
    Omit,
    IncludeWhenAvailable,
    Require,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationScope {
    pub root: SemanticRef,
    #[serde(default = "default_ancestors")]
    pub ancestor_depth: u16,
    #[serde(default = "default_descendants")]
    pub descendant_depth: u16,
    #[serde(default = "default_relationships")]
    pub relationship_depth: u16,
    #[serde(default)]
    pub include_structural_neighbors: bool,
    #[serde(default)]
    pub include_spatial_neighbors: bool,
    #[serde(default = "default_scope_nodes")]
    pub max_nodes: u32,
}

const fn default_ancestors() -> u16 {
    2
}
const fn default_descendants() -> u16 {
    3
}
const fn default_relationships() -> u16 {
    2
}
const fn default_scope_nodes() -> u32 {
    512
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticObservationOptions {
    #[serde(default = "default_profile")]
    pub profile: ProjectionProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ObservationScope>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub geometry: GeometryPolicy,
    #[serde(default = "default_true")]
    pub include_structures: bool,
}

impl Default for SemanticObservationOptions {
    fn default() -> Self {
        Self {
            profile: ProjectionProfile::Compact,
            scope: None,
            include_hidden: false,
            geometry: GeometryPolicy::Omit,
            include_structures: true,
        }
    }
}

const fn default_profile() -> ProjectionProfile {
    ProjectionProfile::Compact
}
const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticField {
    Role,
    Name,
    Description,
    Value,
    State,
    Action,
    Relationship,
    Visibility,
    Geometry,
    Structure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticFieldEvidence {
    pub field: SemanticField,
    pub trust: ContentTrust,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<AccessibilityEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNode {
    #[serde(flatten)]
    pub semantic: crate::v3::SemanticNodeView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relationships: BTreeMap<AccessibilityRelation, Vec<SemanticRef>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accessibility_properties: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<NodeGeometry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_evidence: Vec<SemanticFieldEvidence>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSummary {
    pub graph_nodes: u32,
    pub included_nodes: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub omitted_by_reason: BTreeMap<String, u32>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticObservation {
    pub contract_version: u16,
    pub page: PageInfo,
    pub revision: Revision,
    pub semantic_revision: u64,
    pub profile: ProjectionProfile,
    pub source: ObservationSource,
    pub nodes: Vec<SemanticNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accessibility: Vec<AccessibilityNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structures: Vec<SemanticStructure>,
    pub projection: ProjectionSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<crate::v2::CapabilityDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    pub execution_domain: ExecutionDomain,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SemanticGraphChanges {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserted_nodes: Vec<SemanticNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_nodes: Vec<SemanticRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserted_accessibility: Vec<AccessibilityNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_accessibility: Vec<SemanticRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserted_structures: Vec<SemanticStructure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_structures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticDelta {
    pub contract_version: u16,
    pub page_id: PageId,
    pub from_revision: Revision,
    pub to_revision: Revision,
    pub semantic_revision: u64,
    pub changes: SemanticGraphChanges,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionCandidate {
    pub semantic_ref: SemanticRef,
    pub score: i32,
    pub actionable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionResult {
    pub revision: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<SemanticRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ResolutionCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl SemanticNode {
    pub fn actions(&self) -> &BTreeSet<SemanticAction> {
        &self.semantic.actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_bounded_and_backward_safe() {
        let options = SemanticObservationOptions::default();
        assert_eq!(options.profile, ProjectionProfile::Compact);
        assert_eq!(options.geometry, GeometryPolicy::Omit);
        assert!(options.include_structures);
    }

    #[test]
    fn delta_changes_are_a_named_additive_payload() {
        let delta = SemanticDelta {
            contract_version: CONTRACT_VERSION,
            page_id: PageId(1),
            from_revision: Revision(2),
            to_revision: Revision(3),
            semantic_revision: 4,
            changes: SemanticGraphChanges::default(),
            diagnostics: Vec::new(),
        };
        let encoded = serde_json::to_value(delta).expect("serialize semantic delta");
        assert!(encoded.get("changes").is_some());
        assert!(encoded.get("upserted_nodes").is_none());
    }
}
