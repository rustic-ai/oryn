//! Native editing algorithms suspended at owned mutation operations.
use super::*;
use editing_continuation::Context;

pub(super) async fn apply_block_command(
    context: &Context,
    command: &str,
    value: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(anchor) = state.selection_anchor.get() else {
        return Ok(());
    };
    let alignment = match command {
        "justifyleft" => "left",
        "justifycenter" => "center",
        "justifyright" => "right",
        "justifyfull" => "justify",
        _ => return apply_block_command_at_boundary(context, command, value, anchor).await,
    };
    clear_contained_alignment(context).await?;
    wrap_alignment_contents(context, alignment).await
}

pub(super) async fn apply_block_command_at_boundary(
    context: &Context,
    command: &str,
    value: &str,
    anchor: BoundaryPoint,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let block = nearest_editing_block(&state.dom.borrow(), anchor.container, host)?;
    match command {
        "indent" => {
            context.style(block, "margin-left", "40px").await?;
        }
        "outdent" => {
            context.style(block, "margin-left", "").await?;
        }
        "insertorderedlist" | "insertunorderedlist" => {
            let list = if command == "insertorderedlist" {
                "ol"
            } else {
                "ul"
            };
            let list_node = insert_editing_element(context, list, &[]).await?;
            let children = light_children(&state.dom.borrow(), list_node)?;
            if !children
                .iter()
                .all(|child| element_local_name(&state.dom.borrow(), *child) == Some("li"))
            {
                let item = state.dom.borrow_mut().create_element("li");
                for child in children {
                    context.append(item, child, None).await?;
                }
                context.append(list_node, item, None).await?;
            }
        }
        "formatblock" => {
            let tag = value.trim_matches(|character| character == '<' || character == '>');
            let allowed = matches!(
                tag.to_ascii_lowercase().as_str(),
                "address"
                    | "article"
                    | "blockquote"
                    | "dd"
                    | "div"
                    | "dt"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "p"
                    | "pre"
            );
            if allowed {
                insert_editing_element(context, &tag.to_ascii_lowercase(), &[]).await?;
            }
        }
        "defaultparagraphseparator" => {
            let separator = value.to_ascii_lowercase();
            if matches!(separator.as_str(), "div" | "p") {
                *state.default_paragraph_separator.borrow_mut() = separator;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) async fn apply_inline_format(
    context: &Context,
    command: &str,
    value: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let collapsed = state.selection_anchor.get().is_some()
        && state.selection_anchor.get() == state.selection_focus.get();
    let mut overrides = state.editing_overrides.borrow().clone();
    let css = state.style_with_css.get();
    let (tag, attribute) = match command {
        "bold" if css => ("span", Some(("style", "font-weight: bold"))),
        "bold" => ("b", None),
        "italic" if css => ("span", Some(("style", "font-style: italic"))),
        "italic" => ("i", None),
        "underline" if css => ("span", Some(("style", "text-decoration: underline"))),
        "underline" => ("u", None),
        "strikethrough" if css => ("span", Some(("style", "text-decoration: line-through"))),
        "strikethrough" => ("strike", None),
        "subscript" => ("sub", None),
        "superscript" => ("sup", None),
        "createlink" => ("a", Some(("href", value))),
        "fontname" => ("span", Some(("style", value))),
        "fontsize" => ("span", Some(("style", value))),
        "forecolor" => ("span", Some(("style", value))),
        "backcolor" | "hilitecolor" => ("span", Some(("style", value))),
        _ => ("span", None),
    };
    let owned;
    let attributes = match command {
        "fontname" => {
            owned = format!("font-family: {value}");
            vec![("style", owned.as_str())]
        }
        "fontsize" => {
            let size = match value {
                "1" => "x-small",
                "2" => "small",
                "3" => "medium",
                "4" => "large",
                "5" => "x-large",
                "6" => "xx-large",
                "7" => "xxx-large",
                other => other,
            };
            owned = format!("font-size: {size}");
            vec![("style", owned.as_str())]
        }
        "forecolor" => {
            owned = format!("color: {value}");
            vec![("style", owned.as_str())]
        }
        "backcolor" | "hilitecolor" => {
            owned = format!("background-color: {value}");
            vec![("style", owned.as_str())]
        }
        _ => attribute.into_iter().collect(),
    };
    insert_editing_element(context, tag, &attributes).await?;
    if collapsed {
        if let Some(value) = selection_dom_format_value(state, command) {
            let key = if command == "backcolor" {
                "hilitecolor"
            } else {
                command
            };
            overrides.insert(key.to_string(), value);
        }
        context.overrides(overrides).await?;
    }
    Ok(())
}

// Editing moves keep boundaries attached to moved content instead of applying
// the ordinary DOM removal rule that relocates them to the old parent.

pub(super) async fn apply_typing_overrides(
    context: &Context,
    overrides: &EditingOverrides,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    for (command, value) in &overrides.0 {
        if selection_dom_format_value(state, command).as_ref() != Some(value) {
            apply_inline_format(context, command, value).await?;
        }
    }
    Ok(())
}

pub(super) async fn delete_selection_direction(
    context: &Context,
    forward: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        if join_rendered_editing_boundary(context, &range, forward).await? {
            return Ok(());
        }
        // Selecting only the boundary between adjacent paragraphs has the
        // same line-joining behavior as deleting at either collapsed endpoint.
        if let Some(host) = editing_host(state)? {
            let left = nearest_rendered_editing_block(state, range.start_container, host)?;
            let right = nearest_rendered_editing_block(state, range.end_container, host)?;
            let boundary_only = {
                let dom = state.dom.borrow();
                left != right
                    && dom.parent(left)? == dom.parent(right)?
                    && editing_point_is_at_block_edge(
                        &dom,
                        BoundaryPoint {
                            container: range.start_container,
                            offset: range.start_offset,
                        },
                        left,
                        true,
                    )?
                    && editing_point_is_at_block_edge(
                        &dom,
                        BoundaryPoint {
                            container: range.end_container,
                            offset: range.end_offset,
                        },
                        right,
                        false,
                    )?
                    && dom.parent(left)?.is_some_and(|parent| {
                        light_children(&dom, parent).is_ok_and(|children| {
                            children
                                .iter()
                                .skip_while(|node| **node != left)
                                .skip(1)
                                .find(|node| !is_ignored_editing_sibling(&dom, **node))
                                == Some(&right)
                        })
                    })
            };
            if boundary_only
                && merge_adjacent_editing_block(context, range.start_container, true).await?
            {
                return Ok(());
            }
        }
        let preserve_nested_caret =
            !forward && selection_ends_at_nested_block_start(state, &range)?;
        replace_selection_with_text(context, "").await?;
        if preserve_nested_caret && state.dom.borrow().arena().get(range.end_container).is_ok() {
            let point = BoundaryPoint {
                container: range.end_container,
                offset: 0,
            };
            set_selection_boundaries(state, point, point)?;
        }
        return Ok(());
    }
    let kind = state
        .dom
        .borrow()
        .arena()
        .get(range.start_container)?
        .kind
        .clone();
    if let NodeKind::Text { data } | NodeKind::CDataSection { data } = kind {
        let (start, end) = utf16_delete_span(&data, range.start_offset, forward);
        if start == end {
            if !forward && remove_line_before_nested_block(context, range.start_container).await? {
                let point = BoundaryPoint {
                    container: range.start_container,
                    offset: range.start_offset,
                };
                set_selection_boundaries(state, point, point)?;
                return Ok(());
            }
            if join_rendered_editing_boundary(context, &range, forward).await? {
                return Ok(());
            }
            return merge_adjacent_editing_block(context, range.start_container, forward)
                .await
                .map(|_| ());
        }
        context
            .text(range.start_container, start, end.saturating_sub(start), "")
            .await?;
        if forward
            && end == oryn_dom::dom_string::len(&data) as u32
            && move_caret_to_following_editing_block(
                state,
                range.start_container,
                range.start_offset,
            )?
        {
            return Ok(());
        }
        let point = BoundaryPoint {
            container: range.start_container,
            offset: start,
        };
        set_selection_boundaries(state, point, point)?;
    } else {
        if join_rendered_editing_boundary(context, &range, forward).await? {
            return Ok(());
        }
        let placeholder = node_is_placeholder_empty(&state.dom.borrow(), range.start_container)?;
        if placeholder
            && merge_adjacent_editing_block(context, range.start_container, forward).await?
        {
            return Ok(());
        }
        let adjacent = {
            let dom = state.dom.borrow();
            let children = dom.arena().get(range.start_container)?.children.clone();
            if forward {
                children.get(range.start_offset as usize).copied()
            } else {
                range
                    .start_offset
                    .checked_sub(1)
                    .and_then(|index| children.get(index as usize))
                    .copied()
            }
        };
        if let Some(adjacent) = adjacent {
            let adjacent_kind = state.dom.borrow().arena().get(adjacent)?.kind.clone();
            if let NodeKind::Text { data } | NodeKind::CDataSection { data } = adjacent_kind {
                let length = oryn_dom::dom_string::len(&data) as u32;
                let (start, end) =
                    utf16_delete_span(&data, if forward { 0 } else { length }, forward);
                context
                    .text(adjacent, start, end.saturating_sub(start), "")
                    .await?;
                let point = if forward {
                    BoundaryPoint {
                        container: range.start_container,
                        offset: range.start_offset,
                    }
                } else {
                    BoundaryPoint {
                        container: adjacent,
                        offset: start,
                    }
                };
                set_selection_boundaries(state, point, point)?;
            } else if matches!(
                element_local_name(&state.dom.borrow(), adjacent),
                Some("br")
            ) {
                let container = range.start_container;
                context.remove(adjacent).await?;
                if forward
                    && move_caret_to_following_editing_block(state, container, range.start_offset)?
                {
                    return Ok(());
                }
                if light_children(&state.dom.borrow(), container)?.is_empty()
                    && is_editing_block(&state.dom.borrow(), container)
                {
                    let br = state.dom.borrow_mut().create_element("br");
                    context.append(container, br, None).await?;
                }
                let point = BoundaryPoint {
                    container,
                    offset: (if forward {
                        range.start_offset
                    } else {
                        range.start_offset.saturating_sub(1)
                    })
                    .min(boundary_length(&state.dom.borrow(), container)?),
                };
                set_selection_boundaries(state, point, point)?;
            } else {
                let trailing_break = (!forward)
                    .then(|| {
                        light_children(&state.dom.borrow(), range.start_container)
                            .ok()?
                            .get(range.start_offset as usize)
                            .copied()
                    })
                    .flatten()
                    .filter(|node| element_local_name(&state.dom.borrow(), *node) == Some("br"));
                if let Some(trailing_break) = trailing_break {
                    context.remove(trailing_break).await?;
                    let preceding = light_children(&state.dom.borrow(), range.start_container)?
                        .get(range.start_offset.saturating_sub(1) as usize)
                        .copied();
                    let point = if let Some(preceding) = preceding {
                        if node_is_placeholder_empty(&state.dom.borrow(), preceding)? {
                            editing_placeholder_caret(&state.dom.borrow(), preceding)?
                        } else {
                            BoundaryPoint {
                                container: preceding,
                                offset: editing_caret_end(&state.dom.borrow(), preceding)?,
                            }
                        }
                    } else {
                        BoundaryPoint {
                            container: range.start_container,
                            offset: range.start_offset.saturating_sub(1),
                        }
                    };
                    set_selection_boundaries(state, point, point)?;
                } else {
                    merge_adjacent_editing_block(context, adjacent, forward).await?;
                }
            }
        } else {
            merge_adjacent_editing_block(context, range.start_container, forward).await?;
        }
    }
    Ok(())
}

// Editing decisions use the same resolved style snapshot as getComputedStyle.
// Binding-only embedders without a layout provider retain an inherited fallback.

pub(super) async fn insert_editing_element(
    context: &Context,
    tag: &str,
    attributes: &[(&str, &str)],
) -> Result<NodeId, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let marker = context.marker(RangeRecord {
        start_container: range.start_container,
        start_offset: range.start_offset,
        end_container: range.start_container,
        end_offset: range.start_offset,
    })?;
    let fragment = editing_range::contents(context, &range, RangeContentsMode::Extract).await?;
    synchronize_live_ranges(state)?;
    let insertion = marker.range()?;
    drop(marker);
    let element = state.dom.borrow_mut().create_element(tag);
    for (name, value) in attributes {
        context.attribute(element, None, *name, *value).await?;
    }
    let fragment_children = state.dom.borrow().children(fragment)?.to_vec();
    for child in fragment_children {
        context.append(element, child, None).await?;
    }
    context
        .insert_at_boundary(insertion.start_container, insertion.start_offset, element)
        .await?;
    let end = light_children(&state.dom.borrow(), element)?.len() as u32;
    set_selection_boundaries(
        state,
        BoundaryPoint {
            container: element,
            offset: 0,
        },
        BoundaryPoint {
            container: element,
            offset: end,
        },
    )?;
    Ok(element)
}

pub(super) async fn insert_editing_line_break(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    replace_selection_with_text(context, "").await?;
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let whitespace = editing_white_space(state, range.start_container)?;
    // Preformatted HTML containers use an explicit break element for editing
    // commands, even though their text layout preserves literal newlines.
    let preformatted_container = {
        let dom = state.dom.borrow();
        ["pre", "listing"]
            .into_iter()
            .any(|tag| has_ancestor_element_before(&dom, range.start_container, tag, host))
    };
    let preserves = !preformatted_container
        && matches!(
            whitespace.as_str(),
            "pre" | "pre-wrap" | "pre-line" | "break-spaces"
        );
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    let needs_placeholder = !editing_line_has_following_content(state, point, host, &whitespace)?;
    if preserves {
        replace_selection_with_text(context, "\n").await?;
        let range = state
            .ranges
            .borrow()
            .get(&id)
            .cloned()
            .ok_or(V8BindingError::Allocation("selection range"))?;
        point = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
    } else {
        let mut dom = state.dom.borrow_mut();
        if matches!(
            dom.arena().get(point.container)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            let length = boundary_length(&dom, point.container)?;
            if point.offset == 0 || point.offset == length {
                let (container, offset) = relative_boundary(
                    &dom,
                    point.container,
                    point.offset == length && length != 0,
                )?;
                point = BoundaryPoint { container, offset };
            }
        }
        let br = dom.create_element("br");
        drop(dom);
        context
            .insert_at_boundary(point.container, point.offset, br)
            .await?;
        let mut dom = state.dom.borrow_mut();
        let (container, offset) = relative_boundary(&dom, br, true)?;
        point = BoundaryPoint { container, offset };
    }
    if needs_placeholder {
        let mut dom = state.dom.borrow_mut();
        let br = dom.create_element("br");
        drop(dom);
        context
            .insert_at_boundary(point.container, point.offset, br)
            .await?;
        let mut dom = state.dom.borrow_mut();
    }
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

// Pasted markup carries its own inline formatting. Split the containing inline
// ancestors up to the rendered block, preserving the content on either side.

pub(super) async fn insert_editing_text(
    context: &Context,
    text: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let overrides = state.editing_overrides.borrow().clone();
    let units = oryn_dom::dom_string::units(text);
    let mut lines = units
        .split(|unit| *unit == 10)
        .map(oryn_dom::dom_string::from_units);
    let first = lines.next().unwrap_or_default();
    replace_selection_with_text(context, "").await?;
    if !first.is_empty() {
        apply_typing_overrides(context, &overrides).await?;
        remove_editing_placeholder_before_typing(context, &first).await?;
        let prepared = prepare_typing_whitespace(state, &first)?;
        replace_selection_with_text(context, &prepared).await?;
    }
    if text.is_empty() {
        prepare_typing_whitespace(state, text)?;
    }
    for line in lines {
        split_editing_paragraph(context).await?;
        if !line.is_empty() {
            apply_typing_overrides(context, &overrides).await?;
            remove_editing_placeholder_before_typing(context, &line).await?;
            let prepared = prepare_typing_whitespace(state, &line)?;
            replace_selection_with_text(context, &prepared).await?;
        }
    }
    context.overrides(overrides).await?;
    Ok(())
}

// The inserted text will give this line a caret position, so remove its
// redundant placeholder first. Text insertion must be the final DOM mutation
// exposed to editors. Keep visible breaks and stay inside this editing block.

pub(super) async fn join_first_editing_line(
    context: &Context,
    destination: NodeId,
    source: NodeId,
) -> Result<BoundaryPoint, V8BindingError> {
    let state: &V8BindingState = &context.state;
    join_first_editing_line_before(context, destination, source, None).await
}

pub(super) async fn join_first_editing_line_before(
    context: &Context,
    destination: NodeId,
    source: NodeId,
    before: Option<NodeId>,
) -> Result<BoundaryPoint, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let destination_mode = editing_white_space(state, destination)?;
    let source_is_placeholder = matches!(
        state.dom.borrow().arena().get(source)?.kind,
        NodeKind::Element { .. }
    ) && node_is_placeholder_empty(&state.dom.borrow(), source)?;
    let mut line_root = source;
    loop {
        let first = light_children(&state.dom.borrow(), line_root)?
            .into_iter()
            .find(|node| !is_ignored_editing_sibling(&state.dom.borrow(), *node));
        let Some(first) = first else { break };
        if nearest_rendered_editing_block(state, first, source)? != first {
            break;
        }
        line_root = first;
    }
    let source_mode = editing_white_space(state, line_root)?;
    // Matching whitespace modes can retain the source's internal line breaks
    // in a single merged block. Different modes and nested source blocks use
    // the first-line join, preserving the remaining source paragraph's style.
    let host = editing_host(state)?.unwrap_or(destination);
    let merge_whole_block = before.is_none()
        && line_root == source
        && source_mode == destination_mode
        && nearest_rendered_editing_block(state, destination, host)? == destination
        && nearest_rendered_editing_block(state, source, host)? == source;
    let source_is_inline = matches!(
        state.dom.borrow().arena().get(line_root)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) || state
        .layout
        .borrow()
        .styles
        .get(&node_key(line_root))
        .and_then(|style| style.get("display"))
        .is_some_and(|display| display == "inline");
    // Semantic inline elements are part of the moved line, rather than its
    // paragraph container. A span used as a whitespace carrier is rebuilt below.
    let preserve_inline_shell = source_is_inline
        && matches!(
            state.dom.borrow().arena().get(line_root)?.kind,
            NodeKind::Element { .. }
        )
        && element_local_name(&state.dom.borrow(), line_root) != Some("span");
    let destination_absorbs_inline_whitespace = source_is_inline
        && editing_host(state)?.is_some_and(|host| {
            let dom = state.dom.borrow();
            ["pre", "listing"]
                .into_iter()
                .any(|tag| has_ancestor_element_before(&dom, destination, tag, host))
        });
    let source_has_author_whitespace = {
        let dom = state.dom.borrow();
        let style_node = if matches!(
            dom.arena().get(line_root)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            dom.parent(line_root)?.unwrap_or(line_root)
        } else {
            line_root
        };
        state
            .layout
            .borrow()
            .author_whitespace
            .contains(&node_key(style_node))
    };
    // Reuse a single inline container when that preserves its computed mode
    // without rewriting an existing inline declaration block.
    let reusable_inline = light_children(&state.dom.borrow(), line_root)?
        .first()
        .copied()
        .is_some_and(|node| {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            matches!(
                dom.arena().get(node).map(|n| &n.kind),
                Ok(NodeKind::Element { .. })
            ) && dom
                .attribute_in_null_namespace(node, "style")
                .ok()
                .flatten()
                .is_none()
                && layout.styles.get(&node_key(node)).is_some_and(|style| {
                    style
                        .get("display")
                        .is_some_and(|display| display == "inline")
                        && style.get("white-space") == Some(&source_mode)
                })
        });
    let mut end = BoundaryPoint {
        container: line_root,
        offset: boundary_length(&state.dom.borrow(), line_root)?,
    };
    let mut separator = None;
    let mut cleanup = Vec::new();
    let mut line_nodes = if matches!(
        state.dom.borrow().arena().get(line_root)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) {
        vec![line_root]
    } else {
        light_children(&state.dom.borrow(), line_root)?
    };
    line_nodes.reverse();
    while let Some(node) = line_nodes.pop() {
        let kind = state.dom.borrow().arena().get(node)?.kind.clone();
        if is_preserved_editing_boundary(&state.dom.borrow(), node) {
            let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
            end = BoundaryPoint { container, offset };
            break;
        }
        match kind {
            NodeKind::Element { .. } => {
                let hidden = state
                    .layout
                    .borrow()
                    .styles
                    .get(&node_key(node))
                    .and_then(|style| style.get("display"))
                    .is_some_and(|display| display == "none");
                if hidden || is_ignored_editing_sibling(&state.dom.borrow(), node) {
                    continue;
                }
                if !merge_whole_block && element_local_name(&state.dom.borrow(), node) == Some("br")
                {
                    let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
                    end = BoundaryPoint { container, offset };
                    separator = Some((node, false));
                    break;
                }
                if nearest_rendered_editing_block(state, node, line_root)? == node {
                    let (container, offset) = relative_boundary(&state.dom.borrow(), node, false)?;
                    end = BoundaryPoint { container, offset };
                    break;
                }
                line_nodes.extend(light_children(&state.dom.borrow(), node)?.into_iter().rev());
            }
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                let mode = editing_white_space(state, node)?;
                if !merge_whole_block
                    && matches!(
                        mode.as_str(),
                        "pre" | "pre-wrap" | "pre-line" | "break-spaces"
                    )
                    && let Some(index) = oryn_dom::dom_string::units(&data)
                        .iter()
                        .position(|unit| *unit == 10)
                {
                    end = BoundaryPoint {
                        container: node,
                        offset: index as u32,
                    };
                    separator = Some((node, true));
                    break;
                }
            }
            _ => {}
        }
    }
    // The paragraph's first line can end at a break immediately outside an
    // inline source container (including a bare text node).
    let following_break = if source_is_inline
        && !source_is_placeholder
        && separator.is_none()
        && end.container == line_root
        && end.offset == boundary_length(&state.dom.borrow(), line_root)?
    {
        let dom = state.dom.borrow();
        dom.parent(source)?
            .map(|parent| light_children(&dom, parent))
            .transpose()?
            .and_then(|children| {
                children
                    .windows(2)
                    .find(|pair| pair[0] == source)
                    .map(|pair| pair[1])
            })
            .filter(|node| element_local_name(&dom, *node) == Some("br"))
    } else {
        None
    };
    if let Some((node, _)) = separator {
        let mut current = Some(node);
        while let Some(node) = current {
            cleanup.push(node);
            if node == line_root {
                break;
            }
            current = state.dom.borrow().parent(node)?;
        }
    }
    let mut dom = state.dom.borrow_mut();
    let caret_offset = if let Some(before) = before {
        relative_boundary(&dom, before, false)?.1
    } else {
        editing_caret_end(&dom, destination)?
    };
    drop(dom);
    let fragment = editing_range::contents(
        context,
        &RangeRecord {
            start_container: line_root,
            start_offset: 0,
            end_container: end.container,
            end_offset: end.offset,
        },
        RangeContentsMode::Extract,
    )
    .await?;
    let mut dom = state.dom.borrow_mut();
    if let Some((node, newline)) = separator {
        if newline {
            {
                drop(dom);
                let result = context.text(node, 0, 1, "").await;
                dom = state.dom.borrow_mut();
                result
            }?;
        } else if let Some(parent) = dom.parent(node)? {
            {
                drop(dom);
                let result = context.remove(node).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
    }
    for node in cleanup {
        if node == line_root {
            break;
        }
        let empty = match &dom.arena().get(node)?.kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } => data.is_empty(),
            NodeKind::Element { .. } => dom.arena().get(node)?.children.is_empty(),
            _ => false,
        };
        if empty && let Some(parent) = dom.parent(node)? {
            {
                drop(dom);
                let result = context.remove(node).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
    }
    if let Some(node) = following_break
        && let Some(parent) = dom.parent(node)?
    {
        {
            drop(dom);
            let result = context.remove(node).await;
            dom = state.dom.borrow_mut();
            result
        }?;
    }
    if preserve_inline_shell && !source_is_placeholder {
        let children = light_children(&dom, fragment)?;
        if !children.is_empty() {
            let shell = if light_children(&dom, line_root)?.is_empty() {
                // Reuse a wholly consumed element so listeners and identity survive.
                line_root
            } else {
                dom.clone_node(line_root, false)?
            };
            for child in children {
                {
                    drop(dom);
                    let result = context.append(shell, child, None).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
            }
            {
                drop(dom);
                let result = context.append(fragment, shell, None).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
    }
    let moving = if source_is_placeholder {
        Vec::new()
    } else {
        light_children(&dom, fragment)?
    };
    if !moving.is_empty() {
        // A final placeholder is replaced by the joined line.
        if before.is_none()
            && let Some(last) = light_children(&dom, destination)?.last().copied()
            && element_local_name(&dom, last) == Some("br")
        {
            {
                drop(dom);
                let result = context.remove(last).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
        let target = if source_mode != destination_mode
            && !destination_absorbs_inline_whitespace
            && (source_has_author_whitespace || source_is_inline)
        {
            if reusable_inline
                && !preserve_inline_shell
                && moving.len() == 1
                && matches!(dom.arena().get(moving[0])?.kind, NodeKind::Element { .. })
            {
                drop(dom);
                context
                    .attribute(
                        moving[0],
                        None,
                        "style",
                        format!("white-space: {source_mode};"),
                    )
                    .await?;
                dom = state.dom.borrow_mut();
                destination
            } else {
                let span = dom.create_element("span");
                drop(dom);
                context
                    .attribute(span, None, "style", format!("white-space: {source_mode};"))
                    .await?;
                dom = state.dom.borrow_mut();
                {
                    drop(dom);
                    let result = context.append(destination, span, before).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
                span
            }
        } else {
            destination
        };
        for child in moving {
            {
                drop(dom);
                let result = context
                    .append(
                        target,
                        child,
                        if target == destination { before } else { None },
                    )
                    .await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
    }
    let mut empty = line_root;
    while match &dom.arena().get(empty)?.kind {
        NodeKind::Text { data } | NodeKind::CDataSection { data } => data.is_empty(),
        _ => light_children(&dom, empty)?.is_empty(),
    } {
        let Some(parent) = dom.parent(empty)? else {
            break;
        };
        {
            drop(dom);
            let result = context.remove(empty).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        if empty == source {
            break;
        }
        empty = parent;
    }
    if node_is_placeholder_empty(&dom, destination)? {
        return editing_placeholder_caret(&dom, destination);
    }
    Ok(BoundaryPoint {
        container: destination,
        offset: caret_offset.min(boundary_length(&dom, destination)?),
    })
}

pub(super) async fn join_rendered_editing_boundary(
    context: &Context,
    range: &RangeRecord,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    if element_local_name(&state.dom.borrow(), host) == Some("html") {
        return Ok(false);
    }
    refresh_layout(state)?;
    let selected =
        range.start_container != range.end_container || range.start_offset != range.end_offset;
    let start = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    let Some((left, right)) = rendered_editing_boundary(state, start, host, selected || forward)?
    else {
        return Ok(false);
    };
    if selected {
        let end = BoundaryPoint {
            container: range.end_container,
            offset: range.end_offset,
        };
        if rendered_editing_boundary(state, end, host, false)? != Some((left, right)) {
            return Ok(false);
        }
    }
    let parent = state
        .dom
        .borrow()
        .parent(left)?
        .ok_or(V8BindingError::Allocation("editing boundary parent"))?;
    let left_is_block = nearest_rendered_editing_block(state, left, host)? == left;
    let (mut destination, before) = if left_is_block {
        (left, None)
    } else {
        (parent, Some(right))
    };
    if left_is_block {
        loop {
            let children = light_children(&state.dom.borrow(), destination)?;
            let mut last = None;
            for child in children.into_iter().rev() {
                if !editing_join_ignored(state, child)? {
                    last = Some(child);
                    break;
                }
            }
            let Some(last) = last else {
                break;
            };
            if editing_join_structural_boundary(&state.dom.borrow(), last)
                || nearest_rendered_editing_block(state, last, host)? != last
            {
                break;
            }
            destination = last;
        }
    }
    let children = light_children(&state.dom.borrow(), parent)?;
    let left_index = children
        .iter()
        .position(|node| *node == left)
        .ok_or(V8BindingError::Allocation("editing left unit"))?;
    let right_index = children
        .iter()
        .position(|node| *node == right)
        .ok_or(V8BindingError::Allocation("editing right unit"))?;
    for node in &children[left_index + 1..right_index] {
        if matches!(
            state.dom.borrow().arena().get(*node)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            context.remove(*node).await?;
        }
    }
    let point = join_first_editing_line_before(context, destination, right, before).await?;
    set_selection_boundaries(state, point, point)?;
    Ok(true)
}

pub(super) async fn merge_adjacent_editing_block(
    context: &Context,
    container: NodeId,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    let (block, parent, sibling, ignored) = {
        let dom = state.dom.borrow();
        let block = nearest_editing_block(&dom, container, host)?;
        if block == host {
            drop(dom);
            return merge_document_root_editing_boundary(context, container, host, forward).await;
        }
        let Some(parent) = dom.parent(block)? else {
            return Ok(false);
        };
        let children = light_children(&dom, parent)?;
        let Some(index) = children.iter().position(|node| *node == block) else {
            return Ok(false);
        };
        let sibling_index = if forward {
            children
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, candidate)| !is_ignored_editing_sibling(&dom, **candidate))
                .map(|(index, _)| index)
        } else {
            children[..index]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, candidate)| !is_ignored_editing_sibling(&dom, **candidate))
                .map(|(index, _)| index)
        };
        let sibling = sibling_index.map(|index| children[index]);
        let ignored = sibling_index.map_or_else(Vec::new, |sibling_index| {
            let range = if forward {
                index + 1..sibling_index
            } else {
                sibling_index + 1..index
            };
            children[range]
                .iter()
                .copied()
                .filter(|candidate| is_ignored_editing_sibling(&dom, *candidate))
                .collect()
        });
        (block, parent, sibling, ignored)
    };
    if let Some(sibling) = sibling
        && state.dom.borrow().arena().get(sibling).is_ok()
        && matches!(
            state.dom.borrow().arena().get(sibling)?.kind,
            NodeKind::Element { .. }
        )
        && is_mergeable_editing_sibling(&state.dom.borrow(), sibling)
    {
        let (destination, source) = if forward {
            (block, sibling)
        } else {
            (sibling, block)
        };
        for separator in ignored {
            let mut dom = state.dom.borrow_mut();
            if matches!(dom.arena().get(separator).map(|record| &record.kind),
                Ok(NodeKind::Text { data } | NodeKind::CDataSection { data }) if data.trim().is_empty())
            {
                {
                    drop(dom);
                    let result = context.remove(separator).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
            }
        }
        let point = join_first_editing_line(context, destination, source).await?;
        set_selection_boundaries(state, point, point)?;
        return Ok(true);
    }

    // An empty final nested list/list-item collapses back into its containing
    // editable item rather than leaving an unreachable empty sub-list.
    if node_is_placeholder_empty(&state.dom.borrow(), block)?
        && matches!(
            element_local_name(&state.dom.borrow(), block),
            Some("li" | "dt" | "dd")
        )
        && matches!(
            element_local_name(&state.dom.borrow(), parent),
            Some("ul" | "ol" | "dl")
        )
        && parent != host
    {
        let grand = state.dom.borrow().parent(parent)?.unwrap_or(host);
        let grand_tag = element_local_name(&state.dom.borrow(), grand).map(str::to_string);
        if matches!(grand_tag.as_deref(), Some("li" | "dt" | "dd")) {
            context.remove(parent).await?;
        } else {
            let replacement_tag = if grand_tag.as_deref() == Some("dl") {
                "dd"
            } else {
                "li"
            };
            let replacement =
                if element_local_name(&state.dom.borrow(), block) == Some(replacement_tag) {
                    block
                } else {
                    // Flatten into the item type required by the containing list.
                    let item = state.dom.borrow_mut().create_element(replacement_tag);
                    let children = light_children(&state.dom.borrow(), block)?;
                    for child in children {
                        context.append(item, child, None).await?;
                    }
                    item
                };
            context.append(grand, replacement, Some(parent)).await?;
            context.remove(parent).await?;
        }
        let point = BoundaryPoint {
            container: grand,
            offset: editing_caret_end(&state.dom.borrow(), grand)?,
        };
        set_selection_boundaries(state, point, point)?;
        return Ok(true);
    }
    merge_document_root_editing_boundary(context, container, host, forward).await
}

pub(super) async fn replace_selection_with_html(
    context: &Context,
    markup: &str,
    pasted_markup: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let overrides = state.editing_overrides.borrow().clone();
    replace_selection_with_text(context, "").await?;
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let parsed = oryn_dom::html::parse_fragment_dom_string_with_context(
        markup,
        markup5ever::QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "div".into()),
        Vec::new(),
        false,
    );
    let document_children = parsed.dom.get(parsed.document)?.children.clone();
    let children = document_children
        .first()
        .and_then(|root| parsed.dom.get(*root).ok())
        .filter(|root| {
            matches!(&root.kind, NodeKind::Element { name } if name.local.as_ref() == "html")
        })
        .map(|root| root.children.clone())
        .unwrap_or(document_children);
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    if children.is_empty() {
        return Ok(());
    }
    if pasted_markup {
        point = html_insertion_boundary(state, point)?;
    }
    let mut last_inserted = None;
    for child in children {
        let imported = state.dom.borrow_mut().import_subtree(&parsed.dom, child)?;
        context
            .insert_at_boundary(point.container, point.offset, imported)
            .await?;
        last_inserted = Some(imported);
        let (container, offset) = relative_boundary(&state.dom.borrow(), imported, true)?;
        point = BoundaryPoint { container, offset };
    }
    if pasted_markup && let Some(mut last) = last_inserted {
        let dom = state.dom.borrow();
        while let Some(child) = light_children(&dom, last)?.last().copied() {
            last = child;
        }
        if matches!(
            dom.arena().get(last)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            point = BoundaryPoint {
                container: last,
                offset: boundary_length(&dom, last)?,
            };
        }
    }
    set_selection_boundaries(state, point, point)?;
    context.overrides(overrides).await?;
    Ok(())
}

pub(super) async fn replace_selection_with_text(
    context: &Context,
    replacement: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "no active selection".into(),
        })?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let affected_blocks = editing_host(state)?.and_then(|host| {
        let dom = state.dom.borrow();
        let start = nearest_editing_block(&dom, range.start_container, host).ok()?;
        let end = nearest_editing_block(&dom, range.end_container, host).ok()?;
        (start != end).then_some((start, end))
    });
    let has_selection = compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0;
    let preserve_nested_caret =
        has_selection && selection_ends_at_nested_block_start(state, &range)?;
    if has_selection {
        editing_range::contents(context, &range, RangeContentsMode::Delete).await?;
        if let Some((start_block, end_block)) = affected_blocks
            && state.dom.borrow().arena().get(start_block).is_ok()
            && state.dom.borrow().arena().get(end_block).is_ok()
            && state.dom.borrow().parent(start_block)? == state.dom.borrow().parent(end_block)?
        {
            let end_children = light_children(&state.dom.borrow(), end_block)?;
            if node_is_placeholder_empty(&state.dom.borrow(), end_block)? {
                let parent = state
                    .dom
                    .borrow()
                    .parent(end_block)?
                    .ok_or(V8BindingError::Allocation("editing block parent"))?;
                context.remove(end_block).await?;
            } else {
                for child in end_children {
                    context.append(start_block, child, None).await?;
                }
                let parent = state
                    .dom
                    .borrow()
                    .parent(end_block)?
                    .ok_or(V8BindingError::Allocation("editing block parent"))?;
                context.remove(end_block).await?;
            }
            if light_children(&state.dom.borrow(), start_block)?.is_empty() {
                let br = state.dom.borrow_mut().create_element("br");
                context.append(start_block, br, None).await?;
            }
        }
    }
    let mut point = BoundaryPoint {
        container: if preserve_nested_caret {
            range.end_container
        } else {
            range.start_container
        },
        offset: if preserve_nested_caret {
            0
        } else {
            range.start_offset
        },
    };
    if !replacement.is_empty() {
        let kind = state
            .dom
            .borrow()
            .arena()
            .get(point.container)?
            .kind
            .clone();
        match kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                let _ = data;
                context
                    .text(point.container, point.offset, 0, replacement)
                    .await?;
                point.offset = point
                    .offset
                    .saturating_add(oryn_dom::dom_string::len(replacement) as u32);
            }
            _ => {
                let text = state.dom.borrow_mut().create_text(replacement);
                context
                    .insert_at_boundary(point.container, point.offset, text)
                    .await?;
                let (container, offset) = relative_boundary(&state.dom.borrow(), text, true)?;
                point = BoundaryPoint { container, offset };
            }
        }
    }
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

pub(super) async fn split_editing_paragraph(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let overrides = state.editing_overrides.borrow().clone();
    let result = split_editing_paragraph_contents(context).await;
    context.overrides(overrides).await?;
    result
}

pub(super) async fn split_editing_paragraph_contents(
    context: &Context,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let Some(range_id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&range_id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        replace_selection_with_text(context, "").await?;
    }
    synchronize_live_ranges(state)?;
    let range = state
        .ranges
        .borrow()
        .get(&range_id)
        .cloned()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let block = nearest_rendered_editing_block(state, range.start_container, host)?;
    // These containers represent several lines or table structure. A paragraph
    // command must not clone the container itself into a sibling table cell or
    // a second preformatted/address element.
    let multiline_container = matches!(
        element_local_name(&state.dom.borrow(), block),
        Some("pre" | "listing" | "address" | "td" | "th" | "caption")
    );
    let host_accepts_paragraphs = {
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let flow_container = matches!(
            element_local_name(&dom, host),
            Some(
                "body"
                    | "div"
                    | "main"
                    | "section"
                    | "article"
                    | "aside"
                    | "nav"
                    | "header"
                    | "footer"
                    | "blockquote"
                    | "fieldset"
                    | "figure"
                    | "figcaption"
                    | "li"
                    | "dd"
            )
        );
        let block_display = layout
            .styles
            .get(&node_key(host))
            .and_then(|style| style.get("display"))
            .map_or_else(
                || is_editing_block(&dom, host) || element_local_name(&dom, host) == Some("body"),
                |display| {
                    matches!(
                        display.as_str(),
                        "block" | "flow-root" | "list-item" | "flex" | "grid"
                    )
                },
            );
        flow_container && block_display
    };
    if (block == host && !host_accepts_paragraphs)
        || multiline_container
        || *state.default_paragraph_separator.borrow() == "br"
    {
        return insert_editing_line_break(context).await;
    }
    // Capture existing block boundaries before splitting inline ancestors.
    let mut block_siblings = std::collections::HashSet::new();
    if block == host {
        // `nearest_rendered_editing_block` has already refreshed layout above.
        // Reusing that snapshot avoids an O(children) sequence of full layout
        // refreshes for every paragraph command in a block editing host.
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let children = light_children(&dom, host)?;
        for child in children {
            let display = layout
                .styles
                .get(&node_key(child))
                .and_then(|style| style.get("display"));
            let is_block = display.map_or_else(
                || is_editing_block(&dom, child),
                |display| {
                    matches!(
                        display.as_str(),
                        "block"
                            | "flow-root"
                            | "list-item"
                            | "table-cell"
                            | "table-caption"
                            | "flex"
                            | "grid"
                    )
                },
            );
            if is_block {
                block_siblings.insert(child);
            }
        }
    }
    let mut dom = state.dom.borrow_mut();
    let mut right = match dom.arena().get(range.start_container)?.kind.clone() {
        NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
            drop(dom);
            let result = context
                .split(range.start_container, range.start_offset)
                .await;
            dom = state.dom.borrow_mut();
            result
        }?,
        _ => {
            let children = light_children(&dom, range.start_container)?;
            if let Some(child) = children.get(range.start_offset as usize).copied() {
                child
            } else {
                let br = dom.create_element("br");
                {
                    drop(dom);
                    let result = context.append(range.start_container, br, None).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
                br
            }
        }
    };

    loop {
        let parent = dom
            .parent(right)?
            .ok_or(V8BindingError::Allocation("editing split parent"))?;
        if parent == block {
            break;
        }
        let clone = clone_editing_element(&mut dom, parent)?;
        let siblings = light_children(&dom, parent)?;
        let index = siblings
            .iter()
            .position(|candidate| *candidate == right)
            .ok_or(V8BindingError::Allocation("editing split child"))?;
        for child in siblings[index..].iter().copied() {
            {
                drop(dom);
                let result = context.append(clone, child, None).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
        let grand = dom
            .parent(parent)?
            .ok_or(V8BindingError::Allocation("editing split ancestor"))?;
        {
            drop(dom);
            let result = context.after(grand, parent, clone).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        right = clone;
    }

    let siblings = light_children(&dom, block)?;
    let index = siblings
        .iter()
        .position(|candidate| *candidate == right)
        .ok_or(V8BindingError::Allocation("editing paragraph boundary"))?;
    let new_block;
    if block == host {
        // Wrap the current inline run only. Other paragraphs retain their
        // identity, position and contents.
        let start = siblings[..index]
            .iter()
            .rposition(|node| block_siblings.contains(node))
            .map_or(0, |position| position + 1);
        let end = siblings[index..]
            .iter()
            .position(|node| block_siblings.contains(node))
            .map_or(siblings.len(), |position| index + position);
        let separator = state.default_paragraph_separator.borrow().clone();
        let needs_div = siblings[start..end].iter().any(|node| {
            traversal_nodes(&dom, *node)
                .is_ok_and(|nodes| nodes.into_iter().any(|child| is_editing_block(&dom, child)))
        });
        let tag = if separator.as_str() == "p" && !needs_div {
            "p"
        } else {
            "div"
        };
        let left_block = dom.create_element(tag);
        new_block = dom.create_element(tag);
        {
            drop(dom);
            let result = context
                .append(host, left_block, siblings.get(start).copied())
                .await;
            dom = state.dom.borrow_mut();
            result
        }?;
        {
            drop(dom);
            let result = context.after(host, left_block, new_block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        for child in siblings[start..index].iter().copied() {
            {
                drop(dom);
                let result = context.append(left_block, child, None).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
        for child in siblings[index..end].iter().copied() {
            {
                drop(dom);
                let result = context.append(new_block, child, None).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
        {
            drop(dom);
            let result = ensure_split_paragraph_placeholder(context, left_block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        {
            drop(dom);
            let result = ensure_split_paragraph_placeholder(context, new_block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        // Host wrapping uses the transaction snapshot for undo; the special
        // block-split undo assumes that both halves existed as sibling blocks.
    } else {
        new_block = clone_editing_element(&mut dom, block)?;
        for child in siblings[index..].iter().copied() {
            {
                drop(dom);
                let result = context.append(new_block, child, None).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
        let left_placeholder = {
            drop(dom);
            let result = ensure_split_paragraph_placeholder(context, block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        let right_placeholder = {
            drop(dom);
            let result = ensure_split_paragraph_placeholder(context, new_block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        let parent = dom
            .parent(block)?
            .ok_or(V8BindingError::Allocation("editing block parent"))?;
        {
            drop(dom);
            let result = context.after(parent, block, new_block).await;
            dom = state.dom.borrow_mut();
            result
        }?;
        state.pending_split_undo.set(Some((
            block,
            new_block,
            left_placeholder,
            right_placeholder,
        )));
    }
    drop(dom);
    let caret = traversal_nodes(&state.dom.borrow(), new_block)?
        .into_iter()
        .find(|node| {
            state.dom.borrow().arena().get(*node).is_ok_and(|record| {
                matches!(
                    record.kind,
                    NodeKind::Text { .. } | NodeKind::CDataSection { .. }
                )
            })
        })
        .unwrap_or(new_block);
    let point = BoundaryPoint {
        container: caret,
        offset: 0,
    };
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

pub(super) async fn merge_document_root_editing_boundary(
    context: &Context,
    container: NodeId,
    host: NodeId,
    forward: bool,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    if element_local_name(&state.dom.borrow(), host) != Some("html") {
        return Ok(false);
    }
    let (current, adjacent) = {
        let dom = state.dom.borrow();
        let Some(current) = document_root_editing_unit(&dom, container, host)? else {
            return Ok(false);
        };
        let mut units = Vec::new();
        collect_document_root_editing_units(&dom, host, &mut units)?;
        let Some(index) = units.iter().position(|candidate| *candidate == current) else {
            return Ok(false);
        };
        let adjacent = if forward {
            units.get(index + 1).copied()
        } else {
            index
                .checked_sub(1)
                .and_then(|index| units.get(index))
                .copied()
        };
        (current, adjacent)
    };
    let Some(adjacent) = adjacent else {
        return Ok(false);
    };
    let (left, right) = if forward {
        (current, adjacent)
    } else {
        (adjacent, current)
    };
    {
        let dom = state.dom.borrow();
        if !matches!(element_local_name(&dom, left), Some("body")) && !is_editing_block(&dom, left)
            || !matches!(element_local_name(&dom, right), Some("body"))
                && !is_editing_block(&dom, right)
        {
            return Ok(false);
        }
    }

    let destination = {
        let dom = state.dom.borrow();
        let left_in_body = has_ancestor_element_before(&dom, left, "body", host);
        let right_in_body = has_ancestor_element_before(&dom, right, "body", host);
        if right_in_body && !left_in_body {
            right
        } else {
            left
        }
    };
    let source = if destination == left { right } else { left };
    let destination_was_empty = node_is_placeholder_empty(&state.dom.borrow(), destination)?;
    let source_was_empty = node_is_placeholder_empty(&state.dom.borrow(), source)?;
    let destination_children = light_children(&state.dom.borrow(), destination)?;
    let source_children = light_children(&state.dom.borrow(), source)?;
    let caret_offset = if destination == left {
        editing_caret_end(&state.dom.borrow(), destination)?
    } else {
        source_children
            .iter()
            .filter(|child| !is_preserved_editing_boundary(&state.dom.borrow(), **child))
            .count() as u32
    };
    let moving = source_children
        .into_iter()
        .filter(|child| !is_preserved_editing_boundary(&state.dom.borrow(), *child))
        .collect::<Vec<_>>();
    {
        let mut dom = state.dom.borrow_mut();
        if destination_was_empty {
            for child in destination_children {
                if element_local_name(&dom, child) == Some("br") {
                    {
                        drop(dom);
                        let result = context.remove(child).await;
                        dom = state.dom.borrow_mut();
                        result
                    }?;
                }
            }
        }
        if destination == left {
            for child in moving {
                {
                    drop(dom);
                    let result = context.append(destination, child, None).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
            }
        } else {
            let before = light_children(&dom, destination)?.first().copied();
            for child in moving {
                {
                    drop(dom);
                    let result = context.append(destination, child, before).await;
                    dom = state.dom.borrow_mut();
                    result
                }?;
            }
        }
        if (source_was_empty || light_children(&dom, source)?.is_empty())
            && element_local_name(&dom, source) != Some("body")
            && let Some(parent) = dom.parent(source)?
        {
            {
                drop(dom);
                let result = context.remove(source).await;
                dom = state.dom.borrow_mut();
                result
            }?;
        }
    }
    let point = if node_is_placeholder_empty(&state.dom.borrow(), destination)? {
        editing_placeholder_caret(&state.dom.borrow(), destination)?
    } else {
        BoundaryPoint {
            container: destination,
            offset: caret_offset.min(boundary_length(&state.dom.borrow(), destination)?),
        }
    };
    set_selection_boundaries(state, point, point)?;
    Ok(true)
}

pub(super) async fn remove_line_before_nested_block(
    context: &Context,
    container: NodeId,
) -> Result<bool, V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    let block = nearest_editing_block(&state.dom.borrow(), container, host)?;
    if block == host {
        return Ok(false);
    }
    let (parent, preceding) = {
        let dom = state.dom.borrow();
        let Some(parent) = dom.parent(block)? else {
            return Ok(false);
        };
        let children = light_children(&dom, parent)?;
        let Some(index) = children.iter().position(|candidate| *candidate == block) else {
            return Ok(false);
        };
        let preceding = index
            .checked_sub(1)
            .and_then(|index| children.get(index))
            .copied();
        (parent, preceding)
    };
    let Some(preceding) = preceding else {
        return Ok(false);
    };
    let trailing_newline = {
        let dom = state.dom.borrow();
        if editing_context_preserves_newlines(&dom, preceding, host)? {
            rightmost_text_with_trailing_newline(&dom, preceding)?
        } else {
            None
        }
    };
    if let Some((trailing_text, length)) = trailing_newline {
        context
            .text(trailing_text, length.saturating_sub(1), 1, "")
            .await?;
        if matches!(
            state.dom.borrow().arena().get(preceding)?.kind,
            NodeKind::Element { .. }
        ) && node_is_placeholder_empty(&state.dom.borrow(), preceding)?
        {
            context.remove(preceding).await?;
        }
        return Ok(true);
    }
    if element_local_name(&state.dom.borrow(), preceding) == Some("br")
        || (matches!(
            state.dom.borrow().arena().get(preceding)?.kind,
            NodeKind::Element { .. }
        ) && is_mergeable_editing_sibling(&state.dom.borrow(), preceding)
            && node_is_placeholder_empty(&state.dom.borrow(), preceding)?)
    {
        context.remove(preceding).await?;
        return Ok(true);
    }
    let preceding_can_own_editable_line = !matches!(
        element_local_name(&state.dom.borrow(), preceding),
        Some(
            "html"
                | "head"
                | "body"
                | "meta"
                | "link"
                | "base"
                | "template"
                | "title"
                | "style"
                | "script"
        )
    );
    let trailing_break = preceding_can_own_editable_line
        .then(|| traversal_nodes(&state.dom.borrow(), preceding))
        .transpose()?
        .into_iter()
        .flatten()
        .rev()
        .find(|candidate| element_local_name(&state.dom.borrow(), *candidate) == Some("br"));
    let trailing_break_owner = trailing_break
        .map(|trailing_break| {
            state
                .dom
                .borrow()
                .parent(trailing_break)
                .map(|owner| owner.map(|owner| (trailing_break, owner)))
        })
        .transpose()?
        .flatten();
    if let Some((trailing_break, owner)) = trailing_break_owner {
        context.remove(trailing_break).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn ensure_split_paragraph_placeholder(
    context: &Context,
    block: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let state = &context.state;
    let mut dom = state.dom.borrow_mut();
    let nodes = traversal_nodes(&dom, block)?;
    for node in &nodes {
        match &dom.arena().get(*node)?.kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } if !data.is_empty() => {
                return Ok(None);
            }
            NodeKind::Element { name }
                if matches!(
                    name.local.as_ref(),
                    "br" | "img"
                        | "input"
                        | "hr"
                        | "audio"
                        | "video"
                        | "canvas"
                        | "iframe"
                        | "embed"
                        | "object"
                        | "select"
                        | "textarea"
                ) =>
            {
                return Ok(None);
            }
            _ => {}
        }
    }
    let parent = nodes
        .iter()
        .rev()
        .copied()
        .find(|node| {
            dom.arena()
                .get(*node)
                .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
        })
        .unwrap_or(block);
    let br = dom.create_element("br");
    drop(dom);
    context.append(parent, br, None).await?;
    Ok(Some(br))
}

pub(super) async fn remove_selection_format(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let selected = state
        .selection_range
        .get()
        .and_then(|id| state.ranges.borrow().get(&id).cloned());
    let Some(range) = selected else {
        return Ok(());
    };
    let removable = traversal_nodes(&state.dom.borrow(), host)?
        .into_iter()
        .filter(|node| {
            let dom = state.dom.borrow();
            let Ok(before) = relative_boundary(&dom, *node, false) else {
                return false;
            };
            let Ok(after) = relative_boundary(&dom, *node, true) else {
                return false;
            };
            let intersects =
                compare_boundary_points(&dom, (range.end_container, range.end_offset), before)
                    .is_ok_and(|ordering| ordering > 0)
                    && compare_boundary_points(
                        &dom,
                        (range.start_container, range.start_offset),
                        after,
                    )
                    .is_ok_and(|ordering| ordering < 0);
            intersects
                && matches!(
                    element_local_name(&dom, *node),
                    Some(
                        "a" | "b"
                            | "font"
                            | "i"
                            | "s"
                            | "span"
                            | "strike"
                            | "strong"
                            | "sub"
                            | "sup"
                            | "u"
                    )
                )
        })
        .collect::<Vec<_>>();
    for node in removable.into_iter().rev() {
        if state.dom.borrow().arena().get(node).is_ok() {
            let (parent, children) = {
                let dom = state.dom.borrow();
                (
                    dom.parent(node)?.ok_or_else(hierarchy_request_error)?,
                    light_children(&dom, node)?,
                )
            };
            let _parent = context.lease(parent)?;
            let _node = context.lease(node)?;
            for child in &children {
                context.lease(*child)?;
            }
            for child in children {
                context.append(parent, child, Some(node)).await?;
            }
            context.remove(node).await?;
        }
    }
    synchronize_live_ranges(state)
}

pub(super) async fn clear_contained_alignment(context: &Context) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(range) = editing_alignment_range(state)? else {
        return Ok(());
    };
    let nodes = contained_alignment_nodes(state, &range)?;
    for node in nodes {
        let needs_cleanup = {
            let dom = state.dom.borrow();
            matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. })
                && (dom.attribute_in_null_namespace(node, "align")?.is_some()
                    || element_local_name(&dom, node) == Some("center")
                    || dom
                        .attribute_in_null_namespace(node, "style")?
                        .is_some_and(|style| {
                            style.split(';').any(|declaration| {
                                declaration.split_once(':').is_some_and(|(name, _)| {
                                    name.trim().eq_ignore_ascii_case("text-align")
                                })
                            })
                        }))
        };
        if !needs_cleanup {
            continue;
        }
        {
            let mut dom = state.dom.borrow_mut();
            drop(dom);
            context.remove_attribute(node, "align").await?;
            dom = state.dom.borrow_mut();
            drop(dom);
            context.style(node, "text-align", "").await?;
            let mut dom = state.dom.borrow_mut();
            if dom.attribute_in_null_namespace(node, "style")? == Some("") {
                drop(dom);
                context.remove_attribute(node, "style").await?;
                dom = state.dom.borrow_mut();
            }
        }
        let (name, attributes, parent) = {
            let dom = state.dom.borrow();
            (
                element_local_name(&dom, node).unwrap_or("").to_owned(),
                dom.arena().get(node)?.attributes.clone(),
                dom.parent(node)?,
            )
        };
        let Some(parent) = parent else {
            continue;
        };
        if matches!(name.as_str(), "div" | "span" | "center") && attributes.is_empty() {
            let children = light_children(&state.dom.borrow(), node)?;
            for child in children {
                let (_, index) = relative_boundary(&state.dom.borrow(), node, false)?;
                context.move_preserving_ranges(child, parent, index).await?;
            }
            context.remove(node).await?;
            synchronize_live_ranges(state)?;
        } else if name == "center" {
            let replacement = state.dom.borrow_mut().create_element("div");
            {
                let mut dom = state.dom.borrow_mut();
                for attribute in attributes.records() {
                    drop(dom);
                    context
                        .attribute(
                            replacement,
                            attribute.namespace.clone(),
                            attribute.name.clone(),
                            attribute.value.clone(),
                        )
                        .await?;
                    dom = state.dom.borrow_mut();
                }
            }
            context.append(parent, replacement, Some(node)).await?;
            let children = light_children(&state.dom.borrow(), node)?;
            for child in children {
                let index = boundary_length(&state.dom.borrow(), replacement)?;
                context
                    .move_preserving_ranges(child, replacement, index)
                    .await?;
            }
            context.remove(node).await?;
            synchronize_live_ranges(state)?;
        }
    }
    Ok(())
}

pub(super) async fn separate_alignment_inline_runs(
    context: &Context,
    wrapper: NodeId,
    incoming: &[NodeId],
    prepend: bool,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    refresh_layout(state)?;
    let needs_break = {
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let children = light_children(&dom, wrapper)?;
        let visible = |nodes: &[NodeId]| -> Result<Vec<NodeId>, V8BindingError> {
            let mut result = Vec::new();
            for node in nodes {
                if alignment_node_is_visible(&dom, &layout, *node)? {
                    result.push(*node);
                }
            }
            Ok(result)
        };
        let existing = visible(&children)?;
        let incoming_visible = visible(incoming)?;
        let (left, right, last) = if prepend {
            (incoming_visible.last(), existing.first(), incoming.last())
        } else {
            (existing.last(), incoming_visible.first(), children.last())
        };
        alignment_node_is_block(&dom, &layout, wrapper)
            && left.zip(right).is_some_and(|(left, right)| {
                !alignment_node_is_block(&dom, &layout, *left)
                    && !alignment_node_is_block(&dom, &layout, *right)
            })
            && !last.is_some_and(|node| element_local_name(&dom, *node) == Some("br"))
    };
    if needs_break {
        let reference = if prepend {
            light_children(&state.dom.borrow(), wrapper)?
                .first()
                .copied()
        } else {
            None
        };
        let mut dom = state.dom.borrow_mut();
        let br = dom.create_element("br");
        drop(dom);
        context.append(wrapper, br, reference).await?;
    }
    Ok(())
}

pub(super) async fn wrap_alignment_contents(
    context: &Context,
    alignment: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    let Some(range) = editing_alignment_range(state)? else {
        return Ok(());
    };
    let mut selected = Vec::new();
    for node in contained_alignment_nodes(state, &range)? {
        let covered = {
            let dom = state.dom.borrow();
            let mut parent = dom.parent(node)?;
            let mut covered = false;
            while let Some(ancestor) = parent {
                if selected.contains(&ancestor) {
                    covered = true;
                    break;
                }
                parent = dom.parent(ancestor)?;
            }
            covered || !alignment_allows_div_child(&dom, node)?
        };
        if !covered && editing_alignment_value(state, node)? != alignment {
            selected.push(node);
        }
    }
    let mut index = 0;
    while index < selected.len() {
        let first = selected[index];
        let (parent, position) = relative_boundary(&state.dom.borrow(), first, false)?;
        let mut end = index + 1;
        {
            let dom = state.dom.borrow();
            let siblings = light_children(&dom, parent)?;
            while end < selected.len()
                && siblings.get(position as usize + end - index) == Some(&selected[end])
            {
                end += 1;
            }
        }
        let mut has_visible_content = false;
        {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            for node in &selected[index..end] {
                if alignment_node_is_visible(&dom, &layout, *node)?
                    || element_local_name(&dom, *node) == Some("br")
                {
                    has_visible_content = true;
                    break;
                }
            }
        }
        if !has_visible_content {
            index = end;
            continue;
        }
        let (run, position) = {
            let dom = state.dom.borrow();
            let layout = state.layout.borrow();
            let siblings = light_children(&dom, parent)?;
            let mut start = position as usize;
            let mut limit = start + end - index;
            let last = siblings[limit - 1];
            if !alignment_node_is_block(&dom, &layout, last)
                && element_local_name(&dom, last) != Some("br")
                && siblings
                    .get(limit)
                    .is_some_and(|node| element_local_name(&dom, *node) == Some("br"))
            {
                limit += 1;
            }
            while start > 0 && !alignment_node_is_visible(&dom, &layout, siblings[start - 1])? {
                start -= 1;
            }
            while limit < siblings.len()
                && !alignment_node_is_visible(&dom, &layout, siblings[limit])?
            {
                limit += 1;
            }
            (siblings[start..limit].to_vec(), start as u32)
        };
        while end < selected.len() && run.contains(&selected[end]) {
            end += 1;
        }
        let first = run[0];
        let (previous, next) = {
            let dom = state.dom.borrow();
            let siblings = light_children(&dom, parent)?;
            (
                position
                    .checked_sub(1)
                    .and_then(|i| siblings.get(i as usize))
                    .copied()
                    .filter(|node| reusable_alignment_wrapper(&dom, *node, alignment)),
                siblings
                    .get(position as usize + run.len())
                    .copied()
                    .filter(|node| reusable_alignment_wrapper(&dom, *node, alignment)),
            )
        };
        let wrapper = if let Some(previous) = previous {
            previous
        } else if let Some(next) = next {
            next
        } else {
            let wrapper = state.dom.borrow_mut().create_element("div");
            context.style(wrapper, "text-align", alignment).await?;
            context
                .insert_alignment_wrapper(parent, wrapper, first, position)
                .await?;
            wrapper
        };
        let prepend = previous.is_none() && next.is_some();
        separate_alignment_inline_runs(context, wrapper, &run, prepend).await?;
        if prepend {
            for node in run.iter().rev() {
                context.move_preserving_ranges(*node, wrapper, 0).await?;
            }
        } else {
            for node in &run {
                let offset = boundary_length(&state.dom.borrow(), wrapper)?;
                context
                    .move_preserving_ranges(*node, wrapper, offset)
                    .await?;
            }
        }
        if let Some(next) = next.filter(|next| *next != wrapper) {
            let children = light_children(&state.dom.borrow(), next)?;
            separate_alignment_inline_runs(context, wrapper, &children, false).await?;
            for child in children {
                let offset = boundary_length(&state.dom.borrow(), wrapper)?;
                context
                    .move_preserving_ranges(child, wrapper, offset)
                    .await?;
            }
            context.remove(next).await?;
            synchronize_live_ranges(state)?;
        }
        index = end;
    }
    synchronize_live_ranges(state)
}

pub(super) async fn remove_editing_placeholder_before_typing(
    context: &Context,
    text: &str,
) -> Result<(), V8BindingError> {
    let state: &V8BindingState = &context.state;
    if text.is_empty() {
        return Ok(());
    }
    let Some(host) = editing_host(state)? else {
        return Ok(());
    };
    let Some(range) = state
        .selection_range
        .get()
        .and_then(|id| state.ranges.borrow().get(&id).cloned())
    else {
        return Ok(());
    };
    let whitespace = editing_white_space(state, range.start_container)?;
    if !editing_text_has_visible_content(text, &whitespace) {
        // A typed space on an empty line becomes a visible NBSP. It replaces
        // the placeholder, while a break following existing content remains.
        let point = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
        let run = editing_space_run(state, point, host)?;
        if !text.contains(' ') || !run.starts_line || !run.ends_line {
            return Ok(());
        }
    }
    let block = nearest_rendered_editing_block(state, range.start_container, host)?;
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    {
        let dom = state.dom.borrow();
        if matches!(
            dom.arena().get(point.container)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            if point.offset != boundary_length(&dom, point.container)? {
                return Ok(());
            }
            let (container, offset) = relative_boundary(&dom, point.container, true)?;
            point = BoundaryPoint { container, offset };
        }
    }
    loop {
        let candidate = light_children(&state.dom.borrow(), point.container)?
            .get(point.offset as usize)
            .copied();
        if let Some(candidate) = candidate {
            if element_local_name(&state.dom.borrow(), candidate) != Some("br") {
                return Ok(());
            }
            let after = BoundaryPoint {
                container: point.container,
                offset: point.offset + 1,
            };
            if !editing_line_has_following_content(state, after, host, &whitespace)? {
                context.remove(candidate).await?;
            }
            return Ok(());
        }
        if point.container == block {
            return Ok(());
        }
        let (container, offset) = relative_boundary(&state.dom.borrow(), point.container, true)?;
        point = BoundaryPoint { container, offset };
    }
}
