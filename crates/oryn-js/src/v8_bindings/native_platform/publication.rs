//! Retained native notification dispatch. A failed allocation/graph update
//! resumes the same event and phase snapshot; consumed callbacks never replay.
use super::*;
use std::collections::VecDeque;

pub(in super::super) enum Payload {
    Csp(crate::platform_control::CspEvent),
    LocalCsp(crate::platform_control::CspEvent),
    Storage(crate::platform_control::StorageEventRecord),
    Visibility(bool),
    LifecycleVisibility(bool),
    WindowMove(i32, i32),
    Permission(u64, PermissionState),
    Platform {
        target: u64,
        name: String,
        event: v8::Global<v8::Object>,
    },
    Dom {
        target: v8::Global<v8::Object>,
        name: String,
        event: v8::Global<v8::Object>,
    },
}
#[derive(Clone, Copy)]
enum Target {
    Document,
    Global,
    Platform(u64),
    Node(usize),
}
#[derive(Clone, Copy)]
struct Phase {
    target: Target,
    capture: bool,
    number: u16,
    property: bool,
}
enum Listener {
    Node(EventListener),
    Platform(PlatformEventListener),
}
struct Entry {
    listener: Listener,
    // Once removal can precede a fallible graph refresh. Keep the callback
    // alive and remember removal so retry neither skips nor removes it twice.
    keepalive: Option<v8::Global<v8::Value>>,
    removed: bool,
    method_resolved: bool,
    method: Option<(v8::Global<v8::Value>, v8::Global<v8::Value>)>,
}
pub(in super::super) struct Notification {
    creator: Option<crate::worker_wire::Creator>,
    local: Option<(NodeId, u64)>,
    payload: Payload,
    event: Option<v8::Global<v8::Object>>,
    target: Option<v8::Global<v8::Object>>,
    phase: usize,
    idb_path: Option<Vec<u64>>,
    idb_callback_checkpoint_pending: bool,
    threw: bool,
    listeners: Option<VecDeque<Entry>>,
    invocation_type: Option<String>,
    property_done: bool,
    property_callback: Option<v8::Global<v8::Value>>,
    property_result: Option<v8::Global<v8::Value>>,
    listener_result: Option<v8::Global<v8::Value>>,
    initialized: bool,
    event_id: Option<u64>,
    cancelled: bool,
    dom_path: Vec<v8::Global<v8::Object>>,
    dom_phases: Option<Vec<Phase>>,
    clear_target: bool,
    parser_stream: Option<u64>,
    suppress_parser_insertion: bool,
    legacy_document_target: bool,
    unload_guard: RefCell<Option<UnloadGuard>>,
    unload_timing: Option<crate::navigation_timing::UnloadTiming>,
    navigation_timing: Option<(
        crate::navigation_timing::Stage,
        crate::navigation_timing::Stage,
    )>,
    initial_pageshow: Option<bool>,
    visibility_snapshot: Option<(bool, bool)>,
}
pub(in crate::v8_bindings) fn finish_idb_task_checkpoint(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
        let ceiling = registry.instances.borrow().keys().next_back().copied();
        let mut cursor = None;
        loop {
            let next = {
                let instances = registry.instances.borrow();
                match cursor {
                    None => instances.iter().next(),
                    Some(id) => instances
                        .range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                        .next(),
                }
                .filter(|(id, _)| ceiling.is_some_and(|ceiling| **id <= ceiling))
                .map(|(id, owner)| (*id, owner.clone()))
            };
            let Some((id, owner)) = next else { break };
            cursor = Some(id);
            // No registry borrow may survive last-Rc destruction: retirement
            // can cancel native transports and invoke their cleanup closures.
            if let Some(owner) = owner.upgrade() {
                owner.finish_platform_task_checkpoint()?;
            }
        }
    }
    // The target remains covered if no isolate registry is installed.
    if scope.get_slot::<BindingStateRegistry>().is_none() {
        state.finish_platform_task_checkpoint()?;
    }
    Ok(())
}

