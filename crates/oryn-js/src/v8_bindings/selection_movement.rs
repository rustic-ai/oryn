//! Native Selection.modify movement over document boundaries and shaped lines.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Caret {
    point: BoundaryPoint,
    inline: f64,
    position: (f64, f64),
    inline_basis: (f64, f64),
    block_basis: (f64, f64),
    rtl: bool,
    block: f64,
    line: (u64, usize),
    paragraph: (NodeId, usize),
    order: usize,
}

#[derive(Clone, Copy)]
struct Axes {
    vertical: bool,
    block_sign: f64,
    inline_sign: f64,
}

fn style<'a>(layout: &'a BindingLayout, key: u64, name: &str) -> Option<&'a str> {
    layout.styles.get(&key)?.get(name).map(String::as_str)
}

fn axes(layout: &BindingLayout, key: u64) -> Axes {
    let mode = style(layout, key, "writing-mode").unwrap_or("horizontal-tb");
    let vertical = matches!(
        mode,
        "vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr"
    );
    let rtl = style(layout, key, "direction") == Some("rtl")
        && style(layout, key, "text-orientation") != Some("upright");
    Axes {
        vertical,
        block_sign: if matches!(mode, "vertical-rl" | "sideways-rl") {
            -1.
        } else {
            1.
        },
        inline_sign: if rtl ^ (mode == "sideways-lr") {
            -1.
        } else {
            1.
        },
    }
}

fn projected(x: f64, y: f64, m: [f64; 16]) -> Option<(f64, f64)> {
    let w = m[3] * x + m[7] * y + m[15];
    if w <= 0. || !w.is_finite() {
        return None;
    }
    let output_x = (m[0] * x + m[4] * y + m[12]) / w;
    let output_y = (m[1] * x + m[5] * y + m[13]) / w;
    (output_x.is_finite() && output_y.is_finite()).then_some((output_x, output_y))
}

fn scope_root(
    state: &V8BindingState,
    focus: BoundaryPoint,
) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    if shadow_including_tree_root(&dom, focus.container)? != state.document_root() {
        return Ok(None);
    }
    // Editable movement does not accidentally enter another editable host. A
    // noneditable selection remains bounded by its own tree, including shadows.
    Ok(Some(
        editing_host_for_node(
            &dom,
            Some(focus.container),
            state.design_mode_enabled(),
            state.document_root(),
        )?
        .unwrap_or(tree_root(&dom, focus.container)?),
    ))
}

fn paragraph_for(
    dom: &DomRuntime,
    layout: &BindingLayout,
    node: NodeId,
    root: NodeId,
) -> Result<NodeId, V8BindingError> {
    let mut cursor = node;
    loop {
        if cursor == root {
            return Ok(root);
        }
        let block = style(layout, node_key(cursor), "display").map_or_else(
            || is_editing_block(dom, cursor),
            |display| {
                matches!(
                    display,
                    "block"
                        | "flow-root"
                        | "list-item"
                        | "flex"
                        | "grid"
                        | "table-cell"
                        | "table-caption"
                )
            },
        );
        if block {
            return Ok(cursor);
        }
        let Some(parent) = dom.parent(cursor)? else {
            return Ok(root);
        };
        cursor = parent;
    }
}

fn carets(state: &V8BindingState, root: NodeId) -> Result<Vec<Caret>, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut result = Vec::new();
    let mut breaks: BTreeMap<NodeId, usize> = BTreeMap::new();
    let root_host = editing_host_for_node(
        &dom,
        Some(root),
        state.design_mode_enabled(),
        state.document_root(),
    )?;
    let traversal = traversal_nodes(&dom, root)?;
    let orders: BTreeMap<_, _> = traversal
        .iter()
        .enumerate()
        .map(|(order, node)| (*node, order))
        .collect();
    for (order, node) in traversal.into_iter().enumerate() {
        let record = dom.arena().get(node)?;
        let paragraph = paragraph_for(&dom, &layout, node, root)?;
        if matches!(&record.kind, NodeKind::Element {name} if name.local.as_ref() == "br") {
            *breaks.entry(paragraph).or_default() += 1;
            continue;
        }
        let (NodeKind::Text { data } | NodeKind::CDataSection { data }) = &record.kind else {
            continue;
        };
        if root_host.is_some()
            && editing_host_for_node(
                &dom,
                Some(node),
                state.design_mode_enabled(),
                state.document_root(),
            )? != root_host
        {
            continue;
        }
        if !selectable_text(&dom, &layout, node)? {
            continue;
        }
        let Some(clusters) = layout.text_clusters.get(&node_key(node)) else {
            continue;
        };
        let source = segmentation_text(data);
        let data = source.as_ref();
        let mut boundaries = vec![0usize];
        let mut utf16 = 0;
        for grapheme in data.graphemes(true) {
            utf16 += grapheme.encode_utf16().count();
            boundaries.push(utf16);
        }
        let preserved = matches!(
            style(&layout, node_key(node), "white-space").or_else(|| clusters
                .first()
                .and_then(|c| style(&layout, c.layout_node, "white-space"))),
            Some("pre" | "pre-wrap" | "pre-line" | "break-spaces")
        );
        let mut newline_offsets = Vec::new();
        let mut source_offset = 0;
        for ch in data.chars() {
            source_offset += ch.len_utf16();
            if preserved && matches!(ch, '\n' | '\u{2028}' | '\u{2029}') {
                newline_offsets.push(source_offset);
            }
        }
        for cluster in clusters {
            let axes = axes(&layout, cluster.layout_node);
            let first = boundaries.partition_point(|offset| *offset < cluster.start);
            let last = boundaries.partition_point(|offset| *offset <= cluster.end);
            for &offset in &boundaries[first..last] {
                let fraction =
                    (offset - cluster.start) as f64 / (cluster.end - cluster.start).max(1) as f64;
                let reverse = cluster.rtl
                    ^ (style(&layout, cluster.layout_node, "writing-mode") == Some("sideways-lr"));
                let fraction = if reverse { 1. - fraction } else { fraction };
                let (x, y) = if axes.vertical {
                    (
                        cluster.rect.x + cluster.rect.width / 2.,
                        cluster.rect.y + cluster.rect.height * fraction,
                    )
                } else {
                    (
                        cluster.rect.x + cluster.rect.width * fraction,
                        cluster.rect.y + cluster.rect.height / 2.,
                    )
                };
                let origin = (x, y);
                let Some((x, y)) = projected(x, y, cluster.matrix) else {
                    continue;
                };
                let Some((inline_basis, block_basis)) =
                    projected_axes(origin, axes, cluster.matrix)
                else {
                    continue;
                };
                result.push(Caret {
                    point: BoundaryPoint {
                        container: node,
                        offset: offset as u32,
                    },
                    inline: x * inline_basis.0 + y * inline_basis.1,
                    position: (x, y),
                    inline_basis,
                    block_basis,
                    rtl: cluster.rtl,
                    block: if axes.vertical { x } else { y },
                    line: (cluster.layout_node, cluster.line),
                    paragraph: (
                        paragraph,
                        *breaks.get(&paragraph).unwrap_or(&0)
                            + newline_offsets.partition_point(|end| *end <= offset),
                    ),
                    order,
                });
            }
        }
        *breaks.entry(paragraph).or_default() += newline_offsets.len();
    }
    for stop in &layout.caret_stops {
        let node = NodeId {
            slot: stop.boundary_node as u32,
            generation: (stop.boundary_node >> 32) as u32,
        };
        let Some(&order) = orders.get(&node) else {
            continue;
        };
        let Ok(offset) = u32::try_from(stop.offset) else {
            continue;
        };
        if validate_boundary_offset(&dom, node, offset).is_err()
            || !selectable_text(&dom, &layout, node)?
        {
            continue;
        }
        if root_host.is_some()
            && editing_host_for_node(
                &dom,
                Some(node),
                state.design_mode_enabled(),
                state.document_root(),
            )? != root_host
        {
            continue;
        }
        let axis = axes(&layout, stop.layout_node);
        let origin = (
            stop.rect.x + stop.rect.width / 2.,
            stop.rect.y + stop.rect.height / 2.,
        );
        let Some(position) = projected(origin.0, origin.1, stop.matrix) else {
            continue;
        };
        let Some((inline_basis, block_basis)) = projected_axes(origin, axis, stop.matrix) else {
            continue;
        };
        result.push(Caret {
            point: BoundaryPoint {
                container: node,
                offset,
            },
            position,
            inline_basis,
            block_basis,
            inline: position.0 * inline_basis.0 + position.1 * inline_basis.1,
            block: if axis.vertical {
                position.0
            } else {
                position.1
            },
            line: (stop.layout_node, stop.line),
            paragraph: (paragraph_for(&dom, &layout, node, root)?, 0),
            rtl: stop.rtl,
            order,
        });
    }
    result.sort_by_key(|caret| (caret.order, caret.point.offset, caret.line));
    result.dedup_by(|a, b| a.point == b.point && a.line == b.line && a.inline == b.inline);
    Ok(result)
}

fn nearest(
    state: &V8BindingState,
    carets: &[Caret],
    focus: BoundaryPoint,
    forward: bool,
) -> Result<Option<Caret>, V8BindingError> {
    let mut matching: Vec<_> = carets
        .iter()
        .filter(|caret| caret.point.container == focus.container)
        .copied()
        .collect();
    matching.sort_by(|a, b| {
        a.point
            .offset
            .abs_diff(focus.offset)
            .cmp(&b.point.offset.abs_diff(focus.offset))
            .then_with(|| {
                if forward {
                    b.line.1.cmp(&a.line.1)
                } else {
                    a.line.1.cmp(&b.line.1)
                }
            })
    });
    if let Some(caret) = matching.first() {
        return Ok(Some(*caret));
    }
    let dom = state.dom.borrow();
    let mut previous = None;
    for caret in carets {
        if compare_boundary_points(
            &dom,
            (caret.point.container, caret.point.offset),
            (focus.container, focus.offset),
        )? >= 0
        {
            return Ok(Some(*caret));
        }
        previous = Some(*caret);
    }
    Ok(previous)
}

fn projected_basis(
    origin: (f64, f64),
    vector: (f64, f64),
    matrix: [f64; 16],
) -> Option<(f64, f64)> {
    let a = projected(origin.0, origin.1, matrix)?;
    let b = projected(origin.0 + vector.0, origin.1 + vector.1, matrix)?;
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    (length > 1e-9).then_some(((b.0 - a.0) / length, (b.1 - a.1) / length))
}

fn projected_axes(
    origin: (f64, f64),
    axis: Axes,
    matrix: [f64; 16],
) -> Option<((f64, f64), (f64, f64))> {
    let inline = projected_basis(
        origin,
        if axis.vertical { (0., 1.) } else { (1., 0.) },
        matrix,
    )?;
    let block = projected_basis(
        origin,
        if axis.vertical {
            (axis.block_sign, 0.)
        } else {
            (0., axis.block_sign)
        },
        matrix,
    )?;
    let determinant = inline.0 * block.1 - inline.1 * block.0;
    if determinant.abs() < 1e-9 {
        return None;
    }
    // Dual basis: projections must remove the other axis, including under shear.
    // Dotting against primal transformed axes would mistake inline progress for
    // block movement and choose the wrong column on the next line.
    Some((
        (block.1 / determinant, -block.0 / determinant),
        (-inline.1 / determinant, inline.0 / determinant),
    ))
}

