//! Deterministic, non-painting style and layout service.
//!
//! Canonical selectors and accessibility text transformations are V8-free and
//! shared with native semantic consumers. Browser style and layout execution
//! remains in the sandboxed page worker: it accepts a read-only snapshot of
//! Oryn's canonical DOM and returns computed style and geometry keyed by stable
//! node identifiers. This crate never owns DOM, JavaScript, networking, cookies,
//! or browser policy.

/// Inputs for taffy child placements.
pub(crate) struct ChildPlacementInputs<'a> {
    pub(crate) node: &'a LayoutNode,
    pub(crate) parent_style: &'a BTreeMap<String, String>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) width: f64,
    pub(crate) height: Option<f64>,
    pub(crate) viewport: ViewportState,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) text_engine: &'a mut TextEngine,
}
/// Inputs for project layout tree.
pub(crate) struct LayoutTreeProjectionInputs<'a> {
    pub(crate) input: &'a LayoutInput,
    pub(crate) by_id: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) geometry: &'a mut BTreeMap<u64, NodeGeometry>,
    pub(crate) text_fragments: &'a mut BTreeMap<u64, Vec<PaintTextFragment>>,
    pub(crate) resolved_values: &'a mut BTreeMap<u64, BTreeMap<String, String>>,
    pub(crate) paint_projections: &'a mut BTreeMap<u64, oryn_common::v4::PaintProjection>,
    pub(crate) scrollbar_gutters: &'a BTreeMap<u64, EdgeSizes>,
    pub(crate) viewport_scrollport: Option<CssRect>,
    pub(crate) diagnostics: &'a mut Vec<String>,
}
/// Inputs for shape paragraph.
pub(crate) struct ParagraphShapeInputs<'a> {
    pub(crate) text: &'a str,
    pub(crate) width: f64,
    pub(crate) font_size: f64,
    pub(crate) style: &'a BTreeMap<String, String>,
    pub(crate) viewport: ViewportState,
    pub(crate) ranges: &'a [StyledTextRange<'a>],
    pub(crate) first_paragraph: bool,
}
mod anchor_positioning;
mod backdrop;
mod columns;
mod control_state;
mod counter_tree;
pub mod cssom;
pub mod directionality;
mod effect_resources;
mod generated;
mod generated_counter_styles;
mod list_markers;
pub mod media;
mod object_images;
mod permission_controls;
mod permission_prompt;
mod positioning;
pub mod print;
mod scroll_geometry;
mod scrollbars;
mod select_picker;
mod shadow_tree;
#[cfg(test)]
mod shadow_tree_tests;
mod sticky;
mod svg_clip;
mod svg_clip_external;
mod svg_clip_text;
pub mod svg_fonts;
mod svg_geometry;
pub mod svg_instances;
mod tables;
mod temporal_picker;
mod text_decoration;
mod text_emphasis;
mod text_features;
mod text_pseudos;
mod text_source;
mod transforms;
mod vertical_text;
mod web_font_faces;
mod writing_mode;
pub use control_state::control_paint_state;
mod float_lines;
mod inline_formatting;
mod margin_collapse;
mod ruby;
mod text_transform;
pub use text_transform::accessibility_case_fragment;
mod greek_upper;

#[cfg(test)]
use std::ptr::NonNull;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use blitz_dom::{
    Attribute as BlitzAttribute, BaseDocument, DocumentConfig, LocalName, Namespace, QualName,
};
use blitz_traits::shell::{ColorScheme as BlitzColorScheme, Viewport as BlitzViewport};
use cssparser::{Parser, ParserInput, ToCss as _, parse_nth};
use oryn_common::v4::{
    BoxModel, CssRect, EdgeSizes, GeometryAvailability, NodeGeometry, PaintTextFragment,
    ScrollState, ViewportState,
};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, Layout, LayoutContext, StyleProperty,
};
#[cfg(test)]
use selectors::{
    Element as SelectorElement, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    matching::{
        MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, SelectorCaches,
        matches_selector,
    },
    parser::Selector,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skrifa::FontRef;
#[cfg(test)]
use style::media_queries::MediaList;
#[cfg(test)]
use style::{
    CaseSensitivityExt,
    properties::parse_style_attribute,
    selector_parser::{NonTSPseudoClass, SelectorImpl},
    values::{AtomIdent, AtomString},
};
#[cfg(test)]
use style::{
    context::QuirksMode,
    servo_arc::Arc as ServoArc,
    shared_lock::SharedRwLock,
    stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
};
use style::{
    properties::PropertyId,
    selector_parser::PseudoElement,
    stylesheets::{Origin, UrlExtraData},
};
#[cfg(test)]
use style::{
    shared_lock::SharedRwLockReadGuard,
    stylesheets::{CssRuleType, CssRules},
};
use style_dom::ElementState;
use style_traits::ToCss;
use taffy::{
    prelude::{
        AvailableSpace, Display, FlexDirection, GridAutoFlow, Line, Size as TaffySize,
        Style as TaffyStyle, TaffyTree, auto, fr, length as taffy_length,
    },
    style::{Clear, Float},
};

pub mod dom_queries;

pub const ENGINE_CONTRACT_VERSION: u16 = 1;
pub const DEFAULT_FONT_FAMILY: &str = "Oryn Noto Sans";
pub const DEFAULT_FONT_SHA256: &str =
    "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5";
pub const DEFAULT_FONT_BYTES: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");
pub const STYLE_ENGINE_VERSION: &str = oryn_common::STYLE_ENGINE_VERSION;
pub const BOX_ENGINE_VERSION: &str = "taffy-0.14";
pub const INLINE_ENGINE_VERSION: &str = "parley-0.11.1";

/// Parse and serialize a selector through Stylo's author-stylesheet parser.
/// CSSOM uses this to reject malformed selectors and expose canonical
/// `selectorText` without maintaining a second selector grammar.
pub fn canonicalize_selector(selector: &str) -> Result<String, String> {
    let url = UrlExtraData::from(url::Url::parse("about:blank").expect("constant stylesheet URL"));
    let selectors =
        style::selector_parser::SelectorParser::parse_author_origin_no_namespace(selector, &url)
            .map_err(|error| format!("invalid selector: {error:?}"))?;
    canonicalize_nth_expressions(&selectors.to_css_string())
}

fn canonicalize_nth_expressions(selector: &str) -> Result<String, String> {
    const FUNCTIONS: [&str; 4] = [
        ":nth-child(",
        ":nth-last-child(",
        ":nth-of-type(",
        ":nth-last-of-type(",
    ];
    let mut output = selector.to_string();
    let mut search_from = 0;
    loop {
        let lowercase = output.to_ascii_lowercase();
        let Some((start, function)) = FUNCTIONS
            .iter()
            .filter_map(|function| {
                lowercase[search_from..]
                    .find(function)
                    .map(|offset| (search_from + offset, *function))
            })
            .min_by_key(|(start, _)| *start)
        else {
            break;
        };
        let content_start = start + function.len();
        let Some(relative_end) = output[content_start..].find(')') else {
            return Err("unterminated An+B selector".into());
        };
        let end = content_start + relative_end;
        let raw = output[content_start..end].trim();
        // Selectors Level 4 permits an optional `of <selector-list>` tail.
        // Stylo already owns that grammar; normalize only the standalone
        // An+B token sequence here.
        if raw.to_ascii_lowercase().contains(" of ") {
            search_from = end + 1;
            continue;
        }
        let normalized = if raw.eq_ignore_ascii_case("odd") || raw.eq_ignore_ascii_case("even") {
            raw.to_ascii_lowercase()
        } else {
            let mut input = ParserInput::new(raw);
            let mut parser = Parser::new(&mut input);
            let (a, b) = parse_nth(&mut parser).map_err(|_| "invalid An+B selector")?;
            parser
                .expect_exhausted()
                .map_err(|_| "invalid trailing An+B tokens")?;
            let mut value = match a {
                0 => String::new(),
                1 => "n".into(),
                -1 => "-n".into(),
                coefficient => format!("{coefficient}n"),
            };
            if a == 0 || b != 0 {
                if a != 0 && b > 0 {
                    value.push('+');
                }
                value.push_str(&b.to_string());
            }
            value
        };
        output.replace_range(content_start..end, &normalized);
        search_from = content_start + normalized.len() + 1;
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutLimits {
    pub max_nodes: usize,
    pub max_font_bytes: usize,
    pub max_page_font_bytes: usize,
    pub max_viewport_css_px: f64,
}

impl Default for LayoutLimits {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_font_bytes: 8 * 1024 * 1024,
            max_page_font_bytes: 32 * 1024 * 1024,
            max_viewport_css_px: 16_384.0,
        }
    }
}

fn default_document_base_url() -> String {
    "https://oryn.invalid/".into()
}

fn default_element_namespace() -> String {
    "http://www.w3.org/1999/xhtml".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNode {
    /// Native document/shadow tree root; never derived from authored attributes.
    #[serde(default)]
    pub tree_scope: Option<u64>,
    #[serde(default)]
    pub document_url: Option<String>,
    pub id: u64,
    pub parent: Option<u64>,
    #[serde(default)]
    pub children: Vec<u64>,
    pub tag: String,
    /// Namespace of the native element; legacy layout inputs describe HTML.
    #[serde(default = "default_element_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Namespace-bearing attributes, kept separate from the null-namespace
    /// compatibility projection so equal qualified names cannot overwrite.
    #[serde(default)]
    pub namespaced_attributes: Vec<LayoutAttribute>,
    /// Live native input state, distinct from the default content attribute.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Native constraint validity; None means no validation pseudo-class.
    #[serde(default)]
    pub validity: Option<bool>,
    /// Native disabled state for eligible controls, including custom elements.
    /// None leaves legacy attribute-based layout inputs to the mirror engine.
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default)]
    pub indeterminate: bool,
    #[serde(default)]
    pub intrinsic_image_size: Option<(u32, u32)>,
    #[serde(default)]
    pub image_intrinsics: Option<oryn_dom::ImageIntrinsics>,
    /// Native hosted SVG root, never inferred from object fallback children.
    #[serde(default)]
    pub image_document_root: Option<u64>,
    #[serde(default)]
    pub document_replaced: bool,
    #[serde(default)]
    pub text: String,
    /// Ordered text/element children. None identifies older element-only inputs.
    /// Text node identities survive the boundary for future range geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<LayoutContent>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayoutContent {
    Text {
        node_id: u64,
        text: String,
    },
    /// Text promoted through a boxless wrapper retains its original style owner.
    StyledText {
        owner: u64,
        node_id: Option<u64>,
        text: String,
    },
    Element(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutAttribute {
    pub namespace: String,
    pub local_name: String,
    pub prefix: Option<String>,
    pub value: String,
}

/// The native tree supplies scope ownership; Stylo performs scope matching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StylesheetScopeRoot {
    Element(u64),
    DocumentElement,
    Constructed,
}

/// Native shadow identities are independent of implicit CSS @scope roots.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShadowTrees {
    pub roots: BTreeMap<u64, ShadowTree>,
    pub node_scopes: BTreeMap<u64, u64>,
    pub stylesheet_scopes: BTreeMap<usize, u64>,
    pub assigned_slots: BTreeMap<u64, u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowTree {
    pub host: u64,
    pub content: Vec<LayoutContent>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSelectorState {
    #[serde(default)]
    pub interest_source: bool,
    #[serde(default)]
    pub media_state: u8,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub interest_target: bool,
    #[serde(default)]
    pub interest_invoker: bool,
    #[serde(default)]
    pub inline_style_blocked: bool,
    #[serde(default)]
    pub permission_granted: bool,
    #[serde(default)]
    pub user_validity: Option<bool>,
    #[serde(default)]
    pub default_control: bool,
    #[serde(default)]
    pub in_range: Option<bool>,
    #[serde(default)]
    pub required: Option<bool>,
    pub defined: bool,
    pub placeholder_shown: bool,
    pub read_write: bool,
    pub target: bool,
}

/// Validate using the exact native registration algorithm used by layout.
pub fn validate_custom_property(
    registration: &oryn_dom::RegisteredCustomProperty,
) -> Result<(), String> {
    let mut document = BaseDocument::new(DocumentConfig::default());
    document.register_custom_property(
        &registration.name,
        &registration.syntax,
        registration.inherits,
        registration.initial_value.as_deref(),
        &registration.base_url,
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutInput {
    #[serde(default)]
    pub svg_clip_resources: Vec<oryn_common::v4::PaintImageResource>,
    /// Resolved native legacy image hints; raw attributes remain selector-visible.
    #[serde(default)]
    pub legacy_background_urls: BTreeMap<u64, String>,
    #[serde(default)]
    pub registered_custom_properties: Vec<oryn_dom::RegisteredCustomProperty>,
    #[serde(default)]
    pub document_language: Option<String>,
    #[serde(default)]
    pub selector_states: BTreeMap<u64, LayoutSelectorState>,
    #[serde(default)]
    pub shadow_trees: ShadowTrees,
    #[serde(default = "default_document_base_url")]
    pub document_base_url: String,
    #[serde(default)]
    pub image_intrinsics: BTreeMap<String, (u32, u32)>,
    #[serde(default)]
    pub control_states: BTreeMap<u64, oryn_common::v4::PaintControl>,
    #[serde(default)]
    pub temporal_pickers: BTreeMap<u64, oryn_dom::TemporalPicker>,
    #[serde(default)]
    pub permission_prompts: BTreeMap<u64, oryn_dom::PermissionPrompt>,
    /// Ordered document-owned state; never derived from author attributes.
    #[serde(default)]
    pub top_layer: Vec<oryn_common::v4::TopLayerEntry>,
    /// Native classic scrollbar thickness; zero selects overlay scrollbars.
    #[serde(default)]
    pub scrollbar_width: f64,
    /// Native element scroll offsets in CSS pixels, separate from viewport scroll.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub element_scrolls: BTreeMap<u64, (f64, f64)>,
    /// Rendered flat-tree content, supplied independently of DOM selector ancestry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_tree: Option<BTreeMap<u64, Vec<LayoutContent>>>,
    /// Original native ancestry retained when formatting reparents or omits boxes.
    /// Kept outside computed CSS so identical styles remain shareable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchor_parents: BTreeMap<u64, Option<u64>>,
    pub nodes: Vec<LayoutNode>,
    pub stylesheets: Vec<String>,
    /// CSSOM media text keyed by stylesheet position in cascade order.
    #[serde(default)]
    pub stylesheet_media: BTreeMap<usize, String>,
    /// Implicit scope roots keyed by stylesheet position in cascade order.
    #[serde(default)]
    pub stylesheet_scope_roots: BTreeMap<usize, StylesheetScopeRoot>,
    /// Native base URL per stylesheet, retained independently of DOM base changes.
    #[serde(default)]
    pub stylesheet_base_urls: BTreeMap<usize, String>,
    /// Rust-owned `ElementInternals.states` keyed by the stable DOM node id.
    /// These are selector state, not serialized HTML attributes.
    #[serde(default)]
    pub custom_states: BTreeMap<u64, BTreeSet<String>>,
    /// Native focused element, projected into the private Stylo mirror.
    #[serde(default)]
    pub focused_node: Option<u64>,
    /// Native focus indicator state, independent of whether focus exists.
    #[serde(default)]
    pub focus_visible: bool,
    pub viewport: ViewportState,
    pub scroll: ScrollState,
    #[serde(default)]
    pub color_scheme: ColorScheme,
    #[serde(default)]
    pub reduced_motion: bool,
    #[serde(default)]
    pub web_fonts: Vec<WebFont>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebFont {
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub order: u64,
    #[serde(default)]
    pub descriptors: String,
    pub family: String,
    pub source_url: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedFont {
    pub family: String,
    pub source_url: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

/// Immutable style projections can be shared across equal elements and snapshots.
pub type ComputedStyle = Arc<BTreeMap<String, String>>;
pub type ComputedStyles = BTreeMap<u64, ComputedStyle>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutOutput {
    #[serde(default)]
    pub font_demands: BTreeMap<String, BTreeSet<u32>>,
    #[serde(default)]
    pub resolved_text: Vec<oryn_common::v4::ResolvedTextRun>,
    #[serde(default)]
    pub css_image_requests: BTreeSet<String>,
    #[serde(default)]
    pub generated_image_urls: BTreeMap<u64, String>,
    #[serde(default)]
    pub backdrops: BTreeMap<u64, oryn_common::v4::PaintNodeSnapshot>,
    /// Formatting ancestry only; never changes DOM or accessibility ownership.
    #[serde(default)]
    pub paint_parent_overrides: BTreeMap<u64, Option<u64>>,
    #[serde(default)]
    pub generated_nodes: Vec<oryn_common::v4::PaintNodeSnapshot>,
    #[serde(default)]
    pub generated_content: Vec<oryn_common::v4::GeneratedContent>,
    #[serde(default)]
    pub control_states: BTreeMap<u64, oryn_common::v4::PaintControl>,
    /// Native pseudo styles are distinct from originating-element CSSOM values.
    #[serde(default)]
    pub pseudo_styles: BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_scroll: Option<oryn_common::v4::PaintViewportScroll>,
    /// CSSOM used values measured before transforms, including invisible boxes.
    /// Properties without an applicable layout box retain their computed value.
    #[serde(default)]
    pub resolved_values: BTreeMap<u64, BTreeMap<String, String>>,
    pub computed_styles: ComputedStyles,
    /// Elements/text whose computed whitespace includes author-specified values.
    /// Internal editing metadata, deliberately separate from CSSOM properties.
    #[serde(default)]
    pub author_whitespace: BTreeSet<u64>,
    pub geometry: BTreeMap<u64, NodeGeometry>,
    /// Untransformed border-box dimensions, including visibility-hidden boxes.
    #[serde(default)]
    pub offset_dimensions: BTreeMap<u64, (f64, f64)>,
    #[serde(default)]
    pub text_fragments: BTreeMap<u64, Vec<PaintTextFragment>>,
    #[serde(default)]
    pub paint_fonts: BTreeMap<String, oryn_common::v4::PaintFontResource>,
    #[serde(default)]
    pub text_clusters: BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
    #[serde(default)]
    pub caret_stops: Vec<oryn_common::v4::CaretStopGeometry>,
    #[serde(default)]
    pub paint_projections: BTreeMap<u64, oryn_common::v4::PaintProjection>,
    pub scroll: ScrollState,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub loaded_fonts: Vec<LoadedFont>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum LayoutError {
    #[error("layout snapshot contains {actual} nodes, limit is {limit}")]
    NodeLimit { actual: usize, limit: usize },
    #[error("viewport dimension {value} is outside 0..={limit} CSS px")]
    ViewportLimit { value: f64, limit: f64 },
    #[error("Stylo computed-value traversal failed: {0}")]
    StyleTraversal(String),
    #[error("invalid rendered layout tree: {0}")]
    InvalidRenderedTree(String),
    #[error("invalid element scroll offset for node {0}")]
    InvalidElementScroll(u64),
}

#[derive(Debug, Clone)]
#[cfg(test)]
#[allow(dead_code)]
struct Rule {
    selector: Selector<SelectorImpl>,
    declarations: BTreeMap<String, (String, bool)>,
    specificity: u32,
    order: usize,
}

pub use blitz_dom::{
    NativeAnimationEffect, NativeAnimationKeyframe, NativeCssAnimation, NativeCssAnimationControl,
};
pub fn animation_pseudo_name(pseudo: &style::selector_parser::PseudoElement) -> String {
    pseudo.to_css_string()
}
pub fn animation_pseudo(name: &str) -> Option<style::selector_parser::PseudoElement> {
    use selectors::parser::Parser;
    let url = style::stylesheets::UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let parser = style::selector_parser::SelectorParser {
        stylesheet_origin: style::stylesheets::Origin::Author,
        namespaces: &Default::default(),
        url_data: &url,
        for_supports_rule: false,
    };
    let mut input = cssparser::ParserInput::new("");
    let input = cssparser::Parser::new(&mut input);
    parser
        .parse_pseudo_element(
            input.current_source_location(),
            name.trim_start_matches(':').into(),
        )
        .ok()
}

#[derive(Debug, Default)]
pub struct StyleEngine {
    effects: Vec<blitz_dom::NativeAnimationEffect>,
    samples: BTreeMap<u64, BTreeMap<String, String>>,
    continuity: blitz_dom::StyleContinuity,
    instance_sample: Option<svg_instances::StyleSample>,
    static_layout: Option<(LayoutInput, LayoutLimits, LayoutOutput)>,
}

impl StyleEngine {
    /// Snapshot of the last successful layout's native animation session.
    pub fn instance_style_sample(&self) -> Option<svg_instances::StyleSample> {
        self.instance_sample.clone()
    }
    pub fn compute(
        &mut self,
        input: &LayoutInput,
        limits: &LayoutLimits,
    ) -> Result<LayoutOutput, LayoutError> {
        self.compute_at(input, limits, 0.0)
    }
    pub fn css_animations(&self, now: f64) -> Vec<NativeCssAnimation> {
        self.continuity.css_animations(now)
    }
    pub fn control_css_animation(
        &mut self,
        id: u64,
        control: NativeCssAnimationControl,
        time: Option<f64>,
        now: f64,
    ) -> Result<(), String> {
        self.static_layout = None;
        self.continuity
            .control_css_animation(id, control, time, now)
    }
    pub fn css_animation_style_values(&self, id: u64, now: f64) -> BTreeMap<String, String> {
        self.continuity.css_animation_style_values(id, now)
    }
    pub fn set_animation_effects(
        &mut self,
        effects: Vec<blitz_dom::NativeAnimationEffect>,
    ) -> bool {
        if self.effects == effects {
            return false;
        }
        self.static_layout = None;
        self.effects = effects;
        true
    }
    pub fn animation_style_values(&self, id: u64) -> BTreeMap<String, String> {
        self.samples.get(&id).cloned().unwrap_or_default()
    }
    pub fn has_active_animations(&self) -> bool {
        self.continuity.active
    }
    /// Sample against the embedding document's monotonic timeline, in seconds.
    pub fn compute_at(
        &mut self,
        input: &LayoutInput,
        limits: &LayoutLimits,
        now: f64,
    ) -> Result<LayoutOutput, LayoutError> {
        validate(input, limits)?;
        // Forced getters and actor rendering can request the same static scene
        // several times in one turn. Reuse only a complete successful result;
        // animated sessions retain their native timeline sampling below.
        if let Some((previous, previous_limits, output)) = &self.static_layout
            && previous == input
            && previous_limits == limits
        {
            self.instance_sample = Some(svg_instances::StyleSample {
                now,
                continuity: self.continuity.clone(),
                effects: self.effects.clone(),
            });
            return Ok(output.clone());
        }
        // A failed recomputation must not leave a result from an older native
        // style session eligible for reuse.
        self.static_layout = None;
        let (computed_styles, author_whitespace, pseudo_styles) = stylo_computed_values_timed(
            input,
            false,
            now,
            Some(&mut self.continuity),
            &self.effects,
            Some(&mut self.samples),
        )?;
        // Preparation owns copies only for controls or replaced content. Release
        // those intermediates before the materialized tree enters layout.
        let (generated, object_hidden) = {
            let native_input = permission_controls::project(input);
            let (native_input, hidden) =
                object_images::project_styled(&native_input, &computed_styles);
            (
                generated::materialize(&native_input, computed_styles, &pseudo_styles)?,
                hidden,
            )
        };
        let mut output = compute_with_styles(&generated.input, limits, generated.styles.clone())?;
        output.author_whitespace = author_whitespace;
        output.pseudo_styles = pseudo_styles;
        select_picker::append(&generated.input, limits, &mut output)?;
        temporal_picker::append(&generated.input, limits, &mut output)?;
        permission_prompt::append(&generated.input, limits, &mut output)?;
        generated::finish(&generated, &mut output);
        backdrop::append(&generated.input, limits, &mut output)?;
        object_images::finish(&mut output, &object_hidden);
        self.instance_sample = Some(svg_instances::StyleSample {
            now,
            continuity: self.continuity.clone(),
            effects: self.effects.clone(),
        });
        if !self.continuity.active
            && self.effects.is_empty()
            && self.continuity.css_animations(now).is_empty()
        {
            self.static_layout = Some((input.clone(), limits.clone(), output.clone()));
        }
        Ok(output)
    }
}

#[derive(Clone, Copy)]
#[cfg(test)]
#[allow(dead_code)]
struct LayoutElement<'a> {
    node: &'a LayoutNode,
    by_id: &'a BTreeMap<u64, &'a LayoutNode>,
}

#[cfg(test)]
impl std::fmt::Debug for LayoutElement<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LayoutElement")
            .field("id", &self.node.id)
            .field("tag", &self.node.tag)
            .finish()
    }
}

#[cfg(test)]
impl PartialEq for LayoutElement<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node.id == other.node.id
    }
}

#[allow(dead_code)]
#[cfg(test)]
impl LayoutElement<'_> {
    fn element(&self, id: u64) -> Option<Self> {
        self.by_id
            .get(&id)
            .copied()
            .filter(|node| node.tag != "#text")
            .map(|node| Self {
                node,
                by_id: self.by_id,
            })
    }

    fn state(&self) -> ElementState {
        let mut state = ElementState::empty();
        let disabled = self.node.disabled.or_else(|| {
            (self.node.namespace == "http://www.w3.org/1999/xhtml"
                && matches!(
                    self.node.tag.as_str(),
                    "button" | "input" | "select" | "textarea" | "fieldset" | "optgroup" | "option"
                ))
            .then(|| self.node.attributes.contains_key("disabled"))
        });
        state.set(ElementState::ENABLED, disabled == Some(false));
        state.set(ElementState::DISABLED, disabled == Some(true));
        if let Some(valid) = self.node.validity {
            state.insert(if valid {
                ElementState::VALID
            } else {
                ElementState::INVALID
            });
        }
        if self
            .node
            .checked
            .unwrap_or_else(|| self.node.attributes.contains_key("checked"))
        {
            state.insert(ElementState::CHECKED);
        }
        if self.node.attributes.contains_key("required") {
            state.insert(ElementState::REQUIRED);
        }
        if self.node.attributes.contains_key("readonly") {
            state.insert(ElementState::READWRITE);
        }
        if self.node.attributes.contains_key("data-oryn-focused") {
            state.insert(ElementState::FOCUS);
        }
        if self.node.attributes.contains_key("data-oryn-hover") {
            state.insert(ElementState::HOVER);
        }
        if self.node.tag == "a" && self.node.attributes.contains_key("href") {
            state.insert(ElementState::UNVISITED);
        }
        state
    }
}

#[cfg(test)]
impl SelectorElement for LayoutElement<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        let pointer = NonNull::new((self.node.id as usize).wrapping_add(1) as *mut ())
            .expect("non-zero synthetic element identity");
        OpaqueElement::from_non_null_ptr(pointer)
    }

    fn parent_element(&self) -> Option<Self> {
        self.node.parent.and_then(|parent| self.element(parent))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }
    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }
    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let parent = self
            .node
            .parent
            .and_then(|parent| self.by_id.get(&parent).copied())?;
        let index = parent
            .children
            .iter()
            .position(|child| *child == self.node.id)?;
        parent.children[..index]
            .iter()
            .rev()
            .find_map(|child| self.element(*child))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let parent = self
            .node
            .parent
            .and_then(|parent| self.by_id.get(&parent).copied())?;
        let index = parent
            .children
            .iter()
            .position(|child| *child == self.node.id)?;
        parent.children[index.saturating_add(1)..]
            .iter()
            .find_map(|child| self.element(*child))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node
            .children
            .iter()
            .find_map(|child| self.element(*child))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(
        &self,
        local_name: &<SelectorImpl as selectors::SelectorImpl>::BorrowedLocalName,
    ) -> bool {
        local_name.as_ref() == self.node.tag
    }

    fn has_namespace(
        &self,
        ns: &<SelectorImpl as selectors::SelectorImpl>::BorrowedNamespaceUrl,
    ) -> bool {
        ns.as_ref() == "http://www.w3.org/1999/xhtml"
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.node.tag == other.node.tag
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<SelectorImpl as selectors::SelectorImpl>::NamespaceUrl>,
        local_name: &<SelectorImpl as selectors::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        let null_namespace = matches!(ns, NamespaceConstraint::Any)
            || matches!(ns, NamespaceConstraint::Specific(wanted) if wanted.0.as_ref().is_empty());
        (null_namespace
            && self
                .node
                .attributes
                .get(local_name.0.as_ref())
                .is_some_and(|value| operation.eval_str(value)))
            || self.node.namespaced_attributes.iter().any(|entry| {
                entry.local_name == local_name.0.as_ref()
                    && match ns {
                        NamespaceConstraint::Any => true,
                        NamespaceConstraint::Specific(wanted) => {
                            wanted.0.as_ref() == entry.namespace
                        }
                    }
                    && operation.eval_str(&entry.value)
            })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo: &NonTSPseudoClass,
        _context: &mut MatchingContext<SelectorImpl>,
    ) -> bool {
        let state = self.state();
        match pseudo {
            NonTSPseudoClass::Active => state.contains(ElementState::ACTIVE),
            NonTSPseudoClass::AnyLink => state.intersects(ElementState::VISITED_OR_UNVISITED),
            NonTSPseudoClass::Checked => state.contains(ElementState::CHECKED),
            NonTSPseudoClass::Valid => state.contains(ElementState::VALID),
            NonTSPseudoClass::Invalid => state.contains(ElementState::INVALID),
            NonTSPseudoClass::Disabled => state.contains(ElementState::DISABLED),
            NonTSPseudoClass::Enabled => state.contains(ElementState::ENABLED),
            NonTSPseudoClass::Focus => state.contains(ElementState::FOCUS),
            NonTSPseudoClass::Hover => state.contains(ElementState::HOVER),
            NonTSPseudoClass::Link => state.contains(ElementState::UNVISITED),
            NonTSPseudoClass::ReadOnly => self.node.attributes.contains_key("readonly"),
            NonTSPseudoClass::ReadWrite => !self.node.attributes.contains_key("readonly"),
            NonTSPseudoClass::Required => self.node.attributes.contains_key("required"),
            NonTSPseudoClass::Optional => !self.node.attributes.contains_key("required"),
            NonTSPseudoClass::Open => self.node.attributes.contains_key("open"),
            _ => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pseudo: &PseudoElement,
        _context: &mut MatchingContext<SelectorImpl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: selectors::matching::ElementSelectorFlags) {}
    fn is_link(&self) -> bool {
        self.node.tag == "a" && self.node.attributes.contains_key("href")
    }
    fn is_html_slot_element(&self) -> bool {
        self.node.tag == "slot"
    }

    fn has_id(&self, id: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        let value = self
            .node
            .attributes
            .get("id")
            .map(|value| AtomIdent::from(value.as_str()));
        self.node.attributes.get("id").is_some_and(|_| {
            value
                .as_ref()
                .is_some_and(|value| case_sensitivity.eq_atom(&value.0, &id.0))
        })
    }

    fn has_class(&self, name: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.node.attributes.get("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .map(AtomIdent::from)
                .any(|class| case_sensitivity.eq_atom(&class.0, &name.0))
        })
    }

    fn has_custom_state(&self, _name: &AtomIdent) -> bool {
        false
    }
    fn imported_part(&self, _name: &AtomIdent) -> Option<AtomIdent> {
        None
    }
    fn is_part(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.node.children.iter().all(|child| {
            self.by_id
                .get(child)
                .is_none_or(|child| child.tag == "#text" && child.text.is_empty())
        })
    }

    fn is_root(&self) -> bool {
        self.parent_element().is_none()
    }

    fn add_element_unique_hashes(&self, filter: &mut selectors::bloom::BloomFilter) -> bool {
        let _ = filter;
        false
    }
}

#[derive(Debug, Clone, Copy)]
struct FlowBox {
    x: f64,
    y: f64,
    width: f64,
    height: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct TextBrush {
    pseudo: u8,
    families: String,
    background: Option<[u8; 4]>,
    owner: u64,
    decorations: Vec<oryn_common::v4::PaintTextDecoration>,
    color: [u8; 4],
    synthesize_weight: bool,
    synthesize_style: bool,
}
impl Default for TextBrush {
    fn default() -> Self {
        Self {
            pseudo: 0,
            families: DEFAULT_FONT_FAMILY.into(),
            background: None,
            owner: 0,
            decorations: Vec::new(),
            color: [0, 0, 0, 255],
            synthesize_weight: true,
            synthesize_style: true,
        }
    }
}

// Text ownership belongs to the span, not its shareable CSS declaration map.
type StyledTextRange<'a> = (
    std::ops::Range<usize>,
    &'a BTreeMap<String, String>,
    Option<fontique::Language>,
    u64,
);

struct TextEngine {
    font_demands: BTreeMap<u64, BTreeMap<String, BTreeSet<u32>>>,
    font_demand_bytes: usize,
    font_demand_error: Option<LayoutError>,
    resolved_text: BTreeMap<u64, Vec<oryn_common::v4::ResolvedTextRun>>,
    select_sizes: BTreeMap<u64, (f64, f64)>,
    scrollbar_gutters: BTreeMap<u64, EdgeSizes>,
    fonts: FontContext,
    web_faces: Vec<web_font_faces::Face>,
    layout: LayoutContext<TextBrush>,
    paint_fonts: BTreeMap<String, oryn_common::v4::PaintFontResource>,
    font_keys: BTreeMap<(u64, u32), String>,
    source_geometry: BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
    languages: BTreeMap<u64, Option<fontique::Language>>,
    decoration_styles: ComputedStyles,
    inline_sizes: BTreeMap<u64, (f64, f64, parley::InlineBoxKind)>,
    inline_extents: BTreeMap<u64, f64>,
    inline_floats: BTreeMap<u64, float_lines::FloatSpec>,
    float_context: Option<taffy::compute::FloatContext>,
    float_block_offset: f64,
    float_context_width: f64,
    float_vertical: bool,
    float_fragment: Option<(f64, f64)>,
    absorbed_margins: BTreeMap<u64, [f64; 2]>,
    clearance_nodes: BTreeSet<u64>,
    absorbed_vertical_margins: BTreeMap<u64, [f64; 2]>,
    bfc_available_width: Option<(u64, f64)>,
    float_descendants: BTreeMap<u64, bool>,
    float_inline_insets: [f64; 2],
    ruby_baselines: BTreeMap<u64, f64>,
    first_line_baselines: BTreeMap<u64, f64>,
    column_runs: BTreeMap<u64, Vec<columns::ColumnRun>>,
    column_float_heights: BTreeMap<u64, f64>,
    column_float_trials: BTreeMap<u64, f64>,
    column_float_extents: BTreeMap<u64, f64>,
    column_balance_work: usize,
    column_float_section_heights: BTreeMap<u64, Vec<f64>>,
    column_float_section_counts: BTreeMap<u64, Vec<usize>>,
    column_float_section_sizes: BTreeMap<u64, Vec<f64>>,
    column_float_section_index: BTreeMap<u64, usize>,
    column_context: Option<(u64, f64)>,
    active_inline_boxes: Vec<parley::InlineBox>,
    measuring_nodes: BTreeSet<u64>,
    out_of_flow: Vec<(u64, FlowBox)>,
    table_rows: BTreeMap<u64, tables::RowPlan>,
    table_borders: BTreeMap<u64, EdgeSizes>,
    collapsed_tables: Vec<tables::CollapsedTable>,
    consumed_table_rows: BTreeSet<u64>,
    break_boxes: BTreeMap<u64, NativeBreak>,
    table_end_spacing: BTreeMap<u64, f64>,
    table_widths: BTreeMap<u64, f64>,
    table_block_sizes: BTreeMap<u64, f64>,
    table_baseline_offsets: BTreeMap<u64, f64>,
    active_out_of_flow: Option<u64>,
}

#[derive(Debug, Clone)]
struct NativeBreak {
    node: u64,
    layout_node: u64,
    line: usize,
    before: CssRect,
    after: Option<CssRect>,
    rtl: bool,
}
#[derive(Debug, Clone)]
struct RunProvenance {
    families: String,
    text_override: Option<String>,
    owner: u64,
    bytes: std::ops::Range<usize>,
    glyph_ranges: Vec<std::ops::Range<usize>>,
}
#[derive(Debug, Clone)]
struct InlinePlacement {
    node: u64,
    rect: CssRect,
    line: usize,
}
#[derive(Debug, Default)]
struct TextLayout {
    resolved_text: Vec<oryn_common::v4::ResolvedTextRun>,
    /// Native line baseline relative to its block start, including atomic-only lines.
    baselines: Vec<f64>,
    line_heights: Vec<f64>,
    provenance: Vec<Vec<RunProvenance>>,
    inline_boxes: Vec<InlinePlacement>,
    breaks: Vec<NativeBreak>,
    lines: Vec<(String, f64, f64, f64, f64)>,
    line_ranges: Vec<std::ops::Range<usize>>,
    glyph_runs: Vec<Vec<oryn_common::v4::PaintGlyphRun>>,
    clusters: Vec<text_source::TextCluster>,
    source_clusters: Vec<text_source::SourceCluster>,
    width: f64,
    height: f64,
}

impl TextLayout {
    fn flow_rect(&self, index: usize, mode: &str) -> CssRect {
        let line = &self.lines[index];
        let advance = self.line_heights.get(index).copied().unwrap_or(line.2);
        if writing_mode::Axes::new(mode, false).vertical {
            CssRect {
                x: line.4
                    + if matches!(mode, "vertical-rl" | "sideways-rl") {
                        line.1 - advance
                    } else {
                        0.
                    },
                y: line.3,
                width: advance,
                height: line.2,
            }
        } else {
            CssRect {
                x: line.4,
                y: line.3,
                width: line.1,
                height: advance,
            }
        }
    }
    fn paint_runs(&self, line: usize) -> Vec<oryn_common::v4::PaintGlyphRun> {
        let Some(range) = self.line_ranges.get(line) else {
            return Vec::new();
        };
        let mut runs = self.glyph_runs.get(line).cloned().unwrap_or_default();
        for (run, source) in runs
            .iter_mut()
            .zip(self.provenance.get(line).into_iter().flatten())
        {
            for (glyph, bytes) in run.glyphs.iter_mut().zip(&source.glyph_ranges) {
                glyph.source = (bytes.start >= range.start
                    && bytes.end <= range.end
                    && bytes.start < bytes.end)
                    .then(|| bytes.start - range.start..bytes.end - range.start);
            }
        }
        runs
    }
    fn paint_clusters(
        &self,
        line: usize,
        x: f64,
        y: f64,
    ) -> Vec<oryn_common::v4::PaintTextCluster> {
        let Some(range) = self.line_ranges.get(line) else {
            return Vec::new();
        };
        self.clusters
            .iter()
            .filter(|cluster| {
                cluster.line == line
                    && cluster.bytes.start >= range.start
                    && cluster.bytes.end <= range.end
            })
            .map(|cluster| oryn_common::v4::PaintTextCluster {
                start: cluster.bytes.start - range.start,
                end: cluster.bytes.end - range.start,
                rect: CssRect {
                    x: x + cluster.rect.x,
                    y: y + cluster.rect.y,
                    ..cluster.rect
                },
                rtl: cluster.rtl,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct ChildPlacement {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

type PreparedFonts = (
    Vec<(String, Vec<u8>, String, bool, bool)>,
    Vec<LoadedFont>,
    Vec<String>,
);

fn effective_languages(input: &LayoutInput) -> BTreeMap<u64, Option<String>> {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut resolved = BTreeMap::<u64, Option<String>>::new();
    for node in &input.nodes {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = Some(node.id);
        while let Some(id) = current {
            if resolved.contains_key(&id) || !seen.insert(id) {
                break;
            }
            let Some(node) = nodes.get(&id) else {
                break;
            };
            path.push(id);
            current = node.parent.or_else(|| {
                input
                    .shadow_trees
                    .node_scopes
                    .get(&id)
                    .and_then(|root| input.shadow_trees.roots.get(root))
                    .map(|tree| tree.host)
            });
        }
        let mut inherited = current
            .and_then(|id| resolved.get(&id).cloned())
            .flatten()
            .or_else(|| input.document_language.clone());
        for id in path.into_iter().rev() {
            let node = nodes[&id];
            let own = node
                .namespaced_attributes
                .iter()
                .find(|attribute| {
                    attribute.namespace == "http://www.w3.org/XML/1998/namespace"
                        && attribute.local_name == "lang"
                })
                .map(|attribute| attribute.value.clone())
                .or_else(|| node.attributes.get("lang").cloned());
            if own.is_some() {
                inherited = own;
            }
            resolved.insert(id, inherited.clone());
        }
    }
    resolved
}

fn resolved_publication_bytes(
    lengths: impl IntoIterator<Item = (usize, usize, usize)>,
) -> Option<usize> {
    lengths.into_iter().try_fold(0usize, |bytes, (text, spans, aliases)| {
        let provenance = spans.checked_add(aliases)?.checked_mul(64)?;
        bytes.checked_add(text)?.checked_add(provenance)
    })
}

fn has_text_line_content(
    source: &text_source::TextSource,
    styles: &ComputedStyles,
    default: &BTreeMap<String, String>,
) -> bool {
    !source.text.is_empty()
        && !(source.text.bytes().all(|byte| byte == b' ')
            && source
                .spans
                .iter()
                .filter(|span| !span.bytes.is_empty())
                .all(|span| {
                    let style = styles
                        .get(&span.owner)
                        .map(|value| value.as_ref())
                        .unwrap_or(default);
                    line_whitespace(style) == parley::LineWhitespace::Collapse
                }))
}

fn text_indent(
    style: &BTreeMap<String, String>,
    basis: f64,
    font_size: f64,
    viewport: ViewportState,
    first_paragraph: bool,
) -> (f32, parley::IndentOptions) {
    let mut indent_options = parley::IndentOptions {
        skip_first_line: !first_paragraph,
        ..Default::default()
    };
    let indent_value = style_value(style, "text-indent", "0px")
        .split_ascii_whitespace()
        .filter(|token| match *token {
            "each-line" => {
                indent_options.each_line = true;
                false
            }
            "hanging" => {
                indent_options.hanging = true;
                false
            }
            _ => true,
        })
        .collect::<Vec<_>>()
        .join(" ");
    (
        length(&indent_value, basis, font_size, viewport) as f32,
        indent_options,
    )
}

fn line_whitespace(style: &BTreeMap<String, String>) -> parley::LineWhitespace {
    let whitespace = style_value(style, "white-space", "normal");
    let collapse = style_value(
        style,
        "white-space-collapse",
        match whitespace {
            "pre" | "pre-wrap" => "preserve",
            "pre-line" => "preserve-breaks",
            "break-spaces" => "break-spaces",
            _ => "collapse",
        },
    );
    let nowrap = style_value(
        style,
        "text-wrap-mode",
        if matches!(whitespace, "pre" | "nowrap") {
            "nowrap"
        } else {
            "wrap"
        },
    ) == "nowrap";
    match collapse {
        "break-spaces" => parley::LineWhitespace::BreakSpaces,
        "preserve" | "preserve-spaces" if nowrap => parley::LineWhitespace::Preserve,
        "preserve" | "preserve-spaces" => parley::LineWhitespace::ConditionalHang,
        _ => parley::LineWhitespace::Collapse,
    }
}

fn text_style_properties(
    style: &BTreeMap<String, String>,
    font_size: f64,
    viewport: ViewportState,
) -> Vec<StyleProperty<'static, TextBrush>> {
    let mut properties = Vec::new();
    let tab_size = style_value(style, "tab-size", "8");
    let tab_size = tab_size
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.)
        .map(parley::TabSize::Spaces)
        .unwrap_or_else(|| {
            parley::TabSize::Length(length(tab_size, 0., font_size, viewport).max(0.) as f32)
        });
    properties.push(StyleProperty::TabSize(tab_size));
    properties.push(StyleProperty::LineWhitespace(line_whitespace(style)));
    properties.push(StyleProperty::FontWeight(
        parley::FontWeight::parse_css(style_value(style, "font-weight", "normal"))
            .unwrap_or_default(),
    ));
    properties.push(StyleProperty::FontStyle(
        parley::FontStyle::parse_css(style_value(style, "font-style", "normal"))
            .unwrap_or_default(),
    ));
    properties.push(StyleProperty::FontWidth(
        parley::FontWidth::parse_css(style_value(style, "font-stretch", "normal"))
            .unwrap_or_default(),
    ));

    properties.push(StyleProperty::FontSize(font_size as f32));
    // Preserve CSS family order, including quoted names containing commas.
    // The bundled final fallback keeps resolution independent of host fonts.
    let family = style_value(style, "font-family", DEFAULT_FONT_FAMILY);
    properties.push(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(
        format!("{family}, \"{DEFAULT_FONT_FAMILY}\""),
    ))));
    let height = style_value(style, "line-height", "normal");
    let height = if height == "normal" {
        parley::LineHeight::MetricsRelative(1.)
    } else if let Some(number) = oryn_common::css_math::resolve_number(height) {
        parley::LineHeight::FontSizeRelative(number.max(0.) as f32)
    } else {
        parley::LineHeight::Absolute(length(height, font_size, font_size, viewport).max(0.) as f32)
    };
    properties.push(StyleProperty::LineHeight(height));
    for (property, is_word) in [("letter-spacing", false), ("word-spacing", true)] {
        let value = style_value(style, property, "normal");
        if value != "normal" {
            let spacing = length(value, font_size, font_size, viewport) as f32;
            properties.push(if is_word {
                StyleProperty::WordSpacing(spacing)
            } else {
                StyleProperty::LetterSpacing(spacing)
            });
        }
    }
    let word_break = style_value(style, "word-break", "normal");
    properties.push(StyleProperty::WordBreak(match word_break {
        "break-all" => parley::WordBreak::BreakAll,
        "keep-all" => parley::WordBreak::KeepAll,
        _ => parley::WordBreak::Normal,
    }));
    // The deprecated break-word value has normal word breaking and anywhere
    // overflow wrapping regardless of the computed overflow-wrap value.
    let overflow = if word_break == "break-word" {
        "anywhere"
    } else {
        style_value(style, "overflow-wrap", "normal")
    };
    properties.push(StyleProperty::OverflowWrap(match overflow {
        "anywhere" => parley::OverflowWrap::Anywhere,
        "break-word" => parley::OverflowWrap::BreakWord,
        _ => parley::OverflowWrap::Normal,
    }));
    let default_wrap = if matches!(
        style_value(style, "white-space", "normal"),
        "pre" | "nowrap"
    ) {
        "nowrap"
    } else {
        "wrap"
    };
    properties.push(StyleProperty::TextWrapMode(
        if style_value(style, "text-wrap-mode", default_wrap) == "nowrap" {
            parley::TextWrapMode::NoWrap
        } else {
            parley::TextWrapMode::Wrap
        },
    ));
    properties.push(StyleProperty::FontLanguageOverride(
        text_features::language_override(style),
    ));
    properties.push(StyleProperty::FontFeatures(parley::FontFeatures::List(
        Cow::Owned(text_features::resolve(style)),
    )));
    properties.push(StyleProperty::FontVariations(parley::FontVariations::List(
        Cow::Owned(text_features::variations(style, font_size)),
    )));
    properties.push(StyleProperty::Brush(TextBrush {
        pseudo: style
            .get("-oryn-text-pseudo-kind")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        families: style_value(style, "font-family", DEFAULT_FONT_FAMILY).into(),
        background: style
            .get("-oryn-text-background")
            .and_then(|color| oryn_common::color::parse_color(color)),
        owner: 0,
        decorations: text_decoration::read(style, font_size, viewport),
        color: oryn_common::color::parse_color(style_value(style, "color", "black"))
            .unwrap_or([0, 0, 0, 255]),
        synthesize_weight: style_value(style, "font-synthesis-weight", "auto") != "none",
        synthesize_style: style_value(style, "font-synthesis-style", "auto") != "none",
    }));
    properties
}

impl TextEngine {
    fn new(web_fonts: &[(String, Vec<u8>, String, bool, bool)]) -> Self {
        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        collection.register_fonts(
            fontique::Blob::new(Arc::new(DEFAULT_FONT_BYTES.to_vec())),
            Some(fontique::FontInfoOverride {
                family_name: Some(DEFAULT_FONT_FAMILY),
                ..fontique::FontInfoOverride::default()
            }),
        );
        let mut web_faces = Vec::new();
        let mut face_overrides = Vec::new();
        for (index, (family, bytes, descriptors, pending, blocked)) in web_fonts.iter().enumerate()
        {
            let mut face = web_font_faces::Face::new(index, family, descriptors);
            face.pending = *pending;
            face.blocked = *blocked;
            if *pending {
                web_faces.push(face);
                continue;
            }
            let registered = collection.register_fonts(
                fontique::Blob::new(Arc::new(bytes.clone())),
                Some(face.override_info()),
            );
            for (family, _) in registered {
                face_overrides.push((family, face.shaping_overrides()));
            }
            web_faces.push(face);
        }
        Self {
            web_faces,
            fonts: FontContext {
                collection,
                source_cache: fontique::SourceCache::default(),
                face_overrides,
            },
            layout: LayoutContext::new(),
            paint_fonts: BTreeMap::new(),
            font_keys: BTreeMap::new(),
            font_demands: BTreeMap::new(),
            font_demand_bytes: 0,
            font_demand_error: None,
            resolved_text: BTreeMap::new(),
            source_geometry: BTreeMap::new(),
            languages: BTreeMap::new(),
            decoration_styles: BTreeMap::new(),
            select_sizes: BTreeMap::new(),
            scrollbar_gutters: BTreeMap::new(),
            inline_sizes: BTreeMap::new(),
            inline_extents: BTreeMap::new(),
            inline_floats: BTreeMap::new(),
            float_context: None,
            float_block_offset: 0.,
            float_context_width: 0.,
            float_vertical: false,
            float_fragment: None,
            absorbed_margins: BTreeMap::new(),
            clearance_nodes: BTreeSet::new(),
            absorbed_vertical_margins: BTreeMap::new(),
            bfc_available_width: None,
            float_descendants: BTreeMap::new(),
            float_inline_insets: [0.; 2],
            ruby_baselines: BTreeMap::new(),
            first_line_baselines: BTreeMap::new(),
            column_runs: BTreeMap::new(),
            column_float_heights: BTreeMap::new(),
            column_float_trials: BTreeMap::new(),
            column_float_extents: BTreeMap::new(),
            column_balance_work: 0,
            column_float_section_heights: BTreeMap::new(),
            column_float_section_counts: BTreeMap::new(),
            column_float_section_sizes: BTreeMap::new(),
            column_float_section_index: BTreeMap::new(),
            column_context: None,
            active_inline_boxes: Vec::new(),
            measuring_nodes: BTreeSet::new(),
            out_of_flow: Vec::new(),
            table_rows: BTreeMap::new(),
            table_borders: BTreeMap::new(),
            collapsed_tables: Vec::new(),
            consumed_table_rows: BTreeSet::new(),
            break_boxes: BTreeMap::new(),
            table_end_spacing: BTreeMap::new(),
            table_widths: BTreeMap::new(),
            table_block_sizes: BTreeMap::new(),
            table_baseline_offsets: BTreeMap::new(),
            active_out_of_flow: None,
        }
    }

    #[cfg(test)]
    fn shape(&mut self, text: &str, width: f64, font_size: f64, font_family: &str) -> TextLayout {
        self.shape_styled(
            text,
            width,
            font_size,
            &BTreeMap::from([("font-family".into(), font_family.into())]),
            ViewportState::default(),
        )
    }

    fn shape_styled(
        &mut self,
        text: &str,
        width: f64,
        font_size: f64,
        style: &BTreeMap<String, String>,
        viewport: ViewportState,
    ) -> TextLayout {
        let source = text_source::TextSource::synthetic(text);
        self.shape_source(&source, width, font_size, style, viewport, &BTreeMap::new())
    }

    fn text_span_styles(
        &self,
        source: &text_source::TextSource,
        styles: &ComputedStyles,
        fallback: &BTreeMap<String, String>,
    ) -> ComputedStyles {
        let mut default = None;
        source
            .spans
            .iter()
            .map(|span| {
                let style = self
                    .decoration_styles
                    .get(&span.owner)
                    .or_else(|| styles.get(&span.owner))
                    .cloned()
                    .unwrap_or_else(|| {
                        default
                            .get_or_insert_with(|| Arc::new(fallback.clone()))
                            .clone()
                    });
                (span.owner, style)
            })
            .collect()
    }

    fn shape_source(
        &mut self,
        source: &text_source::TextSource,
        width: f64,
        font_size: f64,
        style: &BTreeMap<String, String>,
        viewport: ViewportState,
        styles: &ComputedStyles,
    ) -> TextLayout {
        let normalized = source.normalize(styles, style);
        let (source, pseudo_style) = text_pseudos::transform_source(
            self,
            &normalized,
            width,
            font_size,
            style,
            viewport,
            styles,
        );
        let style = &pseudo_style;
        // Decide after transformation: full-width spaces and preserved breaks
        // still create line boxes, even when the authored text was collapsible.
        let inline_boxes = source
            .spans
            .iter()
            .filter(|span| span.transformed && span.text_node.is_none() && span.bytes.is_empty())
            .filter_map(|span| {
                self.inline_sizes
                    .get(&span.owner)
                    .map(|(width, height, kind)| parley::InlineBox {
                        id: span.owner,
                        kind: *kind,
                        index: span.bytes.start,
                        width: *width as f32,
                        height: *height as f32,
                    })
            })
            .collect::<Vec<_>>();
        if !has_text_line_content(&source, styles, style) && inline_boxes.is_empty() {
            return TextLayout {
                resolved_text: vec![source.resolved_run(0, &[], styles, style)],
                ..TextLayout::default()
            };
        }
        let owned_styles = self.text_span_styles(&source, styles, style);
        let ranges = source
            .spans
            .iter()
            .filter_map(|span| {
                if span.bytes.is_empty() {
                    return None;
                }
                owned_styles.get(&span.owner).map(|style| {
                    (
                        span.bytes.clone(),
                        style.as_ref(),
                        self.languages.get(&span.owner).copied().flatten(),
                        span.owner,
                    )
                })
            })
            .collect::<Vec<_>>();
        let previous_boxes = std::mem::replace(&mut self.active_inline_boxes, inline_boxes);
        let has_floats = self
            .active_inline_boxes
            .iter()
            .any(|item| self.inline_floats.contains_key(&item.id));
        let owns_float_context = has_floats && self.float_context.is_none();
        let previous_float_offset = self.float_block_offset;
        let previous_float_width = self.float_context_width;
        let previous_float_insets = self.float_inline_insets;
        if owns_float_context {
            self.float_context = Some(taffy::compute::FloatContext::new());
            self.float_context_width = width;
            self.float_inline_insets = [0.; 2];
            self.float_block_offset = 0.;
        }
        let mut shaped =
            self.shape_ranged(&source.text, width, font_size, style, viewport, &ranges);
        if owns_float_context {
            self.float_context = None;
            self.float_block_offset = previous_float_offset;
            self.float_context_width = previous_float_width;
            self.float_inline_insets = previous_float_insets;
        }
        self.active_inline_boxes = previous_boxes;
        if !writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        {
            inline_formatting::resolve_baselines(&mut shaped, &self.inline_extents);
        }
        text_emphasis::apply(
            self,
            &mut shaped,
            &source.text,
            style,
            &owned_styles,
            viewport,
        );
        shaped.source_clusters = source.map_clusters(&shaped.clusters);
        shaped.resolved_text = vec![source.resolved_run(0, &shaped.clusters, styles, style)];
        for span in source.spans.iter().filter(|span| {
            span.text_node.is_none() && source.text.get(span.bytes.clone()) == Some("\u{2028}")
        }) {
            let mut offset = 0;
            let line = shaped.lines.iter().position(|line| {
                let inside = offset <= span.bytes.start && span.bytes.start < offset + line.0.len();
                offset += line.0.len();
                inside
            });
            let Some(line) = line else {
                continue;
            };
            let rtl = style_value(style, "direction", "ltr") == "rtl";
            let line_caret = |index: usize, end: bool| {
                shaped.lines.get(index).map(|line| CssRect {
                    x: line.4 + if end != rtl { line.1 } else { 0. },
                    y: line.3,
                    width: 0.,
                    height: shaped.line_heights.get(index).copied().unwrap_or(line.2),
                })
            };
            let before = line_caret(line, true).unwrap();
            let after = line_caret(line + 1, false);
            shaped.breaks.push(NativeBreak {
                node: span.owner,
                layout_node: 0,
                line,
                before,
                after,
                rtl,
            });
        }

        shaped
    }

    fn shape_ranged(
        &mut self,
        text: &str,
        width: f64,
        font_size: f64,
        style: &BTreeMap<String, String>,
        viewport: ViewportState,
        ranges: &[StyledTextRange<'_>],
    ) -> TextLayout {
        if !text.contains(['\n', '\u{2029}']) {
            return self.shape_paragraph(ParagraphShapeInputs {
                text,
                width,
                font_size,
                style,
                viewport,
                ranges,
                first_paragraph: true,
            });
        }
        let mut combined = TextLayout {
            resolved_text: Vec::new(),
            baselines: Vec::new(),
            line_heights: Vec::new(),
            provenance: Vec::new(),
            inline_boxes: Vec::new(),
            breaks: Vec::new(),
            lines: Vec::new(),
            line_ranges: Vec::new(),
            glyph_runs: Vec::new(),
            clusters: Vec::new(),
            source_clusters: Vec::new(),
            width: 0.,
            height: 0.,
        };
        let all_boxes = self.active_inline_boxes.clone();
        let total_bytes = text.len();
        let mut byte_offset = 0;
        // A preserved paragraph break resets Unicode bidi state. A line
        // separator (U+2028) only breaks the line and deliberately stays within
        // the same paragraph/base direction.
        for text in text.split_inclusive(['\n', '\u{2029}']) {
            let end = byte_offset + text.len();
            let paragraph_ranges = ranges
                .iter()
                .filter_map(|(range, style, language, owner)| {
                    let first = range.start.max(byte_offset);
                    let last = range.end.min(end);
                    (first < last).then(|| {
                        (
                            first - byte_offset..last - byte_offset,
                            *style,
                            *language,
                            *owner,
                        )
                    })
                })
                .collect::<Vec<_>>();
            self.active_inline_boxes = all_boxes
                .iter()
                .filter(|item| {
                    item.index >= byte_offset && (item.index < end || end == total_bytes)
                })
                .cloned()
                .map(|mut item| {
                    item.index -= byte_offset;
                    item
                })
                .collect();
            let previous_offset = self.float_block_offset;
            if self.float_context.is_some() {
                self.float_block_offset = previous_offset + combined.height;
            }
            let mut paragraph = self.shape_paragraph(ParagraphShapeInputs {
                text,
                width,
                font_size,
                style,
                viewport,
                ranges: &paragraph_ranges,
                first_paragraph: byte_offset == 0,
            });
            self.float_block_offset = previous_offset;
            // Parley exposes a trailing empty caret line after a final break.
            // CSS does not add an extra final line box for that caret position.
            // A real box after the final break has an empty text range too.
            // Retain that line and its index; only the synthetic terminal caret
            // is excluded from CSS flow (Parley makes the same distinction).
            let terminal = paragraph.lines.len().checked_sub(1);
            if text.ends_with(['\n', '\u{2029}'])
                && paragraph.lines.last().is_some_and(|line| line.0.is_empty())
                && !paragraph
                    .inline_boxes
                    .iter()
                    .any(|item| Some(item.line) == terminal)
            {
                paragraph.lines.pop();
                paragraph.baselines.pop();
                paragraph.line_heights.pop();
                paragraph.line_ranges.pop();
                paragraph.glyph_runs.pop();
                paragraph.provenance.pop();
                paragraph
                    .clusters
                    .retain(|cluster| cluster.line < paragraph.lines.len());
                // Parley already excludes the terminal caret line from layout
                // height. Selection rectangles can extend beyond CSS line-height
                // (negative leading); they must not replace the flow advance.
                paragraph.width = paragraph.lines.iter().map(|line| line.1).fold(0., f64::max);
            }
            let line_offset = combined.lines.len();
            for line in &mut paragraph.lines {
                line.3 += combined.height;
            }
            for cluster in &mut paragraph.clusters {
                cluster.bytes.start += byte_offset;
                cluster.bytes.end += byte_offset;
                cluster.line += line_offset;
                cluster.rect.y += combined.height;
            }
            for item in &mut paragraph.inline_boxes {
                if item.line != usize::MAX {
                    item.line += line_offset;
                }
                item.rect.y += combined.height;
            }
            combined.inline_boxes.extend(paragraph.inline_boxes);
            combined.width = combined.width.max(paragraph.width);
            combined.height += paragraph.height;
            combined.line_ranges.extend(
                paragraph
                    .line_ranges
                    .into_iter()
                    .map(|range| range.start + byte_offset..range.end + byte_offset),
            );
            for line in &mut paragraph.provenance {
                for source in line {
                    source.bytes.start += byte_offset;
                    source.bytes.end += byte_offset;
                    for range in &mut source.glyph_ranges {
                        range.start += byte_offset;
                        range.end += byte_offset;
                    }
                }
            }
            combined.provenance.extend(paragraph.provenance);
            combined.baselines.extend(paragraph.baselines);
            combined.line_heights.extend(paragraph.line_heights);
            combined.lines.extend(paragraph.lines);
            combined.glyph_runs.extend(paragraph.glyph_runs);
            combined.clusters.extend(paragraph.clusters);
            byte_offset = end;
        }
        self.active_inline_boxes = all_boxes;
        combined
    }

    fn paragraph_layout(
        &mut self,
        text: &str,
        font_size: f64,
        style: &BTreeMap<String, String>,
        viewport: ViewportState,
        ranges: &[StyledTextRange<'_>],
    ) -> Layout<TextBrush> {
        // Layout is in CSS pixels and is subsequently transformed/scaled by
        // the native paint owner. Device-pixel snapping here would round line
        // origins independently of their retained fractional flow advances.
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        let plaintext = style_value(style, "unicode-bidi", "normal") == "plaintext";
        builder.set_base_direction(if plaintext {
            parley::BaseDirection::Auto
        } else if style_value(style, "direction", "ltr") == "rtl" {
            parley::BaseDirection::Rtl
        } else {
            parley::BaseDirection::Ltr
        });
        for item in &self.active_inline_boxes {
            builder.push_inline_box(item.clone());
        }
        for property in text_style_properties(style, font_size, viewport) {
            builder.push_default(property);
        }
        for (range, span_style, language, owner) in ranges {
            builder.push(StyleProperty::Locale(*language), range.clone());
            // Provenance-only ranges inherit the paragraph's already resolved
            // size. An absent declaration is not a new 16px initial value.
            let size = span_style
                .get("font-size")
                .map(|value| length(value, font_size, font_size, viewport))
                .unwrap_or(font_size);
            for mut property in text_style_properties(span_style, size, viewport) {
                if let StyleProperty::Brush(brush) = &mut property {
                    brush.owner = *owner;
                }
                builder.push(property, range.clone());
            }
        }
        if !self.web_faces.is_empty() {
            let mut pending: Option<(usize, usize, String)> = None;
            use unicode_segmentation::UnicodeSegmentation;
            for (offset, cluster) in text.grapheme_indices(true) {
                let (current, owner) = ranges
                    .iter()
                    .rev()
                    .find(|(range, _, _, _)| range.contains(&offset))
                    .map(|(_, style, _, owner)| (*style, *owner))
                    .unwrap_or((style, 0));
                let family =
                    web_font_faces::families_for_cluster(&self.web_faces, current, cluster);
                if web_font_faces::blocked_for_cluster(&self.web_faces, current, cluster) {
                    for property in text_style_properties(current, font_size, viewport) {
                        if let StyleProperty::Brush(mut brush) = property {
                            brush.owner = owner;
                            brush.color[3] = 0;
                            brush.decorations.clear();
                            builder
                                .push(StyleProperty::Brush(brush), offset..offset + cluster.len());
                        }
                    }
                }
                if let Some((_, end, previous)) = &mut pending
                    && *previous == family
                {
                    *end = offset + cluster.len();
                    continue;
                }
                if let Some((start, end, family)) = pending.take() {
                    builder.push(
                        StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(family))),
                        start..end,
                    );
                }
                pending = Some((offset, offset + cluster.len(), family));
            }
            if let Some((start, end, family)) = pending {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(family))),
                    start..end,
                );
            }
        }
        let mut layout = builder.build(text);
        layout.set_first_line_end(
            style
                .get("-oryn-first-line-end")
                .and_then(|end| end.parse().ok()),
        );
        layout
    }

    fn measure_content_height(
        &mut self,
        node: &LayoutNode,
        width: f64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        viewport: ViewportState,
    ) -> f64 {
        if !self.measuring_nodes.insert(node.id) {
            return 0.;
        }
        let result = (|| {
            let Some(style) = styles.get(&node.id) else {
                return 0.;
            };
            if style_value(style, "display", "block") == "none" {
                return 0.;
            }
            if let Some((_, image_height)) = node
                .intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
            {
                let font_size = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
                let padding = edge(style, "padding", width, font_size, viewport);
                let border = border_edge(style, width, font_size, viewport);
                return preferred_ratio(node, style)
                    .map(|ratio| {
                        ratio.content_height(
                            width,
                            padding.left + padding.right + border.left + border.right,
                            padding.top + padding.bottom + border.top + border.bottom,
                        )
                    })
                    .unwrap_or(f64::from(image_height));
            }
            let font_size = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
            let source = text_source::TextSource::direct_with_breaks(node, nodes, styles);
            let mut height = self
                .shape_source(&source, width, font_size, style, viewport, styles)
                .height;
            if node.children.is_empty() {
                return height;
            }
            let container = matches!(
                style_value(style, "display", "block"),
                "flex" | "inline-flex" | "grid" | "inline-grid" | "table-row"
            ) || node.children.iter().any(|child| {
                styles
                    .get(child)
                    .is_some_and(|style| style_value(style, "float", "none") != "none")
            });
            if container {
                if let Some(placements) = taffy_child_placements(ChildPlacementInputs {
                    node,
                    parent_style: style,
                    styles,
                    width,
                    height: None,
                    viewport,
                    nodes,
                    text_engine: self,
                }) {
                    height += node
                        .children
                        .iter()
                        .zip(placements)
                        .map(|(child, placement)| {
                            let bottom_margin = styles
                                .get(child)
                                .map(|style| {
                                    edge(style, "margin", width, font_size, viewport).bottom
                                })
                                .unwrap_or(0.);
                            placement.y + placement.height + bottom_margin
                        })
                        .fold(0_f64, f64::max);
                }
                return height;
            }
            height = 0.;
            for item in inline_formatting::collect(node, nodes, styles) {
                let child_id = match item {
                    inline_formatting::FlowItem::Paragraph(paragraph) => {
                        height += inline_formatting::measure_paragraph(
                            paragraph, style, width, nodes, styles, viewport, self,
                        )
                        .height;
                        continue;
                    }
                    inline_formatting::FlowItem::Block(id) => id,
                };
                let (Some(child), Some(child_style)) =
                    (nodes.get(&child_id), styles.get(&child_id))
                else {
                    continue;
                };
                if style_value(child_style, "display", "block") == "none"
                    || matches!(
                        style_value(child_style, "position", "static"),
                        "absolute" | "fixed"
                    )
                {
                    continue;
                }
                let child_font = length(
                    style_value(child_style, "font-size", "16px"),
                    font_size,
                    font_size,
                    viewport,
                );
                let padding = edge(child_style, "padding", width, child_font, viewport);
                let border = border_edge(child_style, width, child_font, viewport);
                let margin = edge(child_style, "margin", width, child_font, viewport);
                let horizontal = padding.left + padding.right + border.left + border.right;
                let vertical = padding.top + padding.bottom + border.top + border.bottom;
                let available = (width - margin.left - margin.right - horizontal).max(0.);
                let border_box =
                    style_value(child_style, "box-sizing", "content-box") == "border-box";
                let sizes =
                    self.intrinsic_node(child, nodes, styles, viewport, &mut BTreeSet::new());
                let resolve = |value: &str| {
                    if is_intrinsic_width(value) {
                        resolve_intrinsic_width(value, available, sizes, child_font, viewport)
                    } else {
                        optional_length(value, width, child_font, viewport)
                            .map(|value| (value - if border_box { horizontal } else { 0. }).max(0.))
                    }
                };
                let mut child_width = resolve(style_value(child_style, "width", "auto"))
                    .unwrap_or_else(|| {
                        if is_inline(child_style) {
                            sizes.1.min(available.max(sizes.0))
                        } else {
                            available
                        }
                    });
                if let Some(max) = resolve(style_value(child_style, "max-width", "none")) {
                    child_width = child_width.min(max);
                }
                if let Some(min) = resolve(style_value(child_style, "min-width", "auto")) {
                    child_width = child_width.max(min);
                }
                // This is an intrinsic measurement with no definite parent
                // height. Percentage block sizes must not use the viewport.
                let child_height = block_height_length(
                    style_value(child_style, "height", "auto"),
                    None,
                    child_font,
                    viewport,
                )
                .map(|height| (height - if border_box { vertical } else { 0. }).max(0.))
                .unwrap_or_else(|| {
                    self.measure_content_height(child, child_width, nodes, styles, viewport)
                });
                let child_height = clamp_content_height(
                    child_style,
                    child_height,
                    None,
                    child_font,
                    viewport,
                    vertical,
                );
                height += margin.top + child_height + vertical + margin.bottom;
            }
            height
        })();
        self.measuring_nodes.remove(&node.id);
        result
    }

    fn vertical_block_extent(
        &mut self,
        node: &LayoutNode,
        inline_size: f64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        viewport: ViewportState,
        visiting: &mut BTreeSet<u64>,
    ) -> f64 {
        if !visiting.insert(node.id) {
            return 0.;
        }
        let Some(style) = styles.get(&node.id) else {
            visiting.remove(&node.id);
            return 0.;
        };
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let source = text_source::TextSource::direct_with_breaks(node, nodes, styles);
        let mut extent = self
            .shape_source(&source, inline_size.max(1.), font, style, viewport, styles)
            .height;
        for id in &node.children {
            let (Some(child), Some(child_style)) = (nodes.get(id), styles.get(id)) else {
                continue;
            };
            if style_value(child_style, "display", "block") == "none"
                || matches!(
                    style_value(child_style, "position", "static"),
                    "absolute" | "fixed"
                )
            {
                continue;
            }
            let child_font = length(
                style_value(child_style, "font-size", "16px"),
                font,
                font,
                viewport,
            );
            let margins = edge(child_style, "margin", inline_size, child_font, viewport);
            let padding = edge(child_style, "padding", inline_size, child_font, viewport);
            let border = border_edge(child_style, inline_size, child_font, viewport);
            let edges = padding.left + padding.right + border.left + border.right;
            let preferred = optional_length(
                style_value(child_style, "width", "auto"),
                0.,
                child_font,
                viewport,
            )
            .map(|w| {
                if style_value(child_style, "box-sizing", "content-box") == "border-box" {
                    (w - edges).max(0.)
                } else {
                    w
                }
            });
            let width = preferred.unwrap_or_else(|| {
                if writing_mode::Axes::new(
                    style_value(child_style, "writing-mode", "horizontal-tb"),
                    false,
                )
                .vertical
                {
                    let height = optional_length(
                        style_value(child_style, "height", "auto"),
                        inline_size,
                        child_font,
                        viewport,
                    )
                    .unwrap_or(inline_size);
                    self.vertical_block_extent(child, height, nodes, styles, viewport, visiting)
                } else {
                    self.intrinsic_node(child, nodes, styles, viewport, visiting)
                        .1
                }
            });
            extent += width + edges + margins.left + margins.right;
        }
        visiting.remove(&node.id);
        extent
    }

    fn intrinsic_node(
        &mut self,
        node: &LayoutNode,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        viewport: ViewportState,
        visiting: &mut BTreeSet<u64>,
    ) -> (f64, f64) {
        if !visiting.insert(node.id) {
            return (0., 0.);
        }
        let Some(style) = styles.get(&node.id) else {
            visiting.remove(&node.id);
            return (0., 0.);
        };
        let font_size = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        if let Some((width, _)) = self.select_sizes.get(&node.id) {
            visiting.remove(&node.id);
            return (*width, *width);
        }
        if matches!(
            style_value(style, "display", "block"),
            "ruby" | "block ruby"
        ) {
            let (_, layout, _) = ruby::layout(node, nodes, styles, font_size, viewport, self);
            let width = if writing_mode::Axes::new(
                style_value(style, "writing-mode", "horizontal-tb"),
                false,
            )
            .vertical
            {
                layout.height
            } else {
                layout.width
            };
            visiting.remove(&node.id);
            return (width, width);
        }
        if writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
            && node
                .intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
                .is_none()
        {
            visiting.remove(&node.id);
            let height = block_height_length(
                style_value(style, "height", "auto"),
                None,
                font_size,
                viewport,
            )
            .unwrap_or(viewport.height);
            let width = self.vertical_block_extent(node, height, nodes, styles, viewport, visiting);
            return (width, width);
        }
        let mut widths = (0_f64, 0_f64);
        let mut inline_max = 0_f64;
        let mut inline_min = 0_f64;
        let whitespace = style_value(style, "white-space", "normal");
        let nowrap = style_value(
            style,
            "text-wrap-mode",
            if matches!(whitespace, "pre" | "nowrap") {
                "nowrap"
            } else {
                "wrap"
            },
        ) == "nowrap";
        if let Some((width, _)) = node
            .intrinsic_image_size
            .or(node.document_replaced.then_some((300, 150)))
        {
            widths = (f64::from(width), f64::from(width));
        } else {
            for item in text_source::TextSource::intrinsic_items(node, nodes, styles) {
                match item {
                    text_source::IntrinsicItem::Text(source) => {
                        let text =
                            self.intrinsic_source(&source, font_size, style, viewport, styles);
                        if nowrap {
                            inline_min += text.0;
                        } else {
                            widths.0 = widths.0.max(text.0);
                        }
                        inline_max += text.1;
                    }
                    text_source::IntrinsicItem::Break => {
                        widths.0 = widths.0.max(inline_min);
                        widths.1 = widths.1.max(inline_max);
                        inline_min = 0.;
                        inline_max = 0.;
                    }
                    text_source::IntrinsicItem::Box { node: id, inline } => {
                        let Some(child) = nodes.get(&id) else {
                            continue;
                        };
                        let Some(child_style) = styles.get(&id) else {
                            continue;
                        };
                        let mut child_widths =
                            self.intrinsic_node(child, nodes, styles, viewport, visiting);
                        let child_font = length(
                            style_value(child_style, "font-size", "16px"),
                            font_size,
                            font_size,
                            viewport,
                        );
                        let padding = edge(child_style, "padding", 0., child_font, viewport);
                        let border = border_edge(child_style, 0., child_font, viewport);
                        let margin = edge(child_style, "margin", 0., child_font, viewport);
                        let edges = padding.left + padding.right + border.left + border.right;
                        let border_box =
                            style_value(child_style, "box-sizing", "content-box") == "border-box";
                        let resolve = |value: &str| {
                            if is_intrinsic_width(value) {
                                resolve_intrinsic_width(
                                    value,
                                    child_widths.1,
                                    child_widths,
                                    child_font,
                                    viewport,
                                )
                            } else if value.contains('%') {
                                None // Cyclic preferred sizes behave as auto during intrinsic sizing.
                            } else {
                                optional_length(value, 0., child_font, viewport).map(|value| {
                                    (value - if border_box { edges } else { 0. }).max(0.)
                                })
                            }
                        };
                        let preferred = resolve(style_value(child_style, "width", "auto"));
                        let minimum = resolve(style_value(child_style, "min-width", "auto"));
                        let maximum = resolve(style_value(child_style, "max-width", "none"));
                        if let Some(width) = preferred {
                            child_widths = (width, width);
                        }
                        for width in [&mut child_widths.0, &mut child_widths.1] {
                            if let Some(max) = maximum {
                                *width = (*width).min(max);
                            }
                            if let Some(min) = minimum {
                                *width = (*width).max(min);
                            }
                            *width = (*width + edges + margin.left + margin.right).max(0.);
                        }
                        if inline {
                            if nowrap {
                                inline_min += child_widths.0;
                            } else {
                                widths.0 = widths.0.max(child_widths.0);
                            }
                            inline_max += child_widths.1;
                        } else {
                            widths.0 = widths.0.max(inline_min).max(child_widths.0);
                            widths.1 = widths.1.max(inline_max).max(child_widths.1);
                            inline_min = 0.;
                            inline_max = 0.;
                        }
                    }
                }
            }
        }
        widths.0 = widths.0.max(inline_min);
        widths.1 = widths.1.max(inline_max);
        visiting.remove(&node.id);
        let gutter = self
            .scrollbar_gutters
            .get(&node.id)
            .copied()
            .unwrap_or_default();
        (
            widths.0 + gutter.left + gutter.right,
            widths.1 + gutter.left + gutter.right,
        )
    }

    fn intrinsic_source(
        &mut self,
        source: &text_source::TextSource,
        font_size: f64,
        style: &BTreeMap<String, String>,
        viewport: ViewportState,
        styles: &ComputedStyles,
    ) -> (f64, f64) {
        if text_pseudos::read(style).active {
            // Intrinsic widths must use the same first formatted line, font and
            // source-preserving transformation as final layout. Keep measurement
            // float placement isolated from the caller's formatting context.
            let saved_context = self.float_context.clone();
            let saved_boxes = self.active_inline_boxes.clone();
            let mut maximum = style.clone();
            maximum.insert("text-wrap-mode".into(), "nowrap".into());
            let extent = |layout: TextLayout| {
                let floats = layout
                    .glyph_runs
                    .iter()
                    .flatten()
                    .filter_map(|run| run.text_box.as_ref())
                    .filter(|box_style| style_value(&box_style.style, "float", "none") != "none")
                    .map(|box_style| {
                        let model = &box_style.model;
                        if writing_mode::Axes::new(
                            style_value(style, "writing-mode", "horizontal-tb"),
                            false,
                        )
                        .vertical
                        {
                            model.border_box.height + model.margin.top + model.margin.bottom
                        } else {
                            model.border_box.width + model.margin.left + model.margin.right
                        }
                    })
                    .fold(0., f64::max);
                layout.width.max(floats)
            };
            let max = extent(self.shape_source(
                source,
                1_000_000_000.,
                font_size,
                &maximum,
                viewport,
                styles,
            ));
            self.float_context = saved_context.clone();
            self.active_inline_boxes = saved_boxes.clone();
            let mut minimum = style.clone();
            if style_value(&minimum, "overflow-wrap", "normal") == "break-word" {
                minimum.insert("overflow-wrap".into(), "normal".into());
            }
            let min = extent(self.shape_source(source, 0., font_size, &minimum, viewport, styles));
            self.float_context = saved_context;
            self.active_inline_boxes = saved_boxes;
            return (min, max.max(min));
        }
        let source = source
            .normalize(styles, style)
            .transform(styles, style, &self.languages);
        if !has_text_line_content(&source, styles, style) {
            return (0., 0.);
        }
        let ranges = source
            .spans
            .iter()
            .filter_map(|span| {
                if span.bytes.is_empty() {
                    return None;
                }
                styles.get(&span.owner).map(|style| {
                    (
                        span.bytes.clone(),
                        style.as_ref(),
                        self.languages.get(&span.owner).copied().flatten(),
                        span.owner,
                    )
                })
            })
            .collect::<Vec<_>>();
        if writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        {
            let mut maximum = style.clone();
            maximum.insert("text-wrap-mode".into(), "nowrap".into());
            let max = self
                .shape_ranged(
                    &source.text,
                    1_000_000_000.,
                    font_size,
                    &maximum,
                    viewport,
                    &ranges,
                )
                .width;
            let mut minimum = style.clone();
            // break-word introduces emergency breaks in layout, but only
            // anywhere contributes those opportunities to min-content sizing.
            if style_value(&minimum, "overflow-wrap", "normal") == "break-word" {
                minimum.insert("overflow-wrap".into(), "normal".into());
            }
            let min = self
                .shape_ranged(&source.text, 1., font_size, &minimum, viewport, &ranges)
                .width;
            return (min, max.max(min));
        }
        let mut layout = self.paragraph_layout(&source.text, font_size, style, viewport, &ranges);
        // Cyclic percentages contribute zero during intrinsic sizing. Absolute
        // terms in calc() and the each-line/hanging flags still participate.
        let (indent, options) = text_indent(style, 0., font_size, viewport, true);
        layout.set_text_indent(indent, options);
        let widths = layout.calculate_content_widths();
        (f64::from(widths.min), f64::from(widths.max))
    }

    fn shape_paragraph(&mut self, params: ParagraphShapeInputs<'_>) -> TextLayout {
        let ParagraphShapeInputs {
            text,
            width,
            font_size,
            style,
            viewport,
            ranges,
            first_paragraph,
        } = params;

        if !first_paragraph && style.contains_key("-oryn-first-line-end") {
            let mut plain = style.clone();
            for key in [
                text_pseudos::KEY,
                "-oryn-first-line-end",
                "-oryn-first-letter-start",
                "-oryn-first-letter-end",
            ] {
                plain.remove(key);
            }
            return self.shape_paragraph(ParagraphShapeInputs {
                text,
                width,
                font_size,
                style: &plain,
                viewport,
                ranges,
                first_paragraph: false,
            });
        }
        if text.is_empty() && self.active_inline_boxes.is_empty() {
            return TextLayout {
                resolved_text: Vec::new(),
                baselines: Vec::new(),
                line_heights: Vec::new(),
                provenance: Vec::new(),
                inline_boxes: Vec::new(),
                breaks: Vec::new(),
                lines: Vec::new(),
                line_ranges: Vec::new(),
                glyph_runs: Vec::new(),
                clusters: Vec::new(),
                source_clusters: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }
        if first_paragraph && text_pseudos::read(style).active {
            return text_pseudos::shape(self, text, width, font_size, style, viewport, ranges);
        }
        if writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        {
            return vertical_text::shape(self, text, width, font_size, style, viewport, ranges);
        }
        let mut layout = self.paragraph_layout(text, font_size, style, viewport, ranges);
        let (indent, options) = text_indent(style, width, font_size, viewport, first_paragraph);
        layout.set_text_indent(indent, options);
        let float_origin = if self.float_context.is_some() {
            self.float_block_offset
        } else {
            0.
        };
        let float_placements = if self.float_context.is_some()
            || self
                .active_inline_boxes
                .iter()
                .any(|item| self.inline_floats.contains_key(&item.id))
        {
            let mut context = self.float_context.take().unwrap_or_default();
            let placed = float_lines::break_layout(crate::float_lines::FloatLineInputs {
                layout: &mut layout,
                width,
                specs: &self.inline_floats,
                context: &mut context,
                origin: float_origin,
                context_width: self.float_context_width,
                containing_insets: self.float_inline_insets,
                fragment: self.float_fragment,
            });
            self.float_context = Some(context);
            placed
        } else {
            layout.break_all_lines(Some(width.max(1.0) as f32));
            Vec::new()
        };
        let rtl = layout.is_rtl();
        let alignment = match style_value(style, "text-align", "start") {
            "center" | "-moz-center" => Alignment::Center,
            "right" | "-moz-right" => Alignment::Right,
            "left" | "-moz-left" => Alignment::Left,
            "end" => {
                if rtl {
                    Alignment::Left
                } else {
                    Alignment::Right
                }
            }
            "justify" => Alignment::Justify,
            _ => {
                if rtl {
                    Alignment::Right
                } else {
                    Alignment::Left
                }
            }
        };
        let last_line = match style_value(style, "text-align-last", "auto") {
            "start" => Some(Alignment::Start),
            "end" => Some(Alignment::End),
            "left" => Some(Alignment::Left),
            "right" => Some(Alignment::Right),
            "center" => Some(Alignment::Center),
            "justify" => Some(Alignment::Justify),
            _ => None,
        };
        layout.align(
            alignment,
            AlignmentOptions {
                align_when_overflowing: true,
                last_line,
            },
        );
        let mut clusters = Vec::new();
        for (line_index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            let mut x = metrics.offset;
            for run in line.runs() {
                let mut first = true;
                for cluster in run.visual_clusters() {
                    if first {
                        x = cluster.visual_offset().unwrap_or(x);
                        first = false;
                    }
                    let advance = cluster.advance();
                    clusters.push(text_source::TextCluster {
                        bytes: cluster.text_range(),
                        line: line_index,
                        rtl: cluster.is_rtl(),
                        rect: CssRect {
                            x: f64::from(x),
                            y: f64::from(metrics.block_min_coord) - float_origin,
                            width: f64::from(advance),
                            height: f64::from(metrics.block_max_coord - metrics.block_min_coord),
                        },
                    });
                    x += advance;
                }
            }
        }
        let mut provenance = Vec::new();
        let glyph_runs = layout
            .lines()
            .map(|line| {
                let mut sources = Vec::new();
                let metrics = line.metrics();
                let runs = line
                    .items()
                    .filter_map(|item| {
                        let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                            return None;
                        };
                        let run = glyph_run.run();
                        let font = run.font();
                        let key = self
                            .font_keys
                            .entry((font.data.id(), font.index))
                            .or_insert_with(|| {
                                format!("{:x}:{}", Sha256::digest(font.data.data()), font.index)
                            })
                            .clone();
                        self.paint_fonts.entry(key.clone()).or_insert_with(|| {
                            oryn_common::v4::PaintFontResource {
                                bytes: font.data.data().to_vec(),
                                collection_index: font.index,
                            }
                        });
                        let owner = glyph_run.style().brush.owner;
                        let begin = glyph_run.offset();
                        let end = begin + glyph_run.advance();
                        let mut offset = run
                            .visual_clusters()
                            .next()
                            .and_then(|cluster| cluster.visual_offset())
                            .unwrap_or(begin);
                        let bytes = run
                            .visual_clusters()
                            .filter_map(|cluster| {
                                let x = offset;
                                offset += cluster.advance();
                                (cluster.first_style().brush.owner == owner
                                    && ((x < end && offset > begin)
                                        || (cluster.advance() == 0. && x >= begin && x <= end)))
                                    .then(|| cluster.text_range())
                            })
                            .fold(None::<std::ops::Range<usize>>, |range, next| {
                                Some(
                                    range
                                        .map(|range| {
                                            range.start.min(next.start)..range.end.max(next.end)
                                        })
                                        .unwrap_or(next),
                                )
                            })
                            .unwrap_or(0..0);
                        let mut pen = run
                            .visual_clusters()
                            .next()
                            .and_then(|cluster| cluster.visual_offset())
                            .unwrap_or(begin);
                        let mut glyph_ranges = Vec::new();
                        for cluster in run.visual_clusters() {
                            for glyph in cluster.glyphs() {
                                let x = pen;
                                pen += glyph.advance;
                                if std::ptr::eq(
                                    &layout.styles()[glyph.style_index()],
                                    glyph_run.style(),
                                ) && x >= begin - 0.001
                                    && x <= end + 0.001
                                {
                                    glyph_ranges.push(cluster.text_range());
                                }
                            }
                        }
                        if glyph_ranges.len() != glyph_run.glyphs().count() {
                            glyph_ranges.clear();
                        }
                        sources.push(RunProvenance {
                            families: glyph_run.style().brush.families.clone(),
                            text_override: None,
                            owner,
                            bytes,
                            glyph_ranges,
                        });
                        Some(oryn_common::v4::PaintGlyphRun {
                            line_box: None,
                            text_box: None,
                            background: glyph_run.style().brush.background,
                            decoration_rect: Some(CssRect {
                                x: f64::from(glyph_run.offset() - metrics.offset),
                                y: f64::from(metrics.block_min_coord - metrics.block_origin),
                                width: f64::from(glyph_run.advance()),
                                height: f64::from(
                                    metrics.block_max_coord - metrics.block_min_coord,
                                ),
                            }),
                            scale_x: 1.,
                            vertical: false,
                            decorations: glyph_run.style().brush.decorations.clone(),
                            orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
                            font_key: key,
                            font_size: f64::from(run.font_size()),
                            color: Some(glyph_run.style().brush.color),
                            embolden: glyph_run.style().brush.synthesize_weight
                                && run.synthesis().embolden(),
                            skew_degrees: if glyph_run.style().brush.synthesize_style {
                                run.synthesis().skew().unwrap_or(0.)
                            } else {
                                0.
                            },
                            normalized_coords: run.normalized_coords().to_vec(),
                            glyphs: glyph_run
                                .positioned_glyphs()
                                .map(|glyph| oryn_common::v4::PaintGlyph {
                                    source: None,
                                    id: glyph.id,
                                    x: f64::from(glyph.x - metrics.offset),
                                    y: f64::from(glyph.y - metrics.block_origin),
                                    advance: f64::from(glyph.advance),
                                })
                                .collect(),
                        })
                    })
                    .collect();
                provenance.push(sources);
                runs
            })
            .collect();
        let lines = layout
            .lines()
            .map(|line| {
                let metrics = line.metrics();
                (
                    text.get(line.text_range()).unwrap_or_default().to_string(),
                    f64::from(metrics.advance - metrics.trailing_whitespace),
                    f64::from(metrics.block_max_coord - metrics.block_min_coord),
                    f64::from(metrics.block_origin) - float_origin,
                    f64::from(metrics.offset),
                )
            })
            .collect();
        let float_ids = self.inline_floats.keys().copied().collect::<BTreeSet<_>>();
        let float_ids = &float_ids;
        TextLayout {
            resolved_text: Vec::new(),
            baselines: layout
                .lines()
                .map(|line| f64::from(line.metrics().baseline - line.metrics().block_origin))
                .collect(),
            line_heights: layout
                .lines()
                .map(|line| f64::from(line.metrics().line_height))
                .collect(),
            provenance,
            inline_boxes: layout
                .lines()
                .enumerate()
                .flat_map(|(line_index, line)| {
                    line.items().filter_map(move |item| match item {
                        parley::PositionedLayoutItem::InlineBox(item)
                            if !float_ids.contains(&item.id) =>
                        {
                            Some(InlinePlacement {
                                node: item.id,
                                rect: CssRect {
                                    x: f64::from(item.x),
                                    y: f64::from(item.y) - float_origin,
                                    width: f64::from(item.width),
                                    height: f64::from(item.height),
                                },
                                line: line_index,
                            })
                        }
                        _ => None,
                    })
                })
                .chain(float_placements)
                .collect(),
            breaks: Vec::new(),
            lines,
            line_ranges: layout.lines().map(|line| line.text_range()).collect(),
            glyph_runs,
            clusters,
            source_clusters: Vec::new(),
            width: f64::from(layout.width()),
            // Parley height sums line advances; unlike positions, it has no BFC origin.
            height: f64::from(layout.height()).max(0.),
        }
    }
}

/// Compute a deterministic style/layout snapshot.  Geometry is expressed in
/// viewport coordinates, matching `getBoundingClientRect`.
/// Resource demand from native computed boxes and generated content.
pub fn image_requests(output: &LayoutOutput) -> BTreeSet<String> {
    let mut urls = output.css_image_requests.clone();
    for (id, style) in &output.computed_styles {
        if !output.geometry.contains_key(id) {
            continue;
        }
        for value in style.values() {
            urls.extend(
                cssom::resource_references(value)
                    .into_iter()
                    .map(|reference| reference.url),
            );
        }
    }
    for node in output
        .generated_nodes
        .iter()
        .chain(output.backdrops.values())
    {
        for value in node.style.values() {
            urls.extend(
                cssom::resource_references(value)
                    .into_iter()
                    .map(|reference| reference.url),
            );
        }
    }
    for fragments in output.text_fragments.values() {
        for fragment in fragments {
            for run in &fragment.glyph_runs {
                for box_style in [run.line_box.as_ref(), run.text_box.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    for value in box_style.style.values() {
                        urls.extend(
                            cssom::resource_references(value)
                                .into_iter()
                                .map(|reference| reference.url),
                        );
                    }
                }
            }
        }
    }
    urls.retain(|url| !url.starts_with('#'));
    urls
}

pub fn compute(input: &LayoutInput, limits: &LayoutLimits) -> Result<LayoutOutput, LayoutError> {
    StyleEngine::default().compute(input, limits)
}

/// Format one rendered subtree in an independent local viewport while retaining
/// the full document's already matched styles. Used by SVG foreignObject: SVG
/// owns the wrapper transform/opacity; descendants retain their own CSS effects.
pub fn compute_subtree_with_styles(
    input: &LayoutInput,
    root: u64,
    viewport: ViewportState,
    styles: &ComputedStyles,
    limits: &LayoutLimits,
) -> Result<LayoutOutput, LayoutError> {
    compute_subtree_with_styles_and_pseudos(input, root, viewport, styles, &BTreeMap::new(), limits)
}

/// Match pseudo content in the owning document before selecting the local
/// formatting subtree, so inherited counters and selector scope remain intact.
pub fn compute_subtree_with_styles_and_pseudos(
    input: &LayoutInput,
    root: u64,
    viewport: ViewportState,
    styles: &ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    limits: &LayoutLimits,
) -> Result<LayoutOutput, LayoutError> {
    validate(input, limits)?;
    let (generated, object_hidden) = {
        let native_input = permission_controls::project(input);
        let (native_input, hidden) = object_images::project_styled(&native_input, styles);
        let mut native_styles = styles.clone();
        let mut native_pseudos = pseudos.clone();
        permission_controls::adjust(input, &mut native_styles, &mut native_pseudos);
        (
            generated::materialize(&native_input, native_styles, &native_pseudos)?,
            hidden,
        )
    };
    let input = &generated.input;
    let styles = &generated.styles;
    let language = effective_languages(input).get(&root).cloned().flatten();
    let mut local = project_rendered_tree(input);
    local.rendered_tree = None;
    let by_id = local
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    if !by_id.contains_key(&root) {
        return Err(LayoutError::InvalidRenderedTree(format!(
            "subtree root {root} is not rendered"
        )));
    }
    let mut included = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !included.insert(id) {
            return Err(LayoutError::InvalidRenderedTree(
                "subtree repeats a rendered node".into(),
            ));
        }
        let node = by_id.get(&id).ok_or_else(|| {
            LayoutError::InvalidRenderedTree(format!("missing subtree node {id}"))
        })?;
        pending.extend(node.children.iter().copied());
    }
    local.nodes.retain(|node| included.contains(&node.id));
    local
        .top_layer
        .retain(|entry| included.contains(&entry.node_id));
    local
        .element_scrolls
        .retain(|id, _| included.contains(id) && *id != root);
    local.viewport = viewport;
    local.scroll = ScrollState::default();
    let wrapper = local.nodes.iter_mut().find(|node| node.id == root).unwrap();
    wrapper.parent = None;
    wrapper.tag = "div".into();
    wrapper.namespace = default_element_namespace();
    wrapper.intrinsic_image_size = None;
    wrapper.document_replaced = false;
    if let Some(language) = language {
        wrapper.attributes.insert("lang".into(), language);
    }
    let mut computed = styles.clone();
    let style = computed.entry(root).or_default();
    let style = Arc::make_mut(style);
    for (name, value) in [
        ("display", "flow-root"),
        ("position", "relative"),
        ("float", "none"),
        ("clear", "none"),
        ("transform", "none"),
        ("translate", "none"),
        ("rotate", "none"),
        ("scale", "none"),
        ("perspective", "none"),
        ("opacity", "1"),
        ("filter", "none"),
        ("backdrop-filter", "none"),
        ("clip-path", "none"),
        ("mask-image", "none"),
        ("mix-blend-mode", "normal"),
        ("isolation", "auto"),
        ("contain", "none"),
        ("will-change", "auto"),
        ("background-color", "transparent"),
        ("background-image", "none"),
        ("box-shadow", "none"),
        ("box-sizing", "border-box"),
        ("min-width", "0px"),
        ("min-height", "0px"),
        ("max-width", "none"),
        ("max-height", "none"),
        ("aspect-ratio", "auto"),
    ] {
        style.insert(name.into(), value.into());
    }
    for side in ["top", "right", "bottom", "left"] {
        style.insert(side.into(), "auto".into());
        style.insert(format!("margin-{side}"), "0px".into());
        style.insert(format!("padding-{side}"), "0px".into());
        style.insert(format!("border-{side}-width"), "0px".into());
        style.insert(format!("border-{side}-style"), "none".into());
    }
    style.insert("width".into(), format!("{}px", viewport.width));
    style.insert("height".into(), format!("{}px", viewport.height));
    let vertical = matches!(
        style.get("writing-mode").map(String::as_str),
        Some("vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr")
    );
    style.insert(
        "inline-size".into(),
        format!(
            "{}px",
            if vertical {
                viewport.height
            } else {
                viewport.width
            }
        ),
    );
    style.insert(
        "block-size".into(),
        format!(
            "{}px",
            if vertical {
                viewport.width
            } else {
                viewport.height
            }
        ),
    );
    let mut output = compute_with_styles(&local, limits, computed)?;
    output.pseudo_styles = pseudos
        .iter()
        .filter(|(id, _)| included.contains(id))
        .map(|(id, styles)| (*id, styles.clone()))
        .collect();
    generated::finish(&generated, &mut output);
    object_images::finish(&mut output, &object_hidden);
    Ok(output)
}

fn preserve_anchor_parents(input: &LayoutInput) -> LayoutInput {
    let mut projected = input.clone();
    for node in &input.nodes {
        projected
            .anchor_parents
            .entry(node.id)
            .or_insert(node.parent);
    }
    projected
}

fn project_rendered_tree(input: &LayoutInput) -> LayoutInput {
    let native_tree;
    let tree = if let Some(tree) = &input.rendered_tree {
        tree
    } else if !input.shadow_trees.roots.is_empty() {
        native_tree = shadow_tree::rendered(input);
        &native_tree
    } else {
        return preserve_anchor_parents(input);
    };
    let parents = tree
        .iter()
        .flat_map(|(parent, content)| {
            content.iter().filter_map(move |item| {
                if let LayoutContent::Element(child) = item {
                    Some((*child, *parent))
                } else {
                    None
                }
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut projected = preserve_anchor_parents(input);
    projected.shadow_trees = ShadowTrees::default();
    projected.nodes.retain(|node| tree.contains_key(&node.id));
    for node in &mut projected.nodes {
        let content = tree[&node.id].clone();
        node.parent = parents.get(&node.id).copied();
        node.children = content
            .iter()
            .filter_map(|item| {
                if let LayoutContent::Element(child) = item {
                    Some(*child)
                } else {
                    None
                }
            })
            .collect();
        node.text = content
            .iter()
            .filter_map(|item| match item {
                LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        node.content = Some(content);
    }
    projected
}

/// Canvas fallback remains in the selector/DOM tree but does not generate
/// boxes or text when the canvas bitmap is rendered.
fn suppress_canvas_fallback(input: &mut LayoutInput) {
    let mut removed = BTreeSet::new();
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut pending = input
        .nodes
        .iter()
        .filter(|node| node.namespace == "http://www.w3.org/1999/xhtml" && node.tag == "canvas")
        .flat_map(|node| node.children.iter().copied())
        .collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        if !removed.insert(id) {
            continue;
        }
        if let Some(node) = by_id.get(&id) {
            pending.extend(node.children.iter().copied());
        }
    }
    drop(by_id);
    input.nodes.retain(|node| !removed.contains(&node.id));
    for node in &mut input.nodes {
        if node.namespace == "http://www.w3.org/1999/xhtml" && node.tag == "canvas" {
            node.children.clear();
            node.content = Some(Vec::new());
            node.text.clear();
        }
    }
}

fn flatten_contents_boxes(input: &LayoutInput, styles: &ComputedStyles) -> LayoutInput {
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let boxless = input
        .nodes
        .iter()
        .filter(|node| {
            node.parent.is_some()
                && styles
                    .get(&node.id)
                    .is_some_and(|style| style_value(style, "display", "block") == "contents")
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let source_content = |node: &LayoutNode| {
        node.content.clone().unwrap_or_else(|| {
            let mut content = vec![LayoutContent::StyledText {
                owner: node.id,
                node_id: None,
                text: node.text.clone(),
            }];
            content.extend(
                node.children
                    .iter()
                    .map(|child| LayoutContent::Element(*child)),
            );
            content
        })
    };
    let mut projected = preserve_anchor_parents(input);
    projected.nodes.retain(|node| !boxless.contains(&node.id));
    let mut parents = BTreeMap::new();
    for node in &mut projected.nodes {
        let mut pending = source_content(node)
            .into_iter()
            .rev()
            .map(|item| (node.id, item))
            .collect::<Vec<_>>();
        let mut expanded = BTreeSet::new();
        let mut content = Vec::new();
        while let Some((owner, item)) = pending.pop() {
            match item {
                LayoutContent::Element(child) if boxless.contains(&child) => {
                    if expanded.insert(child)
                        && let Some(wrapper) = by_id.get(&child)
                    {
                        pending.extend(
                            source_content(wrapper)
                                .into_iter()
                                .rev()
                                .map(|item| (child, item)),
                        );
                    }
                }
                LayoutContent::Text { node_id, text } if owner != node.id => {
                    content.push(LayoutContent::StyledText {
                        owner,
                        node_id: Some(node_id),
                        text,
                    });
                }
                LayoutContent::Element(child) => {
                    parents.insert(child, node.id);
                    content.push(LayoutContent::Element(child));
                }
                item => content.push(item),
            }
        }
        node.children = content
            .iter()
            .filter_map(|item| {
                if let LayoutContent::Element(child) = item {
                    Some(*child)
                } else {
                    None
                }
            })
            .collect();
        node.text = content
            .iter()
            .filter_map(|item| match item {
                LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        node.content = Some(content);
    }
    for node in &mut projected.nodes {
        node.parent = parents.get(&node.id).copied();
    }
    projected
}

fn materialize_anonymous_text_items(
    input: &mut LayoutInput,
    styles: &mut ComputedStyles,
) -> BTreeMap<u64, u64> {
    let mut used = input
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    for node in &input.nodes {
        for item in node.content.iter().flatten() {
            match item {
                LayoutContent::Text { node_id, .. }
                | LayoutContent::StyledText {
                    node_id: Some(node_id),
                    ..
                } => {
                    used.insert(*node_id);
                }
                _ => {}
            }
        }
    }
    let mut next_id = u64::MAX;
    let mut generated = Vec::new();
    let mut owners = BTreeMap::new();
    for parent in &mut input.nodes {
        let Some(parent_style) = styles.get(&parent.id).cloned() else {
            continue;
        };
        if !matches!(
            style_value(&parent_style, "display", "block"),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        ) {
            continue;
        }
        let content = parent.content.clone().unwrap_or_else(|| {
            let mut content = vec![LayoutContent::StyledText {
                owner: parent.id,
                node_id: None,
                text: parent.text.clone(),
            }];
            content.extend(
                parent
                    .children
                    .iter()
                    .map(|child| LayoutContent::Element(*child)),
            );
            content
        });
        let mut result = Vec::new();
        let mut pending = Vec::new();
        let mut flush = |pending: &mut Vec<LayoutContent>, result: &mut Vec<LayoutContent>| {
            let text = pending
                .iter()
                .filter_map(|item| match item {
                    LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect::<String>();
            // Whitespace-only anonymous flex/grid items are not rendered,
            // including preserved ASCII whitespace; NBSP remains content.
            if text
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}'))
            {
                pending.clear();
                return;
            }
            while used.contains(&next_id) {
                next_id -= 1;
            }
            let id = next_id;
            used.insert(id);
            next_id -= 1;
            let mut style = parent_style
                .iter()
                .filter(|(name, _)| {
                    style::properties::PropertyId::parse_enabled_for_all_content(name)
                        .ok()
                        .and_then(|id| id.longhand_id())
                        .is_some_and(|id| id.inherited())
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            style.insert("display".into(), "block".into());
            styles.insert(id, Arc::new(style));
            let content = std::mem::take(pending)
                .into_iter()
                .map(|item| match item {
                    LayoutContent::Text { node_id, text } => LayoutContent::StyledText {
                        owner: parent.id,
                        node_id: Some(node_id),
                        text,
                    },
                    item => item,
                })
                .collect();
            generated.push(LayoutNode {
                tree_scope: parent.tree_scope,
                document_url: parent.document_url.clone(),
                id,
                parent: Some(parent.id),
                children: Vec::new(),
                tag: "oryn-anonymous-text".into(),
                namespace: default_element_namespace(),
                attributes: BTreeMap::new(),
                namespaced_attributes: Vec::new(),
                checked: None,
                disabled: None,
                validity: None,
                indeterminate: false,
                image_intrinsics: None,
                image_document_root: None,
                document_replaced: false,
                intrinsic_image_size: None,
                text,
                content: Some(content),
            });
            owners.insert(id, parent.id);
            result.push(LayoutContent::Element(id));
        };
        for item in content {
            if matches!(item, LayoutContent::Element(_)) {
                flush(&mut pending, &mut result);
                result.push(item);
            } else {
                pending.push(item);
            }
        }
        flush(&mut pending, &mut result);
        parent.children = result
            .iter()
            .filter_map(|item| {
                if let LayoutContent::Element(id) = item {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        parent.content = Some(result);
        parent.text.clear();
    }
    input.nodes.extend(generated);
    owners
}

/// Format a print-media embedded document at its iframe viewport. The owning
/// printed page supplies fragmentation; this subtree keeps its viewport clip.
pub fn compute_print_viewport(
    input: &LayoutInput,
    limits: &LayoutLimits,
) -> Result<LayoutOutput, LayoutError> {
    compute_print_viewport_with_capture_input(input, limits).map(|(output, _)| output)
}

/// Compute a print frame and retain its unprojected native capture context.
pub fn compute_print_viewport_with_capture_input(
    input: &LayoutInput,
    limits: &LayoutLimits,
) -> Result<(LayoutOutput, LayoutInput), LayoutError> {
    compute_print_viewport_with_style_sample(input, limits, None)
}

pub fn compute_print_viewport_with_style_sample(
    input: &LayoutInput,
    limits: &LayoutLimits,
    sample: Option<&svg_instances::StyleSample>,
) -> Result<(LayoutOutput, LayoutInput), LayoutError> {
    let mut input = input.clone();
    input.scroll = ScrollState::default();
    input.element_scrolls.clear();
    input.scrollbar_width = 0.;
    let capture_input = input.clone();
    let (styles, whitespace, pseudos) =
        stylo_computed_values_for_media_with_sample(&input, true, sample)?;
    let (generated, object_hidden) = {
        let (projected, hidden) = object_images::project_styled(&input, &styles);
        (
            generated::materialize_for_media(&projected, styles, &pseudos, true)?,
            hidden,
        )
    };
    drop(input);
    let mut output = compute_with_styles(&generated.input, limits, generated.styles.clone())?;
    generated::finish(&generated, &mut output);
    output.author_whitespace = whitespace;
    output.pseudo_styles = pseudos;
    backdrop::append(&generated.input, limits, &mut output)?;
    object_images::finish(&mut output, &object_hidden);
    output.viewport_scroll = None;
    Ok((output, capture_input))
}

fn compute_with_styles(
    input: &LayoutInput,
    limits: &LayoutLimits,
    computed_styles: ComputedStyles,
) -> Result<LayoutOutput, LayoutError> {
    compute_with_styles_fragmented(input, limits, computed_styles, None)
}
fn compute_with_styles_fragmented(
    input: &LayoutInput,
    limits: &LayoutLimits,
    computed_styles: ComputedStyles,
    fragmentainer_height: Option<f64>,
) -> Result<LayoutOutput, LayoutError> {
    compute_with_styles_fragmented_with_rows(
        input,
        limits,
        computed_styles,
        fragmentainer_height,
        &BTreeSet::new(),
    )
}
fn compute_with_styles_fragmented_with_rows(
    input: &LayoutInput,
    limits: &LayoutLimits,
    computed_styles: ComputedStyles,
    fragmentainer_height: Option<f64>,
    consumed_rows: &BTreeSet<u64>,
) -> Result<LayoutOutput, LayoutError> {
    validate(input, limits)?;
    let viewport_policy = scrollbars::viewport_style(input, &computed_styles);
    let mut layout_styles = computed_styles.clone();
    if let Some((root, source, _)) = &viewport_policy {
        for id in [root, source] {
            if let Some(style) = layout_styles.get(id) {
                let mut style = style.as_ref().clone();
                style.insert("overflow-x".into(), "visible".into());
                style.insert("overflow-y".into(), "visible".into());
                layout_styles.insert(*id, Arc::new(style));
            }
        }
    }
    let mut gutters = layout_styles
        .iter()
        .map(|(id, style)| {
            (
                *id,
                scrollbars::gutter(style, input.scrollbar_width, (false, false)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut viewport_gutter = viewport_policy
        .as_ref()
        .map(|(_, _, style)| scrollbars::gutter(style, input.scrollbar_width, (false, false)))
        .unwrap_or_default();
    // Cross-axis auto scrollbar reservation is monotone within one update.
    for pass in 0..8 {
        let mut output = compute_with_scrollbar_gutters(
            input,
            limits,
            layout_styles.clone(),
            fragmentainer_height,
            consumed_rows,
            &gutters,
            viewport_gutter,
        )?;
        let mut changed = false;
        if input.scrollbar_width > 0. {
            for (id, projection) in &output.paint_projections {
                let Some(scroll) = &projection.scroll else {
                    continue;
                };
                let Some(style) = layout_styles.get(id) else {
                    continue;
                };
                let proposed = scrollbars::gutter(
                    style,
                    input.scrollbar_width,
                    (
                        scroll.scrollable.width > scroll.scrollport.width + 0.01,
                        scroll.scrollable.height > scroll.scrollport.height + 0.01,
                    ),
                );
                let old = gutters.get(id).copied().unwrap_or_default();
                let next = scrollbars::merge(old, proposed);
                if next != old {
                    gutters.insert(*id, next);
                    changed = true;
                }
            }
        }
        if let Some((root, source, style)) = &viewport_policy {
            let port = inset(
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: input.viewport.width,
                    height: input.viewport.height,
                },
                viewport_gutter,
            );
            let mut area = output
                .paint_projections
                .get(root)
                .and_then(|projection| projection.scroll.as_ref())
                .map(|scroll| scroll.scrollable)
                .unwrap_or(port);
            if let Some(projection) = output.paint_projections.get(root) {
                let transform = transforms::Projection::from_matrix(projection.matrix);
                let transformed = transform.project(area);
                if [
                    transformed.x,
                    transformed.y,
                    transformed.width,
                    transformed.height,
                ]
                .iter()
                .all(|value| value.is_finite())
                {
                    let left = area.x.min(transformed.x);
                    let top = area.y.min(transformed.y);
                    area = CssRect {
                        x: left,
                        y: top,
                        width: area.right().max(transformed.right()) - left,
                        height: area.bottom().max(transformed.bottom()) - top,
                    };
                } else {
                    output
                        .diagnostics
                        .push("root transform produced nonfinite viewport overflow".into());
                }
            }
            area.x += input.scroll.x;
            area.y += input.scroll.y;
            let mode = style_value(style, "writing-mode", "horizontal-tb");
            let axes =
                writing_mode::Axes::new(mode, style_value(style, "direction", "ltr") == "rtl");
            let left = if axes.x_reverse {
                area.x.min(port.x)
            } else {
                port.x
            };
            let top = if axes.y_reverse {
                area.y.min(port.y)
            } else {
                port.y
            };
            let right = if axes.x_reverse {
                port.right()
            } else {
                area.right().max(port.right())
            };
            let bottom = if axes.y_reverse {
                port.bottom()
            } else {
                area.bottom().max(port.bottom())
            };
            area = CssRect {
                x: left,
                y: top,
                width: (right - left).max(port.width),
                height: (bottom - top).max(port.height),
            };
            let offset_x = input.scroll.x.clamp(
                (area.x - port.x).min(0.),
                (area.right() - port.right()).max(0.),
            );
            let offset_y = input.scroll.y.clamp(
                (area.y - port.y).min(0.),
                (area.bottom() - port.bottom()).max(0.),
            );
            output.scroll = ScrollState {
                x: offset_x,
                y: offset_y,
                content_width: area.width,
                content_height: area.height,
            };
            output.viewport_scroll = Some(oryn_common::v4::PaintViewportScroll {
                scroll: oryn_common::v4::PaintScrollGeometry {
                    offset_x,
                    offset_y,
                    scrollport: port,
                    scrollable: area,
                },
                style: style.clone(),
            });
            let next = scrollbars::merge(
                viewport_gutter,
                scrollbars::gutter(
                    style,
                    input.scrollbar_width,
                    (
                        area.width > port.width + 0.01,
                        area.height > port.height + 0.01,
                    ),
                ),
            );
            if next != viewport_gutter {
                viewport_gutter = next;
                changed = true;
            }
            // Restore CSSOM authored computed values after internal viewport propagation.
            for id in [root, source] {
                if let Some(style) = computed_styles.get(id) {
                    output.computed_styles.insert(*id, style.clone());
                }
            }
        }
        if !changed {
            return Ok(output);
        }
        if pass == 7 {
            output.diagnostics.push(
                "native scrollbar layout exceeded eight cross-axis stabilization passes".into(),
            );
            return Ok(output);
        }
    }
    unreachable!()
}
fn compute_with_scrollbar_gutters(
    input: &LayoutInput,
    limits: &LayoutLimits,
    mut computed_styles: ComputedStyles,
    fragmentainer_height: Option<f64>,
    consumed_rows: &BTreeSet<u64>,
    scrollbar_gutters: &BTreeMap<u64, EdgeSizes>,
    viewport_gutter: EdgeSizes,
) -> Result<LayoutOutput, LayoutError> {
    validate(input, limits)?;
    let original_input = input;
    let projected;
    let mut anonymous_owners = BTreeMap::new();
    let mut visible_boxless = BTreeSet::new();
    let input = if !input.top_layer.is_empty()
        || input.rendered_tree.is_some()
        || !input.shadow_trees.roots.is_empty()
        || input.nodes.iter().any(|node| {
            node.namespace == "http://www.w3.org/1999/xhtml"
                && matches!(node.tag.as_str(), "canvas" | "select")
        })
        || computed_styles.values().any(|style| {
            matches!(
                style_value(style, "display", "block"),
                "contents" | "flex" | "inline-flex" | "grid" | "inline-grid"
            )
        }) {
        let mut rendered = project_rendered_tree(input);
        let top = rendered
            .top_layer
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.kind,
                    oryn_common::v4::TopLayerKind::SelectPicker
                        | oryn_common::v4::TopLayerKind::TemporalPicker
                        | oryn_common::v4::TopLayerKind::PermissionPrompt
                )
            })
            .map(|entry| entry.node_id)
            .collect::<BTreeSet<_>>();
        for node in &mut rendered.nodes {
            if top.contains(&node.id) {
                node.parent = None;
                if let Some(style) = computed_styles.get_mut(&node.id) {
                    let style = Arc::make_mut(style);
                    if !matches!(
                        style_value(style, "position", "static"),
                        "absolute" | "fixed"
                    ) {
                        style.insert("position".into(), "absolute".into());
                    }
                    if style_value(style, "display", "inline") == "contents" {
                        style.insert("display".into(), "block".into());
                    }
                }
            }
            node.children.retain(|child| !top.contains(child));
            if let Some(content) = &mut node.content {
                content.retain(
                    |item| !matches!(item,LayoutContent::Element(child) if top.contains(child)),
                );
            }
        }
        suppress_canvas_fallback(&mut rendered);
        control_state::project_select_buttons(&mut rendered, &computed_styles);
        let rendered_nodes = rendered
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        for node in &rendered.nodes {
            if node.parent.is_none()
                || !computed_styles.get(&node.id).is_some_and(|style| {
                    style_value(style, "display", "block") == "contents"
                        && style_value(style, "visibility", "visible") == "visible"
                })
            {
                continue;
            }
            let mut current = Some(node.id);
            let mut hidden = false;
            let mut seen = BTreeSet::new();
            while let Some(id) = current {
                if !seen.insert(id) {
                    hidden = true;
                    break;
                }
                if computed_styles.get(&id).is_some_and(|style| {
                    style_value(style, "display", "block") == "none"
                        || style_value(style, "content-visibility", "visible") == "hidden"
                }) {
                    hidden = true;
                    break;
                }
                current = rendered_nodes.get(&id).and_then(|node| node.parent);
            }
            if !hidden {
                visible_boxless.insert(node.id);
            }
        }
        let mut formatting = flatten_contents_boxes(&rendered, &computed_styles);
        anonymous_owners = materialize_anonymous_text_items(&mut formatting, &mut computed_styles);
        projected = formatting;
        &projected
    } else {
        input
    };
    let (web_fonts, loaded_fonts, mut diagnostics) = prepare_web_fonts(input, limits);
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let roots = input
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .map(|node| node.id)
        .collect::<Vec<_>>();
    let mut geometry = BTreeMap::new();
    let mut offset_dimensions = BTreeMap::new();
    // Inline/flex/grid collectors omit display:none children from flow. Publish
    // their native hidden geometry even when layout_node never visits them.
    // Use the computed cascade and the rendered child graph; visible boxless
    // display:contents nodes do not enter this set.
    let mut hidden = by_id
        .keys()
        .copied()
        .filter(|id| {
            computed_styles
                .get(id)
                .is_some_and(|style| style_value(style, "display", "inline") == "none")
        })
        .collect::<Vec<_>>();
    let mut hidden_seen = BTreeSet::new();
    while let Some(id) = hidden.pop() {
        if !hidden_seen.insert(id) {
            continue;
        }
        geometry.insert(id, hidden_geometry());
        offset_dimensions.insert(id, (0., 0.));
        if let Some(node) = by_id.get(&id) {
            hidden.extend(node.children.iter().copied());
        }
    }
    for node in &original_input.nodes {
        if !by_id.contains_key(&node.id) {
            // A visible display:contents wrapper has no principal box; it is
            // not a hidden semantic node. Excluded rendered nodes stay hidden.
            if !visible_boxless.contains(&node.id) {
                geometry.insert(node.id, hidden_geometry());
            }
            offset_dimensions.insert(node.id, (0., 0.));
        }
    }
    let mut resolved_values = BTreeMap::new();
    let mut text_fragments = BTreeMap::new();
    let mut text_engine = TextEngine::new(&web_fonts);
    text_engine.scrollbar_gutters = scrollbar_gutters.clone();
    text_engine.decoration_styles = text_decoration::propagate(input, &computed_styles);
    // Display-contents boxes still decorate descendants after their own boxes
    // have been removed. Retain propagation over the rendered tree as well.
    text_engine
        .decoration_styles
        .extend(text_decoration::propagate(
            &project_rendered_tree(original_input),
            &computed_styles,
        ));
    text_engine.consumed_table_rows = consumed_rows.clone();
    text_engine.languages = effective_languages(original_input)
        .into_iter()
        .map(|(id, language)| {
            (
                id,
                language.and_then(|language| fontique::Language::parse(&language).ok()),
            )
        })
        .collect();
    permission_controls::measure(original_input, &mut computed_styles, &mut text_engine);
    control_state::measure_selects(original_input, &computed_styles, &mut text_engine);
    let mut cursor_y = 0.0;
    for root in roots {
        let height = layout_node(
            root,
            FlowBox {
                x: viewport_gutter.left,
                y: if input.top_layer.iter().any(|entry| {
                    entry.node_id == root
                        && !matches!(
                            entry.kind,
                            oryn_common::v4::TopLayerKind::SelectPicker
                                | oryn_common::v4::TopLayerKind::TemporalPicker
                                | oryn_common::v4::TopLayerKind::PermissionPrompt
                        )
                }) {
                    viewport_gutter.top
                } else {
                    cursor_y + viewport_gutter.top
                },
                width: (input.viewport.width - viewport_gutter.left - viewport_gutter.right)
                    .max(0.),
                height: Some(
                    (input.viewport.height - viewport_gutter.top - viewport_gutter.bottom).max(0.),
                ),
            },
            &by_id,
            &computed_styles,
            &mut geometry,
            &mut offset_dimensions,
            &mut resolved_values,
            &mut text_fragments,
            input,
            false,
            &mut diagnostics,
            &mut text_engine,
            None,
        );
        if !input.top_layer.iter().any(|entry| {
            entry.node_id == root
                && !matches!(
                    entry.kind,
                    oryn_common::v4::TopLayerKind::SelectPicker
                        | oryn_common::v4::TopLayerKind::TemporalPicker
                        | oryn_common::v4::TopLayerKind::PermissionPrompt
                )
        }) {
            cursor_y += height;
        }
    }
    // Out-of-flow descendants are formatted only after their ancestor used
    // padding boxes exist. This also prevents their sizes contributing to an
    // auto-height normal-flow ancestor. Nested positioned roots append here.
    let anchor_styles = computed_styles.clone();
    let anchor_dependencies = anchor_styles
        .iter()
        .filter(|(_, style)| anchor_positioning::needed(style))
        .map(|(id, _)| {
            (
                *id,
                anchor_positioning::dependencies(*id, input, &anchor_styles),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let cyclic_anchors = anchor_positioning::cycles(&anchor_dependencies);
    let mut deferred_index = 0;
    let mut anchor_work = 0usize;
    while deferred_index < text_engine.out_of_flow.len() {
        let ready = (deferred_index..text_engine.out_of_flow.len()).find(|index| {
            anchor_dependencies
                .get(&text_engine.out_of_flow[*index].0)
                .is_none_or(|dependencies| {
                    dependencies
                        .iter()
                        .all(|dependency| geometry.contains_key(dependency))
                })
        });
        let unresolved = ready.is_none();
        let ready = ready.unwrap_or(deferred_index);
        text_engine.out_of_flow.swap(deferred_index, ready);
        let (id, static_box) = text_engine.out_of_flow[deferred_index];
        deferred_index += 1;
        let fixed = style_value(&computed_styles[&id], "position", "static") == "fixed";
        let cb = positioning::containing_box(
            id,
            fixed,
            input,
            &by_id,
            &computed_styles,
            &geometry,
            viewport_gutter,
        );
        if anchor_dependencies.contains_key(&id) {
            anchor_work = anchor_work.saturating_add(geometry.len());
            if anchor_work > 4_000_000 {
                return Err(LayoutError::StyleTraversal(
                    "anchor projection work exceeds native bound".into(),
                ));
            }
            let mut projections = BTreeMap::new();
            if !unresolved && !cyclic_anchors.contains(&id) {
                let mut projected_geometry = geometry.clone();
                let mut projected_text = text_fragments.clone();
                let mut projected_values = resolved_values.clone();
                positioning::normalize_fixed(
                    input,
                    &by_id,
                    &computed_styles,
                    &mut projected_geometry,
                    &mut projected_text,
                    &mut BTreeMap::new(),
                );
                let viewport_scrollport = input
                    .nodes
                    .iter()
                    .any(|node| node.parent.is_none() && node.tag == "html")
                    .then(|| {
                        inset(
                            CssRect {
                                x: 0.,
                                y: 0.,
                                width: input.viewport.width,
                                height: input.viewport.height,
                            },
                            viewport_gutter,
                        )
                    });
                project_layout_tree(LayoutTreeProjectionInputs {
                    input,
                    by_id: &by_id,
                    styles: &computed_styles,
                    geometry: &mut projected_geometry,
                    text_fragments: &mut projected_text,
                    resolved_values: &mut projected_values,
                    paint_projections: &mut projections,
                    scrollbar_gutters,
                    viewport_scrollport,
                    diagnostics: &mut diagnostics,
                });
            } else {
                diagnostics.push(format!(
                    "node {id}: unavailable or cyclic anchor uses CSS fallback"
                ));
            }
            let candidates =
                anchor_positioning::candidates(id, input, &anchor_styles, &projections, cb);
            let base = candidates[0].clone();
            let order = style_value(&anchor_styles[&id], "position-try-order", "normal");
            let axes = anchor_positioning::containing_axes(id, input, &anchor_styles);
            let mut indices = (0..candidates.len()).collect::<Vec<_>>();
            if order != "normal" {
                indices.sort_by(|a, b| {
                    let score = |index: usize| {
                        let (w, h) = anchor_positioning::available(&candidates[index], cb, input);
                        match order {
                            "most-width" => w,
                            "most-height" => h,
                            "most-inline-size" => {
                                if axes.vertical {
                                    h
                                } else {
                                    w
                                }
                            }
                            _ => {
                                if axes.vertical {
                                    w
                                } else {
                                    h
                                }
                            }
                        }
                    };
                    score(*b).total_cmp(&score(*a))
                });
            }
            let mut selected = base;
            if candidates.len() > 1 {
                for index in indices {
                    anchor_work = anchor_work.saturating_add(input.nodes.len() + geometry.len());
                    if anchor_work > 4_000_000 {
                        return Err(LayoutError::StyleTraversal(
                            "anchor fallback formatting exceeds native bound".into(),
                        ));
                    }
                    let mut trial_styles = computed_styles.clone();
                    trial_styles.insert(id, candidates[index].clone());
                    let mut trial_geometry = geometry.clone();
                    let mut trial_text = BTreeMap::new();
                    let mut trial_values = BTreeMap::new();
                    let mut trial_offsets = BTreeMap::new();
                    let mut trial_diagnostics = Vec::new();
                    let mut trial = TextEngine::new(&web_fonts);
                    trial.active_out_of_flow = Some(id);
                    trial.languages = text_engine.languages.clone();
                    trial.scrollbar_gutters = text_engine.scrollbar_gutters.clone();
                    trial.decoration_styles = text_engine.decoration_styles.clone();
                    trial.select_sizes = text_engine.select_sizes.clone();
                    trial.consumed_table_rows = text_engine.consumed_table_rows.clone();
                    layout_node(
                        id,
                        FlowBox {
                            x: static_box.x,
                            y: static_box.y,
                            width: cb.width,
                            height: Some(cb.height),
                        },
                        &by_id,
                        &trial_styles,
                        &mut trial_geometry,
                        &mut trial_offsets,
                        &mut trial_values,
                        &mut trial_text,
                        input,
                        false,
                        &mut trial_diagnostics,
                        &mut trial,
                        None,
                    );
                    positioning::resolve_one(positioning::PositionedNodeInputs {
                        id,
                        input,
                        by_id: &by_id,
                        styles: &trial_styles,
                        geometry: &mut trial_geometry,
                        text: &mut trial_text,
                        clusters: &mut trial.source_geometry,
                        viewport_gutter,
                    });
                    if let Some(error) = trial.font_demand_error {
                        return Err(error);
                    }
                    if trial_geometry
                        .get(&id)
                        .and_then(|value| value.box_model.as_ref())
                        .is_some_and(|model| {
                            model.border_box.x - model.margin.left >= cb.x
                                && model.border_box.y - model.margin.top >= cb.y
                                && model.border_box.right() + model.margin.right <= cb.right()
                                && model.border_box.bottom() + model.margin.bottom <= cb.bottom()
                        })
                    {
                        selected = candidates[index].clone();
                        break;
                    }
                }
            }
            computed_styles.insert(id, selected);
        }
        text_engine.active_out_of_flow = Some(id);
        layout_node(
            id,
            FlowBox {
                x: static_box.x,
                y: static_box.y,
                width: cb.width,
                height: Some(cb.height),
            },
            &by_id,
            &computed_styles,
            &mut geometry,
            &mut offset_dimensions,
            &mut resolved_values,
            &mut text_fragments,
            input,
            false,
            &mut diagnostics,
            &mut text_engine,
            None,
        );
        let before = geometry
            .get(&id)
            .and_then(|value| value.box_model.as_ref())
            .map(|model| model.border_box);
        positioning::resolve_one(positioning::PositionedNodeInputs {
            id,
            input,
            by_id: &by_id,
            styles: &computed_styles,
            geometry: &mut geometry,
            text: &mut text_fragments,
            clusters: &mut text_engine.source_geometry,
            viewport_gutter,
        });
        if let (Some(before), Some(after)) = (
            before,
            geometry
                .get(&id)
                .and_then(|value| value.box_model.as_ref())
                .map(|model| model.border_box),
        ) {
            // Positioned parents move deferred descendants' static positions,
            // including break provenance, before those descendants are laid out.
            let (dx, dy) = (after.x - before.x, after.y - before.y);
            let mut descendants = BTreeSet::new();
            let mut pending = by_id[&id].children.clone();
            while let Some(child) = pending.pop() {
                if descendants.insert(child) {
                    pending.extend(by_id[&child].children.iter().copied());
                }
            }
            for (child, anchor) in text_engine.out_of_flow.iter_mut().skip(deferred_index) {
                if descendants.contains(child) {
                    anchor.x += dx;
                    anchor.y += dy;
                }
            }
            for line_break in text_engine.break_boxes.values_mut() {
                if descendants.contains(&line_break.layout_node) || line_break.layout_node == id {
                    line_break.before.x += dx;
                    line_break.before.y += dy;
                    if let Some(after) = &mut line_break.after {
                        after.x += dx;
                        after.y += dy;
                    }
                }
            }
        }
    }
    text_engine.active_out_of_flow = None;
    tables::align_contents(
        &by_id,
        &computed_styles,
        &mut geometry,
        &mut text_fragments,
        &mut text_engine,
    );
    for (id, line_break) in &text_engine.break_boxes {
        let rect = line_break.before;
        geometry.insert(
            *id,
            NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: rect,
                    padding_box: rect,
                    content_box: rect,
                    margin: EdgeSizes::default(),
                    padding: EdgeSizes::default(),
                    border: EdgeSizes::default(),
                }),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: false,
                diagnostic: None,
            },
        );
        offset_dimensions.insert(*id, (rect.width, rect.height));
    }

    positioning::resolve(
        input,
        &by_id,
        &computed_styles,
        &mut geometry,
        &mut text_fragments,
        &mut text_engine.source_geometry,
        viewport_gutter,
    );
    list_markers::place_outside(
        &by_id,
        &computed_styles,
        &mut geometry,
        &mut text_fragments,
        &mut text_engine,
    );
    if let Some(height) = fragmentainer_height {
        print::fragment_flow(
            height,
            &by_id,
            &computed_styles,
            &mut geometry,
            &mut text_fragments,
            &mut text_engine,
        )?;
    }
    tables::column_geometry(
        &text_engine.collapsed_tables,
        &mut geometry,
        &mut offset_dimensions,
    );
    // A transform can extend scroll overflow, but must not shrink the original
    // layout overflow when it moves or scales a box toward the origin.
    let layout_width = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|model| model.border_box.right() + input.scroll.x)
        .fold(input.viewport.width, f64::max);
    let layout_height = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|model| model.border_box.bottom() + input.scroll.y)
        .fold(input.viewport.height, f64::max);
    // Read anonymous participants in their rendered parent's child order.
    // Their synthetic IDs count down and therefore sort opposite to DOM order.
    let anonymous_order = input
        .nodes
        .iter()
        .flat_map(|node| node.children.iter())
        .filter_map(|id| anonymous_owners.get(id).map(|parent| (*id, *parent)))
        .collect::<Vec<_>>();
    for (anonymous, parent) in &anonymous_order {
        if let Some(fragments) = text_fragments.remove(anonymous) {
            text_fragments.entry(*parent).or_default().extend(fragments);
        }
        for clusters in text_engine.source_geometry.values_mut() {
            for cluster in clusters {
                if cluster.layout_node == *anonymous {
                    cluster.layout_node = *parent;
                }
            }
        }
    }
    let mut paint_projections = BTreeMap::new();
    let viewport_scrollport = input
        .nodes
        .iter()
        .any(|node| node.parent.is_none() && node.tag == "html")
        .then(|| {
            inset(
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: input.viewport.width,
                    height: input.viewport.height,
                },
                viewport_gutter,
            )
        });
    project_layout_tree(LayoutTreeProjectionInputs {
        input,
        by_id: &by_id,
        styles: &computed_styles,
        geometry: &mut geometry,
        text_fragments: &mut text_fragments,
        resolved_values: &mut resolved_values,
        paint_projections: &mut paint_projections,
        scrollbar_gutters,
        viewport_scrollport,
        diagnostics: &mut diagnostics,
    });
    anchor_positioning::visibility(
        input,
        &anchor_styles,
        &mut computed_styles,
        &mut geometry,
        &paint_projections,
        viewport_gutter,
    );
    tables::project_borders(
        &text_engine.collapsed_tables,
        &by_id,
        &computed_styles,
        &mut paint_projections,
    );
    columns::project_rules(
        &text_engine.column_runs,
        &computed_styles,
        input.viewport,
        &mut paint_projections,
    );
    let content_width = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|item| item.border_box.right() + input.scroll.x)
        .fold(layout_width, f64::max);
    let content_height = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|item| item.border_box.bottom() + input.scroll.y)
        .fold(layout_height, f64::max);
    let scroll = ScrollState {
        x: input
            .scroll
            .x
            .clamp(0.0, (content_width - input.viewport.width).max(0.0)),
        y: input
            .scroll
            .y
            .clamp(0.0, (content_height - input.viewport.height).max(0.0)),
        content_width,
        content_height,
    };
    apply_ancestor_clipping(
        &by_id,
        &computed_styles,
        &mut geometry,
        CssRect {
            x: 0.0,
            y: 0.0,
            width: input.viewport.width,
            height: input.viewport.height,
        },
    );
    svg_geometry::project(
        original_input,
        &computed_styles,
        limits,
        &mut geometry,
        &mut paint_projections,
    )
    .map_err(LayoutError::StyleTraversal)?;
    for clusters in text_engine.source_geometry.values_mut() {
        for cluster in clusters {
            if let Some(projection) = paint_projections.get(&cluster.layout_node) {
                cluster.matrix = projection.text_matrix.unwrap_or(projection.matrix);
            }
        }
    }
    for anonymous in anonymous_owners.keys() {
        geometry.remove(anonymous);
        offset_dimensions.remove(anonymous);
        resolved_values.remove(anonymous);
        computed_styles.remove(anonymous);
        paint_projections.remove(anonymous);
    }
    let caret_stops = empty_caret_stops(
        original_input,
        &computed_styles,
        &paint_projections,
        &mut text_engine,
    );
    let mut control_states = original_input
        .nodes
        .iter()
        .filter_map(|node| control_paint_state(node).map(|state| (node.id, state)))
        .collect::<BTreeMap<_, _>>();
    control_states.extend(original_input.control_states.clone());
    // Closed native button labels have paint provenance but no DOM text node
    // offsets. The real selectedcontent descendants retain their own clusters.
    for (id, state) in &control_states {
        if matches!(
            state,
            oryn_common::v4::PaintControl::Select {
                multiple: false,
                display_size: 1,
                ..
            }
        ) {
            text_engine.source_geometry.remove(id);
            text_engine.resolved_text.remove(id);
        }
    }
    if let Some(error) = text_engine.font_demand_error {
        return Err(error);
    }
    // Reject before allocating the flattened publication vector. Existing run
    // ownership stays in the engine until the complete preflight succeeds.
    if resolved_publication_bytes(text_engine.resolved_text.values().flatten().map(|run| {
        (run.text.len(), run.spans.len(), run.aliases.len())
    }))
        .is_none_or(|bytes| bytes > 64 * 1024 * 1024)
    {
        return Err(LayoutError::StyleTraversal(
            "resolved text provenance exceeds native 64-MiB bound".into(),
        ));
    }
    let resolved_text = text_engine
        .resolved_text
        .into_values()
        .flatten()
        .collect::<Vec<_>>();
    let mut font_demands = BTreeMap::<String, BTreeSet<u32>>::new();
    for demands in text_engine.font_demands.values() {
        for (family, characters) in demands {
            font_demands
                .entry(family.clone())
                .or_default()
                .extend(characters);
        }
    }
    Ok(LayoutOutput {
        font_demands,
        resolved_text,
        css_image_requests: Default::default(),
        generated_image_urls: Default::default(),
        backdrops: Default::default(),
        paint_parent_overrides: Default::default(),
        generated_nodes: Vec::new(),
        generated_content: Vec::new(),
        control_states,
        pseudo_styles: BTreeMap::new(),
        viewport_scroll: None,
        caret_stops,
        resolved_values,
        computed_styles,
        author_whitespace: BTreeSet::new(),
        geometry,
        offset_dimensions,
        text_fragments,
        paint_fonts: text_engine.paint_fonts,
        text_clusters: text_engine.source_geometry,
        paint_projections,
        scroll,
        diagnostics,
        loaded_fonts,
    })
}

/// Empty element boundaries have real font line metrics but no text glyphs.
/// Shape a preserved space solely to obtain the selected font's line box; do
/// not add that probe to paint text or native UTF16 source clusters.
fn empty_caret_stops(
    input: &LayoutInput,
    styles: &ComputedStyles,
    projections: &BTreeMap<u64, oryn_common::v4::PaintProjection>,
    engine: &mut TextEngine,
) -> Vec<oryn_common::v4::CaretStopGeometry> {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut stops = Vec::new();
    for node in &input.nodes {
        let Some(projection) = projections.get(&node.id) else {
            continue;
        };
        let Some(style) = styles.get(&node.id) else {
            continue;
        };
        if style_value(style, "visibility", "visible") != "visible"
            || style_value(style, "-oryn-anchor-hidden", "false") == "true"
        {
            continue;
        }
        let kind = node
            .attributes
            .get("type")
            .map(|kind| kind.to_ascii_lowercase())
            .unwrap_or_default();
        let text_control = node.namespace == "http://www.w3.org/1999/xhtml"
            && (node.tag == "textarea"
                || (node.tag == "input"
                    && !matches!(
                        kind.as_str(),
                        "hidden"
                            | "button"
                            | "submit"
                            | "reset"
                            | "image"
                            | "checkbox"
                            | "radio"
                            | "range"
                            | "color"
                            | "file"
                            | "date"
                            | "month"
                            | "week"
                            | "time"
                            | "datetime-local"
                    )));
        let content = input
            .rendered_tree
            .as_ref()
            .and_then(|tree| tree.get(&node.id))
            .or(node.content.as_ref());
        let empty = (text_control || node.children.is_empty())
            && content.map_or(
                if text_control && node.tag == "input" {
                    node.attributes.get("value").is_none_or(String::is_empty)
                } else {
                    node.text.is_empty()
                },
                |items| {
                    items.iter().all(|item| match item {
                        LayoutContent::Text { text, .. }
                        | LayoutContent::StyledText { text, .. } => text.is_empty(),
                        _ => false,
                    })
                },
            );
        let br = node.tag == "br";
        if br {
            let Some(line_break) = engine.break_boxes.get(&node.id) else {
                continue;
            };
            let Some(parent) = node.parent.and_then(|id| nodes.get(&id)) else {
                continue;
            };
            let index = parent
                .content
                .as_ref()
                .and_then(|items| {
                    items
                        .iter()
                        .position(|item| matches!(item,LayoutContent::Element(id) if *id==node.id))
                })
                .or_else(|| parent.children.iter().position(|id| *id == node.id))
                .unwrap_or(0);
            let before = projection.box_model.content_box;
            for (offset, line, rect) in std::iter::once((index, line_break.line, before)).chain(
                line_break.after.map(|after| {
                    (
                        index + 1,
                        line_break.line + 1,
                        CssRect {
                            x: before.x + after.x - line_break.before.x,
                            y: before.y + after.y - line_break.before.y,
                            ..after
                        },
                    )
                }),
            ) {
                stops.push(oryn_common::v4::CaretStopGeometry {
                    boundary_node: parent.id,
                    offset,
                    layout_node: line_break.layout_node,
                    rect,
                    matrix: projection.text_matrix.unwrap_or(projection.matrix),
                    line,
                    rtl: line_break.rtl,
                });
            }
            continue;
        }

        if !br
            && (!empty
                || (!text_control
                    && !matches!(
                        style_value(style, "display", "block"),
                        "block" | "flow-root" | "list-item" | "inline-block" | "table-cell"
                    )))
        {
            continue;
        }
        let font = length(
            style_value(style, "font-size", "16px"),
            16.,
            16.,
            input.viewport,
        );
        let mut probe_style = style.as_ref().clone();
        probe_style.insert("white-space".into(), "pre".into());
        probe_style.insert("white-space-collapse".into(), "preserve".into());
        probe_style.insert("text-wrap-mode".into(), "nowrap".into());
        let rect = projection.box_model.content_box;
        let mode = style_value(style, "writing-mode", "horizontal-tb");
        let axes = writing_mode::Axes::new(mode, style_value(style, "direction", "ltr") == "rtl");
        let probe = engine.shape_styled(
            " ",
            if axes.vertical {
                rect.height
            } else {
                rect.width
            },
            font,
            &probe_style,
            input.viewport,
        );
        let extent = probe.lines.first().map(|line| line.2).unwrap_or(0.);
        if extent <= 0. || !extent.is_finite() {
            continue;
        }
        let caret = if axes.vertical {
            CssRect {
                x: if axes.x_reverse {
                    rect.right() - extent
                } else {
                    rect.x
                },
                y: if axes.y_reverse {
                    rect.bottom()
                } else {
                    rect.y
                },
                width: extent,
                height: 0.,
            }
        } else {
            let align = style_value(style, "text-align", "start");
            let x = match align {
                "center" => rect.x + rect.width / 2.,
                "right" => rect.right(),
                "left" => rect.x,
                "end" => {
                    if axes.x_reverse {
                        rect.x
                    } else {
                        rect.right()
                    }
                }
                _ => {
                    if axes.x_reverse {
                        rect.right()
                    } else {
                        rect.x
                    }
                }
            };
            CssRect {
                x,
                y: rect.y,
                width: 0.,
                height: extent,
            }
        };
        let (boundary_node, offset) = (node.id, 0);
        stops.push(oryn_common::v4::CaretStopGeometry {
            boundary_node,
            offset,
            layout_node: node.id,
            rect: caret,
            matrix: projection.text_matrix.unwrap_or(projection.matrix),
            line: 0,
            rtl: style_value(style, "direction", "ltr") == "rtl",
        });
    }
    stops
}

/// Compose ancestors before projecting bounds, so rotations do not repeatedly
/// inflate axis-aligned boxes. Flow and offset dimensions remain untransformed.
fn project_layout_tree(params: LayoutTreeProjectionInputs<'_>) {
    let LayoutTreeProjectionInputs {
        input,
        by_id,
        styles,
        geometry,
        text_fragments,
        resolved_values,
        paint_projections,
        scrollbar_gutters,
        viewport_scrollport,
        diagnostics,
    } = params;

    let flow_boxes = geometry
        .iter()
        .filter_map(|(id, g)| g.box_model.clone().map(|model| (*id, model)))
        .collect::<BTreeMap<_, _>>();
    let scroll_metrics = scroll_geometry::collect(
        input,
        by_id,
        styles,
        &flow_boxes,
        text_fragments,
        scrollbar_gutters,
        diagnostics,
    );
    let mut pending = input
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .map(|node| {
            (
                node.id,
                transforms::Projection::IDENTITY,
                Vec::<oryn_common::v4::PaintClip>::new(),
            )
        })
        .collect::<Vec<_>>();
    let mut containing_states =
        BTreeMap::<u64, (transforms::Projection, Vec<oryn_common::v4::PaintClip>)>::new();
    let viewport = CssRect {
        x: 0.,
        y: 0.,
        width: input.viewport.width,
        height: input.viewport.height,
    };
    let viewport_clips = viewport_scrollport
        .map(|rect| oryn_common::v4::PaintClip {
            shape: None,
            owner: None,
            scroll_x: true,
            scroll_y: true,
            matrix: transforms::Projection::IDENTITY.matrix(),
            rect,
            clip_x: true,
            clip_y: true,
        })
        .into_iter()
        .collect::<Vec<_>>();
    while let Some((id, parent, clips)) = pending.pop() {
        let Some(node) = by_id.get(&id) else {
            continue;
        };
        // Viewport-fixed descendants escape intervening scroll/overflow clips.
        // A transformed/contained ancestor instead establishes their local CB.
        let viewport_fixed = positioning::viewport_fixed(id, by_id, styles);
        let position = styles
            .get(&id)
            .map(|style| style_value(style, "position", "static"))
            .unwrap_or("static");
        let mut containing = None;
        if matches!(position, "absolute" | "fixed") && !viewport_fixed {
            let mut ancestor = node.parent;
            while let Some(candidate) = ancestor {
                if styles
                    .get(&candidate)
                    .is_some_and(|style| positioning::establishes(style, position == "fixed"))
                {
                    containing = containing_states.get(&candidate).cloned();
                    break;
                }
                ancestor = by_id.get(&candidate).and_then(|node| node.parent);
            }
            // Initial-containing-block positioned boxes escape ordinary DOM
            // scroll/clip ancestors. The final viewport still bounds painting.
            if containing.is_none() {
                containing = Some((transforms::Projection::IDENTITY, viewport_clips.clone()));
            }
        }
        let (parent, clips) = containing.unwrap_or((parent, clips));
        let parent = if viewport_fixed {
            transforms::Projection::IDENTITY
        } else {
            parent
        };
        let (sticky_x, sticky_y) = sticky::translation(
            id,
            input,
            by_id,
            styles,
            &flow_boxes,
            &scroll_metrics,
            viewport_scrollport,
        );
        let parent = parent.then(transforms::Projection::translation(sticky_x, sticky_y));
        let mut clips = if viewport_fixed {
            viewport_clips.clone()
        } else {
            clips
        };
        let mut child_clips = clips.clone();
        if node.parent.is_none() {
            child_clips.extend(viewport_clips.iter().cloned());
        }
        let mut projection = parent;
        let mut perspective = transforms::Projection::IDENTITY;
        let mut scroll_translation = transforms::Projection::IDENTITY;
        let applicable = styles
            .get(&id)
            .is_some_and(|style| transformable_box(node, style));
        if let (Some(style), Some(value)) = (styles.get(&id), geometry.get_mut(&id))
            && let Some(model) = value.box_model.as_mut()
        {
            let reference = model.border_box;
            let font = length(
                style_value(style, "font-size", "16px"),
                16.,
                16.,
                input.viewport,
            );
            let depth = style_value(style, "perspective", "none");
            if applicable
                && depth != "none"
                && let Some(value) = transforms::prepare(
                    reference,
                    &format!("perspective({depth})"),
                    style_value(style, "perspective-origin", "50% 50%"),
                    font,
                    input.viewport,
                )
            {
                perspective = value;
            }
            let source = transforms::compose_source(
                style_value(style, "translate", "none"),
                style_value(style, "rotate", "none"),
                style_value(style, "scale", "none"),
                style_value(style, "transform", "none"),
            );
            // For elements with CSS layout boxes, fill-box maps to the
            // content box; stroke-box and view-box map to the border box.
            let transform_reference = match style_value(style, "transform-box", "view-box") {
                "content-box" | "fill-box" => model.content_box,
                _ => reference,
            };
            if applicable {
                for (property, reference) in [
                    ("transform-origin", transform_reference),
                    ("perspective-origin", reference),
                ] {
                    if let Some(value) = transforms::resolved_origin(
                        reference,
                        style_value(style, property, "50% 50%"),
                        font,
                        input.viewport,
                    ) {
                        resolved_values
                            .entry(id)
                            .or_default()
                            .insert(property.into(), value);
                    }
                }
            }
            if let Some(value) = transforms::resolved_value(
                transform_reference,
                style_value(style, "transform", "none"),
                font,
                input.viewport,
            ) && value != "none"
            {
                resolved_values
                    .entry(id)
                    .or_default()
                    .insert("transform".into(), value);
            }
            if let Some(local) = source.filter(|_| applicable).and_then(|source| {
                transforms::prepare(
                    transform_reference,
                    &source,
                    style_value(style, "transform-origin", "50% 50%"),
                    font,
                    input.viewport,
                )
            }) {
                projection = parent.then(local);
            }
            let source = style_value(style, "clip-path", "none");
            if source != "none" {
                let (shape_source, reference) = if let Some(close) = source.rfind(')') {
                    let suffix = source[close + 1..].trim();
                    let reference = match suffix {
                        "padding-box" => model.padding_box,
                        "content-box" | "fill-box" => model.content_box,
                        _ => model.border_box,
                    };
                    (&source[..=close], reference)
                } else {
                    (
                        "inset(0px)",
                        match source {
                            "padding-box" => model.padding_box,
                            "content-box" | "fill-box" => model.content_box,
                            _ => model.border_box,
                        },
                    )
                };
                let shape_result = if shape_source.starts_with("url(") {
                    svg_clip::resolve(input, node, shape_source, reference, styles).map(Some)
                } else {
                    oryn_common::basic_shape::parse(
                        shape_source,
                        reference,
                        font,
                        (input.viewport.width, input.viewport.height),
                    )
                };
                let shape = match shape_result {
                    Ok(Some(shape)) => shape,
                    Ok(None) => unreachable!(),
                    Err(error) => {
                        diagnostics.push(format!("node {id}: {error}"));
                        oryn_common::basic_shape::Shape::RoundedRect {
                            rect: oryn_common::v4::CssRect {
                                x: reference.x,
                                y: reference.y,
                                width: 0.,
                                height: 0.,
                            },
                            radii: [[0.; 2]; 4],
                        }
                    }
                };
                let clip = oryn_common::v4::PaintClip {
                    shape: Some(shape.clone()),
                    owner: Some(id),
                    scroll_x: false,
                    scroll_y: false,
                    matrix: projection.matrix(),
                    rect: shape.bounds(),
                    clip_x: true,
                    clip_y: true,
                };
                clips.push(clip.clone());
                child_clips.push(clip);
            }
            let clip_x = style_value(style, "overflow-x", "visible") != "visible";
            let clip_y = style_value(style, "overflow-y", "visible") != "visible";
            if clip_x || clip_y {
                child_clips.push(oryn_common::v4::PaintClip {
                    shape: None,
                    owner: Some(id),
                    scroll_x: matches!(
                        style_value(style, "overflow-x", "visible"),
                        "auto" | "scroll" | "hidden"
                    ),
                    scroll_y: matches!(
                        style_value(style, "overflow-y", "visible"),
                        "auto" | "scroll" | "hidden"
                    ),
                    matrix: projection.matrix(),
                    rect: scroll_metrics
                        .get(&id)
                        .map(|scroll| scroll.scrollport)
                        .unwrap_or(model.padding_box),
                    clip_x,
                    clip_y,
                });
            }
            let (scroll_x, scroll_y) = scroll_metrics
                .get(&id)
                .map(|scroll| (scroll.offset_x, scroll.offset_y))
                .unwrap_or_default();
            // Clip and visible boxes are not programmatically scrollable.
            // Scroll offsets are expressed in the container's local axes.
            let scrollable = |axis| {
                matches!(
                    style_value(style, axis, "visible"),
                    "auto" | "scroll" | "hidden"
                )
            };
            scroll_translation = transforms::Projection::translation(
                if scrollable("overflow-x") {
                    -scroll_x
                } else {
                    0.
                },
                if scrollable("overflow-y") {
                    -scroll_y
                } else {
                    0.
                },
            );
            let text_projection = projection.then(scroll_translation);
            paint_projections.insert(
                id,
                oryn_common::v4::PaintProjection {
                    scroll: scroll_metrics.get(&id).cloned(),
                    fragment_boxes: if value.client_rects.len() > 1 {
                        inline_formatting::fragment_models(&value.client_rects, model, style)
                    } else {
                        Vec::new()
                    },
                    border_segments: Vec::new(),
                    border_edges: None,
                    matrix: projection.matrix(),
                    box_model: model.clone(),
                    clips,
                    text_matrix: Some(text_projection.matrix()),
                    text_clips: child_clips.clone(),
                    text: text_fragments.get(&id).cloned().unwrap_or_default(),
                },
            );
            if let Some(bounds) =
                oryn_common::layout_domain::projected_bounds(projection.matrix(), model.border_box)
            {
                if bounds.saturated {
                    diagnostics.push(format!(
                        "node {id}: {} projected bounds saturated to native coordinate domain",
                        if bounds.unbounded {
                            "mathematically unbounded"
                        } else {
                            "out-of-domain"
                        }
                    ));
                }
            } else {
                diagnostics.push(format!("node {id}: nonfinite native projection rejected"));
            }
            model.border_box = projection.project(model.border_box);
            model.padding_box = projection.project(model.padding_box);
            model.content_box = projection.project(model.content_box);
            for fragment in &mut value.client_rects {
                *fragment = projection.project(*fragment);
            }
            if let Some(fragments) = text_fragments.get_mut(&id) {
                for fragment in fragments {
                    fragment.rect = text_projection.project(fragment.rect);
                    fragment.flow_rect =
                        fragment.flow_rect.map(|flow| text_projection.project(flow));
                    for cluster in &mut fragment.clusters {
                        cluster.rect = text_projection.project(cluster.rect);
                    }
                }
            }
            value.in_viewport = value.availability == GeometryAvailability::Available
                && model.border_box.intersects(viewport);
            value.interactable = value.in_viewport
                && style_value(style, "opacity", "1")
                    .parse::<f64>()
                    .unwrap_or(1.)
                    > 0.
                && style_value(style, "pointer-events", "auto") != "none";
        }
        let preserve = styles.get(&id).is_some_and(|style| preserves_3d(style));
        // Even a non-transformable inline intermediate flattens descendants:
        // its parent's perspective applies to direct children, not every later
        // transformable descendant in the DOM subtree.
        let children = (if preserve && applicable {
            projection
        } else {
            projection.flatten_children()
        })
        .then(perspective)
        .then(scroll_translation);
        containing_states.insert(id, (children, child_clips.clone()));
        pending.extend(
            node.children
                .iter()
                .rev()
                .map(|child| (*child, children, child_clips.clone())),
        );
    }
}

fn is_replaced_box(node: &LayoutNode) -> bool {
    if node.tag == "object"
        && node.namespace == "http://www.w3.org/1999/xhtml"
        && node.intrinsic_image_size.is_none()
        && !node.document_replaced
    {
        return false;
    }
    matches!(
        node.tag.as_str(),
        "img"
            | "video"
            | "audio"
            | "canvas"
            | "iframe"
            | "frame"
            | "embed"
            | "object"
            | "input"
            | "textarea"
            | "select"
            | "button"
    ) || (node.namespace == "http://www.w3.org/2000/svg" && node.tag == "svg")
}

fn transformable_box(node: &LayoutNode, style: &BTreeMap<String, String>) -> bool {
    let display = style_value(style, "display", "block");
    if matches!(
        display,
        "none" | "contents" | "table-column" | "table-column-group"
    ) {
        return false;
    }
    display != "inline" || is_replaced_box(node) || node.namespace == "http://www.w3.org/2000/svg"
}

fn preserves_3d(style: &BTreeMap<String, String>) -> bool {
    if style_value(style, "transform-style", "flat") != "preserve-3d" {
        return false;
    }
    if style_value(style, "opacity", "1")
        .parse::<f64>()
        .unwrap_or(1.)
        < 1.
    {
        return false;
    }
    if ["overflow-x", "overflow-y"]
        .iter()
        .any(|key| !matches!(style_value(style, key, "visible"), "visible" | "clip"))
    {
        return false;
    }
    if [
        ("filter", "none"),
        ("clip", "auto"),
        ("clip-path", "none"),
        ("isolation", "auto"),
        ("mask-image", "none"),
        ("mask-border-source", "none"),
        ("mix-blend-mode", "normal"),
    ]
    .iter()
    .any(|(key, default)| style_value(style, key, default) != *default)
    {
        return false;
    }
    if style_value(style, "contain", "none")
        .split_whitespace()
        .any(|v| matches!(v, "paint" | "strict" | "content"))
    {
        return false;
    }
    if style_value(style, "content-visibility", "visible") != "visible" {
        return false;
    }
    true
}

const COMPUTED_PROPERTIES: &[&str] = &[
    "zoom",
    "x",
    "y",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "d",
    "clip-rule",
    "fill",
    "fill-rule",
    "fill-opacity",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
    "baseline-shift",
    "vector-effect",
    "animation-name",
    "animation-duration",
    "animation-delay",
    "animation-timing-function",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
    "animation-composition",
    "transition-property",
    "transition-duration",
    "transition-delay",
    "transition-timing-function",
    "transition-behavior",
    "user-select",
    "interest-delay-start",
    "interest-delay-end",
    "anchor-name",
    "anchor-scope",
    "position-anchor",
    "position-area",
    "position-try-fallbacks",
    "position-try-order",
    "position-visibility",
    "list-style-type",
    "list-style-position",
    "list-style-image",
    "column-count",
    "column-width",
    "column-fill",
    "column-span",
    "column-rule-color",
    "column-rule-style",
    "column-rule-width",
    "ruby-align",
    "ruby-position",
    "ruby-overhang",
    "text-combine-upright",
    "text-decoration-line",
    "text-decoration-color",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-decoration-skip-ink",
    "text-underline-offset",
    "text-underline-position",
    "text-emphasis-style",
    "text-emphasis-color",
    "text-emphasis-position",
    "text-emphasis-skip",
    "box-shadow",
    "background-origin",
    "background-clip",
    "background-size",
    "background-position",
    "background-repeat",
    "background-attachment",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
    "display",
    "position",
    "table-layout",
    "vertical-align",
    "caption-side",
    "border-collapse",
    "border-spacing",
    "box-decoration-break",
    "page",
    "break-before",
    "break-after",
    "break-inside",
    "orphans",
    "widows",
    "visibility",
    "opacity",
    "filter",
    "backdrop-filter",
    "marker-start",
    "marker-mid",
    "marker-end",
    "pointer-events",
    "box-sizing",
    "width",
    "height",
    "left",
    "top",
    "right",
    "bottom",
    "inset-inline-start",
    "inset-inline-end",
    "inset-block-start",
    "inset-block-end",
    "transform",
    "transform-origin",
    "transform-box",
    "transform-style",
    "perspective",
    "perspective-origin",
    "clip",
    "clip-path",
    "isolation",
    "mask-image",
    "mask-type",
    "mask-border-source",
    "mix-blend-mode",
    "contain",
    "content-visibility",
    "translate",
    "rotate",
    "scale",
    "z-index",
    "background-color",
    "background-image",
    "image-rendering",
    "object-fit",
    "object-position",
    "color-scheme",
    "color",
    "direction",
    "writing-mode",
    "text-orientation",
    "unicode-bidi",
    "content",
    "quotes",
    "counter-reset",
    "counter-increment",
    "counter-set",
    "text-transform",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "font-stretch",
    "font-synthesis-weight",
    "font-synthesis-style",
    "font-feature-settings",
    "font-variation-settings",
    "font-optical-sizing",
    "font-language-override",
    "font-kerning",
    "font-variant-ligatures",
    "font-variant-caps",
    "font-variant-numeric",
    "font-variant-east-asian",
    "font-variant-position",
    "letter-spacing",
    "word-spacing",
    "tab-size",
    "line-height",
    "text-align",
    "text-indent",
    "text-align-last",
    "white-space",
    "white-space-collapse",
    "text-wrap-mode",
    "word-break",
    "overflow-wrap",
    "scrollbar-color",
    "scrollbar-width",
    "scrollbar-gutter",
    "overflow-x",
    "overflow-y",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "scroll-padding-top",
    "scroll-padding-right",
    "scroll-padding-bottom",
    "scroll-padding-left",
    "scroll-margin-top",
    "scroll-margin-right",
    "scroll-margin-bottom",
    "scroll-margin-left",
    "min-width",
    "max-width",
    "aspect-ratio",
    "min-height",
    "max-height",
    "will-change",
    "outline-color",
    "outline-style",
    "outline-width",
    "outline-offset",
    "accent-color",
    "appearance",
    "field-sizing",
    "float",
    "clear",
    "flex-direction",
    "flex-wrap",
    "order",
    "align-items",
    "align-self",
    "align-content",
    "justify-content",
    "justify-items",
    "justify-self",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "row-gap",
    "column-gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-template-areas",
    "grid-auto-flow",
    "grid-auto-columns",
    "grid-auto-rows",
    "grid-column-start",
    "grid-column-end",
    "grid-row-start",
    "grid-row-end",
    "margin-top",
    "margin-right",
    "margin-inline-end",
    "margin-bottom",
    "margin-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
];

/// Run Stylo's own DOM traversal and serialize the computed longhands consumed
/// by Oryn's non-painting Taffy/Parley layout layer. The mirror is an immutable
/// style snapshot: the live DOM remains Rust-owned by `oryn-dom`, while selector
/// matching, origins, cascade, variables and inheritance are all resolved by
/// Stylo rather than reconstructed by Oryn.
#[cfg(test)]
fn stylo_computed_values(input: &LayoutInput) -> Result<ComputedStyles, LayoutError> {
    stylo_computed_values_with_provenance(input).map(|(styles, _, _)| styles)
}

type StyloStyleProjection = (
    ComputedStyles,
    BTreeSet<u64>,
    BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
);

fn intern_computed_style(
    values: BTreeMap<String, String>,
    pool: &mut BTreeSet<ComputedStyle>,
) -> ComputedStyle {
    if let Some(shared) = pool.get(&values) {
        return shared.clone();
    }
    let shared = Arc::new(values);
    pool.insert(shared.clone());
    shared
}

#[cfg(test)]
fn stylo_computed_values_with_provenance(
    input: &LayoutInput,
) -> Result<StyloStyleProjection, LayoutError> {
    stylo_computed_values_for_media(input, false)
}
fn projected_pseudo_kinds<'a>(
    input: &'a LayoutInput,
    node: &'a LayoutNode,
) -> impl Iterator<Item = (&'static str, PseudoElement)> + 'a {
    [
        ("before", PseudoElement::Before),
        ("first-line", PseudoElement::FirstLine),
        ("first-letter", PseudoElement::FirstLetter),
        ("marker", PseudoElement::Marker),
        ("after", PseudoElement::After),
        ("backdrop", PseudoElement::Backdrop),
        ("picker", PseudoElement::PickerSelect),
        ("picker::before", PseudoElement::Before),
        ("picker::after", PseudoElement::After),
        ("picker-icon", PseudoElement::PickerIcon),
        ("permission-icon", PseudoElement::PermissionIcon),
        ("interest-button", PseudoElement::InterestButton),
        ("checkmark", PseudoElement::Checkmark),
    ]
    .into_iter()
    .filter(move |(name, _)| {
        let name = *name;

        if name == "backdrop" && !input.top_layer.iter().any(|entry| entry.node_id == node.id) {
            return false;
        }
        if matches!(
            name,
            "picker" | "picker::before" | "picker::after" | "picker-icon"
        ) && node.tag != "select"
        {
            return false;
        }
        if name == "checkmark" && node.tag != "option" {
            return false;
        }
        if name == "interest-button"
            && !input
                .selector_states
                .get(&node.id)
                .is_some_and(|state| state.interest_invoker)
        {
            return false;
        }
        if name == "permission-icon"
            && !matches!(
                node.tag.as_str(),
                "geolocation" | "usermedia" | "camera" | "microphone" | "install"
            )
        {
            return false;
        }
        true
    })
}

#[cfg(test)]
fn stylo_computed_values_for_media(
    input: &LayoutInput,
    print_media: bool,
) -> Result<StyloStyleProjection, LayoutError> {
    stylo_computed_values_for_media_with_sample(input, print_media, None)
}
fn stylo_computed_values_for_media_with_sample(
    input: &LayoutInput,
    print_media: bool,
    sample: Option<&svg_instances::StyleSample>,
) -> Result<StyloStyleProjection, LayoutError> {
    let mut continuity = sample.map(|sample| sample.continuity.clone());
    stylo_computed_values_timed(
        input,
        print_media,
        sample.map_or(0., |sample| sample.now),
        continuity.as_mut(),
        sample.map_or(&[][..], |sample| sample.effects.as_slice()),
        None,
    )
}
fn stylo_computed_values_timed(
    input: &LayoutInput,
    print_media: bool,
    now: f64,
    continuity: Option<&mut blitz_dom::StyleContinuity>,
    effects: &[blitz_dom::NativeAnimationEffect],
    samples: Option<&mut BTreeMap<u64, BTreeMap<String, String>>>,
) -> Result<StyloStyleProjection, LayoutError> {
    stylo_computed_values_with_instances(
        input,
        print_media,
        now,
        continuity,
        effects,
        samples,
        None,
    )
}
fn stylo_computed_values_with_instances(
    input: &LayoutInput,
    print_media: bool,
    now: f64,
    continuity: Option<&mut blitz_dom::StyleContinuity>,
    effects: &[blitz_dom::NativeAnimationEffect],
    samples: Option<&mut BTreeMap<u64, BTreeMap<String, String>>>,
    instances: Option<(
        &[svg_instances::StyleInstance],
        &mut svg_instances::InstanceProjection,
        &LayoutLimits,
    )>,
) -> Result<StyloStyleProjection, LayoutError> {
    shadow_tree::validate(input)?;
    let viewport = BlitzViewport::new(
        (input.viewport.width * input.viewport.device_pixel_ratio).clamp(1.0, f64::from(u32::MAX))
            as u32,
        (input.viewport.height * input.viewport.device_pixel_ratio).clamp(1.0, f64::from(u32::MAX))
            as u32,
        input.viewport.device_pixel_ratio as f32,
        match input.color_scheme {
            ColorScheme::Light => BlitzColorScheme::Light,
            ColorScheme::Dark => BlitzColorScheme::Dark,
        },
    );
    let mut document = BaseDocument::new(DocumentConfig {
        style_lock: continuity
            .as_deref()
            .and_then(|continuity| continuity.lock.clone()),
        media_type: Some(if print_media {
            style::media_queries::MediaType::print()
        } else {
            style::media_queries::MediaType::screen()
        }),
        viewport: Some(viewport),
        base_url: Some(input.document_base_url.clone()),
        style_threading: blitz_dom::StyleThreading::Sequential,
        ..Default::default()
    });
    for registration in &input.registered_custom_properties {
        document
            .register_custom_property(
                &registration.name,
                &registration.syntax,
                registration.inherits,
                registration.initial_value.as_deref(),
                &registration.base_url,
            )
            .map_err(LayoutError::StyleTraversal)?;
    }
    // Shell backing pixels are integral; CSS dimensions and media queries are not.
    document.set_css_viewport_size(input.viewport.width as f32, input.viewport.height as f32);
    document.set_prefers_reduced_motion(input.reduced_motion);
    // SVG non-root transforms use the local origin. Outermost SVG viewports
    // and direct SVG children of foreignObject retain the CSS initial center.
    // This UA rule yields to presentation hints and every author declaration.
    document.add_user_agent_stylesheet("@namespace url(http://www.w3.org/2000/svg); *:not(svg), *:not(foreignObject) > svg { transform-origin:0 0; }");
    document.add_user_agent_stylesheet("::interest-button{display:none;color:inherit;background-color:transparent;box-sizing:border-box;min-inline-size:24px;min-block-size:max(24px,1lh);margin-inline-start:.25em;margin-block:min(-24px,-1lh);align-content:center;text-align:center;user-select:none;cursor:help}");
    document.add_user_agent_stylesheet("geolocation::permission-icon,usermedia::permission-icon,camera::permission-icon,microphone::permission-icon,install::permission-icon{display:inline-block;content:\"◎\";fill:currentColor;height:1em;width:1em;margin-inline-end:0.5em;vertical-align:middle}");
    document.add_user_agent_stylesheet("geolocation,usermedia,camera,microphone,install{display:inline-block;box-sizing:border-box;min-width:120px;min-height:32px;padding:4px 12px;border:1px solid ButtonBorder;border-radius:4px;background-color:ButtonFace;color:ButtonText;font:13px sans-serif;vertical-align:middle;cursor:pointer} geolocation>* ,usermedia>* ,camera>* ,microphone>* ,install>* {display:none}");
    document.add_user_agent_stylesheet(
        "input[type=image i] { appearance:none; padding:0; border:0; width:auto; height:auto; } slot { display:contents; } textarea { white-space:pre-wrap; overflow-wrap:break-word; overflow:auto; } textarea[wrap=off i] { white-space:pre; overflow-wrap:normal; }",
    );
    document.add_user_agent_stylesheet("meter,progress,input[type=checkbox i],input[type=radio i],input[type=range i],select{appearance:auto} meter{display:inline-block;width:5em;height:1em} progress{display:inline-block;width:10em;height:1em} input[type=range i]{width:129px;height:16px;padding:0;border:0} select{display:inline-block;border:1px solid #999;background-color:#efefef;padding:2px 20px 2px 4px}option{display:block;white-space:nowrap}optgroup{display:block;font-weight:bold}::backdrop{position:fixed;inset:0}select::picker(select){position:fixed;background-color:white;color:black;border:1px solid #999;padding:2px;overflow:auto}select::picker-icon{content:\"\\25be\"}option::checkmark{content:\"\\2713\"}");
    let html_namespace = Namespace::from("http://www.w3.org/1999/xhtml");
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let inherited_languages = effective_languages(input);
    let mut shadow_mirrors = BTreeMap::new();
    for (root, tree) in &input.shadow_trees.roots {
        if !by_id.contains_key(&tree.host) || by_id.contains_key(root) {
            return Err(LayoutError::StyleTraversal(
                "invalid native shadow root identity".into(),
            ));
        }
        let mirror = document.create_node(blitz_dom::node::NodeData::Document(Box::default()));
        shadow_mirrors.insert(*root, mirror);
    }
    let mut mirror_ids = BTreeMap::new();
    let mut mirror_text_ids = BTreeMap::new();
    {
        let mut mutator = document.mutate();
        for node in &input.nodes {
            let mirror = match node.tag.as_str() {
                "#text" => mutator.create_text_node(&node.text),
                "#comment" => mutator.create_comment_node(&node.text),
                _ => {
                    let mut attributes = node
                        .attributes
                        .iter()
                        .map(|(name, value)| BlitzAttribute {
                            name: QualName::new(
                                None,
                                Namespace::from(""),
                                LocalName::from(name.as_str()),
                            ),
                            value: value.clone(),
                        })
                        .collect::<Vec<_>>();
                    attributes.extend(node.namespaced_attributes.iter().map(|entry| {
                        BlitzAttribute {
                            name: QualName::new(
                                entry.prefix.as_deref().map(Into::into),
                                Namespace::from(entry.namespace.as_str()),
                                LocalName::from(entry.local_name.as_str()),
                            ),
                            value: entry.value.clone(),
                        }
                    }));
                    mutator.create_element(
                        QualName::new(
                            None,
                            Namespace::from(node.namespace.as_str()),
                            LocalName::from(node.tag.as_str()),
                        ),
                        attributes,
                    )
                }
            };
            mirror_ids.insert(node.id, mirror);
            if !node.tag.starts_with('#') {
                let mut current = Some(node.id);
                let mut visited = BTreeSet::new();
                let mut direction = None;
                while let Some(id) = current.filter(|id| visited.insert(*id)) {
                    let Some(ancestor) = by_id.get(&id) else {
                        break;
                    };
                    direction = ancestor
                        .attributes
                        .get("data-oryn-effective-dir")
                        .or_else(|| ancestor.attributes.get("dir"))
                        .filter(|value| {
                            matches!(value.to_ascii_lowercase().as_str(), "ltr" | "rtl")
                        })
                        .cloned();
                    if direction.is_some() {
                        break;
                    }
                    current = ancestor.parent.or_else(|| {
                        input
                            .shadow_trees
                            .node_scopes
                            .get(&id)
                            .and_then(|root| input.shadow_trees.roots.get(root))
                            .map(|tree| tree.host)
                    });
                }
                let native = mutator
                    .doc
                    .get_node_mut(mirror)
                    .expect("native selector mirror");
                native.native_language = inherited_languages.get(&node.id).cloned().flatten();
                native.native_direction = direction.unwrap_or_else(|| "ltr".into());
                native.native_custom_states = input
                    .custom_states
                    .get(&node.id)
                    .into_iter()
                    .flatten()
                    .map(|state| style::values::AtomIdent::from(state.as_str()))
                    .collect();
            }

            // Element snapshots carry their direct text separately from the
            // element-child list. Stylo must see it for selectors such as
            // :empty; it is not sufficient to pass text only to Parley.
            let explicit_text = node
                .children
                .iter()
                .any(|child| by_id.get(child).is_some_and(|child| child.tag == "#text"));
            if node.tag != "#text"
                && node.tag != "#comment"
                && !node.text.is_empty()
                && node.content.is_none()
                && !explicit_text
            {
                let text = mutator.create_text_node(&node.text);
                mutator.append_children(mirror, &[text]);
            }
        }
        // Attach shadow-owned children before processing light DOM children.
        for (root, tree) in &input.shadow_trees.roots {
            let parent = shadow_mirrors[root];
            for item in &tree.content {
                let child = match item {
                    LayoutContent::Element(id) => *mirror_ids.get(id).ok_or_else(|| {
                        LayoutError::StyleTraversal("missing shadow child".into())
                    })?,
                    LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                        mutator.create_text_node(text)
                    }
                };
                if mutator.node_has_parent(child) {
                    return Err(LayoutError::StyleTraversal(
                        "multiply owned native shadow child".into(),
                    ));
                }
                mutator.append_children(parent, &[child]);
            }
        }
        // Establish authored child order before the compatibility parent-link
        // pass. Input vector order is an ownership inventory, not DOM order.
        for node in &input.nodes {
            if node.tag.starts_with('#') {
                continue;
            }
            let Some(parent) = mirror_ids.get(&node.id).copied() else {
                continue;
            };
            if let Some(content) = &node.content {
                for item in content {
                    let child = match item {
                        LayoutContent::Text { node_id, text } => {
                            let child = mutator.create_text_node(text);
                            mirror_text_ids.insert(*node_id, child);
                            child
                        }
                        LayoutContent::StyledText { text, .. } => mutator.create_text_node(text),
                        LayoutContent::Element(id) => {
                            let Some(child) = mirror_ids.get(id).copied() else {
                                continue;
                            };
                            child
                        }
                    };
                    if !mutator.node_has_parent(child) {
                        mutator.append_children(parent, &[child]);
                    }
                }
            } else {
                for id in &node.children {
                    let Some(child) = mirror_ids.get(id).copied() else {
                        continue;
                    };
                    if !mutator.node_has_parent(child) {
                        mutator.append_children(parent, &[child]);
                    }
                }
            }
        }
        for node in &input.nodes {
            let parent = node
                .parent
                .and_then(|parent| mirror_ids.get(&parent).copied())
                .unwrap_or_else(|| mutator.doc.root_node().id);
            let Some(child) = mirror_ids.get(&node.id).copied() else {
                continue;
            };
            if !mutator.node_has_parent(child) {
                mutator.append_children(parent, &[child]);
            }
        }
    }
    for (root, tree) in &input.shadow_trees.roots {
        let mirror = shadow_mirrors[root];
        let host = mirror_ids[&tree.host];
        let children = document
            .get_node(mirror)
            .expect("shadow mirror")
            .children
            .to_vec();
        let node = document.get_node_mut(mirror).expect("shadow mirror");
        node.shadow_host = Some(host);
        node.parent = Some(host);
        let node = document.get_node_mut(host).expect("host mirror");
        node.shadow_root = Some(mirror);
        node.style_children = Some(children);
    }
    // Shadow roots are linked after detached mirror construction. The mutator
    // cannot mark their descendants connected at append time; publish the
    // final shadow-including connectivity before restoring animation state.
    let mut connected = vec![document.root_node().id];
    let mut visited = std::collections::HashSet::new();
    while let Some(id) = connected.pop() {
        if !visited.insert(id) {
            continue;
        }
        let node = document
            .get_node_mut(id)
            .expect("connected selector mirror");
        node.flags
            .set(blitz_dom::node::NodeFlags::IS_IN_DOCUMENT, true);
        connected.extend(node.children.iter().copied());
        connected.extend(node.shadow_root);
    }
    for (id, root) in &input.shadow_trees.node_scopes {
        let mirror = *mirror_ids
            .get(id)
            .ok_or_else(|| LayoutError::StyleTraversal("unknown shadow member".into()))?;
        let root = *shadow_mirrors
            .get(root)
            .ok_or_else(|| LayoutError::StyleTraversal("unknown shadow scope".into()))?;
        document
            .get_node_mut(mirror)
            .expect("member mirror")
            .containing_shadow = Some(root);
    }
    let mut slot_children = BTreeMap::<u64, Vec<_>>::new();
    // Native light-DOM order includes assigned text nodes.
    for parent in &input.nodes {
        let ids = parent
            .content
            .as_ref()
            .map(|content| {
                content
                    .iter()
                    .filter_map(|item| match item {
                        LayoutContent::Element(id) | LayoutContent::Text { node_id: id, .. } => {
                            Some(*id)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| parent.children.clone());
        for id in ids {
            if let Some(slot) = input.shadow_trees.assigned_slots.get(&id) {
                let mirror = *mirror_ids
                    .get(&id)
                    .or_else(|| mirror_text_ids.get(&id))
                    .ok_or_else(|| LayoutError::StyleTraversal("unknown slotted member".into()))?;
                let target = *mirror_ids
                    .get(slot)
                    .ok_or_else(|| LayoutError::StyleTraversal("unknown assigned slot".into()))?;
                document
                    .get_node_mut(mirror)
                    .expect("member mirror")
                    .assigned_slot = Some(target);
                slot_children.entry(*slot).or_default().push(mirror);
            }
        }
    }
    for (slot, children) in slot_children {
        document
            .get_node_mut(mirror_ids[&slot])
            .expect("slot mirror")
            .style_children = Some(children);
    }
    let animation_nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    for input_node in &input.nodes {
        if !input_node.tag.starts_with('#')
            && let Some(mirror) = mirror_ids.get(&input_node.id).copied()
        {
            let node = document
                .get_node_mut(mirror)
                .expect("mapped validation node");
            let selector = input.selector_states.get(&input_node.id);
            node.element_state_mut().set(
                ElementState::DEFINED,
                selector.map(|state| state.defined).unwrap_or_else(|| {
                    input_node.namespace != "http://www.w3.org/1999/xhtml"
                        || !oryn_dom::valid_custom_element_name(&input_node.tag)
                }),
            );
            node.native_permission_granted = selector.is_some_and(|state| state.permission_granted);
            node.native_fullscreen = selector.is_some_and(|state| state.fullscreen);
            node.native_legacy_background_url =
                input.legacy_background_urls.get(&input_node.id).cloned();
            node.native_svg_document_url = input_node
                .document_url
                .clone()
                .or_else(|| Some(input.document_base_url.clone()));
            node.native_svg_transform = input_node
                .attributes
                .get("transform")
                .and_then(|value| oryn_common::basic_shape::svg_transform(value).ok())
                .map(|m| {
                    format!(
                        "matrix({},{},{},{},{},{})",
                        m[0], m[1], m[2], m[3], m[4], m[5]
                    )
                });
            node.element_state_mut().set(
                ElementState::FULLSCREEN,
                selector.is_some_and(|state| state.fullscreen),
            );
            node.native_media_state = selector.map_or_else(
                || {
                    if input_node.namespace == "http://www.w3.org/1999/xhtml"
                        && matches!(input_node.tag.as_str(), "audio" | "video")
                    {
                        3
                    } else {
                        0
                    }
                },
                |state| state.media_state,
            );
            node.native_interest_source = selector.is_some_and(|state| state.interest_source);
            // A single-select's closed picker does not generate its option
            // subtree. Its button remains generated; script effects are retained.
            let mut branch = input_node;
            let mut generated = true;
            for _ in 0..input.nodes.len() {
                let Some(parent) = branch
                    .parent
                    .and_then(|id| animation_nodes.get(&id).copied())
                else {
                    break;
                };
                if parent.namespace == "http://www.w3.org/1999/xhtml"
                    && parent.tag == "object"
                    && (parent.intrinsic_image_size.is_some() || parent.document_replaced)
                {
                    generated = false;
                    break;
                }
                if parent.namespace == "http://www.w3.org/1999/xhtml" && parent.tag == "select" {
                    let listbox = parent.attributes.contains_key("multiple")
                        || parent
                            .attributes
                            .get("size")
                            .and_then(|size| size.trim().parse::<u32>().ok())
                            .is_some_and(|size| size > 1);
                    let open = input.top_layer.iter().any(|entry| {
                        entry.node_id == parent.id
                            && entry.kind == oryn_common::v4::TopLayerKind::SelectPicker
                    });
                    if !listbox && !open && branch.tag != "button" {
                        generated = false;
                        break;
                    }
                }
                branch = parent;
            }
            node.native_css_animations_rendered = generated;

            node.native_interest_target = selector.is_some_and(|state| state.interest_target);
            node.native_inline_style_blocked =
                selector.is_some_and(|state| state.inline_style_blocked);
            if let Some(state) = selector {
                node.element_state_mut()
                    .set(ElementState::USER_VALID, state.user_validity == Some(true));
                node.element_state_mut().set(
                    ElementState::USER_INVALID,
                    state.user_validity == Some(false),
                );
                node.element_state_mut()
                    .set(ElementState::DEFAULT, state.default_control);
                node.element_state_mut()
                    .set(ElementState::INRANGE, state.in_range == Some(true));
                node.element_state_mut()
                    .set(ElementState::OUTOFRANGE, state.in_range == Some(false));
                node.element_state_mut()
                    .set(ElementState::REQUIRED, state.required == Some(true));
                node.element_state_mut()
                    .set(ElementState::OPTIONAL_, state.required == Some(false));
                node.element_state_mut()
                    .set(ElementState::PLACEHOLDER_SHOWN, state.placeholder_shown);
                node.element_state_mut()
                    .set(ElementState::READWRITE, state.read_write);
                node.element_state_mut()
                    .set(ElementState::READONLY, !state.read_write);
                node.element_state_mut()
                    .set(ElementState::URLTARGET, state.target);
            }
            let top = input
                .top_layer
                .iter()
                .find(|entry| entry.node_id == input_node.id);
            node.element_state_mut().set(
                ElementState::OPEN,
                input_node.namespace == "http://www.w3.org/1999/xhtml"
                    && ((matches!(input_node.tag.as_str(), "details" | "dialog")
                        && input_node.attributes.contains_key("open"))
                        || (matches!(input_node.tag.as_str(), "select" | "input")
                            && top.is_some_and(|entry| {
                                matches!(
                                    entry.kind,
                                    oryn_common::v4::TopLayerKind::SelectPicker
                                        | oryn_common::v4::TopLayerKind::TemporalPicker
                                        | oryn_common::v4::TopLayerKind::PermissionPrompt
                                )
                            }))),
            );
            node.element_state_mut().set(
                ElementState::MODAL,
                top.is_some_and(|entry| entry.kind == oryn_common::v4::TopLayerKind::ModalDialog),
            );
            node.element_state_mut().set(
                ElementState::POPOVER_OPEN,
                top.is_some_and(|entry| entry.kind == oryn_common::v4::TopLayerKind::Popover),
            );
            node.element_state_mut()
                .set(ElementState::VALID, input_node.validity == Some(true));
            node.element_state_mut()
                .set(ElementState::INVALID, input_node.validity == Some(false));
            if let Some(disabled) = input_node.disabled {
                node.element_state_mut()
                    .set(ElementState::DISABLED, disabled);
                node.element_state_mut()
                    .set(ElementState::ENABLED, !disabled);
            }
        }
        if let Some(mirror) = mirror_ids.get(&input_node.id).copied()
            && input_node.namespace == "http://www.w3.org/1999/xhtml"
            && (matches!(input_node.tag.as_str(), "progress" | "option")
                || (input_node.tag == "input"
                    && input_node.attributes.get("type").is_some_and(|kind| {
                        kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
                    })))
        {
            let node = document.get_node_mut(mirror).expect("mapped input node");
            if let Some(checked) = input_node.checked {
                node.element_state_mut().set(ElementState::CHECKED, checked);
            }
            node.element_state_mut()
                .set(ElementState::INDETERMINATE, input_node.indeterminate);
        }
    }
    // The native DOM owns focus. Set Stylo state directly on the fresh mirror;
    // no synthetic HTML attributes or focus events are produced by a style flush.
    if let Some(focused) = input.focused_node.filter(|id| {
        mirror_ids.contains_key(id)
            && by_id
                .get(id)
                .is_some_and(|node| !matches!(node.tag.as_str(), "#text" | "#comment"))
    }) {
        for id in shadow_tree::focus_within(input, focused) {
            if let Some(mirror) = mirror_ids.get(&id).copied() {
                let node = document.get_node_mut(mirror).expect("mapped style node");
                node.element_state_mut().insert(ElementState::FOCUS_WITHIN);
                if id == focused {
                    node.element_state_mut().insert(ElementState::FOCUS);
                    if input.focus_visible {
                        node.element_state_mut().insert(ElementState::FOCUSRING);
                    }
                }
            }
        }
    }
    // Building the mirror can parse <style> text automatically. CSSOM source
    // is owned by LayoutInput and can differ from that text; keeping both would
    // duplicate declarations and establish stale cascade-layer ordering.
    document.clear_author_stylesheets();
    let mut shadow_sheets = BTreeMap::<u64, Vec<_>>::new();
    for (sheet_index, stylesheet) in input.stylesheets.iter().enumerate() {
        let style_node = {
            let mut mutator = document.mutate();
            mutator.create_element(
                QualName::new(None, html_namespace.clone(), LocalName::from("style")),
                Vec::new(),
            )
        };
        // Native tree/state hooks preserve selector syntax and specificity.
        let base = input
            .stylesheet_base_urls
            .get(&sheet_index)
            .and_then(|url| url::Url::parse(url).ok());
        let mut stylesheet = document.make_stylesheet_with_base(stylesheet, Origin::Author, base);
        use style::stylesheets::scope_rule::ImplicitScopeRoot;
        stylesheet.1 = match input.stylesheet_scope_roots.get(&sheet_index) {
            Some(StylesheetScopeRoot::Element(id)) => mirror_ids
                .get(id)
                .and_then(|id| document.get_node(*id))
                .map(|node| ImplicitScopeRoot::InLightTree(selectors::Element::opaque(&node))),
            Some(StylesheetScopeRoot::Constructed) => Some(ImplicitScopeRoot::Constructed),
            Some(StylesheetScopeRoot::DocumentElement) => Some(ImplicitScopeRoot::DocumentElement),
            None => None,
        };
        if let Some(root) = input.shadow_trees.stylesheet_scopes.get(&sheet_index) {
            let tree = input.shadow_trees.roots.get(root).ok_or_else(|| {
                LayoutError::StyleTraversal("unknown stylesheet shadow scope".into())
            })?;
            let host = document
                .get_node(mirror_ids[&tree.host])
                .expect("host mirror");
            stylesheet.1 = Some(match input.stylesheet_scope_roots.get(&sheet_index) {
                Some(StylesheetScopeRoot::Element(id))
                    if input.shadow_trees.node_scopes.get(id) == Some(root) =>
                {
                    let node = document
                        .get_node(*mirror_ids.get(id).ok_or_else(|| {
                            LayoutError::StyleTraversal("unknown implicit shadow scope".into())
                        })?)
                        .expect("scope mirror");
                    ImplicitScopeRoot::InShadowTree(selectors::Element::opaque(&node))
                }
                _ => ImplicitScopeRoot::ShadowHost(selectors::Element::opaque(&host)),
            });
        }
        if let Some(text) = input.stylesheet_media.get(&sheet_index) {
            use style::{
                context::QuirksMode,
                stylesheets::{CssRuleType, StylesheetInDocument},
            };
            use style_traits::ParsingMode;
            let url_data = {
                let guard = document.guard().read();
                stylesheet.contents(&guard).url_data.clone()
            };
            let mut context = style::parser::ParserContext::new(
                Origin::Author,
                &url_data,
                Some(CssRuleType::Media),
                ParsingMode::DEFAULT,
                QuirksMode::NoQuirks,
                Default::default(),
                None,
                None,
                Default::default(),
            );
            let mut input = cssparser::ParserInput::new(text);
            let media = style::media_queries::MediaList::parse(
                &mut context,
                &mut cssparser::Parser::new(&mut input),
            );
            let mut guard = document.guard().write();
            *stylesheet.0.media.write_with(&mut guard) = media;
        }
        if let Some(root) = input.shadow_trees.stylesheet_scopes.get(&sheet_index) {
            shadow_sheets.entry(*root).or_default().push(stylesheet);
        } else {
            document.add_stylesheet_for_node(stylesheet, style_node);
        }
    }
    for (root, sheets) in shadow_sheets {
        document
            .install_shadow_styles(shadow_mirrors[&root], sheets)
            .map_err(LayoutError::StyleTraversal)?;
    }
    // This mirror supplies computed values only. Running Blitz's layout and
    // inline construction here duplicates Oryn's Taffy/Parley pass and can
    // attempt to attach inline layout data to the document node when a DOM
    // snapshot contains text beside its root element. Stylo needs only the
    // element traversal; empty/text-only documents have no element to style.
    if input.nodes.iter().any(|node| {
        node.tag != "#text"
            && node.tag != "#comment"
            && node
                .parent
                .is_none_or(|parent| !mirror_ids.contains_key(&parent))
    }) {
        if let Some(previous) = continuity.as_deref() {
            document.restore_style_continuity(previous, &mirror_ids);
        }
        document.resolve_stylist(now);
        if !effects.is_empty() {
            let sampled = document.sample_native_effects(&mirror_ids, effects, now);
            if let Some(samples) = samples {
                *samples = sampled;
            }
            document.resolve_stylist(now);
        } else if let Some(samples) = samples {
            samples.clear();
        }
    }

    if let Some((requests, output, limits)) = instances {
        *output = svg_instances::resolve(
            &mut document,
            input,
            &mirror_ids,
            requests,
            &shadow_mirrors,
            limits,
        )?;
    }

    let mut output = ComputedStyles::new();
    let mut shared_styles = BTreeSet::<ComputedStyle>::new();
    let mut author_whitespace = BTreeSet::new();
    let mut pending = BTreeSet::new();
    for node in &input.nodes {
        let mut values = BTreeMap::new();
        if node.tag == "#text" || node.tag == "#comment" {
            pending.insert(node.id);
        } else {
            let mirror = mirror_ids.get(&node.id).copied().ok_or_else(|| {
                LayoutError::StyleTraversal(format!("missing mirror for node {}", node.id))
            })?;
            if let Some(computed) = document.resolved_primary_style(mirror) {
                values.insert(
                    "oryn-effective-zoom".into(),
                    computed.effective_zoom.value().to_string(),
                );
                values.insert(
                    "oryn-preferred-color-scheme".into(),
                    if input.color_scheme == ColorScheme::Dark {
                        "dark"
                    } else {
                        "light"
                    }
                    .into(),
                );
                values.insert(
                    "oryn-reduced-motion".into(),
                    if input.reduced_motion {
                        "reduce"
                    } else {
                        "no-preference"
                    }
                    .into(),
                );
                use style::computed_value_flags::ComputedValueFlags;
                if computed.flags.intersects(
                    ComputedValueFlags::HAS_AUTHOR_WHITE_SPACE_COLLAPSE
                        | ComputedValueFlags::HAS_AUTHOR_TEXT_WRAP_MODE,
                ) {
                    author_whitespace.insert(node.id);
                }
                for name in COMPUTED_PROPERTIES {
                    let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
                        continue;
                    };
                    let value = match property.as_shorthand() {
                        Ok(shorthand) => {
                            // Modern white-space is a shorthand. Serialize its
                            // computed longhands through Stylo instead of losing
                            // the cascade and falling back to inline declarations.
                            let declarations = shorthand
                                .longhands()
                                .map(|id| computed.computed_or_resolved_declaration(id, None))
                                .collect::<Vec<_>>();
                            let references = declarations.iter().collect::<Vec<_>>();
                            let mut value = String::new();
                            shorthand
                                .longhands_to_css(&references, &mut value)
                                .map_err(|_| {
                                    LayoutError::StyleTraversal(format!(
                                        "cannot serialize computed {name}"
                                    ))
                                })?;
                            value
                        }
                        Err(longhand) => computed.computed_value_to_string(longhand),
                    };
                    values.insert((*name).to_string(), value);
                }
                let list_scope = computed.get_list().clone_list_style_type().1;
                let native_scope = shadow_mirrors.iter().find_map(|(root, mirror)| {
                    (mirror.as_u64() as usize == list_scope).then_some(*root)
                });
                if list_scope != 0 && native_scope.is_none() {
                    return Err(LayoutError::StyleTraversal(
                        "unknown native computed list-style scope".into(),
                    ));
                }
                values.insert(
                    "oryn-list-style-tree".into(),
                    native_scope
                        .map(|root| root.to_string())
                        .unwrap_or_else(|| "document".into()),
                );
                let custom = computed.custom_properties();
                for (name, value) in custom.inherited.iter().chain(custom.non_inherited.iter()) {
                    if let Some(value) = value {
                        values.insert(format!("--{name}"), value.to_css_string());
                    }
                }
            } else {
                return Err(LayoutError::StyleTraversal(format!(
                    "Stylo did not resolve element {}",
                    node.id
                )));
            }
        }
        output.insert(node.id, intern_computed_style(values, &mut shared_styles));
    }
    // Parent-first traversal is required for text nodes even when the input
    // snapshot itself is deliberately out of order.
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|id| {
            let Some(node) = by_id.get(id).copied() else {
                return false;
            };
            let Some(parent) = node.parent else {
                output.insert(
                    *id,
                    intern_computed_style(ua_style(node), &mut shared_styles),
                );
                return false;
            };
            let Some(parent_style) = output.get(&parent).cloned() else {
                return true;
            };
            if author_whitespace.contains(&parent) {
                author_whitespace.insert(*id);
            }
            let mut style = (*parent_style).clone();
            if node.tag == "#text" || node.tag == "#comment" {
                style.insert("display".into(), "inline".into());
            }
            output.insert(*id, intern_computed_style(style, &mut shared_styles));
            false
        });
        if pending.len() == before {
            break;
        }
    }
    let mut pseudo_styles = BTreeMap::new();
    for node in &input.nodes {
        let Some(mirror) = mirror_ids.get(&node.id).copied() else {
            continue;
        };
        for (name, pseudo) in projected_pseudo_kinds(input, node) {
            let Some(computed) = (if name.starts_with("picker::") {
                document.resolved_nested_pseudo_style(mirror, &PseudoElement::PickerSelect, &pseudo)
            } else {
                document.resolved_pseudo_style(mirror, &pseudo)
            }) else {
                continue;
            };
            let mut values = BTreeMap::new();
            values.insert(
                "oryn-effective-zoom".into(),
                computed.effective_zoom.value().to_string(),
            );
            values.insert(
                "oryn-preferred-color-scheme".into(),
                if input.color_scheme == ColorScheme::Dark {
                    "dark"
                } else {
                    "light"
                }
                .into(),
            );
            values.insert(
                "oryn-reduced-motion".into(),
                if input.reduced_motion {
                    "reduce"
                } else {
                    "no-preference"
                }
                .into(),
            );
            for property in COMPUTED_PROPERTIES {
                if let Ok(parsed) = PropertyId::parse_enabled_for_all_content(property)
                    && let Err(longhand) = parsed.as_shorthand()
                {
                    values.insert(
                        (*property).into(),
                        computed.computed_value_to_string(longhand),
                    );
                }
            }
            if let style::values::generics::counters::Content::Items(items) =
                &computed.get_counters().content
                && let Some(scope) = items.native_scope
            {
                let native = shadow_mirrors.iter().find_map(|(root, mirror)| {
                    (mirror.as_u64() as usize == scope).then_some(*root)
                });
                if scope != 0 && native.is_none() {
                    return Err(LayoutError::StyleTraversal(
                        "unknown native computed content scope".into(),
                    ));
                }
                values.insert(
                    "oryn-content-tree".into(),
                    native
                        .map(|root| root.to_string())
                        .unwrap_or_else(|| "document".into()),
                );
            }
            if matches!(name, "picker" | "interest-button") {
                anchor_positioning::picker_cascade(
                    crate::anchor_positioning::PickerCascadeInputs {
                        document: &document,
                        input,
                        mirrors: &mirror_ids,
                        styles: &output,
                        top_layer: name == "picker",
                        owner: node.id,
                        computed: &computed,
                        values: &mut values,
                    },
                )?;
            }
            pseudo_styles
                .entry(node.id)
                .or_insert_with(BTreeMap::new)
                .insert(
                    name.into(),
                    intern_computed_style(values, &mut shared_styles),
                );
        }
    }
    text_pseudos::cascade(
        &mut document,
        input,
        &mirror_ids,
        &mut output,
        &mut pseudo_styles,
    );
    permission_controls::adjust(input, &mut output, &mut pseudo_styles);
    anchor_positioning::cascade(&document, input, &mirror_ids, &mut output, &pseudo_styles)?;
    // Native post-cascade adjustments may copy maps. Restore sharing by value
    // after those final changes, without merging distinct metadata or values.
    for value in output.values_mut().chain(
        pseudo_styles
            .values_mut()
            .flat_map(|pseudos| pseudos.values_mut()),
    ) {
        if let Some(shared) = shared_styles.get(value.as_ref()) {
            *value = shared.clone();
        } else {
            shared_styles.insert(value.clone());
        }
    }
    if let Some(previous) = continuity {
        let next_id = previous.next_animation_id();
        *previous = document.style_continuity(&mirror_ids);
        previous.assign_animation_ids(next_id);
    }
    Ok((output, author_whitespace, pseudo_styles))
}

/// Traverse parent dependencies rather than relying on snapshot vector order.
/// The resulting map is the compatibility projection of Stylo's parsed values
/// consumed by Taffy/Parley and by `getComputedStyle`.
#[allow(dead_code)]
#[cfg(test)]
fn cascade_subtree(
    id: u64,
    by_id: &BTreeMap<u64, &LayoutNode>,
    rules: &[Rule],
    visiting: &mut BTreeSet<u64>,
    output: &mut BTreeMap<u64, BTreeMap<String, String>>,
) {
    if output.contains_key(&id) || !visiting.insert(id) {
        return;
    }
    let Some(node) = by_id.get(&id).copied() else {
        visiting.remove(&id);
        return;
    };
    if let Some(parent) = node.parent {
        cascade_subtree(parent, by_id, rules, visiting, output);
    }
    let parent_style = node.parent.and_then(|parent| output.get(&parent));
    output.insert(id, cascade(node, by_id, parent_style, rules));
    visiting.remove(&id);
}

#[allow(dead_code)]
#[cfg(test)]
fn stylesheet_fingerprint(input: &LayoutInput) -> [u8; 32] {
    let mut digest = Sha256::new();
    for sheet in &input.stylesheets {
        digest.update((sheet.len() as u64).to_le_bytes());
        digest.update(sheet.as_bytes());
    }
    digest.update(input.viewport.width.to_bits().to_le_bytes());
    digest.update(input.viewport.height.to_bits().to_le_bytes());
    digest.update([match input.color_scheme {
        ColorScheme::Light => 0,
        ColorScheme::Dark => 1,
    }]);
    digest.update([u8::from(input.reduced_motion)]);
    digest.finalize().into()
}

fn apply_ancestor_clipping(
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    viewport: CssRect,
) {
    let ids = geometry.keys().copied().collect::<Vec<_>>();
    for id in ids {
        let Some(rect) = geometry
            .get(&id)
            .and_then(|value| value.box_model.as_ref())
            .map(|model| model.border_box)
        else {
            continue;
        };
        let mut visible = rect.intersects(viewport);
        let mut ancestor = if positioning::viewport_fixed(id, by_id, styles) {
            None
        } else {
            by_id.get(&id).and_then(|node| node.parent)
        };
        while visible {
            let Some(parent_id) = ancestor else {
                break;
            };
            let Some(parent_node) = by_id.get(&parent_id).copied() else {
                visible = false;
                break;
            };
            let style = styles.get(&parent_id);
            // Inline ancestors split around block descendants can have no
            // principal box. Their absence is not a clip: continue through
            // them so a real outer scroll/clip container still participates.
            let boxless_inline = style.is_some_and(|style| {
                matches!(
                    style_value(style, "display", "block"),
                    "inline" | "contents"
                ) && style_value(style, "content-visibility", "visible") != "hidden"
            }) && geometry.get(&parent_id).is_none_or(|value| {
                value.availability == GeometryAvailability::Available && value.box_model.is_none()
            });
            if boxless_inline {
                ancestor = parent_node.parent;
                continue;
            }
            let Some(parent_geometry) = geometry.get(&parent_id) else {
                visible = false;
                break;
            };
            if parent_geometry.availability != GeometryAvailability::Available {
                visible = false;
                break;
            }
            let Some(parent_box) = parent_geometry.box_model.as_ref() else {
                visible = false;
                break;
            };
            let (clip_x, clip_y) = style.map_or((false, false), |style| overflow_clip_axes(style));
            if clip_x || clip_y {
                let clip = parent_box.padding_box;
                visible = (!clip_x || horizontal_overlap(rect, clip))
                    && (!clip_y || vertical_overlap(rect, clip));
            }
            ancestor = if positioning::viewport_fixed(parent_id, by_id, styles) {
                None
            } else {
                parent_node.parent
            };
        }
        if let Some(value) = geometry.get_mut(&id) {
            value.in_viewport = visible;
            value.interactable &= visible;
        }
    }
}

fn overflow_clip_axes(style: &BTreeMap<String, String>) -> (bool, bool) {
    let mut x = style_value(style, "overflow-x", "visible");
    let mut y = style_value(style, "overflow-y", "visible");
    // CSS Overflow converts a lone `visible` axis to `auto` when the other
    // axis establishes a scroll container.
    if x == "visible" && y != "visible" {
        x = "auto";
    }
    if y == "visible" && x != "visible" {
        y = "auto";
    }
    (overflow_clips(x), overflow_clips(y))
}

fn overflow_clips(value: &str) -> bool {
    matches!(value, "hidden" | "clip" | "scroll" | "auto")
}

fn horizontal_overlap(rect: CssRect, clip: CssRect) -> bool {
    rect.width > 0.0 && clip.width > 0.0 && rect.x < clip.right() && rect.right() > clip.x
}

fn vertical_overlap(rect: CssRect, clip: CssRect) -> bool {
    rect.height > 0.0 && clip.height > 0.0 && rect.y < clip.bottom() && rect.bottom() > clip.y
}

/// This renderer deliberately exposes only its bundled native font inventory.
/// CSS local() uses full/PostScript names, not arbitrary family aliases.
pub fn local_font_bytes(name: &str) -> Option<&'static [u8]> {
    use skrifa::{MetadataProvider, string::StringId};
    let font = FontRef::new(DEFAULT_FONT_BYTES).ok()?;
    [StringId::FULL_NAME, StringId::POSTSCRIPT_NAME]
        .into_iter()
        .any(|id| {
            font.localized_strings(id)
                .any(|value| value.to_string().eq_ignore_ascii_case(name))
        })
        .then_some(DEFAULT_FONT_BYTES)
}

/// Bound declared decompressed size before handing compressed font data to the decoder.
pub fn decode_web_font(bytes: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if bytes.len() > limit {
        return Err("font source exceeds byte limit".into());
    }
    let compressed = matches!(bytes.get(..4), Some(b"wOFF") | Some(b"wOF2"));
    if compressed {
        let size = bytes
            .get(16..20)
            .ok_or_else(|| "truncated compressed font header".to_owned())?;
        let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
        if size > limit {
            return Err("declared decoded font exceeds byte limit".into());
        }
    }
    let decoded = match bytes.get(..4) {
        Some(b"wOFF") => wuff::decompress_woff1(bytes).map_err(|error| error.to_string())?,
        Some(b"wOF2") => wuff::decompress_woff2(bytes).map_err(|error| error.to_string())?,
        _ => bytes.to_vec(),
    };
    if decoded.len() > limit || FontRef::new(&decoded).is_err() {
        return Err("decoded font is invalid or exceeds byte limit".into());
    }
    Ok(decoded)
}

fn prepare_web_fonts(input: &LayoutInput, limits: &LayoutLimits) -> PreparedFonts {
    let mut prepared = Vec::new();
    let mut loaded = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total = 0usize;
    for font in &input.web_fonts {
        if font.pending {
            prepared.push((
                font.family.clone(),
                Vec::new(),
                font.descriptors.clone(),
                true,
                font.blocked,
            ));
            continue;
        }
        if font.bytes.len() > limits.max_font_bytes {
            diagnostics.push(format!(
                "font {} rejected: {} bytes exceeds per-font limit {}",
                font.source_url,
                font.bytes.len(),
                limits.max_font_bytes
            ));
            continue;
        }
        if total.saturating_add(font.bytes.len()) > limits.max_page_font_bytes {
            diagnostics.push(format!(
                "font {} rejected: page font budget {} exceeded",
                font.source_url, limits.max_page_font_bytes
            ));
            continue;
        }
        let actual = format!("{:x}", Sha256::digest(&font.bytes));
        if actual != font.sha256 {
            diagnostics.push(format!(
                "font {} rejected: SHA-256 mismatch",
                font.source_url
            ));
            continue;
        }
        let decoded = match decode_web_font(&font.bytes, limits.max_font_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                diagnostics.push(format!("font {} rejected: {error}", font.source_url));
                continue;
            }
        };
        total = total.saturating_add(font.bytes.len());
        prepared.push((
            font.family.clone(),
            decoded,
            font.descriptors.clone(),
            false,
            false,
        ));
        loaded.push(LoadedFont {
            family: font.family.clone(),
            source_url: font.source_url.clone(),
            sha256: font.sha256.clone(),
            bytes: font.bytes.len(),
        });
    }
    (prepared, loaded, diagnostics)
}

fn validate(input: &LayoutInput, limits: &LayoutLimits) -> Result<(), LayoutError> {
    shadow_tree::validate(input)?;
    if !input.scrollbar_width.is_finite() || !(0.0..=128.0).contains(&input.scrollbar_width) {
        return Err(LayoutError::InvalidRenderedTree(
            "native scrollbar width must be finite within 0..128 CSS pixels".into(),
        ));
    }
    if input.nodes.len() > limits.max_nodes {
        return Err(LayoutError::NodeLimit {
            actual: input.nodes.len(),
            limit: limits.max_nodes,
        });
    }
    // Nested browsing contexts may have zero or fractional CSS dimensions.
    // Preserve that viewport for layout and viewport units; raster allocation
    // has its own minimum storage dimensions and must not change CSS geometry.
    for value in [input.viewport.width, input.viewport.height] {
        if !value.is_finite() || !(0.0..=limits.max_viewport_css_px).contains(&value) {
            return Err(LayoutError::ViewportLimit {
                value,
                limit: limits.max_viewport_css_px,
            });
        }
    }
    if !input.element_scrolls.is_empty() {
        let known = input
            .nodes
            .iter()
            .map(|node| node.id)
            .collect::<BTreeSet<_>>();
        for (node, (x, y)) in &input.element_scrolls {
            if !known.contains(node) || !x.is_finite() || !y.is_finite() {
                return Err(LayoutError::InvalidElementScroll(*node));
            }
        }
    }
    if let Some(tree) = &input.rendered_tree {
        let known = input
            .nodes
            .iter()
            .map(|node| node.id)
            .collect::<BTreeSet<_>>();
        if known.len() != input.nodes.len() {
            return Err(LayoutError::InvalidRenderedTree(
                "duplicate element identity".into(),
            ));
        }
        let mut parents = BTreeMap::new();
        for (parent, content) in tree {
            if !known.contains(parent) {
                return Err(LayoutError::InvalidRenderedTree(format!(
                    "unknown element {parent}"
                )));
            }
            for item in content {
                let LayoutContent::Element(child) = item else {
                    continue;
                };
                if !tree.contains_key(child) {
                    return Err(LayoutError::InvalidRenderedTree(format!(
                        "missing rendered child {child}"
                    )));
                }
                if parents.insert(*child, *parent).is_some() {
                    return Err(LayoutError::InvalidRenderedTree(format!(
                        "multiple occurrences of element {child}"
                    )));
                }
            }
        }
        let mut pending = tree
            .keys()
            .filter(|id| !parents.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        let mut visited = 0;
        while let Some(id) = pending.pop() {
            visited += 1;
            pending.extend(tree[&id].iter().filter_map(|item| {
                if let LayoutContent::Element(child) = item {
                    Some(*child)
                } else {
                    None
                }
            }));
        }
        if visited != tree.len() {
            return Err(LayoutError::InvalidRenderedTree(
                "cycle in projected ancestry".into(),
            ));
        }
    }
    Ok(())
}

#[allow(dead_code)]
#[cfg(test)]
fn parse_rules(stylesheets: &[String], input: &LayoutInput) -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut order = 0;
    for sheet in stylesheets {
        let lock = SharedRwLock::new();
        let media = ServoArc::new(lock.wrap(MediaList::empty()));
        let parsed = Stylesheet::from_str(
            sheet,
            UrlExtraData::from(
                url::Url::parse("about:blank").expect("constant Stylo stylesheet base URL"),
            ),
            Origin::Author,
            media,
            lock.clone(),
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );
        let guard = parsed.shared_lock.read();
        let contents = parsed.contents(&guard);
        collect_stylo_rules(
            contents.rules.read_with(&guard),
            &guard,
            input,
            &mut rules,
            &mut order,
        );
    }
    rules
}

#[allow(dead_code)]
#[cfg(test)]
fn collect_stylo_rules(
    css_rules: &CssRules,
    guard: &SharedRwLockReadGuard,
    input: &LayoutInput,
    output: &mut Vec<Rule>,
    order: &mut usize,
) {
    for css_rule in &css_rules.0 {
        match css_rule {
            CssRule::Style(rule) => {
                let rule = rule.read_with(guard);
                let mut serialized = String::new();
                rule.block
                    .read_with(guard)
                    .to_css(&mut serialized)
                    .expect("serializing a Stylo declaration block to String cannot fail");
                let declarations = declarations_with_importance(&serialized);
                for selector in rule.selectors.slice() {
                    output.push(Rule {
                        selector: selector.clone(),
                        declarations: declarations.clone(),
                        specificity: selector.specificity(),
                        order: *order,
                    });
                    *order += 1;
                }
            }
            CssRule::Media(media) => {
                let condition = media.media_queries.read_with(guard).to_css_string();
                if media_matches(&condition, input) {
                    collect_stylo_rules(media.rules.read_with(guard), guard, input, output, order);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
fn media_matches(condition: &str, input: &LayoutInput) -> bool {
    let condition = condition.to_ascii_lowercase();
    if let Some(value) = condition
        .split("min-width:")
        .nth(1)
        .and_then(|value| value.split(')').next())
    {
        return length(value.trim(), input.viewport.width, 16.0, input.viewport)
            <= input.viewport.width;
    }
    if let Some(value) = condition
        .split("max-width:")
        .nth(1)
        .and_then(|value| value.split(')').next())
    {
        return input.viewport.width
            <= length(value.trim(), input.viewport.width, 16.0, input.viewport);
    }
    if condition.contains("prefers-color-scheme: dark") {
        return input.color_scheme == ColorScheme::Dark;
    }
    if condition.contains("prefers-color-scheme: light") {
        return input.color_scheme == ColorScheme::Light;
    }
    if condition.contains("prefers-reduced-motion: reduce") {
        return input.reduced_motion;
    }
    true
}

#[allow(dead_code)]
#[cfg(test)]
fn cascade(
    node: &LayoutNode,
    by_id: &BTreeMap<u64, &LayoutNode>,
    parent: Option<&BTreeMap<String, String>>,
    rules: &[Rule],
) -> BTreeMap<String, String> {
    let mut style = ua_style(node);
    let initial = style.clone();
    // Match Stylo's HTML presentation hint: author declarations can override it.
    if node.attributes.contains_key("hidden") {
        style.insert("display".into(), "none".into());
    }
    for property in [
        "color",
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "line-height",
        "text-align",
        "text-indent",
        "text-align-last",
        "visibility",
        "white-space",
        "tab-size",
    ] {
        if let Some(value) = parent.and_then(|style| style.get(property)) {
            style.insert(property.into(), value.clone());
        }
    }
    let mut winners = BTreeMap::<String, (bool, u32, usize, String)>::new();
    for rule in rules
        .iter()
        .filter(|rule| stylo_selector_matches(node, &rule.selector, by_id))
    {
        for (property, (value, important)) in &rule.declarations {
            let candidate = (*important, rule.specificity, rule.order, value.clone());
            if winners.get(property).is_none_or(|current| {
                (candidate.0, candidate.1, candidate.2) >= (current.0, current.1, current.2)
            }) {
                winners.insert(property.clone(), candidate);
            }
        }
    }
    if let Some(inline) = node.attributes.get("style") {
        for (property, (value, important)) in declarations_with_importance(inline) {
            let candidate = (important, u32::MAX, usize::MAX, value);
            if winners.get(&property).is_none_or(|current| {
                (candidate.0, candidate.1, candidate.2) >= (current.0, current.1, current.2)
            }) {
                winners.insert(property, candidate);
            }
        }
    }
    for (property, (_, _, _, value)) in winners {
        let value = resolve_value(&property, &value, &style, parent, &initial);
        style.insert(property, value);
    }
    style
}

#[allow(dead_code)]
#[cfg(test)]
fn stylo_selector_matches(
    node: &LayoutNode,
    selector: &Selector<SelectorImpl>,
    by_id: &BTreeMap<u64, &LayoutNode>,
) -> bool {
    let element = LayoutElement { node, by_id };
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    matches_selector(selector, 0, None, &element, &mut context)
}

fn ua_style(node: &LayoutNode) -> BTreeMap<String, String> {
    let display = match node.tag.as_str() {
        "#text" => "inline",
        "html" | "body" | "main" | "section" | "article" | "header" | "footer" | "nav" | "div"
        | "p" | "form" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre"
        | "blockquote" => "block",
        "li" => "list-item",
        "table" => "table",
        "tr" => "table-row",
        "td" | "th" => "table-cell",
        "script" | "style" | "head" | "template" => "none",
        _ => "inline-block",
    };
    let (width, height) = match node.tag.as_str() {
        "input" => ("auto", "24px"),
        "button" | "select" => ("auto", "28px"),
        "textarea" => ("240px", "80px"),
        "canvas" => ("300px", "150px"),
        _ => ("auto", "auto"),
    };
    BTreeMap::from([
        ("display".into(), display.into()),
        ("position".into(), "static".into()),
        ("visibility".into(), "visible".into()),
        ("opacity".into(), "1".into()),
        ("pointer-events".into(), "auto".into()),
        ("box-sizing".into(), "content-box".into()),
        ("width".into(), width.into()),
        ("height".into(), height.into()),
        ("font-family".into(), DEFAULT_FONT_FAMILY.into()),
        ("font-size".into(), "16px".into()),
        ("line-height".into(), "normal".into()),
        (
            "white-space".into(),
            if node.tag == "pre"
                || (node.tag == "textarea"
                    && node
                        .attributes
                        .get("wrap")
                        .is_some_and(|v| v.eq_ignore_ascii_case("off")))
            {
                "pre"
            } else if node.tag == "textarea" {
                "pre-wrap"
            } else {
                "normal"
            }
            .into(),
        ),
        (
            "overflow-wrap".into(),
            if node.tag == "textarea"
                && !node
                    .attributes
                    .get("wrap")
                    .is_some_and(|v| v.eq_ignore_ascii_case("off"))
            {
                "break-word"
            } else {
                "normal"
            }
            .into(),
        ),
        (
            "overflow-x".into(),
            if node.tag == "textarea" {
                "auto"
            } else {
                "visible"
            }
            .into(),
        ),
        (
            "overflow-y".into(),
            if node.tag == "textarea" {
                "auto"
            } else {
                "visible"
            }
            .into(),
        ),
        ("flex-direction".into(), "row".into()),
        ("gap".into(), "0px".into()),
    ])
}

#[cfg(test)]
fn declarations_with_importance(input: &str) -> BTreeMap<String, (String, bool)> {
    let url_data = UrlExtraData::from(
        url::Url::parse("about:blank").expect("constant Stylo declaration base URL"),
    );
    let block = parse_style_attribute(
        input,
        &url_data,
        None,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    );
    let mut declarations = block
        .declaration_importance_iter()
        .filter_map(|(declaration, importance)| {
            let mut value = String::new();
            declaration.to_css(&mut value).ok()?;
            Some((
                declaration.id().name().into_owned(),
                (value, importance.important()),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    // Stylo's standalone Servo profile omits a small set of grid computed
    // structs. Preserve only those already-validated authored values for the
    // Taffy compatibility bridge; selector matching, normal declarations,
    // origins, importance, inheritance, and all non-grid values remain
    // Stylo-owned.
    for item in input.split(';') {
        let Some((name, raw_value)) = item.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if !matches!(
            name.as_str(),
            "display"
                | "grid-template-columns"
                | "grid-template-rows"
                | "grid-auto-flow"
                | "grid-auto-columns"
                | "grid-auto-rows"
                | "grid-column-start"
                | "grid-column-end"
                | "grid-row-start"
                | "grid-row-end"
                | "column-gap"
                | "row-gap"
                | "gap"
        ) {
            continue;
        }
        let raw_value = raw_value.trim();
        let important = raw_value
            .to_ascii_lowercase()
            .strip_suffix("!important")
            .is_some();
        let value = raw_value
            .strip_suffix("!important")
            .unwrap_or(raw_value)
            .trim();
        let supported = match name.as_str() {
            "display" => matches!(
                value,
                "none"
                    | "block"
                    | "inline"
                    | "inline-block"
                    | "flex"
                    | "grid"
                    | "table"
                    | "table-row"
                    | "table-cell"
            ),
            "grid-template-columns"
            | "grid-template-rows"
            | "grid-auto-flow"
            | "grid-auto-columns"
            | "grid-auto-rows"
            | "grid-column-start"
            | "grid-column-end"
            | "grid-row-start"
            | "grid-row-end"
            | "column-gap"
            | "row-gap"
            | "gap" => {
                !value.is_empty()
                    && value.len() <= 1024
                    && value.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || character.is_ascii_whitespace()
                            || matches!(
                                character,
                                '.' | '%' | '(' | ')' | ',' | '-' | '/' | '[' | ']'
                            )
                    })
            }
            _ => false,
        };
        if supported {
            declarations.insert(name, (value.to_string(), important));
        }
    }
    declarations
}

#[allow(dead_code)]
#[cfg(test)]
fn resolve_value(
    property: &str,
    value: &str,
    current: &BTreeMap<String, String>,
    parent: Option<&BTreeMap<String, String>>,
    initial: &BTreeMap<String, String>,
) -> String {
    if value == "inherit" {
        return parent
            .and_then(|style| style.get(property))
            .cloned()
            .or_else(|| initial.get(property).cloned())
            .unwrap_or_default();
    }
    if value == "initial" || value == "revert" || value == "revert-layer" {
        return initial.get(property).cloned().unwrap_or_default();
    }
    if value == "unset" {
        return parent
            .and_then(|style| style.get(property))
            .cloned()
            .or_else(|| initial.get(property).cloned())
            .unwrap_or_default();
    }
    let mut resolved = value.to_string();
    for _ in 0..16 {
        let Some(start) = resolved.find("var(") else {
            break;
        };
        let Some(relative_end) = resolved[start + 4..].find(')') else {
            break;
        };
        let end = start + 4 + relative_end;
        let expression = &resolved[start + 4..end];
        let (name, fallback) = expression.split_once(',').unwrap_or((expression, ""));
        let replacement = current
            .get(name.trim())
            .or_else(|| parent.and_then(|style| style.get(name.trim())))
            .cloned()
            .unwrap_or_else(|| fallback.trim().to_string());
        resolved.replace_range(start..=end, &replacement);
    }
    resolved
}

#[allow(clippy::too_many_arguments)]
fn layout_node(
    id: u64,
    containing: FlowBox,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    offset_dimensions: &mut BTreeMap<u64, (f64, f64)>,
    resolved_values: &mut BTreeMap<u64, BTreeMap<String, String>>,
    text_fragments: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    input: &LayoutInput,
    ancestor_hidden: bool,
    diagnostics: &mut Vec<String>,
    text_engine: &mut TextEngine,
    allocated: Option<TaffySize<f64>>,
) -> f64 {
    let Some(node) = by_id.get(&id).copied() else {
        return 0.0;
    };
    let formatting_style = tables::formatting_style(id, by_id, styles);
    let style = formatting_style.as_ref().unwrap_or(styles[&id].as_ref());
    let no_box = ancestor_hidden || style.get("display").is_some_and(|value| value == "none");
    let invisible = style
        .get("visibility")
        .is_some_and(|value| value == "hidden" || value == "collapse");
    if no_box {
        let mut pending = vec![id];
        let mut visited = BTreeSet::new();
        while let Some(hidden_id) = pending.pop() {
            if !visited.insert(hidden_id) {
                continue;
            }
            let Some(node) = by_id.get(&hidden_id).copied() else {
                continue;
            };
            if let Some(content) = &node.content {
                for item in content {
                    match item {
                        LayoutContent::Text { node_id, .. }
                        | LayoutContent::StyledText {
                            node_id: Some(node_id),
                            ..
                        } => {
                            text_engine.source_geometry.remove(node_id);
                        }
                        _ => {}
                    }
                }
            }
            geometry.insert(hidden_id, hidden_geometry());
            offset_dimensions.insert(hidden_id, (0.0, 0.0));
            pending.extend(node.children.iter().rev().copied());
        }
        return 0.0;
    }
    let containing = if let Some((root, inline)) = text_engine
        .column_context
        .filter(|(root, _)| columns::eligible_spanner(*root, id, by_id, styles))
    {
        let vertical =
            style_value(&styles[&root], "writing-mode", "horizontal-tb") != "horizontal-tb";
        if vertical {
            FlowBox {
                height: Some(inline),
                ..containing
            }
        } else {
            FlowBox {
                width: inline,
                ..containing
            }
        }
    } else {
        containing
    };
    let out_of_flow = matches!(
        style_value(style, "position", "static"),
        "absolute" | "fixed"
    );
    if out_of_flow && text_engine.active_out_of_flow != Some(id) {
        text_engine.out_of_flow.push((id, containing));
        return 0.;
    }
    let font_size = length(
        style_value(style, "font-size", "16px"),
        16.0,
        16.0,
        input.viewport,
    );
    let parent_vertical = node.parent.and_then(|id| styles.get(&id)).is_some_and(|s| {
        if matches!(
            style_value(s, "display", "block"),
            "table-row" | "table-row-group" | "table-header-group" | "table-footer-group"
        ) {
            tables::containing_axes(id, by_id, styles).vertical
        } else {
            writing_mode::Axes::new(style_value(s, "writing-mode", "horizontal-tb"), false).vertical
        }
    });
    let edge_basis = if parent_vertical {
        containing.height.unwrap_or(input.viewport.height)
    } else {
        containing.width
    };
    let mut margin = edge(style, "margin", edge_basis, font_size, input.viewport);
    let padding = edge(style, "padding", edge_basis, font_size, input.viewport);
    let border = text_engine
        .table_borders
        .get(&id)
        .copied()
        .unwrap_or_else(|| border_edge(style, containing.width, font_size, input.viewport));
    let resolve_inset = |name, basis| {
        optional_length(
            style_value(style, name, "auto"),
            basis,
            font_size,
            input.viewport,
        )
    };
    let left = out_of_flow
        .then(|| resolve_inset("left", containing.width))
        .flatten();
    let right = out_of_flow
        .then(|| resolve_inset("right", containing.width))
        .flatten();
    let available = text_engine
        .bfc_available_width
        .filter(|(owner, _)| *owner == id && !parent_vertical)
        .map(|(_, width)| width)
        .unwrap_or(
            containing.width
                - margin.left
                - margin.right
                - left.unwrap_or(0.)
                - right.unwrap_or(0.),
        )
        .max(0.0);
    let width_value = style_value(style, "width", "auto");
    let mut specified_width = (!is_intrinsic_width(width_value))
        .then(|| optional_length(width_value, containing.width, font_size, input.viewport))
        .flatten();
    let height_basis = containing.height;
    let mut specified_height = block_height_length(
        style_value(style, "height", "auto"),
        height_basis,
        font_size,
        input.viewport,
    );
    let border_box_sizing = style_value(style, "box-sizing", "content-box") == "border-box";
    if style_value(style, "height", "auto") == "auto"
        && writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        && !is_inline(style)
        && !is_replaced_box(node)
        && !out_of_flow
    {
        specified_height = Some(
            (text_engine
                .bfc_available_width
                .filter(|(owner, _)| *owner == id && parent_vertical)
                .map(|(_, width)| width)
                .unwrap_or(
                    containing.height.unwrap_or(input.viewport.height) - margin.top - margin.bottom,
                )
                - if border_box_sizing {
                    0.
                } else {
                    padding.top + padding.bottom + border.top + border.bottom
                })
            .max(0.),
        );
    }
    if out_of_flow && !is_replaced_box(node) {
        if width_value == "auto" && left.is_some() && right.is_some() {
            specified_width = Some(
                (available
                    - if border_box_sizing {
                        0.
                    } else {
                        padding.left + padding.right + border.left + border.right
                    })
                .max(0.),
            );
        }
        if style_value(style, "height", "auto") == "auto"
            && let Some((top, bottom)) = resolve_inset("top", height_basis.unwrap_or(0.))
                .zip(resolve_inset("bottom", height_basis.unwrap_or(0.)))
        {
            specified_height = height_basis.map(|height| {
                (height
                    - top
                    - bottom
                    - margin.top
                    - margin.bottom
                    - if border_box_sizing {
                        0.
                    } else {
                        padding.top + padding.bottom + border.top + border.bottom
                    })
                .max(0.)
            });
        }
    }
    let descendant_source = (node.text.is_empty()
        && is_inline(style)
        && !matches!(node.tag.as_str(), "input" | "button" | "option"))
    .then(|| text_source::TextSource::collect_visible(node, by_id, false, Some(styles)));
    let direct_source = text_source::TextSource::direct_with_breaks(node, by_id, styles);
    let intrinsic_text =
        if node.text.is_empty() && matches!(node.tag.as_str(), "input" | "button" | "option") {
            node.attributes.get("value").cloned().unwrap_or_default()
        } else if let Some(source) = &descendant_source {
            source.text.clone()
        } else {
            direct_source.text.clone()
        };
    let shaped_source = descendant_source
        .or(Some(direct_source))
        // Control values do not describe DOM text
        // ranges. Keep those synthetic runs separate from native provenance.
        .filter(|source| source.text == intrinsic_text);
    let horizontal_edges = padding.left + padding.right + border.left + border.right;
    let needs_intrinsic = [
        width_value,
        style_value(style, "min-width", "auto"),
        style_value(style, "max-width", "none"),
    ]
    .iter()
    .any(|value| is_intrinsic_width(value))
        || ((is_inline(style) || out_of_flow || style_value(style, "float", "none") != "none")
            && specified_width.is_none());
    let intrinsic_widths = needs_intrinsic.then(|| {
        if let Some((width, _)) = text_engine.select_sizes.get(&node.id) {
            return (*width, *width);
        }
        if let Some((width, _)) = node
            .intrinsic_image_size
            .or(node.document_replaced.then_some((300, 150)))
        {
            return (f64::from(width), f64::from(width));
        }
        if !node.children.is_empty() {
            return text_engine.intrinsic_node(
                node,
                by_id,
                styles,
                input.viewport,
                &mut BTreeSet::new(),
            );
        }
        let synthetic;
        let source = if let Some(source) = &shaped_source {
            source
        } else {
            synthetic = text_source::TextSource::synthetic(&intrinsic_text);
            &synthetic
        };
        text_engine.intrinsic_source(source, font_size, style, input.viewport, styles)
    });
    let intrinsic_width = intrinsic_widths.and_then(|widths| {
        resolve_intrinsic_width(
            width_value,
            (available - horizontal_edges).max(0.),
            widths,
            font_size,
            input.viewport,
        )
    });
    let mut content_width = intrinsic_width.or(specified_width).unwrap_or_else(|| {
        if let Some((width, _)) = node
            .intrinsic_image_size
            .or(node.document_replaced.then_some((300, 150)))
        {
            if let Some((specified_height, ratio)) =
                specified_height.zip(preferred_ratio(node, style))
            {
                let content_height = if border_box_sizing {
                    (specified_height - padding.top - padding.bottom - border.top - border.bottom)
                        .max(0.0)
                } else {
                    specified_height.max(0.0)
                };
                ratio.content_width(
                    content_height,
                    horizontal_edges,
                    padding.top + padding.bottom + border.top + border.bottom,
                )
            } else {
                f64::from(width)
            }
        } else if is_inline(style) || out_of_flow || style_value(style, "float", "none") != "none" {
            let (min, max) = intrinsic_widths.unwrap_or_default();
            max.min((available - horizontal_edges).max(min))
        } else {
            (available - padding.left - padding.right - border.left - border.right).max(0.0)
        }
    });
    if border_box_sizing && specified_width.is_some() && intrinsic_width.is_none() {
        content_width =
            (content_width - padding.left - padding.right - border.left - border.right).max(0.0);
    }
    let bound = |property, default| {
        let value = style_value(style, property, default);
        if is_intrinsic_width(value) {
            intrinsic_widths.and_then(|widths| {
                resolve_intrinsic_width(
                    value,
                    (available - horizontal_edges).max(0.),
                    widths,
                    font_size,
                    input.viewport,
                )
            })
        } else {
            optional_length(value, containing.width, font_size, input.viewport).map(|value| {
                if border_box_sizing {
                    (value - horizontal_edges).max(0.)
                } else {
                    value.max(0.)
                }
            })
        }
    };
    // CSS resolves a conflicting minimum in favor of min-width.
    if let Some(max) = bound("max-width", "none") {
        content_width = content_width.min(max);
    }
    if let Some(min) = bound("min-width", "auto") {
        content_width = content_width.max(min);
    }
    if node
        .intrinsic_image_size
        .or(node.document_replaced.then_some((300, 150)))
        .is_some()
        && width_value == "auto"
        && specified_height.is_none()
        && let Some(ratio) = preferred_ratio(node, style)
    {
        let vertical = padding.top + padding.bottom + border.top + border.bottom;
        let tentative_height = ratio.content_height(content_width, horizontal_edges, vertical);
        let constrained_height = clamp_content_height(
            style,
            tentative_height,
            height_basis,
            font_size,
            input.viewport,
            vertical,
        );
        // Transfer the height constraint back through the ratio, then
        // reapply width constraints. Incompatible axis bounds necessarily
        // break the ratio; final height clamping below honors those bounds.
        content_width = ratio.content_width(constrained_height, horizontal_edges, vertical);
        if let Some(maximum) = bound("max-width", "none") {
            content_width = content_width.min(maximum);
        }
        if let Some(minimum) = bound("min-width", "auto") {
            content_width = content_width.max(minimum);
        }
    }
    if let Some(size) = allocated {
        content_width = (size.width - horizontal_edges).max(0.);
    }
    if matches!(
        style_value(style, "display", "block"),
        "table" | "inline-table"
    ) {
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let table_inline = if vertical {
            specified_height.unwrap_or(containing.height.unwrap_or(input.viewport.height))
        } else {
            content_width
        };
        tables::prepare(
            node,
            table_inline,
            by_id,
            styles,
            input.viewport,
            text_engine,
        );
        if vertical {
            specified_height = Some(
                text_engine
                    .table_widths
                    .get(&id)
                    .copied()
                    .unwrap_or(table_inline),
            );
            if width_value == "auto" {
                content_width = text_engine
                    .table_block_sizes
                    .get(&id)
                    .copied()
                    .unwrap_or(content_width);
            }
        } else {
            content_width = text_engine
                .table_widths
                .get(&id)
                .copied()
                .unwrap_or(content_width);
        }
    }
    if let Some(row) = text_engine.table_rows.get(&id).filter(|row| row.vertical) {
        content_width = row.height;
        specified_height = Some(row.inline_size);
    }
    let scrollbar_gutter = text_engine
        .scrollbar_gutters
        .get(&id)
        .copied()
        .unwrap_or_default();
    let authored_padding = padding;
    content_width = (content_width - scrollbar_gutter.left - scrollbar_gutter.right).max(0.);
    let dimension_bound = bound;
    let bound = |property, default| {
        dimension_bound(property, default)
            .map(|value| (value - scrollbar_gutter.left - scrollbar_gutter.right).max(0.))
    };
    if !border_box_sizing {
        specified_height = specified_height
            .map(|height| (height - scrollbar_gutter.top - scrollbar_gutter.bottom).max(0.));
    }
    let padding = EdgeSizes {
        left: padding.left + scrollbar_gutter.left,
        right: padding.right + scrollbar_gutter.right,
        top: padding.top + scrollbar_gutter.top,
        bottom: padding.bottom + scrollbar_gutter.bottom,
    };
    let mut outer_width = content_width + padding.left + padding.right + border.left + border.right;
    let position = style_value(style, "position", "static");
    let containing_rtl = node
        .parent
        .and_then(|id| styles.get(&id))
        .is_some_and(|parent| style_value(parent, "direction", "ltr") == "rtl");
    if allocated.is_none()
        && !matches!(position, "absolute" | "fixed")
        && !writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        && style_value(style, "float", "none") == "none"
        && matches!(
            style_value(style, "display", "block"),
            "block" | "flow-root" | "flex" | "grid" | "list-item"
        )
    {
        let margin_space = text_engine
            .bfc_available_width
            .filter(|(owner, _)| *owner == id)
            .map(|(_, width)| width + margin.left + margin.right)
            .unwrap_or(containing.width);
        let (left, right) = block_inline_margins(
            margin_space,
            outer_width,
            margin.left,
            margin.right,
            style_value(style, "margin-left", "0px") == "auto",
            style_value(style, "margin-right", "0px") == "auto",
            containing_rtl,
        );
        margin.left = left;
        margin.right = right;
    }
    if allocated.is_none() && !out_of_flow {
        for child in &node.children {
            text_engine.clearance_nodes.remove(child);
            text_engine.absorbed_margins.remove(child);
            text_engine.absorbed_vertical_margins.remove(child);
        }
        if let Some((child, strut)) =
            margin_collapse::child_edge(crate::margin_collapse::ChildMarginEdgeInputs {
                id,
                bottom: false,
                width: content_width,
                nodes: by_id,
                styles,
                viewport: input.viewport,
                clear_obstacles: margin_collapse::clear_obstacles(
                    text_engine.float_context.as_ref(),
                    text_engine.float_block_offset,
                ),
                own_margin: margin.top,
            })
        {
            margin.top = strut.append(margin.top).value();
            text_engine.absorbed_margins.entry(child).or_insert([0.; 2])[0] = strut.value();
        }
        // Trailing margins use actual child clearance after native placement.
        let axes =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false);
        if axes.vertical {
            let inline = specified_height
                .map(|value| {
                    (value
                        - if border_box_sizing {
                            padding.top + padding.bottom + border.top + border.bottom
                        } else {
                            0.
                        })
                    .max(0.)
                })
                .or(containing.height)
                .unwrap_or(input.viewport.height);
            for after in [false] {
                if let Some((child, strut)) = margin_collapse::vertical_child_edge(
                    crate::margin_collapse::VerticalChildMarginInputs {
                        id,
                        after,
                        reverse: axes.x_reverse,
                        basis: inline,
                        nodes: by_id,
                        styles,
                        viewport: input.viewport,
                        clear_obstacles: margin_collapse::clear_obstacles(
                            text_engine.float_context.as_ref(),
                            text_engine.float_block_offset,
                        ),
                        own_margin: if after == axes.x_reverse {
                            margin.left
                        } else {
                            margin.right
                        },
                    },
                ) {
                    let left = after == axes.x_reverse;
                    if left {
                        margin.left = strut.append(margin.left).value();
                    } else {
                        margin.right = strut.append(margin.right).value();
                    }
                    text_engine
                        .absorbed_vertical_margins
                        .entry(child)
                        .or_insert([0.; 2])[if left { 0 } else { 1 }] = strut.value();
                }
            }
        }
    }
    let absorbed = text_engine
        .absorbed_margins
        .get(&id)
        .copied()
        .unwrap_or([0.; 2]);
    let absorbed_vertical = text_engine
        .absorbed_vertical_margins
        .get(&id)
        .copied()
        .unwrap_or([0.; 2]);
    let mut x = containing.x
        + if allocated.is_none() {
            margin.left - absorbed_vertical[0]
        } else {
            0.
        };
    let mut y = containing.y
        + if allocated.is_none() {
            margin.top - absorbed[0]
        } else {
            0.
        };
    if position == "fixed" {
        x = margin.left
            + optional_length(
                style_value(style, "left", "auto"),
                input.viewport.width,
                font_size,
                input.viewport,
            )
            .unwrap_or(0.0);
        y = margin.top
            + optional_length(
                style_value(style, "top", "auto"),
                input.viewport.height,
                font_size,
                input.viewport,
            )
            .unwrap_or(0.0);
    } else if position == "relative" {
        let (dx, dy) = relative_position_offset(
            style,
            containing.width,
            containing.height,
            containing_rtl,
            node.parent
                .and_then(|id| styles.get(&id))
                .map(|s| style_value(s, "writing-mode", "horizontal-tb"))
                .unwrap_or("horizontal-tb"),
            font_size,
            input.viewport,
        );
        x += dx;
        y += dy;
    } else if position == "absolute" {
        x += optional_length(
            style_value(style, "left", "auto"),
            containing.width,
            font_size,
            input.viewport,
        )
        .unwrap_or(0.0);
        y += optional_length(
            style_value(style, "top", "auto"),
            input.viewport.height,
            font_size,
            input.viewport,
        )
        .unwrap_or(0.0);
    }

    let axes = writing_mode::Axes::new(
        style_value(style, "writing-mode", "horizontal-tb"),
        style_value(style, "direction", "ltr") == "rtl",
    );
    let inline_size = if axes.vertical {
        allocated
            .map(|size| {
                (size.height - padding.top - padding.bottom - border.top - border.bottom).max(0.)
            })
            .or_else(|| {
                specified_height.map(|h| {
                    (h - if border_box_sizing {
                        padding.top + padding.bottom + border.top + border.bottom
                    } else {
                        0.
                    })
                    .max(0.)
                })
            })
            .or_else(|| text_engine.select_sizes.get(&node.id).map(|size| size.1))
            .or(containing.height)
            .unwrap_or(input.viewport.height)
    } else {
        content_width
    };
    let column_limit = if axes.vertical {
        specified_width.map(|value| {
            (value
                - if border_box_sizing {
                    horizontal_edges
                } else {
                    0.
                })
            .max(0.)
        })
    } else {
        specified_height.map(|value| {
            (value
                - if border_box_sizing {
                    padding.top + padding.bottom + border.top + border.bottom
                } else {
                    0.
                })
            .max(0.)
        })
    };
    let column_format = match columns::Columns::resolve(
        style,
        inline_size,
        column_limit,
        font_size,
        input.viewport,
    ) {
        Ok(columns) => columns,
        Err(error) => {
            diagnostics.push(format!("node {id}: {error}"));
            None
        }
    };
    let text_inline = column_format
        .map(|columns| columns.inline)
        .unwrap_or(inline_size);
    let float_scope = inline_formatting::enter_float_scope(
        text_engine,
        style,
        text_inline,
        if axes.vertical {
            y + border.top + padding.top
        } else {
            x + border.left + padding.left
        },
        if axes.vertical {
            if axes.x_reverse {
                margin.right + border.right + padding.right
            } else {
                x + border.left + padding.left
            }
        } else {
            y + border.top + padding.top
        },
        node.parent.is_none() || column_format.is_some(),
        axes.vertical,
    );
    let column_has_floats = column_format.is_some()
        && inline_formatting::has_flow_floats(
            id,
            by_id,
            styles,
            &mut text_engine.float_descendants,
        );
    let column_float_height = if column_has_floats {
        text_engine
            .column_float_trials
            .get(&id)
            .copied()
            .or_else(|| {
                column_format
                    .filter(|columns| !columns.balance)
                    .and_then(|columns| columns.limit)
            })
    } else {
        None
    };
    let column_deferred_start = text_engine.out_of_flow.len();
    if let Some(height) = column_float_height {
        text_engine.float_fragment = Some((height, 0.));
        text_engine.column_float_heights.insert(id, height);
    } else {
        text_engine.column_float_heights.remove(&id);
        text_engine.column_float_section_heights.remove(&id);
        text_engine.column_float_section_counts.remove(&id);
        text_engine.column_float_section_sizes.remove(&id);
    }
    let mut inline_context = None;
    let mut prepared_inline = None;
    let mut column_block_flow = false;
    let mut ruby_baseline = None;
    if matches!(
        style_value(style, "display", "block"),
        "ruby" | "block ruby"
    ) {
        let (context, layout, baseline) =
            ruby::layout(node, by_id, styles, font_size, input.viewport, text_engine);
        inline_context = Some(context);
        prepared_inline = Some(layout);
        ruby_baseline = Some(baseline);
    }
    if ruby_baseline.is_none()
        && !matches!(
            style_value(style, "display", "block"),
            "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "table"
                | "inline-table"
                | "table-row"
                | "ruby"
                | "block ruby"
        )
    {
        let flow = inline_formatting::collect(node, by_id, styles);
        let active=(axes.vertical&&matches!(style_value(style,"display","block"),"block"|"flow-root"|"list-item")&&!flow.is_empty())||text_engine.float_context.as_ref().is_some_and(|context|context.has_floats())||inline_formatting::has_flow_floats(id,by_id,styles,&mut text_engine.float_descendants)||column_format.is_some()&&!flow.is_empty()||flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Paragraph(paragraph) if !paragraph.inline_nodes.is_empty()||!paragraph.atomic.is_empty()))
            ||flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Block(child) if styles.get(child).is_some_and(|style|style_value(style,"clear","none")!="none")))
            ||(flow.len()>1&&flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Paragraph(paragraph) if has_text_line_content(&paragraph.source,styles,style))));
        if active {
            let previous_columns = text_engine.column_context;
            if column_format.is_some() {
                text_engine.column_context = Some((id, inline_size));
                text_engine.column_float_section_index.insert(id, 0);
            }
            let (context, layout, has_blocks) =
                inline_formatting::layout_flow(crate::inline_formatting::InlineFlowInputs {
                    flow,
                    style,
                    inline: text_inline,
                    full_inline: inline_size,
                    block_available: content_width,
                    block_limit: specified_height,
                    font_size,
                    nodes: by_id,
                    styles,
                    input,
                    geometry,
                    offsets: offset_dimensions,
                    resolved: resolved_values,
                    paint: text_fragments,
                    diagnostics,
                    engine: text_engine,
                });
            text_engine.column_context = previous_columns;
            inline_context = Some(context);
            prepared_inline = Some(layout);
            column_block_flow = has_blocks || column_has_floats;
        }
    }
    let mut text_layout = if let Some(layout) = prepared_inline {
        layout
    } else if let Some(source) = &shaped_source {
        text_engine.shape_source(
            source,
            text_inline.max(1.0),
            font_size,
            style,
            input.viewport,
            styles,
        )
    } else {
        text_engine.shape_styled(
            &intrinsic_text,
            text_inline.max(1.0),
            font_size,
            style,
            input.viewport,
        )
    };
    let float_bfc_extent = inline_formatting::leave_float_scope(text_engine, float_scope);
    if column_has_floats {
        text_layout.height = text_layout.height.max(float_bfc_extent.unwrap_or(0.));
        text_engine
            .column_float_extents
            .insert(id, text_layout.height);
    }
    if let Some(columns) = column_format.filter(|columns| {
        (columns.balance || columns.limit.is_none())
            && column_has_floats
            && column_float_height.is_none()
    }) {
        let natural = text_layout.height;
        // First replay establishes the complete set partition from positioned
        // native geometry, including eligible spanners nested in ordinary blocks.
        let mut high = vec![natural.max(0.01)];
        let mut low = high.clone();
        // Every candidate is freshly formatted against its fragment boundaries.
        // Bound total replay work across nested multicol contexts as well as
        // each binary search; exhaustion is an explicit incomplete layout.
        for pass in 0..16 {
            if text_engine.column_balance_work >= 128 {
                diagnostics.push(format!(
                    "node {id}: native float column balance exceeds 128 re-layout work budget"
                ));
                mark_unsupported_subtree(id, "float column balance work budget", by_id, geometry);
                return 0.;
            }
            text_engine.column_balance_work += 1;
            let targets = low
                .iter()
                .zip(&high)
                .map(|(low, high)| if pass == 15 { *high } else { (low + high) / 2. })
                .collect::<Vec<_>>();
            let target = targets[0];
            text_engine.out_of_flow.truncate(column_deferred_start);
            text_engine.column_float_trials.insert(id, target);
            text_engine
                .column_float_section_heights
                .insert(id, targets.clone());
            let used = layout_node(
                id,
                containing,
                by_id,
                styles,
                geometry,
                offset_dimensions,
                resolved_values,
                text_fragments,
                input,
                ancestor_hidden,
                diagnostics,
                text_engine,
                allocated,
            );
            text_engine.column_float_trials.remove(&id);
            if pass == 15 {
                return used;
            }
            if pass == 0 {
                let sizes = text_engine
                    .column_float_section_sizes
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| vec![natural]);
                high = sizes
                    .iter()
                    .map(|size| {
                        columns
                            .limit
                            .map(|limit| size.min(limit))
                            .unwrap_or(*size)
                            .max(0.01)
                    })
                    .collect();
                low = sizes
                    .iter()
                    .zip(&high)
                    .map(|(size, high)| (size / columns.count as f64).max(0.01).min(*high))
                    .collect();
                continue;
            }
            let counts = text_engine.column_float_section_counts.get(&id);
            for (index, target) in targets.into_iter().enumerate() {
                let fits = counts
                    .and_then(|counts| counts.get(index))
                    .is_some_and(|count| *count <= columns.count);
                if fits {
                    high[index] = target
                } else {
                    low[index] = target;
                }
            }
        }
    }
    if ruby_baseline.is_some() && width_value == "auto" && allocated.is_none() {
        content_width = if axes.vertical {
            text_layout.height
        } else {
            text_layout.width
        };
        if let Some(max) = bound("max-width", "none") {
            content_width = content_width.min(max);
        }
        if let Some(min) = bound("min-width", "auto") {
            content_width = content_width.max(min);
        }
        outer_width =
            content_width + horizontal_edges + scrollbar_gutter.left + scrollbar_gutter.right;
    }
    if let Some(columns) = column_format.filter(|_| !column_block_flow) {
        columns.place(&mut text_layout);
        let step = (columns.inline + columns.gap).max(1.);
        let count = text_layout
            .lines
            .iter()
            .map(|line| {
                ((if columns.reverse {
                    columns.available - columns.inline - line.4
                } else {
                    line.4
                }) / step)
                    .round()
                    .max(0.) as usize
                    + 1
            })
            .max()
            .unwrap_or(0);
        text_engine.column_runs.insert(
            id,
            vec![columns::ColumnRun {
                columns,
                count,
                start: 0.,
                block: text_layout.height,
            }],
        );
    }
    if axes.vertical {
        if allocated.is_none()
            && !out_of_flow
            && inline_context.is_some()
            && let Some((child, value)) = margin_collapse::used_child_vertical_end(
                crate::margin_collapse::VerticalChildEndInputs {
                    id,
                    reverse: axes.x_reverse,
                    basis: inline_size,
                    nodes: by_id,
                    styles,
                    viewport: input.viewport,
                    geometry,
                    clearance: &text_engine.clearance_nodes,
                },
            )
        {
            text_layout.height = (text_layout.height - value).max(0.);
            for item in &mut text_layout.inline_boxes {
                if item.node == child {
                    item.rect.height = (item.rect.height - value).max(0.);
                }
            }
            if axes.x_reverse {
                let previous = margin.left;
                margin.left = inline_formatting::MarginStrut::default()
                    .append(previous)
                    .append(value)
                    .value();
                x += margin.left - previous;
            } else {
                margin.right = inline_formatting::MarginStrut::default()
                    .append(margin.right)
                    .append(value)
                    .value();
            }
            text_engine
                .absorbed_vertical_margins
                .entry(child)
                .or_insert([0.; 2])[if axes.x_reverse { 0 } else { 1 }] = value;
        }
        if width_value == "auto"
            && allocated.is_none()
            && (node.children.is_empty() || inline_context.is_some())
            && specified_width.is_none()
        {
            content_width = text_layout.height.max(float_bfc_extent.unwrap_or(0.));
            if let Some(max) = bound("max-width", "none") {
                content_width = content_width.min(max);
            }
            if let Some(min) = bound("min-width", "auto") {
                content_width = content_width.max(min);
            }
            outer_width =
                content_width + horizontal_edges + scrollbar_gutter.left + scrollbar_gutter.right;
        }
        vertical_text::physicalize(
            &mut text_layout,
            content_width,
            inline_size,
            style_value(style, "writing-mode", "horizontal-tb"),
        );
    }
    let mut resolved = text_layout.resolved_text.clone();
    for run in &mut resolved {
        run.layout_node = id;
    }
    text_engine.resolved_text.insert(id, resolved);
    if text_engine.font_demand_error.is_none() {
        match text_pseudos::font_demands(&text_layout) {
            Ok(demands) => {
                let old = text_engine
                    .font_demands
                    .get(&id)
                    .map(text_pseudos::demand_bytes)
                    .unwrap_or(0);
                let bytes = text_engine
                    .font_demand_bytes
                    .saturating_sub(old)
                    .saturating_add(text_pseudos::demand_bytes(&demands));
                if bytes > 64 * 1024 * 1024 {
                    text_engine.font_demand_error = Some(LayoutError::StyleTraversal(
                        "page font demand exceeds native 64-MiB bound".into(),
                    ));
                } else {
                    text_engine.font_demand_bytes = bytes;
                    text_engine.font_demands.insert(id, demands);
                }
            }
            Err(error) => text_engine.font_demand_error = Some(error),
        }
    }
    for line_break in &text_layout.breaks {
        let mut line_break = line_break.clone();
        line_break.layout_node = id;
        let shift = |rect: &mut CssRect| {
            rect.x += x + border.left + padding.left
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.x
                };
            rect.y += y + border.top + padding.top
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.y
                };
        };
        shift(&mut line_break.before);
        if let Some(after) = &mut line_break.after {
            shift(after);
        }
        text_engine.break_boxes.insert(line_break.node, line_break);
    }
    let mut source_geometry = BTreeMap::<u64, Vec<oryn_common::v4::TextClusterGeometry>>::new();
    for cluster in &text_layout.source_clusters {
        let rect = CssRect {
            x: x + border.left + padding.left + cluster.rect.x
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.x
                },
            y: y + border.top + padding.top + cluster.rect.y
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.y
                },
            ..cluster.rect
        };
        source_geometry.entry(cluster.text_node).or_default().push(
            oryn_common::v4::TextClusterGeometry {
                layout_node: if inline_context
                    .as_ref()
                    .is_some_and(|paragraph| paragraph.inline_nodes.contains(&cluster.owner))
                {
                    cluster.owner
                } else {
                    id
                },
                start: cluster.utf16.start,
                end: cluster.utf16.end,
                rect,
                matrix: transforms::Projection::IDENTITY.matrix(),
                line: cluster.line,
                rtl: cluster.rtl,
            },
        );
    }
    // Deeper direct-text layout supersedes ancestor aggregate measurements.
    text_engine.source_geometry.extend(source_geometry);
    let mut intrinsic_text_height = text_layout.height;
    let display = style_value(style, "display", "block");
    let vertical_edges = padding.top + padding.bottom + border.top + border.bottom;
    let clamp_height = |value| {
        let gutter = scrollbar_gutter.top + scrollbar_gutter.bottom;
        (clamp_content_height(
            style,
            value + gutter,
            height_basis,
            font_size,
            input.viewport,
            vertical_edges - gutter,
        ) - gutter)
            .max(0.)
    };
    // An auto height remains indefinite even when min-height gives the box a
    // used height. Percentage descendants must not use that minimum as a basis.
    let definite_content_height = allocated
        .map(|size| (size.height - vertical_edges).max(0.))
        .or_else(|| {
            specified_height.map(|value| {
                clamp_height(
                    (value
                        - if border_box_sizing {
                            vertical_edges
                        } else {
                            0.
                        })
                    .max(0.),
                )
            })
        });
    let child_x = x + border.left + padding.left;
    let child_y = y
        + border.top
        + padding.top
        + if axes.vertical {
            0.
        } else {
            intrinsic_text_height
        };
    let mut children_height: f64 = 0.0;
    let has_float_or_clear = node.children.iter().any(|child| {
        styles.get(child).is_some_and(|child_style| {
            style_value(child_style, "float", "none") != "none"
                || style_value(child_style, "clear", "none") != "none"
        })
    });
    if let Some(paragraph) = &inline_context {
        for item in &text_layout.inline_boxes {
            let target_x = x + border.left + padding.left + item.rect.x
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.x
                };
            let target_y = y + border.top + padding.top + item.rect.y
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.y
                };
            if let Some(model) = geometry
                .get(&item.node)
                .and_then(|value| value.box_model.clone())
            {
                positioning::translate_subtree(
                    item.node,
                    target_x
                        - (model.border_box.x - model.margin.left
                            + text_engine
                                .absorbed_vertical_margins
                                .get(&item.node)
                                .map(|value| value[0])
                                .unwrap_or(0.)),
                    target_y
                        - (model.border_box.y - model.margin.top
                            + text_engine
                                .absorbed_margins
                                .get(&item.node)
                                .map(|value| value[0])
                                .unwrap_or(0.)),
                    by_id,
                    geometry,
                    text_fragments,
                    text_engine,
                );
            }
            for (child, containing) in &mut text_engine.out_of_flow {
                if *child == item.node {
                    containing.x = target_x + input.scroll.x;
                    containing.y = target_y + input.scroll.y;
                }
            }
        }
        for child in paragraph.atomic.iter().chain(paragraph.boundaries.keys()) {
            text_engine.inline_sizes.remove(child);
        }
    } else if matches!(
        display,
        "flex" | "inline-flex" | "grid" | "inline-grid" | "table-row"
    ) || has_float_or_clear
    {
        let container_height = definite_content_height;
        let Some(placements) = taffy_child_placements(ChildPlacementInputs {
            node,
            parent_style: style,
            styles,
            width: content_width,
            height: container_height,
            viewport: input.viewport,
            nodes: by_id,
            text_engine,
        }) else {
            let diagnostic = "layout computation failed for this flex/grid/float subtree; geometry is unavailable";
            diagnostics.push(format!("node {id}: {diagnostic}"));
            mark_unsupported_subtree(id, diagnostic, by_id, geometry);
            return 0.0;
        };
        for (child, placement) in node.children.iter().zip(placements) {
            let child_height = layout_node(
                *child,
                FlowBox {
                    x: child_x + placement.x,
                    y: child_y + placement.y,
                    width: content_width,
                    height: definite_content_height,
                },
                by_id,
                styles,
                geometry,
                offset_dimensions,
                resolved_values,
                text_fragments,
                input,
                false,
                diagnostics,
                text_engine,
                Some(TaffySize {
                    width: placement.width,
                    height: placement.height,
                }),
            );
            if !styles.get(child).is_some_and(|s| {
                matches!(style_value(s, "position", "static"), "absolute" | "fixed")
            }) {
                children_height = children_height.max(placement.y + child_height);
            }
        }
    } else if axes.vertical {
        let mut block_cursor = if axes.x_reverse {
            x + border.left + padding.left + content_width - text_layout.width
        } else {
            x + border.left + padding.left + text_layout.width
        };
        let mut block_extent = text_layout.width;
        for child in &tables::ordered_children(node, styles) {
            layout_node(
                *child,
                FlowBox {
                    x: block_cursor,
                    y: y + border.top + padding.top,
                    width: content_width,
                    height: Some(inline_size),
                },
                by_id,
                styles,
                geometry,
                offset_dimensions,
                resolved_values,
                text_fragments,
                input,
                false,
                diagnostics,
                text_engine,
                None,
            );
            if styles.get(child).is_some_and(|s| {
                matches!(style_value(s, "position", "static"), "absolute" | "fixed")
            }) {
                continue;
            }
            let Some(model) = geometry.get(child).and_then(|g| g.box_model.clone()) else {
                continue;
            };
            let extent = model.margin.left + model.border_box.width + model.margin.right;
            if axes.x_reverse {
                positioning::translate_subtree(
                    *child,
                    -extent,
                    0.,
                    by_id,
                    geometry,
                    text_fragments,
                    text_engine,
                );
                block_cursor -= extent;
            } else {
                block_cursor += extent;
            }
            block_extent += extent;
            children_height = children_height
                .max(model.border_box.height + model.margin.top + model.margin.bottom);
        }
        block_extent += text_engine
            .table_end_spacing
            .get(&id)
            .copied()
            .unwrap_or(0.);
        if width_value == "auto" && allocated.is_none() && specified_width.is_none() {
            let previous = content_width;
            content_width = block_extent.max(0.);
            if let Some(max) = bound("max-width", "none") {
                content_width = content_width.min(max);
            }
            if let Some(min) = bound("min-width", "auto") {
                content_width = content_width.max(min);
            }
            outer_width =
                content_width + horizontal_edges + scrollbar_gutter.left + scrollbar_gutter.right;
            if axes.x_reverse {
                let dx = content_width - previous;
                for child in &node.children {
                    positioning::translate_subtree(
                        *child,
                        dx,
                        0.,
                        by_id,
                        geometry,
                        text_fragments,
                        text_engine,
                    );
                }
                for line in &mut text_layout.lines {
                    line.4 += dx;
                }
                for clusters in text_engine.source_geometry.values_mut() {
                    for cluster in clusters {
                        if cluster.layout_node == id {
                            cluster.rect.x += dx;
                        }
                    }
                }
            }
        }
    } else {
        let mut cursor = child_y;
        let mut preceding_margin: Option<inline_formatting::MarginStrut> = None;
        for child in &tables::ordered_children(node, styles) {
            let child_style = &styles[child];
            let collapse = !display.starts_with("table")
                && !axes.vertical
                && matches!(
                    style_value(child_style, "display", "block"),
                    "block" | "flow-root" | "list-item" | "flex" | "grid"
                )
                && !matches!(
                    style_value(child_style, "position", "static"),
                    "absolute" | "fixed"
                );
            let mut incoming = inline_formatting::MarginStrut::default();
            if collapse {
                let top = margin_collapse::outer_edge(
                    *child,
                    false,
                    content_width,
                    by_id,
                    styles,
                    input.viewport,
                    margin_collapse::clear_obstacles(
                        text_engine.float_context.as_ref(),
                        text_engine.float_block_offset,
                    ),
                );
                let previous = preceding_margin.take().unwrap_or_default();
                incoming = previous.merge(top);
                let delta = incoming.value() - previous.value() - top.value();
                cursor += delta;
                children_height += delta;
            }
            let child_height = layout_node(
                *child,
                FlowBox {
                    x: child_x,
                    y: cursor,
                    width: content_width,
                    height: definite_content_height,
                },
                by_id,
                styles,
                geometry,
                offset_dimensions,
                resolved_values,
                text_fragments,
                input,
                false,
                diagnostics,
                text_engine,
                None,
            );
            cursor += child_height;
            children_height += child_height;
            if collapse {
                if let Some(model) = geometry
                    .get(child)
                    .and_then(|value| value.box_model.as_ref())
                {
                    let through = text_engine
                        .absorbed_margins
                        .get(child)
                        .is_none_or(|value| *value == [0.; 2])
                        && model.border_box.height == 0.
                        && model.border.top == 0.
                        && model.border.bottom == 0.
                        && model.padding.top == 0.
                        && model.padding.bottom == 0.
                        && !inline_formatting::establishes_bfc(child_style)
                        && !text_engine.clearance_nodes.contains(child);
                    if through {
                        let joined = incoming.append(model.margin.bottom);
                        let delta = joined.value() - incoming.value() - model.margin.bottom;
                        cursor += delta;
                        children_height += delta;
                        preceding_margin = Some(joined);
                    } else {
                        preceding_margin = Some(
                            if text_engine
                                .absorbed_margins
                                .get(child)
                                .is_some_and(|value| value[1] != 0.)
                            {
                                inline_formatting::MarginStrut::default()
                            } else {
                                inline_formatting::MarginStrut::default()
                                    .append(model.margin.bottom)
                            },
                        );
                    }
                }
            } else if child_height != 0. {
                preceding_margin = None;
            }
        }
    }
    if let Some(columns) = column_format.filter(|_| column_block_flow) {
        let origin = (
            x + border.left + padding.left
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.x
                },
            y + border.top + padding.top
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.y
                },
        );
        match columns::fragment_flow(crate::columns::ColumnFragmentFlow {
            columns,
            root: id,
            origin,
            scroll: (input.scroll.x, input.scroll.y),
            block_extent: content_width,
            mode: style_value(style, "writing-mode", "horizontal-tb"),
            nodes: by_id,
            styles,
            geometry,
            offsets: offset_dimensions,
            text: text_fragments,
            layout: &mut text_layout,
            engine: text_engine,
        }) {
            Ok(frame) => {
                intrinsic_text_height = text_layout.height;
                if axes.vertical
                    && width_value == "auto"
                    && allocated.is_none()
                    && specified_width.is_none()
                {
                    content_width = frame;
                    if let Some(max) = bound("max-width", "none") {
                        content_width = content_width.min(max);
                    }
                    if let Some(min) = bound("min-width", "auto") {
                        content_width = content_width.max(min);
                    }
                    outer_width = content_width
                        + horizontal_edges
                        + scrollbar_gutter.left
                        + scrollbar_gutter.right;
                }
            }
            Err(error) => diagnostics.push(format!("node {id}: {error}")),
        }
    }
    if let Some(row) = text_engine.table_rows.get(&id) {
        children_height = if row.vertical {
            row.inline_size
        } else {
            row.height
        };
    }
    if !axes.vertical {
        children_height += text_engine
            .table_end_spacing
            .get(&id)
            .copied()
            .unwrap_or(0.);
    }
    let mut content_height = specified_height
        .unwrap_or_else(|| {
            node.intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
                .map_or(
                    if axes.vertical {
                        intrinsic_text_height.max(children_height)
                    } else {
                        intrinsic_text_height + children_height
                    },
                    |(_, height)| {
                        preferred_ratio(node, style)
                            .map(|ratio| {
                                ratio.content_height(
                                    content_width,
                                    horizontal_edges,
                                    vertical_edges,
                                )
                            })
                            .unwrap_or(f64::from(height))
                    },
                )
        })
        .max(0.0);
    if border_box_sizing && specified_height.is_some() {
        content_height =
            (content_height - padding.top - padding.bottom - border.top - border.bottom).max(0.0);
    }
    if column_float_height.is_none()
        && !axes.vertical
        && specified_height.is_none()
        && let Some(extent) = float_bfc_extent
    {
        content_height = content_height.max(extent);
    }
    if specified_height.is_none()
        && let Some(context) = inline_context.as_ref()
        && !axes.vertical
        && (node.parent.is_none()
            || matches!(
                display,
                "flow-root" | "inline-block" | "inline-flex" | "inline-grid"
            )
            || out_of_flow
            || style_value(style, "float", "none") != "none"
            || ["overflow-x", "overflow-y"]
                .iter()
                .any(|key| !matches!(style_value(style, key, "visible"), "visible" | "clip")))
    {
        for child in &context.atomic {
            if style_value(&styles[child], "float", "none") != "none"
                && let Some(model) = geometry
                    .get(child)
                    .and_then(|value| value.box_model.as_ref())
            {
                content_height = content_height.max(
                    model.border_box.bottom() + model.margin.bottom
                        - (y + border.top + padding.top
                            - if position == "fixed" {
                                0.
                            } else {
                                input.scroll.y
                            }),
                );
            }
        }
    }
    if allocated.is_none()
        && !out_of_flow
        && !axes.vertical
        && let Some((child, value)) = margin_collapse::used_child_bottom(
            id,
            content_width,
            by_id,
            styles,
            input.viewport,
            geometry,
            &text_engine.clearance_nodes,
        )
    {
        content_height = (content_height - value).max(0.);
        margin.bottom = inline_formatting::MarginStrut::default()
            .append(margin.bottom)
            .append(value)
            .value();
        text_engine.absorbed_margins.entry(child).or_insert([0.; 2])[1] = value;
    }
    content_height = clamp_height(content_height);
    if let Some(size) = allocated {
        content_height = (size.height - vertical_edges).max(0.);
    }
    let border_width = outer_width.max(0.0);
    let border_height = content_height + padding.top + padding.bottom + border.top + border.bottom;
    offset_dimensions.insert(id, (border_width, border_height));
    if let Some(baseline) = ruby_baseline {
        text_engine
            .ruby_baselines
            .insert(id, margin.top + border.top + padding.top + baseline);
    }
    // CSSOM resolves dimensions before transforms. Visibility suppresses paint,
    // not layout boxes. Non-replaced inline boxes have no used width/height.
    let replaced = is_replaced_box(node);
    if !matches!(display, "none" | "contents") && (display != "inline" || replaced) {
        let mut values = BTreeMap::new();
        if !matches!(
            display,
            "table-row" | "table-row-group" | "table-header-group" | "table-footer-group"
        ) {
            let width = if border_box_sizing {
                border_width
            } else {
                content_width + scrollbar_gutter.left + scrollbar_gutter.right
            };
            values.insert("width".into(), format!("{width}px"));
        }
        if !matches!(display, "table-column" | "table-column-group") {
            let height = if border_box_sizing {
                border_height
            } else {
                content_height + scrollbar_gutter.top + scrollbar_gutter.bottom
            };
            values.insert("height".into(), format!("{height}px"));
        }
        resolved_values.insert(id, values);
    }

    let reference_rect = CssRect {
        x,
        y,
        width: border_width,
        height: border_height,
    };
    let to_viewport = |fragment: CssRect| {
        let document_rect = fragment;
        CssRect {
            x: document_rect.x
                - if position == "fixed" {
                    0.0
                } else {
                    input.scroll.x
                },
            y: document_rect.y
                - if position == "fixed" {
                    0.0
                } else {
                    input.scroll.y
                },
            ..document_rect
        }
    };
    if let (Some(baseline), Some(line)) = (text_layout.baselines.first(), text_layout.lines.first())
    {
        text_engine.first_line_baselines.insert(
            id,
            if axes.vertical {
                line.4 + baseline
            } else {
                line.3 + baseline
            },
        );
    } else {
        text_engine.first_line_baselines.remove(&id);
    }
    let rect = to_viewport(reference_rect);
    let viewport_rect = CssRect {
        x: 0.0,
        y: 0.0,
        width: input.viewport.width,
        height: input.viewport.height,
    };
    let opacity = style_value(style, "opacity", "1")
        .parse::<f64>()
        .unwrap_or(1.0);
    let pointer_events = style_value(style, "pointer-events", "auto");
    let in_viewport = rect.intersects(viewport_rect);
    let client_rects =
        if intrinsic_text.is_empty() || style_value(style, "display", "block") != "inline" {
            // Atomic/block boxes contribute their border box, including zero-sized
            // boxes. Text paint lines inside them are not element client fragments.
            vec![rect]
        } else {
            text_layout
                .lines
                .iter()
                .map(|(_, width, height, top, left)| {
                    to_viewport(CssRect {
                        x: x + border.left + padding.left + left,
                        y: y + border.top + padding.top + top,
                        width: (*width).min(content_width),
                        height: *height,
                    })
                })
                .collect()
        };
    if let Some(paragraph) = &inline_context {
        let text_x = rect.x + border.left + padding.left;
        let text_y = rect.y + border.top + padding.top;
        inline_formatting::inline_geometry(crate::inline_formatting::InlineGeometryInputs {
            content_width,
            content_height,
            paragraph,
            layout: &text_layout,
            root: id,
            x: text_x,
            y: text_y,
            nodes: by_id,
            styles,
            viewport: input.viewport,
            inline_basis: text_inline,
            geometry,
            offsets: offset_dimensions,
        });
        for (owner, fragments) in
            inline_formatting::paint_fragments(&text_layout, id, text_x, text_y, styles)
        {
            text_fragments.entry(owner).or_default().extend(fragments);
        }
        for child in &paragraph.inline_nodes {
            if let Some(style) = styles.get(child)
                && style_value(style, "position", "static") == "relative"
            {
                let (dx, dy) = relative_position_offset(
                    style,
                    content_width,
                    definite_content_height,
                    style_value(style, "direction", "ltr") == "rtl",
                    style_value(style, "writing-mode", "horizontal-tb"),
                    font_size,
                    input.viewport,
                );
                positioning::translate_subtree(
                    *child,
                    dx,
                    dy,
                    by_id,
                    geometry,
                    text_fragments,
                    text_engine,
                );
            }
        }
    } else if !intrinsic_text.is_empty() && !invisible {
        let color = style_value(style, "color", "#000000").to_string();
        let font_family = style_value(style, "font-family", DEFAULT_FONT_FAMILY).to_string();
        let fragments = text_layout
            .lines
            .iter()
            .enumerate()
            .map(
                |(line, (text, width, height, top, left))| PaintTextFragment {
                    flow_line: Some(line),
                    flow_rect: Some({
                        let mut flow = text_layout
                            .flow_rect(line, style_value(style, "writing-mode", "horizontal-tb"));
                        flow.x += rect.x + border.left + padding.left;
                        flow.y += rect.y + border.top + padding.top;
                        flow
                    }),
                    clusters: text_layout.paint_clusters(
                        line,
                        rect.x + border.left + padding.left,
                        rect.y + border.top + padding.top,
                    ),
                    text: text.clone(),
                    glyph_runs: text_layout.paint_runs(line),
                    rect: CssRect {
                        x: rect.x + border.left + padding.left + left,
                        y: rect.y + border.top + padding.top + top,
                        width: *width,
                        height: *height,
                    },
                    font_family: font_family.clone(),
                    font_size,
                    color: color.clone(),
                },
            )
            .collect::<Vec<_>>();
        text_fragments.insert(id, fragments);
    }
    geometry.insert(
        id,
        NodeGeometry {
            availability: GeometryAvailability::Available,
            box_model: Some(BoxModel {
                border_box: rect,
                padding_box: to_viewport(inset(reference_rect, border)),
                content_box: to_viewport(inset(inset(reference_rect, border), padding)),
                margin,
                border,
                padding: authored_padding,
            }),
            client_rects,
            in_viewport,
            interactable: in_viewport
                && rect.width > 0.0
                && rect.height > 0.0
                && opacity > 0.0
                && pointer_events != "none",
            diagnostic: None,
        },
    );
    if invisible {
        // Hidden visibility retains layout boxes for CSSOM. Keep the semantic
        // and painting availability hidden, unlike display:none's absent box.
        if let Some(value) = geometry.get_mut(&id) {
            value.availability = GeometryAvailability::Hidden;
            value.in_viewport = false;
            value.interactable = false;
        }
    }
    if matches!(position, "absolute" | "fixed") {
        0.0
    } else {
        (if allocated.is_none() {
            margin.top - absorbed[0]
        } else {
            0.
        }) + border_height
            + margin.bottom
            - absorbed[1]
    }
}

#[cfg(test)]
fn descendant_text(node: &LayoutNode, by_id: &BTreeMap<u64, &LayoutNode>) -> String {
    text_source::TextSource::collect(node, by_id, false).text
}

/// Ask Taffy to place direct block/flex/grid children. Oryn retains ownership of
/// DOM identity and recursively measures text with Parley, while Taffy owns
/// the container algorithm and therefore the relational x/y/width result.
fn taffy_child_placements(params: ChildPlacementInputs<'_>) -> Option<Vec<ChildPlacement>> {
    let ChildPlacementInputs {
        node,
        parent_style,
        styles,
        width,
        height,
        viewport,
        nodes,
        text_engine,
    } = params;

    if node.children.is_empty() {
        return Some(Vec::new());
    }
    if let Some(row) = text_engine.table_rows.get(&node.id) {
        return Some(
            node.children
                .iter()
                .map(|id| {
                    row.cells.get(id).copied().unwrap_or(ChildPlacement {
                        x: 0.,
                        y: 0.,
                        width: 0.,
                        height: 0.,
                    })
                })
                .collect(),
        );
    }
    let logical_parent = parent_style;
    let axes = writing_mode::Axes::new(
        style_value(logical_parent, "writing-mode", "horizontal-tb"),
        style_value(logical_parent, "direction", "ltr") == "rtl",
    );
    let physical_parent = writing_mode::container(logical_parent);
    let parent_style = &physical_parent;
    let parent_display = style_value(parent_style, "display", "block");
    let display = match parent_display {
        "flex" | "inline-flex" => Display::Flex,
        "grid" | "inline-grid" | "table-row" => Display::Grid,
        _ => Display::Block,
    };
    let font_size = length(
        style_value(parent_style, "font-size", "16px"),
        16.0,
        16.0,
        viewport,
    );
    let column_gap = layout_gap(
        style_value(
            parent_style,
            "column-gap",
            style_value(parent_style, "gap", "0px"),
        ),
        width,
        font_size,
        viewport,
    );
    let row_gap = layout_gap(
        style_value(
            parent_style,
            "row-gap",
            style_value(parent_style, "gap", "0px"),
        ),
        height.unwrap_or(0.),
        font_size,
        viewport,
    );
    let mut tree = TaffyTree::<(u64, (f64, f64))>::new();
    tree.disable_rounding();
    let mut children = Vec::with_capacity(node.children.len());
    for child in &node.children {
        let physical_child = writing_mode::item(styles.get(child)?, logical_parent);
        let child_style = &physical_child;
        let child_font_size = length(
            style_value(child_style, "font-size", "16px"),
            16.0,
            font_size,
            viewport,
        );
        let child_node = nodes.get(child)?;
        let intrinsic =
            text_engine.intrinsic_node(child_node, nodes, styles, viewport, &mut BTreeSet::new());
        let resolve_width = |value: &str| {
            if is_intrinsic_width(value) {
                resolve_intrinsic_width(value, width, intrinsic, child_font_size, viewport)
                    .map(|value| taffy_length(value as f32))
                    .unwrap_or_else(auto)
            } else {
                grid_flex_dimension(value, Some(width), child_font_size, viewport)
            }
        };
        let resolve_height =
            |value: &str| grid_flex_dimension(value, height, child_font_size, viewport);
        let mut child_size = TaffySize {
            width: resolve_width(style_value(child_style, "width", "auto")).into(),
            height: resolve_height(style_value(child_style, "height", "auto")).into(),
        };
        let column_axis = matches!(
            style_value(parent_style, "flex-direction", "row"),
            "column" | "column-reverse"
        );
        let basis_value = style_value(child_style, "flex-basis", "auto");
        // Taffy's auto basis consults the preferred main size. For CSS content,
        // suppress that preferred size so its measurement callback supplies the
        // content contribution; retain the cross size and explicit constraints.
        if display == Display::Flex && basis_value == "content" {
            if column_axis {
                child_size.height = auto();
            } else {
                child_size.width = auto();
            }
        }
        let float = match style_value(child_style, "float", "none") {
            "left" => Float::Left,
            "right" => Float::Right,
            _ => Float::None,
        };
        let clear = match style_value(child_style, "clear", "none") {
            "left" => Clear::Left,
            "right" => Clear::Right,
            "both" => Clear::Both,
            _ => Clear::None,
        };
        let padding = edge(
            child_style,
            "padding",
            if axes.vertical {
                height.unwrap_or(viewport.height)
            } else {
                width
            },
            child_font_size,
            viewport,
        );
        let border = border_edge(child_style, width, child_font_size, viewport);
        let margin = edge(
            child_style,
            "margin",
            if axes.vertical {
                height.unwrap_or(viewport.height)
            } else {
                width
            },
            child_font_size,
            viewport,
        );
        let margin_value = |side: &str, resolved: f64| {
            if style_value(child_style, side, "0px") == "auto" {
                auto()
            } else {
                taffy_length(resolved as f32)
            }
        };
        let leaf = tree
            .new_leaf_with_context(
                TaffyStyle {
                    display: if style_value(child_style, "display", "block") == "none" {
                        Display::None
                    } else {
                        Display::Block
                    },
                    position: if matches!(
                        style_value(child_style, "position", "static"),
                        "absolute" | "fixed"
                    ) {
                        taffy::style::Position::Absolute
                    } else {
                        taffy::style::Position::Relative
                    },
                    box_sizing: if style_value(child_style, "box-sizing", "content-box")
                        == "border-box"
                    {
                        taffy::style::BoxSizing::BorderBox
                    } else {
                        taffy::style::BoxSizing::ContentBox
                    },
                    padding: taffy::Rect {
                        left: taffy_length(padding.left as f32),
                        right: taffy_length(padding.right as f32),
                        top: taffy_length(padding.top as f32),
                        bottom: taffy_length(padding.bottom as f32),
                    },
                    border: taffy::Rect {
                        left: taffy_length(border.left as f32),
                        right: taffy_length(border.right as f32),
                        top: taffy_length(border.top as f32),
                        bottom: taffy_length(border.bottom as f32),
                    },
                    margin: taffy::Rect {
                        left: margin_value("margin-left", margin.left),
                        right: margin_value("margin-right", margin.right),
                        top: margin_value("margin-top", margin.top),
                        bottom: margin_value("margin-bottom", margin.bottom),
                    },
                    size: child_size,
                    overflow: layout_overflow(child_style),
                    min_size: TaffySize {
                        width: resolve_width(style_value(child_style, "min-width", "auto")),
                        height: resolve_height(style_value(child_style, "min-height", "auto")),
                    },
                    max_size: TaffySize {
                        width: resolve_width(style_value(child_style, "max-width", "none")),
                        height: resolve_height(style_value(child_style, "max-height", "none")),
                    },
                    float,
                    clear,
                    direction: if style_value(child_style, "direction", "ltr") == "rtl" {
                        taffy::style::Direction::Rtl
                    } else {
                        taffy::style::Direction::Ltr
                    },
                    align_self: item_alignment(style_value(child_style, "align-self", "auto")),
                    justify_self: item_alignment(style_value(child_style, "justify-self", "auto")),
                    grid_column: grid_line(
                        style_value(child_style, "grid-column-start", "auto"),
                        style_value(child_style, "grid-column-end", "auto"),
                    ),
                    grid_row: grid_line(
                        style_value(child_style, "grid-row-start", "auto"),
                        style_value(child_style, "grid-row-end", "auto"),
                    ),
                    aspect_ratio: preferred_aspect_ratio(child_node, child_style)
                        .map(|ratio| ratio as f32),
                    flex_grow: style_value(child_style, "flex-grow", "0")
                        .parse::<f32>()
                        .unwrap_or(0.)
                        .max(0.),
                    flex_shrink: style_value(child_style, "flex-shrink", "1")
                        .parse::<f32>()
                        .unwrap_or(1.)
                        .max(0.),
                    flex_basis: if column_axis {
                        resolve_height(basis_value).into()
                    } else {
                        resolve_width(basis_value).into()
                    },
                    ..TaffyStyle::default()
                },
                (*child, intrinsic),
            )
            .ok()?;
        children.push(leaf);
    }
    let mut grid_style = grid_container_style(
        parent_style,
        parent_display,
        node.children.len(),
        width,
        height,
        font_size,
        viewport,
    );
    if axes.vertical
        && matches!(parent_display, "grid" | "inline-grid")
        && let Some(areas) = &mut grid_style.grid_template_areas
    {
        std::mem::swap(&mut areas.row_count, &mut areas.column_count);
        for area in &mut areas.areas {
            std::mem::swap(&mut area.row_start, &mut area.column_start);
            std::mem::swap(&mut area.row_end, &mut area.column_end);
        }
    }
    // CSS order changes layout participation, not DOM identity. Stable sorting
    // retains authored order for ties; result lookup below uses original handles.
    let mut ordered_indices = (0..node.children.len()).collect::<Vec<_>>();
    oryn_common::paint_order::sort_siblings(&mut ordered_indices, parent_display, |index| {
        styles
            .get(&node.children[*index])
            .and_then(|style| style.get("order"))
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0)
    });
    let ordered_children = ordered_indices
        .iter()
        .map(|index| children[*index])
        .collect::<Vec<_>>();
    let root = tree
        .new_with_children(
            TaffyStyle {
                display,
                overflow: layout_overflow(parent_style),
                direction: if style_value(parent_style, "direction", "ltr") == "rtl" {
                    taffy::style::Direction::Rtl
                } else {
                    taffy::style::Direction::Ltr
                },
                flex_direction: match style_value(parent_style, "flex-direction", "row") {
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => FlexDirection::Row,
                },
                flex_wrap: match style_value(parent_style, "flex-wrap", "nowrap") {
                    "wrap" => taffy::style::FlexWrap::Wrap,
                    "wrap-reverse" => taffy::style::FlexWrap::WrapReverse,
                    _ => taffy::style::FlexWrap::NoWrap,
                },
                align_items: item_alignment(style_value(parent_style, "align-items", "normal")),
                justify_items: item_alignment(style_value(parent_style, "justify-items", "normal")),
                align_content: content_alignment(style_value(
                    parent_style,
                    "align-content",
                    "normal",
                )),
                justify_content: content_alignment(style_value(
                    parent_style,
                    "justify-content",
                    "normal",
                )),
                size: TaffySize {
                    width: taffy_length(width as f32),
                    height: height
                        .map(|value| taffy_length(value as f32))
                        .unwrap_or_else(auto),
                },
                gap: TaffySize {
                    width: column_gap,
                    height: row_gap,
                },
                grid_auto_flow: match style_value(parent_style, "grid-auto-flow", "row") {
                    "column" => GridAutoFlow::Column,
                    "column dense" | "dense column" => GridAutoFlow::ColumnDense,
                    "row dense" | "dense" | "dense row" => GridAutoFlow::RowDense,
                    _ => GridAutoFlow::Row,
                },
                grid_template_columns: grid_style.grid_template_columns,
                grid_template_rows: grid_style.grid_template_rows,
                grid_template_areas: grid_style.grid_template_areas,
                grid_template_column_names: grid_style.grid_template_column_names,
                grid_template_row_names: grid_style.grid_template_row_names,
                grid_auto_columns: grid_style.grid_auto_columns,
                grid_auto_rows: grid_style.grid_auto_rows,
                ..TaffyStyle::default()
            },
            &ordered_children,
        )
        .ok()?;
    // Taffy can request the same child height repeatedly while resolving flex
    // bases, cross sizes, and grid tracks. Keep measurements local to this
    // immutable tree calculation: a later layout may have different styles,
    // fonts, inline boxes, or float context even at the same width.
    let mut measured_heights = BTreeMap::new();
    tree.compute_layout_with_measure(
        root,
        TaffySize {
            width: AvailableSpace::Definite(width as f32),
            height: height
                .map(|value| AvailableSpace::Definite(value as f32))
                .unwrap_or(AvailableSpace::MaxContent),
        },
        |inputs, _, context, taffy_style| {
            taffy::compute::compute_leaf_layout(
                inputs,
                taffy_style,
                |_, _| 0.,
                |known, available| {
                    let Some((id, intrinsic)) = context.as_deref() else {
                        return known.unwrap_or(TaffySize::ZERO);
                    };
                    let Some(child) = nodes.get(id) else {
                        return known.unwrap_or(TaffySize::ZERO);
                    };
                    let gutter = text_engine
                        .scrollbar_gutters
                        .get(id)
                        .copied()
                        .unwrap_or_default();
                    let gx = gutter.left + gutter.right;
                    let gy = gutter.top + gutter.bottom;
                    if styles.get(id).is_some_and(|s| {
                        writing_mode::Axes::new(
                            style_value(s, "writing-mode", "horizontal-tb"),
                            false,
                        )
                        .vertical
                    }) {
                        let inline = known
                            .height
                            .map(f64::from)
                            .unwrap_or_else(|| match available.height {
                                AvailableSpace::Definite(value) => f64::from(value),
                                _ => viewport.height,
                            });
                        let measured_width = known.width.map(f64::from).unwrap_or_else(|| {
                            text_engine.vertical_block_extent(
                                child,
                                (inline - gy).max(0.),
                                nodes,
                                styles,
                                viewport,
                                &mut BTreeSet::new(),
                            ) + gx
                        });
                        return TaffySize {
                            width: measured_width as f32,
                            height: inline as f32,
                        };
                    }
                    let measured_width =
                        known
                            .width
                            .map(f64::from)
                            .unwrap_or_else(|| match available.width {
                                AvailableSpace::MinContent => intrinsic.0,
                                AvailableSpace::MaxContent => intrinsic.1,
                                AvailableSpace::Definite(value) => {
                                    intrinsic.1.min(f64::from(value).max(intrinsic.0))
                                }
                            });
                    let measured_height = known.height.map(f64::from).unwrap_or_else(|| {
                        let content_width = (measured_width - gx).max(0.);
                        *measured_heights
                            .entry((*id, content_width.to_bits()))
                            .or_insert_with(|| {
                                text_engine.measure_content_height(
                                    child,
                                    content_width,
                                    nodes,
                                    styles,
                                    viewport,
                                )
                            })
                            + gy
                    });
                    TaffySize {
                        width: measured_width as f32,
                        height: measured_height as f32,
                    }
                },
            )
        },
    )
    .ok()?;
    let flex_row = matches!(
        style_value(logical_parent, "flex-direction", "row"),
        "row" | "row-reverse"
    );
    let nowrap = style_value(logical_parent, "flex-wrap", "nowrap") == "nowrap";
    let reverse_x = axes.vertical
        && axes.x_reverse
        && (display == Display::Grid || (display == Display::Flex && flex_row && nowrap));
    let reverse_y = axes.vertical
        && axes.y_reverse
        && (display == Display::Grid || (display == Display::Flex && !flex_row && nowrap));
    children
        .iter()
        .map(|child| {
            let layout = tree.layout(*child).ok()?;
            Some(ChildPlacement {
                x: if reverse_x {
                    width - f64::from(layout.location.x + layout.size.width)
                } else {
                    f64::from(layout.location.x)
                },
                y: if reverse_y {
                    height.unwrap_or_else(|| {
                        tree.layout(root)
                            .map(|l| f64::from(l.size.height))
                            .unwrap_or(0.)
                    }) - f64::from(layout.location.y + layout.size.height)
                } else {
                    f64::from(layout.location.y)
                },
                width: f64::from(layout.size.width),
                height: f64::from(layout.size.height),
            })
        })
        .collect()
}

fn item_alignment(value: &str) -> Option<taffy::style::AlignItems> {
    use taffy::style::{AlignItems, AlignItemsKeyword as K, AlignmentSafety};
    let value = value.trim();
    let (value, safety) = if let Some(value) = value.strip_prefix("safe ") {
        (value, AlignmentSafety::Safe)
    } else {
        (
            value.strip_prefix("unsafe ").unwrap_or(value),
            AlignmentSafety::Unsafe,
        )
    };
    let keyword = match value {
        "start" => K::Start,
        "end" => K::End,
        "flex-start" => K::FlexStart,
        "flex-end" => K::FlexEnd,
        "self-start" => K::SelfStart,
        "self-end" => K::SelfEnd,
        "center" => K::Center,
        "stretch" => K::Stretch,
        "baseline" | "first baseline" => K::Baseline,
        _ => return None,
    };
    Some(AlignItems { keyword, safety })
}

fn content_alignment(value: &str) -> Option<taffy::style::AlignContent> {
    use taffy::style::{AlignContent, AlignContentKeyword as K, AlignmentSafety};
    let value = value.trim();
    let (value, safety) = if let Some(value) = value.strip_prefix("safe ") {
        (value, AlignmentSafety::Safe)
    } else {
        (
            value.strip_prefix("unsafe ").unwrap_or(value),
            AlignmentSafety::Unsafe,
        )
    };
    let keyword = match value {
        "start" => K::Start,
        "end" => K::End,
        "flex-start" => K::FlexStart,
        "flex-end" => K::FlexEnd,
        "center" => K::Center,
        "stretch" => K::Stretch,
        "space-between" => K::SpaceBetween,
        "space-around" => K::SpaceAround,
        "space-evenly" => K::SpaceEvenly,
        _ => return None,
    };
    Some(AlignContent { keyword, safety })
}

/// Percentage gaps are resolved by the layout algorithm on their own axis.
/// Flex/grid `normal` gaps are zero (multicolumn has separate normal behavior).
fn layout_gap(
    value: &str,
    basis: f64,
    font_size: f64,
    viewport: ViewportState,
) -> taffy::style::LengthPercentage {
    use taffy::style::LengthPercentage;
    if let Some(percent) = value
        .trim()
        .strip_suffix('%')
        .and_then(|value| value.parse::<f32>().ok())
        && percent.is_finite()
        && percent >= 0.
    {
        return LengthPercentage::percent(percent / 100.);
    }
    LengthPercentage::length(length(value, basis, font_size, viewport).max(0.) as f32)
}

/// Auto overflow has the same automatic-minimum behavior as hidden. Taffy's
/// Scroll variant participates in automatic minima. Native classic gutters
/// are reserved by the outer formatting pass, avoiding double reservation here.
fn layout_overflow(style: &BTreeMap<String, String>) -> taffy::Point<taffy::style::Overflow> {
    use taffy::style::Overflow;
    let axis = |name| match style_value(style, name, "visible") {
        "clip" => Overflow::Clip,
        "hidden" | "auto" => Overflow::Hidden,
        "scroll" => Overflow::Scroll,
        _ => Overflow::Visible,
    };
    taffy::Point {
        x: axis("overflow-x"),
        y: axis("overflow-y"),
    }
}

#[derive(Clone, Copy)]
struct PreferredRatio {
    value: f64,
    border_box: bool,
}

impl PreferredRatio {
    fn content_width(self, height: f64, horizontal_edges: f64, vertical_edges: f64) -> f64 {
        if self.border_box {
            ((height + vertical_edges) * self.value - horizontal_edges).max(0.)
        } else {
            height * self.value
        }
    }
    fn content_height(self, width: f64, horizontal_edges: f64, vertical_edges: f64) -> f64 {
        if self.border_box {
            ((width + horizontal_edges) / self.value - vertical_edges).max(0.)
        } else {
            width / self.value
        }
    }
}

fn preferred_aspect_ratio(node: &LayoutNode, style: &BTreeMap<String, String>) -> Option<f64> {
    preferred_ratio(node, style).map(|ratio| ratio.value)
}

/// `auto <ratio>` prefers a replaced element's natural ratio once available.
/// A degenerate authored ratio behaves as auto, rather than dividing by zero.
fn preferred_ratio(node: &LayoutNode, style: &BTreeMap<String, String>) -> Option<PreferredRatio> {
    let natural = if let Some(intrinsics) = node.image_intrinsics {
        intrinsics.ratio().map(|value| PreferredRatio {
            value,
            border_box: false,
        })
    } else {
        node.intrinsic_image_size.and_then(|(width, height)| {
            (width > 0 && height > 0).then(|| PreferredRatio {
                value: f64::from(width) / f64::from(height),
                border_box: false,
            })
        })
    };
    let mut input = ParserInput::new(style_value(style, "aspect-ratio", "auto"));
    let mut parser = Parser::new(&mut input);
    let auto_first = parser
        .try_parse(|parser| parser.expect_ident_matching("auto"))
        .is_ok();
    if auto_first && parser.is_exhausted() {
        return natural;
    }
    let numerator = parser.expect_number().ok().map(f64::from)?;
    let denominator = if parser.try_parse(|parser| parser.expect_delim('/')).is_ok() {
        f64::from(parser.expect_number().ok()?)
    } else {
        1.
    };
    let auto_last = !auto_first
        && parser
            .try_parse(|parser| parser.expect_ident_matching("auto"))
            .is_ok();
    if !parser.is_exhausted() || numerator < 0. || denominator < 0. {
        return None;
    }
    if (auto_first || auto_last) && natural.is_some() {
        return natural;
    }
    let ratio = numerator / denominator;
    if ratio.is_finite() && ratio > 0. {
        Some(PreferredRatio {
            value: ratio,
            border_box: style_value(style, "box-sizing", "content-box") == "border-box",
        })
    } else {
        natural
    }
}

/// Leave percentages symbolic: grid areas and flex main/cross axes define
/// their percentage basis during layout.
fn grid_flex_dimension(
    value: &str,
    definite_basis: Option<f64>,
    font_size: f64,
    viewport: ViewportState,
) -> taffy::style::LengthPercentageAuto {
    use taffy::style::LengthPercentageAuto;
    let value = value.trim();
    if let Some(percent) = value
        .strip_suffix('%')
        .and_then(|value| value.parse::<f32>().ok())
    {
        return if percent.is_finite() && percent >= 0. {
            LengthPercentageAuto::percent(percent / 100.)
        } else {
            LengthPercentageAuto::auto()
        };
    }
    if matches!(value, "auto" | "none" | "normal")
        || (value.contains('%') && definite_basis.is_none())
    {
        return LengthPercentageAuto::auto();
    }
    let resolved = oryn_common::css_math::resolve_length(
        value,
        oryn_common::css_math::LengthContext {
            percentage_basis: definite_basis.unwrap_or(0.),
            font_size,
            root_font_size: 16.,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
        },
    );
    resolved
        .filter(|value| value.is_finite())
        .map(|value| LengthPercentageAuto::length(value.max(0.) as f32))
        .unwrap_or_else(LengthPercentageAuto::auto)
}

/// Bridge CSS length tokens to Taffy's px/percentage/fr track parser without
/// mistaking repeat counts, line identifiers or numbers inside math for lengths.
fn normalize_grid_tracks(
    value: &str,
    basis: Option<f64>,
    font_size: f64,
    viewport: ViewportState,
) -> Option<String> {
    fn walk<'i>(
        parser: &mut Parser<'i, '_>,
        mut track: bool,
        repetition: bool,
        basis: Option<f64>,
        font_size: f64,
        viewport: ViewportState,
        depth: usize,
    ) -> Result<String, cssparser::ParseError<'i, ()>> {
        use cssparser::Token;
        if depth > 128 {
            return Err(parser.new_custom_error(()));
        }
        let mut output = String::new();
        while !parser.is_exhausted() {
            let token = parser.next()?.clone();
            let serialized = cssparser::ToCss::to_css_string(&token);
            let value = match token {
                Token::Function(ref name) => {
                    let repeat = name.eq_ignore_ascii_case("repeat");
                    let nested_tracks = name.eq_ignore_ascii_case("minmax")
                        || name.eq_ignore_ascii_case("fit-content");
                    let inner = parser.parse_nested_block(|parser| {
                        walk(
                            parser,
                            nested_tracks,
                            repeat,
                            basis,
                            font_size,
                            viewport,
                            depth + 1,
                        )
                    })?;
                    let expression = format!("{serialized}{inner})");
                    if track
                        && !repeat
                        && !nested_tracks
                        && (basis.is_some() || !expression.contains('%'))
                    {
                        oryn_common::css_math::resolve_length(
                            &expression,
                            oryn_common::css_math::LengthContext {
                                percentage_basis: basis.unwrap_or(0.),
                                font_size,
                                root_font_size: 16.,
                                viewport_width: viewport.width,
                                viewport_height: viewport.height,
                            },
                        )
                        .filter(|value| value.is_finite())
                        .map(|value| format!("{}px", value.max(0.)))
                        .unwrap_or(expression)
                    } else {
                        expression
                    }
                }
                Token::SquareBracketBlock => {
                    let inner = parser.parse_nested_block(|parser| {
                        walk(parser, false, false, basis, font_size, viewport, depth + 1)
                    })?;
                    format!("[{inner}]")
                }
                Token::ParenthesisBlock => {
                    let inner = parser.parse_nested_block(|parser| {
                        walk(parser, false, false, basis, font_size, viewport, depth + 1)
                    })?;
                    format!("({inner})")
                }
                Token::Number { value, .. } if track && value == 0. => "0px".into(),
                Token::Dimension { .. } if track => oryn_common::css_math::resolve_length(
                    &serialized,
                    oryn_common::css_math::LengthContext {
                        percentage_basis: 0.,
                        font_size,
                        root_font_size: 16.,
                        viewport_width: viewport.width,
                        viewport_height: viewport.height,
                    },
                )
                .filter(|value| value.is_finite())
                .map(|value| format!("{value}px"))
                .unwrap_or(serialized),
                Token::Comma => {
                    if repetition {
                        track = true;
                    }
                    serialized
                }
                _ => serialized,
            };
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(&value);
        }
        Ok(output)
    }
    let mut input = ParserInput::new(value);
    walk(
        &mut Parser::new(&mut input),
        true,
        false,
        basis,
        font_size,
        viewport,
        0,
    )
    .ok()
}

/// Transfer complete track definitions, including repeat fragments and line names.
/// `none` leaves the explicit grid empty; implicit tracks use grid-auto-* sizing.
fn grid_container_style(
    style: &BTreeMap<String, String>,
    display: &str,
    child_count: usize,
    width: f64,
    height: Option<f64>,
    font_size: f64,
    viewport: ViewportState,
) -> TaffyStyle {
    use taffy::style::{GridAutoTracks, GridTemplateComponent, GridTemplateTracks};
    let mut result = TaffyStyle::default();
    if display == "table-row" {
        result.grid_template_columns = vec![fr(1.); child_count.max(1)];
        return result;
    }
    if !matches!(display, "grid" | "inline-grid") {
        return result;
    }
    result.grid_template_areas =
        parse_grid_template_areas(style_value(style, "grid-template-areas", "none"));
    let normalized = |property, default, basis| {
        normalize_grid_tracks(
            style_value(style, property, default),
            basis,
            font_size,
            viewport,
        )
        .unwrap_or_else(|| style_value(style, property, default).to_owned())
    };
    if let Ok(columns) = normalized("grid-template-columns", "none", Some(width))
        .parse::<GridTemplateTracks<_, GridTemplateComponent<_>>>()
    {
        result.grid_template_columns = columns.tracks;
        result.grid_template_column_names = columns.line_names;
    }
    if let Ok(rows) = normalized("grid-template-rows", "none", height)
        .parse::<GridTemplateTracks<_, GridTemplateComponent<_>>>()
    {
        result.grid_template_rows = rows.tracks;
        result.grid_template_row_names = rows.line_names;
    }
    if let Ok(columns) =
        normalized("grid-auto-columns", "auto", Some(width)).parse::<GridAutoTracks>()
    {
        result.grid_auto_columns = columns.0;
    }
    if let Ok(rows) = normalized("grid-auto-rows", "auto", height).parse::<GridAutoTracks>() {
        result.grid_auto_rows = rows.0;
    }
    result
}

/// Parse CSS string rows and validate that every named area is a rectangle.
/// Dot runs denote one empty cell, even without whitespace between a dot run
/// and a name. Keep empty outer rows/columns in the template dimensions.
fn parse_grid_template_areas(value: &str) -> Option<taffy::style::GridTemplateAreas<String>> {
    use taffy::style::{GridTemplateArea, GridTemplateAreas};
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut row_count = 0usize;
    let mut column_count = None;
    // Bounds are zero-based exclusive; conversion to grid lines happens below.
    let mut bounds: BTreeMap<String, (usize, usize, usize, usize, usize)> = BTreeMap::new();
    while !parser.is_exhausted() {
        let row = parser.expect_string().ok()?.to_string();
        let mut chars = row.chars().peekable();
        let mut column = 0usize;
        while let Some(character) = chars.peek().copied() {
            if matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                chars.next();
                continue;
            }
            if character == '.' {
                while chars.peek() == Some(&'.') {
                    chars.next();
                }
            } else {
                let mut name = String::new();
                while let Some(character) = chars.peek().copied() {
                    if character.is_ascii_alphanumeric()
                        || matches!(character, '_' | '-')
                        || character >= '\u{80}'
                    {
                        name.push(character);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    return None;
                }
                let entry =
                    bounds
                        .entry(name)
                        .or_insert((row_count, row_count + 1, column, column + 1, 0));
                entry.1 = row_count + 1;
                entry.2 = entry.2.min(column);
                entry.3 = entry.3.max(column + 1);
                entry.4 += 1;
            }
            column += 1;
        }
        if column == 0 || column_count.is_some_and(|expected| expected != column) {
            return None;
        }
        column_count = Some(column);
        row_count += 1;
    }
    let column_count = column_count?;
    let mut areas = Vec::with_capacity(bounds.len());
    for (name, (top, bottom, left, right, count)) in bounds {
        if (bottom - top).checked_mul(right - left)? != count {
            return None;
        }
        areas.push(GridTemplateArea {
            name,
            row_start: u16::try_from(top + 1).ok()?,
            row_end: u16::try_from(bottom + 1).ok()?,
            column_start: u16::try_from(left + 1).ok()?,
            column_end: u16::try_from(right + 1).ok()?,
        });
    }
    Some(GridTemplateAreas {
        areas,
        row_count: u16::try_from(row_count).ok()?,
        column_count: u16::try_from(column_count).ok()?,
    })
}

fn grid_line(start: &str, end: &str) -> Line<taffy::style::GridPlacement> {
    Line {
        start: start.parse().unwrap_or_default(),
        end: end.parse().unwrap_or_default(),
    }
}

fn hidden_geometry() -> NodeGeometry {
    NodeGeometry {
        availability: GeometryAvailability::Hidden,
        box_model: None,
        client_rects: Vec::new(),
        in_viewport: false,
        interactable: false,
        diagnostic: None,
    }
}

fn mark_unsupported_subtree(
    id: u64,
    diagnostic: &str,
    by_id: &BTreeMap<u64, &LayoutNode>,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
) {
    geometry.insert(
        id,
        NodeGeometry {
            availability: GeometryAvailability::Unsupported,
            box_model: None,
            client_rects: Vec::new(),
            in_viewport: false,
            interactable: false,
            diagnostic: Some(diagnostic.into()),
        },
    );
    if let Some(node) = by_id.get(&id) {
        for child in &node.children {
            mark_unsupported_subtree(*child, diagnostic, by_id, geometry);
        }
    }
}

fn inset(rect: CssRect, edges: EdgeSizes) -> CssRect {
    CssRect {
        x: rect.x + edges.left,
        y: rect.y + edges.top,
        width: (rect.width - edges.left - edges.right).max(0.0),
        height: (rect.height - edges.top - edges.bottom).max(0.0),
    }
}

fn style_value<'a>(style: &'a BTreeMap<String, String>, name: &str, default: &'a str) -> &'a str {
    style.get(name).map(String::as_str).unwrap_or(default)
}

fn is_inline(style: &BTreeMap<String, String>) -> bool {
    matches!(
        style_value(style, "display", "block"),
        "inline"
            | "inline-block"
            | "inline-flex"
            | "inline-grid"
            | "inline-table"
            | "ruby"
            | "ruby-base"
            | "ruby-text"
            | "ruby-base-container"
            | "ruby-text-container"
    )
}

fn edge(
    style: &BTreeMap<String, String>,
    prefix: &str,
    basis: f64,
    font_size: f64,
    viewport: ViewportState,
) -> EdgeSizes {
    let shorthand = style
        .get(prefix)
        .map(|value| value.split_whitespace().collect::<Vec<_>>())
        .unwrap_or_default();
    let values = match shorthand.as_slice() {
        [] => ["0", "0", "0", "0"],
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left, ..] => [*top, *right, *bottom, *left],
    };
    let side_value = |side: &str, fallback: &str| {
        style
            .get(&format!("{prefix}-{side}"))
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    };
    let top = side_value("top", values[0]);
    let right = side_value("right", values[1]);
    let bottom = side_value("bottom", values[2]);
    let left = side_value("left", values[3]);
    EdgeSizes {
        top: length(&top, basis, font_size, viewport),
        right: length(&right, basis, font_size, viewport),
        bottom: length(&bottom, basis, font_size, viewport),
        left: length(&left, basis, font_size, viewport),
    }
}

fn border_edge(
    style: &BTreeMap<String, String>,
    basis: f64,
    font_size: f64,
    viewport: ViewportState,
) -> EdgeSizes {
    let shorthand = style
        .get("border-width")
        .cloned()
        .or_else(|| {
            style
                .get("border")
                .map(|value| value.split_whitespace().next().unwrap_or("0").to_string())
        })
        .unwrap_or_else(|| "0".into());
    let side = |name: &str| {
        style
            .get(&format!("border-{name}-width"))
            .cloned()
            .unwrap_or_else(|| shorthand.clone())
    };
    let top = side("top");
    let right = side("right");
    let bottom = side("bottom");
    let left = side("left");
    EdgeSizes {
        top: length(&top, basis, font_size, viewport),
        right: length(&right, basis, font_size, viewport),
        bottom: length(&bottom, basis, font_size, viewport),
        left: length(&left, basis, font_size, viewport),
    }
}

fn is_intrinsic_width(value: &str) -> bool {
    matches!(value.trim(), "min-content" | "max-content" | "fit-content")
        || value.trim().starts_with("fit-content(")
}

fn resolve_intrinsic_width(
    value: &str,
    available: f64,
    widths: (f64, f64),
    font_size: f64,
    viewport: ViewportState,
) -> Option<f64> {
    let (min, max) = widths;
    match value.trim() {
        "min-content" => Some(min),
        "max-content" => Some(max),
        "fit-content" => Some(max.min(available.max(min))),
        value => value
            .strip_prefix("fit-content(")
            .and_then(|value| value.strip_suffix(')'))
            .map(|limit| max.min(length(limit, available, font_size, viewport).max(min))),
    }
}

/// Solve the CSS block inline-size equation after width/min/max resolution.
/// Auto margins absorb nonnegative free space; an overconstrained block adjusts
/// the end margin according to its containing block's direction.
fn block_inline_margins(
    containing_width: f64,
    border_width: f64,
    mut left: f64,
    mut right: f64,
    auto_left: bool,
    auto_right: bool,
    rtl: bool,
) -> (f64, f64) {
    if auto_left {
        left = 0.;
    }
    if auto_right {
        right = 0.;
    }
    let free = containing_width - border_width - left - right;
    if free >= 0. && (auto_left || auto_right) {
        match (auto_left, auto_right) {
            (true, true) => {
                left = free / 2.;
                right = free / 2.;
            }
            (true, false) => left = free,
            (false, true) => right = free,
            _ => unreachable!(),
        }
    } else if rtl {
        left += free;
    } else {
        right += free;
    }
    (left, right)
}

/// Relative insets move the painted box while its normal-flow allocation stays
/// unchanged. Overconstrained inline insets use the containing block direction.
fn relative_position_offset(
    style: &BTreeMap<String, String>,
    width: f64,
    height: Option<f64>,
    containing_rtl: bool,
    writing_mode: &str,
    font_size: f64,
    viewport: ViewportState,
) -> (f64, f64) {
    let resolve = |property, basis: Option<f64>| {
        let value = style_value(style, property, "auto");
        if value.contains('%') && basis.is_none() {
            return None;
        }
        oryn_common::css_math::resolve_length(
            value,
            oryn_common::css_math::LengthContext {
                percentage_basis: basis.unwrap_or(0.),
                font_size,
                root_font_size: 16.,
                viewport_width: viewport.width,
                viewport_height: viewport.height,
            },
        )
        .filter(|value| value.is_finite())
    };
    let left = resolve("left", Some(width));
    let right = resolve("right", Some(width));
    let top = resolve("top", height);
    let bottom = resolve("bottom", height);
    let axes = writing_mode::Axes::new(writing_mode, containing_rtl);
    let x = if axes.x_reverse {
        right.map(|value| -value).or(left)
    } else {
        left.or_else(|| right.map(|value| -value))
    }
    .unwrap_or(0.);
    let y = if axes.y_reverse {
        bottom.map(|value| -value).or(top)
    } else {
        top.or_else(|| bottom.map(|value| -value))
    }
    .unwrap_or(0.);
    (x, y)
}

/// Apply physical block-size bounds to a content-box measurement. Keep this
/// shared by intrinsic measurement and final formatting so nested containers
/// reserve the same height that their descendants ultimately occupy.
fn clamp_content_height(
    style: &BTreeMap<String, String>,
    mut height: f64,
    basis: Option<f64>,
    font_size: f64,
    viewport: ViewportState,
    vertical_edges: f64,
) -> f64 {
    let border_box = style_value(style, "box-sizing", "content-box") == "border-box";
    let bound = |property, default| {
        block_height_length(
            style_value(style, property, default),
            basis,
            font_size,
            viewport,
        )
        .map(|value| (value - if border_box { vertical_edges } else { 0. }).max(0.))
    };
    if let Some(maximum) = bound("max-height", "none") {
        height = height.min(maximum);
    }
    if let Some(minimum) = bound("min-height", "auto") {
        height = height.max(minimum);
    }
    height
}

fn block_height_length(
    value: &str,
    basis: Option<f64>,
    font_size: f64,
    viewport: ViewportState,
) -> Option<f64> {
    if value.contains('%') && basis.is_none() {
        return None;
    }
    oryn_common::css_math::resolve_length(
        value,
        oryn_common::css_math::LengthContext {
            percentage_basis: basis.unwrap_or(0.),
            font_size,
            root_font_size: 16.,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
        },
    )
    .filter(|value| value.is_finite())
    .map(|value| value.max(0.))
}

fn optional_length(
    value: &str,
    basis: f64,
    font_size: f64,
    viewport: ViewportState,
) -> Option<f64> {
    (!matches!(value.trim(), "auto" | "none" | "normal"))
        .then(|| length(value, basis, font_size, viewport))
}

fn length(value: &str, basis: f64, font_size: f64, viewport: ViewportState) -> f64 {
    // Preserve numeric used values supplied by layout callers. Typed CSS math
    // itself rejects nonzero unitless values as lengths.
    value
        .trim()
        .parse::<f64>()
        .ok()
        .or_else(|| {
            oryn_common::css_math::resolve_length(
                value,
                oryn_common::css_math::LengthContext {
                    percentage_basis: basis,
                    font_size,
                    root_font_size: 16.0,
                    viewport_width: viewport.width,
                    viewport_height: viewport.height,
                },
            )
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn resolved_publication_preflight_checks_all_arithmetic_and_preserves_bound() {
        assert_eq!(resolved_publication_bytes([]), Some(0));
        assert_eq!(resolved_publication_bytes([(3, 1, 1), (5, 0, 1)]), Some(200));
        assert_eq!(resolved_publication_bytes([(0, usize::MAX, 1)]), None);
        assert_eq!(resolved_publication_bytes([(0, usize::MAX / 64 + 1, 0)]), None);
        assert_eq!(resolved_publication_bytes([(usize::MAX, 1, 0)]), None);
        assert_eq!(resolved_publication_bytes([(usize::MAX, 0, 0), (1, 0, 0)]), None);
        let limit = 64 * 1024 * 1024;
        assert!(!resolved_publication_bytes([(limit, 0, 0)])
            .is_none_or(|bytes| bytes > limit));
        assert!(resolved_publication_bytes([(limit, 0, 0), (1, 0, 0)])
            .is_none_or(|bytes| bytes > limit));
    }

    #[test]
    fn native_paint_clusters_retain_line_local_utf8_ranges_in_both_axes() {
        for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            let mut engine = TextEngine::new(&[]);
            let style = BTreeMap::from([
                ("writing-mode".into(), mode.into()),
                ("white-space".into(), "pre-wrap".into()),
                ("overflow-wrap".into(), "anywhere".into()),
            ]);
            let mut shaped =
                engine.shape_styled("office é 😀 אב", 40., 16., &style, ViewportState::default());
            if mode != "horizontal-tb" {
                vertical_text::physicalize(&mut shaped, 100., 40., mode);
            }
            assert_eq!(shaped.lines.len(), shaped.line_ranges.len());
            for (index, line) in shaped.lines.iter().enumerate() {
                let clusters = shaped.paint_clusters(index, 10., 20.);
                assert!(!clusters.is_empty());
                for cluster in clusters {
                    assert!(line.0.get(cluster.start..cluster.end).is_some());
                    assert!(cluster.end <= line.0.len());
                    assert!(cluster.rect.x.is_finite() && cluster.rect.y.is_finite());
                }
            }
        }
    }
    #[test]
    fn subtree_layout_preserves_document_selectors_and_uses_local_viewport() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        root.attributes.insert("class".into(), "ancestor".into());
        let mut foreign = node(2, Some(1), "foreignObject", "");
        foreign.namespace = "http://www.w3.org/2000/svg".into();
        foreign.children = vec![3];
        foreign.attributes.insert(
            "style".into(),
            "transform:translate(90px,80px);opacity:0.5;width:300px;height:200px".into(),
        );
        let child = node(3, Some(2), "div", "Native layout");
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, foreign, child],
            stylesheets: vec![".ancestor div { width:50%;height:20px;background:green }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState {
                width: 500.,
                height: 400.,
                device_pixel_ratio: 1.,
            },
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let full = compute(&input, &LayoutLimits::default()).unwrap();
        let local = compute_subtree_with_styles(
            &input,
            2,
            ViewportState {
                width: 120.,
                height: 60.,
                device_pixel_ratio: 1.,
            },
            &full.computed_styles,
            &LayoutLimits::default(),
        )
        .unwrap();
        assert!(!local.geometry.contains_key(&1));
        let wrapper = local.geometry[&2].box_model.as_ref().unwrap();
        assert_eq!(
            wrapper.border_box,
            CssRect {
                x: 0.,
                y: 0.,
                width: 120.,
                height: 60.
            }
        );
        let child = local.geometry[&3].box_model.as_ref().unwrap();
        assert_eq!(child.border_box.width, 60.);
        assert_eq!(child.border_box.x, 0.);
        assert_eq!(
            local.computed_styles[&2].get("opacity").map(String::as_str),
            Some("1")
        );
        assert_eq!(local.paint_projections[&2].matrix[12], 0.);
        assert!(
            local.text_fragments[&3]
                .iter()
                .all(|fragment| !fragment.glyph_runs.is_empty())
        );
    }

    #[test]
    fn subtree_pseudo_content_keeps_owning_document_counter_scope() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut foreign = node(2, Some(1), "foreignObject", "");
        foreign.namespace = "http://www.w3.org/2000/svg".into();
        foreign.children = vec![3];
        let mut input = column_fixture(
            "section{counter-reset:item 8} section div::before{content:counter(item) ' ';color:red}",
            Vec::new(),
        );
        input.nodes = vec![root, foreign, node(3, Some(2), "div", "Value")];
        let full = compute(&input, &LayoutLimits::default()).unwrap();
        let local = compute_subtree_with_styles_and_pseudos(
            &input,
            2,
            ViewportState {
                width: 120.,
                height: 60.,
                device_pixel_ratio: 1.,
            },
            &full.computed_styles,
            &full.pseudo_styles,
            &LayoutLimits::default(),
        )
        .unwrap();
        assert!(
            local
                .generated_content
                .iter()
                .any(|content| content.owner == 3 && content.text == "8 ")
        );
        assert!(!local.geometry.contains_key(&1));
        assert!(
            local
                .generated_nodes
                .iter()
                .any(|node| node.parent_id == Some(3) && !node.text.is_empty())
        );
        assert!(
            local
                .generated_nodes
                .iter()
                .all(|node| !local.text_clusters.contains_key(&node.node_id))
        );
    }
    #[test]
    fn nested_scroll_projects_contents_without_moving_container_clips_or_flow_boxes() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        root.attributes.insert(
            "style".into(),
            "width:200px;height:200px;overflow:hidden;transform:scale(2);transform-origin:0 0"
                .into(),
        );
        let mut inner = node(2, Some(1), "div", "");
        inner.children = vec![3];
        inner.attributes.insert(
            "style".into(),
            "width:300px;height:300px;overflow:auto".into(),
        );
        let mut leaf = node(3, Some(2), "div", "AB");
        leaf.attributes
            .insert("style".into(), "width:500px;height:500px".into());
        leaf.content = Some(vec![LayoutContent::Text {
            node_id: 10,
            text: "AB".into(),
        }]);
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, inner, leaf],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let baseline = compute(&input, &LayoutLimits::default()).unwrap();
        input.element_scrolls = BTreeMap::from([(1, (10., 20.)), (2, (30., 40.))]);
        let scrolled = compute(&input, &LayoutLimits::default()).unwrap();
        let rect =
            |output: &LayoutOutput, id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert_eq!(rect(&baseline, 1), rect(&scrolled, 1));
        for (id, dx, dy) in [(2, 20., 40.), (3, 80., 120.)] {
            assert!((rect(&scrolled, id).x - rect(&baseline, id).x + dx).abs() < 0.001);
            assert!((rect(&scrolled, id).y - rect(&baseline, id).y + dy).abs() < 0.001);
            assert_eq!(
                scrolled.paint_projections[&id].box_model,
                baseline.paint_projections[&id].box_model
            );
            assert_eq!(
                scrolled.offset_dimensions[&id],
                baseline.offset_dimensions[&id]
            );
        }
        assert_eq!(
            scrolled.paint_projections[&2].clips,
            baseline.paint_projections[&2].clips
        );
        let before = &baseline.text_clusters[&10][0];
        let after = &scrolled.text_clusters[&10][0];
        assert_eq!(before.rect, after.rect);
        assert!((after.matrix[12] - before.matrix[12] + 80.).abs() < 0.001);
        assert!((after.matrix[13] - before.matrix[13] + 120.).abs() < 0.001);
        // The scroller's own text also has an independent content transform.
        input.nodes[1].children.clear();
        input.nodes.truncate(2);
        // Native offsets clamp to real overflow. Long preformatted own text
        // supplies both axes after the original overflowing child is removed.
        let own_text = format!("{}\n{}", "W".repeat(80), "W".repeat(80));
        input.nodes[1]
            .attributes
            .get_mut("style")
            .unwrap()
            .push_str(";white-space:pre;line-height:200px");
        input.nodes[1].text = own_text.clone();
        input.nodes[1].content = Some(vec![LayoutContent::Text {
            node_id: 11,
            text: own_text,
        }]);
        let own = compute(&input, &LayoutLimits::default()).unwrap();
        let p = &own.paint_projections[&2];
        assert!((p.text_matrix.unwrap()[12] - p.matrix[12] + 60.).abs() < 0.001);
        assert!((p.text_matrix.unwrap()[13] - p.matrix[13] + 80.).abs() < 0.001);
        input.nodes[1].attributes.insert(
            "style".into(),
            "width:300px;height:300px;overflow:auto".into(),
        );
        input.nodes[1].text = "AB".into();
        input.nodes[1].content = Some(vec![LayoutContent::Text {
            node_id: 11,
            text: "AB".into(),
        }]);
        let short = compute(&input, &LayoutLimits::default()).unwrap();
        let p = &short.paint_projections[&2];
        assert_eq!(
            p.scroll
                .as_ref()
                .map(|scroll| (scroll.offset_x, scroll.offset_y)),
            Some((0., 0.))
        );
        assert_eq!(p.text_matrix, Some(p.matrix));
        assert_eq!(p.text_clips.len(), p.clips.len() + 1);
        assert_eq!(p.text_clips.last().unwrap().matrix, p.matrix);
        assert_eq!(
            own.text_clusters[&11][0].matrix,
            own.paint_projections[&2].text_matrix.unwrap()
        );
        // Both axes clipped (as opposed to hidden) suppress programmatic scrolling.
        input.nodes[1].attributes.insert(
            "style".into(),
            "width:300px;height:300px;overflow:clip".into(),
        );
        let clipped = compute(&input, &LayoutLimits::default()).unwrap();
        let p = &clipped.paint_projections[&2];
        assert_eq!(p.text_matrix, Some(p.matrix));
    }

    #[test]
    fn nested_scroll_inputs_preserve_identity_and_reject_invalid_offsets() {
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "div", "")],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let original = input.clone();
        input.element_scrolls.insert(1, (-10., 20.));
        assert_ne!(input, original);
        assert!(validate(&input, &LayoutLimits::default()).is_ok());
        let encoded = serde_json::to_vec(&input).unwrap();
        assert_eq!(
            serde_json::from_slice::<LayoutInput>(&encoded).unwrap(),
            input
        );
        input.element_scrolls.insert(2, (0., 0.));
        assert_eq!(
            validate(&input, &LayoutLimits::default()),
            Err(LayoutError::InvalidElementScroll(2))
        );
        input.element_scrolls.remove(&2);
        input.element_scrolls.insert(1, (f64::NAN, 0.));
        assert_eq!(
            validate(&input, &LayoutLimits::default()),
            Err(LayoutError::InvalidElementScroll(1))
        );
        let mut legacy = serde_json::to_value(original).unwrap();
        legacy.as_object_mut().unwrap().remove("element_scrolls");
        assert!(
            serde_json::from_value::<LayoutInput>(legacy)
                .unwrap()
                .element_scrolls
                .is_empty()
        );
    }

    #[test]
    fn layout_transports_native_text_offsets_and_original_transform_plane() {
        let mut root = node(1, None, "div", "AB");
        root.content = Some(vec![LayoutContent::Text {
            node_id: 10,
            text: "AB".into(),
        }]);
        root.attributes.insert(
            "style".into(),
            "width:200px;font-size:16px;transform:translateX(10px)".into(),
        );
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let clusters = &output.text_clusters[&10];
        assert_eq!(clusters.len(), 2);
        assert_eq!((clusters[0].start, clusters[0].end), (0, 1));
        assert_eq!((clusters[1].start, clusters[1].end), (1, 2));
        assert!(clusters[1].rect.x > clusters[0].rect.x);
        assert!((clusters[0].matrix[12] - 10.).abs() < 0.001);
        let decoded: LayoutOutput =
            serde_json::from_slice(&serde_json::to_vec(&output).unwrap()).unwrap();
        assert_eq!(output.text_clusters, decoded.text_clusters);
        input.nodes[0]
            .attributes
            .get_mut("style")
            .unwrap()
            .push_str(";display:none");
        assert!(
            compute(&input, &LayoutLimits::default())
                .unwrap()
                .text_clusters
                .is_empty()
        );
    }

    #[test]
    fn shaping_language_inherits_xml_precedence_and_empty_unknown_language() {
        let mut root = node(1, None, "html", "");
        root.attributes.insert("lang".into(), "en".into());
        let mut middle = node(2, Some(1), "span", "");
        middle.attributes.insert("lang".into(), "de".into());
        middle.namespaced_attributes.push(LayoutAttribute {
            namespace: "http://www.w3.org/XML/1998/namespace".into(),
            local_name: "lang".into(),
            prefix: Some("xml".into()),
            value: "tr".into(),
        });
        let inherited = node(3, Some(2), "span", "");
        let mut unknown = node(4, Some(2), "span", "");
        unknown.attributes.insert("lang".into(), String::new());
        let leaf = node(5, Some(4), "span", "");
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![leaf, inherited, unknown, middle, root],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let languages = effective_languages(&input);
        assert_eq!(languages[&1].as_deref(), Some("en"));
        assert_eq!(languages[&2].as_deref(), Some("tr"));
        assert_eq!(languages[&3].as_deref(), Some("tr"));
        assert_eq!(languages[&4].as_deref(), Some(""));
        assert_eq!(languages[&5].as_deref(), Some(""));
    }
    #[test]
    fn document_language_fallback_reaches_paint_and_preserves_explicit_empty() {
        let root = node(1, None, "html", "");
        let mut middle = node(2, Some(1), "span", "");
        middle.attributes.insert("lang".into(), "de".into());
        middle.namespaced_attributes.push(LayoutAttribute {
            namespace: "http://www.w3.org/XML/1998/namespace".into(),
            local_name: "lang".into(),
            prefix: Some("xml".into()),
            value: "tr".into(),
        });
        let inherited = node(3, Some(2), "span", "");
        let mut unknown = node(4, Some(2), "span", "");
        unknown.attributes.insert("lang".into(), String::new());
        let leaf = node(5, Some(4), "span", "");
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: Some("ko".into()),
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![leaf, inherited, unknown, middle, root],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let languages = effective_languages(&input);
        assert_eq!(languages[&1].as_deref(), Some("ko"));
        let restored: LayoutInput =
            serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert_eq!(effective_languages(&restored), languages);
        assert_eq!(languages[&2].as_deref(), Some("tr"));
        assert_eq!(languages[&3].as_deref(), Some("tr"));
        assert_eq!(languages[&4].as_deref(), Some(""));
        assert_eq!(languages[&5].as_deref(), Some(""));
    }

    #[test]
    fn font_synthesis_permissions_and_ligature_features_reach_glyph_runs() {
        let mut engine = TextEngine::new(&[]);
        let mut style = BTreeMap::from([
            ("font-weight".into(), "700".into()),
            ("font-style".into(), "italic".into()),
        ]);
        let enabled = engine.shape_styled("A", 200., 16., &style, ViewportState::default());
        style.insert("font-synthesis-weight".into(), "none".into());
        style.insert("font-synthesis-style".into(), "none".into());
        let disabled = engine.shape_styled("A", 200., 16., &style, ViewportState::default());
        assert!(enabled.glyph_runs[0][0].embolden);
        assert!(!disabled.glyph_runs[0][0].embolden);
        assert_eq!(disabled.glyph_runs[0][0].skew_degrees, 0.);
        assert_eq!(
            enabled.glyph_runs[0][0].font_key,
            disabled.glyph_runs[0][0].font_key
        );
        let normal = engine.shape("ffi", 200., 16., DEFAULT_FONT_FAMILY);
        let separated = engine.shape_styled(
            "ffi",
            200.,
            16.,
            &BTreeMap::from([(
                "font-feature-settings".into(),
                "\"liga\" 0, \"clig\" 0".into(),
            )]),
            ViewportState::default(),
        );
        let count = |layout: &TextLayout| {
            layout
                .glyph_runs
                .iter()
                .flatten()
                .map(|run| run.glyphs.len())
                .sum::<usize>()
        };
        assert!(count(&separated) > count(&normal));
    }

    #[test]
    fn explicit_variation_coordinates_override_weight_matching() {
        use skrifa::MetadataProvider;
        let bytes = include_bytes!("../../oryn-render/tests/assets/variabletest_matching.ttf");
        let mut engine = TextEngine::new(&[(
            "Variable Fixture".into(),
            bytes.to_vec(),
            String::new(),
            false,
            false,
        )]);
        let style = BTreeMap::from([
            ("font-family".into(), "Variable Fixture".into()),
            ("font-weight".into(), "700".into()),
            ("font-variation-settings".into(), "\"wght\" 100".into()),
        ]);
        let shaped = engine.shape_styled("P", 500., 64., &style, ViewportState::default());
        let font = FontRef::from_index(bytes, 0).unwrap();
        let expected = font
            .axes()
            .location(&[("wght", 100.)])
            .coords()
            .iter()
            .map(|coord| coord.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(shaped.glyph_runs[0][0].normalized_coords, expected);
    }

    #[test]
    fn font_matching_exports_synthesis_and_variation_state() {
        let mut engine = TextEngine::new(&[]);
        let normal = engine.shape("A", 200., 16., DEFAULT_FONT_FAMILY);
        let styled = engine.shape_styled(
            "A",
            200.,
            16.,
            &BTreeMap::from([
                ("font-weight".into(), "700".into()),
                ("font-style".into(), "italic".into()),
            ]),
            ViewportState::default(),
        );
        assert!(!normal.glyph_runs[0][0].embolden);
        assert_eq!(normal.glyph_runs[0][0].skew_degrees, 0.);
        assert!(styled.glyph_runs[0][0].embolden);
        assert!(styled.glyph_runs[0][0].skew_degrees > 0.);
        let decoded: Vec<Vec<oryn_common::v4::PaintGlyphRun>> =
            serde_json::from_slice(&serde_json::to_vec(&styled.glyph_runs).unwrap()).unwrap();
        assert_eq!(styled.glyph_runs, decoded);
    }

    #[test]
    fn ruby_annotation_pairs_with_first_base_and_preserves_owner_geometry() {
        // Pinned css/css-ruby/ruby-annotation-pairing-001.html.
        let mut root = node(1, None, "ruby", "");
        root.children = vec![2, 3, 4];
        root.content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 30,
                text: " ".into(),
            },
            LayoutContent::Element(3),
            LayoutContent::Element(4),
        ]);
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![root,node(2,Some(1),"rb","W"),node(3,Some(1),"rb","Z"),node(4,Some(1),"rt","A")],stylesheets:vec!["ruby {font-size:60px;line-height:1;ruby-align:center} rt {font-size:30px;line-height:1}".into()],element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let base = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let annotation = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        let second = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!((annotation.x + annotation.width / 2. - base.x - base.width / 2.).abs() < 0.01);
        assert!(annotation.bottom() <= base.y + 0.01);
        assert!(second.x > base.right());
        assert_eq!(output.text_fragments[&2][0].text, "W");
        assert_eq!(output.text_fragments[&3][0].text, "Z");
        assert_eq!(output.text_fragments[&4][0].text, "A");
        let mut block_input = input.clone();
        block_input
            .stylesheets
            .push("ruby {display:block ruby}".into());
        let block_output = compute(&block_input, &LayoutLimits::default()).unwrap();
        assert_eq!(block_output.computed_styles[&1]["display"], "block ruby");
        assert!(!is_inline(&block_output.computed_styles[&1]));
        let block_base = block_output.geometry[&2]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        let block_annotation = block_output.geometry[&4]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!(
            (block_annotation.x + block_annotation.width / 2.
                - block_base.x
                - block_base.width / 2.)
                .abs()
                < 0.01
        );
        assert!(block_annotation.bottom() <= block_base.y + 0.01);
        assert_eq!(block_output.text_fragments[&2][0].text, "W");
        assert_eq!(block_output.text_fragments[&4][0].text, "A");
    }

    #[test]
    fn stylesheet_resource_urls_use_each_native_sheet_base() {
        let mut input = column_fixture("div { background-image:url('../image.png') }", Vec::new());
        input
            .stylesheet_base_urls
            .insert(0, "https://cdn.test/themes/one/style.css".into());
        let first = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            first.computed_styles[&1]["background-image"]
                .contains("https://cdn.test/themes/image.png")
        );
        input
            .stylesheet_base_urls
            .insert(0, "https://other.test/two/style.css".into());
        let second = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            second.computed_styles[&1]["background-image"].contains("https://other.test/image.png")
        );
        let copy: LayoutInput =
            serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
        assert_eq!(copy.stylesheet_base_urls, input.stylesheet_base_urls);
    }

    pub(super) fn column_fixture(css: &str, children: Vec<LayoutNode>) -> LayoutInput {
        let mut root = node(1, None, "div", "");
        root.children = children.iter().map(|node| node.id).collect();
        root.content = Some(
            root.children
                .iter()
                .map(|id| LayoutContent::Element(*id))
                .collect(),
        );
        let mut nodes = vec![root];
        nodes.extend(children);
        LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            nodes,
            stylesheets: vec![css.into()],
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        }
    }

    #[test]
    fn zero_and_subpixel_frame_viewports_preserve_css_viewport_units() {
        for dpr in [1., 1.25, 2.] {
            for (width, height) in [(0., 0.), (0., 120.), (240., 0.), (0.25, 0.5)] {
                let mut input = column_fixture(
                    "div{margin:0;padding:0;border:0}#sized{width:100vw;height:100vh}",
                    vec![node(2, Some(1), "div", "")],
                );
                input.nodes[1]
                    .attributes
                    .insert("id".into(), "sized".into());
                input.viewport.width = width;
                input.viewport.height = height;
                input.viewport.device_pixel_ratio = dpr;
                let output = compute(&input, &LayoutLimits::default()).unwrap();
                let content = output.geometry[&2].box_model.as_ref().unwrap().content_box;
                assert!(
                    (content.width - width).abs() < 0.02,
                    "width {width}: {content:?}"
                );
                assert!(
                    (content.height - height).abs() < 0.02,
                    "height {height}: {content:?}"
                );
            }
        }
    }

    #[test]
    fn exact_css_viewport_survives_native_media_device_rebuild() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        document.set_css_viewport_size(0.25, 0.5);
        document.set_media_type(style::media_queries::MediaType::print());
        let size = document.stylist_device().viewport_size();
        assert_eq!((size.width, size.height), (0.25, 0.5));
        document.set_css_viewport_size(0., 0.);
        document.set_media_type(style::media_queries::MediaType::screen());
        let size = document.stylist_device().viewport_size();
        assert_eq!((size.width, size.height), (0., 0.));
    }

    #[test]
    fn invalid_frame_viewports_still_reject_before_layout() {
        let limits = LayoutLimits::default();
        for value in [
            -0.01,
            f64::NAN,
            f64::INFINITY,
            limits.max_viewport_css_px + 1.,
        ] {
            for width in [true, false] {
                let mut input = column_fixture("", Vec::new());
                if width {
                    input.viewport.width = value;
                } else {
                    input.viewport.height = value;
                }
                assert!(matches!(
                    validate(&input, &limits),
                    Err(LayoutError::ViewportLimit { .. })
                ));
            }
        }
    }

    #[test]
    fn generated_content_shapes_counters_and_attributes_without_dom_identity() {
        let mut first = node(2, Some(1), "p", "Body");
        first.attributes.insert("data-label".into(), "Alpha".into());
        let mut second = node(3, Some(1), "p", "Body");
        second.attributes.insert("data-label".into(), "Beta".into());
        let input = column_fixture(
            r#"div{counter-reset:item}p{margin:0}p::before{counter-increment:item;content:counter(item) ". " attr(data-label) " ";color:red}"#,
            vec![first, second],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 2 && item.pseudo == "before" && item.text == "1. Alpha ")
        );
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 3 && item.pseudo == "before" && item.text == "2. Beta ")
        );
        for generated in &output.generated_nodes {
            assert!(!input.nodes.iter().any(|node| node.id == generated.node_id));
            assert!(!output.text_clusters.contains_key(&generated.node_id));
            assert!(generated.geometry.box_model.is_some());
        }
        assert_eq!(input.nodes[1].text, "Body");
    }

    #[test]
    fn counter_style_scope_handles_deep_ancestry_and_rejects_cycles() {
        let mut input = column_fixture(
            "@counter-style inherited{system:cyclic;symbols:'root'}",
            vec![node(2, Some(1), "p", "Body")],
        );
        for root in 1..=999_u64 {
            let host = 10_000 + root;
            input.shadow_trees.roots.insert(
                root,
                ShadowTree {
                    host,
                    content: vec![],
                },
            );
            if root > 1 {
                input.shadow_trees.node_scopes.insert(host, root - 1);
            }
        }
        let registry =
            generated_counter_styles::Registry::read_for_scope(&input, false, Some(999)).unwrap();
        assert_eq!(registry.format(1, "inherited"), "root");
        assert_eq!(registry.marker(1, "inherited"), "root. ");
        let mut cache = BTreeMap::new();
        let cached = generated_counter_styles::Registry::read_for_scope_cached(
            &input, false, Some(999), &mut cache,
        ).unwrap();
        assert_eq!(cache.len(), 1000);
        for root in 1..=999_u64 {
            let ancestor = generated_counter_styles::Registry::read_for_scope_cached(
                &input, false, Some(root), &mut cache,
            ).unwrap();
            assert!(Arc::ptr_eq(&cached, &ancestor));
        }
        input.shadow_trees.node_scopes.insert(10_001, 999);
        let error = generated_counter_styles::Registry::read_for_scope(&input, false, Some(999))
            .err()
            .expect("cyclic scope ancestry must fail");
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn hidden_layout_handles_deep_descendants_without_recursive_frames() {
        let mut input = column_fixture("#n2 { display: none }", vec![node(2, Some(1), "div", "")]);
        input.nodes.iter_mut().find(|node| node.id == 2).unwrap()
            .attributes.insert("id".into(), "n2".into());
        for id in 3..=1001 {
            let parent = input.nodes.iter_mut().find(|node| node.id == id - 1).unwrap();
            parent.children.push(id);
            parent.content = Some(vec![LayoutContent::Element(id)]);
            input.nodes.push(node(id, Some(id - 1), "div", ""));
        }
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for id in 2..=1001 {
            assert_eq!(output.geometry[&id].availability, GeometryAvailability::Hidden);
        }
    }

    #[test]
    fn author_counter_systems_flow_through_generated_layout_and_media() {
        let css = r#"@counter-style binary{system:numeric;symbols:'0' '1'}
            @counter-style padded{system:extends decimal;pad:4 '0';negative:'(' ')'}
            @counter-style cycle{system:cyclic;symbols:'A' 'B' 'C'}
            @counter-style fixed{system:fixed -1;symbols:'low' 'zero' 'high';fallback:binary}
            @counter-style alpha{system:alphabetic;symbols:'a' 'b'}
            @counter-style tally{system:symbolic;symbols:'x' 'y'}
            @counter-style additive{system:additive;additive-symbols:5 'V', 1 'I', 0 'Z'}
            @counter-style media{system:extends decimal}
            @media print{@counter-style media{system:extends lower-alpha}}
            div{counter-reset:n 5 neg -2 zero 0}
            p::before{content:counter(n,binary) '|' counter(neg,padded) '|' counter(zero,cycle) '|' counter(n,fixed) '|' counter(n,alpha) '|' counter(n,tally) '|' counter(n,additive) '|' counter(n,media)}"#;
        let input = column_fixture(css, vec![node(2, Some(1), "p", "Body")]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 2 && item.text == "101|(02)|C|101|ba|xxx|V|5")
        );
        let registry = generated_counter_styles::Registry::read(&input, true).unwrap();
        assert_eq!(registry.format(5, "media"), "e");
        assert_eq!(registry.format(0, "additive"), "Z");
        assert_eq!(registry.format(-1, "fixed"), "low");
        let mut changed = input.clone();
        changed.stylesheets[0].push_str("@counter-style binary{system:numeric;symbols:'a' 'b'}");
        let output = compute(&changed, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.text.starts_with("bab|"))
        );
    }

    #[test]
    fn named_counter_rules_respect_layer_order_and_global_scope_definitions() {
        let input = column_fixture(
            r#"@layer low,high;
            @layer high{@counter-style layered{system:cyclic;symbols:'H'}}
            @layer low{@counter-style layered{system:cyclic;symbols:'L'}}
            @layer low.inner{@counter-style nested{system:cyclic;symbols:'inner'}}
            @layer low{@counter-style nested{system:cyclic;symbols:'outer'}}
            @scope (.does-not-exist){@counter-style scoped{system:cyclic;symbols:'global'}}
            p{counter-reset:n 1}p::before{content:counter(n,layered) '/' counter(n,nested) '/' counter(n,scoped)}"#,
            vec![node(2, Some(1), "p", "")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.text == "H/outer/global")
        );
        let mut changed = input.clone();
        changed.stylesheets.push("@counter-style layered{system:cyclic;symbols:'unlayered'}@layer high{@counter-style layered{system:cyclic;symbols:'late-layer'}}".into());
        let registry = generated_counter_styles::Registry::read(&changed, false).unwrap();
        assert_eq!(registry.format(1, "layered"), "unlayered");
    }

    #[test]
    fn counter_identity_inherits_values_from_previous_tree_element() {
        let mut first = node(2, Some(1), "section", "");
        first.children = vec![4];
        first.content = Some(vec![LayoutContent::Element(4)]);
        let mut input = column_fixture(
            "div{counter-reset:n}section:first-child{counter-increment:n}em{counter-increment:n}section::after{content:counter(n)}",
            vec![first, node(3, Some(1), "section", "")],
        );
        input.nodes.push(node(4, Some(2), "em", "Nested"));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 3 && item.pseudo == "after" && item.text == "2")
        );
        input.stylesheets[0].push_str("em{counter-reset:n 9}");
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 3 && item.pseudo == "after" && item.text == "1")
        );
    }

    #[test]
    fn pinned_display_contents_counter_order_and_reversed_pseudos() {
        let mut input = column_fixture(
            "section{display:contents;counter-reset:n 999}p{counter-increment:n}p::before{content:counter(n)}",
            vec![],
        );
        input.nodes[0].children = vec![2, 3, 4];
        input.nodes[0].content = None;
        for (wrapper, child) in [(2, 5), (3, 6), (4, 7)] {
            let mut section = node(wrapper, Some(1), "section", "");
            section.children = vec![child];
            input.nodes.push(section);
            input.nodes.push(node(child, Some(wrapper), "p", ""));
        }
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(5, "1"), (6, "2"), (7, "3")] {
            assert!(
                output
                    .generated_content
                    .iter()
                    .any(|item| item.owner == owner && item.text == expected)
            );
        }
        // WPT counter-reset-reversed-pseudo-003 and its B6A3 reference.
        let input = column_fixture(
            "p{counter-reset:reversed(foo);counter-increment:foo -1}p::before{counter-increment:foo -2;content:'B' counter(foo)}p::after{counter-increment:foo -3;content:'A' counter(foo)}",
            vec![node(2, Some(1), "p", "")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.pseudo == "before" && item.text == "B6")
        );
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.pseudo == "after" && item.text == "A3")
        );
    }

    #[test]
    fn counter_set_follows_increment_and_nonexistent_pseudos_do_not_increment() {
        let input = column_fixture(
            "div{counter-reset:n}p{counter-increment:n 2;counter-set:n 7}p::before{content:none;counter-increment:n 99}p::after{content:counter(n)}",
            vec![node(2, Some(1), "p", "")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.owner == 2 && item.pseudo == "after" && item.text == "7")
        );
    }

    #[test]
    fn reversed_counter_initial_value_includes_last_step_and_first_set() {
        let input = column_fixture(
            "div{counter-reset:reversed(n)}p{counter-increment:n -2}p::after{content:counter(n)}",
            vec![
                node(2, Some(1), "p", ""),
                node(3, Some(1), "p", ""),
                node(4, Some(1), "p", ""),
            ],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(2, "6"), (3, "4"), (4, "2")] {
            assert!(
                output
                    .generated_content
                    .iter()
                    .any(|item| item.owner == owner
                        && item.pseudo == "after"
                        && item.text == expected)
            );
        }
        let mut changed = input.clone();
        changed.stylesheets[0].push_str("p:nth-child(2){counter-set:n 10}");
        let output = compute(&changed, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(2, "12"), (3, "10"), (4, "8")] {
            assert!(
                output
                    .generated_content
                    .iter()
                    .any(|item| item.owner == owner
                        && item.pseudo == "after"
                        && item.text == expected)
            );
        }
    }

    #[test]
    fn html_list_counter_hints_yield_to_author_counter_rules() {
        let mut input = column_fixture(
            "li::marker{content:counter(list-item) '. '}",
            vec![node(2, Some(1), "li", ""), node(3, Some(1), "li", "")],
        );
        input.nodes[0].tag = "ol".into();
        input.nodes[0].attributes.insert("start".into(), "7".into());
        input.nodes[2]
            .attributes
            .insert("value".into(), "20".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(2, "7. "), (3, "20. ")] {
            let marker = output
                .generated_nodes
                .iter()
                .find(|node| node.parent_id == Some(owner) && node.tag == "::marker")
                .unwrap();
            assert!(marker.text.iter().any(|text| text.text == expected));
        }
        input.stylesheets[0].push_str(
            "ol{counter-reset:list-item 0}li{counter-increment:list-item 2;counter-set:none}",
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(2, "2. "), (3, "4. ")] {
            let marker = output
                .generated_nodes
                .iter()
                .find(|node| node.parent_id == Some(owner) && node.tag == "::marker")
                .unwrap();
            assert!(marker.text.iter().any(|text| text.text == expected));
        }
    }

    #[test]
    fn pinned_ethiopic_counter_values_reach_real_generated_text() {
        // WPT719d5e38: css/css-counter-styles/ethiopic-numeric/
        // counter-ethiopic-numeric.html and its literal reference.
        for (value, expected) in [
            (43, "፵፫"),
            (222, "፪፻፳፪"),
            (1005, "፲፻፭"),
            (7864, "፸፰፻፷፬"),
            (10000, "፼"),
            (1000001, "፻፼፩"),
            (78010092, "፸፰፻፩፼፺፪"),
            (0, "0"),
            (-1, "-1"),
        ] {
            let css = format!(
                "p{{counter-reset:n {value}}}p::after{{content:counter(n,ethiopic-numeric)}}"
            );
            let output = compute(
                &column_fixture(&css, vec![node(2, Some(1), "p", "")]),
                &LayoutLimits::default(),
            )
            .unwrap();
            assert!(
                output
                    .generated_content
                    .iter()
                    .any(|item| item.owner == 2 && item.pseudo == "after" && item.text == expected)
            );
        }
    }

    #[test]
    fn marker_speech_keeps_alternative_text_and_live_replacement() {
        let mut input = column_fixture(
            "li{display:list-item}li::marker{content:'Icon ' / 'Bullet'}",
            vec![node(2, Some(1), "li", "Body")],
        );
        for (content, text, alternative) in [
            ("'Icon ' / 'Bullet'", "Icon ", Some("Bullet")),
            ("'Icon ' / ''", "Icon ", Some("")),
            ("'Two '", "Two ", None),
        ] {
            input.stylesheets[0] =
                format!("li{{display:list-item}}li::marker{{content:{content}}}");
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let entry = output
                .generated_content
                .iter()
                .find(|entry| entry.owner == 2 && entry.pseudo == "marker")
                .unwrap();
            assert_eq!(entry.text, text);
            assert_eq!(entry.alternative.as_deref(), alternative);
            assert!(
                output
                    .generated_nodes
                    .iter()
                    .any(|entry| entry.parent_id == Some(2) && entry.tag == "::marker")
            );
        }
    }

    #[test]
    fn hidden_presentation_hint_yields_to_cascade_without_mutating_attributes() {
        let mut hidden = node(2, Some(1), "button", "Override");
        hidden.attributes.insert("hidden".into(), String::new());
        hidden
            .attributes
            .insert("style".into(), "display:none".into());
        let mut input = column_fixture("button{display:block!important}", vec![hidden]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["display"], "block");
        assert_eq!(
            output.geometry[&2].availability,
            GeometryAvailability::Available
        );
        assert!(input.nodes[1].attributes.contains_key("hidden"));
        assert_eq!(input.nodes[1].attributes["style"], "display:none");
        input.stylesheets.clear();
        assert_eq!(
            compute(&input, &LayoutLimits::default())
                .unwrap()
                .computed_styles[&2]["display"],
            "none"
        );
        input.nodes[1].attributes.remove("style");
        assert_eq!(
            compute(&input, &LayoutLimits::default())
                .unwrap()
                .computed_styles[&2]["display"],
            "none"
        );
        input.stylesheets.push("button{display:block}".into());
        assert_eq!(
            compute(&input, &LayoutLimits::default())
                .unwrap()
                .computed_styles[&2]["display"],
            "block"
        );
    }

    #[test]
    fn list_markers_share_counter_styles_and_remain_outside_normal_flow() {
        let mut input = column_fixture(
            r#"div{display:block;padding-left:40px}li{display:list-item;list-style-type:custom;list-style-position:outside;margin:0} @counter-style custom{system:extends decimal;prefix:'[';suffix:'] '}"#,
            vec![
                node(2, Some(1), "li", "First"),
                node(3, Some(1), "li", "Second"),
            ],
        );
        input.nodes[0].tag = "ol".into();
        input.nodes[0].attributes.insert("start".into(), "7".into());
        input.nodes[2]
            .attributes
            .insert("value".into(), "11".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (owner, expected) in [(2, "[7] "), (3, "[11] ")] {
            let marker = output
                .generated_nodes
                .iter()
                .find(|item| item.parent_id == Some(owner) && item.tag == "::marker")
                .unwrap();
            assert_eq!(
                marker
                    .text
                    .iter()
                    .map(|text| text.text.as_str())
                    .collect::<String>(),
                expected
            );
            let parent = output.geometry[&owner].box_model.as_ref().unwrap();
            let rect = marker.geometry.box_model.as_ref().unwrap().border_box;
            assert!(rect.right() <= parent.content_box.x + 0.01);
            assert!(!output.text_clusters.contains_key(&marker.node_id));
        }
        input.nodes[0].attributes.remove("start");
        input.nodes[0]
            .attributes
            .insert("reversed".into(), String::new());
        input.nodes[2].attributes.remove("value");
        input.stylesheets[0].push_str("li{list-style-position:inside}");
        let inside = compute(&input, &LayoutLimits::default()).unwrap();
        let markers = inside
            .generated_nodes
            .iter()
            .filter(|item| item.tag == "::marker")
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 2);
        assert!(
            markers
                .iter()
                .any(|item| item.text.iter().any(|text| text.text == "[2] "))
        );
        assert!(
            markers
                .iter()
                .any(|item| item.text.iter().any(|text| text.text == "[1] "))
        );
    }

    #[test]
    fn list_image_requests_preserve_fallback_until_native_intrinsics_arrive() {
        let mut input = column_fixture(
            "li{display:list-item;list-style-type:decimal;list-style-image:url(marker.svg)}",
            vec![node(2, Some(1), "li", "Body")],
        );
        let pending = compute(&input, &LayoutLimits::default()).unwrap();
        let url = "https://oryn.invalid/marker.svg";
        assert!(pending.css_image_requests.contains(url));
        assert!(pending.generated_image_urls.is_empty());
        assert!(
            pending
                .generated_nodes
                .iter()
                .any(|node| node.tag == "::marker"
                    && node.text.iter().any(|text| text.text == "1. "))
        );
        input.image_intrinsics.insert(url.into(), (17, 13));
        let ready = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(ready.css_image_requests.contains(url));
        let marker = ready
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::marker")
            .unwrap();
        assert_eq!(
            ready
                .generated_image_urls
                .get(&marker.node_id)
                .map(String::as_str),
            Some(url)
        );
        let rect = marker.geometry.box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.width, rect.height), (17., 13.));
        assert!(
            rect.right() <= ready.geometry[&2].box_model.as_ref().unwrap().content_box.x + 0.01
        );
    }

    #[test]
    fn gradient_list_marker_has_default_image_size_and_real_background_paint() {
        let input = column_fixture(
            "li{display:list-item;font-size:20px;list-style-image:linear-gradient(red,blue)}",
            vec![node(2, Some(1), "li", "Body")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let marker = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::marker")
            .unwrap();
        let rect = marker.geometry.box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.width, rect.height), (20., 20.));
        assert!(marker.style["background-image"].starts_with("linear-gradient("));
        assert!(marker.text.is_empty());
        assert!(output.css_image_requests.is_empty());
    }

    #[test]
    fn outside_marker_uses_native_first_line_baseline_with_different_font_size() {
        let input = column_fixture(
            "li{display:list-item;font-size:32px;line-height:48px}li::marker{font-size:12px;content:'X '}",
            vec![node(2, Some(1), "li", "Body")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let marker = output
            .generated_nodes
            .iter()
            .find(|item| item.tag == "::marker")
            .unwrap();
        let body = &output.text_fragments[&2][0];
        let mark = &marker.text[0];
        assert!(
            (body.rect.y + body.glyph_runs[0].glyphs[0].y
                - mark.rect.y
                - mark.glyph_runs[0].glyphs[0].y)
                .abs()
                < 0.05
        );
    }

    #[test]
    fn vertical_markers_follow_native_line_center_and_inline_start() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-lr"] {
            for direction in ["ltr", "rtl"] {
                let css = format!(
                    "li{{display:list-item;writing-mode:{mode};direction:{direction};width:80px;height:120px;font-size:24px}}li::marker{{font-size:12px;content:'X '}}"
                );
                let output = compute(
                    &column_fixture(&css, vec![node(2, Some(1), "li", "Body")]),
                    &LayoutLimits::default(),
                )
                .unwrap();
                let marker = output
                    .generated_nodes
                    .iter()
                    .find(|item| item.tag == "::marker")
                    .unwrap();
                let rect = marker.geometry.box_model.as_ref().unwrap().border_box;
                let owner = output.geometry[&2].box_model.as_ref().unwrap().content_box;
                let axes = writing_mode::Axes::new(mode, direction == "rtl");
                if axes.y_reverse {
                    assert!(rect.y >= owner.bottom() - 0.01);
                } else {
                    assert!(rect.bottom() <= owner.y + 0.01);
                }
                let body = &output.text_fragments[&2][0];
                let mark = &marker.text[0];
                assert!(
                    (body.rect.x + body.rect.width / 2. - mark.rect.x - mark.rect.width / 2.).abs()
                        < 0.05
                );
            }
        }
    }

    #[test]
    fn anonymous_symbols_are_shaped_as_generated_counter_text() {
        let input = column_fixture(
            r#"div{counter-reset:n 5}p::before{content:counter(n,symbols(numeric '0' '1')) '/' counters(n,'.',symbols(alphabetic 'a' 'b'))}"#,
            vec![node(2, Some(1), "p", "Body")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .generated_content
                .iter()
                .any(|item| item.text == "101/ba")
        );
    }

    #[test]
    fn counter_fallback_cycles_and_resource_bounds_terminate() {
        let input = column_fixture(
            r#"@counter-style a{system:fixed;symbols:'x';fallback:b}
            @counter-style b{system:fixed;symbols:'y';fallback:a}
            @counter-style grapheme{system:cyclic;symbols:'é';pad:3 '_'}
            @counter-style extended{system:extends simp-chinese-informal;prefix:'[';suffix:']';range:-20000 20000}
            @counter-style empty{system:symbolic;symbols:''}
            @counter-style huge{system:symbolic;symbols:'xx'}
            @counter-style roman{system:extends upper-roman;negative:'minus '}
            @counter-style limited{system:extends decimal;range:2 3;fallback:lower-alpha}"#,
            vec![],
        );
        let registry = generated_counter_styles::Registry::read(&input, false).unwrap();
        assert_eq!(registry.format(9, "a"), "9");
        assert_eq!(registry.format(i64::MAX, "empty"), "");
        assert_eq!(registry.format(i64::MAX, "huge"), i64::MAX.to_string());
        assert_eq!(registry.format(4000, "roman"), "4000");
        assert_eq!(registry.format(9, "roman"), "IX");
        assert_eq!(registry.format(4, "limited"), "d");
        assert_eq!(registry.format(19, "arabic-indic"), "١٩");
        assert_eq!(registry.format(2026, "armenian"), "ՍԻԶ");
        assert_eq!(registry.format(10000, "armenian"), "10000");
        assert_eq!(registry.format(10, "cjk-decimal"), "一〇");
        assert_eq!(registry.format(12, "cjk-earthly-branch"), "亥");
        assert_eq!(registry.format(13, "cjk-earthly-branch"), "一三");
        assert_eq!(registry.format(1, "hiragana-iroha"), "い");
        assert_eq!(registry.format(1, "katakana"), "ア");
        assert_eq!(registry.format(15, "hebrew"), "טו");
        assert_eq!(registry.format(16, "hebrew"), "טז");
        assert_eq!(registry.format(10000, "hebrew"), "י׳");
        assert_eq!(registry.format(19999, "georgian"), "ჵჰშჟთ");
        assert_eq!(registry.format(20000, "georgian"), "20000");
        assert_eq!(registry.format(-101, "japanese-informal"), "マイナス百一");
        assert_eq!(registry.format(101, "japanese-formal"), "壱百壱");
        assert_eq!(
            registry.format(1111, "korean-hangul-formal"),
            "일천일백일십일"
        );
        assert_eq!(
            registry.format(1111, "korean-hanja-formal"),
            "壹仟壹百壹拾壹"
        );
        assert_eq!(registry.format(10000, "japanese-informal"), "一〇〇〇〇");
        assert_eq!(registry.format(101, "simp-chinese-informal"), "一百零一");
        assert_eq!(registry.format(-11, "trad-chinese-informal"), "負十一");
        assert_eq!(registry.format(1010, "simp-chinese-formal"), "壹仟零壹拾");
        assert_eq!(registry.format(1, "ethiopic-numeric"), "፩");
        assert_eq!(registry.format(100, "ethiopic-numeric"), "፻");
        assert_eq!(
            registry.format(780100000092, "ethiopic-numeric"),
            "፸፰፻፩፼፼፺፪"
        );
        assert_eq!(registry.format(0, "ethiopic-numeric"), "0");
        assert_eq!(registry.format(1, "grapheme"), "__é");
        assert_eq!(registry.format(101, "extended"), "一百零一");
        assert_eq!(registry.marker(101, "extended"), "[一百零一]");
        assert_eq!(registry.format(10000, "extended"), "一〇〇〇〇");
        assert_eq!(registry.format(-10000, "japanese-informal"), "-10000");
        assert_eq!(registry.format(5, r#"symbols(numeric "0" "1")"#), "101");
        assert_eq!(registry.format(5, r#"symbols("x" "y")"#), "xxx");
        assert_eq!(registry.format(i64::MIN, "decimal"), i64::MIN.to_string());
    }

    #[test]
    fn generated_images_use_native_intrinsics_and_document_css_base() {
        let mut input = column_fixture(
            r#"p::before{content:url(icons/check.svg) / "Selected"}p::after{content:"A" url(icons/check.svg) "B";transform:translateX(7px)}"#,
            vec![node(2, Some(1), "p", "Body")],
        );
        input.document_base_url = "https://example.test/assets/".into();
        input.image_intrinsics.insert(
            "https://example.test/assets/icons/check.svg".into(),
            (37, 19),
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.generated_image_urls.len(), 2);
        for (id, url) in &output.generated_image_urls {
            assert_eq!(url, "https://example.test/assets/icons/check.svg");
            let box_model = output.geometry[id].box_model.as_ref().unwrap();
            assert_eq!(
                (box_model.content_box.width, box_model.content_box.height),
                (37., 19.)
            );
            assert!(!output.text_clusters.contains_key(id));
        }
        assert!(output.generated_content.iter().any(|entry| entry.owner == 2
            && entry.pseudo == "before"
            && entry.alternative.as_deref() == Some("Selected")));
        let child = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::generated-image")
            .unwrap();
        assert_ne!(
            child.style.get("transform").map(String::as_str),
            Some("translateX(7px)")
        );
        assert_eq!(input.nodes[1].children.len(), 0);
        input.image_intrinsics.clear();
        let pending = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(pending.generated_image_urls, output.generated_image_urls);
    }

    #[test]
    fn native_meter_minimum_normalizes_to_same_paint_fraction() {
        let mut a = node(2, Some(1), "meter", "");
        a.attributes.extend([
            ("min".into(), "1".into()),
            ("max".into(), "2".into()),
            ("value".into(), "1.5".into()),
        ]);
        let mut b = node(3, Some(1), "meter", "");
        b.attributes.insert("value".into(), "0.5".into());
        let output = compute(&column_fixture("", vec![a, b]), &LayoutLimits::default()).unwrap();
        assert_eq!(output.control_states[&2], output.control_states[&3]);
        assert_ne!(output.computed_styles[&2]["appearance"], "none");
    }

    #[test]
    fn closed_select_shapes_selected_label_and_hides_option_geometry() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3, 4];
        let mut input = column_fixture("select{font-size:20px}", vec![select]);
        input.nodes.extend([
            node(3, Some(2), "option", "First"),
            node(4, Some(2), "option", "Second"),
        ]);
        input.control_states.insert(
            2,
            oryn_common::v4::PaintControl::Select {
                active_option: None,
                open: false,
                multiple: false,
                disabled: false,
                selected: vec![4],
                label: "Second".into(),
                display_size: 1,
            },
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.text_fragments[&2]
                .iter()
                .any(|fragment| fragment.text == "Second")
        );
        assert!(output.geometry[&3].box_model.is_none());
        assert!(output.geometry[&4].box_model.is_none());
        assert!(!output.text_clusters.contains_key(&2));
        assert_eq!(input.nodes[1].children, vec![3, 4]);
    }

    #[test]
    fn closed_select_intrinsic_width_tracks_longest_option_not_current_selection() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![4, 3];
        let mut input = column_fixture("select{font-size:20px;width:auto}", vec![select]);
        input.nodes.extend([
            node(3, Some(2), "option", "A substantially wider choice"),
            node(4, Some(2), "option", "A"),
        ]);
        let fallback = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            fallback.text_fragments[&2]
                .iter()
                .any(|fragment| fragment.text == "A")
        );
        input.control_states.insert(
            2,
            oryn_common::v4::PaintControl::Select {
                active_option: None,
                open: false,
                multiple: false,
                disabled: false,
                selected: vec![3],
                label: "A substantially wider choice".into(),
                display_size: 1,
            },
        );
        let selected = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            fallback.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width,
            selected.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
        );
        input
            .nodes
            .iter_mut()
            .find(|node| node.id == 3)
            .unwrap()
            .text = "Another choice even wider than the previous choice".into();
        let changed = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            changed.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
                > selected.geometry[&2]
                    .box_model
                    .as_ref()
                    .unwrap()
                    .border_box
                    .width
        );
    }

    #[test]
    fn first_line_native_inheritance_keeps_inline_override_and_later_line_color() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.children = vec![3];
        paragraph.content = Some(vec![
            LayoutContent::Element(3),
            LayoutContent::Text {
                node_id: 44,
                text: " more words more words more words".into(),
            },
        ]);
        let mut input = column_fixture(
            "p{width:90px;font-size:16px;color:black}p::first-line{color:green;font-size:24px;background:pink}span{color:black}",
            vec![paragraph],
        );
        input.nodes.push(node(3, Some(2), "span", "First"));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let fragments = output
            .text_fragments
            .iter()
            .filter(|(id, _)| matches!(**id, 2 | 3))
            .flat_map(|(_, fragments)| fragments)
            .collect::<Vec<_>>();
        let top = fragments
            .iter()
            .map(|fragment| fragment.rect.y)
            .fold(f64::INFINITY, f64::min);
        assert!(
            fragments
                .iter()
                .filter(|fragment| fragment.rect.y < top + 0.1)
                .flat_map(|fragment| &fragment.glyph_runs)
                .any(|run| run.font_size == 24. && run.color == Some([0, 0, 0, 255]))
        );
        assert!(fragments.iter().any(|fragment| fragment.rect.y > top + 0.1));
        assert!(
            fragments
                .iter()
                .filter(|fragment| fragment.rect.y > top + 0.1)
                .flat_map(|fragment| &fragment.glyph_runs)
                .all(|run| run.color == Some([0, 0, 0, 255]) && run.font_size == 16.)
        );
    }

    #[test]
    fn first_letter_flag_preserves_native_utf16_cluster_and_font_scale() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "🇬🇧 UK flag".into(),
        }]);
        let input = column_fixture(
            "p{font-size:24px}p::first-letter{font-size:2em;color:red}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.text_fragments[&2]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .any(|run| run.font_size == 48. && run.color == Some([255, 0, 0, 255]))
        );
        let clusters = output.text_clusters[&44]
            .iter()
            .filter(|cluster| cluster.start < 4)
            .collect::<Vec<_>>();
        assert_eq!(clusters.iter().map(|cluster| cluster.start).min(), Some(0));
        assert_eq!(clusters.iter().map(|cluster| cluster.end).max(), Some(4));
        assert!(
            clusters
                .iter()
                .all(|cluster| cluster.start % 2 == 0 && cluster.end % 2 == 0)
        );
    }

    #[test]
    fn first_line_transform_stops_at_preserved_break_and_retains_utf16_offsets() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "ßa\nßb".into(),
        }]);
        let input = column_fixture(
            "p{white-space:pre;width:200px}p::first-line{text-transform:uppercase;font-family:lineface}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rendered = output
            .resolved_text
            .iter()
            .find(|text| text.layout_node == 2)
            .unwrap();
        assert_eq!(rendered.text, "SSA\nßb");
        assert!(output.font_demands["lineface"].contains(&('S' as u32)));
        assert!(!output.font_demands["lineface"].contains(&('b' as u32)));
        assert!(
            output.text_clusters[&44]
                .iter()
                .any(|cluster| cluster.start == 3 && cluster.end == 4)
        );
    }

    #[test]
    fn inline_first_letter_edges_advance_text_and_retain_separate_line_background() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "Ab".into(),
        }]);
        let input = column_fixture(
            "p{width:200px;font-size:20px}p::first-line{background:pink}p::first-letter{padding:0 3px;border:2px solid green;background:yellow}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let runs = output.text_fragments[&2]
            .iter()
            .flat_map(|fragment| &fragment.glyph_runs)
            .collect::<Vec<_>>();
        assert_eq!(runs.iter().filter(|run| run.line_box.is_some()).count(), 1);
        let letter = runs.iter().find_map(|run| run.text_box.as_ref()).unwrap();
        assert!(
            (letter.model.border_box.width - letter.model.content_box.width - 10.).abs() < 0.01
        );
        let clusters = &output.text_clusters[&44];
        let a = clusters.iter().find(|cluster| cluster.start == 0).unwrap();
        let b = clusters.iter().find(|cluster| cluster.start == 1).unwrap();
        assert!(b.rect.x >= a.rect.right() + 4.9);
    }

    #[test]
    fn first_line_background_uses_one_native_line_box_and_publishes_image_demand() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "First line".into(),
        }]);
        let input = column_fixture(
            "p{width:200px}p::first-line{background-image:url(https://example.test/line.png);background-color:pink}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let boxes = output.text_fragments[&2]
            .iter()
            .flat_map(|fragment| &fragment.glyph_runs)
            .filter_map(|run| run.line_box.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(boxes.len(), 1);
        assert!(boxes[0].model.content_box.width > 0.);
        assert!(image_requests(&output).contains("https://example.test/line.png"));
    }

    #[test]
    fn vertical_drop_cap_keeps_physical_padding_border_and_source_cluster() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
            let mut paragraph = node(2, Some(1), "p", "");
            paragraph.content = Some(vec![LayoutContent::Text {
                node_id: 44,
                text: "Ab".into(),
            }]);
            let input = column_fixture(
                &format!(
                    "p{{writing-mode:{mode};height:200px}}p::first-letter{{float:left;font-size:40px;padding:2px 3px 4px 5px;border:1px solid red;background:yellow}}"
                ),
                vec![paragraph],
            );
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let text_box = output.text_fragments[&2]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .find_map(|run| run.text_box.as_ref())
                .unwrap();
            let model = &text_box.model;
            assert_eq!(model.padding.left, 5.);
            assert_eq!(model.padding.top, 2.);
            assert!((model.border_box.width - model.content_box.width - 10.).abs() < 0.01);
            assert!((model.border_box.height - model.content_box.height - 8.).abs() < 0.01);
            assert!(
                output.text_clusters[&44]
                    .iter()
                    .any(|cluster| cluster.start == 0
                        && cluster.end == 1
                        && cluster.rect.width > 0.)
            );
        }
    }

    #[test]
    fn first_letter_transform_keeps_source_identity_and_demands_the_pseudo_font() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "ßabc".into(),
        }]);
        let input = column_fixture(
            "p{font-family:ordinary;font-size:16px}p::first-letter{text-transform:uppercase;font-family:dropcap;float:left;background:linear-gradient(red,blue);padding:3px;border:2px solid green}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rendered = output
            .resolved_text
            .iter()
            .find(|text| text.layout_node == 2)
            .unwrap();
        assert_eq!(rendered.text, "SSabc");
        assert!(output.font_demands["dropcap"].contains(&('S' as u32)));
        assert!(!output.font_demands["dropcap"].contains(&('a' as u32)));
        assert!(output.font_demands["ordinary"].contains(&('a' as u32)));
        let initial = output.text_clusters[&44]
            .iter()
            .filter(|cluster| cluster.start == 0)
            .collect::<Vec<_>>();
        assert!(!initial.is_empty());
        assert!(initial.iter().all(|cluster| cluster.end == 1));
        let text_box = output.text_fragments[&2]
            .iter()
            .flat_map(|fragment| &fragment.glyph_runs)
            .find_map(|run| run.text_box.as_ref())
            .unwrap();
        assert_eq!(text_box.owner, 2);
        assert_eq!(text_box.model.padding.left, 3.);
        assert_eq!(text_box.model.border.left, 2.);
        assert!(text_box.style["background-image"].contains("linear-gradient"));
    }

    #[test]
    fn picker_private_scroll_reveals_active_option_without_moving_button() {
        let mut select = node(2, Some(1), "select", "");
        select.children = (3..13).collect();
        let mut input = column_fixture(
            "select,select::picker(select){appearance:base-select}select{width:120px;height:24px}select::picker(select){height:60px;overflow:auto}option{display:block;height:20px}",
            vec![select],
        );
        input
            .nodes
            .extend((3..13).map(|id| node(id, Some(2), "option", &format!("Option {id}"))));
        let closed = compute(&input, &LayoutLimits::default()).unwrap();
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
            picker: Some(oryn_common::v4::SelectPickerState {
                scroll: (0., 0.),
                active: Some(12),
                reveal: true,
            }),
        });
        let revealed = compute(&input, &LayoutLimits::default()).unwrap();
        let popup = revealed
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        let scroll = popup.projection.as_ref().unwrap().scroll.as_ref().unwrap();
        assert!(scroll.offset_y > 0.);
        assert_eq!(revealed.geometry[&2], closed.geometry[&2]);
        let option = revealed.geometry[&12]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!(
            option.y >= scroll.scrollport.y - 0.1
                && option.bottom() <= scroll.scrollport.bottom() + 0.1
        );
        let state = input.top_layer[0].picker.as_mut().unwrap();
        state.reveal = false;
        state.scroll = (0., 0.);
        let reset = compute(&input, &LayoutLimits::default()).unwrap();
        let popup = reset
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        assert_eq!(
            popup
                .projection
                .as_ref()
                .unwrap()
                .scroll
                .as_ref()
                .unwrap()
                .offset_y,
            0.
        );
        input.top_layer[0].picker.as_mut().unwrap().scroll = (0., 1e9);
        let clamped = compute(&input, &LayoutLimits::default()).unwrap();
        let popup = clamped
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        let scroll = popup.projection.as_ref().unwrap().scroll.as_ref().unwrap();
        assert!(
            (scroll.offset_y - (scroll.scrollable.bottom() - scroll.scrollport.bottom())).abs()
                < 0.01
        );
    }

    #[test]
    fn picker_first_formatted_child_receives_first_line_and_letter() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3, 4];
        let mut input = column_fixture(
            "select,select::picker(select){appearance:base-select}select::picker(select)::first-line{color:navy}select::picker(select)::first-letter{color:red;font-size:20px}",
            vec![select],
        );
        input.nodes.extend([
            node(3, Some(2), "div", "Hello World"),
            node(4, Some(2), "option", "one"),
        ]);
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.text_fragments[&3]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .any(|run| run.color == Some([255, 0, 0, 255]) && run.font_size == 20.)
        );
        assert!(
            output.text_fragments[&3]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .any(|run| run.color == Some([0, 0, 128, 255]))
        );
        assert!(
            output.text_fragments[&4]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .all(|run| run.color != Some([0, 0, 128, 255]))
        );
    }

    #[test]
    fn floating_first_letter_places_following_text_in_native_exclusion_band() {
        let mut paragraph = node(2, Some(1), "p", "");
        paragraph.content = Some(vec![LayoutContent::Text {
            node_id: 44,
            text: "Ab".into(),
        }]);
        let input = column_fixture(
            "p{width:200px;font-size:16px}p::first-letter{float:left;font-size:64px}p::first-line{background:lightblue}",
            vec![paragraph],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let clusters = &output.text_clusters[&44];
        let letter = clusters
            .iter()
            .find(|cluster| cluster.start == 0 && cluster.end == 1)
            .unwrap();
        let rest = clusters
            .iter()
            .find(|cluster| cluster.start == 1 && cluster.end == 2)
            .unwrap();
        assert!(letter.rect.width > 0. && rest.rect.x >= letter.rect.right() - 0.1);
        assert!(
            output.text_fragments[&2]
                .iter()
                .flat_map(|fragment| &fragment.glyph_runs)
                .any(|run| run.font_size == 64.)
        );
    }

    #[test]
    fn picker_nested_pseudos_join_authored_grid_without_changing_button() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3, 4];
        let mut input = column_fixture(
            r#"select{appearance:base-select;width:100px}
            select::picker(select){appearance:base-select;display:grid;grid-template-columns:40px 40px;gap:5px;min-width:0;width:85px}
            select::picker(select)::before{content:"First";display:block;height:20px;background:lime}
            select::picker(select)::after{content:"Last";display:block;height:20px;background:blue}
            option{height:20px}"#,
            vec![select],
        );
        input.nodes.extend([
            node(3, Some(2), "option", "One"),
            node(4, Some(2), "option", "Two"),
        ]);
        let closed = compute(&input, &LayoutLimits::default()).unwrap();
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let open = compute(&input, &LayoutLimits::default()).unwrap();
        let picker = open
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        assert_eq!(picker.style["display"], "grid");
        assert_eq!(open.geometry[&2], closed.geometry[&2]);
        let before = open
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::before" && node.parent_id == Some(picker.node_id))
            .unwrap();
        let after = open
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::after" && node.parent_id == Some(picker.node_id))
            .unwrap();
        assert!(before.text.iter().any(|text| text.text == "First"));
        assert!(after.text.iter().any(|text| text.text == "Last"));
        let a = before.geometry.box_model.as_ref().unwrap().border_box;
        let b = open.geometry[&3].box_model.as_ref().unwrap().border_box;
        let c = open.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!((a.y - b.y).abs() < 0.1 && b.x > a.x);
        assert!(c.y > a.y);
        assert!(
            open.generated_content
                .iter()
                .all(|entry| entry.owner != picker.node_id)
        );
    }

    #[test]
    fn picker_nested_generated_image_retains_demand_and_identity() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3];
        let mut input = column_fixture(
            r#"select,select::picker(select){appearance:base-select}
            select::picker(select)::before{content:url("https://example.test/picker.png");display:block;width:20px;height:20px}
            option::before{content:"Option prefix"}"#,
            vec![select],
        );
        input.nodes.push(node(3, Some(2), "option", "One"));
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output
                .css_image_requests
                .contains("https://example.test/picker.png")
        );
        assert!(
            output
                .generated_image_urls
                .values()
                .any(|url| url == "https://example.test/picker.png")
        );
        assert_eq!(
            output
                .generated_nodes
                .iter()
                .filter(|node| node.tag == "::before" && node.parent_id == Some(3))
                .count(),
            1
        );
        let ids = output
            .generated_nodes
            .iter()
            .map(|node| node.node_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), output.generated_nodes.len());
    }

    #[test]
    fn picker_anchor_fallbacks_use_outer_geometry_and_native_pseudo_rules() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3];
        let mut input = column_fixture(
            "select{appearance:base-select;position:absolute;left:160px;top:90px;width:20px;height:20px;padding:0;border:0}select::picker(select){appearance:base-select;position:fixed;position-area:bottom right;position-try-fallbacks:--above;min-width:0;width:40px;height:30px;margin:0;padding:0;border:0}@position-try --above{position-area:top left}",
            vec![select],
        );
        input.nodes.push(node(3, Some(2), "option", "One"));
        input.viewport.width = 200.;
        input.viewport.height = 120.;
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let owner = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let picker = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap()
            .geometry
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert!((picker.right() - owner.x).abs() < 0.1);
        assert!((picker.bottom() - owner.y).abs() < 0.1);
    }

    #[test]
    fn picker_respects_authored_fixed_insets_and_size_constraints() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3];
        let mut input = column_fixture(
            "select{appearance:base-select;width:120px}select::picker(select){appearance:base-select;position:fixed;left:17px;top:23px;width:77px;min-width:0;max-height:31px}",
            vec![select],
        );
        input.nodes.push(node(3, Some(2), "option", "An option"));
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let picker = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        let model = picker.geometry.box_model.as_ref().unwrap();
        assert!((model.border_box.x - 17.).abs() < 0.1);
        assert!((model.border_box.y - 23.).abs() < 0.1);
        assert!((model.content_box.width - 77.).abs() < 0.1);
        assert!(model.content_box.height <= 31.);
    }

    #[test]
    fn picker_keeps_button_position_and_native_option_identity() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3, 4];
        let mut input = column_fixture(
            "select{font-size:20px;width:120px}option{display:block}",
            vec![select],
        );
        input.nodes.extend([
            node(3, Some(2), "option", "First"),
            node(4, Some(2), "option", "Second"),
        ]);
        input.control_states.insert(
            2,
            oryn_common::v4::PaintControl::Select {
                active_option: None,
                open: false,
                multiple: false,
                disabled: false,
                selected: vec![3],
                label: "First".into(),
                display_size: 1,
            },
        );
        let closed = compute(&input, &LayoutLimits::default()).unwrap();
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
        });
        let opened = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(opened.geometry[&2], closed.geometry[&2]);
        let picker = opened
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap();
        assert_eq!(picker.parent_id, Some(2));
        assert_eq!(picker.top_layer, Some(0));
        assert_eq!(opened.paint_parent_overrides[&3], Some(picker.node_id));
        assert_eq!(opened.paint_parent_overrides[&4], Some(picker.node_id));
        assert!(opened.geometry[&3].box_model.is_some());
        assert!(
            opened.text_fragments[&4]
                .iter()
                .any(|fragment| fragment.text == "Second")
        );
        assert_eq!(
            input.nodes.iter().find(|node| node.id == 3).unwrap().parent,
            Some(2)
        );
    }

    #[test]
    fn top_layer_and_open_states_reach_native_css_matching() {
        let mut dialog = node(2, Some(1), "dialog", "");
        dialog.attributes.insert("open".into(), "".into());
        let mut details = node(3, Some(1), "details", "");
        details.attributes.insert("open".into(), "".into());
        let mut input = column_fixture(
            "dialog,details,select,section{width:1px}dialog:open{width:21px}details:open{width:22px}dialog:modal{height:31px}section:popover-open{width:23px}select:open{width:24px}",
            vec![
                dialog,
                details,
                node(4, Some(1), "section", ""),
                node(5, Some(1), "select", ""),
            ],
        );
        input.top_layer = vec![
            oryn_common::v4::TopLayerEntry {
                anchor_source: None,
                picker: None,
                node_id: 2,
                kind: oryn_common::v4::TopLayerKind::ModalDialog,
            },
            oryn_common::v4::TopLayerEntry {
                anchor_source: None,
                picker: None,
                node_id: 4,
                kind: oryn_common::v4::TopLayerKind::Popover,
            },
            oryn_common::v4::TopLayerEntry {
                anchor_source: None,
                picker: None,
                node_id: 5,
                kind: oryn_common::v4::TopLayerKind::SelectPicker,
            },
        ];
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["width"], "21px");
        assert_eq!(styles[&2]["height"], "31px");
        assert_eq!(styles[&3]["width"], "22px");
        assert_eq!(styles[&4]["width"], "23px");
        assert_eq!(styles[&5]["width"], "24px");
        input.top_layer.clear();
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&4]["width"], "1px");
        assert_eq!(styles[&5]["width"], "1px");
        assert_eq!(styles[&2]["width"], "21px");
    }

    #[test]
    fn backdrop_uses_own_insets_and_transform_without_owner_clip() {
        let dialog = node(2, Some(1), "dialog", "");
        let mut input = column_fixture(
            "div{transform:translate(70px);overflow:hidden;width:20px;height:20px}dialog{display:block;width:10px;height:10px}dialog::backdrop{left:30px;top:40px;right:auto;bottom:auto;width:100px;height:80px;transform:translateX(5px);background:red}",
            vec![dialog],
        );
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            anchor_source: None,
            picker: None,
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::ModalDialog,
        });
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let backdrop = &output.backdrops[&2];
        assert_eq!(backdrop.parent_id, Some(2));
        let projection = backdrop.projection.as_ref().unwrap();
        assert!(projection.contains_point(40., 45.));
        assert!(!projection.contains_point(10., 10.));
        assert!((projection.box_model.border_box.width - 100.).abs() < 0.02);
        assert!(backdrop.backdrop.is_none());
        assert!(backdrop.backdrop_style.is_none());
    }

    #[test]
    fn computed_styles_export_native_formatting_and_paint_longhands() {
        let input = column_fixture(
            "div {column-count:3;column-width:80px;column-rule:2px dashed red;ruby-align:space-between;text-decoration:underline wavy blue;text-decoration-thickness:2px;text-emphasis:open dot green;text-combine-upright:all;box-shadow:1px 2px 3px black;border-radius:7px;background-origin:content-box;background-clip:padding-box}",
            Vec::new(),
        );
        let computed = stylo_computed_values(&input).unwrap();
        let style = &computed[&1];
        assert_eq!(style["column-count"], "3");
        assert_eq!(style["column-width"], "80px");
        assert_eq!(style["column-rule-style"], "dashed");
        assert_eq!(style["ruby-align"], "space-between");
        assert_eq!(style["text-decoration-line"], "underline");
        assert_eq!(style["text-decoration-style"], "wavy");
        assert_eq!(style["text-decoration-thickness"], "2px");
        assert!(style["text-emphasis-style"].contains("dot"));
        assert_eq!(style["text-combine-upright"], "all");
        assert_ne!(style["box-shadow"], "none");
        assert_eq!(style["border-top-left-radius"], "7px");
        assert_eq!(style["background-origin"], "content-box");
        assert_eq!(style["background-clip"], "padding-box");
    }

    #[test]
    fn computed_table_alignment_and_caption_side_preserve_authored_and_initial_values() {
        for (declarations, alignment, caption) in [
            (
                "vertical-align:middle;caption-side:bottom",
                "middle",
                "bottom",
            ),
            ("vertical-align:bottom;caption-side:top", "bottom", "top"),
            ("", "baseline", "top"),
        ] {
            let input = column_fixture(&format!("div {{{declarations}}}"), Vec::new());
            let computed = stylo_computed_values(&input).unwrap();
            let style = &computed[&1];
            assert_eq!(style["vertical-align"], alignment);
            assert_eq!(style["caption-side"], caption);
        }
    }

    #[test]
    fn multicol_block_flow_uses_column_measure_and_native_breaks() {
        let input = column_fixture(
            "div {width:220px;column-count:2;column-gap:20px} p {margin:0;height:30px}",
            vec![
                node(2, Some(1), "p", "a"),
                node(3, Some(1), "p", "b"),
                node(4, Some(1), "p", "c"),
                node(5, Some(1), "p", "d"),
            ],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(2).width - 100.).abs() < 0.01);
        assert!((rect(4).x - rect(2).x - 120.).abs() < 0.01);
        assert!((rect(4).y - rect(2).y).abs() < 0.01);
        assert!((rect(1).height - 60.).abs() < 0.01);
        assert!(
            (output.text_fragments[&4][0].rect.x - output.text_fragments[&2][0].rect.x - 120.)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn multicol_split_block_boxes_keep_native_text_and_slice_edges() {
        let input = column_fixture(
            "div {width:220px;column-count:2;column-gap:20px;height:40px;column-fill:auto;column-rule:4px solid red} p {margin:0;white-space:pre;font-size:16px;line-height:20px;border-left:1px solid black;border-right:1px solid black}",
            vec![node(2, Some(1), "p", "a\nb\nc\nd")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let fragments = &output.geometry[&2].client_rects;
        assert_eq!(fragments.len(), 2);
        assert!((fragments[1].x - fragments[0].x - 120.).abs() < 0.01);
        let text = &output.text_fragments[&2];
        assert_eq!(text.len(), 4);
        assert!((text[2].rect.x - text[0].rect.x - 120.).abs() < 0.01);
        assert!((text[2].rect.y - text[0].rect.y).abs() < 0.01);
        assert_eq!(
            text.iter()
                .map(|fragment| fragment.flow_line.unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        let flow = text
            .iter()
            .map(|fragment| fragment.flow_rect.unwrap())
            .collect::<Vec<_>>();
        assert!(flow.iter().all(|rect| (rect.height - 20.).abs() < 0.01));
        assert!((flow[2].x - flow[0].x - 120.).abs() < 0.01);
        assert!((flow[2].y - flow[0].y).abs() < 0.01);
        assert_eq!(output.paint_projections[&2].fragment_boxes.len(), 2);
        let rule = &output.paint_projections[&1].border_segments[0];
        assert_eq!(rule.edge.width, 4.);
        assert!(!rule.horizontal);
        assert_eq!(rule.rect.height, 40.);
    }

    #[test]
    fn multicol_spanner_formats_at_full_width_and_separates_column_rules() {
        let input = column_fixture(
            "div {width:220px;column-count:2;column-gap:20px;column-rule:2px solid red} p {margin:0;height:20px} h3 {margin:0;height:30px;column-span:all}",
            vec![
                node(2, Some(1), "p", "a"),
                node(3, Some(1), "p", "b"),
                node(4, Some(1), "h3", "spanner"),
                node(5, Some(1), "p", "c"),
                node(6, Some(1), "p", "d"),
            ],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(4).width - 220.).abs() < 0.01);
        assert!((rect(4).y - rect(2).y - 20.).abs() < 0.01);
        assert!((rect(5).y - rect(4).bottom()).abs() < 0.01);
        assert!((rect(6).x - rect(5).x - 120.).abs() < 0.01);
        let rules = &output.paint_projections[&1].border_segments;
        assert_eq!(rules.len(), 2);
        assert!(
            rules
                .iter()
                .all(|rule| rule.rect.bottom() <= rect(4).y + 0.01
                    || rule.rect.y >= rect(4).bottom() - 0.01)
        );
    }

    #[test]
    fn multicol_nested_spanner_escapes_ordinary_block_but_not_flow_root() {
        // Pinned css/css-multicol/multicol-span-all-007.html puts h3 inside a div.
        let mut nested = node(3, Some(1), "section", "");
        nested.children = vec![4];
        nested.content = Some(vec![LayoutContent::Element(4)]);
        let mut input = column_fixture(
            "div {width:220px;column-count:2;column-gap:20px} p {margin:0;height:20px} section {margin:0} h3 {margin:0;height:30px;column-span:all}",
            vec![
                node(2, Some(1), "p", "before"),
                nested,
                node(5, Some(1), "p", "after"),
            ],
        );
        input.nodes.push(node(4, Some(3), "h3", "nested span"));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            (output.geometry[&4]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
                - 220.)
                .abs()
                < 0.01
        );
        // Keep this width assertion in one fragment. A normally fragmented
        // 100px box has a wider union when it occupies both physical columns.
        input
            .stylesheets
            .push("div {height:200px;column-fill:auto} section {display:flow-root}".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&3]["display"], "flow-root");
        assert!(
            (output.geometry[&4]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
                - 100.)
                .abs()
                < 0.01,
            "{:?}",
            output.geometry[&4]
        );
    }

    #[test]
    fn multicol_forced_break_moves_following_block_even_with_room() {
        let input = column_fixture(
            "div {width:220px;column-count:2;column-gap:20px;height:100px;column-fill:auto} p {margin:0;height:20px} p:last-child {break-before:column}",
            vec![node(2, Some(1), "p", "a"), node(3, Some(1), "p", "b")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let a = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let b = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!((b.x - a.x - 120.).abs() < 0.01);
        assert!((b.y - a.y).abs() < 0.01);
    }

    #[test]
    fn internal_ruby_boxes_remain_in_anonymous_inline_runs_around_blocks() {
        for display in [
            "ruby-base",
            "ruby-text",
            "ruby-base-container",
            "ruby-text-container",
        ] {
            let mut input = column_fixture(
                &format!(
                    "div{{width:300px;font-size:16px;line-height:24px}} span{{display:{display}}} p{{margin:0;height:30px}}"
                ),
                vec![
                    node(2, Some(1), "span", "ruby"),
                    node(3, Some(1), "p", "block"),
                ],
            );
            input.nodes[0].content = Some(vec![
                LayoutContent::Text {
                    node_id: 10,
                    text: "before ".into(),
                },
                LayoutContent::Element(2),
                LayoutContent::Element(3),
                LayoutContent::Text {
                    node_id: 11,
                    text: "after".into(),
                },
            ]);
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert_eq!(output.computed_styles[&2]["display"], display);
            let ruby = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            let block = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            let before = output.text_fragments[&1]
                .iter()
                .find(|fragment| fragment.text.trim() == "before")
                .unwrap();
            let after = output.text_fragments[&1]
                .iter()
                .find(|fragment| fragment.text == "after")
                .unwrap();
            assert!(ruby.width > 0. && ruby.height > 0., "{display}: {ruby:?}");
            assert!(
                ruby.x >= before.rect.right() - 0.01,
                "{display}: ruby={ruby:?} before={:?}",
                before.rect
            );
            assert!(
                ruby.y < before.rect.bottom() && ruby.bottom() > before.rect.y,
                "{display}: ruby={ruby:?} before={:?}",
                before.rect
            );
            assert!(
                block.y >= ruby.bottom() - 0.01,
                "{display}: ruby={ruby:?} block={block:?}"
            );
            assert!(
                after.rect.y >= block.bottom() - 0.01,
                "{display}: after={:?} block={block:?}",
                after.rect
            );
            assert_eq!(
                output.text_fragments[&2]
                    .iter()
                    .map(|fragment| fragment.text.as_str())
                    .collect::<String>(),
                "ruby"
            );
        }
    }

    #[test]
    fn anonymous_inline_paragraphs_surround_block_children_in_source_order() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "before".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "after".into(),
            },
        ]);
        let block = node(2, Some(1), "p", "middle");
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            nodes: vec![root, block],
            stylesheets: vec!["div {width:200px} p {margin:0;height:30px}".into()],
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let fragments = &output.text_fragments[&1];
        let before = fragments
            .iter()
            .find(|fragment| fragment.text == "before")
            .unwrap();
        let after = fragments
            .iter()
            .find(|fragment| fragment.text == "after")
            .unwrap();
        let block = output.geometry[&2].box_model.as_ref().unwrap();
        assert!(
            before.rect.bottom() <= block.border_box.y + 0.01,
            "before={:?}, block={:?}, after={:?}",
            before.rect,
            block,
            after.rect
        );
        assert!(
            after.rect.y >= block.border_box.bottom() - 0.01,
            "after={:?}, block={:?}",
            after.rect,
            block
        );
        assert_eq!(output.text_fragments[&2][0].text, "middle");
    }

    #[test]
    fn mixed_inline_text_and_replaced_boxes_share_native_lines_and_owner_paint() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a ".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Element(3),
            LayoutContent::Text {
                node_id: 11,
                text: " c".into(),
            },
        ]);
        let span = node(2, Some(1), "span", "b");
        let mut image = node(3, Some(1), "img", "");
        image.intrinsic_image_size = Some((20, 20));
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            nodes: vec![root, span, image],
            stylesheets: vec!["div {width:200px;font-size:16px} span {color:red}".into()],
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let root_box = output.geometry[&1].box_model.as_ref().unwrap();
        let span_box = output.geometry[&2].box_model.as_ref().unwrap();
        let image_box = output.geometry[&3].box_model.as_ref().unwrap();
        assert!(span_box.border_box.x > root_box.content_box.x);
        assert!(image_box.border_box.x >= span_box.border_box.right());
        assert!(image_box.border_box.y < root_box.content_box.bottom());
        assert_eq!(
            output.text_fragments[&2]
                .iter()
                .map(|f| f.text.as_str())
                .collect::<String>(),
            "b"
        );
        assert!(
            !output.text_fragments[&1]
                .iter()
                .any(|f| f.text.contains('b'))
        );
        assert_eq!(image_box.border_box.width, 20.);
        assert!(root_box.content_box.height < 50.);
    }

    // These fixtures use one font and an explicit line-height. CSS leading is
    // split equally around ascent/descent, while Range selection retains the
    // larger font bounds. Float exclusion is defined against the flow band.
    fn single_font_flow_top(selection: CssRect, line_height: f64) -> f64 {
        selection.y + (selection.height - line_height).max(0.) / 2.
    }

    #[test]
    fn native_inline_text_wraps_beside_float_then_returns_to_full_width() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 10,
                text: "abcdefghij ".repeat(12),
            },
        ]);
        let mut input = column_fixture(
            "div {width:100px;font-size:16px;line-height:20px;overflow-wrap:anywhere} aside {float:left;width:40px;height:40px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let floated = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let root = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let clusters = &output.text_clusters[&10];
        assert!(
            clusters
                .iter()
                .any(|cluster| single_font_flow_top(cluster.rect, 20.) < floated.bottom() - 0.01)
        );
        assert!(
            clusters
                .iter()
                .filter(|cluster| single_font_flow_top(cluster.rect, 20.) < floated.bottom() - 0.01)
                .all(|cluster| cluster.rect.x >= floated.right() - 0.01),
            "float={floated:?}, overlapping={:?}, text={:?}",
            clusters
                .iter()
                .filter(
                    |cluster| single_font_flow_top(cluster.rect, 20.) < floated.bottom() - 0.01
                        && cluster.rect.x < floated.right() - 0.01
                )
                .take(8)
                .collect::<Vec<_>>(),
            output.text_fragments[&1]
                .iter()
                .map(|fragment| fragment.rect)
                .collect::<Vec<_>>()
        );
        assert!(
            clusters
                .iter()
                .any(
                    |cluster| single_font_flow_top(cluster.rect, 20.) >= floated.bottom() - 0.01
                        && cluster.rect.x < root.x + 1.
                )
        );
    }

    #[test]
    fn float_exclusions_persist_across_preserved_native_paragraph_breaks() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 10,
                text: "a\nb\nc\nd".into(),
            },
        ]);
        let mut input = column_fixture(
            "div {width:100px;font-size:16px;line-height:20px;white-space:pre} aside {float:left;width:40px;height:40px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let text = &output.text_fragments[&1];
        let a = text
            .iter()
            .find(|fragment| fragment.text.starts_with('a'))
            .unwrap();
        let b = text
            .iter()
            .find(|fragment| fragment.text.starts_with('b'))
            .unwrap();
        let c = text
            .iter()
            .find(|fragment| fragment.text.starts_with('c'))
            .unwrap();
        assert!((a.rect.x - b.rect.x).abs() < 0.01);
        assert!((a.rect.x - c.rect.x - 40.).abs() < 0.01);
        assert!((b.rect.y - a.rect.y - 20.).abs() < 0.01);
    }

    #[test]
    fn floats_cross_ordinary_block_boundaries_and_nested_padding() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3];
        let mut nested = node(2, Some(1), "section", "");
        nested.children = vec![4];
        nested.content = Some(vec![
            LayoutContent::Element(4),
            LayoutContent::Text {
                node_id: 20,
                text: "a".into(),
            },
        ]);
        let mut later = node(3, Some(1), "p", "");
        later.content = Some(vec![LayoutContent::Text {
            node_id: 30,
            text: "b c d e f g h i j k l m n o p q r s t u v w x y z ".repeat(4),
        }]);
        let mut input = column_fixture(
            "main {width:140px;font:16px/20px sans-serif} section {padding-left:10px} p {margin:0} aside {float:left;width:40px;height:80px}",
            Vec::new(),
        );
        input.nodes = vec![root, nested, later, node(4, Some(2), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let float = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        let main = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        assert!((float.x - main.x - 10.).abs() < 0.01);
        let clusters = &output.text_clusters[&30];
        assert!(
            clusters
                .iter()
                .any(|cluster| single_font_flow_top(cluster.rect, 20.) < float.bottom())
        );
        assert!(
            clusters
                .iter()
                .filter(|cluster| single_font_flow_top(cluster.rect, 20.) < float.bottom() - 0.01)
                .all(|cluster| cluster.rect.x >= float.right() - 0.01)
        );
        assert!(
            clusters
                .iter()
                .any(
                    |cluster| single_font_flow_top(cluster.rect, 20.) >= float.bottom()
                        && cluster.rect.x < main.x + 1.
                )
        );
        assert!(main.height >= float.bottom() - main.y);
    }

    #[test]
    fn independent_bfc_avoids_float_and_percentage_width_keeps_containing_basis() {
        for (width, expected_width, below) in [
            ("auto", 60., false),
            ("50%", 50., false),
            ("80%", 80., true),
        ] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3];
            let mut input = column_fixture(
                &format!(
                    "main {{width:100px}} aside {{float:left;width:40px;height:80px}} section {{display:flow-root;width:{width};height:20px}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "aside", ""),
                node(3, Some(1), "section", "text"),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            let block = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            assert!(
                (block.width - expected_width).abs() < 0.01,
                "{width}: {block:?}"
            );
            if below {
                assert!(block.y >= float.bottom() - 0.01);
            } else {
                assert!(block.x >= float.right() - 0.01);
                assert!(block.y < float.bottom());
            }
        }
    }

    #[test]
    fn parent_child_margin_struts_are_absorbed_once_and_join_siblings() {
        for floating in [false, true] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3, 5];
            if floating {
                root.children.insert(0, 6);
            }
            let mut section = node(3, Some(1), "section", "");
            section.children = vec![4];
            let mut input = column_fixture(
                "main {display:flow-root;width:200px} header {height:20px;margin-bottom:30px} section {margin-top:10px;margin-bottom:5px} p {height:20px;margin:50px 0 40px} footer {height:10px;margin-top:15px} aside {float:left;width:1px;height:1px}",
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "header", ""),
                section,
                node(4, Some(3), "p", ""),
                node(5, Some(1), "footer", ""),
            ];
            if floating {
                input.nodes.push(node(6, Some(1), "aside", ""));
            }
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            assert!(
                (rect(3).y - rect(2).bottom() - 50.).abs() < 0.01,
                "floating={floating}"
            );
            assert!((rect(4).y - rect(3).y).abs() < 0.01);
            assert!((rect(3).height - 20.).abs() < 0.01);
            assert!((rect(5).y - rect(3).bottom() - 40.).abs() < 0.01);
        }
    }

    #[test]
    fn vertical_parent_child_margin_edges_are_absorbed_once() {
        for mode in ["vertical-lr", "vertical-rl"] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3, 5];
            let mut section = node(3, Some(1), "section", "");
            section.children = vec![4];
            let mut input = column_fixture(
                &format!(
                    "main {{writing-mode:{mode};height:200px;display:flow-root}} header {{width:20px;margin-block-end:30px}} section {{margin-block-start:10px;margin-block-end:5px}} p {{width:20px;margin-block-start:50px;margin-block-end:40px;margin-inline:0}} footer {{width:10px;margin-block-start:15px}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "header", ""),
                section,
                node(4, Some(3), "p", ""),
                node(5, Some(1), "footer", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            let gap = |a: CssRect, b: CssRect| {
                if mode == "vertical-rl" {
                    a.x - b.right()
                } else {
                    b.x - a.right()
                }
            };
            assert!((gap(rect(2), rect(3)) - 50.).abs() < 0.01, "{mode}");
            assert!((rect(4).x - rect(3).x).abs() < 0.01, "{mode}");
            assert!((rect(3).width - 20.).abs() < 0.01, "{mode}");
            assert!((gap(rect(3), rect(5)) - 40.).abs() < 0.01, "{mode}");
        }
    }

    #[test]
    fn empty_inline_line_boxes_and_parent_margin_barriers_use_same_edges() {
        for (decoration, barrier) in [
            ("", false),
            ("padding-left:1px", true),
            ("border-top:1px solid", true),
            ("margin-left:2px;margin-right:-2px", true),
            ("display:contents;padding-left:10px", false),
        ] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2];
            let mut section = node(2, Some(1), "section", "");
            section.children = vec![3, 4];
            section.content = Some(vec![LayoutContent::Element(3), LayoutContent::Element(4)]);
            let mut input = column_fixture(
                &format!(
                    "main {{width:200px}} section {{margin-top:10px;font:16px/20px sans-serif}} span {{{decoration}}} p {{height:10px;margin:30px 0 0}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                section,
                node(3, Some(2), "span", " \t\n"),
                node(4, Some(2), "p", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            if barrier {
                assert!((rect(2).y - rect(1).y - 10.).abs() < 0.01, "{decoration}");
                assert!(rect(4).y - rect(2).y >= 49.99, "{decoration}");
            } else {
                assert!((rect(2).y - rect(1).y - 30.).abs() < 0.01, "{decoration}");
                assert!((rect(4).y - rect(2).y).abs() < 0.01, "{decoration}");
            }
        }
    }

    #[test]
    fn empty_inline_anchors_retain_positions_without_adding_line_height() {
        for mode in ["horizontal-tb", "vertical-lr", "vertical-rl"] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3, 4];
            root.content = Some(vec![
                LayoutContent::Element(2),
                LayoutContent::Element(3),
                LayoutContent::Element(4),
            ]);
            let mut input = column_fixture(
                &format!(
                    "main {{writing-mode:{mode};width:200px;height:200px}} div {{block-size:50px}} a {{display:inline}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "div", ""),
                node(3, Some(1), "a", ""),
                node(4, Some(1), "div", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            let (before, anchor, after) = (rect(2), rect(3), rect(4));
            assert_eq!((anchor.width, anchor.height), (0., 0.), "{mode}");
            if mode == "horizontal-tb" {
                assert!(
                    (anchor.y - before.bottom()).abs() < 0.01,
                    "{mode}: {anchor:?}"
                );
                assert!((after.y - anchor.y).abs() < 0.01, "{mode}");
            } else if mode == "vertical-lr" {
                assert!(
                    (anchor.x - before.right()).abs() < 0.01,
                    "{mode}: {anchor:?}"
                );
                assert!((after.x - anchor.x).abs() < 0.01, "{mode}");
            } else {
                assert!((anchor.x - before.x).abs() < 0.01, "{mode}: {anchor:?}");
                assert!((after.right() - anchor.x).abs() < 0.01, "{mode}");
            }
        }
    }

    #[test]
    fn trailing_parent_margin_uses_actual_clearance_and_used_child_height() {
        for (clear, height, expected_margin, inside) in [
            ("both", 10., 40., 0.),
            ("both", 0., 10., 40.),
            ("right", 0., 40., 0.),
        ] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3, 5];
            let mut section = node(3, Some(1), "section", "");
            section.children = vec![4];
            let mut input = column_fixture(
                &format!(
                    "main {{width:200px}} aside {{float:left;width:40px;height:80px}} section {{margin:0 0 10px}} p {{clear:{clear};height:{height}px;margin:0 0 40px}} footer {{height:10px;margin:0}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "aside", ""),
                section,
                node(4, Some(3), "p", ""),
                node(5, Some(1), "footer", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let model = |id| output.geometry[&id].box_model.as_ref().unwrap();
            assert!(
                (model(3).margin.bottom - expected_margin).abs() < 0.01,
                "{clear} {height}"
            );
            assert!(
                (model(3).border_box.bottom() - model(4).border_box.bottom() - inside).abs() < 0.01,
                "{clear} {height}"
            );
            assert!(
                (model(5).border_box.y - model(3).border_box.bottom() - expected_margin).abs()
                    < 0.01,
                "{clear} {height}"
            );
        }
    }

    #[test]
    fn vertical_trailing_clearance_uses_native_logical_end_before_projection() {
        for mode in ["vertical-lr", "vertical-rl"] {
            for (clear, width, expected, inside) in [
                ("both", 10., 40., 0.),
                ("both", 0., 10., 40.),
                ("right", 0., 40., 0.),
            ] {
                let end = if mode == "vertical-rl" {
                    "left"
                } else {
                    "right"
                };
                let mut root = node(1, None, "main", "");
                root.children = vec![2, 3, 5];
                let mut section = node(3, Some(1), "section", "");
                section.children = vec![4];
                let mut input = column_fixture(
                    &format!(
                        "main {{writing-mode:{mode};height:200px}} aside {{float:left;width:80px;height:40px}} section {{margin:0;margin-{end}:10px}} p {{clear:{clear};width:{width}px;margin:0;margin-{end}:40px}} footer {{width:10px;margin:0}}"
                    ),
                    Vec::new(),
                );
                input.nodes = vec![
                    root,
                    node(2, Some(1), "aside", ""),
                    section,
                    node(4, Some(3), "p", ""),
                    node(5, Some(1), "footer", ""),
                ];
                let output = compute(&input, &LayoutLimits::default()).unwrap();
                let model = |id| output.geometry[&id].box_model.as_ref().unwrap();
                let parent = model(3);
                let child = model(4);
                let footer = model(5);
                let reverse = mode == "vertical-rl";
                let margin = if reverse {
                    parent.margin.left
                } else {
                    parent.margin.right
                };
                let inset = if reverse {
                    child.border_box.x - parent.border_box.x
                } else {
                    parent.border_box.right() - child.border_box.right()
                };
                let gap = if reverse {
                    parent.border_box.x - footer.border_box.right()
                } else {
                    footer.border_box.x - parent.border_box.right()
                };
                assert!((margin - expected).abs() < 0.01, "{mode} {clear} {width}");
                assert!((inset - inside).abs() < 0.01, "{mode} {clear} {width}");
                assert!((gap - expected).abs() < 0.01, "{mode} {clear} {width}");
            }
        }
    }

    #[test]
    fn positive_collapsed_margin_past_float_does_not_introduce_clearance() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3];
        let mut section = node(3, Some(1), "section", "");
        section.children = vec![4];
        let mut input = column_fixture(
            "main {width:200px} aside {float:left;width:40px;height:80px} section {margin-top:10px} p {clear:both;height:10px;margin:100px 0 0}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            section,
            node(4, Some(3), "p", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(3).y - rect(1).y - 100.).abs() < 0.01);
        assert!((rect(4).y - rect(3).y).abs() < 0.01);
    }

    #[test]
    fn clearance_on_opposite_float_side_does_not_block_parent_margin_collapse() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3];
        let mut section = node(3, Some(1), "section", "");
        section.children = vec![4];
        let mut input = column_fixture(
            "main {width:200px} aside {float:left;width:40px;height:80px} section {margin-top:10px} p {clear:right;height:10px;margin:30px 0 0}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            section,
            node(4, Some(3), "p", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(3).y - rect(1).y - 30.).abs() < 0.01);
        assert!((rect(4).y - rect(3).y).abs() < 0.01);
    }

    #[test]
    fn cleared_child_without_float_interference_collapses_percentage_margin_at_used_width() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        let mut section = node(2, Some(1), "section", "");
        section.children = vec![3];
        let mut input = column_fixture(
            "main {width:300px} section {max-width:100px;margin-top:5px} p {clear:both;height:10px;margin:20% 0 0}",
            Vec::new(),
        );
        input.nodes = vec![root, section, node(3, Some(2), "p", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(2).y - rect(1).y - 20.).abs() < 0.01);
        assert!((rect(3).y - rect(2).y).abs() < 0.01);
    }

    #[test]
    fn padding_and_independent_formatting_context_stop_parent_margin_collapse() {
        for barrier in ["padding-top:1px", "display:flow-root"] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2];
            let mut section = node(2, Some(1), "section", "");
            section.children = vec![3];
            let mut input = column_fixture(
                &format!(
                    "main {{width:200px}} section {{margin-top:10px;{barrier}}} p {{height:20px;margin:50px 0 0}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![root, section, node(3, Some(2), "p", "")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            let padding = if barrier.starts_with("padding") {
                1.
            } else {
                0.
            };
            assert!((rect(3).y - rect(2).y - 50. - padding).abs() < 0.01);
        }
    }

    #[test]
    fn float_flow_empty_block_margin_chain_retains_positive_and_negative_struts() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4, 5];
        let mut input = column_fixture(
            "main {width:200px} aside {float:left;width:1px;height:1px} header {height:20px;margin-bottom:30px} div {margin-top:-20px;margin-bottom:10px} footer {height:10px;margin-top:25px}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            node(3, Some(1), "header", ""),
            node(4, Some(1), "div", ""),
            node(5, Some(1), "footer", ""),
        ];
        for floating in [true, false] {
            if !floating {
                input.nodes.retain(|node| node.id != 2);
                input.nodes[0].children.retain(|id| *id != 2);
            }
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            assert!(
                (rect(5).y - rect(3).bottom() - 10.).abs() < 0.01,
                "floating={floating}"
            );
        }
    }

    #[test]
    fn bfc_auto_margins_center_inside_remaining_float_space() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3];
        let mut input = column_fixture(
            "main {width:200px} aside {float:left;width:60px;height:80px} section {display:flow-root;width:80px;height:20px;margin-inline:auto}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            node(3, Some(1), "section", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let block = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!((block.x - float.right() - 30.).abs() < 0.01);
        assert!((block.width - 80.).abs() < 0.01);
    }

    #[test]
    fn clear_uses_shared_float_bottom_but_nested_bfc_floats_do_not_escape() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let mut inner = node(3, Some(1), "section", "");
        inner.children = vec![5];
        let mut input = column_fixture(
            "main {width:200px} aside {float:left;width:40px;height:50px} section {display:flow-root;width:100px;height:20px} section aside {height:100px} p {clear:both;margin:0;height:10px}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            inner,
            node(4, Some(1), "p", ""),
            node(5, Some(3), "aside", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(4).y - rect(2).bottom()).abs() < 0.01);
        assert!(rect(4).y < rect(5).bottom());
    }

    #[test]
    fn exported_glyph_source_ranges_are_native_fragment_local_clusters() {
        for mode in ["horizontal-tb", "vertical-rl"] {
            let mut input = column_fixture(
                &format!(
                    "main {{width:120px;height:120px;writing-mode:{mode};font:16px sans-serif}} span {{color:red}}"
                ),
                Vec::new(),
            );
            let mut root = node(1, None, "main", "");
            root.children = vec![2];
            root.content = Some(vec![
                LayoutContent::Text {
                    node_id: 10,
                    text: "A ".into(),
                },
                LayoutContent::Element(2),
            ]);
            input.nodes = vec![root, node(2, Some(1), "span", "Bé")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let fragments = output.text_fragments.values().flatten().collect::<Vec<_>>();
            assert!(!fragments.is_empty());
            let mut count = 0;
            for fragment in fragments {
                for glyph in fragment.glyph_runs.iter().flat_map(|run| &run.glyphs) {
                    if let Some(source) = &glyph.source {
                        assert!(
                            fragment.text.is_char_boundary(source.start)
                                && fragment.text.is_char_boundary(source.end)
                        );
                        assert!(source.start < source.end && source.end <= fragment.text.len());
                        assert!(
                            fragment
                                .clusters
                                .iter()
                                .any(|cluster| cluster.start <= source.start
                                    && cluster.end >= source.end)
                        );
                        count += 1;
                    }
                }
            }
            assert!(count >= 3, "{mode}");
        }
    }

    #[test]
    fn oversized_replaced_float_stays_whole_without_next_column_exclusion() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 10,
                text: ["A"; 10].join("\n"),
            },
        ]);
        let mut input = column_fixture(
            "main {width:200px;height:100px;columns:2;column-gap:0;column-fill:auto;white-space:pre;font:16px/20px sans-serif} img {float:left;width:30px;height:150px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "img", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let origin = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let fragments = &output.geometry[&2].client_rects;
        assert_eq!(fragments.len(), 1);
        assert!((fragments[0].height - 150.).abs() < 0.01);
        assert!((fragments[0].x - origin.x).abs() < 0.01);
        let clusters = &output.text_clusters[&10];
        assert!(
            clusters
                .iter()
                .any(|cluster| (cluster.rect.x - origin.x - 100.).abs() < 0.01
                    && cluster.rect.y < origin.y + 1.)
        );
        // Newlines retain a native caret after A, at A's right edge. Only
        // visible letters start at the float-free column's inline origin.
        let letters = clusters
            .iter()
            .filter(|cluster| cluster.start % 2 == 0 && cluster.end == cluster.start + 1)
            .collect::<Vec<_>>();
        assert_eq!(letters.len(), 10, "{clusters:?}");
        assert!(
            letters
                .iter()
                .filter(|cluster| cluster.rect.x >= origin.x + 100.)
                .all(|cluster| cluster.rect.x < origin.x + 101.),
            "{clusters:?}"
        );
        let covered = clusters
            .iter()
            .flat_map(|cluster| cluster.start..cluster.end)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            covered,
            (0..19).collect(),
            "newline source ranges must remain present: {clusters:?}"
        );
    }

    #[test]
    fn oversized_float_continues_exclusion_and_native_fragments_across_columns() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 10,
                text: ["A"; 10].join("\n"),
            },
        ]);
        let mut input = column_fixture(
            "main {width:200px;height:100px;columns:2;column-gap:0;column-fill:auto;white-space:pre;font:16px/20px sans-serif} aside {float:left;width:30px;height:150px;background:red}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let origin = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let fragments = &output.geometry[&2].client_rects;
        assert_eq!(fragments.len(), 2);
        assert!((fragments[0].height - 100.).abs() < 0.01);
        assert!((fragments[1].height - 50.).abs() < 0.01);
        assert!((fragments[1].x - fragments[0].x - 100.).abs() < 0.01);
        let clusters = &output.text_clusters[&10];
        assert!(
            clusters
                .iter()
                .filter(|cluster| cluster.rect.x < origin.x + 100.)
                .all(|cluster| cluster.rect.x >= origin.x + 30. - 0.01)
        );
        assert!(
            clusters
                .iter()
                .any(|cluster| cluster.rect.x >= origin.x + 130. - 0.01
                    && cluster.rect.y < origin.y + 50.)
        );
        assert!(
            clusters
                .iter()
                .any(|cluster| cluster.rect.x >= origin.x + 100. - 0.01
                    && cluster.rect.x < origin.x + 101.
                    && cluster.rect.y >= origin.y + 50.)
        );
    }

    #[test]
    fn float_column_sets_balance_to_independent_native_heights() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4, 5];
        let mut input = column_fixture(
            "main {width:200px;columns:2;column-gap:0;column-fill:balance} aside {float:left;width:10px;height:10px} section {height:40px} header {column-span:all;height:10px} footer {height:200px}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            node(3, Some(1), "section", ""),
            node(4, Some(1), "header", ""),
            node(5, Some(1), "footer", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(4).y - rect(1).y - 20.).abs() < 0.05);
        assert!((rect(4).width - 200.).abs() < 0.01);
        assert!((rect(1).height - 130.).abs() < 0.1);
        assert!(output.geometry[&3].client_rects.len() >= 2);
        assert!(output.geometry[&5].client_rects.len() >= 2);
    }

    #[test]
    fn spanning_block_ends_float_exclusions_before_next_column_set() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let mut first = node(2, Some(1), "section", "");
        first.children = vec![5];
        first.content = Some(vec![
            LayoutContent::Element(5),
            LayoutContent::Text {
                node_id: 10,
                text: "a".into(),
            },
        ]);
        let mut last = node(4, Some(1), "p", "");
        last.content = Some(vec![LayoutContent::Text {
            node_id: 11,
            text: "b\nc\nd".into(),
        }]);
        let mut input = column_fixture(
            "main {width:220px;height:60px;columns:2;column-gap:20px;column-fill:auto;font:16px/20px sans-serif;white-space:pre-wrap;orphans:1;widows:1} aside {float:left;width:40px;height:40px} h2 {column-span:all;height:20px;margin:0} p {margin:0}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            first,
            node(3, Some(1), "h2", ""),
            last,
            node(5, Some(2), "aside", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!(rect(3).y >= rect(5).bottom() - 0.01);
        assert!((rect(3).width - 220.).abs() < 0.01);
        let b = &output.text_clusters[&11][0];
        assert!((b.rect.x - rect(1).x).abs() < 0.01);
        assert!(single_font_flow_top(b.rect, 20.) >= rect(3).bottom() - 0.01);
    }

    #[test]
    fn balanced_multicol_float_reflows_against_candidate_column_heights() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a\nb\n".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "c\nd\ne\nf\ng\nh".into(),
            },
        ]);
        let mut input = column_fixture(
            "main {width:220px;columns:2;column-gap:20px;font:16px/20px sans-serif;white-space:pre-wrap;orphans:1;widows:1} aside {float:left;width:40px;height:60px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let root = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!(float.bottom() <= root.bottom() + 0.01);
        assert!(
            output
                .text_clusters
                .values()
                .flatten()
                .all(|cluster| cluster.rect.x < root.right() + 0.01)
        );
        assert!(
            output.text_clusters[&11]
                .iter()
                .all(|cluster| cluster.rect.right() <= float.x + 0.01
                    || cluster.rect.x >= float.right() - 0.01
                    || cluster.rect.bottom() <= float.y + 0.01
                    || cluster.rect.y >= float.bottom() - 0.01)
        );
    }

    #[test]
    fn fixed_multicol_float_moves_before_native_line_rebreaking() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a\nb\n".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "c\nd\ne\nf".into(),
            },
        ]);
        let mut input = column_fixture(
            "main {width:220px;height:60px;columns:2;column-gap:20px;column-fill:auto;font:16px/20px sans-serif;white-space:pre-wrap;orphans:1;widows:1} aside {float:left;width:40px;height:40px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let root = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!((float.x - root.x - 120.).abs() < 0.01);
        assert!((float.y - root.y).abs() < 0.01);
        assert!((float.height - 40.).abs() < 0.01);
        assert!(
            output.text_clusters[&11]
                .iter()
                .filter(|cluster| cluster.rect.x >= float.x
                    && single_font_flow_top(cluster.rect, 20.) < float.bottom())
                .all(|cluster| cluster.rect.x >= float.right() - 0.01)
        );
    }

    #[test]
    fn vertical_native_shaping_wraps_around_float_in_logical_line_bands() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2];
            root.content = Some(vec![
                LayoutContent::Element(2),
                LayoutContent::Text {
                    node_id: 10,
                    text: "abcdefghij ".repeat(12),
                },
            ]);
            let mut input = column_fixture(
                &format!(
                    "main {{writing-mode:{mode};height:100px;font:16px/20px sans-serif;overflow-wrap:anywhere}} aside {{float:left;width:40px;height:40px}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![root, node(2, Some(1), "aside", "")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            let clusters = &output.text_clusters[&10];
            let beside = clusters
                .iter()
                .filter(|cluster| {
                    cluster.rect.x < float.right() - 0.01 && cluster.rect.right() > float.x + 0.01
                })
                .collect::<Vec<_>>();
            assert!(!beside.is_empty(), "{mode}");
            assert!(
                beside.iter().all(|cluster| if mode == "sideways-lr" {
                    cluster.rect.bottom() <= float.y + 0.01
                } else {
                    cluster.rect.y >= float.bottom() - 0.01
                }),
                "{mode}"
            );
            assert!(
                output.text_fragments[&1]
                    .iter()
                    .flat_map(|fragment| &fragment.glyph_runs)
                    .all(|run| run.vertical)
            );
        }
    }

    #[test]
    fn vertical_block_sibling_margins_collapse_along_block_progression() {
        for mode in ["vertical-rl", "vertical-lr"] {
            let mut root = node(1, None, "main", "");
            root.children = vec![2, 3];
            let (after, before) = if mode == "vertical-rl" {
                ("left", "right")
            } else {
                ("right", "left")
            };
            let mut input = column_fixture(
                &format!(
                    "main {{writing-mode:{mode};height:200px}} header {{width:20px;margin-{after}:30px}} footer {{width:10px;margin-{before}:20px}}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                root,
                node(2, Some(1), "header", ""),
                node(3, Some(1), "footer", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
            let gap = if mode == "vertical-rl" {
                rect(2).x - rect(3).right()
            } else {
                rect(3).x - rect(2).right()
            };
            assert!((gap - 30.).abs() < 0.01, "{mode}: {gap}");
            assert!((rect(1).width - 60.).abs() < 0.01);
        }
    }

    #[test]
    fn vertical_float_only_bfc_auto_block_size_contains_float() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2];
        let mut input = column_fixture(
            "main {writing-mode:vertical-rl;display:flow-root;height:100px} aside {float:left;width:60px;height:30px}",
            Vec::new(),
        );
        input.nodes = vec![root, node(2, Some(1), "aside", "")];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let root = output.geometry[&1].box_model.as_ref().unwrap().content_box;
        let float = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!((root.width - 60.).abs() < 0.01);
        assert!((float.x - root.x).abs() < 0.01);
        assert!(!output.text_fragments.contains_key(&1));
    }

    #[test]
    fn float_only_formatting_context_contains_floats_without_inline_advance() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        root.content = Some(vec![LayoutContent::Element(2), LayoutContent::Element(3)]);
        let mut input = column_fixture(
            "div {display:flow-root;width:100px} aside {float:left;width:30px;height:40px} aside:last-child {float:right;height:60px}",
            Vec::new(),
        );
        input.nodes = vec![
            root,
            node(2, Some(1), "aside", ""),
            node(3, Some(1), "aside", ""),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.geometry[&id].box_model.as_ref().unwrap().border_box;
        assert!((rect(3).x - rect(2).x - 70.).abs() < 0.01);
        assert_eq!(rect(1).height, 60.);
        assert!(!output.text_fragments.contains_key(&1));
    }

    #[test]
    fn native_inline_boxes_share_line_breaking_without_synthetic_text_characters() {
        let mut engine = TextEngine::new(&[]);
        engine
            .inline_sizes
            .insert(7, (30., 20., parley::InlineBoxKind::InFlow));
        let mut source = text_source::TextSource::synthetic("a ");
        source.append_box(7);
        source.append(0, None, " b");
        let style = BTreeMap::from([("font-size".into(), "16px".into())]);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert_eq!(shaped.inline_boxes.len(), 1);
        assert_eq!(shaped.inline_boxes[0].node, 7);
        assert_eq!(shaped.inline_boxes[0].rect.width, 30.);
        assert_eq!(shaped.lines.len(), 1);
        assert_eq!(shaped.lines[0].0, "a  b");
        assert!(!shaped.lines[0].0.contains('\u{fffc}'));
        let mut only = text_source::TextSource::default();
        only.append_box(7);
        let shaped = engine.shape_source(
            &only,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert_eq!(shaped.inline_boxes.len(), 1);
        assert!(shaped.height >= 20.);
        let mut vertical = style.clone();
        vertical.insert("writing-mode".into(), "vertical-rl".into());
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &vertical,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert_eq!(shaped.inline_boxes.len(), 1);
    }

    #[test]
    fn ruby_annotation_does_not_inherit_base_decoration_paint() {
        let input = column_fixture(
            "div {text-decoration:underline red} rt {display:ruby-text;text-decoration:none}",
            vec![node(2, Some(1), "rt", "annotation")],
        );
        let styles = stylo_computed_values(&input).unwrap();
        let propagated = text_decoration::propagate(&input, &styles);
        assert!(!text_decoration::read(&propagated[&1], 16., input.viewport).is_empty());
        assert!(text_decoration::read(&propagated[&2], 8., input.viewport).is_empty());
    }

    #[test]
    fn native_decoration_runs_retain_nested_owner_lines_and_atomic_boundary() {
        // css/css-text-decor/text-decoration-style-multiple.html
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Element(3),
        ]);
        let mut child = node(2, Some(1), "span", "b");
        child.content = Some(vec![LayoutContent::Text {
            node_id: 11,
            text: "b".into(),
        }]);
        let atomic = node(3, Some(1), "span", "c");
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![root,child,atomic],stylesheets:vec!["div {text-decoration: underline solid coral} span {text-decoration: overline dashed skyblue} span:last-child {display:inline-block}".into()],
            element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let styles = stylo_computed_values(&input).unwrap();
        let decorated = text_decoration::propagate(&input, &styles);
        let a = text_decoration::read(&decorated[&1], 16., input.viewport);
        let b = text_decoration::read(&decorated[&2], 16., input.viewport);
        let c = text_decoration::read(&decorated[&3], 16., input.viewport);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 2);
        assert_eq!(c.len(), 1);
        assert_eq!(b[0], a[0]);
        let mut engine = TextEngine::new(&[]);
        engine.decoration_styles = decorated;
        let nodes = input.nodes.iter().map(|node| (node.id, node)).collect();
        let source =
            text_source::TextSource::collect_visible(&input.nodes[0], &nodes, true, Some(&styles));
        let shaped = engine.shape_source(&source, 500., 16., &styles[&1], input.viewport, &styles);
        assert!(
            shaped
                .glyph_runs
                .iter()
                .flatten()
                .any(|run| run.decorations.len() == 2)
        );
        let mut vertical = styles[&2].as_ref().clone();
        vertical.insert("writing-mode".into(), "vertical-rl".into());
        let shaped = engine.shape_styled("漢", 100., 16., &vertical, input.viewport);
        assert!(
            shaped
                .glyph_runs
                .iter()
                .flatten()
                .all(|run| run.vertical && !run.decorations.is_empty())
        );
    }

    #[test]
    fn paragraph_style_ranges_preserve_size_color_and_shared_baseline() {
        use text_source::{TextSource, TextSpan};
        let source = TextSource {
            geometry_aliases: Vec::new(),
            text: "AA".into(),
            spans: vec![
                TextSpan {
                    atomic: false,
                    utf16_start: 0,
                    transformed: false,
                    owner: 1,
                    text_node: Some(10),
                    bytes: 0..1,
                    utf16_len: 1,
                },
                TextSpan {
                    atomic: false,
                    utf16_start: 0,
                    transformed: false,
                    owner: 2,
                    text_node: Some(11),
                    bytes: 1..2,
                    utf16_len: 1,
                },
            ],
        };
        let first = BTreeMap::from([
            ("font-size".into(), "16px".into()),
            ("color".into(), "red".into()),
        ]);
        let second = BTreeMap::from([
            ("font-size".into(), "32px".into()),
            ("color".into(), "blue".into()),
        ]);
        let styles = BTreeMap::from([(1, Arc::new(first.clone())), (2, Arc::new(second))]);
        let mut engine = TextEngine::new(&[]);
        let shaped = engine.shape_source(
            &source,
            500.,
            16.,
            &first,
            ViewportState::default(),
            &styles,
        );
        assert_eq!(shaped.lines.len(), 1);
        let runs = &shaped.glyph_runs[0];
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].font_size, 16.);
        assert_eq!(runs[1].font_size, 32.);
        assert_eq!(runs[0].color, Some([255, 0, 0, 255]));
        assert_eq!(runs[1].color, Some([0, 0, 255, 255]));
        assert_eq!(runs[0].glyphs[0].y, runs[1].glyphs[0].y);
        assert_eq!(shaped.source_clusters.len(), 2);
        assert_eq!(shaped.source_clusters[0].text_node, 10);
        assert_eq!(shaped.source_clusters[1].text_node, 11);
        assert_eq!(shaped.source_clusters[0].utf16, 0..1);
        assert_eq!(shaped.source_clusters[1].utf16, 0..1);
        assert!(shaped.source_clusters[1].rect.x > shaped.source_clusters[0].rect.x);
        assert!(runs[1].glyphs[0].x > runs[0].glyphs[0].x);
        assert_eq!(engine.paint_fonts.len(), 1);
    }

    #[test]
    fn css_wrapping_preserves_hard_breaks_and_controls_soft_breaks() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, text: &str, properties: &[(&str, &str)]| {
            engine.shape_styled(
                text,
                30.,
                16.,
                &properties
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
                ViewportState::default(),
            )
        };
        let wrapped = shape(&mut engine, "one two three", &[]);
        let nowrap = shape(
            &mut engine,
            "one two three",
            &[("text-wrap-mode", "nowrap")],
        );
        assert!(wrapped.lines.len() > 1);
        assert_eq!(nowrap.lines.len(), 1);
        assert!(nowrap.width > 30.);
        let hard = shape(
            &mut engine,
            "one\ntwo",
            &[
                ("text-wrap-mode", "nowrap"),
                ("white-space-collapse", "preserve-breaks"),
            ],
        );
        assert_eq!(hard.lines.len(), 2);
        let normal = shape(&mut engine, "abcdefghijk", &[]);
        let anywhere = shape(&mut engine, "abcdefghijk", &[("overflow-wrap", "anywhere")]);
        let break_all = shape(&mut engine, "abcdefghijk", &[("word-break", "break-all")]);
        let legacy = shape(
            &mut engine,
            "abcdefghijk",
            &[("word-break", "break-word"), ("overflow-wrap", "normal")],
        );
        assert_eq!(normal.lines.len(), 1);
        assert!(anywhere.lines.len() > 1);
        assert!(break_all.lines.len() > 1);
        assert_eq!(legacy.lines, anywhere.lines);
        let forced_nowrap = shape(
            &mut engine,
            "abcdefghijk",
            &[("overflow-wrap", "anywhere"), ("text-wrap-mode", "nowrap")],
        );
        assert_eq!(forced_nowrap.lines.len(), 1);
    }

    #[test]
    fn whitespace_only_line_boxes_follow_post_transform_policy() {
        let mut engine = TextEngine::new(&[]);
        for (text, whitespace, transform, has_line) in [
            ("", "pre", "none", false),
            (" \t\r\n ", "normal", "none", false),
            (" \t ", "nowrap", "none", false),
            ("   ", "pre-line", "none", false),
            ("\n", "pre-line", "none", true),
            (" ", "pre", "none", true),
            (" ", "pre-wrap", "none", true),
            (" ", "break-spaces", "none", true),
            (" ", "normal", "full-width", true),
            ("\u{a0}", "normal", "none", true),
        ] {
            let style = BTreeMap::from([
                ("white-space".into(), whitespace.into()),
                ("text-transform".into(), transform.into()),
            ]);
            let shaped = engine.shape_styled(text, 200., 16., &style, ViewportState::default());
            assert_eq!(
                !shaped.lines.is_empty(),
                has_line,
                "{text:?} {whitespace} {transform}"
            );
            assert_eq!(shaped.height > 0., has_line);
            if !has_line {
                assert!(shaped.clusters.is_empty());
                assert!(shaped.glyph_runs.is_empty());
                assert_eq!(shaped.width, 0.);
            }
        }
    }

    #[test]
    fn whitespace_line_suppression_uses_text_owner_style() {
        let mut engine = TextEngine::new(&[]);
        let mut source = text_source::TextSource::synthetic(" ");
        source.spans[0].owner = 7;
        source.spans[0].text_node = Some(70);
        for (parent_mode, owner_mode, has_line) in
            [("normal", "pre", true), ("pre", "normal", false)]
        {
            let parent = BTreeMap::from([("white-space".into(), parent_mode.into())]);
            let styles = BTreeMap::from([(
                7,
                Arc::new(BTreeMap::from([("white-space".into(), owner_mode.into())])),
            )]);
            let shaped = engine.shape_source(
                &source,
                200.,
                16.,
                &parent,
                ViewportState::default(),
                &styles,
            );
            assert_eq!(!shaped.lines.is_empty(), has_line);
            assert_eq!(shaped.height > 0., has_line);
            if has_line {
                assert!(
                    shaped
                        .source_clusters
                        .iter()
                        .any(|cluster| cluster.text_node == 70)
                );
            }
        }
    }

    #[test]
    fn break_spaces_wraps_each_preserved_space_without_hanging_or_trimming() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([("white-space".into(), "break-spaces".into())]);
        let one = engine.shape_styled(" ", 200., 16., &style, ViewportState::default());
        let space = one.clusters[0].rect.width;
        let wrapped =
            engine.shape_styled("   ", space * 1.1, 16., &style, ViewportState::default());
        assert_eq!(wrapped.lines.len(), 3);
        assert_eq!(wrapped.clusters.len(), 3);
        for (index, cluster) in wrapped.clusters.iter().enumerate() {
            assert_eq!(cluster.line, index);
            assert!((cluster.rect.width - space).abs() < 0.01);
            assert!(cluster.rect.x.abs() < 0.01);
        }
        assert!(
            wrapped
                .lines
                .iter()
                .all(|line| (line.1 - space).abs() < 0.01)
        );
        let mut nowrap = style.clone();
        nowrap.insert("text-wrap-mode".into(), "nowrap".into());
        let unwrapped =
            engine.shape_styled("   ", space * 1.1, 16., &nowrap, ViewportState::default());
        assert_eq!(unwrapped.lines.len(), 1);
        assert!((unwrapped.width - 3. * space).abs() < 0.01);
        let forced = engine.shape_styled(" \n ", 200., 16., &style, ViewportState::default());
        assert_eq!(forced.lines.len(), 2);
    }

    #[test]
    fn break_spaces_wraps_tabs_and_preserves_the_space_after_a_word() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "break-spaces".into()),
            ("tab-size".into(), "40px".into()),
        ]);
        let tabs = engine.shape_styled("\t\t", 45., 16., &style, ViewportState::default());
        assert_eq!(tabs.lines.len(), 2);
        assert_eq!(tabs.clusters.len(), 2);
        assert!(
            tabs.clusters
                .iter()
                .all(|cluster| (cluster.rect.width - 40.).abs() < 0.01
                    && cluster.rect.x.abs() < 0.01)
        );
        let word = engine.shape_styled("aa", 200., 16., &style, ViewportState::default());
        let spaced = engine.shape_styled(
            "aa a",
            word.width + 0.1,
            16.,
            &style,
            ViewportState::default(),
        );
        assert_eq!(spaced.lines.len(), 2);
        let space = spaced
            .clusters
            .iter()
            .find(|cluster| cluster.bytes == (2..3))
            .unwrap();
        assert_eq!(space.line, 0);
        assert!(space.rect.width > 0.);
        assert!(spaced.lines[0].1 > word.width + 0.1);
    }

    #[test]
    fn line_edges_distinguish_collapsed_preserved_and_hanging_spaces() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, text: &str, whitespace: &str| {
            engine.shape_styled(
                text,
                200.,
                16.,
                &BTreeMap::from([("white-space".into(), whitespace.into())]),
                ViewportState::default(),
            )
        };
        let letter = shape(&mut engine, "a", "normal");
        let collapsed = shape(&mut engine, " a ", "normal");
        assert!((collapsed.width - letter.width).abs() < 0.01);
        assert_eq!(collapsed.clusters[0].rect.width, 0.);
        assert_eq!(collapsed.clusters.last().unwrap().rect.width, 0.);
        assert!(
            collapsed
                .clusters
                .iter()
                .find(|cluster| cluster.bytes == (1..2))
                .unwrap()
                .rect
                .x
                .abs()
                < 0.01
        );
        let preserved = shape(&mut engine, " a ", "pre");
        assert!(preserved.width > letter.width);
        assert!(preserved.clusters[0].rect.width > 0.);
        assert!((preserved.clusters.last().unwrap().rect.right() - preserved.width).abs() < 0.01);
        let hanging = shape(&mut engine, "a  ", "pre-wrap");
        assert!(hanging.width > letter.width);
        assert!((hanging.clusters.last().unwrap().rect.right() - hanging.width).abs() < 0.01);
        let break_spaces = shape(&mut engine, "a  ", "break-spaces");
        assert!((break_spaces.width - hanging.width).abs() < 0.01);
        assert!(
            (break_spaces.clusters.last().unwrap().rect.right() - break_spaces.width).abs() < 0.01
        );
        let nbsp = shape(&mut engine, "a\u{a0}", "normal");
        assert!(nbsp.width > letter.width);
    }

    #[test]
    fn intrinsic_text_indent_keeps_absolute_terms_and_ignores_percentages() {
        let mut engine = TextEngine::new(&[]);
        let source = text_source::TextSource::synthetic("aa aa");
        let mut measure = |indent: &str, source: &text_source::TextSource| {
            let style = BTreeMap::from([("text-indent".into(), indent.into())]);
            engine.intrinsic_source(
                source,
                16.,
                &style,
                ViewportState::default(),
                &BTreeMap::new(),
            )
        };
        let plain = measure("0px", &source);
        let indented = measure("50px", &source);
        assert!((indented.0 - plain.0 - 50.).abs() < 0.02);
        assert!((indented.1 - plain.1 - 50.).abs() < 0.02);
        for value in ["100%", "calc(100% + 0px)"] {
            let widths = measure(value, &source);
            assert!((widths.0 - plain.0).abs() < 0.02);
            assert!((widths.1 - plain.1).abs() < 0.02);
        }
        let calc = measure("calc(100% + 50px)", &source);
        assert!((calc.0 - indented.0).abs() < 0.02);
        assert!((calc.1 - indented.1).abs() < 0.02);
        assert_eq!(
            measure("50px", &text_source::TextSource::synthetic("  ")),
            (0., 0.)
        );
    }

    #[test]
    fn intrinsic_each_line_indent_applies_after_preserved_breaks() {
        let mut engine = TextEngine::new(&[]);
        let source = text_source::TextSource::synthetic("a\nmmmm");
        let measure = |engine: &mut TextEngine, indent: &str| {
            engine.intrinsic_source(
                &source,
                16.,
                &BTreeMap::from([
                    ("white-space".into(), "pre".into()),
                    ("text-indent".into(), indent.into()),
                ]),
                ViewportState::default(),
                &BTreeMap::new(),
            )
        };
        let plain = measure(&mut engine, "0px");
        let first = measure(&mut engine, "10px");
        let each = measure(&mut engine, "10px each-line");
        assert!((first.1 - plain.1).abs() < 0.02);
        assert!((each.0 - plain.0 - 10.).abs() < 0.02);
        assert!((each.1 - plain.1 - 10.).abs() < 0.02);
    }

    #[test]
    fn intrinsic_measurements_share_preserved_space_and_tab_rules() {
        let mut engine = TextEngine::new(&[]);
        let mut measure = |text: &str, mode: &str, overflow: &str| {
            let style = BTreeMap::from([
                ("white-space".into(), mode.into()),
                ("overflow-wrap".into(), overflow.into()),
                ("tab-size".into(), "40px".into()),
            ]);
            let mut builder = engine
                .layout
                .ranged_builder(&mut engine.fonts, text, 1., true);
            for property in text_style_properties(&style, 16., ViewportState::default()) {
                builder.push_default(property);
            }
            let mut layout: Layout<TextBrush> = builder.build(text);
            layout.break_all_lines(Some(200.));
            layout.align(Alignment::Right, AlignmentOptions::default());
            let before = (
                layout.width(),
                layout.height(),
                layout
                    .lines()
                    .map(|line| line.metrics().offset)
                    .collect::<Vec<_>>(),
            );
            let sizes = layout.calculate_content_widths();
            assert_eq!(
                before,
                (
                    layout.width(),
                    layout.height(),
                    layout
                        .lines()
                        .map(|line| line.metrics().offset)
                        .collect::<Vec<_>>()
                )
            );
            sizes
        };
        let letter = measure("a", "normal", "normal");
        let hung = measure("a   ", "pre-wrap", "normal");
        assert!((hung.min - letter.min).abs() < 0.02);
        assert!((hung.max - letter.max).abs() < 0.02);
        let preserved = measure("a   ", "pre", "normal");
        assert!(preserved.min > letter.min);
        assert!((preserved.min - preserved.max).abs() < 0.02);
        let tabs = measure("a\tb", "pre", "normal");
        assert!(tabs.min > 40.);
        assert!((tabs.min - tabs.max).abs() < 0.02);
        let nbsp = measure("a\u{a0}b", "normal", "normal");
        assert!((nbsp.min - nbsp.max).abs() < 0.02);
        let word = measure("abcdef", "normal", "normal");
        let break_word = measure("abcdef", "normal", "break-word");
        let anywhere = measure("abcdef", "normal", "anywhere");
        assert!((word.min - break_word.min).abs() < 0.02);
        assert!(anywhere.min < word.min);
        assert!((anywhere.max - word.max).abs() < 0.02);
    }

    #[test]
    fn unicode_separator_hanging_preserves_glyph_and_source_advances() {
        let mut engine = TextEngine::new(&[]);
        for separator in [
            '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
            '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{202f}', '\u{205f}', '\u{3000}',
        ] {
            for mode in ["normal", "nowrap", "pre-line"] {
                let style = BTreeMap::from([("white-space".into(), mode.into())]);
                let plain = engine.shape_styled("a", 200., 16., &style, ViewportState::default());
                let text = format!("a{separator} {separator} ");
                let shaped =
                    engine.shape_styled(&text, 200., 16., &style, ViewportState::default());
                assert!(
                    (shaped.width - plain.width).abs() < 0.02,
                    "{separator:?} {mode}"
                );
                let between = 1 + separator.len_utf8();
                assert!(
                    shaped
                        .clusters
                        .iter()
                        .find(|cluster| cluster.bytes == (between..between + 1))
                        .unwrap()
                        .rect
                        .width
                        > 0.
                );
                assert_eq!(shaped.clusters.last().unwrap().rect.width, 0.);
                assert!(
                    shaped
                        .clusters
                        .iter()
                        .any(|cluster| cluster.rect.right() > shaped.width)
                );
            }
            let style = BTreeMap::from([("white-space".into(), "pre".into())]);
            let text = format!("a{separator}");
            let shaped = engine.shape_styled(&text, 200., 16., &style, ViewportState::default());
            assert!((shaped.clusters.last().unwrap().rect.right() - shaped.width).abs() < 0.02);
        }
    }

    #[test]
    fn ideographic_space_sequences_hang_together_at_soft_wraps() {
        let mut engine = TextEngine::new(&[]);
        for mode in ["normal", "pre-wrap", "pre-line"] {
            let style = BTreeMap::from([("white-space".into(), mode.into())]);
            let width = engine
                .shape_styled("aa", 200., 16., &style, ViewportState::default())
                .width
                + 0.1;
            let shaped = engine.shape_styled(
                "aa\u{3000} \u{3000}bb",
                width,
                16.,
                &style,
                ViewportState::default(),
            );
            assert_eq!(shaped.lines.len(), 2, "{mode}");
            assert!(
                shaped
                    .clusters
                    .iter()
                    .filter(|cluster| cluster.bytes.start < 9)
                    .all(|cluster| cluster.line == 0)
            );
            assert_eq!(shaped.clusters.last().unwrap().line, 1);
        }
    }

    #[test]
    fn overflowing_pre_wrap_sequences_stay_on_their_preceding_line() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre-wrap".into()),
            ("tab-size".into(), "40px".into()),
        ]);
        let width = engine
            .shape_styled("aa", 200., 16., &style, ViewportState::default())
            .width
            + 0.1;
        for text in [
            "aa     ",
            "aa\t\t",
            "aa     \n",
            "aa\t\t\n",
            "aa     b",
            "aa\t\tb",
        ] {
            let shaped = engine.shape_styled(text, width, 16., &style, ViewportState::default());
            assert_eq!(
                shaped.lines.len(),
                if text.ends_with('b') { 2 } else { 1 },
                "{text:?}"
            );
            for cluster in &shaped.clusters {
                if text[cluster.bytes.clone()]
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t'))
                {
                    assert_eq!(cluster.line, 0, "{text:?}");
                    assert!(cluster.rect.width > 0.);
                }
            }
            if text.ends_with('b') {
                assert_eq!(shaped.clusters.last().unwrap().line, 1);
            }
        }
    }

    #[test]
    fn overflowing_nbsp_does_not_create_a_wrap_opportunity() {
        let mut engine = TextEngine::new(&[]);
        for whitespace in ["normal", "pre-wrap", "break-spaces"] {
            let style = BTreeMap::from([("white-space".into(), whitespace.into())]);
            let shaped = engine.shape_styled("a\u{a0}b", 1., 16., &style, ViewportState::default());
            assert_eq!(shaped.lines.len(), 1, "{whitespace}");
            assert!(shaped.width > 1.);
            assert!(shaped.clusters.iter().all(|cluster| cluster.line == 0));
        }
    }

    #[test]
    fn pre_wrap_final_and_forced_break_spaces_use_available_alignment_width() {
        let mut engine = TextEngine::new(&[]);
        let preserved = BTreeMap::from([("white-space".into(), "pre".into())]);
        let natural = engine.shape_styled("a  ", 200., 16., &preserved, ViewportState::default());
        let glyph_width = engine
            .shape_styled("a", 200., 16., &preserved, ViewportState::default())
            .width;
        for text in ["a  ", "a  \n"] {
            for align in ["left", "right", "center"] {
                let style = BTreeMap::from([
                    ("white-space".into(), "pre-wrap".into()),
                    ("text-align".into(), align.into()),
                ]);
                for width in [200., (natural.width + glyph_width) * 0.5] {
                    let shaped =
                        engine.shape_styled(text, width, 16., &style, ViewportState::default());
                    assert_eq!(shaped.lines.len(), 1);
                    let used = natural.width.min(width);
                    let expected_x = match align {
                        "right" => width - used,
                        "center" => (width - used) * 0.5,
                        _ => 0.,
                    };
                    let first = shaped
                        .clusters
                        .iter()
                        .find(|cluster| cluster.bytes == (0..1))
                        .unwrap();
                    assert!(
                        (first.rect.x - expected_x).abs() < 0.02,
                        "{text:?} {align} {width}"
                    );
                    // Hanging preserves the actual advances and source boxes.
                    let last = shaped
                        .clusters
                        .iter()
                        .find(|cluster| cluster.bytes == (2..3))
                        .unwrap();
                    assert!((last.rect.right() - first.rect.x - natural.width).abs() < 0.02);
                }
            }
        }
    }

    #[test]
    fn rtl_trailing_collapsed_space_is_removed_at_the_visual_start_edge() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([("direction".into(), "rtl".into())]);
        let plain = engine.shape_styled("אב", 200., 16., &style, ViewportState::default());
        let padded = engine.shape_styled("אב ", 200., 16., &style, ViewportState::default());
        assert!((plain.width - padded.width).abs() < 0.01);
        assert_eq!(
            padded
                .clusters
                .iter()
                .find(|cluster| cluster.bytes == (4..5))
                .unwrap()
                .rect
                .width,
            0.
        );
    }

    #[test]
    fn trailing_hanging_spaces_cross_font_runs_and_collapsed_spaces_do_not_justify() {
        let mut engine = TextEngine::new(&[]);
        let small = BTreeMap::from([
            ("white-space".into(), "pre-wrap".into()),
            ("font-size".into(), "16px".into()),
        ]);
        let large = BTreeMap::from([
            ("white-space".into(), "pre-wrap".into()),
            ("font-size".into(), "32px".into()),
        ]);
        let letter = engine.shape_styled("a", 200., 16., &small, ViewportState::default());
        let ranged = engine.shape_ranged(
            "a  ",
            200.,
            16.,
            &small,
            ViewportState::default(),
            &[(0..2, &small, None, 0), (2..3, &large, None, 0)],
        );
        assert!(ranged.width > letter.width);
        assert!((ranged.clusters.last().unwrap().rect.right() - ranged.width).abs() < 0.01);
        let justified = engine.shape_styled(
            " a a ",
            200.,
            16.,
            &BTreeMap::from([("text-align-last".into(), "justify".into())]),
            ViewportState::default(),
        );
        assert!((justified.width - 200.).abs() < 0.01);
        assert_eq!(justified.clusters[0].rect.width, 0.);
        assert_eq!(justified.clusters.last().unwrap().rect.width, 0.);
        let inner = justified
            .clusters
            .iter()
            .find(|cluster| cluster.bytes == (2..3))
            .unwrap();
        assert!(inner.rect.width > 100.);
    }

    #[test]
    fn textarea_ua_wrap_preserves_value_offsets_and_off_disables_soft_wraps() {
        let mut control = node(1, None, "textarea", "longwordlongword\nsecond");
        control.content = Some(vec![LayoutContent::Text {
            node_id: 1,
            text: control.text.clone(),
        }]);
        let mut engine = TextEngine::new(&[]);
        let source = text_source::TextSource::direct(&control);
        let wrapped = engine.shape_source(
            &source,
            35.,
            16.,
            &ua_style(&control),
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert!(wrapped.lines.len() > 2);
        assert!(
            wrapped
                .source_clusters
                .iter()
                .all(|cluster| cluster.text_node == 1)
        );
        assert_eq!(
            wrapped
                .source_clusters
                .iter()
                .map(|cluster| cluster.utf16.end)
                .max(),
            Some(control.text.encode_utf16().count())
        );
        control.attributes.insert("wrap".into(), "OFF".into());
        let unwrapped = engine.shape_source(
            &source,
            35.,
            16.,
            &ua_style(&control),
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert_eq!(unwrapped.lines.len(), 2);
    }

    #[test]
    fn preserved_tabs_share_dynamic_advances_with_glyphs_and_native_geometry() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre".into()),
            ("tab-size".into(), "40px".into()),
        ]);
        let mut node = node(1, None, "span", "a\tb");
        node.content = Some(vec![LayoutContent::Text {
            node_id: 10,
            text: node.text.clone(),
        }]);
        let source = text_source::TextSource::direct(&node);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        let tab = shaped
            .clusters
            .iter()
            .find(|cluster| cluster.bytes == (1..2))
            .unwrap();
        let b = shaped
            .clusters
            .iter()
            .find(|cluster| cluster.bytes == (2..3))
            .unwrap();
        assert!((tab.rect.right() - 40.).abs() < 0.01);
        assert!((b.rect.x - 40.).abs() < 0.01);
        let native = shaped
            .source_clusters
            .iter()
            .find(|cluster| cluster.utf16 == (1..2))
            .unwrap();
        assert_eq!(native.rect, tab.rect);
        let last = shaped.glyph_runs[0]
            .iter()
            .flat_map(|run| &run.glyphs)
            .last()
            .unwrap();
        assert!((last.x - 40.).abs() < 0.01);
        let mut indented_style = style.clone();
        indented_style.insert("text-indent".into(), "10px".into());
        let indented =
            engine.shape_styled("a\tb", 200., 16., &indented_style, ViewportState::default());
        assert!(
            (indented
                .clusters
                .iter()
                .find(|cluster| cluster.bytes == (2..3))
                .unwrap()
                .rect
                .x
                - 40.)
                .abs()
                < 0.01
        );
        assert_eq!(node.text, "a\tb");
    }

    #[test]
    fn tab_sizes_resolve_spaces_zero_and_new_line_origins() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([("white-space".into(), "pre".into())]);
        let space = engine.shape_styled(" ", 200., 16., &style, ViewportState::default());
        let space_width = space.clusters[0].rect.width;
        let mut four = style.clone();
        four.insert("tab-size".into(), "4".into());
        let numbered = engine.shape_styled("\tb", 200., 16., &four, ViewportState::default());
        assert!(
            (numbered
                .clusters
                .iter()
                .find(|cluster| cluster.bytes == (1..2))
                .unwrap()
                .rect
                .x
                - 4. * space_width)
                .abs()
                < 0.05
        );
        let mut zero = style.clone();
        zero.insert("tab-size".into(), "0".into());
        let removed = engine.shape_styled("a\tb", 200., 16., &zero, ViewportState::default());
        assert_eq!(
            removed
                .clusters
                .iter()
                .find(|cluster| cluster.bytes == (1..2))
                .unwrap()
                .rect
                .width,
            0.
        );
        let mut fixed = style.clone();
        fixed.insert("tab-size".into(), "40px".into());
        let lines = engine.shape_styled("a\tb\nx\tb", 200., 16., &fixed, ViewportState::default());
        for byte in [2, 6] {
            let b = lines
                .clusters
                .iter()
                .find(|cluster| cluster.bytes.contains(&byte))
                .unwrap();
            assert!((b.rect.x - 40.).abs() < 0.01);
        }
        let collapsed = engine.shape_styled(
            "a\tb",
            200.,
            16.,
            &BTreeMap::new(),
            ViewportState::default(),
        );
        let reference =
            engine.shape_styled("a b", 200., 16., &BTreeMap::new(), ViewportState::default());
        assert_eq!(collapsed.glyph_runs, reference.glyph_runs);
    }

    #[test]
    fn text_align_last_controls_final_and_forced_lines() {
        let mut engine = TextEngine::new(&[]);
        let shape =
            |engine: &mut TextEngine, text: &str, align: &str, last: &str, direction: &str| {
                engine.shape_styled(
                    text,
                    100.,
                    16.,
                    &BTreeMap::from([
                        ("text-align".into(), align.into()),
                        ("text-align-last".into(), last.into()),
                        ("direction".into(), direction.into()),
                        ("white-space".into(), "pre-wrap".into()),
                    ]),
                    ViewportState::default(),
                )
            };
        let centered = shape(&mut engine, "a a", "left", "center", "ltr");
        assert!((centered.lines[0].4 - (100. - centered.lines[0].1) / 2.).abs() < 0.01);
        let justified = shape(&mut engine, "a a", "left", "justify", "ltr");
        assert!((justified.lines[0].1 - 100.).abs() < 0.01);
        assert!((justified.width - 100.).abs() < 0.01);
        let end = justified
            .clusters
            .iter()
            .map(|cluster| cluster.rect.right())
            .fold(0., f64::max);
        assert!((end - 100.).abs() < 0.01);
        let auto = shape(&mut engine, "a a", "justify", "auto", "ltr");
        assert!(auto.lines[0].1 < 100. && auto.lines[0].4.abs() < 0.01);
        let forced = shape(&mut engine, "a a\nb b", "left", "right", "ltr");
        assert_eq!(forced.lines.len(), 2);
        assert!(
            forced
                .lines
                .iter()
                .all(|line| (line.4 + line.1 - 100.).abs() < 0.01)
        );
        let rtl = shape(&mut engine, "a a", "left", "start", "rtl");
        assert!((rtl.lines[0].4 + rtl.lines[0].1 - 100.).abs() < 0.01);
        let wrapped = shape(&mut engine, "aaaa aaaa aaaa aaaa", "left", "center", "ltr");
        assert!(wrapped.lines.len() > 1);
        assert!(wrapped.lines[0].4.abs() < 0.01);
        assert!(wrapped.lines.last().unwrap().4 > 0.);
    }

    #[test]
    fn justified_run_extents_match_clusters_and_hit_testing() {
        let mut engine = TextEngine::new(&[]);
        let text = "a b";
        let mut builder = engine
            .layout
            .ranged_builder(&mut engine.fonts, text, 1., true);
        for property in text_style_properties(&BTreeMap::new(), 16., ViewportState::default()) {
            builder.push_default(property);
        }
        // Force two shaping runs so hit testing must cross a run boundary.
        builder.push(StyleProperty::FontSize(24.), 2..3);
        let mut layout: Layout<TextBrush> = builder.build(text);
        layout.break_all_lines(Some(200.));
        let original_width = layout.width();
        layout.align(
            Alignment::Left,
            AlignmentOptions {
                last_line: Some(Alignment::Justify),
                align_when_overflowing: true,
            },
        );
        let line = layout.lines().next().unwrap();
        let y = (line.metrics().block_min_coord + line.metrics().block_max_coord) / 2.;
        let mut run_count = 0;
        let mut total = 0.;
        for run in line.runs() {
            run_count += 1;
            let cluster_width: f32 = run.visual_clusters().map(|cluster| cluster.advance()).sum();
            assert!((run.advance() - cluster_width).abs() < 0.001);
            total += run.advance();
        }
        assert!(run_count >= 2);
        assert!((total - 200.).abs() < 0.01);
        let (space, _) = parley::Cluster::from_point_exact(&layout, 100., y).unwrap();
        assert_eq!(space.text_range(), 1..2);
        let (last, _) = parley::Cluster::from_point_exact(&layout, 199., y).unwrap();
        assert_eq!(last.text_range(), 2..3);
        layout.align(Alignment::Left, AlignmentOptions::default());
        assert!((layout.width() - original_width).abs() < 0.01);
        assert!(parley::Cluster::from_point_exact(&layout, 100., y).is_none());
        for line in layout.lines() {
            for run in line.runs() {
                let cluster_width: f32 =
                    run.visual_clusters().map(|cluster| cluster.advance()).sum();
                assert!((run.advance() - cluster_width).abs() < 0.001);
            }
        }
    }

    #[test]
    fn last_line_justification_is_undone_before_realigning_or_rebreaking() {
        let mut engine = TextEngine::new(&[]);
        let text = "a a";
        let mut builder = engine
            .layout
            .ranged_builder(&mut engine.fonts, text, 1., true);
        for property in text_style_properties(&BTreeMap::new(), 16., ViewportState::default()) {
            builder.push_default(property);
        }
        let mut layout: Layout<TextBrush> = builder.build(text);
        layout.break_all_lines(Some(100.));
        layout.align(Alignment::Left, AlignmentOptions::default());
        let advances = |layout: &Layout<TextBrush>| {
            let mut result = Vec::new();
            for line in layout.lines() {
                for run in line.runs() {
                    for cluster in run.visual_clusters() {
                        result.push(cluster.advance());
                    }
                }
            }
            result
        };
        let original = advances(&layout);
        let original_width = layout.width();
        for _ in 0..3 {
            layout.align(
                Alignment::Left,
                AlignmentOptions {
                    last_line: Some(Alignment::Justify),
                    align_when_overflowing: true,
                },
            );
            assert!((layout.width() - 100.).abs() < 0.01);
            layout.align(Alignment::Left, AlignmentOptions::default());
            assert!((layout.width() - original_width).abs() < 0.01);
            let restored = advances(&layout);
            assert_eq!(original.len(), restored.len());
            assert!(
                original
                    .iter()
                    .zip(restored)
                    .all(|(original, restored)| (original - restored).abs() < 0.001)
            );
        }
        layout.align(
            Alignment::Justify,
            AlignmentOptions {
                last_line: Some(Alignment::Justify),
                ..Default::default()
            },
        );
        layout.break_all_lines(Some(100.));
        assert!((layout.width() - original_width).abs() < 0.01);
    }

    #[test]
    fn text_indent_respects_block_first_line_each_line_and_hanging() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, indent: &str| {
            engine.shape_styled(
                "a\nb",
                100.,
                16.,
                &BTreeMap::from([
                    ("white-space".into(), "pre".into()),
                    ("text-indent".into(), indent.into()),
                    ("text-align".into(), "start".into()),
                ]),
                ViewportState::default(),
            )
        };
        let normal = shape(&mut engine, "20px");
        assert_eq!(normal.lines.len(), 2);
        assert!((normal.lines[0].4 - 20.).abs() < 0.01);
        assert!(normal.lines[1].4.abs() < 0.01);
        let each = shape(&mut engine, "20px each-line");
        assert!(each.lines.iter().all(|line| (line.4 - 20.).abs() < 0.01));
        let hanging = shape(&mut engine, "20px hanging");
        assert!(hanging.lines[0].4.abs() < 0.01);
        assert!((hanging.lines[1].4 - 20.).abs() < 0.01);
        let each_hanging = shape(&mut engine, "hanging 20px each-line");
        assert!(each_hanging.lines.iter().all(|line| line.4.abs() < 0.01));
        let percent = shape(&mut engine, "calc(10% + 5px)");
        assert!((percent.lines[0].4 - 15.).abs() < 0.01);
        let negative = shape(&mut engine, "-10px");
        assert!((negative.lines[0].4 + 10.).abs() < 0.01);
        assert!(
            negative
                .clusters
                .iter()
                .any(|cluster| cluster.line == 0 && cluster.rect.x < 0.)
        );
    }

    #[test]
    fn text_indent_uses_the_rtl_start_edge_and_affects_wrap_width() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("direction".into(), "rtl".into()),
            ("text-align".into(), "start".into()),
            ("text-indent".into(), "20px".into()),
        ]);
        let rtl = engine.shape_styled("abc", 100., 16., &style, ViewportState::default());
        assert!((rtl.lines[0].4 + rtl.lines[0].1 - 80.).abs() < 0.01);
        let unindented = engine.shape_styled(
            "aa aa",
            200.,
            16.,
            &BTreeMap::new(),
            ViewportState::default(),
        );
        let width = unindented.width + 1.;
        let wrapped = engine.shape_styled(
            "aa aa",
            width,
            16.,
            &BTreeMap::from([("text-indent".into(), "10px".into())]),
            ViewportState::default(),
        );
        assert!(wrapped.lines.len() > 1);
        assert!((wrapped.lines[0].4 - 10.).abs() < 0.01);
        assert!(wrapped.lines[1].4.abs() < 0.01);
    }

    #[test]
    fn computed_text_indent_accepts_keywords_and_rejects_duplicates() {
        let mut root = node(1, None, "div", "text");
        root.children = vec![2];
        let input = |rules: &str| LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root.clone(), node(2, Some(1), "span", "child")],
            stylesheets: vec![rules.into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let valid = compute(
            &input("div { text-indent: hanging 10px each-line; }"),
            &LayoutLimits::default(),
        )
        .unwrap();
        let value = &valid.computed_styles[&1]["text-indent"];
        assert!(value.contains("10px") && value.contains("each-line") && value.contains("hanging"));
        assert_eq!(valid.computed_styles[&2]["text-indent"], *value);
        let invalid = compute(
            &input("div { text-indent: 7px; text-indent: 10px hanging hanging; }"),
            &LayoutLimits::default(),
        )
        .unwrap();
        assert_eq!(invalid.computed_styles[&1]["text-indent"], "7px");
    }

    #[test]
    fn text_span_styles_share_maps_without_losing_decoration_or_fallback() {
        let mut engine = TextEngine::new(&[]);
        let shared = Arc::new(BTreeMap::from([("color".into(), "blue".into())]));
        let decorated = Arc::new(BTreeMap::from([("color".into(), "red".into())]));
        let mut source = text_source::TextSource::default();
        let mut styles = ComputedStyles::new();
        for owner in 1..=5000 {
            source.append(owner, Some(owner + 10000), "x");
            styles.insert(owner, shared.clone());
        }
        engine.decoration_styles.insert(1, decorated.clone());
        source.append(6001, Some(16001), "a");
        source.append(6002, Some(16002), "b");
        let projected = engine.text_span_styles(&source, &styles, &shared);
        assert!(Arc::ptr_eq(&projected[&1], &decorated));
        assert!((2..=5000).all(|owner| Arc::ptr_eq(&projected[&owner], &shared)));
        assert!(Arc::ptr_eq(&projected[&6001], &projected[&6002]));
        assert_eq!(*projected[&6001], *shared);
        assert_eq!(shared.len(), 1);
    }

    #[test]
    fn text_shared_styles_preserve_owner_boundaries_in_horizontal_and_vertical_runs() {
        for (mode, combine, text) in [
            ("horizontal-tb", "none", "A\nB"),
            ("vertical-rl", "none", "AB"),
            ("vertical-rl", "all", "12"),
            ("sideways-lr", "none", "AB"),
        ] {
            let mut engine = TextEngine::new(&[]);
            let style = Arc::new(BTreeMap::from([
                ("white-space".into(), "pre".into()),
                ("writing-mode".into(), mode.into()),
                ("text-combine-upright".into(), combine.into()),
            ]));
            let mut source = text_source::TextSource::default();
            source.append(1, Some(11), &text[..1]);
            source.append(2, Some(12), &text[1..]);
            let styles = ComputedStyles::from([(1, style.clone()), (2, style.clone())]);
            let shaped = engine.shape_source(
                &source,
                200.,
                16.,
                &style,
                ViewportState::default(),
                &styles,
            );
            let owners = shaped
                .provenance
                .iter()
                .flatten()
                .filter(|run| !run.bytes.is_empty())
                .map(|run| run.owner)
                .collect::<BTreeSet<_>>();
            assert_eq!(owners, BTreeSet::from([1, 2]), "{mode} {combine}");
            assert!(
                shaped
                    .source_clusters
                    .iter()
                    .any(|cluster| cluster.owner == 1 && cluster.text_node == 11)
            );
            assert!(
                shaped
                    .source_clusters
                    .iter()
                    .any(|cluster| cluster.owner == 2 && cluster.text_node == 12)
            );
        }
    }

    #[test]
    fn plaintext_resolves_each_preserved_paragraph_and_restores_source_offsets() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("unicode-bidi".into(), "plaintext".into()),
            ("white-space".into(), "pre".into()),
            ("text-align".into(), "start".into()),
            ("line-height".into(), "20px".into()),
        ]);
        let shaped =
            engine.shape_styled("אב\nabc\nאב", 200., 16., &style, ViewportState::default());
        assert_eq!(shaped.lines.len(), 3);
        assert!(shaped.lines[0].4 > 0.);
        assert!(shaped.lines[1].4.abs() < 0.01);
        assert!(shaped.lines[2].4 > 0.);
        for (index, line) in shaped.lines.iter().enumerate() {
            assert!((line.3 - index as f64 * 20.).abs() < 0.01);
        }
        let latin = shaped
            .clusters
            .iter()
            .find(|cluster| cluster.bytes.contains(&5))
            .unwrap();
        assert_eq!(latin.line, 1);
        // Range rectangles use font ascent/descent, not the CSS flow origin.
        // Equal font metrics retain the same negative-leading offset in each
        // paragraph while their baselines and flow origins advance by 20px.
        let first = shaped
            .clusters
            .iter()
            .find(|cluster| cluster.line == 0)
            .unwrap();
        assert!(
            (latin.rect.y - first.rect.y - 20.).abs() < 0.01,
            "latin={:?}, first={:?}, lines={:?}",
            latin.rect,
            first.rect,
            shaped.lines
        );
        assert!((latin.rect.height - first.rect.height).abs() < 0.01);
        assert!((shaped.baselines[1] - shaped.baselines[0]).abs() < 0.01);
        assert!((shaped.height - 60.).abs() < 0.01);
        let same_paragraph =
            engine.shape_styled("אב\u{2028}abc", 200., 16., &style, ViewportState::default());
        assert_eq!(same_paragraph.lines.len(), 2);
        assert!(same_paragraph.lines.iter().all(|line| line.4 > 0.));
    }

    #[test]
    fn preserved_empty_paragraphs_have_line_boxes_without_an_extra_final_box() {
        let mut engine = TextEngine::new(&[]);
        let style = BTreeMap::from([
            ("white-space".into(), "pre".into()),
            ("line-height".into(), "20px".into()),
        ]);
        let shaped = engine.shape_styled("a\n\nb\n", 200., 16., &style, ViewportState::default());
        assert_eq!(shaped.lines.len(), 3);
        assert!((shaped.height - 60.).abs() < 0.01);
        assert!((shaped.lines[2].3 - 40.).abs() < 0.01);
        let single = engine.shape_styled("\n", 200., 16., &style, ViewportState::default());
        assert_eq!(single.lines.len(), 1);
        assert!((single.height - 20.).abs() < 0.01);
        let collapsed = engine.shape_styled(
            "a\nb",
            200.,
            16.,
            &BTreeMap::new(),
            ViewportState::default(),
        );
        assert_eq!(collapsed.lines.len(), 1);
    }

    #[test]
    fn paragraph_split_preserves_ranged_fonts_and_native_utf16_positions() {
        let mut engine = TextEngine::new(&[]);
        let mut node = node(1, None, "span", "AA\nBB");
        node.content = Some(vec![LayoutContent::Text {
            node_id: 10,
            text: node.text.clone(),
        }]);
        let style = BTreeMap::from([("white-space".into(), "pre".into())]);
        let source = text_source::TextSource::direct(&node);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert!(
            shaped
                .source_clusters
                .iter()
                .any(|cluster| cluster.text_node == 10
                    && cluster.utf16.start == 3
                    && cluster.line == 1)
        );
        let small = BTreeMap::from([("font-size".into(), "16px".into())]);
        let large = BTreeMap::from([("font-size".into(), "32px".into())]);
        let ranged = engine.shape_ranged(
            "AA\nBB",
            200.,
            16.,
            &style,
            ViewportState::default(),
            &[(0..3, &small, None, 0), (3..5, &large, None, 0)],
        );
        assert_eq!(ranged.lines.len(), 2);
        assert!(ranged.glyph_runs[0].iter().all(|run| run.font_size == 16.));
        assert!(ranged.glyph_runs[1].iter().all(|run| run.font_size == 32.));
        assert!(ranged.lines[1].2 > ranged.lines[0].2);
        assert!(
            ranged
                .clusters
                .iter()
                .any(|cluster| cluster.bytes.start == 3 && cluster.line == 1)
        );
    }

    #[test]
    fn removed_case_mark_remains_in_shaped_native_range_geometry() {
        let mut engine = TextEngine::new(&[]);
        engine
            .languages
            .insert(1, fontique::Language::parse("tr").ok());
        let mut node = node(1, None, "span", "I\u{307}");
        node.content = Some(vec![LayoutContent::Text {
            node_id: 2,
            text: node.text.clone(),
        }]);
        let source = text_source::TextSource::direct(&node);
        let style = BTreeMap::from([("text-transform".into(), "lowercase".into())]);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        let reference =
            engine.shape_styled("i", 200., 16., &BTreeMap::new(), ViewportState::default());
        assert_eq!(shaped.glyph_runs, reference.glyph_runs);
        assert_eq!(shaped.source_clusters.len(), 1);
        assert_eq!(shaped.source_clusters[0].utf16, 0..2);
        assert!(shaped.source_clusters[0].rect.width > 0.);
        assert_eq!(node.text, "I\u{307}");
    }

    #[test]
    fn css_uppercase_shapes_expanded_text_and_keeps_original_dom_offsets() {
        let mut engine = TextEngine::new(&[]);
        let mut node = node(1, None, "span", "ß");
        node.text = "ß".into();
        node.content = Some(vec![LayoutContent::Text {
            node_id: 2,
            text: "ß".into(),
        }]);
        let source = text_source::TextSource::direct(&node);
        let style = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &style,
            ViewportState::default(),
            &BTreeMap::new(),
        );
        let reference =
            engine.shape_styled("SS", 200., 16., &BTreeMap::new(), ViewportState::default());
        assert_eq!(shaped.lines, reference.lines);
        assert_eq!(shaped.glyph_runs, reference.glyph_runs);
        assert!(!shaped.source_clusters.is_empty());
        assert!(
            shaped
                .source_clusters
                .iter()
                .all(|cluster| cluster.text_node == 2 && cluster.utf16 == (0..1))
        );
        assert_eq!(node.text, "ß");
    }

    #[test]
    fn css_direction_controls_neutral_order_independently_of_alignment() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, direction: &str| {
            engine.shape_styled(
                "abc!",
                200.,
                16.,
                &BTreeMap::from([
                    ("direction".into(), direction.into()),
                    ("text-align".into(), "left".into()),
                ]),
                ViewportState::default(),
            )
        };
        let ltr = shape(&mut engine, "ltr");
        let rtl = shape(&mut engine, "rtl");
        let position = |layout: &TextLayout, byte: usize| {
            layout
                .clusters
                .iter()
                .find(|cluster| cluster.bytes.contains(&byte))
                .unwrap()
                .rect
                .x
        };
        assert!(position(&ltr, 0) < position(&ltr, 3));
        assert!(position(&rtl, 3) < position(&rtl, 0));
        // Reusing the context must not preserve the previous paragraph setting.
        let again = shape(&mut engine, "ltr");
        assert_eq!(position(&ltr, 3), position(&again, 3));
    }

    #[test]
    fn paragraph_direction_defaults_reset_and_plaintext_uses_first_strong() {
        let mut engine = TextEngine::new(&[]);
        {
            let mut builder = engine
                .layout
                .ranged_builder(&mut engine.fonts, "abc", 1., true);
            builder.set_base_direction(parley::BaseDirection::Rtl);
            let layout: Layout<TextBrush> = builder.build("abc");
            assert!(layout.is_rtl());
        }
        let layout: Layout<TextBrush> = engine
            .layout
            .ranged_builder(&mut engine.fonts, "abc", 1., true)
            .build("abc");
        assert!(!layout.is_rtl());
        let shape = |engine: &mut TextEngine, text: &str, bidi: &str, direction: &str| {
            engine.shape_styled(
                text,
                200.,
                16.,
                &BTreeMap::from([
                    ("unicode-bidi".into(), bidi.into()),
                    ("direction".into(), direction.into()),
                    ("text-align".into(), "start".into()),
                ]),
                ViewportState::default(),
            )
        };
        let hebrew = shape(&mut engine, "אב", "plaintext", "ltr");
        assert!(hebrew.lines[0].4 > 0.);
        let normal = shape(&mut engine, "אב", "normal", "ltr");
        assert!(normal.lines[0].4.abs() < 0.01);
        let latin = shape(&mut engine, "abc", "plaintext", "rtl");
        assert!(latin.lines[0].4.abs() < 0.01);
    }

    #[test]
    fn css_alignment_moves_fragments_without_double_shifting_glyphs() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, alignment: &str, direction: &str, width: f64| {
            engine.shape_styled(
                "AV",
                width,
                16.,
                &BTreeMap::from([
                    ("text-align".into(), alignment.into()),
                    ("direction".into(), direction.into()),
                ]),
                ViewportState::default(),
            )
        };
        let left = shape(&mut engine, "left", "ltr", 200.);
        let center = shape(&mut engine, "center", "ltr", 200.);
        let right = shape(&mut engine, "right", "ltr", 200.);
        let free = 200. - left.lines[0].1;
        assert!((left.lines[0].4).abs() < 0.01);
        assert!((center.lines[0].4 - free / 2.).abs() < 0.01);
        assert!((right.lines[0].4 - free).abs() < 0.01);
        for aligned in [&center, &right] {
            let original = left.glyph_runs[0]
                .iter()
                .flat_map(|run| &run.glyphs)
                .collect::<Vec<_>>();
            let moved = aligned.glyph_runs[0]
                .iter()
                .flat_map(|run| &run.glyphs)
                .collect::<Vec<_>>();
            assert_eq!(original.len(), moved.len());
            for (original, moved) in original.into_iter().zip(moved) {
                assert_eq!(original.id, moved.id);
                assert!((original.x - moved.x).abs() < 0.001);
                assert_eq!(original.y, moved.y);
            }
        }
        let start = shape(&mut engine, "start", "rtl", 200.);
        let end = shape(&mut engine, "end", "rtl", 200.);
        assert_eq!(start.lines, right.lines);
        assert_eq!(end.lines, left.lines);
        let overflow = shape(&mut engine, "center", "ltr", 1.);
        assert!(overflow.lines[0].4 < 0.);
    }

    #[test]
    fn css_line_height_controls_flow_and_shaped_baselines() {
        let mut engine = TextEngine::new(&[]);
        let shape = |engine: &mut TextEngine, height: &str| {
            engine.shape_styled(
                "A\nA",
                200.,
                16.,
                &BTreeMap::from([
                    ("line-height".into(), height.into()),
                    ("white-space-collapse".into(), "preserve-breaks".into()),
                ]),
                ViewportState::default(),
            )
        };
        let number = shape(&mut engine, "3");
        let length = shape(&mut engine, "48px");
        let percent = shape(&mut engine, "300%");
        assert_eq!(number.lines.len(), 2);
        assert_eq!(number.lines, length.lines);
        assert_eq!(number.lines, percent.lines);
        assert_eq!(number.glyph_runs, length.glyph_runs);
        assert!((number.height - 96.).abs() < 0.01);
        let baseline = |layout: &TextLayout, index: usize| {
            layout.lines[index].3 + layout.glyph_runs[index][0].glyphs[0].y
        };
        assert!((baseline(&number, 1) - baseline(&number, 0) - 48.).abs() < 0.01);
        let tight = shape(&mut engine, "8px");
        assert!((baseline(&tight, 1) - baseline(&tight, 0) - 8.).abs() < 0.01);
        // Ink bounds may overlap when the author deliberately sets a small line height.
        assert!(tight.lines[0].2 > 8.);
    }

    #[test]
    fn css_spacing_reaches_glyph_positions_and_intrinsic_width() {
        let mut engine = TextEngine::new(&[]);
        let ordinary = engine.shape("A A", 500., 16., DEFAULT_FONT_FAMILY);
        let spaced = engine.shape_styled(
            "A A",
            500.,
            16.,
            &BTreeMap::from([
                ("letter-spacing".into(), "2px".into()),
                ("word-spacing".into(), "5px".into()),
            ]),
            ViewportState::default(),
        );
        assert!(spaced.width > ordinary.width + 5.);
        let last_x = |layout: &TextLayout| {
            layout.glyph_runs[0]
                .iter()
                .flat_map(|run| &run.glyphs)
                .last()
                .unwrap()
                .x
        };
        assert!(last_x(&spaced) > last_x(&ordinary) + 5.);
        let normal = engine.shape_styled(
            "A A",
            500.,
            16.,
            &BTreeMap::from([
                ("letter-spacing".into(), "normal".into()),
                ("word-spacing".into(), "normal".into()),
            ]),
            ViewportState::default(),
        );
        assert_eq!(normal.glyph_runs, ordinary.glyph_runs);
    }

    #[test]
    fn css_family_list_preserves_later_candidates_and_quoted_commas() {
        let mut engine = TextEngine::new(&[(
            "Custom, Face".into(),
            DEFAULT_FONT_BYTES.to_vec(),
            String::new(),
            false,
            false,
        )]);
        let reference = engine.shape("AV office", 500., 16., "\"Custom, Face\"");
        let fallback = engine.shape("AV office", 500., 16., "\"Unavailable\", \"Custom, Face\"");
        assert!(reference.width > 0.);
        assert_eq!(reference.glyph_runs, fallback.glyph_runs);
        assert_eq!(engine.paint_fonts.len(), 1);
        // Although fixture bytes match the bundled font, resource identities differ.
        // Both queries must choose the custom face, not silently skip to the bundle.
        assert_eq!(engine.font_keys.len(), 1);
    }

    #[test]
    fn shaping_exports_visual_glyphs_and_deduplicated_exact_faces() {
        let mut engine = TextEngine::new(&[]);
        let first = engine.shape(
            "AV office a\u{301} AV office",
            60.,
            16.,
            DEFAULT_FONT_FAMILY,
        );
        assert!(first.lines.len() > 1);
        assert_eq!(first.lines.len(), first.glyph_runs.len());
        assert_eq!(engine.paint_fonts.len(), 1);
        for runs in &first.glyph_runs {
            assert!(!runs.is_empty());
            for run in runs {
                let face = &engine.paint_fonts[&run.font_key];
                assert_eq!(face.bytes, DEFAULT_FONT_BYTES);
                assert_eq!(face.collection_index, 0);
                assert_eq!(run.font_size, 16.);
                assert!(
                    run.glyphs
                        .iter()
                        .all(|glyph| glyph.x.is_finite() && glyph.y.is_finite())
                );
                // Each line owns relative baselines, not an accumulated paragraph offset.
                assert!(run.glyphs.iter().all(|glyph| glyph.y < 40.));
            }
        }
        let second = engine.shape(
            "AV office a\u{301} AV office",
            60.,
            16.,
            DEFAULT_FONT_FAMILY,
        );
        assert_eq!(first.glyph_runs, second.glyph_runs);
        assert_eq!(engine.paint_fonts.len(), 1);
        let encoded = serde_json::to_vec(&first.glyph_runs).unwrap();
        let decoded: Vec<Vec<oryn_common::v4::PaintGlyphRun>> =
            serde_json::from_slice(&encoded).unwrap();
        assert_eq!(first.glyph_runs, decoded);
    }

    pub(super) fn node(id: u64, parent: Option<u64>, tag: &str, text: &str) -> LayoutNode {
        LayoutNode {
            tree_scope: None,
            document_url: None,
            content: None,
            id,
            parent,
            children: Vec::new(),
            tag: tag.into(),
            namespace: default_element_namespace(),
            attributes: BTreeMap::new(),
            namespaced_attributes: Vec::new(),
            checked: None,
            disabled: None,
            validity: None,
            indeterminate: false,
            image_intrinsics: None,
            image_document_root: None,
            document_replaced: false,
            intrinsic_image_size: None,
            text: text.into(),
        }
    }

    #[test]
    fn repeated_styles_share_storage_without_merging_distinct_values() {
        let mut root = node(1, None, "html", "");
        root.children = (2..=2049).collect();
        let mut nodes = vec![root];
        for id in 2..=2049 {
            let mut child = node(id, Some(1), "span", "");
            if id == 2049 {
                child
                    .attributes
                    .insert("style".into(), "color: red; --label: special".into());
            }
            nodes.push(child);
        }
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes,
            stylesheets: vec!["span { color: blue; --label: ordinary }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).unwrap();
        for id in 3..2049 {
            assert!(Arc::ptr_eq(&styles[&2], &styles[&id]));
        }
        assert!(!Arc::ptr_eq(&styles[&2], &styles[&2049]));
        assert_eq!(styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(styles[&2049]["color"], "rgb(255, 0, 0)");
        assert_eq!(styles[&2049]["--label"], "special");
        let mut snapshot = styles.clone();
        Arc::make_mut(snapshot.get_mut(&2).unwrap()).insert("color".into(), "green".into());
        assert_eq!(styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(snapshot[&3]["color"], "rgb(0, 0, 255)");
    }

    #[test]
    fn repeated_report_rows_share_style_storage() {
        let mut root = node(1, None, "html", "");
        root.children = vec![2];
        let mut table = node(2, Some(1), "table", "");
        let mut nodes = Vec::new();
        for row in 0..1000 {
            let id = 3 + row * 8;
            table.children.push(id);
            let mut tr = node(id, Some(2), "tr", "");
            tr.children = vec![id + 1, id + 3];
            let mut td = node(id + 1, Some(id), "td", "");
            td.attributes.insert("class".into(), "pass".into());
            td.children = vec![id + 2];
            let text = node(id + 2, Some(id + 1), "#text", "PASS");
            let mut other = node(id + 3, Some(id), "td", "");
            other.children = vec![id + 4];
            let mut pre = node(id + 4, Some(id + 3), "pre", "");
            pre.children = vec![id + 5, id + 7];
            let mut strong = node(id + 5, Some(id + 4), "strong", "");
            strong.children = vec![id + 6];
            nodes.extend([
                tr,
                td,
                text,
                other,
                pre,
                strong,
                node(id + 6, Some(id + 5), "#text", "assert_not_equals"),
                node(
                    id + 7,
                    Some(id + 4),
                    "#text",
                    &format!("blob:https://example.org/{row}"),
                ),
            ]);
        }
        // The WPT report places its assertion table inside a details disclosure.
        root.children = vec![8003];
        table.parent = Some(8003);
        let mut details = node(8003, Some(1), "details", "");
        details.children = vec![8004, 2];
        let mut summary = node(8004, Some(8003), "summary", "");
        summary.children = vec![8005];
        nodes.extend([
            details,
            summary,
            node(8005, Some(8004), "#text", "Asserts run"),
        ]);
        nodes.insert(0, table);
        nodes.insert(0, root);
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes,
            stylesheets: vec!["td{padding:1em} .pass{color:green} pre{white-space:pre}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        // Native snapshots carry text in ordered content, not LayoutNode entries.
        let text_nodes = input
            .nodes
            .iter()
            .filter(|node| node.tag == "#text")
            .map(|node| (node.id, node.text.clone()))
            .collect::<BTreeMap<_, _>>();
        input.nodes.retain(|node| node.tag != "#text");
        for node in &mut input.nodes {
            node.content = Some(
                node.children
                    .iter()
                    .map(|id| match text_nodes.get(id) {
                        Some(text) => LayoutContent::Text {
                            node_id: *id,
                            text: text.clone(),
                        },
                        None => LayoutContent::Element(*id),
                    })
                    .collect(),
            );
            node.children.retain(|id| !text_nodes.contains_key(id));
            node.attributes
                .insert("data-oryn-effective-dir".into(), "ltr".into());
        }
        for mode in ["ordinary", "anchor", "hidden", "decorated"] {
            input.stylesheets.truncate(1);
            match mode {
                "anchor" => input.stylesheets.push("html{anchor-name:--report}".into()),
                "hidden" => input.stylesheets.push("html{display:none}".into()),
                "decorated" => input
                    .stylesheets
                    .push("html{text-decoration:underline red}".into()),
                _ => (),
            }
            input.rendered_tree = Some(
                input
                    .nodes
                    .iter()
                    .map(|node| (node.id, node.content.clone().unwrap_or_default()))
                    .collect(),
            );
            let output = StyleEngine::default()
                .compute(&input, &LayoutLimits::default())
                .unwrap();
            let styles = output.computed_styles;
            let decorations = text_decoration::propagate(&input, &styles);
            let decoration_maps = decorations
                .values()
                .map(Arc::as_ptr)
                .collect::<BTreeSet<_>>()
                .len();
            assert!(
                decoration_maps < 64,
                "{decoration_maps} duplicate decoration maps"
            );
            let identities = styles.values().map(Arc::as_ptr).collect::<BTreeSet<_>>();
            assert!(
                identities.len() < 64,
                "{} distinct styles",
                identities.len()
            );
            for row in 1..1000 {
                for offset in [0, 1, 3, 4, 5] {
                    assert!(Arc::ptr_eq(
                        &styles[&(3 + offset)],
                        &styles[&(3 + row * 8 + offset)]
                    ));
                }
            }
        }
    }

    #[test]
    fn ordered_layout_content_preserves_mixed_text_and_native_identity() {
        let mut root = node(1, None, "span", "beforeafter");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "before".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "after".into(),
            },
        ]);
        let mut child = node(2, Some(1), "em", "inside");
        child.content = Some(vec![LayoutContent::Text {
            node_id: 12,
            text: "inside".into(),
        }]);
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        assert_eq!(descendant_text(&root, &nodes), "beforeinsideafter");
        let source = text_source::TextSource::collect(&root, &nodes, false);
        assert_eq!(
            source
                .spans
                .iter()
                .map(|span| (span.owner, span.text_node, span.bytes.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, Some(10), 0..6),
                (2, Some(12), 6..12),
                (1, Some(11), 12..17)
            ]
        );
        assert_eq!(source.byte_boundary(12, 3, false), Some(9));
        let direct = text_source::TextSource::direct(&root);
        assert_eq!(direct.text, "beforeafter");
        assert_eq!(
            direct
                .spans
                .iter()
                .map(|span| (span.text_node, span.bytes.clone()))
                .collect::<Vec<_>>(),
            vec![(Some(10), 0..6), (Some(11), 6..11)]
        );
        assert_eq!(direct.byte_boundary(11, 1, false), Some(7));
        let restored: LayoutNode =
            serde_json::from_slice(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert_eq!(root.content, restored.content);
        // A present empty sequence is authoritative, even if compatibility text is stale.
        root.content = Some(vec![]);
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        assert_eq!(descendant_text(&root, &nodes), "");
    }

    #[test]
    fn descendant_text_preserves_sibling_order_and_authored_separators() {
        let mut root = node(1, None, "span", "");
        root.children = vec![2, 4, 5];
        let mut first = node(2, Some(1), "span", "left");
        first.children = vec![3];
        let nested = node(3, Some(2), "span", "\t");
        let space = node(4, Some(1), "span", "  ");
        let last = node(5, Some(1), "span", "right\n");
        let nodes = BTreeMap::from([
            (1, &root),
            (2, &first),
            (3, &nested),
            (4, &space),
            (5, &last),
        ]);
        assert_eq!(descendant_text(&root, &nodes), "left\t  right\n");
    }

    #[test]
    fn descendant_text_preserves_complete_utf8_content() {
        let mut root = node(1, None, "div", "");
        root.children.push(2);
        let child = node(2, Some(1), "span", &format!("{}😀tail", "a".repeat(4095)));
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        assert_eq!(
            descendant_text(&root, &nodes),
            format!("{}😀tail", "a".repeat(4095))
        );
    }

    #[test]
    fn cssom_layer_order_uses_effective_sheets_without_duplicate_dom_source() {
        let rules =
            "@layer first { #target { color: green; } } @layer second { #target { color: red; } }";
        let mut target = node(5, Some(2), "div", "");
        target.attributes.insert("id".into(), "target".into());
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                node(1, None, "html", ""),
                node(2, Some(1), "body", ""),
                node(3, Some(2), "style", ""),
                node(4, Some(2), "style", rules),
                target,
            ],
            stylesheets: vec![String::new(), rules.into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        assert_eq!(
            stylo_computed_values(&input).unwrap()[&5]["color"],
            "rgb(255, 0, 0)"
        );
        input.stylesheets[0] = "@layer second {}".into();
        assert_eq!(
            stylo_computed_values(&input).unwrap()[&5]["color"],
            "rgb(0, 128, 0)"
        );
        // DOM text remains authored even when the effective CSSOM sheet is empty.
        input.nodes[2].text = "@layer second {}".into();
        input.stylesheets[0].clear();
        assert_eq!(
            stylo_computed_values(&input).unwrap()[&5]["color"],
            "rgb(255, 0, 0)"
        );
    }

    #[test]
    fn style_mirror_uses_authored_content_order_independent_of_inventory() {
        let mut root = node(1, None, "html", "");
        root.children = vec![2, 3];
        root.content = Some(vec![LayoutContent::Element(2), LayoutContent::Element(3)]);
        let mut first = node(2, Some(1), "div", "stale compatibility text");
        first.content = Some(vec![]);
        let mut second = node(3, Some(1), "div", "");
        second.content = Some(vec![LayoutContent::Text {
            node_id: 10,
            text: "present".into(),
        }]);
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![second, root, first],
            stylesheets: vec![
                "div {color:red} div:first-child {color:blue} div:empty {background-color:lime}"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![],
        };
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(styles[&3]["color"], "rgb(255, 0, 0)");
        assert_eq!(styles[&2]["background-color"], "rgb(0, 255, 0)");
        assert_ne!(styles[&3]["background-color"], "rgb(0, 255, 0)");
    }

    #[test]
    fn computed_styles_handle_document_level_text_without_inline_layout() {
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "#text", "before"), node(2, None, "html", "")],
            stylesheets: vec!["html { color: rgb(1, 2, 3) }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).expect("computed styles");
        assert_eq!(styles[&2]["color"], "rgb(1, 2, 3)");
        let text_only = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "#text", "plain text")],
            ..input
        };
        assert!(stylo_computed_values(&text_only).is_ok());
    }

    #[test]
    fn stylo_resolves_rules_inside_display_none_subtrees() {
        let mut root = node(1, None, "html", "");
        root.children = vec![2];
        let mut hidden = node(2, Some(1), "section", "");
        hidden.children = vec![3];
        let mut child = node(3, Some(2), "div", "");
        child.children = vec![4];
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, hidden, child, node(4, Some(3), "span", "")],
            stylesheets: vec!["section { display: none; color: red; opacity: .5 } div { color: green; width: 37px } div.changed { color: lime } span { background-color: blue }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["display"], "none");
        assert_eq!(styles[&3]["color"], "rgb(0, 128, 0)");
        assert_eq!(styles[&3]["width"], "37px");
        assert_eq!(styles[&3]["opacity"], "1");
        assert_eq!(styles[&4]["color"], "rgb(0, 128, 0)");
        assert_eq!(styles[&4]["background-color"], "rgb(0, 0, 255)");
        input.nodes[2]
            .attributes
            .insert("class".into(), "changed".into());
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&3]["color"], "rgb(0, 255, 0)");
        assert_eq!(styles[&4]["color"], "rgb(0, 255, 0)");
        let output = StyleEngine::default()
            .compute(&input, &LayoutLimits::default())
            .unwrap();
        for id in [2, 3, 4] {
            assert_eq!(
                output.geometry[&id].availability,
                GeometryAvailability::Hidden
            );
            assert!(output.geometry[&id].box_model.is_none());
        }
    }

    #[test]
    fn stylo_empty_selector_sees_element_text() {
        let mut root = node(1, None, "html", "");
        root.children = vec![2, 3];
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                root,
                node(2, Some(1), "div", "text"),
                node(3, Some(1), "div", ""),
            ],
            stylesheets: vec!["div { color: blue } div:empty { color: red }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(styles[&3]["color"], "rgb(255, 0, 0)");
    }

    #[test]
    fn cascade_and_block_flow_produce_relational_geometry() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                root,
                node(2, Some(1), "button", "First"),
                node(3, Some(1), "button", "Second"),
            ],
            stylesheets: vec![
                "button { display:block; width: 120px; height: 30px; margin: 4px }".into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).expect("layout");
        let first = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let second = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!(first.width, 134.0);
        assert_eq!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .width,
            120.0
        );
        assert!(second.y > first.y);
        assert!(output.geometry[&2].client_rects[0].width > 0.0);
        assert_eq!(output.computed_styles[&2]["height"], "30px");
        let mut inline_input = input.clone();
        inline_input.stylesheets =
            vec!["button { display:inline-block; width:120px; height:30px; margin:4px }".into()];
        let inline = compute(&inline_input, &LayoutLimits::default()).unwrap();
        let a = inline.geometry[&2].box_model.as_ref().unwrap().border_box;
        let b = inline.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!((a.width, b.width), (134., 134.));
        assert!((a.y - b.y).abs() < 0.01);
        assert!(b.x >= a.right());
    }

    #[test]
    fn rust_owned_custom_states_participate_in_stylo_relational_selectors() {
        let mut subject = node(1, None, "div", "");
        subject.attributes.insert("id".into(), "subject".into());
        subject.children = vec![2];
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![subject, node(2, Some(1), "my-element", "")],
            stylesheets: vec![
                "#subject { background-color: #f00 } my-element:state(--green) { color: #0f0 } #subject:has(:state(--green)) { background-color: #0f0 }"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::from([(2, BTreeSet::from(["--green".into()]))]),
            focused_node: None, focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).expect("layout");
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 255, 0)");
        assert_eq!(
            output.computed_styles[&1]["background-color"],
            "rgb(0, 255, 0)"
        );
    }

    #[test]
    fn stylo_canonicalizes_cssom_selectors() {
        assert_eq!(
            canonicalize_selector("div ol>li p").unwrap(),
            "div ol > li p"
        );
        assert_eq!(canonicalize_selector("[att=val]").unwrap(), "[att=\"val\"]");
        assert!(canonicalize_selector(":not()").is_err());
    }

    #[test]
    fn stylo_normalizes_and_validates_an_plus_b_selectors() {
        assert_eq!(
            canonicalize_selector(":nth-child( 23n\n +\n 123 )").unwrap(),
            ":nth-child(23n+123)"
        );
        assert_eq!(
            canonicalize_selector(":nth-last-of-type(N- 123)").unwrap(),
            ":nth-last-of-type(n-123)"
        );
        assert!(canonicalize_selector(":nth-child(n- 1 2)").is_err());
        assert!(canonicalize_selector(":nth-of-type(+ n + 7)").is_err());
    }

    #[test]
    fn shadow_parts_use_stylo_for_directional_matching() {
        let mut host = node(1, None, "my-element", "");
        host.children = vec![2];
        let mut inner = node(2, Some(1), "div", "Test");
        inner.attributes.insert("part".into(), "inner".into());
        inner.attributes.insert("dir".into(), "rtl".into());
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![host, inner],
            stylesheets: vec![
                "my-element::part(inner) { background-color: #f00 } my-element::part(inner):dir(rtl) { background-color: #00f }"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None, focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        // A part attribute in light DOM is not an exported shadow part.
        let light = compute(&input, &LayoutLimits::default()).expect("light DOM layout");
        assert_eq!(
            light.computed_styles[&2]["background-color"],
            "rgba(0, 0, 0, 0)"
        );
        input.nodes[0].children.clear();
        input.nodes[1].parent = None;
        input.shadow_trees.roots.insert(
            100,
            ShadowTree {
                host: 1,
                content: vec![LayoutContent::Element(2)],
            },
        );
        input.shadow_trees.node_scopes.insert(2, 100);
        let output = compute(&input, &LayoutLimits::default()).expect("layout");
        assert_eq!(
            output.computed_styles[&2]["background-color"],
            "rgb(0, 0, 255)"
        );
        input.nodes[1].attributes.insert("dir".into(), "ltr".into());
        let ltr = compute(&input, &LayoutLimits::default()).expect("LTR shadow layout");
        assert_eq!(
            ltr.computed_styles[&2]["background-color"],
            "rgb(255, 0, 0)"
        );
    }

    #[test]
    fn unavailable_system_font_uses_embedded_fallback() {
        let mut input_node = node(2, Some(1), "input", "");
        input_node
            .attributes
            .insert("value".into(), "Search".into());
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, input_node, node(3, Some(1), "span", "Result")],
                stylesheets: vec!["div { font-family: Arial }".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("layout");

        assert!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
                > 0.0
        );
        assert!(
            output.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width
                > 0.0
        );
    }

    #[test]
    fn table_columns_rowspan_and_collapsed_border_segments_share_one_grid() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2, 5];
        let mut first = node(2, Some(1), "tr", "");
        first.children = vec![3, 4];
        let mut span = node(3, Some(2), "td", "span");
        span.attributes.insert("rowspan".into(), "2".into());
        let mut second = node(5, Some(1), "tr", "");
        second.children = vec![6];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,first,span,node(4,Some(2),"td","wide content"),second,node(6,Some(5),"td","end")],
            stylesheets:vec!["table{width:300px;border-spacing:0;border-collapse:collapse} td{padding:0;border:2px solid red} tr:first-child td:last-child{height:30px;border-left:4px dashed blue} tr:last-child td{height:40px;border-left:6px double green}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let box_for = |id| output.paint_projections[&id].box_model.border_box;
        assert!((box_for(4).x - box_for(6).x).abs() < 0.01);
        assert!((box_for(3).height - box_for(2).height - box_for(5).height).abs() < 0.01);
        let right = output.paint_projections[&3]
            .border_segments
            .iter()
            .filter(|s| !s.horizontal && s.rect.x > box_for(3).x + box_for(3).width / 2.)
            .collect::<Vec<_>>();
        assert_eq!(right.len(), 2);
        assert!(
            right
                .iter()
                .any(|s| s.edge.width == 4. && s.edge.style == "dashed")
        );
        assert!(
            right
                .iter()
                .any(|s| s.edge.width == 6. && s.edge.style == "double")
        );
        assert_eq!(output.computed_styles[&3]["border-right-width"], "2px");
    }

    #[test]
    fn table_cell_middle_and_bottom_align_native_text_without_moving_cell_boxes() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2];
        let mut row = node(2, Some(1), "tr", "");
        row.children = vec![3, 4, 5];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,row,node(3,Some(2),"td","top"),node(4,Some(2),"td","middle"),node(5,Some(2),"td","bottom")],
            stylesheets:vec!["table{width:300px;border-spacing:0}td{height:100px;padding:0;line-height:20px;vertical-align:top}td:nth-child(2){vertical-align:middle}td:nth-child(3){vertical-align:bottom}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let boxes = (3..=5)
            .map(|id| output.paint_projections[&id].box_model.border_box)
            .collect::<Vec<_>>();
        assert!(
            boxes
                .iter()
                .all(|rect| rect.y == boxes[0].y && rect.height == boxes[0].height)
        );
        let top = output.text_fragments[&3][0].rect.y;
        assert!(
            (output.text_fragments[&4][0].rect.y - top - 40.).abs() < 0.01,
            "middle table alignment: boxes={boxes:?}, text={:?}, styles={:?}",
            output.text_fragments,
            output.computed_styles
        );
        assert!(
            (output.text_fragments[&5][0].rect.y - top - 80.).abs() < 0.01,
            "bottom table alignment: boxes={boxes:?}, text={:?}, styles={:?}",
            output.text_fragments,
            output.computed_styles
        );
    }

    #[test]
    fn table_baseline_measures_full_inline_participants_and_replaced_boxes() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2];
        let mut row = node(2, Some(1), "tr", "");
        row.children = vec![3, 4];
        let mut a = node(3, Some(2), "td", "small");
        a.children = vec![5];
        let mut b = node(4, Some(2), "td", "");
        b.children = vec![6, 7];
        let image = node(6, Some(4), "img", "");
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,row,a,b,node(5,Some(3),"span","Big"),image,node(7,Some(4),"span","Tail")],stylesheets:vec!["table{width:500px;border-spacing:0}td{padding:0;font-size:16px;line-height:20px;vertical-align:baseline}td:first-child span{font-size:32px;line-height:40px}img{width:20px;height:35px}".into()],stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let baseline = |id| {
            let fragment = &output.text_fragments[&id][0];
            fragment.rect.y + fragment.glyph_runs[0].glyphs[0].y
        };
        assert!((baseline(5) - baseline(7)).abs() < 0.05);
        let image = output.paint_projections[&6].box_model.border_box;
        assert!((image.bottom() - baseline(7)).abs() < 0.05);
        let row = output.paint_projections[&2].box_model.border_box;
        assert!(image.bottom() <= row.bottom() + 0.05);
    }

    #[test]
    fn table_baseline_alignment_grows_row_for_native_ascent_and_descent() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2];
        let mut row = node(2, Some(1), "tr", "");
        row.children = vec![3, 4];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,row,node(3,Some(2),"td","Ag"),node(4,Some(2),"td","Ag")],
            stylesheets:vec!["table{width:300px;border-spacing:0}td{padding:0;vertical-align:baseline;font-size:16px;line-height:20px}td+td{font-size:32px;line-height:40px;padding-top:8px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let baseline = |id| {
            let f = &output.text_fragments[&id][0];
            f.rect.y + f.glyph_runs[0].glyphs[0].y
        };
        assert!((baseline(3) - baseline(4)).abs() < 0.01);
        let row = output.paint_projections[&2].box_model.border_box;
        // CSS row sizing contains the line boxes, including the baseline
        // offset and cell padding. Negative leading permits native selection
        // extents outside those line boxes; it must not inflate the CSS row.
        assert!(
            output
                .text_fragments
                .values()
                .flatten()
                .all(|f| f.flow_rect.unwrap().bottom() <= row.bottom() + 0.01),
            "baseline table row={row:?}, geometry={:?}, text={:?}, styles={:?}",
            output.geometry,
            output.text_fragments,
            output.computed_styles
        );
        assert!((row.height - 48.).abs() < 0.01, "{row:?}");
        assert!(
            output.text_fragments[&4][0].rect.height
                > output.text_fragments[&4][0].flow_rect.unwrap().height,
            "native selection extent must remain intact"
        );
    }

    #[test]
    fn fixed_table_zero_width_is_definite_and_later_rows_do_not_allocate_columns() {
        for rule in [
            "col:first-child {width:0}",
            "tr:first-of-type td:first-child {width:0}",
        ] {
            let mut table = node(1, None, "table", "");
            table.children = vec![2, 3, 4, 7];
            let mut first = node(4, Some(1), "tr", "");
            first.children = vec![5, 6];
            let mut later = node(7, Some(1), "tr", "");
            later.children = vec![8, 9];
            let mut input = column_fixture(
                &format!(
                    "table {{width:200px;table-layout:fixed;border-spacing:0}} td {{padding:0}} tr:last-child td:first-child {{width:150px}} {rule}"
                ),
                Vec::new(),
            );
            input.nodes = vec![
                table,
                node(2, Some(1), "col", ""),
                node(3, Some(1), "col", ""),
                first,
                node(5, Some(4), "td", ""),
                node(6, Some(4), "td", ""),
                later,
                node(8, Some(7), "td", ""),
                node(9, Some(7), "td", ""),
            ];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let width = |id| {
                output.geometry[&id]
                    .box_model
                    .as_ref()
                    .unwrap()
                    .border_box
                    .width
            };
            assert!(width(5).abs() < 0.01, "{rule}: {}", width(5));
            assert!((width(6) - 200.).abs() < 0.01);
            assert!((width(8) - width(5)).abs() < 0.01);
        }
    }

    #[test]
    fn collapsed_colspan_edges_keep_column_colors_and_rtl_grid_order() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2, 3, 4, 6];
        let mut first = node(4, Some(1), "tr", "");
        first.children = vec![5];
        let mut span = node(5, Some(4), "td", "span");
        span.attributes.insert("colspan".into(), "2".into());
        let mut second = node(6, Some(1), "tr", "");
        second.children = vec![7, 8];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,node(2,Some(1),"col",""),node(3,Some(1),"col",""),first,span,second,node(7,Some(6),"td","first"),node(8,Some(6),"td","second")],
            stylesheets:vec!["table{width:200px;table-layout:fixed;border-collapse:collapse;direction:rtl}col{width:100px;border-top:4px solid red}col+col{border-top:6px double blue}td{padding:0}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.paint_projections[&7].box_model.border_box.x
                > output.paint_projections[&8].box_model.border_box.x
        );
        let projection = &output.paint_projections[&5];
        let segments = projection
            .border_segments
            .iter()
            .filter(|segment| {
                segment.horizontal && segment.rect.y < projection.box_model.border_box.y
            })
            .collect::<Vec<_>>();
        assert_eq!(segments.len(), 2);
        assert!(
            segments
                .iter()
                .any(|s| s.edge.width == 4. && s.edge.style == "solid")
        );
        assert!(
            segments
                .iter()
                .any(|s| s.edge.width == 6. && s.edge.style == "double")
        );
        assert!(
            segments.iter().find(|s| s.edge.width == 4.).unwrap().rect.x
                > segments.iter().find(|s| s.edge.width == 6.).unwrap().rect.x
        );
    }

    #[test]
    fn vertical_table_rows_columns_and_native_shaping_share_logical_axes() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-lr"] {
            for rtl in [false, true] {
                let mut table = node(1, None, "table", "");
                table.children = vec![2, 3, 4, 7];
                let mut first = node(4, Some(1), "tr", "");
                first.children = vec![5, 6];
                let mut second = node(7, Some(1), "tr", "");
                second.children = vec![8, 9];
                let input = LayoutInput {
                    svg_clip_resources: Vec::new(),
                    legacy_background_urls: Default::default(),
                    registered_custom_properties: Vec::new(),
                    document_language: None,
                    document_base_url: "https://oryn.invalid/".into(),
                    image_intrinsics: Default::default(),
                    control_states: Default::default(),
                    temporal_pickers: Default::default(),
                    permission_prompts: Default::default(),
                    top_layer: Vec::new(),
                    scrollbar_width: 0.,
                    nodes: vec![
                        table,
                        node(2, Some(1), "col", ""),
                        node(3, Some(1), "col", ""),
                        first,
                        node(5, Some(4), "td", "ABCD"),
                        node(6, Some(4), "td", "EF"),
                        second,
                        node(8, Some(7), "td", "GH"),
                        node(9, Some(7), "td", "IJ"),
                    ],
                    // Fitting a whole Latin word is a wrapping policy, independent
                    // of upright glyph orientation and table track direction.
                    stylesheets: vec![format!(
                        "table{{height:160px;table-layout:fixed;border-collapse:collapse;writing-mode:{mode};direction:{}}}col{{height:80px}}td{{width:40px;padding:0;font-size:20px;line-height:20px;text-orientation:upright;word-break:break-all;border:2px solid red}}",
                        if rtl { "rtl" } else { "ltr" }
                    )],
                    stylesheet_media: BTreeMap::new(),
                    selector_states: Default::default(),
                    shadow_trees: Default::default(),
                    stylesheet_base_urls: Default::default(),
                    stylesheet_scope_roots: BTreeMap::new(),
                    element_scrolls: BTreeMap::new(),
                    anchor_parents: Default::default(),
                    rendered_tree: None,
                    custom_states: BTreeMap::new(),
                    focused_node: None,
                    focus_visible: false,
                    viewport: ViewportState::default(),
                    scroll: ScrollState::default(),
                    color_scheme: ColorScheme::Light,
                    reduced_motion: false,
                    web_fonts: Vec::new(),
                };
                let output = compute(&input, &LayoutLimits::default()).unwrap();
                let model = |id| output.paint_projections[&id].box_model.border_box;
                let axes = writing_mode::Axes::new(mode, rtl);
                assert_eq!(model(5).y > model(6).y, axes.y_reverse, "{mode} rtl={rtl}");
                assert_eq!(model(5).x > model(8).x, axes.x_reverse, "{mode} rtl={rtl}");
                assert!((model(5).height - 80.).abs() < 0.01);
                assert!((model(2).y - model(5).y).abs() < 0.01);
                assert!(model(2).width >= model(5).width + model(8).width - 0.01);
                assert!(
                    output.text_fragments[&5]
                        .iter()
                        .all(|fragment| fragment.rect.height <= model(5).height + 0.01),
                    "vertical table mode={mode} rtl={rtl} cell={:?}, text={:?}, styles={:?}",
                    model(5),
                    output.text_fragments[&5],
                    output.computed_styles
                );
                assert!(!output.paint_projections[&5].border_segments.is_empty());
            }
        }
    }

    #[test]
    fn orthogonal_table_cell_alignment_uses_its_line_over_and_under_edges() {
        for align in ["top", "middle", "bottom"] {
            let mut table = node(1, None, "table", "");
            table.children = vec![2];
            let mut row = node(2, Some(1), "tr", "");
            row.children = vec![3];
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                nodes: vec![table, row, node(3, Some(2), "td", "AB")],
                stylesheets: vec![format!(
                    "table{{width:100px;writing-mode:horizontal-tb;border-spacing:0}}td{{writing-mode:vertical-lr;width:100px;height:80px;padding:0;font-size:16px;line-height:20px;text-orientation:upright;vertical-align:{align}}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let content = output.paint_projections[&3].box_model.content_box;
            let text = output.text_fragments[&3][0].rect;
            match align {
                "top" => assert!((text.x - content.x).abs() < 0.01),
                "middle" => assert!(
                    (text.x + text.width / 2. - content.x - content.width / 2.).abs() < 0.01,
                    "orthogonal middle text={text:?} content={content:?} styles={:?}",
                    output.computed_styles
                ),
                _ => assert!((text.right() - content.right()).abs() < 0.01),
            }
            assert!(
                output.text_fragments[&3][0]
                    .glyph_runs
                    .iter()
                    .all(|run| run.orientation == oryn_common::v4::PaintGlyphOrientation::Upright)
            );
        }
    }

    #[test]
    fn table_grid_progression_ignores_row_and_group_writing_mode_overrides() {
        // Pinned css-writing-modes/table-progression-vrl-001.html requires
        // grid progression from the table, while cell text may be orthogonal.
        let mut table = node(1, None, "table", "");
        table.children = vec![2];
        let mut group = node(2, Some(1), "tbody", "");
        group.children = vec![3, 6];
        let mut first = node(3, Some(2), "tr", "");
        first.children = vec![4, 5];
        let mut second = node(6, Some(2), "tr", "");
        second.children = vec![7, 8];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,group,first,node(4,Some(3),"td",""),node(5,Some(3),"td",""),second,node(7,Some(6),"td",""),node(8,Some(6),"td","")],
            stylesheets:vec!["table{height:80px;writing-mode:vertical-rl;direction:ltr;border-spacing:0}tbody,tr{writing-mode:horizontal-tb;direction:rtl}td{width:20px;height:40px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.paint_projections[&id].box_model.border_box;
        assert!(rect(4).x > rect(7).x);
        assert!(rect(4).y < rect(5).y);
        assert_eq!(output.computed_styles[&2]["writing-mode"], "horizontal-tb");
        assert_eq!(output.computed_styles[&3]["direction"], "rtl");
    }

    #[test]
    fn auto_table_uses_intrinsic_columns_and_caption_constraints_in_visual_order() {
        let mut table = node(1, None, "table", "");
        table.children = vec![2, 4, 5];
        let mut row = node(2, Some(1), "tr", "");
        row.children = vec![3];
        let mut input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,nodes:vec![table,row,node(3,Some(2),"td","word"),node(4,Some(1),"caption","top"),node(5,Some(1),"caption","bottom")],
            stylesheets:vec!["table{border-spacing:0}td{padding:0}caption{height:20px}caption:last-child{caption-side:bottom}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = |id| output.paint_projections[&id].box_model.border_box;
        assert!(rect(1).width < input.viewport.width / 2.);
        assert!(rect(4).bottom() <= rect(2).y + 0.01);
        assert!(
            rect(5).y >= rect(2).bottom() - 0.01,
            "bottom caption geometry={:?}, styles={:?}",
            output.geometry,
            output.computed_styles
        );
        input.stylesheets.push("caption{width:200px}".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!((output.paint_projections[&1].box_model.border_box.width - 200.).abs() < 0.01);
    }

    #[test]
    fn bounded_table_rows_and_cells_have_common_web_geometry() {
        let mut table = node(1, None, "table", "");
        table.children.push(2);
        let mut row = node(2, Some(1), "tr", "");
        row.children.extend([3, 4]);
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    table,
                    row,
                    node(3, Some(2), "td", "Name"),
                    node(4, Some(2), "td", "Value"),
                ],
                stylesheets: vec!["table{width:300px} td{padding:4px}".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("layout");
        assert_eq!(
            output.geometry[&1].availability,
            GeometryAvailability::Available
        );
        let left = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        let right = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!(left.width > 0.0);
        assert!(right.x >= left.right());
    }

    #[test]
    fn aria_hidden_changes_semantics_but_not_layout_geometry() {
        let mut child = node(2, Some(1), "div", "Visible box");
        child.attributes.insert("aria-hidden".into(), "true".into());
        let mut root = node(1, None, "div", "");
        root.children.push(2);
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, child],
                stylesheets: vec!["div { width: 100px; min-height: 20px }".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("layout");

        assert_eq!(
            output.geometry[&2].availability,
            GeometryAvailability::Available
        );
        assert!(output.geometry[&2].box_model.is_some());
    }

    #[test]
    fn native_basic_shape_clips_owner_and_descendant_projections() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let mut input = column_fixture(
            "div{width:100px;height:100px}div:first-child{clip-path:inset(10px round 5px)}",
            Vec::new(),
        );
        root.attributes
            .insert("style".into(), "clip-path:inset(10px round 5px)".into());
        input.nodes = vec![root, child];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let owner = output.paint_projections[&1]
            .clips
            .iter()
            .find(|clip| clip.owner == Some(1) && clip.shape.is_some())
            .unwrap();
        let descendant = output.paint_projections[&2]
            .clips
            .iter()
            .find(|clip| clip.owner == Some(1) && clip.shape.is_some())
            .unwrap();
        assert_eq!(owner, descendant);
        assert!(
            !owner
                .shape
                .as_ref()
                .unwrap()
                .contains(owner.rect.x, owner.rect.y)
        );
        assert!(!owner.scroll_x && !owner.scroll_y);
    }
    #[test]
    fn boxless_inline_ancestor_preserves_descendant_hits_and_outer_clipping() {
        let mut wrapper = node(2, Some(1), "shop-container", "");
        wrapper.children = vec![3];
        wrapper.content = Some(vec![LayoutContent::Element(3)]);
        let mut block = node(3, Some(2), "section", "");
        block.children = vec![4];
        block.content = Some(vec![LayoutContent::Element(4)]);
        let button = node(4, Some(3), "button", "Price");
        let mut input = column_fixture(
            "div{width:100px;height:100px;overflow:hidden}section{display:block}button{width:40px;height:20px}",
            vec![wrapper, block, button],
        );
        input.nodes[0].children = vec![2];
        input.nodes[0].content = Some(vec![LayoutContent::Element(2)]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        // Empty inline anchors retain a position for fragment navigation,
        // while contributing no area or hit target of their own.
        let wrapper = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert_eq!((wrapper.width, wrapper.height), (0., 0.));
        assert!(!output.geometry[&2].interactable);
        assert!(output.geometry[&4].in_viewport);
        assert!(output.geometry[&4].interactable);
        input.nodes[2]
            .attributes
            .insert("style".into(), "margin-left:150px".into());
        let clipped = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(!clipped.geometry[&4].in_viewport);
        assert!(!clipped.geometry[&4].interactable);
        input.nodes[2].attributes.remove("style");
        input.nodes[1]
            .attributes
            .insert("style".into(), "display:none".into());
        let hidden = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(!hidden.geometry[&4].interactable);
    }

    #[test]
    fn overflow_ancestors_clip_interactability_on_both_axes() {
        let mut root = node(1, None, "div", "");
        root.attributes.insert(
            "style".into(),
            "position:relative;width:100px;height:100px;overflow:hidden".into(),
        );
        root.children = vec![2, 3];
        let mut outside = node(2, Some(1), "button", "Outside");
        outside.attributes.insert(
            "style".into(),
            "position:absolute;left:150px;top:10px;width:20px;height:20px".into(),
        );
        let mut inside = node(3, Some(1), "button", "Inside");
        inside.attributes.insert(
            "style".into(),
            "position:absolute;left:90px;top:90px;width:20px;height:20px".into(),
        );
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, outside, inside],
                stylesheets: Vec::new(),
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("layout");

        assert!(!output.geometry[&2].in_viewport);
        assert!(!output.geometry[&2].interactable);
        assert!(output.geometry[&3].in_viewport);
        assert!(output.geometry[&3].interactable);
    }

    #[test]
    fn embedded_font_matches_the_frozen_asset_contract() {
        assert_eq!(DEFAULT_FONT_BYTES.len(), 569_208);
        assert_eq!(
            format!("{:x}", Sha256::digest(DEFAULT_FONT_BYTES)),
            DEFAULT_FONT_SHA256
        );
    }

    #[test]
    fn stylo_parses_and_filters_author_declarations() {
        let parsed = declarations_with_importance(
            "color: rgb(255, 0, 0) !important; width: calc(1px + ); not-a-real-property: nope",
        );
        assert!(parsed.contains_key("color"));
        assert!(parsed["color"].1);
        assert!(!parsed.contains_key("width"));
        assert!(!parsed.contains_key("not-a-real-property"));
    }

    #[test]
    fn native_focus_state_is_projected_into_stylo_and_cleared_on_next_flush() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3];
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![
                root,
                node(2, Some(1), "input", ""),
                node(3, Some(1), "input", ""),
            ],
            stylesheets: vec![
                "* {color:black; outline-color:black} :focus-within {color:red} :focus {color:blue} :focus-visible {outline-color:green}".into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: Some(2), focus_visible: true,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&1]["color"], "rgb(255, 0, 0)");
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(output.computed_styles[&3]["color"], "rgb(0, 0, 0)");
        assert_eq!(
            output.computed_styles[&2]["outline-color"],
            "rgb(0, 128, 0)"
        );
        input.focus_visible = false;
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 0, 255)");
        assert_eq!(output.computed_styles[&2]["outline-color"], "rgb(0, 0, 0)");
        input.focused_node = Some(3);
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 0, 0)");
        assert_eq!(output.computed_styles[&3]["color"], "rgb(0, 0, 255)");
        input.focused_node = None;
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        for id in 1..=3 {
            assert_eq!(output.computed_styles[&id]["color"], "rgb(0, 0, 0)");
        }
    }

    #[test]
    fn live_checked_state_does_not_rewrite_attribute_selectors() {
        let mut checkbox = node(1, None, "input", "");
        checkbox.attributes.insert("type".into(), "checkbox".into());
        checkbox.attributes.insert("checked".into(), String::new());
        checkbox.checked = Some(false);
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![checkbox],
            stylesheets: vec!["input {color:black;outline-color:black} input[checked] {background-color:red} input:checked {color:blue} input:indeterminate {outline-color:green}".into()],
            stylesheet_media: BTreeMap::new(), selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&1]["color"], "rgb(0, 0, 0)");
        assert_eq!(
            output.computed_styles[&1]["background-color"],
            "rgb(255, 0, 0)"
        );
        input.nodes[0].checked = Some(true);
        input.nodes[0].indeterminate = true;
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&1]["color"], "rgb(0, 0, 255)");
        assert_eq!(
            output.computed_styles[&1]["outline-color"],
            "rgb(0, 128, 0)"
        );
    }

    #[test]
    fn stylo_relational_and_filtered_nth_rules_recascade_after_mutation() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let mut nodes = vec![root];
        for (id, class) in [(2, "hot"), (3, "cold"), (4, "hot")] {
            let mut child = node(id, Some(1), "div", "");
            child.attributes.insert("class".into(), class.into());
            nodes.push(child);
        }
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes,
            stylesheets: vec!["main {color:black} main:has(> .hot + .cold) {color:green} div {width:10px} div:nth-child(2 of .hot) {width:73px}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None, focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&1]["color"], "rgb(0, 128, 0)");
        assert_eq!(output.computed_styles[&4]["width"], "73px");
        input.nodes[1]
            .attributes
            .insert("class".into(), "cold".into());
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&1]["color"], "rgb(0, 0, 0)");
        assert_eq!(output.computed_styles[&4]["width"], "10px");
    }

    #[test]
    fn stylo_mirror_preserves_native_element_namespaces() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let html = node(2, Some(1), "circle", "");
        let mut svg = node(3, Some(1), "circle", "");
        svg.namespace = "http://www.w3.org/2000/svg".into();
        let mut unqualified = node(4, Some(1), "circle", "");
        unqualified.namespace.clear();
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, html, svg, unqualified],
            stylesheets: vec!["@namespace svg url(http://www.w3.org/2000/svg); circle {color:red} svg|circle {color:green} |circle {color:blue}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None, focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["color"], "rgb(255, 0, 0)");
        assert_eq!(styles[&3]["color"], "rgb(0, 128, 0)");
        assert_eq!(styles[&4]["color"], "rgb(0, 0, 255)");
        let legacy: LayoutNode =
            serde_json::from_str(r#"{"id":5,"parent":null,"tag":"div"}"#).unwrap();
        assert_eq!(legacy.namespace, default_element_namespace());
    }

    #[test]
    fn stylo_attribute_value_case_sensitivity_follows_element_namespace() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let mut html = node(2, Some(1), "a", "");
        html.attributes.insert("type".into(), "FOO".into());
        let mut svg = html.clone();
        svg.id = 3;
        svg.namespace = "http://www.w3.org/2000/svg".into();
        let mut unqualified = html.clone();
        unqualified.id = 4;
        unqualified.namespace.clear();
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, html, svg, unqualified],
            stylesheets: vec!["* {color:red} [type=foo] {color:green}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).unwrap();
        assert_eq!(styles[&2]["color"], "rgb(0, 128, 0)");
        assert_eq!(styles[&3]["color"], "rgb(255, 0, 0)");
        assert_eq!(styles[&4]["color"], "rgb(255, 0, 0)");
        input.stylesheets = vec!["* {color:red} [type=foo i] {color:green}".into()];
        let styles = stylo_computed_values(&input).unwrap();
        for id in 2..=4 {
            assert_eq!(styles[&id]["color"], "rgb(0, 128, 0)");
        }
        input.stylesheets = vec!["* {color:red} [type=foo s] {color:green}".into()];
        let styles = stylo_computed_values(&input).unwrap();
        for id in 2..=4 {
            assert_eq!(styles[&id]["color"], "rgb(255, 0, 0)");
        }
    }

    #[test]
    fn stylo_scope_bounds_and_root_recascade_after_mutation() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 5];
        let mut scoped = node(2, Some(1), "section", "");
        scoped.attributes.insert("class".into(), "scope".into());
        scoped.children = vec![3, 4];
        let inside = node(3, Some(2), "p", "inside");
        let mut stop = node(4, Some(2), "section", "");
        stop.attributes.insert("class".into(), "stop".into());
        stop.children = vec![6];
        let outside = node(5, Some(1), "p", "outside");
        let excluded = node(6, Some(4), "p", "excluded");
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, scoped, inside, stop, outside, excluded],
            stylesheets: vec![
                "p {color:blue} @scope (.scope) to (.stop) { p {color:green} :scope {width:73px} }"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["width"], "73px");
        assert_eq!(output.computed_styles[&3]["color"], "rgb(0, 128, 0)");
        assert_eq!(output.computed_styles[&5]["color"], "rgb(0, 0, 255)");
        assert_eq!(output.computed_styles[&6]["color"], "rgb(0, 0, 255)");
        input.nodes[1].attributes.remove("class");
        let output = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&3]["color"], "rgb(0, 0, 255)");
    }

    #[test]
    fn stylo_whitespace_provenance_follows_the_winning_cascade_and_inheritance() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 4];
        let mut pre = node(2, Some(1), "pre", "");
        pre.children = vec![3];
        let child = node(3, Some(2), "span", "text");
        let sibling = node(4, Some(1), "span", "text");
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![child, sibling, pre, root],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for (css, expected, mode) in [
            ("", vec![], "pre"),
            ("pre {white-space:pre}", vec![2, 3], "pre"),
            ("div {white-space:pre-wrap}", vec![1, 4], "pre"),
            ("pre {white-space:revert}", vec![], "pre"),
            (
                "pre {white-space:pre-wrap} pre {white-space:revert}",
                vec![],
                "pre",
            ),
            (
                "@layer base, override; @layer base {pre {white-space:pre-wrap}} @layer override {pre {white-space:revert-layer}}",
                vec![2, 3],
                "pre-wrap",
            ),
            (
                "div {white-space:pre-wrap} pre {white-space:inherit}",
                vec![1, 2, 3, 4],
                "pre-wrap",
            ),
            ("pre {white-space:initial}", vec![2, 3], "normal"),
            ("pre {text-wrap-mode:wrap}", vec![2, 3], "pre-wrap"),
            (
                "pre {--ws:pre-wrap; white-space:var(--ws)}",
                vec![2, 3],
                "pre-wrap",
            ),
        ] {
            input.stylesheets = vec![css.into()];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert_eq!(
                output.author_whitespace,
                expected.into_iter().collect(),
                "{css}"
            );
            assert_eq!(output.computed_styles[&2]["white-space"], mode, "{css}");
        }
        input.nodes[2]
            .attributes
            .insert("style".into(), "white-space:normal".into());
        input.stylesheets = vec!["pre {white-space:pre !important}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["white-space"], "pre");
        assert_eq!(output.author_whitespace, BTreeSet::from([2, 3]));
    }

    #[test]
    fn stylo_exports_computed_whitespace_shorthand_and_longhands() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        let mut child = node(2, Some(1), "span", "text");
        child
            .attributes
            .insert("style".into(), "white-space: pre".into());
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, child],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for (shorthand, collapse, wrap) in [
            ("normal", "collapse", "wrap"),
            ("nowrap", "collapse", "nowrap"),
            ("pre", "preserve", "nowrap"),
            ("pre-wrap", "preserve", "wrap"),
            ("pre-line", "preserve-breaks", "wrap"),
            ("break-spaces", "break-spaces", "wrap"),
        ] {
            input.stylesheets = vec![format!("span {{ white-space: {shorthand} !important }}")];
            let styles = stylo_computed_values(&input).unwrap();
            assert_eq!(styles[&2]["white-space"], shorthand);
            assert_eq!(styles[&2]["white-space-collapse"], collapse);
            assert_eq!(styles[&2]["text-wrap-mode"], wrap);
            input.nodes[1].attributes.remove("style");
            input.stylesheets = vec![format!(
                "div {{ white-space-collapse: {collapse}; text-wrap-mode: {wrap} }}"
            )];
            let inherited = stylo_computed_values(&input).unwrap();
            assert_eq!(inherited[&2]["white-space"], shorthand);
            input.nodes[1]
                .attributes
                .insert("style".into(), "white-space: pre".into());
        }
    }

    #[test]
    fn stylo_owns_selector_parsing_specificity_importance_and_media_rules() {
        let mut root = node(1, None, "main", "");
        root.attributes.insert("id".into(), "app".into());
        root.children = vec![2, 3];
        let mut first = node(2, Some(1), "button", "First");
        first.attributes.insert("class".into(), "action".into());
        let mut second = node(3, Some(1), "button", "Second");
        second.attributes.insert("class".into(), "action".into());
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, first, second],
                stylesheets: vec![
                    "button { color: black !important }\n\
                     #app > .action { color: red }\n\
                     .action + .action { width: 71px }\n\
                     @media (min-width: 600px) { #app > button { height: 33px } }"
                        .into(),
                ],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("Stylo cascade projection");
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 0, 0)");
        assert_eq!(output.computed_styles[&3]["width"], "71px");
        assert_eq!(output.computed_styles[&2]["height"], "33px");
    }

    #[test]
    fn computed_style_traversal_is_parent_order_independent_and_bridges_grid_values() {
        let mut parent = node(1, None, "main", "");
        parent.children = vec![2];
        parent.attributes.insert(
            "style".into(),
            "color:blue;display:grid;grid-template-columns:repeat(2,1fr);grid-template-rows:1fr 2fr;grid-auto-flow:column dense;column-gap:7px;row-gap:9px"
                .into(),
        );
        let mut child = node(2, Some(1), "span", "Save");
        child.attributes.insert(
            "style".into(),
            "grid-column-start:2;grid-row-start:1".into(),
        );
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                // Deliberately place the child before its parent. A full style
                // traversal must still see inherited parent ComputedValues.
                nodes: vec![child, parent],
                stylesheets: Vec::new(),
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("Stylo/Taffy grid projection");
        assert_eq!(
            output.computed_styles[&2]["color"], "rgb(0, 0, 255)",
            "{:?}",
            output.computed_styles
        );
        assert_eq!(output.computed_styles[&1]["display"], "grid");
        assert_eq!(output.computed_styles[&1]["grid-template-rows"], "1fr 2fr");
        assert_eq!(output.computed_styles[&2]["grid-column-start"], "2");
        assert!(output.geometry[&2].box_model.is_some());
    }

    #[test]
    fn cascade_respects_importance_property_inheritance_and_nested_variables() {
        let mut root = node(1, None, "main", "");
        root.attributes.insert(
            "style".into(),
            "color:blue;visibility:hidden;--size:41px".into(),
        );
        root.children = vec![2];
        let mut child = node(2, Some(1), "button", "Save");
        child.attributes.insert(
            "style".into(),
            "color:red;visibility:inherit;width:calc(var(--size) + 1px)".into(),
        );
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, child],
                stylesheets: vec!["button { color: black !important }".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("cascade projection");
        assert_eq!(output.computed_styles[&2]["color"], "rgb(0, 0, 0)");
        assert_eq!(output.computed_styles[&2]["visibility"], "hidden");
        assert_eq!(output.computed_styles[&2]["width"], "42px");
    }

    #[test]
    fn implicit_scope_tracks_native_root_identity_and_replacement() {
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                node(1, None, "html", ""),
                node(2, Some(1), "section", ""),
                node(3, Some(2), "button", "Inside"),
                node(4, Some(1), "section", ""),
                node(5, Some(4), "button", "Outside"),
            ],
            stylesheets: vec![
                "button { color: red } @scope { button { color: blue } :scope { z-index: 7 } }"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::from([(0, StylesheetScopeRoot::Element(2))]),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let first = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(first.computed_styles[&3]["color"], "rgb(0, 0, 255)");
        assert_eq!(first.computed_styles[&5]["color"], "rgb(255, 0, 0)");
        assert_eq!(first.computed_styles[&2]["z-index"], "7");
        assert_eq!(first.computed_styles[&4]["z-index"], "auto");
        input
            .stylesheet_scope_roots
            .insert(0, StylesheetScopeRoot::Element(4));
        let moved = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(moved.computed_styles[&3]["color"], "rgb(255, 0, 0)");
        assert_eq!(moved.computed_styles[&5]["color"], "rgb(0, 0, 255)");
        assert_eq!(moved.computed_styles[&2]["z-index"], "auto");
        assert_eq!(moved.computed_styles[&4]["z-index"], "7");
        input.stylesheets[0] = "@scope { button { color: green } }".into();
        input
            .stylesheet_scope_roots
            .insert(0, StylesheetScopeRoot::Constructed);
        let replaced = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(replaced.computed_styles[&3]["color"], "rgb(0, 128, 0)");
        assert_eq!(replaced.computed_styles[&5]["color"], "rgb(0, 128, 0)");
    }

    #[test]
    fn stylesheet_media_changes_invalidate_computed_style() {
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "button", "Save")],
            stylesheets: vec![
                "button { color: red }".into(),
                "button { color: blue }".into(),
            ],
            stylesheet_media: BTreeMap::from([(1, "print".into())]),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let first = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(first.computed_styles[&1]["color"], "rgb(255, 0, 0)");
        input.stylesheet_media.insert(1, "screen".into());
        let second = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(second.computed_styles[&1]["color"], "rgb(0, 0, 255)");
        input.stylesheet_media.insert(1, "not all".into());
        let third = engine.compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(third.computed_styles[&1]["color"], "rgb(255, 0, 0)");
    }

    #[test]
    fn persistent_style_engine_reuses_and_invalidates_stylesheets() {
        let input = |color: &str| LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "button", "Save")],
            stylesheets: vec![format!("button {{ color: {color} }}")],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut engine = StyleEngine::default();
        let first = engine
            .compute(&input("red"), &LayoutLimits::default())
            .expect("first style pass");
        let second = engine
            .compute(&input("blue"), &LayoutLimits::default())
            .expect("invalidated style pass");
        assert_eq!(first.computed_styles[&1]["color"], "rgb(255, 0, 0)");
        assert_eq!(second.computed_styles[&1]["color"], "rgb(0, 0, 255)");
    }

    #[test]
    fn taffy_places_flex_children_on_the_inline_axis() {
        let mut root = node(1, None, "div", "");
        root.attributes
            .insert("style".into(), "display:flex;width:300px;gap:10px".into());
        root.children = vec![2, 3];
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    root,
                    node(2, Some(1), "button", "One"),
                    node(3, Some(1), "button", "Two"),
                ],
                stylesheets: Vec::new(),
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("flex layout");
        let one = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let two = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!(two.x > one.x);
    }

    #[test]
    fn taffy_places_grid_children_by_column_and_row() {
        let mut root = node(1, None, "div", "");
        root.attributes.insert(
            "style".into(),
            "display:grid;grid-template-columns:1fr 1fr;width:300px;gap:8px".into(),
        );
        root.children = vec![2, 3, 4];
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    root,
                    node(2, Some(1), "span", "One"),
                    node(3, Some(1), "span", "Two"),
                    node(4, Some(1), "span", "Three"),
                ],
                stylesheets: vec!["span{height:20px}".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("grid layout");
        let one = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let two = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        let three = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!(
            two.x > one.x,
            "one={one:?} two={two:?} style={:?}",
            output.computed_styles[&1]
        );
        assert!(three.y > one.y, "one={one:?} three={three:?}");
    }

    #[test]
    fn cssom_dimensions_use_untransformed_boxes_and_preserve_non_applicable_values() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3, 4, 5, 6];
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", ""), node(4, Some(1), "div", ""), node(5, Some(1), "div", ""), node(6, Some(1), "span", "")],
            stylesheets: vec!["div { width: 200px } div > div { width: 50%; height: 100px; padding: 5px; border: 2px solid } div > div:nth-child(1) { transform: scale(2) } div > div:nth-child(2) { box-sizing: border-box } div > div:nth-child(3) { visibility: hidden } div > div:nth-child(4) { display: none } span { width: 50%; height: 100px }".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for id in [2, 3, 4] {
            assert_eq!(output.resolved_values[&id]["width"], "100px");
            assert_eq!(output.resolved_values[&id]["height"], "100px");
            assert_eq!(output.computed_styles[&id]["width"], "50%");
        }
        assert_eq!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width,
            228.0
        );
        assert_eq!(output.offset_dimensions[&3], (100.0, 100.0));
        assert_eq!(
            output.geometry[&4].availability,
            GeometryAvailability::Hidden
        );
        assert!(!output.resolved_values.contains_key(&5));
        assert!(!output.resolved_values.contains_key(&6));
    }

    #[test]
    fn flex_measurement_includes_nested_block_and_flex_content_height() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 5];
        let mut nested = node(2, Some(1), "article", "");
        nested.children = vec![3, 4];
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                root,
                nested,
                node(3, Some(2), "div", ""),
                node(4, Some(2), "aside", ""),
                node(5, Some(1), "footer", ""),
            ],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for (nested_css, height, second_y) in [
            ("display:block", 62., 26.),
            ("display:flex;flex-direction:column;gap:4px", 66., 30.),
        ] {
            input.stylesheets = vec![format!(
                "section {{display:flex;flex-direction:column;width:100px}} article {{width:100px;box-sizing:border-box;padding:5px;border:1px solid;{nested_css}}} div {{height:20px}} aside {{height:30px}} footer {{height:10px}}"
            )];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert!(
                (output.offset_dimensions[&2].1 - height).abs() < 0.02,
                "{nested_css}"
            );
            assert!(
                (output.geometry[&5].box_model.as_ref().unwrap().border_box.y - height).abs()
                    < 0.02,
                "{nested_css}"
            );
            assert!(
                (output.geometry[&4].box_model.as_ref().unwrap().border_box.y - second_y).abs()
                    < 0.02,
                "{nested_css}"
            );
            assert!(
                (output.offset_dimensions[&1].1 - height - 10.).abs() < 0.02,
                "{nested_css}"
            );
        }
    }

    #[test]
    fn anonymous_flex_text_items_preserve_order_provenance_and_real_node_outputs() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "b".into(),
            },
        ]);
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", "")],
            stylesheets: vec![
                "section {display:flex;width:200px;line-height:20px} div {width:20px;height:20px}"
                    .into(),
            ],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let child = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!(child.x > 0.);
        assert!(child.y.abs() < 0.02);
        assert!((child.width - 20.).abs() < 0.02);
        assert!(output.text_clusters[&10][0].rect.x.abs() < 0.02);
        assert!((output.text_clusters[&11][0].rect.x - child.right()).abs() < 0.02);
        assert!(
            output
                .text_clusters
                .values()
                .flatten()
                .all(|cluster| cluster.layout_node == 1)
        );
        assert_eq!(
            output.geometry.keys().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            output.computed_styles.keys().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            output.text_fragments[&1]
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        input.nodes[0].content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: " \n".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "\t ".into(),
            },
        ]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .x
                .abs()
                < 0.02
        );
        assert!(!output.text_clusters.contains_key(&10));
        assert!(!output.text_clusters.contains_key(&11));
    }

    #[test]
    fn contents_boxes_promote_flex_children_without_edges_or_own_geometry() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut wrapper = node(2, Some(1), "div", "");
        wrapper.children = vec![3, 4];
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),
            anchor_parents: Default::default(), rendered_tree:None,
            nodes:vec![root,wrapper,node(3,Some(2),"span",""),node(4,Some(2),"span","")],
            stylesheets:vec!["section {display:flex;width:200px} div {display:contents;padding:50px;border:10px solid} span {display:block;width:20px;height:10px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),
            custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),
            color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&2], (0., 0.));
        assert!(!output.geometry.contains_key(&2));
        assert_eq!(output.offset_dimensions[&3], (20., 10.));
        assert_eq!(output.offset_dimensions[&4], (20., 10.));
        assert!(
            output.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .x
                .abs()
                < 0.02
        );
        assert!((output.geometry[&4].box_model.as_ref().unwrap().border_box.x - 20.).abs() < 0.02);
        for exclusion in [
            "section{display:none}",
            "section{content-visibility:hidden}",
            "div{visibility:hidden}",
        ] {
            let mut hidden = input.clone();
            hidden.stylesheets.push(exclusion.into());
            let hidden = compute(&hidden, &LayoutLimits::default()).unwrap();
            assert_eq!(hidden.offset_dimensions[&2], (0., 0.));
            assert_eq!(
                hidden.geometry[&2].availability,
                GeometryAvailability::Hidden,
                "{exclusion}"
            );
        }
    }

    #[test]
    fn promoted_contents_text_retains_source_identity_and_style_owner() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        let mut wrapper = node(2, Some(1), "span", "a");
        wrapper.content = Some(vec![LayoutContent::Text {
            node_id: 20,
            text: "a".into(),
        }]);
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, wrapper],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = BTreeMap::from([
            (1, Arc::new(BTreeMap::new())),
            (
                2,
                Arc::new(BTreeMap::from([
                    ("display".into(), "contents".into()),
                    ("font-size".into(), "32px".into()),
                ])),
            ),
        ]);
        let flattened = flatten_contents_boxes(&input, &styles);
        assert_eq!(flattened.nodes.len(), 1);
        let source = text_source::TextSource::direct(&flattened.nodes[0]);
        assert!(
            source
                .spans
                .iter()
                .any(|span| span.owner == 2 && span.text_node == Some(20))
        );
        let mut engine = TextEngine::new(&[]);
        let shaped = engine.shape_source(
            &source,
            200.,
            16.,
            &BTreeMap::new(),
            ViewportState::default(),
            &styles,
        );
        assert!(
            shaped
                .source_clusters
                .iter()
                .any(|cluster| cluster.owner == 2 && cluster.text_node == 20)
        );
        let plain = engine.shape("a", 200., 16., DEFAULT_FONT_FAMILY);
        assert!(shaped.width > plain.width * 1.9);
    }

    #[test]
    fn rendered_tree_validation_rejects_unknown_duplicate_and_cyclic_participants() {
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "div", ""), node(2, Some(1), "span", "")],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for tree in [
            BTreeMap::from([(99, Vec::new())]),
            BTreeMap::from([(1, vec![LayoutContent::Element(2)])]),
            BTreeMap::from([
                (
                    1,
                    vec![LayoutContent::Element(2), LayoutContent::Element(2)],
                ),
                (2, Vec::new()),
            ]),
            BTreeMap::from([
                (1, vec![LayoutContent::Element(2)]),
                (2, vec![LayoutContent::Element(1)]),
            ]),
            BTreeMap::from([(1, vec![LayoutContent::Element(1)])]),
        ] {
            input.rendered_tree = Some(tree);
            assert!(matches!(
                validate(&input, &LayoutLimits::default()),
                Err(LayoutError::InvalidRenderedTree(_))
            ));
        }
        input.rendered_tree = Some(BTreeMap::from([
            (1, vec![LayoutContent::Element(2)]),
            (2, Vec::new()),
        ]));
        assert!(validate(&input, &LayoutLimits::default()).is_ok());
        input.rendered_tree = Some(BTreeMap::new());
        assert!(validate(&input, &LayoutLimits::default()).is_ok());
    }

    #[test]
    fn rendered_tree_changes_geometry_without_changing_selector_ancestry() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3, 4];
        let child = node(2, Some(1), "span", "text");
        let container = node(3, Some(1), "div", "");
        let excluded = node(4, Some(1), "aside", "excluded");
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: Some(BTreeMap::from([
                (1, vec![LayoutContent::Element(3)]),
                (3, vec![LayoutContent::Element(2)]),
                (2, vec![LayoutContent::Text {node_id:20,text:"text".into()}]),
            ])),
            nodes:vec![root,child,container,excluded],
            stylesheets:vec!["section > span {display:block;width:40px;height:10px;color:red} div {padding-left:10px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),
            custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),
            color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&2], (40., 10.));
        assert!((output.geometry[&2].box_model.as_ref().unwrap().border_box.x - 10.).abs() < 0.02);
        assert!(!output.geometry.contains_key(&4));
        assert!(!output.offset_dimensions.contains_key(&4));
        assert!(output.computed_styles.contains_key(&4));
        assert_eq!(input.nodes[1].parent, Some(1));
        let first_styles = output.computed_styles[&2].clone();
        let first = input.clone();
        let tree = input.rendered_tree.as_mut().unwrap();
        tree.insert(
            1,
            vec![LayoutContent::Element(2), LayoutContent::Element(3)],
        );
        tree.insert(3, Vec::new());
        assert_ne!(input, first);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2], first_styles);
        assert!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .x
                .abs()
                < 0.02
        );
    }

    #[test]
    fn flex_order_is_stable_and_hidden_children_do_not_consume_space() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3, 4, 5];
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", ""), node(4, Some(1), "div", ""), node(5, Some(1), "div", "")],
            stylesheets: vec!["section {display:flex;width:300px;gap:5px} div {width:40px;height:20px} div:nth-child(1) {order:2} div:nth-child(2) {order:-1} div:nth-child(3) {order:-1} div:nth-child(4) {display:none;width:200px;order:-2}".into()],
            stylesheet_media: BTreeMap::new(), selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (id, expected_x) in [(3, 0.), (4, 45.), (2, 90.)] {
            let rect = output.geometry[&id].box_model.as_ref().unwrap().border_box;
            assert!((rect.x - expected_x).abs() < 0.02, "node {id}: {rect:?}");
        }
        assert_eq!(output.offset_dimensions[&5], (0., 0.));
        assert_eq!(input.nodes[0].children, vec![2, 3, 4, 5]);
        input.stylesheets.push("div:nth-child(1) {order:-3}".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .x
                .abs()
                < 0.02
        );
        assert!((output.geometry[&3].box_model.as_ref().unwrap().border_box.x - 45.).abs() < 0.02);
    }

    #[test]
    fn flex_direction_alignment_and_wrapping_reach_geometry() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![
                root,
                node(2, Some(1), "div", ""),
                node(3, Some(1), "div", ""),
            ],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for (direction, first, second) in [
            ("row", (0., 40.), (160., 40.)),
            ("row-reverse", (160., 40.), (0., 40.)),
            ("column", (80., 0.), (80., 80.)),
            ("column-reverse", (80., 80.), (80., 0.)),
        ] {
            input.stylesheets = vec![format!(
                "section {{display:flex;width:200px;height:100px;flex-direction:{direction};justify-content:space-between;align-items:center}} div {{width:40px;height:20px}}"
            )];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            for (id, expected) in [(2, first), (3, second)] {
                let rect = output.geometry[&id].box_model.as_ref().unwrap().border_box;
                assert!(
                    (rect.x - expected.0).abs() < 0.02,
                    "{direction} node {id}: {rect:?}"
                );
                assert!(
                    (rect.y - expected.1).abs() < 0.02,
                    "{direction} node {id}: {rect:?}"
                );
            }
        }
        input.stylesheets = vec!["section {display:flex;width:100px;height:100px;flex-wrap:wrap;align-content:flex-start;gap:5px} div {width:60px;height:20px}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let second = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!(second.x.abs() < 0.02);
        assert!((second.y - 25.).abs() < 0.02);
    }

    #[test]
    fn flex_allocated_sizes_and_edges_reach_native_geometry() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", "")],
            stylesheets: vec!["section {display:flex;width:200px} div {width:40px;height:10px;flex-grow:1;padding:5px;border:2px solid;margin:3px}".into()],
            stylesheet_media: BTreeMap::new(), selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for (id, x) in [(2, 3.), (3, 103.)] {
            let rect = output.geometry[&id].box_model.as_ref().unwrap().border_box;
            assert!((rect.x - x).abs() < 0.02);
            assert!((rect.y - 3.).abs() < 0.02);
            assert!((rect.width - 94.).abs() < 0.02);
            assert!((rect.height - 24.).abs() < 0.02);
            assert_eq!(output.resolved_values[&id]["width"], "80px");
        }
        assert!((output.offset_dimensions[&1].1 - 30.).abs() < 0.02);
        input.stylesheets = vec![
            "section {display:flex;width:100px} div {width:80px;height:10px;min-width:0}".into(),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!((output.offset_dimensions[&2].0 - 50.).abs() < 0.02);
        assert!((output.offset_dimensions[&3].0 - 50.).abs() < 0.02);
        assert!((output.geometry[&3].box_model.as_ref().unwrap().border_box.x - 50.).abs() < 0.02);
    }

    #[test]
    fn grid_tracks_preserve_lengths_fractions_repeats_and_implicit_rows() {
        // Explicit geometric expectations distinguish real track sizing from
        // equal fractions, token counts, and flattening repeat() to one track.
        for (template, expected) in [
            ("40px 1fr 2fr", vec![(0., 40.), (40., 120.), (160., 240.)]),
            ("0 1fr 1fr", vec![(0., 0.), (0., 200.), (200., 200.)]),
            (
                "minmax(0, 1fr) minmax(0, 2fr) 100px",
                vec![(0., 100.), (100., 200.), (300., 100.)],
            ),
            ("2em 1fr 1fr", vec![(0., 32.), (32., 184.), (216., 184.)]),
            (
                "calc(2em + 8px) 1fr 2fr",
                vec![(0., 40.), (40., 120.), (160., 240.)],
            ),
            (
                "min(25%, 80px) 1fr 1fr",
                vec![(0., 80.), (80., 160.), (240., 160.)],
            ),
            (
                "minmax(calc(10% + 10px), 1fr) 100px 200px",
                vec![(0., 100.), (100., 100.), (200., 200.)],
            ),
            (
                "repeat(3, minmax(0, 1fr))",
                vec![
                    (0., 400. / 3.),
                    (400. / 3., 400. / 3.),
                    (800. / 3., 400. / 3.),
                ],
            ),
            (
                "25% minmax(20px, 1fr) 2fr",
                vec![(0., 100.), (100., 100.), (200., 200.)],
            ),
            (
                "repeat(2, 30px 70px)",
                vec![(0., 30.), (30., 70.), (100., 30.)],
            ),
            (
                "repeat(auto-fill, 100px)",
                vec![(0., 100.), (100., 100.), (200., 100.)],
            ),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2, 3, 4, 5, 6];
            let mut all_nodes = vec![root];
            all_nodes.extend((2..=6).map(|id| node(id, Some(1), "div", "")));
            let nodes = all_nodes.iter().map(|node| (node.id, node)).collect();
            let parent = BTreeMap::from([
                ("display".into(), "grid".into()),
                ("grid-template-columns".into(), template.into()),
                ("grid-auto-rows".into(), "25px 35px".into()),
            ]);
            let mut styles = BTreeMap::from([(1, Arc::new(parent.clone()))]);
            styles.extend((2..=6).map(|id| (id, Arc::new(BTreeMap::new()))));
            let mut engine = TextEngine::new(&[]);
            let placements = taffy_child_placements(ChildPlacementInputs {
                node: &all_nodes[0],
                parent_style: &parent,
                styles: &styles,
                width: 400.,
                height: None,
                viewport: ViewportState::default(),
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap();
            for (index, (x, width)) in expected.iter().enumerate() {
                assert!(
                    (placements[index].x - x).abs() < 0.02,
                    "{template}: item {index}"
                );
                assert!(
                    (placements[index].width - width).abs() < 0.02,
                    "{template}: item {index}"
                );
                assert!((placements[index].height - 25.).abs() < 0.02);
            }
            assert!((placements[4].y - 25.).abs() < 0.02, "{template}");
            assert!((placements[4].height - 35.).abs() < 0.02, "{template}");
        }
    }

    #[test]
    fn grid_area_strings_validate_rectangles_and_preserve_empty_cells() {
        let template = parse_grid_template_areas(
            r#""... ... ..." "side main main" "side main main" "... ... ...""#,
        )
        .unwrap();
        assert_eq!((template.row_count, template.column_count), (4, 3));
        let main = template
            .areas
            .iter()
            .find(|area| area.name == "main")
            .unwrap();
        assert_eq!(
            (
                main.row_start,
                main.row_end,
                main.column_start,
                main.column_end
            ),
            (2, 4, 2, 4)
        );
        let compact = parse_grid_template_areas(r#""left...right""#).unwrap();
        assert_eq!(compact.column_count, 3);
        let escaped = parse_grid_template_areas(r#""\6d ain main""#).unwrap();
        assert_eq!(escaped.areas[0].name, "main");
        assert_eq!(escaped.areas[0].column_end, 3);
        assert_eq!(
            parse_grid_template_areas(r#""...""#).unwrap().areas.len(),
            0
        );
        for invalid in [
            "none",
            "",
            "main",
            r#""""#,
            r#""a a" "a .""#,
            r#""a . a""#,
            r#""a a" "b""#,
            r#""a/a""#,
            r#""a" junk"#,
        ] {
            assert!(parse_grid_template_areas(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn named_grid_areas_place_items_and_supply_implicit_boundary_lines() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let parent = BTreeMap::from([
            ("display".into(), "grid".into()),
            ("grid-template-columns".into(), "40px 60px 80px".into()),
            ("grid-template-rows".into(), "20px 30px 40px".into()),
            (
                "grid-template-areas".into(),
                r#""head head head" "side main main" "side main main""#.into(),
            ),
        ]);
        for (start, end) in [("main", "main"), ("main-start", "main-end")] {
            let child_style = BTreeMap::from([
                ("grid-column-start".into(), start.into()),
                ("grid-column-end".into(), end.into()),
                ("grid-row-start".into(), start.into()),
                ("grid-row-end".into(), end.into()),
            ]);
            let styles =
                BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
            let mut engine = TextEngine::new(&[]);
            let placements = taffy_child_placements(ChildPlacementInputs {
                node: &root,
                parent_style: &parent,
                styles: &styles,
                width: 180.,
                height: None,
                viewport: ViewportState::default(),
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap();
            let rect = &placements[0];
            assert!((rect.x - 40.).abs() < 0.02, "{start}");
            assert!((rect.y - 20.).abs() < 0.02, "{start}");
            assert!((rect.width - 140.).abs() < 0.02, "{start}");
            assert!((rect.height - 70.).abs() < 0.02, "{start}");
        }
    }

    #[test]
    fn grid_named_lines_and_spans_select_tracks() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let parent = BTreeMap::from([
            ("display".into(), "grid".into()),
            (
                "grid-template-columns".into(),
                "[first] 40px [middle] 60px [last] 80px [end]".into(),
            ),
            ("grid-template-rows".into(), "30px".into()),
        ]);
        for (start, end, x, width) in [
            ("middle", "end", 40., 140.),
            ("2", "span 2", 40., 140.),
            ("-3", "-1", 40., 140.),
            ("first", "last", 0., 100.),
        ] {
            let child_style = BTreeMap::from([
                ("grid-column-start".into(), start.into()),
                ("grid-column-end".into(), end.into()),
            ]);
            let styles =
                BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
            let mut engine = TextEngine::new(&[]);
            let placements = taffy_child_placements(ChildPlacementInputs {
                node: &root,
                parent_style: &parent,
                styles: &styles,
                width: 180.,
                height: None,
                viewport: ViewportState::default(),
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap();
            assert!((placements[0].x - x).abs() < 0.02, "{start} / {end}");
            assert!(
                (placements[0].width - width).abs() < 0.02,
                "{start} / {end}"
            );
            assert!((placements[0].height - 30.).abs() < 0.02);
        }
    }

    #[test]
    fn canvas_fallback_has_no_layout_boxes_or_painted_text() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 4];
        let mut canvas = node(2, Some(1), "canvas", "Fallback text");
        canvas.children = vec![3];
        canvas.intrinsic_image_size = Some((300, 150));
        canvas.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "Fallback text".into(),
            },
            LayoutContent::Element(3),
        ]);
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, canvas, node(3, Some(2), "div", "Fallback child"), node(4, Some(1), "footer", "")],
            stylesheets: vec!["section {width:400px} canvas {display:block} canvas > div {height:500px;color:red} footer {height:10px}".into()],
            stylesheet_media: BTreeMap::new(), selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&2], (300., 150.));
        assert_eq!(output.offset_dimensions[&3], (0., 0.));
        assert!(output.geometry[&3].box_model.is_none());
        assert!(output.text_fragments.get(&2).is_none_or(Vec::is_empty));
        assert!(output.text_fragments.get(&3).is_none_or(Vec::is_empty));
        assert_eq!(output.computed_styles[&3]["height"], "500px");
        assert_eq!(input.nodes[1].children, vec![3]);
        let footer = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!((footer.y - 150.).abs() < 0.02);
    }

    #[test]
    fn ordinary_image_layout_uses_preferred_ratio_after_width_constraints() {
        for (rule, expected_width, expected_height) in [
            ("width:120px", 120., 40.),
            ("width:120px;aspect-ratio:2 / 1", 120., 60.),
            ("height:40px;aspect-ratio:2 / 1", 80., 40.),
            ("height:40px;aspect-ratio:auto 2 / 1", 120., 40.),
            ("max-width:90px", 90., 30.),
            ("max-width:90px;aspect-ratio:2 / 1", 90., 45.),
            ("max-height:50px", 150., 50.),
            ("min-height:200px", 600., 200.),
            ("max-width:200px;max-height:50px", 150., 50.),
            ("min-width:400px;min-height:200px", 600., 200.),
            ("max-width:200px;min-height:100px", 200., 100.),
            ("min-width:400px;max-height:100px", 400., 100.),
            ("width:120px;height:25px;aspect-ratio:2 / 1", 120., 25.),
            (
                "width:120px;box-sizing:border-box;padding:10px;border:2px solid;aspect-ratio:2 / 1",
                120.,
                60.,
            ),
            (
                "width:120px;box-sizing:border-box;padding:10px;border:2px solid;aspect-ratio:auto 2 / 1",
                120.,
                56.,
            ),
            (
                "height:60px;box-sizing:border-box;padding:10px;border:2px solid;aspect-ratio:2 / 1",
                120.,
                60.,
            ),
            (
                "height:60px;box-sizing:border-box;padding:10px;border:2px solid;aspect-ratio:auto 2 / 1",
                132.,
                60.,
            ),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2, 3];
            let mut image = node(2, Some(1), "img", "");
            image.intrinsic_image_size = Some((300, 100));
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, image, node(3, Some(1), "footer", "")],
                stylesheets: vec![format!(
                    "section {{width:400px}} img {{display:block;{rule}}} footer {{height:10px}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            assert!(
                (rect.width - expected_width).abs() < 0.02,
                "{rule}: width {}",
                rect.width
            );
            assert!(
                (rect.height - expected_height).abs() < 0.02,
                "{rule}: height {}",
                rect.height
            );
            let footer = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            assert!(
                (footer.y - expected_height).abs() < 0.02,
                "{rule}: footer {}",
                footer.y
            );
        }
    }

    #[test]
    fn normal_blocks_resolve_auto_margins_and_rtl_width_overconstraints() {
        for (direction, rule, expected_x, expected_width) in [
            (
                "ltr",
                "width:80px;margin-left:auto;margin-right:auto",
                60.,
                80.,
            ),
            (
                "ltr",
                "width:80px;margin-left:auto;margin-right:10px",
                110.,
                80.,
            ),
            (
                "ltr",
                "width:auto;max-width:80px;margin-left:auto;margin-right:auto",
                60.,
                80.,
            ),
            ("rtl", "width:80px", 120., 80.),
            (
                "ltr",
                "width:240px;margin-left:auto;margin-right:auto",
                0.,
                240.,
            ),
            (
                "rtl",
                "width:240px;margin-left:auto;margin-right:auto",
                -40.,
                240.,
            ),
            (
                "ltr",
                "width:50%;margin-left:20px;margin-right:20px",
                20.,
                100.,
            ),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2];
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, node(2, Some(1), "div", "")],
                stylesheets: vec![format!(
                    "section {{width:200px;direction:{direction}}} div {{height:20px;{rule}}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            assert!(
                (rect.x - expected_x).abs() < 0.02,
                "{direction} {rule}: x {}",
                rect.x
            );
            assert!(
                (rect.width - expected_width).abs() < 0.02,
                "{direction} {rule}: width {}",
                rect.width
            );
        }
    }

    #[test]
    fn relative_offsets_resolve_opposite_insets_without_moving_siblings() {
        for (parent_rule, child_rule, expected_x, expected_y) in [
            ("height:100px", "right:20px;bottom:10px", -20., -10.),
            ("height:100px", "left:10%;top:50%", 20., 50.),
            ("height:auto", "left:-10px;top:50%", -10., 0.),
            (
                "height:100px;direction:ltr",
                "left:10px;right:30px;top:10px;bottom:40px",
                10.,
                10.,
            ),
            (
                "height:100px;direction:rtl",
                "direction:ltr;left:10px;right:30px",
                130.,
                0.,
            ),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2, 3];
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    root,
                    node(2, Some(1), "div", ""),
                    node(3, Some(1), "footer", ""),
                ],
                stylesheets: vec![format!(
                    "section {{width:200px;{parent_rule}}} div {{position:relative;width:40px;height:20px;{child_rule}}} footer {{height:10px}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            assert!(
                (rect.x - expected_x).abs() < 0.02,
                "{parent_rule};{child_rule}: x {}",
                rect.x
            );
            assert!(
                (rect.y - expected_y).abs() < 0.02,
                "{parent_rule};{child_rule}: y {}",
                rect.y
            );
            let footer = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            assert!(
                (footer.y - 20.).abs() < 0.02,
                "relative position must preserve normal flow"
            );
        }
    }

    #[test]
    fn flex_intrinsic_measurement_and_nested_block_height_constraints_agree() {
        for (rule, expected) in [
            ("min-height:80px", 80.),
            ("height:100px;max-height:30px", 30.),
            ("height:100px;max-height:30px;min-height:50px", 50.),
            (
                "height:100px;max-height:30px;box-sizing:border-box;padding:5px;border:1px solid",
                30.,
            ),
            ("height:50%", 0.),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2, 4];
            let mut wrapper = node(2, Some(1), "article", "");
            wrapper.children = vec![3];
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    root,
                    wrapper,
                    node(3, Some(2), "div", ""),
                    node(4, Some(1), "footer", ""),
                ],
                stylesheets: vec![format!(
                    "section {{display:flex;flex-direction:column;width:200px}} div {{{rule}}} footer {{height:10px}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert!(
                (output.offset_dimensions[&2].1 - expected).abs() < 0.02,
                "{rule}: wrapper"
            );
            assert!(
                (output.offset_dimensions[&3].1 - expected).abs() < 0.02,
                "{rule}: child"
            );
            let footer = output.geometry[&4].box_model.as_ref().unwrap().border_box;
            assert!(
                (footer.y - expected).abs() < 0.02,
                "{rule}: footer {}",
                footer.y
            );
        }
    }

    #[test]
    fn block_percentage_heights_use_definite_content_boxes_and_height_bounds() {
        for (parent_rule, expected_parent, expected_child) in [
            ("height:50%", 200., 100.),
            ("height:200px;max-height:100px", 100., 50.),
            ("height:200px;min-height:120px;max-height:80px", 120., 60.),
            (
                "height:100px;box-sizing:border-box;padding:5px;border:1px solid",
                100.,
                44.,
            ),
            ("height:auto;min-height:100px", 100., 0.),
        ] {
            let mut root = node(1, None, "section", "");
            root.children = vec![2];
            let mut parent = node(2, Some(1), "div", "");
            parent.children = vec![3];
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, parent, node(3, Some(2), "aside", "")],
                stylesheets: vec![format!(
                    "section {{height:400px;width:200px}} div {{{parent_rule}}} aside {{height:50%}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert!(
                (output.offset_dimensions[&2].1 - expected_parent).abs() < 0.02,
                "{parent_rule}: parent {}",
                output.offset_dimensions[&2].1
            );
            assert!(
                (output.offset_dimensions[&3].1 - expected_child).abs() < 0.02,
                "{parent_rule}: child {}",
                output.offset_dimensions[&3].1
            );
        }
    }

    #[test]
    fn percentage_gaps_use_width_and_height_on_their_respective_axes() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3, 4, 5];
        let children = (2..=5)
            .map(|id| node(id, Some(1), "div", ""))
            .collect::<Vec<_>>();
        let mut nodes = BTreeMap::from([(1, &root)]);
        nodes.extend(children.iter().map(|child| (child.id, child)));
        for display in ["flex", "grid"] {
            for gap in ["10%", "calc(10% + 0px)"] {
                let parent = BTreeMap::from([
                    ("display".into(), display.into()),
                    ("flex-wrap".into(), "wrap".into()),
                    ("align-content".into(), "start".into()),
                    ("align-items".into(), "start".into()),
                    ("grid-template-columns".into(), "80px 80px".into()),
                    ("grid-template-rows".into(), "20px 20px".into()),
                    ("column-gap".into(), gap.into()),
                    ("row-gap".into(), gap.into()),
                ]);
                let child_style = Arc::new(BTreeMap::from([
                    ("width".into(), "80px".into()),
                    ("height".into(), "20px".into()),
                    ("flex-shrink".into(), "0".into()),
                ]));
                let mut styles = BTreeMap::from([(1, Arc::new(parent.clone()))]);
                styles.extend((2..=5).map(|id| (id, child_style.clone())));
                let mut engine = TextEngine::new(&[]);
                let placements = taffy_child_placements(ChildPlacementInputs {
                    node: &root,
                    parent_style: &parent,
                    styles: &styles,
                    width: 200.,
                    height: Some(100.),
                    viewport: ViewportState::default(),
                    nodes: &nodes,
                    text_engine: &mut engine,
                })
                .unwrap();
                for (placement, (x, y)) in
                    placements
                        .iter()
                        .zip([(0., 0.), (100., 0.), (0., 30.), (100., 30.)])
                {
                    assert!(
                        (placement.x - x).abs() < 0.02,
                        "{display} {gap}: x {}",
                        placement.x
                    );
                    assert!(
                        (placement.y - y).abs() < 0.02,
                        "{display} {gap}: y {}",
                        placement.y
                    );
                }
            }
        }
    }

    #[test]
    fn flex_grid_scrollable_items_can_shrink_below_content_minimum() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "unbreakablelongwordunbreakablelongword");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        for display in ["flex", "grid"] {
            let parent = BTreeMap::from([
                ("display".into(), display.into()),
                ("grid-template-columns".into(), "1fr".into()),
            ]);
            for overflow in ["visible", "clip", "hidden", "auto", "scroll"] {
                for minimum in ["auto", "90px"] {
                    let child_style = BTreeMap::from([
                        ("overflow-x".into(), overflow.into()),
                        ("overflow-y".into(), overflow.into()),
                        ("min-width".into(), minimum.into()),
                    ]);
                    let styles =
                        BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
                    let mut engine = TextEngine::new(&[]);
                    let placements = taffy_child_placements(ChildPlacementInputs {
                        node: &root,
                        parent_style: &parent,
                        styles: &styles,
                        width: 60.,
                        height: None,
                        viewport: ViewportState::default(),
                        nodes: &nodes,
                        text_engine: &mut engine,
                    })
                    .unwrap();
                    let actual = placements[0].width;
                    if minimum == "90px" {
                        assert!(
                            (actual - 90.).abs() < 0.02,
                            "{display} {overflow}: {actual}"
                        );
                    } else if matches!(overflow, "visible" | "clip") {
                        assert!(actual > 90., "{display} {overflow}: {actual}");
                    } else {
                        assert!(
                            (actual - 60.).abs() < 0.02,
                            "{display} {overflow}: {actual}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn flex_content_basis_ignores_declared_main_size_and_keeps_cross_size() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut image = node(2, Some(1), "img", "");
        image.intrinsic_image_size = Some((60, 30));
        let nodes = BTreeMap::from([(1, &root), (2, &image)]);
        for direction in ["row", "row-reverse", "column", "column-reverse"] {
            let column = direction.starts_with("column");
            let parent = BTreeMap::from([
                ("display".into(), "flex".into()),
                ("flex-direction".into(), direction.into()),
                ("align-items".into(), "start".into()),
            ]);
            for basis in ["content", "auto"] {
                let child_style = BTreeMap::from([
                    ("width".into(), if column { "60px" } else { "200px" }.into()),
                    (
                        "height".into(),
                        if column { "200px" } else { "auto" }.into(),
                    ),
                    ("flex-basis".into(), basis.into()),
                    ("flex-shrink".into(), "0".into()),
                    ("min-width".into(), "0px".into()),
                    ("min-height".into(), "0px".into()),
                ]);
                let styles =
                    BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
                let mut engine = TextEngine::new(&[]);
                let placements = taffy_child_placements(ChildPlacementInputs {
                    node: &root,
                    parent_style: &parent,
                    styles: &styles,
                    width: 400.,
                    height: Some(400.),
                    viewport: ViewportState::default(),
                    nodes: &nodes,
                    text_engine: &mut engine,
                })
                .unwrap();
                let actual = if column {
                    placements[0].height
                } else {
                    placements[0].width
                };
                let expected = if basis == "auto" {
                    200.
                } else if column {
                    30.
                } else {
                    60.
                };
                assert!(
                    (actual - expected).abs() < 0.02,
                    "{direction} {basis}: {actual}"
                );
            }
        }
    }

    #[test]
    fn flex_content_basis_includes_box_edges_for_both_box_sizing_modes() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "x");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let parent = BTreeMap::from([
            ("display".into(), "flex".into()),
            ("flex-direction".into(), "column".into()),
        ]);
        for sizing in ["content-box", "border-box"] {
            let child_style = BTreeMap::from([
                ("width".into(), "100px".into()),
                ("height".into(), "200px".into()),
                ("line-height".into(), "20px".into()),
                ("flex-basis".into(), "content".into()),
                ("box-sizing".into(), sizing.into()),
                ("min-height".into(), "0px".into()),
                ("padding-top".into(), "5px".into()),
                ("padding-bottom".into(), "5px".into()),
                ("border-top-width".into(), "1px".into()),
                ("border-bottom-width".into(), "1px".into()),
                ("border-top-style".into(), "solid".into()),
                ("border-bottom-style".into(), "solid".into()),
            ]);
            let styles =
                BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
            let mut engine = TextEngine::new(&[]);
            let placements = taffy_child_placements(ChildPlacementInputs {
                node: &root,
                parent_style: &parent,
                styles: &styles,
                width: 200.,
                height: Some(400.),
                viewport: ViewportState::default(),
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap();
            assert!(
                (placements[0].height - 32.).abs() < 0.02,
                "{sizing}: {}",
                placements[0].height
            );
        }
    }

    #[test]
    fn preferred_ratios_distinguish_authored_and_natural_image_sizes() {
        let mut image = node(2, Some(1), "img", "");
        image.intrinsic_image_size = Some((300, 100));
        for (value, expected) in [
            ("auto", Some(3.)),
            ("2 / 1", Some(2.)),
            ("auto 2 / 1", Some(3.)),
            ("2 / 1 auto", Some(3.)),
            ("0 / 1", Some(3.)),
            ("1 / 0", Some(3.)),
            ("-1 / 2", None),
            ("2 / 1 extra", None),
        ] {
            let style = BTreeMap::from([("aspect-ratio".into(), value.into())]);
            assert_eq!(preferred_aspect_ratio(&image, &style), expected, "{value}");
        }
        image.intrinsic_image_size = None;
        let style = BTreeMap::from([("aspect-ratio".into(), "auto 2 / 1".into())]);
        assert_eq!(preferred_aspect_ratio(&image, &style), Some(2.));
        let style = BTreeMap::from([("aspect-ratio".into(), "0 / 0".into())]);
        assert_eq!(preferred_aspect_ratio(&image, &style), None);
    }

    #[test]
    fn flex_grid_transfers_aspect_ratio_between_definite_and_auto_axes() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        for display in ["flex", "grid"] {
            let parent = BTreeMap::from([
                ("display".into(), display.into()),
                ("align-items".into(), "start".into()),
                ("justify-items".into(), "start".into()),
                ("grid-template-columns".into(), "400px".into()),
                ("grid-template-rows".into(), "200px".into()),
            ]);
            for (image_size, ratio, width, height, expected_width, expected_height) in [
                (None, "2 / 1", "120px", "auto", 120., 60.),
                (None, "2 / 1", "auto", "40px", 80., 40.),
                (Some((300, 100)), "auto", "auto", "40px", 120., 40.),
                (Some((300, 100)), "2 / 1", "auto", "40px", 80., 40.),
                (Some((300, 100)), "auto 2 / 1", "auto", "40px", 120., 40.),
                (None, "2 / 1", "90px", "40px", 90., 40.),
            ] {
                let mut child = node(
                    2,
                    Some(1),
                    if image_size.is_some() { "img" } else { "div" },
                    "",
                );
                child.intrinsic_image_size = image_size;
                let nodes = BTreeMap::from([(1, &root), (2, &child)]);
                let child_style = BTreeMap::from([
                    ("width".into(), width.into()),
                    ("height".into(), height.into()),
                    ("aspect-ratio".into(), ratio.into()),
                    ("min-width".into(), "0px".into()),
                    ("flex-shrink".into(), "0".into()),
                ]);
                let styles =
                    BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
                let mut engine = TextEngine::new(&[]);
                let placements = taffy_child_placements(ChildPlacementInputs {
                    node: &root,
                    parent_style: &parent,
                    styles: &styles,
                    width: 400.,
                    height: Some(200.),
                    viewport: ViewportState::default(),
                    nodes: &nodes,
                    text_engine: &mut engine,
                })
                .unwrap();
                assert!(
                    (placements[0].width - expected_width).abs() < 0.02,
                    "{display} {ratio} {width}/{height}: width {}",
                    placements[0].width
                );
                assert!(
                    (placements[0].height - expected_height).abs() < 0.02,
                    "{display} {ratio} {width}/{height}: height {}",
                    placements[0].height
                );
            }
        }
    }

    #[test]
    fn grid_percentage_width_constraints_use_the_item_area() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let parent = BTreeMap::from([
            ("display".into(), "grid".into()),
            ("grid-template-columns".into(), "80px 320px".into()),
            ("grid-template-rows".into(), "30px".into()),
        ]);
        for (width, minimum, maximum, expected) in [
            ("50%", "0px", "none", 40.),
            ("10px", "50%", "none", 40.),
            ("100px", "0px", "75%", 60.),
            ("10px", "75%", "50%", 60.),
        ] {
            let child_style = BTreeMap::from([
                ("width".into(), width.into()),
                ("min-width".into(), minimum.into()),
                ("max-width".into(), maximum.into()),
            ]);
            let styles =
                BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
            let mut engine = TextEngine::new(&[]);
            let placements = taffy_child_placements(ChildPlacementInputs {
                node: &root,
                parent_style: &parent,
                styles: &styles,
                width: 400.,
                height: None,
                viewport: ViewportState::default(),
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap();
            assert!(
                (placements[0].width - expected).abs() < 0.02,
                "{width}, min={minimum}, max={maximum}: {}",
                placements[0].width
            );
        }
    }

    #[test]
    fn flex_basis_percentages_and_math_use_the_main_axis() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        for direction in ["row", "row-reverse", "column", "column-reverse"] {
            let column = direction.starts_with("column");
            let parent = BTreeMap::from([
                ("display".into(), "flex".into()),
                ("flex-direction".into(), direction.into()),
            ]);
            for (basis, addition) in [("25%", 0.), ("calc(25% + 10px)", 10.)] {
                let child_style = BTreeMap::from([
                    ("width".into(), "20px".into()),
                    ("height".into(), "20px".into()),
                    ("flex-basis".into(), basis.into()),
                    ("flex-shrink".into(), "0".into()),
                ]);
                let styles =
                    BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
                let mut engine = TextEngine::new(&[]);
                let placements = taffy_child_placements(ChildPlacementInputs {
                    node: &root,
                    parent_style: &parent,
                    styles: &styles,
                    width: 400.,
                    height: Some(200.),
                    viewport: ViewportState::default(),
                    nodes: &nodes,
                    text_engine: &mut engine,
                })
                .unwrap();
                let actual = if column {
                    placements[0].height
                } else {
                    placements[0].width
                };
                let expected = if column { 50. } else { 100. } + addition;
                assert!(
                    (actual - expected).abs() < 0.02,
                    "{direction}: {basis}: {actual}"
                );
            }
        }
    }

    #[test]
    fn flex_grid_height_bounds_and_percentage_basis_reach_layout() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        for display in ["flex", "grid"] {
            let parent = BTreeMap::from([
                ("display".into(), display.into()),
                ("align-items".into(), "start".into()),
                ("grid-template-columns".into(), "100px".into()),
                ("grid-template-rows".into(), "80px".into()),
            ]);
            for (height, minimum, maximum, sizing, expected) in [
                ("10px", "30px", "none", "content-box", 42.),
                ("100px", "0px", "40px", "content-box", 52.),
                ("100px", "60px", "40px", "border-box", 60.),
                ("100px", "0px", "40px", "border-box", 40.),
                (
                    "25%",
                    "0px",
                    "none",
                    "border-box",
                    if display == "grid" { 20. } else { 50. },
                ),
            ] {
                let child_style = BTreeMap::from([
                    ("height".into(), height.into()),
                    ("min-height".into(), minimum.into()),
                    ("max-height".into(), maximum.into()),
                    ("box-sizing".into(), sizing.into()),
                    ("padding-top".into(), "5px".into()),
                    ("padding-bottom".into(), "5px".into()),
                    ("border-top-width".into(), "1px".into()),
                    ("border-bottom-width".into(), "1px".into()),
                    ("border-top-style".into(), "solid".into()),
                    ("border-bottom-style".into(), "solid".into()),
                ]);
                let styles =
                    BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
                let mut engine = TextEngine::new(&[]);
                let placements = taffy_child_placements(ChildPlacementInputs {
                    node: &root,
                    parent_style: &parent,
                    styles: &styles,
                    width: 100.,
                    height: Some(200.),
                    viewport: ViewportState::default(),
                    nodes: &nodes,
                    text_engine: &mut engine,
                })
                .unwrap();
                assert!(
                    (placements[0].height - expected).abs() < 0.02,
                    "{display}: {height}, min={minimum}, max={maximum}, {sizing}: {}",
                    placements[0].height
                );
            }
        }
    }

    #[test]
    fn indefinite_flex_percentage_height_does_not_use_viewport_height() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let parent = BTreeMap::from([
            ("display".into(), "flex".into()),
            ("align-items".into(), "start".into()),
        ]);
        let child_style = BTreeMap::from([
            ("height".into(), "50%".into()),
            ("min-height".into(), "12px".into()),
        ]);
        let styles = BTreeMap::from([(1, Arc::new(parent.clone())), (2, Arc::new(child_style))]);
        let mut engine = TextEngine::new(&[]);
        let placements = taffy_child_placements(ChildPlacementInputs {
            node: &root,
            parent_style: &parent,
            styles: &styles,
            width: 100.,
            height: None,
            viewport: ViewportState::default(),
            nodes: &nodes,
            text_engine: &mut engine,
        })
        .unwrap();
        assert!((placements[0].height - 12.).abs() < 0.02);
    }

    #[test]
    fn flex_leaf_measurement_uses_text_widths_without_implicit_growth() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        let first = node(2, Some(1), "div", "a");
        let second = node(3, Some(1), "div", "mmmm");
        let nodes = BTreeMap::from([(1, &root), (2, &first), (3, &second)]);
        let parent = BTreeMap::from([("display".into(), "flex".into())]);
        let styles = BTreeMap::from([
            (1, Arc::new(parent.clone())),
            (2, Arc::new(BTreeMap::new())),
            (3, Arc::new(BTreeMap::new())),
        ]);
        let mut engine = TextEngine::new(&[]);
        let placements = taffy_child_placements(ChildPlacementInputs {
            node: &root,
            parent_style: &parent,
            styles: &styles,
            width: 400.,
            height: None,
            viewport: ViewportState::default(),
            nodes: &nodes,
            text_engine: &mut engine,
        })
        .unwrap();
        assert!(placements[0].width > 0.);
        assert!(placements[1].width > placements[0].width);
        assert!(placements[0].width + placements[1].width < 200.);
        assert!((placements[1].x - placements[0].width).abs() < 0.02);
    }

    #[test]
    fn flex_column_measurement_uses_wrapped_text_height() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3];
        let first = node(2, Some(1), "div", "one two three four five");
        let second = node(3, Some(1), "div", "end");
        let nodes = BTreeMap::from([(1, &root), (2, &first), (3, &second)]);
        let parent = BTreeMap::from([
            ("display".into(), "flex".into()),
            ("flex-direction".into(), "column".into()),
        ]);
        let child_style = BTreeMap::from([
            ("width".into(), "50px".into()),
            ("line-height".into(), "20px".into()),
        ]);
        let styles = BTreeMap::from([
            (1, Arc::new(parent.clone())),
            (2, Arc::new(child_style.clone())),
            (3, Arc::new(child_style.clone())),
        ]);
        let mut engine = TextEngine::new(&[]);
        let expected = engine
            .shape_styled(
                &first.text,
                50.,
                16.,
                &child_style,
                ViewportState::default(),
            )
            .height;
        assert!(expected > 20.);
        let placements = taffy_child_placements(ChildPlacementInputs {
            node: &root,
            parent_style: &parent,
            styles: &styles,
            width: 100.,
            height: None,
            viewport: ViewportState::default(),
            nodes: &nodes,
            text_engine: &mut engine,
        })
        .unwrap();
        assert!((placements[1].y - expected).abs() < 0.02);
    }

    #[test]
    fn nested_flex_measurement_reflows_after_width_and_style_changes() {
        let mut tree = (1..=8)
            .map(|id| {
                let mut value = node(id, (id > 1).then_some(id - 1), "div", "");
                value.children = vec![id + 1];
                value
            })
            .collect::<Vec<_>>();
        tree.push(node(9, Some(8), "div", "one two three four five six seven"));
        let nodes = tree.iter().map(|node| (node.id, node)).collect();
        let parent = BTreeMap::from([
            ("display".into(), "flex".into()),
            ("flex-direction".into(), "column".into()),
        ]);
        let mut styles = (1..=8)
            .map(|id| (id, Arc::new(parent.clone())))
            .collect::<ComputedStyles>();
        let viewport = ViewportState::default();
        let mut engine = TextEngine::new(&[]);
        let mut heights = Vec::new();
        for (width, line_height) in [(200., "20px"), (50., "20px"), (50., "40px")] {
            let text_style = BTreeMap::from([("line-height".into(), line_height.into())]);
            let expected = engine
                .shape_styled(&tree[8].text, width, 16., &text_style, viewport)
                .height;
            styles.insert(9, Arc::new(text_style));
            let placement = taffy_child_placements(ChildPlacementInputs {
                node: &tree[0],
                parent_style: &parent,
                styles: &styles,
                width,
                height: None,
                viewport,
                nodes: &nodes,
                text_engine: &mut engine,
            })
            .unwrap()[0];
            assert!(
                (placement.height - expected).abs() < 0.02,
                "width={width}, line-height={line_height}: {placement:?}, expected {expected}"
            );
            heights.push(placement.height);
        }
        assert!(heights[1] > heights[0]);
        assert!((heights[2] - 2. * heights[1]).abs() < 0.02);
    }

    #[test]
    fn intrinsic_atomic_boxes_respect_nowrap_and_explicit_breaks() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        let left = node(2, Some(1), "span", "");
        let right = node(3, Some(1), "span", "");
        let br = node(4, Some(1), "br", "");
        let mut styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("display".into(), "block".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([
                    ("display".into(), "inline-block".into()),
                    ("width".into(), "40px".into()),
                ])),
            ),
            (
                3,
                Arc::new(BTreeMap::from([
                    ("display".into(), "inline-block".into()),
                    ("width".into(), "60px".into()),
                ])),
            ),
            (
                4,
                Arc::new(BTreeMap::from([("display".into(), "inline".into())])),
            ),
        ]);
        let mut engine = TextEngine::new(&[]);
        let measure = |engine: &mut TextEngine, root: &LayoutNode, styles: &ComputedStyles| {
            let nodes = BTreeMap::from([(1, root), (2, &left), (3, &right), (4, &br)]);
            engine.intrinsic_node(
                root,
                &nodes,
                styles,
                ViewportState::default(),
                &mut BTreeSet::new(),
            )
        };
        assert_eq!(measure(&mut engine, &root, &styles), (60., 100.));
        Arc::make_mut(styles.get_mut(&1).unwrap()).insert("white-space".into(), "nowrap".into());
        assert_eq!(measure(&mut engine, &root, &styles), (100., 100.));
        root.children = vec![2, 4, 3];
        assert_eq!(measure(&mut engine, &root, &styles), (60., 60.));
        Arc::make_mut(styles.get_mut(&4).unwrap()).insert("display".into(), "none".into());
        assert_eq!(measure(&mut engine, &root, &styles), (100., 100.));
    }

    #[test]
    fn descendant_intrinsic_widths_include_boxes_and_ignore_out_of_flow_content() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3, 4];
        let mut input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "aside", ""), node(4, Some(1), "nav", "")],
            stylesheets: vec!["section {width:max-content} div {width:40px;padding:5px;border:2px solid;margin:3px} aside {display:none;width:500px} nav {position:absolute;width:700px}".into()],
            stylesheet_media: BTreeMap::new(), selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(), focused_node: None, focus_visible: false,
            viewport: ViewportState::default(), scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light, reduced_motion: false, web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&1].0, 60.);
        input.stylesheets.push("div {box-sizing:border-box}".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&1].0, 46.);
    }

    #[test]
    fn intrinsic_inline_descendant_text_keeps_order_and_owner_styles() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 10,
                text: "a".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 11,
                text: "c".into(),
            },
        ]);
        let child = node(2, Some(1), "span", "b");
        let nodes = BTreeMap::from([(1, &root), (2, &child)]);
        let styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("display".into(), "block".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([
                    ("display".into(), "inline".into()),
                    ("font-size".into(), "32px".into()),
                ])),
            ),
        ]);
        let items = text_source::TextSource::intrinsic_items(&root, &nodes, &styles);
        assert_eq!(items.len(), 1);
        let text_source::IntrinsicItem::Text(source) = &items[0] else {
            panic!("expected inline text");
        };
        assert_eq!(source.text, "abc");
        assert_eq!(
            source
                .spans
                .iter()
                .map(|span| span.owner)
                .collect::<Vec<_>>(),
            vec![1, 2, 1]
        );
        let mut engine = TextEngine::new(&[]);
        let actual = engine.intrinsic_node(
            &root,
            &nodes,
            &styles,
            ViewportState::default(),
            &mut BTreeSet::new(),
        );
        let plain = engine.intrinsic_source(
            &text_source::TextSource::synthetic("abc"),
            16.,
            &BTreeMap::new(),
            ViewportState::default(),
            &BTreeMap::new(),
        );
        assert!(actual.0 > plain.0 && actual.1 > plain.1);
    }

    #[test]
    fn css_intrinsic_width_keywords_and_bounds_reach_used_geometry() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        let child = node(2, Some(1), "div", "alpha beta gamma");
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, child],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let mut width = |css: &str| {
            input.stylesheets = vec![format!("div {{font:16px Noto Sans}} div > div {{{css}}}")];
            compute(&input, &LayoutLimits::default())
                .unwrap()
                .offset_dimensions[&2]
                .0
        };
        let min = width("width:min-content");
        let max = width("width:max-content");
        assert!(min > 0. && max > min);
        assert!((width("width:fit-content") - max).abs() < 0.1);
        assert!((width("width:0;min-width:min-content") - min).abs() < 0.1);
        assert!((width("width:1000px;max-width:max-content") - max).abs() < 0.1);
        assert!(
            (width("width:min-content;padding:5px;border:2px solid;box-sizing:border-box")
                - min
                - 14.)
                .abs()
                < 0.1
        );
        assert_eq!(width("width:10px;min-width:80px;max-width:40px"), 80.);
        assert_eq!(
            width("width:100px;max-width:60px;padding:5px;border:2px solid;box-sizing:border-box"),
            60.
        );
    }

    #[test]
    fn fit_content_function_clamps_between_intrinsic_limits() {
        let viewport = ViewportState::default();
        for (value, available, expected) in [
            ("fit-content", 20., 40.),
            ("fit-content", 80., 80.),
            ("fit-content", 200., 120.),
            ("fit-content(70px)", 200., 70.),
            ("fit-content(50%)", 200., 100.),
        ] {
            assert_eq!(
                resolve_intrinsic_width(value, available, (40., 120.), 16., viewport),
                Some(expected)
            );
        }
    }

    #[test]
    fn intrinsic_images_preserve_ratio_with_one_specified_dimension() {
        let mut root = node(1, None, "div", "\n  \n");
        root.children = vec![2];
        let mut image = node(2, Some(1), "img", "");
        image.intrinsic_image_size = Some((200, 100));
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, image],
            stylesheets: vec![],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        for (css, expected) in [
            ("width:auto;height:auto", (200.0, 100.0)),
            ("width:80px;height:auto", (80.0, 40.0)),
            ("width:auto;height:30px", (60.0, 30.0)),
            ("width:80px;height:30px", (80.0, 30.0)),
            (
                "width:auto;height:44px;padding:5px;border:2px solid;box-sizing:border-box",
                (74.0, 44.0),
            ),
            (
                "width:94px;height:auto;padding:5px;border:2px solid;box-sizing:border-box",
                (94.0, 54.0),
            ),
        ] {
            input.stylesheets = vec![format!("img {{{css}}}")];
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            assert_eq!(output.offset_dimensions[&2], expected, "{css}");
            assert_eq!(
                output.geometry[&2].box_model.as_ref().unwrap().border_box.y,
                0.0,
                "collapsible block whitespace: {css}"
            );
        }
        input.stylesheets = vec!["div {white-space:pre}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(output.geometry[&2].box_model.as_ref().unwrap().border_box.y > 0.0);
        // Preserved spaces before a block replaced box form an anonymous
        // line. Before an inline image they instead consume inline advance.
        input.stylesheets = vec!["div {text-transform:full-width} img {display:block}".into()];
        input.nodes[0].text = " ".into();
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(output.geometry[&2].box_model.as_ref().unwrap().border_box.y > 0.0);
        input.stylesheets = vec!["img {display:block}".into()];
        input.nodes[0].text = "\u{a0}".into();
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(output.geometry[&2].box_model.as_ref().unwrap().border_box.y > 0.0);
    }

    #[test]
    fn offset_dimensions_preserve_invisible_boxes_and_ignore_transforms() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3, 4];
        let input = LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0., element_scrolls: BTreeMap::new(), anchor_parents: Default::default(), rendered_tree: None,
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", ""), node(4, Some(1), "div", "")],
            stylesheets: vec!["div > div {width:100px;height:20px;padding:5px;border:2px solid} div > div:first-child {visibility:hidden} div > div:last-child {display:none}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None, focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.offset_dimensions[&2], (114.0, 34.0));
        assert_eq!(output.offset_dimensions[&3], (114.0, 34.0));
        assert_eq!(output.offset_dimensions[&4], (0.0, 0.0));
        assert_eq!(output.geometry[&2].client_rects.len(), 1);
        assert_eq!(output.geometry[&2].client_rects[0].width, 114.0);
        assert!(output.geometry[&2].box_model.is_some());
        assert!(!output.geometry[&2].interactable);
        assert!(output.geometry[&4].box_model.is_none());
        assert!(output.geometry[&4].client_rects.is_empty());
        assert_eq!(
            output.geometry[&2].availability,
            GeometryAvailability::Hidden
        );
        assert_eq!(
            output.geometry[&3].box_model.as_ref().unwrap().border_box.y,
            34.0
        );
        let mut styles = output.computed_styles;
        Arc::make_mut(styles.get_mut(&3).unwrap()).insert("transform".into(), "scale(2)".into());
        let transformed = compute_with_styles(&input, &LayoutLimits::default(), styles).unwrap();
        assert_eq!(transformed.offset_dimensions[&3], (114.0, 34.0));
        assert_eq!(
            transformed.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width,
            228.0
        );
    }

    #[test]
    fn transform_applicability_excludes_non_replaced_inline_and_columns() {
        let inline = BTreeMap::from([("display".into(), "inline".into())]);
        let mut subject = node(1, None, "span", "label");
        assert!(!transformable_box(&subject, &inline));
        subject.tag = "img".into();
        assert!(transformable_box(&subject, &inline));
        subject.tag = "span".into();
        for display in [
            "inline-block",
            "block",
            "flex",
            "grid",
            "table-row",
            "table-cell",
        ] {
            let style = BTreeMap::from([("display".into(), display.into())]);
            assert!(transformable_box(&subject, &style), "{display}");
        }
        for display in ["table-column", "table-column-group", "none", "contents"] {
            let style = BTreeMap::from([("display".into(), display.into())]);
            assert!(!transformable_box(&subject, &style), "{display}");
        }
    }

    #[test]
    fn grouping_properties_force_flattening_of_preserved_depth() {
        let mut style = BTreeMap::from([("transform-style".into(), "preserve-3d".into())]);
        assert!(preserves_3d(&style));
        for (name, value) in [
            ("opacity", "0.5"),
            ("overflow-x", "hidden"),
            ("filter", "blur(1px)"),
            ("clip-path", "inset(0px)"),
            ("isolation", "isolate"),
            ("mask-image", "url(mask.png)"),
            ("mix-blend-mode", "multiply"),
            ("contain", "paint"),
            ("content-visibility", "auto"),
        ] {
            style.insert(name.into(), value.into());
            assert!(!preserves_3d(&style), "{name}");
            style.remove(name);
        }
        style.insert("overflow-x".into(), "clip".into());
        assert!(preserves_3d(&style));
    }

    #[test]
    fn ancestor_transforms_compose_before_bounding_descendant_boxes() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        root.attributes.insert("style".into(),"width:100px;height:100px;transform-origin:0 0;transform:translate(30px,40px) rotate(45deg)".into());
        let mut child = node(2, Some(1), "div", "");
        child.attributes.insert(
            "style".into(),
            "width:10px;height:20px;transform-origin:0 0;transform:rotate(-45deg)".into(),
        );
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![root, child],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!((rect.x - 30.).abs() < 1e-8 && (rect.y - 40.).abs() < 1e-8);
        assert!((rect.width - 10.).abs() < 1e-8 && (rect.height - 20.).abs() < 1e-8);
        assert_eq!(output.geometry[&2].client_rects, vec![rect]);
        assert_eq!(output.offset_dimensions[&2], (10., 20.));
        let paint = &output.paint_projections[&2];
        assert_eq!(
            (
                paint.box_model.border_box.width,
                paint.box_model.border_box.height
            ),
            (10., 20.)
        );
        assert!((paint.matrix[12] - 30.).abs() < 1e-8 && (paint.matrix[13] - 40.).abs() < 1e-8);
        input.nodes[0].attributes.insert(
            "style".into(),
            "width:100px;height:100px;perspective:100px;perspective-origin:0 0".into(),
        );
        input.nodes[1].attributes.insert(
            "style".into(),
            "width:10px;height:20px;transform:translateZ(50px)".into(),
        );
        let perspective = compute(&input, &LayoutLimits::default()).unwrap();
        let parent = perspective.geometry[&1]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        let child = perspective.geometry[&2]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert_eq!((parent.width, parent.height), (100., 100.));
        assert_eq!(
            (child.x, child.y, child.width, child.height),
            (0., 0., 20., 40.)
        );
        // Pinned WPT perspective-children-only-inline.html: an intervening
        // non-replaced inline box terminates the outer perspective context.
        input.nodes[0].children = vec![3];
        input.nodes[1].parent = Some(3);
        let mut middle = node(3, Some(1), "span", "");
        middle.children = vec![2];
        middle
            .attributes
            .insert("style".into(), "display:inline".into());
        input.nodes.push(middle);
        let flattened = compute(&input, &LayoutLimits::default()).unwrap();
        let child = flattened.geometry[&2]
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert_eq!((child.width, child.height), (10., 20.));
        input.nodes[0]
            .attributes
            .get_mut("style")
            .unwrap()
            .push_str(";overflow:hidden");
        let clipped = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(clipped.paint_projections[&1].clips.is_empty());
        let clips = &clipped.paint_projections[&2].clips;
        assert_eq!(clips.len(), 1);
        assert!(clips[0].clip_x && clips[0].clip_y);
        assert_eq!((clips[0].rect.width, clips[0].rect.height), (100., 100.));
    }

    #[test]
    fn independent_transforms_flow_from_style_to_viewport_geometry() {
        let mut subject = node(1, None, "div", "");
        subject.attributes.insert("style".into(),
            "width:10px;height:20px;translate:10px 20px;rotate:90deg;scale:2 3;transform:translateX(5px);transform-origin:0 0".into());
        let mut input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![subject],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState {
                x: 3.,
                y: 4.,
                ..Default::default()
            },
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let geometry = &output.geometry[&1];
        let rect = geometry.box_model.as_ref().unwrap().border_box;
        assert!((rect.x + 53.).abs() < 1e-8 && (rect.y - 26.).abs() < 1e-8);
        assert!((rect.width - 60.).abs() < 1e-8 && (rect.height - 20.).abs() < 1e-8);
        assert_eq!(geometry.client_rects, vec![rect]);
        assert_eq!(output.offset_dimensions[&1], (10., 20.));
        assert_eq!(
            output.resolved_values[&1]["transform"],
            "matrix(1, 0, 0, 1, 5, 0)"
        );
        assert_eq!(output.resolved_values[&1]["transform-origin"], "0px 0px");
        assert_eq!(output.resolved_values[&1]["perspective-origin"], "5px 10px");
        input.nodes[0]
            .attributes
            .get_mut("style")
            .unwrap()
            .push_str(";visibility:hidden");
        let hidden = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(hidden.geometry[&1].client_rects, vec![rect]);
        assert_eq!(
            hidden.geometry[&1].availability,
            GeometryAvailability::Hidden
        );
        assert!(!hidden.geometry[&1].interactable);
        input.nodes[0].attributes.get_mut("style").unwrap().push_str(
            ";padding:4px;border:2px solid;translate:none;rotate:none;scale:none;transform:rotate(90deg) scale(2,3)"
        );
        let bordered = compute(&input, &LayoutLimits::default()).unwrap();
        let model = bordered.geometry[&1].box_model.as_ref().unwrap();
        for (rect, expected) in [
            (model.border_box, (-99., -4., 96., 44.)),
            (model.padding_box, (-93., 0., 84., 36.)),
            (model.content_box, (-81., 8., 60., 20.)),
        ] {
            assert!((rect.x - expected.0).abs() < 1e-8 && (rect.y - expected.1).abs() < 1e-8);
            assert!(
                (rect.width - expected.2).abs() < 1e-8 && (rect.height - expected.3).abs() < 1e-8
            );
        }
        assert_eq!(bordered.offset_dimensions[&1], (22., 32.));
        let base = input.nodes[0].attributes["style"].clone();
        for (reference, expected) in [
            ("content-box", (-4., 0.)),
            ("fill-box", (-4., 0.)),
            ("border-box", (8., 12.)),
            ("stroke-box", (8., 12.)),
            ("view-box", (8., 12.)),
        ] {
            input.nodes[0].attributes.insert(
                "style".into(),
                format!("{base};transform-box:{reference};transform:translate(50%,50%) scale(2)"),
            );
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = output.geometry[&1].box_model.as_ref().unwrap().border_box;
            assert_eq!((rect.x, rect.y), expected, "{reference}");
            assert_eq!((rect.width, rect.height), (44., 64.), "{reference}");
        }
    }

    #[test]
    fn position_transform_and_scroll_remain_relational() {
        let mut root = node(1, None, "div", "");
        root.attributes.insert(
            "style".into(),
            "position:relative;width:300px;height:500px".into(),
        );
        let mut absolute = node(2, Some(1), "div", "");
        absolute.attributes.insert(
            "style".into(),
            "position:absolute;left:30px;top:140px;width:10px;height:10px".into(),
        );
        let mut fixed = node(3, Some(1), "div", "");
        fixed.attributes.insert(
            "style".into(),
            "position:fixed;left:5px;top:7px;width:10px;height:10px".into(),
        );
        let mut moved = node(4, Some(1), "div", "");
        moved.attributes.insert(
            "style".into(),
            "width:10px;height:10px;transform:translate(12px,8px)".into(),
        );
        root.children = vec![2, 3, 4];
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, absolute, fixed, moved],
                stylesheets: Vec::new(),
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState {
                    x: 0.0,
                    y: 100.0,
                    content_width: 1280.0,
                    content_height: 900.0,
                },
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("position layout");
        let absolute = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let fixed = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        let moved = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!(absolute.y >= 40.0);
        assert_eq!(fixed.y, 7.0);
        assert!(moved.x >= 12.0);
    }

    fn positioned_sizing_fixture(rule: &str, parent_rule: &str) -> LayoutOutput {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 5];
        let mut wrapper = node(2, Some(1), "article", "");
        wrapper.children = vec![3];
        let mut positioned = node(3, Some(2), "nav", "");
        positioned.children = vec![4];
        compute(&LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
            nodes:vec![root,wrapper,positioned,node(4,Some(3),"div",""),node(5,Some(1),"footer","")],
            stylesheets:vec![format!("section {{{parent_rule};width:300px;height:200px;padding:10px}} article {{width:40px;height:20px}} nav {{{rule}}} nav>div {{width:50%;height:50%}} footer {{height:5px}}")],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),
            focused_node:None,focus_visible:false,viewport:ViewportState::default(),scroll:ScrollState::default(),
            color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()},&LayoutLimits::default()).unwrap()
    }

    #[test]
    fn positioned_percentages_use_actual_padding_containing_block_before_child_layout() {
        for position in ["absolute", "fixed"] {
            let root_rule = if position == "fixed" {
                "transform:translateX(0px)"
            } else {
                "position:relative"
            };
            let output = positioned_sizing_fixture(
                &format!(
                    "position:{position};left:0;top:0;width:50%;height:50%;padding:5%;box-sizing:border-box"
                ),
                root_rule,
            );
            let model = &output.paint_projections[&3].box_model;
            assert!((model.border_box.width - 160.).abs() < 0.02);
            assert!((model.border_box.height - 110.).abs() < 0.02);
            assert!((model.padding.left - 16.).abs() < 0.02);
            let child = &output.paint_projections[&4].box_model.border_box;
            assert!((child.width - 64.).abs() < 0.02);
            assert!((child.height - 39.).abs() < 0.02);
        }
    }

    fn logical_container_fixture(rule: &str, item_rule: &str) -> LayoutOutput {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![
                    root,
                    node(2, Some(1), "article", ""),
                    node(3, Some(1), "aside", ""),
                ],
                stylesheets: vec![format!("section {{{rule}}} article,aside {{{item_rule}}}")],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn br_boundaries_use_the_paragraphs_native_line_boxes() {
        let mut root = node(1, None, "div", "AB");
        root.children = vec![2];
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 20,
                text: "A".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 21,
                text: "B".into(),
            },
        ]);
        let input = LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            nodes: vec![root, node(2, Some(1), "br", "")],
            stylesheets: vec!["div{font-size:16px;line-height:20px;width:100px}".into()],
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(output.text_clusters[&21][0].rect.y > output.text_clusters[&20][0].rect.y);
        let before = output
            .caret_stops
            .iter()
            .find(|s| s.boundary_node == 1 && s.offset == 1)
            .unwrap();
        let after = output
            .caret_stops
            .iter()
            .find(|s| s.boundary_node == 1 && s.offset == 2)
            .unwrap();
        assert_eq!(before.layout_node, 1);
        assert_eq!(after.layout_node, 1);
        assert!((after.rect.y - before.rect.y - 20.).abs() < 0.01);
        assert_eq!(before.rect.height, 20.);
        assert_eq!(after.rect.width, 0.);
        assert!(!output.text_clusters.contains_key(&2));
    }

    #[test]
    fn consecutive_and_vertical_br_boundaries_follow_native_line_axes() {
        for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            let mut root = node(1, None, "div", "AB");
            root.children = vec![2, 3];
            root.content = Some(vec![
                LayoutContent::Text {
                    node_id: 20,
                    text: "A".into(),
                },
                LayoutContent::Element(2),
                LayoutContent::Element(3),
                LayoutContent::Text {
                    node_id: 21,
                    text: "B".into(),
                },
            ]);
            let input = LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                nodes: vec![root, node(2, Some(1), "br", ""), node(3, Some(1), "br", "")],
                stylesheets: vec![format!(
                    "div{{font-size:16px;line-height:20px;width:100px;height:100px;writing-mode:{mode}}}"
                )],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            };
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let first = output
                .caret_stops
                .iter()
                .find(|s| s.boundary_node == 1 && s.offset == 1)
                .unwrap();
            let last = output
                .caret_stops
                .iter()
                .find(|s| s.boundary_node == 1 && s.offset == 3)
                .unwrap();
            if mode == "horizontal-tb" {
                assert!((last.rect.y - first.rect.y - 40.).abs() < 0.01);
            } else {
                assert!(((last.rect.x - first.rect.x).abs() - 40.).abs() < 0.01);
                assert_eq!(last.rect.height, 0.);
                assert_eq!(last.rect.x < first.rect.x, mode == "vertical-rl");
            }
        }
    }

    #[test]
    fn empty_control_caret_uses_live_value_over_default_content() {
        let mut input = column_fixture(
            "textarea,input{font-size:20px;line-height:30px;width:100px;height:40px;transform:translateX(8px)}",
            vec![
                node(2, Some(1), "textarea", "default value"),
                node(3, Some(1), "input", ""),
            ],
        );
        input.rendered_tree = Some(BTreeMap::from([
            (
                1,
                vec![LayoutContent::Element(2), LayoutContent::Element(3)],
            ),
            (
                2,
                vec![LayoutContent::Text {
                    node_id: 2,
                    text: String::new(),
                }],
            ),
            (
                3,
                vec![LayoutContent::Text {
                    node_id: 3,
                    text: String::new(),
                }],
            ),
        ]));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        for id in [2, 3] {
            let stop = output
                .caret_stops
                .iter()
                .find(|stop| stop.boundary_node == id)
                .unwrap();
            assert_eq!(stop.offset, 0);
            assert!((stop.rect.height - 30.).abs() < 0.01);
            assert_eq!(
                stop.matrix,
                output.paint_projections[&id]
                    .text_matrix
                    .unwrap_or(output.paint_projections[&id].matrix)
            );
        }
        input.rendered_tree.as_mut().unwrap().insert(
            2,
            vec![LayoutContent::Text {
                node_id: 2,
                text: "live".into(),
            }],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            !output
                .caret_stops
                .iter()
                .any(|stop| stop.boundary_node == 2)
        );
    }

    #[test]
    fn empty_block_carets_use_font_line_metrics_and_element_boundaries() {
        let output = logical_container_fixture(
            "width:200px;transform:translateX(12px)",
            "height:40px;font-size:20px;line-height:30px;direction:rtl",
        );
        let caret = output
            .caret_stops
            .iter()
            .find(|stop| stop.boundary_node == 2)
            .unwrap();
        assert_eq!(caret.offset, 0);
        assert_eq!(caret.layout_node, 2);
        assert_eq!(caret.rect.width, 0.);
        assert!((caret.rect.height - 30.).abs() < 0.01);
        assert!(caret.rtl);
        assert_eq!(
            caret.rect.x,
            output.paint_projections[&2].box_model.content_box.right()
        );
        assert_eq!(caret.matrix, output.paint_projections[&2].matrix);
        assert!(output.text_clusters.is_empty());
        assert!(output.text_fragments.values().all(Vec::is_empty));
    }

    #[test]
    fn vertical_flex_main_and_cross_axes_use_logical_direction() {
        for (mode, rtl, row_reverse, column_reverse) in [
            ("vertical-rl", false, false, true),
            ("vertical-lr", false, false, false),
            ("vertical-rl", true, true, true),
            ("sideways-lr", false, true, false),
        ] {
            for row in [true, false] {
                let output = logical_container_fixture(
                    &format!(
                        "display:flex;writing-mode:{mode};direction:{};width:120px;height:200px;flex-direction:{};gap:10px;align-items:flex-start",
                        if rtl { "rtl" } else { "ltr" },
                        if row { "row" } else { "column" }
                    ),
                    "width:30px;height:40px;flex-shrink:0",
                );
                let a = output.geometry[&2].box_model.as_ref().unwrap().border_box;
                let b = output.geometry[&3].box_model.as_ref().unwrap().border_box;
                assert_eq!((a.width, a.height), (30., 40.));
                if row {
                    assert_eq!(a.y > b.y, row_reverse, "{mode} rtl={rtl}");
                    assert_eq!(a.x, b.x);
                    assert!(((a.y - b.y).abs() - 50.).abs() < 0.02);
                } else {
                    assert_eq!(a.x > b.x, column_reverse, "{mode} rtl={rtl}");
                    assert_eq!(a.y, b.y);
                    assert!(((a.x - b.x).abs() - 40.).abs() < 0.02);
                }
            }
        }
    }

    #[test]
    fn vertical_grid_transposes_tracks_gaps_areas_and_item_lines() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        for named in [false, true] {
            let rules = if named {
                "section {grid-template-areas:'first .' '. second'} article {grid-area:first} aside {grid-area:second}"
            } else {
                "article {grid-column:1;grid-row:1} aside {grid-column:2;grid-row:2}"
            };
            let output=compute(&LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
                nodes:vec![root.clone(),node(2,Some(1),"article",""),node(3,Some(1),"aside","")],
                stylesheets:vec![format!("section {{display:grid;writing-mode:vertical-rl;width:100px;height:150px;grid-template-columns:30px 50px;grid-template-rows:20px 40px;column-gap:7px;row-gap:9px}} {rules}")],
                stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
                viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()},&LayoutLimits::default()).unwrap();
            let a = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            let b = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            assert_eq!((a.x, a.y, a.width, a.height), (80., 0., 20., 30.));
            assert_eq!((b.x, b.y, b.width, b.height), (31., 37., 40., 50.));
        }
    }

    #[test]
    fn orthogonal_auto_inline_size_subtracts_margins_from_parent_inline_basis() {
        // Pinned sizing-orthogonal-percentage-margin-001.html asserts 10% of
        // horizontal parent's 500px inline size => 50px margins and 100px height.
        let output = logical_container_fixture(
            "width:500px;height:200px",
            "writing-mode:vertical-lr;margin:10%;font-size:25px",
        );
        let model = output.geometry[&2].box_model.as_ref().unwrap();
        assert_eq!(model.margin.top, 50.);
        assert_eq!(model.margin.bottom, 50.);
        assert_eq!(model.border_box.height, 100.);
    }

    #[test]
    fn vertical_block_columns_have_real_inline_height_and_block_progression() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        for (mode, reverse) in [("vertical-rl", true), ("vertical-lr", false)] {
            let output=compute(&LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
                nodes:vec![root.clone(),node(2,Some(1),"article","ABC"),node(3,Some(1),"article","DEF")],
                stylesheets:vec![format!("section {{writing-mode:{mode};width:200px;height:120px}} article {{width:30px;height:100%;text-orientation:upright;font-size:20px}}")],
                stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
                viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()},&LayoutLimits::default()).unwrap();
            let first = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            let second = output.geometry[&3].box_model.as_ref().unwrap().border_box;
            assert_eq!((first.width, first.height), (30., 120.));
            assert_eq!(first.y, second.y);
            assert_eq!(first.x > second.x, reverse);
            assert!(((first.x - second.x).abs() - 30.).abs() < 0.02);
            let fragment = &output.text_fragments[&2][0];
            assert!(fragment.rect.height > fragment.rect.width);
            assert!(
                fragment
                    .glyph_runs
                    .iter()
                    .all(|r| r.orientation == oryn_common::v4::PaintGlyphOrientation::Upright)
            );
        }
    }

    #[test]
    fn relative_overconstraints_follow_logical_start_sides_in_vertical_modes() {
        let style = BTreeMap::from([
            ("left".into(), "10px".into()),
            ("right".into(), "20px".into()),
            ("top".into(), "30px".into()),
            ("bottom".into(), "40px".into()),
        ]);
        for (mode, rtl, expected) in [
            ("horizontal-tb", false, (10., 30.)),
            ("horizontal-tb", true, (-20., 30.)),
            ("vertical-rl", false, (-20., 30.)),
            ("vertical-rl", true, (-20., -40.)),
            ("vertical-lr", false, (10., 30.)),
            ("vertical-lr", true, (10., -40.)),
            ("sideways-lr", false, (10., -40.)),
            ("sideways-lr", true, (10., 30.)),
        ] {
            assert_eq!(
                relative_position_offset(
                    &style,
                    200.,
                    Some(100.),
                    rtl,
                    mode,
                    16.,
                    ViewportState::default()
                ),
                expected,
                "{mode} rtl={rtl}"
            );
        }
    }

    #[test]
    fn opposing_insets_determine_auto_size_before_percent_descendants() {
        for sizing in ["content-box", "border-box"] {
            let output = positioned_sizing_fixture(
                &format!(
                    "position:absolute;left:10px;right:20px;top:15px;bottom:25px;padding:5px;border:2px solid;box-sizing:{sizing}"
                ),
                "position:relative",
            );
            let model = &output.paint_projections[&3].box_model;
            assert!((model.border_box.width - 290.).abs() < 0.02);
            assert!((model.border_box.height - 180.).abs() < 0.02);
            let child = &output.paint_projections[&4].box_model.border_box;
            assert!((child.width - 138.).abs() < 0.02);
            assert!((child.height - 83.).abs() < 0.02);
        }
        let constrained = positioned_sizing_fixture(
            "position:absolute;left:10px;right:20px;top:15px;bottom:25px;max-width:100px;min-height:190px",
            "position:relative",
        );
        let rect = constrained.paint_projections[&3].box_model.border_box;
        assert!((rect.width - 100.).abs() < 0.02);
        assert!((rect.height - 190.).abs() < 0.02);
    }

    #[test]
    fn out_of_flow_children_do_not_consume_flex_space_or_auto_block_height() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2, 3];
        for display in ["block", "flex", "grid"] {
            let output=compute(&LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
                nodes:vec![root.clone(),node(2,Some(1),"nav",""),node(3,Some(1),"footer","")],
                stylesheets:vec![format!("section {{position:relative;display:{display};width:200px}} nav {{position:absolute;width:500px;height:900px}} footer {{width:100px;height:20px;flex-shrink:0}}")],
                stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
                viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()},&LayoutLimits::default()).unwrap();
            assert!(
                (output.paint_projections[&1].box_model.border_box.height - 20.).abs() < 0.02,
                "{display}"
            );
            assert!(
                (output.paint_projections[&3].box_model.border_box.width - 100.).abs() < 0.02,
                "{display}"
            );
        }
    }

    #[test]
    fn viewport_overscroll_policy_uses_root_axes_when_body_overflow_propagates() {
        let mut html = node(1, None, "html", "");
        html.children = vec![2];
        let mut input = column_fixture(
            "html{overscroll-behavior-x:none;overscroll-behavior-y:contain}body{margin:0;height:20px;overflow:auto;overscroll-behavior:auto}",
            Vec::new(),
        );
        input.nodes = vec![html, node(2, Some(1), "body", "")];
        input.viewport = ViewportState {
            width: 100.,
            height: 100.,
            device_pixel_ratio: 1.,
        };
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let viewport = output.viewport_scroll.as_ref().unwrap();
        assert_eq!(output.computed_styles[&1]["overscroll-behavior-x"], "none");
        assert_eq!(
            output.computed_styles[&1]["overscroll-behavior-y"],
            "contain"
        );
        assert_eq!(viewport.style["overflow-y"], "auto");
        assert_eq!(viewport.style["overscroll-behavior-x"], "none");
        assert_eq!(viewport.style["overscroll-behavior-y"], "contain");
        input.stylesheets = vec![
            "html{overscroll-behavior:auto}body{overflow:auto;overscroll-behavior:contain}".into(),
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            output.viewport_scroll.as_ref().unwrap().style["overscroll-behavior-y"],
            "auto"
        );
    }

    #[test]
    fn viewport_classic_scrollbar_reflows_document_without_an_element_gutter() {
        let mut html = node(1, None, "html", "");
        html.children = vec![2];
        let mut input = column_fixture(
            "html{scrollbar-width:auto}body{margin:0;height:200px}",
            Vec::new(),
        );
        input.nodes = vec![html, node(2, Some(1), "body", "")];
        input.viewport = ViewportState {
            width: 100.,
            height: 100.,
            device_pixel_ratio: 1.,
        };
        input.scrollbar_width = 16.;
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let viewport = output.viewport_scroll.as_ref().unwrap();
        assert!((viewport.scroll.scrollport.width - 84.).abs() < 0.02);
        assert!((viewport.scroll.scrollport.height - 100.).abs() < 0.02);
        assert!((output.paint_projections[&2].box_model.border_box.width - 84.).abs() < 0.02);
        assert_eq!(output.computed_styles[&1]["overflow-y"], "visible");
        assert_eq!(viewport.style["overflow-y"], "auto");
        input.stylesheets = vec!["body{margin:0;height:200px;overflow:hidden}".into()];
        let hidden = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            hidden.viewport_scroll.as_ref().unwrap().style["overflow-y"],
            "hidden"
        );
        assert!(
            (hidden
                .viewport_scroll
                .as_ref()
                .unwrap()
                .scroll
                .scrollport
                .width
                - 100.)
                .abs()
                < 0.02
        );
    }
    #[test]
    fn classic_scrollbars_reflow_children_and_preserve_outer_size() {
        let mut input = column_fixture(
            "div{width:100px;height:100px;overflow:auto}p{margin:0;width:100%;height:200px}",
            vec![node(2, Some(1), "p", "")],
        );
        input.scrollbar_width = 16.;
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let parent = &output.paint_projections[&1];
        let scroll = parent.scroll.as_ref().unwrap();
        assert!((parent.box_model.border_box.width - 100.).abs() < 0.02);
        assert!((scroll.scrollport.width - 84.).abs() < 0.02);
        assert!((output.paint_projections[&2].box_model.border_box.width - 84.).abs() < 0.02);
        assert!(
            (scroll.scrollport.height - 100.).abs() < 0.02,
            "a shrinking percentage child must not induce a horizontal scrollbar"
        );
        input.stylesheets=vec!["div{width:100px;height:100px;overflow:auto;scrollbar-gutter:stable both-edges;scrollbar-width:thin}p{margin:0;width:100%;height:20px}".into()];
        let stable = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            (stable.paint_projections[&1]
                .scroll
                .as_ref()
                .unwrap()
                .scrollport
                .width
                - 84.)
                .abs()
                < 0.02
        );
        assert!(
            (stable.paint_projections[&2].box_model.border_box.x
                - stable.paint_projections[&1].box_model.border_box.x
                - 8.)
                .abs()
                < 0.02
        );
        input.scrollbar_width = 0.;
        let overlay = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            (overlay.paint_projections[&1]
                .scroll
                .as_ref()
                .unwrap()
                .scrollport
                .width
                - 100.)
                .abs()
                < 0.02
        );
    }
    #[test]
    fn pinned_perspective_overflow_modes_keep_the_same_projected_child() {
        // css/css-transforms/overflow-perspective-001.html through -003.html.
        for overflow in ["auto", "clip", "hidden"] {
            let input = column_fixture(
                &format!(
                    "div{{width:300px;height:300px;overflow:{overflow};perspective:1000px}}p{{margin:0;width:60px;height:60px;transform:translateX(100px) translateY(140px) translateZ(550px);transform-origin:top left}}"
                ),
                vec![node(2, Some(1), "p", "")],
            );
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
            assert!((rect.x - (150. - 50. / 0.45)).abs() < 0.02, "{overflow}");
            assert!((rect.y - (150. - 10. / 0.45)).abs() < 0.02, "{overflow}");
            assert!((rect.width - 60. / 0.45).abs() < 0.02, "{overflow}");
            let scroll = output.paint_projections[&1].scroll.as_ref().unwrap();
            assert!((scroll.scrollable.width - 300.).abs() < 0.02);
            assert!((scroll.scrollable.height - 300.).abs() < 0.02);
        }
    }
    #[test]
    fn native_scroll_overflow_applies_ancestor_perspective_before_flattening() {
        let input = column_fixture(
            "div{width:50px;height:30px;overflow:auto;perspective:200px;perspective-origin:0 0}p{margin:0;width:50px;height:30px;transform:translateZ(100px);transform-origin:0 0}",
            vec![node(2, Some(1), "p", "")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let scroll = output.paint_projections[&1].scroll.as_ref().unwrap();
        assert!((scroll.scrollable.width - 100.).abs() < 0.02);
        assert!((scroll.scrollable.height - 60.).abs() < 0.02);
    }
    #[test]
    fn native_background_features_survive_stylo_servo_cascade() {
        let input = column_fixture(
            "div{background-image:cross-fade(20% linear-gradient(red,red),linear-gradient(blue,blue));background-clip:text}",
            Vec::new(),
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let style = &output.computed_styles[&1];
        assert!(style["background-image"].contains("cross-fade("));
        assert_eq!(style["background-clip"], "text");
    }

    #[test]
    fn camera_crossing_scroll_metrics_and_chrome_share_native_domain() {
        let mut input = column_fixture(
            "div{width:100px;height:100px;overflow:scroll}p{margin:0;width:100px;height:100px;transform:matrix3d(1,0,0,-0.02,0,1,0,0,0,0,1,0,0,0,0,1);transform-origin:0 0}",
            vec![node(2, Some(1), "p", "")],
        );
        input.scrollbar_width = 16.;
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let projection = &output.paint_projections[&1];
        let scroll = projection.scroll.as_ref().unwrap();
        let limit = oryn_common::layout_domain::MAX_SCROLL_COORDINATE;
        assert!((scroll.scrollable.right() - limit).abs() < 0.1);
        assert!(scroll.scrollable.width.is_finite());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|message| message.contains("mathematically unbounded"))
        );
        let bars = oryn_common::scrollbar::geometry(projection, &output.computed_styles[&1]);
        let horizontal = bars.iter().find(|bar| bar.horizontal).unwrap();
        assert!(
            (horizontal.max_offset - (scroll.scrollable.width - scroll.scrollport.width)).abs()
                < 0.01
        );
        assert!(
            (horizontal
                .offset_at_thumb_origin(horizontal.track.right())
                .unwrap()
                - horizontal.max_offset)
                .abs()
                < 0.01
        );
        let child = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert!(child.width.is_finite() && child.right() <= limit);
    }

    #[test]
    fn sticky_end_inset_uses_classic_scrollport_not_outer_padding_box() {
        let mut content = node(2, Some(1), "article", "");
        content.children = vec![3, 4];
        content.content = Some(vec![LayoutContent::Element(3), LayoutContent::Element(4)]);
        let mut input = column_fixture(
            "div{width:100px;height:100px;overflow-x:hidden;overflow-y:scroll}article{width:200px;height:200px;white-space:nowrap}span{display:inline-block;width:90px;height:20px}nav{display:inline-block;position:sticky;right:0;width:20px;height:20px}",
            vec![content],
        );
        input
            .nodes
            .extend([node(3, Some(2), "span", ""), node(4, Some(2), "nav", "")]);
        input.scrollbar_width = 16.;
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert!((rect.x - 64.).abs() < 0.02);
    }
    #[test]
    fn native_scroll_area_includes_trailing_padding() {
        let input = column_fixture(
            "div{width:100px;height:100px;padding:10px;overflow:auto}p{margin:0;width:200px;height:300px}",
            vec![node(2, Some(1), "p", "")],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let scroll = output.paint_projections[&1].scroll.as_ref().unwrap();
        assert!((scroll.scrollable.width - 220.).abs() < 0.02);
        assert!((scroll.scrollable.height - 320.).abs() < 0.02);
    }

    #[test]
    fn native_scroll_metrics_preserve_local_axes_and_stop_clipped_overflow() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut inner = node(2, Some(1), "article", "");
        inner.children = vec![3];
        let mut input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::from([(1,(10.,20.))]),anchor_parents: Default::default(), rendered_tree:None,
            nodes:vec![root,inner,node(3,Some(2),"main","")],
            stylesheets:vec!["section{width:100px;height:100px;overflow:auto;transform:scale(2);transform-origin:0 0}article{width:200px;height:300px;overflow:hidden}main{width:1000px;height:1000px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let metrics = output.paint_projections[&1].scroll.as_ref().unwrap();
        assert_eq!((metrics.offset_x, metrics.offset_y), (10., 20.));
        assert!((metrics.scrollport.width - 100.).abs() < 0.02);
        assert!((metrics.scrollable.width - 200.).abs() < 0.02);
        assert!((metrics.scrollable.height - 300.).abs() < 0.02);
        input.element_scrolls.insert(1, (30., 40.));
        let moved = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            moved.paint_projections[&1]
                .scroll
                .as_ref()
                .unwrap()
                .scrollable,
            metrics.scrollable
        );
        let child = output.paint_projections[&2].scroll.as_ref().unwrap();
        assert!((child.scrollable.width - 1000.).abs() < 0.02);
        input.stylesheets=vec!["section{position:relative;width:100px;height:100px;overflow:auto}article{width:200px;height:300px;overflow:hidden}main{position:absolute;left:0;top:0;width:1000px;height:1000px}".into()];
        let escaped = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            (escaped.paint_projections[&1]
                .scroll
                .as_ref()
                .unwrap()
                .scrollable
                .width
                - 1000.)
                .abs()
                < 0.02
        );
        assert!(
            (escaped.paint_projections[&2]
                .scroll
                .as_ref()
                .unwrap()
                .scrollable
                .width
                - 200.)
                .abs()
                < 0.02
        );
        assert_eq!(
            escaped.paint_projections[&3].clips.len(),
            1,
            "only containing scrollport clips positioned descendant"
        );
        input.stylesheets=vec!["section{width:100px;height:100px;overflow:auto}article{width:200px;height:300px;overflow:hidden}main{position:absolute;left:0;top:0;width:1000px;height:1000px}".into()];
        let initial = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            initial.paint_projections[&3].clips.is_empty(),
            "initial containing block escapes ordinary clipping ancestors"
        );
    }

    #[test]
    fn sticky_scroll_translation_preserves_flow_and_moves_text_with_geometry() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut scroller = node(2, Some(1), "article", "");
        scroller.children = vec![3, 7];
        // The following sibling admits scroll=280 while main remains the sticky containing block.
        let mut content = node(3, Some(2), "main", "");
        content.children = vec![4, 5, 6];
        let mut input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::new(),anchor_parents: Default::default(), rendered_tree:None,
            nodes:vec![root,scroller,content,node(4,Some(3),"header",""),node(5,Some(3),"nav","sticky"),node(6,Some(3),"footer",""),node(7,Some(2),"aside","")],
            stylesheets:vec!["section {width:200px} article {width:200px;height:100px;overflow:auto} main {height:300px} header {height:30px} nav {position:sticky;top:10px;height:20px} footer {height:10px} article>aside {height:100px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState::default(),color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let initial = compute(&input, &LayoutLimits::default()).unwrap();
        let initial_top = initial.geometry[&5]
            .box_model
            .as_ref()
            .unwrap()
            .border_box
            .y;
        assert!((initial_top - 30.).abs() < 0.02);
        for (scroll, expected) in [(60., 10.), (280., 0.)] {
            input.element_scrolls.insert(2, (0., scroll));
            let output = compute(&input, &LayoutLimits::default()).unwrap();
            let sticky = output.geometry[&5].box_model.as_ref().unwrap().border_box;
            let footer = output.geometry[&6].box_model.as_ref().unwrap().border_box;
            assert!((sticky.y - expected).abs() < 0.02);
            assert!(
                (footer.y - (50. - scroll)).abs() < 0.02,
                "sticky must not move siblings"
            );
            assert!(
                (output.text_fragments[&5][0].rect.y
                    - initial.text_fragments[&5][0].rect.y
                    - (expected - initial_top))
                    .abs()
                    < 0.02
            );
            assert_eq!(output.offset_dimensions[&5], initial.offset_dimensions[&5]);
        }
    }

    #[test]
    fn positioned_boxes_use_ancestor_padding_box_and_viewport_fixed_plane() {
        let mut root = node(1, None, "section", "");
        root.children = vec![2];
        let mut wrapper = node(2, Some(1), "article", "");
        wrapper.children = vec![3, 4];
        let mut fixed = node(4, Some(2), "aside", "");
        fixed.children = vec![5];
        let input=LayoutInput {svg_clip_resources:Vec::new(),legacy_background_urls:Default::default(),registered_custom_properties:Vec::new(),document_language:None,document_base_url:"https://oryn.invalid/".into(),image_intrinsics:Default::default(),control_states:Default::default(),temporal_pickers:Default::default(),permission_prompts:Default::default(),top_layer:Vec::new(),scrollbar_width:0.,element_scrolls:BTreeMap::from([(2,(0.,20.))]),anchor_parents: Default::default(), rendered_tree:None,
            nodes:vec![root,wrapper,node(3,Some(2),"nav",""),fixed,node(5,Some(4),"div","")],
            stylesheets:vec!["section {position:relative;width:200px;height:150px;padding:10px;border:5px solid;margin:20px} article {width:50px;height:40px;margin:30px;overflow:hidden} nav {position:absolute;right:10px;bottom:15px;width:20px;height:25px} aside {position:fixed;left:8px;top:9px;width:20px;height:20px} div {width:5px;height:5px}".into()],
            stylesheet_media:BTreeMap::new(),selector_states:Default::default(),shadow_trees: Default::default(), stylesheet_base_urls: Default::default(), stylesheet_scope_roots:BTreeMap::new(),custom_states:BTreeMap::new(),focused_node:None,focus_visible:false,
            viewport:ViewportState::default(),scroll:ScrollState{x:0.,y:100.,content_width:1280.,content_height:900.},
            color_scheme:ColorScheme::Light,reduced_motion:false,web_fonts:Vec::new()};
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let root = output.paint_projections[&1].box_model.padding_box;
        let absolute = output.paint_projections[&3].box_model.border_box;
        assert!((absolute.x - (root.right() - 30.)).abs() < 0.02);
        assert!((absolute.y - (root.bottom() - 40.)).abs() < 0.02);
        let fixed = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        let child = output.geometry[&5].box_model.as_ref().unwrap().border_box;
        assert_eq!((fixed.x, fixed.y), (8., 9.));
        assert_eq!((child.x, child.y), (8., 9.));
        assert!(output.geometry[&4].in_viewport);
        assert!(output.paint_projections[&4].clips.is_empty());
        assert!(output.paint_projections[&5].clips.is_empty());
    }

    #[test]
    fn taffy_honors_bounded_float_and_clear_flow() {
        let mut root = node(1, None, "div", "");
        root.attributes
            .insert("style".into(), "display:block;width:300px".into());
        let mut floated = node(2, Some(1), "div", "Float");
        floated
            .attributes
            .insert("style".into(), "float:left;width:100px;height:40px".into());
        let mut cleared = node(3, Some(1), "div", "Cleared");
        cleared
            .attributes
            .insert("style".into(), "clear:both;width:120px;height:20px".into());
        root.children = vec![2, 3];
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![root, floated, cleared],
                stylesheets: Vec::new(),
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: Vec::new(),
            },
            &LayoutLimits::default(),
        )
        .expect("bounded float layout");
        let floated = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let cleared = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert!(cleared.y >= floated.bottom());
    }

    #[test]
    fn registered_web_font_is_hash_checked_and_reported() {
        let bytes = DEFAULT_FONT_BYTES.to_vec();
        let output = compute(
            &LayoutInput {
                svg_clip_resources: Vec::new(),
                legacy_background_urls: Default::default(),
                registered_custom_properties: Vec::new(),
                document_language: None,
                document_base_url: "https://oryn.invalid/".into(),
                image_intrinsics: Default::default(),
                control_states: Default::default(),
                temporal_pickers: Default::default(),
                permission_prompts: Default::default(),
                top_layer: Vec::new(),
                scrollbar_width: 0.,
                element_scrolls: BTreeMap::new(),
                anchor_parents: Default::default(),
                rendered_tree: None,
                nodes: vec![node(1, None, "p", "Measured")],
                stylesheets: vec!["p{font-family:Fixture Font}".into()],
                stylesheet_media: BTreeMap::new(),
                selector_states: Default::default(),
                shadow_trees: Default::default(),
                stylesheet_base_urls: Default::default(),
                stylesheet_scope_roots: BTreeMap::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
                focus_visible: false,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: vec![WebFont {
                    pending: false,
                    blocked: false,
                    order: 0,
                    descriptors: String::new(),
                    family: "Fixture Font".into(),
                    source_url: "https://font.example/fixture.ttf".into(),
                    sha256: DEFAULT_FONT_SHA256.into(),
                    bytes,
                }],
            },
            &LayoutLimits::default(),
        )
        .expect("web font layout");
        assert_eq!(output.loaded_fonts.len(), 1);
        assert_eq!(output.loaded_fonts[0].family, "Fixture Font");
    }

    #[test]
    fn web_font_hash_and_quota_rejections_are_diagnostic() {
        let fixture = |sha256: &str| LayoutInput {
            svg_clip_resources: Vec::new(),
            legacy_background_urls: Default::default(),
            registered_custom_properties: Vec::new(),
            document_language: None,
            document_base_url: "https://oryn.invalid/".into(),
            image_intrinsics: Default::default(),
            control_states: Default::default(),
            temporal_pickers: Default::default(),
            permission_prompts: Default::default(),
            top_layer: Vec::new(),
            scrollbar_width: 0.,
            element_scrolls: BTreeMap::new(),
            anchor_parents: Default::default(),
            rendered_tree: None,
            nodes: vec![node(1, None, "p", "Measured")],
            stylesheets: Vec::new(),
            stylesheet_media: BTreeMap::new(),
            selector_states: Default::default(),
            shadow_trees: Default::default(),
            stylesheet_base_urls: Default::default(),
            stylesheet_scope_roots: BTreeMap::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            focus_visible: false,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![WebFont {
                pending: false,
                blocked: false,
                order: 0,
                descriptors: String::new(),
                family: "Fixture Font".into(),
                source_url: "https://font.example/fixture.ttf".into(),
                sha256: sha256.into(),
                bytes: DEFAULT_FONT_BYTES.to_vec(),
            }],
        };
        let wrong_hash = compute(&fixture(&"0".repeat(64)), &LayoutLimits::default())
            .expect("hash rejection remains a successful layout");
        assert!(wrong_hash.loaded_fonts.is_empty());
        assert!(
            wrong_hash
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("SHA-256 mismatch"))
        );
        let quota = compute(
            &fixture(DEFAULT_FONT_SHA256),
            &LayoutLimits {
                max_font_bytes: 1,
                ..LayoutLimits::default()
            },
        )
        .expect("quota rejection remains a successful layout");
        assert!(quota.loaded_fonts.is_empty());
        assert!(
            quota
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("per-font limit"))
        );
    }
    #[test]
    fn empty_embed_has_no_box_until_a_response_selects_a_representation() {
        let embed = node(2, Some(1), "embed", "");
        let mut input = column_fixture(
            "embed{width:100px;height:100px;background:red}",
            vec![embed],
        );
        let (projected, hidden) = object_images::project(&input);
        assert!(hidden.contains(&2));
        assert!(!projected.nodes.iter().any(|node| node.id == 2));
        input.nodes[1].document_replaced = true;
        let (projected, hidden) = object_images::project(&input);
        assert!(!hidden.contains(&2));
        assert!(projected.nodes.iter().any(|node| node.id == 2));
        input.nodes[1].document_replaced = false;
        input.nodes[1].intrinsic_image_size = Some((100, 100));
        assert!(object_images::project(&input).1.is_empty());
    }

    #[test]
    fn print_frame_capture_returns_unscrolled_native_source_context() {
        let mut input = column_fixture("p{height:20px}", vec![node(2, Some(1), "p", "frame")]);
        input.scroll.x = 7.;
        input.scroll.y = 17.;
        input.element_scrolls.insert(2, (3., 5.));
        input.scrollbar_width = 15.;
        let original = input.clone();
        let (layout, captured) =
            compute_print_viewport_with_capture_input(&input, &LayoutLimits::default()).unwrap();
        assert!(layout.geometry.contains_key(&2));
        assert_eq!(input, original);
        assert_eq!(captured.nodes, original.nodes);
        assert_eq!(captured.viewport, original.viewport);
        assert_eq!(captured.scroll, ScrollState::default());
        assert!(captured.element_scrolls.is_empty());
        assert_eq!(captured.scrollbar_width, 0.);
    }

    #[test]
    fn object_image_replacement_hides_fallback_in_screen_and_print_and_restores_on_failure() {
        let mut object = node(2, Some(1), "object", "");
        object.children = vec![3];
        object.content = Some(vec![LayoutContent::Element(3)]);
        object.intrinsic_image_size = Some((100, 100));
        let mut input = column_fixture(
            "object{width:200px}dialog{display:block;width:40px;height:40px}",
            vec![object],
        );
        input.nodes.push(node(3, Some(2), "dialog", "Fallback"));
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            node_id: 3,
            kind: oryn_common::v4::TopLayerKind::ModalDialog,
            anchor_source: None,
            picker: None,
        });
        let limits = LayoutLimits::default();
        let screen = compute(&input, &limits).unwrap();
        assert_eq!(
            screen.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        assert!(!screen.backdrops.contains_key(&3));
        let printed = compute_print_viewport(&input, &limits).unwrap();
        assert_eq!(
            printed.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        assert!(!printed.backdrops.contains_key(&3));
        let pages =
            print::compute(&input, &limits, &oryn_common::v4::PrintOptions::default()).unwrap();
        assert_eq!(
            pages.document.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        assert!(!pages.document.backdrops.contains_key(&3));
        input
            .nodes
            .iter_mut()
            .find(|node| node.id == 2)
            .unwrap()
            .intrinsic_image_size = None;
        let fallback = compute(&input, &limits).unwrap();
        assert!(fallback.geometry[&3].box_model.is_some());
    }
    #[test]
    fn svg_default_dimensions_preserve_absent_natural_ratio() {
        let mut object = node(2, Some(1), "object", "");
        object.document_replaced = true;
        object.image_intrinsics = Some(oryn_dom::ImageIntrinsics::new(Some(80.), None, None));
        object.intrinsic_image_size = Some((80, 150));
        let mut input = column_fixture("object{width:160px}", vec![object]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&2].box_model.as_ref().unwrap().content_box;
        assert_eq!(rect.width, 160.);
        assert_eq!(rect.height, 150.);
        input.nodes[1].image_intrinsics =
            Some(oryn_dom::ImageIntrinsics::new(Some(80.), None, Some(2.)));
        input.nodes[1].intrinsic_image_size = Some((80, 40));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            output.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .height,
            80.
        );
    }

    #[test]
    fn embedded_svg_computed_geometry_updates_natural_size_without_percentage_feedback() {
        let mut object = node(2, Some(1), "object", "");
        object.document_replaced = true;
        object.image_document_root = Some(3);
        object.image_intrinsics = Some(oryn_dom::ImageIntrinsics::new(Some(80.), None, Some(2.)));
        object.intrinsic_image_size = Some((80, 40));
        object.children = vec![3];
        object.content = Some(vec![LayoutContent::Element(3)]);
        let mut svg = node(3, Some(2), "svg", "");
        svg.namespace = "http://www.w3.org/2000/svg".into();
        svg.attributes.insert("width".into(), "80".into());
        svg.attributes
            .insert("viewBox".into(), "0 0 200 100".into());
        let mut input = column_fixture("svg{width:120px;height:auto}", vec![object, svg]);
        let screen = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = screen.geometry[&2].box_model.as_ref().unwrap().content_box;
        assert_eq!((rect.width, rect.height), (120., 60.));
        assert_eq!(
            screen.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        input.stylesheets = vec!["svg{width:50%;height:60px}".into()];
        let screen = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = screen.geometry[&2].box_model.as_ref().unwrap().content_box;
        assert_eq!((rect.width, rect.height), (120., 60.));
        let printed = compute_print_viewport(&input, &LayoutLimits::default()).unwrap();
        let rect = printed.geometry[&2].box_model.as_ref().unwrap().content_box;
        assert_eq!((rect.width, rect.height), (120., 60.));
    }

    #[test]
    fn embedded_document_default_size_does_not_create_an_intrinsic_ratio() {
        let mut object = node(2, Some(1), "object", "");
        object.document_replaced = true;
        object.children = vec![3];
        object.content = Some(vec![LayoutContent::Element(3)]);
        let mut input = column_fixture("object{width:600px}span{display:block}", vec![object]);
        input.nodes.push(node(3, Some(2), "span", "Fallback"));
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&2].box_model.as_ref().unwrap().content_box;
        assert_eq!(rect.width, 600.);
        assert_eq!(rect.height, 150.);
        assert_eq!(
            output.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        assert!(
            preferred_ratio(
                input.nodes.iter().find(|node| node.id == 2).unwrap(),
                &BTreeMap::new()
            )
            .is_none()
        );
        input
            .nodes
            .iter_mut()
            .find(|node| node.id == 2)
            .unwrap()
            .document_replaced = false;
        let fallback = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(fallback.geometry[&3].box_model.is_some());
    }
    #[test]
    fn static_layout_reuse_preserves_input_limits_and_animation_changes() {
        let mut input = column_fixture(
            "div>div{width:30px;height:10px}",
            vec![node(2, Some(1), "div", "")],
        );
        let limits = LayoutLimits::default();
        let mut engine = StyleEngine::default();
        let first = engine.compute_at(&input, &limits, 0.).unwrap();
        assert!(engine.static_layout.is_some());
        let repeated = engine.compute_at(&input, &limits, 5.).unwrap();
        assert_eq!(first, repeated);
        assert_eq!(engine.instance_style_sample().unwrap().now, 5.);
        input.nodes[1]
            .attributes
            .insert("style".into(), "width:70px".into());
        let changed = engine.compute_at(&input, &limits, 6.).unwrap();
        assert_eq!(changed.computed_styles[&2]["width"], "70px");
        assert!(
            engine
                .compute_at(
                    &input,
                    &LayoutLimits {
                        max_nodes: 1,
                        ..limits.clone()
                    },
                    6.
                )
                .is_err()
        );
        input.nodes[1].attributes.remove("style");
        input.stylesheets = vec!["@keyframes grow{from{width:10px}to{width:110px}}div>div{animation:grow 10s linear both;height:10px}".into()];
        engine.compute_at(&input, &limits, 10.).unwrap();
        assert!(engine.static_layout.is_none());
        let animated = engine.compute_at(&input, &limits, 15.).unwrap();
        assert_eq!(animated.computed_styles[&2]["width"], "60px");
        assert!(engine.static_layout.is_none());
    }

    #[test]
    fn zero_duration_infinite_css_animation_has_finite_terminal_snapshot() {
        // column_fixture also has a div root; animate only its intended child.
        let input = column_fixture(
            "@keyframes empty{}div>div{animation:empty 0s infinite}",
            vec![node(2, Some(1), "div", "")],
        );
        let mut engine = StyleEngine::default();
        engine
            .compute_at(&input, &LayoutLimits::default(), 0.)
            .unwrap();
        engine
            .compute_at(&input, &LayoutLimits::default(), 1.)
            .unwrap();
        let snapshots = engine.css_animations(1.);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].duration_ms, 0.);
        assert!(snapshots[0].current_time_ms.is_finite());
        assert_eq!(snapshots[0].play_state, "finished");
        assert!(!engine.has_active_animations());
    }
    #[test]
    fn css_animation_session_keeps_identity_across_native_mirror_rebuilds() {
        let target = node(2, Some(1), "div", "");
        let mut input = column_fixture(
            "@keyframes grow{from{width:10px}to{width:110px}}div{animation:grow 10s linear both;height:10px}",
            vec![target],
        );
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        let id = engine.css_animations(0.)[0].id;
        let halfway = engine.compute_at(&input, &limits, 5.).unwrap();
        assert_eq!(halfway.computed_styles[&2]["width"], "60px");
        input.nodes[1]
            .attributes
            .insert("title".into(), "unrelated mutation".into());
        input.nodes.reverse();
        let later = engine.compute_at(&input, &limits, 7.).unwrap();
        assert_eq!(later.computed_styles[&2]["width"], "80px");
        assert_eq!(engine.css_animations(7.)[0].id, id);
        engine
            .control_css_animation(id, NativeCssAnimationControl::Pause, None, 7.)
            .unwrap();
        let paused = engine.compute_at(&input, &limits, 9.).unwrap();
        assert_eq!(paused.computed_styles[&2]["width"], "80px");
    }
    #[test]
    fn native_effect_sampling_uses_typed_interpolation_and_animation_cascade() {
        let mut parent = node(2, Some(1), "div", "");
        parent.children = vec![3];
        parent.content = Some(vec![LayoutContent::Element(3)]);
        let mut input = column_fixture(
            "div{interest-delay-start:normal;color:black}span{color:inherit}",
            vec![parent],
        );
        input.nodes.push(node(3, Some(2), "span", "Child"));
        let frame = |offset, delay: &str, color: &str| NativeAnimationKeyframe {
            offset,
            easing: "linear".into(),
            composite: "replace".into(),
            properties: BTreeMap::from([
                ("interest-delay-start".into(), delay.into()),
                ("color".into(), color.into()),
            ]),
        };
        let mut engine = StyleEngine::default();
        engine.set_animation_effects(vec![NativeAnimationEffect {
            id: 17,
            target: 2,
            keyframes: vec![frame(0., "10s", "red"), frame(1., "30s", "blue")],
            progress: Some(0.5),
            composite: "replace".into(),
            ..Default::default()
        }]);
        let output = engine
            .compute_at(&input, &LayoutLimits::default(), 3.)
            .unwrap();
        assert_eq!(output.computed_styles[&2]["interest-delay-start"], "20s");
        assert_eq!(
            output.computed_styles[&3]["color"],
            output.computed_styles[&2]["color"]
        );
        assert_eq!(
            engine.animation_style_values(17)["interest-delay-start"],
            "20s"
        );
        engine.set_animation_effects(Vec::new());
        let cleared = engine
            .compute_at(&input, &LayoutLimits::default(), 4.)
            .unwrap();
        assert_eq!(
            cleared.computed_styles[&2]["interest-delay-start"],
            "normal"
        );
    }
    #[test]
    fn empty_keyframes_keep_css_lifecycle_without_changing_properties() {
        // column_fixture also has a div root; animate only its intended child.
        let input = column_fixture(
            "@keyframes empty{}div>div{animation:empty 1ms linear 2;width:15px;height:10px}",
            vec![node(2, Some(1), "div", "")],
        );
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        let running = engine.css_animations(0.);
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].name, "empty");
        let output = engine.compute_at(&input, &limits, 0.003).unwrap();
        assert_eq!(output.computed_styles[&2]["width"], "15px");
        let terminal = engine.css_animations(0.003);
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].id, running[0].id);
        assert_eq!(terminal[0].play_state, "finished");
        assert!(!terminal[0].in_effect);
    }

    #[test]
    fn css_sampling_crosses_multiple_iterations_and_keeps_zero_duration_terminal_records() {
        // column_fixture also has a div root; animate only its intended child.
        let input = column_fixture(
            "@keyframes grow{from{width:10px}to{width:110px}}div>div{height:10px;animation:grow 1s linear 10 alternate both}",
            vec![node(2, Some(1), "div", "")],
        );
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        let result = engine.compute_at(&input, &limits, 2.5).unwrap();
        assert_eq!(result.computed_styles[&2]["width"], "60px");
        assert_eq!(engine.css_animations(2.5)[0].current_time_ms, 2500.);
        let zero = column_fixture(
            "@keyframes instant{from{width:10px}to{width:110px}}div>div{height:10px;animation:instant 0s linear forwards}",
            vec![node(2, Some(1), "div", "")],
        );
        let mut engine = StyleEngine::default();
        engine.compute_at(&zero, &limits, 0.).unwrap();
        let terminal = engine.compute_at(&zero, &limits, 1.).unwrap();
        assert_eq!(terminal.computed_styles[&2]["width"], "110px");
        let records = engine.css_animations(1.);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].play_state, "finished");
        assert_eq!(records[0].current_time_ms, 0.);
        assert!(records[0].in_effect);
    }

    #[test]
    fn css_option_animation_restarts_when_native_picker_reopens() {
        let mut select = node(2, Some(1), "select", "");
        select.children = vec![3];
        select.content = Some(vec![LayoutContent::Element(3)]);
        let mut input = column_fixture(
            "select,::picker(select){appearance:base-select}@keyframes option-in{from{opacity:0}to{opacity:1}}option{animation:option-in 100s forwards}",
            vec![select],
        );
        input.nodes.push(node(3, Some(2), "option", "One"));
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        assert!(
            !engine
                .css_animations(0.)
                .iter()
                .any(|animation| animation.target == 3)
        );
        let entry = oryn_common::v4::TopLayerEntry {
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::SelectPicker,
            anchor_source: None,
            picker: None,
        };
        input.top_layer.push(entry);
        engine.compute_at(&input, &limits, 1.).unwrap();
        let first = engine
            .css_animations(1.)
            .into_iter()
            .find(|animation| animation.target == 3)
            .unwrap()
            .id;
        input.top_layer.clear();
        engine.compute_at(&input, &limits, 2.).unwrap();
        assert!(
            !engine
                .css_animations(2.)
                .iter()
                .any(|animation| animation.target == 3)
        );
        input.top_layer.push(entry);
        engine.compute_at(&input, &limits, 3.).unwrap();
        let second = engine
            .css_animations(3.)
            .into_iter()
            .find(|animation| animation.target == 3)
            .unwrap();
        assert_ne!(first, second.id);
        assert_eq!(second.play_state, "running");
    }

    #[test]
    fn lazy_pseudo_css_animations_use_the_document_timeline() {
        let target = node(2, Some(1), "select", "");
        let input = column_fixture(
            "@keyframes icon{from{opacity:0}to{opacity:1}}select{appearance:base-select}select::picker-icon{animation:icon 10s linear both}",
            vec![target],
        );
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        let output = engine.compute_at(&input, &limits, 5.).unwrap();
        assert_eq!(output.pseudo_styles[&2]["picker-icon"]["opacity"], "0.5");
        let css = engine.css_animations(5.);
        assert!(css.iter().any(|animation| {
            animation
                .pseudo
                .as_ref()
                .is_some_and(|pseudo| animation_pseudo_name(pseudo) == "::picker-icon")
        }));
    }

    #[test]
    fn lazy_pseudo_keyframes_inherit_origin_without_animating_primary_style() {
        let input = column_fixture(
            "@keyframes icon-font{from{font-size:inherit}to{font-size:40px}}select{appearance:base-select;font-size:20px;opacity:.8}select::picker-icon{animation:icon-font 10s linear both}",
            vec![node(2, Some(1), "select", "")],
        );
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        engine.compute_at(&input, &limits, 0.).unwrap();
        let output = engine.compute_at(&input, &limits, 5.).unwrap();
        assert_eq!(output.computed_styles[&2]["font-size"], "20px");
        assert_eq!(output.computed_styles[&2]["opacity"], "0.8");
        assert_eq!(output.pseudo_styles[&2]["picker-icon"]["font-size"], "30px");
    }

    #[test]
    fn media_states_drive_native_selectors_and_has_invalidation() {
        let video = node(2, Some(1), "video", "");
        let mut input = column_fixture(
            "video:paused{opacity:.2}video:playing{opacity:.8}div:has(video:stalled){color:red}video:seeking{background-color:blue}",
            vec![video],
        );
        let limits = LayoutLimits::default();
        let mut engine = StyleEngine::default();
        let first = engine.compute_at(&input, &limits, 0.).unwrap();
        assert_eq!(first.computed_styles[&2]["opacity"], "0.2");
        input.selector_states.insert(
            2,
            LayoutSelectorState {
                media_state: 1 | 4 | 8 | 16 | 32,
                ..Default::default()
            },
        );
        let second = engine.compute_at(&input, &limits, 0.).unwrap();
        assert_eq!(second.computed_styles[&2]["opacity"], "0.8");
        assert_eq!(
            oryn_common::color::parse_color(&second.computed_styles[&2]["background-color"]),
            Some([0, 0, 255, 255])
        );
        assert_eq!(
            oryn_common::color::parse_color(&second.computed_styles[&1]["color"]),
            Some([255, 0, 0, 255])
        );
        input.selector_states.get_mut(&2).unwrap().media_state = 3;
        let third = engine.compute_at(&input, &limits, 0.).unwrap();
        assert_eq!(third.computed_styles[&2]["opacity"], "0.2");
        assert_ne!(
            oryn_common::color::parse_color(&third.computed_styles[&1]["color"]),
            Some([255, 0, 0, 255])
        );
    }

    #[test]
    fn native_interest_pseudo_states_and_delays_reach_cascade() {
        let button = node(2, Some(1), "button", "Invoke");
        let target = node(3, Some(1), "div", "Target");
        let mut input = column_fixture(
            "button{interest-delay:123ms normal}button:interest-source{color:red}div:interest-target{background-color:blue}",
            vec![button, target],
        );
        input.selector_states.insert(
            2,
            LayoutSelectorState {
                interest_invoker: true,
                interest_source: true,
                ..Default::default()
            },
        );
        input.selector_states.insert(
            3,
            LayoutSelectorState {
                interest_target: true,
                ..Default::default()
            },
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&2]["interest-delay-start"], "0.123s");
        assert_eq!(output.computed_styles[&2]["interest-delay-end"], "normal");
        assert_eq!(
            oryn_common::color::parse_color(&output.computed_styles[&2]["color"]),
            Some([255, 0, 0, 255])
        );
        assert_eq!(
            oryn_common::color::parse_color(&output.computed_styles[&3]["background-color"]),
            Some([0, 0, 255, 255])
        );
        input.stylesheets[0].push_str("button{interest-delay-start:-1s;interest-delay-end:2s}");
        input.selector_states.get_mut(&2).unwrap().interest_source = false;
        let changed = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            changed.computed_styles[&2]["interest-delay-start"],
            "0.123s"
        );
        assert_eq!(changed.computed_styles[&2]["interest-delay-end"], "2s");
        assert_ne!(
            oryn_common::color::parse_color(&changed.computed_styles[&2]["color"]),
            Some([255, 0, 0, 255])
        );
    }

    #[test]
    fn interest_button_follows_after_and_anchors_to_native_origin() {
        let mut button = node(2, Some(1), "button", "Button");
        button.attributes.insert("id".into(), "origin".into());
        let mut input = column_fixture(
            "#origin{position:absolute;left:40px;top:30px;width:70px;height:20px;padding:0;border:0}#origin::before{content:'BEFORE'}#origin::after{content:'AFTER'}#origin::interest-button{content:'INTEREST';display:block;position:absolute;position-anchor:auto;left:anchor(right);top:anchor(top);width:50px;height:50px;min-width:0;min-height:0;margin:0}",
            vec![button],
        );
        input.selector_states.insert(
            2,
            LayoutSelectorState {
                interest_invoker: true,
                ..Default::default()
            },
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let pseudo = output
            .generated_nodes
            .iter()
            .find(|node| node.tag == "::interest-button")
            .unwrap();
        let box_ = pseudo.geometry.box_model.as_ref().unwrap().border_box;
        assert_eq!((box_.x, box_.y), (110., 30.));
        let pseudos = output
            .generated_nodes
            .iter()
            .filter(|entry| entry.parent_id == Some(2))
            .map(|entry| entry.tag.as_str())
            .collect::<Vec<_>>();
        assert_eq!(pseudos, vec!["::before", "::after", "::interest-button"]);
        input.selector_states.get_mut(&2).unwrap().interest_invoker = false;
        let removed = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(
            !removed
                .generated_nodes
                .iter()
                .any(|node| node.tag == "::interest-button")
        );
    }

    #[test]
    fn script_custom_properties_override_rules_and_recompute_inherited_values() {
        let mut parent = node(2, Some(1), "div", "");
        parent.attributes.insert("id".into(), "parent".into());
        parent.children = vec![3];
        parent.content = Some(vec![LayoutContent::Element(3)]);
        let child = node(3, Some(2), "span", "");
        let mut input = column_fixture(
            "@property --length{syntax:'<length>';inherits:true;initial-value:2px}#parent{--length:40px}span{display:block;width:var(--length);height:10px}",
            vec![parent],
        );
        input.nodes.push(child);
        let registration = oryn_dom::RegisteredCustomProperty {
            name: "--length".into(),
            syntax: "<length>".into(),
            inherits: false,
            initial_value: Some("13px".into()),
            base_url: input.document_base_url.clone(),
        };
        assert!(validate_custom_property(&registration).is_ok());
        input
            .registered_custom_properties
            .push(registration.clone());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&3]["--length"], "13px");
        assert_eq!(
            output.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .content_box
                .width,
            13.
        );
        input.registered_custom_properties[0].inherits = true;
        let changed = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(changed.computed_styles[&3]["--length"], "40px");
        let mut invalid = registration;
        invalid.initial_value = Some("1em".into());
        assert_eq!(
            validate_custom_property(&invalid),
            Err("SyntaxError".into())
        );
        invalid.syntax = "*".into();
        invalid.initial_value = None;
        assert!(validate_custom_property(&invalid).is_ok());
    }

    #[test]
    fn anchors_resolve_projected_geometry_and_recompute_on_name_changes() {
        let mut anchor = node(2, Some(1), "button", "");
        anchor.attributes.insert("id".into(), "source".into());
        let mut target = node(3, Some(1), "aside", "");
        target.attributes.insert("id".into(), "target".into());
        let mut input = column_fixture(
            "#source{position:absolute;left:40px;top:30px;width:70px;height:20px;padding:0;border:0;anchor-name:--source;transform:translate(10px,5px)}#target{position:fixed;position-anchor:--source;left:calc(anchor(right, 9px) + 2px);top:anchor(bottom, 11px);width:anchor-size(width, 13px);height:10px;margin:0}",
            vec![anchor, target],
        );
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.x, rect.y, rect.width), (122., 55., 70.));
        input.stylesheets[0] =
            input.stylesheets[0].replace("anchor-name:--source", "anchor-name:--renamed");
        let changed = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = changed.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.x, rect.y, rect.width), (11., 11., 13.));
        input.stylesheets[0] =
            input.stylesheets[0].replace("anchor-name:--renamed", "anchor-name:--source");
        input.scroll.y = 10.;
        let scrolled = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            scrolled.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .y,
            45.
        );
    }

    #[test]
    fn anchor_scope_survives_boxless_parent_without_css_identity_copies() {
        let mut scope = node(2, Some(1), "article", "");
        scope.attributes.insert("id".into(), "scope".into());
        scope.children = vec![3, 4];
        let mut inside = node(3, Some(2), "button", "");
        inside.attributes.insert("id".into(), "inside".into());
        let mut target = node(4, Some(2), "aside", "");
        target.attributes.insert("id".into(), "target".into());
        let mut outside = node(5, Some(1), "button", "");
        outside.attributes.insert("id".into(), "outside".into());
        let mut input = column_fixture(
            "div{position:relative;width:200px;height:200px}#scope{display:contents;anchor-scope:--a}button{position:absolute;anchor-name:--a;width:20px;height:10px;padding:0;border:0}#inside{left:10px;top:20px}#outside{left:80px;top:90px}#target{position:absolute;position-anchor:--a;left:anchor(right);top:anchor(bottom);width:10px;height:10px;margin:0}",
            vec![scope, inside, outside, target],
        );
        input.nodes[0].children = vec![2, 5];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.x, rect.y), (30., 30.));
        assert!(
            output
                .computed_styles
                .values()
                .all(|style| !style.contains_key("-oryn-anchor-parent"))
        );
    }

    #[test]
    fn anchor_start_uses_containing_block_direction_instead_of_target_direction() {
        let mut source = node(2, Some(1), "div", "");
        source.attributes.insert("id".into(), "source".into());
        let mut target = node(3, Some(1), "div", "");
        target.attributes.insert("id".into(), "target".into());
        let mut input = column_fixture(
            "#source{position:absolute;left:40px;top:30px;width:70px;height:20px;anchor-name:--a}#target{direction:ltr;position:fixed;position-anchor:--a;left:anchor(start);top:anchor(top);width:10px;height:10px}div{margin:0;padding:0;border:0}",
            vec![source, target],
        );
        input.stylesheets[0].push_str(":root{direction:rtl}#target{direction:ltr}");
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            output.geometry[&3].box_model.as_ref().unwrap().border_box.x,
            110.
        );
        input.stylesheets[0].push_str(":root{direction:ltr}");
        let changed = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            changed.geometry[&3]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .x,
            40.
        );
    }

    #[test]
    fn nested_top_layer_anchors_follow_native_invokers() {
        let mut button = node(2, Some(1), "button", "");
        button.attributes.insert("id".into(), "source".into());
        let mut popup = node(3, Some(1), "div", "");
        popup.attributes.insert("id".into(), "popup".into());
        popup.children = vec![4];
        popup.content = Some(vec![LayoutContent::Element(4)]);
        let nested = node(4, Some(3), "button", "");
        let mut last = node(5, Some(1), "div", "");
        last.attributes.insert("id".into(), "last".into());
        let mut input = column_fixture(
            "button{width:100px;height:20px;margin:0;padding:0;border:0}#source{position:absolute;left:100px;top:200px}#popup,#last{position:fixed;left:anchor(right);top:anchor(top);width:150px;height:60px;margin:0;padding:0;border:0}",
            vec![button, popup, last],
        );
        input.nodes.push(nested);
        input.top_layer = vec![
            oryn_common::v4::TopLayerEntry {
                anchor_source: Some(2),
                node_id: 3,
                kind: oryn_common::v4::TopLayerKind::Popover,
                picker: None,
            },
            oryn_common::v4::TopLayerEntry {
                anchor_source: Some(4),
                node_id: 5,
                kind: oryn_common::v4::TopLayerKind::Popover,
                picker: None,
            },
        ];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let first = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        let source = output.geometry[&4].box_model.as_ref().unwrap().border_box;
        let last = output.geometry[&5].box_model.as_ref().unwrap().border_box;
        assert_eq!((first.x, first.y), (200., 200.));
        assert_eq!((last.x, last.y), (source.right(), source.y));
    }

    #[test]
    fn native_position_try_changes_area_and_preserves_conditional_boxes() {
        let mut anchor = node(2, Some(1), "button", "");
        anchor.attributes.insert("id".into(), "source".into());
        let mut target = node(3, Some(1), "aside", "");
        target.attributes.insert("id".into(), "target".into());
        let mut input = column_fixture(
            "#source{position:absolute;left:160px;top:90px;width:20px;height:20px;padding:0;border:0;anchor-name:--source}#target{position:fixed;position-anchor:--source;position-area:bottom right;position-try-fallbacks:--above;position-visibility:anchors-visible;width:40px;height:30px;margin:0}@position-try --above{position-area:top left}",
            vec![anchor, target],
        );
        input.viewport.width = 200.;
        input.viewport.height = 120.;
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let rect = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!((rect.x, rect.y), (120., 60.));
        input.stylesheets[0].push_str("#source{visibility:hidden}");
        let hidden = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            hidden.geometry[&3].availability,
            GeometryAvailability::Hidden
        );
        assert!(hidden.geometry[&3].box_model.is_some());
        assert_eq!(hidden.computed_styles[&3]["visibility"], "visible");
        assert_eq!(hidden.computed_styles[&3]["-oryn-anchor-hidden"], "true");
    }

    #[test]
    fn native_fullscreen_uses_viewport_and_restores_author_style() {
        let mut input = column_fixture(
            "section{width:40px;height:20px;transform:translateX(50px)}section:fullscreen{color:red}",
            vec![node(2, Some(1), "section", "")],
        );
        input.viewport.width = 320.;
        input.viewport.height = 240.;
        input.selector_states.insert(
            2,
            LayoutSelectorState {
                fullscreen: true,
                ..Default::default()
            },
        );
        input.top_layer.push(oryn_common::v4::TopLayerEntry {
            node_id: 2,
            kind: oryn_common::v4::TopLayerKind::Fullscreen,
            anchor_source: None,
            picker: None,
        });
        let mut engine = StyleEngine::default();
        let limits = LayoutLimits::default();
        let output = engine.compute(&input, &limits).unwrap();
        let rect = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0., 0., 320., 240.)
        );
        assert!(output.backdrops.contains_key(&2));
        input.selector_states.get_mut(&2).unwrap().fullscreen = false;
        input.top_layer.clear();
        let restored = engine.compute(&input, &limits).unwrap();
        assert_eq!(
            restored.geometry[&2]
                .box_model
                .as_ref()
                .unwrap()
                .border_box
                .width,
            40.
        );
        assert!(restored.backdrops.is_empty());
    }
}

#[cfg(test)]
mod top_layer_clipping_tests;

#[cfg(test)]
mod repair42_layout_tests;
