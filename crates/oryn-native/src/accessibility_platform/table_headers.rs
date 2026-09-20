//! HTML table header assignment, following the native table-model algorithm at
//! https://html.spec.whatwg.org/multipage/tables.html#header-and-data-cell-semantics
//! Coordinate bands replace repeated identical slot scans; overlapping slots are
//! skipped exactly as the specification requires, not arbitrarily attributed.

/// Inputs for scan.
struct HeaderScan<'a> {
    table: &'a Table,
    nodes: &'a BTreeMap<SemanticRef, Node>,
    model: &'a Model,
    principal: SemanticRef,
    fixed: usize,
    horizontal: bool,
    result: &'a mut Vec<SemanticRef>,
    budget: &'a mut Budget,
}
use super::*;
#[derive(Clone, Debug, Default)]
pub(super) struct Model {
    pub rows: BTreeSet<SemanticRef>,
    pub columns: BTreeSet<SemanticRef>,
    associations: BTreeMap<SemanticRef, Vec<SemanticRef>>,
}
impl Model {
    pub fn for_cell(&self, id: SemanticRef, row: bool) -> Vec<SemanticRef> {
        let axis = if row { &self.rows } else { &self.columns };
        self.associations.get(&id).map_or_else(
            || axis.iter().copied().collect(),
            |headers| {
                headers
                    .iter()
                    .copied()
                    .filter(|header| axis.contains(header) || (!row && !self.rows.contains(header)))
                    .collect()
            },
        )
    }
}
struct Budget(usize);
impl Budget {
    fn charge(&mut self, count: usize) -> Result<(), String> {
        self.0 = self
            .0
            .checked_add(count)
            .ok_or("native header work overflow")?;
        if self.0 > 8_388_608 {
            Err("native header assignment work budget exceeded".into())
        } else {
            Ok(())
        }
    }
}
fn overlaps(a: usize, width: usize, b: usize, other: usize) -> bool {
    a < b + other && b < a + width
}
fn scope(nodes: &BTreeMap<SemanticRef, Node>, id: SemanticRef) -> Option<&str> {
    nodes[&id]
        .properties
        .get("native_scope")
        .and_then(serde_json::Value::as_str)
}
fn is_header(nodes: &BTreeMap<SemanticRef, Node>, id: SemanticRef) -> bool {
    nodes[&id]
        .properties
        .get("native_header")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}
fn empty(nodes: &BTreeMap<SemanticRef, Node>, id: SemanticRef) -> bool {
    nodes[&id]
        .properties
        .get("native_empty_cell")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}
