//! Column measure and line-fragment placement. Text is shaped at the used
//! column inline size before fragment selection, never sliced from screen paint.
use super::*;
#[derive(Clone, Copy, Debug)]
pub(super) struct Columns {
    pub count: usize,
    pub inline: f64,
    pub gap: f64,
    pub available: f64,
    pub limit: Option<f64>,
    pub balance: bool,
    pub reverse: bool,
}
#[derive(Clone, Copy)]
pub(super) struct ColumnRun {
    pub columns: Columns,
    pub count: usize,
    pub start: f64,
    pub block: f64,
}
impl Columns {
    pub fn resolve(
        style: &BTreeMap<String, String>,
        available: f64,
        limit: Option<f64>,
        font: f64,
        viewport: ViewportState,
    ) -> Result<Option<Self>, String> {
        let count = style_value(style, "column-count", "auto");
        let width = style_value(style, "column-width", "auto");
        if count == "auto" && width == "auto" {
            return Ok(None);
        }
        if !available.is_finite() || available < 0. {
            return Err("nonfinite multicol inline size".into());
        }
        let gap = if style_value(style, "column-gap", "normal") == "normal" {
            font
        } else {
            length(
                style_value(style, "column-gap", "normal"),
                available,
                font,
                viewport,
            )
            .max(0.)
        };
        let count = if count == "auto" {
            None
        } else {
            Some(
                count
                    .parse::<usize>()
                    .map_err(|_| "invalid computed column-count")?,
            )
        };
        if count.is_some_and(|count| count == 0 || count > 1024) {
            return Err("column-count exceeds 1..1024 layout budget".into());
        }
        let width = if width == "auto" {
            None
        } else {
            Some(length(width, available, font, viewport))
        };
        if !gap.is_finite() || width.is_some_and(|width| !width.is_finite() || width < 0.) {
            return Err("invalid computed column dimensions".into());
        }
        let fitting =
            width.map(|width| ((available + gap) / (width.max(1.) + gap)).floor().max(1.) as usize);
        let count = match (count, fitting) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => 1,
        };
        if count > 1024 {
            return Err("used column-count exceeds 1024 layout budget".into());
        }
        let inline = ((available - gap * (count - 1) as f64) / count as f64).max(0.);
        Ok(Some(Self {
            count,
            inline,
            gap,
            available,
            limit: limit.filter(|v| v.is_finite() && *v > 0.),
            balance: style_value(style, "column-fill", "balance") != "auto",
            reverse: style_value(style, "direction", "ltr") == "rtl",
        }))
    }
    fn ranges(&self, heights: &[f64], height: f64) -> Vec<std::ops::Range<usize>> {
        let mut output = Vec::new();
        let mut start = 0;
        let mut used = 0.;
        for (index, advance) in heights.iter().enumerate() {
            if index > start && used + advance > height + 1e-7 {
                output.push(start..index);
                start = index;
                used = 0.;
            }
            used += advance;
        }
        if start < heights.len() {
            output.push(start..heights.len());
        }
        output
    }
    pub fn place(&self, layout: &mut TextLayout) -> f64 {
        if layout.lines.is_empty() {
            return 0.;
        }
        // Selection/ink extents can exceed CSS line-height. Column capacity and
        // the next line's flow position use the actual formatting advance.
        let heights = layout
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| layout.line_heights.get(index).copied().unwrap_or(line.2))
            .collect::<Vec<_>>();
        let total = heights.iter().sum::<f64>();
        let tallest = heights.iter().copied().fold(0f64, f64::max);
        let target = if !self.balance && self.limit.is_some() {
            self.limit.unwrap()
        } else {
            let mut low = tallest.max(total / self.count as f64);
            let mut high = total.max(low);
            for _ in 0..48 {
                let middle = (low + high) / 2.;
                if self.ranges(&heights, middle).len() > self.count {
                    low = middle
                } else {
                    high = middle
                }
            }
            self.limit.map(|limit| high.min(limit)).unwrap_or(high)
        };
        let ranges = self.ranges(&heights, target);
        let mut shifts = vec![(0., 0.); layout.lines.len()];
        let mut block = 0f64;
        for (column, range) in ranges.iter().enumerate() {
            let inline = if self.reverse {
                self.available - self.inline - column as f64 * (self.inline + self.gap)
            } else {
                column as f64 * (self.inline + self.gap)
            };
            let mut top = 0.;
            for index in range.clone() {
                let line = &mut layout.lines[index];
                shifts[index] = (inline, top - line.3);
                line.4 += inline;
                line.3 = top;
                top += heights[index];
            }
            block = block.max(top);
        }
        for item in &mut layout.inline_boxes {
            if let Some((dx, dy)) = shifts.get(item.line) {
                item.rect.x += dx;
                item.rect.y += dy;
            }
        }
        for cluster in &mut layout.clusters {
            if let Some((dx, dy)) = shifts.get(cluster.line) {
                cluster.rect.x += dx;
                cluster.rect.y += dy;
            }
        }
        for cluster in &mut layout.source_clusters {
            if let Some((dx, dy)) = shifts.get(cluster.line) {
                cluster.rect.x += dx;
                cluster.rect.y += dy;
            }
        }
        for line_break in &mut layout.breaks {
            if let Some((dx, dy)) = shifts.get(line_break.line) {
                line_break.before.x += dx;
                line_break.before.y += dy;
            }
            if let Some(after) = &mut line_break.after {
                if let Some((dx, dy)) = shifts.get(line_break.line + 1) {
                    after.x += dx;
                    after.y += dy;
                }
            }
        }
        layout.width = self.available;
        layout.height = block;
        block
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_capacity_uses_flow_advance_without_trimming_selection_bounds() {
        let columns = Columns {
            count: 2,
            inline: 100.,
            gap: 20.,
            available: 220.,
            limit: Some(40.),
            balance: false,
            reverse: false,
        };
        let mut layout = TextLayout::default();
        layout.lines = (0..4)
            .map(|index| ("A".into(), 10., 24., index as f64 * 20., 0.))
            .collect();
        layout.line_heights = vec![20.; 4];
        layout.height = 80.;
        layout.width = 100.;
        assert_eq!(columns.place(&mut layout), 40.);
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| (line.3, line.4))
                .collect::<Vec<_>>(),
            vec![(0., 0.), (20., 0.), (0., 120.), (20., 120.)]
        );
        assert!(layout.lines.iter().all(|line| line.2 == 24.));
    }
    #[test]
    fn count_width_gap_resolution_and_balanced_native_lines() {
        let style = BTreeMap::from([
            ("column-count".into(), "3".into()),
            ("column-width".into(), "80px".into()),
            ("column-gap".into(), "20px".into()),
        ]);
        let columns = Columns::resolve(&style, 280., None, 16., ViewportState::default())
            .unwrap()
            .unwrap();
        assert_eq!(columns.count, 3);
        assert_eq!(columns.inline, 80.);
        let mut engine = TextEngine::new(&[]);
        let mut text = style.clone();
        text.insert("white-space".into(), "pre".into());
        let mut shaped = engine.shape_styled(
            "a\nb\nc\nd\ne\nf",
            columns.inline,
            16.,
            &text,
            ViewportState::default(),
        );
        let natural = shaped.height;
        columns.place(&mut shaped);
        assert!(shaped.height < natural / 2.);
        assert_eq!(shaped.lines[2].4, 100.);
        assert_eq!(shaped.lines[4].4, 200.);
        assert_eq!(shaped.lines[2].3, 0.);
    }
    #[test]
    fn fixed_height_auto_fill_overflows_in_inline_direction() {
        let style = BTreeMap::from([
            ("column-count".into(), "2".into()),
            ("column-gap".into(), "10px".into()),
            ("column-fill".into(), "auto".into()),
            ("direction".into(), "rtl".into()),
        ]);
        let columns = Columns::resolve(&style, 110., Some(10.), 16., ViewportState::default())
            .unwrap()
            .unwrap();
        let mut layout = TextLayout::default();
        layout.lines = (0..3).map(|_| ("a".into(), 5., 10., 0., 0.)).collect();
        columns.place(&mut layout);
        assert_eq!(
            layout.lines.iter().map(|line| line.4).collect::<Vec<_>>(),
            vec![60., 0., -60.]
        );
        assert_eq!(layout.height, 10.);
    }
}

