//! Original-Document activation publication. Parent authority promotion and
//! candidate Document creation precede polling this native-only receipt. The
//! original parser may continue after activation; load remains a separate task.
use super::*;

pub struct NativePrerenderActivation {
    owner: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    event: retained_dom_event::Event,
    visibility: Option<native_platform::publication::Notification>,
    started: bool,
    history_published: bool,
    done: bool,
    canceled: bool,
}
fn identity(state: &V8BindingState) -> (NodeId, u64, u64, u64) {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}
impl NativePrerenderActivation {
    pub fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Self, V8BindingError> {
        if state.execution_retired() || !state.document_prerendering() {
            return Err(V8BindingError::Allocation("prerender activation owner"));
        }
        let event = create_event_object_with_interface(
            scope,
            state,
            "Event",
            "prerenderingchange",
            false,
            false,
        )?;
        let id = event_id(scope, event)?;
        state
            .events
            .borrow_mut()
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("prerender activation event"))?
            .is_trusted = true;
        let event = retained_dom_event::Event::new(
            scope,
            state,
            state.document_root(),
            "prerenderingchange",
            event,
        )?;
        let visibility = Some(native_platform::publication::pending_lifecycle_visibility(
            scope, state, false,
        )?);
        Ok(Self {
            owner: state.clone(),
            identity: identity(state),
            event,
            visibility,
            started: false,
            history_published: false,
            done: false,
            canceled: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    pub fn canceled(&self) -> bool {
        self.canceled
    }
    pub fn retry_safe(&self) -> bool {
        !self.done && !self.owner.execution_retired() && identity(&self.owner) == self.identity
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if !self.retry_safe() {
            self.cancel();
            return Ok(true);
        }
        if self.owner.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        if !self.started {
            // Freeze reveal before observable state changes. Reentrant rAF or
            // paint queries cannot publish the candidate ahead of this event.
            self.owner.prepare_history_reveal();
            self.owner
                .record_navigation_timing_stage(crate::navigation_timing::Stage::Activation);
            self.owner.document_prerendering.set(false);
            self.started = true;
        }
        if !self.history_published {
            navigation_api::publish_current_native_history_projection(&self.owner)?;
            self.history_published = true;
        }
        if let Some(visibility) = self.visibility.as_mut() {
            if !visibility.resume(scope, &self.owner)? {
                return Ok(false);
            }
            visibility.finish(scope, &self.owner);
            self.visibility = None;
        }
        if !self.retry_safe() {
            self.cancel();
            return Ok(true);
        }
        if self.owner.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        if self.event.poll(scope, &self.owner)?.is_none() {
            return Ok(false);
        }
        if identity(&self.owner) == self.identity && !self.owner.execution_retired() {
            self.owner.complete_history_reveal();
        }
        self.done = true;
        Ok(true)
    }
    pub fn cancel(&mut self) {
        if self.done {
            return;
        }
        if let Some(visibility) = self.visibility.take() {
            visibility.cancel(&self.owner);
        }
        self.event.cancel(&self.owner);
        // Cancellation is disposal, never reveal admission. Once activation
        // started, the exact document stays blocked until its owner retires it;
        // otherwise a lost callback suffix could expose a half-activated page.
        self.canceled = true;
        self.done = true;
    }
}
impl Drop for NativePrerenderActivation {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn activation_visibility_failure_retains_suffix_before_prerender_event_and_reveal() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.initialize_prerender_document(7);
        eval(
            scope,
            "globalThis.order=[];document.addEventListener('visibilitychange',()=>order.push('visible:'+document.hidden));document.addEventListener('visibilitychange',()=>order.push('tail'));document.addEventListener('prerenderingchange',()=>order.push('activated'))",
        );
        assert!(eval(scope, "document.hidden").is_true());
        let mut receipt = NativePrerenderActivation::new(scope, &state).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(
            eval(
                scope,
                "order.join()==='visible:false'&&!document.prerendering"
            )
            .is_true()
        );
        assert!(!state.document_rendering_allowed());
        assert!(receipt.poll(scope).unwrap());
        assert!(receipt.poll(scope).unwrap());
        assert!(eval(scope, "order.join()==='visible:false,tail,activated'").is_true());
    }
    #[test]
    fn activation_changes_flag_once_and_reveal_waits_for_document_event() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.initialize_prerender_document(7);
        eval(
            scope,
            "globalThis.order=[];globalThis.saved=document;document.addEventListener('prerenderingchange',e=>order.push([e.isTrusted,e.target===saved,!document.prerendering]));addEventListener('pagereveal',()=>order.push('reveal'));requestAnimationFrame(()=>order.push('raf'))",
        );
        assert!(!state.document_rendering_allowed());
        let mut receipt = NativePrerenderActivation::new(scope, &state).unwrap();
        assert!(
            state.document_prerendering(),
            "preparation does not activate"
        );
        assert!(receipt.poll(scope).unwrap());
        assert!(receipt.poll(scope).unwrap());
        assert!(
            eval(
                scope,
                "order.length===1&&order[0].every(Boolean)&&saved===document"
            )
            .is_true()
        );
        assert!(animation_frames::reveal(scope, &state).unwrap());
        assert!(eval(scope, "order.length===2&&order[1]==='reveal'").is_true());
    }
    #[test]
    fn changed_stream_never_receives_old_activation_event() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.initialize_prerender_document(2);
        let mut receipt = NativePrerenderActivation::new(scope, &state).unwrap();
        eval(
            scope,
            "document.open();document.write('<body></body>');document.close();globalThis.fired=0;document.addEventListener('prerenderingchange',()=>fired++)",
        );
        assert!(receipt.poll(scope).unwrap());
        assert!(eval(scope, "fired===0").is_true());
    }
}