impl Notification {
    fn finish_idb_callback(
        &mut self,
        scope: &mut v8::PinScope,
        state: &V8BindingState,
    ) -> Result<bool, V8BindingError> {
        if !self.idb_callback_checkpoint_pending {
            return Ok(true);
        }
        // Terminal transaction events have an algorithm-owned cleanup suffix
        // (openRequest.transaction removal) before their microtask checkpoint.
        // Only request-event invocation cleanup belongs between listeners here.
        let request_target = self
            .idb_path
            .as_ref()
            .and_then(|path| path.first())
            .is_some_and(|id| {
                matches!(
                    state.platform_objects.borrow().get(id),
                    Some(PlatformObjectRecord::IdbRequest(_))
                )
            });
        if !request_target {
            self.idb_callback_checkpoint_pending = false;
            return Ok(true);
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        // Synchronous dispatch from an author stack is not a task boundary.
        if !v8::StackTrace::current_stack_trace(scope, 1)
            .is_some_and(|trace| trace.get_frame_count() == 0)
        {
            self.idb_callback_checkpoint_pending = false;
            return Ok(true);
        }
        let protected = self
            .idb_path
            .as_ref()
            .and_then(|path| path.first())
            .and_then(|id| match state.platform_objects.borrow().get(id) {
                Some(PlatformObjectRecord::IdbRequest(request)) => request.transaction_id,
                _ => None,
            });
        // Microtask cleanup can reenter native dispatch; protection must remain
        // visible to every nested task cleanup, not just the local snapshot.
        struct Protection<'a>(&'a V8BindingState, Option<u64>);
        impl Drop for Protection<'_> {
            fn drop(&mut self) {
                if let Some(id) = self.1 {
                    let mut protected = self.0.idb_callback_protection.borrow_mut();
                    if let Some(depth) = protected.get_mut(&id) {
                        *depth -= 1;
                        if *depth == 0 {
                            protected.remove(&id);
                        }
                    }
                }
            }
        }
        if let Some(id) = protected {
            let mut protection = state.idb_callback_protection.borrow_mut();
            let depth = protection
                .get(&id)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("IDB callback depth"))?;
            protection.insert(id, depth);
        }
        let _protection = Protection(state, protected);
        // Consume the checkpoint before running microtasks: suspension may
        // retain a suffix, but must not replay an already executed callback.
        self.idb_callback_checkpoint_pending = false;
        scope.perform_microtask_checkpoint();
        finish_idb_task_checkpoint(scope, state)?;
        Ok(!state.execution_suspended() && !scope.is_execution_terminating())
    }
    pub(in super::super) fn new(creator: crate::worker_wire::Creator, payload: Payload) -> Self {
        Self {
            creator: Some(creator),
            local: None,
            payload,
            event: None,
            target: None,
            phase: 0,
            idb_path: None,
            idb_callback_checkpoint_pending: false,
            threw: false,
            listeners: None,
            invocation_type: None,
            property_done: false,
            property_callback: None,
            property_result: None,
            listener_result: None,
            initialized: false,
            event_id: None,
            cancelled: false,
            dom_path: Vec::new(),
            dom_phases: None,
            clear_target: false,
            parser_stream: None,
            suppress_parser_insertion: false,
            legacy_document_target: false,
            unload_guard: RefCell::new(None),
            unload_timing: None,
            navigation_timing: None,
            initial_pageshow: None,
            visibility_snapshot: None,
        }
    }
    fn consume_beforeunload_result(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        id: u64,
    ) -> Result<bool, V8BindingError> {
        if !self.current(state)? {
            self.cancelled = true;
            return Ok(true);
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        if let Some(value) = self.listener_result.take() {
            let value = v8::Local::new(scope, &value);
            if !value.is_null_or_undefined() {
                let (converted, exception, message) = {
                    v8::tc_scope!(let tc,scope);
                    let converted = value
                        .to_string(tc)
                        .and_then(|value| rust_string_preserving_utf16(tc, value).ok());
                    let exception = tc.exception();
                    let message = tc.message();
                    if !tc.is_execution_terminating() {
                        tc.reset();
                    }
                    (converted, exception, message)
                };
                if let Some(exception) = exception
                    && !scope.is_execution_terminating()
                {
                    let _ = report_exception(scope, exception, message);
                }
                if !self.current(state)? {
                    self.cancelled = true;
                    return Ok(true);
                }
                if let Some(value) = converted {
                    let mut records = state.events.borrow_mut();
                    if let Some(record) = records.get_mut(&id)
                        && let Some(old) = record.before_unload_return_value.as_mut()
                    {
                        if old.is_empty() {
                            *old = value;
                        }
                        record.default_prevented = true;
                    }
                }
            }
        }
        Ok(!state.execution_suspended() && !scope.is_execution_terminating())
    }
    pub(in super::super) fn without_parser_insertion_point(mut self) -> Self {
        self.suppress_parser_insertion = true;
        self
    }
    pub(in super::super) fn for_parser_stream(mut self, stream: u64) -> Self {
        self.parser_stream = Some(stream);
        self
    }
    pub(in super::super) fn for_unload_timing(
        mut self,
        timing: crate::navigation_timing::UnloadTiming,
    ) -> Self {
        self.unload_timing = Some(timing);
        self
    }
    pub(in super::super) fn for_navigation_timing(
        mut self,
        start: crate::navigation_timing::Stage,
        end: crate::navigation_timing::Stage,
    ) -> Self {
        self.navigation_timing = Some((start, end));
        self
    }
    pub(in super::super) fn for_initial_pageshow(mut self) -> Self {
        self.initial_pageshow = Some(false);
        self
    }
    pub(in super::super) fn with_document_target(mut self) -> Self {
        self.legacy_document_target = true;
        self
    }
    fn current(&self, state: &V8BindingState) -> Result<bool, V8BindingError> {
        if self
            .parser_stream
            .is_some_and(|stream| stream != state.document_stream_epoch())
        {
            return Ok(false);
        }
        if state.execution_retired() {
            return Ok(false);
        }
        if let Some(creator) = &self.creator {
            return Ok(observed_creator(state) == *creator);
        }
        Ok(self.local == Some((state.document_root(), state.document_authority_epoch())))
    }
    fn name(&self) -> &str {
        match &self.payload {
            Payload::Csp(_) | Payload::LocalCsp(_) => "securitypolicyviolation",
            Payload::Storage(_) => "storage",
            Payload::Visibility(_) | Payload::LifecycleVisibility(_) => "visibilitychange",
            Payload::WindowMove(..) => "move",
            Payload::Permission(..) => "change",
            Payload::Platform { name, .. } | Payload::Dom { name, .. } => name,
        }
    }
    fn phases(&self, state: &V8BindingState) -> Vec<Phase> {
        if let Some(path) = &self.idb_path {
            let target = path[0];
            return path
                .iter()
                .rev()
                .map(|id| Phase {
                    target: Target::Platform(*id),
                    capture: true,
                    number: if *id == target { 2 } else { 1 },
                    property: false,
                })
                .chain(path.iter().map(|id| Phase {
                    target: Target::Platform(*id),
                    capture: false,
                    number: if *id == target { 2 } else { 3 },
                    property: false,
                }))
                .collect();
        }
        if let Some(phases) = &self.dom_phases {
            return phases.clone();
        }
        let document = matches!(
            self.payload,
            Payload::Visibility(_) | Payload::LifecycleVisibility(_)
        ) || matches!(self.payload, Payload::Csp(_) | Payload::LocalCsp(_))
            && state.binding_global == BindingGlobal::Window;
        if document {
            return vec![
                Phase {
                    target: Target::Global,
                    capture: true,
                    number: 1,
                    property: false,
                },
                Phase {
                    target: Target::Document,
                    capture: true,
                    number: 2,
                    property: false,
                },
                Phase {
                    target: Target::Document,
                    capture: false,
                    number: 2,
                    property: true,
                },
                Phase {
                    target: Target::Global,
                    capture: false,
                    number: 3,
                    property: false,
                },
            ];
        }
        let target = match self.payload {
            Payload::Platform { target: id, .. } if id == global_events::GLOBAL_TARGET => {
                Target::Global
            }
            Payload::Permission(id, _) | Payload::Platform { target: id, .. } => {
                Target::Platform(id)
            }
            _ => Target::Global,
        };
        vec![
            Phase {
                target,
                capture: true,
                number: 2,
                property: false,
            },
            Phase {
                target,
                capture: false,
                number: 2,
                property: false,
            },
        ]
    }
    fn receiver<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        state: &Rc<V8BindingState>,
        target: Target,
    ) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
        match target {
            Target::Node(index) => self
                .dom_path
                .get(index)
                .map(|object| v8::Local::new(scope, object))
                .ok_or(V8BindingError::Allocation("notification path node")),
            Target::Document => wrap_node(scope, state, state.document_root()),
            Target::Global => state
                .global_object
                .to_local(scope)
                .ok_or(V8BindingError::Allocation("notification global")),
            Target::Platform(id) => {
                let interface = platform_interface_for_id(state, id)
                    .ok_or(V8BindingError::Allocation("notification target"))?;
                new_platform_object(scope, state, interface, id)
            }
        }
    }
    fn prepare(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<(), V8BindingError> {
        if self.event.is_none() {
            let event = match &self.payload {
                Payload::Csp(value) | Payload::LocalCsp(value) => {
                    csp_events::create(scope, state, value)?
                }
                Payload::Storage(value) => storage_events::create(scope, state, value)?,
                Payload::Platform { event, .. } | Payload::Dom { event, .. } => {
                    v8::Local::new(scope, event)
                }
                _ => create_event_object(
                    scope,
                    state,
                    self.name(),
                    matches!(
                        self.payload,
                        Payload::Visibility(_) | Payload::LifecycleVisibility(_)
                    ),
                    false,
                )?,
            };
            self.event = Some(v8::Global::new(scope, event));
        }
        if self.initialized {
            return Ok(());
        }
        let event = v8::Local::new(scope, self.event.as_ref().unwrap());
        let id = event_id(scope, event)?;
        if matches!(self.payload, Payload::Dom { .. }) {
            return self.prepare_dom_path(scope, state, event, id);
        }
        let phases = self.phases(state);
        // Capture phases begin at an ancestor, not at the event target. An
        // IDB transaction/database path also has four phases, like a Document.
        let target = if let Some(path) = &self.idb_path {
            Target::Platform(path[0])
        } else if phases.len() == 4 {
            Target::Document
        } else {
            phases[0].target
        };
        let receiver = self.receiver(scope, state, target)?;
        let path = if matches!(target, Target::Document) {
            vec![
                receiver.into(),
                self.receiver(scope, state, Target::Global)?.into(),
            ]
        } else {
            vec![receiver.into()]
        };
        let path = if let Some(ids) = &self.idb_path {
            ids.iter()
                .map(|id| {
                    self.receiver(scope, state, Target::Platform(*id))
                        .map(Into::into)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            path
        };
        set_event_path(scope, event, &path)?;
        #[cfg(test)]
        fail_at(FailurePoint::Prepare)?;
        let event_target = if self.legacy_document_target {
            wrap_node(scope, state, state.document_root())?
        } else {
            receiver
        };
        if !matches!(target, Target::Platform(_)) {
            event.set_internal_field(4, event_target.into());
        }
        self.target = Some(v8::Global::new(scope, receiver));
        let mut events = state.events.borrow_mut();
        let record = events
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("notification event"))?;
        record.dispatching = true;
        record.is_trusted = true;
        if let Some(path) = &self.idb_path {
            record.platform_path = path.clone();
        }
        record.target = (self.legacy_document_target || matches!(target, Target::Document))
            .then(|| state.document_root());
        record.platform_target = if self.legacy_document_target {
            None
        } else {
            match target {
                Target::Document | Target::Node(_) => None,
                Target::Global => Some(global_events::GLOBAL_TARGET),
                Target::Platform(id) => Some(id),
            }
        };
        drop(events);
        // Observed properties change with publication, before the first callback.
        match self.payload {
            Payload::WindowMove(x, y) => {
                state.native_platform.borrow_mut().window_position = Some((x, y))
            }
            Payload::Visibility(_) | Payload::LifecycleVisibility(_) => {
                let (backend, hidden) = self
                    .visibility_snapshot
                    .expect("visibility snapshot before preparation");
                let mut platform = state.native_platform.borrow_mut();
                platform.backend_hidden = backend;
                platform.hidden = hidden;
                if let Payload::LifecycleVisibility(value) = self.payload {
                    platform.lifecycle_hidden = value;
                }
            }
            Payload::Permission(id, current) => {
                if let Some(permission) =
                    state.native_platform.borrow_mut().permissions.get_mut(&id)
                {
                    permission.state = current;
                }
            }
            _ => {}
        }
        self.event_id = Some(id);
        self.initialized = true;
        // This native publication commits once before author callbacks. Retry
        // of a partially dispatched event must not enqueue another report.
        if let Payload::Csp(value) | Payload::LocalCsp(value) = &self.payload {
            reporting::csp(state, value);
        }
        Ok(())
    }
    fn prepare_dom_path<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        state: &Rc<V8BindingState>,
        event: v8::Local<'s, v8::Object>,
        id: u64,
    ) -> Result<(), V8BindingError> {
        // DOM and platform notifications share the same preparation boundary;
        // neither has begun dispatch when this injected failure is observed.
        #[cfg(test)]
        fail_at(FailurePoint::Prepare)?;
        let Payload::Dom { target, .. } = &self.payload else {
            unreachable!()
        };
        let target = v8::Local::new(scope, target);
        let (owner, node) = node_identity::argument(scope, target.into())?
            .ok_or(V8BindingError::Allocation("notification DOM target"))?;
        if !Rc::ptr_eq(&owner.dom, &state.dom) {
            return Err(V8BindingError::Allocation(
                "notification DOM owner changed before dispatch",
            ));
        }
        let composed = state
            .events
            .borrow()
            .get(&id)
            .ok_or(V8BindingError::Allocation("notification event"))?
            .composed;
        let related = event_related_target(scope, event)
            .ok()
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .and_then(|value| {
                let owner = binding_state_for_receiver(scope, value)?;
                if !Rc::ptr_eq(&owner.dom, &state.dom) {
                    return None;
                }
                embedded_node_identity(scope, value).map(|identity| identity.0)
            });
        if let Some(related) = related
            && related != node
            && state.dom.borrow().retarget_node(related, Some(node))? == node
        {
            self.dom_phases = Some(Vec::new());
            self.initialized = true;
            self.event_id = Some(id);
            self.clear_target = true;
            return Ok(());
        }

        let (nodes, flags) = {
            let dom = state.dom.borrow();
            let origin = dom.control_tree_root(node)?;
            let mut nodes = Vec::new();
            let mut flags = Vec::new();
            let mut current = node;
            let mut adjusted = node;
            let mut closed_slot = false;
            let mut seen = BTreeSet::new();
            loop {
                if !seen.insert(current) {
                    return Err(V8BindingError::Allocation("notification event path cycle"));
                }
                let record = dom.arena().get(current)?;
                let mut flag = if closed_slot { 8 } else { 0 };
                if let NodeKind::ShadowRoot { mode, .. } = record.kind {
                    flag |= if mode == ShadowMode::Closed { 2 } else { 1 };
                    if dom.control_tree_root(adjusted)? == current {
                        flag |= 4;
                        if (composed || current != origin)
                            && let Some(host) = record.parent
                        {
                            adjusted = host;
                        }
                    }
                }
                nodes.push(current);
                flags.push(flag);
                if matches!(record.kind, NodeKind::Document)
                    || !composed
                        && current == origin
                        && matches!(record.kind, NodeKind::ShadowRoot { .. })
                {
                    break;
                }
                let slot = dom.assigned_slot(current, false)?;
                closed_slot = slot.is_some_and(|slot| {
                    dom.control_tree_root(slot)
                        .ok()
                        .and_then(|root| dom.arena().get(root).ok())
                        .is_some_and(|record| {
                            matches!(
                                record.kind,
                                NodeKind::ShadowRoot {
                                    mode: ShadowMode::Closed,
                                    ..
                                }
                            )
                        })
                });
                let Some(parent) = slot.or(record.parent) else {
                    break;
                };
                if let Some(related) = related {
                    let adjusted = dom.retarget_node(node, Some(parent))?;
                    if (related != node || adjusted != node)
                        && adjusted == dom.retarget_node(related, Some(parent))?
                    {
                        break;
                    }
                }
                current = parent;
            }
            (nodes, flags)
        };
        self.clear_target = {
            let dom = state.dom.borrow();
            let target = dom.retarget_node(node, nodes.last().copied())?;
            matches!(
                dom.arena().get(dom.control_tree_root(target)?)?.kind,
                NodeKind::ShadowRoot { .. }
            )
        };
        let reaches = nodes.contains(&state.document_root());
        let mut objects = Vec::new();
        for node in &nodes {
            objects.push(wrap_node(scope, state, *node)?);
        }
        let mut path = objects
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<v8::Local<v8::Value>>>();
        if reaches {
            path.push(self.receiver(scope, state, Target::Global)?.into());
        }
        set_event_path(scope, event, &path)?;
        capture_event_related_path(scope, state, event, node, &nodes[1..], reaches)?;
        touch_objects::capture_path(scope, state, event, node, &nodes[1..], reaches)?;
        let mut flags = flags
            .into_iter()
            .map(|flag| v8::Integer::new(scope, flag).into())
            .collect::<Vec<v8::Local<v8::Value>>>();
        if reaches {
            flags.push(v8::Integer::new(scope, 0).into());
        }
        let flags = v8::Array::new_with_elements(scope, &flags);
        let key = event_boundaries_key(scope)?;
        event
            .set_private(scope, key, flags.into())
            .filter(|set| *set)
            .ok_or(V8BindingError::Allocation("notification event boundaries"))?;
        self.dom_path = objects
            .into_iter()
            .map(|object| v8::Global::new(scope, object))
            .collect();
        let mut phases = Vec::new();
        if reaches {
            phases.push(Phase {
                target: Target::Global,
                capture: true,
                number: 1,
                property: false,
            });
        }
        for index in (1..nodes.len()).rev() {
            phases.push(Phase {
                target: Target::Node(index),
                capture: true,
                number: 1,
                property: false,
            });
        }
        phases.push(Phase {
            target: Target::Node(0),
            capture: true,
            number: 2,
            property: false,
        });
        phases.push(Phase {
            target: Target::Node(0),
            capture: false,
            number: 2,
            property: true,
        });
        for index in 1..nodes.len() {
            phases.push(Phase {
                target: Target::Node(index),
                capture: false,
                number: 3,
                property: true,
            });
        }
        if reaches {
            phases.push(Phase {
                target: Target::Global,
                capture: false,
                number: 3,
                property: false,
            });
        }
        self.dom_phases = Some(phases);
        self.target = Some(v8::Global::new(scope, target));
        event.set_internal_field(4, target.into());
        let mut events = state.events.borrow_mut();
        let record = events.get_mut(&id).unwrap();
        record.target = Some(node);
        record.platform_target = None;
        record.is_trusted = true;
        record.dispatching = true;
        drop(events);
        self.event_id = Some(id);
        self.initialized = true;
        Ok(())
    }
    fn snapshot(
        &self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        phase: &Phase,
        name: &str,
    ) -> Result<VecDeque<Entry>, V8BindingError> {
        if let Target::Node(_) = phase.target {
            let receiver = self.receiver(scope, state, phase.target)?;
            let (owner, node) = node_identity::argument(scope, receiver.into())?
                .ok_or(V8BindingError::Allocation("notification node owner"))?;
            let entries = owner
                .listeners
                .borrow()
                .get(&(node, name.into()))
                .cloned()
                .unwrap_or_default();
            return Ok(entries
                .into_iter()
                .filter(|entry| entry.capture == phase.capture)
                .map(|listener| Entry {
                    keepalive: Some(listener.callback.clone()),
                    listener: Listener::Node(listener),
                    removed: false,
                    method_resolved: false,
                    method: None,
                })
                .collect());
        }
        Ok(match phase.target {
            Target::Document => state
                .listeners
                .borrow()
                .get(&(state.document_root(), name.into()))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|entry| entry.capture == phase.capture)
                .map(|listener| Entry {
                    keepalive: Some(listener.callback.clone()),
                    listener: Listener::Node(listener),
                    removed: false,
                    method_resolved: false,
                    method: None,
                })
                .collect(),
            target => {
                let id = match target {
                    Target::Global => global_events::GLOBAL_TARGET,
                    Target::Platform(id) => id,
                    _ => unreachable!(),
                };
                state
                    .platform_listeners
                    .borrow()
                    .get(&(id, name.into()))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|entry| entry.capture == phase.capture)
                    .map(|listener| {
                        let keepalive = listener
                            .callback
                            .local(scope)
                            .map(|value| v8::Global::new(scope, value));
                        Entry {
                            listener: Listener::Platform(listener),
                            keepalive,
                            removed: false,
                            method_resolved: false,
                            method: None,
                        }
                    })
                    .collect()
            }
        })
    }
    pub(in super::super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        if !self.current(state)? {
            self.cancelled = true;
            return Ok(true);
        }
        if !self.finish_idb_callback(scope, state)? {
            return Ok(false);
        }
        struct InsertionGuard(Rc<V8BindingState>, Option<(NodeId, u64, u64, u64)>);
        impl Drop for InsertionGuard {
            fn drop(&mut self) {
                self.0.parser_insertion_suppression.set(self.1);
            }
        }
        let _insertion = if self.suppress_parser_insertion {
            let previous = state.parser_insertion_suppression.replace(Some((
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            )));
            Some(InsertionGuard(state.clone(), previous))
        } else {
            None
        };
        if self.unload_timing.is_some() && self.unload_guard.get_mut().is_none() {
            *self.unload_guard.get_mut() = Some(state.begin_unload());
        }
        let _destructive_writes = self
            .unload_timing
            .as_ref()
            .map(|_| state.ignore_destructive_writes());
        if !self.initialized && self.visibility_snapshot.is_none() {
            let snapshot = match self.payload {
                Payload::Visibility(backend) => Some((backend, effective_hidden(state, backend))),
                Payload::LifecycleVisibility(value) => {
                    Some(visibility_snapshot(state, Some(value))?)
                }
                _ => None,
            };
            if let Some((backend, hidden)) = snapshot {
                self.visibility_snapshot = Some((backend, hidden));
                let previous = state.native_platform.borrow().hidden
                    || state.document_prerendering()
                    || matches!(self.payload, Payload::LifecycleVisibility(_)) && backend;
                if previous == hidden {
                    let mut platform = state.native_platform.borrow_mut();
                    platform.backend_hidden = backend;
                    platform.hidden = hidden;
                    if let Payload::LifecycleVisibility(value) = self.payload {
                        platform.lifecycle_hidden = value;
                    }
                    self.cancelled = true;
                    return Ok(true);
                }
            }
        }
        self.prepare(scope, state)?;
        let event = v8::Local::new(scope, self.event.as_ref().unwrap());
        let id = event_id(scope, event)?;
        let phases = self.phases(state);
        while self.phase < phases.len() {
            if !self.current(state)? {
                self.cancelled = true;
                return Ok(true);
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if self.initial_pageshow == Some(false) {
                if state.initial_pageshow_delivered.replace(true) {
                    self.cancelled = true;
                    return Ok(true);
                }
                self.initial_pageshow = Some(true);
            }
            if let Some(timing) = &self.unload_timing {
                timing.start();
            }
            if let Some((start, _)) = self.navigation_timing {
                state.record_navigation_timing_stage(start);
            }
            let phase = &phases[self.phase];
            let (stopped, immediate, bubbles) = {
                let records = state.events.borrow();
                let record = records.get(&id).unwrap();
                (
                    record.propagation_stopped,
                    record.immediate_stopped,
                    record.bubbles,
                )
            };
            if immediate
                || stopped
                    && self.listeners.is_none()
                    && !(self.phase > 0 && phase.number == 2 && phases[self.phase - 1].number == 2)
            {
                break;
            }
            let receiver = self.receiver(scope, state, phase.target)?;
            let node_target = if matches!(phase.target, Target::Document | Target::Node(_)) {
                node_identity::argument(scope, receiver.into())?
            } else {
                None
            };
            let at_host = if self.dom_phases.is_some() {
                retarget_event_for_listener(scope, event, receiver.into())?
            } else {
                false
            };
            if phase.number == 3 && !bubbles && !at_host {
                self.phase += 1;
                self.listeners = None;
                self.invocation_type = None;
                self.property_done = false;
                self.property_callback = None;
                self.property_result = None;
                continue;
            }
            event.set_internal_field(5, receiver.into());
            {
                let mut records = state.events.borrow_mut();
                let record = records.get_mut(&id).unwrap();
                record.phase = if at_host { 2 } else { phase.number };
                record.current_target = node_target.as_ref().map(|(_, node)| *node);
                record.platform_current_target = match phase.target {
                    Target::Document | Target::Node(_) => None,
                    Target::Global => Some(global_events::GLOBAL_TARGET),
                    Target::Platform(id) => Some(id),
                };
            }
            if self.listeners.is_none() {
                let key = web_animations::invocation_type(
                    scope,
                    state,
                    receiver,
                    state,
                    id,
                    self.name(),
                )?;
                self.listeners = Some(self.snapshot(scope, state, phase, &key)?);
                self.invocation_type = Some(key);
            }
            let key = self
                .invocation_type
                .clone()
                .unwrap_or_else(|| self.name().to_owned());
            let _event_type = web_animations::InvocationTypeGuard::new(state, id, &key);
            loop {
                if self.listener_result.is_some() {
                    if !self.consume_beforeunload_result(scope, state, id)? {
                        return Ok(false);
                    }
                    if self.cancelled {
                        return Ok(true);
                    }
                }
                let Some(entry) = self.listeners.as_mut().unwrap().front_mut() else {
                    break;
                };
                if state.execution_suspended() || scope.is_execution_terminating() {
                    return Ok(false);
                }
                if state.events.borrow().get(&id).unwrap().immediate_stopped {
                    break;
                }
                #[cfg(test)]
                fail_at(FailurePoint::BeforeCallback)?;
                let (capture, once, passive, signal, registration, handler) = match &entry.listener
                {
                    Listener::Node(listener) => (
                        listener.capture,
                        listener.once,
                        listener.passive,
                        listener.signal.as_ref(),
                        listener.registration_id,
                        false,
                    ),
                    Listener::Platform(listener) => (
                        listener.capture,
                        listener.once,
                        listener.passive,
                        listener.signal.as_ref(),
                        listener.registration_id,
                        listener.global_handler,
                    ),
                };
                let _ = capture;
                let platform = match phase.target {
                    Target::Document | Target::Node(_) => None,
                    Target::Global => Some(global_events::GLOBAL_TARGET),
                    Target::Platform(id) => Some(id),
                };
                let node_owner = if platform.is_none() {
                    Some(
                        node_identity::argument(scope, receiver.into())?
                            .ok_or(V8BindingError::Allocation("notification current owner"))?,
                    )
                } else {
                    None
                };
                let active = entry.removed
                    || match platform {
                        None => {
                            let (owner, node) = node_owner.as_ref().unwrap();
                            owner
                                .listeners
                                .borrow()
                                .get(&(*node, key.clone()))
                                .is_some_and(|list| {
                                    list.iter().any(|item| item.registration_id == registration)
                                })
                        }
                        Some(target) => state
                            .platform_listeners
                            .borrow()
                            .get(&(target, key.clone()))
                            .is_some_and(|list| {
                                list.iter().any(|item| item.registration_id == registration)
                            }),
                    };
                if !entry.method_resolved
                    && (!active || signal.is_some_and(node_event_listeners::aborted))
                {
                    self.listeners.as_mut().unwrap().pop_front();
                    continue;
                }
                let callback = if handler {
                    if platform == Some(global_events::GLOBAL_TARGET) {
                        global_events::handler_value(scope, state, &key)
                    } else {
                        state
                            .platform_handlers
                            .borrow()
                            .get(&(platform.unwrap(), key.clone()))
                            .map(|handler| handler.value(scope))
                            .unwrap_or_else(|| v8::null(scope).into())
                    }
                } else {
                    entry
                        .keepalive
                        .as_ref()
                        .map(|callback| v8::Local::new(scope, callback))
                        .unwrap_or_else(|| v8::null(scope).into())
                };
                if once {
                    if !entry.removed {
                        if let Some(target) = platform {
                            if let Some(list) = state
                                .platform_listeners
                                .borrow_mut()
                                .get_mut(&(target, key.clone()))
                            {
                                list.retain(|item| item.registration_id != registration);
                            }
                        } else {
                            let (owner, node) = node_owner.as_ref().unwrap();
                            if let Some(list) =
                                owner.listeners.borrow_mut().get_mut(&(*node, key.clone()))
                            {
                                list.retain(|item| item.registration_id != registration);
                            }
                        }
                        entry.removed = true;
                    }
                    #[cfg(test)]
                    fail_at(FailurePoint::OnceRemoved)?;
                    if let Some(target) = platform {
                        platform_listener_graph::refresh(scope, state, target)?;
                    } else {
                        let (owner, node) = node_owner.as_ref().unwrap();
                        let present = owner
                            .listeners
                            .borrow()
                            .get(&(*node, key.clone()))
                            .is_some_and(|list| !list.is_empty());
                        owner
                            .dom
                            .borrow_mut()
                            .set_event_listener_presence(*node, &key, present)?;
                    }
                }
                state
                    .events
                    .borrow_mut()
                    .get_mut(&id)
                    .unwrap()
                    .in_passive_listener = passive;
                if !entry.method_resolved {
                    // Preserve the returned method before yielding; once removal
                    // has already committed but invocation has not begun.
                    let (resolved, resolution_threw) =
                        match global_events::resolve_listener_capturing(
                            scope,
                            callback,
                            receiver.into(),
                        ) {
                            Ok(resolved) => resolved,
                            Err(error) => {
                                state
                                    .events
                                    .borrow_mut()
                                    .get_mut(&id)
                                    .unwrap()
                                    .in_passive_listener = false;
                                return Err(error);
                            }
                        };
                    self.threw |= resolution_threw;
                    self.idb_callback_checkpoint_pending |=
                        resolution_threw && self.idb_path.is_some();
                    entry.method = resolved.map(|(function, receiver)| {
                        let function: v8::Local<v8::Value> = function.into();
                        (
                            v8::Global::new(scope, function),
                            v8::Global::new(scope, receiver),
                        )
                    });
                    entry.method_resolved = true;
                }
                if state.execution_suspended() || scope.is_execution_terminating() {
                    state
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .unwrap()
                        .in_passive_listener = false;
                    return Ok(false);
                }
                if !self.current(state)? {
                    state
                        .events
                        .borrow_mut()
                        .get_mut(&id)
                        .unwrap()
                        .in_passive_listener = false;
                    self.cancelled = true;
                    return Ok(true);
                }
                let error_handler = handler
                    && key == "error"
                    && platform == Some(global_events::GLOBAL_TARGET)
                    && error_event_property(scope, state, event, "error").is_ok();
                let arguments = if error_handler {
                    ["message", "filename", "lineno", "colno", "error"]
                        .into_iter()
                        .map(|name| error_event_property(scope, state, event, name))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    vec![event.into()]
                };
                let method = self
                    .listeners
                    .as_mut()
                    .unwrap()
                    .front_mut()
                    .unwrap()
                    .method
                    .take();
                self.listeners.as_mut().unwrap().pop_front();
                let invoked = method.is_some();
                let result = method.and_then(|(function, receiver)| {
                    let function = v8::Local::new(scope, &function);
                    let receiver = v8::Local::new(scope, &receiver);
                    global_events::invoke(scope, function, receiver, &arguments)
                });
                self.threw |= invoked && result.is_none();
                self.idb_callback_checkpoint_pending |= invoked && self.idb_path.is_some();
                {
                    let mut records = state.events.borrow_mut();
                    let record = records.get_mut(&id).unwrap();
                    record.in_passive_listener = false;
                    if handler
                        && key != "beforeunload"
                        && result.is_some_and(|value| {
                            if error_handler {
                                value.is_true()
                            } else {
                                value.is_false()
                            }
                        })
                        && record.cancelable
                    {
                        record.default_prevented = true;
                    }
                }
                if handler && key == "beforeunload" {
                    self.listener_result = result.map(|value| v8::Global::new(scope, value));
                }
                #[cfg(test)]
                if invoked {
                    fail_at(FailurePoint::AfterCallback)?;
                }
                if !self.current(state)? {
                    self.cancelled = true;
                    return Ok(true);
                }
                if !self.finish_idb_callback(scope, state)? {
                    return Ok(false);
                }
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if phase.property
                && !web_animations::is_animation_event(&key)
                && (!self.property_done
                    || self.property_callback.is_some()
                    || self.property_result.is_some())
                && !state.events.borrow().get(&id).unwrap().immediate_stopped
                && node_target.as_ref().is_none_or(|(owner, node)| {
                    !is_window_handler_alias(owner, *node, &web_animations::handler_property(&key))
                })
            {
                if !self.property_done {
                    let property = v8::String::new(scope, &web_animations::handler_property(&key))
                        .ok_or(V8BindingError::Allocation("notification handler"))?;
                    // The getter and returned callback are separate accepted
                    // steps. Parking after the getter retains its exact value.
                    self.property_done = true;
                    let callback = {
                        v8::tc_scope!(let tc,scope);
                        let value = receiver.get(tc, property.into());
                        let exception = tc.exception();
                        self.idb_callback_checkpoint_pending |= self.idb_path.is_some();
                        let message = tc.message();
                        if !tc.is_execution_terminating() {
                            tc.reset();
                        }
                        if let Some(exception) = exception {
                            let _ = report_exception(tc, exception, message);
                        }
                        value
                    };
                    self.property_callback = callback
                        .filter(|value| value.is_function())
                        .map(|value| v8::Global::new(scope, value));
                }
                if !self.current(state)? {
                    self.cancelled = true;
                    return Ok(true);
                }
                if state.execution_suspended() || scope.is_execution_terminating() {
                    return Ok(false);
                }
                let error_handler = key == "error"
                    && matches!(phase.target, Target::Global)
                    && error_event_property(scope, state, event, "error").is_ok();
                let arguments = if error_handler {
                    ["message", "filename", "lineno", "colno", "error"]
                        .into_iter()
                        .map(|name| error_event_property(scope, state, event, name))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    vec![event.into()]
                };
                #[cfg(test)]
                let invoked = self.property_callback.is_some();
                if let Some(callback) = self.property_callback.take() {
                    self.idb_callback_checkpoint_pending = self.idb_path.is_some();
                    let callback = v8::Local::new(scope, &callback);
                    self.property_result =
                        global_events::invoke(scope, callback, receiver.into(), &arguments)
                            .map(|value| v8::Global::new(scope, value));
                }
                if !self.current(state)? {
                    self.cancelled = true;
                    return Ok(true);
                }
                if state.execution_suspended() || scope.is_execution_terminating() {
                    return Ok(false);
                }
                if !self.finish_idb_callback(scope, state)? {
                    return Ok(false);
                }
                if let Some(value) = self.property_result.take() {
                    let value = v8::Local::new(scope, &value);
                    if key == "beforeunload" && !value.is_null_or_undefined() {
                        // Consume the callback return once. DOMString conversion
                        // can invoke author code, so it follows the same parking
                        // and owner checks as the handler itself.
                        let (converted, exception, message) = {
                            v8::tc_scope!(let tc,scope);
                            let converted = value
                                .to_string(tc)
                                .and_then(|value| rust_string_preserving_utf16(tc, value).ok());
                            let exception = tc.exception();
                            let message = tc.message();
                            if !tc.is_execution_terminating() {
                                tc.reset();
                            }
                            (converted, exception, message)
                        };
                        if let Some(exception) = exception
                            && !scope.is_execution_terminating()
                        {
                            let _ = report_exception(scope, exception, message);
                        }
                        if !self.current(state)? {
                            self.cancelled = true;
                            return Ok(true);
                        }
                        if let Some(value) = converted {
                            let mut records = state.events.borrow_mut();
                            if let Some(record) = records.get_mut(&id)
                                && let Some(old) = record.before_unload_return_value.as_mut()
                            {
                                if old.is_empty() {
                                    *old = value;
                                }
                                record.default_prevented = true;
                            }
                        }
                    } else if key != "beforeunload"
                        && (if error_handler {
                            value.is_true()
                        } else {
                            value.is_false()
                        })
                    {
                        let mut records = state.events.borrow_mut();
                        let record = records.get_mut(&id).unwrap();
                        if record.cancelable {
                            record.default_prevented = true;
                        }
                    }
                }
                #[cfg(test)]
                if invoked {
                    fail_at(FailurePoint::AfterCallback)?;
                }
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            self.phase += 1;
            self.listeners = None;
            self.invocation_type = None;
            self.property_done = false;
            self.property_callback = None;
            self.property_result = None;
        }
        if let Some(phases) = self.dom_phases.as_ref() {
            let reaches = phases
                .iter()
                .any(|phase| matches!(phase.target, Target::Global));
            let last = if reaches {
                Some(self.receiver(scope, state, Target::Global)?)
            } else {
                self.dom_path
                    .last()
                    .map(|object| v8::Local::new(scope, object))
            };
            if let Some(last) = last {
                retarget_event_for_listener(scope, event, last.into())?;
            }
            if self.clear_target {
                touch_objects::clear_targets(scope, state, event)?;
                let value = v8::null(scope);
                let related = event_related_path_key(scope)?;
                if event
                    .get_private(scope, related)
                    .is_some_and(|value| value.is_array())
                {
                    set_event_related_target(scope, event, value.into())?;
                }
                state.events.borrow_mut().get_mut(&id).unwrap().target = None;
                event.set_internal_field(4, value.into());
            }
        }
        Ok(true)
    }
    pub(in super::super) fn threw(&self) -> bool {
        self.threw
    }
    pub(in super::super) fn started(&self) -> bool {
        self.initialized && self.event_id.is_some()
    }
    pub(in super::super) fn cancel(&self, state: &V8BindingState) {
        self.unload_guard.borrow_mut().take();
        if let Some(id) = self.event_id {
            drop(EventDispatchGuard { state, id });
        }
    }
    pub(in super::super) fn finish(&self, scope: &mut v8::PinScope, state: &Rc<V8BindingState>) {
        self.unload_guard.borrow_mut().take();
        if !self.cancelled && self.current(state).unwrap_or(false) {
            if let Some(timing) = &self.unload_timing {
                timing.finish();
            }
            if let Some((_, end)) = self.navigation_timing {
                state.record_navigation_timing_stage(end);
            }
        }
        if let Some(event) = &self.event {
            let event = v8::Local::new(scope, event);
            if let Ok(id) = event_id(scope, event) {
                drop(EventDispatchGuard { state, id });
            }
            clear_event_path(scope, event);
        }
    }
}

/// A synchronously dispatched native platform event can retain this same
/// continuation in its owning algorithm without entering the async queue.
pub(in super::super) fn pending_platform_dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: u64,
    name: &str,
    event: v8::Local<v8::Object>,
) -> Notification {
    Notification {
        creator: None,
        local: Some((state.document_root(), state.document_authority_epoch())),
        payload: Payload::Platform {
            target,
            name: name.into(),
            event: v8::Global::new(scope, event),
        },
        event: None,
        target: None,
        phase: 0,
        idb_path: None,
        idb_callback_checkpoint_pending: false,
        threw: false,
        listeners: None,
        invocation_type: None,
        property_done: false,
        property_callback: None,
        property_result: None,
        listener_result: None,
        initialized: false,
        event_id: None,
        cancelled: false,
        dom_path: Vec::new(),
        dom_phases: None,
        clear_target: false,
        parser_stream: None,
        suppress_parser_insertion: false,
        legacy_document_target: false,
        unload_guard: RefCell::new(None),
        unload_timing: None,
        navigation_timing: None,
        initial_pageshow: None,
        visibility_snapshot: None,
    }
}
pub(in super::super) fn prepare_platform_dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: u64,
    name: &str,
    event: v8::Local<v8::Object>,
) -> Result<Notification, V8BindingError> {
    if name.len() > 128 {
        return Err(failure("native event name limit"));
    }
    let mut pending = Notification {
        creator: None,
        local: Some((state.document_root(), state.document_authority_epoch())),
        payload: Payload::Platform {
            target,
            name: name.into(),
            event: v8::Global::new(scope, event),
        },
        event: None,
        target: None,
        phase: 0,
        idb_path: None,
        idb_callback_checkpoint_pending: false,
        threw: false,
        listeners: None,
        invocation_type: None,
        property_done: false,
        property_callback: None,
        property_result: None,
        listener_result: None,
        initialized: false,
        event_id: None,
        cancelled: false,
        dom_path: Vec::new(),
        dom_phases: None,
        clear_target: false,
        parser_stream: None,
        suppress_parser_insertion: false,
        legacy_document_target: false,
        unload_guard: RefCell::new(None),
        unload_timing: None,
        navigation_timing: None,
        initial_pageshow: None,
        visibility_snapshot: None,
    };
    pending.prepare(scope, state)?;
    Ok(pending)
}
pub(in super::super) fn pending_lifecycle_visibility(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    hidden: bool,
) -> Result<Notification, V8BindingError> {
    let event = create_event_object(scope, state, "visibilitychange", true, false)?;
    let mut notification = pending_platform_dispatch(
        scope,
        state,
        global_events::GLOBAL_TARGET,
        "visibilitychange",
        event,
    );
    notification.payload = Payload::LifecycleVisibility(hidden);
    notification.event = Some(v8::Global::new(scope, event));
    Ok(notification)
}
pub(in super::super) fn pending_dom_dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Object>,
    name: &str,
    event: v8::Local<v8::Object>,
) -> Notification {
    Notification {
        creator: None,
        local: Some((state.document_root(), state.document_authority_epoch())),
        payload: Payload::Dom {
            target: v8::Global::new(scope, target),
            name: name.into(),
            event: v8::Global::new(scope, event),
        },
        event: None,
        target: None,
        phase: 0,
        idb_path: None,
        idb_callback_checkpoint_pending: false,
        threw: false,
        listeners: None,
        invocation_type: None,
        property_done: false,
        property_callback: None,
        property_result: None,
        listener_result: None,
        initialized: false,
        event_id: None,
        cancelled: false,
        dom_path: Vec::new(),
        dom_phases: None,
        clear_target: false,
        parser_stream: None,
        suppress_parser_insertion: false,
        legacy_document_target: false,
        unload_guard: RefCell::new(None),
        unload_timing: None,
        navigation_timing: None,
        initial_pageshow: None,
        visibility_snapshot: None,
    }
}

