//! Native AX control text and selection. No author getter participates.
use super::*;
use crate::accessibility::{TextCommand, TextSnapshot};
#[path = "native_accessibility_receipt.rs"]
mod receipt;
pub use receipt::Receipt as NativeAccessibilityReceipt;
fn live(state: &V8BindingState, target: NodeId, identity: (NodeId, u64, u64)) -> bool {
    !state.execution_retired()
        && (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
        ) == identity
        && state.dom.borrow().is_connected(target).unwrap_or(false)
        && state.dom.borrow().node_document(target).ok().flatten() == Some(identity.0)
}
pub fn text(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    command: &TextCommand,
) -> Result<TextSnapshot, V8BindingError> {
    if !matches!(command, TextCommand::Read) {
        let mut operation = NativeAccessibilityReceipt::new(state, target, command)?;
        return operation.poll(scope)?.ok_or(V8BindingError::Allocation(
            "synchronous AX adapter suspended",
        ));
    }
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    if !live(state, target, identity) {
        return Err(V8BindingError::Message(
            "stale accessibility text target".into(),
        ));
    }
    refresh_layout(state)?;
    if !state.dom.borrow().is_editable_text_control(target) {
        return editing_text(scope, state, target, command, identity);
    }
    let (value, protected) = {
        let dom = state.dom.borrow();
        if !dom.is_editable_text_control(target) || dom.is_inert(target)? {
            return Err(V8BindingError::Message(
                "accessibility text target is not an exposed control".into(),
            ));
        }
        let protected = dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
            && dom
                .attribute_in_null_namespace(target, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("password"));
        let value = if protected {
            Vec::new()
        } else {
            oryn_dom::dom_string::units(&dom.control_edit_value(target)?)
        };
        (value, protected)
    };
    if value.len() > 1024 * 1024 {
        return Err(V8BindingError::Message(
            "accessibility text exceeds snapshot budget".into(),
        ));
    }
    let default = value.len().min(u32::MAX as usize) as u32;
    let (start, end, _) = state
        .text_control_selections
        .borrow()
        .get(&target)
        .cloned()
        .unwrap_or((default, default, "none".into()));
    let mut boxes = Vec::new();
    if !protected {
        if let Some(clusters) = state.layout.borrow().text_clusters.get(&node_key(target)) {
            if clusters.len() > 262_144 {
                return Err(V8BindingError::Message(
                    "accessibility text geometry budget exceeded".into(),
                ));
            }
            for cluster in clusters {
                if cluster.start > cluster.end || cluster.end > value.len() {
                    continue;
                }
                let r = cluster.rect;
                let m = cluster.matrix;
                let mut points = Vec::new();
                for (x, y) in [
                    (r.x, r.y),
                    (r.right(), r.y),
                    (r.right(), r.bottom()),
                    (r.x, r.bottom()),
                ] {
                    let w = m[3] * x + m[7] * y + m[15];
                    if w > 0. && w.is_finite() {
                        let point = (
                            (m[0] * x + m[4] * y + m[12]) / w,
                            (m[1] * x + m[5] * y + m[13]) / w,
                        );
                        if point.0.is_finite() && point.1.is_finite() {
                            points.push(point);
                        }
                    }
                }
                if points.len() != 4 {
                    continue;
                }
                let left = points
                    .iter()
                    .map(|point| point.0)
                    .fold(f64::INFINITY, f64::min);
                let right = points
                    .iter()
                    .map(|point| point.0)
                    .fold(f64::NEG_INFINITY, f64::max);
                let top = points
                    .iter()
                    .map(|point| point.1)
                    .fold(f64::INFINITY, f64::min);
                let bottom = points
                    .iter()
                    .map(|point| point.1)
                    .fold(f64::NEG_INFINITY, f64::max);
                boxes.push(crate::accessibility::TextBox {
                    start: cluster.start as u32,
                    end: cluster.end as u32,
                    line: cluster.line,
                    rect: oryn_common::v4::CssRect {
                        x: left,
                        y: top,
                        width: right - left,
                        height: bottom - top,
                    },
                });
            }
        }
    }
    let carets = if protected {
        Vec::new()
    } else {
        selection_movement::accessibility_carets(state, target)?
            .into_iter()
            .filter(|caret| {
                caret.point.container == target && caret.point.offset as usize <= value.len()
            })
            .map(|caret| crate::accessibility::TextCaret {
                offset: caret.point.offset,
                x: caret.position.0,
                y: caret.position.1,
                line: caret.line.1,
                rect: caret.rect,
            })
            .collect()
    };
    let protected_caret = if protected && state.dom.borrow().focused() == Some(target) {
        let selection = crate::composition::TextRange {
            start,
            length: end.saturating_sub(start),
        };
        let offset = state
            .text_control_selections
            .borrow()
            .get(&target)
            .map_or(end, |selection| {
                if selection.2 == "backward" {
                    start
                } else {
                    end
                }
            });
        selection_movement::accessibility_control_caret_at(state, target, offset)?.map(|caret| {
            crate::accessibility::ProtectedCaret {
                selection,
                mutation_revision: state.dom.borrow().mutation_cursor().0,
                caret: crate::accessibility::TextCaret {
                    offset,
                    x: caret.position.0,
                    y: caret.position.1,
                    line: caret.line.1,
                    rect: caret.rect,
                },
            }
        })
    } else {
        None
    };
    Ok(TextSnapshot {
        objects: Vec::new(),
        control: (!protected).then_some(target),
        boundary_revision: state.dom.borrow().boundary_revision(),
        boundaries: Vec::new(),
        protected_caret,
        value,
        start: if protected { 0 } else { start },
        end: if protected { 0 } else { end },
        selection_revision: state.selection_revision.get(),
        protected,
        boxes,
        carets,
    })
}

