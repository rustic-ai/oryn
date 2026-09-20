//! Native sequential focus candidate ordering. Browsing-context transitions
//! still require the page host's cross-realm focus controller.
use super::*;

pub(super) fn radio_arrow(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    key: &str,
) -> Result<(), V8BindingError> {
    {
        let dom = state.dom.borrow();
        if dom.focused() != Some(target)
            || !dom.is_radio(target)
            || !dom.is_connected(target)?
            || dom.control_disabled(target)?
            || dom.is_inert(target)?
        {
            return Ok(());
        }
    }
    refresh_layout(state)?;
    if css_suppresses_focus_snapshot(state, target)? {
        return Ok(());
    }
    let (nodes, rtl) = {
        let dom = state.dom.borrow();
        let root = dom.control_tree_root(target)?;
        (
            traversal_nodes(&dom, root)?,
            editing_snapshot_property(&dom, &state.layout.borrow(), target, "direction")?
                .as_deref()
                == Some("rtl"),
        )
    };
    let mut group = Vec::new();
    for node in nodes {
        let available = {
            let dom = state.dom.borrow();
            (node == target || dom.same_radio_group(target, node)?)
                && dom.is_connected(node)?
                && !dom.control_disabled(node)?
                && !dom.is_inert(node)?
        };
        if available && !css_suppresses_focus_snapshot(state, node)? {
            group.push(node);
        }
    }
    let Some(index) = group.iter().position(|node| *node == target) else {
        return Ok(());
    };
    if group.len() < 2 {
        return Ok(());
    }
    let forward = key == "ArrowDown" || key == if rtl { "ArrowLeft" } else { "ArrowRight" };
    let next = group[(index + if forward { 1 } else { group.len() - 1 }) % group.len()];
    focus_html_element(scope, state, next, false)?;
    // Focus listeners may remove, disable, retype or redirect the destination.
    {
        let dom = state.dom.borrow();
        if dom.focused() != Some(next)
            || !dom.is_connected(next)?
            || !dom.is_radio(next)
            || dom.control_disabled(next)?
            || dom.is_inert(next)?
        {
            return Ok(());
        }
    }
    let click = create_synthetic_click_event(scope, state)?;
    let id = event_id(scope, click)?;
    state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
    dispatch_event(scope, state, next, click)?;
    Ok(())
}

fn eligible(state: &V8BindingState, node: NodeId) -> Result<bool, V8BindingError> {
    eligible_for(state, node, true)
}

pub(super) fn eligible_for(
    state: &V8BindingState,
    node: NodeId,
    sequential: bool,
) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    if !dom.is_connected(node)? || dom.is_inert(node)? || dom.control_disabled(node)? {
        return Ok(false);
    }
    let NodeKind::Element { name } = &dom.arena().get(node)?.kind else {
        return Ok(false);
    };
    let html = name.ns.as_ref() == "http://www.w3.org/1999/xhtml";
    let svg = name.ns.as_ref() == "http://www.w3.org/2000/svg";
    if !html && !svg {
        return Ok(false);
    }
    if html
        && name.local.as_ref() == "input"
        && dom
            .attribute_in_null_namespace(node, "type")?
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    {
        return Ok(false);
    }
    let natural = dom.arena().get(node)?.submit_behavior.is_some()
        || match element_local_name(&dom, node) {
            Some("input") => !dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|v| v.eq_ignore_ascii_case("hidden")),
            Some("button" | "select" | "textarea" | "iframe" | "object") => true,
            Some("dialog") => {
                !sequential && dom.attribute_in_null_namespace(node, "open")?.is_some()
            }
            Some("a" | "area") => dom.attribute_in_null_namespace(node, "href")?.is_some(),
            Some("summary") => dom.parent(node)?.is_some_and(|parent| {
                dom.is_element(parent, "http://www.w3.org/1999/xhtml", "details")
                    && dom.children(parent).is_ok_and(|children| {
                        children.iter().find(|child| {
                            dom.is_element(**child, "http://www.w3.org/1999/xhtml", "summary")
                        }) == Some(&node)
                    })
            }),
            _ => {
                editing_host_for_node(
                    &dom,
                    Some(node),
                    state.design_mode_enabled(),
                    state.document_root(),
                )? == Some(node)
            }
        };
    let natural = if svg {
        name.local.as_ref() == "a" && dom.attribute_in_null_namespace(node, "href")?.is_some()
    } else {
        natural
    };
    let tab = dom.parsed_tab_index(node)?;
    let allowed = tab.is_some_and(|value| !sequential || value >= 0) || (tab.is_none() && natural);
    drop(dom);
    Ok(allowed && !css_suppresses_focus_snapshot(state, node)?)
}

