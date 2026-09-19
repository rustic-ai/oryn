//! Contact suppression and pinch cancellation retain their event suffixes.
use super::*;
struct CancelContact {
    state: Rc<V8BindingState>,
    owner: InputOwner,
    input: NativePointerInput,
    target: NodeId,
    contact: Option<Box<native_touch::Contact>>,
    pinch: Option<
        Box<(
            native_touch::PinchCandidate,
            Vec<(i32, native_touch::Contact)>,
        )>,
    >,
    phase: u8,
    event: Option<Box<Event>>,
    capture: Option<Box<Capture>>,
    boundary_started: bool,
}
impl CancelContact {
    fn new(state: &Rc<V8BindingState>, input: &NativePointerInput, target: NodeId) -> Self {
        let mut input = input.clone();
        input.button = -1;
        input.buttons = 0;
        input.pressure = 0.;
        Self {
            state: state.clone(),
            owner: InputOwner::new(state),
            contact: state
                .native_pointers
                .borrow()
                .pointers
                .get(&input.id)
                .and_then(|pointer| pointer.touch_contact.clone())
                .map(|contact| allocation::boxed(|| contact)),
            input,
            target,
            pinch: None,
            phase: 0,
            event: None,
            capture: None,
            boundary_started: false,
        }
    }
    fn current(&self) -> bool {
        self.owner.current()
            && self.contact.as_ref().is_none_or(|contact| {
                self.state
                    .native_pointers
                    .borrow()
                    .pointers
                    .get(&self.input.id)
                    .and_then(|pointer| pointer.touch_contact.as_ref())
                    .is_some_and(|live| live.same_contact(contact))
            })
            && self.pinch.as_deref().is_none_or(|(candidate, contacts)| {
                let registry = self.state.native_pointers.borrow();
                registry
                    .pinch
                    .as_ref()
                    .is_some_and(|live| live.same_update(candidate))
                    && contacts.iter().all(|(id, contact)| {
                        registry
                            .pointers
                            .get(id)
                            .and_then(|pointer| pointer.touch_contact.as_ref())
                            .is_some_and(|live| live.same_contact(contact))
                    })
            })
    }
    #[inline(never)]
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if !self.current() {
                self.event.take();
                self.capture.take();
                return Ok(true);
            }
            if self.state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.phase {
                0 => {
                    if self.event.is_none() {
                        self.event = Some(allocation::try_boxed(|| {
                            Event::new(
                                scope,
                                &self.state,
                                &self.input,
                                self.target,
                                "pointercancel",
                                true,
                                false,
                                None,
                            )
                        })?);
                    }
                    if self.event.as_mut().unwrap().poll(scope)?.is_none() {
                        return Ok(false);
                    }
                    self.event.take();
                    self.phase = 1;
                }
                1 => {
                    if self.event.is_none() {
                        if let Some((target, object)) =
                            prepare_touch_event(scope, &self.state, &self.input, "touchcancel")?
                        {
                            self.event = Some(allocation::try_boxed(|| {
                                Event::from_object(
                                    scope,
                                    &self.state,
                                    target,
                                    "touchcancel",
                                    object,
                                )
                            })?);
                        }
                    }
                    if let Some(event) = &mut self.event {
                        if event.poll(scope)?.is_none() {
                            return Ok(false);
                        }
                    }
                    self.event.take();
                    self.phase = 2;
                }
                2 => {
                    if let Some(pointer) = self
                        .state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&self.input.id)
                    {
                        pointer.pending_capture = None;
                    }
                    self.capture =
                        Some(allocation::boxed(|| Capture::new(&self.state, &self.input)));
                    self.phase = 3;
                }
                3 => {
                    if !self.capture.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.capture.take();
                    self.phase = 4;
                }
                4 => {
                    if !self.boundary_started {
                        boundary_dispatch::drain_owned(scope, &self.state, self.input.id)?;
                        let result = boundary_dispatch::dispatch_owned(
                            scope,
                            &self.state,
                            &self.input,
                            Some(self.target),
                            None,
                        );
                        self.boundary_started = self
                            .state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&self.input.id)
                            .is_some_and(|pointer| pointer.boundary_pending.is_some());
                        result?;
                    } else {
                        boundary_dispatch::drain_owned(scope, &self.state, self.input.id)?;
                    }
                    self.phase = 5;
                }
                _ => {
                    if let Some(pointer) = self
                        .state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&self.input.id)
                    {
                        pointer.target = None;
                    }
                    return Ok(true);
                }
            }
        }
    }
}
struct Pinch {
    state: Rc<V8BindingState>,
    owner: InputOwner,
    id: i32,
    prepared: bool,
    candidate: Option<Box<native_touch::PinchCandidate>>,
    contacts: Vec<(i32, native_touch::Contact, NativePointerInput)>,
    geometry: Option<visual_viewport::Geometry>,
    index: usize,
    cancel: Option<Box<CancelContact>>,
    result: Option<bool>,
}
impl Pinch {
    fn new(state: &Rc<V8BindingState>, id: i32) -> Self {
        Self {
            state: state.clone(),
            owner: InputOwner::new(state),
            id,
            prepared: false,
            candidate: None,
            contacts: Vec::new(),
            geometry: None,
            index: 0,
            cancel: None,
            result: None,
        }
    }
    fn current(&self) -> bool {
        self.owner.current()
            && self.candidate.as_ref().is_some_and(|candidate| {
                let registry = self.state.native_pointers.borrow();
                registry
                    .pinch
                    .as_ref()
                    .is_some_and(|pinch| pinch.same_update(candidate))
                    && self.contacts.iter().all(|(id, contact, _)| {
                        registry
                            .pointers
                            .get(id)
                            .and_then(|pointer| pointer.touch_contact.as_ref())
                            .is_some_and(|live| live.same_contact(contact))
                    })
            })
    }
    #[inline(never)]
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        if !self.owner.current() {
            self.cancel.take();
            self.result = Some(true);
            return Ok(self.result);
        }
        if !self.prepared {
            let Some(mut candidate) = self.state.native_pointers.borrow().pinch.clone() else {
                self.result = Some(false);
                return Ok(self.result);
            };
            if !candidate.contains(self.id) {
                self.result = Some(false);
                return Ok(self.result);
            }
            let contacts = pinch_contacts(&self.state)
                .into_iter()
                .filter(|entry| candidate.contains(entry.0))
                .collect::<Vec<_>>();
            if contacts.len() != 2 {
                self.state.native_pointers.borrow_mut().pinch = None;
                self.result = Some(false);
                return Ok(self.result);
            }
            let a = &contacts[0];
            let b = &contacts[1];
            let was_active = candidate.active();
            let mut geometry = *self.state.native_visual_viewport.borrow();
            if candidate
                .update(
                    &mut geometry,
                    (&a.1, a.0, a.2.x, a.2.y),
                    (&b.1, b.0, b.2.x, b.2.y),
                )
                .is_none()
            {
                self.state.native_pointers.borrow_mut().pinch = None;
                self.result = Some(false);
                return Ok(self.result);
            }
            self.state.native_pointers.borrow_mut().pinch = Some(candidate.clone());
            if !candidate.active() {
                self.result = Some(false);
                return Ok(self.result);
            }
            if !was_active {
                let mut registry = self.state.native_pointers.borrow_mut();
                for (id, _, _) in &contacts {
                    if let Some(pointer) = registry.pointers.get_mut(id) {
                        pointer.touch_zooming = true;
                        pointer.prevent_mouse = true;
                        pointer.down_targets.clear();
                        pointer.click_details.clear();
                        pointer.click_history = None;
                        pointer.drag_start = None;
                        pointer.selection = None;
                        pointer.pending_capture = None;
                    }
                }
            }
            self.index = if was_active { contacts.len() } else { 0 };
            self.contacts = contacts;
            self.candidate = Some(allocation::boxed(|| candidate));
            self.geometry = Some(geometry);
            self.prepared = true;
        }
        while self.index < self.contacts.len() {
            if !self.current() {
                self.cancel.take();
                self.result = Some(true);
                return Ok(self.result);
            }
            if self.cancel.is_none() {
                let (id, contact, last) = &self.contacts[self.index];
                let target = self
                    .state
                    .native_pointers
                    .borrow()
                    .pointers
                    .get(id)
                    .and_then(|pointer| pointer.capture.or(pointer.target))
                    .unwrap_or(contact.target);
                let mut cancel =
                    allocation::boxed(|| CancelContact::new(&self.state, last, target));
                cancel.pinch = Some(allocation::boxed(|| {
                    (
                        self.candidate.as_deref().unwrap().clone(),
                        self.contacts
                            .iter()
                            .map(|(id, contact, _)| (*id, contact.clone()))
                            .collect(),
                    )
                }));
                self.cancel = Some(cancel);
            }
            if !self.cancel.as_mut().unwrap().poll(scope)? {
                return Ok(None);
            }
            self.cancel.take();
            self.index += 1;
        }
        if self.current() {
            *self.state.native_visual_viewport.borrow_mut() = self.geometry.unwrap();
        }
        self.result = Some(true);
        Ok(self.result)
    }
}
pub(super) struct Touch {
    state: Rc<V8BindingState>,
    owner: InputOwner,
    input: NativePointerInput,
    action: NativePointerAction,
    phase: u8,
    contact: Option<Box<native_touch::Contact>>,
    was_active: bool,
    zooming: bool,
    event: Option<Box<Event>>,
    pinch: Option<Box<Pinch>>,
    cancel: Option<Box<CancelContact>>,
    pan: Option<Box<native_touch::Pan>>,
    movement: Option<((f64, f64), (f64, f64))>,
    active: bool,
    result: Option<Option<NodeId>>,
}
impl Touch {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
        action: NativePointerAction,
    ) -> Self {
        Self {
            state: state.clone(),
            owner: InputOwner::new(state),
            input: input.clone(),
            action,
            phase: 0,
            contact: None,
            was_active: false,
            zooming: false,
            event: None,
            pinch: None,
            cancel: None,
            pan: None,
            movement: None,
            active: false,
            result: None,
        }
    }
    #[inline(never)]
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<Option<NodeId>>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            if !self.owner.current() {
                self.event.take();
                self.pinch.take();
                self.cancel.take();
                self.result = Some(Some(self.state.document_root()));
                return Ok(self.result);
            }
            if self.state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            let state = self.state.clone();
            let input = &self.input;
            match self.phase {
                0 => {
                    if input.device != NativePointerDevice::Touch
                        || state
                            .native_pointers
                            .borrow()
                            .pointers
                            .get(&input.id)
                            .is_some_and(|pointer| pointer.range.is_some())
                    {
                        self.result = Some(None);
                        return Ok(self.result);
                    }
                    self.zooming = state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .is_some_and(|pointer| pointer.touch_zooming);
                    if matches!(
                        self.action,
                        NativePointerAction::Up | NativePointerAction::Cancel
                    ) {
                        let mut registry = state.native_pointers.borrow_mut();
                        if registry
                            .pinch
                            .as_ref()
                            .is_some_and(|pinch| pinch.contains(input.id))
                        {
                            registry.pinch = None;
                        }
                        if self.zooming {
                            registry.pointers.remove(&input.id);
                            self.result = Some(Some(state.document_root()));
                            return Ok(self.result);
                        }
                    }
                    if self.zooming {
                        self.phase = 2;
                        continue;
                    }
                    if state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .is_some_and(|pointer| pointer.touch_suppressed)
                    {
                        if matches!(
                            self.action,
                            NativePointerAction::Up | NativePointerAction::Cancel
                        ) {
                            state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .remove(&input.id);
                        }
                        self.result = Some(Some(state.document_root()));
                        return Ok(self.result);
                    }
                    self.contact = state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .and_then(|pointer| pointer.touch_contact.clone())
                        .map(|contact| allocation::boxed(|| contact));
                    let Some(contact) = &self.contact else {
                        self.result = Some(None);
                        return Ok(self.result);
                    };
                    self.was_active = contact.active();
                    if matches!(
                        self.action,
                        NativePointerAction::Up | NativePointerAction::Cancel
                    ) {
                        if self.was_active {
                            state
                                .native_pointers
                                .borrow_mut()
                                .pointers
                                .remove(&input.id);
                            self.result = Some(Some(contact.target));
                        } else {
                            self.result = Some(None)
                        }
                        return Ok(self.result);
                    }
                    if self.action != NativePointerAction::Move {
                        self.result = Some(None);
                        return Ok(self.result);
                    }
                    self.phase = 1;
                }
                1 => {
                    if !self.was_active {
                        if self.event.is_none() {
                            if let Some((target, object)) =
                                prepare_touch_event(scope, &state, input, "touchmove")?
                            {
                                self.event = Some(allocation::try_boxed(|| {
                                    Event::from_object(scope, &state, target, "touchmove", object)
                                })?);
                            }
                        }
                        if let Some(event) = &mut self.event {
                            let Some(accepted) = event.poll(scope)? else {
                                return Ok(None);
                            };
                            if !accepted {
                                self.contact.as_mut().unwrap().prevent_default();
                                if self.owner.current() {
                                    if let Some(pointer) = state
                                        .native_pointers
                                        .borrow_mut()
                                        .pointers
                                        .get_mut(&input.id)
                                    {
                                        pointer.prevent_mouse = true;
                                        pointer.down_targets.clear();
                                    }
                                }
                            }
                        }
                        self.event.take();
                    }
                    if !state
                        .native_pointers
                        .borrow()
                        .pointers
                        .get(&input.id)
                        .and_then(|pointer| pointer.touch_contact.as_ref())
                        .is_some_and(|contact| contact.same_contact(self.contact.as_ref().unwrap()))
                    {
                        self.result = Some(Some(state.document_root()));
                        return Ok(self.result);
                    }
                    if let Some(pointer) = state
                        .native_pointers
                        .borrow_mut()
                        .pointers
                        .get_mut(&input.id)
                    {
                        pointer.touch_contact = self.contact.as_deref().cloned();
                    }
                    self.phase = 2;
                }
                2 => {
                    if self.pinch.is_none() {
                        self.pinch = Some(allocation::boxed(|| Pinch::new(&state, input.id)));
                    }
                    let Some(active) = self.pinch.as_mut().unwrap().poll(scope)? else {
                        return Ok(None);
                    };
                    self.pinch.take();
                    if self.zooming || active {
                        self.result = Some(Some(
                            self.contact
                                .as_ref()
                                .map_or(state.document_root(), |contact| contact.target),
                        ));
                        return Ok(self.result);
                    }
                    if state
                        .native_pointers
                        .borrow()
                        .pinch
                        .as_ref()
                        .is_some_and(|pinch| pinch.contains(input.id))
                    {
                        self.result = Some(None);
                        return Ok(self.result);
                    }
                    self.phase = 3;
                }
                3 => {
                    let contact = self.contact.as_mut().unwrap();
                    self.movement = contact.movement(input.x, input.y);
                    self.active = contact.active();
                    let target = contact.target;
                    let cancel_target = {
                        let mut registry = state.native_pointers.borrow_mut();
                        let Some(pointer) = registry.pointers.get_mut(&input.id) else {
                            self.result = Some(Some(state.document_root()));
                            return Ok(self.result);
                        };
                        if pointer.touch_suppressed {
                            self.result = Some(Some(state.document_root()));
                            return Ok(self.result);
                        }
                        pointer.touch_contact = Some((**contact).clone());
                        if self.active && !self.was_active {
                            pointer.prevent_mouse = true;
                            pointer.down_targets.clear();
                            pointer.click_details.clear();
                            pointer.click_history = None;
                            pointer.drag_start = None;
                            pointer.selection = None;
                            pointer.pending_capture = None;
                            Some(pointer.capture.or(pointer.target).unwrap_or(target))
                        } else {
                            None
                        }
                    };
                    if let Some(target) = cancel_target {
                        self.cancel = Some(allocation::boxed(|| {
                            CancelContact::new(&state, input, target)
                        }));
                    }
                    self.phase = 4;
                }
                4 => {
                    if let Some(cancel) = &mut self.cancel {
                        if !cancel.poll(scope)? {
                            return Ok(None);
                        }
                    }
                    self.cancel.take();
                    self.phase = 5;
                }
                _ => {
                    let contact = self.contact.as_ref().unwrap();
                    if let Some((start, end)) = self.movement {
                        if self.pan.is_none() {
                            self.pan = Some(allocation::boxed(|| {
                                native_touch::Pan::new(&state, contact.target, start, end)
                            }));
                        }
                        self.pan.as_mut().unwrap().poll(&state)?;
                    }
                    self.result = Some(self.active.then_some(contact.target));
                    return Ok(self.result);
                }
            }
        }
    }
}

