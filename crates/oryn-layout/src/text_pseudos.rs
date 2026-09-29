//! Native first formatted-line/letter styles. Metadata is generated exclusively
//! from Stylo's cascade and is carried alongside other private text metadata.
use super::*;
use serde::{Deserialize, Serialize};
use style::selector_parser::PseudoElement;
use style::servo_arc::Arc as ServoArc;
use unicode_segmentation::UnicodeSegmentation;

pub(super) const KEY: &str = "-oryn-text-pseudos";
pub(super) const PICKER_KEY: &str = "-oryn-picker-text-pseudos";
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Variants {
    pub line: Option<BTreeMap<String, String>>,
    pub letter: Option<BTreeMap<String, String>>,
    pub line_origin: Option<u64>,
    pub letter_origin: Option<u64>,
    pub active: bool,
}
pub(super) fn read(style: &BTreeMap<String, String>) -> Variants {
    style
        .get(KEY)
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}
fn serialize(style: &style::properties::ComputedValues) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for name in COMPUTED_PROPERTIES {
        if let Ok(property) = PropertyId::parse_enabled_for_all_content(name)
            && let Err(longhand) = property.as_shorthand()
        {
            values.insert((*name).to_owned(), style.computed_value_to_string(longhand));
        }
    }
    values
}
fn block(style: &BTreeMap<String, String>) -> bool {
    matches!(
        style_value(style, "display", "block"),
        "block" | "flow-root" | "list-item" | "table-cell" | "table-caption" | "inline-block"
    )
}
fn in_flow(style: &BTreeMap<String, String>) -> bool {
    style_value(style, "display", "block") != "none"
        && style_value(style, "float", "none") == "none"
        && !matches!(
            style_value(style, "position", "static"),
            "absolute" | "fixed"
        )
}
fn first_block(
    node: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    pseudos: &BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) -> Option<u64> {
    if pseudos
        .get(&node.id)
        .and_then(|p| p.get("before"))
        .is_some_and(|style| {
            !matches!(
                style_value(style, "content", "normal"),
                "normal" | "none" | "\"\"" | "''"
            )
        })
    {
        return None;
    }
    let content = node.content.clone().unwrap_or_else(|| {
        let mut items = vec![LayoutContent::StyledText {
            owner: node.id,
            node_id: None,
            text: node.text.clone(),
        }];
        items.extend(node.children.iter().copied().map(LayoutContent::Element));
        items
    });
    for item in content {
        match item {
            LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                let preserves_break = styles.get(&node.id).is_some_and(|style| {
                    matches!(
                        style_value(style, "white-space-collapse", "collapse"),
                        "preserve" | "preserve-breaks" | "break-spaces"
                    ) || matches!(
                        style_value(style, "white-space", "normal"),
                        "pre" | "pre-wrap" | "pre-line" | "break-spaces"
                    )
                });
                if !text.chars().all(char::is_whitespace)
                    || (preserves_break && text.contains(['\n', '\r']))
                {
                    return None;
                }
            }
            LayoutContent::Element(id) => {
                let (Some(child), Some(style)) = (nodes.get(&id), styles.get(&id)) else {
                    continue;
                };
                if !in_flow(style) {
                    continue;
                }
                if block(style) {
                    return Some(id);
                }
                if style_value(style, "display", "inline") == "contents"
                    && let Some(id) = first_block(child, nodes, styles, pseudos)
                {
                    return Some(id);
                }
                // A replaced or text-bearing inline participates in the first line.
                if is_replaced_box(child)
                    || !child.text.trim().is_empty()
                    || !child.children.is_empty()
                    || child
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.is_empty())
                {
                    return None;
                }
            }
        }
    }
    None
}
#[derive(Clone, Default)]
struct Native {
    line: Option<ServoArc<style::properties::ComputedValues>>,
    letter: Option<ServoArc<style::properties::ComputedValues>>,
    line_origin: Option<u64>,
    letter_origin: Option<u64>,
    active: bool,
}
/// Reparent native declarations, rather than overlaying property differences.
/// This preserves explicit inline values even when equal to the normal parent.
pub(super) fn cascade(
    document: &mut BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
    pseudos: &mut BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
) {
    cascade_from_source(
        document,
        input,
        mirrors,
        styles,
        pseudos,
        NativeSources::default(),
    );
}

pub(super) fn cascade_instances(
    document: &mut BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
    pseudos: &mut BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    primary: &BTreeMap<u64, ServoArc<style::properties::ComputedValues>>,
) {
    cascade_from_source(
        document,
        input,
        mirrors,
        styles,
        pseudos,
        NativeSources {
            primary: Some(primary),
        },
    );
}

