//! Native hit testing and wheel input. No author geometry or event constructor
//! participates in target selection or trusted event initialization.
type ScrollbarTarget = (Option<NodeId>, Option<(ScrollbarGesture, f64)>);

use super::*;
mod boundary_dispatch;
mod boundary_receipt;
pub use boundary_receipt::NativePointerBoundary;
mod dispatch_receipt;
mod drag_admission;
mod image_maps;
pub use dispatch_receipt::NativePointerDispatch;
pub use drag_admission::NativePointerDragAdmission;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePointerDevice {
    Mouse,
    Pen,
    Touch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePointerAction {
    Down,
    Up,
    Move,
    Cancel,
}
/// Exact native input registry custody; numeric pointer IDs may be reused after reset.
pub(super) fn registry_owner(state: &V8BindingState) -> Rc<()> {
    let mut registry = state.native_pointers.borrow_mut();
    registry.synchronize(state);
    registry.boundary_owner.clone()
}
pub(super) fn registry_owner_current(state: &V8BindingState, owner: &Rc<()>) -> bool {
    Rc::ptr_eq(&state.native_pointers.borrow().boundary_owner, owner)
}
/// Input authority can belong to another document than an editing target.
/// A weak owner avoids a cycle if a future native task stores this guard.
pub(super) struct InputOwner {
    state: std::rc::Weak<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    registry: Rc<()>,
}
impl InputOwner {
    pub(super) fn new(state: &Rc<V8BindingState>) -> Self {
        Self {
            state: Rc::downgrade(state),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            registry: registry_owner(state),
        }
    }
    pub(super) fn current(&self) -> bool {
        self.state.upgrade().is_some_and(|state| {
            !state.execution_retired()
                && !state.execution_tasks_stopped()
                && self.identity
                    == (
                        state.document_root(),
                        state.document_generation(),
                        state.document_stream_epoch(),
                        state.document_authority_epoch(),
                    )
                && registry_owner_current(&state, &self.registry)
        })
    }
}
#[derive(Default)]
pub(super) struct PointerRegistry {
    identity: Option<(NodeId, u64, u64)>,
    pointers: BTreeMap<i32, PointerRecord>,
    boundary_draining: bool,
    boundary_owner: Rc<()>,
    pinch: Option<native_touch::PinchCandidate>,
}
#[derive(Default)]
struct PointerRecord {
    boundary_claim: Option<Rc<()>>,
    boundary_retry: Option<std::time::Instant>,
    boundary_path: Vec<NodeId>,
    boundary_leases: Vec<Rc<oryn_dom::TrackedNodeLease>>,
    boundary_pending: Option<boundary_dispatch::Pending>,
    range: Option<NodeId>,
    buttons: u32,
    target: Option<NodeId>,
    capture: Option<NodeId>,
    pending_capture: Option<NodeId>,
    prevent_mouse: bool,
    down_targets: BTreeMap<i32, NodeId>,
    drag_start: Option<(NodeId, f64, f64)>,
    drag_consumed: bool,
    last_input: Option<NativePointerInput>,
    click_history: Option<ClickHistory>,
    click_details: BTreeMap<i32, i32>,
    scrollbar: Option<ScrollbarGesture>,
    selection: Option<native_pointer_selection::Gesture>,
    touch_policy: Option<native_touch::Policy>,
    touch_contact: Option<native_touch::Contact>,
    touch_suppressed: bool,
    touch_zooming: bool,
}
#[derive(Clone, Copy)]
struct ScrollbarGesture {
    picker_epoch: Option<u64>,
    picker: bool,
    node: NodeId,
    viewport: bool,
    horizontal: bool,
    thumb_delta: Option<f64>,
    repeat_at: std::time::Instant,
    repeat_direction: f64,
}

fn prepare_range_input(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    x: f64,
    y: f64,
) -> Result<Option<bool>, V8BindingError> {
    {
        let dom = state.dom.borrow();
        if state.execution_retired()
            || !dom.is_connected(node)?
            || dom
                .arena()
                .node_document(node)?
                .is_none_or(|document| document != state.document_root())
            || !dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            || !dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("range"))
            || dom.control_disabled(node)?
            || dom.is_inert(node)?
        {
            return Ok(None);
        }
    }
    refresh_layout_in_scope(scope, state)?;
    let fraction = {
        let layout = state.layout.borrow();
        let Some(projection) = layout.hit_projections.get(&node_key(node)) else {
            return Ok(None);
        };
        let Some((x, y)) = projection.viewport_to_local(x, y) else {
            return Ok(None);
        };
        let rect = projection.box_model.content_box;
        let style = layout.styles.get(&node_key(node));
        let vertical = style
            .and_then(|style| style.get("writing-mode"))
            .is_some_and(|mode| {
                matches!(
                    mode.as_str(),
                    "vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr"
                )
            });
        let rtl = style
            .and_then(|style| style.get("direction"))
            .is_some_and(|direction| direction == "rtl");
        let diameter = rect.width.min(rect.height).min(16.);
        let span = (if vertical { rect.height } else { rect.width }) - diameter;
        if span <= 0. {
            return Ok(None);
        }
        let fraction = ((if vertical { y - rect.y } else { x - rect.x }) - diameter / 2.) / span;
        let reversed = if style
            .and_then(|style| style.get("writing-mode"))
            .is_some_and(|mode| mode == "sideways-lr")
        {
            !rtl
        } else {
            rtl
        };
        let fraction = if reversed { 1. - fraction } else { fraction };
        fraction.clamp(0., 1.)
    };
    let changed = {
        let mut dom = state.dom.borrow_mut();
        let number = |name: &str| {
            dom.attribute_in_null_namespace(node, name)
                .ok()
                .flatten()
                .and_then(oryn_dom::parse_html_floating_point)
        };
        let min = number("min").unwrap_or(0.);
        let max = number("max").unwrap_or(100.).max(min);
        let value = min * (1. - fraction) + max * fraction;
        let before = dom.control_value(node)?;
        dom.set_value(node, value.to_string(), false)?;
        let changed = before != dom.control_value(node)?;
        if changed {
            let record = dom.arena_mut().get_mut(node)?;
            record.user_edited_value = true;
            if record.pending_edit_value.is_none() {
                record.pending_edit_value = Some(before);
            }
            dom.set_user_validity(node, true)?;
        }
        changed
    };
    Ok(Some(changed))
}

fn scrollbar_surface(
    state: &V8BindingState,
    node: NodeId,
    viewport: bool,
    picker: bool,
    x: f64,
    y: f64,
) -> Option<(Vec<oryn_common::scrollbar::ScrollbarGeometry>, f64, f64)> {
    if picker {
        let source = select_picker_scroll::snapshot(state, node)?;
        let projection = source.projection.as_ref()?;
        let (x, y) = projection.viewport_to_local(x, y)?;
        return Some((
            oryn_common::scrollbar::geometry(projection, &source.style),
            x,
            y,
        ));
    }
    let visual = if viewport {
        Some(state.native_visual_viewport_geometry_snapshot().ok()?)
    } else {
        None
    };
    let document_scroll = if viewport {
        Some(state.document_scroll_offset())
    } else {
        None
    };
    let layout = state.layout.borrow();
    if viewport {
        let surface = layout.viewport_scroll.as_ref()?;
        let (width, height) = layout.viewport?;
        let mut scroll = surface.scroll.clone();
        if let Some(offset) = document_scroll {
            scroll.offset_x = offset.0;
            scroll.offset_y = offset.1;
        }
        let bars = oryn_common::scrollbar::geometry_for_viewport_scroll(
            &scroll,
            oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width,
                height,
            },
            &surface.style,
            visual,
        );
        // WebDriver/native event coordinates remain layout CSS coordinates;
        // only the chrome hit boundary converts them to the physical surface.
        let (x, y) = visual?.layout_to_visual(x, y)?;
        Some((bars, x, y))
    } else {
        let projection = layout.hit_projections.get(&node_key(node))?;
        let style = layout.styles.get(&node_key(node))?;
        let (x, y) = projection.viewport_to_local(x, y)?;
        Some((oryn_common::scrollbar::geometry(projection, style), x, y))
    }
}

fn picker_gesture_current(state: &V8BindingState, gesture: ScrollbarGesture) -> bool {
    state.dom.borrow().top_layer().any(|entry| {
        entry.node == gesture.node
            && entry.kind == oryn_dom::TopLayerKind::SelectPicker
            && Some(entry.picker_epoch) == gesture.picker_epoch
    })
}
fn commit_scrollbar_offset(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    gesture: ScrollbarGesture,
    total: f64,
) -> Result<(), V8BindingError> {
    let lifetime = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    );
    refresh_layout_in_scope(scope, state)?;
    if state.execution_retired()
        || lifetime
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            )
    {
        return Err(V8BindingError::Allocation(
            "scrollbar gesture document retired",
        ));
    }
    {
        let dom = state.dom.borrow();
        if !dom.is_connected(gesture.node)? || dom.node_document(gesture.node)? != Some(lifetime.0)
        {
            return Ok(());
        }
    }
    if gesture.picker {
        if !picker_gesture_current(state, gesture) {
            return Ok(());
        }
        return select_picker_scroll::scrollbar_offset_snapshot(
            state,
            gesture.node,
            gesture.horizontal,
            total,
        );
    }
    if !gesture.viewport {
        return perform_scroll_from_snapshot(
            state,
            gesture.node,
            if gesture.horizontal {
                (Some(total), None)
            } else {
                (None, Some(total))
            },
            false,
        );
    }
    let mut visual = state.native_visual_viewport_geometry_snapshot()?;
    let current = state.document_scroll_offset();
    let split = {
        let layout = state.layout.borrow();
        layout.viewport_scroll.as_ref().and_then(|surface| {
            let mut scroll = surface.scroll.clone();
            scroll.offset_x = current.0;
            scroll.offset_y = current.1;
            oryn_common::scrollbar::split_viewport_offset(
                &scroll,
                visual,
                gesture.horizontal,
                total,
            )
        })
    };
    let Some((offset, pan)) = split else {
        return Ok(());
    };
    perform_scroll_from_snapshot(
        state,
        gesture.node,
        if gesture.horizontal {
            (Some(offset), None)
        } else {
            (None, Some(offset))
        },
        false,
    )?;
    if gesture.horizontal {
        visual.offset_x = pan;
    } else {
        visual.offset_y = pan;
    }
    if !state.execution_retired() && visual.is_valid() {
        *state.native_visual_viewport.borrow_mut() = visual;
    }
    Ok(())
}

