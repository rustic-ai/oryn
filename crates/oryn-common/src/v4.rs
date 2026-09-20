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
/// Display-list protocol shared by workers, renderers, and V8-free parents.
/// Supporting this wire version does not imply an embedded renderer.
pub const RENDER_CONTRACT_VERSION: u16 = 3;
/// Additive native fragment ownership metadata within paint snapshot v4.
pub const LOCAL_EFFECTS_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A shaped text run positioned by the worker-owned layout engine.  This is a
/// neutral transfer contract: it contains no rasterizer or page state.
/// Native shaped cluster provenance in one fragment's logical UTF8 text.
/// Rectangles share the fragment's physical coordinate plane, before projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTextCluster {
    pub start: usize,
    pub end: usize,
    pub rect: CssRect,
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTextFragment {
    /// Native CSS line-flow bounds in the same coordinate plane as `rect`.
    /// Selection and glyph extents may overflow these bounds with negative leading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_rect: Option<CssRect>,
    /// Native line identity within the owning layout paragraph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<PaintTextCluster>,
    pub text: String,
    pub rect: CssRect,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyph_runs: Vec<PaintGlyphRun>,
    pub font_family: String,
    pub font_size: f64,
    pub color: String,
}

/// Glyph positions are relative to the text fragment's origin, in CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintGlyph {
    /// Native source cluster in the owning fragment's UTF8 bytes. Annotation
    /// glyphs and legacy snapshots have no text identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<std::ops::Range<usize>>,
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub advance: f64,
}

/// Per-glyph outline orientation around its physical baseline origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintGlyphOrientation {
    #[default]
    Upright,
    Clockwise,
    CounterClockwise,
}

/// Decoration owner values retained across inline shaping and descendant colors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTextDecoration {
    pub line: String,
    pub style: String,
    pub color: [u8; 4],
    pub thickness: Option<f64>,
    pub underline_offset: Option<f64>,
    pub position: String,
    pub skip_ink: bool,
    pub font_size: f64,
}

/// A native text pseudo's independently formatted border/background box.
/// Coordinates are relative to the owning text fragment, like glyph positions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTextBox {
    pub owner: u64,
    pub model: BoxModel,
    pub style: BTreeMap<String, String>,
}
impl PaintTextBox {
    pub fn translate(&mut self, x: f64, y: f64) {
        for rect in [
            &mut self.model.border_box,
            &mut self.model.padding_box,
            &mut self.model.content_box,
        ] {
            rect.x += x;
            rect.y += y;
        }
    }
    pub fn mirror_y(&mut self, extent: f64) {
        let content = &mut self.model.content_box;
        content.y = extent - content.bottom();
        let c = *content;
        let p = self.model.padding;
        let b = self.model.border;
        self.model.padding_box = CssRect {
            x: c.x - p.left,
            y: c.y - p.top,
            width: c.width + p.left + p.right,
            height: c.height + p.top + p.bottom,
        };
        let p = self.model.padding_box;
        self.model.border_box = CssRect {
            x: p.x - b.left,
            y: p.y - b.top,
            width: p.width + b.left + b.right,
            height: p.height + b.top + b.bottom,
        };
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintGlyphRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_box: Option<PaintTextBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_box: Option<PaintTextBox>,
    /// Native text-fragment background, painted behind glyphs using the run box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[u8; 4]>,
    /// Native physical run box relative to the owning text fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration_rect: Option<CssRect>,
    #[serde(default = "unit_glyph_scale")]
    pub scale_x: f64,
    #[serde(default)]
    pub vertical: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorations: Vec<PaintTextDecoration>,
    #[serde(default)]
    pub orientation: PaintGlyphOrientation,
    #[serde(default)]
    pub embolden: bool,
    #[serde(default)]
    pub skew_degrees: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normalized_coords: Vec<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    pub font_key: String,
    pub font_size: f64,
    pub glyphs: Vec<PaintGlyph>,
}

fn unit_glyph_scale() -> f64 {
    1.
}

/// Native text-node range geometry. Rectangles are in the original viewport
/// plane; retain the matrix so range clipping happens before projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextClusterGeometry {
    pub layout_node: u64,
    pub start: usize,
    pub end: usize,
    pub rect: CssRect,
    pub matrix: [f64; 16],
    pub line: usize,
    pub rtl: bool,
}

