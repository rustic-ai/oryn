//! Deterministic, non-painting style and layout service.
//!
//! The crate is linked only into the sandboxed page worker.  It accepts a
//! read-only snapshot of Oryn's canonical DOM and returns computed style and
//! geometry keyed by the stable DOM node identifier.  It never owns DOM,
//! JavaScript, networking, cookies, or browser policy.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use oryn_common::v4::{
    BoxModel, CssRect, EdgeSizes, GeometryAvailability, NodeGeometry, PaintTextFragment,
    ScrollState, ViewportState,
};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, Layout, LayoutContext, StyleProperty,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skrifa::FontRef;
use style::{
    context::QuirksMode,
    properties::parse_style_attribute,
    stylesheets::{CssRuleType, UrlExtraData},
};
use taffy::{
    prelude::{
        AvailableSpace, Display, FlexDirection, Size as TaffySize, Style as TaffyStyle, TaffyTree,
        auto, fr, length as taffy_length,
    },
    style::{Clear, Float},
};

pub const ENGINE_CONTRACT_VERSION: u16 = 1;
pub const DEFAULT_FONT_FAMILY: &str = "Oryn Noto Sans";
pub const DEFAULT_FONT_SHA256: &str =
    "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5";
pub const DEFAULT_FONT_BYTES: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");
pub const STYLE_ENGINE_VERSION: &str = "stylo-declarations-0.20+oryn-cascade-v1";
pub const BOX_ENGINE_VERSION: &str = "taffy-0.14";
pub const INLINE_ENGINE_VERSION: &str = "parley-0.11.1";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNode {
    pub id: u64,
    pub parent: Option<u64>,
    #[serde(default)]
    pub children: Vec<u64>,
    pub tag: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutInput {
    pub nodes: Vec<LayoutNode>,
    pub stylesheets: Vec<String>,
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
    pub computed_styles: BTreeMap<u64, BTreeMap<String, String>>,
    pub geometry: BTreeMap<u64, NodeGeometry>,
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
}

#[derive(Debug, Clone)]
struct Rule {
    selector: String,
    declarations: BTreeMap<String, String>,
    specificity: (u16, u16, u16),
    order: usize,
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
    validate(input, limits)?;
    let (web_fonts, loaded_fonts, mut diagnostics) = prepare_web_fonts(input, limits);
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let rules = parse_rules(&input.stylesheets, input);
    let mut computed_styles = BTreeMap::new();
    for node in &input.nodes {
        let parent_style = node.parent.and_then(|parent| computed_styles.get(&parent));
        computed_styles.insert(node.id, cascade(node, &by_id, parent_style, &rules));
    }

    let roots = input
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .map(|node| node.id)
        .collect::<Vec<_>>();
    let mut geometry = BTreeMap::new();
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
    Ok(LayoutOutput {
        computed_styles,
        geometry,
        text_fragments,
        scroll,
        diagnostics,
        loaded_fonts,
    })
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

fn parse_rules(stylesheets: &[String], input: &LayoutInput) -> Vec<Rule> {
    let comments = Regex::new(r"(?s)/\*.*?\*/").expect("constant regex");
    let mut rules = Vec::new();
    let mut order = 0;
    for sheet in stylesheets {
        let clean = comments.replace_all(sheet, "");
        parse_sheet(&clean, input, &mut rules, &mut order);
    }
    rules
}

fn parse_sheet(source: &str, input: &LayoutInput, rules: &mut Vec<Rule>, order: &mut usize) {
    let mut cursor = 0;
    while let Some(open_relative) = source[cursor..].find('{') {
        let open = cursor + open_relative;
        let prelude = source[cursor..open].trim();
        let mut depth = 1_u32;
        let mut close = open + 1;
        for (offset, character) in source[open + 1..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + 1 + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            break;
        }
        let body = &source[open + 1..close];
        if let Some(condition) = prelude.strip_prefix("@media") {
            if media_matches(condition.trim(), input) {
                parse_sheet(body, input, rules, order);
            }
        } else if !prelude.starts_with('@') {
            let rule_declarations = declarations(body);
            for selector in prelude
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                rules.push(Rule {
                    selector: selector.to_string(),
                    declarations: rule_declarations.clone(),
                    specificity: specificity(selector),
                    order: *order,
                });
                *order += 1;
            }
        }
        cursor = close + 1;
    }
}

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

fn cascade(
    node: &LayoutNode,
    by_id: &BTreeMap<u64, &LayoutNode>,
    parent: Option<&BTreeMap<String, String>>,
    rules: &[Rule],
) -> BTreeMap<String, String> {
    let mut style = ua_style(node);
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
    let mut winners = BTreeMap::<String, ((u16, u16, u16), usize, String)>::new();
    for rule in rules
        .iter()
        .filter(|rule| selector_matches(node, &rule.selector, by_id))
    {
        for (property, value) in &rule.declarations {
            let candidate = (rule.specificity, rule.order, value.clone());
            if winners
                .get(property)
                .is_none_or(|current| (candidate.0, candidate.1) >= (current.0, current.1))
            {
                winners.insert(property.clone(), candidate);
            }
        }
    }
    for (property, (_, _, value)) in winners {
        style.insert(property, resolve_value(&value, &style, parent));
    }
    if let Some(inline) = node.attributes.get("style") {
        for (property, value) in declarations(inline) {
            style.insert(property, resolve_value(&value, &style, parent));
        }
    }
    if node.attributes.contains_key("hidden") {
        style.insert("display".into(), "none".into());
    }
    style
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

fn declarations(input: &str) -> BTreeMap<String, String> {
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
    let mut canonical = String::new();
    block
        .to_css(&mut canonical)
        .expect("writing canonical CSS to a String cannot fail");
    let mut parsed = raw_declarations(&canonical);
    let raw = raw_declarations(input);
    for property in ["display", "grid-template-columns"] {
        if parsed.contains_key(property) {
            continue;
        }
        let Some(value) = raw.get(property) else {
            continue;
        };
        let accepted = match property {
            "display" => matches!(
                value.as_str(),
                "none" | "block" | "inline" | "inline-block" | "flex" | "grid"
            ),
            "grid-template-columns" => bounded_grid_template(value),
            _ => false,
        };
        if accepted {
            parsed.insert(property.into(), value.clone());
        }
    }
    parsed
}

fn bounded_grid_template(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character.is_ascii_whitespace()
                || matches!(character, '.' | '%' | '(' | ')' | ',' | '-')
        })
        && (value.contains("fr")
            || value.contains("px")
            || value.contains('%')
            || value.starts_with("repeat("))
}

