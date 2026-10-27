//! Formatting-tree participants for shared inline paragraphs. DOM ancestry and
//! text identities stay intact; anonymous paragraph boundaries are local layout
//! records rather than fabricated DOM elements.

/// Inputs for layout flow.
pub(crate) struct InlineFlowInputs<'a> {
    pub(crate) flow: Vec<FlowItem>,
    pub(crate) style: &'a BTreeMap<String, String>,
    pub(crate) inline: f64,
    pub(crate) full_inline: f64,
    pub(crate) block_available: f64,
    pub(crate) block_limit: Option<f64>,
    pub(crate) font_size: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) input: &'a LayoutInput,
    pub(crate) geometry: &'a mut BTreeMap<u64, NodeGeometry>,
    pub(crate) offsets: &'a mut BTreeMap<u64, (f64, f64)>,
    pub(crate) resolved: &'a mut BTreeMap<u64, BTreeMap<String, String>>,
    pub(crate) paint: &'a mut BTreeMap<u64, Vec<PaintTextFragment>>,
    pub(crate) diagnostics: &'a mut Vec<String>,
    pub(crate) engine: &'a mut TextEngine,
}
/// Inputs for inline geometry.
pub(crate) struct InlineGeometryInputs<'a> {
    pub(crate) content_width: f64,
    pub(crate) content_height: f64,
    pub(crate) paragraph: &'a Paragraph,
    pub(crate) layout: &'a TextLayout,
    pub(crate) root: u64,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) viewport: ViewportState,
    pub(crate) inline_basis: f64,
    pub(crate) geometry: &'a mut BTreeMap<u64, NodeGeometry>,
    pub(crate) offsets: &'a mut BTreeMap<u64, (f64, f64)>,
}
use super::*;
#[derive(Debug, Default)]
pub(super) struct Paragraph {
    pub source: text_source::TextSource,
    pub inline_nodes: BTreeSet<u64>,
    pub atomic: BTreeSet<u64>,
    pub boundaries: BTreeMap<u64, (u64, bool)>,
    // Suppressed empty lines still give inline boxes a scroll/geometry position.
    pub empty_positions: BTreeMap<u64, f64>,
}
#[derive(Debug)]
pub(super) enum FlowItem {
    Paragraph(Paragraph),
    Block(u64),
}
/// CSS empty-line suppression is also the parent-margin collapse barrier.
/// Decorations on an empty inline participate even when their advances cancel;
/// display:contents contributes no box of its own.
pub(super) fn paragraph_forms_line(
    paragraph: &Paragraph,
    styles: &ComputedStyles,
    default: &BTreeMap<String, String>,
    basis: f64,
    viewport: ViewportState,
) -> bool {
    if has_text_line_content(
        &paragraph.source.normalize(styles, default),
        styles,
        default,
    ) {
        return true;
    }
    if paragraph.atomic.iter().any(|id| {
        styles.get(id).is_some_and(|style| {
            style_value(style, "float", "none") == "none"
                && !matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                )
        })
    }) {
        return true;
    }
    paragraph.inline_nodes.iter().any(|id| {
        styles.get(id).is_some_and(|style| {
            if style_value(style, "display", "inline") == "contents" {
                return false;
            }
            let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
            [
                edge(style, "margin", basis, font, viewport),
                edge(style, "padding", basis, font, viewport),
                border_edge(style, basis, font, viewport),
            ]
            .iter()
            .any(|edge| {
                [edge.top, edge.right, edge.bottom, edge.left]
                    .iter()
                    .any(|value| *value != 0.)
            })
        })
    })
}
fn content(node: &LayoutNode) -> Vec<LayoutContent> {
    node.content.clone().unwrap_or_else(|| {
        let mut content = vec![LayoutContent::StyledText {
            owner: node.id,
            node_id: None,
            text: node.text.clone(),
        }];
        content.extend(node.children.iter().map(|id| LayoutContent::Element(*id)));
        content
    })
}
pub(super) fn collect(
    root: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Vec<FlowItem> {
    let mut result = Vec::new();
    let mut paragraph = Paragraph::default();
    // Block-only collections never allocate synthetic inline identities. Do
    // not scan the entire document for each nested block's local collection.
    let mut used = None::<BTreeSet<u64>>;
    let mut next = u64::MAX;
    let mut allocate = || {
        let used = used.get_or_insert_with(|| {
            let mut identities = nodes.keys().copied().collect::<BTreeSet<_>>();
            for node in nodes.values() {
                for item in node.content.iter().flatten() {
                    if let LayoutContent::Text { node_id, .. }
                    | LayoutContent::StyledText {
                        node_id: Some(node_id),
                        ..
                    } = item
                    {
                        identities.insert(*node_id);
                    }
                }
            }
            identities
        });
        while used.contains(&next) {
            next = next
                .checked_sub(1)
                .expect("finite inline participant identities");
        }
        let id = next;
        used.insert(id);
        next = next.saturating_sub(1);
        id
    };
    let mut boundaries = BTreeMap::<u64, (u64, bool)>::new();
    let mut pending = content(root)
        .into_iter()
        .rev()
        .map(|item| (root.id, item))
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::from([root.id]);
    let flush = |paragraph: &mut Paragraph, result: &mut Vec<FlowItem>| {
        if !paragraph.source.spans.is_empty() || !paragraph.inline_nodes.is_empty() {
            result.push(FlowItem::Paragraph(std::mem::take(paragraph)));
        }
    };
    while let Some((owner, item)) = pending.pop() {
        match item {
            LayoutContent::Text { node_id, text } => {
                paragraph.source.append(owner, Some(node_id), &text)
            }
            LayoutContent::StyledText {
                owner,
                node_id,
                text,
            } => paragraph.source.append(owner, node_id, &text),
            LayoutContent::Element(id) => {
                if let Some(boundary) = boundaries.get(&id).copied() {
                    paragraph.boundaries.insert(id, boundary);
                    paragraph.source.append_boundary(id);
                    continue;
                }
                if !seen.insert(id) {
                    continue;
                }
                let (Some(node), Some(style)) = (nodes.get(&id), styles.get(&id)) else {
                    continue;
                };
                let display = style_value(style, "display", "inline");
                if display == "none" {
                    continue;
                }
                let out_of_flow = matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                ) || style_value(style, "float", "none") != "none";
                if node.tag == "br" && !out_of_flow {
                    paragraph.inline_nodes.insert(id);
                    paragraph.source.append(id, None, "\u{2028}");
                    continue;
                }
                if !out_of_flow
                    && (display == "contents"
                        || (matches!(
                            display,
                            "inline"
                                | "ruby-base"
                                | "ruby-text"
                                | "ruby-base-container"
                                | "ruby-text-container"
                        ) && !is_replaced_box(node)))
                {
                    paragraph.inline_nodes.insert(id);
                    let start = allocate();
                    let end = allocate();
                    boundaries.insert(start, (id, true));
                    boundaries.insert(end, (id, false));
                    paragraph.boundaries.insert(start, (id, true));
                    paragraph.source.append_boundary(start);
                    pending.push((id, LayoutContent::Element(end)));
                    pending.extend(content(node).into_iter().rev().map(|item| (id, item)));
                    continue;
                }
                if out_of_flow
                    || display.starts_with("inline")
                    || display == "ruby"
                    || (display == "inline" && is_replaced_box(node))
                {
                    paragraph.atomic.insert(id);
                    paragraph.source.append_box(id);
                    continue;
                }
                flush(&mut paragraph, &mut result);
                result.push(FlowItem::Block(id));
            }
        }
    }
    flush(&mut paragraph, &mut result);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cluster_line_index_preserves_visual_order_and_ignores_invalid_lines() {
        use crate::text_source::TextCluster;
        let rect = CssRect { x: 0., y: 0., width: 1., height: 1. };
        let clusters = vec![
            TextCluster { bytes: 8..10, rect, line: 1, rtl: true },
            TextCluster { bytes: 0..2, rect, line: 0, rtl: false },
            TextCluster { bytes: 6..8, rect, line: 1, rtl: true },
            TextCluster { bytes: 2..4, rect, line: 0, rtl: true },
            TextCluster { bytes: 10..12, rect, line: 9, rtl: false },
        ];
        let index = ClusterIndex::new(&clusters, 2);
        for line in 0..2 {
            let original: Vec<_> = clusters.iter().filter(|c| c.line == line).collect();
            assert_eq!(index.select(line, 0, 12), original);
            for range in [0..12, 2..8, 6..10, 9..8] {
                let selected = index.select(line, range.start, range.end);
                let original: Vec<_> = clusters.iter().filter(|c| c.line == line && c.bytes.start >= range.start && c.bytes.end <= range.end).collect();
                assert_eq!(selected, original);
            }
        }
    }
    fn node(id: u64, parent: Option<u64>, tag: &str, text: &str) -> LayoutNode {
        LayoutNode {
            tree_scope: None,
            document_url: None,
            disabled: None,
            id,
            parent,
            tag: tag.into(),
            namespace: default_element_namespace(),
            children: Vec::new(),
            attributes: BTreeMap::new(),
            namespaced_attributes: Vec::new(),
            checked: None,
            validity: None,
            indeterminate: false,
            image_intrinsics: None,
            image_document_root: None,
            document_replaced: false,
            intrinsic_image_size: None,
            text: text.into(),
            content: None,
        }
    }
    #[test]
    fn lazy_boundary_ids_avoid_document_and_text_identity_collisions() {
        let mut root = node(1, None, "div", "");
        root.children = vec![2];
        let span = node(2, Some(1), "span", "text");
        let mut other = node(u64::MAX, None, "div", "");
        other.content = Some(vec![LayoutContent::Text {
            node_id: u64::MAX - 1,
            text: "outside this paragraph".into(),
        }]);
        let nodes = BTreeMap::from([(1, &root), (2, &span), (u64::MAX, &other)]);
        let styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("display".into(), "block".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([("display".into(), "inline".into())])),
            ),
        ]);
        let flow = collect(&root, &nodes, &styles);
        let FlowItem::Paragraph(paragraph) = &flow[0] else {
            panic!("paragraph")
        };
        assert_eq!(paragraph.boundaries.len(), 2);
        assert!(paragraph.boundaries.keys().all(|id| *id < u64::MAX - 1));
        assert_eq!(paragraph.source.text, "text");
    }

    #[test]
    fn nested_text_and_atomic_boxes_keep_one_ordered_paragraph() {
        let mut root = node(1, None, "div", "");
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
        let image = node(3, Some(1), "img", "");
        let nodes = BTreeMap::from([(1, &root), (2, &span), (3, &image)]);
        let styles = BTreeMap::from([
            (
                1,
                Arc::new(BTreeMap::from([("display".into(), "block".into())])),
            ),
            (
                2,
                Arc::new(BTreeMap::from([("display".into(), "inline".into())])),
            ),
            (
                3,
                Arc::new(BTreeMap::from([("display".into(), "inline".into())])),
            ),
        ]);
        let flow = collect(&root, &nodes, &styles);
        assert_eq!(flow.len(), 1);
        let FlowItem::Paragraph(paragraph) = &flow[0] else {
            panic!("paragraph")
        };
        assert_eq!(paragraph.source.text, "a b c");
        assert_eq!(paragraph.atomic, BTreeSet::from([3]));
        assert!(
            paragraph
                .source
                .spans
                .iter()
                .any(|span| span.owner == 3 && span.bytes == (3..3))
        );
        let normalized = paragraph.source.normalize(&styles, &styles[&1]);
        assert!(
            normalized
                .spans
                .iter()
                .any(|span| span.owner == 3 && span.transformed && span.bytes.is_empty())
        );
    }
}

