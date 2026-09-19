//! One OS text operation, owning every callback and its native edit commit.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    Start,
    PrepareEdit,
    Update,
    Before,
    Edit,
    After,
    Input,
    Complete,
    Finish,
    Command,
    Snapshot,
    Interrupted,
    Done,
}
pub struct NativeComposition {
    owner: Rc<V8BindingState>,
    token: Rc<()>,
    identity: (NodeId, u64, u64, u64),
    target: NodeId,
    operation: Operation,
    phase: Phase,
    session: Option<Session>,
    event: Option<retained_dom_event::Event>,
    input: Option<clipboard_actions::ClipboardNotification>,
    finish: Option<finish::Finish>,
    edit: Option<retained_editing::Transaction>,
    command: Option<native_keyboard::TextCommandReceipt>,
    text: Vec<u16>,
    value: String,
    before: Vec<u16>,
    range: TextRange,
    selected: Option<TextRange>,
    marked: bool,
    mutated: bool,
    needs_start: bool,
    notify_interrupted: bool,
    result: Option<StateSnapshot>,
    terminal: bool,
    edit_cursor: Option<oryn_dom::MutationCursor>,
}
impl NativeComposition {
    pub fn new(
        state: &Rc<V8BindingState>,
        target: NodeId,
        operation: &Operation,
    ) -> Result<Self, V8BindingError> {
        if !operation.validate() || state.composition.active.borrow().is_some() {
            return Err(error());
        }
        let token = Rc::new(());
        *state.composition.active.borrow_mut() = Some(token.clone());
        state.composition.operation_target.set(Some(target));
        let session = state.composition.session.borrow_mut().take();
        Ok(Self {
            owner: state.clone(),
            token,
            identity: identity(state),
            target,
            operation: operation.clone(),
            phase: Phase::Prepare,
            session,
            event: None,
            input: None,
            finish: None,
            edit: None,
            command: None,
            text: Vec::new(),
            value: String::new(),
            before: Vec::new(),
            range: TextRange {
                start: 0,
                length: 0,
            },
            selected: None,
            marked: false,
            mutated: false,
            needs_start: false,
            notify_interrupted: false,
            result: None,
            terminal: false,
            edit_cursor: None,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    fn owns(&self) -> bool {
        self.owner
            .composition
            .active
            .borrow()
            .as_ref()
            .is_some_and(|token| Rc::ptr_eq(token, &self.token))
    }
    fn current(&self) -> bool {
        self.owns()
            && !self.owner.execution_retired()
            && !self.owner.execution_tasks_stopped()
            && identity(&self.owner) == self.identity
    }
    pub fn retry_safe(&self) -> bool {
        self.current() && !self.terminal && !matches!(self.phase, Phase::Done)
    }
    fn unlock(&mut self) {
        if self.owns() {
            self.owner.composition.active.borrow_mut().take();
            self.owner.composition.operation_target.set(None);
        }
    }
    pub fn cancel(&mut self) {
        self.event.take();
        self.input.take();
        self.finish.take();
        self.edit.take();
        self.command.take();
        self.session.take();
        self.unlock();
        self.terminal = true;
    }
    fn interrupt(&mut self, notify: bool) {
        self.session.take();
        self.event.take();
        self.input.take();
        self.notify_interrupted = notify;
        self.phase = Phase::Interrupted;
    }
    fn session_unchanged(&self) -> Result<bool, V8BindingError> {
        Ok(self
            .session
            .as_ref()
            .is_none_or(|session| composition_mutations_unchanged(&self.owner, session.cursor))
            && live(&self.owner, self.target, self.identity))
    }
    fn event(
        &mut self,
        scope: &mut v8::PinScope,
        name: &str,
        data: &[u16],
    ) -> Result<bool, V8BindingError> {
        if self.event.is_none() {
            let event = create_native_input_event(
                scope,
                &self.owner,
                "CompositionEvent",
                name,
                true,
                false,
                true,
                0,
                None,
                &[("data", NativeInputValue::String(data))],
            )?;
            self.event = Some(retained_dom_event::Event::new(
                scope,
                &self.owner,
                self.target,
                name,
                event,
            )?);
        }
        if self
            .event
            .as_mut()
            .unwrap()
            .poll(scope, &self.owner)?
            .is_none()
        {
            return Ok(false);
        }
        self.event.take();
        Ok(true)
    }
    pub fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<StateSnapshot>, V8BindingError> {
        if let Some(result) = &self.result {
            return Ok(Some(result.clone()));
        }
        if self.terminal {
            return Err(error());
        }
        loop {
            if !self.current() {
                self.cancel();
                return Err(error());
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                Phase::Prepare => {
                    resume_editing_recovery(&state)?;
                    if !live(&state, self.target, self.identity)
                        || !clipboard_actions::editable(&state, self.target)?
                    {
                        if self.session.as_ref().is_some_and(|session| {
                            session.target == self.target && session.identity == self.identity
                        }) {
                            self.interrupt(true);
                            continue;
                        }
                        self.cancel();
                        return Err(error());
                    }
                    if let Some(session) = &self.session {
                        if session.target != self.target
                            || session.identity != self.identity
                            || session.expected != current(&state, self.target)?.0
                            || !composition_mutations_unchanged(&state, session.cursor)
                        {
                            let notify =
                                session.target == self.target && session.identity == self.identity;
                            self.interrupt(notify);
                            continue;
                        }
                    }
                    match &self.operation {
                        Operation::Read | Operation::Finish { keep_marked: true } => {
                            self.phase = Phase::Snapshot
                        }
                        Operation::Command(_)
                        | Operation::Unmark
                        | Operation::Cancel
                        | Operation::Finish { keep_marked: false } => {
                            if let Some(session) = self.session.take() {
                                self.finish = Some(finish::Finish::new(
                                    &state,
                                    session,
                                    matches!(self.operation, Operation::Cancel),
                                ));
                                self.phase = Phase::Finish;
                            } else {
                                self.phase = if matches!(self.operation, Operation::Command(_)) {
                                    Phase::Command
                                } else {
                                    Phase::Snapshot
                                };
                            }
                        }
                        Operation::SetMarked { text, selected, .. } => {
                            self.text = text.clone();
                            self.value = oryn_dom::dom_string::from_units(text);
                            self.selected = Some(*selected);
                            self.marked = true;
                            self.phase = Phase::Start;
                        }
                        Operation::Commit { text, .. } => {
                            self.text = text.clone();
                            self.value = oryn_dom::dom_string::from_units(text);
                            self.phase = Phase::PrepareEdit;
                        }
                    }
                }
                Phase::Start => {
                    if self.session.is_none() {
                        let (value, selection) = current(&state, self.target)?;
                        let before_value = oryn_dom::dom_string::from_units(&value);
                        let before_selection = state
                            .text_control_selections
                            .borrow()
                            .get(&self.target)
                            .cloned()
                            .unwrap_or((
                                selection.start,
                                selection.end().ok_or_else(error)?,
                                "none".into(),
                            ));
                        let before = if state.dom.borrow().is_editable_text_control(self.target) {
                            None
                        } else {
                            editing_snapshot_budget(&state, &[self.target])?;
                            Some(capture_editing_snapshot(&state, self.target)?)
                        };
                        let flags = {
                            let dom = state.dom.borrow();
                            let node = dom.arena().get(self.target)?;
                            (
                                node.dirty_value,
                                node.user_edited_value,
                                node.pending_edit_value.clone(),
                            )
                        };
                        let replacement = match &self.operation {
                            Operation::SetMarked { replacement, .. } => *replacement,
                            _ => None,
                        };
                        self.session = Some(Session {
                            native_segments: Vec::new(),
                            target: self.target,
                            identity: self.identity,
                            before,
                            before_value,
                            before_selection,
                            before_control_flags: flags,
                            range: replacement.unwrap_or(selection),
                            expected: value,
                            data: Vec::new(),
                            cursor: state.dom.borrow().mutation_cursor(),
                        });
                        self.needs_start = true;
                    }
                    // Creating the event can fail after session capture. Keep
                    // that admission pending until its event exists/completes.
                    if self.needs_start && self.event.is_none() {
                        let event = create_native_input_event(
                            scope,
                            &state,
                            "CompositionEvent",
                            "compositionstart",
                            true,
                            false,
                            true,
                            0,
                            None,
                            &[("data", NativeInputValue::String(&[]))],
                        )?;
                        self.event = Some(retained_dom_event::Event::new(
                            scope,
                            &state,
                            self.target,
                            "compositionstart",
                            event,
                        )?);
                    }
                    if let Some(event) = &mut self.event {
                        if event.poll(scope, &state)?.is_none() {
                            return Ok(None);
                        }
                        self.event.take();
                    }
                    self.needs_start = false;
                    self.phase = Phase::PrepareEdit;
                }
                Phase::PrepareEdit => {
                    if !self.session_unchanged()? {
                        self.interrupt(self.session.is_some());
                        continue;
                    }
                    let (before, selection) = current(&state, self.target)?;
                    self.before = before;
                    let replacement = match &self.operation {
                        Operation::SetMarked { replacement, .. }
                        | Operation::Commit { replacement, .. } => *replacement,
                        _ => None,
                    };
                    self.range = replacement
                        .or_else(|| self.session.as_ref().map(|session| session.range))
                        .unwrap_or(selection);
                    if self
                        .range
                        .end()
                        .is_none_or(|end| end as usize > self.before.len())
                    {
                        self.cancel();
                        return Err(error());
                    }
                    self.phase = if self.marked {
                        Phase::Update
                    } else {
                        Phase::Before
                    };
                }
                Phase::Update => {
                    let text = self.text.clone();
                    if !self.event(scope, "compositionupdate", &text)? {
                        return Ok(None);
                    }
                    self.phase = Phase::Before;
                }
                Phase::Before => {
                    if self.input.is_none() {
                        if !self.session_unchanged()? {
                            self.interrupt(self.session.is_some());
                            continue;
                        }
                        self.input = Some(clipboard_actions::ClipboardNotification::composition(
                            scope,
                            &state,
                            self.target,
                            "beforeinput",
                            if self.session.is_some() {
                                "insertCompositionText"
                            } else {
                                "insertText"
                            },
                            &self.value,
                            self.session.is_some(),
                            self.session.is_none(),
                        )?);
                    }
                    let Some(accepted) = self.input.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.input.take();
                    if !accepted {
                        self.session.take();
                        self.phase = Phase::Snapshot;
                    } else {
                        self.phase = Phase::Edit;
                    }
                }
                Phase::Edit => {
                    if !self.mutated && self.edit.is_none() {
                        if !self.session_unchanged()?
                            || current(&state, self.target)?.0 != self.before
                        {
                            if self.session.is_some() {
                                self.interrupt(true);
                                continue;
                            }
                            self.cancel();
                            return Err(error());
                        }
                        select(&state, self.target, self.range)?;
                        if state.dom.borrow().is_editable_text_control(self.target) {
                            if self.session.is_some() {
                                let caret = state.dom.borrow_mut().edit_control_value(
                                    self.target,
                                    self.range.start,
                                    self.range.end().unwrap(),
                                    &self.value,
                                )?;
                                state
                                    .text_control_selections
                                    .borrow_mut()
                                    .insert(self.target, (caret, caret, "none".into()));
                                state
                                    .selection_revision
                                    .set(state.selection_revision.get().wrapping_add(1));
                            } else {
                                edit_native_control(
                                    &state,
                                    self.target,
                                    self.range.start,
                                    self.range.end().unwrap(),
                                    &self.value,
                                )?;
                            }
                            self.mutated = true;
                        } else {
                            let mut edit = retained_editing::Transaction::new(
                                &state,
                                if self.session.is_some() {
                                    "insertCompositionText"
                                } else {
                                    "insertText"
                                },
                            )?;
                            if self.session.is_some() {
                                edit.defer_history();
                            }
                            self.edit = Some(edit);
                        }
                    }
                    if let Some(edit) = &mut self.edit {
                        let Some(_) = edit.poll_owned(
                            scope,
                            &state,
                            editing_commands::Command::Text(self.value.clone()),
                        )?
                        else {
                            return Ok(None);
                        };
                        if let Some(session) = self.session.as_mut() {
                            session.native_segments.extend(edit.take_segments());
                        }
                        self.mutated = true;
                        if self.session.is_some() && self.edit_cursor.is_none() {
                            self.edit_cursor = Some(state.dom.borrow().mutation_cursor());
                        }
                    } else {
                        if self.session.is_some() && self.edit_cursor.is_none() {
                            self.edit_cursor = Some(state.dom.borrow().mutation_cursor());
                        }
                        complete_editing_range_clones(scope, &state)?;
                        synchronize_live_ranges(&state)?;
                    }
                    #[cfg(test)]
                    if FAIL_AFTER_NATIVE_EDIT.with(|flag| flag.replace(false)) {
                        return Err(V8BindingError::Allocation(
                            "composition native edit completion fault",
                        ));
                    }
                    if self.session.as_ref().is_some_and(|session| {
                        session
                            .native_segments
                            .iter()
                            .fold(0usize, |bytes, segment| {
                                bytes.saturating_add(segment.bytes())
                            })
                            > UndoHistory::MAX_BYTES
                    }) {
                        // The current edit is already committed. Interrupt the
                        // marked session without restoring authored state; the
                        // retained end notification is delivered on retry.
                        self.interrupt(true);
                        return Err(V8BindingError::Unsupported {
                            interface: "Document".into(),
                            member: "QuotaExceededError".into(),
                        });
                    }
                    self.phase = Phase::After;
                }
                Phase::After => {
                    if self.session.is_some()
                        && self
                            .edit_cursor
                            .is_none_or(|cursor| !composition_mutations_unchanged(&state, cursor))
                    {
                        self.interrupt(true);
                        continue;
                    }
                    if !live(&state, self.target, self.identity) {
                        self.interrupt(self.session.is_some());
                        continue;
                    }
                    let after = current(&state, self.target)?.0;
                    let inserted = after
                        .len()
                        .checked_sub(self.before.len() - self.range.length as usize)
                        .ok_or_else(error)?;
                    let range = TextRange {
                        start: self.range.start,
                        length: u32::try_from(inserted).map_err(|_| error())?,
                    };
                    if let Some(selected) = self.selected {
                        if selected.end().is_none_or(|end| end as usize > inserted) {
                            self.cancel();
                            return Err(error());
                        }
                        select(
                            &state,
                            self.target,
                            TextRange {
                                start: self.range.start + selected.start,
                                length: selected.length,
                            },
                        )?;
                    }
                    if let Some(session) = &mut self.session {
                        session.range = range;
                        session.expected = after;
                        session.data = self.text.clone();
                        session.cursor = state.dom.borrow().mutation_cursor();
                    }
                    self.phase = Phase::Input;
                }
                Phase::Input => {
                    if self.input.is_none() {
                        self.input = Some(clipboard_actions::ClipboardNotification::composition(
                            scope,
                            &state,
                            self.target,
                            "input",
                            if self.session.is_some() {
                                "insertCompositionText"
                            } else {
                                "insertText"
                            },
                            &self.value,
                            self.session.is_some(),
                            false,
                        )?);
                    }
                    if self.input.as_mut().unwrap().poll(scope, &state)?.is_none() {
                        return Ok(None);
                    }
                    self.input.take();
                    self.phase = Phase::Complete;
                }
                Phase::Complete => {
                    if !self.session_unchanged()?
                        || self.session.as_ref().is_some_and(|session| {
                            current(&state, self.target)
                                .is_ok_and(|value| value.0 != session.expected)
                        })
                    {
                        self.interrupt(self.session.is_some());
                        continue;
                    }
                    if !self.marked {
                        if let Some(session) = self.session.take() {
                            self.finish = Some(finish::Finish::new(&state, session, false));
                            self.phase = Phase::Finish;
                            continue;
                        }
                    }
                    self.phase = Phase::Snapshot;
                }
                Phase::Finish => {
                    if !self.finish.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.finish.take();
                    self.phase = if matches!(self.operation, Operation::Command(_)) {
                        Phase::Command
                    } else {
                        Phase::Snapshot
                    };
                }
                Phase::Command => {
                    if self.command.is_none() {
                        if !live(&state, self.target, self.identity)
                            || !clipboard_actions::editable(&state, self.target)?
                        {
                            self.interrupt(false);
                            continue;
                        }
                        let Operation::Command(command) = self.operation else {
                            self.cancel();
                            return Err(error());
                        };
                        self.command = Some(native_keyboard::TextCommandReceipt::new(
                            state.clone(),
                            self.target,
                            command,
                        )?);
                    }
                    let command = self.command.as_mut().unwrap();
                    match command.poll(scope) {
                        Ok(true) => {}
                        Ok(false) => return Ok(None),
                        Err(failure) => {
                            if !command.retry_safe() {
                                self.terminal = true;
                            }
                            return Err(failure);
                        }
                    }
                    self.command.take();
                    if !live(&state, self.target, self.identity) {
                        self.interrupt(false);
                    } else {
                        self.phase = Phase::Snapshot;
                    }
                }
                Phase::Snapshot => {
                    let result = snapshot(
                        &state,
                        self.target,
                        self.session.as_ref().map(|session| session.range),
                    )?;
                    *state.composition.session.borrow_mut() = self.session.take();
                    self.result = Some(result.clone());
                    self.phase = Phase::Done;
                    self.unlock();
                    return Ok(Some(result));
                }
                Phase::Interrupted => {
                    if self.notify_interrupted
                        && (self.event.is_some() || live(&state, self.target, self.identity))
                    {
                        if !self.event(scope, "compositionend", &[])? {
                            return Ok(None);
                        }
                    }
                    self.notify_interrupted = false;
                    let mut result = if live(&state, self.target, self.identity) {
                        snapshot(&state, self.target, None)?
                    } else {
                        StateSnapshot {
                            mutation_revision: state.dom.borrow().mutation_cursor().0,
                            aborted: true,
                            protected: true,
                            text: Vec::new(),
                            selection: TextRange {
                                start: 0,
                                length: 0,
                            },
                            marked: None,
                        }
                    };
                    result.aborted = true;
                    self.result = Some(result.clone());
                    self.phase = Phase::Done;
                    self.unlock();
                    return Ok(Some(result));
                }
                Phase::Done => return Ok(self.result.clone()),
            }
        }
    }
}
impl Drop for NativeComposition {
    fn drop(&mut self) {
        if !matches!(self.phase, Phase::Done) {
            self.cancel();
        }
    }
}
#[cfg(test)]
thread_local! {static FAIL_AFTER_NATIVE_EDIT:Cell<bool>=const{Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        super::super::super::test_eval::eval(scope, source);
    }
    #[test]
    fn intervening_edit_after_native_commit_cannot_become_marked_session_content() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(242), 1).unwrap();
        eval(
            scope,
            "t.focus();t.setSelectionRange(1,2);globalThis.ends=0;t.oncompositionend=()=>ends++",
        );
        let target = state.dom.borrow().focused().unwrap();
        let mut job = NativeComposition::new(
            &state,
            target,
            &Operation::SetMarked {
                text: vec![88],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        FAIL_AFTER_NATIVE_EDIT.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        eval(
            scope,
            "if(t.value!=='aXc')throw Error('native commit missing');t.value='replacement'",
        );
        let result = job.poll(scope).unwrap().unwrap();
        assert!(result.aborted);
        assert!(result.marked.is_none());
        assert!(native_composition_target(&state).is_none());
        eval(
            scope,
            "if(t.value!=='replacement'||ends!==1)throw Error('composition stole intervening edit')",
        );
    }
    #[test]
    fn marked_operation_retains_all_event_suffixes_and_one_edit() {
        super::super::super::tests::initialize_v8();
        for control in [true, false] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(if control {
                    "<textarea id=t>abc</textarea>"
                } else {
                    "<div id=t contenteditable>abc</div>"
                })
                .into_runtime()
                .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(240), 1).unwrap();
            eval(
                scope,
                if control {
                    "t.focus();t.setSelectionRange(1,2)"
                } else {
                    "t.focus();getSelection().setBaseAndExtent(t.firstChild,1,t.firstChild,2)"
                },
            );
            eval(
                scope,
                "globalThis.trace=[];for(const k of ['compositionstart','compositionupdate','beforeinput','input'])t.addEventListener(k,()=>trace.push(k));",
            );
            let target = state.dom.borrow().find_by_id("t").unwrap();
            let mut job = NativeComposition::new(
                &state,
                target,
                &Operation::SetMarked {
                    text: vec![88],
                    selected: TextRange {
                        start: 1,
                        length: 0,
                    },
                    replacement: None,
                },
            )
            .unwrap();
            for _ in 0..4 {
                native_platform::publication::fail_after_next_callback();
                assert!(job.poll(scope).is_err());
                assert!(job.retry_safe());
                assert!(NativeComposition::new(&state, target, &Operation::Read).is_err());
                state.set_execution_suspended(true);
                assert!(job.poll(scope).unwrap().is_none());
                state.set_execution_suspended(false);
            }
            let result = job.poll(scope).unwrap().unwrap();
            assert_eq!(result.text, vec![97, 88, 99]);
            assert_eq!(
                result.marked,
                Some(TextRange {
                    start: 1,
                    length: 1
                })
            );
            assert_eq!(job.poll(scope).unwrap(), Some(result));
            eval(
                scope,
                "if(trace.join(',')!=='compositionstart,compositionupdate,beforeinput,input')throw Error('composition prefix replay');",
            );
            apply(scope, &state, target, &Operation::Unmark).unwrap();
            if control {
                assert!(native_keyboard::undo_control(&state, target, false).unwrap());
                assert!(!native_keyboard::undo_control(&state, target, false).unwrap());
            } else {
                assert!(undo_edit(&state, false).unwrap());
                assert!(!undo_edit(&state, false).unwrap());
            }
            eval(
                scope,
                "if((t.value??t.textContent)!=='abc')throw Error('composition undo not atomic');",
            );
        }
    }
    #[test]
    fn command_finishes_marked_session_then_resumes_edit_suffix_and_shared_undo() {
        super::super::super::tests::initialize_v8();
        for control in [true, false] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(if control {
                    "<textarea id=t>abc</textarea>"
                } else {
                    "<div id=t contenteditable>abc</div>"
                })
                .into_runtime()
                .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(243), 1).unwrap();
            eval(
                scope,
                if control {
                    "t.focus();t.setSelectionRange(1,2)"
                } else {
                    "t.focus();getSelection().setBaseAndExtent(t.firstChild,1,t.firstChild,2)"
                },
            );
            let target = state.dom.borrow().find_by_id("t").unwrap();
            apply(
                scope,
                &state,
                target,
                &Operation::SetMarked {
                    text: vec![88],
                    selected: TextRange {
                        start: 1,
                        length: 0,
                    },
                    replacement: None,
                },
            )
            .unwrap();
            eval(
                scope,
                "globalThis.trace=[];t.addEventListener('compositionend',e=>trace.push('end:'+e.data),{once:true});t.addEventListener('beforeinput',e=>{globalThis.original=e;trace.push('before')});t.addEventListener('beforeinput',e=>{if(e!==original)throw Error('event replaced');trace.push('suffix')});t.addEventListener('input',()=>trace.push('input'));",
            );
            let mut job = NativeComposition::new(
                &state,
                target,
                &Operation::Command(crate::composition::TextCommand::Delete {
                    backward: false,
                    unit: crate::composition::TextUnit::Character,
                }),
            )
            .unwrap();
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
            assert!(job.retry_safe());
            eval(
                scope,
                "if((t.value??t.textContent)!=='aXc'||trace.join()!=='end:X')throw Error('command ran before composition finished: '+JSON.stringify({value:t.value,text:t.textContent,trace}))",
            );
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
            assert!(job.retry_safe());
            eval(
                scope,
                "if((t.value??t.textContent)!=='aXc'||trace.join()!=='end:X,before')throw Error('command mutated before accepted input')",
            );
            assert!(NativeComposition::new(&state, target, &Operation::Read).is_err());
            state.set_execution_suspended(true);
            assert!(job.poll(scope).unwrap().is_none());
            state.set_execution_suspended(false);
            let result = job.poll(scope).unwrap().unwrap();
            assert_eq!(result.text, vec![97, 88]);
            assert!(result.marked.is_none());
            assert_eq!(job.poll(scope).unwrap(), Some(result));
            eval(
                scope,
                "if(trace.join()!=='end:X,before,suffix,input')throw Error('command callback prefix repeated')",
            );
            // The command and the completed marked edit use the same native
            // undo owner, with one unit each despite both interrupted events.
            if control {
                assert!(native_keyboard::undo_control(&state, target, false).unwrap());
            } else {
                assert!(undo_edit(&state, false).unwrap());
            }
            eval(
                scope,
                "if((t.value??t.textContent)!=='aXc')throw Error('command undo missing')",
            );
            if control {
                assert!(native_keyboard::undo_control(&state, target, false).unwrap());
                assert!(!native_keyboard::undo_control(&state, target, false).unwrap());
            } else {
                assert!(undo_edit(&state, false).unwrap());
                assert!(!undo_edit(&state, false).unwrap());
            }
            eval(
                scope,
                "if((t.value??t.textContent)!=='abc')throw Error('composition undo duplicated')",
            );
        }
    }
    #[test]
    fn abandoned_old_receipt_cannot_unlock_a_new_operation() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(241), 1).unwrap();
        eval(scope, "t.focus()");
        let target = state.dom.borrow().focused().unwrap();
        let old = NativeComposition::new(&state, target, &Operation::Read).unwrap();
        retire(&state);
        let mut current = NativeComposition::new(&state, target, &Operation::Read).unwrap();
        drop(old);
        assert!(NativeComposition::new(&state, target, &Operation::Read).is_err());
        assert!(current.poll(scope).unwrap().is_some());
        assert!(state.composition.active.borrow().is_none());
    }
}
