//! Bounded native table projection over the exposed accessibility tree. Sparse
//! ARIA coordinates remain sparse; nested tables never contribute outer cells.
use super::*;
#[path = "table_headers.rs"]
mod headers;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
}
#[derive(Clone, Debug)]
pub struct Table {
    header_model: Option<headers::Model>,
    pub cell_order: Vec<SemanticRef>,
    pub row_groups: BTreeMap<SemanticRef, SemanticRef>,
    pub column_groups: Vec<(usize, usize)>,
    pub header_scopes: BTreeMap<SemanticRef, String>,
    pub explicit_headers: BTreeMap<SemanticRef, Vec<SemanticRef>>,
    pub root: SemanticRef,
    pub rows: Vec<SemanticRef>,
    pub cells: BTreeMap<SemanticRef, Cell>,
    pub row_count: Option<usize>,
    pub column_count: Option<usize>,
    pub row_headers: Vec<SemanticRef>,
    pub column_headers: Vec<SemanticRef>,
}
fn integer(node: &Node, name: &str) -> Option<i64> {
    node.properties
        .get(name)
        .and_then(serde_json::Value::as_i64)
}
fn positive(node: &Node, name: &str) -> Option<usize> {
    integer(node, name)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0)
}
fn count(node: &Node, name: &str, inferred: usize) -> Option<usize> {
    match integer(node, name) {
        Some(-1) => None,
        Some(n) if n >= 0 => usize::try_from(n).ok().map(|n| n.max(inferred)),
        _ => Some(inferred),
    }
}
fn descendants(
    nodes: &BTreeMap<SemanticRef, Node>,
    root: SemanticRef,
    want_rows: bool,
) -> Vec<SemanticRef> {
    let mut result = Vec::new();
    let mut stack = nodes[&root]
        .children
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        if matches!(node.role.as_str(), "table" | "grid" | "treegrid") {
            continue;
        }
        if want_rows && node.role == "row" {
            result.push(id);
            continue;
        }
        if !want_rows
            && matches!(
                node.role.as_str(),
                "cell" | "gridcell" | "rowheader" | "columnheader"
            )
        {
            result.push(id);
            continue;
        }
        if !want_rows && node.role == "row" {
            continue;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    result
}
pub fn project(
    nodes: &BTreeMap<SemanticRef, Node>,
) -> Result<BTreeMap<SemanticRef, Table>, String> {
    let mut tables = BTreeMap::new();
    for (root, node) in nodes
        .iter()
        .filter(|(_, node)| matches!(node.role.as_str(), "table" | "grid" | "treegrid"))
    {
        let rows = descendants(nodes, *root, true);
        let mut table = Table {
            header_model: None,
            cell_order: Vec::new(),
            row_groups: BTreeMap::new(),
            column_groups: Vec::new(),
            header_scopes: BTreeMap::new(),
            explicit_headers: BTreeMap::new(),
            root: *root,
            rows: rows.clone(),
            cells: BTreeMap::new(),
            row_count: None,
            column_count: None,
            row_headers: Vec::new(),
            column_headers: Vec::new(),
        };
        let mut next_row = 0usize;
        let mut max_column = 0usize;
        let mut occupied: Vec<Cell> = Vec::new();
        let mut row_groups = BTreeMap::new();
        for (ordinal, row) in rows.iter().enumerate() {
            let row_node = &nodes[row];
            let row_index = positive(row_node, "rowindex")
                .map(|n| n - 1)
                .unwrap_or(next_row);
            next_row = row_index.checked_add(1).ok_or("table row overflow")?;
            let mut column = positive(row_node, "colindex").map(|n| n - 1).unwrap_or(0);
            occupied.retain(|cell| cell.row.saturating_add(cell.row_span) > row_index);
            for id in descendants(nodes, *row, false) {
                let cell_node = &nodes[&id];
                let column_span = if cell_node
                    .properties
                    .get("native_colspan_explicit")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    positive(cell_node, "native_colspan").unwrap_or(1)
                } else {
                    positive(cell_node, "colspan")
                        .or_else(|| positive(cell_node, "native_colspan"))
                        .unwrap_or(1)
                };
                let actual_row = positive(cell_node, "rowindex")
                    .map(|n| n - 1)
                    .unwrap_or(row_index);
                if let Some(index) = positive(cell_node, "colindex") {
                    column = index - 1;
                } else {
                    loop {
                        let end = occupied
                            .iter()
                            .filter(|cell| {
                                cell.row <= actual_row
                                    && actual_row < cell.row.saturating_add(cell.row_span)
                                    && cell.column <= column
                                    && column < cell.column.saturating_add(cell.column_span)
                            })
                            .map(|cell| cell.column + cell.column_span)
                            .max();
                        match end {
                            Some(end) => column = end,
                            None => break,
                        }
                    }
                }
                let row_span = if integer(cell_node, "native_rowspan") == Some(0) {
                    // HTML rowspan=0 ends at the owning row group, not at a nested table.
                    rows.iter()
                        .skip(ordinal)
                        .take_while(|row| nodes[*row].parent == row_node.parent)
                        .count()
                        .max(1)
                } else if cell_node
                    .properties
                    .get("native_rowspan_explicit")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    positive(cell_node, "native_rowspan").unwrap_or(1)
                } else {
                    positive(cell_node, "rowspan")
                        .or_else(|| positive(cell_node, "native_rowspan"))
                        .unwrap_or(1)
                };
                let end = column
                    .checked_add(column_span)
                    .ok_or("table column overflow")?;
                let end_row = actual_row
                    .checked_add(row_span)
                    .ok_or("table row span overflow")?;
                if end > 1_000_000 || end_row > 1_000_000 || table.cells.len() >= MAX_NODES {
                    return Err("accessibility table coordinate budget exceeded".into());
                }
                let cell = Cell {
                    row: actual_row,
                    column,
                    row_span,
                    column_span,
                };
                occupied.push(cell.clone());
                table.cells.insert(id, cell);
                table.cell_order.push(id);
                row_groups.insert(id, row_node.parent.unwrap_or(*root));
                max_column = max_column.max(end);
                column = end;
                match cell_node.role.as_str() {
                    "rowheader" => table.row_headers.push(id),
                    "columnheader" => table.column_headers.push(id),
                    _ => {}
                }
            }
        }
        for id in table.cells.keys() {
            if let Some(headers) = nodes[id]
                .properties
                .get("native_headers")
                .and_then(serde_json::Value::as_array)
            {
                let targets = headers
                    .iter()
                    .filter_map(serde_json::Value::as_u64)
                    .filter_map(|node| table.cells.keys().find(|id| id.node == node).copied())
                    .filter(|header| *header != *id)
                    .collect();
                table.explicit_headers.insert(*id, targets);
            }
        }
        let column_groups = node
            .properties
            .get("native_column_groups")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|group| {
                let group = group.as_array()?;
                let start = usize::try_from(group.first()?.as_u64()?).ok()?;
                let width = usize::try_from(group.get(1)?.as_u64()?).ok()?;
                Some((start, start.checked_add(width)?))
            })
            .collect::<Vec<_>>();
        table.row_groups = row_groups;
        table.column_groups = column_groups;
        for id in table.row_headers.iter().chain(&table.column_headers) {
            if let Some(scope) = nodes[id]
                .properties
                .get("native_scope")
                .and_then(serde_json::Value::as_str)
            {
                table.header_scopes.insert(*id, scope.into());
            }
        }
        let max_row = table
            .cells
            .values()
            .map(|cell| cell.row + cell.row_span)
            .max()
            .unwrap_or(next_row)
            .max(next_row);
        table.row_count = count(node, "rowcount", max_row);
        table.column_count = count(node, "colcount", max_column);
        if node
            .properties
            .get("native_table")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            let model = headers::build(&table, nodes)?;
            table.row_headers = table
                .cell_order
                .iter()
                .filter(|id| model.rows.contains(id))
                .copied()
                .collect();
            table.column_headers = table
                .cell_order
                .iter()
                .filter(|id| model.columns.contains(id))
                .copied()
                .collect();
            table.header_model = Some(model);
        }
        tables.insert(*root, table);
    }
    Ok(tables)
}
impl Table {
    pub fn cell_at(&self, row: usize, column: usize) -> Option<SemanticRef> {
        self.cells
            .iter()
            .find(|(_, cell)| {
                row >= cell.row
                    && row < cell.row + cell.row_span
                    && column >= cell.column
                    && column < cell.column + cell.column_span
            })
            .map(|(id, _)| *id)
    }
}

