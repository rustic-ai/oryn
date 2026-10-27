//! Thread-affine action ticks. The page event loop polls this operation between
//! native task checkpoints; no sleeps or nested script evaluation occur here.
use crate::executable::RustExecutableDocument;
use crate::host::webdriver_script::{AutomationRealmSnapshot, ScriptError};
use crate::webdriver_actions::*;
use oryn_js::v8_bindings::{
    NativeKeyboardInput, NativeKeyboardModifiers, NativePointerAction, NativePointerDevice,
    NativePointerInput, NativeWheelInput,
};
use std::time::{Duration, Instant};

type WheelTransaction = std::rc::Rc<
    std::cell::RefCell<(
        std::sync::Arc<oryn_js::v8_bindings::NativeWheelTransaction>,
        Option<AutomationRealmSnapshot>,
    )>,
>;
enum Motion {
    Pointer {
        source: String,
        start: (f64, f64),
        end: (f64, f64),
        duration: u64,
        properties: PointerProperties,
        progress: f64,
        pending: Option<(Box<PointerProgress>, f64)>,
    },
    Wheel {
        transaction: WheelTransaction,
        position: (f64, f64),
        delta: (f64, f64),
        duration: u64,
        progress: f64,
    },
}
pub(crate) struct ActionExecution {
    request: ActionsRequest,
    selected: AutomationRealmSnapshot,
    tick: usize,
    started: Option<Instant>,
    next_source: usize,
    keyboard: Option<KeyboardProgress>,
    pointer: Option<PointerProgress>,
    text: Option<OsTextProgress>,
    motions: Vec<Motion>,
    wheels: std::collections::BTreeMap<String, WheelTransaction>,
}
impl ActionExecution {
    pub fn new(
        request: ActionsRequest,
        selected: AutomationRealmSnapshot,
        input: &mut InputState,
    ) -> Result<Self, ScriptError> {
        input.admit(&request)?;
        Ok(Self {
            request,
            selected,
            tick: 0,
            started: None,
            next_source: 0,
            keyboard: None,
            pointer: None,
            text: None,
            motions: Vec::new(),
            wheels: Default::default(),
        })
    }
    /// True means all ticks finished. Errors preserve successfully dispatched
    /// held inputs so Release Actions can still release them afterward.
    pub fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        page.validate_action_context(&self.selected)?;
        let ticks = self
            .request
            .sources
            .iter()
            .map(|source| source.actions.len())
            .max()
            .unwrap_or(0);
        if self.tick >= ticks {
            return Ok(true);
        }
        let tick_duration = self.request.tick_duration(self.tick);
        if self.started.is_none() {
            self.started = Some(now);
        }
        // A zero-duration motion can suspend while admitting its drag. Finish
        // that exact event before admitting any later source in the tick.
        for motion in &mut self.motions {
            if let Motion::Pointer {
                pending, progress, ..
            } = motion
                && let Some((receipt, destination)) = pending.as_mut()
            {
                if !receipt.poll(page, now)? {
                    return Ok(false);
                }
                *progress = *destination;
                *pending = None;
            }
        }
        {
            while self.next_source < self.request.sources.len() {
                let source = &self.request.sources[self.next_source];
                let Some(action) = source.actions.get(self.tick) else {
                    self.next_source += 1;
                    continue;
                };
                match action {
                    Action::OsText {
                        token,
                        sequence,
                        operation,
                    } => {
                        if self.text.is_none() {
                            self.text = Some(OsTextProgress::new(
                                page, input, &source.id, *token, *sequence, operation,
                            )?);
                        }
                        if !self.text.as_mut().unwrap().poll(page, input, now)? {
                            return Ok(false);
                        }
                        self.text = None;
                    }
                    Action::OsModifiers(modifiers) => {
                        input.set_os_modifiers(&source.id, *modifiers)
                    }
                    Action::OsKey(key) => {
                        if self.keyboard.is_none() {
                            self.keyboard = prepare_os_key(
                                page,
                                &self.selected,
                                input,
                                &source.id,
                                key,
                                false,
                            )?;
                        }
                        if let Some(keyboard) = self.keyboard.as_mut()
                            && !keyboard.poll(page, input, now)?
                        {
                            return Ok(false);
                        }
                        self.keyboard = None;
                    }
                    Action::OsPointerMove {
                        x,
                        y,
                        screen_x,
                        screen_y,
                    } => {
                        if self.pointer.is_none() {
                            let previous = input.position(&source.id)?;
                            input.move_pointer(&source.id, *x, *y)?;
                            input.set_os_screen_position(&source.id, *screen_x, *screen_y);
                            let properties = input.last_pointer_properties(&source.id)?;
                            let event = pointer_input_snapshot(
                                input,
                                &source.id,
                                (*x, *y),
                                (*x - previous.0, *y - previous.1),
                                -1,
                                properties,
                            )?;
                            self.pointer = Some(PointerProgress::new(
                                page,
                                &self.selected,
                                event,
                                NativePointerAction::Move,
                            )?);
                        }
                        if !self.pointer.as_mut().unwrap().poll(page, now)? {
                            return Ok(false);
                        }
                        self.pointer = None;
                    }
                    Action::Pause { .. } => {}
                    Action::Key { down, value } => {
                        if self.keyboard.is_none() {
                            self.keyboard =
                                prepare_key(page, &self.selected, input, &source.id, value, *down)?;
                        }
                        if let Some(keyboard) = self.keyboard.as_mut()
                            && !keyboard.poll(page, input, now)?
                        {
                            return Ok(false);
                        }
                        self.keyboard = None;
                    }
                    Action::PointerButton {
                        down,
                        button,
                        properties,
                    } => {
                        if self.pointer.is_none() {
                            let held = HeldInput::Button(*button);
                            if input.is_pressed(&source.id, &held) == *down {
                                self.next_source += 1;
                                continue;
                            }
                            page.validate_action_context(&self.selected)?;
                            let button = i32::try_from(*button).map_err(|_| {
                                ScriptError::invalid(
                                    "native pointer button exceeds supported integer range",
                                )
                            })?;
                            if *down {
                                input.press(&source.id, held)?;
                            } else {
                                input.release(&source.id, &held)?;
                            }
                            let position = input.position(&source.id)?;
                            let event = pointer_input(
                                input,
                                &source.id,
                                position,
                                (0., 0.),
                                button,
                                properties,
                            )?;
                            self.pointer = Some(PointerProgress::new(
                                page,
                                &self.selected,
                                event,
                                if *down {
                                    NativePointerAction::Down
                                } else {
                                    NativePointerAction::Up
                                },
                            )?);
                        }
                        if !self.pointer.as_mut().unwrap().poll(page, now)? {
                            return Ok(false);
                        }
                        self.pointer = None;
                    }
                    Action::PointerCancel => {
                        if self.pointer.is_none() {
                            let position = input.position(&source.id)?;
                            let mut properties = input.last_pointer_properties(&source.id)?;
                            input.cancel_pointer(&source.id)?;
                            properties.pressure = 0.;
                            let event = pointer_input_snapshot(
                                input,
                                &source.id,
                                position,
                                (0., 0.),
                                -1,
                                properties,
                            )?;
                            self.pointer = Some(PointerProgress::new(
                                page,
                                &self.selected,
                                event,
                                NativePointerAction::Cancel,
                            )?);
                        }
                        if !self.pointer.as_mut().unwrap().poll(page, now)? {
                            return Ok(false);
                        }
                        self.pointer = None;
                    }
                    Action::PointerMove {
                        x,
                        y,
                        origin,
                        duration_ms,
                        properties,
                    } => {
                        let start = input.position(&source.id)?;
                        let end = page.webdriver_action_coordinates(
                            &self.selected,
                            origin,
                            *x,
                            *y,
                            start,
                        )?;
                        self.motions.push(Motion::Pointer {
                            source: source.id.clone(),
                            start,
                            end,
                            duration: duration_ms.unwrap_or(tick_duration),
                            properties: properties.clone(),
                            progress: 0.,
                            pending: None,
                        });
                        if !advance(
                            self.motions.last_mut().unwrap(),
                            Duration::ZERO,
                            page,
                            &self.selected,
                            input,
                            now,
                        )? {
                            self.next_source += 1;
                            return Ok(false);
                        }
                    }
                    Action::Scroll {
                        x,
                        y,
                        delta_x,
                        delta_y,
                        origin,
                        duration_ms,
                    } => {
                        let position = page.webdriver_action_coordinates(
                            &self.selected,
                            origin,
                            *x,
                            *y,
                            (0., 0.),
                        )?;
                        let transaction = self
                            .wheels
                            .entry(source.id.clone())
                            .or_insert_with(|| {
                                std::rc::Rc::new(std::cell::RefCell::new((
                                    Default::default(),
                                    None,
                                )))
                            })
                            .clone();
                        let mut motion = Motion::Wheel {
                            transaction,
                            position,
                            delta: (*delta_x as f64, *delta_y as f64),
                            duration: duration_ms.unwrap_or(tick_duration),
                            progress: 0.,
                        };
                        advance(
                            &mut motion,
                            Duration::ZERO,
                            page,
                            &self.selected,
                            input,
                            now,
                        )?;
                        self.motions.push(motion);
                    }
                }
                self.next_source += 1;
            }
        }
        let elapsed = now.saturating_duration_since(self.started.unwrap());
        for motion in &mut self.motions {
            if !advance(motion, elapsed, page, &self.selected, input, now)? {
                return Ok(false);
            }
        }
        if elapsed >= Duration::from_millis(tick_duration) {
            self.tick += 1;
            self.started = None;
            self.next_source = 0;
            self.motions.clear();
        }
        Ok(self.tick >= ticks)
    }
}