// Keep the preferred caret in viewport coordinates. A scalar from one line
// cannot be interpreted in another line's rotated or orthogonal coordinate frame.
fn project_goal(goal: (f64, f64), current: Caret) -> f64 {
    goal.0 * current.inline_basis.0 + goal.1 * current.inline_basis.1
}

fn next_line(
    carets: &[Caret],
    current: Caret,
    desired_inline: f64,
    _block_sign: f64,
    forward: bool,
    paragraph: bool,
) -> Option<Caret> {
    let sign = if forward { 1. } else { -1. };
    // Compare every candidate in the current line's projected coordinate frame.
    // Mixing each candidate's local x/y axes fails for rotated and orthogonal runs.
    let block_distance = |caret: &Caret| {
        ((caret.position.0 - current.position.0) * current.block_basis.0
            + (caret.position.1 - current.position.1) * current.block_basis.1)
            * sign
    };
    let inline_distance = |caret: &Caret| {
        (caret.position.0 * current.inline_basis.0 + caret.position.1 * current.inline_basis.1
            - desired_inline)
            .abs()
    };
    carets
        .iter()
        .filter(|caret| {
            caret.line != current.line
                && (!paragraph || caret.paragraph != current.paragraph)
                && block_distance(caret) > 0.5
        })
        .min_by(|a, b| {
            block_distance(a)
                .total_cmp(&block_distance(b))
                .then_with(|| inline_distance(a).total_cmp(&inline_distance(b)))
                .then_with(|| a.order.cmp(&b.order))
                .then_with(|| a.point.offset.cmp(&b.point.offset))
        })
        .copied()
}

fn page_caret(
    state: &V8BindingState,
    carets: &[Caret],
    current: Caret,
    position: (f64, f64),
    forward: bool,
) -> Option<Caret> {
    let axis = axes(&state.layout.borrow(), current.line.0);
    let inline = project_goal(position, current);
    let sign = if forward { 1. } else { -1. };
    let distance = |caret: &Caret| {
        ((caret.position.0 - current.position.0) * current.block_basis.0
            + (caret.position.1 - current.position.1) * current.block_basis.1)
            * sign
    };
    let extent = state
        .layout
        .borrow()
        .viewport
        .map(|(width, height)| if axis.vertical { width } else { height })
        .unwrap_or_else(|| carets.iter().map(&distance).fold(0., f64::max));
    let page = extent * 0.875;
    carets
        .iter()
        .filter(|caret| caret.line != current.line && distance(caret) > 0.5)
        .min_by(|a, b| {
            (distance(a) - page)
                .abs()
                .total_cmp(&(distance(b) - page).abs())
                .then_with(|| {
                    let project = |caret: &Caret| {
                        (caret.position.0 * current.inline_basis.0
                            + caret.position.1 * current.inline_basis.1
                            - inline)
                            .abs()
                    };
                    project(a).total_cmp(&project(b))
                })
        })
        .copied()
}

fn visual_character(
    carets: &[Caret],
    current: Caret,
    right: bool,
    base_rtl: bool,
) -> Option<Caret> {
    let sign = if right { 1. } else { -1. };
    let distance = |caret: &Caret| {
        ((caret.position.0 - current.position.0) * current.inline_basis.0
            + (caret.position.1 - current.position.1) * current.inline_basis.1)
            * sign
    };
    carets
        .iter()
        .filter(|caret| caret.line == current.line && distance(caret) > 0.01)
        .min_by(|a, b| {
            distance(a)
                .total_cmp(&distance(b))
                // At a bidi boundary both runs expose a caret at the same physical
                // position. Prefer the paragraph-direction run for the strong caret.
                .then_with(|| (a.rtl != base_rtl).cmp(&(b.rtl != base_rtl)))
        })
        .copied()
}

// At a visual line edge, enter the adjacent line at its paragraph-direction
// edge. Keep the target Caret even when a soft wrap shares its DOM offset;
// subsequent motion must use the new line's affinity.
fn visual_wrap(carets: &[Caret], current: Caret, right: bool, base_rtl: bool) -> Option<Caret> {
    let forward = right != base_rtl;
    let adjacent = next_line(carets, current, current.inline, 1., forward, false)?;
    carets
        .iter()
        .filter(|caret| caret.line == adjacent.line)
        .min_by(|a, b| {
            let position = if right {
                a.inline.total_cmp(&b.inline)
            } else {
                b.inline.total_cmp(&a.inline)
            };
            position
                .then_with(|| (a.rtl != base_rtl).cmp(&(b.rtl != base_rtl)))
                .then_with(|| a.order.cmp(&b.order))
        })
        .copied()
}

fn selectable_text(
    dom: &DomRuntime,
    layout: &BindingLayout,
    mut node: NodeId,
) -> Result<bool, V8BindingError> {
    loop {
        let record = dom.arena().get(node)?;
        if matches!(&record.kind, NodeKind::Element {name} if matches!(name.local.as_ref(), "script"|"style"|"head"|"template"))
            || record.attributes.get_null("hidden").is_some()
            || record.attributes.get_null("inert").is_some()
            || matches!(
                layout.geometry.get(&node_key(node)),
                Some(BindingGeometry::Hidden)
            )
            || style(layout, node_key(node), "display") == Some("none")
            || style(layout, node_key(node), "visibility") == Some("hidden")
        {
            return Ok(false);
        }
        let Some(parent) = dom.parent(node)? else {
            return Ok(true);
        };
        node = parent;
    }
}

fn segmentation_text(data: &str) -> std::borrow::Cow<'_, str> {
    if data.starts_with(oryn_dom::dom_string::PREFIX) {
        // Lone UTF-16 surrogates are individual opaque text units. Replacement
        // scalars preserve their unit widths for segmentation without treating
        // Oryn's internal string envelope as document text.
        std::borrow::Cow::Owned(String::from_utf16_lossy(&oryn_dom::dom_string::units(data)))
    } else {
        std::borrow::Cow::Borrowed(data)
    }
}

