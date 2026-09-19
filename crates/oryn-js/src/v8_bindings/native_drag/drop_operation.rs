//! Drop notification and default-action custody. The caller retains its native
//! copy/move job; Pending never consumes that job or restarts the drop event.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Drop,
    Default,
    Commit,
    End,
    Cancel,
    Done,
}
pub(super) struct DropOperation {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    store: Rc<RefCell<Store>>,
    identity: (NodeId, u64, u64, u64),
    source: NodeId,
    target: Option<NodeId>,
    init: EventInit,
    phase: Phase,
    event: Option<data_transfer::TrustedTransfer>,
    snapshot: Option<Store>,
    canceled: bool,
    success: bool,
    result: DragEffect,
    cancel: Option<cancel::Cancel>,
}
impl DropOperation {
    pub(super) fn new(state: &Rc<V8BindingState>, session: &Session, init: &EventInit) -> Self {
        Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            store: session.store.clone(),
            identity: (
                session.document,
                session.generation,
                session.epoch,
                session.authority,
            ),
            source: session.source,
            target: session.negotiation.target(),
            init: init.clone(),
            phase: Phase::Drop,
            event: None,
            snapshot: None,
            canceled: false,
            success: false,
            result: DragEffect::None,
            cancel: None,
        }
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
    fn emit(
        &mut self,
        scope: &mut v8::PinScope,
        node: NodeId,
        name: &str,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.event.is_none() {
            self.event = Some(data_transfer::TrustedTransfer::new(
                scope,
                &self.owner,
                node,
                name,
                self.store.clone(),
                &self.init,
            )?);
        }
        let result = self.event.as_mut().unwrap().poll(scope)?;
        if result.is_some() {
            self.event.take();
        }
        Ok(result)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        session: &mut Session,
        mut default: impl FnMut(
            &mut v8::PinScope,
            &Rc<V8BindingState>,
            NodeId,
            &Store,
        ) -> Result<Option<bool>, V8BindingError>,
    ) -> Result<Option<DragEffect>, V8BindingError> {
        if !Rc::ptr_eq(&session.store, &self.store) {
            return Err(V8BindingError::Allocation("drop session changed"));
        }
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(Some(self.result));
            }
            if !self.current() {
                session.abandon();
                native_drag_image::clear(&self.owner, session.feedback_session.take());
                self.event.take();
                self.cancel.take();
                self.phase = Phase::Done;
                return Ok(Some(self.result));
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                Phase::Drop => {
                    if session.ended {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let Some(target) = self
                        .target
                        .filter(|node| session.target_live(&state, *node))
                    else {
                        self.phase = Phase::Cancel;
                        continue;
                    };
                    if session.operation() == DragEffect::None {
                        self.phase = Phase::Cancel;
                        continue;
                    }
                    let Some(accepted) = self.emit(scope, target, "drop")? else {
                        return Ok(None);
                    };
                    self.canceled = !accepted;
                    self.success = self.canceled;
                    self.phase = if self.canceled {
                        Phase::Commit
                    } else {
                        Phase::Default
                    };
                }
                Phase::Default => {
                    if self.snapshot.is_none() {
                        let mut snapshot = self.store.borrow().clone();
                        snapshot.set_mode(Mode::ReadOnly);
                        self.snapshot = Some(snapshot);
                    }
                    let Some(success) = default(
                        scope,
                        &state,
                        self.target.unwrap(),
                        self.snapshot.as_ref().unwrap(),
                    )?
                    else {
                        return Ok(None);
                    };
                    self.success = success;
                    self.phase = Phase::Commit;
                }
                Phase::Commit => {
                    self.result = session.negotiation.finish_drop(
                        self.canceled,
                        self.success,
                        &mut self.store.borrow_mut(),
                    );
                    native_drag_image::clear(&state, session.feedback_session.take());
                    session.ended = true;
                    self.phase = Phase::End;
                }
                Phase::End => {
                    if session.present(&state, self.source)
                        && self.emit(scope, self.source, "dragend")?.is_none()
                    {
                        return Ok(None);
                    }
                    self.store.borrow_mut().set_mode(Mode::Detached);
                    self.phase = Phase::Done;
                }
                Phase::Cancel => {
                    if self.cancel.is_none() {
                        self.cancel = Some(cancel::Cancel::new(&state, session, &self.init));
                    }
                    if !self.cancel.as_mut().unwrap().poll(scope, session)? {
                        return Ok(None);
                    }
                    self.cancel.take();
                    self.phase = Phase::Done;
                }
                Phase::Done => return Ok(Some(self.result)),
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
    fn drop_default_pending_and_dragend_retry_keep_one_default_commit() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=s draggable=true>source</div><div id=t>target</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(227), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let init = EventInit::default();
        let mut session = Session::begin(scope, &state, source, Store::default(), &init)
            .unwrap()
            .unwrap();
        session.negotiation.prepare(
            Some(target),
            DragEffect::Copy,
            &mut session.store.borrow_mut(),
        );
        session
            .negotiation
            .accept_over(false, Some(DragEffect::Copy), &session.store.borrow());
        eval(
            scope,
            "globalThis.trace=[];document.getElementById('t').addEventListener('drop',()=>trace.push('drop'));document.getElementById('s').addEventListener('dragend',()=>trace.push('end'));",
        );
        let mut drop = DropOperation::new(&state, &session, &init);
        native_platform::publication::fail_after_next_callback();
        assert!(
            drop.poll(scope, &mut session, |_, _, _, _| panic!(
                "drop listener incomplete"
            ))
            .is_err()
        );
        assert!(
            drop.poll(scope, &mut session, |_, _, _, _| Ok(None))
                .unwrap()
                .is_none()
        );
        assert!(!session.ended);
        let commits = Cell::new(0);
        native_platform::publication::fail_after_next_callback();
        assert!(
            drop.poll(scope, &mut session, |_, _, _, _| {
                commits.set(commits.get() + 1);
                Ok(Some(true))
            })
            .is_err()
        );
        assert!(session.ended);
        assert_eq!(
            drop.poll(scope, &mut session, |_, _, _, _| panic!(
                "default already committed"
            ))
            .unwrap(),
            Some(DragEffect::Copy)
        );
        assert_eq!(commits.get(), 1);
        eval(
            scope,
            "if(trace.join(',')!=='drop,end')throw Error('drop/end replayed');",
        );
        assert_eq!(session.store.borrow().mode, Mode::Detached);
    }
}
