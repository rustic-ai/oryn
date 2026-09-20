//! Native ruby base/annotation pairing and interlinear geometry. All text uses
//! the same captured fonts, shaping, UTF16 provenance and physical glyph paths
//! as ordinary paragraphs.
use super::*;
struct Unit {
    base: text_source::TextSource,
    annotations: Vec<(u64, text_source::TextSource)>,
    spacing: bool,
}
fn block_bounds(layout: &TextLayout) -> (f64, f64) {
    // Ruby reserves typographic extents, including negative leading, without
    // changing the underlying text's CSS line advance or selection geometry.
    let mut minimum = 0f64;
    let mut maximum = layout.height;
    for cluster in &layout.clusters {
        minimum = minimum.min(cluster.rect.y);
        maximum = maximum.max(cluster.rect.bottom());
    }
    (minimum, maximum)
}
fn source(
    node: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> text_source::TextSource {
    text_source::TextSource::collect_visible(node, nodes, true, Some(styles))
}
fn pair(
    units: &mut Vec<Unit>,
    cursor: &mut usize,
    owner: u64,
    annotation: text_source::TextSource,
) {
    while *cursor < units.len() && units[*cursor].spacing {
        *cursor += 1;
    }
    if *cursor >= units.len() {
        units.push(Unit {
            base: text_source::TextSource::default(),
            annotations: Vec::new(),
            spacing: false,
        });
    }
    units[*cursor].annotations.push((owner, annotation));
    *cursor += 1;
}
fn units(
    root: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Vec<Unit> {
    let content = root.content.clone().unwrap_or_else(|| {
        let mut content = vec![LayoutContent::StyledText {
            owner: root.id,
            node_id: None,
            text: root.text.clone(),
        }];
        content.extend(root.children.iter().map(|id| LayoutContent::Element(*id)));
        content
    });
    let mut units = Vec::new();
    let mut pending = text_source::TextSource::default();
    let mut cursor = 0;
    let flush = |pending: &mut text_source::TextSource, units: &mut Vec<Unit>| {
        if pending.spans.is_empty() {
            return;
        }
        let spacing = pending.text.chars().all(char::is_whitespace);
        if !spacing || !units.is_empty() {
            units.push(Unit {
                base: std::mem::take(pending),
                annotations: Vec::new(),
                spacing,
            });
        } else {
            *pending = text_source::TextSource::default();
        }
    };
    let mut containers = Vec::new();
    for item in content {
        match item {
            LayoutContent::Text { node_id, text } => pending.append(root.id, Some(node_id), &text),
            LayoutContent::StyledText {
                owner,
                node_id,
                text,
            } => pending.append(owner, node_id, &text),
            LayoutContent::Element(id) => {
                let (Some(node), Some(style)) = (nodes.get(&id), styles.get(&id)) else {
                    continue;
                };
                let display = style_value(style, "display", "inline");
                if display == "none" || node.tag == "rp" {
                    continue;
                }
                if display == "ruby-text" || node.tag == "rt" {
                    flush(&mut pending, &mut units);
                    pair(&mut units, &mut cursor, id, source(node, nodes, styles));
                } else if display == "ruby-text-container" || node.tag == "rtc" {
                    flush(&mut pending, &mut units);
                    containers.push(id);
                } else {
                    flush(&mut pending, &mut units);
                    units.push(Unit {
                        base: source(node, nodes, styles),
                        annotations: Vec::new(),
                        spacing: false,
                    });
                }
            }
        }
    }
    flush(&mut pending, &mut units);
    if units.last().is_some_and(|unit| unit.spacing) {
        units.pop();
    }
    for container in containers {
        let mut cursor = 0;
        for child in &nodes[&container].children {
            if let Some(node) = nodes.get(child)
                && styles
                    .get(child)
                    .is_some_and(|style| style_value(style, "display", "none") != "none")
            {
                pair(&mut units, &mut cursor, *child, source(node, nodes, styles));
            }
        }
    }
    units
}
fn append(
    target: &mut TextLayout,
    mut layout: TextLayout,
    inline: f64,
    block: f64,
    byte_offset: &mut usize,
) {
    let lines = target.lines.len();
    let bytes = layout
        .line_ranges
        .last()
        .map(|range| range.end)
        .unwrap_or(0);
    for line in &mut layout.lines {
        line.4 += inline;
        line.3 += block;
    }
    for range in &mut layout.line_ranges {
        range.start += *byte_offset;
        range.end += *byte_offset;
    }
    for sources in &mut layout.provenance {
        for source in sources {
            source.bytes.start += *byte_offset;
            source.bytes.end += *byte_offset;
            for range in &mut source.glyph_ranges {
                range.start += *byte_offset;
                range.end += *byte_offset;
            }
        }
    }
    for cluster in &mut layout.clusters {
        cluster.line += lines;
        cluster.bytes.start += *byte_offset;
        cluster.bytes.end += *byte_offset;
        cluster.rect.x += inline;
        cluster.rect.y += block;
    }
    for cluster in &mut layout.source_clusters {
        cluster.line += lines;
        cluster.rect.x += inline;
        cluster.rect.y += block;
    }
    for item in &mut layout.inline_boxes {
        if item.line != usize::MAX {
            item.line += lines;
        }
        item.rect.x += inline;
        item.rect.y += block;
    }
    for line_break in &mut layout.breaks {
        line_break.line += lines;
        line_break.before.x += inline;
        line_break.before.y += block;
        if let Some(after) = &mut line_break.after {
            after.x += inline;
            after.y += block;
        }
    }
    target.baselines.extend(layout.baselines);
    target.line_heights.extend(layout.line_heights);
    target.lines.extend(layout.lines);
    target.line_ranges.extend(layout.line_ranges);
    target.provenance.extend(layout.provenance);
    target.glyph_runs.extend(layout.glyph_runs);
    target.clusters.extend(layout.clusters);
    target.resolved_text.extend(layout.resolved_text);
    target.source_clusters.extend(layout.source_clusters);
    target.inline_boxes.extend(layout.inline_boxes);
    target.breaks.extend(layout.breaks);
    *byte_offset += bytes + 1;
}
pub(super) fn layout(
    root: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    font: f64,
    viewport: ViewportState,
    engine: &mut TextEngine,
) -> (inline_formatting::Paragraph, TextLayout, f64) {
    let style = &styles[&root.id];
    let mut measured = Vec::new();
    let mut above = 0f64;
    let mut below = 0f64;
    let mut ascent = 0f64;
    let mut descent = 0f64;
    let mut context = inline_formatting::Paragraph::default();
    let mut pending = root.children.clone();
    while let Some(id) = pending.pop() {
        if !context.inline_nodes.insert(id) {
            continue;
        }
        if let Some(node) = nodes.get(&id) {
            pending.extend(node.children.iter().copied());
        }
    }
    for unit in units(root, nodes, styles) {
        let mut base_style = style.as_ref().clone();
        base_style.insert("text-wrap-mode".into(), "nowrap".into());
        base_style.insert("text-indent".into(), "0px".into());
        base_style.insert("text-align".into(), "left".into());
        let mut base_source = unit.base;
        if unit.spacing {
            base_style.insert("white-space".into(), "pre".into());
            base_style.insert("white-space-collapse".into(), "preserve".into());
            if let Some(span) = base_source.spans.first_mut() {
                base_source.text = " ".into();
                span.bytes = 0..1;
                span.transformed = true;
                base_source.spans.truncate(1);
            }
        }
        let base = if unit.spacing {
            let mut layout = engine.shape_styled(" ", 1e9, font, &base_style, viewport);
            layout.source_clusters = base_source.map_clusters(&layout.clusters);
            layout.resolved_text =
                vec![base_source.resolved_run(0, &layout.clusters, styles, &base_style)];
            layout
        } else {
            engine.shape_source(&base_source, 1e9, font, &base_style, viewport, styles)
        };
        let baseline = base
            .baselines
            .first()
            .zip(base.lines.first())
            .map(|(baseline, line)| baseline + line.3)
            .unwrap_or(base.height / 2.);
        let (base_top, base_bottom) = block_bounds(&base);
        ascent = ascent.max(baseline - base_top);
        descent = descent.max((base_bottom - baseline).max(0.));
        let mut annotations = Vec::new();
        let mut top = 0.;
        let mut bottom = 0.;
        let mut width = base.width;
        for (owner, source) in unit.annotations {
            let annotation_style = &styles[&owner];
            let mut local = annotation_style.as_ref().clone();
            local.insert("text-wrap-mode".into(), "nowrap".into());
            local.insert("text-indent".into(), "0px".into());
            local.insert("text-align".into(), "left".into());
            let size = length(
                style_value(&local, "font-size", "8px"),
                font / 2.,
                font / 2.,
                viewport,
            );
            let annotation = engine.shape_source(&source, 1e9, size, &local, viewport, styles);
            let under = style_value(
                annotation_style,
                "ruby-position",
                style_value(style, "ruby-position", "over"),
            )
            .contains("under");
            let (annotation_top, annotation_bottom) = block_bounds(&annotation);
            let extent = annotation_bottom - annotation_top;
            if under {
                bottom += extent
            } else {
                top += extent
            }
            width = width.max(annotation.width);
            annotations.push((
                annotation,
                under,
                style_value(annotation_style, "ruby-align", "space-around").to_string(),
            ));
        }
        above = above.max(top);
        below = below.max(bottom);
        measured.push((base, baseline, annotations, width));
    }
    let baseline = above + ascent;
    let mut output = TextLayout::default();
    let mut cursor = 0.;
    let mut bytes = 0;
    for (mut base, base_baseline, annotations, width) in measured {
        align(
            &mut base,
            width,
            style_value(style, "ruby-align", "space-around"),
        );
        append(
            &mut output,
            base,
            cursor,
            baseline - base_baseline,
            &mut bytes,
        );
        let mut over = above;
        let mut under = baseline + descent;
        for (mut annotation, is_under, alignment) in annotations {
            align(&mut annotation, width, &alignment);
            let (top, bottom) = block_bounds(&annotation);
            let extent = bottom - top;
            let block = if is_under {
                let at = under - top;
                under += extent;
                at
            } else {
                over -= extent;
                over - top
            };
            append(&mut output, annotation, cursor, block, &mut bytes);
        }
        cursor += width;
    }
    output.width = cursor;
    output.height = above + ascent + descent + below;
    (context, output, baseline)
}

/// Distribute typographic units using native cluster byte provenance, retaining
/// ligatures and combining sequences as indivisible spacing units.
fn align(layout: &mut TextLayout, width: f64, mode: &str) {
    let free = (width - layout.width).max(0.);
    if free <= 0. {
        return;
    }
    for line_index in 0..layout.lines.len() {
        let mut indices = layout
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, cluster)| cluster.line == line_index && !cluster.bytes.is_empty())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        indices.sort_by(|a, b| {
            layout.clusters[*a]
                .rect
                .x
                .total_cmp(&layout.clusters[*b].rect.x)
        });
        let count = indices.len();
        let mut shifts = Vec::new();
        for (rank, index) in indices.into_iter().enumerate() {
            let shift = match mode {
                "start" => 0.,
                "space-between" if count > 1 => free * rank as f64 / (count - 1) as f64,
                "space-around" if count > 1 => free * (rank as f64 + 0.5) / count as f64,
                _ => free / 2.,
            };
            let cluster = &mut layout.clusters[index];
            shifts.push((cluster.bytes.clone(), cluster.rect, shift));
            cluster.rect.x += shift;
        }
        for source in &mut layout.source_clusters {
            if source.line == line_index
                && let Some((_, _, shift)) = shifts.iter().find(|(_, rect, _)| *rect == source.rect)
            {
                source.rect.x += shift;
            }
        }
        if let (Some(runs), Some(sources)) = (
            layout.glyph_runs.get_mut(line_index),
            layout.provenance.get(line_index),
        ) {
            for (run, source) in runs.iter_mut().zip(sources) {
                let fallback = shifts
                    .iter()
                    .find(|(range, _, _)| {
                        range.start >= source.bytes.start && range.end <= source.bytes.end
                    })
                    .map(|(_, _, shift)| *shift)
                    .unwrap_or(free / 2.);
                let mut minimum = f64::INFINITY;
                let mut maximum = f64::NEG_INFINITY;
                for (index, glyph) in run.glyphs.iter_mut().enumerate() {
                    let shift = source
                        .glyph_ranges
                        .get(index)
                        .and_then(|range| {
                            shifts.iter().find(|(native, _, _)| {
                                native.start <= range.start && native.end >= range.end
                            })
                        })
                        .map(|(_, _, shift)| *shift)
                        .unwrap_or(fallback);
                    if run.vertical {
                        glyph.y += shift
                    } else {
                        glyph.x += shift
                    }
                    minimum = minimum.min(shift);
                    maximum = maximum.max(shift);
                }
                if let Some(rect) = &mut run.decoration_rect
                    && minimum.is_finite()
                {
                    if run.vertical {
                        rect.y += minimum;
                        rect.height += maximum - minimum;
                    } else {
                        rect.x += minimum;
                        rect.width += maximum - minimum;
                    }
                }
            }
        }
        layout.lines[line_index].1 = width;
    }
    layout.width = width;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typographic_bounds_include_negative_leading_without_changing_flow() {
        let mut layout = TextLayout {
            height: 8.,
            ..TextLayout::default()
        };
        layout.line_heights.push(8.);
        layout.clusters.push(text_source::TextCluster {
            bytes: 0..1,
            line: 0,
            rtl: false,
            rect: CssRect {
                x: 0.,
                y: -2.,
                width: 10.,
                height: 12.,
            },
        });
        assert_eq!(block_bounds(&layout), (-2., 10.));
        assert_eq!(layout.height, 8.);
        assert_eq!(layout.line_heights, vec![8.]);
    }
    #[test]
    fn append_preserves_flow_advances_independently_of_selection_height() {
        let mut target = TextLayout::default();
        let mut bytes = 0;
        for advance in [30., 60.] {
            let mut line = TextLayout::default();
            line.lines.push(("A".into(), 10., advance + 8., 0., 0.));
            line.line_ranges.push(0..1);
            line.line_heights.push(advance);
            line.baselines.push(advance * 0.8);
            append(&mut target, line, 0., 10., &mut bytes);
        }
        assert_eq!(target.line_heights, vec![30., 60.]);
        assert_eq!(target.line_heights.len(), target.lines.len());
        assert_eq!(
            target.lines.iter().map(|line| line.2).collect::<Vec<_>>(),
            vec![38., 68.]
        );
        assert_eq!(target.line_ranges, vec![0..1, 2..3]);
    }
}
