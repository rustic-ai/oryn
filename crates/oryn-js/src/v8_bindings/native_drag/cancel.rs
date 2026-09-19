//! Retained drag cancellation: leave, one cleanup commit, then dragend.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Leave,
    Commit,
    End,
    Done,
}
pub(super) struct Cancel {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    store: Rc<RefCell<Store>>,
    identity: (NodeId, u64, u64, u64),
    source: NodeId,
    target: Option<NodeId>,
    init: EventInit,
    phase: Phase,
    foreign: Option<cross_frame::Leave>,
    event: Option<data_transfer::TrustedTransfer>,
}
impl Cancel {
    pub(super) fn new(state: &Rc<V8BindingState>, session: &mut Session, init: &EventInit) -> Self {
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
            phase: if session.ended {
                Phase::Done
            } else {
                Phase::Leave
            },
            foreign: session
                .cross_target
                .take()
                .map(|target| cross_frame::Leave::new(target, session.store.clone())),
            event: None,
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
    fn present(&self, node: NodeId) -> bool {
        self.owner
            .dom
            .borrow()
            .arena()
            .node_document(node)
            .ok()
            .flatten()
            == Some(self.identity.0)
    }
    fn emit(
        &mut self,
        scope: &mut v8::PinScope,
        node: NodeId,
        name: &str,
    ) -> Result<bool, V8BindingError> {
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
        if self.event.as_mut().unwrap().poll(scope)?.is_none() {
            return Ok(false);
        }
        self.event.take();
        Ok(true)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        session: &mut Session,
    ) -> Result<bool, V8BindingError> {
        if !Rc::ptr_eq(&session.store, &self.store) {
            return Err(V8BindingError::Allocation(
                "drag cancellation session changed",
            ));
        }
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(true);
            }
            if !self.current() {
                session.abandon();
                native_drag_image::clear(&self.owner, session.feedback_session.take());
                self.event.take();
                self.foreign.take();
                self.phase = Phase::Done;
                return Ok(true);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.phase {
                Phase::Leave => {
                    if let Some(foreign) = &mut self.foreign {
                        if !foreign.poll(scope)? {
                            return Ok(false);
                        }
                    } else if let Some(target) = self.target.filter(|node| self.present(*node)) {
                        if !self.emit(scope, target, "dragleave")? {
                            return Ok(false);
                        }
                    }
                    self.foreign.take();
                    self.phase = Phase::Commit;
                }
                Phase::Commit => {
                    session
                        .negotiation
                        .finish(false, &mut self.store.borrow_mut());
                    native_drag_image::clear(&self.owner, session.feedback_session.take());
                    session.ended = true;
                    self.phase = Phase::End;
                }
                Phase::End => {
                    if self.present(self.source) && !self.emit(scope, self.source, "dragend")? {
                        return Ok(false);
                    }
                    self.store.borrow_mut().set_mode(Mode::Detached);
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
    fn cancellation_resumes_dragend_after_single_cleanup_commit() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(226), 1).unwrap();
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
        eval(
            scope,
            "globalThis.trace=[];document.getElementById('t').addEventListener('dragleave',()=>trace.push('leave'));document.getElementById('s').addEventListener('dragend',e=>{trace.push('end');globalThis.saved=e.dataTransfer});",
        );
        let mut cancel = Cancel::new(&state, &mut session, &init);
        native_platform::publication::fail_after_next_callback();
        assert!(cancel.poll(scope, &mut session).is_err());
        assert!(!session.ended);
        native_platform::publication::fail_after_next_callback();
        assert!(cancel.poll(scope, &mut session).is_err());
        assert!(session.ended);
        assert!(cancel.poll(scope, &mut session).unwrap());
        assert!(cancel.poll(scope, &mut session).unwrap());
        assert_eq!(session.store.borrow().mode, Mode::Detached);
        eval(
            scope,
            "if(trace.join(',')!=='leave,end'||saved.types.length!==0)throw Error('cancel replayed events or retained access');",
        );
    }
}