/// Byte-range lookup retaining the native visual order of each line.
struct ClusterIndex<'a>(Vec<Vec<(usize, &'a crate::text_source::TextCluster)>>);
impl<'a> ClusterIndex<'a> {
    fn new(clusters: &'a [crate::text_source::TextCluster], line_count: usize) -> Self {
        let mut index = vec![Vec::new(); line_count];
        for (visual_order, cluster) in clusters.iter().enumerate() {
            if let Some(line) = index.get_mut(cluster.line) {
                line.push((visual_order, cluster));
            }
        }
        for line in &mut index {
            line.sort_unstable_by_key(|(order, cluster)| (cluster.bytes.start, *order));
        }
        Self(index)
    }
    fn select(&self, line: usize, start: usize, end: usize) -> Vec<&'a crate::text_source::TextCluster> {
        let line = &self.0[line];
        let lower = line.partition_point(|(_, c)| c.bytes.start < start);
        let upper = line.partition_point(|(_, c)| c.bytes.start <= end);
        if lower > upper { return Vec::new(); }
        let mut selected: Vec<_> = line[lower..upper].iter().copied().filter(|(_, c)| c.bytes.end <= end).collect();
        selected.sort_unstable_by_key(|(order, _)| *order);
        selected.into_iter().map(|(_, c)| c).collect()
    }
}

