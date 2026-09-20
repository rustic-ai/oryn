//! Host-owned drag command. Strong event/realm owners belong to this receipt,
//! never to their own V8BindingState (which would form a reference cycle).
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Begin,
    Advance,
    Drop,
    Cancel,
}
#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    Begin,
    Advance,
    Drop,
    Cancel,
    Done,
}
enum DefaultEdit {
    Copy(Box<drop_default::DropDefault>),
    Control(Box<control_move::ControlMove>),
    Rich(Box<rich_move::RichMove>),
    Rejected,
}
impl DefaultEdit {
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        match self {
            Self::Copy(job) => job.poll(scope),
            Self::Control(job) => job.poll(scope),
            Self::Rich(job) => job.poll(scope),
            Self::Rejected => Ok(Some(false)),
        }
    }
}
pub struct NativeDragInput {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    operation: Operation,
    target: Option<NodeId>,
    init: EventInit,
    phase: Phase,
    owns_dispatch: bool,
    session: Option<Session>,
    selected: Option<Rc<SelectedSource>>,
    begin: Option<begin::Begin>,
    advance: Option<advance::Advance>,
    drop: Option<drop_operation::DropOperation>,
    cancel: Option<cancel::Cancel>,
    edit: Option<DefaultEdit>,
    result: bool,
}
impl NativeDragInput {
    pub fn new(
        state: &Rc<V8BindingState>,
        operation: &str,
        target: Option<NodeId>,
        init: &EventInit,
    ) -> Result<Self, V8BindingError> {
        let operation = match operation {
            "begin" => Operation::Begin,
            "advance" => Operation::Advance,
            "drop" => Operation::Drop,
            "cancel" => Operation::Cancel,
            _ => return Err(V8BindingError::Allocation("unknown drag input operation")),
        };
        let owns_dispatch =
            !state.execution_retired() && !state.native_drag_dispatching.replace(true);
        let session = if owns_dispatch {
            state.native_drag.borrow_mut().take()
        } else {
            None
        };
        Ok(Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            operation,
            target,
            init: init.clone(),
            phase: if owns_dispatch {
                Phase::Prepare
            } else {
                Phase::Done
            },
            owns_dispatch,
            session,
            selected: None,
            begin: None,
            advance: None,
            drop: None,
            cancel: None,
            edit: None,
            result: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    fn current(&self) -> bool {
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
    }
    fn unlock(&mut self) {
        if self.owns_dispatch {
            let state = &self.owner;
            if self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
            {
                state.native_drag_dispatching.set(false);
            }
            self.owns_dispatch = false;
        }
    }
    fn finish(&mut self) {
        self.result = self
            .session
            .as_ref()
            .is_some_and(|session| session.live(&self.owner));
        if self.result && self.current() {
            *self.owner.native_drag.borrow_mut() = self.session.take();
        }
        self.unlock();
        self.phase = Phase::Done;
    }
    pub fn cancel(&mut self) {
        self.begin.take();
        self.advance.take();
        self.drop.take();
        self.cancel.take();
        self.edit.take();
        if let Some(mut session) = self.session.take() {
            session.abandon();
            native_drag_image::clear(&self.owner, session.feedback_session.take());
        }
        self.unlock();
        self.result = false;
        self.phase = Phase::Done;
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(Some(self.result));
            }
            if !self.current() {
                self.cancel();
                continue;
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                Phase::Prepare => match self.operation {
                    Operation::Begin => {
                        if self.session.is_some() {
                            self.finish();
                            continue;
                        }
                        let selected = self
                            .target
                            .map(|target| {
                                selected_source(
                                    &state,
                                    target,
                                    f64::from(self.init.client_x),
                                    f64::from(self.init.client_y),
                                )
                            })
                            .transpose()?
                            .flatten();
                        let (source, seeded) = if let Some((source, seeded, snapshot)) = selected {
                            self.selected = Some(snapshot);
                            (Some(source), seeded)
                        } else {
                            let source = self
                                .target
                                .map(|node| element_drag_source(&state, node))
                                .transpose()?
                                .flatten();
                            let mut seeded = Store::default();
                            if let Some(node) = source {
                                let dom = state.dom.borrow();
                                let image =
                                    dom.is_element(node, "http://www.w3.org/1999/xhtml", "img");
                                if let Some(url) = dom
                                    .attribute(node, if image { "src" } else { "href" })?
                                    .and_then(|url| state.location_url.borrow().join(url).ok())
                                {
                                    seeded.set_data(
                                        "text/uri-list",
                                        url.as_str().encode_utf16().collect(),
                                    );
                                    seeded.set_data(
                                        "text/plain",
                                        url.as_str().encode_utf16().collect(),
                                    );
                                }
                            }
                            (source, seeded)
                        };
                        let Some(source) = source else {
                            self.finish();
                            continue;
                        };
                        self.begin = Some(begin::Begin::new(
                            scope, &state, source, seeded, &self.init,
                        )?);
                        self.phase = Phase::Begin;
                    }
                    Operation::Advance | Operation::Drop => {
                        let Some(session) = self.session.as_ref() else {
                            self.finish();
                            continue;
                        };
                        if self.operation == Operation::Drop
                            && session.negotiation.target() == self.target
                        {
                            self.phase = Phase::Drop;
                            continue;
                        }
                        let preferred = preferred_effect(&self.init, session.selected.is_some());
                        let default = self
                            .target
                            .and_then(|target| session_default(&state, session, target, preferred));
                        self.advance = Some(advance::Advance::new(
                            &state,
                            session,
                            self.target,
                            preferred,
                            default,
                            &self.init,
                        ));
                        self.phase = Phase::Advance;
                    }
                    Operation::Cancel => self.phase = Phase::Cancel,
                },
                Phase::Begin => {
                    let begin = self.begin.as_mut().unwrap();
                    if !begin.poll(scope)? {
                        return Ok(None);
                    }
                    self.session = begin.take_session();
                    self.begin.take();
                    if let Some(session) = self.session.as_mut() {
                        session.selected = self.selected.take();
                        session.publish_image(&state, &self.init);
                    }
                    self.finish();
                }
                Phase::Advance => {
                    let advance = self.advance.as_mut().unwrap();
                    if !advance.poll(scope, self.session.as_mut().unwrap())? {
                        return Ok(None);
                    }
                    let canceled = advance.cancel_requested();
                    self.advance.take();
                    if canceled {
                        self.phase = Phase::Cancel
                    } else if self.operation == Operation::Drop {
                        self.phase = Phase::Drop
                    } else {
                        self.finish()
                    }
                }
                Phase::Drop => {
                    let session = self.session.as_mut().unwrap();
                    if self.drop.is_none() {
                        self.drop = Some(drop_operation::DropOperation::new(
                            &state, session, &self.init,
                        ));
                    }
                    let selected = session.selected.clone();
                    let effect = session.operation();
                    let x = f64::from(self.init.client_x);
                    let y = f64::from(self.init.client_y);
                    let edit = &mut self.edit;
                    let result = self.drop.as_mut().unwrap().poll(
                        scope,
                        session,
                        |scope, state, target, store| {
                            if edit.is_none() {
                                *edit = Some(
                                    if effect == DragEffect::Move
                                        && let Some(source) = selected.as_ref()
                                    {
                                        if !source.move_target(state, target)
                                            || default_effect(state, target, store).is_none()
                                        {
                                            DefaultEdit::Rejected
                                        } else if let Some(caret) =
                                            selection_movement::drop_caret(state, target, x, y)?
                                        {
                                            let text = oryn_dom::dom_string::from_units(
                                                &store.get_data("text/plain"),
                                            );
                                            if matches!(&source.kind, SelectedKind::Control { .. })
                                                && state.dom.borrow().is_text_control(target)
                                            {
                                                DefaultEdit::Control(Box::new(
                                                    control_move::ControlMove::new(
                                                        state,
                                                        source,
                                                        target,
                                                        caret.offset,
                                                        &text,
                                                    ),
                                                ))
                                            } else {
                                                DefaultEdit::Rich(Box::new(
                                                    rich_move::RichMove::new(
                                                        state, source, target, store, caret, &text,
                                                    ),
                                                ))
                                            }
                                        } else {
                                            DefaultEdit::Rejected
                                        }
                                    } else {
                                        DefaultEdit::Copy(Box::new(drop_default::DropDefault::new(
                                            state, target, store, x, y,
                                        )))
                                    },
                                );
                            }
                            edit.as_mut().unwrap().poll(scope)
                        },
                    )?;
                    if result.is_none() {
                        return Ok(None);
                    }
                    self.drop.take();
                    self.edit.take();
                    self.finish();
                }
                Phase::Cancel => {
                    let Some(session) = self.session.as_mut() else {
                        self.finish();
                        continue;
                    };
                    if self.cancel.is_none() {
                        self.cancel = Some(cancel::Cancel::new(&state, session, &self.init));
                    }
                    if !self.cancel.as_mut().unwrap().poll(scope, session)? {
                        return Ok(None);
                    }
                    self.cancel.take();
                    self.finish();
                }
                Phase::Done => return Ok(Some(self.result)),
            }
        }
    }
}
impl Drop for NativeDragInput {
    fn drop(&mut self) {
        if !matches!(self.phase, Phase::Done) {
            self.cancel();
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
    fn host_owned_input_keeps_session_and_excludes_nested_admission_during_retry() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div draggable=true id=s>source</div><div id=t>target</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(233), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let init = EventInit::default();
        eval(
            scope,
            "globalThis.trace=[];const s=document.getElementById('s'),t=document.getElementById('t');s.addEventListener('dragstart',e=>{trace.push('start');e.dataTransfer.setData('text/plain','payload')});s.addEventListener('dragend',()=>trace.push('end'));t.addEventListener('dragenter',()=>trace.push('enter'));t.addEventListener('dragover',e=>{trace.push('over');e.preventDefault();e.dataTransfer.dropEffect='copy'});t.addEventListener('drop',e=>{trace.push('drop:'+e.dataTransfer.getData('text/plain'));e.preventDefault()});",
        );
        let mut begin = NativeDragInput::new(&state, "begin", Some(source), &init).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(begin.poll(scope).is_err());
        assert!(state.native_drag_dispatching.get());
        assert!(state.native_drag.borrow().is_none());
        let mut nested = NativeDragInput::new(&state, "begin", Some(source), &init).unwrap();
        assert_eq!(nested.poll(scope).unwrap(), Some(false));
        drop(nested);
        assert!(state.native_drag_dispatching.get());
        state.set_execution_suspended(true);
        assert_eq!(begin.poll(scope).unwrap(), None);
        state.set_execution_suspended(false);
        assert_eq!(begin.poll(scope).unwrap(), Some(true));
        assert!(!state.native_drag_dispatching.get());
        assert!(state.native_drag.borrow().is_some());
        drop(begin);
        let mut advance = NativeDragInput::new(&state, "advance", Some(target), &init).unwrap();
        for _ in 0..2 {
            native_platform::publication::fail_after_next_callback();
            assert!(advance.poll(scope).is_err());
        }
        assert_eq!(advance.poll(scope).unwrap(), Some(true));
        drop(advance);
        let mut drop = NativeDragInput::new(&state, "drop", Some(target), &init).unwrap();
        for _ in 0..2 {
            native_platform::publication::fail_after_next_callback();
            assert!(drop.poll(scope).is_err());
            assert!(state.native_drag_dispatching.get());
        }
        assert_eq!(drop.poll(scope).unwrap(), Some(false));
        assert_eq!(drop.poll(scope).unwrap(), Some(false));
        assert!(!state.native_drag_dispatching.get());
        assert!(state.native_drag.borrow().is_none());
        eval(
            scope,
            "if(trace.join(',')!=='start,enter,over,drop:payload,end')throw Error('input receipt repeated drag stage or lost payload');",
        );
    }
}
