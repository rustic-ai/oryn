//! Native keyboard callback/default custody. A receipt owns one accepted key;
//! polling resumes callback suffixes and never replays an earlier native edit.
use super::*;

use super::super::retained_dom_event::Event as EventJob;
/// Eligibility fixup is part of key admission, including its change/blur tail.
struct FocusFixup {
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    phase: u8,
    event: Option<EventJob>,
}
impl FocusFixup {
    fn prepare(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<Self>, V8BindingError> {
        temporal_picker::fixup(scope, state)?;
        let target = {
            let dom = state.dom.borrow();
            let Some(target) = dom.focused() else {
                return Ok(None);
            };
            let mut cursor = Some(target);
            let mut document = None;
            while let Some(node) = cursor {
                let record = dom.arena().get(node)?;
                if matches!(record.kind, NodeKind::Document) {
                    document = Some(node);
                    break;
                }
                cursor = record.parent;
            }
            if document != Some(state.document_root()) {
                return Ok(None);
            }
            target
        };
        if !state.dom.borrow().is_inert(target)?
            && !css_suppresses_focus_in_scope(scope, state, target)?
        {
            return Ok(None);
        }
        Ok(Some(Self {
            target: oryn_dom::TrackedNodeLease::new(&state.dom, target)?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            phase: 0,
            event: None,
        }))
    }
    fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        loop {
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            let (owner, target) = self.target.current();
            if !Rc::ptr_eq(&owner, &state.dom) || !live(state, target, self.generation, self.epoch)
            {
                self.cancel(state);
                return Ok(true);
            }
            if let Some(event) = self.event.as_mut() {
                if event.poll(scope, state)?.is_none() {
                    return Ok(false);
                }
                self.event = None;
                self.phase += 1;
                continue;
            }
            match self.phase {
                0 => {
                    state.dom.borrow_mut().focus(None)?;
                    state
                        .focus_transition_revision
                        .set(state.focus_transition_revision.get().wrapping_add(1));
                    self.phase = 1;
                }
                1 => {
                    let event = create_event_object(scope, state, "change", true, false)?;
                    let id = event_id(scope, event)?;
                    state
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("focus change event"))?
                        .is_trusted = true;
                    let event = EventJob::new(scope, state, target, "change", event)?;
                    if state.dom.borrow_mut().take_control_change(target)? {
                        self.event = Some(event);
                    } else {
                        self.phase = 2;
                    }
                }
                2 | 3 => {
                    let name = if self.phase == 2 { "blur" } else { "focusout" };
                    let event = create_focus_event_object(scope, state, name, None)?;
                    self.event = Some(EventJob::new(scope, state, target, name, event)?);
                }
                _ => return Ok(true),
            }
        }
    }
    fn cancel(&self, state: &V8BindingState) {
        if let Some(event) = &self.event {
            event.cancel(state)
        }
    }
}
use super::super::retained_focus::Transition as FocusJob;
struct SelectionGuard {
    snapshot: SelectionSnapshot,
    range: Option<u64>,
    boundaries: u64,
    generation: u64,
    epoch: u64,
}
impl SelectionGuard {
    fn new(state: &V8BindingState) -> Self {
        Self {
            snapshot: selection_snapshot(state),
            range: state.selection_range.get(),
            boundaries: state.dom.borrow().boundary_revision(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
        }
    }
    fn current(&self, state: &V8BindingState) -> Result<bool, V8BindingError> {
        synchronize_selection_range(state)?;
        Ok(!state.execution_retired()
            && state.document_generation() == self.generation
            && state.document_stream_epoch() == self.epoch
            && state.dom.borrow().boundary_revision() == self.boundaries
            && state.selection_range.get() == self.range
            && selection_snapshot(state) == self.snapshot)
    }
}
pub(super) struct SelectionJob {
    plan: Option<selection_movement::PreparedMovement>,
    guard: SelectionGuard,
    event: Option<EventJob>,
    needs_event: bool,
}
impl SelectionJob {
    pub(super) fn new(state: &V8BindingState, plan: selection_movement::PreparedMovement) -> Self {
        let needs_event = plan
            .boundaries
            .is_some_and(|(anchor, focus)| anchor != focus)
            && state.selection_anchor.get() == state.selection_focus.get();
        Self {
            plan: Some(plan),
            guard: SelectionGuard::new(state),
            event: None,
            needs_event,
        }
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<()>, V8BindingError> {
        if state.execution_suspended() {
            return Ok(None);
        }
        if self.event.is_none() && !self.guard.current(state)? {
            self.plan = None;
            return Ok(Some(()));
        }
        if self.needs_event {
            if self.event.is_none() {
                let (anchor, _) = self
                    .plan
                    .as_ref()
                    .and_then(|plan| plan.boundaries)
                    .ok_or(V8BindingError::Allocation("keyboard selection proposal"))?;
                let target = state
                    .selection_anchor
                    .get()
                    .map_or(anchor.container, |point| point.container);
                let event = create_event_object(scope, state, "selectstart", true, true)?;
                let id = event_id(scope, event)?;
                state
                    .events
                    .borrow_mut()
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("selectstart record"))?
                    .is_trusted = true;
                self.event = Some(EventJob::new(scope, state, target, "selectstart", event)?);
            }
            let Some(accepted) = self.event.as_mut().unwrap().poll(scope, state)? else {
                return Ok(None);
            };
            self.needs_event = false;
            if !accepted || !self.guard.current(state)? {
                self.plan = None;
                return Ok(Some(()));
            }
        }
        if !self.guard.current(state)? {
            self.plan = None;
            return Ok(Some(()));
        }
        if let Some(plan) = self.plan.as_ref() {
            plan.commit(state)?;
            self.plan = None;
        }
        Ok(Some(()))
    }
    pub(super) fn cancel(&self, state: &V8BindingState) {
        if let Some(event) = &self.event {
            event.cancel(state)
        }
    }
}
pub(super) struct EditJob {
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    description: EditDescription,
    phase: u8,
    event: Option<clipboard_actions::ClipboardNotification>,
    changed: bool,
    transaction: Option<retained_editing::Transaction>,
}
impl EditJob {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        target: NodeId,
        description: EditDescription,
    ) -> Result<Self, V8BindingError> {
        Ok(Self {
            target: oryn_dom::TrackedNodeLease::new(&state.dom, target)?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            description,
            phase: 0,
            event: None,
            changed: false,
            transaction: None,
        })
    }
    fn current(&self, state: &Rc<V8BindingState>) -> Option<NodeId> {
        let (owner, target) = self.target.current();
        (Rc::ptr_eq(&owner, &state.dom) && live(state, target, self.generation, self.epoch))
            .then_some(target)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<()>, V8BindingError> {
        if state.execution_suspended() {
            return Ok(None);
        }
        loop {
            let Some(target) = self.current(state) else {
                self.cancel();
                return Ok(Some(()));
            };
            let data = (!self.description.deletion && !self.description.enter)
                .then_some(self.description.replacement.as_str());
            match self.phase {
                0 | 2 => {
                    if self.phase == 2 && !self.changed {
                        return Ok(Some(()));
                    }
                    if self.event.is_none() {
                        self.event = Some(clipboard_actions::ClipboardNotification::editing(
                            scope,
                            state,
                            target,
                            if self.phase == 0 {
                                "beforeinput"
                            } else {
                                "input"
                            },
                            &self.description.input_type,
                            data,
                            None,
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, state)? else {
                        return Ok(None);
                    };
                    self.event = None;
                    if self.phase == 0 && !accepted {
                        return Ok(Some(()));
                    }
                    self.phase += 1;
                }
                1 => {
                    if self.transaction.is_none()
                        && (!clipboard_actions::editable(state, target)?
                            || state.dom.borrow().is_inert(target)?
                            || css_suppresses_focus_in_scope(scope, state, target)?
                            || state.dom.borrow().is_editable_text_control(target)
                                != self.description.is_control)
                    {
                        return Ok(Some(()));
                    }
                    if self.description.is_control {
                        self.changed = apply_edit(scope, state, target, &self.description)?;
                    } else {
                        if self.transaction.is_none() {
                            self.transaction = Some(retained_editing::Transaction::new(
                                state,
                                &self.description.input_type,
                            )?);
                        }
                        let description = &self.description;
                        let command = if description.deletion {
                            editing_commands::Command::Delete {
                                forward: !description.backward,
                                unit: deletion_granularity(description).map(str::to_owned),
                            }
                        } else if description.enter {
                            if description.line {
                                editing_commands::Command::LineBreak
                            } else {
                                editing_commands::Command::Paragraph
                            }
                        } else {
                            editing_commands::Command::Text(description.replacement.clone())
                        };
                        let Some(changed) = self
                            .transaction
                            .as_mut()
                            .unwrap()
                            .poll_owned(scope, state, command)?
                        else {
                            return Ok(None);
                        };
                        self.changed = changed;
                    }
                    self.phase = 2;
                }
                _ => return Ok(Some(())),
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(mut event) = self.event.take() {
            event.cancel();
        }
        if let Some(transaction) = &mut self.transaction {
            transaction.cancel();
        }
    }
}

enum Route {
    Selection(&'static str, &'static str, &'static str),
    SelectAll,
    Edit,
    Deferred,
    Activate,
    ArmActivation,
    Focus(bool),
    Clipboard(crate::clipboard::ClipboardAction),
    None,
    Legacy,
}
enum DefaultJob {
    Selection(SelectionJob),
    Edit(EditJob),
    Focus(FocusJob),
    Click(Box<retained_activation::Click>),
    Clipboard(clipboard_actions::ClipboardJob),
}
/// Thread-affine native operation. The Actions source cursor owns this value;
/// dropping it abandons only its unfinished callback suffix, never replays it.
pub struct Receipt {
    owner: Rc<V8BindingState>,
    input: NativeKeyboardInput,
    generation: u64,
    epoch: u64,
    phase: u8,
    prompt: Option<permission_prompt::Choice>,
    picker_checked: bool,
    picker_handled: bool,
    temporal_picker: Option<temporal_picker::Job>,
    picker: Option<select_picker::Job>,
    fixup: Option<FocusFixup>,
    fixup_prepared: bool,
    target: Option<oryn_dom::TrackedNodeLease>,
    key: String,
    code: String,
    location: u32,
    event: Option<EventJob>,
    route: Option<Route>,
    job: Option<DefaultJob>,
    result: Option<bool>,
    key_accepted: bool,
    space_target: Option<(NodeId, u64, u64)>,
}
impl Receipt {
    pub(super) fn new(
        state: Rc<V8BindingState>,
        input: &NativeKeyboardInput,
    ) -> Result<Self, V8BindingError> {
        if state.execution_retired()
            || input.location > 3
            || input.key.len() > 65536
            || input.code.len() > 256
        {
            return Err(invalid());
        }
        let (key, code, location) = normalized(input);
        Ok(Self {
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            owner: state,
            input: input.clone(),
            phase: 0,
            prompt: None,
            picker_checked: false,
            picker_handled: false,
            temporal_picker: None,
            picker: None,
            fixup: None,
            fixup_prepared: false,
            target: None,
            key,
            code,
            location,
            event: None,
            route: None,
            job: None,
            result: None,
            key_accepted: false,
            space_target: None,
        })
    }
    pub fn retry_safe(&self) -> bool {
        if let Some(DefaultJob::Click(job)) = &self.job
            && !job.retry_safe()
        {
            return false;
        }
        match self.phase {
            1 => true,
            5 => !matches!(self.route, Some(Route::Legacy)),
            _ => true,
        }
    }
    pub fn owns_state(&self, state: &V8BindingState) -> bool {
        std::ptr::eq(self.owner.as_ref(), state)
    }
    pub fn realm(&self) -> RealmId {
        self.owner.realm
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    fn target(&self) -> Option<NodeId> {
        let (owner, target) = self.target.as_ref()?.current();
        (Rc::ptr_eq(&owner, &self.owner.dom)
            && live(&self.owner, target, self.generation, self.epoch))
        .then_some(target)
    }
    fn route(&self, target: NodeId) -> Result<Route, V8BindingError> {
        let state = &self.owner;
        let mods = self.input.modifiers;
        if state.permission_elements.borrow().prompt.is_some()
            || state.dom.borrow().has_native_input_picker(target)
            || state
                .dom
                .borrow()
                .is_element(target, "http://www.w3.org/1999/xhtml", "input")
                && !state.dom.borrow().supports_selection_range(target)?
                && !activatable(state, target, false)?
        {
            return Ok(Route::Legacy);
        }
        if self.input.defer_text
            && (self.input.composing || self.key == "Unidentified" || printable(&self.key))
            && !mods.control
            && !mods.meta
            && clipboard_actions::editable(state, target)?
        {
            return Ok(Route::Deferred);
        }
        if self.input.composing {
            return Ok(Route::Legacy);
        }
        let mac = state.keyboard.platform.get() == NativeKeyboardPlatform::MacOs;
        let shortcut = (if mac { mods.meta } else { mods.control }) && !mods.alt_graph;
        if shortcut && !mods.alt {
            return Ok(match self.key.to_ascii_lowercase().as_str() {
                "a" => Route::SelectAll,
                "c" => Route::Clipboard(crate::clipboard::ClipboardAction::Copy),
                "x" => Route::Clipboard(crate::clipboard::ClipboardAction::Cut),
                "v" => Route::Clipboard(crate::clipboard::ClipboardAction::Paste),
                _ => Route::Legacy,
            });
        }
        if mac && mods.control || !mac && (mods.alt || mods.meta) {
            return Ok(Route::Legacy);
        }
        if matches!(
            self.key.as_str(),
            "Shift"
                | "Control"
                | "Alt"
                | "Meta"
                | "AltGraph"
                | "CapsLock"
                | "NumLock"
                | "ScrollLock"
        ) {
            return Ok(Route::None);
        }
        if self.key == "Enter" && activatable(state, target, false)? {
            return Ok(Route::Activate);
        }
        if self.key == " " && activatable(state, target, true)? {
            return Ok(Route::ArmActivation);
        }
        if self.key == "Tab" && !shortcut && !mods.alt {
            return Ok(Route::Focus(mods.shift));
        }
        if !clipboard_actions::editable(state, target)? {
            return Ok(Route::Legacy);
        }
        if let Some((alter, direction, unit)) = movement_parameters(state, &self.key, mods) {
            return Ok(Route::Selection(alter, direction, unit));
        }
        if printable(&self.key)
            || matches!(self.key.as_str(), "Backspace" | "Delete")
            || self.key == "Enter"
                && !state
                    .dom
                    .borrow()
                    .is_element(target, "http://www.w3.org/1999/xhtml", "input")
        {
            return Ok(Route::Edit);
        }
        Ok(Route::Legacy)
    }
    // Keep default construction temporaries off the listener-dispatch stack.
    #[inline(never)]
    fn prepare_default(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<bool>, V8BindingError> {
        let target = self.target().ok_or_else(invalid)?;
        if !self.picker_checked && !self.input.modifiers.control && !self.input.modifiers.meta {
            let (handled, job) = temporal_picker::prepare_keyboard(
                scope,
                state,
                target,
                &self.key,
                self.input.modifiers.alt,
            )?;
            self.picker_handled = handled;
            self.temporal_picker = job;
            if !handled && self.temporal_picker.is_none() {
                let (handled, job) = select_picker::prepare_keyboard(
                    state,
                    target,
                    &self.key,
                    self.input.modifiers.alt,
                )?;
                self.picker_handled = handled;
                self.picker = job;
            }
            self.picker_checked = true;
        }
        if self.temporal_picker.is_some() {
            self.phase = 31;
            return Ok(None);
        }
        if self.picker.is_some() {
            self.phase = 30;
            return Ok(None);
        }
        if self.picker_handled {
            self.result = Some(true);
            return Ok(self.result);
        }
        match self.route.as_ref().unwrap() {
            Route::Selection(alter, direction, unit) => {
                if state.dom.borrow().is_editable_text_control(target) {
                    selection_movement::modify_control(scope, state, alter, direction, unit)?;
                    self.result = Some(true);
                    return Ok(self.result);
                }
                if let Some(plan) =
                    selection_movement::prepare_movement(scope, state, alter, direction, unit)?
                {
                    self.job = Some(DefaultJob::Selection(SelectionJob::new(state, plan)));
                }
            }
            Route::SelectAll => {
                if state.dom.borrow().is_editable_text_control(target) {
                    select_all(scope, state, target)?;
                    self.result = Some(true);
                    return Ok(self.result);
                }
                synchronize_selection_range(state)?;
                let root = editing_host(state)?.unwrap_or(state.document_root());
                let length = light_children(&state.dom.borrow(), root)?.len() as u32;
                let plan = selection_movement::PreparedMovement::select(
                    BoundaryPoint {
                        container: root,
                        offset: 0,
                    },
                    BoundaryPoint {
                        container: root,
                        offset: length,
                    },
                );
                self.job = Some(DefaultJob::Selection(SelectionJob::new(state, plan)));
            }
            Route::Edit => {
                if let Some(description) =
                    prepare_edit(scope, state, target, &self.key, self.input.modifiers, None)?
                {
                    self.job = Some(DefaultJob::Edit(EditJob::new(state, target, description)?));
                }
            }
            Route::Clipboard(action) => {
                let grant = grant_native_clipboard_action(state, target, *action)?;
                self.job = Some(DefaultJob::Clipboard(clipboard_actions::ClipboardJob::new(
                    state, grant,
                )?));
            }
            Route::Activate => {
                if let Some(job) = retained_activation::Click::new(scope, state, target)? {
                    self.job = Some(DefaultJob::Click(Box::new(job)));
                }
            }
            Route::ArmActivation => {
                if !self.input.repeat {
                    state.keyboard.space_activation.set(Some((
                        target,
                        self.generation,
                        self.epoch,
                    )));
                }
            }
            Route::Focus(reverse) => {
                let target = focus_navigation::next_sequential(scope, state, *reverse)?;
                self.job = Some(DefaultJob::Focus(FocusJob::new(state, target)?));
            }
            Route::Deferred => {
                // OS interpretation is a separate native grant. Publish
                // it only after the original keypress suffix completed.
                if clipboard_actions::editable(state, target)? {
                    let dom = state.dom.borrow();
                    let target = if dom.is_editable_text_control(target) {
                        target
                    } else {
                        editing_host_for_node(
                            &dom,
                            Some(target),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )?
                        .unwrap_or(target)
                    };
                    state.keyboard.deferred_target.set(Some(target));
                    state.keyboard.deferred.set(true);
                }
            }
            Route::None => {}
            // Focus, picker and activation defaults still require
            // separate retained jobs. This compatibility route keeps
            // their already delivered key event out of redispatch.
            Route::Legacy => {
                let result = dispatch_after_key(
                    scope,
                    state,
                    AfterKeyInputs {
                        input: &self.input,
                        target,
                        key: self.key.clone(),
                        code: self.code.clone(),
                        location: self.location,
                        generation: self.generation,
                        epoch: self.epoch,
                        accepted: true,
                    },
                )?;
                self.result = Some(result);
                return Ok(self.result);
            }
        }
        self.phase = 6;
        Ok(None)
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        let state = self.owner.clone();
        if state.execution_retired()
            || state.document_generation() != self.generation
            || state.document_stream_epoch() != self.epoch
        {
            return Err(invalid());
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        loop {
            match self.phase {
                0 => {
                    state.keyboard.deferred.set(false);
                    state.keyboard.deferred_target.set(None);
                    update_native_pointer_modifiers(&state, &self.input.modifiers);
                    if !self.fixup_prepared {
                        self.fixup = FocusFixup::prepare(scope, &state)?;
                        self.fixup_prepared = true;
                    }
                    if let Some(fixup) = self.fixup.as_mut()
                        && !fixup.poll(scope, &state)?
                    {
                        return Ok(None);
                    }
                    self.fixup = None;
                    if state.execution_retired()
                        || state.document_generation() != self.generation
                        || state.document_stream_epoch() != self.epoch
                    {
                        return Err(invalid());
                    }
                    let target = focus_target(&state)?;
                    self.target = Some(oryn_dom::TrackedNodeLease::new(&state.dom, target)?);
                    let mods = self.input.modifiers;
                    if self.input.down && !mods.control && !mods.alt && !mods.meta {
                        state.dom.borrow_mut().set_focus_visible(true);
                    }
                    if self.input.down && self.key != "Escape" {
                        state.notify_user_activation();
                    }
                    self.phase = 1;
                }
                1 => {
                    // A popup may close on keydown. Its matching release still
                    // belongs to native UI and must not reach page listeners.
                    if !self.input.down
                        && state
                            .permission_elements
                            .borrow_mut()
                            .prompt_keys
                            .remove(&self.key)
                    {
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    if state.permission_elements.borrow().prompt.is_some() {
                        if self.input.down {
                            state
                                .permission_elements
                                .borrow_mut()
                                .prompt_keys
                                .insert(self.key.clone());
                            let (_, job) =
                                permission_prompt::prepare_keyboard(scope, &state, &self.key)?;
                            self.prompt = job;
                            if self.prompt.is_some() {
                                self.phase = 32;
                                continue;
                            }
                        }
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    let target = self.target().ok_or_else(invalid)?;
                    let name = if self.input.down { "keydown" } else { "keyup" };
                    let event = event(
                        scope,
                        &state,
                        &self.input,
                        &self.key,
                        &self.code,
                        self.location,
                        name,
                    )?;
                    self.event = Some(EventJob::new(scope, &state, target, name, event)?);
                    self.phase = 2;
                }
                2 => {
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.event = None;
                    self.key_accepted = accepted;
                    self.phase = 8;
                }
                8 => {
                    let accepted = self.key_accepted;
                    if !self.input.down {
                        if self.key == " " {
                            self.space_target = state.keyboard.space_activation.take();
                        }
                        self.phase = 7;
                        continue;
                    }
                    if !accepted || self.target().is_none() {
                        self.result = Some(accepted);
                        return Ok(self.result);
                    }
                    let target = focus_target(&state)?;
                    self.target = Some(oryn_dom::TrackedNodeLease::new(&state.dom, target)?);
                    self.route = Some(self.route(target)?);
                    self.phase = 3;
                }
                7 => {
                    if self.key_accepted
                        && let Some((armed, generation, epoch)) = self.space_target
                        && self.target() == Some(armed)
                        && live(&state, armed, generation, epoch)
                        && activatable(&state, armed, true)?
                        && let Some(job) = retained_activation::Click::new(scope, &state, armed)?
                    {
                        self.job = Some(DefaultJob::Click(Box::new(job)));
                        self.phase = 6;
                        continue;
                    }
                    self.result = Some(self.key_accepted);
                    return Ok(self.result);
                }
                3 => {
                    let target = self.target().ok_or_else(invalid)?;
                    if matches!(self.route, Some(Route::Activate | Route::ArmActivation))
                        || matches!(self.route, Some(Route::Edit))
                            && (printable(&self.key) || self.key == "Enter")
                        || matches!(self.route, Some(Route::Deferred))
                            && !self.input.composing
                            && printable(&self.key)
                    {
                        let event = event(
                            scope,
                            &state,
                            &self.input,
                            &self.key,
                            &self.code,
                            self.location,
                            "keypress",
                        )?;
                        self.event = Some(EventJob::new(scope, &state, target, "keypress", event)?);
                        self.phase = 4;
                    } else {
                        self.phase = 5;
                    }
                }
                4 => {
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &state)? else {
                        return Ok(None);
                    };
                    self.event = None;
                    if !accepted || self.target().is_none() {
                        self.result = Some(accepted);
                        return Ok(self.result);
                    }
                    self.phase = 5;
                }
                5 => {
                    if let Some(result) = self.prepare_default(scope, &state)? {
                        return Ok(Some(result));
                    }
                }
                32 => {
                    if !self.prompt.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.prompt = None;
                    self.result = Some(true);
                    return Ok(self.result);
                }
                31 => {
                    if !self.temporal_picker.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.temporal_picker = None;
                    if self.picker_handled {
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    self.phase = 5;
                }
                30 => {
                    if !self.picker.as_mut().unwrap().poll(scope)? {
                        return Ok(None);
                    }
                    self.picker = None;
                    if self.picker_handled {
                        self.result = Some(true);
                        return Ok(self.result);
                    }
                    self.phase = 5;
                }
                6 => {
                    let ready = match self.job.as_mut() {
                        Some(DefaultJob::Selection(job)) => job.poll(scope, &state)?.is_some(),
                        Some(DefaultJob::Edit(job)) => job.poll(scope, &state)?.is_some(),
                        Some(DefaultJob::Focus(job)) => job.poll(scope, &state)?,
                        Some(DefaultJob::Click(job)) => job.poll(scope)?.is_some(),
                        Some(DefaultJob::Clipboard(job)) => job.poll(scope, &state)?.is_some(),
                        None => true,
                    };
                    if !ready {
                        return Ok(None);
                    }
                    self.result = Some(true);
                    return Ok(self.result);
                }
                _ => return Err(V8BindingError::Allocation("keyboard receipt phase")),
            }
        }
    }
}
impl Drop for Receipt {
    fn drop(&mut self) {
        if let Some(picker) = &mut self.temporal_picker {
            picker.cancel();
        }
        if let Some(picker) = &mut self.picker {
            picker.cancel();
        }
        if let Some(fixup) = &self.fixup {
            fixup.cancel(&self.owner)
        }
        if let Some(event) = &self.event {
            event.cancel(&self.owner)
        }
        if let Some(job) = &mut self.job {
            match job {
                DefaultJob::Selection(job) => job.cancel(&self.owner),
                DefaultJob::Edit(job) => job.cancel(),
                DefaultJob::Focus(job) => job.cancel(&self.owner),
                DefaultJob::Click(job) => job.cancel(),
                DefaultJob::Clipboard(job) => job.cancel(&self.owner),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8_bindings::native_platform::publication;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn key(value: &str) -> NativeKeyboardInput {
        NativeKeyboardInput {
            key: value.encode_utf16().collect(),
            down: true,
            ..Default::default()
        }
    }
    #[test]
    fn space_activation_keeps_checkedness_and_click_suffix_without_replay() {
        for cancel in [false, true] {
            super::super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<input id=t type=checkbox>")
                    .into_runtime()
                    .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(229), 1).unwrap();
            eval(
                scope,
                "t.focus();globalThis.log=[];t.addEventListener('click',e=>{globalThis.original=e;log.push('first');t.click()},{once:true});t.addEventListener('click',e=>{log.push(e===original?'second':'wrong');if(globalThis.cancel)e.preventDefault()});t.addEventListener('input',()=>log.push('input'));t.addEventListener('change',()=>log.push('change'))",
            );
            eval(
                scope,
                if cancel {
                    "globalThis.cancel=true"
                } else {
                    "globalThis.cancel=false"
                },
            );
            let input = key(" ");
            let mut down = Receipt::new(state.clone(), &input).unwrap();
            assert_eq!(down.poll(scope).unwrap(), Some(true));
            let mut input = input;
            input.down = false;
            let mut up = Receipt::new(state.clone(), &input).unwrap();
            publication::fail_after_next_callback();
            assert!(up.poll(scope).is_err());
            assert!(eval(scope, "log.join()==='first'&&t.checked").is_true());
            assert_eq!(up.poll(scope).unwrap(), Some(true));
            assert!(
                eval(
                    scope,
                    if cancel {
                        "log.join()==='first,second'&&!t.checked"
                    } else {
                        "log.join()==='first,second,input,change'&&t.checked"
                    }
                )
                .is_true()
            );
        }
    }
    #[test]
    fn canceled_keyup_disarms_pending_space_activation() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t type=checkbox>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(230), 1).unwrap();
        eval(
            scope,
            "t.focus();t.addEventListener('keyup',e=>e.preventDefault(),{once:true})",
        );
        let mut input = key(" ");
        let mut down = Receipt::new(state.clone(), &input).unwrap();
        assert_eq!(down.poll(scope).unwrap(), Some(true));
        input.down = false;
        let mut up = Receipt::new(state.clone(), &input).unwrap();
        assert_eq!(up.poll(scope).unwrap(), Some(false));
        assert!(state.keyboard.space_activation.get().is_none());
        assert!(eval(scope, "!t.checked").is_true());
    }
    #[test]
    fn tab_focus_resumes_blur_suffix_and_author_redirect_supersedes_target() {
        for redirect in [false, true] {
            super::super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<input id=a><input id=b><input id=c>")
                    .into_runtime()
                    .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(221), 1).unwrap();
            eval(
                scope,
                "a.focus();globalThis.log=[];a.addEventListener('blur',e=>{globalThis.original=e;log.push('first');if(globalThis.redirect)c.focus()},{once:true});a.addEventListener('blur',e=>log.push(e===original?'second':'wrong'));b.addEventListener('focus',()=>log.push('focus-b'))",
            );
            eval(
                scope,
                if redirect {
                    "globalThis.redirect=true"
                } else {
                    "globalThis.redirect=false"
                },
            );
            let mut receipt = Receipt::new(state.clone(), &key("Tab")).unwrap();
            publication::fail_after_next_callback();
            assert!(receipt.poll(scope).is_err());
            assert_eq!(receipt.poll(scope).unwrap(), Some(true));
            assert!(
                eval(
                    scope,
                    "log.filter(x=>x==='first').length===1&&log.filter(x=>x==='second').length===1"
                )
                .is_true()
            );
            assert!(
                eval(
                    scope,
                    if redirect {
                        "document.activeElement===c&&!log.includes('focus-b')"
                    } else {
                        "document.activeElement===b&&log.filter(x=>x==='focus-b').length===1"
                    }
                )
                .is_true()
            );
        }
    }
    #[test]
    fn retained_focus_fixup_finishes_blur_before_key_delivery() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(219), 1).unwrap();
        eval(
            scope,
            "t.focus();globalThis.log=[];t.addEventListener('blur',e=>{globalThis.original=e;log.push('first')},{once:true});t.addEventListener('blur',e=>log.push(e===original?'second':'wrong'));t.addEventListener('focusout',()=>log.push('out'));document.addEventListener('keydown',()=>log.push('key'))",
        );
        let target = focus_target(&state).unwrap();
        state
            .dom
            .borrow_mut()
            .set_attribute(target, "inert", "")
            .unwrap();
        let mut receipt = Receipt::new(state.clone(), &key("Shift")).unwrap();
        publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(receipt.retry_safe());
        assert!(eval(scope, "log.join()==='first'").is_true());
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        assert!(eval(scope, "log.join()==='first,second,out,key'").is_true());
    }
    #[test]
    fn deferred_os_keypress_is_retained_before_interpretation_grant() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(220), 1).unwrap();
        eval(
            scope,
            "t.focus();globalThis.log=[];t.addEventListener('keypress',e=>{globalThis.original=e;log.push('first')},{once:true});t.addEventListener('keypress',e=>log.push(e===original?'second':'wrong'))",
        );
        let mut input = key("x");
        input.defer_text = true;
        let mut receipt = Receipt::new(state.clone(), &input).unwrap();
        publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(!state.keyboard.deferred.get());
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        assert!(state.keyboard.deferred.get());
        assert_eq!(
            state.keyboard.deferred_target.get(),
            Some(focus_target(&state).unwrap())
        );
        assert!(eval(scope, "log.join()==='first,second'&&t.value===''").is_true());
    }
    #[test]
    fn retained_keydown_resumes_listener_suffix_and_edits_once() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(216), 1).unwrap();
        eval(
            scope,
            "t.focus();globalThis.log=[];t.addEventListener('keydown',e=>{globalThis.original=e;log.push('first')},{once:true});t.addEventListener('keydown',e=>log.push(e===original?'second':'wrong'));for(const type of ['keypress','beforeinput','input'])t.addEventListener(type,()=>log.push(type))",
        );
        let mut receipt = Receipt::new(state.clone(), &key("x")).unwrap();
        publication::fail_after_next_callback();
        assert!(receipt.poll(scope).is_err());
        assert!(receipt.retry_safe());
        assert!(eval(scope, "log.join()==='first'&&t.value===''").is_true());
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        assert_eq!(receipt.poll(scope).unwrap(), Some(true));
        assert!(
            eval(
                scope,
                "log.join()==='first,second,keypress,beforeinput,input'&&t.value==='x'"
            )
            .is_true()
        );
    }
    #[test]
    fn retained_selection_admission_preserves_range_until_complete_and_honors_cancel() {
        for cancel in [false, true] {
            super::super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<div id=t contenteditable>abc</div>")
                    .into_runtime()
                    .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(217), 1).unwrap();
            eval(
                scope,
                "t.focus();globalThis.s=getSelection();s.collapse(t.firstChild,0);globalThis.original=s.getRangeAt(0);globalThis.log=[];document.addEventListener('selectstart',e=>{globalThis.event=e;log.push('first')},{once:true});document.addEventListener('selectstart',e=>{log.push(e===event?'second':'wrong');if(globalThis.cancel)e.preventDefault()})",
            );
            eval(
                scope,
                if cancel {
                    "globalThis.cancel=true"
                } else {
                    "globalThis.cancel=false"
                },
            );
            let mut input = key("ArrowRight");
            input.modifiers.shift = true;
            let mut receipt = Receipt::new(state.clone(), &input).unwrap();
            publication::fail_after_next_callback();
            assert!(receipt.poll(scope).is_err());
            assert!(
                eval(
                    scope,
                    "log.join()==='first'&&s.isCollapsed&&s.getRangeAt(0)===original"
                )
                .is_true()
            );
            assert_eq!(receipt.poll(scope).unwrap(), Some(true));
            assert!(eval(scope, "log.join()==='first,second'").is_true());
            assert_eq!(eval(scope, "s.isCollapsed").is_true(), cancel);
            assert_eq!(
                eval(scope, "s.focusOffset").integer_value(scope).unwrap(),
                if cancel { 0 } else { 1 }
            );
        }
    }
    #[test]
    fn edit_beforeinput_and_input_failures_keep_single_native_mutation() {
        for rich in [false, true] {
            for failing_event in ["beforeinput", "input"] {
                super::super::super::tests::initialize_v8();
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse(if rich {
                        "<div id=t contenteditable></div>"
                    } else {
                        "<input id=t>"
                    })
                    .into_runtime()
                    .unwrap(),
                ));
                let mut isolate = v8::Isolate::new(Default::default());
                v8::scope!(let scope,&mut isolate);
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let state =
                    install_generated_bindings(scope, context, dom, RealmId(218), 1).unwrap();
                eval(
                    scope,
                    "t.focus();if(t.isContentEditable)getSelection().collapse(t,0);globalThis.log=[]",
                );
                eval(
                    scope,
                    &format!(
                        "t.addEventListener('{failing_event}',e=>{{globalThis.original=e;log.push('first')}},{{once:true}});t.addEventListener('{failing_event}',e=>log.push(e===original?'second':'wrong'))"
                    ),
                );
                let target = focus_target(&state).unwrap();
                let description =
                    prepare_edit(scope, &state, target, "x", Default::default(), None)
                        .unwrap()
                        .unwrap();
                let mut job = EditJob::new(&state, target, description).unwrap();
                publication::fail_after_next_callback();
                assert!(job.poll(scope, &state).is_err());
                assert!(eval(scope, "log.join()==='first'").is_true());
                assert!(
                    eval(
                        scope,
                        if failing_event == "input" {
                            "(t.isContentEditable?t.textContent:t.value)==='x'"
                        } else {
                            "(t.isContentEditable?t.textContent:t.value)===''"
                        }
                    )
                    .is_true()
                );
                assert_eq!(job.poll(scope, &state).unwrap(), Some(()));
                assert!(eval(scope,"log.join()==='first,second'&&(t.isContentEditable?t.textContent:t.value)==='x'").is_true());
            }
        }
    }
}