/// Partition native shaped runs without re-shaping, distributing characters by
/// glyph count, or changing the original DOM text. Ligature clusters retain the
/// complete native byte range selected by their owning run.
pub(super) fn paint_fragments(
    layout: &TextLayout,
    root: u64,
    x: f64,
    y: f64,
    styles: &ComputedStyles,
) -> BTreeMap<u64, Vec<PaintTextFragment>> {
    let mut output = BTreeMap::<u64, Vec<PaintTextFragment>>::new();
    // Preserve visual order within each line without scanning every other
    // line's clusters for every source span (quadratic for large documents).
    let clusters_by_line = ClusterIndex::new(&layout.clusters, layout.lines.len());
    for (line_index, line) in layout.lines.iter().enumerate() {
        let Some(line_range) = layout.line_ranges.get(line_index) else {
            continue;
        };
        let Some(provenance) = layout.provenance.get(line_index) else {
            continue;
        };
        let mut grouped =
            BTreeMap::<(u64, usize, usize), Vec<oryn_common::v4::PaintGlyphRun>>::new();
        for (run, source) in layout.glyph_runs[line_index].iter().zip(provenance) {
            let owner = if source.owner == 0 {
                root
            } else {
                source.owner
            };
            let start = source.bytes.start.max(line_range.start);
            let end = source.bytes.end.min(line_range.end);
            let mut run = run.clone();
            for (glyph, bytes) in run.glyphs.iter_mut().zip(&source.glyph_ranges) {
                glyph.source =
                    (bytes.start >= start && bytes.end <= end && bytes.start < bytes.end)
                        .then(|| bytes.start - start..bytes.end - start);
            }
            grouped.entry((owner, start, end)).or_default().push(run);
        }
        // Shaping runs retain font and bidi boundaries, but are not themselves
        // paint fragments. In particular vertical shaping emits one run per
        // cluster. Join adjacent source spans of the same owner/direction while
        // keeping every native glyph run and its independently selected font.
        let mut spans = Vec::<(
            (u64, usize, usize),
            Vec<oryn_common::v4::PaintGlyphRun>,
            Option<bool>,
        )>::new();
        for ((owner, start, end), mut runs) in grouped {
            let selected = clusters_by_line.select(line_index, start, end);
            let mut directions = selected
                .iter()
                .map(|cluster| cluster.rtl);
            let direction = directions
                .next()
                .filter(|first| directions.all(|next| next == *first));
            if let Some((
                (previous_owner, previous_start, previous_end),
                previous_runs,
                previous_direction,
            )) = spans.last_mut()
                && *previous_owner == owner
                && *previous_end == start
                && direction.is_some()
                && *previous_direction == direction
            {
                let offset = start - *previous_start;
                for run in &mut runs {
                    for glyph in &mut run.glyphs {
                        if let Some(source) = &mut glyph.source {
                            source.start += offset;
                            source.end += offset;
                        }
                    }
                }
                *previous_end = end;
                previous_runs.extend(runs);
                continue;
            }
            spans.push(((owner, start, end), runs, direction));
        }
        for ((owner, start, end), mut runs, _) in spans {
            let Some(style) = styles.get(&owner).or_else(|| styles.get(&root)) else {
                continue;
            };
            if style_value(style, "visibility", "visible") != "visible" {
                continue;
            }
            let mut bounds = None::<CssRect>;
            for run in &runs {
                if let Some(rect) = run.decoration_rect {
                    bounds = Some(bounds.map(|previous| union(previous, rect)).unwrap_or(rect));
                }
            }
            let bounds = bounds.unwrap_or(CssRect {
                x: 0.,
                y: 0.,
                width: line.1,
                height: line.2,
            });
            for run in &mut runs {
                if let Some(box_style) = &mut run.line_box {
                    box_style.translate(-bounds.x, -bounds.y);
                }
                if let Some(box_style) = &mut run.text_box {
                    box_style.translate(-bounds.x, -bounds.y);
                }
                for glyph in &mut run.glyphs {
                    glyph.x -= bounds.x;
                    glyph.y -= bounds.y;
                }
                if let Some(rect) = &mut run.decoration_rect {
                    rect.x -= bounds.x;
                    rect.y -= bounds.y;
                }
            }
            let text = if start <= end {
                line.0
                    .get(
                        start.saturating_sub(line_range.start)
                            ..end.saturating_sub(line_range.start),
                    )
                    .unwrap_or("")
            } else {
                ""
            };
            let selected = clusters_by_line.select(line_index, start, end);
            let clusters = selected
                .iter()
                .map(|cluster| oryn_common::v4::PaintTextCluster {
                    start: cluster.bytes.start - start,
                    end: cluster.bytes.end - start,
                    rect: CssRect {
                        x: x + cluster.rect.x,
                        y: y + cluster.rect.y,
                        ..cluster.rect
                    },
                    rtl: cluster.rtl,
                })
                .collect();
            let mut flow = layout.flow_rect(
                line_index,
                styles
                    .get(&root)
                    .map(|style| style_value(style, "writing-mode", "horizontal-tb"))
                    .unwrap_or("horizontal-tb"),
            );
            flow.x += x;
            flow.y += y;
            output.entry(owner).or_default().push(PaintTextFragment {
                flow_line: Some(line_index),
                flow_rect: Some(flow),
                text: text.into(),
                rect: CssRect {
                    x: x + line.4 + bounds.x,
                    y: y + line.3 + bounds.y,
                    ..bounds
                },
                font_family: style_value(style, "font-family", DEFAULT_FONT_FAMILY).into(),
                font_size: runs.first().map(|run| run.font_size).unwrap_or(16.),
                color: style_value(style, "color", "black").into(),
                glyph_runs: runs,
                clusters,
            });
        }
    }
    output
}
fn union(a: CssRect, b: CssRect) -> CssRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    CssRect {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}

pub(super) fn inline_geometry(params: InlineGeometryInputs<'_>) {
    let InlineGeometryInputs {
        content_width,
        content_height,
        paragraph,
        layout,
        root,
        x,
        y,
        nodes,
        styles,
        viewport,
        inline_basis,
        geometry,
        offsets,
    } = params;

    let mut rectangles = BTreeMap::<u64, BTreeMap<usize, CssRect>>::new();
    let mut add = |owner: u64, line: usize, rect: CssRect| {
        let mut node = Some(owner);
        let mut seen = BTreeSet::new();
        while let Some(id) = node {
            if id == root || !seen.insert(id) {
                break;
            }
            if paragraph.inline_nodes.contains(&id) {
                rectangles
                    .entry(id)
                    .or_default()
                    .entry(line)
                    .and_modify(|old| *old = union(*old, rect))
                    .or_insert(rect);
            }
            node = nodes.get(&id).and_then(|node| node.parent);
        }
    };
    for (line_index, (runs, sources)) in
        layout.glyph_runs.iter().zip(&layout.provenance).enumerate()
    {
        let line = &layout.lines[line_index];
        for (run, source) in runs.iter().zip(sources) {
            if source.bytes.is_empty() {
                continue;
            }
            if let Some(rect) = run.decoration_rect {
                add(
                    source.owner,
                    line_index,
                    CssRect {
                        x: x + line.4 + rect.x,
                        y: y + line.3 + rect.y,
                        ..rect
                    },
                );
            }
        }
    }
    for item in &layout.inline_boxes {
        if item.line == usize::MAX {
            continue;
        }
        let owner = paragraph
            .boundaries
            .get(&item.node)
            .map(|(owner, _)| *owner)
            .unwrap_or(item.node);
        let mut rect = CssRect {
            x: x + item.rect.x,
            y: y + item.rect.y,
            ..item.rect
        };
        if let Some((_, start)) = paragraph.boundaries.get(&item.node)
            && let Some(style) = styles.get(&owner)
        {
            let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
            let margin = edge(style, "margin", inline_basis, font, viewport);
            let axes = writing_mode::Axes::new(
                style_value(style, "writing-mode", "horizontal-tb"),
                style_value(style, "direction", "ltr") == "rtl",
            );
            let reverse = if axes.vertical {
                axes.y_reverse
            } else {
                axes.x_reverse
            };
            let start = *start != reverse;
            if axes.vertical {
                let edge = if start { margin.top } else { margin.bottom };
                if start {
                    rect.y += edge;
                }
                rect.height -= edge;
            } else {
                let edge = if start { margin.left } else { margin.right };
                if start {
                    rect.x += edge;
                }
                rect.width -= edge;
            }
        }
        add(owner, item.line, rect);
    }
    for line_break in &layout.breaks {
        add(
            line_break.node,
            line_break.line,
            CssRect {
                x: x + line_break.before.x,
                y: y + line_break.before.y,
                ..line_break.before
            },
        );
    }
    for id in &paragraph.inline_nodes {
        let Some(style) = styles.get(id) else {
            continue;
        };
        let lines = rectangles.remove(id).or_else(|| {
            let block = *paragraph.empty_positions.get(id)?;
            if style_value(style, "display", "inline") == "contents" {
                return None;
            }
            let root_style = styles.get(&root)?;
            let axes = writing_mode::Axes::new(
                style_value(root_style, "writing-mode", "horizontal-tb"),
                style_value(root_style, "direction", "ltr") == "rtl",
            );
            let (inline_x, inline_y) = if axes.vertical {
                (
                    if axes.x_reverse {
                        content_width - block
                    } else {
                        block
                    },
                    if axes.y_reverse { content_height } else { 0. },
                )
            } else {
                (if axes.x_reverse { content_width } else { 0. }, block)
            };
            Some(BTreeMap::from([(
                0,
                CssRect {
                    x: x + inline_x,
                    y: y + inline_y,
                    width: 0.,
                    height: 0.,
                },
            )]))
        });
        let Some(lines) = lines else {
            continue;
        };
        let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
        let border = border_edge(style, inline_basis, font, viewport);
        let padding = edge(style, "padding", inline_basis, font, viewport);
        let margin = edge(style, "margin", inline_basis, font, viewport);
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let rects = lines
            .into_values()
            .map(|mut rect| {
                if vertical {
                    rect.x -= border.left + padding.left;
                    rect.width += border.left + padding.left + border.right + padding.right;
                } else {
                    rect.y -= border.top + padding.top;
                    rect.height += border.top + padding.top + border.bottom + padding.bottom;
                }
                rect
            })
            .collect::<Vec<_>>();
        let rect = rects.iter().copied().reduce(union).unwrap();
        offsets.insert(*id, (rect.width, rect.height));
        geometry.insert(
            *id,
            NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(BoxModel {
                    border_box: rect,
                    padding_box: inset(rect, border),
                    content_box: inset(inset(rect, border), padding),
                    border,
                    padding,
                    margin,
                }),
                client_rects: rects,
                in_viewport: rect.intersects(CssRect {
                    x: 0.,
                    y: 0.,
                    width: viewport.width,
                    height: viewport.height,
                }),
                interactable: style_value(style, "visibility", "visible") == "visible"
                    && style_value(style, "pointer-events", "auto") != "none",
                diagnostic: None,
            },
        );
    }
}

