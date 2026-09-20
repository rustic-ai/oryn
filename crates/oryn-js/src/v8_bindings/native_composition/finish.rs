//! Commit/cancel owns its native mutation and compositionend continuation.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Validate,
    Commit,
    RestoreFlags,
    Inverse,
    End,
    Done,
}
pub(super) struct Finish {
    owner: Rc<V8BindingState>,
    session: Session,
    cancel: bool,
    phase: Phase,
    inverse_remaining: usize,
    event: Option<retained_dom_event::Event>,
}
impl Finish {
    pub(super) fn new(state: &Rc<V8BindingState>, session: Session, cancel: bool) -> Self {
        Self {
            inverse_remaining: session.native_segments.len(),
            owner: state.clone(),
            session,
            cancel,
            phase: Phase::Validate,
            event: None,
        }
    }
    fn current(&self) -> bool {
        !self.owner.execution_retired()
            && !self.owner.execution_tasks_stopped()
            && identity(&self.owner) == self.session.identity
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(true);
            }
            if !self.current() {
                self.event.take();
                return Err(error());
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.phase {
                Phase::Validate => {
                    if !live(&state, self.session.target, self.session.identity)
                        || current(&state, self.session.target)?.0 != self.session.expected
                        || !composition_mutations_unchanged(&state, self.session.cursor)
                    {
                        return Err(error());
                    }
                    self.phase = Phase::Commit;
                }
                Phase::Commit => {
                    // Snapshot failure can yield to page tasks before retry.
                    if !live(&state, self.session.target, self.session.identity)
                        || current(&state, self.session.target)?.0 != self.session.expected
                        || !composition_mutations_unchanged(&state, self.session.cursor)
                    {
                        return Err(error());
                    }
                    if self.cancel {
                        if self.session.before.is_some() {
                            self.phase = Phase::Inverse;
                        } else {
                            let mut dom = state.dom.borrow_mut();
                            let sensitive = dom.arena().get(self.session.target)?.sensitive_value;
                            dom.restore_control_edit_value(
                                self.session.target,
                                self.session.before_value.clone(),
                                sensitive,
                            )?;
                            self.phase = Phase::RestoreFlags;
                        }
                    } else if self.session.before.is_some() {
                        editing_snapshot_budget(&state, &[self.session.target])?;
                        let after = capture_editing_snapshot(&state, self.session.target)?;
                        let before = self.session.before.take().unwrap();
                        push_undo_unit(
                            &state,
                            UndoUnit {
                                native_progress: Default::default(),
                                native_segments: self.session.native_segments.clone(),
                                cross_document: None,
                                before,
                                after,
                                companions: Vec::new(),
                                identity: Rc::new(()),
                                control_owners: Vec::new(),
                                command: "insertCompositionText".into(),
                                split_nodes: None,
                            },
                        )?;
                        self.phase = Phase::End;
                    } else {
                        let (_, selection) = current(&state, self.session.target)?;
                        let after = state.dom.borrow().control_edit_value(self.session.target)?;
                        native_keyboard::record_control_edit(
                            &state,
                            self.session.target,
                            self.session.before_value.clone(),
                            self.session.before_selection.clone(),
                            after,
                            (
                                selection.start,
                                selection.end().ok_or_else(error)?,
                                "none".into(),
                            ),
                        );
                        self.phase = Phase::End;
                    }
                }
                Phase::Inverse => {
                    while self.inverse_remaining > 0 {
                        self.session.native_segments[self.inverse_remaining - 1]
                            .apply(&state, false)?;
                        self.inverse_remaining -= 1;
                    }
                    self.session.native_segments.clear();
                    self.session.before.take();
                    self.phase = Phase::End;
                }
                Phase::RestoreFlags => {
                    {
                        let mut dom = state.dom.borrow_mut();
                        let node = dom.arena_mut().get_mut(self.session.target)?;
                        node.dirty_value = self.session.before_control_flags.0;
                        node.user_edited_value = self.session.before_control_flags.1;
                        node.pending_edit_value = self.session.before_control_flags.2.clone();
                    }
                    state
                        .text_control_selections
                        .borrow_mut()
                        .insert(self.session.target, self.session.before_selection.clone());
                    state
                        .selection_revision
                        .set(state.selection_revision.get().wrapping_add(1));
                    self.phase = Phase::End;
                }
                Phase::End => {
                    if self.event.is_none() {
                        let data = if self.cancel {
                            &[][..]
                        } else {
                            self.session.data.as_slice()
                        };
                        let event = create_native_input_event(
                            scope,
                            &state,
                            NativeInputEventInputs {
                                interface: "CompositionEvent",
                                event_type: "compositionend",
                                bubbles: true,
                                cancelable: false,
                                composed: true,
                                detail: 0,
                                related: None,
                                fields: &[("data", NativeInputValue::String(data))],
                            },
                        )?;
                        self.event = Some(retained_dom_event::Event::new(
                            scope,
                            &state,
                            self.session.target,
                            "compositionend",
                            event,
                        )?);
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
    fn marked_beforeinput_retry_preserves_flags_and_listener_suffix() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(239), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.trace=[];t.addEventListener('beforeinput',e=>{trace.push([e.data,e.isComposing,e.cancelable,e.isTrusted]);e.preventDefault()});t.addEventListener('beforeinput',()=>trace.push('suffix'));",
        );
        let mut notification = clipboard_actions::ClipboardNotification::composition(
            scope,
            &state,
            clipboard_actions::CompositionNotificationInputs {
                target,
                name: "beforeinput",
                input_type: "insertCompositionText",
                text: "X",
                composing: true,
                cancelable: false,
            },
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(notification.poll(scope, &state).is_err());
        assert_eq!(notification.poll(scope, &state).unwrap(), Some(true));
        assert_eq!(notification.poll(scope, &state).unwrap(), Some(true));
        eval(
            scope,
            "if(JSON.stringify(trace)!=='[[\"X\",true,false,true],\"suffix\"]')throw Error('marked input receipt lost flags or replayed');",
        );
    }
    #[test]
    fn compositionend_retry_does_not_restore_again_or_repeat_undo() {
        super::super::super::tests::initialize_v8();
        for cancel in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<textarea id=t>abc</textarea>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(238), 1).unwrap();
            eval(
                scope,
                "t.focus();t.setSelectionRange(1,2);globalThis.ends=[];t.addEventListener('compositionend',e=>ends.push('first:'+e.data));t.addEventListener('compositionend',()=>ends.push('second'));",
            );
            let target = state.dom.borrow().focused().unwrap();
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
            let session = state.composition.session.borrow_mut().take().unwrap();
            let mut job = Finish::new(&state, session, cancel);
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
            assert_eq!(
                state.dom.borrow().control_value(target).unwrap(),
                if cancel { "abc" } else { "aXc" }
            );
            if cancel {
                eval(scope, "t.value='replacement'");
            }
            state.set_execution_suspended(true);
            assert!(!job.poll(scope).unwrap());
            state.set_execution_suspended(false);
            assert!(job.poll(scope).unwrap());
            assert!(job.poll(scope).unwrap());
            eval(
                scope,
                if cancel {
                    "if(t.value!=='replacement'||ends.join(',')!=='first:,second')throw Error('cancel completion replay');"
                } else {
                    "if(t.value!=='aXc'||ends.join(',')!=='first:X,second')throw Error('commit completion replay');"
                },
            );
            if !cancel {
                assert!(native_keyboard::undo_control(&state, target, false).unwrap());
                assert_eq!(state.dom.borrow().control_value(target).unwrap(), "abc");
                assert!(!native_keyboard::undo_control(&state, target, false).unwrap());
            }
        }
    }
}
