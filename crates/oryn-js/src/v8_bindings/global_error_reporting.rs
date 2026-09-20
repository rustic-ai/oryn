//! Exact ErrorEvent delivery custody shared by scripts and module reactions.
use super::*;
use std::collections::VecDeque;
#[derive(Default)]
pub(in super::super) struct Registry {
    queue: VecDeque<Report>,
    retry: Option<std::time::Instant>,
    polling: bool,
    overload: Option<Overload>,
    locations: Vec<(v8::Global<v8::Value>, String, u32, u32)>,
}
// One bounded provenance record covers rejected admissions; it is never dispatched.
struct Overload {
    count: u64,
    detail: String,
    _exception: v8::Global<v8::Value>,
    _message: v8::Global<v8::Message>,
}
impl Registry {
    pub(in super::super) fn take_overload_diagnostic(&mut self) -> Option<String> {
        self.overload.take().map(|item| {
            format!(
                "ErrorEvent admission rejected for {} report(s): {}",
                item.count, item.detail
            )
        })
    }
    fn reject(
        &mut self,
        scope: &mut v8::PinScope,
        exception: v8::Local<v8::Value>,
        message: Option<v8::Local<v8::Message>>,
        muted: bool,
    ) {
        if let Some(item) = &mut self.overload {
            item.count = item.count.saturating_add(1);
            return;
        }
        let message = message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
        let detail = if muted {
            "Script error.".to_owned()
        } else {
            message.get(scope).to_rust_string_lossy(scope)
        };
        self.overload = Some(Overload {
            count: 1,
            detail,
            _exception: v8::Global::new(scope, exception),
            _message: v8::Global::new(scope, message),
        });
    }
}
struct Report {
    notification: Option<native_platform::publication::Notification>,
    event: Option<v8::Global<v8::Object>>,
    exception: v8::Global<v8::Value>,
    message: v8::Global<v8::Message>,
    muted: bool,
    generation: u64,
    epoch: u64,
    delivered: Option<bool>,
    detail: String,
}
impl Report {
    fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        exception: v8::Local<v8::Value>,
        message: Option<v8::Local<v8::Message>>,
        muted: bool,
    ) -> Result<Self, V8BindingError> {
        let message = message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
        let text = message.get(scope);
        let filename = message
            .get_script_resource_name(scope)
            .filter(|v| !v.is_undefined())
            .and_then(|v| v.to_string(scope))
            .unwrap_or_else(|| v8::String::new(scope, "").unwrap());
        let line = message.get_line_number(scope).unwrap_or(0) as u32;
        let column = if line == 0 {
            0
        } else {
            message.get_start_column() as u32 + 1
        };
        let native_location = state
            .error_reports
            .borrow()
            .locations
            .iter()
            .find(|(error, _, _, _)| error == &exception)
            .map(|(_, url, line, column)| (url.clone(), *line, *column));
        let (filename, line, column) = match native_location {
            Some((url, line, column)) => (
                v8::String::new(scope, &url)
                    .ok_or(V8BindingError::Allocation("exception source URL"))?,
                line,
                column,
            ),
            None => (filename, line, column),
        };
        let (text, filename, line, column, exception) = if muted {
            (
                v8::String::new(scope, "Script error.").unwrap(),
                v8::String::new(scope, "").unwrap(),
                0,
                0,
                v8::null(scope).into(),
            )
        } else {
            (text, filename, line, column, exception)
        };
        let event =
            create_event_object_with_interface(scope, state, "ErrorEvent", "error", false, true)?;
        let line = v8::Integer::new_from_unsigned(scope, line);
        let column = v8::Integer::new_from_unsigned(scope, column);
        let data = v8::Array::new_with_elements(
            scope,
            &[
                text.into(),
                filename.into(),
                line.into(),
                column.into(),
                exception,
            ],
        );
        let brand = {
            let mut brand = state.error_event_brand.borrow_mut();
            if brand.is_none() {
                let symbol = v8::Symbol::new(scope, None);
                *brand = Some(v8::Global::new(scope, symbol));
            }
            v8::Local::new(scope, brand.as_ref().unwrap())
        };
        event.set_internal_field(2, data.into());
        event.set_internal_field(3, brand.into());
        let id = event_id(scope, event)?;
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;

        let detail = text.to_rust_string_lossy(scope);
        let notification = native_platform::publication::pending_platform_dispatch(
            scope,
            state,
            GLOBAL_TARGET,
            "error",
            event,
        );
        Ok(Self {
            notification: Some(notification),
            event: Some(v8::Global::new(scope, event)),
            exception: v8::Global::new(scope, exception),
            message: v8::Global::new(scope, message),
            muted,
            generation: state.document_generation(),
            epoch: state.document_authority_epoch(),
            delivered: None,
            detail,
        })
    }
    fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<bool>, V8BindingError> {
        if state.execution_retired()
            || state.document_generation() != self.generation
            || state.document_authority_epoch() != self.epoch
        {
            if let Some(notification) = &self.notification {
                notification.cancel(state);
            }
            return Ok(Some(true));
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        let previous = state.reporting_exception.replace(true);
        struct Guard<'a>(&'a Cell<bool>, bool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _guard = Guard(&state.reporting_exception, previous);
        if self.notification.is_none() {
            let exception = v8::Local::new(scope, &self.exception);
            let message = v8::Local::new(scope, &self.message);
            let prepared = Self::new(scope, state, exception, Some(message), self.muted)?;
            self.notification = prepared.notification;
            self.event = prepared.event;
            self.detail = prepared.detail;
        }
        if self.delivered.is_none() {
            if !self.notification.as_mut().unwrap().resume(scope, state)? {
                return Ok(None);
            }
            if state.execution_retired()
                || state.document_generation() != self.generation
                || state.document_authority_epoch() != self.epoch
            {
                self.notification.as_ref().unwrap().cancel(state);
                return Ok(Some(true));
            }
            let event = v8::Local::new(scope, self.event.as_ref().unwrap());
            let id = event_id(scope, event)?;
            self.delivered = Some(
                state
                    .events
                    .borrow()
                    .get(&id)
                    .is_some_and(|event| event.default_prevented),
            );
            self.notification.as_ref().unwrap().finish(scope, state);
        }
        let cancelled = self.delivered.unwrap();
        if !cancelled {
            let exception = v8::Local::new(scope, &self.exception);
            let message = v8::Local::new(scope, &self.message);
            forward_worker_error(scope, state, exception, Some(message), self.muted)?;
        }
        Ok(Some(cancelled))
    }
}
pub(super) fn report(
    scope: &mut v8::PinScope,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
    muted: bool,
) -> Result<bool, V8BindingError> {
    let state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("exception realm"))?;
    if state.reporting_exception.get() {
        forward_worker_error(scope, &state, exception, message, muted)?;
        return Ok(false);
    }
    {
        let mut registry = state.error_reports.borrow_mut();
        if registry.queue.len() >= 128 {
            registry.reject(scope, exception, message, muted);
            return Err(V8BindingError::Allocation("script error report capacity"));
        }
        if registry.queue.try_reserve(1).is_err() {
            registry.reject(scope, exception, message, muted);
            return Err(V8BindingError::Allocation("script error report queue"));
        }
    }
    let message = message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
    let mut report = Report {
        notification: None,
        event: None,
        exception: v8::Global::new(scope, exception),
        message: v8::Global::new(scope, message),
        muted,
        generation: state.document_generation(),
        epoch: state.document_authority_epoch(),
        delivered: None,
        detail: String::new(),
    };
    match report.poll(scope, &state) {
        Ok(Some(cancelled)) => Ok(cancelled),
        pending => {
            let mut registry = state.error_reports.borrow_mut();
            registry.queue.push_back(report);
            registry.retry = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            pending.map(|_| true)
        }
    }
}
pub(in super::super) fn set_location(
    scope: &mut v8::PinScope,
    exception: v8::Local<v8::Value>,
    url: &str,
    line: u32,
    column: u32,
) -> Result<(), V8BindingError> {
    let state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("exception location realm"))?;
    if url.len() > 192 * 1024 {
        return Err(V8BindingError::Allocation("exception source URL capacity"));
    }
    let mut registry = state.error_reports.borrow_mut();
    if let Some(index) = registry
        .locations
        .iter()
        .position(|(error, _, _, _)| error == &exception)
    {
        let (_, name, l, c) = &mut registry.locations[index];
        *name = url.to_owned();
        *l = line;
        *c = column;
        return Ok(());
    }
    if registry.locations.len() >= 256 || url.len() > 192 * 1024 {
        return Err(V8BindingError::Allocation("exception location capacity"));
    }
    registry
        .locations
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("exception location records"))?;
    registry.locations.push((
        v8::Global::new(scope, exception),
        url.to_owned(),
        line,
        column,
    ));
    Ok(())
}
#[cfg(test)]
pub(in super::super) fn retry_now(state: &V8BindingState) {
    state.error_reports.borrow_mut().retry = None;
}
pub(in super::super) fn has_pending(state: &V8BindingState) -> bool {
    !state.error_reports.borrow().queue.is_empty()
}
pub(in super::super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired()
        || state.execution_suspended()
        || state.error_reports.borrow().queue.is_empty()
    {
        return None;
    }
    Some(
        state
            .error_reports
            .borrow()
            .retry
            .map(|retry| retry.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or_default(),
    )
}
pub(in super::super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if !delay(state).is_some_and(|delay| delay.is_zero()) || state.error_reports.borrow().polling {
        return Ok(0);
    }
    state.error_reports.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.error_reports.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(state);
    let mut count = 0;
    for _ in 0..32 {
        let Some(mut report) = state.error_reports.borrow_mut().queue.pop_front() else {
            break;
        };
        match report.poll(scope, state) {
            Ok(Some(cancelled)) => {
                if !cancelled && state.script_error_diagnostics.borrow().len() < 128 {
                    state
                        .script_error_diagnostics
                        .borrow_mut()
                        .push_back(report.detail);
                }
                count += 1;
            }
            pending => {
                let mut registry = state.error_reports.borrow_mut();
                registry.queue.push_front(report);
                registry.retry =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
                return pending.map(|_| count);
            }
        }
    }
    state.error_reports.borrow_mut().retry = None;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) {
        let code = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn error_report_retains_listener_suffix_and_legacy_handler_arguments() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(289), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];addEventListener('error',e=>{globalThis.originalEvent=e;trace.push('first')},{once:true});addEventListener('error',e=>trace.push(e===originalEvent?'tail':'wrong'));onerror=function(message,url,line,column,error){trace.push(arguments.length===5&&error===originalEvent.error?'handler':'bad-args');return true;}",
        );
        let text = v8::String::new(scope, "original failure").unwrap();
        let error = v8::Exception::syntax_error(scope, text);
        native_platform::publication::fail_after_next_callback();
        assert!(report(scope, error, None, false).is_err());
        eval(
            scope,
            "if(trace.join()!=='first')throw Error('report did not pause')",
        );
        state.error_reports.borrow_mut().retry = None;
        assert_eq!(poll(scope, &state).unwrap(), 1);
        eval(
            scope,
            "if(trace.join()!=='first,tail,handler')throw Error('report suffix/handler replay')",
        );
        assert!(state.error_reports.borrow().queue.is_empty());
        assert!(state.script_error_diagnostics.borrow().is_empty());
    }
    #[test]
    fn suspended_error_report_keeps_original_error_without_runnable_spin() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(290), 1).unwrap();
        eval(
            scope,
            "globalThis.count=0;onerror=()=>{count++;return false}",
        );
        state.set_execution_suspended(true);
        let text = v8::String::new(scope, "parked failure").unwrap();
        let error = v8::Exception::type_error(scope, text);
        assert!(report(scope, error, None, false).unwrap());
        assert!(delay(&state).is_none());
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.set_execution_suspended(false);
        state.error_reports.borrow_mut().retry = None;
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert_eq!(state.script_error_diagnostics.borrow().len(), 1);
        eval(scope, "if(count!==1)throw Error('parked report replay')");
    }
    #[test]
    fn saturated_reports_reject_without_dispatch_and_recover_capacity() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(291), 1).unwrap();
        eval(
            scope,
            "globalThis.count=0;onerror=()=>{count++;return true}",
        );
        state.set_execution_suspended(true);
        let text = v8::String::new(scope, "first overload provenance").unwrap();
        let error = v8::Exception::syntax_error(scope, text);
        for _ in 0..128 {
            assert!(report(scope, error, None, false).unwrap());
        }
        for _ in 0..3 {
            assert!(report(scope, error, None, false).is_err());
        }
        assert_eq!(state.error_reports.borrow().queue.len(), 128);
        eval(
            scope,
            "if(count!==0)throw Error('rejected admission dispatched')",
        );
        let diagnostics = state.take_script_error_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("3 report(s)"));
        assert!(diagnostics[0].contains("first overload provenance"));
        assert!(state.take_script_error_diagnostics().is_empty());
        state.set_execution_suspended(false);
        state.error_reports.borrow_mut().retry = None;
        for _ in 0..4 {
            assert_eq!(poll(scope, &state).unwrap(), 32);
        }
        assert!(report(scope, error, None, false).unwrap());
        eval(
            scope,
            "if(count!==129)throw Error('admitted report lost or replayed')",
        );
        state.set_execution_suspended(true);
        for _ in 0..128 {
            assert!(report(scope, error, None, false).unwrap());
        }
        assert!(report(scope, error, None, false).is_err());
        state.retire_execution();
        assert!(state.error_reports.borrow().queue.is_empty());
        assert!(state.error_reports.borrow().overload.is_none());
    }
}
