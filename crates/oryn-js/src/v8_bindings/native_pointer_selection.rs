//! Selection gestures use native shaped caret positions and native ranges.
type SelectionHit = (NodeId, (f64, f64));

use super::*;

#[derive(Clone, Copy)]
pub(super) struct Gesture {
    target: NodeId,
    anchor: BoundaryPoint,
    control: bool,
    deferred: bool,
    generation: u64,
    epoch: u64,
    unit: Option<(BoundaryPoint, BoundaryPoint)>,
    count: i32,
    pub(super) autoscroll_at: Option<std::time::Instant>,
}
impl Gesture {
    pub(super) fn permits_drag(self) -> bool {
        self.deferred
    }
}
fn live(state: &V8BindingState, gesture: Gesture) -> bool {
    !state.execution_retired()
        && state.document_generation() == gesture.generation
        && state.document_stream_epoch() == gesture.epoch
        && state
            .dom
            .borrow()
            .is_connected(gesture.anchor.container)
            .unwrap_or(false)
        && state
            .dom
            .borrow()
            .is_connected(gesture.target)
            .unwrap_or(false)
        && state
            .dom
            .borrow()
            .node_document(gesture.anchor.container)
            .ok()
            .flatten()
            == Some(state.document_root())
        && state
            .dom
            .borrow()
            .node_document(gesture.target)
            .ok()
            .flatten()
            == Some(state.document_root())
}
fn apply(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    gesture: Gesture,
    point: BoundaryPoint,
) -> Result<(), V8BindingError> {
    if !live(state, gesture) {
        return Ok(());
    }
    if gesture.control {
        if point.container != gesture.target {
            return Ok(());
        }
        let selected_control = selection_text::prepare_control(state, gesture.target)?;
        let length =
            oryn_dom::dom_string::len(&state.dom.borrow().control_edit_value(gesture.target)?)
                as u32;
        let anchor = gesture.anchor.offset.min(length);
        let focus = point.offset.min(length);
        let mut selection = (
            anchor.min(focus),
            anchor.max(focus),
            if focus < anchor {
                "backward"
            } else {
                "forward"
            }
            .into(),
        );
        if state.dom.borrow().is_temporal_control(gesture.target) {
            state
                .dom
                .borrow_mut()
                .align_temporal_selection(gesture.target, focus)?;
            if anchor == focus {
                let (start, end) = state.dom.borrow().temporal_selection(gesture.target)?;
                selection = (start, end, "none".into());
            }
        }
        let changed =
            state.text_control_selections.borrow().get(&gesture.target) != Some(&selection);
        let notification = if changed && !state.control_tasks_retired.get() {
            if state.control_event_tasks.borrow().len() >= 1024 {
                return Err(V8BindingError::Allocation("control event queue limit"));
            }
            {
                let wrapper = wrap_node(scope, state, gesture.target)?;
                Some(v8::Global::new(scope, wrapper))
            }
        } else {
            None
        };
        if let Some(control) = selected_control {
            *state.selected_text_control.borrow_mut() = Some(control);
        }
        if changed {
            state
                .text_control_selections
                .borrow_mut()
                .insert(gesture.target, selection);
            state.control_modify_goal.borrow_mut().take();
            if let Some(wrapper) = notification {
                state
                    .control_event_tasks
                    .borrow_mut()
                    .push_back((gesture.target, wrapper));
            }
        }
    } else {
        update_selection_boundaries(state, gesture.anchor, point, true)?;
    }
    Ok(())
}
pub(super) struct Selection {
    owner: Rc<V8BindingState>,
    input_owner: native_pointer::InputOwner,
    gesture: Option<Gesture>,
    point: Option<BoundaryPoint>,
    target: NodeId,
    release: bool,
    x: f64,
    y: f64,
    autoscroll: bool,
    start: bool,
    event: Option<retained_dom_event::Event>,
    phase: u8,
    result: Option<Option<Gesture>>,
}
impl Selection {
    fn completed(state: &Rc<V8BindingState>, result: Option<Gesture>) -> Self {
        Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            gesture: None,
            point: None,
            target: state.document_root(),
            release: false,
            x: 0.,
            y: 0.,
            autoscroll: false,
            start: false,
            event: None,
            phase: 3,
            result: Some(result),
        }
    }
    pub(super) fn begin(
        state: &Rc<V8BindingState>,
        target: NodeId,
        x: f64,
        y: f64,
        shift: bool,
        count: i32,
    ) -> Result<Option<Self>, V8BindingError> {
        if selection_text::preserves_control_on_pointer(state, target)? {
            return Ok(None);
        }
        let Some(point) = selection_movement::pointer_caret(state, target, x, y)? else {
            return Ok(None);
        };
        let control = state.dom.borrow().is_editable_text_control(target);
        let (old_anchor, old_focus) = if control {
            let selected = state.text_control_selections.borrow().get(&target).cloned();
            selected
                .map(|(start, end, direction)| {
                    let (anchor, focus) = if direction == "backward" {
                        (end, start)
                    } else {
                        (start, end)
                    };
                    (
                        Some(BoundaryPoint {
                            container: target,
                            offset: anchor,
                        }),
                        Some(BoundaryPoint {
                            container: target,
                            offset: focus,
                        }),
                    )
                })
                .unwrap_or((None, None))
        } else {
            (state.selection_anchor.get(), state.selection_focus.get())
        };
        let inside = if let (Some(anchor), Some(focus)) = (old_anchor, old_focus) {
            let dom = state.dom.borrow();
            let compare = |a: BoundaryPoint, b: BoundaryPoint| {
                compare_boundary_points(&dom, (a.container, a.offset), (b.container, b.offset))
            };
            if anchor == focus {
                false
            } else if compare(anchor, focus).is_ok_and(|order| order <= 0) {
                compare(anchor, point).is_ok_and(|order| order <= 0)
                    && compare(point, focus).is_ok_and(|order| order <= 0)
            } else {
                compare(focus, point).is_ok_and(|order| order <= 0)
                    && compare(point, anchor).is_ok_and(|order| order <= 0)
            }
        } else {
            false
        };
        let old_anchor = old_anchor.filter(|anchor| {
            let dom = state.dom.borrow();
            dom.is_connected(anchor.container).unwrap_or(false)
                && compare_boundary_points(
                    &dom,
                    (anchor.container, anchor.offset),
                    (point.container, point.offset),
                )
                .is_ok()
        });
        let anchor = if shift {
            old_anchor.unwrap_or(point)
        } else {
            point
        };
        let unit = selection_movement::pointer_unit(state, point, count)?;
        let gesture = Gesture {
            target,
            anchor: unit.map_or(anchor, |range| range.0),
            control,
            deferred: inside && !shift && count == 1,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            unit,
            count,
            autoscroll_at: None,
        };
        if gesture.deferred {
            return Ok(Some(Self::completed(state, Some(gesture))));
        }
        Ok(Some(Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            gesture: Some(gesture),
            point: Some(unit.map_or(point, |range| range.1)),
            target,
            release: false,
            x,
            y,
            autoscroll: false,
            start: !control,
            event: None,
            phase: 0,
            result: None,
        }))
    }
    pub(super) fn update(
        state: &Rc<V8BindingState>,
        mut gesture: Gesture,
        target: NodeId,
        x: f64,
        y: f64,
        release: bool,
    ) -> Result<Option<Self>, V8BindingError> {
        if !live(state, gesture) {
            return Ok(None);
        }
        let target = if gesture.control {
            gesture.target
        } else {
            let dom = state.dom.borrow();
            let original = editing_host_for_node(
                &dom,
                Some(gesture.anchor.container),
                state.design_mode_enabled(),
                state.document_root(),
            )?;
            if let Some(host) = original {
                host
            } else {
                target
            }
        };
        let point = if gesture.control {
            selection_movement::pointer_caret(state, target, x, y)?
        } else {
            match selection_movement::pointer_exclusion_boundary(state, target, gesture.anchor)? {
                Some(point) => Some(point),
                None => selection_movement::pointer_caret(state, target, x, y)?,
            }
        };
        let Some(mut point) = point else {
            return Ok(Some(Self::completed(state, (!release).then_some(gesture))));
        };
        if let Some((start, end)) = gesture.unit
            && let Some((next_start, next_end)) =
                selection_movement::pointer_unit(state, point, gesture.count)?
        {
            let before = if gesture.control {
                next_start.offset < start.offset
            } else {
                compare_boundary_points(
                    &state.dom.borrow(),
                    (next_start.container, next_start.offset),
                    (start.container, start.offset),
                )
                .is_ok_and(|order| order < 0)
            };
            if before {
                gesture.anchor = end;
                point = next_start;
            } else {
                gesture.anchor = start;
                point = next_end;
            }
        }
        let start = gesture.deferred && !release && !gesture.control;
        if gesture.deferred {
            if release {
                gesture.anchor = point;
            }
            gesture.deferred = false;
        }
        if !gesture.control {
            let dom = state.dom.borrow();
            if compare_boundary_points(
                &dom,
                (gesture.anchor.container, gesture.anchor.offset),
                (point.container, point.offset),
            )
            .is_err()
            {
                return Ok(None);
            }
        }
        Ok(Some(Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            gesture: Some(gesture),
            point: Some(point),
            target,
            release,
            x,
            y,
            autoscroll: true,
            start,
            event: None,
            phase: 0,
            result: None,
        }))
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<Option<Gesture>>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            if !self.input_owner.current()
                || !self
                    .gesture
                    .is_some_and(|gesture| live(&self.owner, gesture))
            {
                self.event.take();
                self.result = Some(None);
                return Ok(self.result);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                0 => {
                    if !self.start {
                        self.phase = 2;
                        continue;
                    }
                    let event = create_event_object(scope, &self.owner, "selectstart", true, true)?;
                    let id = event_id(scope, event)?;
                    self.owner
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("selectstart event"))?
                        .is_trusted = true;
                    self.event = Some(retained_dom_event::Event::new(
                        scope,
                        &self.owner,
                        self.target,
                        "selectstart",
                        event,
                    )?);
                    self.phase = 1;
                }
                1 => {
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &self.owner)?
                    else {
                        return Ok(None);
                    };
                    self.event.take();
                    if !accepted {
                        self.result = Some(None);
                        return Ok(self.result);
                    }
                    self.phase = 2;
                }
                2 => {
                    apply(
                        scope,
                        &self.owner,
                        self.gesture.unwrap(),
                        self.point.unwrap(),
                    )?;
                    self.phase = 3;
                }
                _ => {
                    let mut gesture = self.gesture.unwrap();
                    gesture.autoscroll_at = if self.autoscroll
                        && !self.release
                        && scroll_plan(&self.owner, gesture, self.x, self.y)?.is_some()
                    {
                        Some(gesture.autoscroll_at.unwrap_or_else(|| {
                            std::time::Instant::now() + std::time::Duration::from_millis(16)
                        }))
                    } else {
                        None
                    };
                    self.result = Some((!self.release).then_some(gesture));
                    return Ok(self.result);
                }
            }
        }
    }
}
#[cfg(test)]
pub(super) fn begin(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    x: f64,
    y: f64,
    shift: bool,
    count: i32,
) -> Result<Option<Gesture>, V8BindingError> {
    let Some(mut job) = Selection::begin(state, target, x, y, shift, count)? else {
        return Ok(None);
    };
    job.poll(scope)?
        .ok_or(V8BindingError::Allocation("pointer selectstart suspended"))
}
pub(super) fn update(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    gesture: Gesture,
    target: NodeId,
    x: f64,
    y: f64,
    release: bool,
) -> Result<Option<Gesture>, V8BindingError> {
    let Some(mut job) = Selection::update(state, gesture, target, x, y, release)? else {
        return Ok(None);
    };
    job.poll(scope)?
        .ok_or(V8BindingError::Allocation("pointer selection suspended"))
}