fn raw_declarations(input: &str) -> BTreeMap<String, String> {
    input
        .split(';')
        .filter_map(|item| item.split_once(':'))
        .map(|(name, value)| {
            (
                name.trim().to_ascii_lowercase(),
                value
                    .trim()
                    .trim_end_matches("!important")
                    .trim()
                    .to_string(),
            )
        })
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .collect()
}

fn resolve_value(
    value: &str,
    current: &BTreeMap<String, String>,
    parent: Option<&BTreeMap<String, String>>,
) -> String {
    if value == "inherit" {
        return parent
            .and_then(|style| style.get("color"))
            .cloned()
            .unwrap_or_default();
    }
    if let Some(name) = value
        .strip_prefix("var(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (name, fallback) = name.split_once(',').unwrap_or((name, ""));
        return current
            .get(name.trim())
            .or_else(|| parent.and_then(|style| style.get(name.trim())))
            .cloned()
            .unwrap_or_else(|| fallback.trim().to_string());
    }
    value.to_string()
}

fn specificity(selector: &str) -> (u16, u16, u16) {
    let ids = selector.matches('#').count() as u16;
    let classes = selector.matches('.').count() as u16
        + selector.matches('[').count() as u16
        + selector.matches(':').count() as u16;
    let elements = selector
        .split(|character: char| character.is_whitespace() || matches!(character, '>' | '+' | '~'))
        .filter(|part| {
            part.chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        })
        .count() as u16;
    (ids, classes, elements)
}