fn text_offsets(data: &str, forward: bool, granularity: &str) -> Vec<usize> {
    let source = segmentation_text(data);
    let data = source.as_ref();
    let mut offsets = vec![0usize];
    if matches!(granularity, "sentence" | "sentenceboundary") {
        offsets.clear();
        let mut utf16 = 0;
        for segment in data.split_sentence_bounds() {
            let start = utf16;
            utf16 += segment.encode_utf16().count();
            let trimmed_end = start
                + segment
                    .trim_end_matches(char::is_whitespace)
                    .encode_utf16()
                    .count();
            offsets.push(if forward {
                if granularity == "sentenceboundary" {
                    trimmed_end
                } else {
                    utf16
                }
            } else {
                start
            });
        }
    } else if granularity == "word" {
        offsets.clear();
        let mut utf16 = 0usize;
        let mut positions = BTreeMap::new();
        let mut characters = Vec::new();
        for (byte, ch) in data.char_indices() {
            positions.insert(byte, utf16);
            characters.push((utf16, ch));
            utf16 += ch.len_utf16();
        }
        positions.insert(data.len(), utf16);
        for (byte, segment) in data.split_word_bound_indices() {
            if segment.chars().any(|ch| !ch.is_whitespace()) {
                let boundary = if forward { byte + segment.len() } else { byte };
                offsets.push(positions[&boundary]);
            }
        }
        // Adjacent symbols form one navigation unit. Keep classification
        // linear rather than repeatedly rescanning the UTF-16 prefix.
        let mut symbol_interiors = BTreeSet::new();
        for pair in characters.windows(2) {
            let [(_, a), (offset, b)] = pair else {
                unreachable!()
            };
            if !a.is_alphanumeric()
                && !a.is_whitespace()
                && !b.is_alphanumeric()
                && !b.is_whitespace()
            {
                symbol_interiors.insert(*offset);
            }
            let hangul =
                |ch: char| matches!(ch as u32, 0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af);
            if hangul(*a) != hangul(*b) && a.is_alphabetic() && b.is_alphabetic() {
                offsets.push(*offset);
            }
        }
        offsets.retain(|offset| !symbol_interiors.contains(offset));
    } else {
        let mut offset = 0;
        for grapheme in data.graphemes(true) {
            offset += grapheme.encode_utf16().count();
            offsets.push(offset);
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

#[derive(Default)]
struct TextRun {
    text: String,
    units: Vec<u16>,
    spans: Vec<(NodeId, usize, usize)>,
    length: usize,
    paragraph: Option<NodeId>,
}

impl TextRun {
    fn append(&mut self, node: NodeId, text: &str) {
        let units = oryn_dom::dom_string::units(text);
        let end = self.length + units.len();
        self.spans.push((node, self.length, end));
        self.units.extend(units);
        self.length = end;
    }
    fn focus_offset(&self, focus: BoundaryPoint) -> Option<usize> {
        self.spans
            .iter()
            .find(|(node, _, _)| *node == focus.container)
            .map(|(_, start, end)| (*start + focus.offset as usize).min(*end))
    }
    fn boundary(&self, offset: usize, forward: bool) -> Option<BoundaryPoint> {
        // Strong DOM affinity follows the movement direction. Both sides of an
        // inline node split describe the same textual position, never two steps.
        let span = if forward {
            self.spans
                .iter()
                .find(|(_, start, end)| *start <= offset && offset <= *end)
        } else {
            self.spans
                .iter()
                .rev()
                .find(|(_, start, end)| *start <= offset && offset <= *end)
        }?;
        Some(BoundaryPoint {
            container: span.0,
            offset: (offset - span.1) as u32,
        })
    }
}

fn text_runs(state: &V8BindingState, root: NodeId) -> Result<Vec<TextRun>, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let root_host = editing_host_for_node(
        &dom,
        Some(root),
        state.design_mode_enabled(),
        state.document_root(),
    )?;
    let mut runs = Vec::new();
    let mut run = TextRun::default();
    let flush = |run: &mut TextRun, runs: &mut Vec<TextRun>| {
        if !run.spans.is_empty() {
            run.text = oryn_dom::dom_string::from_units(&run.units);
            runs.push(std::mem::take(run));
        }
    };
    for node in traversal_nodes(&dom, root)? {
        let record = dom.arena().get(node)?;
        let foreign = root_host.is_some()
            && editing_host_for_node(
                &dom,
                Some(node),
                state.design_mode_enabled(),
                state.document_root(),
            )? != root_host;
        if foreign || matches!(&record.kind,NodeKind::Element {name} if name.local.as_ref()=="br") {
            flush(&mut run, &mut runs);
            continue;
        }
        let (NodeKind::Text { data } | NodeKind::CDataSection { data }) = &record.kind else {
            continue;
        };
        if !selectable_text(&dom, &layout, node)? {
            continue;
        }
        let paragraph = paragraph_for(&dom, &layout, node, root)?;
        if run.paragraph.is_some_and(|previous| previous != paragraph) {
            flush(&mut run, &mut runs);
        }
        run.paragraph = Some(paragraph);
        run.append(node, data);
    }
    flush(&mut run, &mut runs);
    Ok(runs)
}

fn textual_move(
    state: &V8BindingState,
    root: NodeId,
    focus: BoundaryPoint,
    forward: bool,
    granularity: &str,
) -> Result<Option<BoundaryPoint>, V8BindingError> {
    let runs = text_runs(state, root)?;
    let dom = state.dom.borrow();
    let mut found = None;
    for run in runs {
        let local_focus = run.focus_offset(focus);
        for offset in text_offsets(&run.text, forward, granularity) {
            let Some(point) = run.boundary(offset, forward) else {
                continue;
            };
            let comparison = if let Some(local_focus) = local_focus {
                offset.cmp(&local_focus) as i32
            } else {
                compare_boundary_points(
                    &dom,
                    (point.container, point.offset),
                    (focus.container, focus.offset),
                )?
            };
            if (forward && comparison > 0) || (!forward && comparison < 0) {
                if forward {
                    return Ok(Some(point));
                }
                found = Some(point);
            }
        }
    }
    Ok(found)
}

fn visual_word(
    state: &V8BindingState,
    root: NodeId,
    carets: &[Caret],
    current: Caret,
    right: bool,
    base_rtl: bool,
) -> Result<Option<Caret>, V8BindingError> {
    let mut starts = BTreeSet::new();
    let mut ends = BTreeSet::new();
    for run in text_runs(state, root)? {
        for forward in [false, true] {
            for offset in text_offsets(&run.text, forward, "word") {
                if let Some(point) = run.boundary(offset, forward) {
                    if forward {
                        ends.insert((point.container, point.offset));
                    } else {
                        starts.insert((point.container, point.offset));
                    }
                }
            }
        }
    }
    let eligible: Vec<_> = carets
        .iter()
        .copied()
        .filter(|caret| {
            let boundaries = if right != caret.rtl { &ends } else { &starts };
            boundaries.contains(&(caret.point.container, caret.point.offset))
        })
        .collect();
    // Coincident bidi boundaries can name both the word just traversed and
    // its neighboring run. Keep the traversed run's boundary for word motion.
    Ok(visual_character(&eligible, current, right, current.rtl)
        .or_else(|| visual_wrap(&eligible, current, right, base_rtl)))
}

/// Control-value source clusters must be keyed by the control node and carry
/// UTF-16 offsets into the current native value. A DOM child/defaultValue cluster
/// is not a substitute: dirty textarea values need the live provider's projection.
fn control_carets(state: &V8BindingState, node: NodeId, value: &str) -> Vec<Caret> {
    let source = segmentation_text(value);
    let value = source.as_ref();
    let layout = state.layout.borrow();
    let Some(clusters) = layout.text_clusters.get(&node_key(node)) else {
        return Vec::new();
    };
    let boundaries = text_offsets(value, true, "character");
    let length = value.encode_utf16().count();
    let mut newline_offsets = Vec::new();
    let mut offset = 0;
    for ch in value.chars() {
        offset += ch.len_utf16();
        if ch == '\n' {
            newline_offsets.push(offset);
        }
    }
    let mut result = Vec::new();
    for cluster in clusters {
        if cluster.start > cluster.end || cluster.end > length {
            continue;
        }
        let key = if layout.styles.contains_key(&cluster.layout_node) {
            cluster.layout_node
        } else {
            node_key(node)
        };
        let axis = axes(&layout, key);
        let first = boundaries.partition_point(|offset| *offset < cluster.start);
        let last = boundaries.partition_point(|offset| *offset <= cluster.end);
        for &offset in &boundaries[first..last] {
            let fraction =
                (offset - cluster.start) as f64 / (cluster.end - cluster.start).max(1) as f64;
            let reverse =
                cluster.rtl ^ (style(&layout, key, "writing-mode") == Some("sideways-lr"));
            let fraction = if reverse { 1. - fraction } else { fraction };
            let origin = if axis.vertical {
                (
                    cluster.rect.x + cluster.rect.width / 2.,
                    cluster.rect.y + cluster.rect.height * fraction,
                )
            } else {
                (
                    cluster.rect.x + cluster.rect.width * fraction,
                    cluster.rect.y + cluster.rect.height / 2.,
                )
            };
            let Some(position) = projected(origin.0, origin.1, cluster.matrix) else {
                continue;
            };
            let Some((inline_basis, block_basis)) = projected_axes(origin, axis, cluster.matrix)
            else {
                continue;
            };
            result.push(Caret {
                point: BoundaryPoint {
                    container: node,
                    offset: offset as u32,
                },
                inline: position.0 * inline_basis.0 + position.1 * inline_basis.1,
                block: if axis.vertical {
                    position.0
                } else {
                    position.1
                },
                position,
                inline_basis,
                block_basis,
                rtl: cluster.rtl,
                line: (cluster.layout_node, cluster.line),
                paragraph: (node, newline_offsets.partition_point(|end| *end <= offset)),
                order: 0,
            });
        }
    }
    result
}

/// Read the logical edge of the current shaped control line without changing
/// selection. Deletion uses this after beforeinput so cancellation and undo
/// retain the original native selection, including soft-wrap affinity.
pub(super) fn control_line_boundary(
    state: &V8BindingState,
    node: NodeId,
    value: &str,
    offset: u32,
    forward: bool,
) -> Result<Option<u32>, V8BindingError> {
    refresh_layout(state)?;
    if state.execution_retired()
        || !state.dom.borrow().is_editable_text_control(node)
        || state.dom.borrow().control_edit_value(node)? != value
    {
        return Ok(None);
    }
    let carets = control_carets(state, node, value);
    let selection = state.text_control_selections.borrow().get(&node).cloned();
    let remembered = state
        .control_modify_goal
        .borrow()
        .clone()
        .filter(|(owner, old, _, _)| *owner == node && selection.as_ref() == Some(old));
    let current = remembered
        .as_ref()
        .and_then(|(_, _, _, line)| {
            carets
                .iter()
                .find(|caret| caret.point.offset == offset && caret.line == *line)
        })
        .or_else(|| {
            carets
                .iter()
                .filter(|caret| caret.point.offset == offset)
                .min_by_key(|caret| caret.line.1)
        });
    Ok(current.and_then(|current| {
        carets
            .iter()
            .filter(|caret| caret.line == current.line)
            .map(|caret| caret.point.offset)
            .reduce(|a, b| if forward { a.max(b) } else { a.min(b) })
    }))
}

/// Text controls have native value/selection storage, not DOM text-node ranges.
/// Resolve a drop point from shaped, transformed native caret geometry. The
/// input host has already hit-tested the target; this never guesses from string
/// length or uses public author-replaceable caret/selection methods.
pub(super) fn drop_caret(
    state: &V8BindingState,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<Option<BoundaryPoint>, V8BindingError> {
    caret_at_native_point(state, target, x, y, true)
}

fn pointer_word_span(text: &str, offset: usize) -> (usize, usize) {
    let text = segmentation_text(text);
    let text = text.as_ref();
    pointer_word_span_normalized(text, offset)
}

fn pointer_word_span_normalized(text: &str, offset: usize) -> (usize, usize) {
    let mut start = 0;
    let mut last = (0, 0);
    for segment in text.split_word_bounds() {
        let end = start + segment.encode_utf16().count();
        last = (start, end);
        if offset < end {
            return last;
        }
        start = end;
    }
    last
}

// Collect only the selected paragraph's run, with fallible scratch allocation.
// Shared caret, gesture and activation transport remain unchanged.
fn pointer_word_run(
    state: &V8BindingState,
    point: BoundaryPoint,
    root: NodeId,
) -> Result<Option<TextRun>, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let paragraph = paragraph_for(&dom, &layout, point.container, root)?;
    let host = editing_host_for_node(
        &dom,
        Some(point.container),
        state.design_mode_enabled(),
        state.document_root(),
    )?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(1)
        .map_err(|_| V8BindingError::Allocation("pointer word traversal admission"))?;
    pending.push(paragraph);
    let mut run = TextRun::default();
    let mut found = false;
    while let Some(node) = pending.pop() {
        let record = dom.arena().get(node)?;
        let foreign = host.is_some()
            && editing_host_for_node(
                &dom,
                Some(node),
                state.design_mode_enabled(),
                state.document_root(),
            )? != host;
        let barrier = foreign
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "br")
            || (node != paragraph && paragraph_for(&dom, &layout, node, paragraph)? == node);
        if barrier {
            if found {
                break;
            }
            run = TextRun::default();
            continue;
        }
        if let NodeKind::Text { data } | NodeKind::CDataSection { data } = &record.kind {
            if !selectable_text(&dom, &layout, node)? {
                continue;
            }
            // Decode with a fallible canonical WTF-16 adapter.
            let source = oryn_dom::dom_string::try_units(data, usize::MAX)
                .map_err(V8BindingError::Allocation)?;
            let length = source.len();
            let end = run
                .length
                .checked_add(length)
                .ok_or(V8BindingError::Allocation("pointer word source limit"))?;
            run.units
                .try_reserve_exact(length)
                .map_err(|_| V8BindingError::Allocation("pointer word units admission"))?;
            run.spans
                .try_reserve_exact(1)
                .map_err(|_| V8BindingError::Allocation("pointer word spans admission"))?;
            run.spans.push((node, run.length, end));
            run.units.extend_from_slice(&source);
            run.length = end;
            found |= node == point.container;
        } else {
            // Existing tree-root movement semantics are retained. Flat-tree
            // source mapping is a separate shared Selection contract.
            pending
                .len()
                .checked_add(record.children.len())
                .ok_or(V8BindingError::Allocation(
                    "pointer word traversal overflow",
                ))?;
            pending
                .try_reserve_exact(record.children.len())
                .map_err(|_| V8BindingError::Allocation("pointer word traversal admission"))?;
            for child in record.children.iter().rev().copied() {
                if !matches!(
                    dom.arena().get(child)?.kind,
                    NodeKind::Document | NodeKind::ShadowRoot { .. }
                ) {
                    pending.push(child);
                }
            }
        }
    }
    if !found {
        return Ok(None);
    }
    // Bound decoded storage before the canonical DOM-string conversion.
    let bytes = run
        .units
        .len()
        .checked_mul(3)
        .ok_or(V8BindingError::Allocation("pointer word text admission"))?;
    // Three bytes per source unit also covers replacement scalars for lone
    // surrogates; supplementary pairs require four bytes for two units.
    run.text
        .try_reserve_exact(bytes)
        .map_err(|_| V8BindingError::Allocation("pointer word segmentation admission"))?;
    // Identical normalization to segmentation_text: opaque lone surrogates
    // become one replacement scalar (one UTF-16 unit), without an envelope
    // serialization followed by a second infallible decode/collect.
    for scalar in char::decode_utf16(run.units.iter().copied()) {
        run.text.push(scalar.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    run.paragraph = Some(paragraph);
    Ok(Some(run))
}

/// Multi-click units remain native UTF-16 ranges, including words spanning
/// adjacent inline text nodes and visual lines containing bidi runs.
pub(super) fn pointer_unit(
    state: &V8BindingState,
    point: BoundaryPoint,
    count: i32,
) -> Result<Option<(BoundaryPoint, BoundaryPoint)>, V8BindingError> {
    if count < 2 {
        return Ok(None);
    }
    let control = state.dom.borrow().is_editable_text_control(point.container);
    if count == 2 {
        if control {
            let value = state.dom.borrow().control_edit_value(point.container)?;
            let (start, end) = pointer_word_span(&value, point.offset as usize);
            return Ok(Some((
                BoundaryPoint {
                    container: point.container,
                    offset: start as u32,
                },
                BoundaryPoint {
                    container: point.container,
                    offset: end as u32,
                },
            )));
        }
        let Some(root) = scope_root(state, point)? else {
            return Ok(None);
        };
        if let Some(run) = pointer_word_run(state, point, root)?
            && let Some(offset) = run.focus_offset(point)
        {
            let (start, end) = pointer_word_span_normalized(&run.text, offset);
            return Ok(run.boundary(start, true).zip(run.boundary(end, false)));
        }
        return Ok(None);
    }
    let candidates = if control {
        let value = state.dom.borrow().control_edit_value(point.container)?;
        control_carets(state, point.container, &value)
    } else {
        let Some(root) = scope_root(state, point)? else {
            return Ok(None);
        };
        carets(state, root)?
    };
    let Some(current) = candidates.iter().find(|caret| caret.point == point) else {
        return Ok(None);
    };
    let mut line = candidates
        .iter()
        .filter(|caret| caret.line == current.line)
        .collect::<Vec<_>>();
    line.sort_by_key(|caret| (caret.order, caret.point.offset));
    Ok(line
        .first()
        .zip(line.last())
        .map(|(first, last)| (first.point, last.point)))
}

/// Pointer selection can enter ordinary document text as well as editable
/// content. Drop insertion remains constrained to the native editing host.
pub(super) fn pointer_caret(
    state: &V8BindingState,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<Option<BoundaryPoint>, V8BindingError> {
    refresh_layout(state)?;
    if pointer_exclusion(state, target)?.is_some() {
        return Ok(None);
    }
    caret_at_native_point(state, target, x, y, false)
}

fn pointer_exclusion(
    state: &V8BindingState,
    target: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut cursor = Some(target);
    let mut seen = BTreeSet::new();
    while let Some(node) = cursor.filter(|node| seen.insert(*node)) {
        let used = style(&layout, node_key(node), "user-select");
        match used {
            Some("text" | "all") => return Ok(None),
            Some("none") => return Ok(Some(node)),
            _ => {}
        }
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "button") {
            return Ok(Some(node));
        }
        cursor = dom.flat_tree_parent(node)?;
    }
    Ok(None)
}

/// Extending an existing gesture into an unselectable subtree selects up to
/// its near edge. Dropping the update would leave an unrelated old endpoint.
pub(super) fn pointer_exclusion_boundary(
    state: &V8BindingState,
    target: NodeId,
    anchor: BoundaryPoint,
) -> Result<Option<BoundaryPoint>, V8BindingError> {
    refresh_layout(state)?;
    let Some(excluded) = pointer_exclusion(state, target)? else {
        return Ok(None);
    };
    let dom = state.dom.borrow();
    let Some(parent) = dom.parent(excluded)? else {
        return Ok(None);
    };
    let children = light_children(&dom, parent)?;
    let Some(index) = children.iter().position(|node| *node == excluded) else {
        return Ok(None);
    };
    let before = BoundaryPoint {
        container: parent,
        offset: index as u32,
    };
    let precedes = compare_boundary_points(
        &dom,
        (anchor.container, anchor.offset),
        (before.container, before.offset),
    )
    .is_ok_and(|order| order <= 0);
    Ok(Some(BoundaryPoint {
        container: parent,
        offset: index as u32 + u32::from(!precedes),
    }))
}

fn caret_at_native_point(
    state: &V8BindingState,
    target: NodeId,
    x: f64,
    y: f64,
    editing_only: bool,
) -> Result<Option<BoundaryPoint>, V8BindingError> {
    if !x.is_finite() || !y.is_finite() || state.execution_retired() {
        return Ok(None);
    }
    refresh_layout(state)?;
    let (is_control, host) = {
        let dom = state.dom.borrow();
        if !dom.is_connected(target)?
            || dom.node_document(target)? != Some(state.document_root())
            || dom.is_inert(target)?
        {
            return Ok(None);
        }
        (
            dom.is_editable_text_control(target),
            editing_host_for_node(
                &dom,
                Some(target),
                state.design_mode_enabled(),
                state.document_root(),
            )?,
        )
    };
    let candidates = if is_control {
        if state.dom.borrow().control_disabled(target)? {
            return Ok(None);
        }
        let value = state.dom.borrow().control_edit_value(target)?;
        if value.is_empty() {
            return Ok(Some(BoundaryPoint {
                container: target,
                offset: 0,
            }));
        }
        control_carets(state, target, &value)
    } else {
        let root = match host {
            Some(host) => host,
            None if editing_only => return Ok(None),
            None => target,
        };
        carets(state, root)?
    };
    Ok(candidates
        .iter()
        .min_by(|a, b| {
            let distance = |caret: &Caret| (caret.position.0 - x).hypot(caret.position.1 - y);
            distance(a)
                .total_cmp(&distance(b))
                .then_with(|| a.order.cmp(&b.order))
                .then_with(|| a.point.offset.cmp(&b.point.offset))
        })
        .map(|caret| caret.point))
}

/// Text controls have native value/selection storage, not DOM text-node ranges.
/// Return whether the focused control owns this command; callers queue select
/// through the native task source only when its selection actually changes.
pub(super) struct ControlMovement {
    node: NodeId,
    old: (u32, u32, String),
    next: (u32, u32, String),
    goal: Option<((f64, f64), (u64, usize))>,
}
impl ControlMovement {
    pub(super) fn boundaries(&self) -> (u32, u32) {
        (self.next.0, self.next.1)
    }
    pub(super) fn commit(
        &self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<(), V8BindingError> {
        if self.next != self.old {
            queue_control_select(scope, state, self.node)?;
            state
                .text_control_selections
                .borrow_mut()
                .insert(self.node, self.next.clone());
        }
        *state.control_modify_goal.borrow_mut() = self
            .goal
            .map(|(inline, line)| (self.node, self.next.clone(), inline, line));
        Ok(())
    }
}
pub(super) fn prepare_control_movement(
    state: &Rc<V8BindingState>,
    node: NodeId,
    alter: &str,
    direction: &str,
    granularity: &str,
) -> Result<Option<ControlMovement>, V8BindingError> {
    if state.execution_retired() {
        return Ok(None);
    }
    let dom = state.dom.borrow();
    if !dom.is_editable_text_control(node) {
        return Ok(None);
    }
    if dom.control_disabled(node)?
        || !matches!(alter, "move" | "extend")
        || !matches!(direction, "forward" | "backward" | "left" | "right")
    {
        return Ok(None);
    }
    let value = dom.control_edit_value(node)?;
    let length = oryn_dom::dom_string::len(&value) as u32;
    drop(dom);
    let old = state
        .text_control_selections
        .borrow()
        .get(&node)
        .cloned()
        .unwrap_or((length, length, "none".into()));
    let (start, end) = (old.0.min(length), old.1.min(length));
    let (anchor, focus) = if old.2 == "backward" {
        (end, start)
    } else {
        (start, end)
    };
    let forward = matches!(direction, "forward" | "right");
    let mut next_goal = None;
    let target = match granularity {
        "line" | "paragraph" | "lineboundary" | "page" => {
            refresh_layout(state)?;
            let carets = control_carets(state, node, &value);
            let remembered = state
                .control_modify_goal
                .borrow()
                .clone()
                .filter(|(owner, selection, _, _)| *owner == node && *selection == old);
            let current = remembered
                .as_ref()
                .and_then(|(_, _, _, line)| {
                    carets
                        .iter()
                        .find(|c| c.point.offset == focus && c.line == *line)
                        .copied()
                })
                .or_else(|| {
                    carets
                        .iter()
                        .filter(|c| c.point.offset == focus)
                        .min_by_key(|c| c.line.1)
                        .copied()
                });
            current.and_then(|current| {
                let goal = remembered
                    .as_ref()
                    .map_or(current.position, |(_, _, goal, _)| *goal);
                let inline = project_goal(goal, current);
                let target = if granularity == "lineboundary" {
                    carets
                        .iter()
                        .filter(|c| c.line == current.line)
                        .min_by_key(|c| {
                            if forward {
                                u32::MAX - c.point.offset
                            } else {
                                c.point.offset
                            }
                        })
                        .copied()
                } else if granularity == "page" {
                    page_caret(state, &carets, current, goal, forward)
                } else {
                    next_line(
                        &carets,
                        current,
                        inline,
                        1.,
                        forward,
                        granularity == "paragraph",
                    )
                };
                target.map(|caret| {
                    next_goal = Some((goal, caret.line));
                    caret.point.offset
                })
            })
        }
        "documentboundary" => Some(if forward { length } else { 0 }),
        "character" if alter == "move" && start != end => Some(if forward { end } else { start }),
        "character" | "word" if matches!(direction, "left" | "right") => {
            refresh_layout(state)?;
            let carets = control_carets(state, node, &value);
            let remembered = state
                .control_modify_goal
                .borrow()
                .clone()
                .filter(|(owner, selection, _, _)| *owner == node && *selection == old);
            let current = remembered
                .as_ref()
                .and_then(|(_, _, _, line)| {
                    carets
                        .iter()
                        .find(|c| c.point.offset == focus && c.line == *line)
                        .copied()
                })
                .or_else(|| carets.iter().find(|c| c.point.offset == focus).copied());
            current
                .and_then(|current| {
                    let base_rtl = axes(&state.layout.borrow(), current.line.0).inline_sign < 0.;
                    let eligible: Vec<_> = carets
                        .iter()
                        .copied()
                        .filter(|caret| {
                            granularity == "character"
                                || text_offsets(&value, forward != caret.rtl, "word")
                                    .contains(&(caret.point.offset as usize))
                        })
                        .collect();
                    visual_character(&eligible, current, forward, base_rtl)
                        .or_else(|| visual_wrap(&eligible, current, forward, base_rtl))
                        .map(|caret| {
                            next_goal = Some((caret.position, caret.line));
                            caret.point.offset
                        })
                })
                .or_else(|| {
                    // A host without a rendered control snapshot can still perform
                    // logical segmentation; do not manufacture soft-wrap geometry.
                    if current.is_some() {
                        return None;
                    }
                    let offsets = text_offsets(&value, forward, granularity);
                    if forward {
                        offsets.into_iter().find(|offset| *offset > focus as usize)
                    } else {
                        offsets
                            .into_iter()
                            .rev()
                            .find(|offset| *offset < focus as usize)
                    }
                    .map(|offset| offset as u32)
                })
        }
        "character" | "word" | "sentence" | "sentenceboundary" => {
            let offsets = text_offsets(&value, forward, granularity);
            if forward {
                offsets.into_iter().find(|offset| *offset > focus as usize)
            } else {
                offsets
                    .into_iter()
                    .rev()
                    .find(|offset| *offset < focus as usize)
            }
            .map(|offset| offset as u32)
        }
        // Hard line boundaries include empty lines. Soft wraps require layout's
        // anonymous control-text geometry and are intentionally not guessed.
        "paragraphboundary" => {
            let units = oryn_dom::dom_string::units(&value);
            if forward {
                Some(
                    units[focus as usize..]
                        .iter()
                        .position(|u| *u == 10)
                        .map_or(length, |i| focus + i as u32),
                )
            } else {
                Some(
                    units[..focus as usize]
                        .iter()
                        .rposition(|u| *u == 10)
                        .map_or(0, |i| i as u32 + 1),
                )
            }
        }
        _ => None,
    };

    let Some(target) = target else {
        return Ok(None);
    };
    let anchor = if alter == "move" { target } else { anchor };
    let next = (
        anchor.min(target),
        anchor.max(target),
        if anchor == target {
            "none"
        } else if target < anchor {
            "backward"
        } else {
            "forward"
        }
        .to_owned(),
    );
    Ok(Some(ControlMovement {
        node,
        old,
        next,
        goal: next_goal,
    }))
}
pub(super) fn modify_control(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    alter: &str,
    direction: &str,
    granularity: &str,
) -> Result<bool, V8BindingError> {
    if state.execution_retired() {
        return Ok(false);
    }
    let Some(node) = state.dom.borrow().focused() else {
        return Ok(false);
    };
    if !state.dom.borrow().is_editable_text_control(node) {
        return Ok(false);
    }
    if let Some(plan) = prepare_control_movement(state, node, alter, direction, granularity)? {
        plan.commit(scope, state)?;
    }
    Ok(true)
}

pub(super) fn modify(
    state: &V8BindingState,
    alter: &str,
    direction: &str,
    granularity: &str,
) -> Result<(), V8BindingError> {
    if granularity == "page" {
        return Ok(());
    }
    modify_admitted(state, alter, direction, granularity, |_, _| Ok(true))
}
/// Compute the native destination before dispatching selectstart. The admission
/// callback runs before Range replacement or caret-affinity publication.
pub(super) struct PreparedMovement {
    pub(super) boundaries: Option<(BoundaryPoint, BoundaryPoint)>,
    goal: Option<(f64, f64)>,
    caret: Option<((u64, usize), f64)>,
}
impl PreparedMovement {
    pub(super) fn select(anchor: BoundaryPoint, focus: BoundaryPoint) -> Self {
        Self {
            boundaries: Some((anchor, focus)),
            goal: None,
            caret: None,
        }
    }
    pub(super) fn commit(&self, state: &V8BindingState) -> Result<(), V8BindingError> {
        if let Some((anchor, focus)) = self.boundaries {
            update_selection_boundaries(state, anchor, focus, true)?;
        }
        state.selection_modify_inline.set(
            self.goal
                .map(|inline| (state.selection_revision.get(), inline)),
        );
        state.selection_modify_caret.set(
            self.caret
                .map(|(line, inline)| (state.selection_revision.get(), line, inline)),
        );
        Ok(())
    }
}
pub(super) fn modify_admitted(
    state: &V8BindingState,
    alter: &str,
    direction: &str,
    granularity: &str,
    mut admit: impl FnMut(BoundaryPoint, BoundaryPoint) -> Result<bool, V8BindingError>,
) -> Result<(), V8BindingError> {
    let Some(plan) = prepare_movement(state, alter, direction, granularity)? else {
        return Ok(());
    };
    if let Some((anchor, focus)) = plan.boundaries
        && !admit(anchor, focus)?
    {
        return Ok(());
    }
    plan.commit(state)
}
pub(super) fn prepare_movement(
    state: &V8BindingState,
    alter: &str,
    direction: &str,
    granularity: &str,
) -> Result<Option<PreparedMovement>, V8BindingError> {
    if state.execution_retired() {
        return Ok(None);
    }
    if !matches!(alter, "move" | "extend")
        || !matches!(direction, "forward" | "backward" | "left" | "right")
        || !matches!(
            granularity,
            "character"
                | "word"
                | "sentence"
                | "sentenceboundary"
                | "line"
                | "paragraph"
                | "page"
                | "lineboundary"
                | "paragraphboundary"
                | "documentboundary"
        )
    {
        return Ok(None);
    }
    synchronize_selection_range(state)?;
    let (Some(anchor), Some(focus)) = (state.selection_anchor.get(), state.selection_focus.get())
    else {
        return Ok(None);
    };
    let Some(root) = scope_root(state, focus)? else {
        return Ok(None);
    };
    refresh_layout(state)?;
    let carets = carets(state, root)?;
    let logical_forward = matches!(direction, "forward" | "right");
    let affinity = state
        .selection_modify_caret
        .get()
        .filter(|(revision, _, _)| *revision == state.selection_revision.get());
    let current = affinity
        .and_then(|(_, line, inline)| {
            carets
                .iter()
                .find(|c| c.point == focus && c.line == line && c.inline == inline)
                .copied()
        })
        .or(nearest(state, &carets, focus, logical_forward)?);
    let axis = current.map(|caret| axes(&state.layout.borrow(), caret.line.0));
    let forward = if matches!(direction, "left" | "right")
        && matches!(granularity, "character" | "word" | "lineboundary")
    {
        logical_forward == axis.is_none_or(|axis| axis.inline_sign > 0.)
    } else {
        logical_forward
    };
    let mut goal = None;
    let mut target_caret = None;
    let target = match granularity {
        "page" => current.and_then(|current| {
            let position = state
                .selection_modify_inline
                .get()
                .filter(|(revision, _)| *revision == state.selection_revision.get())
                .map_or(current.position, |(_, position)| position);
            goal = Some(position);
            target_caret = page_caret(state, &carets, current, position, forward);
            target_caret.map(|caret| caret.point)
        }),
        "line" | "paragraph" => current.and_then(|current| {
            let position = state
                .selection_modify_inline
                .get()
                .filter(|(revision, _)| *revision == state.selection_revision.get())
                .map_or(current.position, |(_, position)| position);
            let inline = project_goal(position, current);
            goal = Some(position);
            target_caret = next_line(
                &carets,
                current,
                inline,
                axis.unwrap().block_sign,
                forward,
                granularity == "paragraph",
            );
            target_caret.map(|caret| caret.point)
        }),
        "lineboundary" | "paragraphboundary" => current.and_then(|current| {
            target_caret = carets
                .iter()
                .filter(|caret| {
                    if granularity == "lineboundary" {
                        caret.line == current.line
                    } else {
                        caret.paragraph == current.paragraph
                    }
                })
                .min_by(|a, b| {
                    // Pinned selection/bidi/modify.tentative.html defines
                    // lineboundary by paragraph direction and DOM endpoints,
                    // even when its only resolved run has the opposite direction.
                    let ordering = a
                        .order
                        .cmp(&b.order)
                        .then_with(|| a.point.offset.cmp(&b.point.offset));
                    if forward {
                        ordering.reverse()
                    } else {
                        ordering
                    }
                })
                .copied();
            target_caret.map(|caret| caret.point)
        }),
        "documentboundary" => {
            let edge = if forward {
                carets
                    .iter()
                    .max_by_key(|caret| (caret.order, caret.point.offset))
            } else {
                carets
                    .iter()
                    .min_by_key(|caret| (caret.order, caret.point.offset))
            };
            edge.map(|caret| caret.point)
        }
        _ => {
            if alter == "move" && anchor != focus && granularity == "character" {
                let comparison = compare_boundary_points(
                    &state.dom.borrow(),
                    (anchor.container, anchor.offset),
                    (focus.container, focus.offset),
                )?;
                if matches!(direction, "left" | "right") {
                    let anchor_caret = nearest(state, &carets, anchor, logical_forward)?;
                    match (anchor_caret, current) {
                        (Some(a), Some(f)) if a.line == f.line => {
                            let delta = (f.position.0 - a.position.0) * f.inline_basis.0
                                + (f.position.1 - a.position.1) * f.inline_basis.1;
                            Some(if (delta >= 0.) == logical_forward {
                                focus
                            } else {
                                anchor
                            })
                        }
                        _ => Some(if (comparison < 0) == forward {
                            focus
                        } else {
                            anchor
                        }),
                    }
                } else {
                    Some(if (comparison < 0) == forward {
                        focus
                    } else {
                        anchor
                    })
                }
            } else if matches!(granularity, "character" | "word")
                && matches!(direction, "left" | "right")
                && current.is_some()
            {
                let current = current.unwrap();
                target_caret = if granularity == "word" {
                    visual_word(
                        state,
                        root,
                        &carets,
                        current,
                        direction == "right",
                        axis.is_some_and(|a| a.inline_sign < 0.),
                    )?
                } else {
                    visual_character(
                        &carets,
                        current,
                        direction == "right",
                        axis.is_some_and(|a| a.inline_sign < 0.),
                    )
                };
                if target_caret.is_none() && granularity == "character" {
                    target_caret = visual_wrap(
                        &carets,
                        current,
                        direction == "right",
                        axis.is_some_and(|a| a.inline_sign < 0.),
                    );
                }
                target_caret.map(|caret| caret.point).or(textual_move(
                    state,
                    root,
                    focus,
                    logical_forward != current.rtl,
                    granularity,
                )?)
            } else {
                textual_move(state, root, focus, forward, granularity)?
            }
        }
    };
    let boundaries = if let Some(target) = target {
        Some((if alter == "move" { target } else { anchor }, target))
    } else if alter == "move" && anchor != focus {
        Some((focus, focus))
    } else {
        None
    };
    Ok(Some(PreparedMovement {
        boundaries,
        goal,
        caret: target_caret.map(|caret| (caret.line, caret.inline)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_movement_uses_viewport_distance_and_preserves_native_inline_goal() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=x>text</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("x").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(217), 1).unwrap();
        state.layout.borrow_mut().viewport = Some((300., 200.));
        let make = |offset, x, y, line| Caret {
            point: BoundaryPoint {
                container: node,
                offset,
            },
            inline: x,
            block: y,
            position: (x, y),
            inline_basis: (1., 0.),
            block_basis: (0., 1.),
            rtl: false,
            line: (node_key(node), line),
            paragraph: (node, 0),
            order: offset as usize,
        };
        let start = make(0, 20., 0., 0);
        let close = make(1, 20., 20., 1);
        let page = make(2, 20., 170., 2);
        let other_column = make(3, 70., 170., 2);
        let end = make(4, 20., 200., 3);
        let carets = [start, close, page, other_column, end];
        assert_eq!(
            page_caret(&state, &carets, start, start.position, true)
                .unwrap()
                .point,
            page.point
        );
        assert_eq!(
            page_caret(&state, &carets, end, end.position, false)
                .unwrap()
                .point,
            close.point
        );
        assert!(page_caret(&state, &carets, start, start.position, false).is_none());
    }

    #[test]
    fn pointer_word_units_use_utf16_and_keep_combining_sequences() {
        assert_eq!(pointer_word_span("Hi 😀 friend", 3), (3, 5));
        assert_eq!(pointer_word_span("Hi 😀 friend", 4), (3, 5));
        assert_eq!(pointer_word_span("Hi 😀 friend", 12), (6, 12));
        assert_eq!(pointer_word_span("e\u{301}lan", 1), (0, 5));
        assert_eq!(pointer_word_span("", 0), (0, 0));
    }

    #[test]
    fn native_point_caret_rejects_disabled_detached_and_nonfinite_targets() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<input id=enabled><input id=disabled disabled><div id=plain>text</div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let enabled = dom.borrow().find_by_id("enabled").unwrap();
        let disabled = dom.borrow().find_by_id("disabled").unwrap();
        let plain = dom.borrow().find_by_id("plain").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            pointer_caret(&state, enabled, 5., 5.).unwrap(),
            Some(BoundaryPoint {
                container: enabled,
                offset: 0
            })
        );
        assert_eq!(
            drop_caret(&state, enabled, 5., 5.).unwrap(),
            Some(BoundaryPoint {
                container: enabled,
                offset: 0
            })
        );
        assert!(pointer_caret(&state, disabled, 5., 5.).unwrap().is_none());
        assert!(
            pointer_caret(&state, enabled, f64::NAN, 5.)
                .unwrap()
                .is_none()
        );
        assert!(drop_caret(&state, plain, 5., 5.).unwrap().is_none());
        let source = v8::String::new(scope, "document.getElementById('enabled').remove()").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(pointer_caret(&state, enabled, 5., 5.).unwrap().is_none());
        state.retire_execution();
        assert!(pointer_caret(&state, plain, 5., 5.).unwrap().is_none());
    }

    #[test]
    fn pointer_selection_clips_to_the_near_edge_of_button_text() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><button interestfor=missing>ABC<span id=inside>DEF</span>GHI</button><div id=after>After</div></body>").into_runtime().unwrap()));
        let inside = dom.borrow().find_by_id("inside").unwrap();
        let after = dom.borrow().find_by_id("after").unwrap();
        let button = dom.borrow().parent(inside).unwrap().unwrap();
        let parent = dom.borrow().parent(button).unwrap().unwrap();
        let index = light_children(&dom.borrow(), parent)
            .unwrap()
            .iter()
            .position(|node| *node == button)
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(pointer_caret(&state, inside, 1., 1.).unwrap().is_none());
        assert_eq!(
            pointer_exclusion_boundary(
                &state,
                inside,
                BoundaryPoint {
                    container: after,
                    offset: 0
                }
            )
            .unwrap(),
            Some(BoundaryPoint {
                container: parent,
                offset: index as u32 + 1
            })
        );
        assert_eq!(
            pointer_exclusion_boundary(
                &state,
                inside,
                BoundaryPoint {
                    container: parent,
                    offset: 0
                }
            )
            .unwrap(),
            Some(BoundaryPoint {
                container: parent,
                offset: index as u32
            })
        );
    }

    #[test]
    fn sentence_and_grapheme_offsets_keep_utf16_boundaries() {
        assert_eq!(text_offsets("One. Two!", true, "sentence"), vec![5, 9]);
        assert_eq!(text_offsets("One. Two!", false, "sentence"), vec![0, 5]);
        assert_eq!(
            text_offsets("One. Two!", true, "sentenceboundary"),
            vec![4, 9]
        );
        assert_eq!(text_offsets("😀. Next.", true, "sentence"), vec![4, 9]);
        assert_eq!(text_offsets("a\u{301}👩‍💻", true, "character"), vec![0, 2, 7]);
        let lone = oryn_dom::dom_string::from_units(&[0xd800, 0x41, 0xdc00]);
        assert_eq!(text_offsets(&lone, true, "character"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn projected_line_matching_handles_rotation_and_orthogonal_candidate_axes() {
        let dom = html::parse("<div id=x>abc</div>").into_runtime().unwrap();
        let node = dom.find_by_id("x").unwrap();
        let matrix = [
            0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 100., 20., 0., 1.,
        ];
        assert_eq!(projected_basis((0., 0.), (1., 0.), matrix), Some((0., 1.)));
        assert_eq!(projected_basis((0., 0.), (0., 1.), matrix), Some((-1., 0.)));
        let current = Caret {
            point: BoundaryPoint {
                container: node,
                offset: 0,
            },
            inline: 24.,
            block: 100.,
            position: (100., 24.),
            inline_basis: (0., 1.),
            block_basis: (-1., 0.),
            rtl: false,
            line: (1, 0),
            paragraph: (node, 0),
            order: 0,
        };
        let mut candidate = current;
        candidate.position = (80., 24.);
        candidate.line = (2, 0);
        candidate.point.offset = 1;
        // Its own writing mode is orthogonal; selection compares in the source basis.
        candidate.inline_basis = (1., 0.);
        candidate.block_basis = (0., 1.);
        let mut wrong_column = candidate;
        wrong_column.position = (80., 50.);
        wrong_column.point.offset = 2;
        let selected =
            next_line(&[wrong_column, candidate], current, 24., 1., true, false).unwrap();
        assert_eq!(selected.point.offset, 1);
        assert!(next_line(&[candidate], current, 24., 1., false, false).is_none());
    }

    #[test]
    fn projected_axes_remove_shear_cross_axis_motion() {
        let matrix = [
            1., 0., 0., 0., 1., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let (inline, block) = projected_axes(
            (0., 0.),
            Axes {
                vertical: false,
                block_sign: 1.,
                inline_sign: 1.,
            },
            matrix,
        )
        .unwrap();
        // A source block displacement (0,20) projects to (20,20), with no inline drift.
        assert!((20. * inline.0 + 20. * inline.1).abs() < 1e-9);
        assert!(20. * block.0 + 20. * block.1 > 0.);
        assert!((10. * block.0).abs() < 1e-9);
        assert!(
            projected_axes(
                (0., 0.),
                Axes {
                    vertical: false,
                    block_sign: 1.,
                    inline_sign: 1.
                },
                [0.; 16]
            )
            .is_none()
        );
    }

    #[test]
    fn visual_character_uses_resolved_run_positions_inside_ltr_paragraph() {
        let dom = html::parse("<div id=x>אבג</div>").into_runtime().unwrap();
        let node = dom.find_by_id("x").unwrap();
        let carets: Vec<_> = (0..=3)
            .map(|offset| Caret {
                point: BoundaryPoint {
                    container: node,
                    offset,
                },
                inline: 30. - offset as f64 * 10.,
                block: 0.,
                position: (30. - offset as f64 * 10., 0.),
                inline_basis: (1., 0.),
                block_basis: (0., 1.),
                rtl: true,
                line: (1, 0),
                paragraph: (node, 0),
                order: 0,
            })
            .collect();
        assert_eq!(
            visual_character(&carets, carets[2], true, false)
                .unwrap()
                .point
                .offset,
            1
        );
        assert_eq!(
            visual_character(&carets, carets[1], false, false)
                .unwrap()
                .point
                .offset,
            2
        );
        assert!(visual_character(&carets, carets[0], true, false).is_none());
    }
    #[test]
    fn remembered_viewport_goal_is_reprojected_after_orthogonal_entry() {
        let dom = html::parse("<div id=x>abc</div>").into_runtime().unwrap();
        let node = dom.find_by_id("x").unwrap();
        let mut current = Caret {
            point: BoundaryPoint {
                container: node,
                offset: 0,
            },
            inline: 24.,
            block: 100.,
            position: (100., 24.),
            inline_basis: (0., 1.),
            block_basis: (-1., 0.),
            rtl: false,
            line: (1, 0),
            paragraph: (node, 0),
            order: 0,
        };
        let goal = current.position;
        assert_eq!(project_goal(goal, current), 24.);
        current.position = (80., 24.);
        current.inline_basis = (1., 0.);
        current.block_basis = (0., 1.);
        assert_eq!(project_goal(goal, current), 100.);
        let mut wanted = current;
        wanted.position = (100., 44.);
        wanted.line = (2, 1);
        wanted.point.offset = 1;
        let mut wrong = wanted;
        wrong.position = (24., 44.);
        wrong.point.offset = 2;
        assert_eq!(
            next_line(
                &[wrong, wanted],
                current,
                project_goal(goal, current),
                1.,
                true,
                false
            )
            .unwrap()
            .point
            .offset,
            1
        );
    }

    #[test]
    fn visual_soft_wrap_preserves_shared_boundary_line_affinity_in_both_directions() {
        let dom = html::parse("<div id=x>abcd</div>").into_runtime().unwrap();
        let node = dom.find_by_id("x").unwrap();
        let make = |offset, x, y, line| Caret {
            point: BoundaryPoint {
                container: node,
                offset,
            },
            inline: x,
            block: y,
            position: (x, y),
            inline_basis: (1., 0.),
            block_basis: (0., 1.),
            rtl: false,
            line: (1, line),
            paragraph: (node, 0),
            order: 0,
        };
        let carets = [
            make(0, 0., 0., 0),
            make(2, 20., 0., 0),
            make(2, 0., 20., 1),
            make(4, 20., 20., 1),
        ];
        let next = visual_wrap(&carets, carets[1], true, false).unwrap();
        assert_eq!(next.point, carets[1].point);
        assert_eq!(next.line, (1, 1));
        assert_eq!(
            visual_wrap(&carets, next, false, false).unwrap().line,
            (1, 0)
        );
        let rtl: Vec<_> = carets.iter().map(|c| Caret { rtl: true, ..*c }).collect();
        assert_eq!(visual_wrap(&rtl, rtl[0], false, true).unwrap().line, (1, 1));
        assert_eq!(visual_wrap(&rtl, rtl[3], true, true).unwrap().line, (1, 0));
    }
}

/// Read-only native geometry for the platform accessibility consumer. This uses
/// exactly the keyboard/pointer caret algorithms, with no author property reads.
pub(super) struct AccessibilityCaret {
    pub point: BoundaryPoint,
    pub position: (f64, f64),
    pub line: (u64, usize),
    pub rect: Option<oryn_common::v4::CssRect>,
}
fn accessibility_stop(
    layout: &BindingLayout,
    stop: &oryn_common::v4::CaretStopGeometry,
) -> Option<AccessibilityCaret> {
    let axis = axes(layout, stop.layout_node);
    let r = stop.rect;
    let origin = (r.x + r.width / 2., r.y + r.height / 2.);
    let position = projected(origin.0, origin.1, stop.matrix)?;
    let (_, _block_basis) = projected_axes(origin, axis, stop.matrix)?;
    let (a, b) = if axis.vertical {
        ((r.x, r.y), (r.right(), r.y))
    } else {
        ((r.x, r.y), (r.x, r.bottom()))
    };
    let a = projected(a.0, a.1, stop.matrix)?;
    let b = projected(b.0, b.1, stop.matrix)?;
    Some(AccessibilityCaret {
        point: BoundaryPoint {
            container: NodeId {
                slot: stop.boundary_node as u32,
                generation: (stop.boundary_node >> 32) as u32,
            },
            offset: u32::try_from(stop.offset).ok()?,
        },
        position,
        line: (stop.layout_node, stop.line),
        rect: Some(oryn_common::v4::CssRect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            width: (a.0 - b.0).abs(),
            height: (a.1 - b.1).abs(),
        }),
    })
}

/// Exact masked-control caret offset. Password paint can have one mask glyph
/// per UTF-16 unit, so secret grapheme segmentation must not suppress an input
/// method's native selection endpoint inside an astral source character.
pub(super) fn accessibility_control_caret_at(
    state: &V8BindingState,
    node: NodeId,
    offset: u32,
) -> Result<Option<AccessibilityCaret>, V8BindingError> {
    let empty = offset == 0 && state.dom.borrow().control_edit_value(node)?.is_empty();
    let layout = state.layout.borrow();
    if empty {
        return Ok(layout
            .caret_stops
            .iter()
            .find(|stop| stop.boundary_node == node_key(node) && stop.offset == 0)
            .and_then(|stop| accessibility_stop(&layout, stop)));
    }
    let Some(clusters) = layout.text_clusters.get(&node_key(node)) else {
        return Ok(None);
    };
    if clusters.len() > 262_144 {
        return Err(V8BindingError::Message(
            "private caret geometry budget exceeded".into(),
        ));
    }
    for cluster in clusters {
        if cluster.start > offset as usize
            || cluster.end < offset as usize
            || cluster.start >= cluster.end
        {
            continue;
        }
        let key = if layout.styles.contains_key(&cluster.layout_node) {
            cluster.layout_node
        } else {
            node_key(node)
        };
        let axis = axes(&layout, key);
        let fraction =
            (offset as usize - cluster.start) as f64 / (cluster.end - cluster.start) as f64;
        let fraction = if cluster.rtl ^ (style(&layout, key, "writing-mode") == Some("sideways-lr"))
        {
            1. - fraction
        } else {
            fraction
        };
        let r = cluster.rect;
        let (a, b) = if axis.vertical {
            (
                (r.x, r.y + r.height * fraction),
                (r.right(), r.y + r.height * fraction),
            )
        } else {
            (
                (r.x + r.width * fraction, r.y),
                (r.x + r.width * fraction, r.bottom()),
            )
        };
        let (Some(a), Some(b)) = (
            projected(a.0, a.1, cluster.matrix),
            projected(b.0, b.1, cluster.matrix),
        ) else {
            continue;
        };
        let rect = oryn_common::v4::CssRect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            width: (a.0 - b.0).abs(),
            height: (a.1 - b.1).abs(),
        };
        let origin = (r.x + r.width / 2., r.y + r.height / 2.);
        let Some(_) = projected_axes(origin, axis, cluster.matrix) else {
            continue;
        };
        return Ok(Some(AccessibilityCaret {
            point: BoundaryPoint {
                container: node,
                offset,
            },
            position: ((a.0 + b.0) / 2., (a.1 + b.1) / 2.),
            line: (cluster.layout_node, cluster.line),
            rect: Some(rect),
        }));
    }
    Ok(None)
}

pub(super) fn accessibility_carets(
    state: &V8BindingState,
    root: NodeId,
) -> Result<Vec<AccessibilityCaret>, V8BindingError> {
    let native = if state.dom.borrow().is_editable_text_control(root) {
        let value = state.dom.borrow().control_edit_value(root)?;
        control_carets(state, root, &value)
    } else {
        carets(state, root)?
    };
    let layout = state.layout.borrow();
    Ok(native
        .into_iter()
        .map(|caret| {
            let rect = layout
                .text_clusters
                .get(&node_key(caret.point.container))
                .and_then(|clusters| {
                    clusters.iter().find(|cluster| {
                        cluster.layout_node == caret.line.0
                            && cluster.line == caret.line.1
                            && cluster.start <= caret.point.offset as usize
                            && caret.point.offset as usize <= cluster.end
                    })
                })
                .and_then(|cluster| {
                    let key = if layout.styles.contains_key(&cluster.layout_node) {
                        cluster.layout_node
                    } else {
                        node_key(caret.point.container)
                    };
                    let axis = axes(&layout, key);
                    let fraction = (caret.point.offset as usize - cluster.start) as f64
                        / (cluster.end - cluster.start).max(1) as f64;
                    let fraction = if cluster.rtl
                        ^ (style(&layout, key, "writing-mode") == Some("sideways-lr"))
                    {
                        1. - fraction
                    } else {
                        fraction
                    };
                    let r = cluster.rect;
                    let (a, b) = if axis.vertical {
                        (
                            (r.x, r.y + r.height * fraction),
                            (r.right(), r.y + r.height * fraction),
                        )
                    } else {
                        (
                            (r.x + r.width * fraction, r.y),
                            (r.x + r.width * fraction, r.bottom()),
                        )
                    };
                    let a = projected(a.0, a.1, cluster.matrix)?;
                    let b = projected(b.0, b.1, cluster.matrix)?;
                    Some(oryn_common::v4::CssRect {
                        x: a.0.min(b.0),
                        y: a.1.min(b.1),
                        width: (a.0 - b.0).abs(),
                        height: (a.1 - b.1).abs(),
                    })
                });
            AccessibilityCaret {
                point: caret.point,
                position: caret.position,
                line: caret.line,
                rect,
            }
        })
        .collect())
}

/// UTF-16 editing text with native boundary affinity. Paragraph separators are
/// AX presentation only; every endpoint still names a real DOM boundary.
pub(super) struct AccessibilityText {
    pub objects: Vec<crate::accessibility::TextObject>,
    pub units: Vec<u16>,
    pub points: Vec<BoundaryPoint>,
    pub spans: Vec<(NodeId, usize, usize)>,
    pub end_overrides: BTreeMap<usize, BoundaryPoint>,
    source_offsets: BTreeMap<NodeId, Vec<usize>>,
}
impl AccessibilityText {
    pub fn end_point(&self, offset: usize) -> Option<BoundaryPoint> {
        self.end_overrides
            .get(&offset)
            .copied()
            .or_else(|| self.points.get(offset).copied())
    }
    pub fn offset(&self, point: BoundaryPoint) -> Option<usize> {
        if let Some(offsets) = self.source_offsets.get(&point.container) {
            return offsets
                .get(point.offset as usize)
                .copied()
                .filter(|offset| *offset != usize::MAX);
        }
        self.spans
            .iter()
            .find(|(node, start, end)| {
                *node == point.container && point.offset as usize <= end - start
            })
            .map(|(_, start, _)| start + point.offset as usize)
            .or_else(|| self.points.iter().position(|candidate| *candidate == point))
    }
}
pub(super) fn accessibility_editing_text(
    state: &V8BindingState,
    root: NodeId,
) -> Result<AccessibilityText, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let host = editing_host_for_node(
        &dom,
        Some(root),
        state.design_mode_enabled(),
        state.document_root(),
    )?;
    if host != Some(root) || dom.is_inert(root)? {
        return Err(V8BindingError::Message(
            "accessibility text requires a live editing host".into(),
        ));
    }
    let mut result = AccessibilityText {
        objects: Vec::new(),
        units: Vec::new(),
        points: vec![BoundaryPoint {
            container: root,
            offset: 0,
        }],
        spans: Vec::new(),
        end_overrides: BTreeMap::new(),
        source_offsets: BTreeMap::new(),
    };
    let mut paragraph = None;
    for node in traversal_nodes(&dom, root)? {
        if node == root
            || !selectable_text(&dom, &layout, node)?
            || editing_host_for_node(
                &dom,
                Some(node),
                state.design_mode_enabled(),
                state.document_root(),
            )? != host
        {
            continue;
        }
        if dom
            .arena()
            .ancestors(node)?
            .into_iter()
            .any(|ancestor| ancestor != root && dom.is_editable_text_control(ancestor))
        {
            continue;
        }
        let record = dom.arena().get(node)?;
        if matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="br")
        {
            let Some(parent) = dom.parent(node)? else {
                continue;
            };
            let children = light_children(&dom, parent)?;
            let Some(index) = children.iter().position(|child| *child == node) else {
                continue;
            };
            *result.points.last_mut().unwrap() = BoundaryPoint {
                container: parent,
                offset: index as u32,
            };
            result.units.push(10);
            result.points.push(BoundaryPoint {
                container: parent,
                offset: index as u32 + 1,
            });
            paragraph = None;
        } else if let NodeKind::Text { data } | NodeKind::CDataSection { data } = &record.kind {
            if data.len() > 4 * 1024 * 1024 {
                return Err(V8BindingError::Message(
                    "accessibility source text budget exceeded".into(),
                ));
            }
            let units = oryn_dom::dom_string::units(data);
            if units.is_empty() {
                continue;
            }
            if units.len() > 1024 * 1024 - result.units.len() {
                return Err(V8BindingError::Message(
                    "accessibility editing text budget exceeded".into(),
                ));
            }
            let current = paragraph_for(&dom, &layout, node, root)?;
            if paragraph.is_some_and(|previous| previous != current) && !result.units.is_empty() {
                result.units.push(10);
                result.points.push(BoundaryPoint {
                    container: node,
                    offset: 0,
                });
            }
            *result.points.last_mut().unwrap() = BoundaryPoint {
                container: node,
                offset: 0,
            };
            result
                .spans
                .push((node, result.units.len(), result.units.len() + units.len()));
            for (offset, unit) in units.into_iter().enumerate() {
                result.units.push(unit);
                result.points.push(BoundaryPoint {
                    container: node,
                    offset: offset as u32 + 1,
                });
            }
            paragraph = Some(current);
        }
        if result.units.len() > 1024 * 1024 {
            return Err(V8BindingError::Message(
                "accessibility editing text budget exceeded".into(),
            ));
        }
    }
    Ok(result)
}

// Table separators follow the actual formatting roles, not tag names when
// CSS has replaced their table display. Endpoints remain native cell boundaries.
pub(super) fn accessibility_paragraph_separator(
    dom: &DomRuntime,
    layout: &BindingLayout,
    previous: NodeId,
    current: NodeId,
) -> Result<u16, V8BindingError> {
    let ancestor = |mut node: NodeId,
                    display: &str,
                    tags: &[&str]|
     -> Result<Option<NodeId>, V8BindingError> {
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(node) {
                return Err(V8BindingError::Message(
                    "native AX table ancestry cycle".into(),
                ));
            }
            let matches = match style(layout, node_key(node), "display") {
                Some(actual) => actual == display,
                None => tags
                    .iter()
                    .any(|tag| dom.is_element(node, "http://www.w3.org/1999/xhtml", tag)),
            };
            if matches {
                return Ok(Some(node));
            }
            let Some(parent) = dom.flat_tree_parent(node)? else {
                return Ok(None);
            };
            node = parent;
        }
    };
    let Some(previous_cell) = ancestor(previous, "table-cell", &["td", "th"])? else {
        return Ok(10);
    };
    let Some(current_cell) = ancestor(current, "table-cell", &["td", "th"])? else {
        return Ok(10);
    };
    if previous_cell == current_cell {
        return Ok(10);
    }
    let previous_row = ancestor(previous_cell, "table-row", &["tr"])?;
    Ok(
        if previous_row.is_some() && previous_row == ancestor(current_cell, "table-row", &["tr"])? {
            9
        } else {
            10
        },
    )
}

/// Read-only AX text in native rendered-tree order. Replaced controls contribute
/// one object character; their values belong to their own protected/native AX
/// text objects. Shadow/slot traversal never changes DOM ownership of endpoints.
pub(super) fn accessibility_document_text(
    state: &V8BindingState,
    root: NodeId,
) -> Result<AccessibilityText, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut result = AccessibilityText {
        objects: Vec::new(),
        units: Vec::new(),
        points: vec![BoundaryPoint {
            container: root,
            offset: 0,
        }],
        spans: Vec::new(),
        end_overrides: BTreeMap::new(),
        source_offsets: BTreeMap::new(),
    };
    let resolved = crate::accessibility::resolved_nodes(&layout.resolved_text)
        .map_err(V8BindingError::Message)?;
    let mut stack = vec![root];
    let mut seen = BTreeSet::new();
    let mut paragraph = None;
    let empty_lines = layout
        .caret_stops
        .iter()
        .filter(|stop| stop.offset == 0)
        .map(|stop| stop.boundary_node)
        .collect::<BTreeSet<_>>();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            return Err(V8BindingError::Message(
                "accessibility flat tree cycle".into(),
            ));
        }
        if seen.len() > 262144 {
            return Err(V8BindingError::Message(
                "accessibility flat tree budget exceeded".into(),
            ));
        }
        if dom.node_document(node)? != Some(state.document_root()) {
            continue;
        }
        let record = dom.arena().get(node)?;
        let replaced = matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"input"|"textarea"|"select"|"img"|"iframe"|"object"|"embed"));
        if !replaced {
            stack.extend(dom.flat_tree_children(node)?.into_iter().rev());
        }
        let mut visible = !dom.is_inert(node)?;
        let mut ancestor = Some(node);
        let mut visited = BTreeSet::new();
        while let Some(current) = ancestor {
            if !visited.insert(current) {
                visible = false;
                break;
            }
            let current_node = dom.arena().get(current)?;
            if matches!(&current_node.kind,NodeKind::Element{name} if matches!(name.local.as_ref(),"script"|"style"|"head"|"template"))
                || current_node
                    .attributes
                    .get_null("aria-hidden")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                || current_node.attributes.contains_null("hidden")
                || matches!(
                    layout.geometry.get(&node_key(current)),
                    Some(BindingGeometry::Hidden)
                )
                || style(&layout, node_key(current), "display") == Some("none")
                || style(&layout, node_key(current), "visibility") == Some("hidden")
            {
                visible = false;
                break;
            }
            ancestor = dom.flat_tree_parent(current)?;
        }
        if !visible
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                && dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
        {
            continue;
        }
        let line_break = dom.is_element(node, "http://www.w3.org/1999/xhtml", "br");
        if replaced || line_break {
            let Some(parent) = dom.parent(node)? else {
                continue;
            };
            let Some(index) = dom
                .children(parent)?
                .iter()
                .position(|child| *child == node)
            else {
                continue;
            };
            *result.points.last_mut().unwrap() = BoundaryPoint {
                container: parent,
                offset: index as u32,
            };
            if replaced {
                result.objects.push(crate::accessibility::TextObject {
                    offset: result.units.len() as u32,
                    node,
                });
            }
            result.units.push(if line_break { 10 } else { 0xfffc });
            result.points.push(BoundaryPoint {
                container: parent,
                offset: index as u32 + 1,
            });
            if line_break {
                paragraph = None;
            }
        } else if matches!(&record.kind, NodeKind::Element { .. })
            && empty_lines.contains(&node_key(node))
            && dom.children(node)?.is_empty()
        {
            // Only a real shaped native empty-line stop creates this presentation line.
            // Inline empties without a line box contribute neither text nor fake metrics.
            let current = paragraph_for(&dom, &layout, node, root)?;
            if let Some(previous) = paragraph.filter(|previous| *previous != current) {
                result.units.push(accessibility_paragraph_separator(
                    &dom, &layout, previous, current,
                )?);
                result.points.push(BoundaryPoint {
                    container: node,
                    offset: 0,
                });
            }
            *result.points.last_mut().unwrap() = BoundaryPoint {
                container: node,
                offset: 0,
            };
            result.source_offsets.insert(node, vec![result.units.len()]);
            paragraph = Some(current);
        } else if let NodeKind::Text { data } | NodeKind::CDataSection { data } = &record.kind {
            if data.len() > 4 * 1024 * 1024 {
                return Err(V8BindingError::Message(
                    "accessibility source text budget exceeded".into(),
                ));
            }
            let raw_units = oryn_dom::dom_string::units(data);
            if raw_units.is_empty() {
                continue;
            }
            let mapped = resolved.get(&node_key(node));
            if !layout.resolved_text.is_empty() && mapped.is_none() {
                return Err(V8BindingError::Message(
                    "visible native AX text lacks shaping provenance".into(),
                ));
            }
            let units = mapped.map_or(raw_units.as_slice(), |mapped| mapped.units.as_slice());
            let current = paragraph_for(&dom, &layout, node, root)?;
            if !units.is_empty()
                && let Some(previous) = paragraph.filter(|previous| *previous != current)
            {
                result.units.push(accessibility_paragraph_separator(
                    &dom, &layout, previous, current,
                )?);
                result.points.push(BoundaryPoint {
                    container: node,
                    offset: 0,
                });
            }
            if units.len() > (1024 * 1024usize).saturating_sub(result.units.len()) {
                return Err(V8BindingError::Message(
                    "accessibility document text budget exceeded".into(),
                ));
            }
            let base = result.units.len();
            result.spans.push((node, base, base + units.len()));
            if let Some(mapped) = mapped {
                if mapped.offsets.len() != raw_units.len() + 1 {
                    return Err(V8BindingError::Message(
                        "native AX shaping source length changed".into(),
                    ));
                }
                *result.points.last_mut().unwrap() = BoundaryPoint {
                    container: node,
                    offset: mapped.starts[0],
                };
                for offset in mapped.starts.iter().skip(1) {
                    result.points.push(BoundaryPoint {
                        container: node,
                        offset: *offset,
                    });
                }
                for (index, offset) in mapped.ends.iter().enumerate() {
                    if *offset != mapped.starts[index] {
                        result.end_overrides.insert(
                            base + index,
                            BoundaryPoint {
                                container: node,
                                offset: *offset,
                            },
                        );
                    }
                }
                result.source_offsets.insert(
                    node,
                    mapped
                        .offsets
                        .iter()
                        .map(|offset| {
                            if *offset == usize::MAX {
                                usize::MAX
                            } else {
                                base + offset
                            }
                        })
                        .collect(),
                );
            } else {
                *result.points.last_mut().unwrap() = BoundaryPoint {
                    container: node,
                    offset: 0,
                };
                for offset in 0..units.len() {
                    result.points.push(BoundaryPoint {
                        container: node,
                        offset: offset as u32 + 1,
                    });
                }
            }
            result.units.extend_from_slice(units);
            if !units.is_empty() {
                paragraph = Some(current);
            }
        }
        if result.units.len() > 1024 * 1024 {
            return Err(V8BindingError::Message(
                "accessibility document text budget exceeded".into(),
            ));
        }
    }
    Ok(result)
}