#[derive(Clone, Copy, Default)]
struct NativeSources<'a> {
    primary: Option<&'a BTreeMap<u64, ServoArc<style::properties::ComputedValues>>>,
}
impl NativeSources<'_> {
    fn primary(
        &self,
        document: &BaseDocument,
        mirror: blitz_traits::node_id::NodeId,
        id: u64,
    ) -> Option<ServoArc<style::properties::ComputedValues>> {
        match self.primary {
            Some(styles) => styles.get(&id).cloned(),
            None => document.resolved_primary_style(mirror),
        }
    }
    fn pseudo(
        &self,
        document: &mut BaseDocument,
        mirror: blitz_traits::node_id::NodeId,
        id: u64,
        pseudo: &PseudoElement,
        nested: bool,
    ) -> Option<ServoArc<style::properties::ComputedValues>> {
        if self.primary.is_some() {
            let primary = self.primary(document, mirror, id)?;
            if nested {
                let picker = document.resolved_instance_pseudo_style(
                    mirror,
                    &PseudoElement::PickerSelect,
                    primary,
                )?;
                document.resolved_nested_pseudo_style_with_origin(mirror, pseudo, picker)
            } else {
                document.resolved_instance_pseudo_style(mirror, pseudo, primary)
            }
        } else if nested {
            document.resolved_nested_pseudo_style(mirror, &PseudoElement::PickerSelect, pseudo)
        } else {
            document.resolved_pseudo_style(mirror, pseudo)
        }
    }
    fn first_line(
        &self,
        document: &BaseDocument,
        mirror: blitz_traits::node_id::NodeId,
        id: u64,
        parent: &style::properties::ComputedValues,
    ) -> Option<ServoArc<style::properties::ComputedValues>> {
        let original = self.primary(document, mirror, id)?;
        document.reparent_first_line_style_from(mirror, &original, parent)
    }
}

fn cascade_from_source(
    document: &mut BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
    pseudos: &mut BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    source: NativeSources<'_>,
) {
    cascade_context(document, input, mirrors, styles, pseudos, None, source);
    for entry in &input.top_layer {
        if entry.kind != oryn_common::v4::TopLayerKind::SelectPicker {
            continue;
        }
        let Some(owner) = input.nodes.iter().find(|node| node.id == entry.node_id) else {
            continue;
        };
        let mut included = BTreeSet::new();
        let mut pending = vec![owner.id];
        while let Some(id) = pending.pop() {
            if included.insert(id)
                && let Some(node) = input.nodes.iter().find(|node| node.id == id)
            {
                pending.extend(node.children.iter().copied().filter(|child| {
                    input
                        .nodes
                        .iter()
                        .find(|node| node.id == *child)
                        .is_some_and(|node| node.tag != "button")
                }));
            }
        }
        let mut local = input.clone();
        local.nodes.retain(|node| included.contains(&node.id));
        for node in &mut local.nodes {
            node.children.retain(|child| included.contains(child));
            if node.id == owner.id {
                node.parent = None;
                node.content = Some(
                    node.children
                        .iter()
                        .copied()
                        .map(LayoutContent::Element)
                        .collect(),
                );
                node.text.clear();
            }
        }
        let mut local_styles = styles.clone();
        if let Some(picker) = pseudos
            .get(&owner.id)
            .and_then(|values| values.get("picker"))
        {
            local_styles.insert(owner.id, picker.clone());
        }
        cascade_context(
            document,
            &local,
            mirrors,
            &mut local_styles,
            pseudos,
            Some(owner.id),
            source,
        );
        for id in included {
            if let Some(value) = local_styles
                .get(&id)
                .and_then(|style| style.get(PICKER_KEY))
                && let Some(style) = styles.get_mut(&id)
            {
                Arc::make_mut(style).insert(PICKER_KEY.into(), value.clone());
            }
        }
    }
}
fn cascade_context(
    document: &mut BaseDocument,
    input: &LayoutInput,
    mirrors: &BTreeMap<u64, blitz_traits::node_id::NodeId>,
    styles: &mut ComputedStyles,
    pseudos: &mut BTreeMap<u64, BTreeMap<String, ComputedStyle>>,
    picker: Option<u64>,
    source: NativeSources<'_>,
) {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut pending = input
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let mut native = BTreeMap::<u64, Native>::new();
    while !pending.is_empty() {
        let before = pending.len();
        for id in pending.iter().copied().collect::<Vec<_>>() {
            let node = nodes[&id];
            if node.parent.is_some_and(|parent| pending.contains(&parent)) {
                continue;
            }
            pending.remove(&id);
            let (Some(mirror), Some(style)) = (mirrors.get(&id).copied(), styles.get(&id).cloned())
            else {
                continue;
            };
            let own_block = picker == Some(id) || block(&style);
            let mut inherited = node
                .parent
                .and_then(|parent| native.get(&parent).cloned())
                .unwrap_or_default();
            if !in_flow(&style)
                || is_replaced_box(node)
                || matches!(
                    style_value(&style, "display", "block"),
                    "flex" | "grid" | "inline-flex" | "inline-grid" | "table" | "inline-table"
                )
            {
                inherited = Native::default();
            }
            if own_block
                && node.parent.is_some_and(|parent| {
                    nodes.get(&parent).is_some_and(|parent| {
                        first_block(parent, &nodes, styles, pseudos) != Some(id)
                    })
                })
            {
                inherited = Native::default();
            }
            let line = if picker == Some(id) {
                source.pseudo(document, mirror, id, &PseudoElement::FirstLine, true)
            } else if own_block {
                source.pseudo(document, mirror, id, &PseudoElement::FirstLine, false)
            } else {
                None
            };
            let letter = if picker == Some(id) {
                source.pseudo(document, mirror, id, &PseudoElement::FirstLetter, true)
            } else if own_block {
                source.pseudo(document, mirror, id, &PseudoElement::FirstLetter, false)
            } else {
                None
            };
            let mut current = inherited;
            if let Some(parent) = &current.line {
                current.line = source.first_line(document, mirror, id, parent);
            }
            if let Some(own) = line {
                current.line = Some(if let Some(parent) = &current.line {
                    document
                        .reparent_text_pseudo_style(mirror, &own, parent)
                        .unwrap_or(own)
                } else {
                    own
                });
                current.line_origin = Some(id);
            }
            if let Some(own) = letter {
                current.letter = Some(own);
                current.letter_origin = Some(id);
            }
            if let Some(letter) = &current.letter
                && let Some(parent) = current
                    .line
                    .clone()
                    .or_else(|| source.primary(document, mirror, id))
            {
                current.letter = document.reparent_text_pseudo_style(mirror, letter, &parent);
            }
            current.active = own_block && (current.line.is_some() || current.letter.is_some());
            if current.line.is_some() || current.letter.is_some() {
                let variants = Variants {
                    line: current.line.as_deref().map(serialize),
                    letter: current.letter.as_deref().map(serialize),
                    line_origin: current.line_origin,
                    letter_origin: current.letter_origin,
                    active: current.active,
                };
                Arc::make_mut(styles.get_mut(&id).unwrap()).insert(
                    if picker.is_some() {
                        PICKER_KEY.into()
                    } else {
                        KEY.into()
                    },
                    serde_json::to_string(&variants).expect("native text pseudo values"),
                );
            }
            if let Some(line) = current.line.clone().or_else(|| {
                current
                    .letter
                    .as_ref()
                    .and_then(|_| source.primary(document, mirror, id))
            }) {
                for (name, pseudo) in [
                    ("before", PseudoElement::Before),
                    ("after", PseudoElement::After),
                ] {
                    let nested = picker == Some(id);
                    let key = if nested {
                        format!("picker::{name}")
                    } else {
                        name.into()
                    };
                    let original = source.pseudo(document, mirror, id, &pseudo, nested);
                    let Some(original) = original else { continue };
                    let Some(reparented) =
                        document.reparent_text_pseudo_style(mirror, &original, &line)
                    else {
                        continue;
                    };
                    let letter = current.letter.as_ref().and_then(|letter| {
                        document.reparent_text_pseudo_style(mirror, letter, &reparented)
                    });
                    let values = Variants {
                        line: current.line.as_ref().map(|_| serialize(&reparented)),
                        letter: letter.as_deref().map(serialize),
                        line_origin: current.line_origin,
                        letter_origin: current.letter_origin,
                        active: false,
                    };
                    if let Some(style) = pseudos
                        .get_mut(&id)
                        .and_then(|pseudos| pseudos.get_mut(&key))
                    {
                        Arc::make_mut(style).insert(
                            KEY.into(),
                            serde_json::to_string(&values)
                                .expect("native generated text pseudo values"),
                        );
                    }
                }
            }
            native.insert(id, current);
        }
        if pending.len() == before {
            break;
        }
    }
}