/// Classic scrollbar input is native chrome. Viewport chrome is above page
/// content and lives in viewport coordinates regardless of root transforms.
fn prepare_scrollbar_input(
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    action: NativePointerAction,
    hit: Option<NodeId>,
) -> Result<ScrollbarTarget, V8BindingError> {
    let mut pending = None;
    let active = state
        .native_pointers
        .borrow()
        .pointers
        .get(&input.id)
        .and_then(|pointer| pointer.scrollbar);
    if let Some(active) = active {
        if active.picker && !picker_gesture_current(state, active) {
            state
                .native_pointers
                .borrow_mut()
                .pointers
                .get_mut(&input.id)
                .unwrap()
                .scrollbar = None;
            return Ok((None, None));
        }
        if action == NativePointerAction::Cancel
            || (action == NativePointerAction::Up && input.button == 0)
        {
            state
                .native_pointers
                .borrow_mut()
                .pointers
                .get_mut(&input.id)
                .unwrap()
                .scrollbar = None;
        } else if action == NativePointerAction::Move
            && input.buttons & 1 != 0
            && let Some(delta) = active.thumb_delta
        {
            let next = scrollbar_surface(
                state,
                active.node,
                active.viewport,
                active.picker,
                input.x,
                input.y,
            )
            .and_then(|(bars, x, y)| {
                bars.into_iter()
                    .find(|bar| bar.horizontal == active.horizontal)
                    .and_then(|bar| {
                        bar.offset_at_thumb_origin(if active.horizontal {
                            x - delta
                        } else {
                            y - delta
                        })
                    })
            });
            if let Some(next) = next {
                pending = Some((active, next));
            }
        }
        return Ok((Some(active.node), pending));
    }
    if action != NativePointerAction::Down
        || input.button != 0
        || !input.primary
        || input.device == NativePointerDevice::Touch
    {
        return Ok((None, None));
    }
    if state
        .native_pointers
        .borrow()
        .pointers
        .get(&input.id)
        .is_some_and(|pointer| pointer.capture.is_some())
    {
        return Ok((None, None));
    }
    let root = {
        let dom = state.dom.borrow();
        document_scrolling_element(&dom, &state.layout.borrow(), state.document_root())
    };
    let mut selected = None;
    let pickers = state
        .layout
        .borrow()
        .generated_nodes
        .iter()
        .rev()
        .filter(|node| node.tag == "::picker(select)")
        .filter_map(|node| node.parent_id)
        .map(|key| {
            (
                NodeId {
                    slot: key as u32,
                    generation: (key >> 32) as u32,
                },
                false,
                true,
            )
        })
        .collect::<Vec<_>>();
    for (node, viewport, picker) in root
        .map(|node| (node, true, false))
        .into_iter()
        .chain(pickers)
        .chain(hit.map(|node| (node, false, false)))
    {
        let Some((bars, x, y)) = scrollbar_surface(state, node, viewport, picker, input.x, input.y)
        else {
            continue;
        };
        let contains = |rect: oryn_common::v4::CssRect| {
            x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
        };
        if let Some(bar) = bars.into_iter().find(|bar| contains(bar.track)) {
            let coordinate = if bar.horizontal { x } else { y };
            let leading = if bar.horizontal {
                bar.thumb.x
            } else {
                bar.thumb.y
            };
            let on_thumb = contains(bar.thumb);
            selected = Some((
                ScrollbarGesture {
                    picker_epoch: if picker {
                        state
                            .dom
                            .borrow()
                            .top_layer()
                            .find(|entry| {
                                entry.node == node
                                    && entry.kind == oryn_dom::TopLayerKind::SelectPicker
                            })
                            .map(|entry| entry.picker_epoch)
                    } else {
                        None
                    },
                    picker,
                    node,
                    viewport,
                    horizontal: bar.horizontal,
                    thumb_delta: on_thumb.then_some(coordinate - leading),
                    repeat_at: std::time::Instant::now() + std::time::Duration::from_millis(400),
                    repeat_direction: if coordinate < leading { -1. } else { 1. },
                },
                (!on_thumb).then_some(
                    (bar.offset
                        + if coordinate < leading {
                            -bar.page_step
                        } else {
                            bar.page_step
                        })
                    .clamp(bar.min_offset, bar.max_offset),
                ),
            ));
            break;
        }
    }
    let Some((gesture, next)) = selected else {
        return Ok((None, None));
    };
    let node = gesture.node;
    {
        let mut registry = state.native_pointers.borrow_mut();
        let pointer = registry.pointers.get_mut(&input.id).unwrap();
        pointer.scrollbar = Some(gesture);
        pointer.click_history = None;
        pointer.down_targets.clear();
        pointer.drag_start = None;
        pointer.selection = None;
    }
    if let Some(next) = next {
        pending = Some((gesture, next));
    }
    Ok((Some(node), pending))
}

/// Deadline-driven input defaults share the page task scheduler, not author
/// timers. Delayed checkpoints perform one repeat, avoiding a catch-up burst.
pub fn cancel_native_input_defaults(state: &V8BindingState) {
    speculation_registry::clear_interest(state);
    state.native_pointers.borrow_mut().pinch = None;
    for pointer in state.native_pointers.borrow_mut().pointers.values_mut() {
        pointer.scrollbar = None;
        pointer.selection = None;
        pointer.range = None;
        if pointer.touch_contact.is_some() {
            // A suspended contact cannot later become a tap when physical
            // release arrives after restore. Keep a suppression tombstone.
            pointer.touch_suppressed = true;
            pointer.down_targets.clear();
            pointer.click_details.clear();
            pointer.click_history = None;
            pointer.prevent_mouse = true;
            pointer.pending_capture = None;
            pointer.capture = None;
            pointer.drag_start = None;
            pointer.target = None;
        }
        pointer.touch_contact = None;
        pointer.touch_policy = None;
    }
}

pub fn native_input_delay(
    state: &V8BindingState,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    if state.execution_retired() {
        return None;
    }
    let registry = state.native_pointers.borrow();
    let scrollbar = registry
        .pointers
        .values()
        .filter(|pointer| pointer.boundary_claim.is_none())
        .filter(|pointer| pointer.buttons & 1 != 0)
        .filter_map(|pointer| pointer.scrollbar)
        .filter(|gesture| gesture.thumb_delta.is_none())
        .map(|gesture| gesture.repeat_at.saturating_duration_since(now))
        .min();
    let selection = registry
        .pointers
        .values()
        .filter(|pointer| pointer.boundary_claim.is_none())
        .filter(|pointer| pointer.buttons & 1 != 0 && !pointer.drag_consumed)
        .filter_map(|pointer| pointer.selection?.autoscroll_at)
        .map(|deadline| deadline.saturating_duration_since(now))
        .min();
    let boundary = registry
        .pointers
        .values()
        .filter(|pointer| pointer.boundary_claim.is_none() && pointer.boundary_pending.is_some())
        .map(|pointer| {
            pointer
                .boundary_retry
                .map_or(std::time::Duration::ZERO, |deadline| {
                    deadline.saturating_duration_since(now)
                })
        })
        .min();
    scrollbar.into_iter().chain(selection).chain(boundary).min()
}