/// Selection autoscroll is a native default at 60Hz. Distances are measured in
/// the scrollport's local plane, preserving transformed and signed scrollers.
fn edge_delta(position: f64, start: f64, length: f64) -> f64 {
    if !position.is_finite() || !start.is_finite() || !length.is_finite() || length <= 0. {
        return 0.;
    }
    let band = 16_f64.min(length / 4.);
    if position < start + band {
        -((start + band - position) / band * 8.).clamp(1., 48.)
    } else if position > start + length - band {
        ((position - (start + length - band)) / band * 8.).clamp(1., 48.)
    } else {
        0.
    }
}
fn scroll_plan(
    state: &V8BindingState,
    gesture: Gesture,
    x: f64,
    y: f64,
) -> Result<Option<SelectionHit>, V8BindingError> {
    if !live(state, gesture) || gesture.deferred {
        return Ok(None);
    }
    refresh_layout(state)?;
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut current = Some(gesture.target);
    let mut seen = BTreeSet::new();
    while let Some(node) = current {
        if !seen.insert(node) {
            break;
        }
        current = dom.flat_tree_parent(node)?;
        let root = is_scrolling_document_root(&dom, &layout, state.document_root(), node);
        let surface = if root {
            layout
                .viewport_scroll
                .as_ref()
                .map(|viewport| (viewport.scroll.scrollport, x, y))
        } else {
            layout
                .hit_projections
                .get(&node_key(node))
                .and_then(|projection| {
                    let (x, y) = projection.viewport_to_local(x, y)?;
                    Some((projection.scroll.as_ref()?.scrollport, x, y))
                })
        };
        let Some((port, x, y)) = surface else {
            continue;
        };
        let permits = |axis: &str| -> bool {
            if root {
                return layout
                    .viewport_scroll
                    .as_ref()
                    .and_then(|viewport| viewport.style.get(axis))
                    .is_none_or(|value| value != "clip");
            }
            editing_snapshot_property(&dom, &layout, node, axis)
                .ok()
                .flatten()
                .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll" | "hidden"))
        };
        let bounds = native_scroll_bounds_in_layout(&dom, &layout, state.document_root(), node);
        let previous = state
            .scroll_offsets
            .borrow()
            .get(&scroll_offset_node(state, node))
            .copied()
            .unwrap_or_default();
        let next = (
            if permits("overflow-x") {
                (previous.0 + edge_delta(x, port.x, port.width)).clamp(bounds.min.0, bounds.max.0)
            } else {
                previous.0
            },
            if permits("overflow-y") {
                (previous.1 + edge_delta(y, port.y, port.height)).clamp(bounds.min.1, bounds.max.1)
            } else {
                previous.1
            },
        );
        if next != previous {
            return Ok(Some((node, next)));
        }
    }
    Ok(None)
}

