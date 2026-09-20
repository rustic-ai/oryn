//! Rich editing owned by an input receipt. Once native mutation succeeds,
//! failures in clone completion or undo publication resume without replaying it.
use super::*;

pub(super) struct Transaction {
    continuation: Option<editing_continuation::Continuation>,
    deferred_history: bool,
    completed_segments: Vec<editing_inverse::Segment>,
    identity: (NodeId, u64, u64, u64),
    command: String,
    host: Option<NodeId>,
    before: Option<EditingSnapshot>,
    cursor: Option<oryn_dom::MutationCursor>,
    #[cfg(test)]
    mutated: bool,
    #[cfg(test)]
    split_nodes: Option<ParagraphSplitNodes>,
    result: Option<bool>,
}
impl Transaction {
    pub(super) fn new(state: &V8BindingState, command: &str) -> Result<Self, V8BindingError> {
        resume_editing_recovery(state)?;
        let host = editing_host(state)?;
        let mut job = Self {
            continuation: None,
            deferred_history: false,
            completed_segments: Vec::new(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            command: command.into(),
            host,
            before: None,
            cursor: None,
            #[cfg(test)]
            mutated: false,
            #[cfg(test)]
            split_nodes: None,
            result: host.is_none().then_some(false),
        };
        if host.is_some() {
            job.capture(state)?;
        }
        Ok(job)
    }
    fn capture(&mut self, state: &V8BindingState) -> Result<(), V8BindingError> {
        let host = self.host.unwrap();
        editing_snapshot_budget(state, &[host])?;
        self.before = Some(capture_editing_snapshot(state, host)?);
        self.cursor = Some(state.dom.borrow().mutation_cursor());
        Ok(())
    }
    fn current(&self, state: &V8BindingState) -> bool {
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
    }
    pub(super) fn cancel(&mut self) {
        if let Some(job) = self.continuation.as_mut() {
            job.cancel();
        }
        self.continuation.take();
        self.completed_segments.clear();
        self.before.take();
        self.result = Some(false);
    }
    pub(super) fn defer_history(&mut self) {
        self.deferred_history = true;
    }
    pub(super) fn take_segments(&mut self) -> Vec<editing_inverse::Segment> {
        std::mem::take(&mut self.completed_segments)
    }
    pub(super) fn poll_owned(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        command: editing_commands::Command,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !self.current(state) {
            self.cancel();
            return Ok(Some(false));
        }
        if self.continuation.is_none() {
            self.continuation = editing_continuation::Continuation::new(
                state.clone(),
                self.command.clone(),
                move |context| command.build(context),
            )?;
            self.before.take();
            if self.deferred_history
                && let Some(job) = self.continuation.as_mut()
            {
                job.defer_history();
            }
            if self.continuation.is_none() {
                self.result = Some(false);
                return Ok(Some(false));
            }
        }
        let result = self.continuation.as_mut().unwrap().poll(scope)?;
        if let Some(result) = result {
            self.result = Some(result);
            if let Some(mut job) = self.continuation.take() {
                self.completed_segments = job.take_segments();
            }
        }
        Ok(result)
    }
    #[cfg(test)]
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        mutate: impl FnOnce(&V8BindingState) -> Result<(), V8BindingError>,
    ) -> Result<bool, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        if !self.current(state) {
            self.cancel();
            return Ok(false);
        }
        resume_editing_recovery(state)?;
        if self.before.is_none() {
            self.capture(state)?;
        }
        if !self.mutated {
            if !editing_snapshot_authority_is_current(state, self.before.as_ref().unwrap())? {
                self.cancel();
                return Ok(false);
            }
            state.pending_split_undo.set(None);
            if let Err(error) = mutate(state) {
                // The mutation closure is native-only. Author callbacks begin
                // at clone completion below, after the mutation commit marker.
                state.range_clone_completions.borrow_mut().clear();
                state.pending_split_undo.set(None);
                recover_editing_snapshots(state, vec![self.before.take().unwrap()])?;
                return Err(error);
            }
            self.mutated = true;
            self.split_nodes = state.pending_split_undo.take();
        }
        complete_editing_range_clones(scope, state)?;
        #[cfg(test)]
        if FAIL_AFTER_COMPLETION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "retained editing completion fault",
            ));
        }
        if !self.current(state) {
            self.cancel();
            return Ok(false);
        }
        synchronize_live_ranges(state)?;
        let before = self.before.as_ref().unwrap();
        if !editing_snapshot_authority_is_current(state, before)? {
            // Clone completion can run author callbacks that adopt nodes.
            // The edit has already committed. Its original undo snapshot
            // must not restore nodes now owned by a different document,
            // nor may retry replay the edit or restore stale overrides.
            self.before.take();
            self.split_nodes.take();
            self.result = Some(true);
            return Ok(true);
        }
        if matches!(
            self.command.as_str(),
            "delete"
                | "forwarddelete"
                | "insertlinebreak"
                | "insertparagraph"
                | "formatblock"
                | "insertorderedlist"
                | "insertunorderedlist"
        ) {
            *state.editing_overrides.borrow_mut() = before.overrides.clone();
        }
        let changed = self.cursor != Some(state.dom.borrow().mutation_cursor())
            || before.selection != selection_snapshot(state)
            || before.overrides != *state.editing_overrides.borrow();
        if !changed {
            self.before.take();
            self.result = Some(false);
            return Ok(false);
        }
        editing_snapshot_budget(state, &[self.host.unwrap()])?;
        let after = capture_editing_snapshot(state, self.host.unwrap())?;
        // push_undo_unit publishes infallibly; lease cleanup is deferred through
        // snapshot owners. No fallible work follows the history commit.
        push_undo_unit(
            state,
            UndoUnit {
                native_progress: Default::default(),
                native_segments: Vec::new(),
                cross_document: None,
                before: self.before.take().unwrap(),
                after,
                command: self.command.clone(),
                split_nodes: self.split_nodes.take(),
                companions: Vec::new(),
                identity: Rc::new(()),
                control_owners: Vec::new(),
            },
        )?;
        self.result = Some(true);
        Ok(true)
    }
}
/// Several editors and embedded controls publish one undo identity. Only a
/// failed native mutation rolls back; failures after mutation keep the receipt
/// and resume clone completion/history capture without applying the move again.
pub(super) struct Compound {
    continuation: Option<editing_continuation::Continuation>,
    #[cfg(test)]
    identity: (NodeId, u64, u64, u64),
    hosts: Vec<NodeId>,
    before: Vec<EditingSnapshot>,
    mutated: bool,
    result: Option<bool>,
}
impl Compound {
    pub(super) fn new(state: &V8BindingState, hosts: &[NodeId]) -> Result<Self, V8BindingError> {
        resume_editing_recovery(state)?;
        let mut requested = hosts.to_vec();
        requested.sort();
        requested.dedup();
        let mut roots = Vec::new();
        {
            let dom = state.dom.borrow();
            for host in &requested {
                if dom.arena().node_document(*host)? != Some(state.document_root())
                    || !dom.is_connected(*host)?
                {
                    return Ok(Self {
                        continuation: None,
                        #[cfg(test)]
                        identity: (
                            state.document_root(),
                            state.document_generation(),
                            state.document_stream_epoch(),
                            state.document_authority_epoch(),
                        ),
                        hosts: Vec::new(),
                        before: Vec::new(),
                        mutated: false,
                        result: Some(false),
                    });
                }
                let ancestors = dom.arena().ancestors(*host)?;
                if !requested
                    .iter()
                    .any(|other| other != host && ancestors.contains(other))
                {
                    roots.push(*host);
                }
            }
        }
        editing_snapshot_budget(state, &roots)?;
        let before = roots
            .iter()
            .map(|host| capture_editing_snapshot(state, *host))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            continuation: None,
            #[cfg(test)]
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            result: roots.is_empty().then_some(false),
            hosts: roots,
            before,
            mutated: false,
        })
    }
    #[cfg(test)]
    fn current(&self, state: &V8BindingState) -> bool {
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
    }
    pub(super) fn cancel(&mut self) {
        if let Some(job) = self.continuation.as_mut() {
            self.mutated |= job.has_mutations();
            job.cancel();
        }
        self.continuation.take();
        self.before.clear();
        self.result = Some(self.mutated);
    }
    pub(super) fn poll_future(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        build: impl FnOnce(
            Rc<editing_continuation::Context>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), V8BindingError>>>,
        >,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if self.continuation.is_none() {
            self.continuation = editing_continuation::Continuation::new_at(
                state.clone(),
                self.hosts.first().copied(),
                "moveByDrag".into(),
                build,
            )?;
            if let Some(job) = self.continuation.as_mut() {
                job.add_hosts(&self.hosts)?;
            }
            self.before.clear();
            if self.continuation.is_none() {
                self.result = Some(false);
                return Ok(Some(false));
            }
        }
        let result = self.continuation.as_mut().unwrap().poll(scope)?;
        if let Some(result) = result {
            self.mutated |= result;
            self.result = Some(result);
            self.continuation.take();
        }
        Ok(result)
    }
    #[cfg(test)]
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        mutate: impl FnOnce(&V8BindingState) -> Result<(), V8BindingError>,
    ) -> Result<bool, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        if !self.current(state) {
            self.cancel();
            return Ok(self.mutated);
        }
        resume_editing_recovery(state)?;
        if !self.mutated {
            for snapshot in &self.before {
                if !editing_snapshot_authority_is_current(state, snapshot)? {
                    self.cancel();
                    return Ok(false);
                }
            }
            if let Err(error) = mutate(state) {
                state.range_clone_completions.borrow_mut().clear();
                // Make failure terminal before recovery, which can itself fail
                // and retain a native recovery job in the owning document.
                self.result = Some(false);
                recover_editing_snapshots(state, std::mem::take(&mut self.before))?;
                return Err(error);
            }
            self.mutated = true;
        }
        complete_editing_range_clones(scope, state)?;
        #[cfg(test)]
        if FAIL_AFTER_COMPLETION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "retained compound completion fault",
            ));
        }
        if !self.current(state) {
            self.cancel();
            return Ok(true);
        }
        synchronize_live_ranges(state)?;
        // An author clone callback may adopt either half of a move. Never
        // restore that half or publish an undo pair spanning stale authority.
        for snapshot in &self.before {
            if !editing_snapshot_authority_is_current(state, snapshot)? {
                self.cancel();
                return Ok(true);
            }
        }
        editing_snapshot_budget(state, &self.hosts)?;
        let mut after = self
            .hosts
            .iter()
            .map(|host| capture_editing_snapshot(state, *host))
            .collect::<Result<Vec<_>, _>>()?;
        let mut changes = Vec::new();
        for control in self
            .before
            .iter()
            .flat_map(|snapshot| snapshot.controls.iter())
        {
            if let Some(next) = after
                .iter()
                .flat_map(|snapshot| snapshot.controls.iter())
                .find(|next| next.node == control.node)
                && control.value != next.value
            {
                changes.push((
                    control.node,
                    control.value.clone(),
                    control.selection.clone(),
                    next.value.clone(),
                    next.selection.clone(),
                ));
            }
        }
        native_keyboard::document_edit_budget(&changes)?;
        let identity = Rc::new(());
        let control_owners = changes.iter().map(|change| change.0).collect();
        native_keyboard::record_document_edit(state, &identity, changes);
        let mut before = std::mem::take(&mut self.before);
        let primary_before = before.remove(0);
        let primary_after = after.remove(0);
        push_undo_unit(
            state,
            UndoUnit {
                native_progress: Default::default(),
                native_segments: Vec::new(),
                cross_document: None,
                before: primary_before,
                after: primary_after,
                companions: before.into_iter().zip(after).collect(),
                identity,
                control_owners,
                command: "moveByDrag".into(),
                split_nodes: None,
            },
        )?;
        self.result = Some(true);
        Ok(true)
    }
}

