//! Ordered paragraph source and provenance before shaping or whitespace collapse.

use crate::{LayoutContent, LayoutNode};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct TextSpan {
    /// Zero-byte atomic anchors interrupt whitespace collapse; inline edge anchors do not.
    pub atomic: bool,
    /// Element whose computed style applies to this text node.
    pub owner: u64,
    /// Native text identity; absent for old element-only snapshots.
    pub text_node: Option<u64>,
    /// Byte range in the paragraph string, suitable for Parley's ranged styles.
    pub bytes: Range<usize>,
    /// UTF-16 length in the source text node, as used by DOM Range.
    pub utf16_len: usize,
    pub utf16_start: usize,
    /// The byte range represents a replacement/collapse rather than identical text.
    pub transformed: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct TextSource {
    pub text: String,
    pub spans: Vec<TextSpan>,
    /// Geometry-only provenance for source marks erased by a transformation.
    /// These aliases must not participate in styling or text shaping.
    pub(super) geometry_aliases: Vec<TextSpan>,
}

/// Intrinsic formatting participants in authored order. Ordinary inline text
/// shares one source; independent boxes retain their formatting boundary.
pub(super) enum IntrinsicItem {
    Text(TextSource),
    Break,
    Box { node: u64, inline: bool },
}

/// One visually positioned shaping cluster, in paragraph coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TextCluster {
    pub bytes: Range<usize>,
    pub rect: oryn_common::v4::CssRect,
    pub line: usize,
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SourceCluster {
    pub owner: u64,
    pub text_node: u64,
    pub utf16: Range<usize>,
    pub rect: oryn_common::v4::CssRect,
    pub line: usize,
    pub rtl: bool,
}

impl TextSource {
    pub fn synthetic(text: &str) -> Self {
        let mut source = Self::default();
        source.append(0, None, text);
        source
    }

    /// CSS whitespace transformation before line-edge trimming and tab layout.
    /// Keep a source segment for discarded characters so DOM offsets remain
    /// meaningful even when several characters produce one visible space.
    pub fn normalize(
        &self,
        styles: &crate::ComputedStyles,
        default: &BTreeMap<String, String>,
    ) -> Self {
        let mut output = Self::default();
        let mut previous_cr = false;
        let mut collapsing_space = false;
        let mut after_preserved_break = false;
        for span in &self.spans {
            let style = styles
                .get(&span.owner)
                .map(|style| style.as_ref())
                .unwrap_or(default);
            let shorthand = crate::style_value(style, "white-space", "normal");
            let fallback = match shorthand {
                "pre" | "pre-wrap" => "preserve",
                "pre-line" => "preserve-breaks",
                "break-spaces" => "break-spaces",
                _ => "collapse",
            };
            let mode = crate::style_value(style, "white-space-collapse", fallback);
            let mut offset = span.utf16_start;
            let Some(text) = self.text.get(span.bytes.clone()) else {
                continue;
            };
            if text.is_empty() {
                output.push_piece(span, offset, 0, "", span.transformed);
                if span.atomic {
                    collapsing_space = false;
                    after_preserved_break = false;
                    previous_cr = false;
                }
            }
            for ch in text.chars() {
                let units = ch.len_utf16();
                let skip_lf = previous_cr && ch == '\n';
                previous_cr = ch == '\r';
                let normalized = if ch == '\r' { '\n' } else { ch };
                let whitespace = matches!(normalized, ' ' | '\t' | '\n');
                let mut rendered = normalized.to_string();
                if skip_lf {
                    rendered.clear();
                } else if mode == "collapse" || mode == "preserve-breaks" {
                    if normalized == '\n' && mode == "preserve-breaks" {
                        // Spaces adjacent to a preserved segment break disappear.
                        if collapsing_space && output.text.ends_with(' ') {
                            output.text.pop();
                            let end = output.text.len();
                            for previous in output.spans.iter_mut().rev() {
                                if previous.bytes.end <= end {
                                    break;
                                }
                                previous.bytes.start = previous.bytes.start.min(end);
                                previous.bytes.end = end;
                                previous.transformed = true;
                            }
                        }
                        collapsing_space = false;
                        after_preserved_break = true;
                    } else if whitespace {
                        rendered = if collapsing_space || after_preserved_break {
                            String::new()
                        } else {
                            " ".into()
                        };
                        collapsing_space = true;
                    } else {
                        collapsing_space = false;
                        after_preserved_break = false;
                    }
                } else {
                    if mode == "preserve-spaces" && matches!(normalized, '\t' | '\n') {
                        rendered = " ".into();
                    }
                    collapsing_space = false;
                    after_preserved_break = false;
                }
                let transformed = whitespace || skip_lf || rendered != ch.to_string();
                output.push_piece(span, offset, units, &rendered, transformed);
                offset += units;
            }
        }
        output
    }

    /// Apply rendered case conversion while preserving each original UTF-16
    /// range. A prior whitespace replacement may already cover different source
    /// and rendered lengths; do not derive its source offsets from rendered text.
    pub fn transform(
        &self,
        styles: &crate::ComputedStyles,
        default: &BTreeMap<String, String>,
        languages: &BTreeMap<u64, Option<fontique::Language>>,
    ) -> Self {
        self.transform_resolved(languages, |span| {
            styles
                .get(&span.owner)
                .map(|style| crate::style_value(style, "text-transform", "none"))
                .unwrap_or_else(|| crate::style_value(default, "text-transform", "none"))
        })
    }

    /// The resolver runs over already split source spans; it may select a native
    /// pseudo style without replacing the source's DOM owner or UTF-16 offsets.
    pub(super) fn transform_resolved<'a>(
        &self,
        languages: &BTreeMap<u64, Option<fontique::Language>>,
        resolve: impl Fn(&TextSpan) -> &'a str,
    ) -> Self {
        if !self.spans.iter().any(|span| {
            resolve(span).split_ascii_whitespace().any(|token| {
                matches!(
                    token,
                    "uppercase" | "lowercase" | "capitalize" | "full-width" | "full-size-kana"
                )
            })
        }) {
            return self.clone();
        }
        // Word context spans style boundaries, but a transformation must not
        // consume a mark whose own computed language/case mapping differs.
        let greek_eligible = self
            .spans
            .iter()
            .flat_map(|span| {
                let uppercase = resolve(span)
                    .split_ascii_whitespace()
                    .any(|token| token == "uppercase");
                let greek = languages
                    .get(&span.owner)
                    .and_then(|value| value.as_ref())
                    .is_some_and(|language| language.language() == "el");
                std::iter::repeat_n(
                    uppercase && greek,
                    self.text[span.bytes.clone()].chars().count(),
                )
            })
            .collect::<Vec<_>>();
        let context =
            crate::text_transform::CaseContext::with_greek_eligibility(&self.text, &greek_eligible);
        let mut output = Self::default();
        let mut index = 0;
        let mut character_outputs: Vec<Range<usize>> = Vec::new();
        let mut erased_marks: Vec<(usize, TextSpan)> = Vec::new();
        for span in &self.spans {
            let transform = resolve(span);
            let language = languages
                .get(&span.owner)
                .and_then(|language| language.as_ref());
            let language = language.map(|language| language.language()).unwrap_or("");
            let text = &self.text[span.bytes.clone()];
            if span.transformed {
                let mut rendered = String::new();
                for ch in text.chars() {
                    let piece = context.convert(index, transform, language);
                    let begin = output.text.len() + rendered.len();
                    rendered.push_str(&piece);
                    character_outputs.push(begin..output.text.len() + rendered.len());
                    if piece.is_empty() && unicode_normalization::char::is_combining_mark(ch) {
                        erased_marks.push((index, span.clone()));
                    }
                    index += 1;
                }
                output.push_piece(span, span.utf16_start, span.utf16_len, &rendered, true);
            } else {
                let mut offset = span.utf16_start;
                if text.is_empty() {
                    output.push_piece(span, offset, span.utf16_len, "", false);
                }
                for ch in text.chars() {
                    let rendered = context.convert(index, transform, language);
                    let begin = output.text.len();
                    output.push_piece(
                        span,
                        offset,
                        ch.len_utf16(),
                        &rendered,
                        rendered != ch.to_string(),
                    );
                    character_outputs.push(begin..output.text.len());
                    if rendered.is_empty() && unicode_normalization::char::is_combining_mark(ch) {
                        erased_marks.push((
                            index,
                            TextSpan {
                                atomic: false,
                                utf16_start: offset,
                                utf16_len: ch.len_utf16(),
                                transformed: true,
                                ..span.clone()
                            },
                        ));
                    }
                    offset += ch.len_utf16();
                    index += 1;
                }
            }
        }
        // An erased combining mark still belongs to its original typographic
        // unit. Point its geometry at that unit's complete rendered output,
        // without inserting bytes or overriding the base character's style.
        if erased_marks.is_empty() {
            return output;
        }
        let mut erased = erased_marks.into_iter().peekable();
        let mut first = 0;
        for grapheme in self.text.graphemes(true) {
            let end = first + grapheme.chars().count();
            let rendered = character_outputs[first].start..character_outputs[end - 1].end;
            while erased.peek().is_some_and(|(index, _)| *index < end) {
                let (_, mut alias) = erased.next().unwrap();
                if !rendered.is_empty() {
                    alias.bytes = rendered.clone();
                    alias.transformed = true;
                    output.geometry_aliases.push(alias);
                }
            }
            first = end;
        }
        output
    }

    /// Split styling ranges without changing text or source identity. A previous
    /// expansion/collapse is indivisible so one source unit keeps one mapping.
    pub(super) fn split_ranges(&self, boundaries: &BTreeSet<usize>) -> Self {
        let mut output = Self {
            text: self.text.clone(),
            spans: Vec::new(),
            geometry_aliases: self.geometry_aliases.clone(),
        };
        for span in &self.spans {
            if span.transformed || span.bytes.is_empty() {
                output.spans.push(span.clone());
                continue;
            }
            let mut cuts = vec![span.bytes.start];
            cuts.extend(
                boundaries
                    .range((
                        std::ops::Bound::Excluded(span.bytes.start),
                        std::ops::Bound::Excluded(span.bytes.end),
                    ))
                    .copied()
                    .filter(|byte| self.text.is_char_boundary(*byte)),
            );
            cuts.push(span.bytes.end);
            for pair in cuts.windows(2) {
                let mut piece = span.clone();
                piece.bytes = pair[0]..pair[1];
                piece.utf16_start += self.text[span.bytes.start..pair[0]].encode_utf16().count();
                piece.utf16_len = self.text[piece.bytes.clone()].encode_utf16().count();
                output.spans.push(piece);
            }
        }
        output
    }

    /// Map a rendered boundary between two projections of the same native text.
    /// Expansion and erased-mark boundaries round to their complete source unit.
    pub(super) fn map_boundary_to(&self, other: &Self, byte: usize, up: bool) -> usize {
        if byte == 0 {
            return 0;
        }
        if byte >= self.text.len() {
            return other.text.len();
        }
        let span = self
            .spans
            .iter()
            .find(|span| span.bytes.start < byte && span.bytes.end >= byte)
            .or_else(|| self.spans.iter().find(|span| span.bytes.start == byte));
        let Some(span) = span else {
            return byte.min(other.text.len());
        };
        let offset = if span.transformed {
            if byte == span.bytes.start || (!up && byte < span.bytes.end) {
                span.utf16_start
            } else {
                span.utf16_start + span.utf16_len
            }
        } else {
            span.utf16_start
                + self.text[span.bytes.start..byte.min(span.bytes.end)]
                    .encode_utf16()
                    .count()
        };
        let candidates = other.spans.iter().filter(|candidate| {
            candidate.owner == span.owner
                && candidate.text_node == span.text_node
                && offset >= candidate.utf16_start
                && offset <= candidate.utf16_start + candidate.utf16_len
        });
        let mut result = None;
        for candidate in candidates {
            let units = offset - candidate.utf16_start;
            let projected = if candidate.transformed {
                if units == 0 {
                    candidate.bytes.start
                } else if units == candidate.utf16_len || up {
                    candidate.bytes.end
                } else {
                    candidate.bytes.start
                }
            } else {
                let mut consumed = 0;
                let mut projected = candidate.bytes.end;
                for (index, ch) in other.text[candidate.bytes.clone()].char_indices() {
                    if units <= consumed {
                        projected = candidate.bytes.start + index;
                        break;
                    }
                    if units < consumed + ch.len_utf16() {
                        projected =
                            candidate.bytes.start + index + if up { ch.len_utf8() } else { 0 };
                        break;
                    }
                    consumed += ch.len_utf16();
                }
                projected
            };
            result = Some(
                result
                    .map(|old: usize| {
                        if up {
                            old.max(projected)
                        } else {
                            old.min(projected)
                        }
                    })
                    .unwrap_or(projected),
            );
        }
        result.unwrap_or_else(|| byte.min(other.text.len()))
    }

    fn push_piece(
        &mut self,
        span: &TextSpan,
        start: usize,
        len: usize,
        text: &str,
        transformed: bool,
    ) {
        let byte = self.text.len();
        self.text.push_str(text);
        if !transformed
            && let Some(previous) = self.spans.last_mut()
            && !previous.transformed
            && previous.owner == span.owner
            && previous.text_node == span.text_node
            && previous.utf16_start + previous.utf16_len == start
            && previous.bytes.end == byte
        {
            previous.bytes.end = self.text.len();
            previous.utf16_len += len;
            return;
        }
        self.spans.push(TextSpan {
            atomic: span.atomic,
            owner: span.owner,
            text_node: span.text_node,
            bytes: byte..self.text.len(),
            utf16_start: start,
            utf16_len: len,
            transformed,
        });
    }

    /// Direct text only, matching the current block formatter's text input.
    /// Element descendants remain separate layout participants until paragraph
    /// placement combines them; their text must not be counted here twice.
    pub fn direct(node: &LayoutNode) -> Self {
        let mut source = Self::default();
        if let Some(content) = &node.content {
            for item in content {
                match item {
                    LayoutContent::Text { node_id, text } => {
                        source.append(node.id, Some(*node_id), text)
                    }
                    LayoutContent::StyledText {
                        owner,
                        node_id,
                        text,
                    } => source.append(*owner, *node_id, text),
                    _ => {}
                }
            }
        } else {
            source.append(node.id, None, &node.text);
        }
        source
    }

    pub fn direct_with_breaks(
        node: &LayoutNode,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &crate::ComputedStyles,
    ) -> Self {
        let Some(content) = &node.content else {
            return Self::direct(node);
        };
        let mut source = Self::default();
        for item in content {
            match item {
                LayoutContent::Text { node_id, text } => {
                    source.append(node.id, Some(*node_id), text)
                }
                LayoutContent::StyledText {
                    owner,
                    node_id,
                    text,
                } => source.append(*owner, *node_id, text),
                LayoutContent::Element(id)
                    if nodes.get(id).is_some_and(|node| node.tag == "br")
                        && styles.get(id).is_none_or(|s| {
                            crate::style_value(s, "display", "inline") != "none"
                        })
                        && styles.get(&node.id).is_none_or(|s| {
                            !matches!(
                                crate::style_value(s, "display", "block"),
                                "flex" | "inline-flex" | "grid" | "inline-grid"
                            )
                        }) =>
                {
                    source.append(*id, None, "\u{2028}")
                }
                _ => {}
            }
        }
        source
    }

    pub(super) fn append(&mut self, owner: u64, text_node: Option<u64>, text: &str) {
        let start = self.text.len();
        self.text.push_str(text);
        self.spans.push(TextSpan {
            atomic: false,
            utf16_start: 0,
            transformed: false,
            owner,
            text_node,
            bytes: start..self.text.len(),
            utf16_len: text.encode_utf16().count(),
        });
    }

    pub(super) fn append_box(&mut self, owner: u64) {
        let byte = self.text.len();
        self.spans.push(TextSpan {
            atomic: true,
            owner,
            text_node: None,
            bytes: byte..byte,
            utf16_len: 0,
            utf16_start: 0,
            transformed: true,
        });
    }

    pub(super) fn append_boundary(&mut self, owner: u64) {
        self.append_box(owner);
        if let Some(span) = self.spans.last_mut() {
            span.atomic = false;
        }
    }

    #[cfg(test)]
    pub fn collect(
        root: &LayoutNode,
        nodes: &BTreeMap<u64, &LayoutNode>,
        include_legacy_root: bool,
    ) -> Self {
        Self::collect_visible(root, nodes, include_legacy_root, None)
    }

    pub fn collect_visible(
        root: &LayoutNode,
        nodes: &BTreeMap<u64, &LayoutNode>,
        include_legacy_root: bool,
        styles: Option<&crate::ComputedStyles>,
    ) -> Self {
        enum Item<'a> {
            Text {
                owner: u64,
                id: Option<u64>,
                text: &'a str,
            },
            Element(u64),
        }
        fn enqueue<'a>(node: &'a LayoutNode, stack: &mut Vec<Item<'a>>, include_legacy: bool) {
            if let Some(content) = &node.content {
                stack.extend(content.iter().rev().map(|item| match item {
                    LayoutContent::Text { node_id, text } => Item::Text {
                        owner: node.id,
                        id: Some(*node_id),
                        text,
                    },
                    LayoutContent::StyledText {
                        owner,
                        node_id,
                        text,
                    } => Item::Text {
                        owner: *owner,
                        id: *node_id,
                        text,
                    },
                    LayoutContent::Element(id) => Item::Element(*id),
                }));
            } else {
                stack.extend(node.children.iter().rev().map(|id| Item::Element(*id)));
                if include_legacy {
                    stack.push(Item::Text {
                        owner: node.id,
                        id: None,
                        text: &node.text,
                    });
                }
            }
        }
        let mut result = Self::default();
        let mut stack = Vec::new();
        enqueue(root, &mut stack, include_legacy_root);
        let mut seen = BTreeSet::from([root.id]);
        while let Some(item) = stack.pop() {
            match item {
                Item::Text { owner, id, text } => {
                    // Empty text nodes retain a caret anchor even without glyphs.
                    result.append(owner, id, text);
                }
                Item::Element(id) => {
                    if seen.insert(id)
                        && !styles
                            .and_then(|styles| styles.get(&id))
                            .is_some_and(|s| crate::style_value(s, "display", "inline") == "none")
                        && let Some(node) = nodes.get(&id)
                    {
                        if node.tag == "br" {
                            result.append(id, None, "\u{2028}");
                        } else {
                            enqueue(node, &mut stack, true);
                        }
                    }
                }
            }
        }
        result
    }

    pub fn intrinsic_items(
        root: &LayoutNode,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &crate::ComputedStyles,
    ) -> Vec<IntrinsicItem> {
        enum Item<'a> {
            Text(u64, Option<u64>, &'a str),
            Element(u64),
        }
        fn enqueue<'a>(node: &'a LayoutNode, stack: &mut Vec<Item<'a>>) {
            if let Some(content) = &node.content {
                stack.extend(content.iter().rev().map(|item| match item {
                    LayoutContent::Text { node_id, text } => {
                        Item::Text(node.id, Some(*node_id), text)
                    }
                    LayoutContent::StyledText {
                        owner,
                        node_id,
                        text,
                    } => Item::Text(*owner, *node_id, text),
                    LayoutContent::Element(id) => Item::Element(*id),
                }));
            } else {
                stack.extend(node.children.iter().rev().map(|id| Item::Element(*id)));
                stack.push(Item::Text(node.id, None, &node.text));
            }
        }
        let mut items = Vec::new();
        let mut source = Self::default();
        let mut stack = Vec::new();
        let mut seen = BTreeSet::from([root.id]);
        enqueue(root, &mut stack);
        while let Some(item) = stack.pop() {
            match item {
                Item::Text(owner, id, text) => source.append(owner, id, text),
                Item::Element(id) => {
                    if !seen.insert(id) {
                        continue;
                    }
                    let (Some(node), Some(style)) = (nodes.get(&id), styles.get(&id)) else {
                        continue;
                    };
                    let display = crate::style_value(style, "display", "block");
                    if display == "none"
                        || matches!(
                            crate::style_value(style, "position", "static"),
                            "absolute" | "fixed"
                        )
                    {
                        continue;
                    }
                    if node.tag == "br" {
                        if !source.text.is_empty() {
                            items.push(IntrinsicItem::Text(std::mem::take(&mut source)));
                        }
                        items.push(IntrinsicItem::Break);
                    } else if display == "contents"
                        || (display == "inline"
                            && node.intrinsic_image_size.is_none()
                            && !node.document_replaced
                            && !matches!(
                                node.tag.as_str(),
                                "input" | "button" | "select" | "textarea"
                            ))
                    {
                        enqueue(node, &mut stack);
                    } else {
                        if !source.text.is_empty() {
                            items.push(IntrinsicItem::Text(std::mem::take(&mut source)));
                        }
                        items.push(IntrinsicItem::Box {
                            node: id,
                            inline: display.starts_with("inline"),
                        });
                    }
                }
            }
        }
        if !source.text.is_empty() {
            items.push(IntrinsicItem::Text(source));
        }
        items
    }

    /// Project paragraph cluster ranges back onto their native text nodes.
    /// A ligature/combining cluster may cross a style or text-node boundary;
    /// every participating source range retains that cluster's measured box.
    pub fn map_clusters(&self, clusters: &[TextCluster]) -> Vec<SourceCluster> {
        let mut result: Vec<SourceCluster> = Vec::new();
        let mut utf16 = 0;
        let mut boundaries = self
            .text
            .char_indices()
            .map(|(byte, ch)| {
                let boundary = (byte, utf16);
                utf16 += ch.len_utf16();
                boundary
            })
            .collect::<Vec<_>>();
        boundaries.push((self.text.len(), utf16));
        let units_at = |byte| {
            boundaries
                .binary_search_by_key(&byte, |(byte, _)| *byte)
                .ok()
                .map(|index| boundaries[index].1)
        };
        let mut spans = self
            .spans
            .iter()
            .chain(&self.geometry_aliases)
            .collect::<Vec<_>>();
        spans.sort_by_key(|span| span.bytes.start);
        // Aliases may enclose multiple ordinary spans. Prefix maxima keep the
        // overlap lookup valid without assuming monotonically increasing ends.
        let mut maximum = 0;
        let maximum_ends = spans
            .iter()
            .map(|span| {
                maximum = maximum.max(span.bytes.end);
                maximum
            })
            .collect::<Vec<_>>();
        for cluster in clusters {
            let start = maximum_ends.partition_point(|end| *end <= cluster.bytes.start);
            for span in spans[start..]
                .iter()
                .take_while(|span| span.bytes.start < cluster.bytes.end)
            {
                let Some(text_node) = span.text_node else {
                    continue;
                };
                let first = span.bytes.start.max(cluster.bytes.start);
                let last = span.bytes.end.min(cluster.bytes.end);
                if first >= last {
                    continue;
                }
                let (Some(base), Some(first), Some(last)) =
                    (units_at(span.bytes.start), units_at(first), units_at(last))
                else {
                    continue;
                };
                let utf16 = if span.transformed {
                    span.utf16_start..span.utf16_start + span.utf16_len
                } else {
                    span.utf16_start + first - base..span.utf16_start + last - base
                };
                if let Some(previous) = result.last_mut()
                    && previous.owner == span.owner
                    && previous.text_node == text_node
                    && previous.rect == cluster.rect
                    && previous.line == cluster.line
                    && previous.rtl == cluster.rtl
                    && previous.utf16.end == utf16.start
                {
                    previous.utf16.end = utf16.end;
                    continue;
                }
                result.push(SourceCluster {
                    owner: span.owner,
                    text_node,
                    utf16,
                    rect: cluster.rect,
                    line: cluster.line,
                    rtl: cluster.rtl,
                });
            }
        }
        result
    }

    /// Map a DOM boundary into paragraph bytes. UTF-16 boundaries within a
    /// surrogate pair enclose the scalar according to the requested affinity;
    /// callers can retain the original UTF-16 offset for caret interpolation.
    #[cfg(test)]
    pub fn byte_boundary(
        &self,
        text_node: u64,
        utf16_offset: usize,
        round_up: bool,
    ) -> Option<usize> {
        let span = self.spans.iter().find(|span| {
            span.text_node == Some(text_node)
                && utf16_offset >= span.utf16_start
                && utf16_offset <= span.utf16_start + span.utf16_len
        })?;
        let utf16_offset = utf16_offset - span.utf16_start;
        if span.transformed {
            return Some(if utf16_offset == 0 {
                span.bytes.start
            } else if utf16_offset == span.utf16_len || round_up {
                span.bytes.end
            } else {
                span.bytes.start
            });
        }
        let text = self.text.get(span.bytes.clone())?;
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            if utf16_offset == units {
                return Some(span.bytes.start + byte);
            }
            let next = units + ch.len_utf16();
            if utf16_offset < next {
                return Some(span.bytes.start + byte + if round_up { ch.len_utf8() } else { 0 });
            }
            units = next;
        }
        Some(span.bytes.end)
    }
}