/// One admitted OS continuation sequence; retries keep its grant and payload.
struct OsTextProgress {
    grant: OwnedOsKeyReceipt,
    completed_receipt: Option<OwnedOsKeyReceipt>,
    state_lease: Option<std::sync::Arc<dyn Send+Sync>>,
    operation: oryn_js::composition::Operation,
    receipt: Option<oryn_js::v8_bindings::NativeComposition>,
    state: Option<oryn_js::composition::StateSnapshot>,
    retry: KeyboardRetry,
    done: bool,
    failure: Option<ScriptError>,
}
impl OsTextProgress {
    fn new(
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        source: &str,
        token: u64,
        sequence: u64,
        operation: &oryn_js::composition::Operation,
    ) -> Result<Self, ScriptError> {
        let grant = input.admit_owned_os_text(source, token, sequence)?;
        let admitted = (|| {
            let target = grant.target.ok_or_else(ScriptError::retired)?;
            page.validate_action_context(&grant.context)?;
            let aborted = grant.state.as_ref().is_some_and(|state| state.aborted);
            let receipt = if aborted {
                None
            } else {
                Some(page.begin_composition_in_context(&grant.context, target, operation)?)
            };
            Ok::<_, ScriptError>((aborted, receipt))
        })();
        let (aborted, receipt) = match admitted {
            Ok(value) => value,
            Err(error) => {
                input.finish_os_text(token, false);
                return Err(error);
            }
        };
        Ok(Self {
            completed_receipt: aborted.then(||grant.clone().completed(matches!(operation,oryn_js::composition::Operation::Cancel|oryn_js::composition::Operation::Finish{..}))),
            state_lease: None,
            grant,
            operation: operation.clone(),
            receipt,
            state: None,
            retry: Default::default(),
            done: false,
            failure: None,
        })
    }
    fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.done {
            return Ok(true);
        }
        let result = self.poll_inner(page, input, now);
        if let Err(error) = &result {
            self.failure = Some(error.clone());
            self.receipt = None;
            input.finish_os_text(self.grant.token, false);
        }
        result
    }
    fn poll_inner(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        page.validate_action_context(&self.grant.context)?;
        if !self.retry.ready(now) {
            return Ok(false);
        }
        if self.state.is_none() && self.completed_receipt.is_none() {
            let receipt = self.receipt.as_mut().unwrap();
            receipt.set_parent_metadata_bytes(OwnedOsKeyReceipt::metadata_bytes(&self.grant.source,&self.grant.context)?)
                .map_err(|error|ScriptError{code:"javascript error".into(),message:error.to_string()})?;
            match page.poll_composition_in_context(&self.grant.context, receipt) {
                Ok(Some(state)) => {
                    self.state_lease=Some(std::sync::Arc::new((receipt.snapshot_lease(),receipt.parent_metadata_lease())));
                    self.state = Some(state);
                    self.receipt = None;
                }
                Ok(None) => return Ok(false),
                Err(error) => {
                    self.retry.failed(error, receipt.retry_safe(), now)?;
                    return Ok(false);
                }
            }
        }
        if self.completed_receipt.is_none(){
            let state=self.state.take().unwrap();
            self.completed_receipt=Some(OwnedOsKeyReceipt::new(OsKeyReceipt{
                continuation_done:matches!(self.operation,oryn_js::composition::Operation::Cancel|oryn_js::composition::Operation::Finish{..}),
                source:self.grant.source.clone(),state:Some(state),target:self.grant.target,
                token:self.grant.token,accepted:self.grant.accepted,interpret:false,
                context:self.grant.context.clone(),
            },self.state_lease.take()));
        }
        let completed=self.completed_receipt.as_ref().unwrap();
        let state=completed.state.as_ref().unwrap();
        input.record_os_text_state(self.grant.token, completed.clone());
        let keep = !matches!(
            self.operation,
            oryn_js::composition::Operation::Cancel
                | oryn_js::composition::Operation::Finish { keep_marked: false }
        ) && (!matches!(
            self.operation,
            oryn_js::composition::Operation::Finish { keep_marked: true }
        ) || state.marked.is_some());
        input.finish_os_text(self.grant.token, keep);
        input.push_owned_os_receipt(completed.clone())?;
        self.done = true;
        Ok(true)
    }
}

/// The Actions source owns drag admission while the page task loop runs.
/// Position/button state was frozen at construction and is not changed again
/// when a dragstart or capture notification resumes.
struct PointerProgress {
    selected: AutomationRealmSnapshot,
    input: NativePointerInput,
    action: NativePointerAction,
    admission: Option<(
        AutomationRealmSnapshot,
        oryn_js::v8_bindings::NativePointerDragAdmission,
    )>,
    transport: Option<(
        AutomationRealmSnapshot,
        NativePointerInput,
        bool,
        oryn_js::v8_bindings::NativeCrossFrameDrag,
    )>,
    transport_prepared: bool,
    dispatch: Option<crate::executable::PointerDispatchRoute>,
    retry: KeyboardRetry,
    done: bool,
    failure: Option<ScriptError>,
}
impl PointerProgress {
    fn new(
        page: &mut RustExecutableDocument,
        selected: &AutomationRealmSnapshot,
        input: NativePointerInput,
        action: NativePointerAction,
    ) -> Result<Self, ScriptError> {
        let admission = page.begin_pointer_drag_admission_in_context(selected, &input, action)?;
        Ok(Self {
            selected: selected.clone(),
            input,
            action,
            admission,
            transport: None,
            transport_prepared: false,
            dispatch: None,
            retry: Default::default(),
            done: false,
            failure: None,
        })
    }
    fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.poll_inner(page, now);
        if let Err(error) = &result {
            self.failure = Some(error.clone());
            self.admission = None;
            self.transport = None;
            self.dispatch = None;
        }
        result
    }
    fn poll_inner(
        &mut self,
        page: &mut RustExecutableDocument,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        if self.done {
            return Ok(true);
        }
        page.validate_action_context(&self.selected)?;
        if !self.retry.ready(now) {
            return Ok(false);
        }
        if let Some((source, receipt)) = &mut self.admission {
            match page.poll_pointer_drag_admission_in_context(source, receipt) {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(false),
                Err(error) => {
                    self.retry.failed(error, receipt.retry_safe(), now)?;
                    return Ok(false);
                }
            }
            self.admission = None;
        }
        if !self.transport_prepared {
            self.transport =
                page.begin_pointer_drag_transport(&self.selected, &self.input, self.action)?;
            self.transport_prepared = true;
        }
        if let Some((source, input, terminal, receipt)) = &mut self.transport {
            match page.poll_pointer_drag_transport(source, input, *terminal, receipt) {
                Ok(true) => {
                    self.done = true;
                    return Ok(true);
                }
                Ok(false) => return Ok(false),
                Err(error) => {
                    self.retry.failed(error, receipt.retry_safe(), now)?;
                    return Ok(false);
                }
            }
        }
        if self.dispatch.is_none() {
            self.dispatch = Some(page.begin_pointer_dispatch_in_context(
                &self.selected,
                &self.input,
                self.action,
            )?);
        }
        let receipt = self.dispatch.as_mut().unwrap();
        match page.poll_pointer_dispatch_in_context(receipt) {
            Ok(Some(_)) => {
                self.dispatch = None;
                self.done = true;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(error) => {
                self.retry.failed(error, receipt.retry_safe(), now)?;
                Ok(false)
            }
        }
    }
}

/// Owns one accepted key and its OS interpretation suffix across actor polls.
/// Held-input mutation and native key admission occur only in the constructors.
struct KeyboardProgress {
    interpretation_lease: Option<std::sync::Arc<dyn Send+Sync>>,
    interpretation_allowance: Option<std::sync::Arc<crate::native_input_allowance::InputAllowance>>,
    interpretation_completed: Option<OwnedOsKeyReceipt>,
    context: AutomationRealmSnapshot,
    receipt: oryn_js::v8_bindings::NativeKeyboardReceipt,
    interpretation: Option<(String, u64)>,
    accepted: Option<bool>,
    interpretation_read: Option<(oryn_dom::NodeId, oryn_js::v8_bindings::NativeComposition)>,
    interpretation_result: Option<(
        Option<oryn_dom::NodeId>,
        bool,
        Option<oryn_js::composition::StateSnapshot>,
    )>,
    retry: KeyboardRetry,
}
#[derive(Default)]
struct KeyboardRetry {
    deadline: Option<Instant>,
    first: Option<ScriptError>,
    attempts: u8,
}
impl KeyboardRetry {
    fn ready(&self, now: Instant) -> bool {
        self.deadline.is_none_or(|deadline| now >= deadline)
    }
    fn failed(&mut self, error: ScriptError, safe: bool, now: Instant) -> Result<(), ScriptError> {
        if !safe || !matches!(error.code.as_str(), "javascript error" | "script timeout") {
            return Err(error);
        }
        if self.first.is_none() {
            self.first = Some(error);
        }
        self.attempts += 1;
        if self.attempts >= 8 {
            return Err(self.first.take().unwrap());
        }
        self.deadline = Some(now + Duration::from_millis(10));
        Ok(())
    }
}
impl KeyboardProgress {
    fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        page.validate_action_context(&self.context)?;
        if !self.retry.ready(now) {
            return Ok(false);
        }
        if self.accepted.is_none() {
            if let Some((source,_))=&self.interpretation{
                self.receipt.set_parent_metadata_bytes(OwnedOsKeyReceipt::metadata_bytes(source,&self.context)?)
                    .map_err(|error|ScriptError{code:"javascript error".into(),message:error.to_string()})?;
            }
            match page.poll_webdriver_keyboard_receipt_in_context(&self.context, &mut self.receipt)
            {
                Ok(Some(accepted)) => self.accepted = Some(accepted),
                Ok(None) => return Ok(false),
                Err(error) => {
                    self.retry.failed(error, self.receipt.retry_safe(), now)?;
                    return Ok(false);
                }
            }
        }
        // These reads and the checked receipt insertion cannot redispatch key
        // callbacks. The accepted result remains frozen if a read fails.
        if let Some((source, token)) = &self.interpretation {
            let accepted = self.accepted.unwrap();
            if self.interpretation_read.is_none() && self.interpretation_result.is_none() && self.interpretation_completed.is_none() {
                let target = page.os_keyboard_deferred_target(&self.context)?;
                let interpret = accepted && page.os_keyboard_was_deferred(&self.context)?;
                if let Some(target) = target.filter(|_| interpret) {
                    self.interpretation_read = Some((
                        target,
                        page.begin_composition_in_context(
                            &self.context,
                            target,
                            &oryn_js::composition::Operation::Read,
                        )?,
                    ));
                } else {
                    self.interpretation_lease=self.receipt.parent_metadata_lease();
                    self.interpretation_result = Some((target, interpret, None));
                }
            }
            if let Some((target, receipt)) = self.interpretation_read.as_mut() {
                let capacity=crate::native_input_allowance::InputAllowance::canonical_capacity().ok_or_else(||ScriptError::invalid("native input capacity overflow"))?;
                receipt.set_input_capacity_bytes(capacity).map_err(|error|ScriptError::javascript(error.to_string()))?;
                receipt.set_parent_metadata_bytes(OwnedOsKeyReceipt::metadata_bytes(source,&self.context)?)
                    .map_err(|error|ScriptError{code:"javascript error".into(),message:error.to_string()})?;
                match page.poll_composition_in_context(&self.context, receipt) {
                    Ok(Some(state)) => {
                        self.interpretation_lease=Some(std::sync::Arc::new((receipt.snapshot_lease(),receipt.parent_metadata_lease())));
                        let lease=receipt.input_capacity_lease().ok_or_else(||ScriptError::javascript("native input allowance not prepaid"))?;
                        let cancellation=receipt.input_capacity_cancellation().ok_or_else(||ScriptError::javascript("native input allowance owner unavailable"))?;
                        self.interpretation_allowance=Some(std::sync::Arc::new(crate::native_input_allowance::InputAllowance::new(capacity,cancellation,lease)));
                        self.interpretation_result =
                            Some((Some(*target), !state.aborted, Some(state)));
                    }
                    Ok(None) => return Ok(false),
                    Err(error) => {
                        self.retry.failed(error, receipt.retry_safe(), now)?;
                        return Ok(false);
                    }
                }
                self.interpretation_read = None;
            }
            if self.interpretation_completed.is_none(){
            let (target, interpret, state) = self.interpretation_result.take().unwrap();
            self.interpretation_completed=Some(OwnedOsKeyReceipt::new(OsKeyReceipt {
                continuation_done: !interpret,
                source: source.clone(),
                state,
                target,
                token: *token,
                accepted,
                interpret,
                context: self.context.clone(),
            },self.interpretation_lease.take()));
            if let Some(allowance)=self.interpretation_allowance.take(){
                self.interpretation_completed=Some(self.interpretation_completed.take().unwrap().with_input_allowance(allowance));
            }
            }
            input.push_owned_os_receipt(self.interpretation_completed.as_ref().unwrap().clone())?;
            self.interpretation_completed=None;
            self.interpretation = None;
        }
        Ok(true)
    }
}
fn prepare_os_key(
    page: &mut RustExecutableDocument,
    selected: &AutomationRealmSnapshot,
    input: &mut InputState,
    source: &str,
    key: &OsKey,
    cleanup: bool,
) -> Result<Option<KeyboardProgress>, ScriptError> {
    page.webdriver_keyboard_context(selected)?;
    let held = HeldInput::Key(key.value.clone());
    if !key.down && !input.is_pressed(source, &held) {
        input.remember_native_key(source, key);
        return Ok(None);
    }
    let repeated = if key.down {
        input.press(source, held)?
    } else {
        input.release(source, &held)?;
        false
    };
    let mut key = key.clone();
    if cleanup {
        input.clear_native_modifiers();
        let remaining = input.modifiers();
        key.modifiers.shift = remaining.shift;
        key.modifiers.control = remaining.control;
        key.modifiers.alt = remaining.alt;
        key.modifiers.meta = remaining.meta;
    }
    input.remember_native_key(source, &key);
    let modifiers = input.modifiers();
    let event = NativeKeyboardInput {
        defer_text: key.interpretation.is_some(),
        composing: key.composing,
        key: key.value.clone(),
        code: key.code.clone(),
        location: key.location,
        repeat: key.down && (key.repeat || repeated),
        down: key.down,
        modifiers: NativeKeyboardModifiers {
            shift: modifiers.shift,
            control: modifiers.control,
            alt: modifiers.alt,
            meta: modifiers.meta,
            alt_graph: key.modifiers.alt_graph,
            caps_lock: key.modifiers.caps_lock,
        },
    };
    let (context, receipt) = page.begin_webdriver_keyboard_receipt_in_context(selected, &event)?;
    Ok(Some(KeyboardProgress {
        context,
        receipt,
        interpretation: key.interpretation.map(|token| (source.into(), token)),
        accepted: None,
        interpretation_read: None,
        interpretation_result: None,
        interpretation_lease: None,
        interpretation_allowance: None,
        interpretation_completed: None,
        retry: Default::default(),
    }))
}
fn prepare_key(
    page: &mut RustExecutableDocument,
    selected: &AutomationRealmSnapshot,
    input: &mut InputState,
    source: &str,
    value: &[u16],
    down: bool,
) -> Result<Option<KeyboardProgress>, ScriptError> {
    if let Some(mut key) = input.native_key(source, value) {
        let modifiers = input.modifiers();
        key.down = down;
        key.repeat = false;
        key.interpretation = None;
        key.modifiers = OsKeyModifiers {
            shift: modifiers.shift,
            control: modifiers.control,
            alt: modifiers.alt,
            meta: modifiers.meta,
            ..key.modifiers
        };
        return prepare_os_key(page, selected, input, source, &key, true);
    }
    page.webdriver_keyboard_context(selected)?;
    let held = HeldInput::Key(value.to_vec());
    if !down && !input.is_pressed(source, &held) {
        return Ok(None);
    }
    let repeat = if down {
        input.press(source, held)?
    } else {
        input.release(source, &held)?;
        false
    };
    let modifiers = input.modifiers();
    let event = NativeKeyboardInput {
        defer_text: false,
        composing: false,
        key: value.to_vec(),
        code: String::new(),
        location: 0,
        repeat,
        down,
        modifiers: NativeKeyboardModifiers {
            shift: modifiers.shift,
            control: modifiers.control,
            alt: modifiers.alt,
            meta: modifiers.meta,
            ..Default::default()
        },
    };
    let (context, receipt) = page.begin_webdriver_keyboard_receipt_in_context(selected, &event)?;
    Ok(Some(KeyboardProgress {
        context,
        receipt,
        interpretation: None,
        accepted: None,
        interpretation_read: None,
        interpretation_result: None,
        interpretation_lease: None,
        interpretation_allowance: None,
        interpretation_completed: None,
        retry: Default::default(),
    }))
}

