//! Accepted AX command ownership. The actor retains this across callbacks.
use super::*;
#[cfg(test)]
thread_local! {static FAIL_SNAPSHOT:Cell<bool>=const{Cell::new(false)};}
enum Child {
    Picker(Box<temporal_picker::Job>),
    Permission(Box<permission_prompt::Choice>),
    File(Box<file_picker::Completion>),
}
pub struct Receipt {
    owner: Rc<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    identity: (NodeId, u64, u64),
    command: TextCommand,
    phase: u8,
    child: Option<Child>,
    focus: Option<retained_focus::Transition>,
    keyboard: Option<native_keyboard::NativeKeyboardReceipt>,
    before: Option<TextSnapshot>,
    custom_before: Option<f64>,
    custom: bool,
    event: Option<retained_dom_event::Event>,
    result: Option<TextSnapshot>,
}
impl Receipt {
    pub fn new(
        state: &Rc<V8BindingState>,
        target: NodeId,
        command: &TextCommand,
    ) -> Result<Self, V8BindingError> {
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
        );
        if !live(state, target, identity) {
            return Err(V8BindingError::Message("stale accessibility target".into()));
        }
        Ok(Self {
            owner: state.clone(),
            target: oryn_dom::TrackedNodeLease::new(&state.dom, target)?,
            identity,
            command: command.clone(),
            phase: 0,
            child: None,
            focus: None,
            keyboard: None,
            before: None,
            custom_before: None,
            custom: false,
            event: None,
            result: None,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    /// Admission accounting includes the command payload retained until its
    /// native child takes ownership. The actor separately bounds job count.
    pub fn payload_bytes(&self) -> usize {
        match &self.command {
            TextCommand::FilePicker {
                files: Some(files), ..
            } => files.iter().fold(0usize, |sum, file| {
                sum.saturating_add(file.bytes.len())
                    .saturating_add(file.name.len())
                    .saturating_add(file.media_type.len())
            }),
            TextCommand::Select { expected_value, .. } => expected_value.len().saturating_mul(2),
            _ => 0,
        }
    }
    pub fn retry_safe(&self) -> bool {
        self.phase != 255 && live(&self.owner, self.target.current().1, self.identity)
    }
    pub fn cancel(&mut self) {
        if let Some(focus) = &mut self.focus {
            focus.cancel(&self.owner)
        }
        if let Some(event) = self.event.take() {
            event.cancel(&self.owner)
        }
        self.child = None;
        self.keyboard = None;
        self.phase = 255;
    }
    pub fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<TextSnapshot>, V8BindingError> {
        let result = self.poll_inner(scope);
        if matches!(&result, Err(V8BindingError::Message(_)))
            && matches!(self.phase, 0 | 21 | 30 | 33)
        {
            self.cancel();
        }
        result
    }
    fn poll_inner(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<TextSnapshot>, V8BindingError> {
        loop {
            if let Some(result) = &self.result {
                return Ok(Some(result.clone()));
            }
            let state = self.owner.clone();
            if self.phase == 255 {
                return Err(V8BindingError::Message(
                    "accessibility operation canceled".into(),
                ));
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            let (dom, target) = self.target.current();
            if !Rc::ptr_eq(&dom, &state.dom) || !live(&state, target, self.identity) {
                self.cancel();
                return Err(V8BindingError::Message(
                    "accessibility operation target retired".into(),
                ));
            }
            match self.phase {
                0 => match &self.command {
                    TextCommand::Read => self.phase = 90,
                    TextCommand::Picker { expected, action } => {
                        self.child = temporal_picker::prepare_accessibility(
                            &state, target, expected, action,
                        )?
                        .map(Box::new)
                        .map(Child::Picker);
                        self.phase = 10;
                    }
                    TextCommand::PermissionPrompt { expected, choice } => {
                        self.child = permission_prompt::prepare_accessibility(
                            scope, &state, target, expected, *choice,
                        )?
                        .map(Box::new)
                        .map(Child::Permission);
                        self.phase = 10;
                    }
                    TextCommand::FilePicker { expected, files } => {
                        self.child = Some(Child::File(Box::new(file_picker::Completion::new(
                            &state,
                            target,
                            expected,
                            files.as_deref(),
                        )?)));
                        self.phase = 10;
                    }
                    TextCommand::Select {
                        start,
                        end,
                        expected_value,
                        expected_selection_revision,
                    } => {
                        let before = text(scope, &state, target, &TextCommand::Read)?;
                        if state.dom.borrow().control_disabled(target)?
                            || before.protected
                            || start > end
                            || *end as usize > before.value.len()
                            || &before.value != expected_value
                            || before.selection_revision != *expected_selection_revision
                        {
                            return Err(V8BindingError::Message(
                                "stale or invalid accessibility selection".into(),
                            ));
                        }
                        let focus = state.dom.borrow().is_editable_text_control(target)
                            || editing_host_for_node(
                                &state.dom.borrow(),
                                Some(target),
                                state.design_mode_enabled(),
                                state.document_root(),
                            )? == Some(target);
                        if focus {
                            self.focus =
                                Some(retained_focus::Transition::new(&state, Some(target))?);
                        }
                        self.before = Some(before);
                        self.phase = 20;
                    }
                    TextCommand::Adjust { increase } => {
                        let dom = state.dom.borrow();
                        let record = dom.arena().get(target)?;
                        self.custom =
                            !dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
                                && record.accessibility_attribute("role").is_some_and(|roles| {
                                    roles
                                        .split_ascii_whitespace()
                                        .any(|role| matches!(role, "slider" | "spinbutton"))
                                });
                        if dom.is_inert(target)?
                            || dom.control_disabled(target)?
                            || record
                                .accessibility_attribute("aria-readonly")
                                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                            || record
                                .accessibility_attribute("aria-disabled")
                                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                        {
                            return Err(V8BindingError::Message(
                                "accessibility adjustment unavailable".into(),
                            ));
                        }
                        if self.custom {
                            let parse = |name| {
                                record
                                    .accessibility_attribute(name)
                                    .and_then(|value| oryn_dom::parse_html_floating_point(value))
                            };
                            self.custom_before = parse("aria-valuenow");
                            let bound = parse(if *increase {
                                "aria-valuemax"
                            } else {
                                "aria-valuemin"
                            });
                            if self.custom_before.zip(bound).is_some_and(|(value, bound)| {
                                if *increase {
                                    value >= bound
                                } else {
                                    value <= bound
                                }
                            }) {
                                self.phase = 90;
                                continue;
                            }
                            drop(dom);
                            self.focus =
                                Some(retained_focus::Transition::new(&state, Some(target))?);
                        } else {
                            let kind = dom
                                .attribute_in_null_namespace(target, "type")?
                                .unwrap_or("text")
                                .to_ascii_lowercase();
                            if !dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
                                || !matches!(kind.as_str(), "range" | "number")
                                || (kind == "number"
                                    && dom
                                        .attribute_in_null_namespace(target, "readonly")?
                                        .is_some())
                            {
                                return Err(V8BindingError::Message(
                                    "native adjustment target unavailable".into(),
                                ));
                            }
                        }
                        self.phase = 30;
                    }
                },
                10 => {
                    let ready = match self.child.as_mut() {
                        Some(Child::Picker(job)) => job.poll(scope)?,
                        Some(Child::Permission(job)) => job.poll(scope)?,
                        Some(Child::File(job)) => job.poll(scope)?,
                        None => true,
                    };
                    if !ready {
                        return Ok(None);
                    }
                    self.child = None;
                    self.phase = 90;
                }
                20 => {
                    if let Some(focus) = &mut self.focus
                        && !focus.poll(scope, &state)?
                    {
                        return Ok(None);
                    }
                    self.focus = None;
                    self.phase = 21;
                }
                21 => {
                    let current = text(scope, &state, target, &TextCommand::Read)?;
                    let before = self.before.as_ref().unwrap();
                    if current.value != before.value
                        || current.boundaries != before.boundaries
                        || current.control != before.control
                        || current.selection_revision != before.selection_revision
                    {
                        return Err(V8BindingError::Message(
                            "accessibility selection changed during focus".into(),
                        ));
                    }
                    let TextCommand::Select { start, end, .. } = self.command else {
                        unreachable!()
                    };
                    if current.control.is_some() {
                        let next = (start, end, "none".into());
                        if state.text_control_selections.borrow().get(&target) != Some(&next) {
                            queue_control_select(scope, &state, target)?;
                            if state.dom.borrow().is_temporal_control(target) {
                                state
                                    .dom
                                    .borrow_mut()
                                    .align_temporal_selection(target, start)?;
                            }
                            state
                                .text_control_selections
                                .borrow_mut()
                                .insert(target, next);
                            state
                                .selection_revision
                                .set(state.selection_revision.get().wrapping_add(1));
                        }
                    } else {
                        let first = before
                            .boundaries
                            .get(start as usize)
                            .ok_or(V8BindingError::Allocation("AX start boundary"))?;
                        let last = before
                            .boundaries
                            .get(end as usize)
                            .ok_or(V8BindingError::Allocation("AX end boundary"))?;
                        update_selection_boundaries(&state, first.start, last.end, false)?;
                    }
                    self.phase = 90;
                }
                30 => {
                    if let Some(focus) = &mut self.focus
                        && !focus.poll(scope, &state)?
                    {
                        return Ok(None);
                    }
                    self.focus = None;
                    if self.custom && state.dom.borrow().focused() != Some(target) {
                        return Err(V8BindingError::Message(
                            "accessibility adjustment lost focus".into(),
                        ));
                    }
                    self.phase = 31;
                }
                31 | 32 => {
                    let TextCommand::Adjust { increase } = self.command else {
                        unreachable!()
                    };
                    if self.custom {
                        if self.keyboard.is_none() {
                            let key = if increase { "ArrowUp" } else { "ArrowDown" };
                            let input = NativeKeyboardInput {
                                key: key.encode_utf16().collect(),
                                code: key.into(),
                                down: self.phase == 31,
                                ..Default::default()
                            };
                            self.keyboard =
                                Some(native_keyboard::begin_receipt(state.clone(), &input)?);
                        }
                        if native_keyboard::poll_receipt(scope, self.keyboard.as_mut().unwrap())?
                            .is_none()
                        {
                            return Ok(None);
                        }
                        self.keyboard = None;
                        self.phase += 1;
                    } else {
                        let before = state.dom.borrow().control_edit_value(target)?;
                        state.dom.borrow_mut().step_control(target, 1, !increase)?;
                        let changed = state.dom.borrow().control_edit_value(target)? != before;
                        if changed {
                            state.dom.borrow_mut().set_user_validity(target, true)?;
                        }
                        self.phase = if changed { 40 } else { 90 };
                    }
                }
                33 => {
                    let TextCommand::Adjust { increase } = self.command else {
                        unreachable!()
                    };
                    let after = state
                        .dom
                        .borrow()
                        .arena()
                        .get(target)?
                        .accessibility_attribute("aria-valuenow")
                        .and_then(|value| oryn_dom::parse_html_floating_point(value));
                    if after.is_none()
                        || self
                            .custom_before
                            .zip(after)
                            .is_some_and(|(before, after)| {
                                if increase {
                                    after <= before
                                } else {
                                    after >= before
                                }
                            })
                    {
                        return Err(V8BindingError::Message(
                            "native keyboard did not adjust accessibility range".into(),
                        ));
                    }
                    self.phase = 90;
                }
                40 | 41 => {
                    if self.event.is_none() {
                        let name = if self.phase == 40 { "input" } else { "change" };
                        let event = create_event_object(scope, &state, name, true, false)?;
                        let id = event_id(scope, event)?;
                        {
                            let mut records = state.events.borrow_mut();
                            let record = records
                                .get_mut(&id)
                                .ok_or(V8BindingError::Allocation("AX adjustment event"))?;
                            record.is_trusted = true;
                            record.composed = name == "input";
                        }
                        self.event = Some(retained_dom_event::Event::new(
                            scope, &state, target, name, event,
                        )?);
                    }
                    if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                        return Ok(None);
                    }
                    self.event = None;
                    self.phase = if self.phase == 40 { 41 } else { 90 };
                }
                90 => {
                    #[cfg(test)]
                    if FAIL_SNAPSHOT.with(|flag| flag.replace(false)) {
                        return Err(V8BindingError::Allocation(
                            "injected AX snapshot allocation",
                        ));
                    }
                    let result = text(scope, &state, target, &TextCommand::Read)?;
                    self.result = Some(result.clone());
                    return Ok(Some(result));
                }
                _ => return Err(V8BindingError::Allocation("AX receipt phase")),
            }
        }
    }
}
impl Drop for Receipt {
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
    fn custom_adjustment_retains_key_suffix_and_sends_keyup_once() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t role=spinbutton tabindex=0 aria-valuenow=1></div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(249), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];t.addEventListener('keydown',e=>{t.setAttribute('aria-valuenow',+t.getAttribute('aria-valuenow')+1);trace.push('first')});t.addEventListener('keydown',()=>trace.push('second'));t.onkeyup=()=>trace.push('up')",
        );
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let mut receipt =
            Receipt::new(&state, target, &TextCommand::Adjust { increase: true }).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        state.set_execution_suspended(true);
        assert!(receipt.poll(scope).unwrap().is_none());
        state.set_execution_suspended(false);
        assert!(receipt.poll(scope).unwrap().is_some());
        assert!(receipt.poll(scope).unwrap().is_some());
        eval(
            scope,
            "if(t.getAttribute('aria-valuenow')!=='2'||trace.join()!=='first,second,up')throw Error('AX key replay')",
        );
    }
    #[test]
    fn selection_snapshot_failure_does_not_repeat_native_publication() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abc</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(250), 1).unwrap();
        eval(scope, "t.focus();t.setSelectionRange(0,0)");
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let before = text(scope, &state, target, &TextCommand::Read).unwrap();
        let mut receipt = Receipt::new(
            &state,
            target,
            &TextCommand::Select {
                start: 1,
                end: 2,
                expected_value: before.value,
                expected_selection_revision: before.selection_revision,
            },
        )
        .unwrap();
        FAIL_SNAPSHOT.with(|flag| flag.set(true));
        assert!(receipt.poll(scope).is_err());
        let revision = state.selection_revision.get();
        let result = receipt.poll(scope).unwrap().unwrap();
        assert_eq!((result.start, result.end), (1, 2));
        assert_eq!(state.selection_revision.get(), revision);
        assert_eq!(receipt.poll(scope).unwrap(), Some(result));
    }
    #[test]
    fn file_choice_retains_callback_suffix_and_original_payload() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t type=file>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(251), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];t.addEventListener('input',e=>{globalThis.original=e;trace.push('first')});t.addEventListener('input',e=>{if(e!==original)throw Error('event replaced');trace.push('second')});t.onchange=()=>trace.push('change')",
        );
        let target = state.dom.borrow().find_by_id("t").unwrap();
        state.dom.borrow_mut().open_file_picker(target).unwrap();
        let expected = state
            .dom
            .borrow()
            .file_picker_snapshot(target)
            .unwrap()
            .unwrap();
        let mut receipt = Receipt::new(
            &state,
            target,
            &TextCommand::FilePicker {
                expected,
                files: Some(vec![crate::accessibility::PickerFile {
                    name: "native.txt".into(),
                    media_type: "text/plain".into(),
                    bytes: vec![1, 2, 3],
                    last_modified: 0,
                }]),
            },
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(receipt.poll(scope).unwrap().is_some());
        assert!(receipt.poll(scope).unwrap().is_some());
        eval(
            scope,
            "if(trace.join()!=='first,second,change'||t.files.length!==1||t.files[0].name!=='native.txt'||t.files[0].size!==3)throw Error('AX file replay')",
        );
    }
}
