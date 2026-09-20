//! Selection serializes rendered, selectable text. Range.toString remains a DOM
//! text operation: sharing its collector would expose hidden and unslotted text.
use super::*;

fn style<'a>(layout: &'a BindingLayout, node: NodeId, name: &str) -> Option<&'a str> {
    layout
        .styles
        .get(&node_key(node))?
        .get(name)
        .map(String::as_str)
}
fn presentation(
    dom: &DomRuntime,
    layout: &BindingLayout,
    node: NodeId,
) -> Result<(bool, bool, NodeId), V8BindingError> {
    let mut cursor = Some(node);
    let mut selectable = None;
    let mut visibility = None;
    let mut preserve = None;
    let mut paragraph = None;
    let mut seen = BTreeSet::new();
    while let Some(current) = cursor {
        if !seen.insert(current) {
            return Err(DomError::Cycle.into());
        }
        let record = dom.arena().get(current)?;
        let display = style(layout, current, "display");
        // The hidden attribute supplies a UA display rule, which author CSS
        // can override. Use the resolved display whenever it is available.
        if (display.is_none()
            && matches!(&record.kind,NodeKind::Element{name} if matches!(name.local.as_ref(),"script"|"style"|"head"|"template")))
            || (record.attributes.contains_null("hidden") && display.is_none())
            || display == Some("none")
            || style(layout, current, "content-visibility") == Some("hidden")
        {
            return Ok((false, false, node));
        }
        // user-select:auto obtains the parent's used value; an explicit text
        // descendant can override a none ancestor. Visibility is inherited but
        // can likewise be overridden, unlike display:none.
        if selectable.is_none() {
            selectable = match style(layout, current, "user-select") {
                Some("none") => Some(false),
                Some("text" | "all" | "contain") => Some(true),
                _ => None,
            };
        }
        if visibility.is_none() {
            visibility = style(layout, current, "visibility")
                .map(|value| !matches!(value, "hidden" | "collapse"));
        }
        if preserve.is_none() {
            preserve = style(layout, current, "white-space")
                .map(|value| matches!(value, "pre" | "pre-wrap" | "break-spaces"));
        }
        if paragraph.is_none()
            && style(layout, current, "display").map_or_else(
                || is_editing_block(dom, current),
                |value| {
                    matches!(
                        value,
                        "block"
                            | "flow-root"
                            | "list-item"
                            | "table-cell"
                            | "table-caption"
                            | "flex"
                            | "grid"
                    )
                },
            )
        {
            paragraph = Some(current);
        }
        cursor = dom.flat_tree_parent(current)?;
    }
    Ok((
        selectable.unwrap_or(true) && visibility.unwrap_or(true),
        preserve.unwrap_or(false),
        paragraph.unwrap_or(tree_root(dom, node)?),
    ))
}
fn intersects(
    dom: &DomRuntime,
    start: BoundaryPoint,
    end: BoundaryPoint,
    node: NodeId,
    length: u32,
) -> Result<bool, V8BindingError> {
    Ok(selection_composed::compare(
        dom,
        BoundaryPoint {
            container: node,
            offset: length,
        },
        start,
    )? > 0
        && selection_composed::compare(
            dom,
            BoundaryPoint {
                container: node,
                offset: 0,
            },
            end,
        )? < 0)
}

fn flat_point(
    dom: &DomRuntime,
    point: BoundaryPoint,
    positions: &BTreeMap<NodeId, usize>,
    limits: &BTreeMap<NodeId, usize>,
) -> Result<Option<(usize, u32)>, V8BindingError> {
    if matches!(
        dom.arena().get(point.container)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) {
        return Ok(positions
            .get(&point.container)
            .map(|index| (*index, point.offset)));
    }
    let children = light_children(dom, point.container)?;
    if point.offset == 0
        && let Some(index) = positions.get(&point.container)
    {
        return Ok(Some((*index, 0)));
    }
    if let Some(child) = children.get(point.offset as usize) {
        return Ok(positions.get(child).map(|index| (*index, 0)));
    }
    if point.offset as usize == children.len() {
        return Ok(limits
            .get(&point.container)
            .copied()
            .or_else(|| children.last().and_then(|child| limits.get(child).copied()))
            .map(|index| (index, 0)));
    }
    Ok(None)
}