fn editing_text(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    _command: &TextCommand,
    _identity: (NodeId, u64, u64),
) -> Result<TextSnapshot, V8BindingError> {
    synchronize_selection_range(state)?;
    let editing = {
        let dom = state.dom.borrow();
        editing_host_for_node(
            &dom,
            Some(target),
            state.design_mode_enabled(),
            state.document_root(),
        )? == Some(target)
    };
    let read = |state: &V8BindingState| {
        if editing && state.layout.borrow().resolved_text.is_empty() {
            selection_movement::accessibility_editing_text(state, target)
        } else {
            selection_movement::accessibility_document_text(state, target)
        }
    };
    let text = read(state)?;
    let start = state
        .selection_anchor
        .get()
        .and_then(|point| text.offset(point))
        .unwrap_or(0) as u32;
    let end = state
        .selection_focus
        .get()
        .and_then(|point| text.offset(point))
        .unwrap_or(start as usize) as u32;
    let mut line_ids = BTreeMap::new();
    let mut carets = Vec::new();
    for caret in if editing {
        selection_movement::accessibility_carets(state, target)?
    } else {
        selection_movement::accessibility_document_carets(state, &text)?
    } {
        if let Some(offset) = text.offset(caret.point) {
            let next = line_ids.len();
            let line = *line_ids.entry(caret.line).or_insert(next);
            carets.push(crate::accessibility::TextCaret {
                offset: offset as u32,
                x: caret.position.0,
                y: caret.position.1,
                line,
                rect: caret.rect,
            });
        }
    }
    let mut boxes = Vec::new();
    let layout = state.layout.borrow();
    for (node, _, _) in &text.spans {
        if let Some(clusters) = layout.text_clusters.get(&node_key(*node)) {
            for cluster in clusters {
                if cluster.start > cluster.end {
                    continue;
                }
                let (Some(start), Some(end)) = (
                    text.offset(BoundaryPoint {
                        container: *node,
                        offset: cluster.start as u32,
                    }),
                    text.offset(BoundaryPoint {
                        container: *node,
                        offset: cluster.end as u32,
                    }),
                ) else {
                    continue;
                };
                if start >= end {
                    continue;
                }
                let Some(rect) = project_rect(cluster.rect, cluster.matrix) else {
                    continue;
                };
                let next = line_ids.len();
                let line = *line_ids
                    .entry((cluster.layout_node, cluster.line))
                    .or_insert(next);
                boxes.push(crate::accessibility::TextBox {
                    start: start as u32,
                    end: end as u32,
                    line,
                    rect,
                });
                if boxes.len() > 262_144 {
                    return Err(V8BindingError::Message(
                        "accessibility text geometry budget exceeded".into(),
                    ));
                }
            }
        }
    }

    let boundaries = text
        .points
        .iter()
        .enumerate()
        .map(|(offset, start)| crate::accessibility::SourceBoundary {
            start: *start,
            end: text.end_point(offset).unwrap_or(*start),
        })
        .collect();
    Ok(TextSnapshot {
        objects: text.objects,
        control: None,
        boundary_revision: state.dom.borrow().boundary_revision(),
        boundaries,
        protected_caret: None,
        value: text.units,
        start: start.min(end),
        end: start.max(end),
        selection_revision: state.selection_revision.get(),
        protected: false,
        boxes,
        carets,
    })
}

