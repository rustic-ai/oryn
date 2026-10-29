//! Shared dialog transition, retained across callback preparation failures.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Show,
    ShowModal,
    Close,
    RequestClose,
}
pub(super) struct Job {
    owner: Rc<V8BindingState>,
    node: NodeId,
    source: Option<NodeId>,
    action: Action,
    value: Option<String>,
    document: NodeId,
    generation: u64,
    _lease: oryn_dom::TrackedNodeLease,
    phase: u8,
    event: Option<retained_dom_event::Event>,
    focus: Option<retained_focus::Transition>,
    previous: Option<NodeId>,
    changed: bool,
}
impl Job {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        node: NodeId,
        action: Action,
        value: Option<String>,
        source: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        // End the read before acquiring the tracked lease's mutable DOM borrow.
        // A temporary inside the struct initializer otherwise lives to its end.
        let document = state
            .dom
            .borrow()
            .node_document(node)?
            .unwrap_or(state.document_root());
        let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
        Ok(Self {
            owner: state.clone(),
            node,
            source,
            action,
            value,
            document,
            generation: state.document_generation(),
            _lease: lease,
            phase: 0,
            event: None,
            focus: None,
            previous: None,
            changed: false,
        })
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 9 {
                return Ok(true);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if self.owner.document_generation() != self.generation
                || self
                    .owner
                    .dom
                    .borrow()
                    .node_document(self.node)
                    .ok()
                    .flatten()
                    != Some(self.document)
            {
                self.cancel();
                return Ok(true);
            }
            let opening = matches!(self.action, Action::Show | Action::ShowModal);
            match self.phase {
                0 => {
                    let dom = self.owner.dom.borrow();
                    let open = dom
                        .attribute_in_null_namespace(self.node, "open")?
                        .is_some();
                    if open == opening {
                        self.phase = 9;
                        return Ok(true);
                    }
                    if self.action == Action::ShowModal
                        && (!dom.is_connected(self.node)?
                            || dom.is_popover_open(self.node)
                            || self.document != self.owner.document_root()
                            || self.owner.execution_retired())
                    {
                        if self.source.is_some() {
                            self.phase = 9;
                            return Ok(true);
                        }
                        return Err(V8BindingError::Unsupported {
                            interface: "HTMLDialogElement".into(),
                            member: "InvalidStateError".into(),
                        });
                    }
                    self.previous = if opening {
                        dom.focused()
                    } else {
                        dom.top_layer()
                            .find(|entry| {
                                entry.node == self.node
                                    && entry.kind == oryn_dom::TopLayerKind::ModalDialog
                            })
                            .and_then(|entry| entry.previous_focus)
                    };
                    drop(dom);
                    self.phase = if self.action == Action::RequestClose {
                        1
                    } else {
                        2
                    };
                }
                1 => {
                    if self.event.is_none() {
                        let event = create_event_object(scope, &self.owner, "cancel", false, true)?;
                        let id = event_id(scope, event)?;
                        self.owner
                            .events
                            .borrow_mut()
                            .get_mut(&id)
                            .ok_or(V8BindingError::Allocation("dialog cancel"))?
                            .is_trusted = true;
                        self.event = Some(retained_dom_event::Event::new(
                            scope,
                            &self.owner,
                            self.node,
                            "cancel",
                            event,
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &self.owner)?
                    else {
                        return Ok(false);
                    };
                    self.event = None;
                    if !accepted {
                        self.phase = 9;
                        return Ok(true);
                    }
                    self.phase = 2;
                }
                2 => {
                    if self.event.is_none() {
                        let event = top_layer::event(
                            scope,
                            &self.owner,
                            "beforetoggle",
                            if opening { "closed" } else { "open" },
                            if opening { "open" } else { "closed" },
                            self.source,
                            opening,
                        )?;
                        self.event = Some(retained_dom_event::Event::new(
                            scope,
                            &self.owner,
                            self.node,
                            "beforetoggle",
                            event,
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &self.owner)?
                    else {
                        return Ok(false);
                    };
                    self.event = None;
                    if !accepted {
                        self.phase = 9;
                        return Ok(true);
                    }
                    self.phase = 3;
                }
                3 => {
                    let open = self
                        .owner
                        .dom
                        .borrow()
                        .attribute_in_null_namespace(self.node, "open")?
                        .is_some();
                    if open == opening
                        || self.action == Action::ShowModal
                            && (!self.owner.dom.borrow().is_connected(self.node)?
                                || self.owner.execution_retired())
                    {
                        self.phase = 9;
                        return Ok(true);
                    }
                    if opening {
                        self.previous = self.owner.dom.borrow().focused();
                        if self.action == Action::ShowModal {
                            self.owner.dom.borrow_mut().reserve_top_layer(1)?;
                        }
                        self.owner
                            .dom
                            .borrow_mut()
                            .set_attribute_ns(self.node, None, "open", "")?;
                        if self.action == Action::ShowModal {
                            self.owner
                                .dom
                                .borrow_mut()
                                .add_to_top_layer(self.node, oryn_dom::TopLayerKind::ModalDialog)?;
                            self.owner
                                .dom
                                .borrow_mut()
                                .set_top_layer_source(self.node, self.source);
                        }
                    } else {
                        let mut dom = self.owner.dom.borrow_mut();
                        if let Some(value) = self.value.take() {
                            dom.arena_mut().get_mut(self.node)?.dialog_return_value = value;
                        }
                        dom.remove_attribute_ns(self.node, None, "open")?;
                        dom.remove_from_top_layer(self.node);
                    }
                    self.owner.layout_cache_key.borrow_mut().take();
                    self.changed = true;
                    self.phase = 4;
                }
                4 => {
                    if self.focus.is_none() {
                        let target = if opening {
                            let dom = self.owner.dom.borrow();
                            let nodes = dom.flat_tree_elements(self.node)?;
                            nodes
                                .iter()
                                .map(|entry| entry.0)
                                .find(|node| {
                                    dom.attribute_in_null_namespace(*node, "autofocus")
                                        .ok()
                                        .flatten()
                                        .is_some()
                                })
                                .or_else(|| {
                                    nodes.iter().map(|entry| entry.0).find(|node| {
                                        focus_navigation::eligible_for(&self.owner, *node, false)
                                            .unwrap_or(false)
                                    })
                                })
                                .or(Some(self.node))
                        } else {
                            let dom = self.owner.dom.borrow();
                            let mut cursor = dom.focused();
                            let mut inside = false;
                            while let Some(node) = cursor {
                                if node == self.node {
                                    inside = true;
                                    break;
                                }
                                cursor = dom.flat_tree_parent(node)?;
                            }
                            self.previous.filter(|_| inside)
                        };
                        if let Some(target) = target
                            && self.owner.dom.borrow().is_connected(target)?
                            && !self.owner.dom.borrow().is_inert(target)?
                        {
                            self.focus =
                                Some(retained_focus::Transition::new(&self.owner, Some(target))?);
                        }
                    }
                    if let Some(focus) = &mut self.focus
                        && !focus.poll(scope, &self.owner)?
                    {
                        return Ok(false);
                    }
                    self.focus = None;
                    self.phase = 5;
                }
                5 => {
                    top_layer::queue(
                        scope,
                        &self.owner,
                        self.node,
                        if opening { "closed" } else { "open" },
                        if opening { "open" } else { "closed" },
                        self.source,
                    )?;
                    self.phase = if opening { 9 } else { 6 };
                }
                6 => {
                    let target = DeferredDomTarget::new(
                        &self.owner.dom,
                        self.node,
                        self.owner.document_root(),
                    )?;
                    self.owner
                        .deferred_platform_tasks
                        .borrow_mut()
                        .try_reserve(1)
                        .map_err(|_| V8BindingError::Allocation("dialog close task"))?;
                    self.owner.deferred_platform_tasks.borrow_mut().push_back(
                        DeferredPlatformTask::DomEvent {
                            target,
                            event_type: "close",
                            bubbles: false,
                            stream_epoch: None,
                        },
                    );
                    self.phase = 9;
                }
                _ => unreachable!(),
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        self.event = None;
        if let Some(focus) = &mut self.focus {
            focus.cancel(&self.owner)
        }
        self.focus = None;
        self.phase = 9;
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel()
    }
}

#[derive(Default)]
pub(super) struct Registry {
    queue: VecDeque<Job>,
    retry: Option<Instant>,
    polling: bool,
}
pub(super) fn invoke(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    action: Action,
    value: Option<String>,
) -> Result<(), V8BindingError> {
    {
        let dom = state.dom.borrow();
        let open = dom.attribute_in_null_namespace(node, "open")?.is_some();
        if action == Action::ShowModal && open && !dom.is_modal_dialog(node)
            || action == Action::Show && dom.is_modal_dialog(node)
        {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLDialogElement".into(),
                member: "InvalidStateError".into(),
            });
        }
    }
    {
        let mut registry = state.dialogs.borrow_mut();
        if registry.queue.len() >= 1024 {
            return Err(V8BindingError::Allocation("dialog transition limit"));
        }
        registry
            .queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("dialog transition queue"))?;
    }
    let mut job = Job::new(state, node, action, value, None)?;
    let result = job.poll(scope);
    if matches!(&result, Err(V8BindingError::Unsupported { .. })) {
        return result.map(|_| ());
    }
    if !matches!(result, Ok(true)) {
        let mut registry = state.dialogs.borrow_mut();
        registry.queue.push_back(job);
        registry.retry = Some(Instant::now() + Duration::from_millis(10));
    }
    result.map(|_| ())
}
pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    let registry = state.dialogs.borrow();
    (!registry.queue.is_empty() && !state.execution_suspended()).then(|| {
        registry.retry.map_or(Duration::ZERO, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
    })
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.dialogs.borrow().polling || !delay(state).is_some_and(|delay| delay.is_zero()) {
        return Ok(0);
    }
    let Some(mut job) = state.dialogs.borrow_mut().queue.pop_front() else {
        return Ok(0);
    };
    state.dialogs.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.dialogs.borrow_mut().polling = false
        }
    }
    let _guard = Guard(state);
    match job.poll(scope) {
        Ok(true) => {
            state.dialogs.borrow_mut().retry = None;
            Ok(1)
        }
        result => {
            let mut registry = state.dialogs.borrow_mut();
            registry.queue.push_front(job);
            registry.retry = Some(Instant::now() + Duration::from_millis(10));
            result.map(|_| 0)
        }
    }
}
pub(super) fn retire(state: &V8BindingState) {
    let jobs = std::mem::take(&mut state.dialogs.borrow_mut().queue);
    drop(jobs);
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
    fn retained_dialog_close_does_not_replay_cancel_or_publish_authored_state() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<dialog id=d open data-oryn-return-value=forged></dialog>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(234), 1).unwrap();
        assert!(eval(scope, "d.returnValue===''").is_true());
        eval(
            scope,
            "globalThis.log=[];d.addEventListener('cancel',e=>{globalThis.original=e;log.push('first')},{once:true});d.addEventListener('cancel',e=>log.push(e===original?'second':'wrong'));d.addEventListener('beforetoggle',()=>log.push('beforetoggle'))",
        );
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "d")
            .unwrap()
            .unwrap();
        let mut job = Job::new(
            &state,
            node,
            Action::RequestClose,
            Some("native".into()),
            None,
        )
        .unwrap();
        assert!(
            state.dom.try_borrow_mut().is_ok(),
            "dialog admission must release its DOM borrows before publication"
        );
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(eval(scope, "d.open&&log.join()==='first'&&d.returnValue===''").is_true());
        assert!(job.poll(scope).unwrap());
        assert!(job.poll(scope).unwrap());
        assert!(eval(scope,"!d.open&&d.returnValue==='native'&&d.getAttribute('data-oryn-return-value')==='forged'&&log.join()==='first,second,beforetoggle'").is_true());
    }
    #[test]
    fn argument_conversion_can_mutate_dialog_before_shared_close() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<dialog id=d open></dialog>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(235), 1).unwrap();
        assert!(eval(scope,"(()=>{const d=document.getElementById('d');let calls=0;d.close({toString(){calls++;d.returnValue='conversion';return 'closed'}});return calls===1&&!d.open&&d.returnValue==='closed'&&!d.hasAttribute('data-oryn-return-value')})()").is_true());
    }
}
