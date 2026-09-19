//! One native pointer sample owns every event and default-action continuation.
use super::*;
mod allocation;
mod event;
mod touch;
use event::{Capture, Event};

pub(super) struct Range {
    owner: Rc<V8BindingState>,
    input_owner: InputOwner,
    node: NodeId,
    x: f64,
    y: f64,
    finish: bool,
    phase: u8,
    event: Option<Box<Event>>,
    result: Option<bool>,
}
impl Range {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        node: NodeId,
        x: f64,
        y: f64,
        finish: bool,
    ) -> Self {
        Self {
            owner: state.clone(),
            input_owner: InputOwner::new(state),
            node,
            x,
            y,
            finish,
            phase: 0,
            event: None,
            result: None,
        }
    }
    #[inline(never)]
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            if !self.input_owner.current() {
                self.event.take();
                self.result = Some(false);
                return Ok(self.result);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                0 => {
                    let Some(changed) =
                        prepare_range_input(&self.owner, self.node, self.x, self.y)?
                    else {
                        self.result = Some(false);
                        return Ok(self.result);
                    };
                    self.phase = if changed { 1 } else { 3 };
                }
                1 => {
                    self.owner.layout_cache_key.borrow_mut().take();
                    let object = create_event_object(scope, &self.owner, "input", true, false)?;
                    let id = event_id(scope, object)?;
                    if let Some(record) = self.owner.events.borrow_mut().get_mut(&id) {
                        record.is_trusted = true;
                        record.composed = true;
                    }
                    self.event = Some(allocation::try_boxed(|| {
                        Event::from_object(scope, &self.owner, self.node, "input", object)
                    })?);
                    self.phase = 2;
                }
                2 => {
                    if self.event.as_mut().unwrap().poll(scope)?.is_none() {
                        return Ok(None);
                    }
                    self.event.take();
                    self.phase = 3;
                }
                3 => {
                    if !self.finish || !self.owner.dom.borrow().is_connected(self.node)? {
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    // Prepare all fallible V8 event state before consuming change.
                    let object = create_event_object(scope, &self.owner, "change", true, false)?;
                    let id = event_id(scope, object)?;
                    self.owner
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("range change event"))?
                        .is_trusted = true;
                    let event = allocation::try_boxed(|| {
                        Event::from_object(scope, &self.owner, self.node, "change", object)
                    })?;
                    if !self.owner.dom.borrow_mut().take_control_change(self.node)? {
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    self.event = Some(event);
                    self.phase = 4;
                }
                _ => {
                    if self.event.as_mut().unwrap().poll(scope)?.is_none() {
                        return Ok(None);
                    }
                    self.event.take();
                    self.result = Some(true);
                    return Ok(self.result);
                }
            }
        }
    }
}

