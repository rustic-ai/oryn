// Native task delivery retains no author payload outside V8's weak/traced edges.
fn restore_rejection_record(scope: &mut v8::PinScope, record: PromiseRejectionRecord)
    -> Result<(), oryn_js::v8_bindings::V8BindingError>
{
    scope.get_slot_mut::<PromiseRejections>()
        .and_then(|registry| registry.records.as_mut())
        .ok_or(oryn_js::v8_bindings::V8BindingError::Allocation("rejection registry unavailable"))?
        .push(record)
        .map_err(|_| oryn_js::v8_bindings::V8BindingError::Allocation("rejection registry capacity"))
}

fn reject_notification_admission(scope: &mut v8::PinScope, id: u64) {
    if let Some(registry) = scope.get_slot_mut::<PromiseRejections>() {
        registry.admission_failed = true;
        if let Some(records) = &mut registry.records {
            records.retain(|record| record.notification_id != id);
        }
    }
}

fn deliver_promise_rejection_notification(
    scope: &mut v8::PinScope,
    state: &std::rc::Rc<V8BindingState>,
    id: u64,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    use crate::promise_rejection_lifecycle::RejectionPhase;
    let record = scope.get_slot_mut::<PromiseRejections>()
        .and_then(|registry| registry.records.as_mut())
        .and_then(|records| {
            let index = records.as_slice().iter().position(|record| record.notification_id == id)?;
            Some(records.remove(index))
        });
    let Some(mut record) = record else { return Ok(()) };
    let handled = match record.phase {
        RejectionPhase::NotifyQueued => false,
        RejectionPhase::HandledQueued => true,
        _ => { restore_rejection_record(scope, record)?; return Ok(()) }
    };
    let Some(promise) = record.promise.to_local(scope) else { return Ok(()) };
    let Some(context) = promise.get_creation_context(scope) else { return Ok(()) };
    let Some(environment) = context.get_slot::<EnvironmentDescriptor>() else { return Ok(()) };
    let Some(owner) = context.get_slot::<V8BindingState>() else { return Ok(()) };
    if !std::rc::Rc::ptr_eq(&owner, state) || state.execution_retired()
        || record.environment != Some(environment.id)
        || record.authority_epoch != state.document_authority_epoch()
        || !scope.get_slot::<AgentEnvironments>().is_some_and(|registry| registry.is_live(&environment))
        || (!handled && promise.has_handler())
    { return Ok(()) }
    record.phase = RejectionPhase::Dispatching;
    let authority_epoch = record.authority_epoch;
    restore_rejection_record(scope, record)?;
    // Publish Dispatching before author listeners run, without a registry borrow.
    let reason = promise.result(scope);
    let result = oryn_js::v8_bindings::dispatch_promise_rejection_event(scope, state, promise, reason, handled);
    let still_live = !state.execution_retired()
        && state.document_authority_epoch() == authority_epoch
        && context.get_slot::<V8BindingState>().is_some_and(|owner| std::rc::Rc::ptr_eq(&owner, state))
        && scope.get_slot::<AgentEnvironments>().is_some_and(|registry| registry.is_live(&environment));
    if let Some(records) = scope.get_slot_mut::<PromiseRejections>().and_then(|registry| registry.records.as_mut()) {
        if let Some(record) = records.as_mut_slice().iter_mut().find(|record| record.notification_id == id) {
            if handled {
                record.phase = RejectionPhase::Complete;
            } else {
                record.diagnostic_ready = matches!(&result, Ok(true));
                record.phase = if still_live && !promise.has_handler() && !record.handled_during_dispatch {
                    RejectionPhase::Outstanding
                } else { RejectionPhase::Complete };
            }
        }
        records.retain(|record| record.notification_id != id || still_live);
    }
    result.map(|_| ())
}

fn handle_promise_rejection_handler_added(scope: &mut v8::PinScope, promise: v8::Local<v8::Promise>) {
    use crate::promise_rejection_lifecycle::RejectionPhase;
    let queued = scope.get_slot_mut::<PromiseRejections>()
        .and_then(|registry| registry.records.as_mut())
        .and_then(|records| {
            let index = records.as_slice().iter().position(|record| record.promise == promise)?;
            match records.as_slice()[index].phase {
                RejectionPhase::Outstanding => {
                    let record = &mut records.as_mut_slice()[index];
                    record.phase = RejectionPhase::HandledQueued;
                    Some(record.notification_id)
                }
                RejectionPhase::Dispatching => {
                    records.as_mut_slice()[index].handled_during_dispatch = true;
                    None
                }
                RejectionPhase::HandledQueued | RejectionPhase::Complete => None,
                _ => { records.remove(index); None }
            }
        });
    let Some(id) = queued else { return };
    let Some(context) = promise.get_creation_context(scope) else { reject_notification_admission(scope, id); return };
    let Some(state) = context.get_slot::<V8BindingState>() else { reject_notification_admission(scope, id); return };
    let scope = &mut v8::ContextScope::new(scope, context);
    if state.queue_promise_rejection_task(scope, id, deliver_promise_rejection_notification).is_err() {
        reject_notification_admission(scope, id);
    }
}
