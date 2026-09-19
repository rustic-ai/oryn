//! One native clipboard default, retained by its host/input receipt.
use super::*;

#[derive(Clone, Copy)]
enum Phase {
    Read,
    ClipboardEvent,
    Default,
    Write,
    BeforeInput,
    Edit,
    Input,
    Done,
}
pub(crate) struct ClipboardJob {
    owner: RcWeak<V8BindingState>,
    grant: NativeClipboardGrant,
    active: bool,
    phase: Phase,
    result: ClipboardActionResult,
    terminal_error: Option<ClipboardError>,
    backend: Option<std::sync::Arc<dyn crate::clipboard::ClipboardBackend>>,
    data: Option<ClipboardData>,
    store: Option<data_transfer::SharedStore>,
    write: Option<ClipboardData>,
    event: Option<ClipboardNotification>,
    rich: Option<retained_editing::Transaction>,
    text: String,
    html: Option<String>,
    prepared_edit: bool,
    is_control: bool,
    input_required: bool,
    control_before: Option<(u32, u32, String)>,
    control_committed: bool,
}
impl ClipboardJob {
    pub(crate) fn new(
        state: &Rc<V8BindingState>,
        mut grant: NativeClipboardGrant,
    ) -> Result<Self, V8BindingError> {
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        if grant.used
            || grant.job.is_some()
            || !valid(state, &grant)
            || (state.clipboard_operation_active.get()
                && state.clipboard_operation_identity.get() == Some(identity))
        {
            return Err(error(ClipboardError::NotAllowed));
        }
        grant.used = true;
        state.clipboard_operation_active.set(true);
        state.clipboard_operation_identity.set(Some(identity));
        Ok(Self {
            owner: Rc::downgrade(state),
            grant,
            active: true,
            phase: Phase::Read,
            result: ClipboardActionResult::default(),
            terminal_error: None,
            backend: None,
            data: None,
            store: None,
            write: None,
            event: None,
            rich: None,
            text: String::new(),
            html: None,
            prepared_edit: false,
            is_control: false,
            input_required: false,
            control_before: None,
            control_committed: false,
        })
    }
    fn release(&mut self) {
        if self.active {
            if let Some(owner) = self.owner.upgrade() {
                if owner.clipboard_operation_identity.get()
                    == Some((
                        self.grant.document,
                        self.grant.generation,
                        self.grant.epoch,
                        self.grant.authority,
                    ))
                {
                    owner.clipboard_operation_active.set(false);
                    owner.clipboard_operation_identity.set(None);
                }
            }
            self.active = false;
        }
    }
    fn finish(&mut self) {
        self.event.take();
        self.rich.take();
        self.phase = Phase::Done;
        self.release();
    }
    pub(crate) fn cancel(&mut self, _state: &V8BindingState) {
        if let Some(event) = &mut self.event {
            event.cancel();
        }
        if let Some(rich) = &mut self.rich {
            rich.cancel();
        }
        self.finish();
    }
    fn paste(&self) -> bool {
        self.grant.action == ClipboardAction::Paste
    }
    fn cut(&self) -> bool {
        self.grant.action == ClipboardAction::Cut
    }
    fn input_type(&self) -> &'static str {
        if self.paste() {
            "insertFromPaste"
        } else {
            "deleteByCut"
        }
    }
    pub(crate) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<ClipboardActionResult>, V8BindingError> {
        if let Some(failure) = &self.terminal_error {
            return Err(error(failure.clone()));
        }
        if matches!(self.phase, Phase::Done) {
            return Ok(Some(self.result.clone()));
        }
        if !self
            .owner
            .upgrade()
            .is_some_and(|owner| Rc::ptr_eq(&owner, state))
            || !valid(state, &self.grant)
        {
            self.terminal_error = Some(ClipboardError::Retired);
            self.cancel(state);
            return Err(error(ClipboardError::Retired));
        }
        loop {
            // A parked document keeps its receipt, but cannot advance either
            // backend operations or native default edits until it resumes.
            if state.execution_suspended() {
                return Ok(None);
            }
            let target = self.grant.target;
            match self.phase {
                Phase::Read => {
                    state.refresh_native_container_policy()?;
                    if self.backend.is_none() {
                        self.backend = Some(
                            state
                                .clipboard_backend
                                .borrow()
                                .clone()
                                .ok_or_else(|| error(ClipboardError::Unavailable))?,
                        );
                    }
                    if self.data.is_none() {
                        let data = if self.paste() {
                            self.backend
                                .as_ref()
                                .unwrap()
                                .read_for(&creator(state)?)
                                .map_err(error)?
                        } else {
                            ClipboardData::default()
                        };
                        data.validate().map_err(error)?;
                        self.data = Some(data);
                    }
                    self.store = Some(Rc::new(RefCell::new(from_data(
                        self.data.as_ref().unwrap(),
                    )?)));
                    self.phase = Phase::ClipboardEvent;
                }
                Phase::ClipboardEvent => {
                    if self.event.is_none() {
                        let name = if self.paste() {
                            "paste"
                        } else if self.cut() {
                            "cut"
                        } else {
                            "copy"
                        };
                        self.event = Some(ClipboardNotification::clipboard(
                            scope,
                            state,
                            target,
                            name,
                            self.store.as_ref().unwrap().clone(),
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, state)? else {
                        return Ok(None);
                    };
                    self.result.event_cancelled = !accepted;
                    self.event.take();
                    self.phase = Phase::Default;
                }
                Phase::Default => {
                    if !valid(state, &self.grant) {
                        self.terminal_error = Some(ClipboardError::Retired);
                        self.cancel(state);
                        return Err(error(ClipboardError::Retired));
                    }
                    if !self.paste() {
                        if self.result.event_cancelled {
                            self.write = Some(to_data(&self.store.as_ref().unwrap().borrow())?);
                        } else {
                            if self.cut() && !editable(state, target)? {
                                self.finish();
                                continue;
                            }
                            self.write = selected_data(state, target)?;
                            if self.write.is_none() {
                                self.finish();
                                continue;
                            }
                        }
                        self.phase = Phase::Write;
                    } else {
                        if self.result.event_cancelled || !editable(state, target)? {
                            self.finish();
                            continue;
                        }
                        self.is_control = control(state, target);
                        let data = self.data.as_ref().unwrap();
                        if !data.items.iter().any(|item| {
                            matches!(item.payload, ClipboardPayload::Text(_))
                                && (item.media_type == "text/plain"
                                    || (!self.is_control && item.media_type == "text/html"))
                        }) {
                            self.finish();
                            continue;
                        }
                        self.text = oryn_dom::dom_string::from_units(data.plain_text());
                        self.phase = Phase::BeforeInput;
                    }
                }
                Phase::Write => {
                    // The backend has no idempotency token: an error may mean
                    // the write committed but its acknowledgement was lost.
                    // Preserve that failure instead of overwriting a later
                    // clipboard owner on a host receipt retry. Cut must not
                    // delete its selection without a successful write result.
                    if let Err(failure) = self
                        .backend
                        .as_ref()
                        .unwrap()
                        .write_for(&creator(state)?, self.write.as_ref().unwrap().clone())
                    {
                        self.terminal_error = Some(failure.clone());
                        self.write.take();
                        self.finish();
                        return Err(error(failure));
                    }
                    self.result.clipboard_changed = true;
                    self.write.take();
                    if !self.cut() || self.result.event_cancelled {
                        self.finish();
                        continue;
                    }
                    self.is_control = control(state, target);
                    self.phase = Phase::BeforeInput;
                }
                Phase::BeforeInput => {
                    if self.event.is_none() {
                        self.event = Some(ClipboardNotification::editing(
                            scope,
                            state,
                            target,
                            "beforeinput",
                            self.input_type(),
                            (self.paste() && self.is_control).then_some(self.text.as_str()),
                            if self.paste() {
                                self.data.as_ref()
                            } else {
                                None
                            },
                        )?);
                    }
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, state)? else {
                        return Ok(None);
                    };
                    self.event.take();
                    if !accepted || !valid(state, &self.grant) || !editable(state, target)? {
                        self.finish();
                        continue;
                    }
                    self.phase = Phase::Edit;
                }
                Phase::Edit => {
                    if self.is_control {
                        if self.control_before.is_none() {
                            self.control_before = Some(control_selection(state, target)?);
                        }
                        let (start, end, before) = self.control_before.as_ref().unwrap();
                        if !self.control_committed {
                            edit_native_control(
                                state,
                                target,
                                *start,
                                *end,
                                if self.paste() { &self.text } else { "" },
                            )?;
                            self.control_committed = true;
                        }
                        self.result.content_changed =
                            *before != state.dom.borrow().control_edit_value(target)?;
                        self.input_required = self.result.content_changed
                            || (self.paste() && (*start != *end || !self.text.is_empty()));
                    } else {
                        if !self.prepared_edit {
                            if self.paste() {
                                let html =
                                    self.data.as_ref().unwrap().items.iter().find_map(|item| {
                                        if item.media_type == "text/html" {
                                            if let ClipboardPayload::Text(value) = &item.payload {
                                                Some(oryn_dom::dom_string::from_units(value))
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    });
                                let plaintext = editing_host(state)?.is_some_and(|host| {
                                    host_is_plaintext_only(&state.dom.borrow(), host)
                                });
                                self.html = if plaintext {
                                    None
                                } else {
                                    html.map(|html| sanitized_html_for_state(state, &html))
                                        .transpose()?
                                };
                            }
                            self.rich = Some(retained_editing::Transaction::new(
                                state,
                                self.input_type(),
                            )?);
                            self.prepared_edit = true;
                        }
                        let paste = self.paste();
                        let html = self.html.clone();
                        let text = self.text.clone();
                        let command = if !paste {
                            editing_commands::Command::ReplaceSelection(String::new())
                        } else if let Some(html) = html.filter(|html| !html.is_empty()) {
                            editing_commands::Command::Html(html)
                        } else {
                            editing_commands::Command::Text(text)
                        };
                        let Some(changed) = self
                            .rich
                            .as_mut()
                            .unwrap()
                            .poll_owned(scope, state, command)?
                        else {
                            return Ok(None);
                        };
                        self.result.content_changed = changed;
                        self.input_required = self.result.content_changed;
                    }
                    self.phase = Phase::Input;
                }
                Phase::Input => {
                    if !self.input_required || !valid(state, &self.grant) {
                        self.finish();
                        continue;
                    }
                    if self.event.is_none() {
                        self.event = Some(ClipboardNotification::editing(
                            scope,
                            state,
                            target,
                            "input",
                            self.input_type(),
                            (self.paste() && self.is_control).then_some(self.text.as_str()),
                            if self.paste() {
                                self.data.as_ref()
                            } else {
                                None
                            },
                        )?);
                    }
                    if self.event.as_mut().unwrap().poll(scope, state)?.is_none() {
                        return Ok(None);
                    }
                    self.finish();
                }
                Phase::Done => return Ok(Some(self.result.clone())),
            }
        }
    }
}
impl Drop for ClipboardJob {
    fn drop(&mut self) {
        if let Some(event) = &mut self.event {
            event.cancel();
        }
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Memory {
        data: Mutex<ClipboardData>,
        reads: Mutex<usize>,
        writes: Mutex<usize>,
    }
    impl crate::clipboard::ClipboardBackend for Memory {
        fn read(&self) -> Result<ClipboardData, ClipboardError> {
            *self.reads.lock().unwrap() += 1;
            Ok(self.data.lock().unwrap().clone())
        }
        fn write(&self, data: ClipboardData) -> Result<(), ClipboardError> {
            *self.writes.lock().unwrap() += 1;
            *self.data.lock().unwrap() = data;
            Ok(())
        }
    }
    fn eval(scope: &mut v8::PinScope, source: &str) {
        super::super::super::test_eval::eval(scope, source);
    }
    #[test]
    fn suspended_clipboard_receipt_keeps_payload_without_advancing() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(217), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let backend = Arc::new(Memory::default());
        *backend.data.lock().unwrap() = ClipboardData::text(vec![88]);
        *state.clipboard_backend.borrow_mut() = Some(backend.clone());
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();t.setSelectionRange(3,3);globalThis.trace=[];t.addEventListener('paste',()=>trace.push('first'),{once:true});t.addEventListener('paste',()=>trace.push('second'));",
        );
        let token = grant(&state, target, ClipboardAction::Paste).unwrap();
        let mut job = ClipboardJob::new(&state, token).unwrap();
        state.set_execution_suspended(true);
        assert!(job.poll(scope, &state).unwrap().is_none());
        assert_eq!(*backend.reads.lock().unwrap(), 0);
        state.set_execution_suspended(false);
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope, &state).is_err());
        assert_eq!(*backend.reads.lock().unwrap(), 1);
        state.set_execution_suspended(true);
        *backend.data.lock().unwrap() = ClipboardData::text(vec![89]);
        for _ in 0..2 {
            assert!(job.poll(scope, &state).unwrap().is_none());
        }
        assert_eq!(*backend.reads.lock().unwrap(), 1);
        assert_eq!(
            state.dom.borrow().control_edit_value(target).unwrap(),
            "abc"
        );
        assert!(state.clipboard_operation_active.get());
        state.set_execution_suspended(false);
        assert!(job.poll(scope, &state).unwrap().unwrap().content_changed);
        eval(
            scope,
            "if(t.value!=='abcX'||trace.join(',')!=='first,second')throw Error('parked receipt replayed or lost payload');",
        );
        assert!(!state.clipboard_operation_active.get());
    }
    #[test]
    fn lost_write_reply_is_terminal_and_does_not_delete_cut_selection() {
        struct LostReply(Memory);
        impl crate::clipboard::ClipboardBackend for LostReply {
            fn read(&self) -> Result<ClipboardData, ClipboardError> {
                crate::clipboard::ClipboardBackend::read(&self.0)
            }
            fn write(&self, data: ClipboardData) -> Result<(), ClipboardError> {
                crate::clipboard::ClipboardBackend::write(&self.0, data)?;
                Err(ClipboardError::Unavailable)
            }
        }
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(215), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let backend = Arc::new(LostReply(Memory::default()));
        *state.clipboard_backend.borrow_mut() = Some(backend.clone());
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();t.setSelectionRange(0,2);globalThis.trace=[];for(const name of ['cut','beforeinput','input'])t.addEventListener(name,e=>trace.push(e.type));",
        );
        let token = grant(&state, target, ClipboardAction::Cut).unwrap();
        let mut cut = ClipboardJob::new(&state, token).unwrap();
        assert!(cut.poll(scope, &state).is_err());
        assert!(!state.clipboard_operation_active.get());
        assert_eq!(backend.0.data.lock().unwrap().plain_text(), &[97, 98]);
        *backend.0.data.lock().unwrap() = ClipboardData::text(vec![90]);
        for _ in 0..3 {
            assert!(cut.poll(scope, &state).is_err());
        }
        assert_eq!(*backend.0.writes.lock().unwrap(), 1);
        assert_eq!(backend.0.data.lock().unwrap().plain_text(), &[90]);
        eval(
            scope,
            "if(t.value!=='abc'||trace.join(',')!=='cut')throw Error('failed cut replayed or deleted content');",
        );
        let token = grant(&state, target, ClipboardAction::Copy).unwrap();
        let mut next = ClipboardJob::new(&state, token).unwrap();
        next.cancel(&state);
    }
    #[test]
    fn interrupted_paste_and_cut_keep_each_event_write_and_edit_once() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(214), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let backend = Arc::new(Memory::default());
        *backend.data.lock().unwrap() = ClipboardData::text(vec![88]);
        *state.clipboard_backend.borrow_mut() = Some(backend.clone());
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();t.setSelectionRange(3,3);globalThis.trace=[];for(const name of ['paste','cut','beforeinput','input'])t.addEventListener(name,e=>trace.push(e.type));",
        );
        let token = grant(&state, target, ClipboardAction::Paste).unwrap();
        let mut paste = ClipboardJob::new(&state, token).unwrap();
        for index in 0..3 {
            native_platform::publication::fail_after_next_callback();
            assert!(paste.poll(scope, &state).is_err());
            assert_eq!(*backend.reads.lock().unwrap(), 1);
            assert_eq!(*backend.writes.lock().unwrap(), 0);
            assert!(state.clipboard_operation_active.get());
            if index == 0 {
                *backend.data.lock().unwrap() = ClipboardData::text(vec![89]);
                let nested = grant(&state, target, ClipboardAction::Copy).unwrap();
                assert!(ClipboardJob::new(&state, nested).is_err());
            }
        }
        let result = paste.poll(scope, &state).unwrap().unwrap();
        assert!(result.content_changed);
        assert!(!result.clipboard_changed);
        assert!(!state.clipboard_operation_active.get());
        eval(
            scope,
            "if(t.value!=='abcX'||trace.join(',')!=='paste,beforeinput,input')throw Error('paste replayed or reread clipboard');t.value='abc';t.setSelectionRange(0,2);trace=[];",
        );
        let token = grant(&state, target, ClipboardAction::Cut).unwrap();
        let mut cut = ClipboardJob::new(&state, token).unwrap();
        for index in 0..3 {
            native_platform::publication::fail_after_next_callback();
            let failure = cut.poll(scope, &state).unwrap_err();
            assert!(
                failure
                    .to_string()
                    .contains("injected native notification publication failure"),
                "cut phase {index}: {failure:?}"
            );
            assert_eq!(
                *backend.writes.lock().unwrap(),
                usize::from(index > 0),
                "cut phase {index}: {failure:?}"
            );
        }
        let result = cut.poll(scope, &state).unwrap().unwrap();
        assert!(result.clipboard_changed && result.content_changed);
        assert_eq!(*backend.writes.lock().unwrap(), 1);
        assert_eq!(backend.data.lock().unwrap().plain_text(), &[97, 98]);
        eval(
            scope,
            "if(t.value!=='c'||trace.join(',')!=='cut,beforeinput,input')throw Error('cut replayed an event or mutation');",
        );
        assert!(cut.poll(scope, &state).unwrap().unwrap().content_changed);
        assert_eq!(*backend.writes.lock().unwrap(), 1);
        let token = grant(&state, target, ClipboardAction::Paste).unwrap();
        let mut canceled = ClipboardJob::new(&state, token).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(canceled.poll(scope, &state).is_err());
        canceled.cancel(&state);
        assert!(!state.clipboard_operation_active.get());
        assert!(
            !canceled
                .poll(scope, &state)
                .unwrap()
                .unwrap()
                .content_changed
        );
        assert!(
            state
                .events
                .borrow()
                .values()
                .all(|event| !event.dispatching)
        );
        let token = grant(&state, target, ClipboardAction::Paste).unwrap();
        let mut old = ClipboardJob::new(&state, token).unwrap();
        eval(
            scope,
            "document.open();document.write('<input id=next>');document.close();",
        );
        let next = state.dom.borrow().find_by_id("next").unwrap();
        let token = grant(&state, next, ClipboardAction::Paste).unwrap();
        let mut replacement = ClipboardJob::new(&state, token).unwrap();
        old.cancel(&state);
        assert!(state.clipboard_operation_active.get());
        replacement.cancel(&state);
        assert!(!state.clipboard_operation_active.get());
    }
}