pub fn poll_native_selection_defaults(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    now: std::time::Instant,
) -> Result<usize, V8BindingError> {
    if state.execution_retired() {
        return Ok(0);
    }
    let boundaries = state
        .native_pointers
        .borrow()
        .pointers
        .iter()
        .filter(|(_, pointer)| {
            pointer.boundary_claim.is_none()
                && pointer.boundary_pending.is_some()
                && pointer
                    .boundary_retry
                    .is_none_or(|deadline| deadline <= now)
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in boundaries {
        boundary_dispatch::drain(scope, state, id)?;
    }
    let pending = state
        .native_pointers
        .borrow()
        .pointers
        .iter()
        .filter_map(|(id, pointer)| {
            if pointer.boundary_claim.is_some() {
                return None;
            }
            let gesture = pointer.selection?;
            if pointer.buttons & 1 == 0
                || pointer.drag_consumed
                || !gesture
                    .autoscroll_at
                    .is_some_and(|deadline| deadline <= now)
            {
                return None;
            }
            Some((*id, gesture, pointer.last_input.clone()?))
        })
        .collect::<Vec<_>>();
    let mut changed = 0;
    for (id, gesture, input) in pending {
        let (next, scrolled) =
            native_pointer_selection::autoscroll(scope, state, gesture, input.x, input.y, now)?;
        if let Some(pointer) = state.native_pointers.borrow_mut().pointers.get_mut(&id)
            && pointer.buttons & 1 != 0
            && !pointer.drag_consumed
        {
            pointer.selection = next;
        }
        changed += usize::from(scrolled);
    }
    Ok(changed)
}
pub fn poll_native_input_defaults(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    now: std::time::Instant,
) -> Result<usize, V8BindingError> {
    if state.execution_retired() {
        return Ok(0);
    }
    let pending = state
        .native_pointers
        .borrow()
        .pointers
        .iter()
        .filter_map(|(id, pointer)| {
            if pointer.boundary_claim.is_some() {
                return None;
            }
            let gesture = pointer.scrollbar?;
            (pointer.buttons & 1 != 0 && gesture.thumb_delta.is_none() && gesture.repeat_at <= now)
                .then(|| {
                    pointer
                        .last_input
                        .clone()
                        .map(|input| (*id, gesture, input))
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let mut changed = 0;
    for (id, mut gesture, input) in pending {
        gesture.repeat_at = now + std::time::Duration::from_millis(50);
        if let Some(pointer) = state.native_pointers.borrow_mut().pointers.get_mut(&id) {
            pointer.scrollbar = Some(gesture);
        }
        if !state.dom.borrow().is_connected(gesture.node)?
            || (gesture.picker && !picker_gesture_current(state, gesture))
        {
            if let Some(pointer) = state.native_pointers.borrow_mut().pointers.get_mut(&id) {
                pointer.scrollbar = None;
            }
            continue;
        }
        refresh_layout_in_scope(scope, state)?;
        let Some((bars, x, y)) = scrollbar_surface(
            state,
            gesture.node,
            gesture.viewport,
            gesture.picker,
            input.x,
            input.y,
        ) else {
            continue;
        };
        let Some(bar) = bars
            .into_iter()
            .find(|bar| bar.horizontal == gesture.horizontal)
        else {
            continue;
        };
        if x < bar.track.x || x >= bar.track.right() || y < bar.track.y || y >= bar.track.bottom() {
            continue;
        }
        let (position, start, end) = if gesture.horizontal {
            (x, bar.thumb.x, bar.thumb.right())
        } else {
            (y, bar.thumb.y, bar.thumb.bottom())
        };
        if gesture.repeat_direction < 0. && position >= start
            || gesture.repeat_direction > 0. && position <= end
        {
            continue;
        }
        let next = (bar.offset + gesture.repeat_direction * bar.page_step)
            .clamp(bar.min_offset, bar.max_offset);
        if next != bar.offset {
            commit_scrollbar_offset(scope, state, gesture, next)?;
            changed += 1;
        }
    }
    Ok(changed)
}

// Headless native multi-click policy; author clocks and event constructors do
// not influence the sequence. History is scoped to a pointer and document.
struct ClickHistory {
    target: NodeId,
    button: i32,
    x: f64,
    y: f64,
    at: std::time::Instant,
    count: i32,
}
impl ClickHistory {
    fn next(&self, target: NodeId, input: &NativePointerInput, now: std::time::Instant) -> i32 {
        if self.target == target
            && self.button == input.button
            && now.saturating_duration_since(self.at) <= std::time::Duration::from_millis(500)
            && (self.x - input.x).hypot(self.y - input.y) <= 4.
        {
            self.count.saturating_add(1)
        } else {
            1
        }
    }
}
impl PointerRegistry {
    fn synchronize(&mut self, state: &V8BindingState) {
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
        );
        if self.identity != Some(identity) {
            if let Some((document, _, _)) = self.identity {
                state.dom.borrow_mut().clear_pointer_hover(document);
            }
            self.pinch = None;
            self.pointers.clear();
            self.boundary_owner = Rc::new(());
            self.boundary_draining = false;
            self.identity = Some(identity);
        }
    }
}

/// Release Actions removes the input sources after dispatching their pending
/// releases. Hover/capture history must not follow a reused pointer ID.
pub fn reset_native_pointer_sources(state: &V8BindingState) {
    state
        .dom
        .borrow_mut()
        .clear_pointer_hover(state.document_root());
    speculation_registry::clear_interest(state);
    // Ordinary pending releases already dispatched drop/cancel and dragend.
    // A source may nevertheless have disappeared or retired during a listener.
    // Revoke its payload and compositor feedback as well as pointer identity;
    // neither may survive reuse of a WebDriver input source ID.
    let abandoned = state.native_drag.borrow_mut().take();
    drop(abandoned);
    native_drag_image::clear(state, None);
    *state.native_pointers.borrow_mut() = PointerRegistry::default();
}

pub(super) fn cancel_drag_from_keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    control: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Result<bool, V8BindingError> {
    if state.native_drag.borrow().is_none() {
        return Ok(false);
    }
    let input = state
        .native_pointers
        .borrow()
        .pointers
        .values()
        .find(|pointer| pointer.drag_consumed)
        .and_then(|pointer| pointer.last_input.clone())
        .unwrap_or_default();
    let init = crate::data_transfer::EventInit {
        client_x: input.x as i32,
        client_y: input.y as i32,
        screen_x: input.screen_x as i32,
        screen_y: input.screen_y as i32,
        button: 0,
        buttons: 0,
        ctrl_key: control,
        alt_key: alt,
        shift_key: shift,
        meta_key: meta,
        ..Default::default()
    };
    native_drag::dispatch_input(scope, state, "cancel", None, &init)?;
    // Retain drag_consumed until pointer release: canceled drags cannot become
    // a mouseup/click sequence merely because their payload session ended.
    Ok(true)
}

/// Keyboard menu invocation is targeted at native focus, not pointer hit
/// testing. Its sentinel pointer identity must never create a capture source.
pub(super) fn context_menu_from_keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    control: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Result<(), V8BindingError> {
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let rect = client_rects_for_node(scope, state, target)?
        .first()
        .copied()
        .unwrap_or_default();
    let event = create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface: "PointerEvent",
            event_type: "contextmenu",
            bubbles: true,
            cancelable: true,
            composed: true,
            detail: 0,
            related: None,
            fields: &[
                ("pointerId", NativeInputValue::Number(-1.)),
                ("pointerType", NativeInputValue::String(&[])),
                ("isPrimary", NativeInputValue::Bool(false)),
                ("clientX", NativeInputValue::Number(rect.x)),
                ("clientY", NativeInputValue::Number(rect.y + rect.height)),
                ("screenX", NativeInputValue::Number(rect.x)),
                ("screenY", NativeInputValue::Number(rect.y + rect.height)),
                ("button", NativeInputValue::Number(0.)),
                ("buttons", NativeInputValue::Number(0.)),
                ("width", NativeInputValue::Number(1.)),
                ("height", NativeInputValue::Number(1.)),
                ("pressure", NativeInputValue::Number(0.)),
                ("ctrlKey", NativeInputValue::Bool(control)),
                ("altKey", NativeInputValue::Bool(alt)),
                ("shiftKey", NativeInputValue::Bool(shift)),
                ("metaKey", NativeInputValue::Bool(meta)),
            ],
        },
    )?;
    dispatch_event(scope, state, target, event)?;
    if state.execution_retired()
        || (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
        ) != identity
    {
        return Err(V8BindingError::Allocation(
            "context menu listener retired document",
        ));
    }
    Ok(())
}

pub(super) fn capture(
    state: &V8BindingState,
    node: NodeId,
    id: i32,
    operation: &str,
) -> Result<bool, V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("pointer document retired"));
    }
    if state.dom.borrow().node_document(node)? != Some(state.document_root()) {
        return Err(V8BindingError::Unsupported {
            interface: "Element".into(),
            member: "InvalidStateError".into(),
        });
    }
    let mut registry = state.native_pointers.borrow_mut();
    registry.synchronize(state);
    if operation == "hasPointerCapture" {
        return Ok(registry
            .pointers
            .get(&id)
            .is_some_and(|pointer| pointer.pending_capture == Some(node)));
    }
    let pointer = registry
        .pointers
        .get_mut(&id)
        .ok_or(V8BindingError::Unsupported {
            interface: "Element".into(),
            member: "NotFoundError".into(),
        })?;
    if operation == "releasePointerCapture" {
        if pointer.pending_capture == Some(node) {
            pointer.pending_capture = None;
        }
        return Ok(false);
    }
    if !state.dom.borrow().is_connected(node)? {
        return Err(V8BindingError::Unsupported {
            interface: "Element".into(),
            member: "InvalidStateError".into(),
        });
    }
    if pointer.buttons != 0 {
        pointer.pending_capture = Some(node);
    }
    Ok(false)
}

fn boundary(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    old: Option<NodeId>,
    new: Option<NodeId>,
) -> Result<(), V8BindingError> {
    boundary_dispatch::dispatch(scope, state, input, old, new)
}

pub fn native_pointer_hover_inputs(state: &V8BindingState) -> Vec<NativePointerInput> {
    if state.execution_retired() || state.native_pointers.borrow().boundary_draining {
        return Vec::new();
    }
    state
        .native_pointers
        .borrow()
        .pointers
        .values()
        .filter(|pointer| pointer.boundary_claim.is_none())
        .filter(|pointer| {
            pointer
                .boundary_retry
                .is_none_or(|deadline| deadline <= std::time::Instant::now())
        })
        .filter_map(|pointer| pointer.last_input.clone())
        .filter(|input| input.device != NativePointerDevice::Touch)
        .map(|mut input| {
            input.movement_x = 0.;
            input.movement_y = 0.;
            input.button = -1;
            input
        })
        .collect()
}
pub fn update_native_pointer_modifiers(
    state: &V8BindingState,
    modifiers: &NativeKeyboardModifiers,
) {
    for pointer in state.native_pointers.borrow_mut().pointers.values_mut() {
        if let Some(input) = pointer.last_input.as_mut() {
            input.control = modifiers.control;
            input.shift = modifiers.shift;
            input.alt = modifiers.alt;
            input.meta = modifiers.meta;
        }
    }
}

/// Native cross-document routing updates only hover boundaries in embedding
/// documents. It must not synthesize another pointermove or activation there.
pub fn transition_native_pointer_boundary(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    target: Option<NodeId>,
) -> Result<(), V8BindingError> {
    input.validate()?;
    if state.execution_retired() {
        return Err(V8BindingError::Allocation(
            "retired pointer boundary document",
        ));
    }
    if target.is_some_and(|node| {
        state.dom.borrow().node_document(node).ok().flatten() != Some(state.document_root())
    }) {
        return Err(V8BindingError::Allocation(
            "foreign pointer boundary target",
        ));
    }
    if native_pointer_owns_routing(state, input.id) {
        return Ok(());
    }
    if state
        .native_pointers
        .borrow()
        .pointers
        .get(&input.id)
        .is_some_and(|p| p.boundary_claim.is_some())
    {
        return Err(V8BindingError::Allocation("pointer boundary already owned"));
    }
    let old = {
        let mut registry = state.native_pointers.borrow_mut();
        registry.synchronize(state);
        if target.is_none() && !registry.pointers.contains_key(&input.id) {
            return Ok(());
        }
        if !registry.pointers.contains_key(&input.id)
            && registry.pointers.len() >= oryn_common::MAX_NATIVE_POINTER_SOURCES
        {
            return Err(V8BindingError::Allocation(
                "native pointer source capacity exceeded",
            ));
        }
        if input.device != NativePointerDevice::Touch {
            state
                .dom
                .borrow()
                .prepare_pointer_hover(state.document_root(), input.id, target)?;
        }
        let pointer = registry.pointers.entry(input.id).or_default();
        let old = pointer.target;
        pointer.target = target;
        pointer.buttons = input.buttons;
        pointer.last_input = Some(input.clone());
        if input.buttons == 0 {
            pointer.down_targets.clear();
            pointer.drag_start = None;
            pointer.prevent_mouse = false;
        }
        old
    };
    boundary(scope, state, input, old, target)
}

pub fn native_pointer_has_hover(state: &V8BindingState, id: i32) -> bool {
    !state.execution_retired()
        && state
            .native_pointers
            .borrow()
            .pointers
            .get(&id)
            .is_some_and(|pointer| pointer.target.is_some())
}

