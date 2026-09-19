//! Retained networking tasks for native stylesheet load/error publication.
//! The queue owns the target and callback suffix until completion or cancellation.
use super::native_platform::publication;
use super::*;

type Identity = (NodeId, u64, u64, u64);
fn identity(state: &V8BindingState) -> Identity {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}
pub(super) struct Task {
    target: oryn_dom::TrackedNodeLease,
    render_blocker: Option<oryn_dom::RenderBlocker>,
    revision: u64,
    success: bool,
    retry: Option<std::time::Instant>,
    dom: SharedDom,
    identity: Identity,
    owner: RcWeak<V8BindingState>,
    notification: Option<publication::Notification>,
}
impl Task {
    pub(super) fn targets_adopted_nodes(
        &self,
        destination: &SharedDom,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> bool {
        let (dom, node) = self.target.current();
        Rc::ptr_eq(&dom, destination) && nodes.values().any(|candidate| *candidate == node)
    }
    pub(super) fn new(
        state: &V8BindingState,
        node: NodeId,
        revision: u64,
        success: bool,
    ) -> Result<Self, V8BindingError> {
        let render_blocker = state.dom.borrow().retain_render_resource(node, revision);
        let target = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
        Ok(Self {
            render_blocker,
            revision,
            success,
            retry: None,
            target,
            dom: state.dom.clone(),
            identity: identity(state),
            owner: RcWeak::new(),
            notification: None,
        })
    }
    fn current(&self, state: &V8BindingState) -> bool {
        let (dom, node) = self.target.current();
        !state.execution_retired()
            && identity(state) == self.identity
            && Rc::ptr_eq(&self.dom, &state.dom)
            && (self
                .notification
                .as_ref()
                .is_some_and(|notification| notification.started())
                || Rc::ptr_eq(&dom, &state.dom)
                    && dom.borrow().node_document(node).ok().flatten() == Some(self.identity.0)
                    && dom.borrow().is_connected(node).unwrap_or(false)
                    && dom.borrow().arena().stylesheet_revision(node).ok() == Some(self.revision))
    }
    pub(super) fn delay(&self, state: &V8BindingState) -> Option<std::time::Duration> {
        if !self.current(state) {
            return Some(std::time::Duration::ZERO);
        }
        if state.execution_suspended() {
            return None;
        }
        Some(self.retry.map_or(std::time::Duration::ZERO, |retry| {
            retry.saturating_duration_since(std::time::Instant::now())
        }))
    }
    pub(super) fn retry_later(&mut self) {
        self.retry = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if !self.current(state) {
            return Ok(true);
        }
        if state.execution_suspended()
            || state.execution_tasks_stopped()
            || scope.is_execution_terminating()
        {
            return Ok(false);
        }
        if self.delay(state).is_none_or(|delay| !delay.is_zero()) {
            return Ok(false);
        }
        self.retry = None;
        let name = if self.success { "load" } else { "error" };
        if self.notification.is_none() {
            let (_, node) = self.target.current();
            let wrapper = wrap_node(scope, state, node)?;
            let event = create_event_object(scope, state, name, false, false)?;
            let id = event_id(scope, event)?;
            state
                .events
                .borrow_mut()
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("stylesheet event record"))?
                .is_trusted = true;
            self.owner = Rc::downgrade(state);
            self.notification = Some(
                publication::pending_dom_dispatch(scope, state, wrapper, name, event)
                    .for_parser_stream(self.identity.2),
            );
        }
        let notification = self.notification.as_mut().unwrap();
        if !notification.resume(scope, state)? {
            return Ok(false);
        }
        if identity(state) == self.identity {
            notification.finish(scope, state);
        }
        self.notification = None;
        Ok(true)
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        if let Some(blocker) = &self.render_blocker {
            blocker.release();
        }
        if let (Some(notification), Some(state)) = (&self.notification, self.owner.upgrade()) {
            // Never clean an old event ID out of a replacement Document's map.
            if identity(&state) == self.identity {
                notification.cancel(&state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    #[test]
    fn networking_completion_retains_event_suffix_and_schedules_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><style id=s></style></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];s.addEventListener('load',e=>{globalThis.saved=e;calls.push('one')});s.addEventListener('load',e=>calls.push(e===saved?'two':'wrong'));true",
        );
        let node = state.dom.borrow().find_by_id("s").unwrap();
        let revision = state
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .unwrap();
        queue_stylesheet_event(scope, &state, node, revision, true).unwrap();
        publication::fail_before_next_prepare();
        assert!(deliver_stylesheet_event(scope, &state).is_err());
        assert!(state.has_pending_stylesheet_event());
        assert!(!state.stylesheet_event_ready());
        assert!(eval(scope, "calls.length===0"));
        state
            .stylesheet_event_tasks
            .borrow_mut()
            .front_mut()
            .unwrap()
            .retry = None;
        publication::fail_after_next_callback();
        assert!(deliver_stylesheet_event(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='one'"));
        state
            .stylesheet_event_tasks
            .borrow_mut()
            .front_mut()
            .unwrap()
            .retry = None;
        assert!(deliver_stylesheet_event(scope, &state).unwrap());
        assert!(!state.has_pending_stylesheet_event());
        assert!(eval(
            scope,
            "calls.join()==='one,two'&&saved.isTrusted&&saved.eventPhase===0"
        ));
    }
    #[test]
    fn replaced_stylesheet_revision_cancels_before_any_callback() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><style id=s></style></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "globalThis.fired=false;s.onload=()=>fired=true;true");
        let node = state.dom.borrow().find_by_id("s").unwrap();
        let revision = state
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .unwrap();
        queue_stylesheet_event(scope, &state, node, revision, true).unwrap();
        eval(scope, "s.textContent='body {color:red}';true");
        assert!(deliver_stylesheet_event(scope, &state).unwrap());
        assert!(eval(scope, "fired===false"));
    }
}