#[cfg(test)]
mod stack_tests {
    use super::*;
    #[test]
    fn child_receipts_have_bounded_inline_storage() {
        // Size guards complement the behavioral GC regression. Boxed storage
        // alone is insufficient; allocation helpers also isolate debug returns.
        for (name, size) in [
            ("Event", std::mem::size_of::<Event>()),
            ("Capture", std::mem::size_of::<Capture>()),
            ("CancelContact", std::mem::size_of::<CancelContact>()),
            ("Pinch", std::mem::size_of::<Pinch>()),
            ("Touch", std::mem::size_of::<Touch>()),
            (
                "NativePointerDispatch",
                std::mem::size_of::<NativePointerDispatch>(),
            ),
        ] {
            assert!(
                size <= 2048,
                "{name} inlines {size} bytes of continuation storage"
            );
        }
    }
    #[test]
    fn pinch_callbacks_collect_and_resume_without_replaying_prefix() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<!doctype html><body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://example.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            generated_nodes: Vec::new(),
            pseudo_styles: Default::default(),
            viewport: Some((400., 300.)),
            geometry: BTreeMap::from([(
                node_key(target),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 400.,
                    height: 300.,
                }),
            )]),
            ..Default::default()
        });
        fn collect(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.low_memory_notification();
        }
        let collect = v8::Function::new(scope, collect).unwrap();
        let name = v8::String::new(scope, "collectAtCallback").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), collect.into());
        let code=v8::String::new(scope,"globalThis.log=[];for(const type of ['pointercancel','touchcancel','pointerup','touchend','click'])document.addEventListener(type,e=>{log.push(type);collectAtCallback()})").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let mut first = NativePointerInput {
            id: 1,
            device: NativePointerDevice::Touch,
            x: 100.,
            y: 100.,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            ..Default::default()
        };
        let mut second = NativePointerInput {
            id: 2,
            primary: false,
            x: 200.,
            ..first.clone()
        };
        dispatch_native_pointer(scope, &state, &first, NativePointerAction::Down).unwrap();
        dispatch_native_pointer(scope, &state, &second, NativePointerAction::Down).unwrap();
        second.x = 300.;
        second.button = -1;
        let mut move_job =
            NativePointerDispatch::new(&state, &second, NativePointerAction::Move).unwrap();
        for _ in 0..4 {
            native_platform::publication::fail_after_next_callback();
            assert!(move_job.poll(scope).is_err());
            assert_eq!(state.native_visual_viewport.borrow().scale, 1.);
        }
        assert!(move_job.poll(scope).unwrap().is_some());
        assert!(move_job.poll(scope).unwrap().is_some());
        assert_eq!(state.native_visual_viewport.borrow().scale, 2.);
        assert!(native_pointer_owns_routing(&state, first.id));
        assert!(native_pointer_owns_routing(&state, second.id));
        first.buttons = 0;
        first.button = 0;
        first.pressure = 0.;
        dispatch_native_pointer(scope, &state, &first, NativePointerAction::Up).unwrap();
        second.buttons = 0;
        second.button = 0;
        second.pressure = 0.;
        dispatch_native_pointer(scope, &state, &second, NativePointerAction::Up).unwrap();
        assert!(state.native_pointers.borrow().pinch.is_none());
        assert!(!touch_gesture_active(&state));
        let code = v8::String::new(
            scope,
            "log.join(',')==='pointercancel,touchcancel,pointercancel,touchcancel'",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
