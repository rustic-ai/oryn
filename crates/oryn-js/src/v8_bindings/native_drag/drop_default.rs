//! Retained copy/default insertion. Move transactions have independent source
//! deletion custody and must not route through this copy operation.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    Before,
    Edit,
    Input,
    Change,
    Done,
}
pub(super) struct DropDefault {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    target: NodeId,
    store: Store,
    x: f64,
    y: f64,
    phase: Phase,
    host: Option<NodeId>,
    control: bool,
    file: bool,
    text: String,
    html: Option<String>,
    data: Option<crate::clipboard::ClipboardData>,
    event: Option<clipboard_actions::ClipboardNotification>,
    simple: Option<retained_dom_event::Event>,
    edit: Option<retained_editing::Transaction>,
    committed: bool,
    result: bool,
}
impl DropDefault {
    pub(super) fn set_input_owner(&mut self, state: &Rc<V8BindingState>) {
        self.input_owner = native_pointer::InputOwner::new(state);
    }
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        target: NodeId,
        store: &Store,
        x: f64,
        y: f64,
    ) -> Self {
        let mut store = store.clone();
        store.set_mode(Mode::ReadOnly);
        Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            target,
            store,
            x,
            y,
            phase: Phase::Prepare,
            host: None,
            control: false,
            file: false,
            text: String::new(),
            html: None,
            data: None,
            event: None,
            simple: None,
            edit: None,
            committed: false,
            result: false,
        }
    }
    fn live(&self) -> bool {
        let state = &self.owner;
        self.input_owner.current()
            && !state.execution_retired()
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
            && state
                .dom
                .borrow()
                .arena()
                .node_document(self.target)
                .ok()
                .flatten()
                == Some(self.identity.0)
    }
    pub(super) fn cancel(&mut self) {
        self.event.take();
        self.simple.take();
        if let Some(edit) = &mut self.edit {
            edit.cancel();
        }
        self.phase = Phase::Done;
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(Some(self.result));
            }
            if !self.live() {
                self.cancel();
                return Ok(Some(self.result));
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                Phase::Prepare => {
                    if default_effect(&state, self.target, &self.store).is_none() {
                        self.cancel();
                        continue;
                    }
                    self.file = state.dom.borrow().is_file_input(self.target);
                    if self.file {
                        self.phase = Phase::Edit;
                        continue;
                    }
                    let Some(caret) =
                        selection_movement::drop_caret(scope, &state, self.target, self.x, self.y)?
                    else {
                        self.cancel();
                        continue;
                    };
                    self.control = state.dom.borrow().is_text_control(self.target);
                    if self.control {
                        state
                            .text_control_selections
                            .borrow_mut()
                            .insert(self.target, (caret.offset, caret.offset, "none".into()));
                        self.host = Some(self.target);
                    } else {
                        self.host = editing_host_for_node(
                            &state.dom.borrow(),
                            Some(self.target),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )?;
                        if self.host.is_none() {
                            self.cancel();
                            continue;
                        }
                        set_selection_boundaries(&state, caret, caret)?;
                        self.data = Some(clipboard_actions::to_data(&self.store)?);
                    }
                    self.text =
                        oryn_dom::dom_string::from_units(&self.store.get_data("text/plain"));
                    self.phase = Phase::Before;
                }
                Phase::Before => {
                    if self.event.is_none() {
                        self.event = Some(clipboard_actions::ClipboardNotification::editing(
                            scope,
                            &state,
                            self.host.unwrap(),
                            "beforeinput",
                            "insertFromDrop",
                            self.control.then_some(self.text.as_str()),
                            self.data.as_ref(),
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.event.take();
                    if !accepted {
                        self.cancel();
                        continue;
                    }
                    self.phase = Phase::Edit;
                }
                Phase::Edit => {
                    if !self.committed && self.edit.is_none() {
                        if default_effect(&state, self.target, &self.store).is_none() {
                            self.cancel();
                            continue;
                        }
                        if self.file != state.dom.borrow().is_file_input(self.target)
                            || (!self.file
                                && self.control != state.dom.borrow().is_text_control(self.target))
                        {
                            self.cancel();
                            continue;
                        }
                        if self.file {
                            let mut files = self.store.files();
                            if state
                                .dom
                                .borrow()
                                .attribute(self.target, "multiple")?
                                .is_none()
                            {
                                files.truncate(1);
                            }
                            if files.is_empty() {
                                self.cancel();
                                continue;
                            }
                            state
                                .dom
                                .borrow_mut()
                                .set_selected_files(self.target, std::sync::Arc::new(files))?;
                            self.committed = true;
                            self.result = true;
                        } else if self.control {
                            let length = oryn_dom::dom_string::len(
                                &state.dom.borrow().control_value(self.target)?,
                            ) as u32;
                            let (start, end, _) = state
                                .text_control_selections
                                .borrow()
                                .get(&self.target)
                                .cloned()
                                .unwrap_or((length, length, "none".into()));
                            edit_native_control(
                                &state,
                                self.target,
                                start.min(length),
                                end.min(length).max(start.min(length)),
                                &self.text,
                            )?;
                            self.committed = true;
                            self.result = true;
                        } else {
                            if editing_host(&state)? != self.host {
                                self.cancel();
                                continue;
                            }
                            self.html = if self.store.types().iter().any(|kind| kind == "text/html")
                                && !host_is_plaintext_only(&state.dom.borrow(), self.host.unwrap())
                            {
                                Some(clipboard_actions::sanitized_html_for_state(
                                    &state,
                                    &oryn_dom::dom_string::from_units(
                                        &self.store.get_data("text/html"),
                                    ),
                                )?)
                            } else {
                                None
                            };
                            self.edit = Some(retained_editing::Transaction::new(
                                &state,
                                "insertFromDrop",
                            )?);
                        }
                    }
                    if self.file {
                        state
                            .dom
                            .borrow_mut()
                            .set_user_validity(self.target, true)?;
                    }
                    if let Some(edit) = &mut self.edit {
                        let command = match self.html.clone() {
                            Some(html) => editing_commands::Command::Html(html),
                            None => editing_commands::Command::Text(self.text.clone()),
                        };
                        let Some(changed) = edit.poll_owned(scope, &state, command)? else {
                            return Ok(None);
                        };
                        self.committed = true;
                        self.result = changed;
                    }
                    self.phase = Phase::Input;
                }
                Phase::Input => {
                    if self.file {
                        if self.simple.is_none() {
                            self.simple = Some(self.simple_event(scope, "input")?);
                        }
                        if self.simple.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(None);
                        }
                        self.simple.take();
                        self.phase = Phase::Change;
                    } else {
                        if self.event.is_none() {
                            self.event = Some(clipboard_actions::ClipboardNotification::editing(
                                scope,
                                &state,
                                self.host.unwrap(),
                                "input",
                                "insertFromDrop",
                                self.control.then_some(self.text.as_str()),
                                self.data.as_ref(),
                            )?);
                        }
                        if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                            return Ok(None);
                        }
                        self.event.take();
                        self.phase = Phase::Done;
                    }
                }
                Phase::Change => {
                    if self.simple.is_none() {
                        self.simple = Some(self.simple_event(scope, "change")?);
                    }
                    if self.simple.as_mut().unwrap().poll(scope, &state)?.is_none() {
                        return Ok(None);
                    }
                    self.simple.take();
                    self.phase = Phase::Done;
                }
                Phase::Done => return Ok(Some(self.result)),
            }
        }
    }
    fn simple_event(
        &self,
        scope: &mut v8::PinScope,
        kind: &str,
    ) -> Result<retained_dom_event::Event, V8BindingError> {
        let event = create_event_object(scope, &self.owner, kind, true, false)?;
        let id = event_id(scope, event)?;
        if let Some(record) = self.owner.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = kind == "input";
        }
        retained_dom_event::Event::new(scope, &self.owner, self.target, kind, event)
    }
}
impl Drop for DropDefault {
    fn drop(&mut self) {
        self.cancel();
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
    fn file(name: &str) -> oryn_dom::SelectedFile {
        oryn_dom::SelectedFile {
            name: name.into(),
            last_modified: 0,
            media_type: "text/plain".into(),
            bytes: vec![97].into(),
        }
    }
    #[test]
    fn source_reset_after_file_commit_stops_remaining_notifications() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input type=file id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(236), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.trace=[];const t=document.getElementById('t');t.addEventListener('input',()=>trace.push('input'));t.addEventListener('change',()=>trace.push('change'));",
        );
        let mut store = Store::default();
        store.add_file(file("original.txt"));
        let mut job = DropDefault::new(&state, target, &store, 0., 0.);
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        native_pointer::reset_native_pointer_sources(&state);
        state
            .dom
            .borrow_mut()
            .set_selected_files(target, std::sync::Arc::new(vec![file("replacement.txt")]))
            .unwrap();
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        eval(
            scope,
            "if(document.getElementById('t').files[0].name!=='replacement.txt'||trace.join(',')!=='input')throw Error('revoked input changed new selection or emitted change');",
        );
    }
    #[test]
    fn file_drop_retry_keeps_input_suffix_and_does_not_reinstall_files() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input type=file id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(223), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');globalThis.trace=[];t.addEventListener('input',()=>trace.push('input'));t.addEventListener('change',()=>trace.push('change'));",
        );
        let mut store = Store::default();
        store.add_file(file("first.txt"));
        store.add_file(file("second.txt"));
        let mut job = DropDefault::new(&state, target, &store, 0., 0.);
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        eval(
            scope,
            "if(t.files.length!==1||t.files[0].name!=='first.txt')throw Error('single-file drop selection');",
        );
        state
            .dom
            .borrow_mut()
            .set_selected_files(target, std::sync::Arc::new(vec![file("replacement.txt")]))
            .unwrap();
        state.set_execution_suspended(true);
        assert!(job.poll(scope).unwrap().is_none());
        state.set_execution_suspended(false);
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        eval(
            scope,
            "if(t.files[0].name!=='replacement.txt'||trace.join(',')!=='input,change')throw Error('drop default replayed file mutation or callback');",
        );
    }
}