#[derive(Clone, Copy)]
struct Axis {
    origin: (f64, f64),
    vertical: bool,
    block_reverse: bool,
    inline_reverse: bool,
    inline: f64,
    block: f64,
}
impl Axis {
    fn logical(self, rect: CssRect) -> CssRect {
        if !self.vertical {
            return CssRect {
                x: rect.x - self.origin.0,
                y: rect.y - self.origin.1,
                ..rect
            };
        }
        CssRect {
            x: if self.inline_reverse {
                self.inline - (rect.y - self.origin.1) - rect.height
            } else {
                rect.y - self.origin.1
            },
            y: if self.block_reverse {
                self.block - (rect.x - self.origin.0) - rect.width
            } else {
                rect.x - self.origin.0
            },
            width: rect.height,
            height: rect.width,
        }
    }
    fn physical(self, rect: CssRect, block: f64) -> CssRect {
        if !self.vertical {
            return CssRect {
                x: self.origin.0 + rect.x,
                y: self.origin.1 + rect.y,
                ..rect
            };
        }
        CssRect {
            x: self.origin.0
                + if self.block_reverse {
                    block - rect.y - rect.height
                } else {
                    rect.y
                },
            y: self.origin.1
                + if self.inline_reverse {
                    self.inline - rect.x - rect.width
                } else {
                    rect.x
                },
            width: rect.height,
            height: rect.width,
        }
    }
}
struct ParagraphBreaks {
    lines: Vec<(f64, f64)>,
    orphans: usize,
    widows: usize,
}
impl ParagraphBreaks {
    fn permits(&self, start: f64, at: f64) -> bool {
        if self.lines.is_empty()
            || at <= self.lines[0].0 + 1e-6
            || at >= self.lines.last().unwrap().1 - 1e-6
        {
            return true;
        }
        let first = self.lines.partition_point(|line| line.1 <= start + 1e-6);
        let before = self.lines.partition_point(|line| line.1 <= at + 1e-6);
        before.saturating_sub(first) >= self.orphans && self.lines.len() - before >= self.widows
    }
}
/// Fragment a freshly formatted, column-width logical flow. Break candidates
/// come from native line/block boundaries; glyphs are never cut out of a screen
/// bitmap and fragmented element boxes retain each physical client rectangle.
pub(super) fn fragment_flow(
    columns: Columns,
    root: u64,
    origin: (f64, f64),
    scroll: (f64, f64),
    block_extent: f64,
    mode: &str,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    offsets: &mut BTreeMap<u64, (f64, f64)>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    layout: &mut TextLayout,
    engine: &mut TextEngine,
) -> Result<f64, String> {
    let axis = Axis {
        origin,
        vertical: writing_mode::Axes::new(mode, false).vertical,
        block_reverse: matches!(mode, "vertical-rl" | "sideways-rl"),
        inline_reverse: mode == "sideways-lr",
        inline: columns.available,
        block: block_extent,
    };
    let total = if axis.vertical {
        layout.width
    } else {
        layout.height
    };
    if !total.is_finite() || total < 0. {
        return Err("nonfinite multicol flow extent".into());
    }
    if total == 0. {
        return Ok(0.);
    }
    let mut scope = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !scope.insert(id) {
            continue;
        }
        if let Some(node) = nodes.get(&id) {
            pending.extend(node.children.iter().copied());
        }
    }
    let excluded = |id: &u64| {
        let mut current = Some(*id);
        let mut seen = BTreeSet::new();
        while let Some(node) = current {
            if node == root || !seen.insert(node) {
                break;
            }
            if styles.get(&node).is_some_and(|style| {
                matches!(
                    style_value(style, "position", "static"),
                    "absolute" | "fixed"
                )
            }) {
                return true;
            }
            current = nodes.get(&node).and_then(|node| node.parent);
        }
        false
    };
    let mut candidates = vec![0., total];
    let mut forced = Vec::new();
    let mut avoid = Vec::new();
    let mut paragraphs = Vec::new();
    let monolithic = scope
        .iter()
        .filter(|id| {
            **id != root
                && !excluded(id)
                && styles.get(id).is_some_and(|style| {
                    is_replaced_box(nodes[id])
                        || matches!(
                            style_value(style, "display", "block"),
                            "inline-block" | "inline-flex" | "inline-grid" | "ruby" | "table-row"
                        )
                })
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let paragraph = |id: u64, mut lines: Vec<(f64, f64)>| {
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        lines.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6);
        let style = styles.get(&id);
        ParagraphBreaks {
            lines,
            orphans: style
                .map(|s| style_value(s, "orphans", "2"))
                .unwrap_or("2")
                .parse()
                .unwrap_or(2)
                .max(1),
            widows: style
                .map(|s| style_value(s, "widows", "2"))
                .unwrap_or("2")
                .parse()
                .unwrap_or(2)
                .max(1),
        }
    };
    let root_lines = layout
        .lines
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut flow = layout.flow_rect(index, mode);
            flow.x += origin.0;
            flow.y += origin.1;
            axis.logical(flow)
        })
        .collect::<Vec<_>>();
    for line in &root_lines {
        candidates.push(line.bottom());
    }
    paragraphs.push(paragraph(
        root,
        root_lines
            .iter()
            .map(|line| (line.y, line.bottom()))
            .collect(),
    ));
    let mut source_shifts = BTreeMap::<(u64, usize), (f64, f64)>::new();
    for id in &scope {
        if *id == root || excluded(id) {
            continue;
        }
        if let Some(model) = geometry.get(id).and_then(|value| value.box_model.as_ref()) {
            let rect = axis.logical(model.border_box);
            candidates.push(rect.y);
            candidates.push(rect.bottom());
            if let Some(style) = styles.get(id) {
                if matches!(
                    style_value(style, "break-before", "auto"),
                    "column" | "always"
                ) {
                    forced.push(rect.y);
                }
                if matches!(
                    style_value(style, "break-after", "auto"),
                    "column" | "always"
                ) {
                    forced.push(rect.bottom());
                }
                let atomic = monolithic.contains(id);
                if style_value(style, "float", "none") == "none"
                    && (atomic
                        || matches!(
                            style_value(style, "break-inside", "auto"),
                            "avoid" | "avoid-column"
                        ))
                {
                    avoid.push((rect.y, rect.bottom(), atomic));
                }
            }
        }
        if let Some(fragments) = text.get(id) {
            let lines = fragments
                .iter()
                .map(|fragment| axis.logical(fragment.flow_rect.unwrap_or(fragment.rect)))
                .collect::<Vec<_>>();
            for line in &lines {
                candidates.push(line.bottom());
            }
            paragraphs.push(paragraph(
                *id,
                lines
                    .into_iter()
                    .map(|line| (line.y, line.bottom()))
                    .collect(),
            ));
        }
    }
    candidates.retain(|at| at.is_finite() && *at >= 0. && *at <= total);
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    forced.retain(|at| at.is_finite() && *at > 0. && *at < total);
    forced.sort_by(f64::total_cmp);
    forced.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    // A spanning block ends the preceding column set and establishes a full
    // inline-size row. Each following set balances independently.
    let mut spans = scope
        .iter()
        .filter(|id| eligible_spanner(root, **id, nodes, styles) && !excluded(id))
        .filter_map(|id| {
            geometry
                .get(id)
                .and_then(|value| value.box_model.as_ref())
                .map(|model| {
                    let rect = axis.logical(model.border_box);
                    let margin_before = if axis.vertical {
                        if axis.block_reverse {
                            model.margin.right
                        } else {
                            model.margin.left
                        }
                    } else {
                        model.margin.top
                    };
                    let margin_after = if axis.vertical {
                        if axis.block_reverse {
                            model.margin.left
                        } else {
                            model.margin.right
                        }
                    } else {
                        model.margin.bottom
                    };
                    (
                        rect.y - margin_before,
                        rect.bottom() + margin_after,
                        rect.x
                            - if axis.vertical {
                                if axis.inline_reverse {
                                    model.margin.bottom
                                } else {
                                    model.margin.top
                                }
                            } else {
                                model.margin.left
                            },
                    )
                })
        })
        .collect::<Vec<_>>();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let float_height = engine.column_float_heights.get(&root).copied();
    let section_heights = engine
        .column_float_section_heights
        .get(&root)
        .cloned()
        .unwrap_or_default();
    let section_height = |index: usize| section_heights.get(index).copied().or(float_height);
    if float_height.is_some() {
        if spans.len() >= 1024 {
            return Err("float column flow exceeds 1024 column sets".into());
        }
        let mut begin = 0.;
        let mut budget = 0usize;
        for (index, span) in spans
            .iter()
            .copied()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let height = section_height(index).unwrap();
            let finish = span.map(|span| span.0).unwrap_or(total);
            let count = ((finish - begin).max(0.) / height).ceil() as usize;
            budget = budget.saturating_add(count);
            if budget > 1024 {
                return Err("float column flow exceeds 1024 fragments".into());
            }
            for column in 1..count {
                let at = begin + column as f64 * height;
                candidates.push(at);
                forced.push(at);
            }
            begin = span.map(|span| span.1).unwrap_or(total);
        }
        candidates.sort_by(f64::total_cmp);
        candidates.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        forced.sort_by(f64::total_cmp);
        forced.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    }
    let plan = |begin: f64,
                finish: f64,
                height: f64|
     -> Result<Vec<std::ops::Range<f64>>, String> {
        let mut ranges = Vec::new();
        let mut start = begin;
        while start < finish - 1e-6 {
            if ranges.len() >= 1024 {
                return Err("multicol flow exceeds 1024 fragments".into());
            }
            let next_forced = forced
                .iter()
                .copied()
                .find(|at| {
                    *at > start + 1e-6
                        && *at < finish
                        && !avoid.iter().any(|(top, bottom, atomic)| {
                            *atomic && *at > *top + 1e-6 && *at < *bottom - 1e-6
                        })
                })
                .unwrap_or(finish);
            let limit = (start + height).min(next_forced).min(finish);
            let permitted = |at: f64, strict: bool| {
                !avoid.iter().any(|(top, bottom, atomic)| {
                    at > *top + 1e-6
                        && at < *bottom - 1e-6
                        && (*atomic || bottom - top <= height + 1e-6)
                }) && (!strict || paragraphs.iter().all(|p| p.permits(start, at)))
            };
            let end = if next_forced <= start + height + 1e-6 {
                next_forced
            } else {
                candidates
                    .iter()
                    .copied()
                    .rev()
                    .find(|at| *at > start + 1e-6 && *at <= limit + 1e-6 && permitted(*at, true))
                    .or_else(|| {
                        candidates.iter().copied().find(|at| {
                            *at > start + 1e-6 && *at <= next_forced + 1e-6 && permitted(*at, true)
                        })
                    })
                    .or_else(|| {
                        candidates.iter().copied().rev().find(|at| {
                            *at > start + 1e-6 && *at <= limit + 1e-6 && permitted(*at, false)
                        })
                    })
                    .unwrap_or(next_forced)
            };
            if !end.is_finite() || end <= start {
                return Err("multicol break selection made no progress".into());
            }
            ranges.push(start..end);
            start = end;
        }
        Ok(ranges)
    };
    let mut ranges = Vec::<std::ops::Range<f64>>::new();
    let mut placements = Vec::<(Option<usize>, f64, f64)>::new();
    let mut runs = Vec::new();
    let mut frame = 0.;
    let mut begin = 0.;
    let mut section_counts = Vec::new();
    let mut section_sizes = Vec::new();
    for (index, span) in spans
        .into_iter()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        let finish = span
            .map(|span| span.0)
            .unwrap_or(total)
            .max(begin)
            .min(total);
        section_counts.push(0);
        section_sizes.push((finish - begin).max(0.));
        if finish > begin + 1e-6 {
            let size = finish - begin;
            let height = if let Some(height) = section_height(index) {
                height
            } else if !columns.balance && columns.limit.is_some() && span.is_none() {
                columns.limit.unwrap()
            } else {
                let mut low = (size / columns.count as f64).max(0.01);
                let mut high = size.max(low);
                for _ in 0..48 {
                    let middle = (low + high) / 2.;
                    match plan(begin, finish, middle) {
                        Ok(ranges) if ranges.len() <= columns.count => high = middle,
                        _ => low = middle,
                    }
                }
                columns.limit.map(|limit| high.min(limit)).unwrap_or(high)
            };
            let section = plan(begin, finish, height.max(0.01))?;
            section_counts[index] = section.len();
            let block = if span.is_none() {
                section_height(index).or(columns.limit).unwrap_or_else(|| {
                    section
                        .iter()
                        .map(|range| range.end - range.start)
                        .fold(0f64, f64::max)
                })
            } else {
                section
                    .iter()
                    .map(|range| range.end - range.start)
                    .fold(0f64, f64::max)
            };
            runs.push(ColumnRun {
                columns,
                count: section.len(),
                start: frame,
                block,
            });
            for (column, range) in section.into_iter().enumerate() {
                ranges.push(range);
                placements.push((Some(column), frame, 0.));
            }
            frame += block;
        }
        if let Some((top, bottom, inline_offset)) = span {
            let top = top.max(begin);
            let bottom = bottom.max(top).min(total);
            if bottom > top {
                ranges.push(top..bottom);
                placements.push((None, frame, -inline_offset));
                frame += bottom - top;
            }
            begin = bottom;
        } else {
            begin = finish;
        }
        if ranges.len() > 1024 {
            return Err("multicol flow exceeds 1024 fragments across spanning rows".into());
        }
    }
    engine
        .column_float_section_counts
        .insert(root, section_counts);
    engine
        .column_float_section_sizes
        .insert(root, section_sizes);
    if ranges.is_empty() {
        return Ok(0.);
    }
    engine.column_runs.insert(root, runs);
    let map_logical = |mut rect: CssRect, index: usize| {
        let (column, block, span_offset) = placements[index];
        let offset = column
            .map(|column| {
                if columns.reverse {
                    columns.available
                        - columns.inline
                        - column as f64 * (columns.inline + columns.gap)
                } else {
                    column as f64 * (columns.inline + columns.gap)
                }
            })
            .unwrap_or(span_offset);
        rect.x += offset;
        rect.y += block - ranges[index].start;
        axis.physical(rect, frame)
    };
    let map = |rect: CssRect| {
        let rect = axis.logical(rect);
        let index = ranges
            .partition_point(|range| range.end <= rect.y + 1e-6)
            .min(ranges.len() - 1);
        map_logical(rect, index)
    };
    let mut monolithic_owner = BTreeMap::new();
    for id in &scope {
        let mut current = Some(*id);
        let mut owner = None;
        let mut seen = BTreeSet::new();
        while let Some(node) = current {
            if node == root || !seen.insert(node) {
                break;
            }
            if monolithic.contains(&node) {
                owner = Some(node);
            }
            current = nodes.get(&node).and_then(|node| node.parent);
        }
        if let Some(owner) = owner {
            if let Some(rect) = geometry
                .get(&owner)
                .and_then(|value| value.box_model.as_ref())
                .map(|model| model.border_box)
            {
                let target = map(rect);
                monolithic_owner.insert(*id, (target.x - rect.x, target.y - rect.y));
            }
        }
    }
    let map_owned = |id: u64, rect: CssRect| {
        if let Some((dx, dy)) = monolithic_owner.get(&id) {
            CssRect {
                x: rect.x + dx,
                y: rect.y + dy,
                ..rect
            }
        } else {
            map(rect)
        }
    };
    let fragments = |rect: CssRect| {
        let logical = axis.logical(rect);
        if logical.height == 0. {
            return vec![map(rect)];
        }
        ranges
            .iter()
            .enumerate()
            .filter_map(|(index, range)| {
                let top = if index == 0 {
                    logical.y
                } else {
                    logical.y.max(range.start)
                };
                let bottom = if index + 1 == ranges.len() {
                    logical.bottom()
                } else {
                    logical.bottom().min(range.end)
                };
                (bottom > top).then(|| {
                    map_logical(
                        CssRect {
                            y: top,
                            height: bottom - top,
                            ..logical
                        },
                        index,
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let owned_fragments = |id: u64, rect: CssRect| {
        if monolithic_owner.contains_key(&id) {
            vec![map_owned(id, rect)]
        } else {
            fragments(rect)
        }
    };
    let union = |a: CssRect, b: CssRect| {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        CssRect {
            x,
            y,
            width: a.right().max(b.right()) - x,
            height: a.bottom().max(b.bottom()) - y,
        }
    };
    // Bound retained fragment rectangles before mutating geometry. The source
    // flow and mark glyphs are already bounded by their existing layout limits.
    let mut fragment_count = 0usize;
    for id in &scope {
        if *id == root || excluded(id) {
            continue;
        }
        if let Some(value) = geometry.get(id) {
            if let Some(model) = &value.box_model {
                let input = if value.client_rects.len() > 1 {
                    value.client_rects.as_slice()
                } else {
                    std::slice::from_ref(&model.border_box)
                };
                for rect in input {
                    let logical = axis.logical(*rect);
                    let count = if monolithic_owner.contains_key(id) {
                        1
                    } else {
                        ranges
                            .iter()
                            .filter(|range| logical.bottom() > range.start && logical.y < range.end)
                            .count()
                            .max(1)
                    };
                    fragment_count = fragment_count
                        .checked_add(count)
                        .ok_or("multicol fragment count overflow")?;
                    if fragment_count > 1_000_000 {
                        return Err(
                            "multicol retained rectangles exceed 1000000 resource budget".into(),
                        );
                    }
                }
            }
        }
    }
    for id in &scope {
        if *id == root || excluded(id) {
            continue;
        }
        if let Some(value) = geometry.get_mut(id) {
            if let Some(model) = &mut value.box_model {
                let rects = if value.client_rects.len() > 1 {
                    value
                        .client_rects
                        .iter()
                        .flat_map(|rect| owned_fragments(*id, *rect))
                        .collect::<Vec<_>>()
                } else {
                    owned_fragments(*id, model.border_box)
                };
                if let Some(rect) = rects.iter().copied().reduce(union) {
                    offsets.insert(*id, (rect.width, rect.height));
                    model.border_box = rect;
                    model.padding_box = owned_fragments(*id, model.padding_box)
                        .into_iter()
                        .reduce(union)
                        .unwrap_or(rect);
                    model.content_box = owned_fragments(*id, model.content_box)
                        .into_iter()
                        .reduce(union)
                        .unwrap_or(rect);
                    value.client_rects = rects;
                }
            }
        }
        if let Some(fragments) = text.get_mut(id) {
            for fragment in fragments {
                let flow = fragment.flow_rect.unwrap_or(fragment.rect);
                let target = map_owned(*id, flow);
                let dx = target.x - flow.x;
                let dy = target.y - flow.y;
                fragment.rect.x += dx;
                fragment.rect.y += dy;
                if let Some(flow) = &mut fragment.flow_rect {
                    flow.x += dx;
                    flow.y += dy;
                }
                if let Some(line) = fragment.flow_line {
                    source_shifts.insert((*id, line), (dx, dy));
                }
                for cluster in &mut fragment.clusters {
                    cluster.rect.x += dx;
                    cluster.rect.y += dy;
                }
            }
        }
    }
    let local = |rect: CssRect| {
        let target = map(CssRect {
            x: origin.0 + rect.x,
            y: origin.1 + rect.y,
            ..rect
        });
        CssRect {
            x: target.x - origin.0,
            y: target.y - origin.1,
            ..target
        }
    };
    let line_shifts = layout
        .lines
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let flow = layout.flow_rect(index, mode);
            let moved = local(flow);
            (moved.x - flow.x, moved.y - flow.y)
        })
        .collect::<Vec<_>>();
    for (index, line) in layout.lines.iter_mut().enumerate() {
        let (dx, dy) = line_shifts[index];
        line.4 += dx;
        line.3 += dy;
        source_shifts.insert((root, index), (dx, dy));
    }
    for cluster in &mut layout.clusters {
        if let Some((dx, dy)) = line_shifts.get(cluster.line) {
            cluster.rect.x += dx;
            cluster.rect.y += dy;
        }
    }
    for cluster in &mut layout.source_clusters {
        if let Some((dx, dy)) = line_shifts.get(cluster.line) {
            cluster.rect.x += dx;
            cluster.rect.y += dy;
        }
    }
    for item in &mut layout.inline_boxes {
        if let Some((dx, dy)) = line_shifts.get(item.line) {
            item.rect.x += dx;
            item.rect.y += dy;
        } else {
            item.rect = local(item.rect);
        }
    }
    for line_break in &mut layout.breaks {
        line_break.before = local(line_break.before);
        line_break.after = line_break.after.map(local);
    }
    for clusters in engine.source_geometry.values_mut() {
        for cluster in clusters {
            if scope.contains(&cluster.layout_node) && !excluded(&cluster.layout_node) {
                if let Some((dx, dy)) = source_shifts.get(&(cluster.layout_node, cluster.line)) {
                    cluster.rect.x += dx;
                    cluster.rect.y += dy;
                } else {
                    cluster.rect = map_owned(cluster.layout_node, cluster.rect);
                }
            }
        }
    }
    for line_break in engine.break_boxes.values_mut() {
        if scope.contains(&line_break.layout_node) && !excluded(&line_break.layout_node) {
            line_break.before = map_owned(line_break.layout_node, line_break.before);
            line_break.after = line_break
                .after
                .map(|rect| map_owned(line_break.layout_node, rect));
        }
    }
    for (id, containing) in &mut engine.out_of_flow {
        if scope.contains(id) {
            let rect = map_owned(
                *id,
                CssRect {
                    x: containing.x - scroll.0,
                    y: containing.y - scroll.1,
                    width: 0.,
                    height: 0.,
                },
            );
            containing.x = rect.x + scroll.0;
            containing.y = rect.y + scroll.1;
        }
    }
    if axis.vertical {
        layout.width = frame;
        layout.height = columns.available;
    } else {
        layout.width = columns.available;
        layout.height = frame;
    }
    Ok(frame)
}

/// Rules share the owner's transform and clip plane. They are resolved from
/// final content geometry, so subsequent relative/sticky placement is retained.
pub(super) fn project_rules(
    runs: &BTreeMap<u64, Vec<ColumnRun>>,
    styles: &ComputedStyles,
    viewport: ViewportState,
    projections: &mut BTreeMap<u64, oryn_common::v4::PaintProjection>,
) {
    for (id, sets) in runs {
        for run in sets {
            let columns = run.columns;
            let count = run.count;
            let (Some(style), Some(projection)) = (styles.get(id), projections.get_mut(id)) else {
                continue;
            };
            let line_style = style_value(style, "column-rule-style", "none");
            if matches!(line_style, "none" | "hidden") || count < 2 {
                continue;
            }
            let width = match style_value(style, "column-rule-width", "medium") {
                "thin" => 1.,
                "medium" => 3.,
                "thick" => 5.,
                value => length(value, columns.available, 16., viewport),
            };
            if !width.is_finite() || width <= 0. {
                continue;
            }
            let color = style_value(style, "column-rule-color", "currentcolor");
            let color = if color.eq_ignore_ascii_case("currentcolor") {
                style_value(style, "color", "black")
            } else {
                color
            };
            let mut rect = projection.box_model.content_box;
            let mode = style_value(style, "writing-mode", "horizontal-tb");
            let vertical = mode != "horizontal-tb";
            if vertical {
                rect.x += if matches!(mode, "vertical-rl" | "sideways-rl") {
                    rect.width - run.start - run.block
                } else {
                    run.start
                };
                rect.width = run.block;
            } else {
                rect.y += run.start;
                rect.height = run.block;
            }
            for column in 0..count.saturating_sub(1).min(1023) {
                let position =
                    (column + 1) as f64 * columns.inline + (column as f64 + 0.5) * columns.gap;
                let position = if columns.reverse {
                    columns.available - position
                } else {
                    position
                };
                let rule = if vertical {
                    CssRect {
                        x: rect.x,
                        y: rect.y
                            + if mode == "sideways-lr" {
                                columns.available - position
                            } else {
                                position
                            }
                            - width / 2.,
                        width: rect.width,
                        height: width,
                    }
                } else {
                    CssRect {
                        x: rect.x + position - width / 2.,
                        y: rect.y,
                        width,
                        height: rect.height,
                    }
                };
                projection
                    .border_segments
                    .push(oryn_common::v4::PaintBorderSegment {
                        rect: rule,
                        horizontal: vertical,
                        edge: oryn_common::v4::PaintBorderEdge {
                            width,
                            color: color.into(),
                            style: line_style.into(),
                        },
                    });
            }
        }
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    #[test]
    fn orphan_widow_constraints_use_remaining_native_lines() {
        let paragraph = ParagraphBreaks {
            lines: (0..6)
                .map(|line| (line as f64 * 20., (line + 1) as f64 * 20.))
                .collect(),
            orphans: 2,
            widows: 2,
        };
        assert!(!paragraph.permits(0., 20.));
        assert!(paragraph.permits(0., 40.));
        assert!(!paragraph.permits(0., 100.));
        assert!(!paragraph.permits(40., 60.));
        assert!(paragraph.permits(40., 80.));
    }
    #[test]
    fn vertical_axis_roundtrip_preserves_native_rectangle() {
        let rect = CssRect {
            x: 17.,
            y: 29.,
            width: 11.,
            height: 23.,
        };
        for (mode, reverse) in [
            ("vertical-rl", true),
            ("vertical-lr", false),
            ("sideways-lr", false),
        ] {
            let axis = Axis {
                origin: (3., 5.),
                vertical: true,
                block_reverse: reverse,
                inline_reverse: mode == "sideways-lr",
                inline: 200.,
                block: 100.,
            };
            assert_eq!(axis.physical(axis.logical(rect), 100.), rect);
        }
    }
}

/// Spanners escape ordinary ancestor blocks, but not an intervening independent
/// formatting context (including another multicol container or spanner).
pub(super) fn eligible_spanner(
    root: u64,
    id: u64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> bool {
    let Some(style) = styles.get(&id) else {
        return false;
    };
    if id == root
        || style_value(style, "column-span", "none") != "all"
        || is_inline(style)
        || matches!(
            style_value(style, "position", "static"),
            "absolute" | "fixed"
        )
        || style_value(style, "float", "none") != "none"
    {
        return false;
    }
    let mut parent = nodes.get(&id).and_then(|node| node.parent);
    let mut seen = BTreeSet::new();
    while let Some(id) = parent {
        if id == root {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        let Some(style) = styles.get(&id) else {
            return false;
        };
        if style_value(style, "column-count", "auto") != "auto"
            || style_value(style, "column-width", "auto") != "auto"
            || is_inline(style)
            || inline_formatting::establishes_bfc(style)
        {
            return false;
        }
        parent = nodes.get(&id).and_then(|node| node.parent);
    }
    false
}