pub fn native_pointer_dragging(state: &V8BindingState, id: i32) -> bool {
    !state.execution_retired()
        && state.native_drag.borrow().is_some()
        && state
            .native_pointers
            .borrow()
            .pointers
            .get(&id)
            .is_some_and(|pointer| pointer.drag_consumed)
}
pub fn native_pointer_drag_candidate(state: &V8BindingState, id: i32) -> bool {
    !state.execution_retired()
        && state
            .native_pointers
            .borrow()
            .pointers
            .get(&id)
            .is_some_and(|pointer| {
                pointer.buttons & 1 != 0
                    && pointer.drag_start.is_some()
                    && pointer.scrollbar.is_none()
                    && (pointer
                        .selection
                        .is_none_or(|gesture| gesture.permits_drag())
                        || pointer.drag_start.is_some_and(|(node, _, _)| {
                            native_drag::element_drag_source(state, node)
                                .is_ok_and(|source| source.is_some())
                        }))
            })
}

/// Start the source session before resolving its destination document. This
/// avoids a spurious dragenter/dragover on an iframe's embedding element when
/// the first threshold-crossing move already lies inside its child document.
pub fn prepare_native_pointer_drag(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
) -> Result<bool, V8BindingError> {
    let mut receipt = NativePointerDragAdmission::new(state, input)?;
    receipt.poll(scope)?.ok_or(V8BindingError::Allocation(
        "pointer drag admission suspended",
    ))
}

/// Complete the consumed source transition after a host-routed drag event.
/// No mouseup/click or new pointer identity is created by drag transport.
pub fn finish_native_pointer_drag_transition(
    state: &V8BindingState,
    input: &NativePointerInput,
    terminal: bool,
) {
    if let Some(pointer) = state
        .native_pointers
        .borrow_mut()
        .pointers
        .get_mut(&input.id)
    {
        pointer.last_input = Some(input.clone());
        pointer.buttons = input.buttons;
        if terminal {
            pointer.drag_start = None;
            pointer.drag_consumed = false;
            pointer.buttons = 0;
            pointer.prevent_mouse = false;
            pointer.down_targets.clear();
            pointer.selection = None;
            pointer.pending_capture = None;
            pointer.capture = None;
        }
    }
}

/// Release the mirrored pointer state in documents that observed only boundary
/// events. Their local coordinates stay intact; the destination owns delivery
/// of the actual pointerup/cancel and capture-release notifications.
pub fn synchronize_native_pointer_release(state: &V8BindingState, id: i32) {
    let input = state
        .native_pointers
        .borrow()
        .pointers
        .get(&id)
        .and_then(|pointer| pointer.last_input.clone());
    if let Some(mut input) = input {
        input.buttons = 0;
        input.pressure = 0.;
        finish_native_pointer_drag_transition(state, &input, true);
    }
}
pub fn abandon_native_pointer_drag(state: &V8BindingState, id: i32) {
    let session = state.native_drag.borrow_mut().take();
    drop(session);
    native_drag_image::clear(state, None);
    synchronize_native_pointer_release(state, id);
}

pub fn native_pointer_physical_target(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    x: f64,
    y: f64,
) -> Result<Option<NodeId>, V8BindingError> {
    refresh_layout_in_scope(scope, state)?;
    Ok(
        hit_test_snapshot(state, state.document_root(), x, y, false)?
            .first()
            .copied(),
    )
}

/// Dispatch native pointer and compatibility mouse transitions. Drag and touch
/// gesture coordination can consume transitions before calling this path.
fn prepare_touch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    name: &str,
) -> Result<Option<(NodeId, v8::Local<'s, v8::Object>)>, V8BindingError> {
    let contacts = {
        let registry = state.native_pointers.borrow();
        registry
            .pointers
            .iter()
            .filter_map(|(id, pointer)| {
                let contact = pointer.touch_contact.as_ref()?;
                let last = pointer.last_input.as_ref()?;
                Some((
                    *id,
                    contact.target,
                    last.clone(),
                    contact.active() || pointer.touch_zooming,
                ))
            })
            .collect::<Vec<_>>()
    };
    let Some((_, target, _, _)) = contacts.iter().find(|entry| entry.0 == input.id) else {
        return Ok(None);
    };
    let target = *target;
    if !state.dom.borrow().is_connected(target)? {
        return Ok(None);
    }
    let offset = state
        .layout
        .borrow()
        .viewport_scroll
        .as_ref()
        .map(|value| (value.scroll.offset_x, value.scroll.offset_y))
        .unwrap_or_default();
    let mut touches = Vec::new();
    let mut target_touches = Vec::new();
    let mut changed = Vec::new();
    for (id, contact_target, mut last, suppressed) in contacts {
        if id == input.id {
            last = input.clone();
        }
        if suppressed && id != input.id {
            continue;
        }
        let target_object = wrap_node(scope, state, contact_target)?;
        let record = touch_objects::TouchRecord {
            identifier: id,
            target: v8::Global::new(scope, v8::Local::<v8::Value>::from(target_object)),
            coordinates: [
                last.x,
                last.y,
                last.screen_x,
                last.screen_y,
                last.x + offset.0,
                last.y + offset.1,
                last.width / 2.,
                last.height / 2.,
                last.twist as f64,
                last.pressure,
            ],
        };
        let value = touch_objects::snapshot(scope, state, record)?;
        if id == input.id {
            changed.push(value);
        }
        if last.buttons != 0
            && !(id == input.id && matches!(name, "touchend" | "touchcancel"))
            && !suppressed
        {
            touches.push(value);
            if contact_target == target {
                target_touches.push(value);
            }
        }
    }
    let event = create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface: "TouchEvent",
            event_type: name,
            bubbles: true,
            cancelable: name != "touchcancel",
            composed: true,
            detail: 0,
            related: None,
            fields: &[
                ("altKey", NativeInputValue::Bool(input.alt)),
                ("ctrlKey", NativeInputValue::Bool(input.control)),
                ("metaKey", NativeInputValue::Bool(input.meta)),
                ("shiftKey", NativeInputValue::Bool(input.shift)),
            ],
        },
    )?;
    touch_objects::attach_lists(scope, state, event, &touches, &target_touches, &changed)?;
    Ok(Some((target, event)))
}

fn pinch_contacts(state: &V8BindingState) -> Vec<(i32, native_touch::Contact, NativePointerInput)> {
    state
        .native_pointers
        .borrow()
        .pointers
        .iter()
        .filter_map(|(id, pointer)| {
            if pointer.buttons == 0 || pointer.touch_suppressed || pointer.range.is_some() {
                return None;
            }
            Some((
                *id,
                pointer.touch_contact.clone()?,
                pointer.last_input.clone()?,
            ))
        })
        .collect()
}
fn prepare_pinch(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<(), V8BindingError> {
    // The native realm descriptor, never an author parent/top property, grants
    // ownership of the top-level visual viewport.
    if !scope
        .get_current_context()
        .get_slot::<crate::realm::RealmDescriptor>()
        .is_some_and(|realm| realm.frame_id.is_none())
    {
        return Ok(());
    }
    if state.native_pointers.borrow().pinch.is_some() {
        return Ok(());
    }
    let contacts = pinch_contacts(state);
    if contacts.len() != 2 {
        return Ok(());
    }
    let registry = registry_owner(state);
    let lifetime = (state.document_root(), state.document_generation(),
        state.document_stream_epoch(), state.document_authority_epoch());
    refresh_layout_in_scope(scope, state)?;
    let geometry = state.native_visual_viewport_geometry_snapshot()?;
    if !registry_owner_current(state, &registry) || state.execution_retired()
        || lifetime != (state.document_root(), state.document_generation(),
            state.document_stream_epoch(), state.document_authority_epoch()) {
        return Err(V8BindingError::Allocation("pinch preparation owner retired"));
    }
    if state.native_pointers.borrow().pinch.is_some() { return Ok(()); }
    let contacts = pinch_contacts(state);
    if contacts.len() != 2 { return Ok(()); }
    let a = &contacts[0];
    let b = &contacts[1];
    let candidate = native_touch::PinchCandidate::begin(
        geometry,
        Default::default(),
        (&a.1, a.0, a.2.x, a.2.y),
        (&b.1, b.0, b.2.x, b.2.y),
    );
    state.native_pointers.borrow_mut().pinch = candidate;
    Ok(())
}
/// Compatibility entry point. Production input owners retain the same receipt
/// across Pending/errors instead of constructing another physical sample.
pub fn dispatch_native_pointer(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    action: NativePointerAction,
) -> Result<(NodeId, bool), V8BindingError> {
    let mut job = NativePointerDispatch::new(state, input, action)?;
    let result = job
        .poll(scope)?
        .ok_or(V8BindingError::Allocation("native pointer suspended"))?;
    if job.canceled() {
        return Err(V8BindingError::Allocation(
            "pointer input source retired or reset",
        ));
    }
    Ok(result)
}
#[derive(Debug, Clone)]
pub struct NativePointerInput {
    pub id: i32,
    pub device: NativePointerDevice,
    pub primary: bool,
    pub x: f64,
    pub y: f64,
    pub screen_x: f64,
    pub screen_y: f64,
    pub movement_x: f64,
    pub movement_y: f64,
    pub button: i32,
    pub buttons: u32,
    pub width: f64,
    pub height: f64,
    pub pressure: f64,
    pub tangential_pressure: f64,
    pub tilt_x: i32,
    pub tilt_y: i32,
    pub twist: u32,
    pub altitude_angle: f64,
    pub azimuth_angle: f64,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}
impl Default for NativePointerInput {
    fn default() -> Self {
        Self {
            id: 1,
            device: NativePointerDevice::Mouse,
            primary: true,
            x: 0.,
            y: 0.,
            screen_x: 0.,
            screen_y: 0.,
            movement_x: 0.,
            movement_y: 0.,
            button: -1,
            buttons: 0,
            width: 1.,
            height: 1.,
            pressure: 0.,
            tangential_pressure: 0.,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            altitude_angle: std::f64::consts::FRAC_PI_2,
            azimuth_angle: 0.,
            control: false,
            shift: false,
            alt: false,
            meta: false,
        }
    }
}
impl NativePointerInput {
    fn validate(&self) -> Result<(), V8BindingError> {
        if self.id < 0
            || ![
                self.x,
                self.y,
                self.screen_x,
                self.screen_y,
                self.movement_x,
                self.movement_y,
                self.width,
                self.height,
                self.pressure,
                self.tangential_pressure,
                self.altitude_angle,
                self.azimuth_angle,
            ]
            .iter()
            .all(|v| v.is_finite())
            || self.width < 0.
            || self.height < 0.
            || !(0. ..=1.).contains(&self.pressure)
            || !(-1. ..=1.).contains(&self.tangential_pressure)
            || !(-90..=90).contains(&self.tilt_x)
            || !(-90..=90).contains(&self.tilt_y)
            || self.twist > 359
            || !(0. ..=std::f64::consts::FRAC_PI_2).contains(&self.altitude_angle)
            || !(0. ..=std::f64::consts::TAU).contains(&self.azimuth_angle)
        {
            return Err(V8BindingError::Allocation("invalid native pointer input"));
        }
        Ok(())
    }
}

pub(super) fn pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    related: Option<NodeId>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    input.validate()?;
    let pointer_type: Vec<u16> = match input.device {
        NativePointerDevice::Mouse => "mouse",
        NativePointerDevice::Pen => "pen",
        NativePointerDevice::Touch => "touch",
    }
    .encode_utf16()
    .collect();
    let interface = if event_type.starts_with("mouse") || event_type == "dblclick" {
        "MouseEvent"
    } else {
        "PointerEvent"
    };
    // Compatibility MouseEvents do not use PointerEvent's -1 no-change sentinel.
    let button = if interface == "MouseEvent" && input.button < 0 {
        0
    } else {
        input.button
    };
    let composed = !matches!(
        event_type,
        "pointerenter" | "pointerleave" | "mouseenter" | "mouseleave"
    );
    let detail = if matches!(
        event_type,
        "mousedown" | "mouseup" | "click" | "auxclick" | "dblclick"
    ) {
        state
            .native_pointers
            .borrow()
            .pointers
            .get(&input.id)
            .and_then(|pointer| pointer.click_details.get(&input.button))
            .copied()
            .unwrap_or(0)
    } else {
        0
    };
    create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface,
            event_type,
            bubbles,
            cancelable,
            composed,
            detail,
            related,
            fields: &[
                ("pointerId", NativeInputValue::Number(input.id as f64)),
                ("pointerType", NativeInputValue::String(&pointer_type)),
                ("isPrimary", NativeInputValue::Bool(input.primary)),
                ("clientX", NativeInputValue::Number(input.x)),
                ("clientY", NativeInputValue::Number(input.y)),
                ("screenX", NativeInputValue::Number(input.screen_x)),
                ("screenY", NativeInputValue::Number(input.screen_y)),
                ("movementX", NativeInputValue::Number(input.movement_x)),
                ("movementY", NativeInputValue::Number(input.movement_y)),
                ("button", NativeInputValue::Number(button as f64)),
                ("buttons", NativeInputValue::Number(input.buttons as f64)),
                ("width", NativeInputValue::Number(input.width)),
                ("height", NativeInputValue::Number(input.height)),
                ("pressure", NativeInputValue::Number(input.pressure)),
                (
                    "tangentialPressure",
                    NativeInputValue::Number(input.tangential_pressure),
                ),
                ("tiltX", NativeInputValue::Number(input.tilt_x as f64)),
                ("tiltY", NativeInputValue::Number(input.tilt_y as f64)),
                ("twist", NativeInputValue::Number(input.twist as f64)),
                (
                    "altitudeAngle",
                    NativeInputValue::Number(input.altitude_angle),
                ),
                (
                    "azimuthAngle",
                    NativeInputValue::Number(input.azimuth_angle),
                ),
                ("ctrlKey", NativeInputValue::Bool(input.control)),
                ("shiftKey", NativeInputValue::Bool(input.shift)),
                ("altKey", NativeInputValue::Bool(input.alt)),
                ("metaKey", NativeInputValue::Bool(input.meta)),
                ("modifierAltGraph", NativeInputValue::Bool(false)),
                ("modifierCapsLock", NativeInputValue::Bool(false)),
            ],
        },
    )
}

