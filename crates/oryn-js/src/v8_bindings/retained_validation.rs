//! Constraint validation and submit-event continuation for native activation.
use super::retained_dom_event::Event;
use super::*;

struct InvalidControls {
    controls: Vec<v8::Global<v8::Object>>,
    unhandled: Vec<v8::Global<v8::Object>>,
    cursor: usize,
    event: Option<Event>,
    focus: Option<(Rc<V8BindingState>, retained_focus::Transition)>,
    report_prepared: bool,
}
impl InvalidControls {
    fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        form: NodeId,
    ) -> Result<Self, V8BindingError> {
        let controls = state.dom.borrow().invalid_form_controls(form)?;
        let controls = controls
            .into_iter()
            .map(|node| wrap_node(scope, state, node).map(|object| v8::Global::new(scope, object)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut unhandled = Vec::new();
        unhandled
            .try_reserve(controls.len())
            .map_err(|_| V8BindingError::Allocation("invalid control report"))?;
        Ok(Self {
            controls,
            unhandled,
            cursor: 0,
            event: None,
            focus: None,
            report_prepared: false,
        })
    }
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        while self.cursor < self.controls.len() {
            if scope.is_execution_terminating() {
                return Ok(None);
            }
            if self.event.is_none() {
                let object = v8::Local::new(scope, &self.controls[self.cursor]);
                let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                    self.cursor += 1;
                    continue;
                };
                if owner.execution_suspended() {
                    return Ok(None);
                }
                let event = create_event_object(scope, &owner, "invalid", false, true)?;
                let id = event_id(scope, event)?;
                owner
                    .events
                    .borrow_mut()
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("invalid event"))?
                    .is_trusted = true;
                self.event = Some(Event::new(scope, &owner, node, "invalid", event)?);
            }
            let event = self.event.as_mut().unwrap();
            let owner = event.owner();
            let Some(accepted) = event.poll(scope, &owner)? else {
                return Ok(None);
            };
            self.event = None;
            if accepted {
                self.unhandled.push(self.controls[self.cursor].clone());
            }
            self.cursor += 1;
        }
        if !self.report_prepared {
            for object in &self.unhandled {
                let object = v8::Local::new(scope, object);
                let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                    continue;
                };
                let available = {
                    let dom = owner.dom.borrow();
                    dom.is_connected(node)?
                        && dom.control_will_validate(node)?
                        && !dom.control_validity(node)?
                        && !dom.is_inert(node)?
                };
                if !available || css_suppresses_focus(&owner, node)? {
                    continue;
                }
                let anchor = custom_elements::validation_anchor(scope, &owner, node)?;
                if css_suppresses_focus(&owner, anchor)? {
                    continue;
                }
                let focus = retained_focus::Transition::new(&owner, Some(anchor))?;
                self.focus = Some((owner, focus));
                break;
            }
            self.report_prepared = true;
        }
        if let Some((owner, focus)) = &mut self.focus {
            if !focus.poll(scope, owner)? {
                return Ok(None);
            }
        }
        Ok(Some(self.controls.is_empty()))
    }
    fn cancel(&self) {
        if let Some(event) = &self.event {
            event.cancel(&event.owner());
        }
        if let Some((owner, focus)) = &self.focus {
            focus.cancel(owner);
        }
    }
}
impl Drop for InvalidControls {
    fn drop(&mut self) {
        self.cancel()
    }
}

