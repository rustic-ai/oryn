//! Retained same-document drag update. The host owns the surrounding Session.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Source,
    Prepare,
    Enter,
    Leave,
    OverPrepare,
    Over,
    Negotiate,
    RemovedLeave,
    Done,
}
pub(super) struct Advance {
    input_owner: native_pointer::InputOwner,
    owner: Rc<V8BindingState>,
    store: Rc<RefCell<Store>>,
    target: Option<NodeId>,
    previous: Option<NodeId>,
    preferred: DragEffect,
    native_default: Option<DragEffect>,
    init: EventInit,
    phase: Phase,
    event: Option<data_transfer::TrustedTransfer>,
    over_canceled: bool,
    result: DragEffect,
    cancel_requested: bool,
}
impl Advance {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        session: &Session,
        target: Option<NodeId>,
        preferred: DragEffect,
        native_default: Option<DragEffect>,
        init: &EventInit,
    ) -> Self {
        Self {
            input_owner: native_pointer::InputOwner::new(state),
            owner: state.clone(),
            store: session.store.clone(),
            target,
            previous: None,
            preferred,
            native_default,
            init: init.clone(),
            phase: Phase::Source,
            event: None,
            over_canceled: false,
            result: DragEffect::None,
            cancel_requested: false,
        }
    }
    pub(super) fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    pub(super) fn result(&self) -> DragEffect {
        self.result
    }
    fn emit(
        &mut self,
        scope: &mut v8::PinScope,
        target: NodeId,
        name: &str,
        related: Option<NodeId>,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.event.is_none() {
            let mut init = self.init.clone();
            init.related_target = related;
            self.event = Some(data_transfer::TrustedTransfer::new(
                scope,
                &self.owner,
                target,
                name,
                self.store.clone(),
                &init,
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
    ) -> Result<bool, V8BindingError> {
        if !Rc::ptr_eq(&session.store, &self.store) {
            return Err(V8BindingError::Allocation("drag advance session changed"));
        }
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(true);
            }
            let state = self.owner.clone();
            if !self.input_owner.current() || !session.live(&state) {
                session.abandon();
                self.event.take();
                self.phase = Phase::Done;
                return Ok(true);
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.phase {
                Phase::Source => {
                    if !session.present(&state, session.source) {
                        self.cancel_requested = true;
                        self.phase = Phase::Done;
                        continue;
                    }
                    let Some(accepted) =
                        self.emit(scope, session.source, "drag", self.init.related_target)?
                    else {
                        return Ok(false);
                    };
                    if !accepted {
                        self.cancel_requested = true;
                        self.phase = Phase::Done;
                    } else {
                        self.phase = Phase::Prepare;
                    }
                }
                Phase::Prepare => {
                    self.target = self
                        .target
                        .filter(|target| session.target_live(&state, *target));
                    self.previous = session.negotiation.target();
                    session.negotiation.prepare(
                        self.target,
                        self.preferred,
                        &mut self.store.borrow_mut(),
                    );
                    self.phase = if self.previous != self.target {
                        Phase::Enter
                    } else {
                        Phase::OverPrepare
                    };
                }
                Phase::Enter => {
                    if let Some(target) = self.target {
                        if self
                            .emit(scope, target, "dragenter", self.previous)?
                            .is_none()
                        {
                            return Ok(false);
                        }
                    }
                    self.phase = Phase::Leave;
                }
                Phase::Leave => {
                    if let Some(previous) =
                        self.previous.filter(|node| session.present(&state, *node))
                    {
                        if self
                            .emit(scope, previous, "dragleave", self.target)?
                            .is_none()
                        {
                            return Ok(false);
                        }
                    }
                    self.phase = Phase::OverPrepare;
                }
                Phase::OverPrepare => {
                    self.target = self
                        .target
                        .filter(|node| session.target_live(&state, *node));
                    session.negotiation.prepare(
                        self.target,
                        self.preferred,
                        &mut self.store.borrow_mut(),
                    );
                    self.phase = if self.target.is_some() {
                        Phase::Over
                    } else {
                        Phase::Done
                    };
                }
                Phase::Over => {
                    let Some(accepted) = self.emit(
                        scope,
                        self.target.unwrap(),
                        "dragover",
                        self.init.related_target,
                    )?
                    else {
                        return Ok(false);
                    };
                    self.over_canceled = !accepted;
                    self.phase = Phase::Negotiate;
                }
                Phase::Negotiate => {
                    let target = self.target.unwrap();
                    if !session.target_live(&state, target) {
                        session.negotiation.prepare(
                            None,
                            self.preferred,
                            &mut self.store.borrow_mut(),
                        );
                        self.phase = Phase::RemovedLeave;
                        continue;
                    }
                    self.result = session.negotiation.accept_over(
                        self.over_canceled,
                        self.native_default,
                        &self.store.borrow(),
                    );
                    self.phase = Phase::Done;
                }
                Phase::RemovedLeave => {
                    let target = self.target.unwrap();
                    if session.present(&state, target)
                        && self
                            .emit(scope, target, "dragleave", self.init.related_target)?
                            .is_none()
                    {
                        return Ok(false);
                    }
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
    fn interrupted_enter_and_over_do_not_repeat_source_drag() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(225), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let init = EventInit::default();
        let mut session = Session::begin(scope, &state, source, Store::default(), &init)
            .unwrap()
            .unwrap();
        eval(
            scope,
            "globalThis.trace=[];document.getElementById('s').addEventListener('drag',()=>trace.push('drag'));const t=document.getElementById('t');t.addEventListener('dragenter',()=>trace.push('enter'));t.addEventListener('dragover',e=>{trace.push('over');e.preventDefault();e.dataTransfer.dropEffect='copy'});",
        );
        let mut update = Advance::new(
            &state,
            &session,
            Some(target),
            DragEffect::Copy,
            None,
            &init,
        );
        for _ in 0..3 {
            native_platform::publication::fail_after_next_callback();
            assert!(update.poll(scope, &mut session).is_err());
        }
        assert!(update.poll(scope, &mut session).unwrap());
        assert!(update.poll(scope, &mut session).unwrap());
        assert_eq!(update.result(), DragEffect::Copy);
        assert!(!update.cancel_requested());
        eval(
            scope,
            "if(trace.join(',')!=='drag,enter,over')throw Error('drag advance replayed earlier event');",
        );
    }
}
