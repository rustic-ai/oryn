//! Paired native undo ownership for same-origin cross-document drag moves.
use super::*;
pub(super) struct Bridge {
    left: std::rc::Weak<V8BindingState>,
    right: std::rc::Weak<V8BindingState>,
    left_identity: (NodeId, u64, u64, u64),
    right_identity: (NodeId, u64, u64, u64),
    identity: Rc<()>,
    progress: RefCell<Option<NativeUndoProgress>>,
}
impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrossDocumentUndo")
            .field("left", &self.left_identity)
            .field("right", &self.right_identity)
            .finish()
    }
}
fn identity(state: &V8BindingState) -> (NodeId, u64, u64, u64) {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}
fn changes(before: &EditingSnapshot, after: &EditingSnapshot) -> Vec<ControlEdit> {
    before
        .controls
        .iter()
        .filter_map(|before| {
            after
                .controls
                .iter()
                .find(|after| before.node == after.node && before.value != after.value)
                .map(|after| {
                    (
                        before.node,
                        before.value.clone(),
                        before.selection.clone(),
                        after.value.clone(),
                        after.selection.clone(),
                    )
                })
        })
        .collect()
}
fn segment_changes(segments: &[editing_inverse::Segment]) -> Vec<ControlEdit> {
    let mut result = BTreeMap::new();
    for segment in segments {
        for change in changes(&segment.before, &segment.after) {
            let entry = result.entry(change.0).or_insert(change.clone());
            entry.3 = change.3;
            entry.4 = change.4;
        }
    }
    result.into_values().collect()
}
fn trim(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut stale = Vec::new();
    {
        let mut history = state.undo_history.borrow_mut();
        while let Some(unit) = history.redo.pop_front() {
            history.bytes = history.bytes.saturating_sub(undo_unit_bytes(&unit));
            stale.push(unit);
        }
        while history.undo.len() > UndoHistory::MAX_UNITS || history.bytes > UndoHistory::MAX_BYTES
        {
            let Some(unit) = history.undo.pop_front() else {
                break;
            };
            history.bytes = history.bytes.saturating_sub(undo_unit_bytes(&unit));
            stale.push(unit);
        }
    }
    drop(stale);
    Ok(())
}
/// Owns both native mutation halves until completion and paired history publication.
/// Only the native-only mutation closure may roll back; author completion is resumed.
pub(super) struct Transaction {
    jobs: [Option<editing_continuation::Continuation>; 2],
    native_segments: [Vec<editing_inverse::Segment>; 2],
    right_baselined: bool,
    left: Rc<V8BindingState>,
    right: Rc<V8BindingState>,
    left_identity: (NodeId, u64, u64, u64),
    right_identity: (NodeId, u64, u64, u64),
    before: Option<(EditingSnapshot, EditingSnapshot)>,
    mutated: bool,
    completed: usize,
    result: Option<bool>,
    terminal_error: Option<V8BindingError>,
    terminal_failed: bool,
}
impl Transaction {
    pub(super) fn new(
        left: &Rc<V8BindingState>,
        left_owner: NodeId,
        right: &Rc<V8BindingState>,
        right_owner: NodeId,
    ) -> Result<Self, V8BindingError> {
        resume_editing_recovery(left)?;
        resume_editing_recovery(right)?;
        if Rc::ptr_eq(left, right) {
            return Err(V8BindingError::Allocation(
                "cross-document transaction requires distinct states",
            ));
        }
        let mut job = Self {
            jobs: [None, None],
            native_segments: [Vec::new(), Vec::new()],
            right_baselined: false,
            left: left.clone(),
            right: right.clone(),
            left_identity: identity(left),
            right_identity: identity(right),
            before: None,
            mutated: false,
            completed: 0,
            result: None,
            terminal_error: None,
            terminal_failed: false,
        };
        for (state, owner) in [(left, left_owner), (right, right_owner)] {
            let dom = state.dom.borrow();
            if state.execution_retired()
                || state.execution_tasks_stopped()
                || dom.arena().node_document(owner)? != Some(state.document_root())
                || !dom.is_document_connected(owner)?
            {
                job.result = Some(false);
                return Ok(job);
            }
        }
        if editing_snapshot_budget(left, &[left_owner])?
            .saturating_add(editing_snapshot_budget(right, &[right_owner])?)
            > UndoHistory::MAX_BYTES / 2
        {
            return Err(V8BindingError::Allocation("cross-document edit budget"));
        }
        let before_left = capture_editing_snapshot(left, left_owner)?;
        let before_right = capture_editing_snapshot(right, right_owner)?;
        job.before = Some((before_left, before_right));
        Ok(job)
    }
    fn current(&self) -> bool {
        [
            (&self.left, self.left_identity),
            (&self.right, self.right_identity),
        ]
        .into_iter()
        .all(|(state, original)| {
            !state.execution_retired()
                && !state.execution_tasks_stopped()
                && identity(state) == original
        }) && self.before.as_ref().is_none_or(|(left, right)| {
            [(&self.left, left), (&self.right, right)]
                .into_iter()
                .all(|(state, snapshot)| {
                    snapshot
                        .leases
                        .entries
                        .iter()
                        .find(|(identity, _)| identity.node == snapshot.host)
                        .is_some_and(|(_, lease)| {
                            let (dom, node) = lease.current();
                            Rc::ptr_eq(&dom, &state.dom)
                                && node == snapshot.host
                                && dom.borrow().is_document_connected(node).unwrap_or(false)
                        })
                })
        })
    }
    fn take_result(&mut self) -> Result<Option<bool>, V8BindingError> {
        match self.terminal_error.take() {
            Some(error) => Err(error),
            None => Ok(self.result),
        }
    }
    pub(super) fn failed(&self) -> bool {
        self.terminal_failed
    }
    pub(super) fn committed(&self) -> bool {
        self.mutated || self.jobs.iter().flatten().any(|job| job.has_mutations())
    }
    pub(super) fn cancel(&mut self) {
        self.mutated = self.committed();
        for job in self.jobs.iter_mut().flatten() {
            job.cancel();
        }
        self.jobs = [None, None];
        self.native_segments = [Vec::new(), Vec::new()];
        self.before.take();
        self.result = Some(self.mutated);
    }
    pub(super) fn poll_owned(
        &mut self,
        mut enter: impl FnMut(
            &Rc<V8BindingState>,
            &mut editing_continuation::Continuation,
        ) -> Result<Option<bool>, V8BindingError>,
        left: impl FnOnce(
            Rc<editing_continuation::Context>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), V8BindingError>>>,
        >,
        right: impl FnOnce(
            Rc<editing_continuation::Context>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), V8BindingError>>>,
        >,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return self.take_result();
        }
        if !self.current() {
            self.cancel();
            return self.take_result();
        }
        let (before_left, before_right) = self.before.as_ref().unwrap();
        if self.completed == 0 && self.jobs[0].is_none() {
            self.jobs[0] = editing_continuation::Continuation::new_at(
                self.left.clone(),
                Some(before_left.host),
                "moveAcrossDocuments".into(),
                left,
            )?;
            if let Some(job) = self.jobs[0].as_mut() {
                job.defer_history();
            }
        }
        if self.completed < 2 && self.jobs[1].is_none() {
            self.jobs[1] = editing_continuation::Continuation::new_at(
                self.right.clone(),
                Some(before_right.host),
                "moveAcrossDocuments".into(),
                right,
            )?;
            if let Some(job) = self.jobs[1].as_mut() {
                job.defer_history();
            }
        }
        while self.completed < 2 {
            let index = self.completed;
            if index == 1 && !self.right_baselined {
                if let Some(job) = self.jobs[1].as_mut() {
                    match job.refresh_baseline() {
                        Ok(()) => {}
                        Err(error)
                            if matches!(&error,V8BindingError::Unsupported{member,..}if member=="QuotaExceededError")
                                || matches!(
                                    &error,
                                    V8BindingError::Allocation("compound editing snapshot budget")
                                ) =>
                        {
                            job.stop_unperformed(error)
                        }
                        Err(error) => return Err(error),
                    }
                }
                self.right_baselined = true;
            }
            if let Some(job) = self.jobs[index].as_mut() {
                match enter(if index == 0 { &self.left } else { &self.right }, job) {
                    Ok(Some(changed)) => self.mutated |= changed,
                    Ok(None) => return Ok(None),
                    Err(error) if job.terminal_finished() => {
                        self.mutated |= job.has_mutations();
                        self.terminal_error = Some(error);
                        self.terminal_failed = true;
                    }
                    Err(error) => return Err(error),
                }
                self.native_segments[index] = job.take_segments();
            }
            self.jobs[index].take();
            self.completed += 1;
            if self.terminal_error.is_some() {
                // A completed native failure is not a yield. Keep its committed
                // segments, but never start the other document's mutation half.
                for job in self.jobs.iter_mut().flatten() {
                    job.cancel();
                }
                self.jobs = [None, None];
                self.completed = 2;
            }
            if !self.current() {
                self.cancel();
                return self.take_result();
            }
        }
        if !self.mutated {
            self.before.take();
            self.result = Some(false);
            return self.take_result();
        }
        let (left, right) = self.before.as_ref().unwrap();
        // Native segments already captured the final native state. A fresh
        // whole-tree snapshot here would charge interleaved author additions
        // and could turn destination capacity failure into an endless retry.
        for (index, state, before) in [(0, &self.left, left), (1, &self.right, right)] {
            if self.native_segments[index].is_empty() {
                self.native_segments[index].push(
                    editing_inverse::Segment::new(before.clone(), before.clone()).compact(state)?,
                );
            }
        }
        let after_left = self.native_segments[0].last().unwrap().after.clone();
        let after_right = self.native_segments[1].last().unwrap().after.clone();
        let updates_left =
            native_keyboard::segmented_document_changes(segment_changes(&self.native_segments[0]));
        let updates_right =
            native_keyboard::segmented_document_changes(segment_changes(&self.native_segments[1]));
        let token = Rc::new(());
        let bridge = Rc::new(Bridge {
            left: Rc::downgrade(&self.left),
            right: Rc::downgrade(&self.right),
            left_identity: self.left_identity,
            right_identity: self.right_identity,
            identity: token.clone(),
            progress: RefCell::new(None),
        });
        self.before.take();
        let left = self.native_segments[0].first().unwrap().before.clone();
        let right = self.native_segments[1].first().unwrap().before.clone();
        for (index, state, before, after, updates) in [
            (0, &self.left, left, after_left, updates_left),
            (1, &self.right, right, after_right, updates_right),
        ] {
            let owners = updates.iter().map(|entry| entry.0).collect();
            native_keyboard::record_document_edit(state, &token, updates);
            let unit = UndoUnit {
                native_progress: Default::default(),
                native_segments: std::mem::take(&mut self.native_segments[index]),
                before,
                after,
                companions: Vec::new(),
                identity: token.clone(),
                control_owners: owners,
                command: "moveAcrossDocuments".into(),
                split_nodes: None,
                paragraph_split: None,
                paragraph_redo: Default::default(),
                paragraph_failed: Default::default(),
                paragraph_reservation: Default::default(),
                cross_document: Some(bridge.clone()),
            };
            let mut history = state.undo_history.borrow_mut();
            history.bytes = history.bytes.saturating_add(undo_unit_bytes(&unit));
            history.undo.push_back(unit);
        }
        self.result = Some(self.mutated);
        trim(&self.left)?;
        trim(&self.right)?;
        self.take_result()
    }
    #[cfg(test)]
    pub(super) fn poll(
        &mut self,
        mut complete: impl FnMut(&Rc<V8BindingState>) -> Result<(), V8BindingError>,
        mutate: impl FnOnce() -> Result<(), V8BindingError>,
    ) -> Result<bool, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        if !self.current() {
            self.cancel();
            return Ok(self.mutated);
        }
        resume_editing_recovery(&self.left)?;
        resume_editing_recovery(&self.right)?;
        if !self.mutated {
            let (left, right) = self.before.as_ref().unwrap();
            if !editing_snapshot_authority_is_current(&self.left, left)?
                || !editing_snapshot_authority_is_current(&self.right, right)?
            {
                self.cancel();
                return Ok(false);
            }
            if let Err(error) = mutate() {
                self.result = Some(false);
                self.left.range_clone_completions.borrow_mut().clear();
                self.right.range_clone_completions.borrow_mut().clear();
                let (left, right) = self.before.take().unwrap();
                let a = recover_editing_snapshots(&self.left, vec![left]);
                let b = recover_editing_snapshots(&self.right, vec![right]);
                a?;
                b?;
                return Err(error);
            }
            self.mutated = true;
        }
        while self.completed < 2 {
            complete(if self.completed == 0 {
                &self.left
            } else {
                &self.right
            })?;
            self.completed += 1;
            if !self.current() {
                self.cancel();
                return Ok(true);
            }
        }
        #[cfg(test)]
        if FAIL_AFTER_COMPLETION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation("paired completion fault"));
        }
        synchronize_live_ranges(&self.left)?;
        synchronize_live_ranges(&self.right)?;
        let (left, right) = self.before.as_ref().unwrap();
        if !editing_snapshot_authority_is_current(&self.left, left)?
            || !editing_snapshot_authority_is_current(&self.right, right)?
        {
            self.cancel();
            return Ok(true);
        }
        if editing_snapshot_budget(&self.left, &[left.host])?
            .saturating_add(editing_snapshot_budget(&self.right, &[right.host])?)
            > UndoHistory::MAX_BYTES / 2
        {
            return Err(V8BindingError::Allocation("cross-document edit budget"));
        }
        let after_left = capture_editing_snapshot(&self.left, left.host)?;
        let after_right = capture_editing_snapshot(&self.right, right.host)?;
        let updates_left = changes(left, &after_left);
        let updates_right = changes(right, &after_right);
        native_keyboard::document_edit_budget(&updates_left)?;
        native_keyboard::document_edit_budget(&updates_right)?;
        let token = Rc::new(());
        let bridge = Rc::new(Bridge {
            left: Rc::downgrade(&self.left),
            right: Rc::downgrade(&self.right),
            left_identity: self.left_identity,
            right_identity: self.right_identity,
            identity: token.clone(),
            progress: RefCell::new(None),
        });
        let (left, right) = self.before.take().unwrap();
        // Both budgets and snapshots are ready; no fallible operation between commits.
        for (state, before, after, updates) in [
            (&self.left, left, after_left, updates_left),
            (&self.right, right, after_right, updates_right),
        ] {
            let owners = updates.iter().map(|entry| entry.0).collect();
            native_keyboard::record_document_edit(state, &token, updates);
            let unit = UndoUnit {
                native_progress: Default::default(),
                native_segments: Vec::new(),
                before,
                after,
                companions: Vec::new(),
                identity: token.clone(),
                control_owners: owners,
                command: "moveAcrossDocuments".into(),
                split_nodes: None,
                paragraph_split: None,
                paragraph_redo: Default::default(),
                paragraph_failed: Default::default(),
                paragraph_reservation: Default::default(),
                cross_document: Some(bridge.clone()),
            };
            let mut history = state.undo_history.borrow_mut();
            history.bytes += undo_unit_bytes(&unit);
            history.undo.push_back(unit);
        }
        self.result = Some(true);
        trim(&self.left)?;
        trim(&self.right)?;
        Ok(true)
    }
}
#[cfg(test)]
thread_local! {static FAIL_AFTER_COMPLETION:Cell<bool>=const{Cell::new(false)};static FAIL_AFTER_INVERSE_DOCUMENT:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
pub(super) fn run(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    source_owner: NodeId,
    target: &Rc<V8BindingState>,
    target_owner: NodeId,
    mutate: impl FnOnce() -> Result<(), V8BindingError>,
) -> Result<bool, V8BindingError> {
    Transaction::new(source, source_owner, target, target_owner)?
        .poll(|state| complete_editing_range_clones(scope, state), mutate)
}
pub(super) fn restore(bridge: &Bridge, redo: bool) -> Result<bool, V8BindingError> {
    let (Some(left), Some(right)) = (bridge.left.upgrade(), bridge.right.upgrade()) else {
        return Ok(false);
    };
    let left_recovery = resume_editing_recovery(&left);
    let right_recovery = resume_editing_recovery(&right);
    left_recovery?;
    right_recovery?;
    if left.execution_retired()
        || right.execution_retired()
        || identity(&left) != bridge.left_identity
        || identity(&right) != bridge.right_identity
    {
        return Ok(false);
    }
    let resumed = bridge
        .progress
        .borrow()
        .as_ref()
        .map(|progress| progress.redo);
    if resumed.is_some_and(|direction| direction != redo) {
        return Ok(false);
    }
    let mut units = Vec::new();
    for state in [&left, &right] {
        let history = state.undo_history.borrow();
        let Some(unit) = (if redo {
            history.redo.back()
        } else {
            history.undo.back()
        }) else {
            return Ok(false);
        };
        if unit.native_segments.is_empty()
            && (validate_editing_snapshot(state, &unit.before).is_err()
                || validate_editing_snapshot(state, &unit.after).is_err())
        {
            return Ok(false);
        }
        if !Rc::ptr_eq(&unit.identity, &bridge.identity)
            || resumed.is_some()
                && !native_keyboard::document_undo_owner_current(
                    state,
                    &bridge.identity,
                    &unit.control_owners,
                    redo,
                )
            || resumed.is_none()
                && !native_keyboard::can_document_undo(
                    state,
                    &bridge.identity,
                    &unit.control_owners,
                    redo,
                )?
        {
            return Ok(false);
        }
        {
            let dom = state.dom.borrow();
            if !dom.is_document_connected(unit.before.host)?
                || unit.native_segments.is_empty()
                    && unit
                        .before
                        .nodes
                        .iter()
                        .chain(unit.after.nodes.iter())
                        .any(|node| {
                            dom.arena().node_document(node.node).ok().flatten()
                                != Some(state.document_root())
                        })
            {
                return Ok(false);
            }
        }
        units.push(unit.clone());
    }
    if units.iter().any(|unit| !unit.native_segments.is_empty()) {
        if bridge.progress.borrow().is_none() {
            *bridge.progress.borrow_mut() = Some(NativeUndoProgress { redo, next: 0 });
        }
        loop {
            let next = bridge.progress.borrow().as_ref().unwrap().next;
            if next >= 2 {
                break;
            }
            let state = if next == 0 { &left } else { &right };
            restore_compound_undo(state, &units[next], redo)?;
            bridge.progress.borrow_mut().as_mut().unwrap().next += 1;
            #[cfg(test)]
            if FAIL_AFTER_INVERSE_DOCUMENT.with(|flag| flag.replace(false)) {
                return Err(V8BindingError::Allocation(
                    "injected paired inverse completion",
                ));
            }
        }
        for unit in &units {
            unit.native_progress.borrow_mut().take();
        }
        bridge.progress.borrow_mut().take();
        for state in [&left, &right] {
            let mut history = state.undo_history.borrow_mut();
            let unit = if redo {
                history.redo.pop_back()
            } else {
                history.undo.pop_back()
            }
            .expect("paired inverse owner");
            native_keyboard::finish_document_undo(state, &unit.control_owners, redo);
            if redo {
                history.undo.push_back(unit);
            } else {
                history.redo.push_back(unit);
            }
        }
        return Ok(true);
    }
    let rollback_left = capture_editing_snapshot(&left, units[0].before.host)?;
    let rollback_right = match capture_editing_snapshot(&right, units[1].before.host) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            release_editing_snapshot(&mut left.dom.borrow_mut(), &rollback_left)?;
            return Err(error);
        }
    };
    let result = (|| {
        for (state, unit) in [(&left, &units[0]), (&right, &units[1])] {
            restore_editing_snapshot(state, if redo { &unit.after } else { &unit.before })?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let restored_left = recover_editing_snapshots(&left, vec![rollback_left]);
        let restored_right = recover_editing_snapshots(&right, vec![rollback_right]);
        restored_left?;
        restored_right?;
        return Err(error);
    }
    drop(rollback_left);
    drop(rollback_right);
    for state in [&left, &right] {
        let mut history = state.undo_history.borrow_mut();
        let unit = if redo {
            history.redo.pop_back()
        } else {
            history.undo.pop_back()
        }
        .expect("validated paired undo owner");
        native_keyboard::finish_document_undo(state, &unit.control_owners, redo);
        *state.editing_overrides.borrow_mut() = if redo {
            unit.after.overrides.clone()
        } else {
            unit.before.overrides.clone()
        };
        if redo {
            history.undo.push_back(unit);
        } else {
            history.redo.push_back(unit);
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_pair_resumes_completion_without_reapplying_either_half() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let da = Rc::new(RefCell::new(
            html::parse("<input id=a value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let db = Rc::new(RefCell::new(
            html::parse("<input id=b value=XY>").into_runtime().unwrap(),
        ));
        let na = da.borrow().find_by_id("a").unwrap();
        let nb = db.borrow().find_by_id("b").unwrap();
        let left = {
            let scope = &mut v8::ContextScope::new(scope, a);
            install_generated_bindings(scope, a, da, RealmId(241), 1).unwrap()
        };
        let right = {
            let scope = &mut v8::ContextScope::new(scope, b);
            install_generated_bindings(scope, b, db, RealmId(242), 1).unwrap()
        };
        let mut job = Transaction::new(&left, na, &right, nb).unwrap();
        let mut completions = Vec::new();
        assert!(
            job.poll(
                |state| {
                    completions.push(state.realm);
                    if Rc::ptr_eq(state, &right) {
                        Err(V8BindingError::Allocation(
                            "second clone completion interrupted",
                        ))
                    } else {
                        Ok(())
                    }
                },
                || {
                    left.dom.borrow_mut().edit_control_value(na, 1, 3, "")?;
                    right.dom.borrow_mut().edit_control_value(nb, 2, 2, "bc")?;
                    Ok(())
                }
            )
            .is_err()
        );
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "a");
        assert_eq!(right.dom.borrow().control_value(nb).unwrap(), "XYbc");
        assert!(left.undo_history.borrow().undo.is_empty());
        assert!(
            job.poll(
                |state| {
                    assert!(Rc::ptr_eq(state, &right));
                    completions.push(state.realm);
                    Ok(())
                },
                || panic!("native mutation replay")
            )
            .unwrap()
        );
        assert_eq!(completions, vec![left.realm, right.realm, right.realm]);
        assert_eq!(left.undo_history.borrow().undo.len(), 1);
        assert_eq!(right.undo_history.borrow().undo.len(), 1);
        assert!(
            job.poll(
                |_| panic!("completion replay"),
                || panic!("mutation replay")
            )
            .unwrap()
        );
        assert!(undo_edit(&right, false).unwrap());
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "abc");
        assert_eq!(right.dom.borrow().control_value(nb).unwrap(), "XY");
        let mut retired = Transaction::new(&left, na, &right, nb).unwrap();
        FAIL_AFTER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            retired
                .poll(
                    |_| Ok(()),
                    || {
                        left.dom.borrow_mut().edit_control_value(na, 1, 3, "")?;
                        right.dom.borrow_mut().edit_control_value(nb, 2, 2, "bc")?;
                        Ok(())
                    }
                )
                .is_err()
        );
        {
            let scope = &mut v8::ContextScope::new(scope, b);
            let source=v8::String::new(scope,"globalThis.moved=document.getElementById('b');globalThis.other=document.implementation.createHTMLDocument('other');other.body.appendChild(other.adoptNode(moved));").unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
        }
        assert!(
            retired
                .poll(
                    |_| panic!("completed clone prefix replay"),
                    || panic!("adopted mutation replay")
                )
                .unwrap()
        );
        assert!(left.undo_history.borrow().undo.is_empty());
        assert!(right.undo_history.borrow().undo.is_empty());
        {
            let scope = &mut v8::ContextScope::new(scope, b);
            let source=v8::String::new(scope,"if(moved.ownerDocument!==other||moved.value!=='XYbc')throw Error('adopted half restored');").unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
        }
        right
            .document_authority_epoch
            .set(right.document_authority_epoch() + 1);
        assert!(
            retired
                .poll(
                    |_| panic!("retired completion"),
                    || panic!("retired mutation")
                )
                .unwrap()
        );
        assert!(left.undo_history.borrow().undo.is_empty());
        assert!(right.undo_history.borrow().undo.is_empty());
    }
    #[test]
    fn two_document_control_edit_rolls_back_and_undo_redo_is_paired() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let da = Rc::new(RefCell::new(
            html::parse("<input id=a value=source>")
                .into_runtime()
                .unwrap(),
        ));
        let db = Rc::new(RefCell::new(
            html::parse("<input id=b value=target>")
                .into_runtime()
                .unwrap(),
        ));
        let na = da.borrow().find_by_id("a").unwrap();
        let nb = db.borrow().find_by_id("b").unwrap();
        let left = {
            let scope = &mut v8::ContextScope::new(scope, a);
            install_generated_bindings(scope, a, da, RealmId(111), 1).unwrap()
        };
        let right = {
            let scope = &mut v8::ContextScope::new(scope, b);
            install_generated_bindings(scope, b, db, RealmId(112), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, a);
        assert!(
            run(scope, &left, na, &right, nb, || {
                left.dom.borrow_mut().edit_control_value(na, 0, 6, "")?;
                right
                    .dom
                    .borrow_mut()
                    .edit_control_value(nb, 6, 6, "source")?;
                Err(V8BindingError::Allocation(
                    "injected cross-document failure",
                ))
            })
            .is_err()
        );
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "source");
        assert_eq!(right.dom.borrow().control_value(nb).unwrap(), "target");
        assert!(
            run(scope, &left, na, &right, nb, || {
                left.dom.borrow_mut().edit_control_value(na, 0, 6, "")?;
                right
                    .dom
                    .borrow_mut()
                    .edit_control_value(nb, 6, 6, "source")?;
                Ok(())
            })
            .unwrap()
        );
        {
            let mut history = right.undo_history.borrow_mut();
            let before = &mut history.undo.back_mut().unwrap().before;
            let invalid = BoundaryPoint {
                container: nb,
                offset: u32::MAX,
            };
            before.selection.anchor = Some(invalid);
            before.selection.focus = Some(invalid);
        }
        assert!(undo_edit(&right, false).is_err());
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "");
        assert_eq!(
            right.dom.borrow().control_value(nb).unwrap(),
            "targetsource"
        );
        assert_eq!(left.undo_history.borrow().undo.len(), 1);
        assert_eq!(right.undo_history.borrow().undo.len(), 1);
        assert!(left.undo_history.borrow().redo.is_empty());
        assert!(right.undo_history.borrow().redo.is_empty());
        {
            let mut history = right.undo_history.borrow_mut();
            let before = &mut history.undo.back_mut().unwrap().before;
            before.selection.anchor = None;
            before.selection.focus = None;
        }
        assert!(undo_edit(&right, false).unwrap());
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "source");
        assert_eq!(right.dom.borrow().control_value(nb).unwrap(), "target");
        assert!(undo_edit(&left, true).unwrap());
        assert_eq!(left.dom.borrow().control_value(na).unwrap(), "");
        assert_eq!(
            right.dom.borrow().control_value(nb).unwrap(),
            "targetsource"
        );
        right.retire_execution();
        assert!(!undo_edit(&left, false).unwrap());
    }
}

