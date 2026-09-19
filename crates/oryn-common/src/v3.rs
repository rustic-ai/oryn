//! Page-scoped native browser contracts for application compatibility.
//!
//! Contract v2 remains available unchanged for compatibility domains and
//! historical evidence. Native Oryn uses this contract so a semantic
//! reference can never be interpreted outside its owning top-level page.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::protocol::{Element, ScanResult};

pub use crate::v2::{
    ActionId, ContentTrust, ExecutionDomain, NodeState, ObservationSource, ProjectionProfile,
    Provenance, Revision, SemanticAction, SupportLevel,
};

pub const CONTRACT_VERSION: u16 = 3;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PageId(pub u64);

impl PageId {
    /// Page identifier used only when adapting a v2 compatibility observation.
    pub const LEGACY: Self = Self(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SemanticRef {
    #[serde(default)]
    pub page_id: PageId,
    pub document_generation: u64,
    pub node: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    Open,
    AttentionRequired,
    Closed,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub page_id: PageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener_page_id: Option<PageId>,
    pub url: String,
    pub title: String,
    pub document_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageDescriptor {
    pub page: PageInfo,
    pub active: bool,
    pub status: PageStatus,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PagePredicate {
    Any,
    Id { page_id: PageId },
    Origin { origin: String },
    UrlContains { value: String },
    TitleContains { value: String },
    Opener { page_id: PageId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageLifecycleEvent {
    Opened {
        page: PageDescriptor,
    },
    Activated {
        page_id: PageId,
    },
    Navigated {
        page_id: PageId,
        url: String,
        document_generation: u64,
    },
    AttentionRequired {
        page_id: PageId,
    },
    Closed {
        page_id: PageId,
    },
    Crashed {
        page_id: PageId,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeValidationState {
    /// Eligibility is separate from validity (e.g. disabled radios can be missing).
    pub will_validate: bool,
    pub valid: bool,
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub custom_error: bool,
    #[serde(default)]
    pub bad_input: bool,
    #[serde(default)]
    pub range_underflow: bool,
    #[serde(default)]
    pub range_overflow: bool,
    #[serde(default)]
    pub step_mismatch: bool,
    #[serde(default)]
    pub pattern_mismatch: bool,
    #[serde(default)]
    pub too_short: bool,
    #[serde(default)]
    pub too_long: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNodeView {
    pub semantic_ref: SemanticRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<SemanticRef>,
    pub document_order: u32,
    pub alias: u32,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Native flags only, never page-authored error text or control values.
    /// Absent for non-controls and backends without native validation evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<NativeValidationState>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub states: BTreeSet<NodeState>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actions: BTreeSet<SemanticAction>,
    /// Trust source for page-facing strings and values in this node.
    #[serde(default)]
    pub content_trust: ContentTrust,
    pub provenance: Provenance,
}

pub use crate::v2::CapabilityDiagnostic;

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
    pub page_id: PageId,
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
    PageOpened {
        page_id: PageId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opener_page_id: Option<PageId>,
        url: String,
    },
    PageClosed {
        page_id: PageId,
    },
    PageActivated {
        page_id: PageId,
    },
    Message {
        source_page_id: PageId,
        target_page_id: PageId,
        origin: String,
    },
    Storage {
        area: String,
        origin: String,
        operation: String,
    },
    WebSocket {
        operation: String,
        url: String,
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
    pub page_id: PageId,
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
    /// Lift a frozen contract-v2 observation into a page-scoped v3 view.
    /// This adapter does not infer identity beyond the supplied owning page.
    pub fn from_v2(observation: &crate::v2::Observation, page_id: PageId) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id,
                opener_page_id: None,
                url: observation.page.url.clone(),
                title: observation.page.title.clone(),
                document_generation: observation.page.document_generation,
            },
            revision: observation.revision,
            profile: observation.profile,
            source: observation.source,
            nodes: observation
                .nodes
                .iter()
                .map(|node| SemanticNodeView {
                    semantic_ref: SemanticRef {
                        page_id,
                        document_generation: node.semantic_ref.document_generation,
                        node: node.semantic_ref.node,
                    },
                    parent: node.parent.map(|parent| SemanticRef {
                        page_id,
                        document_generation: parent.document_generation,
                        node: parent.node,
                    }),
                    document_order: node.document_order,
                    alias: node.alias,
                    role: node.role.clone(),
                    name: node.name.clone(),
                    selector: node.selector.clone(),
                    value: node.value.clone(),
                    description: node.description.clone(),
                    validation: None,
                    states: node.states.clone(),
                    actions: node.actions.clone(),
                    content_trust: node.content_trust,
                    provenance: node.provenance.clone(),
                })
                .collect(),
            capabilities: observation.capabilities.clone(),
            diagnostics: observation.diagnostics.clone(),
            execution_domain: observation.execution_domain,
        }
    }

    /// Project v3 into the frozen v2 compatibility shape by explicitly
    /// dropping page ownership. Native callers should keep the v3 value.
    pub fn to_v2_compat(&self) -> crate::v2::Observation {
        crate::v2::Observation {
            contract_version: crate::v2::CONTRACT_VERSION,
            page: crate::v2::PageInfo {
                url: self.page.url.clone(),
                title: self.page.title.clone(),
                document_generation: self.page.document_generation,
            },
            revision: self.revision,
            profile: self.profile,
            source: self.source,
            nodes: self
                .nodes
                .iter()
                .map(|node| crate::v2::SemanticNodeView {
                    semantic_ref: crate::v2::SemanticRef {
                        document_generation: node.semantic_ref.document_generation,
                        node: node.semantic_ref.node,
                    },
                    parent: node.parent.map(|parent| crate::v2::SemanticRef {
                        document_generation: parent.document_generation,
                        node: parent.node,
                    }),
                    document_order: node.document_order,
                    alias: node.alias,
                    role: node.role.clone(),
                    name: node.name.clone(),
                    selector: node.selector.clone(),
                    value: node.value.clone(),
                    description: node.description.clone(),
                    states: node.states.clone(),
                    actions: node.actions.clone(),
                    content_trust: node.content_trust,
                    provenance: node.provenance.clone(),
                })
                .collect(),
            capabilities: self.capabilities.clone(),
            diagnostics: self.diagnostics.clone(),
            execution_domain: self.execution_domain,
        }
    }

    pub fn from_scan(
        scan: &ScanResult,
        execution_domain: ExecutionDomain,
        document_generation: u64,
        revision: Revision,
    ) -> Self {
        Self::from_scan_for_page(
            scan,
            execution_domain,
            PageId::LEGACY,
            document_generation,
            revision,
        )
    }

    pub fn from_scan_for_page(
        scan: &ScanResult,
        execution_domain: ExecutionDomain,
        page_id: PageId,
        document_generation: u64,
        revision: Revision,
    ) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id,
                opener_page_id: None,
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
                .map(|element| node_from_element(element, page_id, document_generation))
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

fn node_from_element(
    element: &Element,
    page_id: PageId,
    document_generation: u64,
) -> SemanticNodeView {
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
        validation: None,
        semantic_ref: SemanticRef {
            page_id,
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
        content_trust: ContentTrust::PageAuthored,
        provenance: Provenance::Heuristic { confidence: 0.8 },
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
    fn native_refs_are_page_scoped() {
        let observation = Observation::from_scan_for_page(
            &scan(),
            ExecutionDomain::Native,
            PageId(9),
            3,
            Revision(11),
        );
        assert_eq!(observation.contract_version, CONTRACT_VERSION);
        assert_eq!(
            observation.resolve_alias(7),
            Some(SemanticRef {
                page_id: PageId(9),
                document_generation: 3,
                node: 7,
            })
        );
        let encoded = serde_json::to_string(&observation).expect("serialize v3 observation");
        let decoded: Observation = serde_json::from_str(&encoded).expect("deserialize v3");
        assert_eq!(decoded, observation);
    }

    #[test]
    fn v2_shaped_ref_deserializes_as_legacy_page() {
        let reference: SemanticRef = serde_json::from_str(r#"{"document_generation":4,"node":2}"#)
            .expect("deserialize legacy ref");
        assert_eq!(reference.page_id, PageId::LEGACY);
    }

    #[test]
    fn native_validation_is_optional_and_roundtrips_without_page_text() {
        let legacy =
            crate::v2::Observation::from_scan(&scan(), ExecutionDomain::Chromium, 5, Revision(8));
        let mut observation = Observation::from_v2(&legacy, PageId(42));
        assert!(observation.nodes.iter().all(|n| n.validation.is_none()));
        let legacy_json = serde_json::to_string(&observation).unwrap();
        assert!(!legacy_json.contains("validation"));
        assert_eq!(
            serde_json::from_str::<Observation>(&legacy_json).unwrap(),
            observation
        );
        observation.nodes[0].validation = Some(NativeValidationState {
            bad_input: false,
            pattern_mismatch: false,
            too_short: false,
            too_long: false,
            range_underflow: false,
            range_overflow: false,
            step_mismatch: false,
            will_validate: false,
            valid: false,
            value_missing: true,
            type_mismatch: false,
            custom_error: true,
        });
        let encoded = serde_json::to_string(&observation).unwrap();
        assert_eq!(
            serde_json::from_str::<Observation>(&encoded).unwrap(),
            observation
        );
    }

    #[test]
    fn v2_observation_adapter_roundtrips_without_fabricating_identity() {
        let v2 =
            crate::v2::Observation::from_scan(&scan(), ExecutionDomain::Chromium, 5, Revision(8));
        let v3 = Observation::from_v2(&v2, PageId(42));
        assert_eq!(v3.page.page_id, PageId(42));
        assert!(
            v3.nodes
                .iter()
                .all(|node| node.semantic_ref.page_id == PageId(42))
        );
        assert!(
            v3.nodes
                .iter()
                .all(|node| node.content_trust == ContentTrust::PageAuthored)
        );
        assert_eq!(v3.to_v2_compat(), v2);
    }
}