fn pointer_input(
    state: &mut InputState,
    source: &str,
    position: (f64, f64),
    movement: (f64, f64),
    button: i32,
    properties: &PointerProperties,
) -> Result<NativePointerInput, ScriptError> {
    let properties = properties.resolve(state.buttons(source)?)?;
    pointer_input_snapshot(state, source, position, movement, button, properties)
}
fn pointer_input_snapshot(
    state: &mut InputState,
    source: &str,
    position: (f64, f64),
    movement: (f64, f64),
    button: i32,
    properties: ResolvedPointerProperties,
) -> Result<NativePointerInput, ScriptError> {
    let device = match state.source_kind(source)? {
        SourceKind::Pointer(PointerKind::Mouse) => NativePointerDevice::Mouse,
        SourceKind::Pointer(PointerKind::Pen) => NativePointerDevice::Pen,
        SourceKind::Pointer(PointerKind::Touch) => NativePointerDevice::Touch,
        _ => {
            return Err(ScriptError::invalid(
                "native pointer requires pointer source",
            ));
        }
    };
    state.observe_pointer_range(source)?;
    let modifiers = state.modifiers();
    let buttons = state.buttons(source)?;
    let screen = state.screen_position(source)?;
    state.remember_pointer_properties(source, properties)?;
    Ok(NativePointerInput {
        id: state.pointer_id(source)?,
        device,
        primary: state.is_primary_pointer(source)?,
        x: position.0,
        y: position.1,
        screen_x: screen.0,
        screen_y: screen.1,
        movement_x: movement.0,
        movement_y: movement.1,
        button,
        buttons,
        width: properties.width,
        height: properties.height,
        pressure: if buttons == 0 {
            0.
        } else {
            properties.pressure
        },
        tangential_pressure: properties.tangential_pressure,
        tilt_x: properties.tilt_x,
        tilt_y: properties.tilt_y,
        twist: properties.twist,
        altitude_angle: properties.altitude_angle,
        azimuth_angle: properties.azimuth_angle,
        control: modifiers.control,
        shift: modifiers.shift,
        alt: modifiers.alt,
        meta: modifiers.meta,
    })
}

fn advance(
    motion: &mut Motion,
    elapsed: Duration,
    page: &mut RustExecutableDocument,
    selected: &AutomationRealmSnapshot,
    input: &mut InputState,
    now: Instant,
) -> Result<bool, ScriptError> {
    if let Motion::Pointer {
        pending, progress, ..
    } = motion
        && let Some((receipt, destination)) = pending.as_mut()
    {
        if !receipt.poll(page, now)? {
            return Ok(false);
        }
        *progress = *destination;
        *pending = None;
    }
    let (duration, previous) = match motion {
        Motion::Pointer {
            duration, progress, ..
        }
        | Motion::Wheel {
            duration, progress, ..
        } => (*duration, *progress),
    };
    let progress = if duration == 0 {
        1.
    } else {
        (elapsed.as_secs_f64() / (duration as f64 / 1000.)).clamp(previous, 1.)
    };
    if progress <= previous {
        return Ok(true);
    }
    match motion {
        Motion::Pointer {
            source,
            start,
            end,
            properties,
            progress: stored,
            pending,
            ..
        } => {
            let position = if progress == 1. {
                *end
            } else {
                (
                    start.0 + (end.0 - start.0) * progress,
                    start.1 + (end.1 - start.1) * progress,
                )
            };
            let old = input.position(source)?;
            let resolved = properties.resolve(input.buttons(source)?)?;
            if position != old || resolved != input.last_pointer_properties(source)? {
                let event = pointer_input_snapshot(
                    input,
                    source,
                    position,
                    (position.0 - old.0, position.1 - old.1),
                    -1,
                    resolved,
                )?;
                input.move_pointer(source, position.0, position.1)?;
                *pending = Some((
                    Box::new(PointerProgress::new(
                        page,
                        selected,
                        event,
                        NativePointerAction::Move,
                    )?),
                    progress,
                ));
                if !pending.as_mut().unwrap().0.poll(page, now)? {
                    return Ok(false);
                }
                *pending = None;
            }
            *stored = progress;
        }
        Motion::Wheel {
            transaction,
            position,
            delta,
            progress: stored,
            ..
        } => {
            let mut transaction = transaction.borrow_mut();
            let modifiers = input.modifiers();
            let event = NativeWheelInput {
                x: position.0,
                y: position.1,
                delta_x: delta.0 * (progress - previous),
                delta_y: delta.1 * (progress - previous),
                control: modifiers.control,
                shift: modifiers.shift,
                alt: modifiers.alt,
                meta: modifiers.meta,
                transaction: Some(transaction.0.clone()),
            };
            page.dispatch_webdriver_wheel_transaction_in_context(
                selected,
                &event,
                &mut transaction.1,
            )?;
            *stored = progress;
        }
    }
    Ok(true)
}

/// Release Actions preserves reverse held-input order while a keyup suffix is
/// pending. The actor owns this operation until every release finishes.
pub(crate) struct ReleaseExecution {
    selected: AutomationRealmSnapshot,
    started: bool,
    keyboard: Option<KeyboardProgress>,
    pointer: Option<PointerProgress>,
    done: bool,
    released_input: bool,
}
impl ReleaseExecution {
    pub(crate) fn completed_without_input_events(&self) -> bool {
        self.done && !self.released_input
    }
    pub(crate) fn new(selected: AutomationRealmSnapshot) -> Self {
        Self {
            selected,
            started: false,
            keyboard: None,
            pointer: None,
            done: false,
            released_input: false,
        }
    }
    pub(crate) fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        if self.done {
            return Ok(true);
        }
        page.validate_action_context(&self.selected)?;
        if !self.started {
            input.clear_native_modifiers();
            self.started = true;
        }
        loop {
            if let Some(keyboard) = self.keyboard.as_mut() {
                if !keyboard.poll(page, input, now)? {
                    return Ok(false);
                }
                self.keyboard = None;
            }
            if let Some(pointer) = self.pointer.as_mut() {
                if !pointer.poll(page, now)? {
                    return Ok(false);
                }
                self.pointer = None;
            }
            let Some(release) = input.next_release() else {
                input.finish_release()?;
                page.reset_webdriver_pointer_sources();
                self.done = true;
                return Ok(true);
            };
            self.released_input = true;
            match release.input {
                HeldInput::Key(value) => {
                    self.keyboard =
                        prepare_key(page, &self.selected, input, &release.source, &value, false)?;
                }
                HeldInput::Button(button) => {
                    let button_index = i32::try_from(button).map_err(|_| {
                        ScriptError::invalid(
                            "native pointer button exceeds supported integer range",
                        )
                    })?;
                    let position = input.position(&release.source)?;
                    input.release(&release.source, &HeldInput::Button(button))?;
                    let properties = input.last_pointer_properties(&release.source)?;
                    let event = pointer_input_snapshot(
                        input,
                        &release.source,
                        position,
                        (0., 0.),
                        button_index,
                        properties,
                    )?;
                    self.pointer = Some(PointerProgress::new(
                        page,
                        &self.selected,
                        event,
                        NativePointerAction::Up,
                    )?);
                }
            }
        }
    }
}
// Convenience driver for existing synchronous source regressions only. Native
// production Release always uses the actor-polled operation above.
#[cfg(test)]
fn release_actions(
    page: &mut RustExecutableDocument,
    selected: &AutomationRealmSnapshot,
    input: &mut InputState,
) -> Result<(), ScriptError> {
    let mut operation = ReleaseExecution::new(selected.clone());
    let start = Instant::now();
    for tick in 0..128 {
        if operation.poll(page, input, start + Duration::from_millis(tick * 10))? {
            return Ok(());
        }
    }
    Err(ScriptError::timeout())
}

