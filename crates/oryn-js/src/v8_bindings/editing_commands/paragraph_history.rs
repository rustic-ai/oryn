// Projected child of editing_commands. Uses the existing continuation queue.
use super::*;
pub(in crate::v8_bindings) mod receipt;

pub(in crate::v8_bindings) fn run(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<Option<bool>, V8BindingError> {
    resume_editing_recovery(state)?;
    let unit = state.undo_history.borrow().undo.back().cloned();
    let Some(unit) = unit else { return Ok(None) };
    let Some(split) = unit.paragraph_split.clone() else { return Ok(None) };
    let failed = unit.paragraph_failed.borrow().clone();
    if unit.native_progress.borrow().is_some() && failed.is_none() {
        // The original queue job owns the exact unperformed suffix. Repeated
        // commands cannot create a second future, or change its direction.
        return Err(V8BindingError::Allocation("native editing suspended"));
    }
    if unit.before.document_identity != (state.document_root(), state.document_generation(), state.document_stream_epoch())
        || !state.dom.borrow().is_document_connected(unit.before.host)?
        || (failed.is_some() && !receipt::undo_owner_current(state,&unit))
        || (failed.is_none() && !native_keyboard::can_document_undo(state, &unit.identity, &unit.control_owners, false)?)
    {
        return Ok(Some(false));
    }
    let registry = state.editing_continuations.upgrade()
        .ok_or(V8BindingError::Allocation("native editing registry"))?;
    {
        let mut queue = registry.borrow_mut();
        if queue.queue.len() >= 32 {
            return Err(V8BindingError::Allocation("native editing admission capacity"));
        }
        queue.queue.try_reserve(1).map_err(|_| V8BindingError::Allocation("native editing queue"))?;
    }
    let selection = unit.before.selection;
    let recovering = failed.is_some();
    let mut job = editing_continuation::Continuation::new_at(
        state_rc(scope, state)?, Some(unit.before.host), "historyUndo".into(),
        move |context| Box::pin(async move {
            if let Some(failed) = failed {
                for segment in failed.segments.iter().rev() {
                    segment.recover_paragraph(&context).await?;
                }
            } else {
                split.undo(&context, selection).await?;
            }
            Ok(())
        }),
    )?.ok_or(V8BindingError::Allocation("paragraph undo host"))?;
    let starting=!recovering;
    if starting {*unit.native_progress.borrow_mut() = Some(NativeUndoProgress { redo: false, next: 0 });}
    if let Err(error)=job.retain_history_undo(unit.clone(), recovering) {
        if starting {unit.paragraph_reservation.release(state,&unit);unit.native_progress.borrow_mut().take();}
        return Err(error);
    }
    match job.poll(scope) {
        Ok(Some(changed)) => Ok(Some(changed)),
        result => {
            registry.borrow_mut().queue.push_back(Queued {
                owner: state.instance_id, job,
                retry: std::time::Instant::now() + std::time::Duration::from_millis(10),
            });
            match result {
                Err(error) => Err(error),
                _ => Err(V8BindingError::Allocation("native editing suspended")),
            }
        }
    }
}

pub(in crate::v8_bindings) fn dispatch(scope: &mut v8::PinScope, state: &V8BindingState, redo: bool) -> Result<bool, V8BindingError> {
    if !redo && let Some(result) = run(scope, state)? {
        return Ok(result);
    }
    undo_edit(state, redo)
}

#[cfg(test)]
pub(in crate::v8_bindings) fn ready_for_test(state: &V8BindingState) {
    let registry = state.editing_continuations.upgrade().unwrap();
    for queued in registry.borrow_mut().queue.iter_mut() {
        if queued.owner == state.instance_id { queued.retry = std::time::Instant::now(); }
    }
}

pub(in crate::v8_bindings) fn commit(state: &V8BindingState, unit: &UndoUnit,
    segments: Vec<editing_inverse::Segment>) -> Result<(), V8BindingError> {
    if !native_keyboard::document_undo_owner_current(state,&unit.identity,&unit.control_owners,false) {
        return Err(V8BindingError::Allocation("paragraph undo history owner"));
    }
    let mut history = state.undo_history.borrow_mut();
    let owned = history.undo.back().is_some_and(|current| Rc::ptr_eq(&current.identity, &unit.identity));
    if !owned || unit.native_progress.borrow().as_ref().is_none_or(|progress| progress.redo) {
        return Err(V8BindingError::Allocation("paragraph undo history owner"));
    }
    // This recipe records the actual moved topology at each observed mutation,
    // not the original Enter parents. Redo restores it in reverse direction.
    history.bytes=history.bytes.checked_sub(unit.paragraph_reservation.bytes.replace(0)).expect("paragraph reserve is accounted");
    let old_bytes=undo_unit_bytes(unit);
    *unit.paragraph_redo.borrow_mut() = segments;
    history.bytes=history.bytes.checked_sub(old_bytes).and_then(|n|n.checked_add(undo_unit_bytes(unit))).expect("paragraph redo replaces accounted recipe");
    unit.native_progress.borrow_mut().take();
    native_keyboard::finish_document_undo(state, &unit.control_owners, false);
    let committed = history.undo.pop_back().expect("checked paragraph undo owner");
    history.redo.push_back(committed);
    Ok(())
}

pub(in crate::v8_bindings) fn recovered(state:&V8BindingState,unit:&UndoUnit)->Result<(),V8BindingError> {
    if !receipt::undo_owner_current(state,unit) {
        return Err(V8BindingError::Allocation("paragraph recovery history owner"));
    }
    unit.paragraph_reservation.release(state,unit);
    let old_bytes=undo_unit_bytes(unit);
    unit.paragraph_failed.borrow_mut().take();
    let mut history=state.undo_history.borrow_mut();
    history.bytes=history.bytes.checked_sub(old_bytes).and_then(|n|n.checked_add(undo_unit_bytes(unit))).expect("paragraph recovery releases accounted recipe");
    unit.native_progress.borrow_mut().take();
    // Recovery returns the original unit to its pre-attempt topology. It does
    // not claim the requested undo complete; the next invocation starts fresh.
    Ok(())
}
