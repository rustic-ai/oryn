//! Scroll authority for the private picker box. Native option DOM identities
//! remain event targets; the originating select button has independent scrolling.
use super::*;
pub(super) fn snapshot(
    state: &V8BindingState,
    owner: NodeId,
) -> Option<oryn_common::v4::PaintNodeSnapshot> {
    if !state
        .dom
        .borrow()
        .top_layer()
        .any(|entry| entry.node == owner && entry.kind == oryn_dom::TopLayerKind::SelectPicker)
    {
        return None;
    }
    state
        .layout
        .borrow()
        .generated_nodes
        .iter()
        .find(|node| node.tag == "::picker(select)" && node.parent_id == Some(node_key(owner)))
        .cloned()
}
fn offset(scroll: &oryn_common::v4::PaintScrollGeometry, desired: (f64, f64)) -> (f64, f64) {
    (
        (desired.0).clamp(
            (scroll.scrollable.x - scroll.scrollport.x).min(0.),
            (scroll.scrollable.right() - scroll.scrollport.right()).max(0.),
        ),
        (desired.1).clamp(
            (scroll.scrollable.y - scroll.scrollport.y).min(0.),
            (scroll.scrollable.bottom() - scroll.scrollport.bottom()).max(0.),
        ),
    )
}
fn publish(state: &V8BindingState, owner: NodeId, next: (f64, f64)) -> Result<(), V8BindingError> {
    if state
        .dom
        .borrow_mut()
        .set_select_picker_scroll(owner, next)?
    {
        state.layout_cache_key.borrow_mut().take();
    }
    Ok(())
}
/// Called at the originating select in the ordinary wheel ancestor chain, after
/// inner authored scrollers have had their opportunity to consume the delta.
pub(super) fn wheel(
    state: &V8BindingState,
    owner: NodeId,
    target: NodeId,
    delta: (f64, f64),
) -> Result<Option<(f64, f64)>, V8BindingError> {
    if owner_for(state, target)? != Some(owner) {
        return Ok(None);
    }
    scroll_wheel(state, owner, delta)
}
fn scroll_wheel(
    state: &V8BindingState,
    owner: NodeId,
    delta: (f64, f64),
) -> Result<Option<(f64, f64)>, V8BindingError> {
    let Some(node) = snapshot(state, owner) else {
        return Ok(None);
    };
    let Some(scroll) = node
        .projection
        .as_ref()
        .and_then(|projection| projection.scroll.as_ref())
    else {
        return Ok(None);
    };
    let eligible = |axis| {
        node.style
            .get(axis)
            .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll"))
    };
    let next = offset(
        scroll,
        (
            scroll.offset_x + if eligible("overflow-x") { delta.0 } else { 0. },
            scroll.offset_y + if eligible("overflow-y") { delta.1 } else { 0. },
        ),
    );
    let mut remaining = (
        delta.0 - (next.0 - scroll.offset_x),
        delta.1 - (next.1 - scroll.offset_y),
    );
    if eligible("overflow-x")
        && node
            .style
            .get("overscroll-behavior-x")
            .is_some_and(|value| matches!(value.as_str(), "contain" | "none"))
    {
        remaining.0 = 0.;
    }
    if eligible("overflow-y")
        && node
            .style
            .get("overscroll-behavior-y")
            .is_some_and(|value| matches!(value.as_str(), "contain" | "none"))
    {
        remaining.1 = 0.;
    }
    publish(state, owner, next)?;
    Ok(Some(remaining))
}
pub(super) fn owner_for(
    state: &V8BindingState,
    target: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let owner = {
        let dom = state.dom.borrow();
        let layout = state.layout.borrow();
        let mut current = node_key(target);
        let mut seen = BTreeSet::new();
        let mut found = None;
        while seen.insert(current) {
            if let Some(picker) = layout
                .generated_nodes
                .iter()
                .find(|node| node.node_id == current && node.tag == "::picker(select)")
            {
                found = picker.parent_id.map(|key| NodeId {
                    slot: key as u32,
                    generation: (key >> 32) as u32,
                });
                break;
            }
            let next = if let Some(parent) = layout.paint_parent_overrides.get(&current) {
                *parent
            } else {
                let node = NodeId {
                    slot: current as u32,
                    generation: (current >> 32) as u32,
                };
                dom.flat_tree_parent(node)?.map(node_key)
            };
            let Some(next) = next else { break };
            current = next;
        }
        found
    };
    Ok(owner)
}
/// Resolve only descendants whose native paint ancestry enters the private
/// picker. A button in the same select never receives the picker's scroll route.
#[cfg(test)]
pub(super) fn reveal(
    state: &V8BindingState,
    target: NodeId,
    block: &str,
    inline: &str,
) -> Result<bool, V8BindingError> {
    refresh_layout(state)?;
    reveal_from_snapshot(state, target, block, inline)
}