/// OS focus loss / lifecycle cancellation is distinct from Release Actions:
/// a held pointer receives pointercancel, never pointerup/click/drop defaults.
pub(crate) struct CancelExecution {
    selected: AutomationRealmSnapshot,
    started: bool,
    sources: Vec<String>,
    next: usize,
    composition_targets: Vec<(AutomationRealmSnapshot, oryn_dom::NodeId)>,
    next_composition: usize,
    composition: Option<(
        AutomationRealmSnapshot,
        oryn_js::v8_bindings::NativeComposition,
        KeyboardRetry,
    )>,
    pointer: Option<PointerProgress>,
    keyboard: Option<KeyboardProgress>,
    composition_error: Option<ScriptError>,
    result: Option<Result<(), ScriptError>>,
}
impl CancelExecution {
    pub(crate) fn new(selected: AutomationRealmSnapshot) -> Self {
        Self {
            selected,
            started: false,
            sources: Vec::new(),
            next: 0,
            composition_targets: Vec::new(),
            next_composition: 0,
            composition: None,
            pointer: None,
            keyboard: None,
            composition_error: None,
            result: None,
        }
    }
    pub(crate) fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        if let Some(result) = &self.result {
            return result.clone().map(|_| true);
        }
        let result = self.poll_inner(page, input, now);
        if !matches!(result, Ok(false)) {
            // Drop pending receipts before clearing registries so each receipt
            // can revoke its exact session without touching a reused source.
            self.pointer = None;
            self.keyboard = None;
            self.composition = None;
            page.cancel_webdriver_keyboard_defaults();
            page.reset_webdriver_pointer_sources();
            input.retire();
            self.result = Some(result.clone().map(|_| ()));
        }
        result
    }
    fn poll_inner(
        &mut self,
        page: &mut RustExecutableDocument,
        input: &mut InputState,
        now: Instant,
    ) -> Result<bool, ScriptError> {
        page.validate_action_context(&self.selected)?;
        if !self.started {
            page.cancel_webdriver_keyboard_defaults();
            self.composition_targets = page.os_composition_targets();
            input.clear_native_modifiers();
            self.sources = input.pointer_sources();
            self.started = true;
        }
        loop {
            if let Some((selected, receipt, retry)) = self.composition.as_mut() {
                if !retry.ready(now) {
                    return Ok(false);
                }
                match page.poll_composition_in_context(selected, receipt) {
                    Ok(Some(_)) => {}
                    Ok(None) => return Ok(false),
                    Err(error) => match retry.failed(error, receipt.retry_safe(), now) {
                        Ok(()) => return Ok(false),
                        Err(error) => {
                            if self.composition_error.is_none() {
                                self.composition_error = Some(error);
                            }
                        }
                    },
                }
                self.composition = None;
            }
            if self.next_composition < self.composition_targets.len() {
                let (selected, target) = self.composition_targets[self.next_composition].clone();
                self.next_composition += 1;
                match page.begin_composition_in_context(
                    &selected,
                    target,
                    &oryn_js::composition::Operation::Cancel,
                ) {
                    Ok(receipt) => {
                        self.composition = Some((selected, receipt, KeyboardRetry::default()))
                    }
                    Err(error) => {
                        if self.composition_error.is_none() {
                            self.composition_error = Some(error);
                        }
                    }
                }
                continue;
            }
            if let Some(pointer) = self.pointer.as_mut() {
                if !pointer.poll(page, now)? {
                    return Ok(false);
                }
                self.pointer = None;
            }
            if self.next < self.sources.len() {
                let source = &self.sources[self.next];
                if input.buttons(source)? == 0 {
                    self.next += 1;
                    continue;
                }
                let position = input.position(source)?;
                let properties = input.last_pointer_properties(source)?;
                let mut event =
                    pointer_input_snapshot(input, source, position, (0., 0.), -1, properties)?;
                input.cancel_pointer(source)?;
                event.buttons = 0;
                event.pressure = 0.;
                self.pointer = Some(PointerProgress::new(
                    page,
                    &self.selected,
                    event,
                    NativePointerAction::Cancel,
                )?);
                self.next += 1;
                continue;
            }
            if let Some(keyboard) = self.keyboard.as_mut() {
                if !keyboard.poll(page, input, now)? {
                    return Ok(false);
                }
                self.keyboard = None;
            }
            if let Some(release) = input.next_release() {
                if let HeldInput::Key(value) = release.input {
                    self.keyboard =
                        prepare_key(page, &self.selected, input, &release.source, &value, false)?;
                    continue;
                }
                return Err(ScriptError::javascript(
                    "pointer cancellation retained a button",
                ));
            }
            return self.composition_error.take().map_or(Ok(true), Err);
        }
    }
}
// Synchronous test convenience only; runtime cancellation polls the receipt.
#[cfg(test)]
fn cancel_actions(
    page: &mut RustExecutableDocument,
    selected: &AutomationRealmSnapshot,
    input: &mut InputState,
) -> Result<(), ScriptError> {
    let mut operation = CancelExecution::new(selected.clone());
    let now = Instant::now();
    for tick in 0..128 {
        if operation.poll(page, input, now + Duration::from_millis(tick * 10))? {
            return Ok(());
        }
    }
    Err(ScriptError::timeout())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(
        operation: &mut ActionExecution,
        page: &mut RustExecutableDocument,
        state: &mut InputState,
    ) {
        let now = Instant::now();
        for _ in 0..32 {
            if operation.poll(page, state, now).unwrap() {
                return;
            }
        }
        panic!("zero-duration source ticks did not finish");
    }
    use crate::host::webdriver_script::ScriptValue;
    fn request(json: &str) -> ActionsRequest {
        ActionsRequest::parse(&ScriptValue::from_json_text(json).unwrap()).unwrap()
    }
    #[test]
    fn stale_cancel_retires_held_inputs_without_dispatching_into_a_new_lifetime() {
        let mut page=RustExecutableDocument::load("<button id=t style='position:absolute;left:0;top:0;width:100px;height:100px'>target</button>").unwrap();
        page.execute("globalThis.events=[];for(const type of ['click','keyup','pointerup','pointercancel'])t.addEventListener(type,e=>events.push(type));t.onpointerdown=e=>t.setPointerCapture(e.pointerId)").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut input = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"pointerMove","x":20,"y":20},{"type":"pointerDown","button":0}]},{"id":"k","type":"key","actions":[{"type":"pause"},{"type":"keyDown","value":" "}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut input).unwrap();
        finish(&mut operation, &mut page, &mut input);
        assert!(input.next_release().is_some());
        let mut stale = selected.clone();
        stale.document_generation += 1;
        assert!(cancel_actions(&mut page, &stale, &mut input).is_err());
        assert!(input.next_release().is_none());
        assert!(input.pointer_sources().is_empty());
        assert_eq!(page.evaluate("events.join(',')").unwrap(), "");
        release_actions(&mut page, &selected, &mut input).unwrap();
        assert_eq!(page.evaluate("events.join(',')").unwrap(), "");
    }
    #[test]
    fn selected_authority_rejects_element_routes_and_disarms_interrupted_inputs() {
        let mut page = RustExecutableDocument::load("<button id=t>target</button>").unwrap();
        page.execute("t.focus();globalThis.keys=[];t.onkeyup=e=>keys.push(e.type)")
            .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut input = InputState::default();
        let mut operation=ActionExecution::new(request(r#"{"actions":[{"id":"k","type":"key","actions":[{"type":"keyDown","value":"a"}]}]}"#),selected.clone(),&mut input).unwrap();
        finish(&mut operation, &mut page, &mut input);
        let mut stale = selected.clone();
        stale.document_authority_epoch += 1;
        assert!(page.validate_action_context(&stale).is_err());
        assert!(
            page.webdriver_element_command(
                Some(&stale),
                crate::host::webdriver_script::ElementCommand::Find {
                    root: None,
                    selectors: vec!["button".into()]
                }
            )
            .is_err()
        );
        assert!(cancel_actions(&mut page, &stale, &mut input).is_err());
        assert!(input.next_release().is_none());
        assert_eq!(page.evaluate("keys.length").unwrap(), "0");
        let mut inactive = selected.clone();
        inactive.live = false;
        assert!(
            page.webdriver_element_command(
                Some(&inactive),
                crate::host::webdriver_script::ElementCommand::Find {
                    root: None,
                    selectors: vec!["button".into()]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn os_keyboard_keeps_physical_layout_capslock_and_repeat_without_json_hooks() {
        let mut page = RustExecutableDocument::load("<textarea id=t></textarea>").unwrap();
        page.execute("t.focus();globalThis.keys=[];for(const name of ['keydown','keyup'])t.addEventListener(name,e=>keys.push([e.type,e.key,e.code,e.location,e.repeat,e.getModifierState('CapsLock'),e.isTrusted]))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut input = InputState::default();
        let key = OsKey {
            interpretation: None,
            composing: false,
            value: vec![65],
            code: "KeyQ".into(),
            location: 0,
            repeat: false,
            down: true,
            modifiers: OsKeyModifiers {
                caps_lock: true,
                ..Default::default()
            },
        };
        let actions = ActionsRequest {
            sources: vec![Source {
                id: "os-key".into(),
                kind: SourceKind::Key,
                actions: vec![
                    Action::OsKey(key.clone()),
                    Action::OsKey(OsKey {
                        repeat: true,
                        ..key
                    }),
                ],
            }],
        };
        actions.validate().unwrap();
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut input).unwrap();
        finish(&mut operation, &mut page, &mut input);
        cancel_actions(&mut page, &selected, &mut input).unwrap();
        assert_eq!(
            page.evaluate("JSON.stringify(keys)").unwrap(),
            r#"[["keydown","A","KeyQ",0,false,true,true],["keydown","A","KeyQ",0,true,true,true],["keyup","A","KeyQ",0,false,true,true]]"#
        );
        assert_eq!(page.evaluate("t.value").unwrap(), "AA");
        assert!(!input.modifiers().shift);
        assert!(ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"k","type":"key","actions":[{"type":"osKey","value":"A","code":"KeyQ"}]}]}"#).unwrap()).is_err());
    }

    #[test]
    fn os_pointer_capture_preserves_outside_viewport_and_screen_coordinates() {
        let mut page=RustExecutableDocument::load("<div id=t style='position:absolute;left:0;top:0;width:100px;height:100px'>Target</div>").unwrap();
        page.execute("globalThis.coords=[];t.onpointerdown=e=>t.setPointerCapture(e.pointerId);t.onpointermove=e=>coords.push([e.clientX,e.clientY,e.screenX,e.screenY,e.isTrusted])").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut input = InputState::default();
        let actions = ActionsRequest {
            sources: vec![Source {
                id: "os".into(),
                kind: SourceKind::Pointer(PointerKind::Mouse),
                actions: vec![
                    Action::OsPointerMove {
                        x: 20.,
                        y: 20.,
                        screen_x: 120.,
                        screen_y: 220.,
                    },
                    Action::PointerButton {
                        down: true,
                        button: 0,
                        properties: Default::default(),
                    },
                    Action::OsPointerMove {
                        x: -25.5,
                        y: 1500.25,
                        screen_x: 74.5,
                        screen_y: 1700.25,
                    },
                ],
            }],
        };
        actions.validate().unwrap();
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut input).unwrap();
        finish(&mut operation, &mut page, &mut input);
        assert_eq!(
            page.evaluate("JSON.stringify(coords.at(-1))").unwrap(),
            "[-25.5,1500.25,74.5,1700.25,true]"
        );
        cancel_actions(&mut page, &selected, &mut input).unwrap();
        assert!(ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"osPointerMove","x":-20,"y":1}]}]}"#).unwrap()).is_err());
    }

    #[test]
    fn native_cancel_disarms_click_space_and_capture_before_source_reuse() {
        let mut page=RustExecutableDocument::load("<button id=t style='position:absolute;left:0;top:0;width:100px;height:100px'>Target</button>").unwrap();
        page.execute("globalThis.events=[];for(const type of ['click','pointerup','pointercancel','lostpointercapture','keyup'])t.addEventListener(type,e=>events.push(type+':'+e.isTrusted));t.onpointerdown=e=>t.setPointerCapture(e.pointerId)").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut input = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"pointerMove","x":20,"y":20},{"type":"pointerDown","button":0},{"type":"pause","duration":0}]},{"id":"k","type":"key","actions":[{"type":"pause","duration":0},{"type":"pause","duration":0},{"type":"keyDown","value":" "}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut input).unwrap();
        finish(&mut operation, &mut page, &mut input);
        cancel_actions(&mut page, &selected, &mut input).unwrap();
        assert_eq!(page.evaluate("events.includes('pointercancel:true')&&!events.some(e=>e.startsWith('click:')||e.startsWith('pointerup:'))&&events.includes('keyup:true')").unwrap(),"true");
        assert!(input.next_release().is_none());
        assert!(input.pointer_id("p").is_err());
        let before = page.evaluate("events.length").unwrap();
        cancel_actions(&mut page, &selected, &mut input).unwrap();
        assert_eq!(page.evaluate("events.length").unwrap(), before);
    }

    #[test]
    fn release_discards_hover_history_before_pointer_identity_reuse() {
        let mut page=RustExecutableDocument::load("<div id=t style='position:absolute;left:0;top:0;width:100px;height:100px'>target</div>").unwrap();
        page.execute("globalThis.entries=0;document.getElementById('t').addEventListener('pointerover',()=>entries++)").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let movement = request(
            r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"pointerMove","x":20,"y":20}]}]}"#,
        );
        let mut first =
            ActionExecution::new(movement.clone(), selected.clone(), &mut state).unwrap();
        assert!(first.poll(&mut page, &mut state, Instant::now()).unwrap());
        let first_id = state.pointer_id("p").unwrap();
        release_actions(&mut page, &selected, &mut state).unwrap();
        let mut second = ActionExecution::new(movement, selected, &mut state).unwrap();
        assert_eq!(state.pointer_id("p").unwrap(), first_id);
        assert!(second.poll(&mut page, &mut state, Instant::now()).unwrap());
        assert_eq!(page.evaluate("entries").unwrap(), "2");
    }
    #[test]
    fn action_admission_returns_before_focus_dispatch_and_stale_admission_never_runs() {
        let mut page=RustExecutableDocument::load("<button id=t style='position:absolute;left:0;top:0;width:100px;height:100px'></button>").unwrap();
        page.execute(
            "globalThis.accepted=false;globalThis.focused=[];t.onfocus=()=>focused.push(accepted)",
        )
        .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"pointerMove","x":20,"y":20},{"type":"pointerDown","button":0}]}]}"#,
        );
        let mut operation =
            ActionExecution::new(actions.clone(), selected.clone(), &mut state).unwrap();
        assert_eq!(
            page.evaluate("focused.length+'|'+(document.activeElement===t)")
                .unwrap(),
            "0|false"
        );
        page.execute("accepted=true").unwrap();
        let now = Instant::now();
        assert!(!operation.poll(&mut page, &mut state, now).unwrap());
        assert!(operation.poll(&mut page, &mut state, now).unwrap());
        assert_eq!(page.evaluate("JSON.stringify(focused)").unwrap(), "[true]");
        release_actions(&mut page, &selected, &mut state).unwrap();
        page.execute("t.blur();focused=[]").unwrap();
        let mut stale = selected;
        stale.stream_epoch += 1;
        let mut operation = ActionExecution::new(actions, stale, &mut state).unwrap();
        assert!(
            operation
                .poll(&mut page, &mut state, Instant::now())
                .is_err()
        );
        assert_eq!(page.evaluate("focused.length").unwrap(), "0");
    }

    #[test]
    fn timed_ticks_wait_for_longest_source_and_never_advance_on_clock_regression() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let mut operation=ActionExecution::new(request(r#"{"actions":[{"id":"a","type":"none","actions":[{"type":"pause","duration":20}]},{"id":"b","type":"none","actions":[{"type":"pause","duration":50}]}]}"#),selected,&mut state).unwrap();
        let now = Instant::now();
        assert!(!operation.poll(&mut page, &mut state, now).unwrap());
        assert!(
            !operation
                .poll(&mut page, &mut state, now + Duration::from_millis(20))
                .unwrap()
        );
        assert!(!operation.poll(&mut page, &mut state, now).unwrap());
        assert!(
            operation
                .poll(&mut page, &mut state, now + Duration::from_millis(50))
                .unwrap()
        );
    }
    #[test]
    fn authenticated_release_reverses_mixed_source_press_order_and_is_idempotent() {
        let mut page=RustExecutableDocument::load("<button id=t style='position:absolute;left:0;top:0;width:100px;height:100px'></button>").unwrap();
        page.execute("globalThis.events=[];for(const type of ['mouseup','keyup','click'])document.addEventListener(type,e=>events.push([type,e.isTrusted]))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"keyboard","type":"key","actions":[{"type":"pause"},{"type":"keyDown","value":"\uE008"}]},{"id":"mouse","type":"pointer","actions":[{"type":"pointerMove","x":20,"y":20},{"type":"pointerDown","button":0}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut state).unwrap();
        let now = Instant::now();
        assert!(!operation.poll(&mut page, &mut state, now).unwrap());
        assert!(operation.poll(&mut page, &mut state, now).unwrap());
        release_actions(&mut page, &selected, &mut state).unwrap();
        release_actions(&mut page, &selected, &mut state).unwrap();
        // Release dispatches the inverse pointerUp action, including its click
        // default. The historical adapter suppressed this default.
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["mouseup",true],["click",true],["keyup",true]]"#
        );
        assert!(state.next_release().is_none());
    }

    #[test]
    fn keyboard_ticks_preserve_repeat_and_release_state() {
        let mut page = RustExecutableDocument::load("<button id=t>target</button>").unwrap();
        page.execute("globalThis.events=[];document.getElementById('t').focus();for(const name of ['keydown','keyup'])document.addEventListener(name,e=>{events.push([e.type,e.key,e.repeat,e.isTrusted]);e.preventDefault()})").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let mut operation=ActionExecution::new(request(r#"{"actions":[{"id":"k","type":"key","actions":[{"type":"keyDown","value":"a"},{"type":"keyDown","value":"a"}]}]}"#),selected.clone(),&mut state).unwrap();
        let now = Instant::now();
        assert!(!operation.poll(&mut page, &mut state, now).unwrap());
        assert!(operation.poll(&mut page, &mut state, now).unwrap());
        assert!(state.is_pressed("k", &HeldInput::Key(vec![97])));
        release_actions(&mut page, &selected, &mut state).unwrap();
        assert!(state.next_release().is_none());
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["keydown","a",false,true],["keydown","a",true,true],["keyup","a",false,true]]"#
        );
    }
}

