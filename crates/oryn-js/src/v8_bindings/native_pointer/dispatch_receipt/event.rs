//! Pointer event dispatch and activation own separate, resumable commits.
use super::*;
pub(super) struct Event {
    owner: Rc<V8BindingState>,
    input_owner: InputOwner,
    target: v8::Global<v8::Object>,
    object: v8::Global<v8::Object>,
    event: Option<Box<retained_dom_event::Event>>,
    activation: Option<form_activation::Activation>,
    default: Option<Box<form_activation::Finish>>,
    auxiliary: Option<Box<submissions::LinkActivation>>,
    phase: u8,
    result: Option<bool>,
    accepted: bool,
    name: String,
    pseudo: bool,
    pseudo_owner: Option<Rc<V8BindingState>>,
}
impl Event {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
        target: NodeId,
        name: &str,
        bubbles: bool,
        cancelable: bool,
        related: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        let object = pointer_event(scope, state, input, name, bubbles, cancelable, related)?;
        if name == "click" && input.button == 0 {
            interest::mark_pseudo_click(scope, state, object, input.id, target, input.x, input.y)?;
        }
        Self::from_object(scope, state, target, name, object)
    }
    pub(super) fn from_object(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        name: &str,
        object: v8::Local<v8::Object>,
    ) -> Result<Self, V8BindingError> {
        let wrapper = wrap_node(scope, state, target)?;
        let pseudo = interest::pseudo_activation(scope, object).is_some();
        Ok(Self {
            owner: state.clone(),
            input_owner: InputOwner::new(state),
            target: v8::Global::new(scope, wrapper),
            object: v8::Global::new(scope, object),
            event: None,
            activation: None,
            default: None,
            auxiliary: None,
            phase: 0,
            result: None,
            accepted: false,
            name: name.into(),
            pseudo,
            pseudo_owner: None,
        })
    }
    pub(super) fn retry_safe(&self) -> bool {
        self.result.is_none()
            && self.input_owner.current()
            && self.default.as_ref().is_none_or(|job| job.retry_safe())
            && self.auxiliary.as_ref().is_none_or(|job| job.retry_safe())
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
                self.cancel();
                return Ok(self.result);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                0 => {
                    let wrapper = v8::Local::new(scope, &self.target);
                    let Some((owner, target)) = node_identity::argument(scope, wrapper.into())?
                    else {
                        self.cancel();
                        return Ok(self.result);
                    };
                    if !owner.dom.borrow().is_connected(target)? {
                        self.cancel();
                        return Ok(self.result);
                    }
                    let object = v8::Local::new(scope, &self.object);
                    let id = event_id(scope, object)?;
                    if !self.pseudo {
                        self.activation = if self.name == "auxclick" {
                            form_activation::prepare_auxiliary(scope, &owner, target, object)?
                        } else {
                            form_activation::prepare(scope, &owner, target, object, id)?
                        };
                    }
                    self.phase = 1;
                }
                1 => {
                    let wrapper = v8::Local::new(scope, &self.target);
                    let Some((owner, target)) = node_identity::argument(scope, wrapper.into())?
                    else {
                        self.cancel();
                        return Ok(self.result);
                    };
                    let object = v8::Local::new(scope, &self.object);
                    self.event = Some(allocation::try_boxed(|| {
                        retained_dom_event::Event::new(scope, &owner, target, &self.name, object)
                    })?);
                    self.phase = 2;
                }
                2 => {
                    let event = self.event.as_mut().unwrap();
                    let owner = event.owner();
                    let Some(accepted) = event.poll(scope, &owner)? else {
                        return Ok(None);
                    };
                    self.accepted = accepted;
                    self.event.take();
                    self.phase = 3;
                }
                3 => {
                    if !matches!(self.name.as_str(), "click" | "auxclick") {
                        self.result = Some(self.accepted);
                        return Ok(self.result);
                    }
                    let wrapper = v8::Local::new(scope, &self.target);
                    let Some((owner, target)) = node_identity::argument(scope, wrapper.into())?
                    else {
                        self.cancel();
                        return Ok(self.result);
                    };
                    if self.pseudo {
                        if self.accepted {
                            interest::admit_activation(&owner, target)?;
                            self.pseudo_owner = Some(owner);
                            self.phase = 5;
                            continue;
                        } else {
                            self.result = Some(false);
                            return Ok(self.result);
                        }
                    }
                    let object = v8::Local::new(scope, &self.object);
                    if self.name == "auxclick" {
                        if self.accepted {
                            if let Some(activation) = &self.activation {
                                if let Some((link_owner, link)) = activation.link_target(scope)? {
                                    let suffix = if Rc::ptr_eq(&owner.dom, &link_owner.dom)
                                        && link_owner.dom.borrow().is_element(
                                            link,
                                            "http://www.w3.org/1999/xhtml",
                                            "a",
                                        ) {
                                        form_activation::image_map_suffix(
                                            scope,
                                            &link_owner,
                                            target,
                                            object,
                                            true,
                                        )?
                                    } else {
                                        None
                                    };
                                    self.auxiliary = allocation::try_optional(|| {
                                        submissions::LinkActivation::new(
                                            scope,
                                            &link_owner,
                                            link,
                                            true,
                                            suffix.as_deref(),
                                            Some("_blank"),
                                        )
                                    })?;
                                }
                            }
                        }
                        self.activation.take();
                        self.phase = 6;
                        continue;
                    }
                    self.default = Some(allocation::try_boxed(|| {
                        form_activation::Finish::new(
                            scope,
                            &owner,
                            self.activation.clone(),
                            target,
                            object,
                            self.accepted,
                        )
                    })?);
                    self.activation.take();
                    self.phase = 4;
                }
                4 => {
                    if !self.default.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.result = Some(self.accepted);
                    return Ok(self.result);
                }
                6 => {
                    if let Some(auxiliary) = &mut self.auxiliary {
                        if !auxiliary.poll(scope)? {
                            return Ok(None);
                        }
                    }
                    self.result = Some(self.accepted);
                    return Ok(self.result);
                }
                _ => {
                    interest::poll(scope, self.pseudo_owner.as_ref().unwrap())?;
                    self.result = Some(self.accepted);
                    return Ok(self.result);
                }
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(activation) = self.activation.take() {
            activation.cancel_native();
        }
        self.event.take();
        if let Some(default) = &mut self.default {
            default.cancel()
        }
        if let Some(auxiliary) = &mut self.auxiliary {
            auxiliary.cancel()
        }
        self.result = Some(false);
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        if self.result.is_none() {
            self.cancel()
        }
    }
}