pub(super) fn autoscroll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    mut gesture: Gesture,
    x: f64,
    y: f64,
    now: std::time::Instant,
) -> Result<(Option<Gesture>, bool), V8BindingError> {
    if !live(state, gesture) {
        return Ok((None, false));
    }
    let Some((node, next)) = scroll_plan(state, gesture, x, y)? else {
        gesture.autoscroll_at = None;
        return Ok((Some(gesture), false));
    };
    perform_scroll(state, node, (Some(next.0), Some(next.1)), false)?;
    refresh_layout(state)?;
    let (width, height) = state.layout.borrow().viewport.unwrap_or_default();
    let point = (x.clamp(0., width.max(0.)), y.clamp(0., height.max(0.)));
    let target = native_pointer::hit_test(state, state.document_root(), point.0, point.1, false)?
        .first()
        .copied()
        .unwrap_or(gesture.target);
    gesture.autoscroll_at = Some(now + std::time::Duration::from_millis(16));
    Ok((update(scope, state, gesture, target, x, y, false)?, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_speed_is_bounded_and_respects_scrollport_size() {
        assert_eq!(edge_delta(50., 0., 100.), 0.);
        assert_eq!(edge_delta(-1000., 0., 100.), -48.);
        assert_eq!(edge_delta(1000., 0., 100.), 48.);
        assert_eq!(edge_delta(5., 0., 10.), 0.);
        assert_eq!(edge_delta(f64::NAN, 0., 100.), 0.);
        assert_eq!(edge_delta(0., 0., 0.), 0.);
    }

    #[test]
    fn selection_autoscroll_uses_signed_native_bounds_and_stops_on_retirement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><input id=t value=abcdef>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let key = node_key(target);
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
                        x: -100.,
                        y: 0.,
                        width: 200.,
                        height: 400.,
                    },
                },
                style: BTreeMap::from([
                    ("overflow-x".into(), "auto".into()),
                    ("overflow-y".into(), "auto".into()),
                ]),
            }),
            text_clusters: BTreeMap::from([(
                key,
                vec![oryn_common::v4::TextClusterGeometry {
                    layout_node: key,
                    start: 0,
                    end: 6,
                    rect: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 60.,
                        height: 10.,
                    },
                    matrix: [
                        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                    ],
                    line: 0,
                    rtl: false,
                }],
            )]),
            ..Default::default()
        });
        let gesture = begin(scope, &state, target, 20., 5., false, 1)
            .unwrap()
            .unwrap();
        let gesture = update(scope, &state, gesture, target, -10., 110., false)
            .unwrap()
            .unwrap();
        assert!(gesture.autoscroll_at.is_some());
        let now = std::time::Instant::now() + std::time::Duration::from_millis(20);
        let (next, changed) = autoscroll(scope, &state, gesture, -10., 110., now).unwrap();
        assert!(changed);
        let offset = state.scroll_offsets.borrow()[&root];
        assert!(offset.0 < 0. && offset.1 > 0.);
        state.scroll_offsets.borrow_mut().insert(root, (-99., 299.));
        let (next, changed) = autoscroll(scope, &state, next.unwrap(), -10., 110., now).unwrap();
        assert!(changed);
        assert_eq!(state.scroll_offsets.borrow()[&root], (-100., 300.));
        let gesture = next.unwrap();
        assert!(gesture.autoscroll_at.is_none());
        assert!(
            update(scope, &state, gesture, target, -10., 110., true)
                .unwrap()
                .is_none()
        );
        state.retire_execution();
        assert!(
            autoscroll(scope, &state, gesture, -10., 110., now)
                .unwrap()
                .0
                .is_none()
        );
    }

    #[test]
    fn control_pointer_selection_preserves_drag_sources_and_utf16_boundaries() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value='a😀b'>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let key = node_key(target);
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            text_clusters: BTreeMap::from([(
                key,
                vec![oryn_common::v4::TextClusterGeometry {
                    layout_node: key,
                    start: 0,
                    end: 4,
                    rect: oryn_common::v4::CssRect {
                        x: 0.,
                        y: 0.,
                        width: 40.,
                        height: 10.,
                    },
                    matrix: [
                        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                    ],
                    line: 0,
                    rtl: false,
                }],
            )]),
            ..Default::default()
        });
        state
            .text_control_selections
            .borrow_mut()
            .insert(target, (1, 3, "forward".into()));
        let gesture = begin(scope, &state, target, 10., 5., false, 1)
            .unwrap()
            .unwrap();
        assert!(gesture.permits_drag());
        assert_eq!(state.text_control_selections.borrow()[&target].0, 1);
        assert_eq!(state.text_control_selections.borrow()[&target].1, 3);
        update(scope, &state, gesture, target, 30., 5., true).unwrap();
        assert_eq!(state.text_control_selections.borrow()[&target].0, 3);
        assert_eq!(state.text_control_selections.borrow()[&target].1, 3);
        let gesture = begin(scope, &state, target, 0., 5., false, 1)
            .unwrap()
            .unwrap();
        assert!(!gesture.permits_drag());
        let gesture = update(scope, &state, gesture, target, 30., 5., false)
            .unwrap()
            .unwrap();
        assert!(!gesture.permits_drag()); // selecting text must not become DnD
        assert_eq!(state.text_control_selections.borrow()[&target].0, 0);
        assert_eq!(state.text_control_selections.borrow()[&target].1, 3);
        begin(scope, &state, target, 40., 5., true, 1).unwrap();
        assert_eq!(state.text_control_selections.borrow()[&target].0, 0);
        assert_eq!(state.text_control_selections.borrow()[&target].1, 4);
        let word = begin(scope, &state, target, 10., 5., false, 2)
            .unwrap()
            .unwrap();
        assert!(!word.permits_drag());
        assert_eq!(
            (
                state.text_control_selections.borrow()[&target].0,
                state.text_control_selections.borrow()[&target].1
            ),
            (1, 3)
        );
        update(scope, &state, word, target, 10., 5., true).unwrap();
        assert_eq!(
            (
                state.text_control_selections.borrow()[&target].0,
                state.text_control_selections.borrow()[&target].1
            ),
            (1, 3)
        );
        let line = begin(scope, &state, target, 10., 5., false, 3)
            .unwrap()
            .unwrap();
        update(scope, &state, line, target, 30., 5., true).unwrap();
        assert_eq!(
            (
                state.text_control_selections.borrow()[&target].0,
                state.text_control_selections.borrow()[&target].1
            ),
            (0, 4)
        );
        state.retire_execution();
        assert!(
            update(scope, &state, gesture, target, 0., 5., false)
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod retained_selection_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn selectstart_suffix_resumes_once_and_adoption_revokes_default() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=t contenteditable>abcd</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(253), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let text = state.dom.borrow().children(target).unwrap()[0];
        eval(
            scope,
            "globalThis.trace=[];t.addEventListener('selectstart',()=>trace.push('once'),{once:true});t.onselectstart=()=>trace.push('handler');",
        );
        let gesture = Gesture {
            target,
            anchor: BoundaryPoint {
                container: text,
                offset: 0,
            },
            control: false,
            deferred: false,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            unit: None,
            count: 1,
            autoscroll_at: None,
        };
        let create = || Selection {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(&state),
            gesture: Some(gesture),
            point: Some(BoundaryPoint {
                container: text,
                offset: 2,
            }),
            target,
            release: false,
            x: 0.,
            y: 0.,
            autoscroll: false,
            start: true,
            event: None,
            phase: 0,
            result: None,
        };
        let mut job = create();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(state.selection_anchor.get().is_none());
        assert!(job.poll(scope).unwrap().unwrap().is_some());
        assert!(job.poll(scope).unwrap().unwrap().is_some());
        eval(
            scope,
            "if(trace.join(',')!=='once,handler'||getSelection().toString()!=='ab')throw Error('selection replay/order');getSelection().removeAllRanges();",
        );
        let mut adopted = create();
        native_platform::publication::fail_after_next_callback();
        assert!(adopted.poll(scope).is_err());
        eval(
            scope,
            "globalThis.other=document.implementation.createHTMLDocument('other');other.body.appendChild(other.adoptNode(t));",
        );
        assert!(adopted.poll(scope).unwrap().unwrap().is_none());
        assert!(state.selection_anchor.get().is_none());
    }
}