/// A native caret position at a DOM element child boundary. These offsets are
/// child indexes, deliberately separate from UTF16 TextClusterGeometry offsets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaretStopGeometry {
    pub boundary_node: u64,
    pub offset: usize,
    pub layout_node: u64,
    pub rect: CssRect,
    pub matrix: [f64; 16],
    pub line: usize,
    pub rtl: bool,
}

/// The exact face used by shaping; keys include the collection face index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintFontResource {
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    pub collection_index: u32,
}

/// Native SVG use-instance projection; identities refer to captured source
/// markers, never DOM IDs supplied by author script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvgInstanceSnapshot {
    pub ancestry: Vec<u64>,
    pub source: u64,
    pub presentation: String,
    #[serde(default)]
    pub content: SvgInstanceContent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reference", rename_all = "snake_case")]
pub enum SvgInstanceContent {
    #[default]
    Original,
    Empty,
    Reference(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintImageResource {
    /// Native serialization of a live document's inline SVG, carrying that
    /// document's admitted font/resource environment. Fetched SVG images and
    /// resource documents are secure and cannot borrow the embedding environment.
    #[serde(default)]
    pub svg_document_snapshot: bool,
    /// Captured native instance data. Independently fetched resources must
    /// leave this empty and compute their own secure SVG instance cascade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svg_instances: Vec<SvgInstanceSnapshot>,
    /// Original successful response Content-Type, including XML charset.
    /// Synthesized resources omit this; media_type remains a normalized essence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Native successful-request aliases; final URL remains the relative-resource base.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url_aliases: Vec<String>,
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
pub struct PaintClip {
    #[serde(default)]
    pub shape: Option<crate::basic_shape::Shape>,
    /// Packed native layout node identity. None denotes a viewport or paint-only clip.
    #[serde(default)]
    pub owner: Option<u64>,
    /// Distinguish scroll containers from overflow:clip independently per axis.
    #[serde(default)]
    pub scroll_x: bool,
    #[serde(default)]
    pub scroll_y: bool,
    pub matrix: [f64; 16],
    pub rect: CssRect,
    pub clip_x: bool,
    pub clip_y: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintBorderSegment {
    pub rect: CssRect,
    pub edge: PaintBorderEdge,
    pub horizontal: bool,
}

/// Resolved used border paint, separate from authored/computed DOM style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintBorderEdge {
    pub width: f64,
    pub color: String,
    pub style: String,
}

/// Native scroll geometry in the box's untransformed physical coordinate plane.
/// Offsets are the exact translations used for descendant and text projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintScrollGeometry {
    pub offset_x: f64,
    pub offset_y: f64,
    pub scrollport: CssRect,
    pub scrollable: CssRect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<PaintScrollGeometry>,
    /// Fragment-local physical boxes before the shared projection. Empty keeps legacy single-box paint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragment_boxes: Vec<BoxModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub border_segments: Vec<PaintBorderSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_edges: Option<[PaintBorderEdge; 4]>,
    /// Text can scroll inside the stationary border/padding box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_matrix: Option<[f64; 16]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_clips: Vec<PaintClip>,
    /// Column-major homogeneous transform from original viewport coordinates.
    pub matrix: [f64; 16],
    pub box_model: BoxModel,
    pub text: Vec<PaintTextFragment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<PaintClip>,
}

/// Map a viewport point onto the original z=0 plane. Reject singular planes
/// and the non-visible side of a perspective horizon.
fn unproject_plane_point(matrix: [f64; 16], x: f64, y: f64) -> Option<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() || !matrix.iter().all(|value| value.is_finite()) {
        return None;
    }
    // Axis-aligned affine planes need only subtraction and division.
    // Normalizing by the viewport point introduces avoidable rounding into
    // authored scroll offsets, even for exactly representable scales.
    if matrix[0] != 0.
        && matrix[5] != 0.
        && matrix[15] == 1.
        && matrix[1] == 0.
        && matrix[4] == 0.
        && matrix[3] == 0.
        && matrix[7] == 0.
    {
        let point = ((x - matrix[12]) / matrix[0], (y - matrix[13]) / matrix[5]);
        if point.0.is_finite() && point.1.is_finite() {
            return Some(point);
        }
        // Subtraction can overflow even when division yields a finite point.
        // The scaled solver below retains that otherwise valid inverse.
    }
    // Only coefficients affecting the z=0 plane participate. Unrelated depth
    // coefficients must not underflow an otherwise invertible planar transform.
    let h = [
        matrix[0], matrix[4], matrix[12], matrix[1], matrix[5], matrix[13], matrix[3], matrix[7],
        matrix[15],
    ];
    let scale = h.iter().fold(0_f64, |scale, value| scale.max(value.abs()));
    if scale == 0. {
        return None;
    }
    let h = h.map(|value| value / scale);
    let row_x = [h[0] - x * h[6], h[1] - x * h[7], x * h[8] - h[2]];
    let row_y = [h[3] - y * h[6], h[4] - y * h[7], y * h[8] - h[5]];
    let normalize = |row: [f64; 3]| {
        let scale = row
            .iter()
            .fold(0_f64, |scale, value| scale.max(value.abs()));
        (scale > 0. && scale.is_finite()).then(|| row.map(|value| value / scale))
    };
    let [a, b, c] = normalize(row_x)?;
    let [d, e, f] = normalize(row_y)?;
    let determinant = a * e - b * d;
    if determinant == 0. || !determinant.is_finite() {
        return None;
    }
    let u = (c * e - b * f) / determinant;
    let v = (a * f - c * d) / determinant;
    let w = h[6] * u + h[7] * v + h[8];
    (u.is_finite() && v.is_finite() && w.is_finite() && w > 0.).then_some((u, v))
}

fn finite_positive_rect(rect: CssRect) -> bool {
    rect.width > 0.
        && rect.height > 0.
        && [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.right(),
            rect.bottom(),
        ]
        .iter()
        .all(|value| value.is_finite())
}

impl PaintProjection {
    /// Convert viewport coordinates back to this box's untransformed plane.
    pub fn viewport_to_local(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        unproject_plane_point(self.matrix, x, y)
    }