#[derive(Debug, Clone, Copy)]
pub struct NativeFramePointerTarget {
    pub frame: NodeId,
    pub document: NodeId,
    pub identity: NativeFrameInputIdentity,
    pub x: f64,
    pub y: f64,
    pub movement_x: f64,
    pub movement_y: f64,
}
pub fn native_pointer_owns_routing(state: &V8BindingState, id: i32) -> bool {
    let registry = state.native_pointers.borrow();
    !state.execution_retired()
        && registry.identity
            == Some((
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            ))
        && registry.pointers.get(&id).is_some_and(|pointer| {
            pointer.capture.is_some()
                || pointer.pending_capture.is_some()
                || pointer.scrollbar.is_some()
                || pointer.selection.is_some()
                || pointer.drag_consumed
                || pointer.touch_suppressed
                || pointer.touch_zooming
                || pointer
                    .touch_contact
                    .as_ref()
                    .is_some_and(|contact| contact.active())
        })
}

/// Captured pointers may lie outside a frame's content box. Only native frame
/// ancestry grants this mapping; ordinary hit testing uses the bounded variant.
pub fn native_frame_pointer_coordinates(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    frame: NodeId,
    x: f64,
    y: f64,
    movement_x: f64,
    movement_y: f64,
) -> Result<Option<NativeFramePointerTarget>, V8BindingError> {
    if state.execution_retired()
        || ![x, y, movement_x, movement_y]
            .iter()
            .all(|value| value.is_finite())
    {
        return Ok(None);
    }
    if !state.dom.borrow().is_connected(frame)? {
        return Ok(None);
    }
    refresh_layout_in_scope(scope, state)?;
    let Some(identity) = state.native_input_frame_identity(frame) else {
        return Ok(None);
    };
    let layout = state.layout.borrow();
    let Some(projection) = layout.hit_projections.get(&node_key(frame)) else {
        return Ok(None);
    };
    let Some((local_x, local_y)) = projection.viewport_to_local(x, y) else {
        return Ok(None);
    };
    let Some((previous_x, previous_y)) =
        projection.viewport_to_local(x - movement_x, y - movement_y)
    else {
        return Ok(None);
    };
    let content = projection.box_model.content_box;
    Ok(Some(NativeFramePointerTarget {
        frame,
        document: identity.document,
        identity,
        x: local_x - content.x,
        y: local_y - content.y,
        movement_x: local_x - previous_x,
        movement_y: local_y - previous_y,
    }))
}

/// Map child viewport coordinates to its native embedding document, including
/// out-of-box positions used for pointerout/leave after a frame exit.
pub fn native_frame_pointer_parent_coordinates(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    frame: NodeId,
    x: f64,
    y: f64,
    movement_x: f64,
    movement_y: f64,
) -> Result<Option<NativeFramePointerTarget>, V8BindingError> {
    if state.execution_retired() || !state.dom.borrow().is_connected(frame)? {
        return Ok(None);
    }
    refresh_layout_in_scope(scope, state)?;
    let Some(identity) = state.native_input_frame_identity(frame) else {
        return Ok(None);
    };
    let layout = state.layout.borrow();
    let Some(projection) = layout.hit_projections.get(&node_key(frame)) else {
        return Ok(None);
    };
    let content = projection.box_model.content_box;
    let matrix = projection.matrix;
    let project = |x: f64, y: f64| {
        let x = x + content.x;
        let y = y + content.y;
        let w = matrix[3] * x + matrix[7] * y + matrix[15];
        let point = (
            (matrix[0] * x + matrix[4] * y + matrix[12]) / w,
            (matrix[1] * x + matrix[5] * y + matrix[13]) / w,
        );
        (w > 0. && w.is_finite() && point.0.is_finite() && point.1.is_finite()).then_some(point)
    };
    let (Some(point), Some(previous)) = (project(x, y), project(x - movement_x, y - movement_y))
    else {
        return Ok(None);
    };
    Ok(Some(NativeFramePointerTarget {
        frame,
        document: identity.document,
        identity,
        x: point.0,
        y: point.1,
        movement_x: point.0 - previous.0,
        movement_y: point.1 - previous.1,
    }))
}

/// Frame descent uses the frontmost native hit and its content box. Border,
/// padding and viewport chrome stay in the embedding document.
pub fn native_frame_pointer_target(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: Option<i32>,
    x: f64,
    y: f64,
    movement_x: f64,
    movement_y: f64,
) -> Result<Option<NativeFramePointerTarget>, V8BindingError> {
    if id.is_some_and(|id| {
        state
            .native_pointers
            .borrow()
            .pointers
            .get(&id)
            .is_some_and(|pointer| {
                pointer.capture.is_some()
                    || pointer.pending_capture.is_some()
                    || pointer.scrollbar.is_some()
            })
    }) {
        return Ok(None);
    }
    refresh_layout_in_scope(scope, state)?;
    let hit = hit_test_snapshot(state, state.document_root(), x, y, false)?
        .first()
        .copied();
    let Some(frame) = hit else {
        return Ok(None);
    };
    let Some(identity) = state.native_input_frame_identity(frame) else {
        return Ok(None);
    };
    if scrollbar_surface(state, frame, true, false, x, y).is_some_and(|(bars, x, y)| {
        bars.iter().any(|bar| {
            x >= bar.track.x && x < bar.track.right() && y >= bar.track.y && y < bar.track.bottom()
        })
    }) {
        return Ok(None);
    }
    let layout = state.layout.borrow();
    let Some(projection) = layout.hit_projections.get(&node_key(frame)) else {
        return Ok(None);
    };
    let Some((local_x, local_y)) = projection.viewport_to_local(x, y) else {
        return Ok(None);
    };
    let content = projection.box_model.content_box;
    if local_x < content.x
        || local_x >= content.right()
        || local_y < content.y
        || local_y >= content.bottom()
    {
        return Ok(None);
    }
    let previous = projection
        .viewport_to_local(x - movement_x, y - movement_y)
        .unwrap_or((local_x, local_y));
    Ok(Some(NativeFramePointerTarget {
        frame,
        document: identity.document,
        identity,
        x: local_x - content.x,
        y: local_y - content.y,
        movement_x: local_x - previous.0,
        movement_y: local_y - previous.1,
    }))
}

#[cfg(test)]
pub(super) fn hit_test(
    state: &V8BindingState,
    root: NodeId,
    x: f64,
    y: f64,
    retarget: bool,
) -> Result<Vec<NodeId>, V8BindingError> {
    refresh_layout(state)?;
    hit_test_snapshot(state, root, x, y, retarget)
}

