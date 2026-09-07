//! Deterministic, non-painting style and layout service.
//!
//! The crate is linked only into the sandboxed page worker.  It accepts a
//! read-only snapshot of Oryn's canonical DOM and returns computed style and
//! geometry keyed by the stable DOM node identifier.  It never owns DOM,
//! JavaScript, networking, cookies, or browser policy.

pub mod cssom;

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
use regex::Regex;
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
    selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl},
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
    stylesheets::{Origin, UrlExtraData},
};
#[cfg(test)]
use style::{
    shared_lock::SharedRwLockReadGuard,
    stylesheets::{CssRuleType, CssRules},
};
use style_dom::ElementState;
#[cfg(test)]
use style_traits::ToCss;
use taffy::{
    prelude::{
        AvailableSpace, Display, FlexDirection, GridAutoFlow, Line, Size as TaffySize,
        Style as TaffyStyle, TaffyTree, auto, fr, length as taffy_length, line,
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

fn default_element_namespace() -> String {
    "http://www.w3.org/1999/xhtml".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNode {
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
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutInput {
    pub nodes: Vec<LayoutNode>,
    pub stylesheets: Vec<String>,
    /// Rust-owned `ElementInternals.states` keyed by the stable DOM node id.
    /// These are selector state, not serialized HTML attributes.
    #[serde(default)]
    pub custom_states: BTreeMap<u64, BTreeSet<String>>,
    /// Native focused element, projected into the private Stylo mirror.
    #[serde(default)]
    pub focused_node: Option<u64>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutOutput {
    /// CSSOM used values measured before transforms, including invisible boxes.
    /// Properties without an applicable layout box retain their computed value.
    #[serde(default)]
    pub resolved_values: BTreeMap<u64, BTreeMap<String, String>>,
    pub computed_styles: BTreeMap<u64, BTreeMap<String, String>>,
    pub geometry: BTreeMap<u64, NodeGeometry>,
    /// Untransformed border-box dimensions, including visibility-hidden boxes.
    #[serde(default)]
    pub offset_dimensions: BTreeMap<u64, (f64, f64)>,
    #[serde(default)]
    pub text_fragments: BTreeMap<u64, Vec<PaintTextFragment>>,
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
    #[error("viewport dimension {value} is outside 1..={limit} CSS px")]
    ViewportLimit { value: f64, limit: f64 },
    #[error("Stylo computed-value traversal failed: {0}")]
    StyleTraversal(String),
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

#[derive(Debug, Default)]
pub struct StyleEngine {
    _private: (),
}

impl StyleEngine {
    pub fn compute(
        &mut self,
        input: &LayoutInput,
        limits: &LayoutLimits,
    ) -> Result<LayoutOutput, LayoutError> {
        let computed_styles = stylo_computed_values(input)?;
        compute_with_styles(input, limits, computed_styles)
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
        let mut state = ElementState::ENABLED;
        if self.node.attributes.contains_key("disabled") {
            state.remove(ElementState::ENABLED);
            state.insert(ElementState::DISABLED);
        }
        if self.node.attributes.contains_key("checked") {
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
        _ns: &NamespaceConstraint<&<SelectorImpl as selectors::SelectorImpl>::NamespaceUrl>,
        local_name: &<SelectorImpl as selectors::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        self.node
            .attributes
            .get(local_name.0.as_ref())
            .is_some_and(|value| operation.eval_str(value))
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
}

struct TextEngine {
    fonts: FontContext,
    layout: LayoutContext<[u8; 4]>,
    available_families: BTreeSet<String>,
}

#[derive(Debug)]
struct TextLayout {
    lines: Vec<(String, f64, f64)>,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy)]
struct ChildPlacement {
    x: f64,
    y: f64,
    width: f64,
}

type PreparedFonts = (Vec<(String, Vec<u8>)>, Vec<LoadedFont>, Vec<String>);

impl TextEngine {
    fn new(web_fonts: &[(String, Vec<u8>)]) -> Self {
        let mut available_families = BTreeSet::from([DEFAULT_FONT_FAMILY.to_ascii_lowercase()]);
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
        for (family, bytes) in web_fonts {
            available_families.insert(family.to_ascii_lowercase());
            collection.register_fonts(
                fontique::Blob::new(Arc::new(bytes.clone())),
                Some(fontique::FontInfoOverride {
                    family_name: Some(family),
                    ..fontique::FontInfoOverride::default()
                }),
            );
        }
        Self {
            fonts: FontContext {
                collection,
                source_cache: fontique::SourceCache::default(),
            },
            layout: LayoutContext::new(),
            available_families,
        }
    }

    fn shape(&mut self, text: &str, width: f64, font_size: f64, font_family: &str) -> TextLayout {
        if text.is_empty() {
            return TextLayout {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }
        let mut builder = self.layout.ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size as f32));
        let requested_family = font_family
            .split(',')
            .next()
            .unwrap_or(DEFAULT_FONT_FAMILY)
            .trim()
            .trim_matches(['\'', '"']);
        // System fonts are intentionally disabled so layout stays reproducible.
        // CSS commonly requests Arial or another host font, however; shape those
        // runs with the embedded deterministic fallback instead of returning
        // zero-width glyph runs.
        let family = if self
            .available_families
            .contains(&requested_family.to_ascii_lowercase())
        {
            requested_family
        } else {
            DEFAULT_FONT_FAMILY
        };
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(
            family.to_string(),
        ))));
        let mut layout: Layout<[u8; 4]> = builder.build(text);
        layout.break_all_lines(Some(width.max(1.0) as f32));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let lines = layout
            .lines()
            .map(|line| {
                let metrics = line.metrics();
                (
                    text.get(line.text_range()).unwrap_or_default().to_string(),
                    f64::from(metrics.advance - metrics.trailing_whitespace),
                    f64::from(metrics.line_height),
                )
            })
            .collect();
        TextLayout {
            lines,
            width: f64::from(layout.width()),
            height: f64::from(layout.height()),
        }
    }
}

/// Compute a deterministic style/layout snapshot.  Geometry is expressed in
/// viewport coordinates, matching `getBoundingClientRect`.
pub fn compute(input: &LayoutInput, limits: &LayoutLimits) -> Result<LayoutOutput, LayoutError> {
    StyleEngine::default().compute(input, limits)
}

fn compute_with_styles(
    input: &LayoutInput,
    limits: &LayoutLimits,
    computed_styles: BTreeMap<u64, BTreeMap<String, String>>,
) -> Result<LayoutOutput, LayoutError> {
    validate(input, limits)?;
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
    let mut resolved_values = BTreeMap::new();
    let mut text_fragments = BTreeMap::new();
    let mut text_engine = TextEngine::new(&web_fonts);
    let mut cursor_y = 0.0;
    for root in roots {
        let height = layout_node(
            root,
            FlowBox {
                x: 0.0,
                y: cursor_y,
                width: input.viewport.width,
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
        );
        cursor_y += height;
    }
    let content_width = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|item| item.border_box.right() + input.scroll.x)
        .fold(input.viewport.width, f64::max);
    let content_height = geometry
        .values()
        .filter_map(|item| item.box_model.as_ref())
        .map(|item| item.border_box.bottom() + input.scroll.y)
        .fold(input.viewport.height, f64::max);
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
    Ok(LayoutOutput {
        resolved_values,
        computed_styles,
        geometry,
        offset_dimensions,
        text_fragments,
        scroll,
        diagnostics,
        loaded_fonts,
    })
}

const COMPUTED_PROPERTIES: &[&str] = &[
    "display",
    "position",
    "visibility",
    "opacity",
    "filter",
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
    "z-index",
    "background-color",
    "background-image",
    "color",
    "direction",
    "content",
    "text-transform",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "line-height",
    "text-align",
    "white-space",
    "overflow-x",
    "overflow-y",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "will-change",
    "outline-color",
    "outline-style",
    "outline-width",
    "accent-color",
    "float",
    "clear",
    "flex-direction",
    "row-gap",
    "column-gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-auto-flow",
    "grid-column-start",
    "grid-column-end",
    "grid-row-start",
    "grid-row-end",
    "margin-top",
    "margin-right",
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

fn custom_state_attribute(state: &str) -> String {
    state_attribute("data-oryn-custom-state-", state)
}

fn state_attribute(prefix: &str, state: &str) -> String {
    let mut encoded = String::from(prefix);
    for byte in state.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn rewrite_custom_state_selectors(stylesheet: &str) -> Cow<'_, str> {
    // The style mirror flattens a shadow child beneath its host while the live
    // DOM retains the real ShadowRoot boundary. Translate exported-part
    // matching into the equivalent private mirror relationship until Blitz
    // exposes Stylo's `is_part` hook.
    let part = Regex::new(r"::part\(\s*([-_a-zA-Z0-9]+)\s*\)")
        .expect("static shadow-part selector expression");
    let stylesheet = part.replace_all(stylesheet, |captures: &regex::Captures<'_>| {
        format!(" [part~={}]", &captures[1])
    });
    let direction =
        Regex::new(r":dir\(\s*(ltr|rtl)\s*\)").expect("static direction selector expression");
    let stylesheet = direction.replace_all(&stylesheet, |captures: &regex::Captures<'_>| {
        format!("[data-oryn-effective-dir={}]", &captures[1])
    });
    let language = Regex::new(r":lang\(\s*([-_a-zA-Z0-9]+)\s*\)")
        .expect("static language selector expression");
    let stylesheet = language.replace_all(&stylesheet, |captures: &regex::Captures<'_>| {
        format!("[data-oryn-effective-lang|={} i]", &captures[1])
    });
    let matcher = Regex::new(r":state\(\s*(--[-_a-zA-Z0-9]+)\s*\)")
        .expect("static custom-state selector expression");
    Cow::Owned(
        matcher
            .replace_all(&stylesheet, |captures: &regex::Captures<'_>| {
                format!("[{}]", custom_state_attribute(&captures[1]))
            })
            .into_owned(),
    )
}