#[cfg(test)]
mod pointer_execution_tests {
    use super::*;
    use crate::host::webdriver_script::ScriptValue;
    fn request(json: &str) -> ActionsRequest {
        ActionsRequest::parse(&ScriptValue::from_json_text(json).unwrap()).unwrap()
    }
    fn finish(
        operation: &mut ActionExecution,
        page: &mut RustExecutableDocument,
        state: &mut InputState,
    ) {
        let now = Instant::now();
        for _ in 0..32 {
            if operation.poll(page, state, now).unwrap() {
                return;
            }
        }
        panic!("zero-duration source ticks did not finish");
    }
    #[test]
    fn overlapping_touch_contacts_keep_primary_identity_through_final_up() {
        let mut page=RustExecutableDocument::load("<div style='position:absolute;left:0;top:0;width:200px;height:200px;touch-action:none'>target</div>").unwrap();
        page.execute("globalThis.events=[];for(const kind of ['pointerdown','pointerup','pointercancel'])document.addEventListener(kind,e=>events.push([e.type,e.pointerId,e.isPrimary,e.pressure,e.isTrusted]))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"a","type":"pointer","parameters":{"pointerType":"touch"},"actions":[{"type":"pointerMove","x":10,"y":10},{"type":"pointerDown","button":0},{"type":"pointerUp","button":0},{"type":"pause"}]},{"id":"b","type":"pointer","parameters":{"pointerType":"touch"},"actions":[{"type":"pointerMove","x":30,"y":30},{"type":"pointerDown","button":0},{"type":"pointerMove","x":40,"y":40},{"type":"pointerUp","button":0}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut state).unwrap();
        finish(&mut operation, &mut page, &mut state);
        assert_eq!(
            page.evaluate("JSON.stringify(events)").unwrap(),
            r#"[["pointerdown",1,true,0.5,true],["pointerdown",2,false,0.5,true],["pointerup",1,true,0,true],["pointerup",2,false,0,true]]"#
        );
        release_actions(&mut page, &selected, &mut state).unwrap();
    }
    #[test]
    fn same_position_pressure_change_dispatches_and_release_retains_actual_shape() {
        let mut page = RustExecutableDocument::load(
            "<div style='position:absolute;left:0;top:0;width:200px;height:200px'>target</div>",
        )
        .unwrap();
        page.execute("globalThis.events=[];for(const kind of ['pointerdown','pointermove','pointerup'])document.addEventListener(kind,e=>{if(e.buttons||e.type==='pointerup')events.push([e.type,e.width,e.pressure,e.tiltX,e.tiltY,e.twist])})").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"p","type":"pointer","parameters":{"pointerType":"pen"},"actions":[{"type":"pointerMove","x":10,"y":10},{"type":"pointerDown","button":0,"pressure":0.36,"altitudeAngle":0.3,"azimuthAngle":0.2419,"twist":86},{"type":"pointerMove","x":10,"y":10,"width":17,"pressure":0.8,"tiltX":21,"tiltY":-8,"twist":99}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected.clone(), &mut state).unwrap();
        finish(&mut operation, &mut page, &mut state);
        release_actions(&mut page, &selected, &mut state).unwrap();
        assert_eq!(
            page.evaluate(
                "JSON.stringify(events.map(e=>[e[0],e[1],Math.round(e[2]*100)/100,e[3],e[4],e[5]]))"
            )
            .unwrap(),
            r#"[["pointerdown",1,0.36,72,38,86],["pointermove",17,0.8,21,-8,99],["pointerup",17,0,21,-8,99]]"#
        );
    }
    #[test]
    fn pointer_cancel_uses_retained_geometry_and_clears_only_its_contact() {
        let mut page = RustExecutableDocument::load(
            "<div style='position:absolute;left:0;top:0;width:200px;height:200px'>target</div>",
        )
        .unwrap();
        page.execute("globalThis.cancelled=[];document.addEventListener('pointercancel',e=>cancelled.push([e.width,e.height,e.twist,e.pressure,e.buttons,e.isPrimary]))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut state = InputState::default();
        let actions = request(
            r#"{"actions":[{"id":"p","type":"pointer","parameters":{"pointerType":"touch"},"actions":[{"type":"pointerMove","x":10,"y":10},{"type":"pointerDown","button":0,"width":23,"height":31,"twist":355},{"type":"pointerCancel"}]}]}"#,
        );
        let mut operation = ActionExecution::new(actions, selected, &mut state).unwrap();
        finish(&mut operation, &mut page, &mut state);
        assert_eq!(
            page.evaluate("JSON.stringify(cancelled)").unwrap(),
            "[[23,31,355,0,0,true]]"
        );
        assert!(!state.pointer_contact_active("p").unwrap());
        assert!(state.next_release().is_none());
    }
}

#[cfg(test)]
mod wheel_transaction_tests {
    use super::*;
    use crate::host::webdriver_script::ScriptValue;
    fn run(page: &mut RustExecutableDocument, input: &mut InputState, json: &str) {
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let request = ActionsRequest::parse(&ScriptValue::from_json_text(json).unwrap()).unwrap();
        let mut operation = ActionExecution::new(request, selected, input).unwrap();
        let now = Instant::now();
        for _ in 0..32 {
            if operation.poll(page, input, now).unwrap() {
                return;
            }
        }
        panic!("wheel action chain did not complete");
    }
    const THREE: &str = r#"{"actions":[{"id":"wheel","type":"wheel","actions":[{"type":"scroll","x":10,"y":2,"deltaX":0,"deltaY":30,"duration":0},{"type":"scroll","x":10,"y":2,"deltaX":0,"deltaY":30,"duration":0},{"type":"scroll","x":10,"y":2,"deltaX":0,"deltaY":30,"duration":0}]}]}"#;
    const ONE: &str = r#"{"actions":[{"id":"wheel","type":"wheel","actions":[{"type":"scroll","x":10,"y":2,"deltaX":0,"deltaY":30,"duration":0}]}]}"#;
    fn page() -> RustExecutableDocument {
        let mut page=RustExecutableDocument::load("<!doctype html><style>body{margin:0}#a,#b{height:25px}#c{height:3000px}</style><div id=a>text</div><div id=b></div><div id=c></div>").unwrap();
        page.execute("globalThis.seen=[];window.addEventListener('wheel',e=>seen.push([e.target.id,e.isTrusted,e.deltaY]),{passive:true})").unwrap();
        page
    }
    #[test]
    fn wheel_chain_keeps_element_target_across_scroll_and_new_chain_retargets() {
        let mut page = page();
        let mut input = InputState::default();
        run(&mut page, &mut input, THREE);
        assert_eq!(page.evaluate("seen.length===3&&seen.every(e=>e[0]==='a'&&e[1]&&e[2]===30)&&document.scrollingElement.scrollTop===90").unwrap(),"true");
        run(&mut page, &mut input, ONE);
        assert_eq!(
            page.evaluate("seen.length===4&&seen[3][0]==='c'").unwrap(),
            "true"
        );
    }
    #[test]
    fn target_removal_dom_move_and_loss_of_box_end_transaction_but_resize_does_not() {
        for (change, retained) in [
            ("e.target.remove()", false),
            ("c.after(e.target)", false),
            ("e.target.style.display='none'", false),
            ("e.target.style.display='contents'", false),
            ("e.target.style.height='10px'", true),
        ] {
            let mut page = page();
            page.execute(&format!(
                "window.addEventListener('wheel',e=>{{{change}}},{{once:true,passive:true}})"
            ))
            .unwrap();
            run(&mut page, &mut InputState::default(), THREE);
            assert_eq!(
                page.evaluate(
                    "seen[0][0]==='a'&&seen.length===3&&document.scrollingElement.scrollTop>0"
                )
                .unwrap(),
                "true",
                "{change}"
            );
            assert_eq!(
                page.evaluate("seen.slice(1).every(e=>e[0]==='a')").unwrap(),
                retained.to_string(),
                "{change}"
            );
        }
    }
    #[test]
    fn simultaneous_wheel_sources_have_independent_targets_and_cancelable_defaults() {
        let mut page=RustExecutableDocument::load("<style>body{margin:0;height:3000px}div{position:fixed;left:0;width:100px;height:40px}#a{top:0}#b{top:50px}</style><div id=a></div><div id=b></div>").unwrap();
        page.execute("globalThis.seen=[];window.addEventListener('wheel',e=>{seen.push(e.target.id);e.preventDefault()},{passive:false})").unwrap();
        run(
            &mut page,
            &mut InputState::default(),
            r#"{"actions":[{"id":"one","type":"wheel","actions":[{"type":"scroll","x":10,"y":10,"deltaX":0,"deltaY":30,"duration":0},{"type":"scroll","x":10,"y":60,"deltaX":0,"deltaY":30,"duration":0}]},{"id":"two","type":"wheel","actions":[{"type":"scroll","x":10,"y":60,"deltaX":0,"deltaY":30,"duration":0},{"type":"scroll","x":10,"y":10,"deltaX":0,"deltaY":30,"duration":0}]}]}"#,
        );
        assert_eq!(
            page.evaluate("seen.join()==='a,b,a,b'&&document.scrollingElement.scrollTop===0")
                .unwrap(),
            "true"
        );
    }
}

#[cfg(test)]
mod stationary_pointer_tests {
    use super::*;
    fn move_to(page: &mut RustExecutableDocument, x: f64, y: f64) {
        let root = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        page.dispatch_webdriver_pointer_in_context(
            &root,
            &NativePointerInput {
                x,
                y,
                ..Default::default()
            },
            NativePointerAction::Move,
        )
        .unwrap();
    }
    #[test]
    fn stationary_layout_change_publishes_boundaries_without_mousemove() {
        let mut page=RustExecutableDocument::load("<style>body{margin:0}#under{position:absolute;left:0;top:0;width:150px;height:150px}#target{position:absolute;left:0;top:0;width:80px;height:80px}</style><div id=under></div><div id=target></div>").unwrap();
        move_to(&mut page, 20., 20.);
        page.execute("globalThis.events=[];for(const type of ['mouseover','mouseout','mouseenter','mouseleave','mousemove'])document.addEventListener(type,e=>events.push([type,e.target.id,e.isTrusted]),true);target.style.left='200px'").unwrap();
        assert_eq!(page.evaluate("events.some(e=>e[0]==='mouseout'&&e[1]==='target')&&events.some(e=>e[0]==='mouseover'&&e[1]==='under')&&!events.some(e=>e[0]==='mousemove')&&events.every(e=>e[2])").unwrap(),"true");
    }
    #[test]
    fn removed_hover_leaf_preserves_ancestor_enter_state_and_current_modifiers() {
        let mut page=RustExecutableDocument::load("<style>body{margin:0}#grand,#parent,#child{width:80px;height:80px}</style><div id=grand><div id=parent><div id=child></div></div></div>").unwrap();
        move_to(&mut page, 20., 20.);
        let root = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        page.dispatch_webdriver_keyboard_in_context(
            &root,
            &NativeKeyboardInput {
                key: vec![0xe009],
                down: true,
                modifiers: NativeKeyboardModifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        page.execute("globalThis.events=[];for(const type of ['mouseover','mouseout','mouseenter','mouseleave','mousemove'])document.addEventListener(type,e=>events.push([type,e.target.id,e.ctrlKey]),true);child.remove()").unwrap();
        assert_eq!(page.evaluate("events.length===1&&events[0][0]==='mouseover'&&events[0][1]==='parent'&&events[0][2]").unwrap(),"true");
        page.execute("events=[]").unwrap();
        move_to(&mut page, 150., 150.);
        assert_eq!(page.evaluate("!events.some(e=>e[0]==='mouseenter'&&(e[1]==='parent'||e[1]==='grand'))&&events.some(e=>e[0]==='mouseleave'&&e[1]==='parent')&&events.some(e=>e[0]==='mouseleave'&&e[1]==='grand')").unwrap(),"true");
    }
    #[test]
    fn temporary_reappend_does_not_duplicate_stationary_enter_events() {
        let mut page=RustExecutableDocument::load("<style>body{margin:0}#parent,#child{width:80px;height:80px}</style><div id=parent><div id=child></div></div>").unwrap();
        move_to(&mut page, 20., 20.);
        page.execute("globalThis.events=[];for(const type of ['mouseover','mouseout','mouseenter','mouseleave'])document.addEventListener(type,e=>events.push(type),true);document.getElementById('parent').appendChild(child)").unwrap();
        assert_eq!(page.evaluate("events.length").unwrap(), "0");
    }
}

#[cfg(test)]
mod keyboard_receipt_tests {
    use super::*;
    use crate::host::webdriver_script::ScriptValue;
    #[test]
    fn pending_second_source_keeps_cursor_and_held_key_mutation_once() {
        let mut page = RustExecutableDocument::load("<input id=t>").unwrap();
        page.execute("t.focus();globalThis.keys=[];t.addEventListener('keydown',e=>keys.push([e.key,e.repeat]))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_none())
            .unwrap();
        let request=ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"first","type":"key","actions":[{"type":"keyDown","value":"a"}]},{"id":"second","type":"key","actions":[{"type":"keyDown","value":"b"}]}]}"#).unwrap()).unwrap();
        let mut input = InputState::default();
        let mut operation = ActionExecution::new(request, selected.clone(), &mut input).unwrap();
        let now = Instant::now();
        // Construct the precise suspended state after source zero completed and
        // source one was admitted. Admission itself must not deliver callbacks.
        let mut first = prepare_key(&mut page, &selected, &mut input, "first", &[97], true)
            .unwrap()
            .unwrap();
        assert!(first.poll(&mut page, &mut input, now).unwrap());
        let mut second = prepare_key(&mut page, &selected, &mut input, "second", &[98], true)
            .unwrap()
            .unwrap();
        second.retry.deadline = Some(now + Duration::from_millis(10));
        operation.started = Some(now);
        operation.next_source = 1;
        operation.keyboard = Some(second);
        assert!(!operation.poll(&mut page, &mut input, now).unwrap());
        assert_eq!(operation.next_source, 1);
        assert_eq!(page.evaluate("t.value").unwrap(), "a");
        assert!(input.is_pressed("second", &HeldInput::Key(vec![98])));
        assert!(
            operation
                .poll(&mut page, &mut input, now + Duration::from_millis(10))
                .unwrap()
        );
        assert_eq!(
            page.evaluate("JSON.stringify(keys)").unwrap(),
            r#"[["a",false],["b",false]]"#
        );
        assert_eq!(page.evaluate("t.value").unwrap(), "ab");
        assert!(operation.keyboard.is_none());
    }
    #[test]
    fn pending_release_keeps_reverse_order_and_defers_source_cleanup() {
        let mut page = RustExecutableDocument::load("<input id=t>").unwrap();
        page.execute(
            "t.focus();globalThis.keys=[];t.addEventListener('keyup',e=>keys.push(e.key))",
        )
        .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_none())
            .unwrap();
        let request=ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"key","type":"key","actions":[{"type":"keyDown","value":"a"},{"type":"keyDown","value":"b"}]}]}"#).unwrap()).unwrap();
        let mut input = InputState::default();
        let mut perform = ActionExecution::new(request, selected.clone(), &mut input).unwrap();
        let now = Instant::now();
        while !perform.poll(&mut page, &mut input, now).unwrap() {}
        let mut release = ReleaseExecution::new(selected.clone());
        release.started = true;
        input.clear_native_modifiers();
        let mut pending = prepare_key(&mut page, &selected, &mut input, "key", &[98], false)
            .unwrap()
            .unwrap();
        pending.retry.deadline = Some(now + Duration::from_millis(10));
        release.keyboard = Some(pending);
        assert!(!release.poll(&mut page, &mut input, now).unwrap());
        assert_eq!(
            input.next_release().unwrap().input,
            HeldInput::Key(vec![97])
        );
        assert!(page.evaluate("keys.length===0").unwrap() == "true");
        assert!(
            release
                .poll(&mut page, &mut input, now + Duration::from_millis(10))
                .unwrap()
        );
        assert_eq!(page.evaluate("keys.join()").unwrap(), "b,a");
        assert!(input.next_release().is_none());
        assert!(
            release
                .poll(&mut page, &mut input, now + Duration::from_millis(20))
                .unwrap()
        );
        assert_eq!(page.evaluate("keys.join()").unwrap(), "b,a");
    }
    #[test]
    fn keyboard_retry_is_delayed_bounded_and_preserves_original_error() {
        let now = Instant::now();
        let first = ScriptError::javascript("original allocation failure");
        let mut retry = KeyboardRetry::default();
        retry.failed(first.clone(), true, now).unwrap();
        assert!(!retry.ready(now));
        assert!(!retry.ready(now + Duration::from_millis(9)));
        assert!(retry.ready(now + Duration::from_millis(10)));
        for _ in 0..6 {
            retry
                .failed(ScriptError::javascript("later error"), true, now)
                .unwrap();
        }
        assert_eq!(
            retry
                .failed(ScriptError::javascript("last error"), true, now)
                .unwrap_err(),
            first
        );
        assert!(
            KeyboardRetry::default()
                .failed(ScriptError::javascript("unretained activation"), false, now)
                .is_err()
        );
        assert!(
            KeyboardRetry::default()
                .failed(ScriptError::invalid("invalid source"), true, now)
                .is_err()
        );
    }
}

