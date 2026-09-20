//! Retained activation default stages. Script dispatch keeps the synchronous
//! finish entry; native Click owns this continuation until its tail completes.
use super::super::retained_dom_event::Event;
use super::*;

pub(in super::super) struct Finish {
    // Retains the originating binding while target identity may migrate.
    _owner: Rc<V8BindingState>,
    target: v8::Global<v8::Object>,
    source: v8::Global<v8::Object>,
    activation: Option<Activation>,
    accepted: bool,
    phase: u8,
    link: Option<submissions::LinkActivation>,
    temporal_picker: Option<temporal_picker::Job>,
    select_picker: Option<select_picker::Job>,
    event: Option<Event>,
    command: Option<command::Job>,
    popover: Option<(Rc<V8BindingState>, top_layer::PendingToggle)>,
    reset: Option<retained_form_reset::Reset>,
    submission: Option<submissions::Request>,
    form: Option<v8::Global<v8::Object>>,
    child: Option<Box<retained_activation::Click>>,
    done: bool,
    terminal: Option<String>,
}
impl Finish {
    pub(in super::super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        activation: Option<Activation>,
        target: NodeId,
        source: v8::Local<v8::Object>,
        accepted: bool,
    ) -> Result<Self, V8BindingError> {
        let done = !is_click(scope, source)?;
        let target = wrap_node(scope, state, target)?;
        Ok(Self {
            _owner: state.clone(),
            target: v8::Global::new(scope, target),
            source: v8::Global::new(scope, source),
            activation,
            accepted,
            phase: 0,
            link: None,
            temporal_picker: None,
            select_picker: None,
            event: None,
            command: None,
            popover: None,
            reset: None,
            submission: None,
            form: None,
            child: None,
            done,
            terminal: None,
        })
    }
    pub(in super::super) fn set_activation(&mut self, activation: Option<Activation>) {
        self.activation = activation;
    }
    pub(in super::super) fn set_accepted(&mut self, accepted: bool) {
        self.accepted = accepted;
    }
    pub(in super::super) fn retry_safe(&self) -> bool {
        self.phase != 5
            && self
                .link
                .as_ref()
                .is_none_or(submissions::LinkActivation::retry_safe)
            && self
                .submission
                .as_ref()
                .is_none_or(submissions::Request::retry_safe)
            && self.child.as_ref().is_none_or(|child| child.retry_safe())
    }
    fn identity(
        &self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<(Rc<V8BindingState>, NodeId)>, V8BindingError> {
        let object = v8::Local::new(
            scope,
            self.activation
                .as_ref()
                .map_or(&self.target, |activation| &activation.object),
        );
        node_identity::argument(scope, object.into())
    }
    pub(in super::super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        if let Some(error) = &self.terminal {
            return Err(V8BindingError::Message(error.clone()));
        }
        if self.done {
            return Ok(true);
        }
        loop {
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            let Some((owner, node)) = self.identity(scope)? else {
                self.done = true;
                return Ok(true);
            };
            if owner.execution_suspended() {
                return Ok(false);
            }
            match self.phase {
                0 => {
                    if self.prepare(scope, &owner, node)? {
                        return Ok(true);
                    }
                }
                5 => {
                    // Native link admission and remaining permission/file defaults
                    // still use this terminal compatibility tail.
                    let source = v8::Local::new(scope, &self.source);
                    let target = v8::Local::new(scope, &self.target);
                    let Some((target_owner, target)) =
                        node_identity::argument(scope, target.into())?
                    else {
                        self.done = true;
                        return Ok(true);
                    };
                    if let Err(error) = finish_legacy(
                        scope,
                        &target_owner,
                        self.activation.take(),
                        target,
                        source,
                        self.accepted,
                    ) {
                        self.terminal = Some(error.to_string());
                        return Err(error);
                    }
                    self.done = true;
                    return Ok(true);
                }
                10 => {
                    if !self.reset.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.reset = None;
                    self.done = true;
                    return Ok(true);
                }
                11 => {
                    if !self.submission.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.submission = None;
                    self.done = true;
                    return Ok(true);
                }
                12 => {
                    if !self.command.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.command = None;
                    self.done = true;
                    return Ok(true);
                }
                13 => {
                    let (owner, popover) = self.popover.as_mut().unwrap();
                    if !popover.resume(scope, owner)? {
                        return Ok(false);
                    }
                    self.popover = None;
                    self.done = true;
                    return Ok(true);
                }
                15 => {
                    if !self.link.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.link = None;
                    self.done = true;
                    return Ok(true);
                }
                14 => {
                    if self.child.as_mut().unwrap().poll(scope)?.is_none() {
                        return Ok(false);
                    }
                    self.child = None;
                    self.done = true;
                    return Ok(true);
                }
                31 => {
                    if !self.temporal_picker.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.temporal_picker = None;
                    self.done = true;
                    return Ok(true);
                }
                30 => {
                    if !self.select_picker.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.select_picker = None;
                    self.done = true;
                    return Ok(true);
                }
                20 | 21 => {
                    if self.event.is_none() {
                        let name = if self.phase == 20 { "input" } else { "change" };
                        let event = create_event_object(scope, &owner, name, true, false)?;
                        let id = event_id(scope, event)?;
                        {
                            let mut records = owner.events.borrow_mut();
                            let record = records
                                .get_mut(&id)
                                .ok_or(V8BindingError::Allocation("activation input event"))?;
                            record.is_trusted = true;
                            record.composed = name == "input";
                        }
                        self.event = Some(Event::new(scope, &owner, node, name, event)?);
                    }
                    let event = self.event.as_mut().unwrap();
                    let event_owner = event.owner();
                    if event.poll(scope, &event_owner)?.is_none() {
                        return Ok(false);
                    }
                    self.event = None;
                    self.phase += 1;
                }
                _ => {
                    self.done = true;
                    return Ok(true);
                }
            }
        }
    }
    // Preparation owns large optional job values. Its frame must unwind
    // before polling a job which can synchronously dispatch author code.
    #[inline(never)]
    fn prepare(
        &mut self,
        scope: &mut v8::PinScope,
        owner: &Rc<V8BindingState>,
        node: NodeId,
    ) -> Result<bool, V8BindingError> {
        if !self.accepted {
            let source = v8::Local::new(scope, &self.source);
            finish_legacy(scope, owner, self.activation.clone(), node, source, false)?;
            self.done = true;
            return Ok(true);
        }
        let target_object = v8::Local::new(scope, &self.target);
        if let Some((target_owner, target)) = node_identity::argument(scope, target_object.into())?
        {
            let source = v8::Local::new(scope, &self.source);
            let (handled, job) =
                temporal_picker::prepare_click(scope, &target_owner, target, source)?;
            if handled {
                if let Some(job) = job {
                    self.temporal_picker = Some(job);
                    self.phase = 31;
                    return Ok(false);
                }
                self.done = true;
                return Ok(true);
            }
            let (handled, job) =
                select_picker::prepare_click(scope, &target_owner, target, source)?;
            if handled {
                if let Some(job) = job {
                    self.select_picker = Some(job);
                    self.phase = 30;
                    return Ok(false);
                }
                self.done = true;
                return Ok(true);
            }
        }
        let Some(activation) = self.activation.as_ref() else {
            self.phase = 5;
            return Ok(false);
        };
        if matches!(activation.previous, Previous::Link) {
            let source = v8::Local::new(scope, &self.source);
            let id = event_id(scope, source)?;
            let event_owner = binding_state_for_receiver(scope, source)
                .ok_or(V8BindingError::Allocation("link activation event owner"))?;
            let trusted = event_owner
                .events
                .borrow()
                .get(&id)
                .is_some_and(|event| event.is_trusted);
            let target_object = v8::Local::new(scope, &self.target);
            let actual = node_identity::argument(scope, target_object.into())?;
            let suffix = if let Some((target_owner, target)) =
                actual.filter(|(target_owner, _)| Rc::ptr_eq(&target_owner.dom, &owner.dom))
            {
                if owner
                    .dom
                    .borrow()
                    .is_element(node, "http://www.w3.org/1999/xhtml", "a")
                {
                    image_map_suffix(scope, &target_owner, target, source, trusted)?
                } else {
                    None
                }
            } else {
                None
            };
            self.link = submissions::LinkActivation::new(
                scope,
                owner,
                node,
                trusted,
                suffix.as_deref(),
                None,
            )?;
            if self.link.is_some() {
                self.phase = 15;
                return Ok(false);
            }
            self.done = true;
            return Ok(true);
        }
        if let Some(command) = command::Job::new(scope, owner, node)? {
            self.command = Some(command);
            self.phase = 12;
            return Ok(false);
        }
        if let Some((target, force)) = top_layer::invoker_target(owner, node)? {
            self.popover = Some((
                owner.clone(),
                top_layer::PendingToggle::new(owner, target, force, Some(node))?,
            ));
            self.phase = 13;
            return Ok(false);
        }
        let reset = {
            let dom = owner.dom.borrow();
            if (dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                || dom.is_element(node, "http://www.w3.org/1999/xhtml", "button"))
                && dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"))
            {
                Some(if dom.control_disabled(node)? {
                    None
                } else {
                    dom.form_owner(node)?
                })
            } else {
                None
            }
        };
        if let Some(form) = reset {
            if let Some(form) = form {
                self.reset = Some(retained_form_reset::Reset::new(scope, owner, form)?);
                self.phase = 10;
                return Ok(false);
            }
            self.done = true;
            return Ok(true);
        }
        if validation::is_submit_button(&owner.dom.borrow(), node)? {
            let form = {
                let dom = owner.dom.borrow();
                if dom.control_disabled(node)? {
                    None
                } else {
                    dom.form_owner(node)?
                }
            };
            if let Some(form) = form {
                let source = v8::Local::new(scope, &self.source);
                select_image_coordinate(scope, owner, node, source)?;
                let form_object = wrap_node(scope, owner, form)?;
                self.form = Some(v8::Global::new(scope, form_object));
                let id = event_id(scope, source)?;
                let source_owner = binding_state_for_receiver(scope, source)
                    .ok_or(V8BindingError::Allocation("activation source"))?;
                let trusted = source_owner
                    .events
                    .borrow()
                    .get(&id)
                    .is_some_and(|event| event.is_trusted);
                self.submission = Some(submissions::Request::new(
                    scope,
                    owner,
                    form,
                    Some(node),
                    submissions::SubmissionSource::Activation,
                    trusted,
                )?);
                self.phase = 11;
                return Ok(false);
            }
            self.done = true;
            return Ok(true);
        }
        if matches!(activation.previous, Previous::Label) {
            let target = v8::Local::new(scope, &self.target);
            let actual = node_identity::argument(scope, target.into())?;
            let same = actual
                .as_ref()
                .is_some_and(|(target_owner, _)| Rc::ptr_eq(&target_owner.dom, &owner.dom));
            let actual = actual.map(|(_, node)| node);
            let control = {
                let dom = owner.dom.borrow();
                let mut cursor = actual.filter(|_| same);
                let mut interactive = false;
                while let Some(current) = cursor.filter(|current| *current != node) {
                    if dom.is_interactive_content(current) {
                        interactive = true;
                        break;
                    }
                    cursor = dom.parent(current)?;
                }
                if interactive {
                    None
                } else {
                    dom.label_control(node)?
                        .filter(|control| !dom.control_disabled(*control).unwrap_or(true))
                }
            };
            if let Some(control) = control {
                let recursive = if let Some(actual) = actual.filter(|_| same) {
                    control == actual
                        || owner
                            .dom
                            .borrow()
                            .arena()
                            .ancestors(actual)?
                            .contains(&control)
                } else {
                    false
                };
                if !recursive {
                    let source = v8::Local::new(scope, &self.source);
                    self.child =
                        retained_activation::Click::from_source(scope, owner, control, source)?
                            .map(Box::new);
                    if self.child.is_some() {
                        self.phase = 14;
                        return Ok(false);
                    }
                }
            }
            self.done = true;
            return Ok(true);
        }
        let fire = {
            let dom = owner.dom.borrow();
            dom.is_connected(node)?
                && match activation.previous {
                    Previous::Checkbox { .. } => dom
                        .attribute_in_null_namespace(node, "type")?
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox")),
                    Previous::Radio { changed, .. } => {
                        changed && dom.is_radio(node) && dom.checked(node)?
                    }
                    _ => false,
                }
        };
        if fire {
            owner.dom.borrow_mut().set_user_validity(node, true)?;
            self.phase = 20;
            return Ok(false);
        }
        self.phase = 5;
        Ok(false)
    }
    pub(in super::super) fn cancel(&mut self) {
        if self.done {
            return;
        }
        if !self.accepted
            && let Some(activation) = &self.activation
        {
            activation.cancel_native();
        }
        if let Some(link) = &mut self.link {
            link.cancel();
        }
        if let Some(job) = &mut self.temporal_picker {
            job.cancel();
        }
        if let Some(job) = &mut self.select_picker {
            job.cancel();
        }
        if let Some(event) = &self.event {
            event.cancel(&event.owner());
        }
        if let Some(command) = &mut self.command {
            command.cancel();
        }
        if let Some((owner, popover)) = &mut self.popover {
            popover.release(owner);
        }
        if let Some(reset) = &mut self.reset {
            reset.cancel();
        }
        if let Some(submission) = &mut self.submission {
            submission.cancel();
        }
        if let Some(child) = &mut self.child {
            child.cancel();
        }
        self.done = true;
    }
}
impl Drop for Finish {
    fn drop(&mut self) {
        self.cancel()
    }
}