    /// Hit the projected border plane and every ancestor clip in its own plane.
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let rect = self.box_model.border_box;
        if !finite_positive_rect(rect) {
            return false;
        }
        let Some((u, v)) = unproject_plane_point(self.matrix, x, y) else {
            return false;
        };
        if u < rect.x || u > rect.right() || v < rect.y || v > rect.bottom() {
            return false;
        }
        if !self.fragment_boxes.is_empty()
            && !self.fragment_boxes.iter().any(|model| {
                let rect = model.border_box;
                finite_positive_rect(rect)
                    && u >= rect.x
                    && u <= rect.right()
                    && v >= rect.y
                    && v <= rect.bottom()
            })
        {
            return false;
        }
        self.clips.iter().all(|clip| {
            if !clip.clip_x && !clip.clip_y {
                return true;
            }
            let Some((u, v)) = unproject_plane_point(clip.matrix, x, y) else {
                return false;
            };
            clip.shape.as_ref().is_none_or(|shape| shape.contains(u, v))
                && (!clip.clip_x
                    || (clip.rect.x.is_finite()
                        && clip.rect.right().is_finite()
                        && clip.rect.width > 0.
                        && u >= clip.rect.x
                        && u <= clip.rect.right()))
                && (!clip.clip_y
                    || (clip.rect.y.is_finite()
                        && clip.rect.bottom().is_finite()
                        && clip.rect.height > 0.
                        && v >= clip.rect.y
                        && v <= clip.rect.bottom()))
        })
    }