#[cfg(test)]
mod composition_receipt_tests {
    use super::*;
    #[test]
    fn pending_lifecycle_composition_cancel_finishes_before_source_retirement() {
        let mut page = RustExecutableDocument::load("<textarea id=t>abc</textarea>").unwrap();
        page.execute(
            "t.focus();t.setSelectionRange(1,2);globalThis.ends=0;t.oncompositionend=()=>ends++",
        )
        .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        page.dispatch_webdriver_keyboard_in_context(
            &selected,
            &NativeKeyboardInput {
                defer_text: true,
                composing: false,
                key: vec![88],
                code: "KeyX".into(),
                location: 0,
                repeat: false,
                down: true,
                modifiers: Default::default(),
            },
        )
        .unwrap();
        let target = page
            .os_keyboard_deferred_target(&selected)
            .unwrap()
            .unwrap();
        let mut marked = page
            .begin_composition_in_context(
                &selected,
                target,
                &oryn_js::composition::Operation::SetMarked {
                    text: vec![88],
                    selected: oryn_js::composition::TextRange {
                        start: 1,
                        length: 0,
                    },
                    replacement: None,
                },
            )
            .unwrap();
        assert!(
            page.poll_composition_in_context(&selected, &mut marked)
                .unwrap()
                .is_some()
        );
        let receipt = page
            .begin_composition_in_context(
                &selected,
                target,
                &oryn_js::composition::Operation::Cancel,
            )
            .unwrap();
        let now = Instant::now();
        let mut cancel = CancelExecution::new(selected.clone());
        cancel.started = true;
        cancel.composition = Some((
            selected.clone(),
            receipt,
            KeyboardRetry {
                deadline: Some(now + Duration::from_millis(10)),
                ..Default::default()
            },
        ));
        let mut input = InputState::default();
        assert!(!cancel.poll(&mut page, &mut input, now).unwrap());
        assert_eq!(page.evaluate("t.value+':'+ends").unwrap(), "aXc:0");
        assert!(
            cancel
                .poll(&mut page, &mut input, now + Duration::from_millis(10))
                .unwrap()
        );
        assert!(
            cancel
                .poll(&mut page, &mut input, now + Duration::from_millis(20))
                .unwrap()
        );
        assert_eq!(page.evaluate("t.value+':'+ends").unwrap(), "abc:1");
        assert!(page.os_composition_targets().is_empty());
        let mut marked = page
            .begin_composition_in_context(
                &selected,
                target,
                &oryn_js::composition::Operation::SetMarked {
                    text: vec![89],
                    selected: oryn_js::composition::TextRange {
                        start: 1,
                        length: 0,
                    },
                    replacement: None,
                },
            )
            .unwrap();
        assert!(
            page.poll_composition_in_context(&selected, &mut marked)
                .unwrap()
                .is_some()
        );
        let mut stale = selected.clone();
        stale.document_generation = stale.document_generation.wrapping_add(1);
        let mut cancellation = CancelExecution::new(selected.clone());
        cancellation.started = true;
        cancellation.composition_targets = vec![(stale, target), (selected, target)];
        let failure = cancellation
            .poll(&mut page, &mut input, now + Duration::from_millis(30))
            .unwrap_err();
        assert_eq!(
            cancellation
                .poll(&mut page, &mut input, now + Duration::from_millis(40))
                .unwrap_err(),
            failure
        );
        assert_eq!(page.evaluate("t.value+':'+ends").unwrap(), "abc:2");
        assert!(page.os_composition_targets().is_empty());
    }
    #[test]
    fn pending_os_text_keeps_one_sequence_and_one_returned_receipt() {
        let mut page = RustExecutableDocument::load("<textarea id=t>abc</textarea>").unwrap();
        page.execute("t.focus();t.setSelectionRange(1,2);globalThis.starts=0;t.oncompositionstart=()=>starts++").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        page.dispatch_webdriver_keyboard_in_context(
            &selected,
            &NativeKeyboardInput {
                defer_text: true,
                composing: false,
                key: vec![88],
                code: "KeyX".into(),
                location: 0,
                repeat: false,
                down: true,
                modifiers: Default::default(),
            },
        )
        .unwrap();
        let target = page
            .os_keyboard_deferred_target(&selected)
            .unwrap()
            .unwrap();
        let mut input = InputState::default();
        input
            .push_os_receipt(OsKeyReceipt {
                continuation_done: false,
                source: "os".into(),
                state: None,
                target: Some(target),
                token: 777,
                accepted: true,
                interpret: true,
                context: selected,
            })
            .unwrap();
        input.take_os_receipts();
        let mut progress = OsTextProgress::new(
            &mut page,
            &mut input,
            "os",
            777,
            1,
            &oryn_js::composition::Operation::SetMarked {
                text: vec![88],
                selected: oryn_js::composition::TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        let now = Instant::now();
        progress.retry.deadline = Some(now + Duration::from_millis(10));
        assert!(!progress.poll(&mut page, &mut input, now).unwrap());
        assert_eq!(page.evaluate("t.value").unwrap(), "abc");
        assert!(input.admit_os_text("os", 777, 1).is_err());
        assert!(
            progress
                .poll(&mut page, &mut input, now + Duration::from_millis(10))
                .unwrap()
        );
        assert!(
            progress
                .poll(&mut page, &mut input, now + Duration::from_millis(20))
                .unwrap()
        );
        assert_eq!(page.evaluate("t.value").unwrap(), "aXc");
        assert_eq!(page.evaluate("starts").unwrap(), "1");
        let receipts = input.take_os_receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].state.as_ref().unwrap().text, vec![97, 88, 99]);
        let mut finish = OsTextProgress::new(
            &mut page,
            &mut input,
            "os",
            777,
            2,
            &oryn_js::composition::Operation::Finish { keep_marked: false },
        )
        .unwrap();
        assert!(
            finish
                .poll(&mut page, &mut input, now + Duration::from_millis(30))
                .unwrap()
        );
        assert!(input.admit_os_text("os", 777, 3).is_err());
    }
}