pub(super) fn prepare_control(
    state: &V8BindingState,
    node: NodeId,
) -> Result<Option<oryn_dom::TrackedNodeLease>, V8BindingError> {
    if !state.dom.borrow().supports_selection_range(node)? {
        return Ok(None);
    }
    Ok(Some(oryn_dom::TrackedNodeLease::new(&state.dom, node)?))
}
fn control_text(state: &V8BindingState) -> Result<Option<String>, V8BindingError> {
    let identity = state
        .selected_text_control
        .borrow()
        .as_ref()
        .map(|lease| lease.current());
    let Some((owner, node)) = identity else {
        return Ok(None);
    };
    if !Rc::ptr_eq(&owner, &state.dom)
        || !owner.borrow().is_connected(node)?
        || owner.borrow().node_document(node)? != Some(state.document_root())
        || !owner.borrow().supports_selection_range(node)?
    {
        state.selected_text_control.borrow_mut().take();
        return Ok(None);
    }
    let value = owner.borrow().control_value(node)?;
    let length = oryn_dom::dom_string::len(&value) as u32;
    let (start, end, _) = state
        .text_control_selections
        .borrow()
        .get(&node)
        .cloned()
        .unwrap_or((0, 0, "none".into()));
    Ok(Some(utf16_slice(
        &value,
        start.min(length),
        end.min(length),
    )))
}

pub(super) fn preserves_control_on_pointer(
    state: &V8BindingState,
    target: NodeId,
) -> Result<bool, V8BindingError> {
    if control_text(state)?.is_none() {
        return Ok(false);
    }
    let dom = state.dom.borrow();
    let mut node = Some(target);
    let mut seen = BTreeSet::new();
    while let Some(current) = node {
        if !seen.insert(current) {
            return Err(DomError::Cycle.into());
        }
        if (dom.is_element(current, "http://www.w3.org/1999/xhtml", "a")
            || dom.is_element(current, "http://www.w3.org/1999/xhtml", "area"))
            && dom.attribute_in_null_namespace(current, "href")?.is_some()
        {
            return Ok(true);
        }
        node = dom.flat_tree_parent(current)?;
    }
    Ok(false)
}