/// Run Stylo's own DOM traversal and serialize the computed longhands consumed
/// by Oryn's non-painting Taffy/Parley layout layer. The mirror is an immutable
/// style snapshot: the live DOM remains Rust-owned by `oryn-dom`, while selector
/// matching, origins, cascade, variables and inheritance are all resolved by
/// Stylo rather than reconstructed by Oryn.
fn stylo_computed_values(
    input: &LayoutInput,
) -> Result<BTreeMap<u64, BTreeMap<String, String>>, LayoutError> {
    let viewport = BlitzViewport::new(
        input.viewport.width.clamp(1.0, f64::from(u32::MAX)) as u32,
        input.viewport.height.clamp(1.0, f64::from(u32::MAX)) as u32,
        1.0,
        match input.color_scheme {
            ColorScheme::Light => BlitzColorScheme::Light,
            ColorScheme::Dark => BlitzColorScheme::Dark,
        },
    );
    let mut document = BaseDocument::new(DocumentConfig {
        viewport: Some(viewport),
        base_url: Some("https://oryn.invalid/".into()),
        style_threading: blitz_dom::StyleThreading::Sequential,
        ..Default::default()
    });
    let html_namespace = Namespace::from("http://www.w3.org/1999/xhtml");
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut mirror_ids = BTreeMap::new();
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
                    if let Some(states) = input.custom_states.get(&node.id) {
                        attributes.extend(states.iter().map(|state| BlitzAttribute {
                            name: QualName::new(
                                None,
                                Namespace::from(""),
                                LocalName::from(custom_state_attribute(state).as_str()),
                            ),
                            value: String::new(),
                        }));
                    }
                    let mut cursor = Some(node.id);
                    let mut direction = None;
                    let mut language = None;
                    while let Some(current) = cursor {
                        let Some(ancestor) = by_id.get(&current).copied() else {
                            break;
                        };
                        direction = direction.or_else(|| ancestor.attributes.get("dir").cloned());
                        language = language.or_else(|| {
                            ancestor
                                .attributes
                                .get("lang")
                                .or_else(|| ancestor.attributes.get("xml:lang"))
                                .cloned()
                        });
                        if direction.is_some() && language.is_some() {
                            break;
                        }
                        cursor = ancestor.parent;
                    }
                    attributes.push(BlitzAttribute {
                        name: QualName::new(
                            None,
                            Namespace::from(""),
                            LocalName::from("data-oryn-effective-dir"),
                        ),
                        value: direction
                            .filter(|value| value.eq_ignore_ascii_case("rtl"))
                            .unwrap_or_else(|| "ltr".into()),
                    });
                    if let Some(language) = language {
                        attributes.push(BlitzAttribute {
                            name: QualName::new(
                                None,
                                Namespace::from(""),
                                LocalName::from("data-oryn-effective-lang"),
                            ),
                            value: language,
                        });
                    }
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
                && !explicit_text
            {
                let text = mutator.create_text_node(&node.text);
                mutator.append_children(mirror, &[text]);
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
    // The native DOM owns focus. Set Stylo state directly on the fresh mirror;
    // no synthetic HTML attributes or focus events are produced by a style flush.
    if let Some(focused) = input.focused_node.filter(|id| {
        mirror_ids.contains_key(id)
            && by_id
                .get(id)
                .is_some_and(|node| !matches!(node.tag.as_str(), "#text" | "#comment"))
    }) {
        let mut current = Some(focused);
        let mut visited = BTreeSet::new();
        while let Some(id) = current.filter(|id| visited.insert(*id)) {
            if let Some(mirror) = mirror_ids.get(&id).copied() {
                let node = document.get_node_mut(mirror).expect("mapped style node");
                node.element_state_mut().insert(ElementState::FOCUS_WITHIN);
                if id == focused {
                    node.element_state_mut().insert(ElementState::FOCUS);
                }
            }
            current = by_id.get(&id).and_then(|node| node.parent);
        }
    }
    // Building the mirror can parse <style> text automatically. CSSOM source
    // is owned by LayoutInput and can differ from that text; keeping both would
    // duplicate declarations and establish stale cascade-layer ordering.
    document.clear_author_stylesheets();
    for stylesheet in &input.stylesheets {
        let style_node = {
            let mut mutator = document.mutate();
            mutator.create_element(
                QualName::new(None, html_namespace.clone(), LocalName::from("style")),
                Vec::new(),
            )
        };
        // Blitz's Stylo adapter does not yet expose Servo's custom-state hook.
        // Project the Rust-owned state into private attributes on this
        // short-lived style mirror and let Stylo parse, match and cascade the
        // resulting selector. The live DOM and HTML serialization are never
        // modified by this compatibility bridge.
        let stylesheet = rewrite_custom_state_selectors(stylesheet);
        let stylesheet = document.make_stylesheet(&stylesheet, Origin::Author);
        document.add_stylesheet_for_node(stylesheet, style_node);
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
        document.resolve_stylist(0.0);
    }

    let mut output: BTreeMap<u64, BTreeMap<String, String>> = BTreeMap::new();
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
                for name in COMPUTED_PROPERTIES {
                    let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
                        continue;
                    };
                    let Err(longhand) = property.as_shorthand() else {
                        continue;
                    };
                    values.insert(
                        (*name).to_string(),
                        computed.computed_value_to_string(longhand),
                    );
                }
            } else {
                return Err(LayoutError::StyleTraversal(format!(
                    "Stylo did not resolve element {}",
                    node.id
                )));
            }
        }
        if node.attributes.contains_key("hidden") {
            values.insert("display".into(), "none".into());
        }
        output.insert(node.id, values);
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
                output.insert(*id, ua_style(node));
                return false;
            };
            let Some(parent_style) = output.get(&parent).cloned() else {
                return true;
            };
            let style = output.entry(*id).or_default();
            *style = parent_style;
            if node.tag == "#text" || node.tag == "#comment" {
                style.insert("display".into(), "inline".into());
            }
            false
        });
        if pending.len() == before {
            break;
        }
    }
    for node in &input.nodes {
        if node.attributes.contains_key("hidden") {
            output
                .entry(node.id)
                .or_default()
                .insert("display".into(), "none".into());
        }
    }
    Ok(output)
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
    styles: &BTreeMap<u64, BTreeMap<String, String>>,
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
        let mut ancestor = by_id.get(&id).and_then(|node| node.parent);
        while visible {
            let Some(parent_id) = ancestor else {
                break;
            };
            let Some(parent_node) = by_id.get(&parent_id).copied() else {
                visible = false;
                break;
            };
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
            let style = styles.get(&parent_id);
            let (clip_x, clip_y) = style.map_or((false, false), overflow_clip_axes);
            if clip_x || clip_y {
                let clip = parent_box.padding_box;
                visible = (!clip_x || horizontal_overlap(rect, clip))
                    && (!clip_y || vertical_overlap(rect, clip));
            }
            ancestor = parent_node.parent;
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

fn prepare_web_fonts(input: &LayoutInput, limits: &LayoutLimits) -> PreparedFonts {
    let mut prepared = Vec::new();
    let mut loaded = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total = 0usize;
    for font in &input.web_fonts {
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
        let decoded = match font.bytes.get(..4) {
            Some(b"wOFF") => wuff::decompress_woff1(&font.bytes).map_err(|error| error.to_string()),
            Some(b"wOF2") => wuff::decompress_woff2(&font.bytes).map_err(|error| error.to_string()),
            _ => Ok(font.bytes.clone()),
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                diagnostics.push(format!("font {} rejected: {error}", font.source_url));
                continue;
            }
        };
        if decoded.len() > limits.max_font_bytes || FontRef::new(&decoded).is_err() {
            diagnostics.push(format!(
                "font {} rejected: decoded font is invalid or exceeds the limit",
                font.source_url
            ));
            continue;
        }
        total = total.saturating_add(font.bytes.len());
        prepared.push((font.family.clone(), decoded));
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
    if input.nodes.len() > limits.max_nodes {
        return Err(LayoutError::NodeLimit {
            actual: input.nodes.len(),
            limit: limits.max_nodes,
        });
    }
    for value in [input.viewport.width, input.viewport.height] {
        if !value.is_finite() || !(1.0..=limits.max_viewport_css_px).contains(&value) {
            return Err(LayoutError::ViewportLimit {
                value,
                limit: limits.max_viewport_css_px,
            });
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
    for property in [
        "color",
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "line-height",
        "text-align",
        "visibility",
        "white-space",
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
    if node.attributes.contains_key("hidden") {
        style.insert("display".into(), "none".into());
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
        | "p" | "form" | "ul" | "ol" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre"
        | "blockquote" => "block",
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
            if node.tag == "pre" { "pre" } else { "normal" }.into(),
        ),
        ("overflow-x".into(), "visible".into()),
        ("overflow-y".into(), "visible".into()),
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
    styles: &BTreeMap<u64, BTreeMap<String, String>>,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    offset_dimensions: &mut BTreeMap<u64, (f64, f64)>,
    resolved_values: &mut BTreeMap<u64, BTreeMap<String, String>>,
    text_fragments: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    input: &LayoutInput,
    ancestor_hidden: bool,
    diagnostics: &mut Vec<String>,
    text_engine: &mut TextEngine,
) -> f64 {
    let Some(node) = by_id.get(&id).copied() else {
        return 0.0;
    };
    let style = &styles[&id];
    let no_box = ancestor_hidden || style.get("display").is_some_and(|value| value == "none");
    let invisible = style
        .get("visibility")
        .is_some_and(|value| value == "hidden" || value == "collapse");
    if no_box {
        geometry.insert(id, hidden_geometry());
        offset_dimensions.insert(id, (0.0, 0.0));
        for child in &node.children {
            layout_node(
                *child,
                containing,
                by_id,
                styles,
                geometry,
                offset_dimensions,
                resolved_values,
                text_fragments,
                input,
                true,
                diagnostics,
                text_engine,
            );
        }
        return 0.0;
    }
    let font_size = length(
        style_value(style, "font-size", "16px"),
        16.0,
        16.0,
        input.viewport,
    );
    let margin = edge(style, "margin", containing.width, font_size, input.viewport);
    let padding = edge(
        style,
        "padding",
        containing.width,
        font_size,
        input.viewport,
    );
    let border = border_edge(style, containing.width, font_size, input.viewport);
    let available = (containing.width - margin.left - margin.right).max(0.0);
    let specified_width = optional_length(
        style_value(style, "width", "auto"),
        available,
        font_size,
        input.viewport,
    );
    let intrinsic_text =
        if node.text.is_empty() && matches!(node.tag.as_str(), "input" | "button" | "option") {
            node.attributes.get("value").cloned().unwrap_or_default()
        } else if node.text.is_empty() && is_inline(style) {
            descendant_text(node, by_id)
        } else {
            node.text.clone()
        };
    let mut content_width = specified_width.unwrap_or_else(|| {
        if is_inline(style) {
            text_engine
                .shape(
                    &intrinsic_text,
                    available,
                    font_size,
                    style_value(style, "font-family", DEFAULT_FONT_FAMILY),
                )
                .width
                .min(available)
        } else {
            (available - padding.left - padding.right - border.left - border.right).max(0.0)
        }
    });
    if style_value(style, "box-sizing", "content-box") == "border-box" && specified_width.is_some()
    {
        content_width =
            (content_width - padding.left - padding.right - border.left - border.right).max(0.0);
    }
    let outer_width = content_width + padding.left + padding.right + border.left + border.right;
    let position = style_value(style, "position", "static");
    let mut x = containing.x + margin.left;
    let mut y = containing.y + margin.top;
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
    } else if matches!(position, "absolute" | "relative") {
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

    let line_height = if style_value(style, "line-height", "normal") == "normal" {
        font_size * 1.2
    } else {
        length(
            style_value(style, "line-height", "normal"),
            font_size,
            font_size,
            input.viewport,
        )
    };
    let text_layout = text_engine.shape(
        &intrinsic_text,
        content_width.max(1.0),
        font_size,
        style_value(style, "font-family", DEFAULT_FONT_FAMILY),
    );
    let intrinsic_text_height = text_layout.height.max(if intrinsic_text.is_empty() {
        0.0
    } else {
        line_height
    });
    let display = style_value(style, "display", "block");
    let child_x = x + border.left + padding.left;
    let child_y = y + border.top + padding.top + intrinsic_text_height;
    let mut children_height: f64 = 0.0;
    let has_float_or_clear = node.children.iter().any(|child| {
        styles.get(child).is_some_and(|child_style| {
            style_value(child_style, "float", "none") != "none"
                || style_value(child_style, "clear", "none") != "none"
        })
    });
    if matches!(display, "flex" | "grid" | "table-row") || has_float_or_clear {
        let Some(placements) =
            taffy_child_placements(node, style, styles, content_width, input.viewport)
        else {
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
                    width: placement.width,
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
            );
            children_height = children_height.max(placement.y + child_height);
        }
    } else {
        let mut cursor = child_y;
        for child in &node.children {
            let child_height = layout_node(
                *child,
                FlowBox {
                    x: child_x,
                    y: cursor,
                    width: content_width,
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
            );
            cursor += child_height;
            children_height += child_height;
        }
    }
    let specified_height = optional_length(
        style_value(style, "height", "auto"),
        input.viewport.height,
        font_size,
        input.viewport,
    );
    let mut content_height = specified_height
        .unwrap_or(intrinsic_text_height + children_height)
        .max(0.0);
    let border_box_sizing = style_value(style, "box-sizing", "content-box") == "border-box";
    if border_box_sizing && specified_height.is_some() {
        content_height =
            (content_height - padding.top - padding.bottom - border.top - border.bottom).max(0.0);
    }
    let border_width = outer_width.max(0.0);
    let border_height = content_height + padding.top + padding.bottom + border.top + border.bottom;
    offset_dimensions.insert(id, (border_width, border_height));
    // CSSOM resolves dimensions from layout, before transforms or visibility
    // suppress geometry. Non-replaced inline boxes have no used width/height.
    let replaced = matches!(
        node.tag.as_str(),
        "img"
            | "video"
            | "audio"
            | "canvas"
            | "iframe"
            | "embed"
            | "object"
            | "input"
            | "textarea"
            | "select"
            | "button"
    ) || (node.namespace == "http://www.w3.org/2000/svg" && node.tag == "svg");
    if !matches!(display, "none" | "contents") && (display != "inline" || replaced) {
        let mut values = BTreeMap::new();
        if !matches!(
            display,
            "table-row" | "table-row-group" | "table-header-group" | "table-footer-group"
        ) {
            let width = if border_box_sizing {
                border_width
            } else {
                content_width
            };
            values.insert("width".into(), format!("{width}px"));
        }
        if !matches!(display, "table-column" | "table-column-group") {
            let height = if border_box_sizing {
                border_height
            } else {
                content_height
            };
            values.insert("height".into(), format!("{height}px"));
        }
        resolved_values.insert(id, values);
    }

    let document_rect = transform_rect(
        CssRect {
            x,
            y,
            width: border_width,
            height: border_height,
        },
        style_value(style, "transform", "none"),
        font_size,
        input.viewport,
    );
    let rect = CssRect {
        x: if position == "fixed" {
            document_rect.x
        } else {
            document_rect.x - input.scroll.x
        },
        y: if position == "fixed" {
            document_rect.y
        } else {
            document_rect.y - input.scroll.y
        },
        ..document_rect
    };
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
    let client_rects = if intrinsic_text.is_empty() {
        if rect.width > 0.0 && rect.height > 0.0 {
            vec![rect]
        } else {
            Vec::new()
        }
    } else {
        text_layout
            .lines
            .iter()
            .enumerate()
            .map(|(line, (_, width, height))| CssRect {
                x: rect.x + border.left + padding.left,
                y: rect.y + border.top + padding.top + line as f64 * line_height,
                width: (*width).min(content_width),
                height: (*height).max(line_height),
            })
            .collect()
    };
    if !intrinsic_text.is_empty() && !invisible {
        let color = style_value(style, "color", "#000000").to_string();
        let font_family = style_value(style, "font-family", DEFAULT_FONT_FAMILY).to_string();
        let fragments = text_layout
            .lines
            .iter()
            .enumerate()
            .map(|(line, (text, width, height))| PaintTextFragment {
                text: text.clone(),
                rect: CssRect {
                    x: rect.x + border.left + padding.left,
                    y: rect.y + border.top + padding.top + line as f64 * line_height,
                    width: (*width).min(content_width),
                    height: (*height).max(line_height),
                },
                font_family: font_family.clone(),
                font_size,
                color: color.clone(),
            })
            .collect::<Vec<_>>();
        text_fragments.insert(id, fragments);
    }
    geometry.insert(
        id,
        NodeGeometry {
            availability: GeometryAvailability::Available,
            box_model: Some(BoxModel {
                border_box: rect,
                padding_box: inset(rect, border),
                content_box: inset(inset(rect, border), padding),
                margin,
                border,
                padding,
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
        geometry.insert(id, hidden_geometry());
    }
    if matches!(position, "absolute" | "fixed") {
        0.0
    } else {
        margin.top + border_height + margin.bottom
    }
}

fn descendant_text(node: &LayoutNode, by_id: &BTreeMap<u64, &LayoutNode>) -> String {
    let mut value = String::new();
    let mut pending = node.children.clone();
    while let Some(id) = pending.pop() {
        let Some(child) = by_id.get(&id) else {
            continue;
        };
        if !child.text.trim().is_empty() {
            if !value.is_empty() {
                value.push(' ');
            }
            value.push_str(child.text.trim());
        }
        pending.extend(child.children.iter().rev().copied());
        if value.len() >= 4096 {
            value.truncate(4096);
            break;
        }
    }
    value
}

/// Ask Taffy to place direct block/flex/grid children. Oryn retains ownership of
/// DOM identity and recursively measures text with Parley, while Taffy owns
/// the container algorithm and therefore the relational x/y/width result.
fn taffy_child_placements(
    node: &LayoutNode,
    parent_style: &BTreeMap<String, String>,
    styles: &BTreeMap<u64, BTreeMap<String, String>>,
    width: f64,
    viewport: ViewportState,
) -> Option<Vec<ChildPlacement>> {
    if node.children.is_empty() {
        return Some(Vec::new());
    }
    let parent_display = style_value(parent_style, "display", "block");
    let display = match parent_display {
        "flex" => Display::Flex,
        "grid" | "table-row" => Display::Grid,
        _ => Display::Block,
    };
    let font_size = length(
        style_value(parent_style, "font-size", "16px"),
        16.0,
        16.0,
        viewport,
    );
    let column_gap = length(
        style_value(
            parent_style,
            "column-gap",
            style_value(parent_style, "gap", "0px"),
        ),
        width,
        font_size,
        viewport,
    ) as f32;
    let row_gap = length(
        style_value(
            parent_style,
            "row-gap",
            style_value(parent_style, "gap", "0px"),
        ),
        width,
        font_size,
        viewport,
    ) as f32;
    let mut tree = TaffyTree::<()>::new();
    tree.disable_rounding();
    let mut children = Vec::with_capacity(node.children.len());
    for child in &node.children {
        let child_style = styles.get(child)?;
        let child_font_size = length(
            style_value(child_style, "font-size", "16px"),
            16.0,
            font_size,
            viewport,
        );
        let child_width = optional_length(
            style_value(child_style, "width", "auto"),
            width,
            child_font_size,
            viewport,
        );
        let child_height = optional_length(
            style_value(child_style, "height", "auto"),
            viewport.height,
            child_font_size,
            viewport,
        );
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
        let leaf = tree
            .new_leaf(TaffyStyle {
                size: TaffySize {
                    width: child_width
                        .map(|value| taffy_length(value as f32))
                        .unwrap_or_else(auto),
                    height: child_height
                        .map(|value| taffy_length(value as f32))
                        .unwrap_or_else(|| taffy_length((child_font_size * 1.2) as f32)),
                },
                float,
                clear,
                grid_column: grid_line(
                    style_value(child_style, "grid-column-start", "auto"),
                    style_value(child_style, "grid-column-end", "auto"),
                ),
                grid_row: grid_line(
                    style_value(child_style, "grid-row-start", "auto"),
                    style_value(child_style, "grid-row-end", "auto"),
                ),
                flex_grow: if child_width.is_none() { 1.0 } else { 0.0 },
                ..TaffyStyle::default()
            })
            .ok()?;
        children.push(leaf);
    }
    let columns = if parent_display == "table-row" {
        node.children.len().clamp(1, 64)
    } else {
        grid_column_count(style_value(parent_style, "grid-template-columns", "1fr"))
    };
    let root = tree
        .new_with_children(
            TaffyStyle {
                display,
                flex_direction: if style_value(parent_style, "flex-direction", "row")
                    .starts_with("column")
                {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                },
                size: TaffySize {
                    width: taffy_length(width as f32),
                    height: auto(),
                },
                gap: TaffySize {
                    width: taffy_length(column_gap),
                    height: taffy_length(row_gap),
                },
                grid_auto_flow: match style_value(parent_style, "grid-auto-flow", "row") {
                    "column" => GridAutoFlow::Column,
                    "column dense" | "dense column" => GridAutoFlow::ColumnDense,
                    "row dense" | "dense" | "dense row" => GridAutoFlow::RowDense,
                    _ => GridAutoFlow::Row,
                },
                grid_template_columns: if display == Display::Grid {
                    vec![fr(1.0); columns]
                } else {
                    Vec::new()
                },
                grid_template_rows: if display == Display::Grid {
                    vec![
                        fr(1.0);
                        grid_track_count(style_value(parent_style, "grid-template-rows", "1fr",))
                    ]
                } else {
                    Vec::new()
                },
                ..TaffyStyle::default()
            },
            &children,
        )
        .ok()?;
    tree.compute_layout(
        root,
        TaffySize {
            width: AvailableSpace::Definite(width as f32),
            height: AvailableSpace::MaxContent,
        },
    )
    .ok()?;
    children
        .iter()
        .map(|child| {
            let layout = tree.layout(*child).ok()?;
            Some(ChildPlacement {
                x: f64::from(layout.location.x),
                y: f64::from(layout.location.y),
                width: f64::from(layout.size.width),
            })
        })
        .collect()
}

fn grid_column_count(value: &str) -> usize {
    grid_track_count(value)
}

fn grid_track_count(value: &str) -> usize {
    let repeated = Regex::new(r"(?i)^repeat\(\s*(\d+)")
        .expect("constant grid repeat regex")
        .captures(value.trim())
        .and_then(|capture| capture[1].parse::<usize>().ok());
    repeated
        .unwrap_or_else(|| value.split_whitespace().count())
        .clamp(1, 64)
}

fn grid_line(start: &str, end: &str) -> Line<taffy::style::GridPlacement> {
    let placement = |value: &str| {
        value
            .trim()
            .parse::<i16>()
            .ok()
            .filter(|value| *value != 0)
            .map(line)
            .unwrap_or_default()
    };
    Line {
        start: placement(start),
        end: placement(end),
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

fn transform_rect(
    mut rect: CssRect,
    transform: &str,
    font_size: f64,
    viewport: ViewportState,
) -> CssRect {
    let arguments = |name: &str| {
        transform
            .split_once(&format!("{name}("))
            .and_then(|(_, value)| value.split_once(')').map(|(value, _)| value))
            .map(|value| value.split(',').map(str::trim).collect::<Vec<_>>())
    };
    if let Some(values) = arguments("translate") {
        rect.x += values
            .first()
            .map(|value| length(value, rect.width, font_size, viewport))
            .unwrap_or(0.0);
        rect.y += values
            .get(1)
            .map(|value| length(value, rect.height, font_size, viewport))
            .unwrap_or(0.0);
    }
    if let Some(values) = arguments("translateX") {
        rect.x += values
            .first()
            .map(|value| length(value, rect.width, font_size, viewport))
            .unwrap_or(0.0);
    }
    if let Some(values) = arguments("translateY") {
        rect.y += values
            .first()
            .map(|value| length(value, rect.height, font_size, viewport))
            .unwrap_or(0.0);
    }
    if let Some(values) = arguments("scale") {
        let x = values
            .first()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        let y = values
            .get(1)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(x);
        rect.width *= x.abs();
        rect.height *= y.abs();
    }
    rect
}

fn style_value<'a>(style: &'a BTreeMap<String, String>, name: &str, default: &'a str) -> &'a str {
    style.get(name).map(String::as_str).unwrap_or(default)
}

fn is_inline(style: &BTreeMap<String, String>) -> bool {
    matches!(
        style_value(style, "display", "block"),
        "inline" | "inline-block"
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
    let value = value.trim();
    let parse = |suffix: &str| value.strip_suffix(suffix)?.trim().parse::<f64>().ok();
    if let Some(value) = parse("px") {
        value
    } else if let Some(value) = parse("%") {
        basis * value / 100.0
    } else if let Some(value) = parse("rem") {
        16.0 * value
    } else if let Some(value) = parse("em") {
        font_size * value
    } else if let Some(value) = parse("vw") {
        viewport.width * value / 100.0
    } else if let Some(value) = parse("vh") {
        viewport.height * value / 100.0
    } else {
        value.parse().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn node(id: u64, parent: Option<u64>, tag: &str, text: &str) -> LayoutNode {
        LayoutNode {
            id,
            parent,
            children: Vec::new(),
            tag: tag.into(),
            namespace: default_element_namespace(),
            attributes: BTreeMap::new(),
            text: text.into(),
        }
    }

    #[test]
    fn cssom_layer_order_uses_effective_sheets_without_duplicate_dom_source() {
        let rules =
            "@layer first { #target { color: green; } } @layer second { #target { color: red; } }";
        let mut target = node(5, Some(2), "div", "");
        target.attributes.insert("id".into(), "target".into());
        let mut input = LayoutInput {
            nodes: vec![
                node(1, None, "html", ""),
                node(2, Some(1), "body", ""),
                node(3, Some(2), "style", ""),
                node(4, Some(2), "style", rules),
                target,
            ],
            stylesheets: vec![String::new(), rules.into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
    fn computed_styles_handle_document_level_text_without_inline_layout() {
        let input = LayoutInput {
            nodes: vec![node(1, None, "#text", "before"), node(2, None, "html", "")],
            stylesheets: vec!["html { color: rgb(1, 2, 3) }".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let styles = stylo_computed_values(&input).expect("computed styles");
        assert_eq!(styles[&2]["color"], "rgb(1, 2, 3)");
        let text_only = LayoutInput {
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
        let mut input = LayoutInput {
            nodes: vec![root, hidden, child, node(4, Some(3), "span", "")],
            stylesheets: vec!["section { display: none; color: red; opacity: .5 } div { color: green; width: 37px } div.changed { color: lime } span { background-color: blue }".into()],
            custom_states: BTreeMap::new(), focused_node: None,
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
            nodes: vec![
                root,
                node(2, Some(1), "div", "text"),
                node(3, Some(1), "div", ""),
            ],
            stylesheets: vec!["div { color: blue } div:empty { color: red }".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
            nodes: vec![
                root,
                node(2, Some(1), "button", "First"),
                node(3, Some(1), "button", "Second"),
            ],
            stylesheets: vec!["button { width: 120px; height: 30px; margin: 4px }".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
    }

    #[test]
    fn rust_owned_custom_states_participate_in_stylo_relational_selectors() {
        let mut subject = node(1, None, "div", "");
        subject.attributes.insert("id".into(), "subject".into());
        subject.children = vec![2];
        let input = LayoutInput {
            nodes: vec![subject, node(2, Some(1), "my-element", "")],
            stylesheets: vec![
                "#subject { background-color: #f00 } my-element:state(--green) { color: #0f0 } #subject:has(:state(--green)) { background-color: #0f0 }"
                    .into(),
            ],
            custom_states: BTreeMap::from([(2, BTreeSet::from(["--green".into()]))]),
            focused_node: None,
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
        let input = LayoutInput {
            nodes: vec![host, inner],
            stylesheets: vec![
                "my-element::part(inner) { background-color: #f00 } my-element::part(inner):dir(rtl) { background-color: #00f }"
                    .into(),
            ],
            custom_states: BTreeMap::new(),
            focused_node: None,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).expect("layout");
        assert_eq!(
            output.computed_styles[&2]["background-color"],
            "rgb(0, 0, 255)"
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
                nodes: vec![root, input_node, node(3, Some(1), "span", "Result")],
                stylesheets: vec!["div { font-family: Arial }".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
    fn bounded_table_rows_and_cells_have_common_web_geometry() {
        let mut table = node(1, None, "table", "");
        table.children.push(2);
        let mut row = node(2, Some(1), "tr", "");
        row.children.extend([3, 4]);
        let output = compute(
            &LayoutInput {
                nodes: vec![
                    table,
                    row,
                    node(3, Some(2), "td", "Name"),
                    node(4, Some(2), "td", "Value"),
                ],
                stylesheets: vec!["table{width:300px} td{padding:4px}".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![root, child],
                stylesheets: vec!["div { width: 100px; min-height: 20px }".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![root, outside, inside],
                stylesheets: Vec::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
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
        let mut input = LayoutInput {
            nodes: vec![
                root,
                node(2, Some(1), "input", ""),
                node(3, Some(1), "input", ""),
            ],
            stylesheets: vec![
                "* {color:black} :focus-within {color:red} :focus {color:blue}".into(),
            ],
            custom_states: BTreeMap::new(),
            focused_node: Some(2),
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
    fn stylo_relational_and_filtered_nth_rules_recascade_after_mutation() {
        let mut root = node(1, None, "main", "");
        root.children = vec![2, 3, 4];
        let mut nodes = vec![root];
        for (id, class) in [(2, "hot"), (3, "cold"), (4, "hot")] {
            let mut child = node(id, Some(1), "div", "");
            child.attributes.insert("class".into(), class.into());
            nodes.push(child);
        }
        let mut input = LayoutInput {
            nodes,
            stylesheets: vec!["main {color:black} main:has(> .hot + .cold) {color:green} div {width:10px} div:nth-child(2 of .hot) {width:73px}".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
        let input = LayoutInput {
            nodes: vec![root, html, svg, unqualified],
            stylesheets: vec!["@namespace svg url(http://www.w3.org/2000/svg); circle {color:red} svg|circle {color:green} |circle {color:blue}".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
            nodes: vec![root, html, svg, unqualified],
            stylesheets: vec!["* {color:red} [type=foo] {color:green}".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
            nodes: vec![root, scoped, inside, stop, outside, excluded],
            stylesheets: vec![
                "p {color:blue} @scope (.scope) to (.stop) { p {color:green} :scope {width:73px} }"
                    .into(),
            ],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
                nodes: vec![root, first, second],
                stylesheets: vec![
                    "button { color: black !important }\n\
                     #app > .action { color: red }\n\
                     .action + .action { width: 71px }\n\
                     @media (min-width: 600px) { #app > button { height: 33px } }"
                        .into(),
                ],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                // Deliberately place the child before its parent. A full style
                // traversal must still see inherited parent ComputedValues.
                nodes: vec![child, parent],
                stylesheets: Vec::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![root, child],
                stylesheets: vec!["button { color: black !important }".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
    fn persistent_style_engine_reuses_and_invalidates_stylesheets() {
        let input = |color: &str| LayoutInput {
            nodes: vec![node(1, None, "button", "Save")],
            stylesheets: vec![format!("button {{ color: {color} }}")],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
                nodes: vec![
                    root,
                    node(2, Some(1), "button", "One"),
                    node(3, Some(1), "button", "Two"),
                ],
                stylesheets: Vec::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![
                    root,
                    node(2, Some(1), "span", "One"),
                    node(3, Some(1), "span", "Two"),
                    node(4, Some(1), "span", "Three"),
                ],
                stylesheets: vec!["span{height:20px}".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
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
        let input = LayoutInput {
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", ""), node(4, Some(1), "div", ""), node(5, Some(1), "div", ""), node(6, Some(1), "span", "")],
            stylesheets: vec!["div { width: 200px } div > div { width: 50%; height: 100px; padding: 5px; border: 2px solid } div > div:nth-child(1) { transform: scale(2) } div > div:nth-child(2) { box-sizing: border-box } div > div:nth-child(3) { visibility: hidden } div > div:nth-child(4) { display: none } span { width: 50%; height: 100px }".into()],
            custom_states: BTreeMap::new(), focused_node: None,
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
    fn offset_dimensions_preserve_invisible_boxes_and_ignore_transforms() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2, 3, 4];
        let input = LayoutInput {
            nodes: vec![root, node(2, Some(1), "div", ""), node(3, Some(1), "div", ""), node(4, Some(1), "div", "")],
            stylesheets: vec!["div > div {width:100px;height:20px;padding:5px;border:2px solid} div > div:first-child {visibility:hidden} div > div:last-child {display:none}".into()],
            custom_states: BTreeMap::new(),
            focused_node: None,
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
        assert_eq!(
            output.geometry[&2].availability,
            GeometryAvailability::Hidden
        );
        assert_eq!(
            output.geometry[&3].box_model.as_ref().unwrap().border_box.y,
            34.0
        );
        let mut styles = output.computed_styles;
        styles
            .get_mut(&3)
            .unwrap()
            .insert("transform".into(), "scale(2)".into());
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
                nodes: vec![root, absolute, fixed, moved],
                stylesheets: Vec::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![root, floated, cleared],
                stylesheets: Vec::new(),
                custom_states: BTreeMap::new(),
                focused_node: None,
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
                nodes: vec![node(1, None, "p", "Measured")],
                stylesheets: vec!["p{font-family:Fixture Font}".into()],
                custom_states: BTreeMap::new(),
                focused_node: None,
                viewport: ViewportState::default(),
                scroll: ScrollState::default(),
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                web_fonts: vec![WebFont {
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
            nodes: vec![node(1, None, "p", "Measured")],
            stylesheets: Vec::new(),
            custom_states: BTreeMap::new(),
            focused_node: None,
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: vec![WebFont {
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
}