fn pointer_focus(
    state: &Rc<V8BindingState>,
    target: NodeId,
) -> Result<Option<retained_focus::Transition>, V8BindingError> {
    let node = {
        let dom = state.dom.borrow();
        if dom.is_inert(target)? {
            return Ok(None);
        }
        let mut current = Some(target);
        let mut found = None;
        while let Some(node) = current {
            current = dom.parent(node)?;
            if !matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. }) {
                continue;
            }
            let name = element_local_name(&dom, node);
            let control = matches!(
                name,
                Some("button" | "select" | "textarea" | "iframe" | "summary")
            ) || (name == Some("input")
                && !dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|value| value.eq_ignore_ascii_case("hidden")))
                || (matches!(name, Some("a" | "area"))
                    && dom.attribute_in_null_namespace(node, "href")?.is_some());
            if !dom.control_disabled(node)?
                && (dom.parsed_tab_index(node)?.is_some()
                    || control
                    || editing_host_for_node(
                        &dom,
                        Some(node),
                        state.design_mode_enabled(),
                        state.document_root(),
                    )? == Some(node))
            {
                found = Some(node);
                break;
            }
        }
        found
    };
    let Some(node) = node else { return Ok(None) };
    let Some(node) = focus_navigation::delegated_target(state, node)? else {
        return Ok(None);
    };
    if css_suppresses_focus(state, node)? {
        return Ok(None);
    }
    let job = retained_focus::Transition::new(state, Some(node))?;
    if state.dom.borrow().focused() != Some(node) {
        state.dom.borrow_mut().set_focus_visible(false);
    }
    Ok(Some(job))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepare,
    Prompt,
    TouchGesture,
    Scrollbar,
    DragBegin,
    Drag,
    Capture,
    Target,
    Boundary,
    Pointer,
    InterestAdmit,
    InterestPoll,
    TopLayer,
    TouchEvent,
    TouchRangeEvent,
    TouchRangeDefault,
    TouchRangeFocus,
    TouchRangeStore,
    Pinch,
    CompatMove,
    CompatDown,
    CompatFocus,
    CompatRange,
    CompatUp,
    Mouse,
    MouseFocus,
    MouseRange,
    MouseSelection,
    MouseStore,
    MoveRange,
    MoveSelection,
    Context,
    ReleaseCapture,
    ClickPrepare,
    Click,
    Permission,
    Dblclick,
    Cleanup,
    ExitBoundary,
    Done,
}
/// An immutable physical sample owns its event/default suffix until completion.
/// Continuations have independent heap custody: an input sample must not inline
/// every nested event, drag, touch and focus state into each native call frame.
pub struct NativePointerDispatch {
    state: Rc<V8BindingState>,
    owner: InputOwner,
    claim: Rc<()>,
    input: NativePointerInput,
    action: NativePointerAction,
    phase: Phase,
    scrollbar: Option<(Option<NodeId>, Option<(ScrollbarGesture, f64)>)>,
    hit: Option<NodeId>,
    target: Option<NodeId>,
    previous: Option<NodeId>,
    previous_buttons: u32,
    up_capture: Option<NodeId>,
    allowed: bool,
    prompt: Option<Box<permission_prompt::Pointer>>,
    event: Option<Box<Event>>,
    capture: Option<Box<Capture>>,
    boundary_started: bool,
    focus: Option<Box<retained_focus::Transition>>,
    focus_started: bool,
    layer: Option<Box<top_layer::Pointer>>,
    range: Option<Box<Range>>,
    range_target: Option<NodeId>,
    range_result: bool,
    selection: Option<Box<native_pointer_selection::Selection>>,
    selection_started: bool,
    selection_result: Option<native_pointer_selection::Gesture>,
    drag_begin: Option<Box<NativePointerDragAdmission>>,
    drag: Option<Box<native_drag::NativeDragInput>>,
    touch: Option<Box<touch::Touch>>,
    mouse: bool,
    mouse_allowed: bool,
    compatibility: NativePointerInput,
    click: Option<NodeId>,
    click_count: i32,
    activation_target: Option<v8::Global<v8::Object>>,
    result: Option<(NodeId, bool)>,
    canceled: bool,
}
impl NativePointerDispatch {
    pub fn new(
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
        action: NativePointerAction,
    ) -> Result<Self, V8BindingError> {
        input.validate()?;
        let owner = InputOwner::new(state);
        if !owner.current() {
            return Err(V8BindingError::Allocation(
                "retired pointer dispatch document",
            ));
        }
        let claim = Rc::new(());
        {
            let mut registry = state.native_pointers.borrow_mut();
            let pointer = registry.pointers.entry(input.id).or_default();
            if pointer.boundary_claim.is_some() {
                return Err(V8BindingError::Allocation(
                    "pointer dispatch lane already owned",
                ));
            }
            pointer.boundary_claim = Some(claim.clone());
        }
        Ok(Self {
            state: state.clone(),
            owner,
            claim,
            input: input.clone(),
            action,
            phase: Phase::Prepare,
            scrollbar: None,
            hit: None,
            target: None,
            previous: None,
            previous_buttons: 0,
            up_capture: None,
            allowed: true,
            prompt: None,
            event: None,
            capture: None,
            boundary_started: false,
            focus: None,
            focus_started: false,
            layer: None,
            range: None,
            range_target: None,
            range_result: false,
            selection: None,
            selection_started: false,
            selection_result: None,
            drag_begin: None,
            drag: None,
            touch: None,
            mouse: false,
            mouse_allowed: false,
            compatibility: input.clone(),
            click: None,
            click_count: 0,
            activation_target: None,
            result: None,
            canceled: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.state.clone()
    }
    pub fn canceled(&self) -> bool {
        self.canceled
    }
    fn release_claim(&self) {
        let mut registry = self.state.native_pointers.borrow_mut();
        if let Some(pointer) = registry.pointers.get_mut(&self.input.id) {
            if pointer
                .boundary_claim
                .as_ref()
                .is_some_and(|claim| Rc::ptr_eq(claim, &self.claim))
            {
                pointer.boundary_claim = None;
            }
        }
    }

    pub fn retry_safe(&self) -> bool {
        !self.canceled
            && self.phase != Phase::Done
            && self.owner.current()
            && self.event.as_ref().is_none_or(|event| event.retry_safe())
    }
    pub fn cancel(&mut self) {
        if self.phase == Phase::Done {
            return;
        }
        if self.boundary_started && self.owner.current() {
            if let Some(pointer) = self
                .state
                .native_pointers
                .borrow_mut()
                .pointers
                .get_mut(&self.input.id)
            {
                pointer.boundary_pending.take();
                pointer.boundary_retry = None;
            }
        }
        self.prompt.take();
        self.event.take();
        self.capture.take();
        self.focus.take();
        self.layer.take();
        self.range.take();
        self.selection.take();
        self.drag_begin.take();
        if let Some(drag) = &mut self.drag {
            drag.cancel()
        }
        self.drag.take();
        self.touch.take();
        self.release_claim();
        self.canceled = true;
        self.phase = Phase::Done;
        self.result = Some((
            self.target
                .or(self.hit)
                .unwrap_or(self.state.document_root()),
            false,
        ));
    }
    fn done(&mut self, target: NodeId, allowed: bool) {
        if !self.owner.current() {
            self.cancel();
            return;
        }
        self.release_claim();
        self.result = Some((target, allowed));
        self.phase = Phase::Done;
    }
    fn event(
        &mut self,
        scope: &mut v8::PinScope,
        input: &NativePointerInput,
        target: NodeId,
        name: &str,
        cancelable: bool,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.event.is_none() {
            if !self.state.dom.borrow().is_connected(target)? {
                return Ok(Some(false));
            }
            self.event = Some(allocation::try_boxed(|| {
                Event::new(
                    scope,
                    &self.state,
                    input,
                    target,
                    name,
                    true,
                    cancelable,
                    None,
                )
            })?);
        }
        let result = self.event.as_mut().unwrap().poll(scope)?;
        if result.is_some() {
            self.event.take();
        }
        Ok(result)
    }
    fn touch_event(
        &mut self,
        scope: &mut v8::PinScope,
        name: &str,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.event.is_none() {
            let Some((target, object)) =
                prepare_touch_event(scope, &self.state, &self.input, name)?
            else {
                return Ok(Some(true));
            };
            self.event = Some(allocation::try_boxed(|| {
                Event::from_object(scope, &self.state, target, name, object)
            })?);
        }
        let result = self.event.as_mut().unwrap().poll(scope)?;
        if result.is_some() {
            self.event.take();
        }
        Ok(result)
    }
    fn focus(&mut self, scope: &mut v8::PinScope, target: NodeId) -> Result<bool, V8BindingError> {
        if !self.focus_started {
            self.focus = allocation::try_optional(|| pointer_focus(&self.state, target))?;
            self.focus_started = true;
        }
        if let Some(focus) = &mut self.focus {
            if !focus.poll(scope, &self.state)? {
                return Ok(false);
            }
        }
        self.focus.take();
        self.focus_started = false;
        Ok(true)
    }
    fn boundary(
        &mut self,
        scope: &mut v8::PinScope,
        old: Option<NodeId>,
        new: Option<NodeId>,
    ) -> Result<(), V8BindingError> {
        if !self.boundary_started {
            boundary_dispatch::drain_owned(scope, &self.state, self.input.id)?;
            let result =
                boundary_dispatch::dispatch_owned(scope, &self.state, &self.input, old, new);
            self.boundary_started = self
                .state
                .native_pointers
                .borrow()
                .pointers
                .get(&self.input.id)
                .is_some_and(|pointer| pointer.boundary_pending.is_some());
            result?;
        } else {
            boundary_dispatch::drain_owned(scope, &self.state, self.input.id)?;
        }
        self.boundary_started = false;
        Ok(())
    }
    #[inline(never)]
    pub fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<(NodeId, bool)>, V8BindingError> {
        loop {
            if self.phase == Phase::Done {
                return Ok(self.result);
            }
            if !self.owner.current() {
                self.cancel();
                return Ok(self.result);
            }
            let state = self.state.clone();
            let input = self.input.clone();
            let action = self.action;
            if state.execution_suspended()
                || scope.is_execution_terminating()
                || state.native_pointers.borrow().boundary_draining
            {
                return Ok(None);
            }
            match self.phase {
                Phase::Prepare => {
                    self.hit = hit_test(&state, state.document_root(), input.x, input.y, false)?
                        .first()
                        .copied();
                    if input.device == NativePointerDevice::Touch
                        && action == NativePointerAction::Move
                        && input.buttons == 0
                    {
                        self.done(self.hit.unwrap_or(state.document_root()), true);
                        continue;
                    }
                    let policy = if input.device == NativePointerDevice::Touch
                        && action == NativePointerAction::Down
                    {
                        self.hit
                            .map(|target| native_touch::policy(&state, target))
                            .transpose()?
                    } else {
                        None
                    };
                    let contact = policy.and_then(|policy| {
                        self.hit.map(|target| {
                            native_touch::Contact::new(&state, target, input.x, input.y, policy)
                        })
                    });
                    {
                        let mut registry = state.native_pointers.borrow_mut();
                        let pointer = registry.pointers.entry(input.id).or_default();
                        self.previous_buttons = pointer.buttons;
                        pointer.buttons = input.buttons;
                        pointer.last_input = Some(input.clone());
                        if input.device == NativePointerDevice::Touch
                            && action == NativePointerAction::Down
                        {
                            pointer.touch_policy = policy;
                            pointer.touch_contact = contact;
                            pointer.touch_suppressed = false;
                            pointer.touch_zooming = false;
                        }
                    }
                    self.phase = Phase::Prompt;
                }
                Phase::Prompt => {
                    if self.prompt.is_none() {
                        self.prompt = Some(allocation::boxed(|| {
                            permission_prompt::Pointer::new(&state, &input, action)
                        }));
                    }
                    let Some(result) = self.prompt.as_mut().unwrap().poll(scope)? else {
                        return Ok(None);
                    };
                    self.prompt.take();
                    if let Some(target) = result {
                        speculation_registry::clear_interest(&state);
                        self.done(target, true)
                    } else {
                        self.phase = Phase::TouchGesture
                    }
                }
                Phase::TouchGesture => {
                    if self.touch.is_none() {
                        self.touch = Some(allocation::boxed(|| {
                            touch::Touch::new(&state, &input, action)
                        }));
                    }
                    let Some(result) = self.touch.as_mut().unwrap().poll(scope)? else {
                        return Ok(None);
                    };
                    self.touch.take();
                    if let Some(target) = result {
                        self.done(target, true)
                    } else {
                        self.phase = Phase::Scrollbar
                    }
                }
                Phase::Scrollbar => {
                    if self.scrollbar.is_none() {
                        self.scrollbar =
                            Some(prepare_scrollbar_input(&state, &input, action, self.hit)?);
                    }
                    let (target, pending) = *self.scrollbar.as_ref().unwrap();
                    if let Some((gesture, next)) = pending {
                        commit_scrollbar_offset(&state, gesture, next)?;
                    }
                    if let Some(target) = target {
                        speculation_registry::clear_interest(&state);
                        self.done(target, true)
                    } else {
                        self.phase = Phase::DragBegin
                    }
                }
                Phase::DragBegin => {
                    let dragging = state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .is_some_and(|pointer| pointer.drag_consumed);
                    if !dragging && action == NativePointerAction::Move {
                        if self.drag_begin.is_none() {
                            self.drag_begin = Some(allocation::try_boxed(|| {
                                NativePointerDragAdmission::new(&state, &input)
                            })?);
                        }
                        let Some(_) = self.drag_begin.as_mut().unwrap().poll(scope)? else {
                            return Ok(None);
                        };
                        self.drag_begin.take();
                    }
                    self.phase = Phase::Drag;
                }
                Phase::Drag => {
                    if state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .is_some_and(|pointer| pointer.drag_consumed)
                    {
                        if self.drag.is_none() {
                            let init = crate::data_transfer::EventInit {
                                client_x: input.x as i32,
                                client_y: input.y as i32,
                                screen_x: input.screen_x as i32,
                                screen_y: input.screen_y as i32,
                                button: input.button as i16,
                                buttons: input.buttons as u16,
                                ctrl_key: input.control,
                                alt_key: input.alt,
                                shift_key: input.shift,
                                meta_key: input.meta,
                                ..Default::default()
                            };
                            let operation = match action {
                                NativePointerAction::Up => "drop",
                                NativePointerAction::Cancel => "cancel",
                                _ => "advance",
                            };
                            self.drag = Some(allocation::try_boxed(|| {
                                native_drag::NativeDragInput::new(
                                    &state, operation, self.hit, &init,
                                )
                            })?);
                        }
                        if self.drag.as_mut().unwrap().poll(scope)?.is_none() {
                            return Ok(None);
                        }
                        self.drag.take();
                        if !self.owner.current() {
                            continue;
                        }
                        let target = self
                            .hit
                            .or_else(|| {
                                state
                                    .native_pointers
                                    .borrow()
                                    .pointers
                                    .get(&input.id)
                                    .and_then(|pointer| pointer.drag_start.map(|entry| entry.0))
                            })
                            .unwrap_or(state.document_root());
                        finish_native_pointer_drag_transition(
                            &state,
                            &input,
                            matches!(
                                action,
                                NativePointerAction::Up | NativePointerAction::Cancel
                            ),
                        );
                        speculation_registry::clear_interest(&state);
                        self.done(target, true);
                    } else {
                        self.phase = Phase::Capture
                    }
                }
                Phase::Capture | Phase::ReleaseCapture => {
                    if self.capture.is_none() {
                        if self.phase == Phase::ReleaseCapture {
                            if let Some(pointer) = state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .get_mut(&input.id)
                            {
                                pointer.buttons = 0;
                                pointer.pending_capture = None;
                            }
                        }
                        self.capture = Some(allocation::boxed(|| Capture::new(&state, &input)));
                    }
                    if !self.capture.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.capture.take();
                    self.phase = if self.phase == Phase::Capture {
                        Phase::Target
                    } else {
                        Phase::ClickPrepare
                    };
                }
                Phase::Target => {
                    {
                        let registry = state.native_pointers.borrow();
                        let pointer = registry
                            .pointers
                            .get(&input.id)
                            .ok_or(V8BindingError::Allocation("pointer source missing"))?;
                        self.target = pointer.capture.or(self.hit).or(pointer.target);
                        self.previous = pointer.target;
                    }
                    speculation_registry::observe_native_pointer(&state, self.hit, &input, action)?;
                    if self.target.is_none() {
                        return Err(V8BindingError::Allocation("pointer target unavailable"));
                    }
                    self.phase = Phase::Boundary;
                }
                Phase::Boundary => {
                    if action != NativePointerAction::Cancel {
                        self.boundary(scope, self.previous, self.target)?;
                    }
                    if !self.owner.current() {
                        continue;
                    }
                    let target = self.target.unwrap();
                    {
                        let mut registry = state.native_pointers.borrow_mut();
                        let pointer = registry
                            .pointers
                            .get_mut(&input.id)
                            .ok_or(V8BindingError::Allocation("pointer source missing"))?;
                        pointer.target = Some(target);
                        if action == NativePointerAction::Down {
                            pointer.down_targets.insert(input.button, target);
                            let count = pointer.click_history.as_ref().map_or(1, |history| {
                                history.next(target, &input, std::time::Instant::now())
                            });
                            pointer.click_details.insert(input.button, count);
                            if input.device == NativePointerDevice::Touch {
                                pointer.pending_capture = Some(target);
                            }
                        }
                        if action == NativePointerAction::Up {
                            self.up_capture = pointer.capture;
                        }
                    }
                    if (action == NativePointerAction::Down
                        && input.device == NativePointerDevice::Mouse)
                        || (action == NativePointerAction::Up
                            && input.device != NativePointerDevice::Mouse)
                    {
                        state.notify_user_activation();
                    }
                    self.phase = Phase::Pointer;
                }
                Phase::Pointer => {
                    let name = match action {
                        NativePointerAction::Down if self.previous_buttons == 0 => "pointerdown",
                        NativePointerAction::Up if input.buttons == 0 => "pointerup",
                        NativePointerAction::Cancel => "pointercancel",
                        _ => "pointermove",
                    };
                    let Some(accepted) = self.event(
                        scope,
                        &input,
                        self.target.unwrap(),
                        name,
                        action != NativePointerAction::Cancel,
                    )?
                    else {
                        return Ok(None);
                    };
                    self.allowed = accepted;
                    if name == "pointerdown" && !accepted && self.owner.current() {
                        if let Some(pointer) = state
                            .native_pointers
                            .borrow_mut()
                            .pointers
                            .get_mut(&input.id)
                        {
                            pointer.prevent_mouse = true;
                        }
                    }
                    self.phase = Phase::InterestAdmit;
                }
                Phase::InterestAdmit => {
                    if input.primary && input.button == 0 && action == NativePointerAction::Down {
                        interest::admit_pointer_down(
                            &state,
                            input.id,
                            self.target.unwrap(),
                            input.x,
                            input.y,
                        )?;
                        self.phase = Phase::InterestPoll
                    } else {
                        self.phase = Phase::TopLayer
                    }
                }
                Phase::InterestPoll => {
                    interest::poll(scope, &state)?;
                    self.phase = Phase::TopLayer
                }
                Phase::TopLayer => {
                    if input.primary
                        && input.button == 0
                        && matches!(action, NativePointerAction::Down | NativePointerAction::Up)
                    {
                        if self.layer.is_none() {
                            self.layer = Some(allocation::boxed(|| {
                                top_layer::Pointer::new(
                                    &state,
                                    self.target.unwrap(),
                                    action == NativePointerAction::Down,
                                )
                            }));
                        }
                        if !self.layer.as_mut().unwrap().poll(scope)? {
                            return Ok(None);
                        }
                        self.layer.take();
                    }
                    if action == NativePointerAction::Cancel {
                        interest::cancel_pointer(&state, input.id);
                        state.dom.borrow_mut().take_popover_pointer_down();
                    }
                    self.phase = Phase::TouchEvent;
                }
                Phase::TouchEvent => {
                    if input.device == NativePointerDevice::Touch
                        && action != NativePointerAction::Move
                    {
                        let name = match action {
                            NativePointerAction::Down => "touchstart",
                            NativePointerAction::Up => "touchend",
                            _ => "touchcancel",
                        };
                        let Some(accepted) = self.touch_event(scope, name)? else {
                            return Ok(None);
                        };
                        if !accepted && self.owner.current() {
                            if let Some(pointer) = state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .get_mut(&input.id)
                            {
                                pointer.prevent_mouse = true;
                                pointer.down_targets.clear();
                                if let Some(contact) = pointer.touch_contact.as_mut() {
                                    contact.prevent_default();
                                }
                            }
                        }
                    }
                    self.range_target =
                        if input.device == NativePointerDevice::Touch && input.primary {
                            state
                                .native_pointers
                                .borrow()
                                .pointers
                                .get(&input.id)
                                .and_then(|pointer| pointer.range)
                        } else {
                            None
                        };
                    self.mouse_allowed = true;
                    self.phase = Phase::TouchRangeEvent;
                }
                Phase::TouchRangeEvent => {
                    if self.range_target.is_some() && action == NativePointerAction::Move {
                        let Some(accepted) = self.touch_event(scope, "touchmove")? else {
                            return Ok(None);
                        };
                        self.mouse_allowed = accepted;
                    }
                    self.phase = Phase::TouchRangeDefault;
                }
                Phase::TouchRangeDefault => {
                    let mut start = false;
                    let node = if let Some(node) = self.range_target {
                        if action == NativePointerAction::Cancel
                            || self.mouse_allowed
                                && self.allowed
                                && matches!(
                                    action,
                                    NativePointerAction::Move | NativePointerAction::Up
                                )
                        {
                            Some(node)
                        } else {
                            None
                        }
                    } else if input.device == NativePointerDevice::Touch
                        && input.primary
                        && action == NativePointerAction::Down
                        && self.allowed
                        && !state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .is_some_and(|pointer| pointer.prevent_mouse)
                    {
                        start = true;
                        self.target
                    } else {
                        None
                    };
                    if let Some(node) = node {
                        if self.range.is_none() {
                            let mut job = Range::new(
                                &state,
                                node,
                                input.x,
                                input.y,
                                action != NativePointerAction::Down
                                    && action != NativePointerAction::Move,
                            );
                            if action == NativePointerAction::Cancel {
                                job.phase = 3;
                            }
                            self.range = Some(allocation::boxed(|| job));
                        }
                        let Some(result) = self.range.as_mut().unwrap().poll(scope)? else {
                            return Ok(None);
                        };
                        self.range.take();
                        self.range_result = result;
                        if start && result {
                            self.phase = Phase::TouchRangeFocus;
                            continue;
                        }
                        if !result
                            || matches!(
                                action,
                                NativePointerAction::Up | NativePointerAction::Cancel
                            )
                        {
                            if let Some(pointer) = state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .get_mut(&input.id)
                            {
                                pointer.range = None;
                            }
                        }
                    }
                    self.phase = Phase::Pinch;
                }
                Phase::TouchRangeFocus => {
                    if !self.focus(scope, self.target.unwrap())? {
                        return Ok(None);
                    }
                    self.phase = Phase::TouchRangeStore
                }
                Phase::TouchRangeStore => {
                    if let Some(pointer) = state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&input.id)
                    {
                        pointer.range = self.target;
                        if let Some(contact) = pointer.touch_contact.as_mut() {
                            contact.prevent_default();
                        }
                        pointer.touch_policy = None;
                        pointer.drag_start = None;
                    }
                    self.phase = Phase::Pinch
                }
                Phase::Pinch => {
                    if input.device == NativePointerDevice::Touch
                        && action == NativePointerAction::Down
                        && !state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .is_some_and(|pointer| pointer.range.is_some())
                    {
                        prepare_pinch(scope, &state);
                    }
                    self.mouse = input.primary
                        && !state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .is_some_and(|pointer| pointer.prevent_mouse);
                    if self.mouse
                        && input.device == NativePointerDevice::Touch
                        && action == NativePointerAction::Up
                    {
                        self.compatibility = input.clone();
                        self.compatibility.button = 0;
                        self.compatibility.buttons = 0;
                        self.compatibility.pressure = 0.;
                        self.phase = Phase::CompatMove
                    } else {
                        self.phase = Phase::Mouse
                    }
                }
                Phase::CompatMove => {
                    let compatibility = self.compatibility.clone();
                    if self
                        .event(
                            scope,
                            &compatibility,
                            self.target.unwrap(),
                            "mousemove",
                            true,
                        )?
                        .is_none()
                    {
                        return Ok(None);
                    }
                    self.compatibility.buttons = 1;
                    self.phase = Phase::CompatDown
                }
                Phase::CompatDown => {
                    let compatibility = self.compatibility.clone();
                    let Some(accepted) = self.event(
                        scope,
                        &compatibility,
                        self.target.unwrap(),
                        "mousedown",
                        true,
                    )?
                    else {
                        return Ok(None);
                    };
                    self.phase = if accepted {
                        Phase::CompatFocus
                    } else {
                        Phase::CompatUp
                    };
                }
                Phase::CompatFocus => {
                    if !self.focus(scope, self.target.unwrap())? {
                        return Ok(None);
                    }
                    self.phase = Phase::CompatRange
                }
                Phase::CompatRange => {
                    if self.range.is_none() {
                        self.range = Some(allocation::boxed(|| {
                            Range::new(&state, self.target.unwrap(), input.x, input.y, true)
                        }));
                    }
                    if self.range.as_mut().unwrap().poll(scope)?.is_none() {
                        return Ok(None);
                    }
                    self.range.take();
                    self.phase = Phase::CompatUp
                }
                Phase::CompatUp => {
                    self.compatibility.buttons = 0;
                    let compatibility = self.compatibility.clone();
                    if self
                        .event(scope, &compatibility, self.target.unwrap(), "mouseup", true)?
                        .is_none()
                    {
                        return Ok(None);
                    }
                    self.phase = Phase::Context
                }
                Phase::Mouse => {
                    if !self.mouse
                        || input.device == NativePointerDevice::Touch
                        || action == NativePointerAction::Cancel
                    {
                        self.phase = Phase::Context;
                        continue;
                    }
                    let name = match action {
                        NativePointerAction::Down => "mousedown",
                        NativePointerAction::Up => "mouseup",
                        _ => "mousemove",
                    };
                    let Some(accepted) =
                        self.event(scope, &input, self.target.unwrap(), name, true)?
                    else {
                        return Ok(None);
                    };
                    self.mouse_allowed = accepted;
                    self.phase =
                        if accepted && action == NativePointerAction::Down && input.button == 0 {
                            Phase::MouseFocus
                        } else {
                            Phase::MoveRange
                        };
                }
                Phase::MouseFocus => {
                    if !self.focus(scope, self.target.unwrap())? {
                        return Ok(None);
                    }
                    self.phase = Phase::MouseRange
                }
                Phase::MouseRange => {
                    if self.range.is_none() {
                        self.range = Some(allocation::boxed(|| {
                            Range::new(&state, self.target.unwrap(), input.x, input.y, false)
                        }));
                    }
                    let Some(result) = self.range.as_mut().unwrap().poll(scope)? else {
                        return Ok(None);
                    };
                    self.range.take();
                    self.range_result = result;
                    self.phase = Phase::MouseSelection
                }
                Phase::MouseSelection => {
                    if !self.range_result {
                        if !self.selection_started {
                            let count = state
                                .native_pointers
                                .borrow()
                                .pointers
                                .get(&input.id)
                                .and_then(|pointer| pointer.click_details.get(&input.button))
                                .copied()
                                .unwrap_or(1);
                            self.selection = allocation::try_optional(|| {
                                native_pointer_selection::Selection::begin(
                                    &state,
                                    self.target.unwrap(),
                                    input.x,
                                    input.y,
                                    input.shift,
                                    count,
                                )
                            })?;
                            self.selection_started = true;
                        }
                        if let Some(selection) = &mut self.selection {
                            let Some(result) = selection.poll(scope)? else {
                                return Ok(None);
                            };
                            self.selection_result = result;
                        }
                        self.selection.take();
                    }
                    self.phase = Phase::MouseStore;
                }
                Phase::MouseStore => {
                    if let Some(pointer) = state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&input.id)
                    {
                        pointer.range = self.range_result.then_some(self.target.unwrap());
                        pointer.drag_start = if self.range_result {
                            None
                        } else {
                            Some((self.target.unwrap(), input.x, input.y))
                        };
                        pointer.selection = self.selection_result;
                    }
                    self.phase = Phase::Context
                }
                Phase::MoveRange => {
                    if !self.mouse_allowed
                        || !(action == NativePointerAction::Move && input.buttons & 1 != 0
                            || action == NativePointerAction::Up && input.button == 0)
                    {
                        self.phase = Phase::Context;
                        continue;
                    }
                    let range = state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .and_then(|pointer| pointer.range);
                    if let Some(node) = range {
                        if self.range.is_none() {
                            self.range = Some(allocation::boxed(|| {
                                Range::new(
                                    &state,
                                    node,
                                    input.x,
                                    input.y,
                                    action == NativePointerAction::Up,
                                )
                            }));
                        }
                        let Some(result) = self.range.as_mut().unwrap().poll(scope)? else {
                            return Ok(None);
                        };
                        self.range.take();
                        if !result || action == NativePointerAction::Up {
                            if let Some(pointer) = state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .get_mut(&input.id)
                            {
                                pointer.range = None;
                            }
                        }
                    }
                    self.phase = Phase::MoveSelection;
                }
                Phase::MoveSelection => {
                    if !self.selection_started {
                        let gesture = state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .and_then(|pointer| pointer.selection);
                        self.selection = allocation::try_optional(|| {
                            gesture
                                .map(|gesture| {
                                    native_pointer_selection::Selection::update(
                                        &state,
                                        gesture,
                                        self.target.unwrap(),
                                        input.x,
                                        input.y,
                                        action == NativePointerAction::Up,
                                    )
                                })
                                .transpose()
                                .map(Option::flatten)
                        })?;
                        self.selection_started = true;
                    }
                    if let Some(selection) = &mut self.selection {
                        let Some(result) = selection.poll(scope)? else {
                            return Ok(None);
                        };
                        self.selection_result = result;
                        self.selection.take();
                    }
                    if self.owner.current() {
                        if let Some(pointer) = state
                            .native_pointers
                            .borrow_mut()
                            .pointers
                            .get_mut(&input.id)
                        {
                            pointer.selection = self.selection_result;
                        }
                    }
                    self.phase = Phase::Context;
                }
                Phase::Context => {
                    let when = if cfg!(target_os = "windows") {
                        NativePointerAction::Up
                    } else {
                        NativePointerAction::Down
                    };
                    if input.primary && input.button == 2 && action == when {
                        if self
                            .event(scope, &input, self.target.unwrap(), "contextmenu", true)?
                            .is_none()
                        {
                            return Ok(None);
                        }
                    }
                    self.phase = if action == NativePointerAction::Cancel
                        || action == NativePointerAction::Up && input.buttons == 0
                    {
                        Phase::ReleaseCapture
                    } else {
                        Phase::ClickPrepare
                    };
                }
                Phase::ClickPrepare => {
                    if action == NativePointerAction::Up
                        && (input.device != NativePointerDevice::Touch || input.primary)
                    {
                        let down = state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .and_then(|pointer| pointer.down_targets.get(&input.button))
                            .copied();
                        let click = if down.is_some() && self.up_capture.is_some() {
                            self.up_capture
                        } else {
                            state.dom.borrow().pointer_click_target(down, self.target)?
                        };
                        if let Some(click) = click {
                            if !state.dom.borrow().control_disabled(click)? {
                                let wrapper = wrap_node(scope, &state, click)?;
                                self.activation_target = Some(v8::Global::new(scope, wrapper));
                                self.click = Some(click);
                            }
                        }
                        if let Some(pointer) = state
                            .native_pointers
                            .borrow_mut()
                            .pointers
                            .get_mut(&input.id)
                        {
                            pointer.down_targets.remove(&input.button);
                            if let Some(click) = self.click {
                                self.click_count = pointer
                                    .click_details
                                    .get(&input.button)
                                    .copied()
                                    .unwrap_or(1);
                                pointer.click_history = Some(ClickHistory {
                                    target: click,
                                    button: input.button,
                                    x: input.x,
                                    y: input.y,
                                    at: std::time::Instant::now(),
                                    count: self.click_count,
                                });
                            }
                        }
                    }
                    self.phase = Phase::Click;
                }
                Phase::Click => {
                    if let Some(click) = self.click {
                        let Some(accepted) = self.event(
                            scope,
                            &input,
                            click,
                            if input.button == 0 {
                                "click"
                            } else {
                                "auxclick"
                            },
                            true,
                        )?
                        else {
                            return Ok(None);
                        };
                        self.mouse_allowed = accepted;
                    } else {
                        self.mouse_allowed = false
                    }
                    self.phase = Phase::Permission
                }
                Phase::Permission => {
                    if self.mouse_allowed && input.button == 0 {
                        let object =
                            v8::Local::new(scope, self.activation_target.as_ref().unwrap());
                        let (owner, node) = node_identity::argument(scope, object.into())?
                            .ok_or(V8BindingError::Allocation("pointer activation owner"))?;
                        permission_elements::activate(&owner, node, true)?;
                    }
                    self.phase = Phase::Dblclick
                }
                Phase::Dblclick => {
                    if input.button == 0 && self.click_count == 2 {
                        if self
                            .event(scope, &input, self.click.unwrap(), "dblclick", true)?
                            .is_none()
                        {
                            return Ok(None);
                        }
                    }
                    self.phase = Phase::Cleanup
                }
                Phase::Cleanup => {
                    if action == NativePointerAction::Cancel
                        || action == NativePointerAction::Up && input.buttons == 0
                    {
                        if let Some(pointer) = state
                            .native_pointers
                            .borrow_mut()
                            .pointers
                            .get_mut(&input.id)
                        {
                            pointer.buttons = 0;
                            pointer.pending_capture = None;
                            pointer.prevent_mouse = false;
                            pointer.down_targets.clear();
                            pointer.drag_start = None;
                            pointer.selection = None;
                            pointer.range = None;
                        }
                        if input.device == NativePointerDevice::Touch
                            || action == NativePointerAction::Cancel
                        {
                            self.phase = Phase::ExitBoundary;
                            continue;
                        }
                    }
                    self.done(self.target.unwrap(), self.allowed);
                }
                Phase::ExitBoundary => {
                    self.boundary(scope, self.target, None)?;
                    if self.owner.current() {
                        state
                            .native_pointers
                            .borrow_mut()
                            .pointers
                            .remove(&input.id);
                    }
                    self.done(self.target.unwrap(), self.allowed)
                }
                Phase::Done => return Ok(self.result),
            }
        }
    }
}
impl Drop for NativePointerDispatch {
    fn drop(&mut self) {
        if self.phase != Phase::Done {
            self.cancel()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_receipt_does_not_inline_all_event_and_default_continuations() {
        let nested = std::mem::size_of::<Event>()
            + std::mem::size_of::<Capture>()
            + std::mem::size_of::<retained_focus::Transition>()
            + std::mem::size_of::<native_drag::NativeDragInput>()
            + std::mem::size_of::<touch::Touch>();
        assert!(
            std::mem::size_of::<NativePointerDispatch>() < nested,
            "one retained input sample must not reserve every nested continuation on the stack"
        );
        assert!(
            std::mem::size_of::<Range>() < std::mem::size_of::<Event>(),
            "range publication retains its event through heap custody"
        );
    }
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    fn fixture(scope: &mut v8::PinScope) -> (Rc<V8BindingState>, NodeId) {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            html::parse("<button id=t type=button>target</button>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("t").unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(251), 1).unwrap();
        state.set_layout(BindingLayout {
            viewport: Some((100., 100.)),
            geometry: BTreeMap::from([(
                node_key(node),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.,
                }),
            )]),
            ..Default::default()
        });
        (state, node)
    }
    #[test]
    fn auxiliary_link_default_retains_original_link_and_callback_suffix() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(html::parse("<a id=link href='https://aux.test/original' target=named rel=noreferrer><span id=child>link</span></a><a id=other href='https://aux.test/wrong'></a>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("child").unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(256), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];child.addEventListener('auxclick',()=>{calls.push('first');other.append(child);link.href='https://aux.test/changed'},{once:true});child.addEventListener('auxclick',()=>calls.push('second'))",
        );
        let input = NativePointerInput {
            button: 1,
            ..Default::default()
        };
        let mut event =
            Event::new(scope, &state, &input, node, "auxclick", true, true, None).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(event.poll(scope).is_err());
        assert!(event.retry_safe());
        assert_eq!(event.poll(scope).unwrap(), Some(true));
        assert_eq!(event.poll(scope).unwrap(), Some(true));
        eval(
            scope,
            "if(calls.join(',')!=='first,second')throw Error('auxclick callbacks replayed')",
        );
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let crate::v8_bindings::PendingDefaultAction::Click(target) = actions.front().unwrap()
        else {
            panic!("auxiliary link action")
        };
        let navigation = target.navigation.as_ref().unwrap();
        assert_eq!(navigation.url, "https://aux.test/changed");
        assert_eq!(navigation.target, "_blank");
        assert_eq!(navigation.referrer_policy.as_deref(), Some("no-referrer"));
        assert!(navigation.noopener);
    }
    #[test]
    fn auxiliary_link_default_rejects_canceled_right_and_untrusted_events() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<a id=t href='https://aux.test/'><input id=check type=checkbox></a>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("t").unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(257), 1).unwrap();
        eval(scope, "t.onauxclick=e=>e.preventDefault()");
        let mut canceled = Event::new(
            scope,
            &state,
            &NativePointerInput {
                button: 1,
                ..Default::default()
            },
            node,
            "auxclick",
            true,
            true,
            None,
        )
        .unwrap();
        assert_eq!(canceled.poll(scope).unwrap(), Some(false));
        eval(scope, "t.onauxclick=null");
        let mut right = Event::new(
            scope,
            &state,
            &NativePointerInput {
                button: 2,
                ..Default::default()
            },
            node,
            "auxclick",
            true,
            true,
            None,
        )
        .unwrap();
        assert_eq!(right.poll(scope).unwrap(), Some(true));
        let object = pointer_event(
            scope,
            &state,
            &NativePointerInput {
                button: 1,
                ..Default::default()
            },
            "auxclick",
            true,
            true,
            None,
        )
        .unwrap();
        let id = event_id(scope, object).unwrap();
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = false;
        let mut untrusted = Event::from_object(scope, &state, node, "auxclick", object).unwrap();
        assert_eq!(untrusted.poll(scope).unwrap(), Some(true));
        assert!(state.pending_default_actions.borrow().is_empty());
        eval(
            scope,
            "if(check.checked)throw Error('auxiliary preactivation')",
        );
    }
    #[test]
    fn pointer_down_up_capture_focus_and_click_resume_each_callback_once() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (state, node) = fixture(scope);
        eval(
            scope,
            "globalThis.trace=[];t.addEventListener('pointerdown',e=>{trace.push('pd1:'+e.clientX);t.setPointerCapture(e.pointerId)},{once:true});t.onpointerdown=()=>trace.push('pd2');for(const type of ['mousedown','focus','gotpointercapture','pointerup','mouseup','lostpointercapture','click'])t.addEventListener(type,e=>{trace.push(type);if(type==='click')e.preventDefault()});",
        );
        let mut input = NativePointerInput {
            x: 25.,
            y: 25.,
            button: 0,
            buttons: 1,
            ..Default::default()
        };
        let mut down =
            NativePointerDispatch::new(&state, &input, NativePointerAction::Down).unwrap();
        input.x = 90.;
        for _ in 0..4 {
            native_platform::publication::fail_after_next_callback();
            assert!(down.poll(scope).is_err());
            assert!(down.retry_safe());
            assert!(native_pointer_hover_inputs(&state).is_empty());
            assert!(NativePointerBoundary::new(&state, &input, None).is_err());
            assert!(NativePointerDispatch::new(&state, &input, NativePointerAction::Move).is_err());
            assert_eq!(
                poll_native_selection_defaults(scope, &state, std::time::Instant::now()).unwrap(),
                0
            );
        }
        assert_eq!(down.poll(scope).unwrap(), Some((node, true)));
        assert_eq!(down.poll(scope).unwrap(), Some((node, true)));
        assert!(!down.retry_safe());
        input.x = 25.;
        input.buttons = 0;
        let mut up = NativePointerDispatch::new(&state, &input, NativePointerAction::Up).unwrap();
        for _ in 0..5 {
            native_platform::publication::fail_after_next_callback();
            assert!(up.poll(scope).is_err());
            assert!(up.retry_safe());
        }
        assert_eq!(up.poll(scope).unwrap(), Some((node, true)));
        eval(
            scope,
            "if(trace.join(',')!=='pd1:25,pd2,mousedown,focus,gotpointercapture,pointerup,mouseup,lostpointercapture,click')throw Error('pointer callback replay/order');if(t.hasPointerCapture(1))throw Error('capture survived release');",
        );
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        assert_eq!(state.native_pointers.borrow().pointers[&1].buttons, 0);
    }
    #[test]
    fn range_publication_retry_keeps_committed_value_and_change_receipt() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<input id=t type=range min=0 max=100 value=0>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("t").unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(252), 1).unwrap();
        let rect = oryn_common::v4::CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 16.,
        };
        let projection = oryn_common::v4::PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            box_model: oryn_common::v4::BoxModel {
                border_box: rect,
                padding_box: rect,
                content_box: rect,
                margin: Default::default(),
                padding: Default::default(),
                border: Default::default(),
            },
            text: Vec::new(),
            clips: Vec::new(),
        };
        state.set_layout(BindingLayout {
            viewport: Some((100., 100.)),
            hit_projections: BTreeMap::from([(node_key(node), projection)]),
            ..Default::default()
        });
        eval(
            scope,
            "globalThis.trace=[];t.oninput=()=>trace.push('input');t.onchange=()=>trace.push('change');",
        );
        let mut range = Range::new(&state, node, 90., 8., true);
        native_platform::publication::fail_after_next_callback();
        assert!(range.poll(scope).is_err());
        let committed = state.dom.borrow().control_value(node).unwrap();
        assert_ne!(committed, "0");
        native_platform::publication::fail_after_next_callback();
        assert!(range.poll(scope).is_err());
        assert_eq!(state.dom.borrow().control_value(node).unwrap(), committed);
        assert_eq!(range.poll(scope).unwrap(), Some(true));
        assert_eq!(range.poll(scope).unwrap(), Some(true));
        eval(
            scope,
            "if(trace.join(',')!=='input,change')throw Error('slider callback replay')",
        );
    }
    #[test]
    fn canceled_or_reset_pointer_receipt_never_focuses_or_touches_reused_source() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (state, node) = fixture(scope);
        eval(
            scope,
            "globalThis.trace=[];t.onpointerdown=()=>trace.push('pointer');t.onmousedown=()=>trace.push('mouse');t.onfocus=()=>trace.push('focus');",
        );
        let input = NativePointerInput {
            x: 20.,
            y: 20.,
            button: 0,
            buttons: 1,
            ..Default::default()
        };
        let mut job =
            NativePointerDispatch::new(&state, &input, NativePointerAction::Down).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        state.set_execution_suspended(true);
        assert!(job.poll(scope).unwrap().is_none());
        state.set_execution_suspended(false);
        reset_native_pointer_sources(&state);
        {
            let mut registry = state.native_pointers.borrow_mut();
            registry.synchronize(&state);
            registry.pointers.entry(input.id).or_default().buttons = 16;
        }
        assert_eq!(job.poll(scope).unwrap(), Some((node, false)));
        assert!(job.canceled());
        assert!(!job.retry_safe());
        job.cancel();
        assert_eq!(
            state.native_pointers.borrow().pointers[&input.id].buttons,
            16
        );
        eval(
            scope,
            "if(trace.join(',')!=='pointer')throw Error('stale pointer default');",
        );
        assert_ne!(state.dom.borrow().focused(), Some(node));
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
    }
}
