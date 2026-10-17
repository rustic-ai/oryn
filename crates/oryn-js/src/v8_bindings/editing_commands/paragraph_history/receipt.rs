use super::*;
// Retained in UndoUnit, independent of any operation-local future cursor.
#[derive(Debug, Clone)]
pub(in crate::v8_bindings) struct FailedParagraphUndo {
    pub(in crate::v8_bindings) segments: Vec<editing_inverse::Segment>,
    pub(in crate::v8_bindings) error: String,
}

pub(in crate::v8_bindings) fn retained_owner(state:&V8BindingState,unit:&UndoUnit)->bool {
    let history=state.undo_history.borrow();
    history.undo.iter().chain(history.redo.iter()).any(|current|Rc::ptr_eq(&current.identity,&unit.identity))
}

// Continuation::current additionally calls this before mutation and after
// preparation author callbacks. Never keep a history/control borrow across await.
pub(in crate::v8_bindings) fn undo_owner_current(state:&V8BindingState,unit:&UndoUnit)->bool {
    state.undo_history.borrow().undo.back().is_some_and(|current|Rc::ptr_eq(&current.identity,&unit.identity))
        && unit.native_progress.borrow().as_ref().is_some_and(|progress|!progress.redo)
        && native_keyboard::document_undo_owner_current(state,&unit.identity,&unit.control_owners,false)
}

pub(in crate::v8_bindings) fn retain_failed(state:&V8BindingState,unit:&UndoUnit,segments:Vec<editing_inverse::Segment>,error:&V8BindingError) {
    if retained_owner(state,unit) {
        unit.paragraph_reservation.release(state,unit);
        let old_bytes=undo_unit_bytes(unit);
        let mut failure=unit.paragraph_failed.borrow_mut();
        if let Some(previous)=failure.as_mut() {
            // On a failed recovery, new segments describe its committed prefix.
            // Append chronologically: reverse recovery first undoes that prefix,
            // then the original failed undo recipe, never dropping either owner.
            previous.segments.extend(segments);
            previous.error=error.to_string();
        } else {
            *failure=Some(FailedParagraphUndo{segments,error:error.to_string()});
        }
        drop(failure);
        let mut history=state.undo_history.borrow_mut();
        history.bytes=history.bytes.checked_sub(old_bytes).and_then(|n|n.checked_add(undo_unit_bytes(unit)))
            .expect("retained paragraph recipe replaces its accounted predecessor");
        // native_progress remains false until Context recovery succeeds.
    }
}

pub(in crate::v8_bindings) fn retire_failed(state:&V8BindingState,unit:&UndoUnit) {
    // Only document retirement may remove an incomplete original operation.
    // A changed author history is not an authorization to clear another unit.
    if state.execution_retired() {unit.paragraph_reservation.release(state,unit);}
    let mut history=state.undo_history.borrow_mut();
    if state.execution_retired() {
        let retained = history.undo.iter().chain(history.redo.iter())
            .any(|entry|Rc::ptr_eq(&entry.identity,&unit.identity));
        if retained {
            let bytes=undo_unit_bytes(unit);
            history.bytes=history.bytes.checked_sub(bytes).expect("retired paragraph owner is accounted");
        }
        history.undo.retain(|entry|!Rc::ptr_eq(&entry.identity,&unit.identity));
        history.redo.retain(|entry|!Rc::ptr_eq(&entry.identity,&unit.identity));
        unit.paragraph_failed.borrow_mut().take();
        unit.native_progress.borrow_mut().take();
    }
}
// Receipt-owned admission. Existing MAX_BYTES is unchanged.
#[derive(Debug, Default)]
pub(in crate::v8_bindings) struct Reservation { pub(in crate::v8_bindings) bytes: Cell<usize> }
impl Reservation {
    pub(in crate::v8_bindings) fn admit(&self,state:&V8BindingState,unit:&UndoUnit,prospective:usize)->Result<(),V8BindingError>{
        if !editing_commands::paragraph_history::receipt::undo_owner_current(state,unit){
            return Err(V8BindingError::Allocation("paragraph recovery history owner"));
        }
        let occupied=state.undo_history.borrow().bytes;
        let old=self.bytes.get();
        let total=occupied.checked_sub(old).and_then(|bytes|bytes.checked_add(prospective))
            .ok_or(V8BindingError::Allocation("paragraph recovery accounting"))?;
        if total>UndoHistory::MAX_BYTES {
            return Err(V8BindingError::Unsupported{interface:"Document".into(),member:"QuotaExceededError".into()});
        }
        self.bytes.set(prospective);
        state.undo_history.borrow_mut().bytes=total;
        Ok(())
    }
    pub(in crate::v8_bindings) fn release(&self,state:&V8BindingState,unit:&UndoUnit){
        let bytes=self.bytes.replace(0);
        if !editing_commands::paragraph_history::receipt::retained_owner(state,unit){return}
        let mut history=state.undo_history.borrow_mut();
        history.bytes=history.bytes.checked_sub(bytes).expect("retained paragraph reservation is included in history bytes");
    }
}