fn scope_children(dom: &DomRuntime, root: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
    if dom.is_element(root, "http://www.w3.org/1999/xhtml", "slot") {
        let assigned = dom.assigned_nodes(root, false)?;
        if !assigned.is_empty() {
            return Ok(assigned);
        }
    }
    Ok(dom.children(root)?.to_vec())
}

fn flat_nodes(
    dom: &DomRuntime,
    root: NodeId,
    output: &mut Vec<NodeId>,
) -> Result<(), V8BindingError> {
    output.push(root);
    let shadow = dom.children(root)?.iter().copied().find(|child| {
        dom.arena()
            .get(*child)
            .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
    });
    let children = if let Some(shadow) = shadow {
        dom.children(shadow)?.to_vec()
    } else {
        scope_children(dom, root)?
    };
    for child in children {
        if !matches!(
            dom.arena().get(child)?.kind,
            NodeKind::Document | NodeKind::ShadowRoot { .. }
        ) {
            flat_nodes(dom, child, output)?;
        }
    }
    Ok(())
}

fn scope_nodes(state: &V8BindingState, root: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
    let mut entries = Vec::new();
    let mut pending = scope_children(&state.dom.borrow(), root)?;
    pending.reverse();
    while let Some(node) = pending.pop() {
        let (children, shadow, tab) = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            if !matches!(record.kind, NodeKind::Element { .. }) {
                continue;
            }
            let shadow = record
                .children
                .iter()
                .copied()
                .find(|child| {
                    dom.arena()
                        .get(*child)
                        .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
                })
                .or_else(|| {
                    dom.is_element(node, "http://www.w3.org/1999/xhtml", "slot")
                        .then_some(node)
                });
            (record.children.clone(), shadow, dom.parsed_tab_index(node)?)
        };
        let own = eligible(state, node)?;
        if own || shadow.is_some() {
            // A shadow host orders its nested scope as one entry in its
            // containing scope, rather than globally sorting shadow children.
            if !tab.is_some_and(|value| value < 0) {
                entries.push((
                    (tab.is_none_or(|v| v <= 0), tab.unwrap_or(0)),
                    node,
                    own,
                    shadow,
                ));
            }
        }
        if shadow.is_none() {
            pending.extend(children.into_iter().rev());
        }
    }
    entries.sort_by_key(|entry| entry.0);
    let mut result = Vec::new();
    for (_, node, own, shadow) in entries {
        if own {
            result.push(node);
        }
        if let Some(shadow) = shadow {
            result.extend(scope_nodes(state, shadow)?);
        }
    }
    Ok(result)
}

