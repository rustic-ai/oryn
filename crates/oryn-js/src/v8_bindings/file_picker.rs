use super::*;
pub(super) fn open(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    if !state.execution_retired() && !css_suppresses_focus(state, node)? {
        state.dom.borrow_mut().open_file_picker(node)?;
    }
    Ok(())
}
pub(super) fn click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    if !state.dom.borrow().is_file_input(node) {
        return Ok(false);
    }
    let id = event_id(scope, event)?;
    if !state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.is_trusted)
    {
        return Ok(false);
    }
    open(state, node)?;
    Ok(true)
}
pub(super) struct Completion {
    owner: Rc<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    expected: oryn_dom::FilePicker,
    selected: Option<std::sync::Arc<Vec<oryn_dom::SelectedFile>>>,
    changed: bool,
    phase: u8,
    event: Option<retained_dom_event::Event>,
}
impl Completion {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        node: NodeId,
        expected: &oryn_dom::FilePicker,
        files: Option<&[crate::accessibility::PickerFile]>,
    ) -> Result<Self, V8BindingError> {
        if state.execution_retired()
            || state.dom.borrow().file_picker_snapshot(node)?.as_ref() != Some(expected)
        {
            return Err(V8BindingError::Message("stale native file chooser".into()));
        }
        if let Some(files) = files {
            if files.is_empty()
                || files.len() > 128
                || (!expected.multiple && files.len() != 1)
                || files
                    .iter()
                    .try_fold(0usize, |sum, file| sum.checked_add(file.bytes.len()))
                    .is_none_or(|bytes| bytes > 64 * 1024 * 1024)
                || files
                    .iter()
                    .any(|file| file.name.is_empty() || file.name.contains(['/', '\\', '\0']))
            {
                return Err(V8BindingError::Message(
                    "native file chooser payload rejected".into(),
                ));
            }
        }
        let selected = files.map(|files| {
            std::sync::Arc::new(
                files
                    .iter()
                    .map(|file| oryn_dom::SelectedFile {
                        name: file.name.clone(),
                        media_type: file.media_type.clone(),
                        bytes: file.bytes.clone().into(),
                        last_modified: file.last_modified,
                    })
                    .collect::<Vec<_>>(),
            )
        });
        Ok(Self {
            owner: state.clone(),
            target: oryn_dom::TrackedNodeLease::new(&state.dom, node)?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            expected: expected.clone(),
            selected,
            changed: false,
            phase: 0,
            event: None,
        })
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 3 {
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
            if self.phase == 0 {
                if state.dom.borrow().file_picker_snapshot(node)?.as_ref() != Some(&self.expected) {
                    self.cancel();
                    return Ok(true);
                }
                self.changed = self.selected.as_ref().is_some_and(|files| {
                    state
                        .dom
                        .borrow()
                        .arena()
                        .get(node)
                        .is_ok_and(|record| record.selected_files.as_ref() != files.as_ref())
                });
                if self.changed {
                    let mut dom = state.dom.borrow_mut();
                    dom.set_selected_files(node, self.selected.as_ref().unwrap().clone())?;
                    dom.take_control_change(node)?;
                    dom.set_user_validity(node, true)?;
                }
                self.phase = 1;
                state.dom.borrow_mut().close_file_picker(node)?;
            } else {
                if self.event.is_none() {
                    let name = if !self.changed {
                        "cancel"
                    } else if self.phase == 1 {
                        "input"
                    } else {
                        "change"
                    };
                    let event = create_event_object(scope, &state, name, true, false)?;
                    let id = event_id(scope, event)?;
                    {
                        let mut records = state.events.borrow_mut();
                        let record = records
                            .get_mut(&id)
                            .ok_or(V8BindingError::Allocation("file picker event"))?;
                        record.is_trusted = true;
                        record.composed = name == "input";
                    }
                    self.event = Some(retained_dom_event::Event::new(
                        scope, &state, node, name, event,
                    )?);
                }
                if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                    return Ok(false);
                }
                self.event = None;
                if self.changed {
                    self.phase += 1
                } else {
                    self.phase = 3
                }
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(event) = self.event.take() {
            event.cancel(&self.owner)
        }
        self.selected = None;
        self.phase = 3;
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.cancel()
    }
}
pub(super) fn complete(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    expected: &oryn_dom::FilePicker,
    files: Option<&[crate::accessibility::PickerFile]>,
) -> Result<(), V8BindingError> {
    let mut job = Completion::new(state, node, expected, files)?;
    if !job.poll(scope)? {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