#[cfg(test)]
thread_local! {static FAIL_AFTER_COMPLETION:Cell<bool>=const{Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn compound_completion_retry_preserves_both_halves_and_one_undo() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div id=s contenteditable>abc</div><div id=t contenteditable>XY</div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(230), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.s=document.getElementById('s');globalThis.t=document.getElementById('t');s.focus();getSelection().setBaseAndExtent(s.firstChild,1,s.firstChild,3);",
        );
        let mut job = Compound::new(&state, &[source, target]).unwrap();
        let before = state.undo_history.borrow().undo.len();
        FAIL_AFTER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            job.poll(scope, &state, |state| {
                replace_selection_with_text(state, "")?;
                let text = state.dom.borrow().children(target)?[0];
                let caret = BoundaryPoint {
                    container: text,
                    offset: 2,
                };
                set_selection_boundaries(state, caret, caret)?;
                insert_editing_text(state, "bc")
            })
            .is_err()
        );
        eval(
            scope,
            "if(s.textContent!=='a'||t.textContent!=='XYbc')throw Error('compound halves not committed');",
        );
        assert_eq!(state.undo_history.borrow().undo.len(), before);
        assert!(
            job.poll(scope, &state, |_| panic!("compound mutation replay"))
                .unwrap()
        );
        assert!(
            job.poll(scope, &state, |_| panic!("completed compound replay"))
                .unwrap()
        );
        assert_eq!(state.undo_history.borrow().undo.len(), before + 1);
        assert!(undo_edit(&state, false).unwrap());
        eval(
            scope,
            "if(s.textContent!=='abc'||t.textContent!=='XY')throw Error('compound undo split');",
        );
    }
    #[test]
    fn adopted_edit_completion_preserves_new_owner_without_replay_or_stale_undo() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t contenteditable>abc</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(216), 1).unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.focus();getSelection().collapse(t.firstChild,3);globalThis.other=document.implementation.createHTMLDocument('other');",
        );
        let mut job = Transaction::new(&state, "insertFromPaste").unwrap();
        let before = state.undo_history.borrow().undo.len();
        FAIL_AFTER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            job.poll(scope, &state, |state| insert_editing_text(state, "X"))
                .is_err()
        );
        eval(scope, "other.body.appendChild(other.adoptNode(t));");
        assert!(
            job.poll(scope, &state, |_| panic!(
                "adoption must not replay the committed edit"
            ))
            .unwrap()
        );
        assert!(
            job.poll(scope, &state, |_| panic!(
                "completed adoption must stay terminal"
            ))
            .unwrap()
        );
        assert!(job.before.is_none());
        assert_eq!(state.undo_history.borrow().undo.len(), before);
        eval(
            scope,
            "if(t.ownerDocument!==other||t.parentNode!==other.body||t.textContent!=='abcX')throw Error('completion changed adopted content');",
        );
        eval(
            scope,
            "document.body.appendChild(document.adoptNode(t));t.focus();getSelection().collapse(t.firstChild,4);",
        );
        let mut uncommitted = Transaction::new(&state, "insertFromPaste").unwrap();
        eval(scope, "other.body.appendChild(other.adoptNode(t));");
        assert!(
            !uncommitted
                .poll(scope, &state, |_| panic!(
                    "obsolete pre-edit ownership must not mutate"
                ))
                .unwrap()
        );
        eval(
            scope,
            "if(t.ownerDocument!==other||t.textContent!=='abcX')throw Error('uncommitted edit changed adopted content');",
        );
    }
    #[test]
    fn completion_retry_keeps_edit_and_commits_one_undo_unit() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t contenteditable>abc</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(213), 1).unwrap();
        eval(
            scope,
            "const t=document.getElementById('t');t.focus();getSelection().collapse(t.firstChild,3);",
        );
        let mut job = Transaction::new(&state, "insertFromPaste").unwrap();
        let before = state.undo_history.borrow().undo.len();
        FAIL_AFTER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            job.poll(scope, &state, |state| insert_editing_text(state, "X"))
                .is_err()
        );
        eval(
            scope,
            "if(document.getElementById('t').textContent!=='abcX')throw Error('committed edit was rolled back');",
        );
        assert_eq!(state.undo_history.borrow().undo.len(), before);
        assert!(
            job.poll(scope, &state, |_| panic!("mutation must not replay"))
                .unwrap()
        );
        assert!(
            job.poll(scope, &state, |_| panic!(
                "completed transaction must not replay"
            ))
            .unwrap()
        );
        assert_eq!(state.undo_history.borrow().undo.len(), before + 1);
        assert!(undo_edit(&state, false).unwrap());
        eval(
            scope,
            "if(document.getElementById('t').textContent!=='abc')throw Error('single undo must restore original text');",
        );
    }
}
