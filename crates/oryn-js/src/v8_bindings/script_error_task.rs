//! Script preparation failures that precede native fetch admission.
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
    dom: SharedDom,
    identity: Identity,
    owner: RcWeak<V8BindingState>,
    notification: Option<publication::Notification>,
}
impl Task {
    pub(super) fn new(state: &V8BindingState, node: NodeId) -> Result<Self, V8BindingError> {
        Ok(Self {
            target: oryn_dom::TrackedNodeLease::new(&state.dom, node)?,
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
                    && dom.borrow().node_document(node).ok().flatten() == Some(self.identity.0))
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
        if self.notification.is_none() {
            let (_, node) = self.target.current();
            let wrapper = wrap_node(scope, state, node)?;
            let event = create_event_object(scope, state, "error", false, false)?;
            let id = event_id(scope, event)?;
            state
                .events
                .borrow_mut()
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("script error record"))?
                .is_trusted = true;
            self.owner = Rc::downgrade(state);
            self.notification = Some(
                publication::pending_dom_dispatch(scope, state, wrapper, "error", event)
                    .for_parser_stream(self.identity.2)
                    .without_parser_insertion_point(),
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
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn dynamic_pre_fetch_error_retains_event_and_callback_suffix() {
        for setup in ["s.src=''", "s.type='importmap';s.src='map.json'"] {
            super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
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
                &format!(
                    "globalThis.calls=[];globalThis.saved=null;globalThis.s=document.createElement('script');{setup};s.addEventListener('error',e=>{{saved=e;calls.push(e.isTrusted?'first':'untrusted')}},{{once:true}});s.addEventListener('error',e=>calls.push(e===saved?'second':'different'));document.body.append(s)"
                ),
            );
            assert!(
                state
                    .deferred_platform_tasks
                    .borrow()
                    .iter()
                    .any(|task| matches!(task, DeferredPlatformTask::ScriptError(_)))
            );
            publication::fail_before_next_prepare();
            assert!(state.drain_platform_completions(scope, 1).is_err());
            assert!(eval(scope, "calls.length===0").is_true());
            publication::fail_after_next_callback();
            assert!(state.drain_platform_completions(scope, 1).is_err());
            assert!(eval(scope, "calls.join()==='first'").is_true());
            state.drain_platform_completions(scope, 1).unwrap();
            assert!(eval(scope, "calls.join()==='first,second'&&saved.eventPhase===0").is_true());
            assert!(
                !state
                    .deferred_platform_tasks
                    .borrow()
                    .iter()
                    .any(|task| matches!(task, DeferredPlatformTask::ScriptError(_)))
            );
        }
    }
    #[test]
    fn adoption_after_dispatch_start_preserves_original_event_path_suffix() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><script id=s type=text/plain></script></body>")
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
            "globalThis.calls=[];globalThis.foreign=document.implementation.createHTMLDocument('foreign');s.addEventListener('error',e=>{globalThis.saved=e;calls.push('first');foreign.adoptNode(e.target)});s.addEventListener('error',e=>calls.push(e===saved?'second':'different'))",
        );
        let node = state.dom.borrow().find_by_id("s").unwrap();
        state.queue_script_error(node).unwrap();
        publication::fail_after_next_callback();
        assert!(state.drain_platform_completions(scope, 1).is_err());
        state.drain_platform_completions(scope, 1).unwrap();
        assert!(
            eval(
                scope,
                "calls.join()==='first,second'&&saved.target.ownerDocument===foreign"
            )
            .is_true()
        );
    }

    #[test]
    fn classic_legacy_filter_precedes_empty_source_error_admission() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
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
            "globalThis.s=document.createElement('script');s.id='blocked';s.setAttribute('for','object');s.setAttribute('event','handler');s.src='';document.body.append(s)",
        );
        assert!(
            !state
                .deferred_platform_tasks
                .borrow()
                .iter()
                .any(|task| matches!(task, DeferredPlatformTask::ScriptError(_)))
        );
        assert!(!state.has_pending_dynamic_scripts());
        let node = state.dom.borrow().find_by_id("blocked").unwrap();
        assert!(state.dom.borrow().script_already_started(node).unwrap());
    }

    #[test]
    fn deferred_error_suppresses_parser_insertion_across_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><p id=before>before</p><script id=s type=text/plain></script></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            oryn_dom::html::StreamingParser::with_document(dom, state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate);
        eval(
            scope,
            "globalThis.calls=[];s.addEventListener('error',()=>calls.push('first'));s.addEventListener('error',()=>{calls.push('second');document.write('<p id=after>after</p>')})",
        );
        let node = state.dom.borrow().find_by_id("s").unwrap();
        state.queue_script_error(node).unwrap();
        publication::fail_after_next_callback();
        assert!(state.drain_platform_completions(scope, 1).is_err());
        assert!(state.parser_insertion_suppression.get().is_none());
        state.drain_platform_completions(scope, 1).unwrap();
        assert!(state.parser_insertion_suppression.get().is_none());
        assert!(
            eval(
                scope,
                "calls.join()==='first,second'&&!document.getElementById('before')"
            )
            .is_true()
        );
    }
    #[test]
    fn parked_or_retired_error_task_cannot_publish_into_replacement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><script id=s type=text/plain></script></body>")
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
            "globalThis.calls=0;s.addEventListener('error',()=>calls++);s.addEventListener('error',()=>calls++)",
        );
        let node = state.dom.borrow().find_by_id("s").unwrap();
        let mut task = Task::new(&state, node).unwrap();
        state.set_execution_suspended(true);
        assert!(!task.poll(scope, &state).unwrap());
        assert!(task.notification.is_none());
        state.set_execution_suspended(false);
        publication::fail_after_next_callback();
        assert!(task.poll(scope, &state).is_err());
        assert!(eval(scope, "calls===1").is_true());
        eval(scope, "document.open()");
        assert!(task.poll(scope, &state).unwrap());
        drop(task);
        assert!(eval(scope, "calls===1").is_true());
    }
}