pub(super) struct Capture {
    state: Rc<V8BindingState>,
    input: NativePointerInput,
    owner: InputOwner,
    targets: [Option<NodeId>; 2],
    phase: usize,
    event: Option<Box<Event>>,
}
impl Capture {
    pub(super) fn new(state: &Rc<V8BindingState>, input: &NativePointerInput) -> Self {
        let owner = InputOwner::new(state);
        let (old, new) = {
            let mut registry = state.native_pointers.borrow_mut();
            let pointer = registry.pointers.entry(input.id).or_default();
            if pointer
                .pending_capture
                .is_some_and(|node| !state.dom.borrow().is_connected(node).unwrap_or(false))
            {
                pointer.pending_capture = None;
            }
            let old = pointer.capture;
            let new = pointer.pending_capture;
            pointer.capture = new;
            (old, new)
        };
        Self {
            state: state.clone(),
            input: input.clone(),
            owner,
            targets: if old == new { [None, None] } else { [old, new] },
            phase: 0,
            event: None,
        }
    }
    #[inline(never)]
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        while self.phase < 2 {
            if !self.owner.current() {
                self.event.take();
                self.phase = 2;
                return Ok(true);
            }
            let Some(target) = self.targets[self.phase] else {
                self.phase += 1;
                continue;
            };
            if self.event.is_none() {
                let target = if self.state.dom.borrow().is_connected(target)? {
                    target
                } else {
                    self.state.document_root()
                };
                self.event = Some(allocation::try_boxed(|| {
                    Event::new(
                        scope,
                        &self.state,
                        &self.input,
                        target,
                        if self.phase == 0 {
                            "lostpointercapture"
                        } else {
                            "gotpointercapture"
                        },
                        true,
                        false,
                        None,
                    )
                })?);
            }
            if self.event.as_mut().unwrap().poll(scope)?.is_none() {
                return Ok(false);
            }
            self.event.take();
            self.phase += 1;
        }
        Ok(true)
    }
}