pub(super) fn fragment_models(
    rects: &[CssRect],
    model: &BoxModel,
    style: &BTreeMap<String, String>,
) -> Vec<BoxModel> {
    let slice = style_value(style, "box-decoration-break", "slice") != "clone";
    let axes = writing_mode::Axes::new(
        style_value(style, "writing-mode", "horizontal-tb"),
        style_value(style, "direction", "ltr") == "rtl",
    );
    rects
        .iter()
        .enumerate()
        .map(|(index, rect)| {
            let mut border = model.border;
            let mut padding = model.padding;
            if slice {
                let start = index == 0;
                let end = index + 1 == rects.len();
                let block = !matches!(
                    style_value(style, "display", "inline"),
                    "inline"
                        | "ruby"
                        | "ruby-base"
                        | "ruby-text"
                        | "ruby-base-container"
                        | "ruby-text-container"
                        | "contents"
                );
                if block && !axes.vertical {
                    if !start {
                        border.top = 0.;
                        padding.top = 0.;
                    }
                    if !end {
                        border.bottom = 0.;
                        padding.bottom = 0.;
                    }
                } else if block {
                    if !(if axes.x_reverse { end } else { start }) {
                        border.left = 0.;
                        padding.left = 0.;
                    }
                    if !(if axes.x_reverse { start } else { end }) {
                        border.right = 0.;
                        padding.right = 0.;
                    }
                } else if axes.vertical {
                    if !(if axes.y_reverse { end } else { start }) {
                        border.top = 0.;
                        padding.top = 0.;
                    }
                    if !(if axes.y_reverse { start } else { end }) {
                        border.bottom = 0.;
                        padding.bottom = 0.;
                    }
                } else {
                    if !(if axes.x_reverse { end } else { start }) {
                        border.left = 0.;
                        padding.left = 0.;
                    }
                    if !(if axes.x_reverse { start } else { end }) {
                        border.right = 0.;
                        padding.right = 0.;
                    }
                }
            }
            BoxModel {
                border_box: *rect,
                padding_box: inset(*rect, border),
                content_box: inset(inset(*rect, border), padding),
                border,
                padding,
                margin: model.margin,
            }
        })
        .collect()
}

pub(super) struct FloatScope {
    context: Option<taffy::compute::FloatContext>,
    isolated: bool,
    fragment: Option<(f64, f64)>,
    previous_vertical: bool,
    width: f64,
    insets: [f64; 2],
    offset: f64,
}
pub(super) fn establishes_bfc(style: &BTreeMap<String, String>) -> bool {
    style_value(style, "column-span", "none") == "all"
        || matches!(
            style_value(style, "display", "block"),
            "flow-root"
                | "inline-block"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "table"
                | "inline-table"
                | "table-cell"
                | "table-caption"
        )
        || matches!(
            style_value(style, "position", "static"),
            "absolute" | "fixed"
        )
        || style_value(style, "float", "none") != "none"
        || ["overflow-x", "overflow-y"]
            .iter()
            .any(|key| !matches!(style_value(style, key, "visible"), "visible" | "clip"))
        || style_value(style, "contain", "none")
            .split_whitespace()
            .any(|part| matches!(part, "layout" | "paint" | "strict" | "content"))
}
pub(super) fn has_flow_floats(
    root: u64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    cache: &mut BTreeMap<u64, bool>,
) -> bool {
    if let Some(value) = cache.get(&root) {
        return *value;
    }
    let mut pending = vec![(root, false)];
    let mut seen = BTreeSet::new();
    while let Some((id, visited)) = pending.pop() {
        if cache.contains_key(&id) {
            continue;
        }
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        if !visited {
            if !seen.insert(id) {
                continue;
            }
            pending.push((id, true));
            for child in &node.children {
                let Some(style) = styles.get(child) else {
                    continue;
                };
                if style_value(style, "display", "block") == "none"
                    || matches!(
                        style_value(style, "position", "static"),
                        "absolute" | "fixed"
                    )
                {
                    continue;
                }
                if style_value(style, "float", "none") != "none" {
                    continue;
                }
                if !establishes_bfc(style) {
                    pending.push((*child, false));
                }
            }
        } else {
            let value = styles.get(&id).is_some_and(|style| {
                let variants = text_pseudos::read(style);
                variants.active
                    && variants
                        .letter
                        .is_some_and(|style| style_value(&style, "float", "none") != "none")
            }) || node.children.iter().any(|child| {
                styles.get(child).is_some_and(|style| {
                    style_value(style, "display", "block") != "none"
                        && !matches!(
                            style_value(style, "position", "static"),
                            "absolute" | "fixed"
                        )
                        && (style_value(style, "float", "none") != "none"
                            || (!establishes_bfc(style)
                                && cache.get(child).copied().unwrap_or(false)))
                })
            });
            cache.insert(id, value);
        }
    }
    cache.get(&root).copied().unwrap_or(false)
}
pub(super) fn enter_float_scope(
    engine: &mut TextEngine,
    style: &BTreeMap<String, String>,
    width: f64,
    x: f64,
    y: f64,
    root: bool,
    vertical: bool,
) -> FloatScope {
    let isolated = root
        || engine.float_vertical != vertical
        || engine.float_context.is_none()
        || establishes_bfc(style);
    let scope = FloatScope {
        context: if isolated {
            engine.float_context.take()
        } else {
            None
        },
        isolated,
        fragment: engine.float_fragment,
        previous_vertical: engine.float_vertical,
        width: engine.float_context_width,
        insets: engine.float_inline_insets,
        offset: engine.float_block_offset,
    };
    if isolated {
        engine.float_fragment = if engine.float_vertical == vertical {
            engine
                .float_fragment
                .map(|(height, origin)| (height, origin - engine.float_block_offset - y))
        } else {
            None
        };
        engine.float_context = Some(taffy::compute::FloatContext::new());
        engine.float_vertical = vertical;
        engine.float_context_width = width;
        engine.float_inline_insets = [0.; 2];
        engine.float_block_offset = 0.;
    } else {
        let left = engine.float_inline_insets[0] + x;
        engine.float_inline_insets = [left, engine.float_context_width - left - width];
        engine.float_block_offset += y;
    }
    scope
}
pub(super) fn leave_float_scope(engine: &mut TextEngine, scope: FloatScope) -> Option<f64> {
    let extent = if scope.isolated {
        engine
            .float_context
            .as_ref()
            .and_then(|context| context.cleared_threshold(taffy::Clear::Both))
            .map(f64::from)
    } else {
        None
    };
    if scope.isolated {
        engine.float_context = scope.context;
        engine.float_fragment = scope.fragment;
    }
    engine.float_vertical = scope.previous_vertical;
    engine.float_context_width = scope.width;
    engine.float_inline_insets = scope.insets;
    engine.float_block_offset = scope.offset;
    extent
}

