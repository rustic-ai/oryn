//! Retained transition from a physical pointer press to native drag ownership.
use super::*;
#[derive(Clone, Copy)]
enum Phase {
    Prepare,
    Begin,
    Consume,
    CancelEvent,
    Capture,
    Lost,
    Got,
    Done,
}
pub struct NativePointerDragAdmission {
    owner: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    registry: Rc<()>,
    input: NativePointerInput,
    phase: Phase,
    source: Option<NodeId>,
    drag: Option<native_drag::NativeDragInput>,
    event: Option<retained_dom_event::Event>,
    old_capture: Option<NodeId>,
    new_capture: Option<NodeId>,
    result: bool,
}
impl NativePointerDragAdmission {
    pub fn new(
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
    ) -> Result<Self, V8BindingError> {
        input.validate()?;
        let registry = {
            let mut registry = state.native_pointers.borrow_mut();
            registry.synchronize(state);
            registry.boundary_owner.clone()
        };
        Ok(Self {
            owner: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            registry,
            input: input.clone(),
            phase: Phase::Prepare,
            source: None,
            drag: None,
            event: None,
            old_capture: None,
            new_capture: None,
            result: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.owner.clone()
    }
    pub fn retry_safe(&self) -> bool {
        self.current() && !matches!(self.phase, Phase::Done)
    }
    fn current(&self) -> bool {
        let state = &self.owner;
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
            && Rc::ptr_eq(
                &self.registry,
                &state.native_pointers.borrow().boundary_owner,
            )
            && state
                .native_pointers
                .borrow()
                .pointers
                .contains_key(&self.input.id)
    }
    pub fn cancel(&mut self) {
        self.drag.take();
        self.event.take();
        if self.current() && self.source.is_some() {
            abandon_native_pointer_drag(&self.owner, self.input.id);
        }
        self.phase = Phase::Done;
        self.result = false;
    }
    fn emit(
        &mut self,
        scope: &mut v8::PinScope,
        target: NodeId,
        name: &str,
    ) -> Result<bool, V8BindingError> {
        if self.event.is_none() {
            let input = NativePointerInput {
                buttons: 0,
                pressure: 0.,
                ..self.input.clone()
            };
            let event = pointer_event(scope, &self.owner, &input, name, true, false, None)?;
            self.event = Some(retained_dom_event::Event::new(
                scope,
                &self.owner,
                target,
                name,
                event,
            )?);
        }
        if self
            .event
            .as_mut()
            .unwrap()
            .poll(scope, &self.owner)?
            .is_none()
        {
            return Ok(false);
        }
        self.event.take();
        Ok(true)
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
                Phase::Prepare => {
                    let (candidate, consumed, permits) = {
                        let registry = state.native_pointers.borrow();
                        let pointer = &registry.pointers[&self.input.id];
                        (
                            pointer.drag_start,
                            pointer.drag_consumed,
                            pointer.scrollbar.is_none()
                                && pointer
                                    .selection
                                    .is_none_or(|gesture| gesture.permits_drag()),
                        )
                    };
                    if consumed {
                        self.result = true;
                        self.phase = Phase::Done;
                        continue;
                    }
                    let permits = permits
                        || candidate
                            .map(|(node, _, _)| native_drag::element_drag_source(&state, node))
                            .transpose()?
                            .flatten()
                            .is_some();
                    if !permits
                        || !self.input.primary
                        || self.input.device != NativePointerDevice::Mouse
                        || self.input.buttons & 1 == 0
                    {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let Some((source, x, y)) = candidate
                        .filter(|(_, x, y)| (self.input.x - *x).hypot(self.input.y - *y) >= 4.)
                    else {
                        self.phase = Phase::Done;
                        continue;
                    };
                    let input = &self.input;
                    let init = crate::data_transfer::EventInit {
                        client_x: x as i32,
                        client_y: y as i32,
                        screen_x: (input.screen_x - input.x + x) as i32,
                        screen_y: (input.screen_y - input.y + y) as i32,
                        button: input.button as i16,
                        buttons: input.buttons as u16,
                        ctrl_key: input.control,
                        alt_key: input.alt,
                        shift_key: input.shift,
                        meta_key: input.meta,
                        ..Default::default()
                    };
                    self.drag = Some(native_drag::NativeDragInput::new(
                        &state,
                        "begin",
                        Some(source),
                        &init,
                    )?);
                    self.source = Some(source);
                    state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&input.id)
                        .unwrap()
                        .drag_start = None;
                    self.phase = Phase::Begin;
                }
                Phase::Begin => {
                    let Some(started) = self.drag.as_mut().unwrap().poll(scope)? else {
                        return Ok(None);
                    };
                    self.drag.take();
                    if !started {
                        self.phase = Phase::Done;
                        continue;
                    }
                    self.phase = Phase::Consume;
                }
                Phase::Consume => {
                    let mut registry = state.native_pointers.borrow_mut();
                    let pointer = registry.pointers.get_mut(&self.input.id).unwrap();
                    pointer.drag_consumed = true;
                    pointer.selection = None;
                    pointer.click_history = None;
                    pointer.click_details.clear();
                    pointer.down_targets.clear();
                    pointer.pending_capture = None;
                    pointer.last_input = Some(self.input.clone());
                    self.phase = Phase::CancelEvent;
                }
                Phase::CancelEvent => {
                    let source = self.source.unwrap();
                    let connected = state.dom.borrow().is_connected(source)?;
                    if (self.event.is_some() || connected)
                        && !self.emit(scope, source, "pointercancel")?
                    {
                        return Ok(None);
                    }
                    self.phase = Phase::Capture;
                }
                Phase::Capture => {
                    let mut registry = state.native_pointers.borrow_mut();
                    let pointer = registry.pointers.get_mut(&self.input.id).unwrap();
                    if pointer
                        .pending_capture
                        .is_some_and(|node| !state.dom.borrow().is_connected(node).unwrap_or(false))
                    {
                        pointer.pending_capture = None;
                    }
                    self.old_capture = pointer.capture;
                    self.new_capture = pointer.pending_capture;
                    pointer.capture = pointer.pending_capture;
                    self.phase = if self.old_capture == self.new_capture {
                        Phase::Done
                    } else {
                        Phase::Lost
                    };
                    if matches!(self.phase, Phase::Done) {
                        self.result = true;
                    }
                }
                Phase::Lost | Phase::Got => {
                    let lost = matches!(self.phase, Phase::Lost);
                    if let Some(target) = if lost {
                        self.old_capture
                    } else {
                        self.new_capture
                    } {
                        let target = if state.dom.borrow().is_connected(target)? {
                            target
                        } else {
                            state.document_root()
                        };
                        if !self.emit(
                            scope,
                            target,
                            if lost {
                                "lostpointercapture"
                            } else {
                                "gotpointercapture"
                            },
                        )? {
                            return Ok(None);
                        }
                    }
                    self.phase = if lost { Phase::Got } else { Phase::Done };
                    if !lost {
                        self.result = true;
                    }
                }
                Phase::Done => return Ok(Some(self.result)),
            }
        }
    }
}
impl Drop for NativePointerDragAdmission {
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
    fn admission_retries_dragstart_pointercancel_and_capture_release_once() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=s draggable=true>source</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(234), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let input = NativePointerInput {
            x: 10.,
            y: 0.,
            buttons: 1,
            ..Default::default()
        };
        {
            let mut registry = state.native_pointers.borrow_mut();
            registry.synchronize(&state);
            registry.pointers.insert(
                input.id,
                PointerRecord {
                    buttons: 1,
                    drag_start: Some((source, 0., 0.)),
                    capture: Some(source),
                    ..Default::default()
                },
            );
        }
        eval(
            scope,
            "globalThis.trace=[];const s=document.getElementById('s');for(const k of ['dragstart','pointercancel','lostpointercapture'])s.addEventListener(k,()=>trace.push(k));",
        );
        let mut receipt = NativePointerDragAdmission::new(&state, &input).unwrap();
        for _ in 0..3 {
            native_platform::publication::fail_after_next_callback();
            assert!(receipt.poll(scope).is_err());
            state.set_execution_suspended(true);
            assert_eq!(receipt.poll(scope).unwrap(), None);
            state.set_execution_suspended(false);
        }
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        let registry = state.native_pointers.borrow();
        let pointer = &registry.pointers[&input.id];
        assert!(pointer.drag_consumed);
        assert!(pointer.drag_start.is_none());
        assert!(pointer.capture.is_none());
        drop(registry);
        eval(
            scope,
            "if(trace.join(',')!=='dragstart,pointercancel,lostpointercapture')throw Error('drag admission repeated prefix');",
        );
    }
    #[test]
    fn canceled_admission_cannot_clear_reused_pointer_identity() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=s draggable=true>source</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(235), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let input = NativePointerInput {
            x: 10.,
            buttons: 1,
            ..Default::default()
        };
        {
            let mut registry = state.native_pointers.borrow_mut();
            registry.synchronize(&state);
            registry.pointers.insert(
                input.id,
                PointerRecord {
                    buttons: 1,
                    drag_start: Some((source, 0., 0.)),
                    ..Default::default()
                },
            );
        }
        eval(
            scope,
            "document.getElementById('s').addEventListener('dragstart',()=>{});",
        );
        let mut receipt = NativePointerDragAdmission::new(&state, &input).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        {
            let mut registry = state.native_pointers.borrow_mut();
            registry.boundary_owner = Rc::new(());
            registry.pointers.insert(
                input.id,
                PointerRecord {
                    buttons: 2,
                    capture: Some(source),
                    ..Default::default()
                },
            );
        }
        assert_eq!(receipt.poll(scope).unwrap(), Some(false));
        drop(receipt);
        let registry = state.native_pointers.borrow();
        assert_eq!(registry.pointers[&input.id].buttons, 2);
        assert_eq!(registry.pointers[&input.id].capture, Some(source));
    }
}