pub(super) fn reveal_from_snapshot(
    state: &V8BindingState,
    target: NodeId,
    block: &str,
    inline: &str,
) -> Result<bool, V8BindingError> {
    let owner = owner_for(state, target)?;
    let Some(owner) = owner else { return Ok(false) };
    let Some(node) = snapshot(state, owner) else {
        return Ok(false);
    };
    let Some(projection) = node.projection.as_ref() else {
        return Ok(true);
    };
    let Some(scroll) = projection.scroll.as_ref() else {
        return Ok(true);
    };
    let rect = match state.layout.borrow().geometry.get(&node_key(target)) {
        Some(BindingGeometry::Available(rect)) => *rect,
        _ => return Ok(true),
    };
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ] {
        let Some((x, y)) = projection.viewport_to_local(x, y) else {
            return Ok(true);
        };
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    }
    let mut port = scroll.scrollport;
    let (target_style, viewport) = {
        let layout = state.layout.borrow();
        (
            layout
                .styles
                .get(&node_key(target))
                .cloned()
                .unwrap_or_default(),
            layout.viewport.unwrap_or_default(),
        )
    };
    let used = |style: &BTreeMap<String, String>, property: &str, basis: f64| {
        style
            .get(property)
            .and_then(|value| {
                oryn_common::css_math::resolve_length(
                    value,
                    oryn_common::css_math::LengthContext {
                        percentage_basis: basis,
                        font_size: style
                            .get("font-size")
                            .and_then(|value| value.strip_suffix("px"))
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(16.),
                        root_font_size: 16.,
                        viewport_width: viewport.0,
                        viewport_height: viewport.1,
                    },
                )
            })
            .filter(|value| value.is_finite())
            .unwrap_or(0.)
    };
    bounds.0 -= used(&target_style, "scroll-margin-left", 0.);
    bounds.1 -= used(&target_style, "scroll-margin-top", 0.);
    bounds.2 += used(&target_style, "scroll-margin-right", 0.);
    bounds.3 += used(&target_style, "scroll-margin-bottom", 0.);
    let left = used(&node.style, "scroll-padding-left", port.width).max(0.);
    let right = used(&node.style, "scroll-padding-right", port.width).max(0.);
    let top = used(&node.style, "scroll-padding-top", port.height).max(0.);
    let bottom = used(&node.style, "scroll-padding-bottom", port.height).max(0.);
    port.x += left;
    port.y += top;
    port.width = (port.width - left - right).max(0.);
    port.height = (port.height - top - bottom).max(0.);
    let mode = node
        .style
        .get("writing-mode")
        .map(String::as_str)
        .unwrap_or("horizontal-tb");
    let rtl = node
        .style
        .get("direction")
        .is_some_and(|value| value == "rtl");
    let (x, y) = scroll_physical_alignments(block, inline, mode, rtl);
    let eligible = |axis| {
        node.style
            .get(axis)
            .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll" | "hidden"))
    };
    let next = offset(
        scroll,
        (
            scroll.offset_x
                + if eligible("overflow-x") {
                    scroll_alignment_delta(bounds.0, bounds.2, port.x, port.right(), x)
                } else {
                    0.
                },
            scroll.offset_y
                + if eligible("overflow-y") {
                    scroll_alignment_delta(bounds.1, bounds.3, port.y, port.bottom(), y)
                } else {
                    0.
                },
        ),
    );
    publish(state, owner, next)?;
    Ok(true)
}

/// A wheel on the private surface is retargeted to its select. Coordinates
/// distinguish that surface from the still-visible originating button.
pub(super) fn wheel_surface_snapshot(
    state: &V8BindingState,
    target: NodeId,
    x: f64,
    y: f64,
    delta: (f64, f64),
) -> Result<Option<(f64, f64)>, V8BindingError> {
    let Some(node) = snapshot(state, target) else {
        return Ok(None);
    };
    let Some(projection) = node.projection.as_ref() else {
        return Ok(None);
    };
    let Some((x, y)) = projection.viewport_to_local(x, y) else {
        return Ok(None);
    };
    let rect = projection.box_model.border_box;
    if x < rect.x || x >= rect.right() || y < rect.y || y >= rect.bottom() {
        return Ok(None);
    }
    scroll_wheel(state, target, delta)
}

pub(super) fn scrollbar_offset_snapshot(
    state: &V8BindingState,
    owner: NodeId,
    horizontal: bool,
    total: f64,
) -> Result<(), V8BindingError> {
    let Some(node) = snapshot(state, owner) else {
        return Ok(());
    };
    let Some(scroll) = node
        .projection
        .as_ref()
        .and_then(|projection| projection.scroll.as_ref())
    else {
        return Ok(());
    };
    let next = offset(
        scroll,
        if horizontal {
            (total, scroll.offset_y)
        } else {
            (scroll.offset_x, total)
        },
    );
    publish(state, owner, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_bounds_preserve_negative_vertical_and_rtl_origins() {
        let port = oryn_common::v4::CssRect {
            x: 100.,
            y: 30.,
            width: 80.,
            height: 60.,
        };
        let scroll = oryn_common::v4::PaintScrollGeometry {
            offset_x: 0.,
            offset_y: 0.,
            scrollport: port,
            scrollable: oryn_common::v4::CssRect {
                x: -100.,
                y: -20.,
                width: 280.,
                height: 110.,
            },
        };
        assert_eq!(offset(&scroll, (-999., -999.)), (-200., -50.));
        assert_eq!(offset(&scroll, (999., 999.)), (0., 0.));
    }
}