/// Consume the same admitted layout snapshot as an input geometry read.
pub(super) fn hit_test_snapshot(
    state: &V8BindingState,
    root: NodeId,
    x: f64,
    y: f64,
    retarget: bool,
) -> Result<Vec<NodeId>, V8BindingError> {
    if state.execution_retired() || !x.is_finite() || !y.is_finite() {
        return Ok(Vec::new());
    }
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    if x < 0.
        || y < 0.
        || layout
            .viewport
            .is_some_and(|(width, height)| x > width || y > height)
    {
        return Ok(Vec::new());
    }
    let root_element = dom
        .arena()
        .get(root)?
        .children
        .iter()
        .copied()
        .find(|child| {
            dom.arena()
                .get(*child)
                .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
        });
    let empty_style = BTreeMap::new();
    let mut stack = Vec::new();
    let mut ids = BTreeMap::new();
    let mut pending = vec![(root, None)];
    let mut seen = BTreeSet::new();
    while let Some((node, parent)) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        let Ok(record) = dom.arena().get(node) else {
            continue;
        };
        let mut next_parent = parent;
        if matches!(record.kind, NodeKind::Element { .. }) {
            let key = node_key(node);
            ids.insert(key, node);
            stack.push(oryn_common::paint_order::StackingNode {
                top_layer: dom.top_layer().position(|entry| {
                    entry.node == node
                        && !matches!(
                            entry.kind,
                            oryn_dom::TopLayerKind::SelectPicker
                                | oryn_dom::TopLayerKind::TemporalPicker
                                | oryn_dom::TopLayerKind::PermissionPrompt
                        )
                }),
                node_id: key,
                parent_id: layout
                    .paint_parent_overrides
                    .get(&key)
                    .copied()
                    .unwrap_or(parent),
                style: layout
                    .styles
                    .get(&key)
                    .map(|style| style.as_ref())
                    .unwrap_or(&empty_style),
            });
            next_parent = Some(key);
        }
        pending.extend(
            dom.flat_tree_children(node)?
                .into_iter()
                .rev()
                .map(|child| (child, next_parent)),
        );
    }
    for generated in &layout.generated_nodes {
        let Some(owner) = generated
            .parent_id
            .and_then(|owner| ids.get(&owner).copied())
        else {
            continue;
        };
        if ids.contains_key(&generated.node_id) {
            return Err(V8BindingError::Allocation(
                "generated hit identity collision",
            ));
        }
        ids.insert(generated.node_id, owner);
        let record = oryn_common::paint_order::StackingNode {
            top_layer: generated.top_layer,
            node_id: generated.node_id,
            parent_id: generated.parent_id,
            style: &generated.style,
        };
        if generated.tag == "::before" {
            if let Some(index) = stack
                .iter()
                .position(|node| Some(node.node_id) == generated.parent_id)
            {
                stack.insert(index + 1, record);
            } else {
                stack.push(record);
            }
        } else {
            stack.push(record);
        }
    }
    let mut hits = Vec::new();
    let mut unique = BTreeSet::new();
    for key in oryn_common::paint_order::stacking_order(&stack)
        .into_iter()
        .rev()
    {
        let Some(node) = ids.get(&key).copied() else {
            continue;
        };
        if dom.is_inert(node).unwrap_or(true) {
            continue;
        }
        let ignores_pointer = layout.styles.get(&key).is_some_and(|style| {
            style
                .get("pointer-events")
                .is_some_and(|value| value == "none")
                || style
                    .get("visibility")
                    .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
                || style
                    .get("-oryn-anchor-hidden")
                    .is_some_and(|value| value == "true")
                || style.get("display").is_some_and(|value| value == "none")
        });
        let Some(BindingGeometry::Available(rect)) = layout.geometry.get(&key) else {
            continue;
        };
        let contains = |rect: &BindingRect| {
            rect.width > 0.
                && rect.height > 0.
                && x >= rect.x
                && y >= rect.y
                && x <= rect.x + rect.width
                && y <= rect.y + rect.height
        };
        let hit = if let Some(projection) = layout.hit_projections.get(&key) {
            projection.contains_point(x, y)
        } else {
            layout
                .client_rects
                .get(&key)
                .map(|rects| rects.iter().any(contains))
                .unwrap_or_else(|| contains(rect))
        };
        let backdrop = layout.backdrops.get(&key).is_some_and(|backdrop| {
            backdrop.parent_id == Some(key)
                && backdrop.backdrop.is_none()
                && backdrop.backdrop_style.is_none()
                && !backdrop
                    .style
                    .get("pointer-events")
                    .is_some_and(|value| value == "none")
                && !backdrop
                    .style
                    .get("visibility")
                    .is_some_and(|value| value == "hidden" || value == "collapse")
                && backdrop
                    .projection
                    .as_ref()
                    .is_some_and(|projection| projection.contains_point(x, y))
        });
        if (hit && !ignores_pointer) || backdrop {
            let node = if hit && !ignores_pointer {
                image_maps::target(&dom, &layout, node, x, y)?
            } else {
                node
            };
            let node = if retarget {
                dom.retarget_node(node, Some(root))?
            } else {
                node
            };
            if unique.insert(node) {
                hits.push(node);
            }
        }
    }
    if let Some(root) = root_element
        && !dom.is_inert(root)?
        && unique.insert(root)
    {
        hits.push(root);
    }
    Ok(hits)
}

#[derive(Debug, Clone)]
struct WheelTarget {
    owner: u64,
    document: NodeId,
    generation: u64,
    stream: u64,
    authority: u64,
    node: NodeId,
    ancestors: Vec<NodeId>,
    cursor: oryn_dom::MutationCursor,
}
/// One native action-chain/source transaction. Its lifetime is owned by the
/// action execution, so completion, cancellation and error drop its target.
#[derive(Debug, Default)]
pub struct NativeWheelTransaction {
    target: std::sync::Mutex<Option<WheelTarget>>,
}
impl NativeWheelTransaction {
    pub fn clear(&self) {
        *self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
#[derive(Debug, Clone, Default)]
pub struct NativeWheelInput {
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub transaction: Option<std::sync::Arc<NativeWheelTransaction>>,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeWheelResult {
    pub allowed: bool,
    pub remaining: (f64, f64),
}
pub fn dispatch_native_wheel(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativeWheelInput,
) -> Result<bool, V8BindingError> {
    dispatch_native_wheel_with_remainder(scope, state, input).map(|result| result.allowed)
}
fn wheel_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativeWheelInput,
) -> Result<NodeId, V8BindingError> {
    refresh_layout_in_scope(scope, state)?;
    let cached = input.transaction.as_ref().and_then(|transaction| {
        transaction
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    });
    if let Some(mut cached) = cached {
        let dom = state.dom.borrow();
        let batch = dom.mutations_since(cached.cursor);
        let layout = state.layout.borrow();
        let key = node_key(cached.node);
        let moved=batch.truncated||batch.mutations.iter().any(|mutation|matches!(mutation,oryn_dom::Mutation::Removed{node,..}|oryn_dom::Mutation::Inserted{node,..} if cached.ancestors.contains(node)));
        let rendered = matches!(layout.geometry.get(&key),Some(BindingGeometry::Available(rect)) if rect.width>0.&&rect.height>0.)
            && !layout.styles.get(&key).is_some_and(|style| {
                style
                    .get("display")
                    .is_some_and(|display| matches!(display.as_str(), "none" | "contents"))
                    || style.get("visibility").is_some_and(|visibility| {
                        matches!(visibility.as_str(), "hidden" | "collapse")
                    })
                    || style
                        .get("pointer-events")
                        .is_some_and(|value| value == "none")
            });
        if cached.owner == state.instance_id
            && cached.document == state.document_root()
            && cached.generation == state.document_generation()
            && cached.stream == state.document_stream_epoch()
            && cached.authority == state.document_authority_epoch()
            && !moved
            && rendered
            && dom.is_connected(cached.node).unwrap_or(false)
            && dom.node_document(cached.node).ok().flatten() == Some(state.document_root())
            && !dom.is_inert(cached.node).unwrap_or(true)
        {
            cached.cursor = batch.next;
            let target = cached.node;
            drop(layout);
            drop(dom);
            if let Some(transaction) = &input.transaction {
                *transaction
                    .target
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cached);
            }
            return Ok(target);
        }
    }
    let target = hit_test_snapshot(state, state.document_root(), input.x, input.y, false)?
        .first()
        .copied()
        .ok_or(V8BindingError::Allocation(
            "native wheel target unavailable",
        ))?;
    if let Some(transaction) = &input.transaction {
        let dom = state.dom.borrow();
        let mut ancestors = Vec::new();
        let mut current = Some(target);
        while let Some(node) = current {
            if ancestors.contains(&node) {
                break;
            }
            ancestors.push(node);
            current = dom.flat_tree_parent(node)?;
        }
        *transaction
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(WheelTarget {
            owner: state.instance_id,
            document: state.document_root(),
            generation: state.document_generation(),
            stream: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            node: target,
            ancestors,
            cursor: dom.mutation_cursor(),
        });
    }
    Ok(target)
}
pub fn dispatch_native_wheel_with_remainder(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativeWheelInput,
) -> Result<NativeWheelResult, V8BindingError> {
    if ![input.x, input.y, input.delta_x, input.delta_y]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(V8BindingError::Allocation(
            "invalid native wheel coordinates",
        ));
    }
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let live = || {
        !state.execution_retired()
            && (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            ) == identity
    };
    if !live() {
        return Err(V8BindingError::Allocation("native wheel document retired"));
    }
    if !input.control && temporal_picker::wheel(scope, state, input.x, input.y, input.delta_y)? {
        return Ok(NativeWheelResult {
            allowed: true,
            remaining: (0., 0.),
        });
    }
    let target = wheel_target(scope, state, input)?;
    let event = create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface: "WheelEvent",
            event_type: "wheel",
            bubbles: true,
            cancelable: true,
            composed: true,
            detail: 0,
            related: None,
            fields: &[
                ("clientX", NativeInputValue::Number(input.x)),
                ("clientY", NativeInputValue::Number(input.y)),
                ("screenX", NativeInputValue::Number(input.x)),
                ("screenY", NativeInputValue::Number(input.y)),
                ("movementX", NativeInputValue::Number(0.)),
                ("movementY", NativeInputValue::Number(0.)),
                ("button", NativeInputValue::Number(0.)),
                ("buttons", NativeInputValue::Number(0.)),
                ("deltaX", NativeInputValue::Number(input.delta_x)),
                ("deltaY", NativeInputValue::Number(input.delta_y)),
                ("deltaZ", NativeInputValue::Number(0.)),
                ("deltaMode", NativeInputValue::Number(0.)),
                ("ctrlKey", NativeInputValue::Bool(input.control)),
                ("shiftKey", NativeInputValue::Bool(input.shift)),
                ("altKey", NativeInputValue::Bool(input.alt)),
                ("metaKey", NativeInputValue::Bool(input.meta)),
                ("modifierAltGraph", NativeInputValue::Bool(false)),
                ("modifierCapsLock", NativeInputValue::Bool(false)),
            ],
        },
    )?;
    let allowed = dispatch_event(scope, state, target, event)?;
    if !live() {
        return Err(V8BindingError::Allocation(
            "wheel listener retired its document",
        ));
    }
    let remaining = if allowed && state.dom.borrow().is_connected(target)? {
        refresh_layout_in_scope(scope, state)?;
        if let Some(remaining) = select_picker_scroll::wheel_surface_snapshot(
            state,
            target,
            input.x,
            input.y,
            (input.delta_x, input.delta_y),
        )? {
            remaining
        } else {
            apply_native_wheel_default_from_snapshot(
                scope,
                state,
                target,
                input.delta_x,
                input.delta_y,
            )?
        }
    } else {
        (0., 0.)
    };
    Ok(NativeWheelResult { allowed, remaining })
}