/// Intrinsic/table measurement reuses the real inline participant formatter,
/// including atomic child geometry, instead of summing descendants as blocks.
pub(super) fn measure_paragraph(
    paragraph: Paragraph,
    style: &BTreeMap<String, String>,
    inline: f64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    viewport: ViewportState,
    engine: &mut TextEngine,
) -> TextLayout {
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
        nodes: Vec::new(),
        stylesheets: Vec::new(),
        stylesheet_media: BTreeMap::new(),
        selector_states: Default::default(),
        shadow_trees: Default::default(),
        stylesheet_base_urls: Default::default(),
        stylesheet_owner_ids: Default::default(),
        stylesheet_scope_roots: BTreeMap::new(),
        custom_states: BTreeMap::new(),
        focused_node: None,
        focus_visible: false,
        viewport,
        scroll: ScrollState::default(),
        color_scheme: ColorScheme::default(),
        reduced_motion: false,
        web_fonts: Vec::new(),
    };
    let sizes = std::mem::take(&mut engine.inline_sizes);
    let extents = std::mem::take(&mut engine.inline_extents);
    let floats = std::mem::take(&mut engine.inline_floats);
    let boxes = std::mem::take(&mut engine.active_inline_boxes);
    let context = engine.float_context.take();
    let columns = engine.column_context.take();
    let offset = engine.float_block_offset;
    let context_width = engine.float_context_width;
    let insets = engine.float_inline_insets;
    engine.float_block_offset = 0.;
    engine.float_context_width = inline;
    engine.float_inline_insets = [0.; 2];
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let (_, layout, _) = layout_flow(InlineFlowInputs {
        flow: vec![FlowItem::Paragraph(paragraph)],
        style,
        inline,
        full_inline: inline,
        block_available: viewport.width,
        block_limit: None,
        font_size: font,
        nodes,
        styles,
        input: &input,
        geometry: &mut BTreeMap::new(),
        offsets: &mut BTreeMap::new(),
        resolved: &mut BTreeMap::new(),
        paint: &mut BTreeMap::new(),
        diagnostics: &mut Vec::new(),
        engine,
    });
    engine.inline_sizes = sizes;
    engine.inline_extents = extents;
    engine.inline_floats = floats;
    engine.active_inline_boxes = boxes;
    engine.float_context = context;
    engine.column_context = columns;
    engine.float_block_offset = offset;
    engine.float_context_width = context_width;
    engine.float_inline_insets = insets;
    layout
}

#[derive(Clone, Copy, Default)]
pub(super) struct MarginStrut {
    positive: f64,
    negative: f64,
}
impl MarginStrut {
    pub(super) fn append(mut self, value: f64) -> Self {
        self.positive = self.positive.max(value);
        self.negative = self.negative.min(value);
        self
    }
    pub(super) fn merge(self, other: Self) -> Self {
        Self {
            positive: self.positive.max(other.positive),
            negative: self.negative.min(other.negative),
        }
    }
    pub(super) fn value(self) -> f64 {
        self.positive + self.negative
    }
}