fn project_rect(
    rect: oryn_common::v4::CssRect,
    matrix: [f64; 16],
) -> Option<oryn_common::v4::CssRect> {
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
        if !w.is_finite() || w <= 0. {
            return None;
        }
        let x1 = (matrix[0] * x + matrix[4] * y + matrix[12]) / w;
        let y1 = (matrix[1] * x + matrix[5] * y + matrix[13]) / w;
        if !x1.is_finite() || !y1.is_finite() {
            return None;
        }
        left = left.min(x1);
        right = right.max(x1);
        top = top.min(y1);
        bottom = bottom.max(y1);
    }
    Some(oryn_common::v4::CssRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn resolved_document_text_uses_native_collapse_and_expansion_source_boundaries() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=d><span id=a>a   </span><span id=b>ß</span></div>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("d").unwrap();
        let a = dom.borrow().find_by_id("a").unwrap();
        let b = dom.borrow().find_by_id("b").unwrap();
        let first = dom.borrow().children(a).unwrap()[0];
        let second = dom.borrow().children(b).unwrap()[0];
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let span = |node, source_start, source_end, rendered_start, rendered_end, transformed| {
            oryn_common::v4::ResolvedTextSpan {
                owner: node_key(target),
                source_node: Some(node_key(node)),
                source_start,
                source_end,
                rendered_start,
                rendered_end,
                transformed,
                atomic: false,
            }
        };
        state.set_layout(BindingLayout {
            resolved_text: vec![oryn_common::v4::ResolvedTextRun {
                layout_node: node_key(target),
                text: "a SS".into(),
                spans: vec![
                    span(first, 0, 1, 0, 1, false),
                    span(first, 1, 4, 1, 2, true),
                    span(second, 0, 1, 2, 4, true),
                ],
                aliases: vec![],
            }],
            ..Default::default()
        });
        let read = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert_eq!(String::from_utf16(&read.value).unwrap(), "a SS");
        text(
            scope,
            &state,
            target,
            &TextCommand::Select {
                start: 2,
                end: 3,
                expected_value: read.value,
                expected_selection_revision: read.selection_revision,
            },
        )
        .unwrap();
        eval(
            scope,
            "const selected=getSelection();if(selected.toString()!=='SS'||selected.getRangeAt(0).toString()!=='ß'||selected.anchorNode!==b.firstChild||selected.anchorOffset!==0||selected.focusNode!==b.firstChild||selected.focusOffset!==1||a.textContent!=='a   '||b.textContent!=='ß')throw Error('native transformed endpoint or source mutation')",
        );
    }
    #[test]
    fn document_text_preserves_native_empty_block_lines_without_inventing_inline_lines() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=first></div><p>a</p><div id=middle></div><p>b</p><span></span><div id=last></div></body>").into_runtime().unwrap()));
        let empty = ["first", "middle", "last"].map(|id| dom.borrow().find_by_id(id).unwrap());
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        state.set_layout(BindingLayout {
            caret_stops: empty
                .iter()
                .enumerate()
                .map(|(line, node)| oryn_common::v4::CaretStopGeometry {
                    boundary_node: node_key(*node),
                    offset: 0,
                    layout_node: node_key(*node),
                    rect: oryn_common::v4::CssRect {
                        x: 0.,
                        y: line as f64 * 20.,
                        width: 0.,
                        height: 16.,
                    },
                    matrix,
                    line,
                    rtl: false,
                })
                .collect(),
            ..Default::default()
        });
        let result = text(scope, &state, state.document_root(), &TextCommand::Read).unwrap();
        assert_eq!(
            result.value,
            "\na\n\nb\n".encode_utf16().collect::<Vec<_>>()
        );
        assert!(result.carets.iter().any(|caret| caret.offset == 0));
        assert!(
            result
                .carets
                .iter()
                .any(|caret| caret.offset == result.value.len() as u32)
        );
        state.set_layout(BindingLayout::default());
        let result = text(scope, &state, state.document_root(), &TextCommand::Read).unwrap();
        assert_eq!(result.value, "a\nb".encode_utf16().collect::<Vec<_>>());
    }
    #[test]
    fn document_table_text_uses_native_cell_and_row_separators() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<table><tr id=r1><td id=a><div>A</div></td><td id=b></td></tr><tr id=r2><td id=c>C</td><td id=d>D</td></tr></table>").into_runtime().unwrap()));
        let cells = ["a", "b", "c", "d"].map(|id| dom.borrow().find_by_id(id).unwrap());
        let rows = ["r1", "r2"].map(|id| dom.borrow().find_by_id(id).unwrap());
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let mut styles = BTreeMap::new();
        for node in cells {
            styles.insert(
                node_key(node),
                std::sync::Arc::new(BTreeMap::from([("display".into(), "table-cell".into())])),
            );
        }
        for node in rows {
            styles.insert(
                node_key(node),
                std::sync::Arc::new(BTreeMap::from([("display".into(), "table-row".into())])),
            );
        }
        let matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        state.set_layout(BindingLayout {
            styles,
            caret_stops: vec![oryn_common::v4::CaretStopGeometry {
                boundary_node: node_key(cells[1]),
                offset: 0,
                layout_node: node_key(cells[1]),
                rect: oryn_common::v4::CssRect {
                    x: 30.,
                    y: 0.,
                    width: 0.,
                    height: 16.,
                },
                matrix,
                line: 0,
                rtl: false,
            }],
            ..Default::default()
        });
        let result = text(scope, &state, state.document_root(), &TextCommand::Read).unwrap();
        assert_eq!(result.value, "A\t\nC\tD".encode_utf16().collect::<Vec<_>>());
    }
    #[test]
    fn protected_text_keeps_only_private_focused_caret_geometry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=p type=password value=secret><input id=other>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("p").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "p.focus();p.setSelectionRange(3,3)");
        let matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        state.set_layout(BindingLayout {
            text_clusters: BTreeMap::from([(
                node_key(target),
                (0..6)
                    .map(|offset| oryn_common::v4::TextClusterGeometry {
                        layout_node: node_key(target),
                        start: offset,
                        end: offset + 1,
                        rect: oryn_common::v4::CssRect {
                            x: offset as f64 * 10.,
                            y: 0.,
                            width: 10.,
                            height: 16.,
                        },
                        matrix,
                        line: 0,
                        rtl: false,
                    })
                    .collect(),
            )]),
            ..Default::default()
        });
        let result = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert!(
            result.protected
                && result.value.is_empty()
                && result.boxes.is_empty()
                && result.carets.is_empty()
        );
        assert_eq!((result.start, result.end), (0, 0));
        let caret = result.protected_caret.unwrap();
        assert_eq!(
            caret.selection,
            crate::composition::TextRange {
                start: 3,
                length: 0
            }
        );
        assert_eq!(caret.caret.offset, 3);
        assert_eq!(caret.caret.x, 30.);
        eval(scope, "p.value='😀';p.setSelectionRange(1,1)");
        state.set_layout(BindingLayout {
            text_clusters: BTreeMap::from([(
                node_key(target),
                (0..2)
                    .map(|offset| oryn_common::v4::TextClusterGeometry {
                        layout_node: node_key(target),
                        start: offset,
                        end: offset + 1,
                        rect: oryn_common::v4::CssRect {
                            x: offset as f64 * 10.,
                            y: 0.,
                            width: 10.,
                            height: 16.,
                        },
                        matrix,
                        line: 0,
                        rtl: false,
                    })
                    .collect(),
            )]),
            ..Default::default()
        });
        let result = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert!(result.value.is_empty());
        assert_eq!(result.protected_caret.unwrap().caret.offset, 1);
        eval(scope, "p.value='';p.setSelectionRange(0,0)");
        let mut translated = matrix;
        translated[12] = 5.;
        translated[13] = 7.;
        state.set_layout(BindingLayout {
            caret_stops: vec![oryn_common::v4::CaretStopGeometry {
                boundary_node: node_key(target),
                offset: 0,
                layout_node: node_key(target),
                rect: oryn_common::v4::CssRect {
                    x: 10.,
                    y: 20.,
                    width: 0.,
                    height: 18.,
                },
                matrix: translated,
                line: 0,
                rtl: false,
            }],
            ..Default::default()
        });
        let empty = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert!(empty.value.is_empty() && empty.boundaries.is_empty() && empty.carets.is_empty());
        let caret = empty.protected_caret.unwrap().caret;
        assert_eq!(caret.offset, 0);
        assert_eq!(
            (
                caret.rect.unwrap().x,
                caret.rect.unwrap().y,
                caret.rect.unwrap().height
            ),
            (15., 27., 18.)
        );
        state.set_layout(BindingLayout::default());
        assert!(
            text(scope, &state, target, &TextCommand::Read)
                .unwrap()
                .protected_caret
                .is_none()
        );
        eval(scope, "other.focus()");
        assert!(
            text(scope, &state, target, &TextCommand::Read)
                .unwrap()
                .protected_caret
                .is_none()
        );
    }
    #[test]
    fn editing_host_text_has_real_split_node_and_break_boundaries() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<div id=t contenteditable>ab<span>cd</span><br><br>ef<input type=password value=secret></div>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "t.focus()");
        let target = state.dom.borrow().focused().unwrap();
        let read = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert_eq!(read.value, "abcd\n\nef".encode_utf16().collect::<Vec<_>>());
        let selected = text(
            scope,
            &state,
            target,
            &TextCommand::Select {
                start: 1,
                end: 3,
                expected_value: read.value.clone(),
                expected_selection_revision: read.selection_revision,
            },
        )
        .unwrap();
        assert_eq!((selected.start, selected.end), (1, 3));
        eval(
            scope,
            "if(getSelection().toString()!=='bc')throw Error('native editing endpoints lost node affinity')",
        );
        eval(scope, "t.firstChild.data='changed'");
        assert!(
            text(
                scope,
                &state,
                target,
                &TextCommand::Select {
                    start: 1,
                    end: 3,
                    expected_value: read.value,
                    expected_selection_revision: selected.selection_revision
                }
            )
            .is_err()
        );
    }
    #[test]
    fn native_text_snapshot_preserves_utf16_and_rejects_changed_selection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<textarea id=t></textarea><input id=p type=password value=secret>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "t.value='a\\ud800b';t.focus();t.setSelectionRange(0,1)",
        );
        let target = state.dom.borrow().focused().unwrap();
        let read = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert_eq!(read.value, vec![97, 0xd800, 98]);
        let selected = text(
            scope,
            &state,
            target,
            &TextCommand::Select {
                start: 1,
                end: 2,
                expected_value: read.value.clone(),
                expected_selection_revision: read.selection_revision,
            },
        )
        .unwrap();
        assert_eq!((selected.start, selected.end), (1, 2));
        assert!(
            text(
                scope,
                &state,
                target,
                &TextCommand::Select {
                    start: 0,
                    end: 1,
                    expected_value: read.value,
                    expected_selection_revision: read.selection_revision
                }
            )
            .is_err()
        );
        eval(scope, "p.focus()");
        let password = state.dom.borrow().focused().unwrap();
        let read = text(scope, &state, password, &TextCommand::Read).unwrap();
        assert!(read.protected);
        assert!(read.value.is_empty());
        assert_eq!((read.start, read.end), (0, 0));
    }
    #[test]
    fn focus_callback_detachment_prevents_native_selection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abc</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "t.focus()");
        let target = state.dom.borrow().focused().unwrap();
        eval(scope, "t.blur();t.onfocus=()=>t.remove()");
        let read = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert!(
            text(
                scope,
                &state,
                target,
                &TextCommand::Select {
                    start: 0,
                    end: 1,
                    expected_value: read.value,
                    expected_selection_revision: read.selection_revision
                }
            )
            .is_err()
        );
    }
    #[test]
    fn readonly_document_text_uses_real_cross_node_boundaries_and_redacts_controls() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<p>a<span>b</span></p><p>c<input type=password value=secret><input type=hidden value=never></p>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = state.document_root();
        let read = text(scope, &state, target, &TextCommand::Read).unwrap();
        assert_eq!(
            read.value,
            "ab\nc\u{fffc}".encode_utf16().collect::<Vec<_>>()
        );
        let result = text(
            scope,
            &state,
            target,
            &TextCommand::Select {
                start: 0,
                end: 2,
                expected_value: read.value.clone(),
                expected_selection_revision: read.selection_revision,
            },
        )
        .unwrap();
        assert_eq!((result.start, result.end), (0, 2));
        eval(
            scope,
            "if(getSelection().toString()!=='ab')throw Error('native readonly cross-node selection')",
        );
    }
    #[test]
    fn readonly_flat_text_follows_shadow_slot_assignment_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div id=h><span slot=x>assigned</span><span>unassigned</span></div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "h.attachShadow({mode:'open'}).innerHTML='<slot name=x></slot><span>shadow</span>'",
        );
        let value = text(scope, &state, state.document_root(), &TextCommand::Read)
            .unwrap()
            .value;
        let value = String::from_utf16(&value).unwrap();
        assert!(value.contains("assigned"));
        assert!(value.contains("shadow"));
        assert!(!value.contains("unassigned"));
        assert_eq!(value.matches("assigned").count(), 1);
    }
}