#[cfg(test)]
mod segmented_tests {
    use super::*;
    #[test]
    fn terminal_half_failure_publishes_committed_pair_once_and_stops_unperformed_half() {
        super::super::tests::initialize_v8();
        for fail_left in [true, false] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let a = v8::Context::new(scope, Default::default());
            let b = v8::Context::new(scope, Default::default());
            let da = Rc::new(RefCell::new(
                html::parse("<div id=a contenteditable>abc</div>")
                    .into_runtime()
                    .unwrap(),
            ));
            let db = Rc::new(RefCell::new(
                html::parse("<div id=b contenteditable>XY</div>")
                    .into_runtime()
                    .unwrap(),
            ));
            let na = da.borrow().find_by_id("a").unwrap();
            let nb = db.borrow().find_by_id("b").unwrap();
            let ta = da.borrow().arena().get(na).unwrap().children[0];
            let tb = db.borrow().arena().get(nb).unwrap().children[0];
            let left = {
                let scope = &mut v8::ContextScope::new(scope, a);
                install_generated_bindings(scope, a, da, RealmId(247), 1).unwrap()
            };
            let right = {
                let scope = &mut v8::ContextScope::new(scope, b);
                install_generated_bindings(scope, b, db, RealmId(248), 1).unwrap()
            };
            let calls = Rc::new(Cell::new(0));
            let right_calls = calls.clone();
            let mut transaction = Transaction::new(&left, na, &right, nb).unwrap();
            let result = transaction.poll_owned(
                |state, job| {
                    let context = if Rc::ptr_eq(state, &left) { a } else { b };
                    job.poll(&mut v8::ContextScope::new(scope, context))
                },
                move |context| {
                    Box::pin(async move {
                        context.text(ta, 1, 2, "").await?;
                        if fail_left {
                            Err(V8BindingError::Unsupported {
                                interface: "Document".into(),
                                member: "AbortError".into(),
                            })
                        } else {
                            Ok(())
                        }
                    })
                },
                move |context| {
                    Box::pin(async move {
                        right_calls.set(right_calls.get() + 1);
                        context.text(tb, 2, 0, "bc").await?;
                        Err(V8BindingError::Unsupported {
                            interface: "Document".into(),
                            member: "AbortError".into(),
                        })
                    })
                },
            );
            assert!(
                matches!(result,Err(V8BindingError::Unsupported{member,..})if member=="AbortError")
            );
            assert_eq!(calls.get(), if fail_left { 0 } else { 1 });
            assert_eq!(left.undo_history.borrow().undo.len(), 1);
            assert_eq!(right.undo_history.borrow().undo.len(), 1);
            assert_eq!(
                transaction
                    .poll_owned(
                        |_, _| panic!("completed pair repolled child"),
                        |_| panic!("recreated left"),
                        |_| panic!("recreated right")
                    )
                    .unwrap(),
                Some(true)
            );
            assert_eq!(left.undo_history.borrow().undo.len(), 1);
            assert_eq!(right.undo_history.borrow().undo.len(), 1);
            assert!(undo_edit(&left, false).unwrap());
            assert_eq!(left.dom.borrow().arena().text_content(na).unwrap(), "abc");
            assert_eq!(right.dom.borrow().arena().text_content(nb).unwrap(), "XY");
        }
    }
    #[test]
    fn paired_control_payload_uses_native_inverse_when_mirror_budget_is_smaller() {
        super::super::tests::initialize_v8();
        for size in [3, 300_000] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let a = v8::Context::new(scope, Default::default());
            let b = v8::Context::new(scope, Default::default());
            let da = Rc::new(RefCell::new(
                html::parse("<input id=a>").into_runtime().unwrap(),
            ));
            let db = Rc::new(RefCell::new(
                html::parse("<input id=b>").into_runtime().unwrap(),
            ));
            let na = da.borrow().find_by_id("a").unwrap();
            let nb = db.borrow().find_by_id("b").unwrap();
            let original = "a".repeat(size);
            da.borrow_mut()
                .restore_control_edit_value(na, original.clone(), false)
                .unwrap();
            db.borrow_mut()
                .restore_control_edit_value(nb, original.clone(), false)
                .unwrap();
            let left = {
                let scope = &mut v8::ContextScope::new(scope, a);
                install_generated_bindings(scope, a, da, RealmId(249), 1).unwrap()
            };
            let right = {
                let scope = &mut v8::ContextScope::new(scope, b);
                install_generated_bindings(scope, b, db, RealmId(250), 1).unwrap()
            };
            let mut transaction = Transaction::new(&left, na, &right, nb).unwrap();
            assert_eq!(
                transaction
                    .poll_owned(
                        |state, job| {
                            let context = if Rc::ptr_eq(state, &left) { a } else { b };
                            job.poll(&mut v8::ContextScope::new(scope, context))
                        },
                        move |context| Box::pin(async move {
                            context.state.dom.borrow_mut().restore_control_edit_value(
                                na,
                                "b".repeat(size),
                                false,
                            )?;
                            Ok(())
                        }),
                        move |context| Box::pin(async move {
                            context.state.dom.borrow_mut().restore_control_edit_value(
                                nb,
                                "c".repeat(size),
                                false,
                            )?;
                            Ok(())
                        })
                    )
                    .unwrap(),
                Some(true)
            );
            assert_eq!(
                left.undo_history
                    .borrow()
                    .undo
                    .back()
                    .unwrap()
                    .control_owners
                    .is_empty(),
                size > 3
            );
            assert_eq!(
                right
                    .undo_history
                    .borrow()
                    .undo
                    .back()
                    .unwrap()
                    .control_owners
                    .is_empty(),
                size > 3
            );
            FAIL_AFTER_INVERSE_DOCUMENT.with(|flag| flag.set(true));
            assert!(undo_edit(&right, false).is_err());
            assert_eq!(left.dom.borrow().control_edit_value(na).unwrap(), original);
            assert_eq!(
                right.dom.borrow().control_edit_value(nb).unwrap(),
                "c".repeat(size)
            );
            assert!(undo_edit(&right, false).unwrap());
            assert_eq!(left.dom.borrow().control_edit_value(na).unwrap(), original);
            assert_eq!(right.dom.borrow().control_edit_value(nb).unwrap(), original);
        }
    }
    #[test]
    fn paired_native_inverses_exclude_changes_between_document_halves() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let da = Rc::new(RefCell::new(
            html::parse("<div id=a contenteditable>abc</div>")
                .into_runtime()
                .unwrap(),
        ));
        let db = Rc::new(RefCell::new(
            html::parse("<div id=b contenteditable>XY</div>")
                .into_runtime()
                .unwrap(),
        ));
        let na = da.borrow().find_by_id("a").unwrap();
        let nb = db.borrow().find_by_id("b").unwrap();
        let ta = da.borrow().arena().get(na).unwrap().children[0];
        let tb = db.borrow().arena().get(nb).unwrap().children[0];
        let left = {
            let scope = &mut v8::ContextScope::new(scope, a);
            install_generated_bindings(scope, a, da, RealmId(245), 1).unwrap()
        };
        let right = {
            let scope = &mut v8::ContextScope::new(scope, b);
            install_generated_bindings(scope, b, db, RealmId(246), 1).unwrap()
        };
        let mut transaction = Transaction::new(&left, na, &right, nb).unwrap();
        let mut interleaved = false;
        let result = transaction
            .poll_owned(
                |state, job| {
                    let context = if Rc::ptr_eq(state, &left) { a } else { b };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let result = job.poll(scope)?;
                    if Rc::ptr_eq(state, &left) && result.is_some() && !interleaved {
                        interleaved = true;
                        let mut dom = right.dom.borrow_mut();
                        dom.set_attribute(nb, "data-author", "kept")?;
                        let node = dom.create_element("span");
                        dom.set_attribute(node, "id", "author")?;
                        dom.append_child(nb, node)?;
                    }
                    Ok(result)
                },
                move |context| Box::pin(async move { context.text(ta, 1, 2, "").await }),
                move |context| Box::pin(async move { context.text(tb, 2, 0, "bc").await }),
            )
            .unwrap();
        assert_eq!(result, Some(true));
        assert_eq!(left.dom.borrow().arena().text_content(na).unwrap(), "a");
        assert_eq!(right.dom.borrow().arena().text_content(nb).unwrap(), "XYbc");
        assert!(undo_edit(&right, false).unwrap());
        assert_eq!(left.dom.borrow().arena().text_content(na).unwrap(), "abc");
        assert_eq!(right.dom.borrow().arena().text_content(nb).unwrap(), "XY");
        assert_eq!(
            right.dom.borrow().attribute(nb, "data-author").unwrap(),
            Some("kept")
        );
        assert!(right.dom.borrow().find_by_id("author").is_some());
        assert!(undo_edit(&left, true).unwrap());
        assert_eq!(right.dom.borrow().arena().text_content(nb).unwrap(), "XYbc");
        assert!(right.dom.borrow().find_by_id("author").is_some());
    }
}
