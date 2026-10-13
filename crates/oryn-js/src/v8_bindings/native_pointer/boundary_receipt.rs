//! Cross-document hover routing owns one pointer lane until its suffix completes.
use super::*;
pub struct NativePointerBoundary {
    state: Rc<V8BindingState>,
    owner: InputOwner,
    claim: Rc<()>,
    input: NativePointerInput,
    old: Option<NodeId>,
    target: Option<NodeId>,
    committed: bool,
    started: bool,
    done: bool,
    canceled: bool,
}
impl NativePointerBoundary {
    pub fn new(
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
        target: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        input.validate()?;
        let owner = InputOwner::new(state);
        if !owner.current() {
            return Err(V8BindingError::Allocation(
                "retired pointer boundary document",
            ));
        }
        if target.is_some_and(|node| {
            state.dom.borrow().node_document(node).ok().flatten() != Some(state.document_root())
        }) {
            return Err(V8BindingError::Allocation(
                "foreign pointer boundary target",
            ));
        }
        let claim = Rc::new(());
        let skip = native_pointer_owns_routing(state, input.id);
        let old = {
            let mut registry = state.native_pointers.borrow_mut();
            if !registry.pointers.contains_key(&input.id)
                && registry.pointers.len() >= oryn_common::MAX_NATIVE_POINTER_SOURCES
            {
                return Err(V8BindingError::Allocation(
                    "native pointer source capacity exceeded",
                ));
            }
            if !skip && input.device != NativePointerDevice::Touch {
                state.dom.borrow().prepare_pointer_hover(
                    state.document_root(),
                    input.id,
                    target,
                )?;
            }
            let pointer = registry.pointers.entry(input.id).or_default();
            if pointer.boundary_claim.is_some() {
                return Err(V8BindingError::Allocation("pointer boundary already owned"));
            }
            if !skip {
                pointer.boundary_claim = Some(claim.clone());
            }
            pointer.target
        };
        Ok(Self {
            state: state.clone(),
            owner,
            claim,
            input: input.clone(),
            old,
            target,
            committed: false,
            started: false,
            done: skip,
            canceled: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.state.clone()
    }
    fn claimed(&self) -> bool {
        self.state
            .native_pointers
            .borrow()
            .pointers
            .get(&self.input.id)
            .and_then(|p| p.boundary_claim.as_ref())
            .is_some_and(|claim| Rc::ptr_eq(claim, &self.claim))
    }
    pub fn retry_safe(&self) -> bool {
        !self.done && self.owner.current() && self.claimed()
    }
    pub fn canceled(&self) -> bool {
        self.canceled
    }
    fn release(&self) {
        let pending = {
            let mut registry = self.state.native_pointers.borrow_mut();
            if let Some(pointer) = registry.pointers.get_mut(&self.input.id) {
                if pointer
                    .boundary_claim
                    .as_ref()
                    .is_some_and(|claim| Rc::ptr_eq(claim, &self.claim))
                {
                    pointer.boundary_claim = None;
                    if self.started {
                        pointer.boundary_retry = None;
                        pointer.boundary_pending.take()
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };
        drop(pending);
    }
    pub fn cancel(&mut self) {
        if self.done {
            return;
        }
        self.release();
        self.done = true;
        self.canceled = true;
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if !self.owner.current() || !self.claimed() {
            self.cancel();
            return Ok(true);
        }
        if self.state.execution_suspended()
            || scope.is_execution_terminating()
            || self.state.native_pointers.borrow().boundary_draining
        {
            return Ok(false);
        }
        if !self.committed {
            // Finish any older stationary publication before installing this sample.
            boundary_dispatch::drain_owned(scope, &self.state, self.input.id)?;
            if !self.owner.current() {
                self.cancel();
                return Ok(true);
            }
            let mut registry = self.state.native_pointers.borrow_mut();
            let pointer = registry.pointers.get_mut(&self.input.id).unwrap();
            pointer.target = self.target;
            pointer.buttons = self.input.buttons;
            pointer.last_input = Some(self.input.clone());
            if self.input.buttons == 0 {
                pointer.down_targets.clear();
                pointer.drag_start = None;
                pointer.prevent_mouse = false;
            }
            self.committed = true;
        }
        let result = if self.started {
            boundary_dispatch::drain_owned(scope, &self.state, self.input.id)
        } else {
            let result = boundary_dispatch::dispatch_owned(
                scope,
                &self.state,
                &self.input,
                self.old,
                self.target,
            );
            self.started = self
                .state
                .native_pointers
                .borrow()
                .pointers
                .get(&self.input.id)
                .is_some_and(|p| p.boundary_pending.is_some());
            result
        };
        if !self.owner.current() {
            self.cancel();
            return Ok(true);
        }
        result?;
        self.release();
        self.done = true;
        Ok(true)
    }
}
impl Drop for NativePointerBoundary {
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
    fn claimed_boundary_retains_suffix_and_reset_never_clears_reused_lane() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=a></div><div id=b></div>")
                .into_runtime()
                .unwrap(),
        ));
        let a = dom.borrow().find_by_id("a").unwrap();
        let b = dom.borrow().find_by_id("b").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let input = NativePointerInput::default();
        transition_native_pointer_boundary(scope, &state, &input, Some(a)).unwrap();
        eval(
            scope,
            "globalThis.seen=[];a.addEventListener('mouseout',e=>{globalThis.saved=e;seen.push('once')},{once:true});a.onmouseout=e=>seen.push(e===saved?'suffix':'wrong');b.onmouseover=()=>seen.push('over')",
        );
        let mut receipt = NativePointerBoundary::new(&state, &input, Some(b)).unwrap();
        assert!(NativePointerBoundary::new(&state, &input, None).is_err());
        assert!(native_pointer_hover_inputs(&state).is_empty());
        native_platform::publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(receipt.retry_safe());
        boundary_dispatch::drain(scope, &state, input.id).unwrap();
        assert!(eval(scope, "seen.join()==='once'").is_true());
        assert!(transition_native_pointer_boundary(scope, &state, &input, None).is_err());
        assert!(receipt.poll(scope).unwrap());
        assert!(!receipt.canceled());
        receipt.cancel();
        assert!(!receipt.canceled());
        assert!(eval(scope, "seen.join()==='once,suffix,over'").is_true());
        eval(scope, "b.onmouseout=()=>seen.push('out')");
        let mut old = NativePointerBoundary::new(&state, &input, Some(a)).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(old.poll(scope).is_err());
        reset_native_pointer_sources(&state);
        let mut fresh = NativePointerBoundary::new(&state, &input, Some(b)).unwrap();
        assert!(old.poll(scope).unwrap());
        assert!(old.canceled());
        assert!(fresh.retry_safe());
        assert!(fresh.poll(scope).unwrap());
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
    }
}