/// CSS first-letter includes adjoining punctuation and a complete typographic
/// character (extended grapheme), including flags, emoji and combining marks.
pub(super) fn letter_range(text: &str) -> Option<std::ops::Range<usize>> {
    use icu_properties::props::GeneralCategory as C;
    let categories = icu_properties::CodePointMapData::<C>::new();
    let punctuation = |cluster: &str| {
        cluster.chars().all(|ch| {
            matches!(
                categories.get(ch),
                C::OpenPunctuation
                    | C::ClosePunctuation
                    | C::InitialPunctuation
                    | C::FinalPunctuation
                    | C::OtherPunctuation
            )
        })
    };
    let mut start = None;
    let mut end = 0;
    let mut character = false;
    for (offset, cluster) in text.grapheme_indices(true) {
        if cluster.contains(['\n', '\u{2028}', '\u{2029}']) {
            break;
        }
        if start.is_none()
            && cluster
                .chars()
                .all(|ch| ch.is_whitespace() || matches!(ch, '\u{feff}' | '\u{200e}' | '\u{200f}'))
        {
            continue;
        }
        if character && !punctuation(cluster) {
            break;
        }
        start.get_or_insert(offset);
        end = offset + cluster.len();
        if !punctuation(cluster) {
            character = true;
        }
    }
    start.filter(|_| character).map(|start| start..end)
}

