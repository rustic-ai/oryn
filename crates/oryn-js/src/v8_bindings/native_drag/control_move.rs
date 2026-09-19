//! A control move owns both beforeinput receipts and the native commit. Input
//! notification failure cannot replay deletion, insertion or the undo record.
use super::*;

#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    BeforeDelete,
    BeforeInsert,
    Commit,
    InputDelete,
    InputInsert,
    Done,
}

pub(super) struct ControlMove {
    owner: Rc<V8BindingState>,
    input_owner: native_pointer::InputOwner,
    source: SelectedSource,
    target: NodeId,
    caret: u32,
    text: String,
    phase: Phase,
    before_target: String,
    after_target: String,
    after_source: String,
    position: u32,
    target_cursor: Option<oryn_dom::MutationCursor>,
    event: Option<clipboard_actions::ClipboardNotification>,
    result: bool,
}

impl ControlMove {
    pub(super) fn set_input_owner(&mut self, state: &Rc<V8BindingState>) {
        self.input_owner = native_pointer::InputOwner::new(state);
    }
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        source: &SelectedSource,
        target: NodeId,
        caret: u32,
        text: &str,
    ) -> Self {
        Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            source: source.clone(),
            target,
            caret,
            text: text.into(),
            phase: Phase::Prepare,
            before_target: String::new(),
            after_target: String::new(),
            after_source: String::new(),
            position: 0,
            target_cursor: None,
            event: None,
            result: false,
        }
    }

    fn live(&self) -> bool {
        let state = &self.owner;
        self.input_owner.current()
            && !state.execution_retired()
            && !state.execution_tasks_stopped()
            && (
                self.source.document,
                self.source.generation,
                self.source.epoch,
                self.source.authority,
            ) == (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            )
            && [self.source_node(), self.target].into_iter().all(|node| {
                let dom = state.dom.borrow();
                dom.is_document_connected(node).unwrap_or(false)
                    && dom.arena().node_document(node).ok().flatten() == Some(self.source.document)
            })
    }

    fn source_node(&self) -> NodeId {
        match &self.source.kind {
            SelectedKind::Control { node, .. } => *node,
            _ => self.target,
        }
    }

    fn unchanged(&self) -> Result<bool, V8BindingError> {
        let state = &self.owner;
        if !self.source.move_target(state, self.target)
            || !clipboard_actions::editable(state, self.target)?
        {
            return Ok(false);
        }
        let dom = state.dom.borrow();
        if !dom.is_text_control(self.target)
            || dom.control_value(self.target)? != self.before_target
        {
            return Ok(false);
        }
        let Some(cursor) = self.target_cursor else {
            return Ok(false);
        };
        let batch = dom.mutations_since(cursor);
        Ok(!batch.truncated && !batch.mutations.iter().any(|mutation| matches!(mutation,
            oryn_dom::Mutation::FormState { node } | oryn_dom::Mutation::Attribute { node, .. }
            | oryn_dom::Mutation::DocumentOwnership { node, .. } | oryn_dom::Mutation::Removed { node, .. }
            if *node == self.target)))
    }

    pub(super) fn cancel(&mut self) {
        self.event.take();
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
                continue;
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            let SelectedKind::Control {
                node,
                start,
                end,
                value,
            } = self.source.kind.clone()
            else {
                self.cancel();
                continue;
            };
            match self.phase {
                Phase::Prepare => {
                    if !self.source.move_target(&state, self.target)
                        || !clipboard_actions::editable(&state, self.target)?
                    {
                        self.cancel();
                        continue;
                    }
                    self.before_target = state.dom.borrow().control_value(self.target)?;
                    self.target_cursor = Some(state.dom.borrow().mutation_cursor());
                    if node == self.target {
                        let Some((next, position)) =
                            moved_control_value(&value, start, end, self.caret, &self.text)
                        else {
                            self.cancel();
                            continue;
                        };
                        if next == value || value.len().saturating_add(next.len()) > 1024 * 1024 {
                            self.cancel();
                            continue;
                        }
                        self.after_target = next;
                        self.position = position;
                    } else {
                        let mut source = oryn_dom::dom_string::units(&value);
                        if start > end || end as usize > source.len() {
                            self.cancel();
                            continue;
                        }
                        source.drain(start as usize..end as usize);
                        self.after_source = oryn_dom::dom_string::from_units(&source);
                        let mut target = oryn_dom::dom_string::units(&self.before_target);
                        if self.caret as usize > target.len() {
                            self.cancel();
                            continue;
                        }
                        let payload = oryn_dom::dom_string::units(&self.text);
                        self.position = self.caret.saturating_add(payload.len() as u32);
                        target.splice(self.caret as usize..self.caret as usize, payload);
                        self.after_target = oryn_dom::dom_string::from_units(&target);
                        if [
                            value.len(),
                            self.before_target.len(),
                            self.after_source.len(),
                            self.after_target.len(),
                        ]
                        .into_iter()
                        .fold(0usize, usize::saturating_add)
                            > 512 * 1024
                        {
                            self.cancel();
                            continue;
                        }
                    }
                    if state
                        .dom
                        .borrow()
                        .control_length_limit(self.target, "maxlength")?
                        .is_some_and(|max| oryn_dom::dom_string::len(&self.after_target) > max)
                    {
                        self.cancel();
                        continue;
                    }
                    state
                        .text_control_selections
                        .borrow_mut()
                        .insert(node, (start, end, "none".into()));
                    self.phase = Phase::BeforeDelete;
                }
                Phase::BeforeDelete
                | Phase::BeforeInsert
                | Phase::InputDelete
                | Phase::InputInsert => {
                    let before = matches!(self.phase, Phase::BeforeDelete | Phase::BeforeInsert);
                    let deletion = matches!(self.phase, Phase::BeforeDelete | Phase::InputDelete);
                    if self.event.is_none() {
                        if before && !self.unchanged()? {
                            self.cancel();
                            continue;
                        }
                        self.event = Some(clipboard_actions::ClipboardNotification::editing(
                            scope,
                            &state,
                            if deletion { node } else { self.target },
                            if before { "beforeinput" } else { "input" },
                            if deletion {
                                "deleteByDrag"
                            } else {
                                "insertFromDrop"
                            },
                            (!deletion).then_some(self.text.as_str()),
                            None,
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.event.take();
                    // Advance before any fallible validation: the receipt has
                    // completed and must never be recreated after an error.
                    self.phase = match self.phase {
                        Phase::BeforeDelete => Phase::BeforeInsert,
                        Phase::BeforeInsert => Phase::Commit,
                        Phase::InputDelete => Phase::InputInsert,
                        _ => Phase::Done,
                    };
                    if before && !accepted {
                        self.cancel();
                        continue;
                    }
                    if before && deletion {
                        // Validation is repeated at the next phase before its
                        // event is created, including after an interrupted check.
                        state
                            .text_control_selections
                            .borrow_mut()
                            .insert(self.target, (self.caret, self.caret, "none".into()));
                    }
                }
                Phase::Commit => {
                    if !self.unchanged()? {
                        self.cancel();
                        continue;
                    }
                    if node == self.target {
                        state
                            .text_control_selections
                            .borrow_mut()
                            .insert(node, (start, end, "none".into()));
                        edit_native_control(
                            &state,
                            node,
                            0,
                            oryn_dom::dom_string::len(&value) as u32,
                            &self.after_target,
                        )?;
                        native_keyboard::finish_control_move_selection(&state, node, self.position);
                    } else {
                        // There are no author callbacks in this transaction.
                        // Normalize and validate insertion before source deletion.
                        let (sensitive, edited, pending) = {
                            let dom = state.dom.borrow();
                            let target = dom.arena().get(self.target)?;
                            (
                                target.sensitive_value,
                                target.user_edited_value,
                                target.pending_edit_value.clone(),
                            )
                        };
                        let outcome = (|| {
                            let mut dom = state.dom.borrow_mut();
                            dom.edit_control_value(
                                self.target,
                                0,
                                oryn_dom::dom_string::len(&self.before_target) as u32,
                                &self.after_target,
                            )?;
                            if dom.control_value(self.target)? != self.after_target {
                                return Ok(false);
                            }
                            dom.edit_control_value(node, start, end, "")?;
                            Ok::<_, V8BindingError>(true)
                        })();
                        if !matches!(outcome, Ok(true)) {
                            // Even rollback failure is terminal. Re-admitting
                            // this job could repeat a partially committed edit.
                            self.cancel();
                            let mut dom = state.dom.borrow_mut();
                            dom.set_value(self.target, self.before_target.clone(), sensitive)?;
                            let target = dom.arena_mut().get_mut(self.target)?;
                            target.user_edited_value = edited;
                            target.pending_edit_value = pending;
                            drop(dom);
                            return outcome.map(Some);
                        }
                        // The bound was checked before callbacks and changes
                        // are frozen, so this native history publication cannot
                        // fail the budget check after the commit.
                        native_keyboard::record_control_move(
                            &state,
                            vec![
                                (
                                    node,
                                    value,
                                    (start, end, "none".into()),
                                    self.after_source.clone(),
                                    (start, start, "none".into()),
                                ),
                                (
                                    self.target,
                                    self.before_target.clone(),
                                    (self.caret, self.caret, "none".into()),
                                    self.after_target.clone(),
                                    (self.position, self.position, "none".into()),
                                ),
                            ],
                        )?;
                        state
                            .text_control_selections
                            .borrow_mut()
                            .insert(node, (start, start, "none".into()));
                        state
                            .text_control_selections
                            .borrow_mut()
                            .insert(self.target, (self.position, self.position, "none".into()));
                    }
                    self.result = true;
                    self.phase = Phase::InputDelete;
                }
                Phase::Done => return Ok(Some(self.result)),
            }
        }
    }
}

impl Drop for ControlMove {
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
    fn snapshot(state: &V8BindingState, node: NodeId) -> SelectedSource {
        SelectedSource {
            kind: SelectedKind::Control {
                node,
                start: 1,
                end: 3,
                value: "abcdef".into(),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([node]),
        }
    }

    #[test]
    fn interrupted_control_moves_commit_once_and_keep_one_shared_undo() {
        super::super::super::tests::initialize_v8();
        for same in [true, false] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<input id=s value=abcdef><input id=t value=XY>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(228), 1).unwrap();
            let source = state.dom.borrow().find_by_id("s").unwrap();
            let target = if same {
                source
            } else {
                state.dom.borrow().find_by_id("t").unwrap()
            };
            eval(
                scope,
                "globalThis.trace=[];for(const n of document.querySelectorAll('input'))for(const k of ['beforeinput','input'])n.addEventListener(k,e=>trace.push(n.id+':'+k+':'+e.inputType));",
            );
            let mut job = ControlMove::new(
                &state,
                &snapshot(&state, source),
                target,
                if same { 6 } else { 2 },
                "bc",
            );
            for _ in 0..4 {
                native_platform::publication::fail_after_next_callback();
                assert!(job.poll(scope).is_err());
                state.set_execution_suspended(true);
                assert_eq!(job.poll(scope).unwrap(), None);
                state.set_execution_suspended(false);
            }
            assert_eq!(job.poll(scope).unwrap(), Some(true));
            assert_eq!(job.poll(scope).unwrap(), Some(true));
            assert_eq!(
                state.dom.borrow().control_value(source).unwrap(),
                if same { "adefbc" } else { "adef" }
            );
            assert_eq!(
                state.dom.borrow().control_value(target).unwrap(),
                if same { "adefbc" } else { "XYbc" }
            );
            eval(
                scope,
                if same {
                    "if(trace.join(',')!=='s:beforeinput:deleteByDrag,s:beforeinput:insertFromDrop,s:input:deleteByDrag,s:input:insertFromDrop')throw Error('replayed move notification');"
                } else {
                    "if(trace.join(',')!=='s:beforeinput:deleteByDrag,t:beforeinput:insertFromDrop,s:input:deleteByDrag,t:input:insertFromDrop')throw Error('replayed paired notification');"
                },
            );
            assert!(native_keyboard::undo_control(&state, target, false).unwrap());
            assert_eq!(state.dom.borrow().control_value(source).unwrap(), "abcdef");
            if !same {
                assert_eq!(state.dom.borrow().control_value(target).unwrap(), "XY");
            }
            assert!(!native_keyboard::undo_control(&state, target, false).unwrap());
        }
    }

    #[test]
    fn source_mutation_during_beforeinput_cancels_without_deleting_replacement() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=s value=abcdef><input id=t value=XY>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(229), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.trace=[];const s=document.getElementById('s');s.addEventListener('beforeinput',()=>{trace.push('delete');s.value='replacement'});document.getElementById('t').addEventListener('beforeinput',()=>trace.push('insert'));",
        );
        let mut job = ControlMove::new(&state, &snapshot(&state, source), target, 2, "bc");
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert_eq!(job.poll(scope).unwrap(), Some(false));
        assert_eq!(
            state.dom.borrow().control_value(source).unwrap(),
            "replacement"
        );
        assert_eq!(state.dom.borrow().control_value(target).unwrap(), "XY");
        eval(
            scope,
            "if(trace.join(',')!=='delete')throw Error('stale move reached destination');",
        );
    }
}