const MAX_PUBLICATIONS: usize = 1024;
const MAX_PUBLICATION_BYTES: usize = 8 * 1024 * 1024;
fn observed_creator(state: &V8BindingState) -> crate::worker_wire::Creator {
    state
        .worker_creator
        .borrow()
        .clone()
        .unwrap_or_else(|| crate::worker_wire::Creator::Window {
            target: state.message_target.borrow().clone(),
            document_generation: state.document_generation(),
            stream_epoch: state.document_authority_epoch(),
        })
}
fn payload_bytes(payload: &Payload) -> usize {
    match payload {
        Payload::Csp(value) | Payload::LocalCsp(value) => [
            &value.document_uri,
            &value.referrer,
            &value.blocked_uri,
            &value.effective_directive,
            &value.original_policy,
            &value.source_file,
            &value.sample,
            &value.disposition,
        ]
        .iter()
        .fold(0usize, |size, value| size.saturating_add(value.len())),
        Payload::Storage(value) => value.url.len().saturating_add(
            [&value.key, &value.old_value, &value.new_value]
                .iter()
                .filter_map(|value| value.as_ref())
                .fold(0usize, |size, value| size.saturating_add(value.len())),
        ),
        Payload::Platform { name, .. } | Payload::Dom { name, .. } => name.len(),
        _ => 0,
    }
}
fn admit(state: &V8BindingState, pending: Notification) -> Result<(), V8BindingError> {
    let mut platform = state.native_platform.borrow_mut();
    let bytes = platform.publications.iter().fold(
        payload_bytes(&pending.payload).saturating_add(platform.publication_bytes),
        |size, value| size.saturating_add(payload_bytes(&value.payload)),
    );
    if platform.publications.len() + usize::from(platform.publishing) >= MAX_PUBLICATIONS
        || bytes > MAX_PUBLICATION_BYTES
    {
        return Err(V8BindingError::Allocation(
            "native notification queue limit",
        ));
    }
    platform
        .publications
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("native notification queue"))?;
    platform.publications.push_back(pending);
    platform.publication_retry_at = None;
    platform.publication_retry_attempt = 0;
    Ok(())
}
pub(in super::super) fn enqueue(
    state: &V8BindingState,
    creator: crate::worker_wire::Creator,
    payload: Payload,
) -> Result<(), V8BindingError> {
    admit(state, Notification::new(creator, payload))
}
pub(in super::super) fn enqueue_local_csp(
    state: &V8BindingState,
    event: crate::platform_control::CspEvent,
) -> Result<(), V8BindingError> {
    let pending = Notification {
        creator: None,
        local: Some((state.document_root(), state.document_authority_epoch())),
        payload: Payload::LocalCsp(event),
        event: None,
        target: None,
        phase: 0,
        idb_path: None,
        idb_callback_checkpoint_pending: false,
        threw: false,
        listeners: None,
        invocation_type: None,
        property_done: false,
        property_callback: None,
        property_result: None,
        listener_result: None,
        initialized: false,
        event_id: None,
        cancelled: false,
        dom_path: Vec::new(),
        dom_phases: None,
        clear_target: false,
        parser_stream: None,
        suppress_parser_insertion: false,
        legacy_document_target: false,
        unload_guard: RefCell::new(None),
        unload_timing: None,
        navigation_timing: None,
        initial_pageshow: None,
        visibility_snapshot: None,
    };
    admit(state, pending)
}
fn defer(state: &V8BindingState) {
    let mut platform = state.native_platform.borrow_mut();
    let millis = (25u64 << platform.publication_retry_attempt.min(6)).min(1000);
    platform.publication_retry_attempt = platform.publication_retry_attempt.saturating_add(1);
    platform.publication_retry_at =
        std::time::Instant::now().checked_add(std::time::Duration::from_millis(millis));
}
fn clear_retry(state: &V8BindingState) {
    let mut platform = state.native_platform.borrow_mut();
    platform.publication_retry_at = None;
    platform.publication_retry_attempt = 0;
}
pub(in super::super) fn drain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.native_platform.borrow().publishing {
        return Ok(0);
    }
    let waiting = {
        let platform = state.native_platform.borrow();
        platform
            .publication_retry_at
            .is_some_and(|deadline| deadline > std::time::Instant::now())
            && platform
                .publications
                .front()
                .is_some_and(|pending| pending.current(state).unwrap_or(false))
    };
    if waiting {
        return Ok(0);
    }
    let mut count = 0;
    loop {
        if state.execution_tasks_stopped() {
            return Ok(count);
        }
        let pending = { state.native_platform.borrow_mut().publications.pop_front() };
        let Some(mut pending) = pending else {
            clear_retry(state);
            return Ok(count);
        };
        if state.execution_retired() {
            pending.finish(scope, state);
            continue;
        }
        let current = match pending.current(state) {
            Ok(current) => current,
            Err(error) => {
                state
                    .native_platform
                    .borrow_mut()
                    .publications
                    .push_front(pending);
                defer(state);
                return Err(error);
            }
        };
        if !current {
            pending.finish(scope, state);
            continue;
        }
        let backend = state.native_platform.borrow().backend.clone();
        if let Some(backend) = backend {
            match backend.request(
                pending
                    .creator
                    .clone()
                    .unwrap_or_else(|| observed_creator(state)),
                Request::WindowState,
            ) {
                Ok(Response::WindowState { .. }) => {
                    state.native_platform.borrow_mut().publication_retry_at = None;
                }
                Ok(Response::WindowRetired) => {
                    pending.finish(scope, state);
                    continue;
                }
                Ok(Response::WindowSuspended) => {
                    state
                        .native_platform
                        .borrow_mut()
                        .publications
                        .push_front(pending);
                    defer(state);
                    return Ok(count);
                }
                Ok(_) => {
                    state
                        .native_platform
                        .borrow_mut()
                        .publications
                        .push_front(pending);
                    defer(state);
                    return Err(failure("notification window state mismatch"));
                }
                Err(error) => {
                    state
                        .native_platform
                        .borrow_mut()
                        .publications
                        .push_front(pending);
                    defer(state);
                    return Err(failure(error));
                }
            }
        }
        {
            let mut platform = state.native_platform.borrow_mut();
            platform.publishing = true;
            platform.publication_bytes = payload_bytes(&pending.payload);
        }
        let result = pending.resume(scope, state);
        {
            let mut platform = state.native_platform.borrow_mut();
            platform.publishing = false;
            platform.publication_bytes = 0;
        }
        match result {
            Ok(true) => {
                clear_retry(state);
                pending.finish(scope, state);
                if state.execution_retired() || pending.cancelled {
                    continue;
                }
                let mut platform = state.native_platform.borrow_mut();
                match pending.payload {
                    Payload::Csp(value) => platform.csp_cursor = value.id,
                    Payload::Storage(value) => platform.storage_cursor = value.id,
                    _ => {}
                }
                count += 1;
            }
            other => {
                state
                    .native_platform
                    .borrow_mut()
                    .publications
                    .push_front(pending);
                defer(state);
                return other.map(|_| count);
            }
        }
    }
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
enum FailurePoint {
    Prepare,
    BeforeCallback,
    OnceRemoved,
    AfterCallback,
}
#[cfg(test)]
thread_local! {static FAILURE:Cell<Option<FailurePoint>>=const{Cell::new(None)};}
#[cfg(test)]
fn fail_at(point: FailurePoint) -> Result<(), V8BindingError> {
    if FAILURE.with(|failure| {
        if failure.get() == Some(point) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        Err(failure("injected native notification publication failure"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(in super::super) fn fail_after_next_callback() {
    FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Messages;
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            _: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn dispatch_now(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<usize, V8BindingError> {
        state.native_platform.borrow_mut().publication_retry_at = None;
        super::drain(scope, state)
    }
    fn violation(id: u64) -> Payload {
        Payload::Csp(crate::platform_control::CspEvent {
            id,
            document_uri: "https://example.test/".into(),
            referrer: String::new(),
            blocked_uri: "https://example.test/image".into(),
            effective_directive: "img-src".into(),
            original_policy: "img-src 'none'".into(),
            source_file: String::new(),
            sample: String::new(),
            disposition: "enforce".into(),
            status_code: 200,
            line_number: 0,
            column_number: 0,
        })
    }
    #[test]
    fn native_notification_retry_retains_event_phase_snapshot_and_once_callback() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            r#"globalThis.seen=[];globalThis.saved=null;
            document.addEventListener('securitypolicyviolation',e=>{saved=e;seen.push('first');document.addEventListener('securitypolicyviolation',()=>seen.push('late'));},{once:true});
            document.addEventListener('securitypolicyviolation',e=>{if(e!==saved||!e.isTrusted||e.target!==document)throw Error('identity');seen.push('second')});
            addEventListener('securitypolicyviolation',()=>seen.push('window'));
        "#,
        );
        enqueue(&state, state.native_worker_creator().unwrap(), violation(7)).unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::Prepare)));
        assert!(dispatch_now(scope, &state).is_err());
        assert_eq!(state.native_platform.borrow().csp_cursor, 0);
        assert!(eval(scope, "seen.length===0").is_true());
        FAILURE.with(|failure| failure.set(Some(FailurePoint::OnceRemoved)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "seen.length===0").is_true());
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='first'").is_true());
        assert_eq!(state.native_platform.borrow().csp_cursor, 0);
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert_eq!(state.native_platform.borrow().csp_cursor, 7);
        assert!(eval(scope,"seen.join(',')==='first,second,window'&&saved.currentTarget===null&&saved.eventPhase===0").is_true());
        assert_eq!(dispatch_now(scope, &state).unwrap(), 0);
    }
    #[test]
    fn global_storage_notification_retry_does_not_duplicate_consumed_handlers() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            "globalThis.seen=[];addEventListener('storage',e=>{seen.push('capture');e.stopPropagation()},{capture:true,once:true});onstorage=e=>seen.push(e.key);addEventListener('storage',()=>seen.push('last'))",
        );
        enqueue(
            &state,
            state.native_worker_creator().unwrap(),
            Payload::Storage(crate::platform_control::StorageEventRecord {
                id: 9,
                local: true,
                key: Some("key".into()),
                old_value: None,
                new_value: Some("value".into()),
                url: "https://example.test/writer".into(),
            }),
        )
        .unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert_eq!(state.native_platform.borrow().storage_cursor, 0);
        assert!(eval(scope, "seen.join(',')==='capture'").is_true());
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert_eq!(state.native_platform.borrow().storage_cursor, 9);
        assert!(eval(scope, "seen.join(',')==='capture,key,last'").is_true());
    }
    #[test]
    fn csp_reporting_observer_publication_is_lossless_and_not_replayed() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            r#"globalThis.observer=new ReportingObserver(()=>{throw Error('records should be drained')},{types:['csp-violation']});observer.observe();
            globalThis.ignored=new ReportingObserver(()=>{throw Error('unrelated report')},{types:['deprecation']});ignored.observe();
            globalThis.calls=0;document.addEventListener('securitypolicyviolation',event=>{calls++;globalThis.eventSeen=event});
        "#,
        );
        let Payload::Csp(mut event) = violation(71) else {
            unreachable!()
        };
        event.sample = oryn_dom::dom_string::from_units(&[0xd800, 0x61, 0xdc00]);
        event.blocked_uri = event.sample.clone();
        enqueue(
            &state,
            state.native_worker_creator().unwrap(),
            Payload::Csp(event),
        )
        .unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::Prepare)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "observer.takeRecords().length===0").is_true());
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        eval(scope, "globalThis.batch=observer.takeRecords()");
        assert!(eval(scope,"calls===1&&batch.length===1&&batch[0].type==='csp-violation'&&batch[0].body.sample===String.fromCharCode(0xd800,0x61,0xdc00)&&batch[0].body.blockedURL===String.fromCharCode(0xfffd,0x61,0xfffd)&&eventSeen.blockedURI===batch[0].body.blockedURL&&eventSeen.sample===batch[0].body.sample&&batch[0].body.sourceFile===null&&batch[0].body.lineNumber===null&&batch[0].body.columnNumber===null&&batch[0].body.statusCode===200&&batch[0].toJSON().body.sample===batch[0].body.sample").is_true());
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert!(
            eval(
                scope,
                "calls===1&&observer.takeRecords().length===0&&ignored.takeRecords().length===0"
            )
            .is_true()
        );
        assert!(eval(scope,"(()=>{const late=new ReportingObserver(()=>{},{buffered:true,types:['csp-violation']});late.observe();return late.takeRecords().length===1})()").is_true());
    }

    #[test]
    fn retirement_releases_pending_publications_without_running_remaining_callbacks() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            "globalThis.calls=0;document.addEventListener('securitypolicyviolation',()=>calls++);document.addEventListener('securitypolicyviolation',()=>calls++)",
        );
        enqueue(&state, state.native_worker_creator().unwrap(), violation(1)).unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "calls===1").is_true());
        state.retire_native_platform();
        assert!(state.native_platform.borrow().publications.is_empty());
        assert_eq!(dispatch_now(scope, &state).unwrap(), 0);
        assert!(eval(scope, "calls===1").is_true());
    }
    #[test]
    fn visibility_retry_preserves_published_state_and_same_event_suffix() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            r#"globalThis.seen=[];globalThis.saved=null;
            document.addEventListener('visibilitychange',e=>{saved=e;seen.push(document.hidden?'hidden':'visible')},{once:true});
            document.addEventListener('visibilitychange',e=>{seen.push(!e.isTrusted||e.target!==document?'invalid':e===saved?'same':'new')});
            addEventListener('visibilitychange',()=>seen.push('window'));
        "#,
        );
        enqueue(
            &state,
            state.native_worker_creator().unwrap(),
            Payload::Visibility(true),
        )
        .unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::Prepare)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "!document.hidden&&seen.length===0").is_true());
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "document.hidden&&seen.join(',')==='hidden'").is_true());
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert!(eval(scope,"document.hidden&&seen.join(',')==='hidden,same,window'&&saved.currentTarget===null&&saved.eventPhase===0").is_true());
        enqueue(
            &state,
            state.native_worker_creator().unwrap(),
            Payload::Visibility(false),
        )
        .unwrap();
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert!(!state.native_platform.borrow().hidden);
        assert!(eval(scope, "seen.join(',')==='hidden,same,window,new,window'").is_true());
    }

    #[test]
    fn permission_status_publication_preserves_native_identity_and_handler_state_on_retry() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        let (id, status) = wrap(scope, &state, "PermissionStatus").unwrap();
        state.native_platform.borrow_mut().permissions.insert(
            id,
            Permission {
                descriptor: PermissionDescriptor {
                    name: "geolocation".into(),
                    sysex: None,
                    pan_tilt_zoom: None,
                    allow_without_gesture: None,
                },
                state: PermissionState::Prompt,
            },
        );
        let key = v8::String::new(scope, "permissionStatus").unwrap();
        context.global(scope).set(scope, key.into(), status.into());
        eval(
            scope,
            "globalThis.seen=[];permissionStatus.addEventListener('change',e=>{seen.push(permissionStatus.state);if(e.target!==permissionStatus||!e.isTrusted)throw Error('status identity')},{once:true});permissionStatus.onchange=()=>seen.push('handler')",
        );
        enqueue(
            &state,
            state.native_worker_creator().unwrap(),
            Payload::Permission(id, PermissionState::Granted),
        )
        .unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::Prepare)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "permissionStatus.state==='prompt'&&seen.length===0").is_true());
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(
            eval(
                scope,
                "permissionStatus.state==='granted'&&seen.join(',')==='granted'"
            )
            .is_true()
        );
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert!(eval(scope, "seen.join(',')==='granted,handler'").is_true());
    }

    #[test]
    fn standalone_csp_source_task_transfers_custody_without_parent_transport() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.seen=[];document.addEventListener('securitypolicyviolation',()=>seen.push('first'),{once:true});document.addEventListener('securitypolicyviolation',()=>seen.push('second'));globalThis.SecurityPolicyViolationEvent=()=>{throw Error('authored constructor')}",
        );
        let Payload::Csp(event) = violation(19) else {
            unreachable!()
        };
        state.native_csp_tasks.borrow_mut().push_back((
            state.document_root(),
            state.document_authority_epoch(),
            event,
        ));
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(deliver_native_csp_event(scope, &state).is_err());
        assert!(state.native_csp_tasks.borrow().is_empty());
        assert_eq!(state.native_platform.borrow().publications.len(), 1);
        assert!(eval(scope, "seen.join(',')==='first'").is_true());
        state.native_platform.borrow_mut().publication_retry_at = None;
        assert!(deliver_native_csp_event(scope, &state).unwrap());
        assert!(eval(scope, "seen.join(',')==='first,second'").is_true());
        assert_eq!(
            state.native_platform.borrow().csp_cursor,
            0,
            "standalone ids must not acknowledge parent transport ids"
        );
        assert!(!deliver_native_csp_event(scope, &state).unwrap());
    }

    #[test]
    fn bounded_admission_preserves_standalone_source_and_existing_queue() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let Payload::Csp(mut event) = violation(1) else {
            unreachable!()
        };
        event.sample = "x".repeat(MAX_PUBLICATION_BYTES + 1);
        state.native_csp_tasks.borrow_mut().push_back((
            state.document_root(),
            state.document_authority_epoch(),
            event,
        ));
        assert!(deliver_native_csp_event(scope, &state).is_err());
        assert_eq!(state.native_csp_tasks.borrow().len(), 1);
        assert!(state.native_platform.borrow().publications.is_empty());
        state.native_csp_tasks.borrow_mut().clear();
        for id in 0..MAX_PUBLICATIONS {
            let Payload::Csp(event) = violation(id as u64) else {
                unreachable!()
            };
            enqueue_local_csp(&state, event).unwrap();
        }
        let Payload::Csp(event) = violation(9999) else {
            unreachable!()
        };
        assert!(enqueue_local_csp(&state, event).is_err());
        assert_eq!(
            state.native_platform.borrow().publications.len(),
            MAX_PUBLICATIONS
        );
    }
    #[test]
    fn suspended_retired_and_transient_parent_states_preserve_distinct_custody() {
        struct WindowBackend(std::sync::atomic::AtomicU8);
        impl Backend for WindowBackend {
            fn request(
                &self,
                _: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                assert!(matches!(request, Request::WindowState));
                match self.0.load(std::sync::atomic::Ordering::Relaxed) {
                    0 => Ok(Response::WindowSuspended),
                    1 => Err("temporary transport failure".into()),
                    2 => Ok(Response::WindowState {
                        hidden: false,
                        revision: 0,
                    }),
                    _ => Ok(Response::WindowRetired),
                }
            }
        }
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let backend = std::sync::Arc::new(WindowBackend(std::sync::atomic::AtomicU8::new(0)));
        state.set_native_platform_backend(backend.clone());
        eval(
            scope,
            "globalThis.calls=0;document.addEventListener('securitypolicyviolation',()=>calls++)",
        );
        let Payload::Csp(event) = violation(1) else {
            unreachable!()
        };
        enqueue_local_csp(&state, event).unwrap();
        assert_eq!(dispatch_now(scope, &state).unwrap(), 0);
        assert_eq!(state.native_platform.borrow().publications.len(), 1);
        assert!(
            state
                .native_platform
                .borrow()
                .publication_retry_at
                .is_some()
        );
        state.native_platform.borrow_mut().publication_retry_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        assert!(
            state.native_platform_ready(),
            "initial resource discovery remains runnable independently of publication retry"
        );
        assert_eq!(super::drain(scope, &state).unwrap(), 0);
        assert_eq!(state.native_platform.borrow().publications.len(), 1);
        backend.0.store(1, std::sync::atomic::Ordering::Relaxed);
        assert!(dispatch_now(scope, &state).is_err());
        assert_eq!(state.native_platform.borrow().publications.len(), 1);
        backend.0.store(2, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(dispatch_now(scope, &state).unwrap(), 1);
        assert!(eval(scope, "calls===1").is_true());
        let Payload::Csp(event) = violation(2) else {
            unreachable!()
        };
        enqueue_local_csp(&state, event).unwrap();
        backend.0.store(3, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(dispatch_now(scope, &state).unwrap(), 0);
        assert!(state.native_platform.borrow().publications.is_empty());
        assert!(eval(scope, "calls===1").is_true());
    }
    #[test]
    fn property_getter_retirement_prevents_returned_callback_invocation() {
        fn retire(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
            scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .unwrap()
                .retire_execution();
        }
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let callback = v8::Function::new(scope, retire).unwrap();
        let key = v8::String::new(scope, "retireNative").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into());
        eval(
            scope,
            "globalThis.calls=0;Object.defineProperty(document,'onsecuritypolicyviolation',{get(){retireNative();return ()=>calls++}})",
        );
        let Payload::Csp(event) = violation(1) else {
            unreachable!()
        };
        enqueue_local_csp(&state, event).unwrap();
        dispatch_now(scope, &state).unwrap();
        assert!(state.execution_retired());
        assert!(eval(scope, "calls===0").is_true());
        assert!(state.native_platform.borrow().publications.is_empty());
    }

    #[test]
    fn subscribed_permission_status_survives_gc_then_releases_registry_after_unsubscribe() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let id = {
            v8::scope!(let inner,scope);
            let (id, status) = wrap(inner, &state, "PermissionStatus").unwrap();
            state.native_platform.borrow_mut().permissions.insert(
                id,
                Permission {
                    descriptor: PermissionDescriptor {
                        name: "geolocation".into(),
                        sysex: None,
                        pan_tilt_zoom: None,
                        allow_without_gesture: None,
                    },
                    state: PermissionState::Prompt,
                },
            );
            let key = v8::String::new(inner, "temporaryStatus").unwrap();
            context.global(inner).set(inner, key.into(), status.into());
            eval(
                inner,
                "globalThis.statusChanges=0;temporaryStatus.onchange=()=>statusChanges++;temporaryStatus=null",
            );
            id
        };
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(state.platform_listener_roots.borrow().contains_key(&id));
        assert!(state.native_platform.borrow().permissions.contains_key(&id));
        enqueue(
            &state,
            observed_creator(&state),
            Payload::Permission(id, PermissionState::Granted),
        )
        .unwrap();
        {
            v8::scope!(let inner,scope);
            dispatch_now(inner, &state).unwrap();
        }
        assert!(eval(scope, "statusChanges===1").is_true());
        {
            v8::scope!(let inner,scope);
            let object = state
                .platform_wrappers
                .borrow()
                .get(&id)
                .unwrap()
                .to_local(inner)
                .unwrap();
            let key = v8::String::new(inner, "onchange").unwrap();
            let null = v8::null(inner);
            object.set(inner, key.into(), null.into());
        }
        assert!(!state.platform_listener_roots.borrow().contains_key(&id));
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(!state.native_platform.borrow().permissions.contains_key(&id));
        assert!(!state.native_platform.borrow().objects.contains_key(&id));
        assert!(!state.platform_objects.borrow().contains_key(&id));
    }

    #[test]
    fn move_publication_retains_event_callback_prefix_and_handler() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        eval(
            scope,
            "globalThis.moves=[];addEventListener('move',e=>moves.push(e.isTrusted&&e.target===window?'first':'bad'),{once:true});window.onmove=()=>moves.push('handler')",
        );
        enqueue(
            &state,
            observed_creator(&state),
            Payload::WindowMove(100, 200),
        )
        .unwrap();
        FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
        assert!(dispatch_now(scope, &state).is_err());
        assert!(eval(scope, "moves.join()==='first'").is_true());
        assert_eq!(
            state.native_platform.borrow().window_position,
            Some((100, 200))
        );
        dispatch_now(scope, &state).unwrap();
        assert!(eval(scope, "moves.join()==='first,handler'").is_true());
    }
}
#[cfg(test)]
pub(in super::super) fn fail_before_next_prepare() {
    FAILURE.with(|failure| failure.set(Some(FailurePoint::Prepare)));
}

