//! Native same-page frame drag transport. No serialization or author capability.
use super::*;
#[derive(Clone)]
pub struct NativeDragRealm {
    pub state: Rc<V8BindingState>,
    context: v8::Global<v8::Context>,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    ancestors: Vec<NativeDragAncestor>,
}
impl NativeDragRealm {
    pub fn new(state: Rc<V8BindingState>, context: v8::Global<v8::Context>) -> Self {
        Self {
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            state,
            context,
            ancestors: Vec::new(),
        }
    }
    pub fn with_ancestors(mut self, ancestors: Vec<NativeDragAncestor>) -> Self {
        self.ancestors = ancestors;
        self
    }
    fn live(&self) -> bool {
        self.ancestors.iter().all(NativeDragAncestor::live)
            && !self.state.execution_retired()
            && !self.state.execution_tasks_stopped()
            && (self.document, self.generation, self.epoch, self.authority)
                == (
                    self.state.document_root(),
                    self.state.document_generation(),
                    self.state.document_stream_epoch(),
                    self.state.document_authority_epoch(),
                )
    }
    fn enter<T>(
        &self,
        scope: &mut v8::PinScope,
        run: impl FnOnce(&mut v8::PinScope) -> Result<T, V8BindingError>,
    ) -> Result<T, V8BindingError> {
        if !self.live() {
            return Err(V8BindingError::Allocation("retired drag realm"));
        }
        let context = v8::Local::new(scope, &self.context);
        let owner = context
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("unbound drag context"))?;
        if !Rc::ptr_eq(&owner, &self.state) {
            return Err(V8BindingError::Allocation("drag context owner mismatch"));
        }
        let scope = &mut v8::ContextScope::new(scope, context);
        run(scope)
    }
}
#[derive(Clone)]
pub struct NativeDragAncestor {
    state: std::rc::Weak<V8BindingState>,
    frame: NodeId,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    cursor: oryn_dom::MutationCursor,
    watched: BTreeSet<NodeId>,
}
impl NativeDragAncestor {
    pub fn new(state: Rc<V8BindingState>, frame: NodeId) -> Result<Self, V8BindingError> {
        let dom = state.dom.borrow();
        if dom.arena().node_document(frame)? != Some(state.document_root())
            || !dom.is_document_connected(frame)?
        {
            return Err(V8BindingError::Allocation("detached drag embedding"));
        }
        let mut watched = dom
            .arena()
            .ancestors(frame)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        watched.insert(frame);
        Ok(Self {
            state: Rc::downgrade(&state),
            frame,
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: dom.mutation_cursor(),
            watched,
        })
    }
    fn live(&self) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        if state.execution_retired()
            || (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ) != (self.document, self.generation, self.epoch, self.authority)
        {
            return false;
        }
        let dom = state.dom.borrow();
        if dom.arena().node_document(self.frame).ok().flatten() != Some(self.document)
            || !dom.is_document_connected(self.frame).unwrap_or(false)
        {
            return false;
        }
        let batch = dom.mutations_since(self.cursor);
        !batch.truncated
            && !batch.mutations.iter().any(|change| match change {
                oryn_dom::Mutation::Removed { node, .. }
                | oryn_dom::Mutation::DocumentOwnership { node, .. }
                | oryn_dom::Mutation::ChildrenReplaced { node, .. } => self.watched.contains(node),
                _ => false,
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCrossFrameDragAction {
    Advance,
    Drop,
    Cancel,
}
pub(super) struct Target {
    state: std::rc::Weak<V8BindingState>,
    context: v8::Global<v8::Context>,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    node: NodeId,
    init: EventInit,
    ancestors: Vec<NativeDragAncestor>,
}
impl Target {
    fn new(realm: &NativeDragRealm, node: NodeId, init: &EventInit) -> Self {
        Self {
            state: Rc::downgrade(&realm.state),
            context: realm.context.clone(),
            document: realm.document,
            generation: realm.generation,
            epoch: realm.epoch,
            authority: realm.authority,
            node,
            init: init.clone(),
            ancestors: realm.ancestors.clone(),
        }
    }
    fn realm(&self) -> Option<NativeDragRealm> {
        let state = self.state.upgrade()?;
        let realm = NativeDragRealm {
            state,
            context: self.context.clone(),
            document: self.document,
            generation: self.generation,
            epoch: self.epoch,
            authority: self.authority,
            ancestors: self.ancestors.clone(),
        };
        realm.live().then_some(realm)
    }
    fn matches(&self, realm: &NativeDragRealm, node: NodeId) -> bool {
        self.realm().is_some()
            && self.node == node
            && self.document == realm.document
            && self.generation == realm.generation
            && self.epoch == realm.epoch
            && self.authority == realm.authority
            && self
                .state
                .upgrade()
                .is_some_and(|state| Rc::ptr_eq(&state, &realm.state))
    }
}
pub fn native_drag_session_active(state: &V8BindingState) -> bool {
    state
        .native_drag
        .borrow()
        .as_ref()
        .is_some_and(|session| session.live(state))
}
fn same_origin(a: &V8BindingState, b: &V8BindingState) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    matches!((&*a.security_origin.borrow(),&*b.security_origin.borrow()),(Some(a),Some(b))if a!="null"&&!a.is_empty()&&a==b)
}
fn payload_budget(store: &Store) -> Result<(), V8BindingError> {
    if store.items().len() > 4096 {
        return Err(V8BindingError::Allocation("cross-frame drag item limit"));
    }
    let mut bytes = 0usize;
    for item in store.items() {
        bytes = bytes
            .checked_add(item.media_type.len())
            .and_then(|bytes| {
                bytes.checked_add(match &item.kind {
                    crate::data_transfer::Data::String(value) => value.len().saturating_mul(2),
                    crate::data_transfer::Data::File(file) => file
                        .bytes
                        .len()
                        .saturating_add(file.name.len())
                        .saturating_add(file.media_type.len()),
                })
            })
            .ok_or(V8BindingError::Allocation(
                "cross-frame drag payload overflow",
            ))?;
        if bytes > crate::clipboard::MAX_CLIPBOARD_BYTES {
            return Err(V8BindingError::Allocation("cross-frame drag payload limit"));
        }
    }
    Ok(())
}
fn live_target(realm: &NativeDragRealm, node: NodeId) -> bool {
    realm.live()
        && realm
            .state
            .dom
            .borrow()
            .arena()
            .node_document(node)
            .ok()
            .flatten()
            == Some(realm.document)
        && realm.state.dom.borrow().is_connected(node).unwrap_or(false)
}
pub(super) struct Leave {
    target: Target,
    store: Rc<RefCell<Store>>,
    event: Option<data_transfer::TrustedTransfer>,
    done: bool,
}
impl Leave {
    pub(super) fn new(target: Target, store: Rc<RefCell<Store>>) -> Self {
        Self {
            target,
            store,
            event: None,
            done: false,
        }
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        let Some(realm) = self.target.realm() else {
            self.event.take();
            self.done = true;
            return Ok(true);
        };
        if self.event.is_none() {
            let mut init = self.target.init.clone();
            init.related_target = None;
            self.event = Some(realm.enter(scope, |scope| {
                data_transfer::TrustedTransfer::new(
                    scope,
                    &realm.state,
                    self.target.node,
                    "dragleave",
                    self.store.clone(),
                    &init,
                )
            })?);
        }
        let complete = realm
            .enter(scope, |scope| self.event.as_mut().unwrap().poll(scope))?
            .is_some();
        if complete {
            self.event.take();
            self.done = true;
        }
        Ok(complete)
    }
}
/// The host supplies physical hit ownership and coordinates for each exact
/// native realm. The existing source session can transition out of its local
/// negotiation target, into a child, and back into its own document.
pub fn dispatch_cross_frame_drag(
    scope: &mut v8::PinScope,
    source: &NativeDragRealm,
    target: Option<(&NativeDragRealm, NodeId)>,
    action: NativeCrossFrameDragAction,
    source_init: &EventInit,
    target_init: &EventInit,
) -> Result<DragEffect, V8BindingError> {
    dispatch_cross_frame_drag_with_previous(
        scope,
        source,
        target,
        action,
        source_init,
        target_init,
        None,
    )
}
pub fn native_drag_target_identity(state: &V8BindingState) -> Option<(RealmId, NodeId, u64, u64)> {
    let session = state.native_drag.borrow();
    let session = session.as_ref()?;
    if !session.live(state) {
        return None;
    }
    if let Some(target) = &session.cross_target {
        let realm = target.realm()?;
        Some((
            realm.state.realm,
            realm.document,
            realm.generation,
            realm.epoch,
        ))
    } else {
        session.negotiation.target().map(|_| {
            (
                state.realm,
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
        })
    }
}
pub fn dispatch_cross_frame_drag_with_previous(
    scope: &mut v8::PinScope,
    source: &NativeDragRealm,
    target: Option<(&NativeDragRealm, NodeId)>,
    action: NativeCrossFrameDragAction,
    source_init: &EventInit,
    target_init: &EventInit,
    previous_init: Option<&EventInit>,
) -> Result<DragEffect, V8BindingError> {
    let mut operation = NativeCrossFrameDrag::new(
        source,
        target,
        action,
        source_init,
        target_init,
        previous_init,
    )?;
    operation
        .poll(scope)?
        .ok_or(V8BindingError::Allocation("cross-frame drag suspended"))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CrossPhase {
    Prepare,
    Drag,
    Enter,
    Leave,
    Over,
    Drop,
    Default,
    CancelLeave,
    Commit,
    End,
    Done,
}
enum CrossEdit {
    Move(Move),
    Copy(drop_default::DropDefault),
    Control(control_move::ControlMove),
    Rich(rich_move::RichMove),
    Rejected,
}
impl CrossEdit {
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        match self {
            Self::Move(job) => job.poll(scope),
            Self::Copy(job) => job.poll(scope),
            Self::Control(job) => job.poll(scope),
            Self::Rich(job) => job.poll(scope),
            Self::Rejected => Ok(Some(false)),
        }
    }
}
/// Host-owned cross-frame input. It exclusively owns the detached Session until
/// completion; strong realm/event references never cycle through the state.
pub struct NativeCrossFrameDrag {
    source: NativeDragRealm,
    target: Option<(NativeDragRealm, NodeId)>,
    action: NativeCrossFrameDragAction,
    source_init: EventInit,
    target_init: EventInit,
    previous: Option<Target>,
    session: Option<Session>,
    phase: CrossPhase,
    event: Option<data_transfer::TrustedTransfer>,
    edit: Option<CrossEdit>,
    snapshot: Option<Store>,
    pointer_owner: Rc<()>,
    mirrors: Vec<PointerMirror>,
    owns_dispatch: bool,
    changed: bool,
    canceled: bool,
    success: bool,
    result: DragEffect,
}
struct PointerMirror {
    state: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    owner: Rc<()>,
}
impl PointerMirror {
    fn current(&self) -> bool {
        !self.state.execution_retired()
            && self.identity
                == (
                    self.state.document_root(),
                    self.state.document_generation(),
                    self.state.document_stream_epoch(),
                    self.state.document_authority_epoch(),
                )
            && native_pointer::registry_owner_current(&self.state, &self.owner)
    }
}
impl NativeCrossFrameDrag {
    pub fn new(
        source: &NativeDragRealm,
        target: Option<(&NativeDragRealm, NodeId)>,
        action: NativeCrossFrameDragAction,
        source_init: &EventInit,
        target_init: &EventInit,
        previous_init: Option<&EventInit>,
    ) -> Result<Self, V8BindingError> {
        if source.state.native_drag_dispatching.replace(true) {
            return Err(V8BindingError::Allocation(
                "reentrant native drag transport",
            ));
        }
        let mut session = source.state.native_drag.borrow_mut().take();
        let mut previous = session.as_mut().and_then(|session| {
            session.cross_target.take().or_else(|| {
                session
                    .negotiation
                    .target()
                    .map(|node| Target::new(source, node, source_init))
            })
        });
        if let (Some(previous), Some(init)) = (&mut previous, previous_init) {
            previous.init = init.clone();
        }
        Ok(Self {
            source: source.clone(),
            target: target.map(|(realm, node)| (realm.clone(), node)),
            action,
            source_init: source_init.clone(),
            target_init: target_init.clone(),
            previous,
            session,
            phase: CrossPhase::Prepare,
            event: None,
            edit: None,
            snapshot: None,
            pointer_owner: native_pointer::registry_owner(&source.state),
            mirrors: Vec::new(),
            owns_dispatch: true,
            changed: false,
            canceled: false,
            success: false,
            result: DragEffect::None,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.source.state.clone()
    }
    pub fn track_pointer_owner(&mut self, state: &Rc<V8BindingState>) {
        if Rc::ptr_eq(state, &self.source.state)
            || self
                .mirrors
                .iter()
                .any(|mirror| Rc::ptr_eq(&mirror.state, state))
        {
            return;
        }
        self.mirrors.push(PointerMirror {
            state: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            owner: native_pointer::registry_owner(state),
        });
    }
    pub fn finish_pointer_transition(
        &self,
        input: &native_pointer::NativePointerInput,
        terminal: bool,
    ) {
        if !self.pointer_owner_current() {
            return;
        }
        native_pointer::finish_native_pointer_drag_transition(&self.source.state, input, terminal);
        if terminal {
            for mirror in &self.mirrors {
                if mirror.current() {
                    native_pointer::synchronize_native_pointer_release(&mirror.state, input.id);
                }
            }
        }
    }
    pub fn pointer_owner_current(&self) -> bool {
        self.source.live()
            && native_pointer::registry_owner_current(&self.source.state, &self.pointer_owner)
    }
    pub fn retry_safe(&self) -> bool {
        self.phase != CrossPhase::Done
            && self.owns_dispatch
            && self.pointer_owner_current()
            && !self.source.state.execution_tasks_stopped()
    }
    fn unlock(&mut self) {
        if self.owns_dispatch {
            if (
                self.source.document,
                self.source.generation,
                self.source.epoch,
                self.source.authority,
            ) == (
                self.source.state.document_root(),
                self.source.state.document_generation(),
                self.source.state.document_stream_epoch(),
                self.source.state.document_authority_epoch(),
            ) {
                self.source.state.native_drag_dispatching.set(false);
            }
            self.owns_dispatch = false;
        }
    }
    fn finish(&mut self) {
        if !self.pointer_owner_current() {
            self.cancel();
            return;
        }
        if self.source.live()
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.live(&self.source.state))
        {
            *self.source.state.native_drag.borrow_mut() = self.session.take();
        }
        self.unlock();
        self.phase = CrossPhase::Done;
    }
    pub fn cancel(&mut self) {
        self.event.take();
        self.edit.take();
        if let Some(mut session) = self.session.take() {
            session.abandon();
            native_drag_image::clear(&self.source.state, session.feedback_session.take());
        }
        self.unlock();
        self.result = DragEffect::None;
        self.phase = CrossPhase::Done;
    }
    fn emit(
        &mut self,
        scope: &mut v8::PinScope,
        realm: &NativeDragRealm,
        node: NodeId,
        name: &str,
        init: &EventInit,
    ) -> Result<Option<bool>, V8BindingError> {
        if self.event.is_none() {
            let store = self.session.as_ref().unwrap().store.clone();
            self.event = Some(realm.enter(scope, |scope| {
                data_transfer::TrustedTransfer::new(scope, &realm.state, node, name, store, init)
            })?);
        }
        let result = realm.enter(scope, |scope| self.event.as_mut().unwrap().poll(scope))?;
        if result.is_some() {
            self.event.take();
        }
        Ok(result)
    }
    fn target(&self) -> Option<(NativeDragRealm, NodeId)> {
        self.target
            .as_ref()
            .filter(|(realm, node)| live_target(realm, *node))
            .cloned()
    }
    fn prepare(&mut self, node: Option<NodeId>) {
        let session = self.session.as_mut().unwrap();
        let preferred = preferred_effect(&self.target_init, session.selected.is_some());
        session
            .negotiation
            .prepare(node, preferred, &mut session.store.borrow_mut());
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<DragEffect>, V8BindingError> {
        loop {
            if self.phase == CrossPhase::Done {
                return Ok(Some(self.result));
            }
            if !self.pointer_owner_current() || self.source.state.execution_tasks_stopped() {
                self.cancel();
                continue;
            }
            if self.source.state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            if self.session.is_none() {
                self.finish();
                continue;
            }
            match self.phase {
                CrossPhase::Prepare => {
                    if !self.session.as_ref().unwrap().live(&self.source.state) {
                        self.cancel();
                        continue;
                    }
                    payload_budget(&self.session.as_ref().unwrap().store.borrow())?;
                    if self.action == NativeCrossFrameDragAction::Cancel {
                        self.phase = CrossPhase::CancelLeave;
                        continue;
                    }
                    if self.action == NativeCrossFrameDragAction::Drop {
                        if self.target().is_none_or(|(realm, node)| {
                            !self
                                .previous
                                .as_ref()
                                .is_some_and(|old| old.matches(&realm, node))
                        }) || self.session.as_ref().unwrap().operation() == DragEffect::None
                        {
                            self.phase = CrossPhase::CancelLeave
                        } else {
                            self.phase = CrossPhase::Drop
                        }
                        continue;
                    }
                    self.phase = CrossPhase::Drag;
                }
                CrossPhase::Drag => {
                    let source = self.source.clone();
                    let node = self.session.as_ref().unwrap().source;
                    let init = self.source_init.clone();
                    let Some(accepted) = self.emit(scope, &source, node, "drag", &init)? else {
                        return Ok(None);
                    };
                    if !accepted {
                        self.phase = CrossPhase::CancelLeave;
                        continue;
                    }
                    let target = self.target();
                    self.changed = match (&self.previous, &target) {
                        (Some(old), Some((realm, node))) => !old.matches(realm, *node),
                        (None, None) => false,
                        _ => true,
                    };
                    self.prepare(target.as_ref().map(|(_, node)| *node));
                    self.phase = CrossPhase::Enter;
                }
                CrossPhase::Enter => {
                    if self.changed {
                        if let Some((realm, node)) = self.target() {
                            let mut init = self.target_init.clone();
                            init.related_target = self.previous.as_ref().and_then(|old| {
                                old.realm()
                                    .filter(|prior| {
                                        same_origin(&prior.state, &realm.state)
                                            && Rc::ptr_eq(&prior.state.dom, &realm.state.dom)
                                    })
                                    .map(|_| old.node)
                            });
                            if self
                                .emit(scope, &realm, node, "dragenter", &init)?
                                .is_none()
                            {
                                return Ok(None);
                            }
                        } else {
                            self.event.take();
                        }
                    }
                    self.phase = CrossPhase::Leave;
                }
                CrossPhase::Leave | CrossPhase::CancelLeave => {
                    let cancel = self.phase == CrossPhase::CancelLeave;
                    if cancel || self.changed {
                        if let Some((realm, node, mut init)) =
                            self.previous.as_ref().and_then(|old| {
                                old.realm().map(|realm| (realm, old.node, old.init.clone()))
                            })
                        {
                            init.related_target = if cancel {
                                None
                            } else {
                                self.target()
                                    .filter(|(next, _)| {
                                        same_origin(&next.state, &realm.state)
                                            && Rc::ptr_eq(&next.state.dom, &realm.state.dom)
                                    })
                                    .map(|(_, node)| node)
                            };
                            if self
                                .emit(scope, &realm, node, "dragleave", &init)?
                                .is_none()
                            {
                                return Ok(None);
                            }
                        } else {
                            self.event.take();
                        }
                    }
                    self.phase = if cancel {
                        CrossPhase::Commit
                    } else {
                        CrossPhase::Over
                    };
                }
                CrossPhase::Over => {
                    let Some((realm, node)) = self.target() else {
                        self.event.take();
                        self.prepare(None);
                        self.finish();
                        continue;
                    };
                    let init = self.target_init.clone();
                    // Preparation changes transfer mode only before first event admission.
                    if self.event.is_none() {
                        self.prepare(Some(node));
                    }
                    let Some(accepted) = self.emit(scope, &realm, node, "dragover", &init)? else {
                        return Ok(None);
                    };
                    if !live_target(&realm, node) {
                        self.prepare(None);
                        self.finish();
                        continue;
                    }
                    let session = self.session.as_mut().unwrap();
                    let preferred = preferred_effect(&self.target_init, session.selected.is_some());
                    let default =
                        default_effect(&realm.state, node, &session.store.borrow()).map(|effect| {
                            if preferred == DragEffect::Move
                                && same_origin(&self.source.state, &realm.state)
                                && session
                                    .selected
                                    .as_ref()
                                    .is_some_and(|source| source.valid(&self.source.state))
                            {
                                DragEffect::Move
                            } else {
                                effect
                            }
                        });
                    self.result = session.negotiation.accept_over(
                        !accepted,
                        default,
                        &session.store.borrow(),
                    );
                    if self.result == DragEffect::Move
                        && !same_origin(&self.source.state, &realm.state)
                    {
                        session.negotiation.prepare(
                            None,
                            preferred,
                            &mut session.store.borrow_mut(),
                        );
                        self.result = DragEffect::None;
                    }
                    session.cross_target = Some(Target::new(&realm, node, &self.target_init));
                    self.finish();
                }
                CrossPhase::Drop => {
                    let Some((realm, node)) = self.target() else {
                        self.event.take();
                        self.phase = CrossPhase::Commit;
                        continue;
                    };
                    let init = self.target_init.clone();
                    let Some(accepted) = self.emit(scope, &realm, node, "drop", &init)? else {
                        return Ok(None);
                    };
                    self.canceled = !accepted;
                    self.success = self.canceled;
                    self.phase = if self.canceled {
                        CrossPhase::Commit
                    } else {
                        CrossPhase::Default
                    };
                }
                CrossPhase::Default => {
                    let Some((realm, node)) = self.target() else {
                        self.edit.take();
                        self.phase = CrossPhase::Commit;
                        continue;
                    };
                    if self.snapshot.is_none() {
                        let mut snapshot = self.session.as_ref().unwrap().store.borrow().clone();
                        payload_budget(&snapshot)?;
                        snapshot.set_mode(Mode::ReadOnly);
                        if snapshot.types().iter().any(|kind| kind == "text/html") {
                            let markup = clipboard_actions::sanitized_html_for_state(
                                &self.source.state,
                                &oryn_dom::dom_string::from_units(&snapshot.get_data("text/html")),
                            )?;
                            snapshot.set_mode(Mode::ReadWrite);
                            snapshot.set_data("text/html", oryn_dom::dom_string::units(&markup));
                            snapshot.set_mode(Mode::ReadOnly);
                        }
                        self.snapshot = Some(snapshot);
                    }
                    if self.edit.is_none() {
                        let session = self.session.as_ref().unwrap();
                        let snapshot = self.snapshot.as_ref().unwrap();
                        self.edit = Some(
                            if session.operation() == DragEffect::Move && session.selected.is_some()
                            {
                                let selected = session.selected.as_ref().unwrap();
                                if !same_origin(&self.source.state, &realm.state) {
                                    CrossEdit::Rejected
                                } else if Rc::ptr_eq(&self.source.state, &realm.state) {
                                    if !selected.move_target(&realm.state, node)
                                        || default_effect(&realm.state, node, snapshot).is_none()
                                    {
                                        CrossEdit::Rejected
                                    } else if let Some(caret) = selection_movement::drop_caret(
                                        &realm.state,
                                        node,
                                        self.target_init.client_x as f64,
                                        self.target_init.client_y as f64,
                                    )? {
                                        let text = oryn_dom::dom_string::from_units(
                                            &snapshot.get_data("text/plain"),
                                        );
                                        if matches!(&selected.kind, SelectedKind::Control { .. })
                                            && realm.state.dom.borrow().is_text_control(node)
                                        {
                                            CrossEdit::Control(control_move::ControlMove::new(
                                                &realm.state,
                                                selected,
                                                node,
                                                caret.offset,
                                                &text,
                                            ))
                                        } else {
                                            CrossEdit::Rich(rich_move::RichMove::new(
                                                &realm.state,
                                                selected,
                                                node,
                                                snapshot,
                                                caret,
                                                &text,
                                            ))
                                        }
                                    } else {
                                        CrossEdit::Rejected
                                    }
                                } else {
                                    Move::new(
                                        &self.source,
                                        &realm,
                                        node,
                                        snapshot,
                                        selected,
                                        &self.target_init,
                                    )?
                                    .map(CrossEdit::Move)
                                    .unwrap_or(CrossEdit::Rejected)
                                }
                            } else {
                                CrossEdit::Copy(drop_default::DropDefault::new(
                                    &realm.state,
                                    node,
                                    snapshot,
                                    self.target_init.client_x as f64,
                                    self.target_init.client_y as f64,
                                ))
                            },
                        );
                        match self.edit.as_mut().unwrap() {
                            CrossEdit::Copy(job) => job.set_input_owner(&self.source.state),
                            CrossEdit::Control(job) => job.set_input_owner(&self.source.state),
                            CrossEdit::Rich(job) => job.set_input_owner(&self.source.state),
                            _ => {}
                        }
                    }
                    // Freeze the external source only once, at child admission.
                    let Some(success) =
                        realm.enter(scope, |scope| self.edit.as_mut().unwrap().poll(scope))?
                    else {
                        return Ok(None);
                    };
                    self.success = success;
                    self.edit.take();
                    self.phase = CrossPhase::Commit;
                }
                CrossPhase::Commit => {
                    let session = self.session.as_mut().unwrap();
                    self.result = session.negotiation.finish_drop(
                        self.canceled,
                        self.success,
                        &mut session.store.borrow_mut(),
                    );
                    session.ended = true;
                    session.cross_target = None;
                    native_drag_image::clear(&self.source.state, session.feedback_session.take());
                    self.phase = CrossPhase::End;
                }
                CrossPhase::End => {
                    let source = self.source.clone();
                    let session = self.session.as_ref().unwrap();
                    let node = session.source;
                    if session.present(&source.state, node) {
                        let init = self.source_init.clone();
                        if self.emit(scope, &source, node, "dragend", &init)?.is_none() {
                            return Ok(None);
                        }
                    }
                    self.session
                        .as_ref()
                        .unwrap()
                        .store
                        .borrow_mut()
                        .set_mode(Mode::Detached);
                    self.finish();
                }
                CrossPhase::Done => return Ok(Some(self.result)),
            }
        }
    }
}
impl Drop for NativeCrossFrameDrag {
    fn drop(&mut self) {
        if self.phase != CrossPhase::Done {
            self.cancel();
        }
    }
}
pub(super) struct Move {
    source: NativeDragRealm,
    target_realm: NativeDragRealm,
    target: NodeId,
    store: Store,
    selected: SelectedSource,
    source_owner: NodeId,
    target_owner: NodeId,
    caret: BoundaryPoint,
    target_control: bool,
    text: String,
    data: crate::clipboard::ClipboardData,
    html: Option<String>,
    before_target: Option<String>,
    next_target: Option<String>,
    pointer_owner: Rc<()>,
    destination: Option<oryn_dom::LiveRangeId>,
    target_cursor: oryn_dom::MutationCursor,
    phase: u8,
    event: Option<clipboard_actions::ClipboardNotification>,
    transaction: Option<cross_document_edit::Transaction>,
    result: Option<bool>,
}
impl Move {
    pub(super) fn new(
        source: &NativeDragRealm,
        target_realm: &NativeDragRealm,
        target: NodeId,
        store: &Store,
        selected: &SelectedSource,
        init: &EventInit,
    ) -> Result<Option<Self>, V8BindingError> {
        if Rc::ptr_eq(&source.state, &target_realm.state)
            || !source.live()
            || !target_realm.live()
            || !same_origin(&source.state, &target_realm.state)
            || !selected.valid(&source.state)
            || default_effect(&target_realm.state, target, store).is_none()
        {
            return Ok(None);
        }
        let source_owner = match &selected.kind {
            SelectedKind::Control { node, .. } => {
                let dom = source.state.dom.borrow();
                if dom.control_disabled(*node)? || dom.attribute(*node, "readonly")?.is_some() {
                    return Ok(None);
                }
                *node
            }
            SelectedKind::Range {
                host: Some(host), ..
            } => *host,
            _ => return Ok(None),
        };
        let Some(caret) = selection_movement::drop_caret(
            &target_realm.state,
            target,
            init.client_x as f64,
            init.client_y as f64,
        )?
        else {
            return Ok(None);
        };
        let target_control = target_realm.state.dom.borrow().is_text_control(target);
        let target_owner = if target_control {
            target
        } else {
            editing_host_for_node(
                &target_realm.state.dom.borrow(),
                Some(target),
                target_realm.state.design_mode_enabled(),
                target_realm.document,
            )?
            .ok_or(V8BindingError::Allocation("cross-frame drop editing host"))?
        };
        let text = oryn_dom::dom_string::from_units(&store.get_data("text/plain"));
        let data = clipboard_actions::to_data(store)?;
        let html = if !target_control
            && !host_is_plaintext_only(&target_realm.state.dom.borrow(), target_owner)
            && store.types().iter().any(|kind| kind == "text/html")
        {
            // Resolve source-relative markup with its native source URL before it
            // enters the target document's independent editing transaction.
            Some(clipboard_actions::sanitized_html_for_state(
                &source.state,
                &oryn_dom::dom_string::from_units(&store.get_data("text/html")),
            )?)
        } else {
            None
        };
        let before_target = if target_control {
            Some(target_realm.state.dom.borrow().control_value(target)?)
        } else {
            None
        };
        let next_target = if let Some(before) = &before_target {
            let mut value = oryn_dom::dom_string::units(before);
            if caret.offset as usize > value.len() {
                return Ok(None);
            }
            value.splice(
                caret.offset as usize..caret.offset as usize,
                oryn_dom::dom_string::units(&text),
            );
            if target_realm
                .state
                .dom
                .borrow()
                .control_length_limit(target, "maxlength")?
                .is_some_and(|maximum| value.len() > maximum)
            {
                return Ok(None);
            }
            Some(oryn_dom::dom_string::from_units(&value))
        } else {
            None
        };
        Ok(Some(Self {
            source: source.clone(),
            target_realm: target_realm.clone(),
            target,
            store: store.clone(),
            selected: selected.clone(),
            source_owner,
            target_owner,
            caret,
            target_control,
            text,
            data,
            html,
            before_target,
            next_target,
            pointer_owner: native_pointer::registry_owner(&source.state),
            destination: None,
            target_cursor: target_realm.state.dom.borrow().mutation_cursor(),
            phase: 0,
            event: None,
            transaction: None,
            result: None,
        }))
    }
    fn set_source(&self) -> Result<(), V8BindingError> {
        match &self.selected.kind {
            SelectedKind::Control {
                node, start, end, ..
            } => {
                self.source
                    .state
                    .text_control_selections
                    .borrow_mut()
                    .insert(*node, (*start, *end, "none".into()));
            }
            SelectedKind::Range { range, .. } => {
                set_selection_boundaries(
                    &self.source.state,
                    BoundaryPoint {
                        container: range.start_container,
                        offset: range.start_offset,
                    },
                    BoundaryPoint {
                        container: range.end_container,
                        offset: range.end_offset,
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        loop {
            if !self.source.live()
                || !self.target_realm.live()
                || !native_pointer::registry_owner_current(&self.source.state, &self.pointer_owner)
            {
                self.event.take();
                if let Some(job) = &mut self.transaction {
                    job.cancel()
                }
                self.result = Some(
                    self.phase >= 3 || self.transaction.as_ref().is_some_and(|job| job.committed()),
                );
                return Ok(self.result);
            }
            if self.source.state.execution_suspended()
                || self.target_realm.state.execution_suspended()
                || scope.is_execution_terminating()
            {
                return Ok(None);
            }
            if self.source.state.execution_tasks_stopped()
                || self.target_realm.state.execution_tasks_stopped()
            {
                self.event.take();
                if let Some(job) = &mut self.transaction {
                    job.cancel()
                }
                self.result = Some(
                    self.phase >= 3 || self.transaction.as_ref().is_some_and(|job| job.committed()),
                );
                return Ok(self.result);
            }
            if self.phase == 2 {
                if self.transaction.is_none() {
                    if !self.selected.valid(&self.source.state)
                        || !live_target(&self.target_realm, self.target)
                        || default_effect(&self.target_realm.state, self.target, &self.store)
                            .is_none()
                    {
                        self.result = Some(false);
                        return Ok(self.result);
                    }
                    if let Some(before) = &self.before_target {
                        if {
                            let dom = self.target_realm.state.dom.borrow();
                            let batch = dom.mutations_since(self.target_cursor);
                            dom.control_value(self.target)?!=*before||batch.truncated||batch.mutations.iter().any(|mutation|matches!(mutation,oryn_dom::Mutation::FormState{node}|oryn_dom::Mutation::Attribute{node,..}|oryn_dom::Mutation::Removed{node,..}|oryn_dom::Mutation::DocumentOwnership{node,..}if *node==self.target))
                        } {
                            self.result = Some(false);
                            return Ok(self.result);
                        }
                    } else if !caret_owned_by(
                        &self.target_realm.state,
                        self.caret,
                        self.target_owner,
                    ) {
                        self.result = Some(false);
                        return Ok(self.result);
                    }
                    self.set_source()?;
                    if !self.target_control && self.destination.is_none() {
                        self.destination = Some(
                            self.target_realm
                                .state
                                .dom
                                .borrow_mut()
                                .create_live_range(self.caret, self.caret)?,
                        );
                    }
                    self.transaction = Some(cross_document_edit::Transaction::new(
                        &self.source.state,
                        self.source_owner,
                        &self.target_realm.state,
                        self.target_owner,
                    )?);
                }
                let source = self.source.clone();
                let target_realm = self.target_realm.clone();
                let selected = self.selected.clone();
                let target = self.target;
                let caret = self.caret;
                let before_target = self.before_target.clone();
                let next_target = self.next_target.clone();
                let text = self.text.clone();
                let html = self.html.clone();
                let destination = self.destination;
                let Some(changed) = self.transaction.as_mut().unwrap().poll_owned(
                    |state, job| {
                        let realm = if Rc::ptr_eq(state, &source.state) {
                            &source
                        } else {
                            &target_realm
                        };
                        realm.enter(scope, |scope| job.poll(scope))
                    },
                    move |context| {
                        Box::pin(async move {
                            let state: &V8BindingState = &context.state;
                            match selected.kind {
                                SelectedKind::Control {
                                    node, start, end, ..
                                } => {
                                    context.control(node, start, end, "").await?;
                                    state
                                        .text_control_selections
                                        .borrow_mut()
                                        .insert(node, (start, start, "none".into()));
                                }
                                SelectedKind::Range { .. } => {
                                    editing_algorithms::replace_selection_with_text(&context, "")
                                        .await?
                                }
                            }
                            Ok(())
                        })
                    },
                    move |context| {
                        Box::pin(async move {
                            let state: &V8BindingState = &context.state;
                            if let (Some(before), Some(next)) = (before_target, next_target) {
                                context
                                    .control(
                                        target,
                                        0,
                                        oryn_dom::dom_string::len(&before) as u32,
                                        next.clone(),
                                    )
                                    .await?;
                                if state.dom.borrow().control_value(target)? != next {
                                    return Err(V8BindingError::Allocation(
                                        "cross-frame drop control normalization",
                                    ));
                                }
                                let offset = caret
                                    .offset
                                    .saturating_add(oryn_dom::dom_string::len(&text) as u32);
                                state
                                    .text_control_selections
                                    .borrow_mut()
                                    .insert(target, (offset, offset, "none".into()));
                            } else {
                                let caret = state
                                    .dom
                                    .borrow()
                                    .live_range(destination.ok_or(V8BindingError::Allocation(
                                        "cross-document drop caret",
                                    ))?)?
                                    .start;
                                set_selection_boundaries(state, caret, caret)?;
                                match html {
                                    Some(html) => {
                                        editing_algorithms::replace_selection_with_html(
                                            &context, &html, true,
                                        )
                                        .await?
                                    }
                                    None => {
                                        editing_algorithms::insert_editing_text(&context, &text)
                                            .await?
                                    }
                                }
                            }
                            Ok(())
                        })
                    },
                )?
                else {
                    return Ok(None);
                };
                if self.transaction.as_ref().is_some_and(|job| job.failed()) {
                    if let Some(range) = self.destination.take() {
                        target_realm.state.dom.borrow_mut().drop_live_range(range)?;
                    }
                    self.result = Some(changed);
                    return Ok(self.result);
                }
                if !changed {
                    self.result = Some(false);
                    return Ok(self.result);
                }
                if let Some(range) = self.destination.take() {
                    target_realm.state.dom.borrow_mut().drop_live_range(range)?;
                }
                self.phase = 3;
                continue;
            }
            if self.phase >= 5 {
                self.result = Some(true);
                return Ok(self.result);
            }
            let is_source = self.phase == 0 || self.phase == 3;
            if self.event.is_none() {
                if self.phase < 2 && !self.selected.valid(&self.source.state) {
                    self.result = Some(false);
                    return Ok(self.result);
                }
                if self.phase == 0 {
                    self.set_source()?;
                }
                if self.phase == 1 {
                    if self.target_control {
                        self.target_realm
                            .state
                            .text_control_selections
                            .borrow_mut()
                            .insert(
                                self.target,
                                (self.caret.offset, self.caret.offset, "none".into()),
                            );
                    } else {
                        set_selection_boundaries(&self.target_realm.state, self.caret, self.caret)?;
                    }
                }
                let realm = if is_source {
                    &self.source
                } else {
                    &self.target_realm
                };
                let owner = if is_source {
                    self.source_owner
                } else {
                    self.target_owner
                };
                if !live_target(realm, owner) {
                    self.result = Some(
                        self.phase >= 3
                            || self.transaction.as_ref().is_some_and(|job| job.committed()),
                    );
                    return Ok(self.result);
                }
                let kind = if self.phase < 2 {
                    "beforeinput"
                } else {
                    "input"
                };
                let input = if is_source {
                    "deleteByDrag"
                } else {
                    "insertFromDrop"
                };
                self.event = Some(realm.enter(scope, |scope| {
                    clipboard_actions::ClipboardNotification::editing(
                        scope,
                        &realm.state,
                        owner,
                        kind,
                        input,
                        (!is_source && self.target_control).then_some(self.text.as_str()),
                        (!is_source && !self.target_control).then_some(&self.data),
                    )
                })?);
            }
            let realm = if is_source {
                &self.source
            } else {
                &self.target_realm
            };
            let Some(accepted) = realm.enter(scope, |scope| {
                self.event.as_mut().unwrap().poll(scope, &realm.state)
            })?
            else {
                return Ok(None);
            };
            self.event.take();
            if !accepted && self.phase < 2 {
                self.result = Some(false);
                return Ok(self.result);
            }
            self.phase += 1;
        }
    }
}
impl Drop for Move {
    fn drop(&mut self) {
        self.event.take();
        if let Some(range) = self.destination.take() {
            let _ = self
                .target_realm
                .state
                .dom
                .borrow_mut()
                .drop_live_range(range);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    thread_local! {static RESET_SOURCE:RefCell<Option<Rc<V8BindingState>>>=const{RefCell::new(None)};}
    fn reset_source(
        _scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _rv: v8::ReturnValue,
    ) {
        RESET_SOURCE.with(|source| {
            if let Some(source) = source.borrow().as_ref() {
                native_pointer::reset_native_pointer_sources(source);
            }
        });
    }
    fn script(scope: &mut v8::PinScope, source: &str) -> Result<(), V8BindingError> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok(())
    }
    #[test]
    fn retained_cross_document_move_keeps_input_suffix_and_pair_identity() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let ga = v8::Global::new(scope, a);
        let gb = v8::Global::new(scope, b);
        let da = Rc::new(RefCell::new(
            html::parse("<input id=s value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let db = Rc::new(RefCell::new(
            html::parse("<input id=t value=XY>").into_runtime().unwrap(),
        ));
        let sa = {
            let scope = &mut v8::ContextScope::new(scope, a);
            install_generated_bindings(scope, a, da, RealmId(243), 1).unwrap()
        };
        let sb = {
            let scope = &mut v8::ContextScope::new(scope, b);
            install_generated_bindings(scope, b, db, RealmId(244), 1).unwrap()
        };
        sa.set_security_origin("https://move.test".into());
        sb.set_security_origin("https://move.test".into());
        let scope = &mut v8::ContextScope::new(scope, a);
        let source = NativeDragRealm::new(sa, ga);
        let target = NativeDragRealm::new(sb, gb);
        let sn = source.state.dom.borrow().find_by_id("s").unwrap();
        let tn = target.state.dom.borrow().find_by_id("t").unwrap();
        source.enter(scope,|scope|script(scope,"globalThis.trace=[];s.onbeforeinput=e=>trace.push(e.inputType);s.oninput=e=>trace.push(e.inputType)")).unwrap();
        target.enter(scope,|scope|script(scope,"globalThis.trace=[];t.onbeforeinput=e=>trace.push(e.inputType);t.oninput=e=>trace.push(e.inputType)")).unwrap();
        let selected = SelectedSource {
            kind: SelectedKind::Control {
                node: sn,
                start: 1,
                end: 3,
                value: "abc".into(),
            },
            document: source.document,
            generation: source.generation,
            epoch: source.epoch,
            authority: source.authority,
            cursor: source.state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([sn]),
        };
        let mut store = Store::default();
        store.set_data("text/plain", vec![98, 99]);
        let data = clipboard_actions::to_data(&store).unwrap();
        let mut job = Move {
            source: source.clone(),
            target_realm: target.clone(),
            target: tn,
            store,
            selected,
            source_owner: sn,
            target_owner: tn,
            caret: BoundaryPoint {
                container: tn,
                offset: 2,
            },
            target_control: true,
            text: "bc".into(),
            data,
            html: None,
            before_target: Some("XY".into()),
            next_target: Some("XYbc".into()),
            pointer_owner: native_pointer::registry_owner(&source.state),
            destination: None,
            target_cursor: target.state.dom.borrow().mutation_cursor(),
            phase: 0,
            event: None,
            transaction: None,
            result: None,
        };
        for index in 0..4 {
            native_platform::publication::fail_after_next_callback();
            assert!(job.poll(scope).is_err());
            if index < 2 {
                assert_eq!(source.state.dom.borrow().control_value(sn).unwrap(), "abc")
            } else {
                assert_eq!(source.state.dom.borrow().control_value(sn).unwrap(), "a");
                assert_eq!(target.state.dom.borrow().control_value(tn).unwrap(), "XYbc");
            }
        }
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert_eq!(source.state.undo_history.borrow().undo.len(), 1);
        assert_eq!(target.state.undo_history.borrow().undo.len(), 1);
        source.enter(scope,|scope|script(scope,"if(trace.join(',')!=='deleteByDrag,deleteByDrag')throw Error('source input replay')")).unwrap();
        target.enter(scope,|scope|script(scope,"if(trace.join(',')!=='insertFromDrop,insertFromDrop')throw Error('destination input replay')")).unwrap();
        assert!(undo_edit(&source.state, false).unwrap());
        assert_eq!(source.state.dom.borrow().control_value(sn).unwrap(), "abc");
        assert_eq!(target.state.dom.borrow().control_value(tn).unwrap(), "XY");
        // A target input listener can reset the separate source registry.
        // The committed file change survives, but the later change event is revoked.
        target.enter(scope,|scope|script(scope,"document.body.innerHTML='<input id=file type=file>';globalThis.changes=0;file.onchange=()=>changes++")).unwrap();
        let file = target.state.dom.borrow().find_by_id("file").unwrap();
        target
            .enter(scope, |scope| {
                let object = wrap_node(scope, &target.state, file)?;
                let key = v8::String::new(scope, "oninput").unwrap();
                let callback = v8::Function::new(scope, reset_source).unwrap();
                object
                    .set(scope, key.into(), callback.into())
                    .ok_or(V8BindingError::Allocation("reset handler"))?;
                Ok(())
            })
            .unwrap();
        let mut payload = Store::default();
        payload.add_file(oryn_dom::SelectedFile {
            name: "owned.txt".into(),
            last_modified: 0,
            media_type: "text/plain".into(),
            bytes: vec![97].into(),
        });
        let mut copy = drop_default::DropDefault::new(&target.state, file, &payload, 0., 0.);
        copy.set_input_owner(&source.state);
        RESET_SOURCE.with(|slot| *slot.borrow_mut() = Some(source.state.clone()));
        assert_eq!(
            target.enter(scope, |scope| copy.poll(scope)).unwrap(),
            Some(true)
        );
        RESET_SOURCE.with(|slot| slot.borrow_mut().take());
        target.enter(scope,|scope|script(scope,"if(file.files.length!==1||changes!==0)throw Error('external source reset lost commit or ran suffix')")).unwrap();
    }
    #[test]
    fn foreign_realm_drop_preserves_event_store_modes_and_detaches_after_end() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let ga = v8::Global::new(scope, a);
        let gb = v8::Global::new(scope, b);
        let da = Rc::new(RefCell::new(
            html::parse("<div id=source draggable=true>source</div>")
                .into_runtime()
                .unwrap(),
        ));
        let db = Rc::new(RefCell::new(
            html::parse("<div id=target>target</div>")
                .into_runtime()
                .unwrap(),
        ));
        let sa = {
            let scope = &mut v8::ContextScope::new(scope, a);
            install_generated_bindings(scope, a, da, RealmId(101), 1).unwrap()
        };
        let sb = {
            let scope = &mut v8::ContextScope::new(scope, b);
            install_generated_bindings(scope, b, db, RealmId(102), 1).unwrap()
        };
        sa.set_security_origin("https://source.test".into());
        sb.set_security_origin("https://target.test".into());
        let scope = &mut v8::ContextScope::new(scope, a);
        let source = NativeDragRealm::new(sa, ga);
        let target = NativeDragRealm::new(sb, gb);
        let sn = source.state.dom.borrow().find_by_id("source").unwrap();
        let tn = target.state.dom.borrow().find_by_id("target").unwrap();
        source.enter(scope,|scope|script(scope,"globalThis.events=[];source.ondragstart=e=>events.push('start');source.ondrag=e=>events.push('drag');source.ondragend=e=>events.push('end:'+e.dataTransfer.dropEffect)")).unwrap();
        target.enter(scope,|scope|script(scope,"globalThis.events=[];target.ondragenter=e=>{if(e.view!==window||!e.isTrusted||e.relatedTarget!==null||e.dataTransfer.getData('text')!=='')throw Error('protected enter');events.push('enter')};target.ondragover=e=>{globalThis.old=e.dataTransfer;e.preventDefault();e.dataTransfer.dropEffect='copy';events.push('over')};target.ondrop=e=>{if(e.dataTransfer.getData('text')!=='payload'||old.getData('text')!=='')throw Error('event association');globalThis.drop=e.dataTransfer;e.preventDefault();events.push('drop')}")).unwrap();
        let mut store = Store::default();
        store.set_data("text/plain", "payload".encode_utf16().collect());
        let session = source
            .enter(scope, |scope| {
                Session::begin(scope, &source.state, sn, store, &Default::default())
            })
            .unwrap()
            .unwrap();
        *source.state.native_drag.borrow_mut() = Some(session);
        let mut advance = NativeCrossFrameDrag::new(
            &source,
            Some((&target, tn)),
            NativeCrossFrameDragAction::Advance,
            &Default::default(),
            &Default::default(),
            None,
        )
        .unwrap();
        for _ in 0..3 {
            native_platform::publication::fail_after_next_callback();
            assert!(advance.poll(scope).is_err());
        }
        assert_eq!(advance.poll(scope).unwrap(), Some(DragEffect::Copy));
        assert_eq!(advance.poll(scope).unwrap(), Some(DragEffect::Copy));
        let mut drop = NativeCrossFrameDrag::new(
            &source,
            Some((&target, tn)),
            NativeCrossFrameDragAction::Drop,
            &Default::default(),
            &Default::default(),
            None,
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(drop.poll(scope).is_err());
        source.state.set_execution_suspended(true);
        assert_eq!(drop.poll(scope).unwrap(), None);
        source.state.set_execution_suspended(false);
        native_platform::publication::fail_after_next_callback();
        assert!(drop.poll(scope).is_err());
        assert_eq!(drop.poll(scope).unwrap(), Some(DragEffect::Copy));
        assert_eq!(drop.poll(scope).unwrap(), Some(DragEffect::Copy));
        assert!(!source.state.native_drag_dispatching.get());
        target.enter(scope,|scope|script(scope,"if(events.join('|')!=='enter|over|drop'||drop.getData('text')!=='')throw Error('transport cleanup')")).unwrap();
        source
            .enter(scope, |scope| {
                script(
                    scope,
                    "if(events.join('|')!=='start|drag|end:copy')throw Error('source ordering')",
                )
            })
            .unwrap();
        assert!(!native_drag_session_active(&source.state));
    }
}

#[cfg(test)]
mod ancestor_tests {
    use super::*;
    #[test]
    fn document_authority_change_revokes_drag_session_source_target_and_ancestor() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let global = v8::Global::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<input id=source value=abc><iframe id=frame></iframe>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(221), 1).unwrap();
        let node = state.dom.borrow().find_by_id("source").unwrap();
        let frame = state.dom.borrow().find_by_id("frame").unwrap();
        let realm = NativeDragRealm::new(state.clone(), global);
        let ancestor = NativeDragAncestor::new(state.clone(), frame).unwrap();
        let init = EventInit::default();
        let target = Target::new(&realm, node, &init);
        let source = SelectedSource {
            kind: SelectedKind::Control {
                node,
                start: 0,
                end: 1,
                value: "abc".into(),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([node]),
        };
        let mut session = Session::begin(scope, &state, node, Store::default(), &init)
            .unwrap()
            .unwrap();
        session.negotiation.prepare(
            Some(node),
            DragEffect::Copy,
            &mut session.store.borrow_mut(),
        );
        assert!(
            source.valid(&state)
                && session.live(&state)
                && realm.live()
                && ancestor.live()
                && target.realm().is_some()
        );
        *state.native_drag.borrow_mut() = Some(session);
        state
            .document_authority_epoch
            .set(state.document_authority_epoch().checked_add(1).unwrap());
        assert!(!source.valid(&state));
        assert!(!realm.live());
        assert!(!ancestor.live());
        assert!(target.realm().is_none());
        assert!(!native_drag_session_active(&state));
        assert!(native_drag_target_identity(&state).is_none());
        assert!(NativeDragRealm::new(state.clone(), realm.context.clone()).live());
        assert!(
            NativeDragAncestor::new(state.clone(), frame)
                .unwrap()
                .live()
        );
        state.native_drag.borrow_mut().take();
    }
    #[test]
    fn removing_and_reinserting_embedding_does_not_restore_drag_authority() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=parent><iframe id=frame></iframe></div>")
                .into_runtime()
                .unwrap(),
        ));
        let frame = dom.borrow().find_by_id("frame").unwrap();
        let parent = dom.borrow().find_by_id("parent").unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(121), 1).unwrap();
        let ancestor = NativeDragAncestor::new(state.clone(), frame).unwrap();
        assert!(ancestor.live());
        state.dom.borrow_mut().remove_child(parent, frame).unwrap();
        state.dom.borrow_mut().append_child(parent, frame).unwrap();
        assert!(!ancestor.live());
    }
    #[test]
    fn element_drag_resolution_preserves_native_defaults_and_explicit_false() {
        super::super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(html::parse("<div draggable=true id=drag><span id=text>text</span></div><img draggable=false id=no><a href=/target id=link>link</a>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(122), 1).unwrap();
        let node = |id: &str| state.dom.borrow().find_by_id(id).unwrap();
        assert_eq!(
            element_drag_source(&state, node("text")).unwrap(),
            Some(node("drag"))
        );
        assert_eq!(element_drag_source(&state, node("no")).unwrap(), None);
        assert_eq!(
            element_drag_source(&state, node("link")).unwrap(),
            Some(node("link"))
        );
    }
}
