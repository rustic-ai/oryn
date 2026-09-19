//! Dragstart owns its event until admission or rejection is complete.
use super::*;
pub(super) struct Begin {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    source: NodeId,
    identity: (NodeId, u64, u64, u64),
    store: Rc<RefCell<Store>>,
    event: data_transfer::TrustedTransfer,
    done: bool,
    session: Option<Session>,
}
impl Begin {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        source: NodeId,
        mut seeded: Store,
        init: &EventInit,
    ) -> Result<Self, V8BindingError> {
        seeded.set_mode(Mode::Protected);
        let store = Rc::new(RefCell::new(seeded));
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let event = data_transfer::TrustedTransfer::new(
            scope,
            state,
            source,
            "dragstart",
            store.clone(),
            init,
        )?;
        Ok(Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            source,
            identity,
            store,
            event,
            done: false,
            session: None,
        })
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        let state = &self.owner;
        if !self.input_owner.current()
            || state.execution_retired()
            || state.execution_tasks_stopped()
            || self.identity
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
        {
            self.cancel();
            return Ok(true);
        }
        let Some(accepted) = self.event.poll(scope)? else {
            return Ok(false);
        };
        // Event completion already rechecked original document authority. A
        // source adopted by a listener cannot seed a drag in its new document.
        if accepted
            && self.input_owner.current()
            && state.dom.borrow().arena().node_document(self.source)? == Some(self.identity.0)
        {
            self.session = Some(Session {
                source: self.source,
                document: self.identity.0,
                generation: self.identity.1,
                epoch: self.identity.2,
                authority: self.identity.3,
                store: self.store.clone(),
                negotiation: DragNegotiation::new(&self.store.borrow()),
                ended: false,
                feedback_session: None,
                selected: None,
                cross_target: None,
            });
        } else {
            self.store.borrow_mut().set_mode(Mode::Detached);
        }
        self.done = true;
        Ok(true)
    }
    pub(super) fn take_session(&mut self) -> Option<Session> {
        self.session.take()
    }
    pub(super) fn cancel(&mut self) {
        self.event.cancel();
        self.session.take();
        self.store.borrow_mut().set_mode(Mode::Detached);
        self.done = true;
    }
}
impl Drop for Begin {
    fn drop(&mut self) {
        if !self.done {
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
    fn dragstart_retry_admits_one_session_with_completed_payload() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t draggable=true>drag</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(224), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');globalThis.trace=[];t.addEventListener('dragstart',e=>{trace.push('first');e.dataTransfer.setData('text/plain','a')},{once:true});t.addEventListener('dragstart',e=>{trace.push('second');e.dataTransfer.setData('text/plain',e.dataTransfer.getData('text/plain')+'b')});",
        );
        let mut start = Begin::new(
            scope,
            &state,
            target,
            Store::default(),
            &EventInit::default(),
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(start.poll(scope).is_err());
        assert!(start.take_session().is_none());
        state.set_execution_suspended(true);
        assert!(!start.poll(scope).unwrap());
        state.set_execution_suspended(false);
        assert!(start.poll(scope).unwrap());
        let session = start.take_session().unwrap();
        assert!(start.poll(scope).unwrap());
        assert!(start.take_session().is_none());
        drop(start);
        assert!(session.live(&state));
        let mut data = session.store.borrow().clone();
        data.set_mode(Mode::ReadOnly);
        assert_eq!(data.get_data("text/plain"), vec![97, 98]);
        eval(
            scope,
            "if(trace.join(',')!=='first,second')throw Error('dragstart replayed');",
        );
    }
}
