//! Native style, layout, viewport, and geometry contracts.
//!
//! Contract v3 remains frozen.  The compatibility adapter below deliberately
//! strips geometry instead of inventing it for older consumers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::v3::{
    ActionId, CapabilityDiagnostic, ContentTrust, ExecutionDomain, NodeState, ObservationSource,
    PageDescriptor, PageId, PageInfo, PageLifecycleEvent, PagePredicate, PageStatus,
    ProjectionProfile, Provenance, Revision, SemanticAction, SemanticRef, SupportLevel,
};

pub const CONTRACT_VERSION: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A shaped text run positioned by the worker-owned layout engine.  This is a
/// neutral transfer contract: it contains no rasterizer or page state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTextFragment {
    pub text: String,
    pub rect: CssRect,
    pub font_family: String,
    pub font_size: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintImageResource {
    pub url: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

mod base64_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        STANDARD.decode(value).map_err(serde::de::Error::custom)
    }
}

/// Read-only paint input exported by a page worker.  Optional renderers may
/// consume this value but can never mutate DOM, JavaScript, cookies or policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintNodeSnapshot {
    pub node_id: u64,
    pub tag: String,
    pub geometry: NodeGeometry,
    pub style: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<PaintTextFragment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_resource: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintSnapshot {
    pub contract_version: u16,
    pub page: PageInfo,
    pub revision: Revision,
    pub style_revision: StyleRevision,
    pub layout_revision: LayoutRevision,
    pub viewport: ViewportState,
    pub scroll: ScrollState,
    pub nodes: Vec<PaintNodeSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PaintImageResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl CssRect {
    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn intersects(self, other: Self) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && other.width > 0.0
            && other.height > 0.0
            && self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeSizes {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for EdgeSizes {
    fn default() -> Self {
        Self {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxModel {
    pub border_box: CssRect,
    pub padding_box: CssRect,
    pub content_box: CssRect,
    pub margin: EdgeSizes,
    pub border: EdgeSizes,
    pub padding: EdgeSizes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryAvailability {
    Available,
    Hidden,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeGeometry {
    pub availability: GeometryAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_model: Option<BoxModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_rects: Vec<CssRect>,
    #[serde(default)]
    pub in_viewport: bool,
    #[serde(default)]
    pub interactable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewportState {
    pub width: f64,
    pub height: f64,
    pub device_pixel_ratio: f64,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 720.0,
            device_pixel_ratio: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrollState {
    pub x: f64,
    pub y: f64,
    pub content_width: f64,
    pub content_height: f64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferredColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReducedMotion {
    Reduce,
    #[default]
    NoPreference,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaState {
    pub color_scheme: PreferredColorScheme,
    pub reduced_motion: ReducedMotion,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            content_width: 1280.0,
            content_height: 720.0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleRevision(pub u64);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutRevision(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationOptions {
    pub profile: ProjectionProfile,
    #[serde(default)]
    pub include_geometry: bool,
    #[serde(default)]
    pub viewport_only: bool,
    #[serde(default)]
    pub include_hidden: bool,
}

impl Default for ObservationOptions {
    fn default() -> Self {
        Self {
            profile: ProjectionProfile::Compact,
            include_geometry: false,
            viewport_only: false,
            include_hidden: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNodeView {
    #[serde(flatten)]
    pub semantic: crate::v3::SemanticNodeView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<NodeGeometry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub contract_version: u16,
    pub page: PageInfo,
    pub revision: Revision,
    pub style_revision: StyleRevision,
    pub layout_revision: LayoutRevision,
    pub profile: ProjectionProfile,
    pub source: ObservationSource,
    pub viewport: ViewportState,
    pub scroll: ScrollState,
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
    pub style_revision: StyleRevision,
    pub layout_revision: LayoutRevision,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserted: Vec<SemanticNodeView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<SemanticRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<ViewportState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<ScrollState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Legacy { effect: crate::v3::Effect },
    StyleChanged { style_revision: StyleRevision },
    LayoutChanged { layout_revision: LayoutRevision },
    ViewportChanged { viewport: ViewportState },
    Scrolled { scroll: ScrollState },
    FontLoaded { family: String, sha256: String },
    InteractabilityRejected { target: SemanticRef, reason: String },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputedStyle {
    pub semantic_ref: SemanticRef,
    pub style_revision: StyleRevision,
    pub properties: BTreeMap<String, String>,
}

impl Observation {
    pub fn from_v3(observation: crate::v3::Observation, viewport: ViewportState) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            page: observation.page,
            revision: observation.revision,
            style_revision: StyleRevision(observation.revision.0),
            layout_revision: LayoutRevision(observation.revision.0),
            profile: observation.profile,
            source: observation.source,
            viewport,
            scroll: ScrollState {
                content_width: viewport.width,
                content_height: viewport.height,
                ..ScrollState::default()
            },
            nodes: observation
                .nodes
                .into_iter()
                .map(|semantic| SemanticNodeView {
                    semantic,
                    geometry: None,
                })
                .collect(),
            capabilities: observation.capabilities,
            diagnostics: observation.diagnostics,
            execution_domain: observation.execution_domain,
        }
    }

    pub fn to_v3_compat(&self) -> crate::v3::Observation {
        crate::v3::Observation {
            contract_version: crate::v3::CONTRACT_VERSION,
            page: self.page.clone(),
            revision: self.revision,
            profile: self.profile,
            source: self.source,
            nodes: self
                .nodes
                .iter()
                .map(|node| node.semantic.clone())
                .collect(),
            capabilities: self.capabilities.clone(),
            diagnostics: self.diagnostics.clone(),
            execution_domain: self.execution_domain,
        }
    }

    pub fn resolve_alias(&self, alias: u32) -> Option<SemanticRef> {
        self.nodes
            .iter()
            .find(|node| node.semantic.alias == alias)
            .map(|node| node.semantic.semantic_ref)
    }
}

impl ObservationDelta {
    pub fn from_v3(delta: crate::v3::ObservationDelta) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            page_id: delta.page_id,
            from_revision: delta.from_revision,
            to_revision: delta.to_revision,
            style_revision: StyleRevision(delta.to_revision.0),
            layout_revision: LayoutRevision(delta.to_revision.0),
            upserted: delta
                .upserted
                .into_iter()
                .map(|semantic| SemanticNodeView {
                    semantic,
                    geometry: None,
                })
                .collect(),
            removed: delta.removed,
            viewport: None,
            scroll: None,
        }
    }
}

impl ActionResult {
    pub fn from_v3(result: crate::v3::ActionResult) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            action_id: result.action_id,
            page_id: result.page_id,
            revision_before: result.revision_before,
            revision_after: result.revision_after,
            delta: result.delta.map(ObservationDelta::from_v3),
            effects: result
                .effects
                .into_iter()
                .map(|effect| Effect::Legacy { effect })
                .collect(),
            diagnostics: result.diagnostics,
            execution_domain: result.execution_domain,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn v3_observation() -> crate::v3::Observation {
        crate::v3::Observation {
            contract_version: crate::v3::CONTRACT_VERSION,
            page: PageInfo {
                page_id: PageId(7),
                opener_page_id: None,
                url: "https://example.test".into(),
                title: "Fixture".into(),
                document_generation: 3,
            },
            revision: Revision(9),
            profile: ProjectionProfile::Compact,
            source: ObservationSource::NativeDom,
            nodes: vec![crate::v3::SemanticNodeView {
                semantic_ref: SemanticRef {
                    page_id: PageId(7),
                    document_generation: 3,
                    node: 11,
                },
                parent: None,
                document_order: 1,
                alias: 1,
                role: "button".into(),
                name: "Continue".into(),
                selector: Some("#continue".into()),
                value: None,
                description: None,
                states: BTreeSet::from([NodeState::Visible]),
                actions: BTreeSet::from([SemanticAction::Click]),
                content_trust: ContentTrust::PageAuthored,
                provenance: Provenance::Heuristic { confidence: 0.7 },
            }],
            capabilities: Vec::new(),
            diagnostics: Vec::new(),
            execution_domain: ExecutionDomain::Native,
        }
    }

    #[test]
    fn rectangle_intersection_excludes_edges_and_empty_boxes() {
        let viewport = CssRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        assert!(viewport.intersects(CssRect {
            x: 99.0,
            y: 99.0,
            width: 2.0,
            height: 2.0,
        }));
        assert!(!viewport.intersects(CssRect {
            x: 100.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        }));
    }

    #[test]
    fn v4_roundtrip_preserves_the_frozen_v3_projection() {
        let v3 = v3_observation();
        let v4 = Observation::from_v3(v3.clone(), ViewportState::default());
        assert_eq!(v4.contract_version, CONTRACT_VERSION);
        assert_eq!(v4.to_v3_compat(), v3);
        let encoded = serde_json::to_string(&v4).expect("serialize v4");
        assert!(encoded.contains(r#""content_trust":"page_authored""#));
        let decoded: Observation = serde_json::from_str(&encoded).expect("deserialize v4");
        assert_eq!(decoded, v4);
    }

    #[test]
    fn v3_adapter_never_invents_geometry() {
        let v4 = Observation::from_v3(v3_observation(), ViewportState::default());
        assert!(v4.nodes.iter().all(|node| node.geometry.is_none()));
        assert_eq!(v4.scroll.x, 0.0);
        assert_eq!(v4.scroll.y, 0.0);
    }
}
