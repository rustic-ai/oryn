//! Index native semantic text by DOM identity, never by painted line order.
use oryn_common::v4::ResolvedTextRun;
use oryn_dom::{DomRuntime, NodeId, NodeKind, RenderedTextItem};
use std::collections::BTreeMap;
use std::sync::Arc;

fn key(node: NodeId) -> u64 {
    (u64::from(node.generation) << 32) | u64::from(node.slot)
}

/// Collect the DOM-ordered rendered stream from an already refreshed native
/// layout. The caller selects textContent fallback for a non-rendered root.
pub fn collect_items(
    dom: &DomRuntime,
    root: NodeId,
    styles: &BTreeMap<u64, Arc<BTreeMap<String, String>>>,
    text: &BTreeMap<u64, String>,
) -> Result<Vec<RenderedTextItem>, String> {
    let mut items = Vec::new();
    let mut stack = vec![(root, false)];
    let mut visited = 0usize;
    while let Some((node, exiting)) = stack.pop() {
        visited += 1;
        if visited > 2_097_152 {
            return Err("rendered text traversal budget exceeded".into());
        }
        let record = dom
            .arena()
            .get(node)
            .map_err(|error| format!("{error:?}"))?;
        let style = styles.get(&key(node));
        let property = |name: &str| style.and_then(|style| style.get(name)).map(String::as_str);
        let display = property("display").unwrap_or("");
        if display == "none" {
            continue;
        }
        match &record.kind {
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
                let parent_style = record.parent.and_then(|parent| styles.get(&key(parent)));
                let visibility = parent_style
                    .and_then(|style| style.get("visibility"))
                    .map(String::as_str);
                if visibility != Some("hidden") && visibility != Some("collapse") {
                    if let Some(value) = text.get(&key(node)) {
                        items.push(RenderedTextItem::Text(value.clone()));
                    }
                }
            }
            NodeKind::Element { name } => {
                let html = name.ns.as_ref() == "http://www.w3.org/1999/xhtml";
                let tag = name.local.as_ref();
                let visible = !matches!(property("visibility"), Some("hidden" | "collapse"));
                let breaks = if !visible {
                    0
                } else if html && tag == "p" {
                    2
                } else if (html && matches!(tag, "option" | "optgroup"))
                    || matches!(
                        display,
                        "block"
                            | "flow-root"
                            | "flex"
                            | "grid"
                            | "table"
                            | "table-caption"
                            | "list-item"
                    )
                {
                    1
                } else {
                    0
                };
                if exiting {
                    if node != root && visible {
                        if display == "table-cell" || display == "table-row" {
                            if let Some(parent) = record.parent {
                                let siblings = &dom
                                    .arena()
                                    .get(parent)
                                    .map_err(|error| format!("{error:?}"))?
                                    .children;
                                if let Some(index) =
                                    siblings.iter().position(|sibling| *sibling == node)
                                {
                                    let later = siblings[index + 1..].iter().any(|sibling| {
                                        styles
                                            .get(&key(*sibling))
                                            .and_then(|style| style.get("display"))
                                            .map(String::as_str)
                                            == Some(display)
                                    });
                                    if later {
                                        items.push(RenderedTextItem::Text(
                                            if display == "table-cell" { "\t" } else { "\n" }
                                                .into(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    if breaks != 0 {
                        items.push(RenderedTextItem::RequiredLineBreaks(breaks));
                    }
                    continue;
                }
                if breaks != 0 {
                    items.push(RenderedTextItem::RequiredLineBreaks(breaks));
                }
                if html && tag == "br" && visible && node != root {
                    items.push(RenderedTextItem::Text("\n".into()));
                }
                if html && tag == "br" {
                    continue;
                }
                // Replaced controls and inert template contents are not child
                // rendered text. The target fallback decision is separate.
                if html
                    && matches!(
                        tag,
                        "input"
                            | "textarea"
                            | "iframe"
                            | "img"
                            | "canvas"
                            | "video"
                            | "audio"
                            | "object"
                            | "template"
                    )
                {
                    continue;
                }
                stack.push((node, true));
                if html
                    && tag == "details"
                    && dom
                        .attribute(node, "open")
                        .map_err(|error| format!("{error:?}"))?
                        .is_none()
                {
                    // Only the first direct summary is the disclosure caption.
                    let summary = record.children.iter().find(|child| {
                        matches!(dom.arena().get(**child).map(|record| &record.kind),
                            Ok(NodeKind::Element { name }) if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "summary")
                    });
                    if let Some(summary) = summary {
                        stack.push((*summary, false));
                    }
                } else if html && tag == "select" {
                    // Use the native option ownership algorithm, including
                    // options under permitted intervening descendants. An
                    // optgroup outside a select is ordinary rendered content.
                    for option in dom
                        .select_options(node)
                        .map_err(|error| format!("{error:?}"))?
                        .into_iter()
                        .rev()
                    {
                        let mut cursor =
                            dom.parent(option).map_err(|error| format!("{error:?}"))?;
                        let mut hidden = false;
                        while let Some(ancestor) = cursor {
                            visited += 1;
                            if visited > 2_097_152 {
                                return Err("rendered text traversal budget exceeded".into());
                            }
                            if ancestor == node {
                                break;
                            }
                            if styles
                                .get(&key(ancestor))
                                .and_then(|style| style.get("display"))
                                .is_some_and(|display| display == "none")
                            {
                                hidden = true;
                                break;
                            }
                            cursor = dom.parent(ancestor).map_err(|error| format!("{error:?}"))?;
                        }
                        if !hidden {
                            stack.push((option, false));
                        }
                    }
                } else {
                    stack.extend(record.children.iter().rev().map(|child| (*child, false)));
                }
            }
            _ => {}
        }
    }
    Ok(items)
}

/// CSS-normalized/transformed text before soft-wrap trimming. Generated and
/// atomic layout placeholders have no DOM text contribution.
pub fn semantic_nodes(runs: &[ResolvedTextRun]) -> Result<BTreeMap<u64, String>, String> {
    let mut pieces: BTreeMap<u64, Vec<(usize, usize, String)>> = BTreeMap::new();
    let mut bytes = 0usize;
    let mut spans = 0usize;
    for run in runs {
        let source = run
            .semantic_source
            .as_ref()
            .ok_or("missing semantic text source")?;
        bytes = bytes
            .checked_add(source.text.len())
            .ok_or("semantic text budget overflow")?;
        if bytes > 8 * 1024 * 1024 {
            return Err("semantic text byte budget exceeded".into());
        }
        for span in &source.spans {
            spans += 1;
            if spans > 1_048_576 {
                return Err("semantic text span budget exceeded".into());
            }
            let text = source
                .text
                .get(span.rendered_start..span.rendered_end)
                .ok_or("semantic text byte boundary invalid")?;
            if span.source_start > span.source_end || span.source_end > 1_048_576 {
                return Err("semantic text source offset invalid".into());
            }
            if span.atomic || span.source_node.is_none() {
                continue;
            }
            if !span.transformed
                && text.encode_utf16().count() != span.source_end - span.source_start
            {
                return Err("semantic identity span changed UTF16 length".into());
            }
            pieces.entry(span.source_node.unwrap()).or_default().push((
                span.source_start,
                span.source_end,
                text.to_owned(),
            ));
        }
    }
    let mut result = BTreeMap::new();
    for (node, mut fragments) in pieces {
        fragments.sort_by_key(|piece| (piece.0, piece.1));
        let mut previous: Option<(usize, usize, String)> = None;
        let mut text = String::new();
        for fragment in fragments {
            if let Some(old) = &previous {
                if (old.0, old.1) == (fragment.0, fragment.1) {
                    if old.2 == fragment.2 {
                        continue;
                    }
                    return Err("conflicting semantic text source runs".into());
                }
                if fragment.0 < old.1 {
                    return Err("overlapping semantic text source runs".into());
                }
            }
            text.push_str(&fragment.2);
            previous = Some(fragment);
        }
        result.insert(node, text);
    }
    Ok(result)
}

/// Verify that native source identities still name DOM character data and that
/// UTF16 provenance does not escape the owning node's actual source.
pub fn semantic_dom_nodes(
    dom: &DomRuntime,
    runs: &[ResolvedTextRun],
) -> Result<BTreeMap<u64, String>, String> {
    let result = semantic_nodes(runs)?;
    for run in runs {
        for span in &run
            .semantic_source
            .as_ref()
            .ok_or("missing semantic text source")?
            .spans
        {
            let Some(identity) = span.source_node else {
                continue;
            };
            let node = NodeId {
                slot: identity as u32,
                generation: (identity >> 32) as u32,
            };
            let record = dom
                .arena()
                .get(node)
                .map_err(|_| "stale semantic text source identity")?;
            // Editable controls use their element identity for native selection
            // geometry. That value is not authored DOM character data and is
            // excluded by the rendered-text collector, but the live identity
            // must still be validated above.
            if identity == span.owner
                && matches!(&record.kind, NodeKind::Element { name }
                    if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && matches!(name.local.as_ref(), "input" | "textarea"))
            {
                continue;
            }
            let data = match &record.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } => data,
                _ => return Err("semantic text source is not DOM character data".into()),
            };
            if span.source_end > oryn_dom::dom_string::units(data).len() {
                return Err("semantic text source range exceeds DOM data".into());
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::v4::{ResolvedTextSpan, SemanticTextSource};
    #[test]
    fn br_has_child_separator_but_no_root_or_authored_child_text() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let br = dom.create_element("br");
        let child = dom.create_text("ignored");
        dom.append_child(root, br).unwrap();
        dom.append_child(br, child).unwrap();
        let text = BTreeMap::from([(key(child), "ignored".into())]);
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
            ),
            "\n"
        );
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, br, &BTreeMap::new(), &text).unwrap()
            ),
            ""
        );
    }
    #[test]
    fn semantic_provenance_requires_live_character_data_within_source_bounds() {
        let mut dom = DomRuntime::new();
        let node = dom.create_text("a");
        let mut value = run("A", 0, 1);
        value.semantic_source.as_mut().unwrap().spans[0].source_node = Some(key(node));
        assert!(semantic_dom_nodes(&dom, &[value.clone()]).is_ok());
        value.semantic_source.as_mut().unwrap().spans[0].source_end = 2;
        value.semantic_source.as_mut().unwrap().spans[0].transformed = true;
        assert!(semantic_dom_nodes(&dom, &[value.clone()]).is_err());
        let element = dom.create_element("div");
        value.semantic_source.as_mut().unwrap().spans[0].source_node = Some(key(element));
        assert!(semantic_dom_nodes(&dom, &[value]).is_err());
    }
    #[test]
    fn control_value_provenance_requires_live_matching_owner_and_is_not_dom_text() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        for tag in ["input", "textarea"] {
            let control = dom.create_element(tag);
            dom.append_child(root, control).unwrap();
            let mut value = run("value", 0, 5);
            let span = &mut value.semantic_source.as_mut().unwrap().spans[0];
            span.owner = key(control);
            span.source_node = Some(key(control));
            let text = semantic_dom_nodes(&dom, &[value.clone()]).unwrap();
            assert_eq!(
                oryn_dom::reduce_rendered_text(
                    collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
                ),
                ""
            );
            value.semantic_source.as_mut().unwrap().spans[0].owner = key(root);
            assert!(semantic_dom_nodes(&dom, &[value.clone()]).is_err());
            let stale = key(control) + (1u64 << 32);
            let span = &mut value.semantic_source.as_mut().unwrap().spans[0];
            span.owner = stale;
            span.source_node = Some(stale);
            assert!(semantic_dom_nodes(&dom, &[value]).is_err());
        }
    }
    #[test]
    fn closed_details_selects_only_first_direct_summary() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("details");
        let mut text = BTreeMap::new();
        for tag in ["summary", "span", "summary"] {
            let element = dom.create_element(tag);
            let child = dom.create_text(tag);
            dom.append_child(root, element).unwrap();
            dom.append_child(element, child).unwrap();
            text.insert(key(child), tag.into());
        }
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
            ),
            "summary"
        );
        dom.set_attribute(root, "open", "").unwrap();
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
            ),
            "summaryspansummary"
        );
    }
    #[test]
    fn optgroup_outside_select_preserves_ordinary_rendered_children() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let group = dom.create_element("optgroup");
        let node = dom.create_text("123");
        dom.append_child(root, group).unwrap();
        dom.append_child(group, node).unwrap();
        let text = BTreeMap::from([(key(node), "123".into())]);
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
            ),
            "123"
        );
    }
    #[test]
    fn select_uses_options_not_arbitrary_text_or_nested_optgroups() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("select");
        let stray = dom.create_text("ignored");
        dom.append_child(root, stray).unwrap();
        let mut text = BTreeMap::from([(key(stray), "ignored".into())]);
        for value in ["one", "two"] {
            let group = dom.create_element("optgroup");
            let option = dom.create_element("option");
            let child = dom.create_text(value);
            dom.append_child(root, group).unwrap();
            dom.append_child(group, option).unwrap();
            dom.append_child(option, child).unwrap();
            text.insert(key(child), value.into());
        }
        assert_eq!(
            oryn_dom::reduce_rendered_text(
                collect_items(&dom, root, &BTreeMap::new(), &text).unwrap()
            ),
            "one\ntwo"
        );
        let group = dom.arena().get(root).unwrap().children[1];
        let styles = BTreeMap::from([(
            key(group),
            Arc::new(BTreeMap::from([("display".into(), "none".into())])),
        )]);
        assert_eq!(
            oryn_dom::reduce_rendered_text(collect_items(&dom, root, &styles, &text).unwrap()),
            "two"
        );
    }
    #[test]
    fn traversal_uses_dom_order_and_native_table_separators() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("table");
        let mut styles = BTreeMap::new();
        let mut text = BTreeMap::new();
        for row_index in 0..2 {
            let row = dom.create_element("tr");
            dom.append_child(root, row).unwrap();
            styles.insert(
                key(row),
                Arc::new(BTreeMap::from([("display".into(), "table-row".into())])),
            );
            for column in 0..2 {
                let cell = dom.create_element("td");
                let child = dom.create_text("source ignored");
                dom.append_child(row, cell).unwrap();
                dom.append_child(cell, child).unwrap();
                styles.insert(
                    key(cell),
                    Arc::new(BTreeMap::from([("display".into(), "table-cell".into())])),
                );
                text.insert(key(child), format!("{row_index}{column}"));
            }
        }
        let items = collect_items(&dom, root, &styles, &text).unwrap();
        assert_eq!(oryn_dom::reduce_rendered_text(items), "00\t01\n10\t11");
        let row = dom.arena().get(root).unwrap().children[0];
        assert_eq!(
            oryn_dom::reduce_rendered_text(collect_items(&dom, row, &styles, &text).unwrap()),
            "00\t01"
        );
    }
    #[test]
    fn traversal_excludes_hidden_and_replaced_children_and_reduces_blocks() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let mut styles = BTreeMap::new();
        let mut text = BTreeMap::new();
        for (tag, display, value) in [
            ("p", "block", "A"),
            ("span", "none", "X"),
            ("canvas", "inline", "Y"),
            ("p", "block", "B"),
        ] {
            let element = dom.create_element(tag);
            let child = dom.create_text(value);
            dom.append_child(root, element).unwrap();
            dom.append_child(element, child).unwrap();
            styles.insert(
                key(element),
                Arc::new(BTreeMap::from([("display".into(), display.into())])),
            );
            text.insert(key(child), value.into());
        }
        assert_eq!(
            oryn_dom::reduce_rendered_text(collect_items(&dom, root, &styles, &text).unwrap()),
            "A\n\nB"
        );
    }
    fn run(text: &str, start: usize, end: usize) -> ResolvedTextRun {
        ResolvedTextRun {
            layout_node: 9,
            text: "paint is not source".into(),
            spans: vec![],
            aliases: vec![],
            semantic_source: Some(SemanticTextSource {
                text: text.into(),
                spans: vec![ResolvedTextSpan {
                    owner: 7,
                    source_node: Some((3u64 << 32) | 2),
                    rendered_start: 0,
                    rendered_end: text.len(),
                    source_start: start,
                    source_end: end,
                    transformed: false,
                    atomic: false,
                }],
            }),
        }
    }
    #[test]
    fn native_identity_order_and_soft_wrap_spaces_survive() {
        let result = semantic_nodes(&[run("b", 2, 3), run("a ", 0, 2), run("b", 2, 3)]).unwrap();
        assert_eq!(result[&((3u64 << 32) | 2)], "a b");
        assert!(!result.contains_key(&2));
    }
    #[test]
    fn invalid_or_conflicting_projection_fails_closed() {
        assert!(semantic_nodes(&[run("a", 0, 1), run("b", 0, 1)]).is_err());
        assert!(semantic_nodes(&[run("ab", 0, 2), run("c", 1, 2)]).is_err());
        assert!(semantic_nodes(&[run("😀", 0, 1)]).is_err());
        let mut value = run("é", 0, 1);
        value.semantic_source.as_mut().unwrap().spans[0].rendered_start = 1;
        assert!(semantic_nodes(&[value]).is_err());
        let mut value = run("a", 0, 1);
        value.semantic_source = None;
        assert!(semantic_nodes(&[value]).is_err());
    }
    #[test]
    fn transformed_text_and_non_dom_placeholders() {
        let mut value = run("SS", 0, 1);
        value.semantic_source.as_mut().unwrap().spans[0].transformed = true;
        assert_eq!(
            semantic_nodes(&[value.clone()])
                .unwrap()
                .values()
                .next()
                .unwrap(),
            "SS"
        );
        value.semantic_source.as_mut().unwrap().spans[0].atomic = true;
        assert!(semantic_nodes(&[value]).unwrap().is_empty());
    }
}