    /// Test the projected border polygon against the viewport before dividing
    /// by w. Corner bounding boxes are insufficient when an edge crosses w=0.
    pub fn intersects_viewport(&self, viewport: ViewportState) -> bool {
        if self.fragment_boxes.is_empty() {
            projected_rect_intersects_viewport(self.box_model.border_box, self.matrix, viewport)
        } else {
            self.fragment_boxes.iter().any(|model| {
                projected_rect_intersects_viewport(model.border_box, self.matrix, viewport)
            })
        }
    }

    /// Conservative paint admission, including text outside the border box.
    /// Rasterization applies each clip in its own projection plane afterwards.
    /// This does not enlarge the element's hit-test region.
    pub fn paint_intersects_viewport(&self, viewport: ViewportState) -> bool {
        self.intersects_viewport(viewport)
            || self.border_segments.iter().any(|segment| {
                projected_rect_intersects_viewport(segment.rect, self.matrix, viewport)
            })
            || self.text.iter().any(|fragment| {
                projected_rect_intersects_viewport(
                    fragment.rect,
                    self.text_matrix.unwrap_or(self.matrix),
                    viewport,
                )
            })
    }
}

fn projected_rect_intersects_viewport(
    rect: CssRect,
    matrix: [f64; 16],
    viewport: ViewportState,
) -> bool {
    if !finite_positive_rect(rect)
        || !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || viewport.width <= 0.
        || viewport.height <= 0.
    {
        return false;
    }
    let m = matrix;
    let h = [m[0], m[4], m[12], m[1], m[5], m[13], m[3], m[7], m[15]];
    if !h.iter().all(|v| v.is_finite()) {
        return false;
    }
    let scale = h.iter().fold(0.0_f64, |a, v| a.max(v.abs()));
    if scale == 0. {
        return false;
    }
    let h = h.map(|v| v / scale);
    let mut polygon = [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ]
    .map(|(x, y)| {
        [
            h[0] * x + h[1] * y + h[2],
            h[3] * x + h[4] * y + h[5],
            h[6] * x + h[7] * y + h[8],
        ]
    })
    .to_vec();
    for plane in [
        [0., 0., 1.],
        [1., 0., 0.],
        [-1., 0., viewport.width],
        [0., 1., 0.],
        [0., -1., viewport.height],
    ] {
        if polygon.is_empty() {
            return false;
        }
        let distance =
            |point: [f64; 3]| point[0] * plane[0] + point[1] * plane[1] + point[2] * plane[2];
        let mut clipped = Vec::new();
        let mut previous = *polygon.last().unwrap();
        let mut before = distance(previous);
        for current in polygon {
            let after = distance(current);
            if (before >= 0.) != (after >= 0.) {
                let t = before / (before - after);
                clipped.push(std::array::from_fn(|i| {
                    previous[i] + (current[i] - previous[i]) * t
                }));
            }
            if after >= 0. {
                clipped.push(current);
            }
            previous = current;
            before = after;
        }
        polygon = clipped;
    }
    let points = polygon
        .iter()
        .filter(|p| p[2] > 0.)
        .map(|p| (p[0] / p[2], p[1] / p[2]))
        .collect::<Vec<_>>();
    if points.len() < 3 {
        return false;
    }
    let area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.0 * b.1 - a.1 * b.0)
        .sum::<f64>();
    area.is_finite() && area != 0.
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintNodeSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_image: Option<crate::builtin_image::BuiltinImage>,
    /// Native independently laid-out pseudo; nesting is limited to one level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<Box<PaintNodeSnapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<PaintControl>,
    /// Native Stylo ::backdrop computed values for a top-layer element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop_style: Option<BTreeMap<String, String>>,
    /// Native document top-layer insertion order, independent of CSS z-index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_layer: Option<usize>,
    pub node_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u64>,
    pub tag: String,
    pub geometry: NodeGeometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<PaintProjection>,
    pub style: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<PaintTextFragment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_resource: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopLayerKind {
    ModalDialog,
    Popover,
    SelectPicker,
    TemporalPicker,
    PermissionPrompt,
    Fullscreen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintMeterRegion {
    Optimum,
    Suboptimal,
    EvenLessGood,
}
/// Final native shaping text and DOM provenance. Rendered ranges are UTF-8
/// byte offsets; source ranges are UTF-16 offsets in the original native node.
/// Formatting/style owners may be private layout identities, never DOM handles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTextRun {
    pub layout_node: u64,
    pub text: String,
    pub spans: Vec<ResolvedTextSpan>,
    pub aliases: Vec<ResolvedTextSpan>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTextSpan {
    pub owner: u64,
    pub source_node: Option<u64>,
    pub rendered_start: usize,
    pub rendered_end: usize,
    pub source_start: usize,
    pub source_end: usize,
    pub transformed: bool,
    pub atomic: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedContent {
    pub owner: u64,
    pub pseudo: String,
    pub text: String,
    #[serde(default)]
    pub alternative: Option<String>,
    /// Native pseudo display creates a separate accessibility text run.
    #[serde(default)]
    pub separates_text: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaintControl {
    Meter {
        fraction: f64,
        region: PaintMeterRegion,
    },
    Progress {
        fraction: Option<f64>,
    },
    Checkbox {
        checked: bool,
        indeterminate: bool,
        disabled: bool,
    },
    Radio {
        checked: bool,
        disabled: bool,
    },
    Range {
        fraction: f64,
        disabled: bool,
        #[serde(default)]
        ticks: Vec<f64>,
    },
    Color {
        rgba: [u8; 4],
        disabled: bool,
    },
    Select {
        #[serde(default)]
        active_option: Option<u64>,
        open: bool,
        multiple: bool,
        disabled: bool,
        #[serde(default)]
        selected: Vec<u64>,
        #[serde(default)]
        label: String,
        #[serde(default)]
        display_size: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SelectPickerState {
    pub scroll: (f64, f64),
    pub active: Option<u64>,
    pub reveal: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TopLayerEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_source: Option<u64>,
    pub node_id: u64,
    pub kind: TopLayerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<SelectPickerState>,
}

/// Independently formatted HTML content for one captured SVG foreignObject.
/// Coordinates are local to its SVG viewport; resource indexes reference the
/// containing PaintSnapshot. `element_id` is capture-generated XML identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgForeignObjectSnapshot {
    pub image_resource: usize,
    pub element_id: String,
    pub viewport: ViewportState,
    pub nodes: Vec<PaintNodeSnapshot>,
}

/// Viewport scrollbars live in physical viewport coordinates, independent of
/// root-element transforms. Style contains the resolved propagated viewport policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintViewportScroll {
    pub scroll: PaintScrollGeometry,
    pub style: BTreeMap<String, String>,
}

/// Captured child document with its own resource authority and local viewport.
/// `owner_node` belongs to the immediately enclosing snapshot only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintFrameSnapshot {
    pub owner_node: u64,
    pub snapshot: Box<PaintSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintSnapshot {
    /// Native current-document fragment bindings; never reconstructed from image URLs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_effects: BTreeMap<String, usize>,
    /// Native visual viewport for this document only; print/local subtree snapshots omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_viewport: Option<crate::visual_viewport::Geometry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<PaintFrameSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_scroll: Option<PaintViewportScroll>,
    pub contract_version: u16,
    pub page: PageInfo,
    pub revision: Revision,
    pub style_revision: StyleRevision,
    pub layout_revision: LayoutRevision,
    pub viewport: ViewportState,
    pub scroll: ScrollState,
    pub nodes: Vec<PaintNodeSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svg_foreign_objects: Vec<SvgForeignObjectSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PaintImageResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fonts: BTreeMap<String, PaintFontResource>,
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

    #[test]
    fn native_line_flow_metadata_is_optional_and_round_trips_separately() {
        let legacy = serde_json::json!({"text":"A","rect":{"x":0.,"y":-1.,"width":10.,"height":22.},"font_family":"native","font_size":16.,"color":"black"});
        let mut fragment: PaintTextFragment = serde_json::from_value(legacy).unwrap();
        assert_eq!(fragment.flow_rect, None);
        assert_eq!(fragment.flow_line, None);
        let selection = fragment.rect;
        fragment.flow_rect = Some(CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 20.,
        });
        fragment.flow_line = Some(3);
        let restored: PaintTextFragment =
            serde_json::from_value(serde_json::to_value(&fragment).unwrap()).unwrap();
        assert_eq!(restored, fragment);
        assert_eq!(restored.rect, selection);
        assert_eq!(restored.flow_rect.unwrap().height, 20.);
    }

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
                validation: None,
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

#[cfg(test)]
mod projection_visibility_tests {
    use super::*;
    #[test]
    fn identity_plane_inverse_preserves_integer_and_fractional_scroll_coordinates() {
        let mut matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        for point in [(0., 200.), (8., 208.), (-12.5, 200.25)] {
            assert_eq!(unproject_plane_point(matrix, point.0, point.1), Some(point));
        }
        matrix[12] = 8.;
        matrix[13] = -4.5;
        assert_eq!(
            unproject_plane_point(matrix, 20.5, 195.75),
            Some((12.5, 200.25))
        );
        matrix[0] = 0.;
        assert_eq!(unproject_plane_point(matrix, 20.5, 195.75), None);
    }

    #[test]
    fn scaled_plane_inverse_preserves_exact_scroll_offsets_without_rounding_fractions() {
        for (sx, sy) in [(2., 2.), (-2., 0.5), (0.25, -4.)] {
            let matrix = [
                sx, 0., 0., 0., 0., sy, 0., 0., 0., 0., 1., 0., 8., -4., 0., 1.,
            ];
            for (x, y) in [(8., 200.), (12.125, 183.25), (-2.5, -0.125)] {
                assert_eq!(
                    unproject_plane_point(matrix, x * sx + 8., y * sy - 4.),
                    Some((x, y))
                );
            }
        }
    }

    #[test]
    fn scaled_inverse_falls_back_when_intermediate_subtraction_overflows() {
        let large = f64::MAX;
        let matrix = [
            large, 0., 0., 0., 0., large, 0., 0., 0., 0., 1., 0., large, 0., 0., 1.,
        ];
        assert_eq!(unproject_plane_point(matrix, -large, 0.), Some((-2., 0.)));
    }

    #[test]
    fn plane_inversion_preserves_small_scales_and_ignores_unrelated_depth_scale() {
        let mut matrix = [
            1e-200, 0., 0., 0., 0., 1e-200, 0., 0., 0., 0., 1e200, 0., 0., 0., 0., 1.,
        ];
        let point = unproject_plane_point(matrix, 2e-200, 3e-200).unwrap();
        assert!((point.0 - 2.).abs() < 1e-12 && (point.1 - 3.).abs() < 1e-12);
        matrix = [
            1., 0., 0., -1., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        assert_eq!(unproject_plane_point(matrix, 1., 1.), Some((0.5, 0.5)));
        assert_eq!(unproject_plane_point(matrix, -3., -1.), None);
        assert_eq!(unproject_plane_point(matrix, -1., 0.), None);
    }

    #[test]
    fn fragmented_inline_hit_regions_exclude_the_union_gap() {
        let model = |rect| BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
        };
        let projection = PaintProjection {
            scroll: None,
            fragment_boxes: vec![
                model(CssRect {
                    x: 0.,
                    y: 0.,
                    width: 20.,
                    height: 10.,
                }),
                model(CssRect {
                    x: 0.,
                    y: 20.,
                    width: 10.,
                    height: 10.,
                }),
            ],
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            box_model: model(CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 30.,
            }),
            text: Vec::new(),
            clips: Vec::new(),
        };
        assert!(projection.contains_point(5., 5.));
        assert!(projection.contains_point(5., 25.));
        assert!(!projection.contains_point(5., 15.));
        assert!(!projection.contains_point(15., 25.));
    }

    #[test]
    fn nonfinite_border_geometry_never_intersects_or_hits() {
        let rect = CssRect {
            x: f64::NAN,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            box_model: BoxModel {
                border_box: rect,
                padding_box: rect,
                content_box: rect,
                margin: EdgeSizes::default(),
                border: EdgeSizes::default(),
                padding: EdgeSizes::default(),
            },
            text: Vec::new(),
            clips: Vec::new(),
        };
        assert!(!projection.contains_point(5., 5.));
        assert!(!projection.intersects_viewport(ViewportState::default()));
    }

    #[test]
    fn projected_hits_reject_bbox_corners_and_respect_clip_planes() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let c = std::f64::consts::FRAC_1_SQRT_2;
        let mut projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: [c, c, 0., 0., -c, c, 0., 0., 0., 0., 1., 0., 10., 0., 0., 1.],
            box_model: BoxModel {
                border_box: rect,
                padding_box: rect,
                content_box: rect,
                margin: EdgeSizes::default(),
                border: EdgeSizes::default(),
                padding: EdgeSizes::default(),
            },
            text: Vec::new(),
            clips: Vec::new(),
        };
        assert!(projection.contains_point(10., 7.));
        assert!(!projection.contains_point(3., 1.));
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        projection.clips.push(PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: identity,
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 9.,
                height: 100.,
            },
            clip_x: true,
            clip_y: false,
        });
        assert!(!projection.contains_point(10., 7.));
        projection.clips[0].clip_x = false;
        assert!(projection.contains_point(10., 7.));
        projection.matrix = identity;
        projection.matrix[15] = -1.;
        assert!(!projection.contains_point(-5., -5.));
        projection.matrix = identity;
        projection.matrix[0] = 0.;
        assert!(!projection.contains_point(0., 5.));
    }

    #[test]
    fn paint_visibility_includes_text_plane_without_expanding_hit_region() {
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let outside = CssRect {
            x: -100.,
            y: 0.,
            width: 20.,
            height: 20.,
        };
        let mut projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            matrix: identity,
            text_matrix: None,
            text_clips: vec![],
            clips: vec![],
            box_model: BoxModel {
                border_box: outside,
                padding_box: outside,
                content_box: outside,
                margin: EdgeSizes::default(),
                border: EdgeSizes::default(),
                padding: EdgeSizes::default(),
            },
            text: vec![PaintTextFragment {
                flow_line: None,
                flow_rect: None,
                clusters: Vec::new(),
                text: "Overflow".into(),
                rect: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 20.,
                    height: 20.,
                },
                font_family: "sans-serif".into(),
                font_size: 16.,
                color: "black".into(),
                glyph_runs: vec![],
            }],
        };
        let viewport = ViewportState {
            width: 50.,
            height: 50.,
            device_pixel_ratio: 1.,
        };
        assert!(!projection.intersects_viewport(viewport));
        assert!(projection.paint_intersects_viewport(viewport));
        assert!(!projection.contains_point(10., 10.));
        let mut shifted = identity;
        shifted[12] = 100.;
        projection.text_matrix = Some(shifted);
        assert!(!projection.paint_intersects_viewport(viewport));
        projection.text[0].rect.x = -100.;
        assert!(projection.paint_intersects_viewport(viewport));
        shifted[15] = -1.;
        projection.text_matrix = Some(shifted);
        assert!(!projection.paint_intersects_viewport(viewport));
        projection.text_matrix = Some([0.; 16]);
        assert!(!projection.paint_intersects_viewport(viewport));
    }

    #[test]
    fn viewport_clipping_preserves_positive_side_of_perspective_horizon() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 2.,
            height: 2.,
        };
        let mut projection = PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: [
                1., 0., 0., -1., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            box_model: BoxModel {
                border_box: rect,
                padding_box: rect,
                content_box: rect,
                margin: EdgeSizes::default(),
                border: EdgeSizes::default(),
                padding: EdgeSizes::default(),
            },
            text: Vec::new(),
            clips: Vec::new(),
        };
        let viewport = ViewportState {
            width: 4.,
            height: 2.,
            device_pixel_ratio: 1.,
        };
        assert!(projection.intersects_viewport(viewport));
        projection.matrix[3] = 0.;
        projection.matrix[12] = 100.;
        assert!(!projection.intersects_viewport(viewport));
        projection.matrix[12] = 0.;
        projection.matrix[15] = -1.;
        assert!(!projection.intersects_viewport(viewport));
        projection.matrix[15] = 1.;
        projection.matrix[0] = 0.;
        assert!(!projection.intersects_viewport(viewport));
    }
}

