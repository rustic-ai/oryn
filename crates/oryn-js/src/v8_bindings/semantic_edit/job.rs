//! One OIL replacement, including its original focus and notification suffix.
use super::*;

#[derive(Clone, Copy)]
enum Phase {
    Initial,
    Focus,
    Prepare,
    Before,
    Verify,
    Edit,
    Input,
    Change,
    Done,
}
pub struct Replacement {
    owner: Rc<V8BindingState>,
    target: NodeId,
    identity: (NodeId, u64, u64, u64),
    value: String,
    sensitive: bool,
    control: bool,
    text_control: bool,
    widget: bool,
    numeric: Option<f64>,
    phase: Phase,
    focus: retained_focus::Transition,
    before: String,
    input: Option<clipboard_actions::ClipboardNotification>,
    event: Option<retained_dom_event::Event>,
    transaction: Option<retained_editing::Transaction>,
    cursor: Option<oryn_dom::MutationCursor>,
    selection_revision: u64,
    control_selection: Option<(u32, u32, String)>,
    mutated: bool,
    changed: bool,
    change_committed: bool,
    failure: Option<&'static str>,
}
impl Replacement {
    pub fn new(
        state: &Rc<V8BindingState>,
        target: NodeId,
        value: &str,
        sensitive: bool,
    ) -> Result<Self, V8BindingError> {
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let (control, text_control, widget, numeric) = {
            let dom = state.dom.borrow();
            if state.execution_retired()
                || state.execution_tasks_stopped()
                || !dom.is_document_connected(target)?
                || dom.node_document(target)? != Some(identity.0)
            {
                return Err(V8BindingError::Allocation(
                    "retired semantic editing target",
                ));
            }
            let control = dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
                || dom.is_element(target, "http://www.w3.org/1999/xhtml", "textarea");
            let text_control = dom.is_editable_text_control(target);
            if dom.control_disabled(target)?
                || dom.is_inert(target)?
                || (text_control
                    && dom
                        .attribute_in_null_namespace(target, "readonly")?
                        .is_some())
            {
                return Err(V8BindingError::Allocation(
                    "semantic editing target is not editable",
                ));
            }
            if control
                && dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
                && dom
                    .attribute_in_null_namespace(target, "type")?
                    .is_some_and(|kind| {
                        matches!(
                            kind.to_ascii_lowercase().as_str(),
                            "file"
                                | "checkbox"
                                | "radio"
                                | "button"
                                | "submit"
                                | "reset"
                                | "image"
                                | "hidden"
                        )
                    })
            {
                return Err(V8BindingError::Allocation(
                    "semantic text replacement requires a value control",
                ));
            }
            let widget = !control
                && editing_host_for_node(
                    &dom,
                    Some(target),
                    state.design_mode_enabled(),
                    identity.0,
                )? != Some(target);
            let numeric = if widget {
                let node = dom.arena().get(target)?;
                let role=node.accessibility_attribute("role").and_then(|roles|roles.split_ascii_whitespace().find(|role|matches!(*role,"textbox"|"searchbox"|"gridcell"|"columnheader"|"rowheader"|"slider"|"spinbutton"))).ok_or(V8BindingError::Allocation("semantic replacement requires an editing host or editable accessibility widget"))?;
                if node
                    .accessibility_attribute("aria-readonly")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                    || node
                        .accessibility_attribute("aria-disabled")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                {
                    return Err(V8BindingError::Allocation(
                        "accessibility widget is read-only or disabled",
                    ));
                }
                if matches!(role, "slider" | "spinbutton") {
                    Some(oryn_dom::parse_html_floating_point(value).ok_or(
                        V8BindingError::Allocation("accessibility numeric value is invalid"),
                    )?)
                } else {
                    None
                }
            } else {
                None
            };
            (control, text_control, widget, numeric)
        };
        Ok(Self {
            owner: state.clone(),
            target,
            identity,
            value: value.into(),
            sensitive,
            control,
            text_control,
            widget,
            numeric,
            phase: Phase::Initial,
            focus: retained_focus::Transition::new(state, Some(target))?,
            before: String::new(),
            input: None,
            event: None,
            transaction: None,
            cursor: None,
            selection_revision: 0,
            control_selection: None,
            mutated: false,
            changed: false,
            change_committed: false,
            failure: None,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    pub fn retryable(&self) -> bool {
        self.failure.is_none() && !matches!(self.phase, Phase::Done)
    }
    fn live(&self) -> bool {
        let state = &self.owner;
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
            && state
                .dom
                .borrow()
                .is_document_connected(self.target)
                .unwrap_or(false)
            && state.dom.borrow().node_document(self.target).ok().flatten() == Some(self.identity.0)
    }
    fn matches_widget(&self) -> Result<bool, V8BindingError> {
        let dom = self.owner.dom.borrow();
        let node = dom.arena().get(self.target)?;
        Ok(if let Some(value) = self.numeric {
            node.accessibility_attribute("aria-valuenow")
                .and_then(|value| oryn_dom::parse_html_floating_point(value))
                == Some(value)
        } else {
            dom.arena().text_content(self.target)? == self.value
        })
    }
    fn input_type(&self) -> &'static str {
        if self.widget || !self.value.is_empty() {
            "insertReplacementText"
        } else {
            "deleteContentBackward"
        }
    }
    fn editable_now(&self) -> Result<bool, V8BindingError> {
        let dom = self.owner.dom.borrow();
        let node = dom.arena().get(self.target)?;
        if dom.control_disabled(self.target)?
            || dom.is_inert(self.target)?
            || node
                .accessibility_attribute("aria-readonly")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            || node
                .accessibility_attribute("aria-disabled")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            return Ok(false);
        }
        if self.control {
            if dom.is_editable_text_control(self.target) != self.text_control
                || (self.text_control
                    && dom
                        .attribute_in_null_namespace(self.target, "readonly")?
                        .is_some())
            {
                return Ok(false);
            }
            if dom.is_element(self.target, "http://www.w3.org/1999/xhtml", "input")
                && dom
                    .attribute_in_null_namespace(self.target, "type")?
                    .is_some_and(|kind| {
                        matches!(
                            kind.to_ascii_lowercase().as_str(),
                            "file"
                                | "checkbox"
                                | "radio"
                                | "button"
                                | "submit"
                                | "reset"
                                | "image"
                                | "hidden"
                        )
                    })
            {
                return Ok(false);
            }
        } else if !self.widget
            && editing_host_for_node(
                &dom,
                Some(self.target),
                self.owner.design_mode_enabled(),
                self.identity.0,
            )? != Some(self.target)
        {
            return Ok(false);
        }
        Ok(true)
    }
    fn fail(&mut self, message: &'static str) -> V8BindingError {
        self.failure = Some(message);
        self.cancel();
        V8BindingError::Allocation(message)
    }
    pub fn cancel(&mut self) {
        self.focus.cancel(&self.owner);
        self.input.take();
        self.event.take();
        if let Some(transaction) = &mut self.transaction {
            transaction.cancel();
        }
        self.phase = Phase::Done;
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if let Some(message) = self.failure {
            return Err(V8BindingError::Allocation(message));
        }
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(true);
            }
            if !self.live() {
                return Err(self.fail("retired semantic editing target"));
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.phase {
                Phase::Initial => {
                    self.phase = if self.widget && self.matches_widget()? {
                        Phase::Done
                    } else {
                        Phase::Focus
                    };
                }
                Phase::Focus => {
                    if !self.focus.poll(scope, &state)? {
                        return Ok(false);
                    }
                    if !self.live()
                        || (!self.widget && state.dom.borrow().focused() != Some(self.target))
                    {
                        return Err(self.fail("semantic editing focus changed"));
                    }
                    self.phase = Phase::Prepare;
                }
                Phase::Prepare => {
                    if !self.editable_now()? {
                        return Err(self.fail("semantic editing target changed during focus"));
                    }
                    if self.widget {
                        self.phase = Phase::Before;
                        continue;
                    }
                    self.before = if self.control {
                        state.dom.borrow().control_edit_value(self.target)?
                    } else {
                        state.dom.borrow().arena().text_content(self.target)?
                    };
                    if self.before == self.value {
                        if self.control {
                            state
                                .dom
                                .borrow_mut()
                                .set_control_sensitive(self.target, self.sensitive)?;
                        }
                        self.phase = Phase::Done;
                        continue;
                    }
                    if self.control {
                        state.text_control_selections.borrow_mut().insert(
                            self.target,
                            (
                                0,
                                oryn_dom::dom_string::len(&self.before) as u32,
                                "none".into(),
                            ),
                        );
                    } else {
                        let end = light_children(&state.dom.borrow(), self.target)?.len() as u32;
                        set_selection_boundaries(
                            &state,
                            BoundaryPoint {
                                container: self.target,
                                offset: 0,
                            },
                            BoundaryPoint {
                                container: self.target,
                                offset: end,
                            },
                        )?;
                    }
                    self.cursor = Some(state.dom.borrow().mutation_cursor());
                    self.selection_revision = state.selection_revision.get();
                    self.control_selection = state
                        .text_control_selections
                        .borrow()
                        .get(&self.target)
                        .cloned();
                    self.phase = Phase::Before;
                }
                Phase::Before => {
                    if self.widget || self.text_control || !self.control {
                        if self.input.is_none() {
                            self.input = Some(clipboard_actions::ClipboardNotification::editing(
                                scope,
                                &state,
                                self.target,
                                "beforeinput",
                                self.input_type(),
                                (self.widget || !self.value.is_empty())
                                    .then_some(self.value.as_str()),
                                None,
                            )?);
                        }
                        let Some(accepted) = self.input.as_mut().unwrap().poll(scope, &state)?
                        else {
                            return Ok(false);
                        };
                        self.input.take();
                        if !self.widget && !accepted {
                            return Err(self.fail("semantic edit canceled by beforeinput"));
                        }
                    }
                    self.phase = Phase::Verify;
                }
                Phase::Verify => {
                    #[cfg(test)]
                    if FAIL_VERIFICATION.with(|flag| flag.replace(false)) {
                        return Err(V8BindingError::Allocation("semantic verification fault"));
                    }
                    if self.widget {
                        if !self.live() || !self.matches_widget()? {
                            return Err(self.fail("native editing request did not establish accessibility widget value"));
                        }
                        self.phase = Phase::Done;
                        continue;
                    }
                    if !self.editable_now()? {
                        return Err(self.fail("semantic editing target is no longer editable"));
                    }
                    if !self.live()
                        || state.dom.borrow().focused() != Some(self.target)
                        || Some(state.dom.borrow().mutation_cursor()) != self.cursor
                        || state.selection_revision.get() != self.selection_revision
                        || state
                            .text_control_selections
                            .borrow()
                            .get(&self.target)
                            .cloned()
                            != self.control_selection
                    {
                        return Err(self.fail("semantic edit target changed during beforeinput"));
                    }
                    self.phase = Phase::Edit;
                }
                Phase::Edit => {
                    if self.control {
                        if !self.mutated {
                            if self.text_control {
                                edit_native_control(
                                    &state,
                                    self.target,
                                    0,
                                    oryn_dom::dom_string::len(&self.before) as u32,
                                    &self.value,
                                )?;
                            } else {
                                state.dom.borrow_mut().set_user_control_value(
                                    self.target,
                                    &self.value,
                                    self.sensitive,
                                )?;
                            }
                            self.mutated = true;
                        }
                        state
                            .dom
                            .borrow_mut()
                            .set_control_sensitive(self.target, self.sensitive)?;
                        self.changed =
                            state.dom.borrow().control_edit_value(self.target)? != self.before;
                        if self.changed && !self.change_committed {
                            state.dom.borrow_mut().take_control_change(self.target)?;
                            self.change_committed = true;
                        }
                    } else {
                        if self.transaction.is_none() {
                            self.transaction = Some(retained_editing::Transaction::new(
                                &state,
                                self.input_type(),
                            )?);
                        }
                        let Some(changed) = self.transaction.as_mut().unwrap().poll_owned(
                            scope,
                            &state,
                            editing_commands::Command::ReplaceSelection(self.value.clone()),
                        )?
                        else {
                            return Ok(false);
                        };
                        self.changed = changed;
                    }
                    self.phase = if self.changed {
                        Phase::Input
                    } else {
                        Phase::Done
                    };
                }
                Phase::Input => {
                    if self.text_control || !self.control {
                        if self.input.is_none() {
                            self.input = Some(clipboard_actions::ClipboardNotification::editing(
                                scope,
                                &state,
                                self.target,
                                "input",
                                self.input_type(),
                                (!self.value.is_empty()).then_some(self.value.as_str()),
                                None,
                            )?);
                        }
                        if self.input.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(false);
                        }
                        self.input.take();
                    } else {
                        if self.event.is_none() {
                            self.event = Some(self.notification(scope, "input", true)?);
                        }
                        if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(false);
                        }
                        self.event.take();
                    }
                    self.phase = if self.control {
                        Phase::Change
                    } else {
                        Phase::Done
                    };
                }
                Phase::Change => {
                    if self.event.is_none() {
                        self.event = Some(self.notification(scope, "change", false)?);
                    }
                    if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                        return Ok(false);
                    }
                    self.event.take();
                    self.phase = Phase::Done;
                }
                Phase::Done => return Ok(true),
            }
        }
    }
    fn notification(
        &self,
        scope: &mut v8::PinScope,
        name: &str,
        composed: bool,
    ) -> Result<retained_dom_event::Event, V8BindingError> {
        let event = create_event_object(scope, &self.owner, name, true, false)?;
        let id = event_id(scope, event)?;
        if let Some(record) = self.owner.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = composed;
        }
        retained_dom_event::Event::new(scope, &self.owner, self.target, name, event)
    }
}
impl Drop for Replacement {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
thread_local! {static FAIL_VERIFICATION:Cell<bool>=const{Cell::new(false)};}

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
    fn widget_verification_and_focus_retries_keep_completed_callbacks() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div id=w tabindex=0 role=textbox>old</div><input id=t value=old>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(220), 1).unwrap();
        let widget = state.dom.borrow().find_by_id("w").unwrap();
        eval(
            scope,
            "globalThis.w=document.getElementById('w');globalThis.trace=[];w.addEventListener('focus',()=>{trace.push('focus-first');w.textContent='new'});w.addEventListener('focus',()=>trace.push('focus-second'));w.addEventListener('beforeinput',()=>{trace.push('request');w.textContent='new'});",
        );
        let mut job = Replacement::new(&state, widget, "new", false).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(trace.join(',')!=='focus-first,focus-second,request')throw Error('matching widget skipped unfinished focus');w.textContent='old';trace.length=0;",
        );
        let mut job = Replacement::new(&state, widget, "new", false).unwrap();
        FAIL_VERIFICATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(trace.join(',')!=='request'||w.textContent!=='new')throw Error('verification replayed widget request');globalThis.t=document.getElementById('t');t.addEventListener('focus',()=>t.readOnly=true);t.addEventListener('beforeinput',()=>trace.push('unexpected'));",
        );
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let mut job = Replacement::new(&state, target, "blocked", false).unwrap();
        assert!(job.poll(scope).is_err());
        assert!(!job.retryable());
        eval(
            scope,
            "if(t.value!=='old'||trace.join(',')!=='request')throw Error('focus made target readonly but edit continued');",
        );
    }
    #[test]
    fn replacement_resumes_each_notification_without_replaying_native_edit() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=old>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(218), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();globalThis.trace=[];for(const name of ['beforeinput','input','change'])t.addEventListener(name,()=>trace.push(name));",
        );
        let mut job = Replacement::new(&state, target, "new", false).unwrap();
        for _ in 0..3 {
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
        }
        assert!(job.poll(scope).unwrap());
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(t.value!=='new'||trace.join(',')!=='beforeinput,input,change')throw Error('replacement replayed its notification');",
        );
        assert!(native_keyboard::undo_control(&state, target, false).unwrap());
        eval(
            scope,
            "if(t.value!=='old')throw Error('replacement must record one undo');",
        );
    }
    #[test]
    fn canceled_and_parked_replacements_preserve_the_original_operation() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=old>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(219), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();globalThis.calls=0;t.addEventListener('beforeinput',e=>{calls++;e.preventDefault()},{once:true});",
        );
        let mut canceled = Replacement::new(&state, target, "blocked", false).unwrap();
        assert!(canceled.poll(scope).is_err());
        assert!(canceled.poll(scope).is_err());
        eval(
            scope,
            "if(t.value!=='old'||calls!==1)throw Error('canceled replacement replayed');",
        );
        let mut job = Replacement::new(&state, target, "new", false).unwrap();
        state.set_execution_suspended(true);
        assert!(!job.poll(scope).unwrap());
        assert_eq!(
            state.dom.borrow().control_edit_value(target).unwrap(),
            "old"
        );
        state.set_execution_suspended(false);
        assert!(job.poll(scope).unwrap());
        eval(
            scope,
            "if(t.value!=='new')throw Error('replacement did not resume');",
        );
    }
}
