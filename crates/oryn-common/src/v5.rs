//! Structured semantic and accessibility observation contract.
//!
//! Contracts v2-v4 remain frozen. This additive contract carries the existing
//! page/node observation together with first-class structures and explicit
//! accessibility evidence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::v3::{ContentTrust, Observation, Provenance, SemanticRef};

pub const CONTRACT_VERSION: u16 = 5;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityRelation {
    LabelledBy,
    DescribedBy,
    Controls,
    Owns,
    ActiveDescendant,
    Details,
    ErrorMessage,
    FlowTo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityEvidenceKind {
    ExplicitAria,
    HostLanguage,
    ReferencedContent,
    TextContent,
    PlaceholderFallback,
    StructuralInference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityEvidence {
    pub kind: AccessibilityEvidenceKind,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<SemanticRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityNode {
    pub semantic_ref: SemanticRef,
    pub excluded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relationships: BTreeMap<AccessibilityRelation, Vec<SemanticRef>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<SemanticRef>,
    pub content_trust: ContentTrust,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<AccessibilityEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticStructureKind {
    Form,
    Table,
    List,
    Navigation,
    Dialog,
    Menu,
    Tree,
    TabSet,
    Grid,
    Landmark,
    LiveRegion,
    RepeatedCollection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticGroup {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub members: Vec<SemanticRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticStructure {
    pub structure_id: String,
    pub kind: SemanticStructureKind,
    pub root: SemanticRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_structure_id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<SemanticRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SemanticGroup>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
    pub content_trust: ContentTrust,
    pub provenance: Provenance,
    pub evidence: Vec<AccessibilityEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticObservation {
    pub contract_version: u16,
    pub observation: Observation,
    pub accessibility: Vec<AccessibilityNode>,
    pub structures: Vec<SemanticStructure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}