#[cfg(test)]
mod click_history_tests {
    use super::*;

    #[test]
    fn pinch_provider_removes_contact_without_resetting_registry() {
        struct Provider {
            owner: RcWeak<V8BindingState>,
            calls: Rc<Cell<usize>>,
        }
        impl BindingLayoutProvider for Provider {
            fn compute_layout(
                &self,
                _: &BTreeMap<NodeId, BTreeSet<String>>,
            ) -> Result<BindingLayout, String> {
                self.calls.set(self.calls.get() + 1);
                self.owner
                    .upgrade()
                    .unwrap()
                    .native_pointers
                    .borrow_mut()
                    .pointers
                    .remove(&2);
                Ok(BindingLayout {
                    viewport: Some((400., 300.)),
                    ..Default::default()
                })
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://example.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        // Production dispatch synchronizes the registry before installing
        // contacts. Do that first here; synchronization clears stale records.
        let registry = registry_owner(&state);
        for (id, x) in [(1, 100.), (2, 200.)] {
            let input = NativePointerInput {
                id,
                device: NativePointerDevice::Touch,
                x,
                y: 100.,
                buttons: 1,
                ..Default::default()
            };
            let contact = native_touch::Contact::new(&state, target, x, 100., Default::default());
            state.native_pointers.borrow_mut().pointers.insert(
                id,
                PointerRecord {
                    buttons: 1,
                    touch_contact: Some(contact),
                    last_input: Some(input),
                    ..Default::default()
                },
            );
        }
        assert_eq!(pinch_contacts(&state).len(), 2);
        let calls = Rc::new(Cell::new(0));
        state.set_layout_provider(Rc::new(Provider {
            owner: Rc::downgrade(&state),
            calls: calls.clone(),
        }));
        prepare_pinch(scope, &state).unwrap();
        assert_eq!(calls.get(), 1);
        assert!(registry_owner_current(&state, &registry));
        assert_eq!(pinch_contacts(&state).len(), 1);
        assert!(
            state.native_pointers.borrow().pinch.is_none(),
            "same-registry removal during provider layout must not publish stale contacts"
        );
    }

    #[test]
    fn zoomed_chrome_commits_visual_pan_and_layout_scroll_from_shared_offsets() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body>content</body>")
                .into_runtime()
                .unwrap(),
        ));
        let root = dom
            .borrow()
            .first_element_child(dom.borrow().document())
            .unwrap()
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((100., 100.)),
            viewport_scroll: Some(oryn_common::v4::PaintViewportScroll {
                scroll: oryn_common::v4::PaintScrollGeometry {
                    offset_x: 0.,
                    offset_y: 0.,
                    scrollport: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 90.,
                        height: 80.,
                    },
                    scrollable: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 90.,
                        height: 180.,
                    },
                },
                style: BTreeMap::from([("overflow-y".into(), "scroll".into())]),
            }),
            ..Default::default()
        });
        *state.native_visual_viewport.borrow_mut() = visual_viewport::Geometry {
            scrollport: None,
            layout_width: 100.,
            layout_height: 100.,
            scale: 2.,
            offset_x: 0.,
            offset_y: 0.,
        };
        let gesture = ScrollbarGesture {
            picker_epoch: None,
            picker: false,
            node: root,
            viewport: true,
            horizontal: false,
            thumb_delta: Some(0.),
            repeat_at: std::time::Instant::now(),
            repeat_direction: 1.,
        };
        let (bars, x, y) = scrollbar_surface(&state, root, true, false, 47.5, 5.).unwrap();
        assert_eq!((x, y), (95., 10.));
        let bar = &bars[0];
        assert_eq!(
            (bar.track.x, bar.track.height, bar.page_step, bar.max_offset),
            (90., 80., 40., 140.)
        );
        commit_scrollbar_offset(scope, &state, gesture, 40.).unwrap();
        assert_eq!(state.document_scroll_offset().1, 0.);
        assert_eq!(state.native_visual_viewport.borrow().offset_y, 40.);
        commit_scrollbar_offset(scope, &state, gesture, 140.).unwrap();
        assert_eq!(state.document_scroll_offset().1, 100.);
        assert_eq!(state.native_visual_viewport.borrow().offset_y, 40.);
        let visual = *state.native_visual_viewport.borrow();
        let point = visual.visual_to_layout(95., 70.).unwrap();
        let (bars, _, _) = scrollbar_surface(&state, root, true, false, point.0, point.1).unwrap();
        assert_eq!(bars[0].offset, 140.);
        assert!((bars[0].offset_at_thumb_origin(bars[0].thumb.y).unwrap() - 140.).abs() < 0.01);
        commit_scrollbar_offset(scope, &state, gesture, 0.).unwrap();
        assert_eq!(state.document_scroll_offset().1, 0.);
        assert_eq!(state.native_visual_viewport.borrow().offset_y, 0.);
    }

    #[test]
    fn pinch_callback_reset_cannot_publish_stale_geometry() {
        fn reset(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            let state = scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .unwrap();
            reset_native_pointer_sources(&state);
            state.native_visual_viewport.borrow_mut().scale = 3.;
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://example.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((400., 300.)),
            geometry: BTreeMap::from([(
                node_key(target),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 400.,
                    height: 300.,
                }),
            )]),
            ..Default::default()
        });
        // Register a native embedding callback directly; no production input
        // hook or privileged author global is installed for this regression.
        let object = wrap_node(scope, &state, target).unwrap();
        let key = v8::String::new(scope, "addEventListener").unwrap();
        let method = object.get(scope, key.into()).unwrap();
        let method = v8::Local::<v8::Function>::try_from(method).unwrap();
        let name = v8::String::new(scope, "pointercancel").unwrap();
        let callback = v8::Function::new(scope, reset).unwrap();
        method
            .call(scope, object.into(), &[name.into(), callback.into()])
            .unwrap();
        let first = NativePointerInput {
            id: 1,
            device: NativePointerDevice::Touch,
            x: 100.,
            y: 100.,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            ..Default::default()
        };
        let mut second = NativePointerInput {
            id: 2,
            primary: false,
            x: 200.,
            ..first.clone()
        };
        dispatch_native_pointer(scope, &state, &first, NativePointerAction::Down).unwrap();
        dispatch_native_pointer(scope, &state, &second, NativePointerAction::Down).unwrap();
        second.x = 300.;
        second.button = -1;
        assert!(
            dispatch_native_pointer(scope, &state, &second, NativePointerAction::Move).is_err()
        );
        assert_eq!(state.native_visual_viewport.borrow().scale, 3.);
        assert!(state.native_pointers.borrow().pinch.is_none());
        assert!(state.native_pointers.borrow().pointers.is_empty());
    }

    #[test]
    fn native_pinch_cancels_both_streams_and_consumes_both_releases() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://example.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((400., 300.)),
            geometry: BTreeMap::from([(
                node_key(target),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 400.,
                    height: 300.,
                }),
            )]),
            ..Default::default()
        });
        let code=v8::String::new(scope,"globalThis.log=[];for(const type of ['pointercancel','touchcancel','pointerup','touchend','click'])document.addEventListener(type,e=>log.push(type))").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let mut first = NativePointerInput {
            id: 1,
            device: NativePointerDevice::Touch,
            x: 100.,
            y: 100.,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            ..Default::default()
        };
        let mut second = NativePointerInput {
            id: 2,
            primary: false,
            x: 200.,
            ..first.clone()
        };
        dispatch_native_pointer(scope, &state, &first, NativePointerAction::Down).unwrap();
        dispatch_native_pointer(scope, &state, &second, NativePointerAction::Down).unwrap();
        second.x = 300.;
        second.button = -1;
        let mut move_job =
            NativePointerDispatch::new(&state, &second, NativePointerAction::Move).unwrap();
        for _ in 0..4 {
            native_platform::publication::fail_after_next_callback();
            assert!(move_job.poll(scope).is_err());
            assert_eq!(state.native_visual_viewport.borrow().scale, 1.);
        }
        assert!(move_job.poll(scope).unwrap().is_some());
        assert!(move_job.poll(scope).unwrap().is_some());
        assert_eq!(state.native_visual_viewport.borrow().scale, 2.);
        assert!(native_pointer_owns_routing(&state, first.id));
        assert!(native_pointer_owns_routing(&state, second.id));
        first.buttons = 0;
        first.button = 0;
        first.pressure = 0.;
        dispatch_native_pointer(scope, &state, &first, NativePointerAction::Up).unwrap();
        second.buttons = 0;
        second.button = 0;
        second.pressure = 0.;
        dispatch_native_pointer(scope, &state, &second, NativePointerAction::Up).unwrap();
        assert!(state.native_pointers.borrow().pinch.is_none());
        assert!(!touch_gesture_active(&state));
        let code = v8::String::new(
            scope,
            "log.join(',')==='pointercancel,touchcancel,pointercancel,touchcancel'",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn suspension_cannot_reanimate_pending_or_panning_touch_as_click() {
        super::super::tests::initialize_v8();
        for pan in [false, true] {
            let dom = Rc::new(RefCell::new(
                html::parse("<!doctype html><body><button id=t>Touch</button></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let target = dom.borrow().find_by_id("t").unwrap();
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            state.set_layout(BindingLayout {
                generated_nodes: Vec::new(),
                pseudo_styles: Default::default(),
                viewport: Some((100., 100.)),
                geometry: BTreeMap::from([(
                    node_key(target),
                    BindingGeometry::Available(BindingRect {
                        x: 0.,
                        y: 0.,
                        width: 80.,
                        height: 80.,
                    }),
                )]),
                ..Default::default()
            });
            let source = "globalThis.log=[];for(const type of ['pointerdown','pointermove','pointerup','pointercancel','touchstart','touchmove','touchend','touchcancel','mousedown','mouseup','click'])document.addEventListener(type,e=>log.push(e.type))";
            let code = v8::String::new(scope, source).unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let mut input = NativePointerInput {
                device: NativePointerDevice::Touch,
                x: 30.,
                y: 50.,
                button: 0,
                buttons: 1,
                pressure: 0.5,
                ..Default::default()
            };
            dispatch_native_pointer(scope, &state, &input, NativePointerAction::Down).unwrap();
            if pan {
                input.y = 30.;
                input.button = -1;
                dispatch_native_pointer(scope, &state, &input, NativePointerAction::Move).unwrap();
            }
            let code = v8::String::new(scope, "globalThis.before=log.join(',')").unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            cancel_native_input_defaults(&state);
            assert!(native_pointer_owns_routing(&state, input.id));
            input.y = 20.;
            input.button = -1;
            dispatch_native_pointer(scope, &state, &input, NativePointerAction::Move).unwrap();
            input.buttons = 0;
            input.button = 0;
            input.pressure = 0.;
            dispatch_native_pointer(scope, &state, &input, NativePointerAction::Up).unwrap();
            assert!(!native_pointer_owns_routing(&state, input.id));
            let code =
                v8::String::new(scope, "log.join(',')===before&&!log.includes('click')").unwrap();
            assert!(
                v8::Script::compile(scope, code, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
            // The next physical press creates a fresh contact, without inheriting
            // suppression from the parked document's old input sequence.
            input.buttons = 1;
            input.pressure = 0.5;
            dispatch_native_pointer(scope, &state, &input, NativePointerAction::Down).unwrap();
            input.buttons = 0;
            input.pressure = 0.;
            dispatch_native_pointer(scope, &state, &input, NativePointerAction::Up).unwrap();
            let code =
                v8::String::new(scope, "log.filter(type=>type==='click').length===1").unwrap();
            assert!(
                v8::Script::compile(scope, code, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
    }

    #[test]
    fn native_touch_pan_scrolls_and_release_removes_suppressed_contact() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let root = dom
            .borrow()
            .first_element_child(dom.borrow().document())
            .unwrap()
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((100., 100.)),
            viewport_scroll: Some(oryn_common::v4::PaintViewportScroll {
                scroll: oryn_common::v4::PaintScrollGeometry {
                    offset_x: 0.,
                    offset_y: 0.,
                    scrollport: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 100.,
                        height: 100.,
                    },
                    scrollable: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 100.,
                        height: 400.,
                    },
                },
                style: BTreeMap::from([("overflow-y".into(), "auto".into())]),
            }),
            ..Default::default()
        });
        let mut input = NativePointerInput {
            device: NativePointerDevice::Touch,
            x: 50.,
            y: 70.,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            ..Default::default()
        };
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Down).unwrap();
        input.y = 40.;
        input.button = -1;
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Move).unwrap();
        assert_eq!(state.scroll_offsets.borrow()[&root].1, 30.);
        assert!(native_pointer_owns_routing(&state, input.id));
        {
            let pointers = state.native_pointers.borrow();
            let pointer = &pointers.pointers[&input.id];
            assert!(pointer.touch_contact.as_ref().unwrap().active());
            assert!(pointer.down_targets.is_empty());
            assert!(pointer.capture.is_none());
        }
        input.y = 55.;
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Move).unwrap();
        assert_eq!(state.scroll_offsets.borrow()[&root].1, 15.);
        input.buttons = 0;
        input.button = 0;
        input.pressure = 0.;
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Up).unwrap();
        assert!(
            !state
                .native_pointers
                .borrow()
                .pointers
                .contains_key(&input.id)
        );
        assert!(!native_pointer_owns_routing(&state, input.id));
    }

    #[test]
    fn held_scrollbar_track_repeats_once_per_checkpoint_and_stops_on_release() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let root = dom
            .borrow()
            .first_element_child(dom.borrow().document())
            .unwrap()
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((100., 100.)),
            viewport_scroll: Some(oryn_common::v4::PaintViewportScroll {
                scroll: oryn_common::v4::PaintScrollGeometry {
                    offset_x: 0.,
                    offset_y: 0.,
                    scrollport: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 90.,
                        height: 80.,
                    },
                    scrollable: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 90.,
                        height: 400.,
                    },
                },
                style: BTreeMap::from([("overflow-y".into(), "scroll".into())]),
            }),
            ..Default::default()
        });
        let mut input = NativePointerInput {
            x: 95.,
            y: 75.,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            ..Default::default()
        };
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Down).unwrap();
        assert_eq!(state.scroll_offsets.borrow()[&root].1, 80.);
        let due = state.native_pointers.borrow().pointers[&input.id]
            .scrollbar
            .unwrap()
            .repeat_at;
        assert_eq!(
            poll_native_input_defaults(scope, &state, due - std::time::Duration::from_millis(1))
                .unwrap(),
            0
        );
        state
            .layout
            .borrow_mut()
            .viewport_scroll
            .as_mut()
            .unwrap()
            .scroll
            .offset_y = 80.;
        let late = due + std::time::Duration::from_secs(10);
        assert_eq!(poll_native_input_defaults(scope, &state, late).unwrap(), 1);
        assert_eq!(state.scroll_offsets.borrow()[&root].1, 160.);
        assert_eq!(
            native_input_delay(&state, late),
            Some(std::time::Duration::from_millis(50))
        );
        assert_eq!(poll_native_input_defaults(scope, &state, late).unwrap(), 0);
        input.buttons = 0;
        input.pressure = 0.;
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Up).unwrap();
        assert_eq!(native_input_delay(&state, late), None);
        assert_eq!(
            poll_native_input_defaults(scope, &state, late + std::time::Duration::from_secs(1))
                .unwrap(),
            0
        );
    }

    #[test]
    fn multi_click_policy_resets_for_time_distance_target_and_button() {
        let dom = html::parse("<div id=a></div><div id=b></div>")
            .into_runtime()
            .unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let now = std::time::Instant::now();
        let history = ClickHistory {
            target: a,
            button: 0,
            x: 10.,
            y: 10.,
            at: now,
            count: 1,
        };
        let input = NativePointerInput {
            x: 10.,
            y: 10.,
            button: 0,
            ..Default::default()
        };
        assert_eq!(
            history.next(a, &input, now + std::time::Duration::from_millis(499)),
            2
        );
        assert_eq!(
            history.next(a, &input, now + std::time::Duration::from_millis(501)),
            1
        );
        assert_eq!(history.next(b, &input, now), 1);
        assert_eq!(
            history.next(
                a,
                &NativePointerInput {
                    button: 2,
                    ..input.clone()
                },
                now
            ),
            1
        );
        assert_eq!(
            history.next(a, &NativePointerInput { x: 15., ..input }, now),
            1
        );
    }
}

