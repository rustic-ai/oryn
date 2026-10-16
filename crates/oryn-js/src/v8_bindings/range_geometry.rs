//! CSSOM View geometry for live DOM ranges.
use super::*;
use oryn_common::v4::{CssRect, TextClusterGeometry};

fn project(rect: CssRect, matrix: [f64; 16]) -> Option<BindingRect> {
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for (x, y) in [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ] {
        let w = matrix[3] * x + matrix[7] * y + matrix[15];
        if w == 0. || !w.is_finite() {
            return None;
        }
        let px = (matrix[0] * x + matrix[4] * y + matrix[12]) / w;
        let py = (matrix[1] * x + matrix[5] * y + matrix[13]) / w;
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        left = left.min(px);
        right = right.max(px);
        top = top.min(py);
        bottom = bottom.max(py);
    }
    Some(BindingRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn text_rects(clusters: &[TextClusterGeometry], start: usize, end: usize) -> Vec<BindingRect> {
    if start == end {
        let Some(first) = clusters
            .iter()
            .find(|cluster| start >= cluster.start && start <= cluster.end)
        else {
            return Vec::new();
        };
        let at_end = start == first.end;
        // A single source character can expand into several shaping clusters
        // (for example uppercase sharp s). Its end boundary follows the entire
        // expansion, including a possible wrap between generated characters.
        let same_source = |cluster: &&TextClusterGeometry| {
            cluster.start == first.start
                && cluster.end == first.end
                && cluster.layout_node == first.layout_node
        };
        let line = if at_end {
            clusters
                .iter()
                .filter(same_source)
                .map(|cluster| cluster.line)
                .max()
        } else {
            clusters
                .iter()
                .filter(same_source)
                .map(|cluster| cluster.line)
                .min()
        }
        .unwrap_or(first.line);
        let Some(anchor) = clusters
            .iter()
            .filter(same_source)
            .find(|cluster| cluster.line == line)
        else {
            return Vec::new();
        };
        let mut rect = anchor.rect;
        for cluster in clusters.iter().filter(same_source).filter(|cluster| {
            cluster.line == line && cluster.matrix == anchor.matrix && cluster.rtl == anchor.rtl
        }) {
            let right = rect.right().max(cluster.rect.right());
            rect.x = rect.x.min(cluster.rect.x);
            rect.width = right - rect.x;
        }
        rect.x += if at_end != anchor.rtl { rect.width } else { 0. };
        rect.width = 0.;
        return project(rect, anchor.matrix).into_iter().collect();
    }
    let mut groups: Vec<(CssRect, [f64; 16], usize, bool)> = Vec::new();
    for cluster in clusters {
        let rect = cluster.rect;
        if cluster.end <= start || cluster.start >= end {
            continue;
        }
        if let Some((previous, matrix, line, rtl)) = groups.last_mut()
            && *matrix == cluster.matrix
            && *line == cluster.line
            && *rtl == cluster.rtl
            && (previous.y - rect.y).abs() < 1e-6
            && (previous.height - rect.height).abs() < 1e-6
            && ((previous.right() - rect.x).abs() < 1e-6
                || (rect.right() - previous.x).abs() < 1e-6)
        {
            let right = previous.right().max(rect.right());
            previous.x = previous.x.min(rect.x);
            previous.width = right - previous.x;
            continue;
        }
        groups.push((rect, cluster.matrix, cluster.line, cluster.rtl));
    }
    groups
        .into_iter()
        .filter_map(|(rect, matrix, _, _)| project(rect, matrix))
        .collect()
}

struct IndexedNode {
    before: usize,
    content_start: usize,
    content_end: usize,
    after: usize,
    children: Vec<NodeId>,
    character_data: bool,
}
struct BoundaryIndex {
    nodes: BTreeMap<NodeId, IndexedNode>,
    order: Vec<NodeId>,
}
impl BoundaryIndex {
    fn new(dom: &DomRuntime, root: NodeId) -> Result<Self, V8BindingError> {
        let mut result = Self {
            nodes: BTreeMap::new(),
            order: Vec::new(),
        };
        let mut stack = vec![(root, false)];
        let mut cursor = 0usize;
        while let Some((node, exit)) = stack.pop() {
            if exit {
                let entry = result.nodes.get_mut(&node).expect("indexed entry");
                entry.content_end = cursor;
                cursor += 1;
                entry.after = cursor;
                continue;
            }
            if result.nodes.contains_key(&node) {
                continue;
            }
            let children = light_children(dom, node)?;
            let character_data = matches!(
                dom.arena().get(node)?.kind,
                NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::Comment { .. }
                    | NodeKind::ProcessingInstruction { .. }
            );
            let before = cursor;
            cursor += 1;
            let content_start = cursor;
            if character_data {
                cursor += boundary_length(dom, node)? as usize;
            }
            stack.push((node, true));
            stack.extend(children.iter().rev().map(|child| (*child, false)));
            result.order.push(node);
            result.nodes.insert(
                node,
                IndexedNode {
                    before,
                    content_start,
                    content_end: 0,
                    after: 0,
                    children,
                    character_data,
                },
            );
        }
        Ok(result)
    }
    fn boundary(&self, node: NodeId, offset: u32) -> Option<usize> {
        let entry = self.nodes.get(&node)?;
        if entry.character_data {
            let value = entry.content_start.checked_add(offset as usize)?;
            (value <= entry.content_end).then_some(value)
        } else if let Some(child) = entry.children.get(offset as usize) {
            self.nodes.get(child).map(|child| child.before)
        } else {
            (offset as usize == entry.children.len()).then_some(entry.content_end)
        }
    }
}

pub(super) fn rects(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    range: &RangeRecord,
) -> Result<Vec<BindingRect>, V8BindingError> {
    // Detached ranges have no layout boxes, even if the previous document's
    // snapshot still contains clusters for these stable node identities.
    if !state.dom.borrow().is_connected(range.start_container)? {
        return Ok(Vec::new());
    }
    refresh_layout_in_scope(scope, state)?;
    let context = state.realm_context.to_local(scope)
        .ok_or(V8BindingError::Allocation("range geometry owner context expired"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let _retained_scope = state.enter_retained_allocation(scope)?;
    let dom = state.dom.borrow();
    let root = shadow_including_tree_root(&dom, range.start_container)?;
    if !matches!(dom.arena().get(root)?.kind, NodeKind::Document) {
        return Ok(Vec::new());
    }
    let layout = state.layout.borrow();
    let start = (range.start_container, range.start_offset);
    let end = (range.end_container, range.end_offset);
    let collapsed = start == end;
    let index = BoundaryIndex::new(&dom, common_ancestor(&dom, start.0, end.0)?)?;
    let start_position = index
        .boundary(start.0, start.1)
        .ok_or(V8BindingError::Allocation("Range start geometry boundary"))?;
    let end_position = index
        .boundary(end.0, end.1)
        .ok_or(V8BindingError::Allocation("Range end geometry boundary"))?;
    let mut selected_elements = BTreeSet::new();
    let mut result = Vec::new();
    for node in index.order.iter().copied() {
        let indexed = &index.nodes[&node];
        match &dom.arena().get(node)?.kind {
            NodeKind::Element { .. } if !collapsed => {
                let selected = start_position <= indexed.before && indexed.after <= end_position;
                if !selected {
                    continue;
                }
                selected_elements.insert(node);
                if dom
                    .parent(node)?
                    .is_some_and(|parent| selected_elements.contains(&parent))
                {
                    continue;
                }
                match layout.geometry.get(&node_key(node)) {
                    Some(BindingGeometry::Available(rect)) => result.extend(
                        layout
                            .client_rects
                            .get(&node_key(node))
                            .cloned()
                            .unwrap_or_else(|| vec![*rect]),
                    ),
                    Some(BindingGeometry::Unsupported(detail)) => {
                        return Err(V8BindingError::Unsupported {
                            interface: "Range".into(),
                            member: format!("client rectangle geometry: {detail}"),
                        });
                    }
                    _ => {}
                }
            }
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                let length = data.encode_utf16().count() as u32;
                if collapsed && node != start.0 {
                    continue;
                }
                if !collapsed
                    && (end_position <= indexed.content_start
                        || start_position >= indexed.content_end)
                {
                    continue;
                }
                let first = if node == start.0 { start.1 } else { 0 };
                let last = if node == end.0 { end.1 } else { length };
                if let Some(clusters) = layout.text_clusters.get(&node_key(node)) {
                    result.extend(text_rects(clusters, first as usize, last as usize));
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expanded_character_selection_and_caret_cover_all_generated_clusters() {
        let make = |x, y, line, rtl| TextClusterGeometry {
            layout_node: 1,
            start: 0,
            end: 1,
            rect: CssRect {
                x,
                y,
                width: 10.,
                height: 16.,
            },
            matrix: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            line,
            rtl,
        };
        let same_line = [make(0., 0., 0, false), make(10., 0., 0, false)];
        let selection = text_rects(&same_line, 0, 1);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection[0].width, 20.);
        assert_eq!(text_rects(&same_line, 0, 0)[0].x, 0.);
        assert_eq!(text_rects(&same_line, 1, 1)[0].x, 20.);
        let wrapped = [make(30., 0., 0, false), make(0., 16., 1, false)];
        assert_eq!(text_rects(&wrapped, 0, 1).len(), 2);
        let caret = text_rects(&wrapped, 1, 1);
        assert_eq!((caret[0].x, caret[0].y), (10., 16.));
        let rtl = [make(0., 0., 0, true), make(10., 0., 0, true)];
        assert_eq!(text_rects(&rtl, 0, 0)[0].x, 20.);
        assert_eq!(text_rects(&rtl, 1, 1)[0].x, 0.);
    }

    #[test]
    fn indexed_boundaries_match_dom_order_across_ancestors_and_utf16_offsets() {
        let dom = oryn_dom::html::parse("<main>a<b>😀</b><!--note-->c<i></i></main>")
            .into_runtime()
            .unwrap();
        let root = dom.query_selector(dom.document(), "main").unwrap().unwrap();
        let index = BoundaryIndex::new(&dom, root).unwrap();
        let mut boundaries = Vec::new();
        for node in &index.order {
            for offset in 0..=boundary_length(&dom, *node).unwrap() {
                boundaries.push((*node, offset));
            }
        }
        for left in &boundaries {
            for right in &boundaries {
                let actual = index
                    .boundary(left.0, left.1)
                    .unwrap()
                    .cmp(&index.boundary(right.0, right.1).unwrap());
                let expected = compare_boundary_points(&dom, *left, *right)
                    .unwrap()
                    .cmp(&0);
                assert_eq!(actual, expected, "{left:?} versus {right:?}");
            }
        }
        assert_eq!(index.boundary(root, 99), None);
    }

    #[test]
    fn text_selection_merges_on_lines_before_transforming_and_handles_carets() {
        let matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 20., 30., 0., 1.,
        ];
        let cluster = |start, end, x, y, line, rtl| TextClusterGeometry {
            layout_node: 1,
            start,
            end,
            rect: CssRect {
                x,
                y,
                width: 10.,
                height: 16.,
            },
            matrix,
            line,
            rtl,
        };
        let clusters = vec![
            cluster(0, 1, 0., 0., 0, false),
            cluster(1, 3, 10., 0., 0, false),
            cluster(3, 4, 0., 16., 1, false),
        ];
        let rects = text_rects(&clusters, 0, 4);
        assert_eq!(rects.len(), 2);
        assert_eq!((rects[0].x, rects[0].y, rects[0].width), (20., 30., 20.));
        assert_eq!((rects[1].y, rects[1].width), (46., 10.));
        let partial = text_rects(&clusters, 2, 3);
        assert_eq!((partial[0].x, partial[0].width), (30., 10.));
        let caret = text_rects(&clusters, 1, 1);
        assert_eq!((caret[0].x, caret[0].width), (30., 0.));
        let rtl = text_rects(&[cluster(0, 1, 0., 0., 0, true)], 0, 0);
        assert_eq!((rtl[0].x, rtl[0].width), (30., 0.));
    }
}
