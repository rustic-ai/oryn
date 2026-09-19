//! Native AX column objects, distinct from DOM/semantic identities. Only columns
//! represented by native cells are materialized; ARIA's total count stays separate.
use super::*;
pub(super) type Key = (SemanticRef, usize);
#[derive(Clone)]
struct Column {
    owner: Id,
    index: usize,
    cells: Vec<Id>,
    visible_cells: Vec<Id>,
    headers: Vec<Id>,
    screen: Option<Rect>,
    window: Id,
    identity: String,
}
thread_local! {static COLUMNS:RefCell<BTreeMap<usize,Column>>=RefCell::new(BTreeMap::new());}
pub(super) fn prepare(snapshot: &Snapshot) -> Result<Vec<Key>, String> {
    let references = snapshot
        .tables
        .values()
        .flat_map(|table| table.cells.values())
        .try_fold(0usize, |total, cell| total.checked_add(cell.column_span))
        .ok_or("accessibility column reference budget overflow")?;
    if references > 1_048_576 {
        return Err("accessibility column cell references exceed budget".into());
    }
    let total = snapshot
        .tables
        .values()
        .try_fold(snapshot.nodes.len(), |total, table| {
            total.checked_add(table.rendered_column_count())
        })
        .ok_or("accessibility column budget overflow")?;
    if total > MAX_NODES {
        return Err("accessibility nodes and represented columns exceed object budget".into());
    }
    let mut result = Vec::with_capacity(total - snapshot.nodes.len());
    for table in snapshot.tables.values() {
        for (start, end) in table.column_intervals() {
            result.extend((start..end).map(|column| (table.root, column)));
        }
    }
    Ok(result)
}
fn read<T>(object: Id, default: T, f: impl FnOnce(&Column) -> T) -> T {
    COLUMNS.with(|entries| {
        entries
            .try_borrow()
            .ok()
            .and_then(|entries| entries.get(&(object as usize)).map(f))
            .unwrap_or(default)
    })
}
unsafe extern "C" fn names(_: Id, _: Sel) -> Id {
    unsafe {
        strings(&[
            "AXRole",
            "AXRoleDescription",
            "AXParent",
            "AXChildren",
            "AXVisibleChildren",
            "AXIndex",
            "AXColumnIndexRange",
            "AXHeader",
            "AXColumnHeaderUIElements",
            "AXPosition",
            "AXSize",
            "AXWindow",
            "AXTopLevelUIElement",
            "AXIdentifier",
            "AXEnabled",
        ])
    }
}
unsafe extern "C" fn attribute(object: Id, _: Sel, name: Id) -> Id {
    let Some(name) = (unsafe { rust_string(name) }) else {
        return std::ptr::null_mut();
    };
    read(object, std::ptr::null_mut(), |column| unsafe {
        match name.as_str() {
            "AXRole" => string("AXColumn"),
            "AXRoleDescription" => string("column"),
            "AXParent" => column.owner,
            "AXChildren" => array(&column.cells),
            "AXVisibleChildren" => array(&column.visible_cells),
            "AXIndex" => integer(column.index),
            "AXColumnIndexRange" => range(Range {
                location: column.index,
                length: 1,
            }),
            "AXHeader" => column
                .headers
                .first()
                .copied()
                .unwrap_or(std::ptr::null_mut()),
            "AXColumnHeaderUIElements" => array(&column.headers),
            "AXPosition" => column
                .screen
                .map_or(std::ptr::null_mut(), |rect| point(rect.origin)),
            "AXSize" => column
                .screen
                .map_or(std::ptr::null_mut(), |rect| size(rect.size)),
            "AXWindow" | "AXTopLevelUIElement" => column.window,
            "AXIdentifier" => string(&column.identity),
            "AXEnabled" => number(true),
            _ => std::ptr::null_mut(),
        }
    })
}
unsafe extern "C" fn ignored(object: Id, _: Sel) -> i8 {
    read(object, 1, |_| 0)
}
unsafe extern "C" fn element(object: Id, _: Sel) -> i8 {
    read(object, 0, |_| 1)
}
unsafe extern "C" fn settable(_: Id, _: Sel, _: Id) -> i8 {
    0
}
unsafe extern "C" fn actions(_: Id, _: Sel) -> Id {
    unsafe { array(&[]) }
}
unsafe fn register() -> Result<Id, String> {
    unsafe {
        let existing = class("OrynAccessibilityColumn");
        if !existing.is_null() {
            return Ok(existing);
        }
        let name = CString::new("OrynAccessibilityColumn").unwrap();
        let kind = objc_allocateClassPair(class("NSAccessibilityElement"), name.as_ptr(), 0);
        if kind.is_null() {
            return Err("cannot allocate native column class".into());
        }
        for (name, method, encoding) in [
            ("accessibilityAttributeNames", names as *const c_void, "@@:"),
            (
                "accessibilityAttributeValue:",
                attribute as *const c_void,
                "@@:@",
            ),
            ("accessibilityIsIgnored", ignored as *const c_void, "c@:"),
            ("isAccessibilityElement", element as *const c_void, "c@:"),
            (
                "accessibilityIsAttributeSettable:",
                settable as *const c_void,
                "c@:@",
            ),
            ("accessibilityActionNames", actions as *const c_void, "@@:"),
        ] {
            let encoding = if cfg!(target_arch = "aarch64") && encoding.starts_with('c') {
                format!("B{}", &encoding[1..])
            } else {
                encoding.into()
            };
            let encoding = CString::new(encoding).unwrap();
            if !class_addMethod(kind, sel(name), method, encoding.as_ptr()) {
                return Err(format!("cannot install column method {name}"));
            }
        }
        objc_registerClassPair(kind);
        Ok(kind)
    }
}
pub(super) unsafe fn retire(objects: &mut BTreeMap<Key, Id>) {
    unsafe {
        for (_, object) in std::mem::take(objects) {
            COLUMNS.with(|entries| {
                entries.borrow_mut().remove(&(object as usize));
            });
            NSAccessibilityPostNotification(object, string("AXUIElementDestroyed"));
            void(object, "release");
        }
    }
}
pub(super) unsafe fn publish(
    objects: &mut BTreeMap<Key, Id>,
    keys: Vec<Key>,
    nodes: &BTreeMap<SemanticRef, Id>,
    snapshot: &Snapshot,
) -> Result<(), String> {
    unsafe {
        let kind = register()?;
        let keep = keys.iter().copied().collect::<BTreeSet<_>>();
        let removed = objects
            .keys()
            .filter(|key| !keep.contains(key))
            .copied()
            .collect::<Vec<_>>();
        let mut retired = BTreeMap::new();
        for key in removed {
            retired.insert(key, objects.remove(&key).unwrap());
        }
        retire(&mut retired);
        for key in keys {
            let table = &snapshot.tables[&key.0];
            let owner = *nodes.get(&key.0).ok_or("native table owner missing")?;
            let cells = table
                .cells_in_column(key.1)
                .iter()
                .filter_map(|id| nodes.get(id).copied())
                .collect::<Vec<_>>();
            let headers = table
                .column_headers
                .iter()
                .filter(|id| {
                    table.cells.get(id).is_some_and(|cell| {
                        key.1 >= cell.column && key.1 < cell.column + cell.column_span
                    })
                })
                .filter_map(|id| nodes.get(id).copied())
                .collect();
            let visible_cells = ENTRIES.with(|entries| {
                let entries = entries.borrow();
                cells
                    .iter()
                    .copied()
                    .filter(|cell| {
                        entries.get(&(*cell as usize)).is_some_and(|entry| {
                            entry.screen.is_some_and(|rect| {
                                rect.origin.x + rect.size.width > entry.root_origin.x
                                    && rect.origin.x < entry.root_origin.x + snapshot.viewport.width
                                    && rect.origin.y + rect.size.height > entry.root_origin.y
                                    && rect.origin.y
                                        < entry.root_origin.y + snapshot.viewport.height
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            });
            let (screen, window) = ENTRIES.with(|entries| {
                let entries = entries.borrow();
                let mut bounds: Option<Rect> = None;
                for cell in &cells {
                    if let Some(rect) = entries
                        .get(&(*cell as usize))
                        .and_then(|entry| entry.screen)
                    {
                        bounds = Some(match bounds {
                            None => rect,
                            Some(old) => {
                                let right = (old.origin.x + old.size.width)
                                    .max(rect.origin.x + rect.size.width);
                                let top = (old.origin.y + old.size.height)
                                    .max(rect.origin.y + rect.size.height);
                                let x = old.origin.x.min(rect.origin.x);
                                let y = old.origin.y.min(rect.origin.y);
                                Rect {
                                    origin: Point { x, y },
                                    size: Size {
                                        width: right - x,
                                        height: top - y,
                                    },
                                }
                            }
                        });
                    }
                }
                (
                    bounds,
                    entries
                        .get(&(owner as usize))
                        .map_or(std::ptr::null_mut(), |entry| entry.window),
                )
            });
            let object = if let Some(object) = objects.get(&key) {
                *object
            } else {
                let object = object(object(kind, "alloc"), "init");
                if object.is_null() {
                    return Err("native column allocation failed".into());
                }
                objects.insert(key, object);
                object
            };
            let identity = format!(
                "oryn-column-{}-{}-{}-{}",
                key.0.page_id.0, key.0.document_generation, key.0.node, key.1
            );
            COLUMNS.with(|entries| {
                entries.borrow_mut().insert(
                    object as usize,
                    Column {
                        owner,
                        index: key.1,
                        cells,
                        visible_cells,
                        headers,
                        screen,
                        window,
                        identity,
                    },
                );
            });
        }
        Ok(())
    }
}
pub(super) unsafe fn for_table(owner: Id, visible: bool) -> Id {
    unsafe {
        COLUMNS.with(|entries| {
            let entries = entries.borrow();
            let mut columns = entries
                .iter()
                .filter(|(_, column)| {
                    column.owner == owner && (!visible || !column.visible_cells.is_empty())
                })
                .collect::<Vec<_>>();
            columns.sort_by_key(|(_, column)| column.index);
            array(
                &columns
                    .into_iter()
                    .map(|(object, _)| *object as Id)
                    .collect::<Vec<_>>(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_column_callbacks_have_no_native_authority() {
        assert_eq!(read(0x1234usize as Id, 0, |_| 1), 0);
        unsafe {
            assert_eq!(ignored(0x1234usize as Id, std::ptr::null_mut()), 1);
            assert_eq!(element(0x1234usize as Id, std::ptr::null_mut()), 0);
        }
    }
    #[test]
    fn represented_column_objects_are_bounded_separately_from_aria_total() {
        let snapshot = super::super::super::tests::fixture();
        assert!(prepare(&snapshot).unwrap().is_empty());
    }
}