type OwnedRange = (
    std::ops::Range<usize>,
    BTreeMap<String, String>,
    Option<fontique::Language>,
    u64,
);
fn ranges(
    text: &str,
    style: &BTreeMap<String, String>,
    source: &[StyledTextRange<'_>],
    variants: &Variants,
    line_end: usize,
    letter: Option<&std::ops::Range<usize>>,
    viewport: ViewportState,
) -> Vec<OwnedRange> {
    let mut boundaries = BTreeSet::from([0, text.len(), line_end.min(text.len())]);
    for (range, _, _, _) in source {
        boundaries.insert(range.start);
        boundaries.insert(range.end);
    }
    if let Some(letter) = letter {
        boundaries.insert(letter.start);
        boundaries.insert(letter.end);
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    let mut output = Vec::new();
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        if range.is_empty() {
            continue;
        }
        let (base, language, owner) = source
            .iter()
            .rev()
            .find(|(source, _, _, _)| source.contains(&range.start))
            .map(|(_, style, language, owner)| (*style, *language, *owner))
            .unwrap_or((style, None, 0));
        let span = read(base);
        let mut resolved = base.clone();
        if range.start < line_end
            && let Some(line) = if span.line_origin == variants.line_origin {
                span.line.as_ref().or(variants.line.as_ref())
            } else {
                None
            }
        {
            text_decoration::add_pseudo(&mut resolved, line, viewport);
            resolved.extend(line.clone());
        }
        if letter.is_some_and(|letter| letter.contains(&range.start))
            && let Some(letter) = if span.letter_origin == variants.letter_origin {
                span.letter.as_ref().or(variants.letter.as_ref())
            } else {
                None
            }
        {
            text_decoration::add_pseudo(&mut resolved, letter, viewport);
            resolved.extend(letter.clone());
        }
        resolved.insert(
            "-oryn-text-pseudo-kind".into(),
            if letter.is_some_and(|letter| letter.contains(&range.start)) {
                "2"
            } else if range.start < line_end {
                "1"
            } else {
                "0"
            }
            .into(),
        );
        // Pseudo background belongs to the formatted text fragments, not the
        // enclosing block background. Ordinary inline backgrounds paint as boxes.
        if range.start < line_end
            && let Some(color) = variants
                .line
                .as_ref()
                .and_then(|line| line.get("background-color"))
        {
            resolved.insert("-oryn-text-background".into(), color.clone());
        }
        if letter.is_some_and(|letter| letter.contains(&range.start))
            && let Some(color) = resolved.get("background-color").cloned().filter(|color| {
                oryn_common::color::parse_color(color).is_some_and(|color| color[3] != 0)
            })
        {
            resolved.insert("-oryn-text-background".into(), color);
        }
        if letter.is_some_and(|letter| letter.contains(&range.start))
            && variants
                .letter
                .as_ref()
                .is_some_and(|style| style_value(style, "float", "none") != "none")
        {
            for (key, value) in [
                ("font-size", "0px"),
                ("line-height", "0px"),
                ("letter-spacing", "0px"),
                ("word-spacing", "0px"),
                ("-oryn-text-background", "transparent"),
            ] {
                resolved.insert(key.into(), value.into());
            }
        }
        resolved.remove(KEY);
        output.push((range, resolved, language, owner));
    }
    output
}
fn refs(ranges: &[OwnedRange]) -> Vec<StyledTextRange<'_>> {
    ranges
        .iter()
        .map(|(range, style, language, owner)| (range.clone(), style, *language, *owner))
        .collect()
}