/// Physical print settings; dimensions and margins are centimeters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintOptions {
    pub page_width_cm: f64,
    pub page_height_cm: f64,
    pub margin_top_cm: f64,
    pub margin_right_cm: f64,
    pub margin_bottom_cm: f64,
    pub margin_left_cm: f64,
    pub landscape: bool,
    pub scale: f64,
    pub background: bool,
    pub shrink_to_fit: bool,
    pub page_ranges: Vec<String>,
}
impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            page_width_cm: 21.59,
            page_height_cm: 27.94,
            margin_top_cm: 1.,
            margin_right_cm: 1.,
            margin_bottom_cm: 1.,
            margin_left_cm: 1.,
            landscape: false,
            scale: 1.,
            background: false,
            shrink_to_fit: true,
            page_ranges: Vec::new(),
        }
    }
}

/// Shared immutable resources and document identity for paginated print output.
/// `source.nodes` is empty; page-local fragments live in `pages`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintSnapshot {
    pub source: PaintSnapshot,
    pub pages: Vec<PrintPageSnapshot>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintPageSnapshot {
    pub viewport: ViewportState,
    pub nodes: Vec<PaintNodeSnapshot>,
}

#[cfg(test)]
mod generated_speech_metadata_tests {
    #[test]
    fn historical_json_preserves_legacy_joining_and_new_separation_round_trips() {
        let mut record: super::GeneratedContent = serde_json::from_str(
            r#"{"owner":1,"pseudo":"before","text":"This","alternative":null}"#,
        )
        .unwrap();
        assert!(!record.separates_text);
        record.separates_text = true;
        let decoded: super::GeneratedContent =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert!(decoded.separates_text);
        assert_eq!(decoded.text, "This");
        assert_eq!(decoded.alternative, None);
    }
}