pub(super) fn touch_gesture_active(state: &V8BindingState) -> bool {
    state
        .native_pointers
        .borrow()
        .pointers
        .values()
        .any(|pointer| {
            pointer.touch_zooming && pointer.buttons != 0
                || pointer
                    .touch_contact
                    .as_ref()
                    .is_some_and(|contact| contact.active())
        })
}

#[cfg(test)]
mod source_reset_lifetime_tests {
    use super::*;
    fn reset_and_reuse(
        scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _rv: v8::ReturnValue,
    ) {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .unwrap();
        reset_native_pointer_sources(&state);
        let mut registry = state.native_pointers.borrow_mut();
        registry.synchronize(&state);
        registry.pointers.entry(1).or_default().buttons = 16;
    }
    #[test]
    fn pointer_callback_reset_skips_later_defaults_and_preserves_reused_source() {
        super::super::tests::initialize_v8();
        for event in [
            "pointerdown",
            "mousedown",
            "mouseup",
            "click",
            "contextmenu",
        ] {
            let dom = Rc::new(RefCell::new(
                html::parse("<button id=t>target</button>")
                    .into_runtime()
                    .unwrap(),
            ));
            let target = dom.borrow().find_by_id("t").unwrap();
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            state.set_layout(BindingLayout {
                viewport: Some((100., 100.)),
                geometry: BTreeMap::from([(
                    node_key(target),
                    BindingGeometry::Available(BindingRect {
                        x: 0.,
                        y: 0.,
                        width: 100.,
                        height: 100.,
                    }),
                )]),
                ..Default::default()
            });
            let object = wrap_node(scope, &state, target).unwrap();
            let key = v8::String::new(scope, "addEventListener").unwrap();
            let method = object.get(scope, key.into()).unwrap();
            let method = v8::Local::<v8::Function>::try_from(method).unwrap();
            let name = v8::String::new(scope, event).unwrap();
            let callback = v8::Function::new(scope, reset_and_reuse).unwrap();
            method
                .call(scope, object.into(), &[name.into(), callback.into()])
                .unwrap();
            let button = if event == "contextmenu" { 2 } else { 0 };
            let down = NativePointerInput {
                x: 20.,
                y: 20.,
                button,
                buttons: if button == 2 { 2 } else { 1 },
                ..Default::default()
            };
            let on_up = matches!(event, "mouseup" | "click")
                || (event == "contextmenu" && cfg!(target_os = "windows"));
            let result = if on_up {
                dispatch_native_pointer(scope, &state, &down, NativePointerAction::Down).unwrap();
                dispatch_native_pointer(
                    scope,
                    &state,
                    &NativePointerInput { buttons: 0, ..down },
                    NativePointerAction::Up,
                )
            } else {
                dispatch_native_pointer(scope, &state, &down, NativePointerAction::Down)
            };
            assert!(result.is_err(), "{event}");
            if matches!(event, "pointerdown" | "mousedown") {
                assert_ne!(state.dom.borrow().focused(), Some(target), "{event}");
            }
            let registry = state.native_pointers.borrow();
            let reused = &registry.pointers[&1];
            assert_eq!(reused.buttons, 16, "{event}");
            assert!(reused.down_targets.is_empty(), "{event}");
            assert!(reused.click_history.is_none(), "{event}");
            assert!(reused.selection.is_none(), "{event}");
        }
    }
}