impl Table {
    pub fn headers_for(&self, id: SemanticRef, row_headers: bool) -> Vec<SemanticRef> {
        if let Some(model) = &self.header_model {
            return model.for_cell(id, row_headers);
        }
        let headers = if row_headers {
            &self.row_headers
        } else {
            &self.column_headers
        };
        let Some(subject) = self.cells.get(&id) else {
            return headers.clone();
        };
        headers
            .iter()
            .copied()
            .filter(|header| *header != id)
            .filter(|header| {
                if let Some(explicit) = self.explicit_headers.get(&id) {
                    return explicit.contains(header);
                }
                let Some(cell) = self.cells.get(header) else {
                    return false;
                };
                let row_overlap = cell.row < subject.row + subject.row_span
                    && subject.row < cell.row + cell.row_span;
                let column_overlap = cell.column < subject.column + subject.column_span
                    && subject.column < cell.column + cell.column_span;
                match self.header_scopes.get(header).map(String::as_str) {
                    Some("rowgroup") => self.row_groups.get(&id) == self.row_groups.get(header),
                    Some("colgroup") => self.column_groups.iter().any(|(start, end)| {
                        cell.column >= *start
                            && cell.column < *end
                            && subject.column < *end
                            && *start < subject.column + subject.column_span
                    }),
                    Some("row") => row_overlap,
                    Some("col") => column_overlap,
                    _ => {
                        if row_headers {
                            row_overlap
                        } else {
                            column_overlap
                        }
                    }
                }
            })
            .collect()
    }
}

