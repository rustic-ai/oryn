//! Native synthetic click custody. Activation defaults are distinct stages from
//! legacy preactivation and the retained click listener sequence.
use super::retained_dom_event::Event;
use super::*;

struct Guard {
    target: Rc<oryn_dom::TrackedNodeLease>,
    active: Cell<bool>,
}
thread_local! {static GUARDS:RefCell<Vec<RcWeak<Guard>>>=const{RefCell::new(Vec::new())};}
pub(super) fn active(state: &V8BindingState, node: NodeId) -> bool {
    GUARDS.with(|guards| {
        let mut guards = guards.borrow_mut();
        guards.retain(|guard| guard.upgrade().is_some_and(|guard| guard.active.get()));
        guards.iter().filter_map(RcWeak::upgrade).any(|guard| {
            let (dom, current) = guard.target.current();
            current == node && Rc::ptr_eq(&dom, &state.dom)
        })
    })
}
fn register(target: Rc<oryn_dom::TrackedNodeLease>) -> Result<Rc<Guard>, V8BindingError> {
    let guard = Rc::new(Guard {
        target,
        active: Cell::new(true),
    });
    GUARDS.with(|guards| {
        let mut guards = guards.borrow_mut();
        guards.retain(|guard| guard.upgrade().is_some_and(|guard| guard.active.get()));
        if guards.len() >= 1024 {
            return Err(V8BindingError::Allocation(
                "native activation nesting limit",
            ));
        }
        guards
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("native activation guard"))?;
        guards.push(Rc::downgrade(&guard));
        Ok(guard)
    })
}

pub(super) struct Click {
    owner: Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    guard: Rc<Guard>,
    object: v8::Global<v8::Object>,
    event: v8::Global<v8::Object>,
    dispatch: Option<Event>,
    activation: Option<form_activation::Activation>,
    phase: u8,
    accepted: bool,
    result: Option<bool>,
    default: Option<form_activation::Finish>,
}
impl Click {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
    ) -> Result<Option<Self>, V8BindingError> {
        if state.dom.borrow().control_disabled(target)?
            || state.click_in_progress.borrow().contains(&target)
            || active(state, target)
        {
            return Ok(None);
        }
        let lease = Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, target)?);
        let object = wrap_node(scope, state, target)?;
        let event = create_synthetic_click_event(scope, state)?;
        let id = event_id(scope, event)?;
        state
            .events
            .borrow_mut()
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("activation event"))?
            .is_trusted = true;
        let guard = register(lease)?;
        Ok(Some(Self {
            owner: state.clone(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            guard,
            object: v8::Global::new(scope, object),
            event: v8::Global::new(scope, event),
            dispatch: None,
            activation: None,
            phase: 0,
            accepted: false,
            result: None,
            default: None,
        }))
    }
    pub(super) fn from_source(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        source: v8::Local<v8::Object>,
    ) -> Result<Option<Self>, V8BindingError> {
        let Some(click) = Self::new(scope, state, target)? else {
            return Ok(None);
        };
        let event = v8::Local::new(scope, &click.event);
        if let Some(payload) = form_activation::mouse_payload(scope, source) {
            event.set_internal_field(2, payload.into());
        }
        let source_owner = binding_state_for_receiver(scope, source)
            .ok_or(V8BindingError::Allocation("label activation source"))?;
        let source_id = event_id(scope, source)?;
        let trusted = source_owner
            .events
            .borrow()
            .get(&source_id)
            .is_some_and(|record| record.is_trusted);
        let id = event_id(scope, event)?;
        state
            .events
            .borrow_mut()
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("label activation event"))?
            .is_trusted = trusted;
        Ok(Some(click))
    }
    fn release(&self) {
        self.guard.active.set(false);
    }
    pub(super) fn retry_safe(&self) -> bool {
        self.default
            .as_ref()
            .is_none_or(form_activation::Finish::retry_safe)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            if self.owner.execution_retired()
                || self.owner.document_generation() != self.generation
                || self.owner.document_stream_epoch() != self.epoch
            {
                self.cancel();
                return Ok(Some(false));
            }
            match self.phase {
                0 => {
                    let object = v8::Local::new(scope, &self.object);
                    let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                        self.cancel();
                        return Ok(Some(false));
                    };
                    let event = v8::Local::new(scope, &self.event);
                    let id = event_id(scope, event)?;
                    self.activation = form_activation::prepare(scope, &owner, node, event, id)?;
                    self.phase = 1;
                }
                1 => {
                    let object = v8::Local::new(scope, &self.object);
                    let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                        self.cancel();
                        return Ok(Some(false));
                    };
                    let event = v8::Local::new(scope, &self.event);
                    self.dispatch = Some(Event::new(scope, &owner, node, "click", event)?);
                    self.phase = 2;
                }
                2 => {
                    let event = self.dispatch.as_mut().unwrap();
                    let owner = event.owner();
                    let Some(accepted) = event.poll(scope, &owner)? else {
                        return Ok(None);
                    };
                    self.accepted = accepted;
                    self.dispatch = None;
                    self.phase = 3;
                }
                3 => {
                    let object = v8::Local::new(scope, &self.object);
                    let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                        self.cancel();
                        return Ok(Some(false));
                    };
                    let event = v8::Local::new(scope, &self.event);
                    self.default = Some(form_activation::Finish::new(
                        scope,
                        &owner,
                        self.activation.clone(),
                        node,
                        event,
                        self.accepted,
                    )?);
                    self.activation = None;
                    self.phase = 4;
                }
                _ => {
                    if !self.default.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.release();
                    self.result = Some(self.accepted);
                    return Ok(self.result);
                }
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if (self.phase < 3 || !self.accepted)
            && let Some(activation) = &self.activation
        {
            activation.cancel_native();
        }
        if let Some(event) = &self.dispatch {
            event.cancel(&event.owner());
        }
        if let Some(default) = &mut self.default {
            default.cancel();
        }
        self.release();
        self.result = Some(false);
    }
}
impl Drop for Click {
    fn drop(&mut self) {
        if self.result.is_none() {
            self.cancel()
        }
    }
}