#[cfg(test)]
mod pointer_admission_tests {
    use super::*;
    #[test]
    fn pending_ordinary_pointer_keeps_admitted_sample_and_runs_default_once() {
        let mut page = RustExecutableDocument::load(
            "<style>body{margin:0}#t{width:100px;height:100px}</style><button id=t>target</button>",
        )
        .unwrap();
        page.execute("globalThis.trace=[];for(const type of ['pointerdown','mousedown','focus','pointerup','mouseup','click'])t.addEventListener(type,e=>trace.push([type,e.clientX??null,e.ctrlKey??null]));").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let original = NativePointerInput {
            x: 10.,
            y: 10.,
            button: 0,
            buttons: 1,
            control: true,
            ..Default::default()
        };
        let mut progress = PointerProgress::new(
            &mut page,
            &selected,
            original.clone(),
            NativePointerAction::Down,
        )
        .unwrap();
        progress.dispatch = Some(
            page.begin_pointer_dispatch_in_context(&selected, &original, NativePointerAction::Down)
                .unwrap(),
        );
        progress.transport_prepared = true;
        // Mutating the caller's staging fields cannot change an admitted sample.
        progress.input.x = 300.;
        progress.input.control = false;
        let now = Instant::now();
        progress.retry.deadline = Some(now + Duration::from_millis(10));
        assert!(!progress.poll(&mut page, now).unwrap());
        assert_eq!(page.evaluate("trace.length").unwrap(), "0");
        assert!(
            progress
                .poll(&mut page, now + Duration::from_millis(10))
                .unwrap()
        );
        assert!(
            progress
                .poll(&mut page, now + Duration::from_millis(20))
                .unwrap()
        );
        let mut release = PointerProgress::new(
            &mut page,
            &selected,
            NativePointerInput {
                buttons: 0,
                ..original
            },
            NativePointerAction::Up,
        )
        .unwrap();
        assert!(
            release
                .poll(&mut page, now + Duration::from_millis(30))
                .unwrap()
        );
        assert!(
            release
                .poll(&mut page, now + Duration::from_millis(40))
                .unwrap()
        );
        assert_eq!(
            page.evaluate("JSON.stringify(trace.filter(e=>e[0]!=='focus'))")
                .unwrap(),
            "[[\"pointerdown\",10,true],[\"mousedown\",10,true],[\"pointerup\",10,true],[\"mouseup\",10,true],[\"click\",10,true]]"
        );
        assert_eq!(
            page.evaluate("trace.filter(e=>e[0]==='focus').length")
                .unwrap(),
            "1"
        );
    }
    #[test]
    fn stale_selected_pointer_owner_cancels_retained_dispatch() {
        let mut page = RustExecutableDocument::load("<button id=t>target</button>").unwrap();
        page.execute("globalThis.downs=0;t.onpointerdown=()=>downs++")
            .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let input = NativePointerInput {
            x: 10.,
            y: 10.,
            buttons: 1,
            button: 0,
            ..Default::default()
        };
        let mut progress = PointerProgress::new(
            &mut page,
            &selected,
            input.clone(),
            NativePointerAction::Down,
        )
        .unwrap();
        let receipt = page
            .begin_pointer_dispatch_in_context(&selected, &input, NativePointerAction::Down)
            .unwrap();
        progress.dispatch = Some(receipt);
        progress.transport_prepared = true;
        progress.selected.document_generation =
            progress.selected.document_generation.wrapping_add(1);
        let now = Instant::now();
        let error = progress.poll(&mut page, now).unwrap_err();
        assert!(progress.dispatch.is_none());
        assert_eq!(
            progress
                .poll(&mut page, now + Duration::from_millis(20))
                .unwrap_err(),
            error
        );
        assert_eq!(page.evaluate("downs").unwrap(), "0");
    }