impl Table {
    /// The rendered column set stays sparse even when ARIA reports a large total.
    /// Merge overlapping spans instead of allocating one entry per covered slot.
    pub fn column_intervals(&self) -> Vec<(usize, usize)> {
        let mut intervals = self
            .cells
            .values()
            .map(|cell| (cell.column, cell.column + cell.column_span))
            .collect::<Vec<_>>();
        intervals.sort_unstable();
        let mut result: Vec<(usize, usize)> = Vec::new();
        for (start, end) in intervals {
            if let Some(last) = result.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                result.push((start, end));
            }
        }
        result
    }
    pub fn column_at_ordinal(&self, mut ordinal: usize) -> Option<usize> {
        for (start, end) in self.column_intervals() {
            let length = end - start;
            if ordinal < length {
                return Some(start + ordinal);
            }
            ordinal = ordinal.checked_sub(length)?;
        }
        None
    }
    pub fn rendered_column_count(&self) -> usize {
        self.column_intervals()
            .iter()
            .map(|(start, end)| end - start)
            .sum()
    }
    pub fn cells_in_column(&self, column: usize) -> Vec<SemanticRef> {
        let mut cells = self
            .cells
            .iter()
            .filter(|(_, cell)| column >= cell.column && column < cell.column + cell.column_span)
            .collect::<Vec<_>>();
        cells.sort_by_key(|(id, cell)| (cell.row, cell.column, **id));
        cells.into_iter().map(|(id, _)| *id).collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_native_spans_precede_aria_but_missing_attributes_allow_aria() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        let cell = nodes.get_mut(&id(4)).unwrap();
        for (key, value) in [("colspan", 2), ("native_colspan", 3)] {
            cell.properties.insert(key.into(), serde_json::json!(value));
        }
        cell.properties
            .insert("native_colspan_explicit".into(), true.into());
        assert_eq!(
            project(&nodes).unwrap()[&id(1)].cells[&id(4)].column_span,
            3
        );
        nodes
            .get_mut(&id(4))
            .unwrap()
            .properties
            .insert("native_colspan_explicit".into(), false.into());
        assert_eq!(
            project(&nodes).unwrap()[&id(1)].cells[&id(4)].column_span,
            2
        );
    }
    use super::*;
    fn nodes() -> BTreeMap<SemanticRef, Node> {
        let template = super::super::tests::fixture()
            .nodes
            .into_values()
            .next()
            .unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..template.id
        };
        let mut nodes = BTreeMap::new();
        for (n, role, parent, children) in [
            (1, "table", None, vec![2, 3]),
            (2, "row", Some(1), vec![4, 5]),
            (3, "row", Some(1), vec![6]),
            (4, "rowheader", Some(2), vec![]),
            (5, "cell", Some(2), vec![7]),
            (6, "cell", Some(3), vec![]),
            (7, "table", Some(5), vec![8]),
            (8, "row", Some(7), vec![9]),
            (9, "cell", Some(8), vec![]),
        ] {
            nodes.insert(
                id(n),
                Node {
                    id: id(n),
                    role: role.into(),
                    parent: parent.map(id),
                    children: children.into_iter().map(id).collect(),
                    ..template.clone()
                },
            );
        }
        nodes
    }
    #[test]
    fn row_spans_and_nested_table_cells_keep_independent_coordinates() {
        let mut nodes = nodes();
        let id = |n| SemanticRef {
            node: n,
            ..nodes.keys().next().copied().unwrap()
        };
        let root = id(1);
        let header = id(4);
        let second = id(6);
        let nested = id(7);
        let inner = id(9);
        nodes
            .get_mut(&header)
            .unwrap()
            .properties
            .insert("native_rowspan".into(), serde_json::json!(2));
        let tables = project(&nodes).unwrap();
        assert_eq!(tables[&root].cells[&second].column, 1);
        assert_eq!(tables[&root].cell_at(1, 0), Some(header));
        assert!(!tables[&root].cells.contains_key(&inner));
        assert_eq!(tables[&nested].cell_at(0, 0), Some(inner));
    }
    #[test]
    fn sparse_aria_coordinates_and_unknown_counts_do_not_allocate_dense_grids() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        nodes
            .get_mut(&id(1))
            .unwrap()
            .properties
            .insert("rowcount".into(), serde_json::json!(-1));
        nodes
            .get_mut(&id(3))
            .unwrap()
            .properties
            .insert("rowindex".into(), serde_json::json!(900_000));
        nodes
            .get_mut(&id(6))
            .unwrap()
            .properties
            .insert("colindex".into(), serde_json::json!(800_000));
        let tables = project(&nodes).unwrap();
        assert_eq!(tables[&id(1)].row_count, None);
        assert_eq!(tables[&id(1)].cell_at(899_999, 799_999), Some(id(6)));
        assert_eq!(tables[&id(1)].cells.len(), 3);
    }
    #[test]
    fn zero_rowspan_stops_at_native_row_group_boundary() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        nodes
            .get_mut(&id(4))
            .unwrap()
            .properties
            .insert("native_rowspan".into(), serde_json::json!(0));
        let tables = project(&nodes).unwrap();
        assert_eq!(tables[&id(1)].cells[&id(4)].row_span, 2);
    }
    #[test]
    fn native_rowgroup_header_does_not_escape_its_group() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        nodes
            .get_mut(&id(4))
            .unwrap()
            .properties
            .insert("native_scope".into(), "rowgroup".into());
        let tables = project(&nodes).unwrap();
        assert_eq!(tables[&id(1)].headers_for(id(6), true), vec![id(4)]);
        let template = nodes[&id(2)].clone();
        let group = Node {
            id: id(10),
            role: "rowgroup".into(),
            parent: Some(id(1)),
            children: vec![id(3)],
            ..template
        };
        nodes.insert(id(10), group);
        nodes.get_mut(&id(1)).unwrap().children = vec![id(2), id(10)];
        nodes.get_mut(&id(3)).unwrap().parent = Some(id(10));
        let tables = project(&nodes).unwrap();
        assert!(tables[&id(1)].headers_for(id(6), true).is_empty());
    }
    #[test]
    fn native_column_group_scope_covers_real_group_and_explicit_empty_suppresses_it() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        nodes.get_mut(&id(4)).unwrap().role = "columnheader".into();
        nodes
            .get_mut(&id(4))
            .unwrap()
            .properties
            .insert("native_scope".into(), "colgroup".into());
        nodes
            .get_mut(&id(1))
            .unwrap()
            .properties
            .insert("native_column_groups".into(), serde_json::json!([[0, 2]]));
        let tables = project(&nodes).unwrap();
        assert_eq!(tables[&id(1)].headers_for(id(5), false), vec![id(4)]);
        nodes
            .get_mut(&id(5))
            .unwrap()
            .properties
            .insert("native_headers".into(), serde_json::json!([]));
        let tables = project(&nodes).unwrap();
        assert!(tables[&id(1)].headers_for(id(5), false).is_empty());
    }

    #[test]
    fn sparse_columns_merge_spans_without_expanding_coordinate_domain() {
        let mut nodes = nodes();
        let identity = *nodes.keys().next().unwrap();
        let id = |n| SemanticRef {
            node: n,
            ..identity
        };
        nodes
            .get_mut(&id(6))
            .unwrap()
            .properties
            .insert("colindex".into(), serde_json::json!(800_000));
        nodes
            .get_mut(&id(6))
            .unwrap()
            .properties
            .insert("colspan".into(), serde_json::json!(3));
        let tables = project(&nodes).unwrap();
        let table = &tables[&id(1)];
        assert_eq!(table.column_intervals(), vec![(0, 2), (799_999, 800_002)]);
        assert_eq!(table.rendered_column_count(), 5);
        assert_eq!(table.column_at_ordinal(2), Some(799_999));
        assert_eq!(table.column_at_ordinal(5), None);
        assert_eq!(table.cells_in_column(800_000), vec![id(6)]);
    }

    fn native_grid(
        pattern: &[&[bool]],
    ) -> (
        BTreeMap<SemanticRef, Node>,
        SemanticRef,
        Vec<Vec<SemanticRef>>,
    ) {
        let template = super::super::tests::fixture()
            .nodes
            .into_values()
            .next()
            .unwrap();
        let root = SemanticRef {
            node: 100,
            ..template.id
        };
        let mut nodes = BTreeMap::new();
        let mut root_node = Node {
            id: root,
            role: "table".into(),
            parent: None,
            children: Vec::new(),
            ..template.clone()
        };
        root_node
            .properties
            .insert("native_table".into(), true.into());
        let mut rows = Vec::new();
        let mut next = 101;
        for pattern in pattern {
            let row = SemanticRef { node: next, ..root };
            next += 1;
            root_node.children.push(row);
            let mut row_node = Node {
                id: row,
                role: "row".into(),
                parent: Some(root),
                children: Vec::new(),
                ..template.clone()
            };
            let mut cells = Vec::new();
            for header in *pattern {
                let id = SemanticRef { node: next, ..root };
                next += 1;
                let mut cell = Node {
                    id,
                    role: if *header { "columnheader" } else { "cell" }.into(),
                    parent: Some(row),
                    children: Vec::new(),
                    ..template.clone()
                };
                cell.properties
                    .insert("native_header".into(), (*header).into());
                cell.properties
                    .insert("native_empty_cell".into(), false.into());
                row_node.children.push(id);
                cells.push(id);
                nodes.insert(id, cell);
            }
            nodes.insert(row, row_node);
            rows.push(cells);
        }
        nodes.insert(root, root_node);
        (nodes, root, rows)
    }
    #[test]
    fn native_auto_headers_distinguish_header_rows_from_header_columns() {
        let (nodes, root, cells) = native_grid(&[
            &[true, true, true],
            &[true, false, false],
            &[true, false, false],
        ]);
        let tables = project(&nodes).unwrap();
        let table = &tables[&root];
        assert_eq!(table.headers_for(cells[1][1], false), vec![cells[0][1]]);
        assert_eq!(table.headers_for(cells[1][1], true), vec![cells[1][0]]);
        assert_eq!(table.headers_for(cells[2][2], false), vec![cells[0][2]]);
        assert_eq!(table.headers_for(cells[2][2], true), vec![cells[2][0]]);
    }
    #[test]
    fn native_header_blocks_are_opaque_and_explicit_headers_override_scanning() {
        let (mut nodes, root, cells) = native_grid(&[&[true], &[false], &[true], &[false]]);
        for row in [0, 2] {
            nodes
                .get_mut(&cells[row][0])
                .unwrap()
                .properties
                .insert("native_scope".into(), "col".into());
        }
        let tables = project(&nodes).unwrap();
        assert_eq!(
            tables[&root].headers_for(cells[3][0], false),
            vec![cells[2][0]]
        );
        nodes.get_mut(&cells[3][0]).unwrap().properties.insert(
            "native_headers".into(),
            serde_json::json!([cells[0][0].node]),
        );
        let tables = project(&nodes).unwrap();
        assert_eq!(
            tables[&root].headers_for(cells[3][0], false),
            vec![cells[0][0]]
        );
        nodes
            .get_mut(&cells[0][0])
            .unwrap()
            .properties
            .insert("native_empty_cell".into(), true.into());
        let tables = project(&nodes).unwrap();
        assert!(tables[&root].headers_for(cells[3][0], false).is_empty());
    }
    #[test]
    fn ambiguous_overlapping_header_slots_are_skipped() {
        let (mut nodes, root, cells) = native_grid(&[&[true], &[true, true], &[false]]);
        nodes
            .get_mut(&cells[1][1])
            .unwrap()
            .properties
            .insert("colindex".into(), serde_json::json!(1));
        let tables = project(&nodes).unwrap();
        assert_eq!(
            tables[&root].headers_for(cells[2][0], false),
            vec![cells[0][0]]
        );
    }
}
