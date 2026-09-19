//! One accepted native picker choice, including its editing event suffix.
use super::*;
pub(in super::super) struct Job {
    owner: Rc<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    expected: Option<oryn_dom::TemporalPicker>,
    index: Option<usize>,
    value: String,
    before: String,
    control: bool,
    changed: bool,
    phase: u8,
    input: Option<clipboard_actions::ClipboardNotification>,
    event: Option<retained_dom_event::Event>,
}
impl Job {
    fn new(
        state: &Rc<V8BindingState>,
        node: NodeId,
        value: String,
        index: Option<usize>,
    ) -> Result<Self, V8BindingError> {
        Ok(Self {
            owner: state.clone(),
            target: oryn_dom::TrackedNodeLease::new(&state.dom, node)?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            expected: state.dom.borrow().temporal_picker_snapshot(node)?,
            index,
            value,
            before: state.dom.borrow().control_edit_value(node)?,
            control: state.dom.borrow().is_editable_text_control(node),
            changed: false,
            phase: 0,
            input: None,
            event: None,
        })
    }
    pub(in super::super) fn value(
        state: &Rc<V8BindingState>,
        node: NodeId,
        value: &str,
    ) -> Result<Self, V8BindingError> {
        Self::new(state, node, value.into(), None)
    }
    pub(in super::super) fn choose(
        state: &Rc<V8BindingState>,
        node: NodeId,
        index: usize,
    ) -> Result<Self, V8BindingError> {
        Self::new(state, node, String::new(), Some(index))
    }
    pub(in super::super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 5 {
                return Ok(true);
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            let (dom, node) = self.target.current();
            if !Rc::ptr_eq(&dom, &state.dom)
                || state.execution_retired()
                || state.document_generation() != self.generation
                || state.document_stream_epoch() != self.epoch
                || !state.dom.borrow().is_connected(node)?
            {
                self.cancel();
                return Ok(true);
            }
            match self.phase {
                0 => {
                    if !eligible(&state, node)?
                        || state.dom.borrow().temporal_picker_snapshot(node)? != self.expected
                    {
                        self.cancel();
                        return Ok(true);
                    }
                    if let Some(index) = self.index {
                        let value = state
                            .dom
                            .borrow_mut()
                            .choose_temporal_picker_item(node, index)?;
                        self.index = None;
                        self.phase = 1;
                        state.layout_cache_key.borrow_mut().take();
                        let Some(value) = value else {
                            self.phase = 5;
                            return Ok(true);
                        };
                        self.value = value;
                    } else {
                        self.phase = 1;
                    }
                }
                1 => {
                    if !self.value.is_empty()
                        && !state
                            .dom
                            .borrow()
                            .temporal_candidate_selectable(node, &self.value)?
                    {
                        self.cancel();
                        return Ok(true);
                    }
                    if self.control {
                        if self.input.is_none() {
                            self.input = Some(clipboard_actions::ClipboardNotification::editing(
                                scope,
                                &state,
                                node,
                                "beforeinput",
                                "insertReplacementText",
                                Some(&self.value),
                                None,
                            )?);
                        }
                        let Some(accepted) = self.input.as_mut().unwrap().poll(scope, &state)?
                        else {
                            return Ok(false);
                        };
                        self.input = None;
                        if !accepted {
                            self.phase = 5;
                            return Ok(true);
                        }
                    }
                    self.phase = 2;
                }
                2 => {
                    // Beforeinput can replace the value or picker session. Never
                    // apply an old choice to its replacement after resumption.
                    if !eligible(&state, node)?
                        || state.dom.borrow().control_edit_value(node)? != self.before
                        || state
                            .dom
                            .borrow()
                            .temporal_picker_snapshot(node)?
                            .as_ref()
                            .is_none_or(|popup| {
                                self.expected
                                    .as_ref()
                                    .is_none_or(|expected| popup.session != expected.session)
                                    || (!self.value.is_empty()
                                        && !popup
                                            .items
                                            .iter()
                                            .any(|item| item.enabled && item.value == self.value)
                                        && popup.draft != self.value)
                            })
                        || (!self.value.is_empty()
                            && !state
                                .dom
                                .borrow()
                                .temporal_candidate_selectable(node, &self.value)?)
                    {
                        self.cancel();
                        return Ok(true);
                    }
                    if self.control {
                        edit_native_control(
                            &state,
                            node,
                            0,
                            oryn_dom::dom_string::len(&self.before) as u32,
                            &self.value,
                        )?;
                    } else {
                        let mut dom = state.dom.borrow_mut();
                        dom.set_value(node, &self.value, false)?;
                        let record = dom.arena_mut().get_mut(node)?;
                        record.user_edited_value = true;
                        if record.pending_edit_value.is_none() {
                            record.pending_edit_value = Some(self.before.clone());
                        }
                    }
                    self.phase = 3;
                    close(&state, node);
                    self.changed = state.dom.borrow().control_edit_value(node)? != self.before;
                }
                3 => {
                    if !self.changed {
                        self.phase = 5;
                        continue;
                    }
                    if self.control {
                        if self.input.is_none() {
                            self.input = Some(clipboard_actions::ClipboardNotification::editing(
                                scope,
                                &state,
                                node,
                                "input",
                                "insertReplacementText",
                                Some(&self.value),
                                None,
                            )?);
                        }
                        if self.input.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(false);
                        }
                        self.input = None;
                    } else {
                        if self.event.is_none() {
                            self.event = Some(Self::event(scope, &state, node, "input")?);
                        }
                        if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(false);
                        }
                        self.event = None;
                    }
                    self.phase = 4;
                }
                4 => {
                    if self.event.is_none() {
                        let event = Self::event(scope, &state, node, "change")?;
                        if !state.dom.borrow_mut().take_control_change(node)? {
                            self.phase = 5;
                            continue;
                        }
                        self.event = Some(event);
                    }
                    if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                        return Ok(false);
                    }
                    self.event = None;
                    self.phase = 5;
                }
                _ => unreachable!(),
            }
        }
    }
    fn event(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        node: NodeId,
        name: &str,
    ) -> Result<retained_dom_event::Event, V8BindingError> {
        let event = create_event_object(scope, state, name, true, false)?;
        let id = event_id(scope, event)?;
        {
            let mut records = state.events.borrow_mut();
            let record = records
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("temporal picker event"))?;
            record.is_trusted = true;
            record.composed = name == "input";
        }
        retained_dom_event::Event::new(scope, state, node, name, event)
    }
    pub(in super::super) fn cancel(&mut self) {
        if let Some(mut input) = self.input.take() {
            input.cancel();
        }
        if let Some(event) = self.event.take() {
            event.cancel(&self.owner)
        }
        self.phase = 5;
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn datalist_choice_retains_beforeinput_input_and_change_suffixes() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<input id=t list=l><datalist id=l><option value=alpha></datalist>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(246), 1).unwrap();
        eval(
            scope,
            "t.focus();globalThis.trace=[];for(const name of ['beforeinput','input','change'])t.addEventListener(name,e=>trace.push(name));",
        );
        let node = state.dom.borrow().find_by_id("t").unwrap();
        state
            .dom
            .borrow_mut()
            .open_temporal_picker(node, 0.)
            .unwrap();
        let mut job = Job::value(&state, node, "alpha").unwrap();
        for expected in ["", "alpha", "alpha"] {
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
            assert_eq!(
                state.dom.borrow().control_edit_value(node).unwrap(),
                expected
            );
            state.set_execution_suspended(true);
            assert!(!job.poll(scope).unwrap());
            state.set_execution_suspended(false);
        }
        assert!(job.poll(scope).unwrap());
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(t.value!=='alpha'||trace.join()!=='beforeinput,input,change')throw Error('picker replay')",
        );
        assert!(native_keyboard::undo_control(&state, node, false).unwrap());
        assert_eq!(state.dom.borrow().control_edit_value(node).unwrap(), "");
        assert!(!native_keyboard::undo_control(&state, node, false).unwrap());
    }
    #[test]
    fn beforeinput_replacing_picker_session_abandons_old_choice() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<input id=t list=l><datalist id=l><option value=alpha></datalist>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(247), 1).unwrap();
        eval(
            scope,
            "t.focus();t.onbeforeinput=()=>{t.value='replacement'};globalThis.inputs=0;t.oninput=()=>inputs++",
        );
        let node = state.dom.borrow().find_by_id("t").unwrap();
        state
            .dom
            .borrow_mut()
            .open_temporal_picker(node, 0.)
            .unwrap();
        let mut job = Job::value(&state, node, "alpha").unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(t.value!=='replacement'||inputs)throw Error('stale picker commit')",
        );
    }
}