impl TextSource {
    /// Export the exact shaping source with native zero-advance collapsible
    /// line-edge spaces removed. Every erased source interval remains an alias.
    pub(super) fn resolved_run(
        &self,
        layout_node: u64,
        clusters: &[TextCluster],
        styles: &crate::ComputedStyles,
        default: &BTreeMap<String, String>,
    ) -> oryn_common::v4::ResolvedTextRun {
        use oryn_common::v4::{ResolvedTextRun, ResolvedTextSpan};
        let mut removed = Vec::<Range<usize>>::new();
        for cluster in clusters {
            if cluster.rect.width != 0. || cluster.bytes.is_empty() {
                continue;
            }
            let Some(text) = self.text.get(cluster.bytes.clone()) else {
                continue;
            };
            if !text.chars().all(|ch| matches!(ch, ' ' | '\t')) {
                continue;
            }
            let spans = self.spans.iter().filter(|span| {
                span.bytes.start < cluster.bytes.end && span.bytes.end > cluster.bytes.start
            });
            if spans.clone().next().is_none() {
                continue;
            }
            if spans.into_iter().all(|span| {
                let style = styles
                    .get(&span.owner)
                    .map(|style| style.as_ref())
                    .unwrap_or(default);
                let fallback = match crate::style_value(style, "white-space", "normal") {
                    "pre" | "pre-wrap" | "break-spaces" => "preserve",
                    "pre-line" => "preserve-breaks",
                    _ => "collapse",
                };
                matches!(
                    crate::style_value(style, "white-space-collapse", fallback),
                    "collapse" | "preserve-breaks"
                )
            }) {
                removed.push(cluster.bytes.clone());
            }
        }
        removed.sort_by_key(|range| range.start);
        let mut cuts = Vec::<Range<usize>>::new();
        for range in removed {
            if let Some(previous) = cuts
                .last_mut()
                .filter(|previous| previous.end >= range.start)
            {
                previous.end = previous.end.max(range.end);
            } else {
                cuts.push(range);
            }
        }
        let mut prefix = vec![0usize];
        for range in &cuts {
            prefix.push(prefix.last().copied().unwrap() + range.len());
        }
        let project = |byte: usize| {
            let count = cuts.partition_point(|range| range.end <= byte);
            let partial = cuts
                .get(count)
                .map_or(0, |range| byte.saturating_sub(range.start));
            byte.saturating_sub(prefix[count] + partial)
        };
        let mut text =
            String::with_capacity(self.text.len().saturating_sub(*prefix.last().unwrap()));
        let mut cursor = 0;
        for range in &cuts {
            text.push_str(&self.text[cursor..range.start]);
            cursor = range.end;
        }
        text.push_str(&self.text[cursor..]);
        let convert = |span: &TextSpan, bytes: Range<usize>, erased: bool| {
            let (start, len) = if span.transformed {
                (span.utf16_start, span.utf16_len)
            } else {
                (
                    span.utf16_start
                        + self.text[span.bytes.start..bytes.start]
                            .encode_utf16()
                            .count(),
                    self.text[bytes.clone()].encode_utf16().count(),
                )
            };
            ResolvedTextSpan {
                owner: span.owner,
                source_node: span.text_node,
                rendered_start: project(bytes.start),
                rendered_end: project(bytes.end),
                source_start: start,
                source_end: start.saturating_add(len),
                transformed: span.transformed || erased,
                atomic: span.atomic,
            }
        };
        // DOM rendered text ignores soft wrapping, but not actual paragraph
        // edges or authored hard breaks. Use only native zero-advance CSS cuts
        // at those boundaries; never trim preserved spaces or interior wraps.
        let semantic_cuts: Vec<_> = cuts.iter().filter(|range| {
            let before = self.text[..range.start].char_indices().rev().find(|(_, ch)| !matches!(ch, ' ' | '\t'));
            let after = self.text[range.end..].char_indices().find(|(_, ch)| !matches!(ch, ' ' | '\t'));
            // Replaced inline boxes occupy the line even though they have no
            // text bytes. A space beside one is not a paragraph-edge space.
            let left = before.map_or(0, |(byte, ch)| byte + ch.len_utf8());
            let right = after.map_or(self.text.len(), |(byte, _)| range.end + byte);
            let left_box = self.spans.iter().any(|span| span.atomic
                && span.bytes.start >= left && span.bytes.start <= range.start);
            let right_box = self.spans.iter().any(|span| span.atomic
                && span.bytes.start >= range.end && span.bytes.start <= right);
            (before.is_none_or(|(_, ch)| matches!(ch, '\n' | '\u{2028}' | '\u{2029}')) && !left_box)
                || (after.is_none_or(|(_, ch)| matches!(ch, '\n' | '\u{2028}' | '\u{2029}')) && !right_box)
        }).cloned().collect();
        let mut semantic_prefix = vec![0usize];
        let mut semantic_text = String::new();
        let mut cursor = 0;
        for range in &semantic_cuts {
            semantic_text.push_str(&self.text[cursor..range.start]);
            cursor = range.end;
            semantic_prefix.push(semantic_prefix.last().copied().unwrap() + range.len());
        }
        semantic_text.push_str(&self.text[cursor..]);
        let semantic_project = |byte: usize| {
            let count = semantic_cuts.partition_point(|range| range.end <= byte);
            let partial = semantic_cuts.get(count).map_or(0, |range| byte.saturating_sub(range.start));
            byte.saturating_sub(semantic_prefix[count] + partial)
        };
        let mut output = ResolvedTextRun {
            layout_node,
            text,
            spans: Vec::new(),
            aliases: Vec::new(),
            semantic_source: Some(oryn_common::v4::SemanticTextSource {
                text: semantic_text,
                spans: self.spans.iter().map(|span| ResolvedTextSpan {
                    owner: span.owner,
                    source_node: span.text_node,
                    rendered_start: semantic_project(span.bytes.start),
                    rendered_end: semantic_project(span.bytes.end),
                    source_start: span.utf16_start,
                    source_end: span.utf16_start.saturating_add(span.utf16_len),
                    transformed: span.transformed || semantic_cuts.get(semantic_cuts.partition_point(|range| range.end <= span.bytes.start)).is_some_and(|range| range.start < span.bytes.end),
                    atomic: span.atomic,
                }).collect(),
            }),
        };
        for span in &self.spans {
            let mut boundaries = vec![span.bytes.start, span.bytes.end];
            for range in &cuts[cuts.partition_point(|range| range.end <= span.bytes.start)..] {
                if range.start >= span.bytes.end {
                    break;
                }
                boundaries.push(range.start.max(span.bytes.start));
                boundaries.push(range.end.min(span.bytes.end));
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            if boundaries.len() == 1 {
                // A normalized-away source character still has a DOM range,
                // unlike a zero-source atomic anchor. Keep erased source in
                // aliases so visible spans describe only rendered content.
                let erased = span.utf16_len != 0;
                let resolved = convert(span, span.bytes.clone(), erased);
                if erased {
                    output.aliases.push(resolved);
                } else {
                    output.spans.push(resolved);
                }
            }
            for pair in boundaries.windows(2) {
                let bytes = pair[0]..pair[1];
                let erased = project(bytes.start) == project(bytes.end);
                let resolved = convert(span, bytes, erased);
                if erased {
                    output.aliases.push(resolved);
                } else {
                    output.spans.push(resolved);
                }
            }
        }
        for alias in &self.geometry_aliases {
            output.aliases.push(ResolvedTextSpan {
                owner: alias.owner,
                source_node: alias.text_node,
                rendered_start: project(alias.bytes.start),
                rendered_end: project(alias.bytes.end),
                source_start: alias.utf16_start,
                source_end: alias.utf16_start.saturating_add(alias.utf16_len),
                transformed: true,
                atomic: alias.atomic,
            });
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_source_trims_hard_break_edges_but_not_preserved_spaces() {
        let mut source = TextSource::default();
        source.append(1, Some(300), "a ");
        source.append(2, None, "\u{2028}");
        source.append(1, Some(301), " b");
        let clusters: Vec<_> = [1..2, 5..6].into_iter().map(|bytes| TextCluster {
            bytes, rect:oryn_common::v4::CssRect { x:0., y:0., width:0., height:1. }, line:0, rtl:false,
        }).collect();
        for (mode, expected) in [("normal", "a\u{2028}b"), ("pre", "a \u{2028} b")] {
            let style = BTreeMap::from([("white-space".into(), mode.into())]);
            let normalized = source.normalize(&BTreeMap::new(), &style);
            assert_eq!(normalized.resolved_run(1, &clusters, &BTreeMap::new(), &style).semantic_source.unwrap().text, expected);
        }
    }
    #[test]
    fn semantic_paragraph_edges_include_atomic_boxes_without_emitting_box_text() {
        for leading in [true, false] {
            let mut source = TextSource::default();
            if leading { source.append_box(2); }
            source.append(1, Some(300), if leading { " abc" } else { "abc " });
            if !leading { source.append_box(2); }
            let source = source.normalize(&BTreeMap::new(), &BTreeMap::new());
            let run = source.resolved_run(1, &[TextCluster {
                bytes: if leading { 0..1 } else { 3..4 },
                rect: oryn_common::v4::CssRect { x: 0., y: 0., width: 0., height: 1. },
                line: 0, rtl: false,
            }], &BTreeMap::new(), &BTreeMap::new());
            assert_eq!(run.text, "abc");
            let semantic = run.semantic_source.unwrap();
            assert_eq!(semantic.text, if leading { " abc" } else { "abc " });
            assert!(semantic.spans.iter().any(|span| span.atomic && span.source_node.is_none()));
        }
    }
    #[test]
    fn semantic_source_trims_native_paragraph_edges_but_keeps_soft_wrap_space() {
        let mut source = TextSource::default();
        source.append(1, Some(300), " a b ");
        let source = source.normalize(&BTreeMap::new(), &BTreeMap::new());
        let clusters: Vec<_> = [0..1, 2..3, 4..5].into_iter().map(|bytes| TextCluster {
            bytes, rect:oryn_common::v4::CssRect { x:0., y:0., width:0., height:1. }, line:0, rtl:false,
        }).collect();
        let run = source.resolved_run(1, &clusters, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(run.text, "ab");
        let semantic = run.semantic_source.unwrap();
        assert_eq!(semantic.text, "a b");
        assert_eq!(semantic.spans.first().unwrap().rendered_start, 0);
        assert_eq!(semantic.spans.first().unwrap().rendered_end, 0);
        assert_eq!(semantic.spans.last().unwrap().source_end, 5);
    }
    #[test]
    fn semantic_source_keeps_soft_line_space_separate_from_paint_projection() {
        let mut source = TextSource::default();
        source.append(1, Some(300), "a  b");
        let source = source.normalize(&BTreeMap::new(), &BTreeMap::new());
        let run = source.resolved_run(1, &[TextCluster {
            bytes: 1..2,
            rect: oryn_common::v4::CssRect { x: 0., y: 0., width: 0., height: 1. },
            line: 0,
            rtl: false,
        }], &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(run.text, "ab");
        let semantic = run.semantic_source.unwrap();
        assert_eq!(semantic.text, "a b");
        assert!(semantic.spans.iter().any(|span| span.source_node == Some(300)
            && span.rendered_start == 1 && span.rendered_end == 2));
        assert!(run.aliases.iter().any(|span| span.rendered_start == span.rendered_end));
    }
    #[test]
    fn erased_source_spaces_are_aliases_but_zero_source_atomic_anchors_remain_spans() {
        let mut source = TextSource::default();
        source.append(1, Some(300), "a  b");
        let normalized = source.normalize(&BTreeMap::new(), &BTreeMap::new());
        let run = normalized.resolved_run(1, &[], &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(run.text, "a b");
        assert!(run.aliases.iter().any(|span| span.source_node == Some(300)
            && span.source_start == 2
            && span.source_end == 3
            && span.rendered_start == 2
            && span.rendered_end == 2));
        assert!(!run.spans.iter().any(|span| span.source_node == Some(300)
            && span.source_start == 2
            && span.source_end == 3));
        let anchor = TextSource {
            text: String::new(),
            spans: vec![TextSpan {
                atomic: true,
                owner: 2,
                text_node: None,
                bytes: 0..0,
                utf16_start: 0,
                utf16_len: 0,
                transformed: false,
            }],
            geometry_aliases: Vec::new(),
        };
        let run = anchor.resolved_run(2, &[], &BTreeMap::new(), &BTreeMap::new());
        assert!(run.aliases.is_empty());
        assert_eq!(run.spans.len(), 1);
        assert!(run.spans[0].atomic);
    }

    #[test]
    fn pseudo_range_transform_preserves_expansions_and_adjacent_text_node_offsets() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "ßa");
        source.append(1, Some(11), "βc");
        let split = source.split_ranges(&BTreeSet::from([2, 3, 5]));
        let transformed = split.transform_resolved(&BTreeMap::new(), |span| {
            if span.bytes.start == 0 {
                "uppercase"
            } else {
                "none"
            }
        });
        assert_eq!(transformed.text, "SSaβc");
        assert_eq!(transformed.map_boundary_to(&source, 1, false), 0);
        assert_eq!(transformed.map_boundary_to(&source, 1, true), 2);
        assert_eq!(source.map_boundary_to(&transformed, 2, true), 2);
        let second = transformed
            .spans
            .iter()
            .find(|span| span.text_node == Some(11))
            .unwrap();
        assert_eq!(second.utf16_start, 0);
        assert_eq!(second.bytes.start, 3);
        assert_eq!(source.map_boundary_to(&transformed, 5, true), 5);
    }

    #[test]
    fn greek_tailoring_does_not_consume_marks_with_a_different_computed_transform() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "α");
        source.append(2, Some(11), "\u{301}");
        source.append(1, Some(12), "ι");
        let style = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let languages = BTreeMap::from([
            (1, fontique::Language::parse("el").ok()),
            (2, fontique::Language::parse("el").ok()),
        ]);
        let styles = BTreeMap::from([(
            2,
            std::sync::Arc::new(BTreeMap::from([("text-transform".into(), "none".into())])),
        )]);
        let transformed = source.transform(&styles, &style, &languages);
        assert_eq!(transformed.text, "Α\u{301}Ι");
        assert!(transformed.geometry_aliases.is_empty());
        let all_greek = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(all_greek.text, "ΑΙ\u{308}");
        assert_eq!(all_greek.geometry_aliases.len(), 1);
        let mut mixed_languages = languages.clone();
        mixed_languages.insert(2, fontique::Language::parse("en").ok());
        let mixed = source.transform(&BTreeMap::new(), &style, &mixed_languages);
        assert_eq!(mixed.text, "Α\u{301}Ι");
        assert!(mixed.geometry_aliases.is_empty());
    }

    #[test]
    fn greek_eta_word_context_includes_untransformed_neighboring_text() {
        let mut source = TextSource::default();
        source.append(2, Some(10), "a");
        source.append(1, Some(11), "ή ");
        source.append(1, Some(12), "ή");
        source.append(2, Some(13), "b");
        let uppercase = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let styles = BTreeMap::from([(
            2,
            std::sync::Arc::new(BTreeMap::from([("text-transform".into(), "none".into())])),
        )]);
        let languages = BTreeMap::from([(1, fontique::Language::parse("el").ok())]);
        let transformed = source.transform(&styles, &uppercase, &languages);
        assert_eq!(transformed.text, "aΗ Ηb");
    }

    #[test]
    fn expanded_case_and_surrogate_keep_independent_native_owners() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "ß");
        source.append(2, Some(11), "😀");
        let style = BTreeMap::from([("text-transform".into(), "uppercase full-width".into())]);
        let rendered = source.normalize(&BTreeMap::new(), &style).transform(
            &BTreeMap::new(),
            &style,
            &BTreeMap::new(),
        );
        assert_eq!(rendered.text, "ＳＳ😀");
        assert_eq!(source.text, "ß😀");
        assert_eq!(rendered.byte_boundary(10, 0, false), Some(0));
        assert_eq!(rendered.byte_boundary(10, 1, true), Some(6));
        assert_eq!(rendered.byte_boundary(11, 0, false), Some(6));
        assert_eq!(rendered.byte_boundary(11, 1, false), Some(6));
        assert_eq!(rendered.byte_boundary(11, 1, true), Some(10));
        assert_eq!(rendered.byte_boundary(11, 2, true), Some(10));
        let first = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 8.,
            height: 16.,
        };
        let second = oryn_common::v4::CssRect { x: 8., ..first };
        let emoji = oryn_common::v4::CssRect {
            x: 16.,
            width: 16.,
            ..first
        };
        let mapped = rendered.map_clusters(&[
            TextCluster {
                bytes: 0..3,
                rect: first,
                line: 0,
                rtl: false,
            },
            TextCluster {
                bytes: 3..6,
                rect: second,
                line: 0,
                rtl: false,
            },
            TextCluster {
                bytes: 6..10,
                rect: emoji,
                line: 0,
                rtl: false,
            },
        ]);
        assert_eq!(
            mapped
                .iter()
                .map(|span| (span.text_node, span.utf16.clone(), span.rect))
                .collect::<Vec<_>>(),
            vec![(10, 0..1, first), (10, 0..1, second), (11, 0..2, emoji)]
        );
    }

    #[test]
    fn capitalization_context_crosses_an_untransformed_native_owner() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "foo");
        source.append(2, Some(11), "bar baz");
        let styles = BTreeMap::from([
            (
                1,
                std::sync::Arc::new(BTreeMap::from([("text-transform".into(), "none".into())])),
            ),
            (
                2,
                std::sync::Arc::new(BTreeMap::from([(
                    "text-transform".into(),
                    "capitalize".into(),
                )])),
            ),
        ]);
        let rendered = source.transform(&styles, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(rendered.text, "foobar Baz");
        assert_eq!(source.text, "foobar baz");
        assert_eq!(rendered.byte_boundary(11, 0, false), Some(3));
        assert_eq!(rendered.byte_boundary(11, 7, true), Some(10));
    }

    #[test]
    fn titlecase_erased_mark_keeps_geometry_when_grapheme_crosses_nodes() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "i");
        source.append(1, Some(11), "\u{307}eva");
        let style = BTreeMap::from([("text-transform".into(), "capitalize".into())]);
        let languages = BTreeMap::from([(1, fontique::Language::parse("lt").ok())]);
        let transformed = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(transformed.text, "Ieva");
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 8.,
            height: 16.,
        };
        let mapped = transformed.map_clusters(&[TextCluster {
            bytes: 0..1,
            rect,
            line: 0,
            rtl: false,
        }]);
        assert_eq!(
            mapped
                .iter()
                .map(|span| (span.text_node, span.utf16.clone()))
                .collect::<Vec<_>>(),
            vec![(10, 0..1), (11, 0..1)]
        );
        assert_eq!(transformed.byte_boundary(11, 1, false), Some(1));
    }

    #[test]
    fn erased_combining_marks_share_grapheme_geometry_across_native_nodes() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "I");
        source.append(1, Some(11), "\u{307}");
        let style = BTreeMap::from([("text-transform".into(), "lowercase".into())]);
        let languages = BTreeMap::from([(1, fontique::Language::parse("tr").ok())]);
        let transformed = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(transformed.text, "i");
        assert!(
            transformed
                .spans
                .iter()
                .find(|span| span.text_node == Some(11))
                .unwrap()
                .bytes
                .is_empty()
        );
        assert_eq!(transformed.geometry_aliases.len(), 1);
        let rect = oryn_common::v4::CssRect {
            x: 4.,
            y: 8.,
            width: 7.,
            height: 16.,
        };
        let mapped = transformed.map_clusters(&[TextCluster {
            bytes: 0..1,
            rect,
            line: 0,
            rtl: false,
        }]);
        assert_eq!(
            mapped
                .iter()
                .map(|span| (span.text_node, span.utf16.clone(), span.rect))
                .collect::<Vec<_>>(),
            vec![(10, 0..1, rect), (11, 0..1, rect)]
        );
    }

    #[test]
    fn greek_removed_marks_cover_the_whole_expanded_typographic_unit() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "α\u{301}\u{345}");
        let style = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let languages = BTreeMap::from([(1, fontique::Language::parse("el").ok())]);
        let transformed = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(transformed.text, "ΑΙ");
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 16.,
        };
        let mapped = transformed.map_clusters(&[
            TextCluster {
                bytes: 0..2,
                rect,
                line: 0,
                rtl: false,
            },
            TextCluster {
                bytes: 2..4,
                rect: oryn_common::v4::CssRect { x: 10., ..rect },
                line: 0,
                rtl: false,
            },
        ]);
        assert_eq!(mapped.len(), 2);
        assert!(mapped.iter().all(|span| span.utf16 == (0..3)));
        assert_eq!(mapped[0].rect.width + mapped[1].rect.width, 20.);
        // Collapsed whitespace is not a combining mark and must not alias a
        // neighboring glyph merely because it also produces no output bytes.
        let mut spaces = TextSource::default();
        spaces.append(1, Some(12), "a  b");
        let normalized = spaces.normalize(&BTreeMap::new(), &style).transform(
            &BTreeMap::new(),
            &style,
            &languages,
        );
        assert!(normalized.geometry_aliases.is_empty());
    }

    #[test]
    fn geometry_alias_lookup_handles_nested_intervals_and_visual_order() {
        let source = TextSource {
            text: "abcd".into(),
            spans: (0..4)
                .map(|byte| TextSpan {
                    atomic: false,
                    owner: 1,
                    text_node: Some(10),
                    bytes: byte..byte + 1,
                    utf16_start: byte,
                    utf16_len: 1,
                    transformed: false,
                })
                .collect(),
            geometry_aliases: vec![TextSpan {
                atomic: false,
                owner: 1,
                text_node: Some(11),
                bytes: 0..4,
                utf16_start: 0,
                utf16_len: 1,
                transformed: true,
            }],
        };
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 16.,
        };
        let mapped = source.map_clusters(&[
            TextCluster {
                bytes: 3..4,
                rect,
                line: 0,
                rtl: true,
            },
            TextCluster {
                bytes: 2..3,
                rect: oryn_common::v4::CssRect { x: 10., ..rect },
                line: 0,
                rtl: true,
            },
        ]);
        assert_eq!(mapped.iter().filter(|span| span.text_node == 11).count(), 2);
        assert_eq!(
            mapped
                .iter()
                .filter(|span| span.text_node == 10)
                .map(|span| span.utf16.clone())
                .collect::<Vec<_>>(),
            vec![3..4, 2..3]
        );
        assert!(mapped.iter().all(|span| span.rtl));
    }

    #[test]
    fn greek_uppercase_context_crosses_text_nodes_and_reacts_to_language_changes() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "Νερά");
        source.append(1, Some(11), "ιδα");
        let style = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let greek = BTreeMap::from([(1, fontique::Language::parse("el").ok())]);
        let transformed = source.transform(&BTreeMap::new(), &style, &greek);
        assert_eq!(transformed.text, "ΝΕΡΑΪΔΑ");
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 16.,
            height: 16.,
        };
        let mapped = transformed.map_clusters(&[TextCluster {
            bytes: 8..10,
            rect,
            line: 0,
            rtl: false,
        }]);
        assert_eq!((mapped[0].text_node, mapped[0].utf16.clone()), (11, 0..1));
        let untailored = source.transform(&BTreeMap::new(), &style, &BTreeMap::new());
        assert_eq!(untailored.text, "ΝΕΡΆΙΔΑ");
        assert_eq!(source.text, "Νεράιδα");
    }

    #[test]
    fn capitalization_retains_word_context_across_style_and_text_node_boundaries() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "i");
        source.append(2, Some(11), "js");
        source.append(3, Some(12), "land ß");
        source.append(3, Some(13), "eta");
        let style = BTreeMap::from([("text-transform".into(), "capitalize".into())]);
        let languages = BTreeMap::from([
            (1, fontique::Language::parse("nl").ok()),
            (2, fontique::Language::parse("nl").ok()),
        ]);
        let transformed = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(transformed.text, "IJsland Sseta");
        assert_eq!(transformed.byte_boundary(11, 1, false), Some(2));
        assert_eq!(transformed.byte_boundary(12, 5, false), Some(8));
        assert_eq!(transformed.byte_boundary(12, 6, false), Some(10));
        assert_eq!(transformed.byte_boundary(13, 0, false), Some(10));
        let styles = BTreeMap::from([(
            1,
            std::sync::Arc::new(BTreeMap::from([("text-transform".into(), "none".into())])),
        )]);
        // The untransformed initial letter still consumes the word's initial
        // position; later spans do not capitalize their own initial characters.
        let english = source.transform(&styles, &style, &BTreeMap::new());
        assert_eq!(english.text, "ijsland Sseta");
    }

    #[test]
    fn width_and_kana_mapping_keep_surrogate_offsets_and_collapsed_space_provenance() {
        let mut source = TextSource::default();
        source.append(1, Some(2), "a  \u{1b132}ｧ");
        let style = BTreeMap::from([("text-transform".into(), "full-width full-size-kana".into())]);
        let transformed = source.normalize(&BTreeMap::new(), &style).transform(
            &BTreeMap::new(),
            &style,
            &BTreeMap::new(),
        );
        assert_eq!(transformed.text, "ａ　こア");
        assert_eq!(transformed.byte_boundary(2, 3, false), Some(6));
        assert_eq!(transformed.byte_boundary(2, 4, false), Some(6));
        assert_eq!(transformed.byte_boundary(2, 4, true), Some(9));
        assert_eq!(transformed.byte_boundary(2, 5, false), Some(9));
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 16.,
            height: 16.,
        };
        let mapped = transformed.map_clusters(&[TextCluster {
            bytes: 6..9,
            rect,
            line: 0,
            rtl: false,
        }]);
        assert_eq!(mapped[0].utf16, 3..5);
        assert_eq!(source.text, "a  \u{1b132}ｧ");
    }

    #[test]
    fn case_expansion_preserves_native_ranges_and_collapsed_whitespace() {
        let mut source = TextSource::default();
        source.append(1, Some(2), "a  ß😀");
        let style = BTreeMap::from([("text-transform".into(), "uppercase".into())]);
        let rendered = source.normalize(&BTreeMap::new(), &style).transform(
            &BTreeMap::new(),
            &style,
            &BTreeMap::new(),
        );
        assert_eq!(rendered.text, "A SS😀");
        assert_eq!(source.text, "a  ß😀");
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 16.,
        };
        let clusters = rendered.map_clusters(&[
            TextCluster {
                bytes: 2..3,
                rect,
                line: 0,
                rtl: false,
            },
            TextCluster {
                bytes: 3..4,
                rect: oryn_common::v4::CssRect { x: 10., ..rect },
                line: 0,
                rtl: false,
            },
            TextCluster {
                bytes: 4..8,
                rect: oryn_common::v4::CssRect { x: 20., ..rect },
                line: 0,
                rtl: false,
            },
        ]);
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| cluster.utf16.clone())
                .collect::<Vec<_>>(),
            vec![3..4, 3..4, 4..6]
        );
        assert_eq!(rendered.byte_boundary(2, 3, false), Some(2));
        assert_eq!(rendered.byte_boundary(2, 4, false), Some(4));
        assert_eq!(rendered.byte_boundary(2, 5, true), Some(8));
    }

    #[test]
    fn case_context_crosses_text_nodes_without_losing_each_nodes_language() {
        let mut source = TextSource::default();
        source.append(1, Some(10), "Ο");
        source.append(2, Some(11), "Σ I");
        source.append(2, Some(12), "\u{307}");
        let style = BTreeMap::from([("text-transform".into(), "lowercase".into())]);
        let languages = BTreeMap::from([(2, fontique::Language::parse("tr").ok())]);
        let rendered = source.transform(&BTreeMap::new(), &style, &languages);
        assert_eq!(rendered.text, "ος i");
        assert_eq!(
            rendered.byte_boundary(12, 0, false),
            Some(rendered.text.len())
        );
        assert_eq!(
            rendered.byte_boundary(12, 1, true),
            Some(rendered.text.len())
        );
        let styles = BTreeMap::from([(
            1,
            std::sync::Arc::new(BTreeMap::from([("text-transform".into(), "none".into())])),
        )]);
        let mixed = source.transform(&styles, &style, &languages);
        assert_eq!(mixed.text, "Ος i");
    }

    #[test]
    fn whitespace_modes_preserve_source_offsets_without_changing_dom_text() {
        let mut source = TextSource::default();
        source.append(1, Some(2), "A  B\r\n C\tD");
        let normal = source.normalize(&BTreeMap::new(), &BTreeMap::new());
        assert_eq!(normal.text, "A B C D");
        assert_eq!(source.text, "A  B\r\n C\tD");
        assert_eq!(normal.byte_boundary(2, 3, false), Some(2));
        assert_eq!(normal.byte_boundary(2, 8, false), Some(5));
        let style = |mode: &str| BTreeMap::from([("white-space-collapse".into(), mode.into())]);
        assert_eq!(
            source.normalize(&BTreeMap::new(), &style("preserve")).text,
            "A  B\n C\tD"
        );
        assert_eq!(
            source
                .normalize(&BTreeMap::new(), &style("preserve-spaces"))
                .text,
            "A  B  C D"
        );
        let breaks = source.normalize(&BTreeMap::new(), &style("preserve-breaks"));
        assert_eq!(breaks.text, "A B\nC D");
        let mut adjacent = TextSource::default();
        adjacent.append(1, Some(2), "A  ");
        adjacent.append(1, Some(3), "\n B");
        let breaks = adjacent.normalize(&BTreeMap::new(), &style("preserve-breaks"));
        assert_eq!(breaks.text, "A\nB");
        assert_eq!(breaks.byte_boundary(2, 1, false), Some(1));
        assert_eq!(breaks.byte_boundary(3, 2, false), Some(2));
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 16.,
        };
        let mapped = normal.map_clusters(&[TextCluster {
            bytes: 2..3,
            rect,
            line: 0,
            rtl: false,
        }]);
        assert_eq!(mapped[0].utf16, 3..4);
    }

    #[test]
    fn cluster_mapping_retains_visual_order_and_cross_node_clusters() {
        let source = TextSource {
            geometry_aliases: Vec::new(),
            text: "a😀z".into(),
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
                    bytes: 1..6,
                    utf16_len: 3,
                },
            ],
        };
        let rect = oryn_common::v4::CssRect {
            x: 20.,
            y: 5.,
            width: 12.,
            height: 16.,
        };
        let mapped = source.map_clusters(&[
            TextCluster {
                bytes: 5..6,
                rect,
                line: 0,
                rtl: true,
            },
            TextCluster {
                bytes: 0..5,
                rect: oryn_common::v4::CssRect { x: 32., ..rect },
                line: 0,
                rtl: true,
            },
        ]);
        assert_eq!(
            mapped
                .iter()
                .map(|item| (item.text_node, item.utf16.clone()))
                .collect::<Vec<_>>(),
            vec![(11, 2..3), (10, 0..1), (11, 0..2)]
        );
        assert!(mapped.iter().all(|item| item.rtl));
        assert_eq!(mapped[1].rect, mapped[2].rect);
        assert_eq!(mapped[0].rect.x, 20.);
    }

    #[test]
    fn utf16_boundaries_preserve_non_bmp_and_empty_node_anchors() {
        let source = TextSource {
            geometry_aliases: Vec::new(),
            text: "a😀z".into(),
            spans: vec![
                TextSpan {
                    atomic: false,
                    utf16_start: 0,
                    transformed: false,
                    owner: 1,
                    text_node: Some(2),
                    bytes: 0..6,
                    utf16_len: 4,
                },
                TextSpan {
                    atomic: false,
                    utf16_start: 0,
                    transformed: false,
                    owner: 1,
                    text_node: Some(3),
                    bytes: 6..6,
                    utf16_len: 0,
                },
            ],
        };
        assert_eq!(source.byte_boundary(2, 0, false), Some(0));
        assert_eq!(source.byte_boundary(2, 1, false), Some(1));
        assert_eq!(source.byte_boundary(2, 2, false), Some(1));
        assert_eq!(source.byte_boundary(2, 2, true), Some(5));
        assert_eq!(source.byte_boundary(2, 3, false), Some(5));
        assert_eq!(source.byte_boundary(2, 4, false), Some(6));
        assert_eq!(source.byte_boundary(2, 5, false), None);
        assert_eq!(source.byte_boundary(3, 0, false), Some(6));
        assert_eq!(source.byte_boundary(99, 0, false), None);
    }
}