pub(super) fn accessibility_document_carets(
    state: &V8BindingState,
    text: &AccessibilityText,
) -> Result<Vec<AccessibilityCaret>, V8BindingError> {
    let layout = state.layout.borrow();
    let mut result = Vec::new();
    for (node, _, _) in &text.spans {
        if let Some(clusters) = layout.text_clusters.get(&node_key(*node)) {
            for cluster in clusters {
                let key = if layout.styles.contains_key(&cluster.layout_node) {
                    cluster.layout_node
                } else {
                    node_key(*node)
                };
                let axis = axes(&layout, key);
                for (offset, fraction) in [(cluster.start, 0.), (cluster.end, 1.)] {
                    let fraction = if cluster.rtl
                        ^ (style(&layout, key, "writing-mode") == Some("sideways-lr"))
                    {
                        1. - fraction
                    } else {
                        fraction
                    };
                    let r = cluster.rect;
                    let (a, b) = if axis.vertical {
                        (
                            (r.x, r.y + r.height * fraction),
                            (r.right(), r.y + r.height * fraction),
                        )
                    } else {
                        (
                            (r.x + r.width * fraction, r.y),
                            (r.x + r.width * fraction, r.bottom()),
                        )
                    };
                    let (Some(a), Some(b)) = (
                        projected(a.0, a.1, cluster.matrix),
                        projected(b.0, b.1, cluster.matrix),
                    ) else {
                        continue;
                    };
                    let point = BoundaryPoint {
                        container: *node,
                        offset: u32::try_from(offset).map_err(|_| {
                            V8BindingError::Message("native AX caret offset overflow".into())
                        })?,
                    };
                    if text.offset(point).is_none() {
                        continue;
                    }
                    result.push(AccessibilityCaret {
                        point,
                        position: a,
                        line: (cluster.layout_node, cluster.line),
                        rect: Some(oryn_common::v4::CssRect {
                            x: a.0.min(b.0),
                            y: a.1.min(b.1),
                            width: (a.0 - b.0).abs(),
                            height: (a.1 - b.1).abs(),
                        }),
                    });
                    if result.len() > 262144 {
                        return Err(V8BindingError::Message(
                            "native AX caret budget exceeded".into(),
                        ));
                    }
                }
            }
        }
    }
    for stop in &layout.caret_stops {
        if let Some(caret) = accessibility_stop(&layout, stop)
            && text.offset(caret.point).is_some()
        {
            result.push(caret);
        }
        if result.len() > 262144 {
            return Err(V8BindingError::Message(
                "native AX caret budget exceeded".into(),
            ));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod projection_input_tests {
    #[test]
    fn both_projected_coordinates_use_the_original_input_and_same_w() {
        let matrix = [
            0., 1., 0., 0.5, -1., 0., 0., 0., 0., 0., 1., 0., 100., 20., 0., 1.,
        ];
        assert_eq!(super::projected(2., 4., matrix), Some((48., 11.)));
        assert_eq!(super::projected(-2., 4., matrix), None);
        let mut invalid = matrix;
        invalid[5] = f64::INFINITY;
        assert_eq!(super::projected(2., 4., invalid), None);
    }
}

#[cfg(test)]
mod bounded_pointer_word_tests {
    use super::*;
    #[test]
    fn pointer_word_context_preserves_existing_break_and_utf16_semantics() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=p>one<br>two<img>three<b>word</b></div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(91), 1).unwrap();
        let parent = state.dom.borrow().find_by_id("p").unwrap();
        let children = state.dom.borrow().children(parent).unwrap().to_vec();
        let point = BoundaryPoint {
            container: children[4],
            offset: 2,
        };
        let run = pointer_word_run(&state, point, parent).unwrap().unwrap();
        assert_eq!(run.text, "twothreeword");
        assert_eq!(run.focus_offset(point), Some(5));
        // A normal HTML div is an allowed shadow host; use the paragraph
        // itself so its separate attached root cannot enter light traversal.
        let code = v8::String::new(
            scope,
            "document.getElementById('p').attachShadow({mode:'open'}).textContent='FOREIGN';",
        )
        .unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let run = pointer_word_run(&state, point, parent).unwrap().unwrap();
        assert_eq!(run.text, "twothreeword");
        let text = children[4];
        state
            .dom
            .borrow_mut()
            .set_text(text, "x".repeat(1_048_577))
            .unwrap();
        let before = (state.selection_anchor.get(), state.selection_focus.get());
        assert!(pointer_word_run(&state, point, parent).is_ok());
        assert_eq!(
            (state.selection_anchor.get(), state.selection_focus.get()),
            before
        );
        let units = oryn_dom::dom_string::from_units(&[0xd800, 65]);
        state.dom.borrow_mut().set_text(text, units).unwrap();
        let run = pointer_word_run(&state, point, parent).unwrap().unwrap();
        assert!(run.text.starts_with("two\u{fffd}A"));
        assert_eq!(run.focus_offset(point), Some(5));
    }
}