pub(super) fn next_sequential(
    state: &Rc<V8BindingState>,
    reverse: bool,
) -> Result<Option<NodeId>, V8BindingError> {
    refresh_layout(state)?;
    let mut candidates = scope_nodes(state, state.document_root())?;
    let current = state.dom.borrow().focused();
    // Radio groups are one sequential stop. An eligible checked member wins;
    // otherwise the first eligible member wins in both directions.
    let all = candidates.clone();
    let mut groups = Vec::new();
    candidates.retain(|node| {
        let dom = state.dom.borrow();
        if !dom.is_radio(*node) {
            return true;
        }
        if all.iter().any(|peer| {
            *peer != *node
                && dom.same_radio_group(*node, *peer).unwrap_or(false)
                && dom.checked(*peer).unwrap_or(false)
        }) {
            return false;
        }
        if groups
            .iter()
            .any(|peer| dom.same_radio_group(*node, *peer).unwrap_or(false))
        {
            return false;
        }
        groups.push(*node);
        true
    });
    if reverse {
        candidates.reverse();
    }
    let mut next = current
        .and_then(|node| candidates.iter().position(|candidate| *candidate == node))
        .map_or_else(
            || candidates.first().copied(),
            |index| candidates.get(index + 1).copied(),
        );
    if let Some(current) = current
        && !candidates.contains(&current)
        && {
            let dom = state.dom.borrow();
            dom.parsed_tab_index(current)?
                .is_some_and(|value| value < 0)
                || !matches!(element_local_name(&dom, current), Some("body" | "html"))
        }
    {
        let mut order = Vec::new();
        flat_nodes(&state.dom.borrow(), state.document_root(), &mut order)?;
        if reverse {
            order.reverse();
        }
        let eligible = candidates.iter().copied().collect::<BTreeSet<_>>();
        if let Some(position) = order.iter().position(|node| *node == current) {
            next = order[position + 1..]
                .iter()
                .copied()
                .find(|node| eligible.contains(node));
        }
    }
    Ok(next)
}
pub(super) fn advance(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    reverse: bool,
) -> Result<(), V8BindingError> {
    let next = next_sequential(state, reverse)?;
    let current = state.dom.borrow().focused();
    if let Some(next) = next {
        focus_html_element(scope, state, next, false)?;
    } else if let Some(previous) = current {
        state
            .focus_transition_revision
            .set(state.focus_transition_revision.get().wrapping_add(1));
        state.dom.borrow_mut().focus(None)?;
        for kind in ["blur", "focusout"] {
            let event = create_focus_event_object(scope, state, kind, None)?;
            dispatch_event(scope, state, previous, event)?;
        }
    }
    Ok(())
}

/// Script focus on a delegating host chooses a focusable area in its flat tree.
/// Slot assignment comes from the same authority used by event dispatch.
pub(super) fn delegated_target(
    state: &V8BindingState,
    host: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let root = {
        let dom = state.dom.borrow();
        if !dom.is_connected(host)? || dom.is_inert(host)? {
            return Ok(None);
        }
        let Some(root) = dom.attached_shadow_root(host, false)? else {
            return Ok(Some(host));
        };
        if !dom.arena().get(root)?.delegates_focus {
            return Ok(Some(host));
        }
        root
    };
    refresh_layout(state)?;
    let mut nodes = Vec::new();
    flat_nodes(&state.dom.borrow(), root, &mut nodes)?;
    let focused = state.dom.borrow().focused();
    if let Some(focused) = focused
        && nodes.contains(&focused)
        && eligible_for(state, focused, false)?
    {
        return Ok(Some(focused));
    }
    for autofocus in [true, false] {
        for node in &nodes {
            let preferred = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(*node)?.kind, NodeKind::Element { .. })
                    && dom
                        .attribute_in_null_namespace(*node, "autofocus")?
                        .is_some()
            };
            if preferred == autofocus && eligible_for(state, *node, false)? {
                return Ok(Some(*node));
            }
        }
    }
    Ok(None)
}

/// Blur follows an existing delegated focus target; it must not choose a new
/// candidate using autofocus or eligibility rules when the active one changed.
pub(super) fn delegated_blur_target(
    state: &V8BindingState,
    host: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    let Some(focused) = dom.focused() else {
        return Ok(None);
    };
    if focused == host {
        return Ok(Some(host));
    }
    let Some(root) = dom.attached_shadow_root(host, false)? else {
        return Ok(None);
    };
    if !dom.arena().get(root)?.delegates_focus {
        return Ok(None);
    }
    let mut nodes = Vec::new();
    flat_nodes(&dom, root, &mut nodes)?;
    Ok(nodes.contains(&focused).then_some(focused))
}