pub(super) fn collect(state: &V8BindingState) -> Result<String, V8BindingError> {
    if let Some(text) = control_text(state)? {
        return Ok(text);
    }
    refresh_layout(state)?;
    synchronize_selection_range(state)?;
    let Some(id) = state
        .selection_composed_range
        .get()
        .or(state.selection_range.get())
    else {
        return Ok(String::new());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Selection range"))?;
    let mut start = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    let mut end = BoundaryPoint {
        container: range.end_container,
        offset: range.end_offset,
    };
    if start == end {
        return Ok(String::new());
    }
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let rendered = crate::accessibility::resolved_nodes(&layout.resolved_text)
        .map_err(V8BindingError::Message)?;
    let mut pending = vec![state.document_root()];
    let mut seen = BTreeSet::new();
    let mut nodes = Vec::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            return Err(DomError::Cycle.into());
        }
        nodes.push(node);
        pending.extend(dom.flat_tree_children(node)?.into_iter().rev());
    }
    let positions: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (*node, index))
        .collect();
    let mut limits: BTreeMap<_, _> = positions
        .iter()
        .map(|(node, index)| (*node, index + 1))
        .collect();
    for node in nodes.iter().rev() {
        if let Some(parent) = dom.flat_tree_parent(*node)? {
            let end = limits[node];
            if let Some(limit) = limits.get_mut(&parent) {
                *limit = (*limit).max(end)
            }
        }
    }
    let text_endpoint = |node| {
        dom.arena().get(node).is_ok_and(|record| {
            matches!(
                record.kind,
                NodeKind::Text { .. } | NodeKind::CDataSection { .. }
            )
        })
    };
    let flat_bounds = if text_endpoint(start.container) || text_endpoint(end.container) {
        flat_point(&dom, start, &positions, &limits)?
            .zip(flat_point(&dom, end, &positions, &limits)?)
            .map(|(a, b)| {
                if a > b {
                    std::mem::swap(&mut start, &mut end);
                    (b, a)
                } else {
                    (a, b)
                }
            })
    } else {
        None
    };
    let mut output = Vec::<u16>::new();
    let mut pending_separators = Vec::new();
    let mut paragraph_break = false;
    let mut paragraph = None;
    let mut preserve_start = false;
    let mut preserve_end = false;
    for node in nodes {
        let record = dom.arena().get(node)?;
        let (visible, preserve, current) = presentation(&dom, &layout, node)?;
        if !visible {
            continue;
        }
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "br") {
            let (parent, index) = relative_boundary(&dom, node, false)?;
            let selected = if let Some((first, last)) = flat_bounds {
                (positions[&node], 0) >= first && (positions[&node] + 1, 0) <= last
            } else {
                selection_composed::compare(
                    &dom,
                    BoundaryPoint {
                        container: parent,
                        offset: index,
                    },
                    start,
                )? >= 0
                    && selection_composed::compare(
                        &dom,
                        BoundaryPoint {
                            container: parent,
                            offset: index + 1,
                        },
                        end,
                    )? <= 0
            };
            if selected {
                if output.is_empty() {
                    preserve_start = true;
                }
                output.push(10);
                preserve_end = true;
                paragraph = None;
            }
            continue;
        }
        let (NodeKind::Text { data } | NodeKind::CDataSection { data }) = &record.kind else {
            continue;
        };
        let raw = oryn_dom::dom_string::units(data);
        let length = u32::try_from(raw.len())
            .map_err(|_| V8BindingError::Allocation("Selection text length"))?;
        if let Some((first, last)) = flat_bounds {
            let index = positions[&node];
            if (index, length) <= first || (index, 0) >= last {
                continue;
            }
        } else if !intersects(&dom, start, end, node, length)? {
            continue;
        }
        let from = if node == start.container {
            start.offset as usize
        } else {
            0
        };
        let to = if node == end.container {
            end.offset as usize
        } else {
            raw.len()
        };
        let mut units = Vec::new();
        if let Some(mapped) = rendered.get(&node_key(node)) {
            if mapped.offsets.len() != raw.len() + 1 {
                return Err(V8BindingError::Message(
                    "Selection shaping source length changed".into(),
                ));
            }
            for (index, unit) in mapped.units.iter().enumerate() {
                if mapped.starts[index] as usize >= from && (mapped.ends[index + 1] as usize) <= to
                {
                    units.push(*unit);
                }
            }
        } else {
            for &unit in &raw[from.min(raw.len())..to.min(raw.len())] {
                if !preserve && matches!(unit, 9 | 10 | 12 | 13 | 32) {
                    if units.last() != Some(&32) {
                        units.push(32)
                    }
                } else {
                    units.push(unit)
                }
            }
        }
        // Even an entirely collapsed whitespace run can separate two block
        // formatting contexts. Retain its break until actual following text;
        // trailing collapsed nodes must not manufacture a final newline.
        if units.is_empty()
            && !raw[from.min(raw.len())..to.min(raw.len())]
                .iter()
                .all(|unit| matches!(unit, 9 | 10 | 12 | 13 | 32))
        {
            continue;
        }
        if let Some(previous) = paragraph.filter(|previous| *previous != current) {
            let separator = selection_movement::accessibility_paragraph_separator(
                &dom, &layout, previous, current,
            )?;
            while output
                .last()
                .is_some_and(|unit| matches!(unit, 9 | 12 | 13 | 32))
            {
                output.pop();
            }
            paragraph_break |= separator == 10
                && [previous, current]
                    .iter()
                    .any(|node| dom.is_element(*node, "http://www.w3.org/1999/xhtml", "p"));
            pending_separators.push(separator);
        }
        paragraph = Some(current);
        if units.is_empty() {
            continue;
        }
        if !preserve
            && !pending_separators.is_empty()
            && pending_separators.iter().all(|unit| *unit == 10)
        {
            let required: usize = if paragraph_break { 2 } else { 1 };
            let existing = output.iter().rev().take_while(|unit| **unit == 10).count();
            output.extend(std::iter::repeat_n(10, required.saturating_sub(existing)));
            pending_separators.clear();
        } else {
            output.append(&mut pending_separators)
        }
        paragraph_break = false;
        if !preserve && output.last() == Some(&10) {
            let first = units
                .iter()
                .position(|unit| !matches!(unit, 9 | 12 | 13 | 32))
                .unwrap_or(units.len());
            units.drain(..first);
        }
        if units.is_empty() {
            continue;
        }
        if output.is_empty() {
            preserve_start = preserve || rendered.contains_key(&node_key(node))
        }
        preserve_end = preserve || rendered.contains_key(&node_key(node));
        output
            .try_reserve(units.len())
            .map_err(|_| V8BindingError::Allocation("Selection rendered text"))?;
        output.extend(units);
        paragraph = Some(current);
    }
    let first = if preserve_start {
        0
    } else {
        output
            .iter()
            .position(|unit| !matches!(unit, 9 | 12 | 13 | 32))
            .unwrap_or(output.len())
    };
    let last = if preserve_end {
        output.len()
    } else {
        output
            .iter()
            .rposition(|unit| !matches!(unit, 9 | 12 | 13 | 32))
            .map_or(first, |index| index + 1)
    };
    output = output[first..last.max(first)].to_vec();
    Ok(oryn_dom::dom_string::from_units(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evaluate(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn resolved_display_can_override_hidden_attribute() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=root>A<span id=shown hidden>visible</span><span hidden>default-hidden</span><span id=removed hidden>display-none</span>B</div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(209), 1).unwrap();
        for (id, display) in [("shown", "inline"), ("removed", "none")] {
            let node = state.dom.borrow().find_by_id(id).unwrap();
            std::sync::Arc::make_mut(
                state
                    .layout
                    .borrow_mut()
                    .styles
                    .entry(node_key(node))
                    .or_default(),
            )
            .insert("display".into(), display.into());
        }
        evaluate(
            scope,
            "getSelection().selectAllChildren(document.getElementById('root'));if(String(getSelection())!=='AvisibleB')throw Error('resolved display must override hidden attribute');",
        );
    }
    #[test]
    fn displayed_style_and_script_blocks_keep_structural_line_breaks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=root>\n<style id=style> a { color: green; }\n b { color: blue; } </style>\n<script id=script>\nfunction example() { return 1; }\n</script>\n<pre id=pre>PASS</pre></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(211), 1).unwrap();
        for id in ["style", "script", "pre"] {
            let node = state.dom.borrow().find_by_id(id).unwrap();
            std::sync::Arc::make_mut(
                state
                    .layout
                    .borrow_mut()
                    .styles
                    .entry(node_key(node))
                    .or_default(),
            )
            .insert("display".into(), "block".into());
            if id == "pre" {
                std::sync::Arc::make_mut(
                    state
                        .layout
                        .borrow_mut()
                        .styles
                        .entry(node_key(node))
                        .or_default(),
                )
                .insert("white-space".into(), "pre".into());
            }
        }
        evaluate(
            scope,
            r#"(()=>{const r=new Range();r.selectNode(document.getElementById('root'));getSelection().addRange(r);const text=String(getSelection());if(text!=='\na { color: green; } b { color: blue; }\nfunction example() { return 1; }\n\nPASS')throw Error(JSON.stringify(text));})()"#,
        );
    }
    #[test]
    fn paragraph_boundaries_and_content_visibility_match_pinned_line_callers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div><p id=one>One</p>\n<p id=two>Two</p>\n<p id=three>Three</p><p id=hidden>Not rendered</p></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(213), 1).unwrap();
        let hidden = state.dom.borrow().find_by_id("hidden").unwrap();
        std::sync::Arc::make_mut(
            state
                .layout
                .borrow_mut()
                .styles
                .entry(node_key(hidden))
                .or_default(),
        )
        .insert("content-visibility".into(), "hidden".into());
        evaluate(
            scope,
            r#"(()=>{const s=getSelection(),one=document.getElementById('one').firstChild,three=document.getElementById('three').firstChild,check=(x,m)=>{if(!x)throw Error(m)};s.setBaseAndExtent(one,2,three,2);check(String(s)==='e\n\nTwo\n\nTh','paragraph required line breaks');s.setBaseAndExtent(three,2,one,2);check(String(s)==='e\n\nTwo\n\nTh','backward same displayed text');s.selectAllChildren(document.getElementById('hidden'));check(String(s)===''&&s.getRangeAt(0).toString()==='Not rendered','content visibility suppresses rendered text only');})()"#,
        );
    }
    #[test]
    fn unrendered_endpoint_ranges_remain_dom_ranges_but_containment_is_flat() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host><span>unassigned</span></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(212), 1).unwrap();
        evaluate(
            scope,
            r#"(()=>{const host=document.getElementById('host'),light=host.firstChild,root=host.attachShadow({mode:'open'}),s=getSelection(),check=(x,m)=>{if(!x)throw Error(m)};root.innerHTML='<slot name=other><span>fallback</span></slot>';s.selectAllChildren(host);check(!s.containsNode(light)&&!s.containsNode(light.firstChild),'unassigned is not visibly contained');check(s.getRangeAt(0).comparePoint(light.firstChild,0)===0,'ordinary Range still contains unassigned endpoint');s.selectAllChildren(light);check(String(s)===''&&s.getRangeAt(0).toString()==='unassigned','unrendered text excludes no DOM endpoint');light.slot='other';const fallback=root.firstChild.firstChild;s.selectAllChildren(fallback);check(!s.containsNode(fallback.firstChild)&&s.getRangeAt(0).comparePoint(fallback.firstChild,0)===0&&String(s)==='','suppressed fallback remains a valid DOM range');})()"#,
        );
    }
    #[test]
    fn rendered_selection_filters_used_user_select_and_preserves_dom_range_text() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=a>start <span id=excluded>unselectable<strong>nested</strong> text</span> end</div><div id=b><span id=one>selectable</span> ignored <span id=two>text</span></div><p id=c>foo bar</p></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(207), 1).unwrap();
        for node in traversal_nodes(&state.dom.borrow(), state.document_root()).unwrap() {
            if !matches!(
                state.dom.borrow().arena().get(node).unwrap().kind,
                NodeKind::Element { .. }
            ) {
                continue;
            }
            let id = state
                .dom
                .borrow()
                .attribute_in_null_namespace(node, "id")
                .unwrap()
                .map(|id| id.to_string());
            if let Some(value) = id.as_deref().and_then(|id| match id {
                "excluded" | "b" => Some("none"),
                "one" | "two" => Some("text"),
                _ => None,
            }) {
                std::sync::Arc::make_mut(
                    state
                        .layout
                        .borrow_mut()
                        .styles
                        .entry(node_key(node))
                        .or_default(),
                )
                .insert("user-select".into(), value.into());
            }
        }
        evaluate(
            scope,
            r#"(()=>{const s=getSelection(),r=new Range(),check=(x,m)=>{if(!x)throw Error(m)};r.selectNodeContents(document.getElementById('a'));s.addRange(r);check(s.toString()==='start  end','excluded subtree preserves surrounding spaces');check(r.toString()==='start unselectablenested text end','Range remains DOM text');s.selectAllChildren(document.getElementById('b'));check(String(s)==='selectabletext','explicit descendants override none ancestor');r.selectNode(document.getElementById('c'));s.removeAllRanges();s.addRange(r);check(String(s)==='foo bar','no paragraph terminator');document.getElementById('c').firstChild.data='a\ud800b';r.setStart(document.getElementById('c').firstChild,1);r.setEnd(document.getElementById('c').firstChild,2);check(String(s)==='\ud800','UTF16 selected unit preserved');})()"#,
        );
    }
    #[test]
    fn flat_slot_order_and_hidden_shadow_text_are_serialized_without_changing_dom_ranges() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host><span slot=a>A</span><span slot=b>B</span><span>unslotted</span></div></body>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(210), 1).unwrap();
        evaluate(
            scope,
            r#"(()=>{const host=document.getElementById('host'),root=host.attachShadow({mode:'closed'}),s=getSelection(),check=(x,m)=>{if(!x)throw Error(m)};root.innerHTML='<i>before</i><slot name=b></slot><slot name=a></slot><span hidden>hidden</span><i>after</i>';s.selectAllChildren(host);check(String(s)==='beforeBAafter','native flat order, hidden and unslotted exclusions');check(s.getRangeAt(0).toString()==='ABunslotted','DOM Range order remains distinct');s.setBaseAndExtent(root.firstChild.firstChild,2,root.lastChild.firstChild,3);check(String(s)==='foreBAaft','partial endpoints across shadow and assigned light nodes');s.setBaseAndExtent(root.lastChild.firstChild,2,host.parentNode,1);check(String(s)==='ter','mixed shadow text/end-of-host excludes preceding assigned nodes');})()"#,
        );
    }
    #[test]
    fn text_control_selection_survives_button_focus_but_not_another_input_or_dom_selection() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><input id=a value='This is a test'><input id=b><button id=button>Button</button><textarea id=t>  Line one\nLine two</textarea><p id=p>DOM</p></body>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(208), 1).unwrap();
        evaluate(
            scope,
            r#"(()=>{const s=getSelection(),a=document.getElementById('a'),b=document.getElementById('b'),t=document.getElementById('t'),check=(x,m)=>{if(!x)throw Error(m)};a.select();check(String(s)===a.value,'select focuses and exposes native value');document.getElementById('button').focus();check(String(s)===a.value,'button focus preserves control selection');b.focus();check(String(s)==='','other control selection replaces source');a.selectionStart=3;a.selectionEnd=7;check(String(s)==='','unfocused selection does not replace source');a.focus();check(String(s)==='s is','focus exposes current control endpoints');t.select();t.setSelectionRange(3,12);check(String(s)==='ine one\nL','textarea preserves whitespace');s.selectAllChildren(document.getElementById('p'));check(String(s)==='DOM','explicit DOM range replaces control source');a.select();s.empty();check(String(s)==='','explicit empty clears control selection source');a.select();check(String(s)===a.value,'select while focused restores source');a.remove();check(String(s)==='','detached control cannot leak stale selection');})()"#,
        );
    }
}

