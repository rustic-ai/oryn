//! Versioned agent/browser contracts shared by native and compatibility domains.
//!
//! The compatibility scanner can be projected into these types during the
//! migration. Native Oryn produces them directly from browser-owned state.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::protocol::{Element, ScanResult};

pub const CONTRACT_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SemanticRef {
    pub document_generation: u64,
    pub node: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionDomain {
    Native,
    Chromium,
    Webkit,
    UserBrowser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    NativeDom,
    CompatibilityScanner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionProfile {
    Compact,
    Full,
    Scoped,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAction {
    Click,
    Type,
    Clear,
    Check,
    Uncheck,
    Select,
    Submit,
    Focus,
    Hover,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Visible,
    Hidden,
    Disabled,
    Focused,
    Primary,
    Checked,
    Selected,
    Required,
    Readonly,
    Expanded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    BrowserDeterministic,
    Heuristic { confidence: f32 },
    Model { confidence: f32, model_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub url: String,
    pub title: String,
    pub document_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNodeView {
    pub semantic_ref: SemanticRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<SemanticRef>,
    /// Stable order within this observation, independent of numeric aliases.
    pub document_order: u32,
    /// Turn-visible numeric alias used by the existing OIL surface.
    pub alias: u32,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub states: BTreeSet<NodeState>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actions: BTreeSet<SemanticAction>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Supported,
    Partial,
    Unsupported,
    PolicyDenied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDiagnostic {
    pub capability: String,
    pub support: SupportLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<ExecutionDomain>,
    pub handoff_lossy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub contract_version: u16,
    pub page: PageInfo,
    pub revision: Revision,
    pub profile: ProjectionProfile,
    pub source: ObservationSource,
    pub nodes: Vec<SemanticNodeView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    pub execution_domain: ExecutionDomain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationDelta {
    pub contract_version: u16,
    pub from_revision: Revision,
    pub to_revision: Revision,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserted: Vec<SemanticNodeView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<SemanticRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Navigation {
        url: String,
    },
    Request {
        method: String,
        url: String,
    },
    Response {
        url: String,
        status: u16,
    },
    DomMutation {
        summary: String,
    },
    Event {
        event_type: String,
        target: SemanticRef,
    },
    Console {
        level: String,
        message: String,
    },
    Capability {
        diagnostic: CapabilityDiagnostic,
    },
    Correlated {
        summary: String,
        confidence: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub contract_version: u16,
    pub action_id: ActionId,
    pub revision_before: Revision,
    pub revision_after: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<ObservationDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    pub execution_domain: ExecutionDomain,
}

impl Observation {
    /// Compatibility bridge used while the scanner-backed domains are frozen.
    /// Scanner IDs become document-generation-scoped refs; this does not claim
    /// cross-churn identity preservation.
    pub fn from_scan(
        scan: &ScanResult,
        execution_domain: ExecutionDomain,
        document_generation: u64,
        revision: Revision,
    ) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                url: scan.page.url.clone(),
                title: scan.page.title.clone(),
                document_generation,
            },
            revision,
            profile: if scan.full_mode {
                ProjectionProfile::Full
            } else {
                ProjectionProfile::Compact
            },
            source: ObservationSource::CompatibilityScanner,
            nodes: scan
                .elements
                .iter()
                .map(|element| node_from_element(element, document_generation))
                .collect(),
            capabilities: Vec::new(),
            diagnostics: Vec::new(),
            execution_domain,
        }
    }

    pub fn resolve_alias(&self, alias: u32) -> Option<SemanticRef> {
        self.nodes
            .iter()
            .find(|node| node.alias == alias)
            .map(|node| node.semantic_ref)
    }
}

fn node_from_element(element: &Element, document_generation: u64) -> SemanticNodeView {
    let mut states = BTreeSet::new();
    let state = &element.state;
    for (enabled, value) in [
        (state.visible, NodeState::Visible),
        (state.hidden, NodeState::Hidden),
        (state.disabled, NodeState::Disabled),
        (state.focused, NodeState::Focused),
        (state.primary, NodeState::Primary),
        (state.checked, NodeState::Checked),
        (state.selected, NodeState::Selected),
        (state.required, NodeState::Required),
        (state.readonly, NodeState::Readonly),
        (state.expanded, NodeState::Expanded),
    ] {
        if enabled {
            states.insert(value);
        }
    }

    SemanticNodeView {
        semantic_ref: SemanticRef {
            document_generation,
            node: u64::from(element.id),
        },
        parent: None,
        document_order: element.id,
        alias: element.id,
        role: element
            .role
            .clone()
            .unwrap_or_else(|| element.element_type.clone()),
        name: element
            .text
            .clone()
            .or_else(|| element.label.clone())
            .or_else(|| element.placeholder.clone())
            .unwrap_or_default(),
        selector: (!element.selector.is_empty()).then(|| element.selector.clone()),
        value: element.value.clone(),
        description: None,
        states,
        actions: actions_for(element),
        provenance: Provenance::BrowserDeterministic,
    }
}

fn actions_for(element: &Element) -> BTreeSet<SemanticAction> {
    let mut actions = BTreeSet::new();
    if !element.state.disabled {
        match element.element_type.as_str() {
            "input" | "textarea" => {
                actions.extend([
                    SemanticAction::Type,
                    SemanticAction::Clear,
                    SemanticAction::Focus,
                ]);
            }
            "checkbox" | "radio" => {
                actions.extend([
                    SemanticAction::Check,
                    SemanticAction::Uncheck,
                    SemanticAction::Click,
                ]);
            }
            "select" => {
                actions.extend([SemanticAction::Select, SemanticAction::Focus]);
            }
            "button" | "link" => {
                actions.insert(SemanticAction::Click);
            }
            _ => {}
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::protocol::{
        ElementState, PageInfo as ScannerPageInfo, Rect, ScanStats, ScrollInfo, ViewportInfo,
    };

    use super::*;

    fn scan() -> ScanResult {
        ScanResult {
            page: ScannerPageInfo {
                url: "https://example.test".into(),
                title: "Example".into(),
                viewport: ViewportInfo::default(),
                scroll: ScrollInfo::default(),
                ready_state: Some("complete".into()),
            },
            elements: vec![Element {
                id: 7,
                element_type: "input".into(),
                role: Some("email".into()),
                text: None,
                label: Some("Email".into()),
                value: None,
                placeholder: None,
                selector: "#email".into(),
                xpath: None,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 20.0,
                },
                attributes: HashMap::new(),
                state: ElementState {
                    visible: true,
                    required: true,
                    ..ElementState::default()
                },
                children: Vec::new(),
            }],
            stats: ScanStats {
                total: 1,
                scanned: 1,
                iframes: None,
            },
            patterns: None,
            changes: None,
            available_intents: None,
            full_mode: false,
            settings_applied: None,
            timing: None,
        }
    }

    #[test]
    fn compatibility_scan_gets_generation_scoped_refs() {
        let observation =
            Observation::from_scan(&scan(), ExecutionDomain::Chromium, 3, Revision(11));

        assert_eq!(observation.contract_version, CONTRACT_VERSION);
        assert_eq!(
            observation.resolve_alias(7),
            Some(SemanticRef {
                document_generation: 3,
                node: 7,
            })
        );
        assert!(observation.nodes[0].actions.contains(&SemanticAction::Type));
        assert!(observation.nodes[0].states.contains(&NodeState::Required));
    }

    #[test]
    fn v2_observation_round_trips() {
        let observation =
            Observation::from_scan(&scan(), ExecutionDomain::Chromium, 1, Revision(2));
        let json = serde_json::to_string(&observation).expect("serialize");
        let decoded: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, observation);
    }
}