pub(super) fn shape(
    engine: &mut TextEngine,
    text: &str,
    width: f64,
    font_size: f64,
    style: &BTreeMap<String, String>,
    viewport: ViewportState,
    source: &[StyledTextRange<'_>],
) -> TextLayout {
    let variants = read(style);
    let mut plain = style.clone();
    plain.remove(KEY);
    let letter = variants
        .letter
        .as_ref()
        .and_then(|_| {
            style
                .get("-oryn-first-letter-start")
                .and_then(|start| start.parse::<usize>().ok())
                .zip(
                    style
                        .get("-oryn-first-letter-end")
                        .and_then(|end| end.parse::<usize>().ok()),
                )
                .map(|(start, end)| start..end)
                .or_else(|| letter_range(text))
        })
        .filter(|letter| {
            letter.start < letter.end
                && letter.end <= text.len()
                && text.is_char_boundary(letter.start)
                && text.is_char_boundary(letter.end)
        })
        .filter(|letter| {
            !engine.active_inline_boxes.iter().any(|item| {
                item.kind == parley::InlineBoxKind::InFlow
                    && item.index <= letter.start
                    && item.width > 0.
                    && engine.inline_extents.contains_key(&item.id)
            })
        });
    let floating = letter
        .as_ref()
        .zip(variants.letter.as_ref())
        .filter(|(_, style)| style_value(style, "float", "none") != "none");
    let inline_letter = letter
        .as_ref()
        .zip(variants.letter.as_ref())
        .filter(|(_, style)| style_value(style, "float", "none") == "none")
        .map(|(range, style)| {
            let font = length(
                style_value(style, "font-size", "16px"),
                font_size,
                font_size,
                viewport,
            );
            (
                range.clone(),
                style.clone(),
                edge(style, "margin", width, font, viewport),
                edge(style, "padding", width, font, viewport),
                border_edge(style, width, font, viewport),
            )
        });
    let original_boxes = engine.active_inline_boxes.clone();
    let mut private_boxes = BTreeSet::new();
    if let Some((range, _, margin, padding, border)) = &inline_letter {
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let edges = if vertical {
            [
                margin.top + padding.top + border.top,
                margin.bottom + padding.bottom + border.bottom,
            ]
        } else {
            [
                margin.left + padding.left + border.left,
                margin.right + padding.right + border.right,
            ]
        };
        let mut next = u64::MAX;
        for (index, extent) in [range.start, range.end].into_iter().zip(edges) {
            if extent == 0. {
                continue;
            }
            while engine
                .active_inline_boxes
                .iter()
                .any(|item| item.id == next)
                || engine.inline_sizes.contains_key(&next)
            {
                next -= 1;
            }
            private_boxes.insert(next);
            engine.active_inline_boxes.push(parley::InlineBox {
                id: next,
                kind: parley::InlineBoxKind::InFlow,
                index,
                width: extent as f32,
                height: 0.,
            });
            next -= 1;
        }
        engine.active_inline_boxes.sort_by_key(|item| item.index);
    }
    let floated = floating.map(|(range, style)| {
        let mut local = style.clone();
        local.remove(KEY);
        local.remove("-oryn-first-line-end");
        local.insert("text-wrap-mode".into(), "nowrap".into());
        local.insert("text-indent".into(), "0px".into());
        local.insert("text-align".into(), "left".into());
        local.insert("float".into(), "none".into());
        let owner = source
            .iter()
            .find(|(span, _, _, _)| span.contains(&range.start))
            .map_or(0, |(_, _, _, owner)| *owner);
        let font = length(
            style_value(&local, "font-size", "16px"),
            font_size,
            font_size,
            viewport,
        );
        let old_context = engine.float_context.take();
        let old_boxes = std::mem::take(&mut engine.active_inline_boxes);
        let dropcap = engine.shape_paragraph(crate::ParagraphShapeInputs {
            text: &text[range.clone()],
            width: 1_000_000.,
            font_size: font,
            style: &local,
            viewport,
            ranges: &[(0..range.len(), &local, None, owner)],
            first_paragraph: true,
        });
        engine.float_context = old_context;
        engine.active_inline_boxes = old_boxes;
        let margin = edge(style, "margin", width, font, viewport);
        let padding = edge(style, "padding", width, font, viewport);
        let border = border_edge(style, width, font, viewport);
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let content = CssRect {
            x: 0.,
            y: 0.,
            width: if vertical {
                dropcap.height
            } else {
                dropcap.width
            },
            height: if vertical {
                dropcap.width
            } else {
                dropcap.height
            },
        };
        let padding_box = CssRect {
            x: -padding.left,
            y: -padding.top,
            width: content.width + padding.left + padding.right,
            height: content.height + padding.top + padding.bottom,
        };
        let border_box = CssRect {
            x: padding_box.x - border.left,
            y: padding_box.y - border.top,
            width: padding_box.width + border.left + border.right,
            height: padding_box.height + border.top + border.bottom,
        };
        let text_box = oryn_common::v4::PaintTextBox {
            owner: variants.letter_origin.unwrap_or(0),
            model: BoxModel {
                content_box: content,
                padding_box,
                border_box,
                padding,
                border,
                margin,
            },
            style: style.clone(),
        };
        let mut extent = EdgeSizes {
            left: margin.left + padding.left + border.left,
            right: margin.right + padding.right + border.right,
            top: margin.top + padding.top + border.top,
            bottom: margin.bottom + padding.bottom + border.bottom,
        };
        if writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
            .vertical
        {
            extent = EdgeSizes {
                left: extent.top,
                right: extent.bottom,
                top: extent.left,
                bottom: extent.right,
            };
        }
        let context = engine
            .float_context
            .get_or_insert_with(taffy::compute::FloatContext::new);
        context.set_width(engine.float_context_width.max(width) as f32);
        let spec = float_lines::FloatSpec {
            width: dropcap.width + extent.left + extent.right,
            height: dropcap.height + extent.top + extent.bottom,
            right: style_value(style, "float", "none") == "right",
            clear: taffy::Clear::None,
            monolithic: false,
        };
        let position = spec.place(
            context,
            engine.float_block_offset,
            engine.float_inline_insets.map(|value| value as f32),
            engine.float_fragment,
        );
        (
            range.clone(),
            dropcap,
            f64::from(position.x) - engine.float_inline_insets[0] + extent.left,
            f64::from(position.y) - engine.float_block_offset + extent.top,
            text_box,
        )
    });
    // A floated letter is outside the paragraph's bidi ordering. Boundary-neutral
    // placeholders retain byte coordinates while removing its directional input;
    // the original text is restored on the returned source projection below.
    let masked = floated.as_ref().map(|(range, _, _, _, _)| {
        let mut value = text.to_owned();
        value.replace_range(range.clone(), &"\0".repeat(range.len()));
        value
    });
    let paragraph_text = masked.as_deref().unwrap_or(text);
    let line_end = if let Some(end) = style
        .get("-oryn-first-line-end")
        .and_then(|end| end.parse::<usize>().ok())
    {
        end.min(text.len())
    } else if variants.line.is_some() {
        let full = ranges(
            text,
            style,
            source,
            &variants,
            text.len(),
            letter.as_ref(),
            viewport,
        );
        // First-line style may change the break itself. Probe the complete
        // candidate line using native fonts and exactly the same float bands.
        let saved = engine.float_context.clone();
        let probe = engine.shape_paragraph(crate::ParagraphShapeInputs {
            text: paragraph_text,
            width,
            font_size,
            style: &plain,
            viewport,
            ranges: &refs(&full),
            first_paragraph: true,
        });
        engine.float_context = saved;
        probe.line_ranges.first().map_or(0, |range| range.end)
    } else {
        0
    };
    let selected = ranges(
        text,
        style,
        source,
        &variants,
        line_end,
        letter.as_ref(),
        viewport,
    );
    if line_end > 0 && line_end < text.len() {
        plain.insert("-oryn-first-line-end".into(), line_end.to_string());
    }
    let mut result = engine.shape_paragraph(crate::ParagraphShapeInputs {
        text: paragraph_text,
        width,
        font_size,
        style: &plain,
        viewport,
        ranges: &refs(&selected),
        first_paragraph: true,
    });
    if masked.is_some() {
        for (line, range) in result.lines.iter_mut().zip(&result.line_ranges) {
            if let Some(original) = text.get(range.clone()) {
                line.0 = original.to_owned();
            }
        }
    }
    if let Some((range, mut dropcap, x, y, mut text_box)) = floated {
        // Replace only the source interval's zero-advance shaping projections;
        // DOM text, source byte ranges and UTF-16 ownership remain.
        for (runs, provenance) in result.glyph_runs.iter_mut().zip(&mut result.provenance) {
            let mut keep = Vec::new();
            for (run, source) in runs.drain(..).zip(provenance.drain(..)) {
                if source.bytes.end <= range.start || source.bytes.start >= range.end {
                    keep.push((run, source));
                }
            }
            for (run, source) in keep {
                runs.push(run);
                provenance.push(source);
            }
        }
        result
            .clusters
            .retain(|cluster| cluster.bytes.end <= range.start || cluster.bytes.start >= range.end);
        if text[..range.start].trim().is_empty() && text[range.end..].trim().is_empty() {
            result.height = 0.;
            result.width = 0.;
            if let Some(line) = result.lines.first_mut() {
                line.1 = 0.;
                line.2 = 0.;
            }
        }
        if let Some(line) = result.lines.first() {
            let left = line.4;
            let top = line.3;
            let mode = style_value(style, "writing-mode", "horizontal-tb");
            let vertical = writing_mode::Axes::new(mode, false).vertical;
            let (dx, dy) = if vertical {
                (
                    if matches!(mode, "vertical-rl" | "sideways-rl") {
                        line.2 - dropcap.height + top - y
                    } else {
                        y - top
                    },
                    x - left,
                )
            } else {
                (x - left, y - top)
            };
            for (runs, sources) in dropcap.glyph_runs.iter_mut().zip(&mut dropcap.provenance) {
                for (run, source) in runs.iter_mut().zip(sources) {
                    for glyph in &mut run.glyphs {
                        glyph.x += dx;
                        glyph.y += dy;
                    }
                    if let Some(rect) = &mut run.decoration_rect {
                        rect.x += dx;
                        rect.y += dy;
                    }
                    source.bytes.start += range.start;
                    source.bytes.end += range.start;
                    for bytes in &mut source.glyph_ranges {
                        bytes.start += range.start;
                        bytes.end += range.start;
                    }
                }
            }
            text_box.translate(dx, dy);
            if let Some(first) = dropcap.glyph_runs.iter_mut().flatten().next() {
                first.background = None;
                first.text_box = Some(text_box);
            }
            result.glyph_runs[0].extend(dropcap.glyph_runs.into_iter().flatten());
            result.provenance[0].extend(dropcap.provenance.into_iter().flatten());
            for mut cluster in dropcap.clusters {
                cluster.bytes.start += range.start;
                cluster.bytes.end += range.start;
                cluster.line = 0;
                cluster.rect.x += x;
                cluster.rect.y += y;
                result.clusters.push(cluster);
            }
        }
    }
    engine.active_inline_boxes = original_boxes;
    result
        .inline_boxes
        .retain(|item| !private_boxes.contains(&item.node));
    if let Some((range, letter_style, margin, padding, border)) = inline_letter {
        for (runs, provenance) in result.glyph_runs.iter_mut().zip(&result.provenance) {
            let mut bounds: Option<CssRect> = None;
            for (run, source) in runs.iter().zip(provenance) {
                if source.bytes.start < range.end
                    && source.bytes.end > range.start
                    && let Some(rect) = run.decoration_rect
                {
                    bounds = Some(
                        bounds
                            .map(|old| {
                                let x = old.x.min(rect.x);
                                let y = old.y.min(rect.y);
                                CssRect {
                                    x,
                                    y,
                                    width: old.right().max(rect.right()) - x,
                                    height: old.bottom().max(rect.bottom()) - y,
                                }
                            })
                            .unwrap_or(rect),
                    );
                }
            }
            if let Some(content) = bounds {
                let padding_box = CssRect {
                    x: content.x - padding.left,
                    y: content.y - padding.top,
                    width: content.width + padding.left + padding.right,
                    height: content.height + padding.top + padding.bottom,
                };
                let border_box = CssRect {
                    x: padding_box.x - border.left,
                    y: padding_box.y - border.top,
                    width: padding_box.width + border.left + border.right,
                    height: padding_box.height + border.top + border.bottom,
                };
                let mut attached = false;
                for (run, source) in runs.iter_mut().zip(provenance) {
                    if source.bytes.start < range.end && source.bytes.end > range.start {
                        run.background = None;
                        if !attached {
                            run.text_box = Some(oryn_common::v4::PaintTextBox {
                                owner: variants.letter_origin.unwrap_or(0),
                                model: BoxModel {
                                    content_box: content,
                                    padding_box,
                                    border_box,
                                    margin,
                                    padding,
                                    border,
                                },
                                style: letter_style.clone(),
                            });
                            attached = true;
                        }
                    }
                }
            }
        }
    }
    if line_end > 0
        && let (Some(line_style), Some(line)) = (variants.line.as_ref(), result.lines.first())
    {
        let background_style = line_style
            .iter()
            .filter(|(key, _)| {
                key.starts_with("background-") || matches!(key.as_str(), "color" | "font-size")
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: if vertical { line.2 } else { line.1 },
            height: if vertical { line.1 } else { line.2 },
        };
        if let (Some(runs), Some(provenance)) =
            (result.glyph_runs.first_mut(), result.provenance.first())
        {
            for (run, source) in runs.iter_mut().zip(provenance) {
                if !letter.as_ref().is_some_and(|letter| {
                    source.bytes.start < letter.end
                        && source.bytes.end > letter.start
                        && variants
                            .letter
                            .as_ref()
                            .and_then(|style| style.get("background-color"))
                            .and_then(|color| oryn_common::color::parse_color(color))
                            .is_some_and(|color| color[3] > 0)
                }) {
                    run.background = None;
                }
            }
            if let Some(run) = runs.first_mut() {
                run.line_box = Some(oryn_common::v4::PaintTextBox {
                    owner: variants.line_origin.unwrap_or(0),
                    model: BoxModel {
                        content_box: rect,
                        padding_box: rect,
                        border_box: rect,
                        margin: Default::default(),
                        padding: Default::default(),
                        border: Default::default(),
                    },
                    style: background_style,
                });
            }
        }
    }
    result
}

/// Read demand from final line runs, after pseudo styling and transformations.
/// Measurement probes never publish here; each laid-out node replaces its prior
/// entry, so font completion cannot retain a stale first-line demand range.
pub(super) fn font_demands(
    layout: &TextLayout,
) -> Result<BTreeMap<String, BTreeSet<u32>>, LayoutError> {
    let mut bytes = 0usize;
    let mut demands = BTreeMap::<String, BTreeSet<u32>>::new();
    for (index, runs) in layout.provenance.iter().enumerate() {
        let (Some(line), Some(range)) = (layout.lines.get(index), layout.line_ranges.get(index))
        else {
            continue;
        };
        for run in runs {
            let text = run
                .text_override
                .as_deref()
                .or_else(|| {
                    let start = run.bytes.start.max(range.start);
                    let end = run.bytes.end.min(range.end);
                    (start <= end)
                        .then(|| line.0.get(start - range.start..end - range.start))
                        .flatten()
                })
                .unwrap_or("");
            if text.is_empty() {
                continue;
            }
            for family in cssom::font_family_names(&run.families) {
                let family = family.to_ascii_lowercase();
                if !demands.contains_key(&family) {
                    bytes = bytes.saturating_add(family.len() + 128);
                }
                let characters = demands.entry(family).or_default();
                for character in cssom::font_text_codepoints(text) {
                    if characters.insert(character) {
                        bytes = bytes.saturating_add(32);
                    }
                    if bytes > 64 * 1024 * 1024 {
                        return Err(LayoutError::StyleTraversal(
                            "font demand exceeds native 64-MiB bound".into(),
                        ));
                    }
                }
            }
        }
    }
    Ok(demands)
}

fn transforms(
    source: &text_source::TextSource,
    styles: &ComputedStyles,
    default: &BTreeMap<String, String>,
    variants: &Variants,
    line_end: usize,
    letter: Option<&std::ops::Range<usize>>,
) -> BTreeMap<usize, String> {
    source
        .spans
        .iter()
        .map(|span| {
            let base = styles
                .get(&span.owner)
                .map(|style| style.as_ref())
                .unwrap_or(default);
            let local = read(base);
            let mut chosen = base;
            if span.bytes.start < line_end
                && local.line_origin == variants.line_origin
                && let Some(line) = local.line.as_ref().or(variants.line.as_ref())
            {
                chosen = line;
            }
            if letter.is_some_and(|letter| letter.contains(&span.bytes.start))
                && local.letter_origin == variants.letter_origin
                && let Some(letter) = local.letter.as_ref().or(variants.letter.as_ref())
            {
                chosen = letter;
            }
            (
                span.bytes.start,
                style_value(chosen, "text-transform", "none").to_owned(),
            )
        })
        .collect()
}

/// Resolve the first-line boundary in transformed text, then map that boundary
/// back through native source spans before applying the final per-range case
/// mapping. No transformed string is ever substituted for DOM text identity.
pub(super) fn transform_source(
    engine: &mut TextEngine,
    source: &text_source::TextSource,
    width: f64,
    font_size: f64,
    style: &BTreeMap<String, String>,
    viewport: ViewportState,
    styles: &ComputedStyles,
) -> (text_source::TextSource, BTreeMap<String, String>) {
    let mut output_style = style.clone();
    let variants = read(style);
    let normal = source.transform(styles, style, &engine.languages);
    if !variants.active {
        return (normal, output_style);
    }
    let changes = |value: Option<&BTreeMap<String, String>>, base: &BTreeMap<String, String>| {
        value.is_some_and(|value| {
            style_value(value, "text-transform", "none")
                != style_value(base, "text-transform", "none")
        })
    };
    if !changes(variants.line.as_ref(), style)
        && !changes(variants.letter.as_ref(), style)
        && !source.spans.iter().any(|span| {
            styles.get(&span.owner).is_some_and(|style| {
                let values = read(style);
                changes(values.line.as_ref(), style) || changes(values.letter.as_ref(), style)
            })
        })
    {
        return (normal, output_style);
    }
    let letter = variants
        .letter
        .as_ref()
        .and_then(|_| letter_range(&normal.text))
        .map(|range| {
            normal.map_boundary_to(source, range.start, false)
                ..normal.map_boundary_to(source, range.end, true)
        });
    let mut cuts = BTreeSet::new();
    if let Some(letter) = &letter {
        cuts.extend([letter.start, letter.end]);
    }
    let split = source.split_ranges(&cuts);
    let full_transforms = transforms(
        &split,
        styles,
        style,
        &variants,
        if variants.line.is_some() {
            source.text.len()
        } else {
            0
        },
        letter.as_ref(),
    );
    let candidate = split.transform_resolved(&engine.languages, |span| {
        full_transforms
            .get(&span.bytes.start)
            .map(String::as_str)
            .unwrap_or("none")
    });
    let line_end = if variants.line.is_some() {
        let ranges = candidate
            .spans
            .iter()
            .filter(|span| !span.bytes.is_empty())
            .map(|span| {
                (
                    span.bytes.clone(),
                    styles
                        .get(&span.owner)
                        .map(|style| style.as_ref())
                        .unwrap_or(style),
                    engine.languages.get(&span.owner).copied().flatten(),
                    span.owner,
                )
            })
            .collect::<Vec<_>>();
        let mut probe_style = style.clone();
        if let Some(letter) = &letter {
            probe_style.insert(
                "-oryn-first-letter-start".into(),
                source
                    .map_boundary_to(&candidate, letter.start, false)
                    .to_string(),
            );
            probe_style.insert(
                "-oryn-first-letter-end".into(),
                source
                    .map_boundary_to(&candidate, letter.end, true)
                    .to_string(),
            );
        }
        let saved_context = engine.float_context.clone();
        let saved_boxes = std::mem::take(&mut engine.active_inline_boxes);
        engine.active_inline_boxes = candidate
            .spans
            .iter()
            .filter(|span| span.transformed && span.text_node.is_none() && span.bytes.is_empty())
            .filter_map(|span| {
                engine
                    .inline_sizes
                    .get(&span.owner)
                    .map(|(width, height, kind)| parley::InlineBox {
                        id: span.owner,
                        kind: *kind,
                        index: span.bytes.start,
                        width: *width as f32,
                        height: *height as f32,
                    })
            })
            .collect();
        let probe = engine.shape_ranged(
            &candidate.text,
            width,
            font_size,
            &probe_style,
            viewport,
            &ranges,
        );
        engine.float_context = saved_context;
        engine.active_inline_boxes = saved_boxes;
        candidate.map_boundary_to(
            source,
            probe.line_ranges.first().map_or(0, |range| range.end),
            true,
        )
    } else {
        0
    };
    cuts.insert(line_end);
    let split = source.split_ranges(&cuts);
    let selected = transforms(&split, styles, style, &variants, line_end, letter.as_ref());
    let result = split.transform_resolved(&engine.languages, |span| {
        selected
            .get(&span.bytes.start)
            .map(String::as_str)
            .unwrap_or("none")
    });
    if line_end > 0 {
        output_style.insert(
            "-oryn-first-line-end".into(),
            source.map_boundary_to(&result, line_end, true).to_string(),
        );
    }
    if let Some(letter) = letter {
        output_style.insert(
            "-oryn-first-letter-start".into(),
            source
                .map_boundary_to(&result, letter.start, false)
                .to_string(),
        );
        output_style.insert(
            "-oryn-first-letter-end".into(),
            source
                .map_boundary_to(&result, letter.end, true)
                .to_string(),
        );
    }
    (result, output_style)
}

pub(super) fn demand_bytes(demands: &BTreeMap<String, BTreeSet<u32>>) -> usize {
    demands.iter().fold(0usize, |bytes, (family, characters)| {
        bytes
            .saturating_add(family.len() + 128)
            .saturating_add(characters.len().saturating_mul(32))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_letter_keeps_flags_combining_clusters_and_punctuation() {
        for (text, expected) in [
            ("  ‘A’bc", "‘A’"),
            ("🇬🇧 UK flag", "🇬🇧"),
            ("A\u{301}bc", "A\u{301}"),
            ("\u{feff}Ab", "A"),
        ] {
            assert_eq!(&text[letter_range(text).unwrap()], expected);
        }
        assert!(letter_range("  \n").is_none());
    }
}