fn bands(table: &Table, start: usize, end: usize, horizontal: bool) -> Vec<usize> {
    let mut values = BTreeSet::from([start]);
    for cell in table.cells.values() {
        let (a, b) = if horizontal {
            (cell.row, cell.row + cell.row_span)
        } else {
            (cell.column, cell.column + cell.column_span)
        };
        for edge in [a, b] {
            if edge > start && edge < end {
                values.insert(edge);
            }
        }
    }
    values.into_iter().collect()
}
fn scan(params: HeaderScan<'_>) -> Result<(), String> {
    let HeaderScan {
        table,
        nodes,
        model,
        principal,
        fixed,
        horizontal,
        result,
        budget,
    } = params;

    let principal_cell = &table.cells[&principal];
    let limit = if horizontal {
        principal_cell.column
    } else {
        principal_cell.row
    };
    if limit == 0 {
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut edges = BTreeSet::from([0, limit]);
    budget.charge(table.cells.len())?;
    for (id, cell) in &table.cells {
        let (cross, width, start, end) = if horizontal {
            (
                cell.row,
                cell.row_span,
                cell.column,
                cell.column + cell.column_span,
            )
        } else {
            (
                cell.column,
                cell.column_span,
                cell.row,
                cell.row + cell.row_span,
            )
        };
        if fixed >= cross && fixed < cross + width && start < limit {
            candidates.push((*id, start, end));
            edges.insert(start);
            edges.insert(end.min(limit));
        }
    }
    let edges = edges.into_iter().collect::<Vec<_>>();
    let mut opaque = Vec::new();
    let mut block = if is_header(nodes, principal) {
        vec![principal]
    } else {
        Vec::new()
    };
    let mut in_block = !block.is_empty();
    let mut previous = None;
    for segment in edges.windows(2).rev() {
        if segment[0] == segment[1] {
            continue;
        }
        let position = segment[1] - 1;
        budget.charge(candidates.len())?;
        let covering = candidates
            .iter()
            .filter(|(_, start, end)| *start <= position && position < *end)
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>();
        if covering.len() != 1 {
            continue;
        }
        let current = covering[0];
        if previous == Some(current) {
            continue;
        }
        previous = Some(current);
        if is_header(nodes, current) {
            in_block = true;
            block.push(current);
            let cell = &table.cells[&current];
            let axis = if horizontal {
                &model.rows
            } else {
                &model.columns
            };
            let blocked = opaque.iter().any(|id| {
                let old = &table.cells[id];
                if horizontal {
                    old.row == cell.row && old.row_span == cell.row_span
                } else {
                    old.column == cell.column && old.column_span == cell.column_span
                }
            });
            if !blocked
                && axis.contains(&current)
                && !matches!(scope(nodes, current), Some("rowgroup" | "colgroup"))
            {
                result.push(current);
            }
        } else if in_block {
            in_block = false;
            opaque.append(&mut block);
        }
    }
    Ok(())
}
pub(super) fn build(table: &Table, nodes: &BTreeMap<SemanticRef, Node>) -> Result<Model, String> {
    let mut model = Model::default();
    let mut budget = Budget(0);
    let headers = table
        .cell_order
        .iter()
        .copied()
        .filter(|id| is_header(nodes, *id))
        .collect::<Vec<_>>();
    for id in &headers {
        let cell = &table.cells[id];
        budget.charge(table.cells.len())?;
        let column = scope(nodes, *id) == Some("col")
            || (scope(nodes, *id).is_none()
                && !table.cells.iter().any(|(other, candidate)| {
                    !is_header(nodes, *other)
                        && overlaps(cell.row, cell.row_span, candidate.row, candidate.row_span)
                }));
        let row = scope(nodes, *id) == Some("row")
            || (scope(nodes, *id).is_none()
                && !column
                && !table.cells.iter().any(|(other, candidate)| {
                    !is_header(nodes, *other)
                        && overlaps(
                            cell.column,
                            cell.column_span,
                            candidate.column,
                            candidate.column_span,
                        )
                }));
        if row || scope(nodes, *id) == Some("rowgroup") {
            model.rows.insert(*id);
        }
        if column || scope(nodes, *id) == Some("colgroup") {
            model.columns.insert(*id);
        }
    }
    let mut references = 0usize;
    for principal in &table.cell_order {
        let subject = &table.cells[principal];
        let mut list = Vec::new();
        if let Some(explicit) = table.explicit_headers.get(principal) {
            list.extend(explicit.iter().copied());
        } else if !headers.is_empty() {
            budget.charge(table.cells.len().saturating_mul(2))?;
            for row in bands(table, subject.row, subject.row + subject.row_span, true) {
                scan(HeaderScan {
                    table,
                    nodes,
                    model: &model,
                    principal: *principal,
                    fixed: row,
                    horizontal: true,
                    result: &mut list,
                    budget: &mut budget,
                })?;
            }
            for column in bands(
                table,
                subject.column,
                subject.column + subject.column_span,
                false,
            ) {
                scan(HeaderScan {
                    table,
                    nodes,
                    model: &model,
                    principal: *principal,
                    fixed: column,
                    horizontal: false,
                    result: &mut list,
                    budget: &mut budget,
                })?;
            }
            for header in &headers {
                budget.charge(1)?;
                let cell = &table.cells[header];
                if cell.column > subject.column + subject.column_span - 1
                    || cell.row > subject.row + subject.row_span - 1
                {
                    continue;
                }
                let row_group = scope(nodes, *header) == Some("rowgroup")
                    && table.row_groups.get(principal).filter(|group| {
                        nodes.get(group).is_some_and(|node| node.role == "rowgroup")
                    }) == table.row_groups.get(header).filter(|group| {
                        nodes.get(group).is_some_and(|node| node.role == "rowgroup")
                    })
                    && table.row_groups.get(principal).is_some_and(|group| {
                        nodes.get(group).is_some_and(|node| node.role == "rowgroup")
                    });
                let column_group = scope(nodes, *header) == Some("colgroup")
                    && table.column_groups.iter().any(|(start, end)| {
                        subject.column >= *start
                            && subject.column < *end
                            && cell.column >= *start
                            && cell.column < *end
                    });
                if row_group || column_group {
                    list.push(*header);
                }
            }
        }
        let mut seen = BTreeSet::new();
        list.retain(|header| {
            *header != *principal && !empty(nodes, *header) && seen.insert(*header)
        });
        references = references
            .checked_add(list.len())
            .ok_or("native header reference overflow")?;
        if references > 1_048_576 {
            return Err("native header references exceed budget".into());
        }
        model.associations.insert(*principal, list);
    }
    model.rows.retain(|id| !empty(nodes, *id));
    model.columns.retain(|id| !empty(nodes, *id));
    Ok(model)
}