#[cfg(test)]
mod paint_image_provenance_tests {
    use super::PaintImageResource;

    #[test]
    fn historical_image_payload_defaults_to_secure_resource_mode() {
        let resource: PaintImageResource = serde_json::from_value(serde_json::json!({
            "url": "https://example.test/image.svg",
            "media_type": "image/svg+xml",
            "bytes": "PHN2Zy8+"
        }))
        .unwrap();
        assert!(!resource.svg_document_snapshot);
        assert_eq!(resource.content_type, None);
        assert!(resource.url_aliases.is_empty());
        assert_eq!(resource.bytes, b"<svg/>");
    }

    #[test]
    fn native_image_provenance_and_raw_encoding_round_trip_without_inference() {
        let resource = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: Vec::new(),
            content_type: Some("image/svg+xml; charset=windows-1252".into()),
            url_aliases: vec!["https://example.test/original.svg".into()],
            url: "https://example.test/final.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: vec![0x3c, 0xe9, 0x3e],
        };
        let encoded = serde_json::to_value(&resource).unwrap();
        assert_eq!(
            serde_json::from_value::<PaintImageResource>(encoded.clone()).unwrap(),
            resource
        );
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!("true"),
            serde_json::json!(1),
        ] {
            let mut malformed = encoded.clone();
            malformed["svg_document_snapshot"] = invalid;
            assert!(serde_json::from_value::<PaintImageResource>(malformed).is_err());
        }
    }
}