#[cfg(test)]
mod suspended_continuation_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn park(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
        let states = scope
            .get_slot::<BindingStateRegistry>()
            .unwrap()
            .instances
            .borrow()
            .values()
            .filter_map(RcWeak::upgrade)
            .collect::<Vec<_>>();
        for state in states {
            state.set_execution_suspended(true);
        }
    }
    #[test]
    fn idb_callback_fault_and_parking_preserve_checkpoint_suffix_once() {
        for parking in [false, true] {
            super::super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(227), 1).unwrap();
            let park_function = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkNative").unwrap();
            context
                .global(scope)
                .set(scope, key.into(), park_function.into())
                .unwrap();
            let id = new_idb_request(&state, None);
            let request = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
            let key = v8::String::new(scope, "request").unwrap();
            context
                .global(scope)
                .set(scope, key.into(), request.into())
                .unwrap();
            eval(
                scope,
                if parking {
                    "globalThis.log=[];request.addEventListener('success',()=>{log.push('first');Promise.resolve().then(()=>log.push('microtask'));parkNative()},{once:true});request.addEventListener('success',()=>log.push('second'))"
                } else {
                    "globalThis.log=[];request.addEventListener('success',()=>{log.push('first');Promise.resolve().then(()=>log.push('microtask'))},{once:true});request.addEventListener('success',()=>log.push('second'))"
                },
            );
            let event = create_event_object(scope, &state, "success", false, false).unwrap();
            let mut pending = pending_idb_dispatch(scope, &state, id, "success", event).unwrap();
            if parking {
                assert!(!pending.resume(scope, &state).unwrap());
                state.set_execution_suspended(false);
            } else {
                FAILURE.with(|failure| failure.set(Some(FailurePoint::AfterCallback)));
                assert!(pending.resume(scope, &state).is_err());
            }
            assert!(pending.resume(scope, &state).unwrap());
            pending.finish(scope, &state);
            assert!(eval(scope, "log.join()==='first,microtask,second'").is_true());
            assert!(state.idb_callback_protection.borrow().is_empty());
        }
    }
    #[test]
    fn idb_callback_cleanup_does_not_checkpoint_inside_author_dispatch_stack() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(228), 1).unwrap();
        let id = new_idb_request(&state, None);
        let request = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
        let key = v8::String::new(scope, "request").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), request.into())
            .unwrap();
        assert!(eval(scope, "globalThis.log=[];request.addEventListener('success',()=>{log.push('first');Promise.resolve().then(()=>log.push('microtask'))});request.addEventListener('success',()=>log.push('second'));request.dispatchEvent(new Event('success'));log.push('returned');log.join()==='first,second,returned'").is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='first,second,returned,microtask'").is_true());
    }
    #[test]
    fn nested_idb_request_dispatch_does_not_deactivate_outer_transaction() {
        fn check_active(
            scope: &mut v8::PinScope,
            _: v8::FunctionCallbackArguments,
            _: v8::ReturnValue,
        ) {
            let key = v8::String::new(scope, "transaction").unwrap();
            let value = scope
                .get_current_context()
                .global(scope)
                .get(scope, key.into())
                .unwrap();
            let object = value.to_object(scope).unwrap();
            let state = binding_state_for_receiver(scope, object).unwrap();
            let id = stateful_id(scope, object).unwrap();
            assert!(
                matches!(&*platform_object(&state,id).unwrap(),PlatformObjectRecord::IdbTransaction(transaction) if transaction.active)
            );
        }
        fn nested(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
            let key = v8::String::new(scope, "innerRequest").unwrap();
            let value = scope
                .get_current_context()
                .global(scope)
                .get(scope, key.into())
                .unwrap();
            let object = value.to_object(scope).unwrap();
            let state = binding_state_for_receiver(scope, object).unwrap();
            let id = stateful_id(scope, object).unwrap();
            state.activate_idb_request_transaction(id).unwrap();
            let event = create_event_object(scope, &state, "success", false, false).unwrap();
            let mut pending = pending_idb_dispatch(scope, &state, id, "success", event).unwrap();
            assert!(pending.resume(scope, &state).unwrap());
            pending.finish(scope, &state);
            state.finish_platform_task_checkpoint().unwrap();
        }
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(231), 1).unwrap();
        scope.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let heap = scope.get_heap_statistics();
        let used = heap.used_heap_size().checked_add(heap.external_memory()).unwrap();
        let limit = heap.heap_size_limit();
        let ledger = &state.idb_payload_ledger;
        let (database_name, name_custody) = crate::idb_owned_admission::Text::copy_string_cancelled(
            ledger, "nested-native", limit, used,
            &mut || scope.is_execution_terminating() || state.execution_retired(),
        ).unwrap().into_parts();
        let database = register_platform_object(
            &state,
            PlatformObjectRecord::IdbDatabase(IdbDatabaseRecord {
                connection: None,
                last_versionchange_event: 0,
                name: database_name,
                _name_custody: Some(name_custody),
                version: 1,
                old_version: 0,
                object_stores: owned_names::Names::from_strings(&vec![], ledger, limit, used, &mut || scope.is_execution_terminating() || state.execution_retired()).unwrap(),
                object_store_definitions: owned_store_map::StoreMap::empty(ledger, limit, used, &mut || scope.is_execution_terminating() || state.execution_retired()).unwrap(),
                _upgrade_base_object_stores: owned_names::Names::from_strings(&[], ledger, limit, used, &mut || scope.is_execution_terminating() || state.execution_retired()).unwrap(),
                upgrade_base_definitions: owned_store_map::StoreMap::empty(ledger, limit, used, &mut || scope.is_execution_terminating() || state.execution_retired()).unwrap(),
                upgrade_transaction_id: None,
                closed: false,
            }),
        );
        let database_wrapper = new_platform_object(scope, &state, "IDBDatabase", database).unwrap();
        let key = v8::String::new(scope, "database").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), database_wrapper.into())
            .unwrap();
        let admitted_scope = crate::idb_owned_admission::Scope::copy_names_cancelled(
            ledger, &vec![], limit, used,
            &mut || scope.is_execution_terminating() || state.execution_retired(),
        ).unwrap().into_parts();
        let tx = register_platform_object(
            &state,
            PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                request_queue: VecDeque::new(),
                abort_event_queued: false,
                settled: false,
                database_id: database,
                request_id: None,
                mode: "readonly".into(),
                durability: "default".into(),
                object_stores: admitted_scope.0,
                scope_custody: Some(admitted_scope.1),
                aborted: false,
                error: None,
                pending_requests: 0,
                completion_requested: false,
                completion_queued: false,
                explicit_commit: false,
                active: true,
                finished: false,
            }),
        );
        let outer = new_idb_request(&state, Some(tx));
        let transaction_wrapper = new_platform_object(scope, &state, "IDBTransaction", tx).unwrap();
        let key = v8::String::new(scope, "transaction").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), transaction_wrapper.into())
            .unwrap();
        let inner = new_idb_request(&state, Some(tx));
        for (name, id) in [("outerRequest", outer), ("innerRequest", inner)] {
            let object = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
            let key = v8::String::new(scope, name).unwrap();
            context
                .global(scope)
                .set(scope, key.into(), object.into())
                .unwrap();
        }
        let function = v8::Function::new(scope, nested).unwrap();
        let key = v8::String::new(scope, "nestedNative").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), function.into())
            .unwrap();
        let function = v8::Function::new(scope, check_active).unwrap();
        let key = v8::String::new(scope, "checkActiveNative").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), function.into())
            .unwrap();
        eval(
            scope,
            "globalThis.log=[];innerRequest.addEventListener('success',()=>log.push('inner'));outerRequest.addEventListener('success',()=>{log.push('outer');Promise.resolve().then(nestedNative)});outerRequest.addEventListener('success',()=>{checkActiveNative();log.push('outer-second')})",
        );
        state.activate_idb_request_transaction(outer).unwrap();
        let event = create_event_object(scope, &state, "success", false, false).unwrap();
        let mut pending = pending_idb_dispatch(scope, &state, outer, "success", event).unwrap();
        assert!(pending.resume(scope, &state).unwrap());
        pending.finish(scope, &state);
        assert!(
            matches!(&*platform_object(&state,tx).unwrap(),PlatformObjectRecord::IdbTransaction(transaction) if transaction.active)
        );
        assert!(eval(scope, "log.join()==='outer,inner,outer-second'").is_true());
        state.finish_platform_task_checkpoint().unwrap();
        assert!(
            matches!(&*platform_object(&state,tx).unwrap(),PlatformObjectRecord::IdbTransaction(transaction) if !transaction.active)
        );
        assert!(state.idb_callback_protection.borrow().is_empty());
    }
    #[test]
    fn idb_throwing_handler_getter_checkpoints_before_parent_listener() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(230), 1).unwrap();
        let id = new_idb_request(&state, None);
        let parent_id = new_idb_request(&state, None);
        for (name, id) in [("request", id), ("parentRequest", parent_id)] {
            let request = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
            let key = v8::String::new(scope, name).unwrap();
            context
                .global(scope)
                .set(scope, key.into(), request.into())
                .unwrap();
        }
        eval(
            scope,
            "globalThis.log=[];request.addEventListener('probe',{get handleEvent(){log.push('getter');Promise.resolve().then(()=>log.push('microtask'));throw new Error('getter')}});parentRequest.addEventListener('probe',()=>log.push('parent'))",
        );
        let event = create_event_object(scope, &state, "probe", true, true).unwrap();
        let mut pending = pending_idb_dispatch(scope, &state, id, "probe", event).unwrap();
        pending.idb_path = Some(vec![id, parent_id]);
        assert!(pending.resume(scope, &state).unwrap());
        pending.finish(scope, &state);
        assert!(eval(scope, "log.join()==='getter,microtask,parent'").is_true());
    }
    #[test]
    fn idb_successful_callback_getters_finish_single_invocation_then_cleanup() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(232), 1).unwrap();
        let id = new_idb_request(&state, None);
        let parent = new_idb_request(&state, None);
        for (name, id) in [("request", id), ("parentRequest", parent)] {
            let object = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
            let key = v8::String::new(scope, name).unwrap();
            context
                .global(scope)
                .set(scope, key.into(), object.into())
                .unwrap();
        }
        eval(
            scope,
            "globalThis.log=[];request.addEventListener('probe',{get handleEvent(){log.push('getter');Promise.resolve().then(()=>log.push('microtask'));return ()=>log.push('method')}});request.addEventListener('probe',()=>log.push('second'));request.addEventListener('probe',{get handleEvent(){log.push('property');Promise.resolve().then(()=>log.push('property-microtask'));return null}});parentRequest.addEventListener('probe',()=>log.push('parent'))",
        );
        let event = create_event_object(scope, &state, "probe", true, true).unwrap();
        let mut pending = pending_idb_dispatch(scope, &state, id, "probe", event).unwrap();
        pending.idb_path = Some(vec![id, parent]);
        assert!(pending.resume(scope, &state).unwrap());
        pending.finish(scope, &state);
        assert!(
            eval(
                scope,
                "log.join()==='getter,method,microtask,second,property,property-microtask,parent'"
            )
            .is_true()
        );
    }
    #[test]
    fn idb_callback_checkpoint_runs_foreign_listener_microtasks_before_suffix() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let foreign = v8::Context::new(scope, Default::default());
        let callback = {
            let foreign_scope = &mut v8::ContextScope::new(scope, foreign);
            let foreign_dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let foreign_state =
                install_generated_bindings(foreign_scope, foreign, foreign_dom, RealmId(233), 1)
                    .unwrap();
            let tx = register_platform_object(
                &foreign_state,
                PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                    request_queue: VecDeque::new(),
                    abort_event_queued: false,
                    settled: false,
                    database_id: 0,
                    request_id: None,
                    mode: "readonly".into(),
                    durability: "default".into(),
                    object_stores: vec![],
                    scope_custody: None,
                    aborted: false,
                    error: None,
                    pending_requests: 0,
                    completion_requested: false,
                    completion_queued: false,
                    explicit_commit: false,
                    active: true,
                    finished: false,
                }),
            );
            foreign_state
                .idb_transactions_to_deactivate
                .borrow_mut()
                .insert(tx);
            let callback = eval(
                foreign_scope,
                "(event)=>{event.target.log.push('foreign');Promise.resolve().then(()=>event.target.log.push('foreign-microtask'))}",
            );
            (v8::Global::new(foreign_scope, callback), foreign_state, tx)
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(229), 1).unwrap();
        let id = new_idb_request(&state, None);
        let request = new_platform_object(scope, &state, "IDBRequest", id).unwrap();
        let key = v8::String::new(scope, "request").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), request.into())
            .unwrap();
        let key = v8::String::new(scope, "foreignListener").unwrap();
        let foreign_state = callback.1.clone();
        let foreign_tx = callback.2;
        let callback = v8::Local::new(scope, &callback.0);
        context
            .global(scope)
            .set(scope, key.into(), callback)
            .unwrap();
        eval(
            scope,
            "request.log=[];request.addEventListener('success',foreignListener);request.addEventListener('success',()=>request.log.push('local'))",
        );
        let event = create_event_object(scope, &state, "success", false, false).unwrap();
        let mut pending = pending_idb_dispatch(scope, &state, id, "success", event).unwrap();
        assert!(pending.resume(scope, &state).unwrap());
        pending.finish(scope, &state);
        assert!(
            eval(
                scope,
                "request.log.join()==='foreign,foreign-microtask,local'"
            )
            .is_true()
        );
        assert!(
            matches!(&*platform_object(&foreign_state,foreign_tx).unwrap(),PlatformObjectRecord::IdbTransaction(transaction) if !transaction.active)
        );
    }
    #[test]
    fn parking_between_listeners_or_in_property_getter_preserves_suffix() {
        for kind in ["listener", "property", "object"] {
            let getter = kind != "listener";
            super::super::super::tests::initialize_v8();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<div id=t></div>")
                    .into_runtime()
                    .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(226), 1).unwrap();
            let callback = v8::Function::new(scope, park).unwrap();
            let name = v8::String::new(scope, "parkNative").unwrap();
            context
                .global(scope)
                .set(scope, name.into(), callback.into())
                .unwrap();
            eval(
                scope,
                if kind == "object" {
                    "globalThis.log=[];globalThis.controller=new AbortController();t.addEventListener('probe',{get handleEvent(){log.push('getter');controller.abort();parkNative();return function(){log.push('method')}}},{once:true,signal:controller.signal});t.addEventListener('probe',()=>log.push('second'))"
                } else if getter {
                    "globalThis.log=[];Object.defineProperty(t,'onprobe',{get(){log.push('getter');parkNative();return ()=>log.push('handler')}})"
                } else {
                    "globalThis.log=[];t.addEventListener('probe',()=>{log.push('first');parkNative()},{once:true});t.addEventListener('probe',()=>log.push('second'));t.onprobe=()=>log.push('handler')"
                },
            );
            let target = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), "t")
                .unwrap()
                .unwrap();
            let wrapper = wrap_node(scope, &state, target).unwrap();
            let event = create_event_object(scope, &state, "probe", true, true).unwrap();
            let mut pending = pending_dom_dispatch(scope, &state, wrapper, "probe", event);
            assert!(!pending.resume(scope, &state).unwrap());
            assert!(state.execution_suspended());
            assert!(!pending.resume(scope, &state).unwrap());
            assert!(
                eval(
                    scope,
                    if getter {
                        "log.join()==='getter'"
                    } else {
                        "log.join()==='first'"
                    }
                )
                .is_true()
            );
            state.set_execution_suspended(false);
            assert!(pending.resume(scope, &state).unwrap());
            pending.finish(scope, &state);
            assert!(
                eval(
                    scope,
                    if kind == "object" {
                        "log.join()==='getter,method,second'"
                    } else if getter {
                        "log.join()==='getter,handler'"
                    } else {
                        "log.join()==='first,second,handler'"
                    }
                )
                .is_true()
            );
        }
    }
}

/// IDB dispatch captures the request/transaction/database path once and keeps
/// per-listener progress across allocation and callback-checkpoint failures.
pub(in super::super) fn pending_idb_dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: u64,
    name: &str,
    event: v8::Local<v8::Object>,
) -> Result<Notification, V8BindingError> {
    let mut path = vec![target];
    loop {
        let parent = match &*platform_object(state, *path.last().unwrap())? {
            PlatformObjectRecord::IdbRequest(request) if !request.open_request => {
                request.transaction_id
            }
            PlatformObjectRecord::IdbTransaction(transaction) => Some(transaction.database_id),
            _ => None,
        };
        match parent {
            Some(parent) => path.push(parent),
            None => break,
        }
    }
    let mut notification = pending_platform_dispatch(scope, state, target, name, event);
    notification.idb_path = Some(path);
    Ok(notification)
}