#[cfg(test)]
mod slot_reflection_tests {
    use super::*;
    #[test]
    fn slot_reflection_reassigns_flat_content_and_preserves_dom_range_text() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            RealmId(951),
            1,
        )
        .unwrap();
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},host=document.createElement('div'),child=document.createElement('span');
            child.textContent='assigned';host.append(child);document.body.append(host);
            const root=host.attachShadow({mode:'open'});root.innerHTML='<slot name=chosen><b>fallback</b></slot>';
            const slot=root.firstChild,fallback=slot.firstChild,s=getSelection();
            check(child.slot===''&&slot.assignedNodes().length===0,'initial assignment');
            child.slot='chosen';check(child.getAttribute('slot')==='chosen'&&slot.assignedNodes()[0]===child,'reflected assignment');
            s.selectAllChildren(fallback);check(String(s)===''&&s.getRangeAt(0).toString()==='fallback','hidden fallback DOM range');
            child.slot='';check(slot.assignedNodes().length===0,'assignment removed');
            s.selectAllChildren(fallback);check(String(s)==='fallback','fallback restored');
            const svg=document.createElementNS('http://www.w3.org/2000/svg','g');svg.slot='chosen';
            check(svg.getAttribute('slot')==='chosen','Element member includes SVG');return true;
        })()"#).is_true());
    }
}