    #[test]
    fn terminal_pointer_failure_remains_failure_on_later_poll() {
        let mut page = RustExecutableDocument::load("<button id=t>target</button>").unwrap();
        page.execute("globalThis.clicks=0;t.onclick=()=>clicks++")
            .unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let mut receipt = PointerProgress::new(
            &mut page,
            &selected,
            NativePointerInput {
                x: f64::NAN,
                buttons: 1,
                button: 0,
                ..Default::default()
            },
            NativePointerAction::Down,
        )
        .unwrap();
        let now = Instant::now();
        let first = receipt.poll(&mut page, now).unwrap_err();
        assert_eq!(
            receipt
                .poll(&mut page, now + Duration::from_secs(1))
                .unwrap_err(),
            first
        );
        assert_eq!(page.evaluate("clicks").unwrap(), "0");
        assert!(receipt.admission.is_none());
        assert!(receipt.transport.is_none());
    }
    #[test]
    fn release_drops_but_lifecycle_cancel_only_ends_the_owned_drag() {
        use crate::host::webdriver_script::ScriptValue;
        for cancel in [false, true] {
            let mut page=RustExecutableDocument::load("<style>body{margin:0}#s,#t{position:absolute;top:0;width:80px;height:80px}#s{left:0}#t{left:200px}</style><div id=s draggable=true>source</div><div id=t>target</div>").unwrap();
            page.execute("globalThis.trace=[];s.addEventListener('dragstart',()=>trace.push('start'));s.addEventListener('dragend',()=>trace.push('end'));t.addEventListener('dragover',e=>{e.preventDefault();e.dataTransfer.dropEffect='copy'});t.addEventListener('drop',e=>{trace.push('drop');e.preventDefault()});document.addEventListener('click',()=>trace.push('click'))").unwrap();
            let selected = page
                .automation_realms()
                .into_iter()
                .find(|entry| entry.parent_realm.is_none())
                .unwrap();
            let request=ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"mouse","type":"pointer","parameters":{"pointerType":"mouse"},"actions":[{"type":"pointerMove","x":10,"y":10,"duration":0},{"type":"pointerDown","button":0},{"type":"pointerMove","x":210,"y":10,"duration":0}]}]}"#).unwrap()).unwrap();
            let mut input = InputState::default();
            let mut operation =
                ActionExecution::new(request, selected.clone(), &mut input).unwrap();
            let now = Instant::now();
            let mut complete = false;
            for tick in 0..10 {
                if operation
                    .poll(
                        &mut page,
                        &mut input,
                        now + Duration::from_millis(tick * 10),
                    )
                    .unwrap()
                {
                    complete = true;
                    break;
                }
            }
            assert!(complete);
            if cancel {
                cancel_actions(&mut page, &selected, &mut input).unwrap();
            } else {
                release_actions(&mut page, &selected, &mut input).unwrap();
            }
            assert_eq!(
                page.evaluate("trace.join(',')").unwrap(),
                if cancel {
                    "start,end"
                } else {
                    "start,drop,end"
                }
            );
            assert!(input.pointer_sources().is_empty());
            assert!(input.next_release().is_none());
        }
    }
    #[test]
    fn actions_drag_owns_transport_until_drop_and_does_not_emit_click() {
        use crate::host::webdriver_script::ScriptValue;
        let mut page=RustExecutableDocument::load("<style>body{margin:0}#s,#t{position:absolute;top:0;width:80px;height:80px}#s{left:0}#t{left:200px}</style><div id=s draggable=true>source</div><div id=t>target</div>").unwrap();
        page.execute("globalThis.trace=[];s.addEventListener('dragstart',e=>{trace.push('start');e.dataTransfer.setData('text/plain','payload')});s.addEventListener('dragend',()=>trace.push('end'));t.addEventListener('dragenter',()=>trace.push('enter'));t.addEventListener('dragover',e=>{trace.push('over');e.preventDefault();e.dataTransfer.dropEffect='copy'});t.addEventListener('drop',e=>{trace.push('drop:'+e.dataTransfer.getData('text/plain'));e.preventDefault()});document.addEventListener('click',()=>trace.push('click'))").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let request=ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"mouse","type":"pointer","parameters":{"pointerType":"mouse"},"actions":[{"type":"pointerMove","x":10,"y":10,"duration":0},{"type":"pointerDown","button":0},{"type":"pointerMove","x":210,"y":10,"duration":0},{"type":"pointerUp","button":0}]}]}"#).unwrap()).unwrap();
        let mut input = InputState::default();
        let mut operation = ActionExecution::new(request, selected, &mut input).unwrap();
        let now = Instant::now();
        let mut completed = false;
        for tick in 0..10 {
            if operation
                .poll(
                    &mut page,
                    &mut input,
                    now + Duration::from_millis(tick * 10),
                )
                .unwrap()
            {
                completed = true;
                break;
            }
        }
        assert!(completed);
        assert!(
            operation
                .poll(&mut page, &mut input, now + Duration::from_millis(200))
                .unwrap()
        );
        assert_eq!(
            page.evaluate("trace.join(',')").unwrap(),
            "start,enter,over,drop:payload,end"
        );
        assert!(!input.is_pressed("mouse", &HeldInput::Button(0)));
    }
    #[test]
    fn pending_motion_keeps_original_pointer_input_and_admits_drag_once() {
        let mut page=RustExecutableDocument::load("<style>body{margin:0}#s{width:200px;height:100px}</style><div id=s draggable=true>source</div>").unwrap();
        page.execute("globalThis.starts=[];s.addEventListener('dragstart',e=>{starts.push([e.clientX,e.ctrlKey]);e.preventDefault()})").unwrap();
        let selected = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        let press = NativePointerInput {
            x: 10.,
            y: 10.,
            button: 0,
            buttons: 1,
            ..Default::default()
        };
        page.dispatch_webdriver_pointer_in_context(&selected, &press, NativePointerAction::Down)
            .unwrap();
        let event = NativePointerInput {
            x: 30.,
            y: 10.,
            movement_x: 20.,
            buttons: 1,
            control: true,
            ..Default::default()
        };
        let mut receipt =
            PointerProgress::new(&mut page, &selected, event, NativePointerAction::Move).unwrap();
        assert!(receipt.admission.is_some());
        let now = Instant::now();
        receipt.retry.deadline = Some(now + Duration::from_millis(10));
        assert!(!receipt.poll(&mut page, now).unwrap());
        assert_eq!(page.evaluate("starts.length").unwrap(), "0");
        assert!(
            receipt
                .poll(&mut page, now + Duration::from_millis(10))
                .unwrap()
        );
        assert!(
            receipt
                .poll(&mut page, now + Duration::from_millis(20))
                .unwrap()
        );
        assert_eq!(
            page.evaluate("JSON.stringify(starts)").unwrap(),
            "[[10,true]]"
        );
        // A canceled dragstart consumed this press's attempt. Later motion
        // must not re-admit it through the local pointer fallback.
        let mut later = PointerProgress::new(
            &mut page,
            &selected,
            NativePointerInput {
                x: 50.,
                y: 10.,
                buttons: 1,
                ..Default::default()
            },
            NativePointerAction::Move,
        )
        .unwrap();
        assert!(
            later
                .poll(&mut page, now + Duration::from_millis(30))
                .unwrap()
        );
        assert_eq!(page.evaluate("starts.length").unwrap(), "1");
    }
}

#[cfg(test)]
mod image_map_pointer_tests {
    use super::*;
    fn image_page(html: &str) -> RustExecutableDocument {
        use crate::network::{
            NetworkBroker, NetworkError, NetworkRequest, NetworkResponse, SharedNetworkBroker,
        };
        struct NoExternalNetwork(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl NetworkBroker for NoExternalNetwork {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(NetworkError::Transport(
                    "image-map fixture attempted external network".into(),
                ))
            }
        }
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut page = RustExecutableDocument::load_with_environment(
            html,
            &std::collections::BTreeMap::new(),
            Some(SharedNetworkBroker::new(NoExternalNetwork(
                requests.clone(),
            ))),
            Some(url::Url::parse("https://image-map.test/").unwrap()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while page
            .evaluate("String(picture.complete && picture.naturalWidth===1)")
            .unwrap()
            != "true"
        {
            assert!(
                std::time::Instant::now() < deadline,
                "native data-image readiness"
            );
            page.drain_tasks().unwrap();
            std::thread::yield_now();
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 0);
        page
    }
    #[test]
    fn area_geometry_reflects_dom_attributes_and_recomputes_native_hits() {
        let mut page = image_page(
            "<style>body{margin:0}img{display:block;width:100px;height:100px}</style><img id=picture usemap=#m src='data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=='><map id=m><area id=first coords='0,0,50,50' href='#first'><area id=rest shape=default href='#rest'></map>",
        );
        assert_eq!(
            page.evaluate("[first.coords,first.shape].join('|')")
                .unwrap(),
            "0,0,50,50|"
        );
        assert_eq!(page.evaluate(r#"(()=>{const area=document.createElement('area');const other=document.implementation.createHTMLDocument();const lone=String.fromCharCode(0xd800);area.coords={toString(){other.adoptNode(area);return lone}};area.shape=lone;return area.ownerDocument===other&&area.coords.charCodeAt(0)===0xd800&&area.getAttribute('coords').charCodeAt(0)===0xd800&&area.shape.charCodeAt(0)===0xd800})()"#).unwrap(), "true");
        page.execute("first.shape='circle';first.coords='75,75,10'")
            .unwrap();
        assert_eq!(page.evaluate("[first.getAttribute('shape'),first.getAttribute('coords'),document.elementFromPoint(75,75).id,document.elementFromPoint(20,20).id].join('|')").unwrap(), "circle|75,75,10|first|rest");
        page.execute("first.setAttribute('coords','20,20,5')")
            .unwrap();
        assert_eq!(
            page.evaluate("[first.coords,document.elementFromPoint(20,20).id].join('|')")
                .unwrap(),
            "20,20,5|first"
        );
        assert_eq!(page.evaluate("(()=>{try{first.coords={toString(){throw Error('conversion')}}}catch(e){return e.message==='conversion'&&first.coords==='20,20,5'}return false})()").unwrap(), "true");
    }

    #[test]
    fn mapped_pointer_uses_area_for_cssom_trusted_click_and_live_shape_changes() {
        let mut page = image_page(
            "<style>body{margin:0}img{display:block;width:100px;height:100px}</style><img id=picture usemap=#m src='data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=='><map id=m><area id=first coords='0,0,50,50' href='#first'><area id=rest shape=default href='#rest'></map>",
        );
        page.execute("globalThis.clicks=[];for(const area of document.querySelectorAll('area'))area.onclick=e=>{e.preventDefault();clicks.push([e.target.id,e.isTrusted,navigator.userActivation.isActive])}").unwrap();
        assert_eq!(
            page.evaluate("document.elementFromPoint(20,20).id")
                .unwrap(),
            "first"
        );
        let root = page
            .automation_realms()
            .into_iter()
            .find(|entry| entry.parent_realm.is_none())
            .unwrap();
        for (action, buttons) in [
            (NativePointerAction::Move, 0),
            (NativePointerAction::Down, 1),
            (NativePointerAction::Up, 0),
        ] {
            page.dispatch_webdriver_pointer_in_context(
                &root,
                &NativePointerInput {
                    x: 20.,
                    y: 20.,
                    button: 0,
                    buttons,
                    ..Default::default()
                },
                action,
            )
            .unwrap();
        }
        assert_eq!(
            page.evaluate("JSON.stringify(clicks)").unwrap(),
            "[[\"first\",true,true]]"
        );
        page.execute("first.coords='60,60,90,90'").unwrap();
        assert_eq!(
            page.evaluate("document.elementFromPoint(20,20).id")
                .unwrap(),
            "rest"
        );
        page.execute("m.id='renamed'").unwrap();
        assert_eq!(
            page.evaluate("document.elementFromPoint(20,20).id")
                .unwrap(),
            "picture"
        );
    }
    #[test]
    fn transformed_image_map_uses_content_coordinates_and_paint_occlusion() {
        let mut page = image_page(
            "<style>body{margin:0}img{display:block;width:100px;height:100px;border:10px solid;padding:10px;transform-origin:0 0;transform:scale(2)}#cover{position:absolute;left:45px;top:45px;width:10px;height:10px}</style><img id=picture usemap=#m src='data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=='><map name=m><area id=region coords='0,0,20,20'></map><div id=cover></div>",
        );
        assert_eq!(
            page.evaluate("document.elementFromPoint(60,60).id")
                .unwrap(),
            "region"
        );
        assert_eq!(
            page.evaluate("document.elementFromPoint(20,20).id")
                .unwrap(),
            "picture"
        );
        assert_eq!(
            page.evaluate("document.elementFromPoint(50,50).id")
                .unwrap(),
            "cover"
        );
    }
}