pub(super) struct Submit {
    form: v8::Global<v8::Object>,
    submitter: Option<v8::Global<v8::Object>>,
    guard: Option<submissions::FormOperationGuard>,
    invalid: Option<InvalidControls>,
    event: Option<Event>,
    phase: u8,
    accepted: bool,
    result: Option<bool>,
}
impl Submit {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        form: NodeId,
        submitter: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        let skip = validation::submit_preflight(scope, state, form, submitter)?;
        let object = wrap_node(scope, state, form)?;
        let object = v8::Global::new(scope, object);
        let submitter = submitter
            .map(|node| wrap_node(scope, state, node).map(|object| v8::Global::new(scope, object)))
            .transpose()?;
        let guard = if skip.is_some() {
            submissions::FormOperationGuard::begin(
                scope,
                state,
                form,
                submissions::FormOperation::SubmitEvent,
            )?
        } else {
            None
        };
        let active = guard.is_some();
        let invalid = if active {
            state.dom.borrow_mut().mark_form_user_validity(form)?;
            if skip == Some(false) {
                Some(InvalidControls::new(scope, state, form)?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            form: object,
            submitter,
            guard,
            invalid,
            event: None,
            phase: 0,
            accepted: false,
            result: (!active).then_some(false),
        })
    }
    fn finish(&mut self, result: bool) -> Option<bool> {
        self.result = Some(result);
        self.guard = None;
        self.invalid = None;
        self.event = None;
        self.result
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            match self.phase {
                0 => {
                    if let Some(invalid) = self.invalid.as_mut() {
                        let Some(valid) = invalid.poll(scope)? else {
                            return Ok(None);
                        };
                        if !valid {
                            return Ok(self.finish(false));
                        }
                    }
                    self.invalid = None;
                    self.phase = 1;
                }
                1 => {
                    let object = v8::Local::new(scope, &self.form);
                    let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                        return Ok(self.finish(false));
                    };
                    if active_document_state_for_node(scope, &owner, node).is_none() {
                        return Ok(self.finish(false));
                    }
                    if owner.execution_suspended() {
                        return Ok(None);
                    }
                    let event = create_event_object_with_interface(
                        scope,
                        &owner,
                        "SubmitEvent",
                        "submit",
                        true,
                        true,
                    )?;
                    let submitter = self
                        .submitter
                        .as_ref()
                        .map(|object| v8::Local::new(scope, object).into())
                        .unwrap_or_else(|| v8::null(scope).into());
                    validation::set_submitter(scope, event, submitter)?;
                    let id = event_id(scope, event)?;
                    owner
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("submit event"))?
                        .is_trusted = true;
                    self.event = Some(Event::new(scope, &owner, node, "submit", event)?);
                    self.phase = 2;
                }
                2 => {
                    let event = self.event.as_mut().unwrap();
                    let owner = event.owner();
                    let Some(accepted) = event.poll(scope, &owner)? else {
                        return Ok(None);
                    };
                    self.accepted = accepted;
                    self.event = None;
                    self.phase = 3;
                }
                _ => {
                    if !self.accepted {
                        return Ok(self.finish(false));
                    }
                    let object = v8::Local::new(scope, &self.form);
                    let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                        return Ok(self.finish(false));
                    };
                    let connected = active_document_state_for_node(scope, &owner, node).is_some()
                        && owner.dom.borrow().is_connected(node)?;
                    return Ok(self.finish(connected));
                }
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(event) = &self.event {
            event.cancel(&event.owner());
        }
        self.finish(false);
    }
}
impl Drop for Submit {
    fn drop(&mut self) {
        self.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn invalid_and_submit_event_suffixes_resume_without_duplicate_callbacks() {
        for invalid in [false, true] {
            super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(
                    "<form id=f><input id=t required><button id=b>Send</button></form>",
                )
                .into_runtime()
                .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(227), 1).unwrap();
            eval(scope, "globalThis.log=[]");
            if !invalid {
                eval(scope, "t.value='valid'");
            }
            let target = if invalid { "t" } else { "f" };
            let event = if invalid { "invalid" } else { "submit" };
            eval(
                scope,
                &format!(
                    "{target}.addEventListener('{event}',e=>{{globalThis.original=e;log.push('first')}},{{once:true}});{target}.addEventListener('{event}',e=>{{log.push(e===original?'second':'wrong');e.preventDefault()}})"
                ),
            );
            let form = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), "f")
                .unwrap()
                .unwrap();
            let button = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), "b")
                .unwrap()
                .unwrap();
            let mut submit = Submit::new(scope, &state, form, Some(button)).unwrap();
            native_platform::publication::fail_after_next_callback();
            assert!(submit.poll(scope).is_err());
            assert!(eval(scope, "log.join()==='first'").is_true());
            assert_eq!(submit.poll(scope).unwrap(), Some(false));
            assert_eq!(submit.poll(scope).unwrap(), Some(false));
            assert!(eval(scope, "log.join()==='first,second'").is_true());
            assert!(!submissions::FormOperation::SubmitEvent.active(scope, &state, form));
        }
    }
}