fn selector_matches(node: &LayoutNode, selector: &str, by_id: &BTreeMap<u64, &LayoutNode>) -> bool {
    let parts = selector.split_whitespace().collect::<Vec<_>>();
    let Some(last) = parts.last() else {
        return false;
    };
    if !simple_selector_matches(node, last) {
        return false;
    }
    let mut ancestor = node.parent;
    for part in parts[..parts.len() - 1].iter().rev() {
        let Some(found) =
            ancestors(ancestor, by_id).find(|candidate| simple_selector_matches(candidate, part))
        else {
            return false;
        };
        ancestor = found.parent;
    }
    true
}

fn ancestors<'a>(
    mut id: Option<u64>,
    by_id: &'a BTreeMap<u64, &'a LayoutNode>,
) -> impl Iterator<Item = &'a LayoutNode> {
    std::iter::from_fn(move || {
        let node = id.and_then(|id| by_id.get(&id).copied());
        id = node.and_then(|node| node.parent);
        node
    })
}

fn simple_selector_matches(node: &LayoutNode, selector: &str) -> bool {
    let selector = selector.trim_matches(|character| matches!(character, '>' | '+' | '~'));
    if selector == "*" {
        return true;
    }
    if selector.contains(":hover") || selector.contains(":active") {
        return false;
    }
    if selector.contains(":root") && node.tag != "html" {
        return false;
    }
    let tag = selector
        .split(['#', '.', '[', ':'])
        .next()
        .unwrap_or_default();
    if !tag.is_empty() && tag != "*" && !tag.eq_ignore_ascii_case(&node.tag) {
        return false;
    }
    for id in selector.split('#').skip(1).map(token) {
        if node.attributes.get("id").is_none_or(|value| value != id) {
            return false;
        }
    }
    let classes = node
        .attributes
        .get("class")
        .map(|value| value.split_whitespace().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    if selector
        .split('.')
        .skip(1)
        .map(token)
        .any(|class| !classes.contains(class))
    {
        return false;
    }
    let attributes =
        Regex::new(r#"\[([\w-]+)(?:\s*=\s*['\"]?([^'\"\]]+)['\"]?)?\]"#).expect("constant regex");
    for capture in attributes.captures_iter(selector) {
        let Some(value) = node.attributes.get(&capture[1]) else {
            return false;
        };
        if capture
            .get(2)
            .is_some_and(|expected| value != expected.as_str())
        {
            return false;
        }
    }
    true
}

fn token(value: &str) -> &str {
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '-' && character != '_'
        })
        .next()
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn layout_node(
    id: u64,
    containing: FlowBox,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &BTreeMap<u64, BTreeMap<String, String>>,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
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
    let hidden = ancestor_hidden
        || style.get("display").is_some_and(|value| value == "none")
        || style
            .get("visibility")
            .is_some_and(|value| value == "hidden" || value == "collapse")
        || node
            .attributes
            .get("aria-hidden")
            .is_some_and(|value| value == "true");
    if hidden {
        geometry.insert(id, hidden_geometry());
        for child in &node.children {
            layout_node(
                *child,
                containing,
                by_id,
                styles,
                geometry,
                text_fragments,
                input,
                true,
                diagnostics,
                text_engine,
            );
        }
        return 0.0;
    }
    if style
        .get("display")
        .is_some_and(|value| value.starts_with("table"))
    {
        let diagnostic =
            "layout.table geometry is unsupported; structured table extraction remains available";
        diagnostics.push(format!("node {id}: {diagnostic}"));
        mark_unsupported_subtree(id, diagnostic, by_id, geometry);
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
    let gap = length(
        style_value(style, "gap", "0px"),
        content_width,
        font_size,
        input.viewport,
    );
    let child_x = x + border.left + padding.left;
    let child_y = y + border.top + padding.top + intrinsic_text_height;
    let mut children_height: f64 = 0.0;
    let has_float_or_clear = node.children.iter().any(|child| {
        styles.get(child).is_some_and(|child_style| {
            style_value(child_style, "float", "none") != "none"
                || style_value(child_style, "clear", "none") != "none"
        })
    });
    if matches!(display, "flex" | "grid") || has_float_or_clear {
        let placements = taffy_child_placements(node, style, styles, content_width, input.viewport)
            .unwrap_or_else(|| {
                let count = node.children.len().max(1) as f64;
                let child_width = ((content_width - gap * (count - 1.0)) / count).max(0.0);
                node.children
                    .iter()
                    .enumerate()
                    .map(|(index, _)| ChildPlacement {
                        x: index as f64 * (child_width + gap),
                        y: 0.0,
                        width: child_width,
                    })
                    .collect()
            });
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
    let content_height = specified_height
        .unwrap_or(intrinsic_text_height + children_height)
        .max(0.0);
    let border_width = outer_width.max(0.0);
    let border_height = content_height + padding.top + padding.bottom + border.top + border.bottom;
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
    if !intrinsic_text.is_empty() {
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
    let display = match style_value(parent_style, "display", "block") {
        "flex" => Display::Flex,
        "grid" => Display::Grid,
        _ => Display::Block,
    };
    let font_size = length(
        style_value(parent_style, "font-size", "16px"),
        16.0,
        16.0,
        viewport,
    );
    let gap = length(
        style_value(parent_style, "gap", "0px"),
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
                flex_grow: if child_width.is_none() { 1.0 } else { 0.0 },
                ..TaffyStyle::default()
            })
            .ok()?;
        children.push(leaf);
    }
    let columns = grid_column_count(style_value(parent_style, "grid-template-columns", "1fr"));
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
                    width: taffy_length(gap),
                    height: taffy_length(gap),
                },
                grid_template_columns: if display == Display::Grid {
                    vec![fr(1.0); columns]
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
    let repeated = Regex::new(r"(?i)^repeat\(\s*(\d+)")
        .expect("constant grid repeat regex")
        .captures(value.trim())
        .and_then(|capture| capture[1].parse::<usize>().ok());
    repeated
        .unwrap_or_else(|| value.split_whitespace().count())
        .clamp(1, 64)
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
            attributes: BTreeMap::new(),
            text: text.into(),
        }
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
            viewport: ViewportState::default(),
            scroll: ScrollState::default(),
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            web_fonts: Vec::new(),
        };
        let output = compute(&input, &LayoutLimits::default()).expect("layout");
        let first = output.geometry[&2].box_model.as_ref().unwrap().border_box;
        let second = output.geometry[&3].box_model.as_ref().unwrap().border_box;
        assert_eq!(first.width, 120.0);
        assert!(second.y > first.y);
        assert!(output.geometry[&2].client_rects[0].width > 0.0);
        assert_eq!(output.computed_styles[&2]["height"], "30px");
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
    fn table_geometry_is_explicitly_unavailable() {
        let output = compute(
            &LayoutInput {
                nodes: vec![node(1, None, "table", "")],
                stylesheets: Vec::new(),
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
            GeometryAvailability::Unsupported
        );
        assert!(output.geometry[&1].box_model.is_none());
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
        let parsed =
            declarations("color: rgb(255, 0, 0); width: calc(1px + ); not-a-real-property: nope");
        assert!(parsed.contains_key("color"));
        assert!(!parsed.contains_key("width"));
        assert!(!parsed.contains_key("not-a-real-property"));
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