pub(super) fn layout_flow(params: InlineFlowInputs<'_>) -> (Paragraph, TextLayout, bool) {
    let InlineFlowInputs {
        flow,
        style,
        inline,
        full_inline,
        block_available,
        block_limit,
        font_size,
        nodes,
        styles,
        input,
        geometry,
        offsets,
        resolved,
        paint,
        diagnostics,
        engine,
    } = params;

    let axes = writing_mode::Axes::new(
        style_value(style, "writing-mode", "horizontal-tb"),
        style_value(style, "direction", "ltr") == "rtl",
    );
    let mut context = Paragraph::default();
    let mut combined = TextLayout::default();
    let mut cursor = 0.;
    let mut byte_offset = 0;
    let mut has_blocks = false;
    let mut preceding_margin: Option<MarginStrut> = None;
    let float_origin = engine.float_block_offset;
    for item in flow {
        engine.float_block_offset = float_origin + cursor;
        match item {
            FlowItem::Block(node) => {
                has_blocks = true;
                let compatible = style_value(&styles[&node], "writing-mode", "horizontal-tb")
                    == style_value(style, "writing-mode", "horizontal-tb");
                if !compatible {
                    preceding_margin = None;
                }
                let spanning = engine
                    .column_context
                    .is_some_and(|(root, _)| columns::eligible_spanner(root, node, nodes, styles));
                if spanning
                    && let Some(bottom) = engine
                        .float_context
                        .as_ref()
                        .and_then(|context| context.cleared_threshold(taffy::Clear::Both))
                {
                    cursor = cursor.max(f64::from(bottom) - float_origin);
                    engine.float_block_offset = float_origin + cursor;
                }
                let incoming_margin;
                {
                    let top = if axes.vertical {
                        margin_collapse::vertical_edge(
                            crate::margin_collapse::VerticalMarginEdgeInputs {
                                id: node,
                                after: false,
                                reverse: axes.x_reverse,
                                basis: inline,
                                nodes,
                                styles,
                                viewport: input.viewport,
                                obstacles: margin_collapse::clear_obstacles(
                                    engine.float_context.as_ref(),
                                    engine.float_block_offset,
                                ),
                            },
                        )
                    } else {
                        margin_collapse::outer_edge(
                            node,
                            false,
                            inline,
                            margin_collapse::clear_obstacles(
                                engine.float_context.as_ref(),
                                engine.float_block_offset,
                            ),
                            margin_collapse::QueryContext {
                                nodes,
                                styles,
                                viewport: input.viewport,
                                cache: Some(&engine.margin_suffixes),
                            },
                        )
                    };
                    let previous = preceding_margin.take().unwrap_or_default();
                    incoming_margin = previous.merge(top);
                    cursor += incoming_margin.value() - previous.value() - top.value();
                    engine.float_block_offset = float_origin + cursor;
                }
                engine.clearance_nodes.remove(&node);
                {
                    let child_style = &styles[&node];
                    let clear = match style_value(child_style, "clear", "none") {
                        "left" => taffy::Clear::Left,
                        "right" => taffy::Clear::Right,
                        "both" => taffy::Clear::Both,
                        _ => taffy::Clear::None,
                    };
                    if let Some(bottom) = engine
                        .float_context
                        .as_ref()
                        .and_then(|context| context.cleared_threshold(clear))
                    {
                        let font = length(
                            style_value(child_style, "font-size", "16px"),
                            font_size,
                            font_size,
                            input.viewport,
                        );
                        let margin = edge(child_style, "margin", inline, font, input.viewport);
                        let cleared = f64::from(bottom)
                            - float_origin
                            - if axes.vertical {
                                if axes.x_reverse {
                                    margin.right
                                } else {
                                    margin.left
                                }
                            } else {
                                margin.top
                            };
                        if cleared > cursor + 0.0001 {
                            engine.clearance_nodes.insert(node);
                        }
                        cursor = cursor.max(cleared);
                        engine.float_block_offset = float_origin + cursor;
                    }
                }
                let used_inline = if spanning {
                    engine
                        .column_context
                        .map(|(_, width)| width)
                        .unwrap_or(full_inline)
                } else {
                    inline
                };
                // Keep the real containing width for percentages. Only the auto-width
                // available space is reduced by floats; margins overlap exclusion edges
                // according to the native BFC slot algorithm.
                let child_style = &styles[&node];
                let font = length(
                    style_value(child_style, "font-size", "16px"),
                    font_size,
                    font_size,
                    input.viewport,
                );
                let margins = edge(child_style, "margin", used_inline, font, input.viewport);
                let inline_margins = if axes.vertical {
                    [margins.top, margins.bottom]
                } else {
                    [margins.left, margins.right]
                };
                let block_margin = if axes.vertical {
                    if axes.x_reverse {
                        margins.right
                    } else {
                        margins.left
                    }
                } else {
                    margins.top
                };
                let direction = if style_value(style, "direction", "ltr") == "rtl" {
                    taffy::Direction::Rtl
                } else {
                    taffy::Direction::Ltr
                };
                let insets = engine.float_inline_insets.map(|value| value as f32);
                let avoids = !spanning
                    && establishes_bfc(child_style)
                    && engine
                        .float_context
                        .as_ref()
                        .is_some_and(|context| context.has_floats());
                let mut slot = if avoids {
                    engine.float_context.as_ref().map(|context| {
                        context.find_bfc_slot(
                            (float_origin + cursor + block_margin) as f32,
                            insets,
                            inline_margins.map(|value| value as f32),
                            direction,
                            taffy::Clear::None,
                            None,
                        )
                    })
                } else {
                    None
                };
                let previous_available = engine.bfc_available_width;
                let mut block_x = 0.;
                loop {
                    if let Some(current) = slot {
                        cursor = cursor.max(f64::from(current.y) - float_origin - block_margin);
                        engine.float_block_offset = float_origin + cursor;
                        engine.bfc_available_width =
                            Some((node, f64::from(current.stretch_width).max(0.)));
                        block_x = f64::from(current.x) - f64::from(insets[0]) - inline_margins[0];
                    }
                    let deferred_start = engine.out_of_flow.len();
                    layout_node(
                        node,
                        FlowBox {
                            x: 0.,
                            y: 0.,
                            width: if axes.vertical {
                                block_available
                            } else {
                                used_inline
                            },
                            height: if axes.vertical {
                                Some(used_inline)
                            } else {
                                block_limit
                            },
                        },
                        nodes,
                        styles,
                        geometry,
                        offsets,
                        resolved,
                        paint,
                        input,
                        false,
                        diagnostics,
                        engine,
                        None,
                    );
                    let fits = slot.is_none_or(|current| {
                        current.segment_id.is_none()
                            || geometry
                                .get(&node)
                                .and_then(|value| value.box_model.as_ref())
                                .is_none_or(|model| {
                                    (if axes.vertical {
                                        model.border_box.height
                                    } else {
                                        model.border_box.width
                                    }) <= f64::from(current.border_width) + 0.0001
                                })
                    });
                    let collision = slot.and_then(|current| {
                        geometry
                            .get(&node)
                            .and_then(|value| value.box_model.as_ref())
                            .and_then(|model| {
                                float_lines::intersecting_bottom(
                                    engine.float_context.as_ref().unwrap(),
                                    engine.float_context_width,
                                    CssRect {
                                        x: f64::from(current.x)
                                            + if axes.vertical {
                                                model.margin.top
                                            } else {
                                                model.margin.left
                                            }
                                            - inline_margins[0],
                                        y: f64::from(current.y),
                                        width: if axes.vertical {
                                            model.border_box.height
                                        } else {
                                            model.border_box.width
                                        },
                                        height: if axes.vertical {
                                            model.border_box.width
                                        } else {
                                            model.border_box.height
                                        },
                                    },
                                )
                            })
                    });
                    if fits && collision.is_none() {
                        break;
                    }
                    let current = slot.unwrap();
                    let (next_y, after) = if let Some(bottom) = collision {
                        (bottom as f32, None)
                    } else {
                        (current.y, current.segment_id)
                    };
                    let next = engine.float_context.as_ref().unwrap().find_bfc_slot(
                        next_y,
                        insets,
                        inline_margins.map(|value| value as f32),
                        direction,
                        taffy::Clear::None,
                        after,
                    );
                    engine.out_of_flow.truncate(deferred_start);
                    slot = Some(next);
                }
                engine.bfc_available_width = previous_available;
                if let Some(model) = geometry
                    .get(&node)
                    .and_then(|value| value.box_model.as_ref())
                {
                    let absorbed = engine
                        .absorbed_margins
                        .get(&node)
                        .copied()
                        .unwrap_or([0.; 2]);
                    let absorbed_vertical = engine
                        .absorbed_vertical_margins
                        .get(&node)
                        .copied()
                        .unwrap_or([0.; 2]);
                    let width = model.border_box.width + model.margin.left + model.margin.right
                        - absorbed_vertical[0]
                        - absorbed_vertical[1];
                    let height = model.border_box.height + model.margin.top + model.margin.bottom
                        - absorbed[0]
                        - absorbed[1];
                    let block = if axes.vertical { width } else { height };
                    let advance = if axes.vertical { height } else { width };
                    combined.inline_boxes.push(InlinePlacement {
                        node,
                        rect: CssRect {
                            x: block_x,
                            y: cursor,
                            width: advance,
                            height: block,
                        },
                        line: usize::MAX,
                    });
                    cursor += block;
                    combined.width = combined.width.max(block_x + advance);
                    {
                        let after = if axes.vertical {
                            if axes.x_reverse {
                                model.margin.left
                            } else {
                                model.margin.right
                            }
                        } else {
                            model.margin.bottom
                        };
                        let empty = if axes.vertical {
                            model.border_box.width == 0.
                                && model.border.left == 0.
                                && model.border.right == 0.
                                && model.padding.left == 0.
                                && model.padding.right == 0.
                        } else {
                            model.border_box.height == 0.
                                && model.border.top == 0.
                                && model.border.bottom == 0.
                                && model.padding.top == 0.
                                && model.padding.bottom == 0.
                        };
                        let through = compatible
                            && absorbed == [0.; 2]
                            && absorbed_vertical == [0.; 2]
                            && empty
                            && !establishes_bfc(child_style)
                            && !engine.clearance_nodes.contains(&node);
                        if through {
                            let joined = incoming_margin.append(after);
                            cursor += joined.value() - incoming_margin.value() - after;
                            preceding_margin = Some(joined);
                        } else {
                            preceding_margin = Some(
                                if axes.vertical
                                    && absorbed_vertical[if axes.x_reverse { 0 } else { 1 }] != 0.
                                {
                                    MarginStrut::default()
                                } else if axes.vertical {
                                    MarginStrut::default().append(after)
                                } else if absorbed[1] != 0. {
                                    MarginStrut::default()
                                } else {
                                    MarginStrut::default().append(model.margin.bottom)
                                },
                            );
                        }
                    }
                }
                if !compatible {
                    preceding_margin = None;
                }
                if spanning {
                    let mut next = taffy::compute::FloatContext::new();
                    next.set_width(engine.float_context_width as f32);
                    engine.float_context = Some(next);
                    let height = engine.column_context.and_then(|(root, _)| {
                        let index = engine.column_float_section_index.entry(root).or_default();
                        *index += 1;
                        engine
                            .column_float_section_heights
                            .get(&root)
                            .and_then(|heights| heights.get(*index))
                            .copied()
                    });
                    engine.float_fragment = engine
                        .float_fragment
                        .map(|(previous, _)| (height.unwrap_or(previous), float_origin + cursor));
                    preceding_margin = None;
                }
                context.atomic.insert(node);
            }
            FlowItem::Paragraph(paragraph) => {
                for (boundary, (owner, start)) in &paragraph.boundaries {
                    let owner_style = &styles[owner];
                    let size = length(
                        style_value(owner_style, "font-size", "16px"),
                        font_size,
                        font_size,
                        input.viewport,
                    );
                    let margin = edge(owner_style, "margin", inline, size, input.viewport);
                    let padding = edge(owner_style, "padding", inline, size, input.viewport);
                    let border = border_edge(owner_style, inline, size, input.viewport);
                    let reversed = if axes.vertical {
                        axes.y_reverse
                    } else {
                        axes.x_reverse
                    };
                    let start = *start != reversed;
                    let advance = if style_value(owner_style, "display", "inline") == "contents" {
                        0.
                    } else if axes.vertical {
                        if start {
                            margin.top + padding.top + border.top
                        } else {
                            margin.bottom + padding.bottom + border.bottom
                        }
                    } else if start {
                        margin.left + padding.left + border.left
                    } else {
                        margin.right + padding.right + border.right
                    };
                    engine
                        .inline_sizes
                        .insert(*boundary, (advance, 0., parley::InlineBoxKind::InFlow));
                }
                for child in &paragraph.atomic {
                    layout_node(
                        *child,
                        FlowBox {
                            x: 0.,
                            y: 0.,
                            width: if axes.vertical {
                                block_available
                            } else {
                                inline
                            },
                            height: if axes.vertical {
                                Some(inline)
                            } else {
                                block_limit
                            },
                        },
                        nodes,
                        styles,
                        geometry,
                        offsets,
                        resolved,
                        paint,
                        input,
                        false,
                        diagnostics,
                        engine,
                        None,
                    );
                    let dimensions = geometry
                        .get(child)
                        .and_then(|value| value.box_model.as_ref())
                        .map(|model| {
                            (
                                model.border_box.width + model.margin.left + model.margin.right,
                                model.border_box.height + model.margin.top + model.margin.bottom,
                            )
                        })
                        .unwrap_or((0., 0.));
                    let out_of_flow = styles.get(child).is_some_and(|style| {
                        matches!(
                            style_value(style, "position", "static"),
                            "absolute" | "fixed"
                        )
                    });
                    let above = if axes.vertical {
                        dimensions.0
                    } else {
                        engine
                            .ruby_baselines
                            .get(child)
                            .copied()
                            .unwrap_or_else(|| {
                                atomic_baseline(
                                    *child,
                                    dimensions.1,
                                    nodes,
                                    styles,
                                    geometry,
                                    paint,
                                )
                            })
                    };
                    engine.inline_extents.insert(
                        *child,
                        if axes.vertical {
                            dimensions.0
                        } else {
                            dimensions.1
                        },
                    );
                    let floating = style_value(&styles[child], "float", "none");
                    let kind = if floating != "none" {
                        let clear = match style_value(&styles[child], "clear", "none") {
                            "left" => taffy::Clear::Left,
                            "right" => taffy::Clear::Right,
                            "both" => taffy::Clear::Both,
                            _ => taffy::Clear::None,
                        };
                        engine.inline_floats.insert(
                            *child,
                            float_lines::FloatSpec {
                                width: if axes.vertical {
                                    dimensions.1
                                } else {
                                    dimensions.0
                                },
                                height: if axes.vertical {
                                    dimensions.0
                                } else {
                                    dimensions.1
                                },
                                right: floating == "right",
                                clear,
                                monolithic: is_replaced_box(nodes[child])
                                    || matches!(
                                        style_value(&styles[child], "display", "block"),
                                        "inline-block" | "inline-flex" | "inline-grid" | "ruby"
                                    ),
                            },
                        );
                        parley::InlineBoxKind::CustomOutOfFlow
                    } else if out_of_flow {
                        parley::InlineBoxKind::OutOfFlow
                    } else {
                        parley::InlineBoxKind::InFlow
                    };
                    engine.inline_sizes.insert(
                        *child,
                        (
                            if axes.vertical {
                                dimensions.1
                            } else {
                                dimensions.0
                            },
                            above,
                            kind,
                        ),
                    );
                }
                let mut layout = if paragraph.atomic.is_empty()
                    && !paragraph_forms_line(&paragraph, styles, style, inline, input.viewport)
                {
                    context
                        .empty_positions
                        .extend(paragraph.inline_nodes.iter().map(|id| (*id, cursor)));
                    TextLayout::default()
                } else {
                    {
                        let mut paragraph_style = style.clone();
                        if !combined.lines.is_empty() || has_blocks {
                            paragraph_style.remove(text_pseudos::KEY);
                        }
                        engine.shape_source(
                            &paragraph.source,
                            inline.max(1.),
                            font_size,
                            &paragraph_style,
                            input.viewport,
                            styles,
                        )
                    }
                };
                for node in paragraph.atomic.iter().chain(paragraph.boundaries.keys()) {
                    engine.inline_sizes.remove(node);
                    engine.inline_extents.remove(node);
                    engine.inline_floats.remove(node);
                }
                let line_offset = combined.lines.len();
                let bytes = layout
                    .line_ranges
                    .last()
                    .map(|range| range.end)
                    .unwrap_or(0);
                for line in &mut layout.lines {
                    line.3 += cursor;
                }
                for range in &mut layout.line_ranges {
                    range.start += byte_offset;
                    range.end += byte_offset;
                }
                for sources in &mut layout.provenance {
                    for source in sources {
                        source.bytes.start += byte_offset;
                        source.bytes.end += byte_offset;
                        for range in &mut source.glyph_ranges {
                            range.start += byte_offset;
                            range.end += byte_offset;
                        }
                    }
                }
                for cluster in &mut layout.clusters {
                    cluster.line += line_offset;
                    cluster.bytes.start += byte_offset;
                    cluster.bytes.end += byte_offset;
                    cluster.rect.y += cursor;
                }
                for cluster in &mut layout.source_clusters {
                    cluster.line += line_offset;
                    cluster.rect.y += cursor;
                }
                for item in &mut layout.inline_boxes {
                    if item.line != usize::MAX {
                        item.line += line_offset;
                    }
                    item.rect.y += cursor;
                }
                for line_break in &mut layout.breaks {
                    line_break.line += line_offset;
                    line_break.before.y += cursor;
                    if let Some(after) = &mut line_break.after {
                        after.y += cursor;
                    }
                }
                if layout.height > 0. {
                    preceding_margin = None;
                }
                cursor += layout.height;
                byte_offset += bytes + 1;
                combined.width = combined.width.max(layout.width);
                combined.baselines.extend(layout.baselines);
                combined.line_heights.extend(layout.line_heights);
                combined.lines.extend(layout.lines);
                combined.line_ranges.extend(layout.line_ranges);
                combined.glyph_runs.extend(layout.glyph_runs);
                combined.provenance.extend(layout.provenance);
                combined.clusters.extend(layout.clusters);
                combined.resolved_text.extend(layout.resolved_text);
                combined.source_clusters.extend(layout.source_clusters);
                combined.inline_boxes.extend(layout.inline_boxes);
                combined.breaks.extend(layout.breaks);
                context.inline_nodes.extend(paragraph.inline_nodes);
                context.atomic.extend(paragraph.atomic);
                context.boundaries.extend(paragraph.boundaries);
            }
        }
    }
    engine.float_block_offset = float_origin;
    combined.height = cursor;
    (context, combined, has_blocks)
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    #[test]
    fn vertical_clusters_join_owner_fragment_without_losing_glyph_source() {
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
            let style = BTreeMap::from([
                ("writing-mode".into(), mode.into()),
                ("text-orientation".into(), "upright".into()),
                ("font-size".into(), "20px".into()),
            ]);
            let styles = BTreeMap::from([(2, Arc::new(style.clone()))]);
            let mut engine = TextEngine::new(&[]);
            let mut shaped =
                engine.shape_styled("ABC", 200., 20., &style, ViewportState::default());
            vertical_text::physicalize(&mut shaped, 100., 200., mode);
            let fragments = paint_fragments(&shaped, 2, 0., 0., &styles);
            assert_eq!(fragments[&2].len(), 1, "{mode}");
            let fragment = &fragments[&2][0];
            assert_eq!(fragment.text, "ABC");
            assert_eq!(fragment.clusters.len(), 3);
            let mut sources = fragment
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .filter_map(|glyph| glyph.source.clone())
                .collect::<Vec<_>>();
            sources.sort_by_key(|range| range.start);
            sources.dedup();
            assert_eq!(sources, vec![0..1, 1..2, 2..3], "{mode}");
            assert!(
                fragment.rect.height > fragment.rect.width,
                "{mode}: {:?}",
                fragment.rect
            );
        }
    }
    #[test]
    fn owner_source_gaps_and_bidi_runs_remain_distinct_fragments() {
        let style = BTreeMap::from([("font-size".into(), "20px".into())]);
        let styles = BTreeMap::from([(2, Arc::new(style.clone())), (3, Arc::new(style.clone()))]);
        let mut source = text_source::TextSource::default();
        source.append(2, Some(20), "A");
        source.append(3, Some(30), "B");
        source.append(2, Some(21), "C");
        let mut engine = TextEngine::new(&[]);
        let shaped = engine.shape_source(
            &source,
            200.,
            20.,
            &style,
            ViewportState::default(),
            &styles,
        );
        let fragments = paint_fragments(&shaped, 2, 0., 0., &styles);
        assert_eq!(
            fragments[&2]
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "C"]
        );
        assert_eq!(fragments[&3][0].text, "B");
        let bidi = engine.shape_styled("Aאב", 200., 20., &style, ViewportState::default());
        let fragments = paint_fragments(&bidi, 2, 0., 0., &styles);
        assert!(fragments[&2].len() >= 2);
    }
    #[test]
    fn joined_owner_keeps_each_native_font_run_and_size() {
        let style = BTreeMap::from([
            ("font-size".into(), "20px".into()),
            ("writing-mode".into(), "vertical-rl".into()),
            ("text-orientation".into(), "upright".into()),
        ]);
        let mut larger = style.clone();
        larger.insert("font-size".into(), "24px".into());
        let mut engine = TextEngine::new(&[]);
        let mut shaped = engine.shape_ranged(
            "ABC",
            200.,
            20.,
            &style,
            ViewportState::default(),
            &[(0..1, &style, None, 2), (1..3, &larger, None, 2)],
        );
        let fonts = shaped
            .glyph_runs
            .iter()
            .flatten()
            .map(|run| (run.font_key.clone(), run.font_size, run.glyphs.len()))
            .collect::<Vec<_>>();
        vertical_text::physicalize(&mut shaped, 100., 200., "vertical-rl");
        let fragments =
            paint_fragments(&shaped, 2, 0., 0., &BTreeMap::from([(2, Arc::new(style))]));
        assert_eq!(fragments[&2].len(), 1);
        assert_eq!(fragments[&2][0].text, "ABC");
        let actual = fragments[&2][0]
            .glyph_runs
            .iter()
            .map(|run| (run.font_key.clone(), run.font_size, run.glyphs.len()))
            .collect::<Vec<_>>();
        assert_eq!(actual, fonts);
        assert!(actual.iter().any(|run| run.1 == 20.));
        assert!(actual.iter().any(|run| run.1 == 24.));
    }
    #[test]
    fn adjoining_margin_struts_keep_extrema_until_resolved() {
        let strut = MarginStrut::default()
            .append(30.)
            .append(-20.)
            .append(10.)
            .append(25.);
        assert_eq!(strut.value(), 10.);
    }

    #[test]
    fn slice_suppresses_only_broken_inline_edges_and_clone_keeps_them() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 30.,
            height: 10.,
        };
        let edges = EdgeSizes {
            top: 1.,
            right: 2.,
            bottom: 3.,
            left: 4.,
        };
        let model = BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            border: edges,
            padding: edges,
            margin: EdgeSizes::default(),
        };
        let rects = [rect, CssRect { y: 20., ..rect }];
        let mut style = BTreeMap::new();
        let fragments = fragment_models(&rects, &model, &style);
        assert_eq!(fragments[0].border.right, 0.);
        assert_eq!(fragments[1].border.left, 0.);
        assert_eq!(fragments[1].border.bottom, 3.);
        style.insert("box-decoration-break".into(), "clone".into());
        assert!(
            fragment_models(&rects, &model, &style)
                .iter()
                .all(|model| model.border == edges)
        );
    }
}

