//! Range mutation producer for owned native editing continuations.
use super::*;
use editing_continuation::Context;
pub(super) async fn contents(
    context: &Context,
    range: &RangeRecord,
    mode: RangeContentsMode,
) -> Result<NodeId, V8BindingError> {
    let state = &context.state;
    let (common, same_data, fragment, selection) = {
        let mut dom = state.dom.borrow_mut();
        validate_boundary_offset(&dom, range.start_container, range.start_offset)?;
        validate_boundary_offset(&dom, range.end_container, range.end_offset)?;
        let common = common_ancestor(&dom, range.start_container, range.end_container)?;
        let collapsed =
            range.start_container == range.end_container && range.start_offset == range.end_offset;
        let same_data = range.start_container == range.end_container
            && matches!(
                dom.arena().get(range.start_container)?.kind,
                NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::Comment { .. }
                    | NodeKind::ProcessingInstruction { .. }
            );
        let mut selection = BTreeMap::new();
        if !collapsed && !same_data {
            let mut pending = light_children(&dom, common)?;
            let mut seen = BTreeSet::new();
            while let Some(node) = pending.pop() {
                if !seen.insert(node) {
                    return Err(DomError::Cycle.into());
                }
                let before = relative_boundary(&dom, node, false)?;
                let after = relative_boundary(&dom, node, true)?;
                let start = (range.start_container, range.start_offset);
                let end = (range.end_container, range.end_offset);
                if compare_boundary_points(&dom, end, before)? <= 0
                    || compare_boundary_points(&dom, start, after)? >= 0
                {
                    continue;
                }
                let fully = compare_boundary_points(&dom, start, before)? <= 0
                    && compare_boundary_points(&dom, after, end)? <= 0;
                if fully
                    && mode != RangeContentsMode::Delete
                    && matches!(dom.arena().get(node)?.kind, NodeKind::DocumentType { .. })
                {
                    return Err(hierarchy_request_error());
                }
                selection.insert(node, fully);
                if !fully {
                    pending.extend(light_children(&dom, node)?);
                }
            }
        }
        let document = dom
            .arena()
            .node_document(range.start_container)?
            .ok_or(V8BindingError::Allocation("Range owner document"))?;
        let fragment = dom.create_fragment();
        dom.set_node_document(fragment, document)?;
        if collapsed {
            return Ok(fragment);
        }
        (common, same_data, fragment, selection)
    };
    let _fragment = context.lease(fragment)?;
    // Selected nodes remain alive while arbitrary author callbacks run. A moved
    // owner fails the native command's authority check rather than reusing an ID.
    let _selected = selection
        .keys()
        .copied()
        .chain([range.start_container, range.end_container])
        .map(|node| context.lease(node))
        .collect::<Result<Vec<_>, _>>()?;
    if same_data {
        let kind = state
            .dom
            .borrow()
            .arena()
            .get(range.start_container)?
            .kind
            .clone();
        let data = match &kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::Comment { data }
            | NodeKind::ProcessingInstruction { data, .. } => data,
            _ => unreachable!(),
        };
        if mode != RangeContentsMode::Delete {
            let selected = create_character_data_like(
                &mut state.dom.borrow_mut(),
                &kind,
                utf16_slice(data, range.start_offset, range.end_offset),
            )?;
            context.append(fragment, selected, None).await?;
        }
        if mode != RangeContentsMode::Clone {
            context
                .text(
                    range.start_container,
                    range.start_offset,
                    range.end_offset - range.start_offset,
                    "",
                )
                .await?;
        }
        return Ok(fragment);
    }
    let mut pending = light_children(&state.dom.borrow(), common)?
        .into_iter()
        .rev()
        .map(|node| (node, fragment))
        .collect::<Vec<_>>();
    while let Some((node, output)) = pending.pop() {
        let Some(&fully) = selection.get(&node) else {
            continue;
        };
        if fully {
            match mode {
                RangeContentsMode::Extract => context.append(output, node, None).await?,
                RangeContentsMode::Delete => context.remove(node).await?,
                RangeContentsMode::Clone => {
                    let clone = state.dom.borrow_mut().clone_node(node, true)?;
                    context.retain_clone(node, clone, true)?;
                    context.append(output, clone, None).await?;
                }
            }
            continue;
        }
        let kind = state.dom.borrow().arena().get(node)?.kind.clone();
        if let NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::Comment { data }
        | NodeKind::ProcessingInstruction { data, .. } = &kind
        {
            let start = if node == range.start_container {
                range.start_offset
            } else {
                0
            };
            let end = if node == range.end_container {
                range.end_offset
            } else {
                oryn_dom::dom_string::len(data) as u32
            };
            if mode != RangeContentsMode::Delete {
                let selected = create_character_data_like(
                    &mut state.dom.borrow_mut(),
                    &kind,
                    utf16_slice(data, start, end),
                )?;
                context.append(output, selected, None).await?;
            }
            if mode != RangeContentsMode::Clone {
                context
                    .text(node, start, end.saturating_sub(start), "")
                    .await?;
            }
        } else {
            let shell = if mode == RangeContentsMode::Delete {
                output
            } else {
                let shell = state.dom.borrow_mut().clone_node(node, false)?;
                context.retain_clone(node, shell, false)?;
                context.append(output, shell, None).await?;
                shell
            };
            pending.extend(
                light_children(&state.dom.borrow(), node)?
                    .into_iter()
                    .rev()
                    .map(|child| (child, shell)),
            );
        }
    }
    Ok(fragment)
}
