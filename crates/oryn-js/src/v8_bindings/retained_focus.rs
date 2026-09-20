//! Traced focus transition with retained native event/default progress.
use super::retained_dom_event::Event as EventJob;
use super::*;
fn live(state: &V8BindingState, node: NodeId, generation: u64, epoch: u64) -> bool {
    !state.execution_retired()
        && state.document_generation() == generation
        && state.document_stream_epoch() == epoch
        && state
            .dom
            .borrow()
            .is_document_connected(node)
            .unwrap_or(false)
        && state
            .dom
            .borrow()
            .arena()
            .node_document(node)
            .ok()
            .flatten()
            == Some(state.document_root())
}
pub(super) struct Transition {
    owner: Rc<V8BindingState>,
    previous: Option<oryn_dom::TrackedNodeLease>,
    target: Option<oryn_dom::TrackedNodeLease>,
    generation: u64,
    epoch: u64,
    revision: Option<u64>,
    phase: u8,
    event: Option<EventJob>,
}
impl Transition {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        target: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        let target = target
            .map(|target| {
                focus_navigation::delegated_target(state, target)
                    .map(|delegated| delegated.unwrap_or(target))
            })
            .transpose()?;
        let previous = state.dom.borrow().focused();
        Ok(Self {
            owner: state.clone(),
            previous: previous
                .map(|node| oryn_dom::TrackedNodeLease::new(&state.dom, node))
                .transpose()?,
            target: target
                .map(|node| oryn_dom::TrackedNodeLease::new(&state.dom, node))
                .transpose()?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            revision: None,
            phase: 0,
            event: None,
        })
    }
    fn node(
        &self,
        state: &Rc<V8BindingState>,
        lease: &Option<oryn_dom::TrackedNodeLease>,
        connected: bool,
    ) -> Option<NodeId> {
        let (owner, node) = lease.as_ref()?.current();
        (Rc::ptr_eq(&owner, &state.dom)
            && state
                .dom
                .borrow()
                .arena()
                .node_document(node)
                .ok()
                .flatten()
                == Some(state.document_root())
            && (!connected || live(state, node, self.generation, self.epoch)))
        .then_some(node)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if !Rc::ptr_eq(&self.owner, state) {
            return Err(V8BindingError::Allocation("retained focus owner"));
        }
        loop {
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if state.execution_retired()
                || state.document_generation() != self.generation
                || state.document_stream_epoch() != self.epoch
            {
                self.cancel(state);
                return Ok(true);
            }
            if let Some(event) = self.event.as_mut() {
                if event.poll(scope, state)?.is_none() {
                    return Ok(false);
                }
                self.event = None;
                self.phase += 1;
                continue;
            }
            let previous = self.node(state, &self.previous, false);
            let target = self.node(state, &self.target, true);
            match self.phase {
                0 => {
                    if previous == target {
                        return Ok(true);
                    }
                    state.dom.borrow_mut().focus(None)?;
                    let revision = state.focus_transition_revision.get().wrapping_add(1);
                    state.focus_transition_revision.set(revision);
                    self.revision = Some(revision);
                    self.phase = 1;
                }
                1 => {
                    if let Some(previous) = previous {
                        let event = create_event_object(scope, state, "change", true, false)?;
                        let id = event_id(scope, event)?;
                        state
                            .events
                            .borrow_mut()
                            .get_mut(&id)
                            .ok_or(V8BindingError::Allocation("focus change event"))?
                            .is_trusted = true;
                        let event = EventJob::new(scope, state, previous, "change", event)?;
                        if state.dom.borrow_mut().take_control_change(previous)? {
                            self.event = Some(event);
                            continue;
                        }
                    }
                    self.phase = 2;
                }
                2 | 3 => {
                    if let Some(previous) = previous {
                        let name = if self.phase == 2 { "blur" } else { "focusout" };
                        let event = create_focus_event_object(scope, state, name, target)?;
                        self.event = Some(EventJob::new(scope, state, previous, name, event)?);
                    } else {
                        self.phase += 1;
                    }
                }
                4 => {
                    let Some(target) = target else {
                        return Ok(true);
                    };
                    refresh_layout(state)?;
                    if self.revision != Some(state.focus_transition_revision.get())
                        || state.dom.borrow().is_inert(target)?
                        || !focus_navigation::eligible_for(state, target, false)?
                    {
                        return Ok(true);
                    }
                    let control = selection_text::prepare_control(state, target)?;
                    state.dom.borrow_mut().focus(Some(target))?;
                    if let Some(control) = control {
                        *state.selected_text_control.borrow_mut() = Some(control);
                    }
                    self.phase = 5;
                }
                5 | 6 => {
                    let Some(target) = target else {
                        return Ok(true);
                    };
                    if self.revision != Some(state.focus_transition_revision.get())
                        || state.dom.borrow().focused() != Some(target)
                    {
                        return Ok(true);
                    }
                    let name = if self.phase == 5 { "focus" } else { "focusin" };
                    let event = create_focus_event_object(scope, state, name, previous)?;
                    self.event = Some(EventJob::new(scope, state, target, name, event)?);
                }
                7 => {
                    if let Some(target) = target
                        && self.revision == Some(state.focus_transition_revision.get())
                        && state.dom.borrow().focused() == Some(target)
                    {
                        initialize_focused_caret(state, target)?;
                    }
                    self.phase = 8;
                }
                8 => {
                    interest::poll(scope, state)?;
                    self.phase = 9;
                    return Ok(true);
                }
                _ => return Ok(true),
            }
        }
    }
    pub(super) fn cancel(&self, state: &V8BindingState) {
        if let Some(event) = &self.event {
            event.cancel(state)
        }
    }
}

impl Drop for Transition {
    fn drop(&mut self) {
        self.cancel(&self.owner)
    }
}