pub(super) fn atomic_baseline(
    root: u64,
    height: f64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &BTreeMap<u64, NodeGeometry>,
    paint: &BTreeMap<u64, Vec<PaintTextFragment>>,
) -> f64 {
    let (Some(node), Some(style), Some(model)) = (
        nodes.get(&root),
        styles.get(&root),
        geometry
            .get(&root)
            .and_then(|value| value.box_model.as_ref()),
    ) else {
        return height;
    };
    if is_replaced_box(node) || style_value(style, "overflow-y", "visible") != "visible" {
        return height;
    }
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut last = None::<(f64, f64)>;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if id != root
            && styles.get(&id).is_some_and(|style| {
                matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                )
            })
        {
            continue;
        }
        if let Some(node) = nodes.get(&id) {
            pending.extend(node.children.iter().copied());
        }
        if let Some(fragments) = paint.get(&id) {
            for fragment in fragments {
                for run in &fragment.glyph_runs {
                    if run.vertical {
                        continue;
                    }
                    if let Some(glyph) = run.glyphs.first() {
                        let baseline = fragment.rect.y + glyph.y;
                        if last.is_none_or(|(top, _)| fragment.rect.y >= top) {
                            last = Some((fragment.rect.y, baseline));
                        }
                    }
                }
            }
        }
    }
    last.map(|(_, baseline)| (baseline - model.border_box.y + model.margin.top).max(0.))
        .unwrap_or(height)
}
/// Parley inline boxes end at the baseline. Preserve the independently laid-out
/// box's below-baseline extent and grow the actual line descent before placing
/// subsequent lines; this is not a second text shaping pass.
pub(super) fn resolve_baselines(layout: &mut TextLayout, extents: &BTreeMap<u64, f64>) {
    if extents.is_empty() || layout.inline_boxes.is_empty() {
        return;
    }
    let mut additions = vec![0f64; layout.lines.len()];
    for item in &mut layout.inline_boxes {
        if let (Some(actual), Some(line)) = (extents.get(&item.node), layout.lines.get(item.line)) {
            additions[item.line] =
                additions[item.line].max((item.rect.y + actual - line.3 - line.2).max(0.));
            item.rect.height = *actual;
        }
    }
    let mut shifts = vec![0.; layout.lines.len()];
    let mut total = 0.;
    for (index, line) in layout.lines.iter_mut().enumerate() {
        shifts[index] = total;
        line.3 += total;
        line.2 += additions[index];
        if let Some(height) = layout.line_heights.get_mut(index) {
            *height += additions[index];
        }
        total += additions[index];
        if let Some(runs) = layout.glyph_runs.get_mut(index) {
            for run in runs {
                if let Some(rect) = &mut run.decoration_rect {
                    rect.height += additions[index];
                }
            }
        }
    }
    for cluster in &mut layout.clusters {
        if let Some(shift) = shifts.get(cluster.line) {
            cluster.rect.y += shift;
            cluster.rect.height += additions[cluster.line];
        }
    }
    for item in &mut layout.inline_boxes {
        if let Some(shift) = shifts.get(item.line) {
            item.rect.y += shift;
        }
    }
    for line_break in &mut layout.breaks {
        if let Some(shift) = shifts.get(line_break.line) {
            line_break.before.y += shift;
            line_break.before.height += additions[line_break.line];
        }
        if let Some(after) = &mut line_break.after
            && let Some(shift) = shifts.get(line_break.line + 1)
        {
            after.y += shift;
            after.height += additions[line_break.line + 1];
        }
    }
    layout.height += total;
}
