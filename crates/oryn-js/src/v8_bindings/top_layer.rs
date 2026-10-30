//! Native popover state and coalesced ToggleEvent tasks.
use super::*;

pub(super) fn event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: &str,
    old: &str,
    new: &str,
    source: Option<NodeId>,
    cancelable: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let event =
        create_event_object_with_interface(scope, state, "ToggleEvent", kind, false, cancelable)?;
    let payload = v8::Array::new(scope, 3);
    let null = v8::null(scope);
    payload.set_prototype(scope, null.into());
    let old = v8_string(scope, old)?;
    payload.set_index(scope, 0, old);
    let new = v8_string(scope, new)?;
    payload.set_index(scope, 1, new);
    let source = optional_node(scope, state, source)?;
    payload.set_index(scope, 2, source);
    event.set_internal_field(2, payload.into());
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    Ok(event)
}

fn mode(state: &V8BindingState, node: NodeId) -> Result<String, V8BindingError> {
    let dom = state.dom.borrow();
    dom.attribute_in_null_namespace(node, "popover")?
        .map(|value| {
            if value.eq_ignore_ascii_case("manual") {
                "manual"
            } else if value.eq_ignore_ascii_case("hint") {
                "hint"
            } else {
                "auto"
            }
            .into()
        })
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "HTMLElement".into(),
            member: "NotSupportedError".into(),
        })
}
struct ToggleTask {
    details: bool,
    target: DeferredDomTarget,
    source: Option<DeferredDomTarget>,
    old: &'static str,
    new: &'static str,
    epoch: u64,
    dispatch: Option<native_platform::publication::Notification>,
}
pub(super) fn queue(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    old: &str,
    new: &str,
    source: Option<NodeId>,
) -> Result<(), V8BindingError> {
    queue_with_kind(scope, state, node, old, new, source, false)
}

fn queue_with_kind(
    scope: &mut v8::PinScope,
    state: &V8BindingState, node: NodeId, old: &str, new: &str,
    source: Option<NodeId>, details: bool,
) -> Result<(), V8BindingError> {
    let canonical = |value: &str| match value {
        "open" => Ok("open"), "closed" => Ok("closed"),
        _ => Err(V8BindingError::Allocation("invalid native toggle state")),
    };
    let old = canonical(old)?;
    let new = canonical(new)?;
    let allocation = state.capture_retained_allocation(
        scope, Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?;
    let document = if details {
        state.dom.borrow().node_document(node)?
            .ok_or(V8BindingError::Allocation("details target document"))?
    } else { state.document_root() };
    if details && !owns_details_document(state, document) {
        return Err(V8BindingError::Allocation("details publication owner"));
    }
    let target = DeferredDomTarget::new(&state.dom, node, document)?;
    let source = source
        .map(|source| DeferredDomTarget::new(&state.dom, source, document))
        .transpose()?;
    let mut registry = state.popover_publications.borrow_mut();
    if let Some(pending) = registry.toggles.iter_mut().find(|pending| {
        pending.target.node == node
            && pending.target.document == document
            && pending.dispatch.is_none()
            && pending.details == details
    }) {
        pending.new = new;
        pending.source = source;
        return Ok(());
    }
    if registry.toggles.len() >= 4096 {
        return Err(V8BindingError::Allocation("toggle task limit"));
    }
    let Registry { toggles, toggle_storage, .. } = &mut *registry;
    reserve_queue_growth(&allocation, toggles, toggle_storage, 1)?;
    let mut tasks = state.deferred_platform_tasks.borrow_mut();
    if tasks.len() >= 4096 {
        return Err(V8BindingError::Allocation("toggle wake task limit"));
    }
    reserve_queue_growth(
        &allocation, &mut tasks, &mut state.deferred_task_queue_storage.borrow_mut(), 1,
    )?;
    registry.toggles.push_back(ToggleTask {
        details,
        target,
        source,
        old,
        new,
        epoch: if details { state.document_authority_epoch() } else { state.document_stream_epoch() },
        dispatch: None,
    });
    tasks.push_back(DeferredPlatformTask::ToggleEvent);
    Ok(())
}

fn owns_details_document(state: &V8BindingState, document: NodeId) -> bool {
    document == state.document_root()
        || state.popover_publications.borrow().inert_documents.iter().any(|(owner, epoch)|
            *owner == document && *epoch == state.document_authority_epoch())
}

pub(super) struct InertDetailsImport {
    issuer: (u64, u64),
    source_document: NodeId,
    imported_document: Option<NodeId>,
    changes: VecDeque<(oryn_dom::AttributePresenceChange, Option<NodeId>)>,
    _storage: Option<(usize, Arc<dyn Send + Sync>)>,
    template_contents: VecDeque<NodeId>,
    template_storage: Option<(usize, Arc<dyn Send + Sync>)>,
    allocation: BindingRetainedAllocation,
}

impl InertDetailsImport {
    pub(super) fn capture(
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        runtime: &SharedDom,
    ) -> Result<Self, V8BindingError> {
        let allocation = state.capture_retained_allocation(
            scope, Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )?;
        let mut changes = VecDeque::new();
        let mut storage = None;
        let mut cursor = oryn_dom::MutationCursor(0);
        loop {
            let read = runtime.borrow().next_attribute_presence_change(cursor, "open");
            if read.truncated {
                return Err(V8BindingError::Allocation("inert details parser journal truncated"));
            }
            let Some(change) = read.change else { break };
            if runtime.borrow().is_element(change.node,
                "http://www.w3.org/1999/xhtml", "details")
            {
                if changes.len() >= 4096 {
                    return Err(V8BindingError::Allocation("inert details parser transition limit"));
                }
                reserve_queue_growth(&allocation, &mut changes, &mut storage, 1)?;
                changes.push_back((change, None));
            }
            cursor = read.next;
        }
        Ok(Self { issuer: (state.instance_id, state.document_authority_epoch()),
            source_document: runtime.borrow().document(), imported_document: None,
            changes, _storage: storage, allocation,
            template_contents: VecDeque::new(), template_storage: None })
    }

    pub(super) fn imported(&mut self, original: NodeId, target: NodeId,
        template_content: bool) -> Result<(), V8BindingError> {
        if original == self.source_document { self.imported_document = Some(target); }
        for (change, mapped) in &mut self.changes {
            if change.node == original { *mapped = Some(target); }
        }
        if template_content {
            if self.template_contents.len() >= 4096 {
                return Err(V8BindingError::Allocation("inert template document owner limit"));
            }
            reserve_queue_growth(&self.allocation, &mut self.template_contents,
                &mut self.template_storage, 1)?;
            self.template_contents.push_back(target);
        }
        Ok(())
    }

    pub(super) fn publish(self, state: &V8BindingState, root: NodeId) -> Result<(), V8BindingError> {
        if state.execution_retired()
            || self.issuer != (state.instance_id, state.document_authority_epoch())
            || self.imported_document != Some(root)
        {
            return Err(V8BindingError::Allocation("inert details publisher authority"));
        }
        let epoch = state.document_authority_epoch();
        let mut tasks: VecDeque<ToggleTask> = VecDeque::new();
        let mut task_storage = None;
        let mut owners = VecDeque::new();
        let mut owner_storage = None;
        if !owns_details_document(state, root) {
            reserve_queue_growth(&self.allocation, &mut owners, &mut owner_storage, 1)?;
            owners.push_back((root, epoch));
        }
        for content in &self.template_contents {
            let document = state.dom.borrow().node_document(*content)?
                .ok_or(V8BindingError::Allocation("inert template imported document"))?;
            if !owns_details_document(state, document) && !owners.contains(&(document, epoch)) {
                reserve_queue_growth(&self.allocation, &mut owners, &mut owner_storage, 1)?;
                owners.push_back((document, epoch));
            }
        }
        for (change, mapped) in &self.changes {
            let node = mapped.ok_or(V8BindingError::Allocation("inert details import identity"))?;
            let document = state.dom.borrow().node_document(node)?
                .ok_or(V8BindingError::Allocation("inert details imported document"))?;
            if !owns_details_document(state, document) && !owners.contains(&(document, epoch)) {
                reserve_queue_growth(&self.allocation, &mut owners, &mut owner_storage, 1)?;
                owners.push_back((document, epoch));
            }
            let old = if change.was_present { "open" } else { "closed" };
            let new = if change.is_present { "open" } else { "closed" };
            if let Some(task) = tasks.iter_mut().find(|task|
                task.target.node == node && task.target.document == document)
            { task.new = new; continue }
            reserve_queue_growth(&self.allocation, &mut tasks, &mut task_storage, 1)?;
            tasks.push_back(ToggleTask {
                details: true, target: DeferredDomTarget::new(&state.dom, node, document)?,
                source: None, old, new, epoch, dispatch: None,
            });
        }
        let mut registry = state.popover_publications.borrow_mut();
        registry.inert_documents.retain(|(document, owner_epoch)|
            *owner_epoch == epoch && state.dom.borrow().arena().get(*document).is_ok());
        let mut wake = state.deferred_platform_tasks.borrow_mut();
        if registry.inert_documents.len().saturating_add(owners.len()) > 4096
            || registry.toggles.len().saturating_add(tasks.len()) > 4096
            || wake.len().saturating_add(tasks.len()) > 4096
        { return Err(V8BindingError::Allocation("inert details publication capacity")); }
        let Registry { inert_documents, inert_document_storage, toggles, toggle_storage, .. }
            = &mut *registry;
        reserve_queue_growth(&self.allocation, inert_documents, inert_document_storage, owners.len())?;
        reserve_queue_growth(&self.allocation, toggles, toggle_storage, tasks.len())?;
        reserve_queue_growth(&self.allocation, &mut wake,
            &mut state.deferred_task_queue_storage.borrow_mut(), tasks.len())?;
        // Everything below is admitted native bookkeeping: no fallible work
        // and no author callbacks can expose a partially published batch.
        inert_documents.append(&mut owners);
        for _ in 0..tasks.len() { wake.push_back(DeferredPlatformTask::ToggleEvent); }
        toggles.append(&mut tasks);
        Ok(())
    }
}

/// Creator-only native registration. IDs do not root the inert document graph;
/// wrappers and pending notification leases own their independent lifetimes.
pub(super) fn register_inert_document(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    document: NodeId,
) -> Result<(), V8BindingError> {
    if state.execution_retired()
        || !matches!(state.dom.borrow().arena().get(document)?.kind, NodeKind::Document)
    {
        return Err(V8BindingError::Allocation("inert document creator authority"));
    }
    let epoch = state.document_authority_epoch();
    let allocation = state.capture_retained_allocation(
        scope, Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?;
    let mut registry = state.popover_publications.borrow_mut();
    registry.inert_documents.retain(|(document, owner_epoch)|
        *owner_epoch == epoch && state.dom.borrow().arena().get(*document).is_ok());
    if document == state.document_root()
        || registry.inert_documents.iter().any(|(owner, _)| *owner == document)
    { return Ok(()) }
    if registry.inert_documents.len() >= 4096 {
        return Err(V8BindingError::Allocation("inert document publication owner limit"));
    }
    let Registry { inert_documents, inert_document_storage, .. } = &mut *registry;
    reserve_queue_growth(&allocation, inert_documents, inert_document_storage, 1)?;
    inert_documents.push_back((document, epoch));
    Ok(())
}

/// Independent journal ownership covers reflection, Attr, namespace-aware and
/// parser mutations without consuming the observer/custom-element cursors.
pub(super) fn collect_details_mutations(
    scope: &mut v8::PinScope, state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() { return Ok(()) }
    loop {
    let read = state.dom.borrow().next_attribute_presence_change(state.details_toggle_cursor.get(), "open");
    if read.truncated {
        return Err(V8BindingError::Allocation("details toggle mutation journal truncated"));
    }
        if let Some(change) = read.change {
            let node = change.node;
            if state.dom.borrow().is_element(node, "http://www.w3.org/1999/xhtml", "details")
                && state.dom.borrow().node_document(node)?
                    .is_some_and(|document| owns_details_document(state, document))
            {
                let context = state.global_object.to_local(scope)
                    .and_then(|global| global.get_creation_context(scope))
                    .ok_or(V8BindingError::Allocation("details toggle owner context"))?;
                let scope = &mut v8::ContextScope::new(scope, context);
                let _allocation = state.enter_retained_allocation(scope)?;
                queue_with_kind(scope, state, node,
                    if change.was_present { "open" } else { "closed" },
                    if change.is_present { "open" } else { "closed" }, None, true)?;
            }
        }
        // A fallible admission leaves this exact change available for retry;
        // earlier acknowledged changes cannot be replayed after task dispatch.
        state.details_toggle_cursor.set(read.next);
        if read.change.is_none() { break }
    }
    Ok(())
}
pub(super) fn dispatch_toggle(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_tasks_stopped()
        || state.popover_publications.borrow().toggle_dispatching
        || state
            .popover_publications
            .borrow()
            .toggle_retry
            .is_some_and(|deadline| deadline > Instant::now())
    {
        return Ok(0);
    }
    let Some(mut task) = state.popover_publications.borrow_mut().toggles.pop_front() else {
        return Ok(0);
    };
    state.popover_publications.borrow_mut().toggle_dispatching = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.popover_publications.borrow_mut().toggle_dispatching = false;
        }
    }
    let _guard = Guard(state);
    let result = (|| -> Result<bool, V8BindingError> {
        if state.execution_retired()
            || task.epoch != if task.details { state.document_authority_epoch() } else { state.document_stream_epoch() }
            || (if task.details { !owns_details_document(state, task.target.document) }
                else { task.target.document != state.document_root() })
            || state
                .dom
                .borrow()
                .node_document(task.target.node)
                .ok()
                .flatten()
                != Some(task.target.document)
        {
            if let Some(dispatch) = task.dispatch.as_ref() {
                dispatch.cancel(state);
            }
            return Ok(true);
        }
        if task.dispatch.is_none() {
            let source = task
                .source
                .as_ref()
                .filter(|source| {
                    state.dom.borrow().node_document(source.node).ok().flatten()
                        == Some(source.document)
                })
                .map(|source| source.node);
            let event = event(scope, state, "toggle", &task.old, &task.new, source, false)?;
            let target = wrap_node(scope, state, task.target.node)?;
            task.dispatch = Some(native_platform::publication::pending_dom_dispatch(
                scope, state, target, "toggle", event,
            ));
        }
        let dispatch = task.dispatch.as_mut().unwrap();
        if !dispatch.resume(scope, state)? {
            return Ok(false);
        }
        dispatch.finish(scope, state);
        Ok(true)
    })();
    match result {
        Ok(true) => {
            state.popover_publications.borrow_mut().toggle_retry = None;
            Ok(1)
        }
        result => {
            let mut registry = state.popover_publications.borrow_mut();
            registry.toggles.push_front(task);
            registry.toggle_retry = Some(Instant::now() + Duration::from_millis(25));
            result.map(|_| 0)
        }
    }
}

pub(super) fn escape(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if state
        .dom
        .borrow()
        .fullscreen_element(state.document_root())
        .is_some()
    {
        return fullscreen::exit(scope, state, state.document_root());
    }
    let entry = {
        let dom = state.dom.borrow();
        dom.top_layer()
            .filter(|entry| {
                dom.node_document(entry.node).ok().flatten() == Some(state.document_root())
            })
            .filter(|entry| {
                entry.kind == oryn_dom::TopLayerKind::ModalDialog
                    || matches!(
                        entry.kind,
                        oryn_dom::TopLayerKind::SelectPicker
                            | oryn_dom::TopLayerKind::TemporalPicker
                    )
                    || (entry.kind == oryn_dom::TopLayerKind::Popover
                        && !dom
                            .attribute_in_null_namespace(entry.node, "popover")
                            .ok()
                            .flatten()
                            .is_some_and(|v| v.eq_ignore_ascii_case("manual")))
            })
            .last()
    };
    let Some(entry) = entry else { return Ok(false) };
    if entry.kind == oryn_dom::TopLayerKind::Popover {
        toggle(scope, state, entry.node, Some(false), None)?;
        return Ok(true);
    }
    if matches!(
        entry.kind,
        oryn_dom::TopLayerKind::SelectPicker | oryn_dom::TopLayerKind::TemporalPicker
    ) {
        state.dom.borrow_mut().remove_from_top_layer(entry.node);
        state.layout_cache_key.borrow_mut().take();
        return Ok(true);
    }
    let cancel = create_event_object(scope, state, "cancel", false, true)?;
    if !dispatch_event(scope, state, entry.node, cancel)? {
        return Ok(true);
    }
    if !state.dom.borrow().is_modal_dialog(entry.node) {
        return Ok(true);
    }
    state
        .dom
        .borrow_mut()
        .remove_attribute_ns(entry.node, None, "open")?;
    state.layout_cache_key.borrow_mut().take();
    if let Some(target) = entry.previous_focus
        && state.dom.borrow().is_connected(target)?
        && !state.dom.borrow().is_inert(target)?
    {
        focus_html_element(scope, state, target, false)?;
    }
    state
        .deferred_platform_tasks
        .borrow_mut()
        .push_back(DeferredPlatformTask::DomEvent {
            target: DeferredDomTarget::new(&state.dom, entry.node, state.document_root())?,
            event_type: "close",
            bubbles: false,
            stream_epoch: Some(state.document_stream_epoch()),
        });
    Ok(true)
}

pub(super) struct Pointer {
    owner: Rc<V8BindingState>,
    input_owner: native_pointer::InputOwner,
    target: NodeId,
    down: bool,
    prepared: bool,
    done: bool,
    close: Vec<NodeId>,
    child: Option<PendingToggle>,
}
impl Pointer {
    pub(super) fn new(state: &Rc<V8BindingState>, target: NodeId, down: bool) -> Self {
        Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            target,
            down,
            prepared: false,
            done: false,
            close: Vec::new(),
            child: None,
        }
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if !self.input_owner.current() {
            if let Some(child) = &mut self.child {
                child.release(&self.owner)
            }
            self.child.take();
            self.done = true;
            return Ok(true);
        }
        let state = self.owner.clone();
        if !self.prepared {
            if self.down {
                let close = {
                    let dom = state.dom.borrow();
                    let mut ancestors = BTreeSet::new();
                    let mut current = Some(self.target);
                    while let Some(node) = current {
                        ancestors.insert(node);
                        current = dom.flat_tree_parent(node)?;
                    }
                    dom.top_layer()
                        .filter(|entry| {
                            matches!(
                                entry.kind,
                                oryn_dom::TopLayerKind::SelectPicker
                                    | oryn_dom::TopLayerKind::TemporalPicker
                            ) && !ancestors.contains(&entry.node)
                        })
                        .map(|entry| entry.node)
                        .collect::<Vec<_>>()
                };
                if !close.is_empty() {
                    let mut dom = state.dom.borrow_mut();
                    for node in close {
                        dom.remove_from_top_layer(node);
                    }
                    state.layout_cache_key.borrow_mut().take();
                }
            }
            let nearest = {
                let dom = state.dom.borrow();
                let mut current = Some(self.target);
                let mut found = None;
                while let Some(node) = current {
                    if dom.is_popover_open(node) {
                        found = Some(node);
                        break;
                    }
                    if let Some(entry) = dom
                        .top_layer()
                        .filter(|entry| {
                            entry.kind == oryn_dom::TopLayerKind::Popover
                                && entry.source == Some(node)
                        })
                        .last()
                    {
                        found = Some(entry.node);
                        break;
                    }
                    current = dom.flat_tree_parent(node)?;
                }
                found
            };
            if self.down {
                state.dom.borrow_mut().popover_pointer_down(nearest);
                self.done = true;
                return Ok(true);
            }
            if state.dom.borrow_mut().take_popover_pointer_down() != Some(nearest) {
                self.done = true;
                return Ok(true);
            }
            let close = {
                let dom = state.dom.borrow();
                let mut passed = nearest.is_none();
                dom.top_layer()
                    .filter_map(|entry| {
                        if Some(entry.node) == nearest {
                            passed = true;
                            return None;
                        }
                        (passed
                            && entry.kind == oryn_dom::TopLayerKind::Popover
                            && dom.node_document(entry.node).ok().flatten()
                                == Some(state.document_root())
                            && !dom
                                .attribute_in_null_namespace(entry.node, "popover")
                                .ok()
                                .flatten()
                                .is_some_and(|value| value.eq_ignore_ascii_case("manual")))
                        .then_some(entry.node)
                    })
                    .collect::<Vec<_>>()
            };
            self.close = close;
            self.prepared = true;
        }
        while let Some(node) = self.close.last().copied() {
            if !self.input_owner.current() {
                if let Some(child) = &mut self.child {
                    child.release(&state)
                }
                self.child.take();
                self.done = true;
                return Ok(true);
            }
            if self.child.is_none() {
                self.child = Some(PendingToggle::new(&state, node, Some(false), None)?);
            }
            if !self.child.as_mut().unwrap().resume(scope, &state)? {
                return Ok(false);
            }
            self.child.take();
            self.close.pop();
        }
        self.done = true;
        Ok(true)
    }
}
impl Drop for Pointer {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            child.release(&self.owner)
        }
    }
}

pub(super) fn invoker_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
) -> Result<Option<(NodeId, Option<bool>)>, V8BindingError> {
    {
        let dom = state.dom.borrow();
        if !dom.is_element(source, "http://www.w3.org/1999/xhtml", "button")
            && !dom.is_element(source, "http://www.w3.org/1999/xhtml", "input") {
            return Ok(None);
        }
    }
    aria::reconcile_node(scope, state, source)?;
    let target = {
        let dom = state.dom.borrow();
        if !dom.is_element(source, "http://www.w3.org/1999/xhtml", "button")
            && !dom.is_element(source, "http://www.w3.org/1999/xhtml", "input")
        {
            return Ok(None);
        }
        if dom.control_disabled(source)?
            || dom.is_inert(source)?
            || (validation::is_submit_button(&dom, source)? && dom.form_owner(source)?.is_some())
        {
            return Ok(None);
        }
        let Some(target) = dom.popover_target_element(source)?
            .map(|target| dom.resolve_shadow_reference_target(target))
            .transpose()?.flatten() else {
            return Ok(None);
        };
        if dom
            .attribute_in_null_namespace(target, "popover")?
            .is_none()
        {
            return Ok(None);
        }
        let force = dom
            .attribute_in_null_namespace(source, "popovertargetaction")?
            .map(|action| action.to_ascii_lowercase())
            .and_then(|action| match action.as_str() {
                "show" => Some(true),
                "hide" => Some(false),
                _ => None,
            });
        (target, force)
    };
    Ok(Some(target))
}
pub(super) fn invoke(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
) -> Result<bool, V8BindingError> {
    let Some((target, force)) = invoker_target(scope, state, source)? else {
        return Ok(false);
    };
    toggle(scope, state, target, force, Some(source))?;
    Ok(true)
}

/// Retains beforetoggle, nested closing and focus callback progress independently
/// of the caller that initiated the top-layer transition.
pub(super) struct PendingToggle {
    node: NodeId,
    source: Option<NodeId>,
    force: Option<bool>,
    mode: Option<String>,
    was: bool,
    show: bool,
    stage: u8,
    epoch: (NodeId, u64, u64),
    event: Option<native_platform::publication::Notification>,
    event_object: Option<v8::Global<v8::Object>>,
    children: VecDeque<NodeId>,
    child: Option<Box<PendingToggle>>,
    focus: Option<native_platform::focus_publication::Transition>,
    loss: Option<interest::Loss>,
    _leases: Vec<oryn_dom::TrackedNodeLease>,
}
#[derive(Default)]
pub(super) struct Registry {
    queue: VecDeque<PendingToggle>,
    retry: Option<Instant>,
    polling: bool,
    toggles: VecDeque<ToggleTask>,
    inert_documents: VecDeque<(NodeId, u64)>,
    inert_document_storage: Option<(usize, Arc<dyn Send + Sync>)>,
    toggle_storage: Option<(usize, Arc<dyn Send + Sync>)>,
    toggle_retry: Option<Instant>,
    toggle_dispatching: bool,
}
impl PendingToggle {
    pub(super) fn new(
        state: &V8BindingState,
        node: NodeId,
        force: Option<bool>,
        source: Option<NodeId>,
    ) -> Result<Self, V8BindingError> {
        let mut leases = vec![oryn_dom::TrackedNodeLease::new(&state.dom, node)?];
        if let Some(source) = source {
            leases.push(oryn_dom::TrackedNodeLease::new(&state.dom, source)?);
        }
        Ok(Self {
            node,
            source,
            force,
            mode: None,
            was: false,
            show: false,
            stage: 0,
            epoch: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            ),
            event: None,
            event_object: None,
            children: VecDeque::new(),
            child: None,
            focus: None,
            loss: None,
            _leases: leases,
        })
    }
    pub(super) fn release(&mut self, state: &V8BindingState) {
        if let Some(event) = self.event.take() {
            event.cancel(state);
        }
        if let Some(focus) = self.focus.take() {
            focus.cancel(state);
        }
        if let Some(child) = self.child.as_mut() {
            child.release(state);
        }
        if let Some(loss) = self.loss.as_mut() {
            loss.cancel(state);
        }
        self.loss = None;
        state.dom.borrow_mut().end_popover_transition(self.node);
        self.stage = 9;
    }
    pub(super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        loop {
            if state.execution_suspended() {
                return Ok(false);
            }
            let (owner, node) = self._leases[0].current();
            if !Rc::ptr_eq(&owner, &state.dom)
                || node != self.node
                || owner.borrow().node_document(node).ok().flatten() != Some(self.epoch.0)
            {
                self.release(state);
                return Ok(true);
            }
            if state.execution_retired()
                || self.epoch
                    != (
                        state.document_root(),
                        state.document_generation(),
                        state.document_stream_epoch(),
                    )
            {
                self.release(state);
                return Ok(true);
            }
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            match self.stage {
                0 => {
                    let current_mode = mode(state, self.node)?;
                    self.was = state.dom.borrow().is_popover_open(self.node);
                    self.show = self.force.unwrap_or(!self.was);
                    if self.show == self.was {
                        self.stage = 9;
                        return Ok(true);
                    }
                    if !state.dom.borrow().is_connected(self.node)?
                        || state.dom.borrow().node_document(self.node)?
                            != Some(state.document_root())
                        || state.dom.borrow().is_modal_dialog(self.node)
                    {
                        return Err(V8BindingError::Unsupported {
                            interface: "HTMLElement".into(),
                            member: "InvalidStateError".into(),
                        });
                    }
                    if !state.dom.borrow_mut().begin_popover_transition(self.node) {
                        self.stage = 9;
                        return Ok(true);
                    }
                    self.mode = Some(current_mode);
                    self.stage = 1;
                }
                1 => {
                    if self.event.is_none() {
                        let event = event(
                            scope,
                            state,
                            "beforetoggle",
                            if self.was { "open" } else { "closed" },
                            if self.show { "open" } else { "closed" },
                            self.source,
                            self.show,
                        )?;
                        let target = wrap_node(scope, state, self.node)?;
                        self.event = Some(native_platform::publication::pending_dom_dispatch(
                            scope,
                            state,
                            target,
                            "beforetoggle",
                            event,
                        ));
                        self.event_object = Some(v8::Global::new(scope, event));
                    }
                    let event = self.event.as_mut().unwrap();
                    if !event.resume(scope, state)? {
                        return Ok(false);
                    }
                    event.finish(scope, state);
                    let object = v8::Local::new(scope, self.event_object.as_ref().unwrap());
                    let id = event_id(scope, object)?;
                    let cancelled = state
                        .events
                        .borrow()
                        .get(&id)
                        .is_some_and(|event| event.default_prevented);
                    self.event = None;
                    self.event_object = None;
                    if cancelled {
                        self.release(state);
                        return Ok(true);
                    }
                    self.stage = 2;
                }
                2 => {
                    if !state.dom.borrow().is_connected(self.node).unwrap_or(false)
                        || mode(state, self.node).ok() != self.mode
                        || state.dom.borrow().is_popover_open(self.node) != self.was
                    {
                        self.release(state);
                        return Ok(true);
                    }
                    if self.show && self.mode.as_deref() != Some("manual") {
                        let dom = state.dom.borrow();
                        let mut ancestors = BTreeSet::new();
                        for mut cursor in [Some(self.node), self.source] {
                            while let Some(node) = cursor.filter(|node| ancestors.insert(*node)) {
                                cursor = dom.flat_tree_parent(node)?;
                            }
                        }
                        self.children = dom
                            .top_layer()
                            .filter(|entry| {
                                entry.kind == oryn_dom::TopLayerKind::Popover
                                    && !ancestors.contains(&entry.node)
                                    && dom.node_document(entry.node).ok().flatten()
                                        == Some(state.document_root())
                            })
                            .filter(|entry| {
                                dom.attribute_in_null_namespace(entry.node, "popover")
                                    .ok()
                                    .flatten()
                                    .is_some_and(|value| {
                                        !value.eq_ignore_ascii_case("manual")
                                            && (self.mode.as_deref() == Some("auto")
                                                || value.eq_ignore_ascii_case("hint"))
                                    })
                            })
                            .map(|entry| entry.node)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                    }
                    self.stage = 3;
                }
                3 => {
                    if self.child.is_none() {
                        if let Some(node) = self.children.front() {
                            self.child =
                                Some(Box::new(Self::new(state, *node, Some(false), self.source)?));
                        } else {
                            self.stage = 4;
                            continue;
                        }
                    }
                    if !self.child.as_mut().unwrap().resume(scope, state)? {
                        return Ok(false);
                    }
                    self.child = None;
                    self.children.pop_front();
                }
                4 => {
                    if !self.show {
                        if self.loss.is_none() {
                            self.loss = Some(interest::Loss::new(state, self.node));
                        }
                        if !self.loss.as_mut().unwrap().resume(scope, state)? {
                            return Ok(false);
                        }
                        self.loss = None;
                    }
                    self.stage = 7;
                }
                7 => {
                    if !state.dom.borrow().is_connected(self.node).unwrap_or(false)
                        || mode(state, self.node).ok() != self.mode
                        || state.dom.borrow().is_popover_open(self.node) != self.was
                    {
                        self.release(state);
                        return Ok(true);
                    }
                    let previous = state
                        .dom
                        .borrow()
                        .top_layer()
                        .find(|entry| entry.node == self.node)
                        .and_then(|entry| entry.previous_focus);
                    let focus_inside = {
                        let dom = state.dom.borrow();
                        let mut cursor = dom.focused();
                        let mut inside = false;
                        while let Some(node) = cursor {
                            if node == self.node {
                                inside = true;
                                break;
                            }
                            cursor = dom.flat_tree_parent(node)?;
                        }
                        inside
                    };
                    let focus = if self.show {
                        let dom = state.dom.borrow();
                        dom.flat_tree_elements(self.node)?
                            .into_iter()
                            .map(|entry| entry.0)
                            .find(|node| {
                                dom.attribute_in_null_namespace(*node, "autofocus")
                                    .ok()
                                    .flatten()
                                    .is_some()
                            })
                    } else {
                        previous.filter(|_| focus_inside)
                    };
                    if let Some(target) = focus
                        && state.dom.borrow().is_connected(target)?
                        && !state.dom.borrow().is_inert(target)?
                        && let Some(target) = focus_navigation::delegated_target(scope, state, target)?
                    {
                        self.focus = Some(native_platform::focus_publication::Transition::new(
                            state, target,
                        ));
                    }
                    if self.show {
                        let mut dom = state.dom.borrow_mut();
                        dom.add_to_top_layer(self.node, oryn_dom::TopLayerKind::Popover)?;
                        dom.set_top_layer_source(self.node, self.source);
                    } else {
                        state.dom.borrow_mut().remove_from_top_layer(self.node);
                    }
                    state.layout_cache_key.borrow_mut().take();
                    self.stage = 5;
                }
                5 => {
                    if let Some(focus) = self.focus.as_mut()
                        && !focus.resume(scope, state)?
                    {
                        return Ok(false);
                    }
                    self.focus = None;
                    self.stage = 6;
                }
                6 => {
                    queue(
                        scope,
                        state,
                        self.node,
                        if self.was { "open" } else { "closed" },
                        if self.show { "open" } else { "closed" },
                        self.source,
                    )?;
                    self.release(state);
                    return Ok(true);
                }
                _ => return Ok(true),
            }
        }
    }
}
pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    let registry = state.popover_publications.borrow();
    let now = Instant::now();
    let pending = (!registry.queue.is_empty() && !registry.polling)
        .then(|| registry.retry.unwrap_or(now).saturating_duration_since(now));
    let toggle = (!registry.toggles.is_empty() && !registry.toggle_dispatching).then(|| {
        registry
            .toggle_retry
            .unwrap_or(now)
            .saturating_duration_since(now)
    });
    pending.into_iter().chain(toggle).min()
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_tasks_stopped()
        || state.popover_publications.borrow().polling
        || delay(state).is_none_or(|delay| !delay.is_zero())
    {
        return Ok(0);
    }
    let toggle_ready = !state.popover_publications.borrow().toggles.is_empty();
    state.popover_publications.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.popover_publications.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(state);
    let mut count = 0;
    if state
        .popover_publications
        .borrow()
        .retry
        .is_none_or(|deadline| deadline <= Instant::now())
    {
        let budget = state.popover_publications.borrow().queue.len();
        for _ in 0..budget {
            let Some(mut pending) = state.popover_publications.borrow_mut().queue.pop_front()
            else {
                break;
            };
            match pending.resume(scope, state) {
                Ok(true) => {
                    count += 1;
                    state.popover_publications.borrow_mut().retry = None;
                }
                result => {
                    let mut registry = state.popover_publications.borrow_mut();
                    registry.queue.push_front(pending);
                    registry.retry = Some(Instant::now() + Duration::from_millis(25));
                    return result.map(|_| count);
                }
            }
        }
    }
    if toggle_ready {
        count += dispatch_toggle(scope, state)?;
    }
    Ok(count)
}
pub(super) fn retire(state: &V8BindingState) {
    let mut previous = std::mem::take(&mut *state.popover_publications.borrow_mut());
    for pending in &mut previous.queue {
        pending.release(state);
    }
    for task in previous.toggles {
        if let Some(dispatch) = task.dispatch {
            dispatch.cancel(state);
        }
    }
}

pub(super) fn toggle(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    force: Option<bool>,
    source: Option<NodeId>,
) -> Result<bool, V8BindingError> {
    {
        let mut registry = state.popover_publications.borrow_mut();
        if registry.queue.len() >= 4096 {
            return Err(V8BindingError::Allocation("popover publication limit"));
        }
        registry
            .queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("popover publication reserve"))?;
    }
    let mut pending = PendingToggle::new(state, node, force, source)?;
    let result = pending.resume(scope, state);
    if !matches!(result, Ok(true)) && pending.stage > 0 && pending.stage < 9 {
        let mut registry = state.popover_publications.borrow_mut();
        registry.queue.push_back(pending);
        registry.retry = Some(Instant::now() + Duration::from_millis(25));
    }
    result.map(|_| state.dom.borrow().is_popover_open(node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_dom::html;
    fn inert_details_owner_fixture<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        state: &Rc<V8BindingState>,
    ) -> NodeId {
        let source = v8::String::new(scope, r#"
            globalThis.ownerDocument=new DOMParser().parseFromString('<details></details>','text/html');
            globalThis.ownerDetails=ownerDocument.querySelector('details');
            globalThis.ownerEvents=[];
            ownerDetails.ontoggle=e=>ownerEvents.push(e.oldState+'|'+e.newState+'|'+e.isTrusted);
            ownerDetails;
        "#).unwrap();
        let value = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        let (_, node) = node_identity::argument(scope, value).unwrap().unwrap();
        let document = state.dom.borrow().node_document(node).unwrap().unwrap();
        register_inert_document(scope, state, document).unwrap();
        node
    }

    #[test]
    fn inert_details_owner_dispatches_native_task_in_actual_document() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = inert_details_owner_fixture(scope, &state);
        queue_with_kind(scope, &state, node, "closed", "open", None, true).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "ownerEvents.join(',')==='closed|open|true'&&ownerDetails.ownerDocument===ownerDocument").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn inert_details_owner_cancels_task_after_adoption() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = inert_details_owner_fixture(scope, &state);
        queue_with_kind(scope, &state, node, "closed", "open", None, true).unwrap();
        let source = v8::String::new(scope, "document.body.appendChild(ownerDetails)").unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "ownerEvents.length===0&&ownerDetails.ownerDocument===document").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn dom_parser_details_initial_toggle_uses_inert_document_owner() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"
            globalThis.parsedDetailsDocument=new DOMParser().parseFromString('<details open></details>','text/html');
            globalThis.parsedDetails=parsedDetailsDocument.querySelector('details');
            globalThis.parsedToggleEvents=[];
            parsedDetails.ontoggle=e=>parsedToggleEvents.push([
                e.oldState,e.newState,e.isTrusted,e.target===parsedDetails,
                parsedDetails.ownerDocument===parsedDetailsDocument
            ].join('|'));
        "#).unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        collect_slot_notifications(scope).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "parsedToggleEvents.join(',')==='closed|open|true|true|true'").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn dom_parser_details_later_mutation_retains_creator_publication() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"
            globalThis.laterDoc=new DOMParser().parseFromString('<details></details>','text/html');
            globalThis.laterDetails=laterDoc.querySelector('details');
            globalThis.laterEvents=[];
            laterDetails.ontoggle=e=>laterEvents.push(e.oldState+'|'+e.newState);
            laterDetails.open=true;
        "#).unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        collect_slot_notifications(scope).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "laterEvents.join(',')==='closed|open'&&laterDetails.ownerDocument===laterDoc").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn dom_parser_details_template_later_mutation_uses_separate_owner() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"
            globalThis.templateDoc=new DOMParser().parseFromString(
                '<template id=t><details></details></template>','text/html');
            globalThis.templateDetails=templateDoc.getElementById('t').content.querySelector('details');
            globalThis.templateEvents=[];
            templateDetails.ontoggle=e=>templateEvents.push(e.oldState+'|'+e.newState+'|'+e.isTrusted);
            templateDetails.open=true;
        "#).unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        collect_slot_notifications(scope).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "templateEvents.join(',')==='closed|open|true'&&templateDetails.ownerDocument!==templateDoc&&templateDetails.ownerDocument!==document").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn inert_details_missing_import_identity_does_not_publish_partial_batch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (_, runtime) = html::parse_inert_dom_string_with_runtime("<details open></details>");
        let mut batch = InertDetailsImport::capture(scope, &state, &runtime).unwrap();
        batch.imported(runtime.borrow().document(), state.document_root(), false).unwrap();
        let owners = state.popover_publications.borrow().inert_documents.len();
        let tasks = state.popover_publications.borrow().toggles.len();
        let wake = state.deferred_platform_tasks.borrow().len();
        assert!(batch.publish(&state, state.document_root()).is_err());
        assert_eq!(state.popover_publications.borrow().inert_documents.len(), owners);
        assert_eq!(state.popover_publications.borrow().toggles.len(), tasks);
        assert_eq!(state.deferred_platform_tasks.borrow().len(), wake);
    }

    #[test]
    fn inert_details_publication_rejects_foreign_and_stale_creator() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let first = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let (_, runtime) = html::parse_inert_dom_string_with_runtime("<body></body>");
        let mut foreign = InertDetailsImport::capture(scope, &first, &runtime).unwrap();
        foreign.imported(runtime.borrow().document(), first.document_root(), false).unwrap();
        let other_context = v8::Context::new(scope, Default::default());
        let other_scope = &mut v8::ContextScope::new(scope, other_context);
        let other = install_generated_bindings(other_scope, other_context, dom, RealmId(1), 1).unwrap();
        assert!(foreign.publish(&other, other.document_root()).is_err());
        assert!(other.popover_publications.borrow().inert_documents.is_empty());
        let mut stale = InertDetailsImport::capture(other_scope, &other, &runtime).unwrap();
        stale.imported(runtime.borrow().document(), other.document_root(), false).unwrap();
        other.document_authority_epoch.set(other.document_authority_epoch()+1);
        assert!(stale.publish(&other, other.document_root()).is_err());
        assert!(other.popover_publications.borrow().inert_documents.is_empty());
    }

    #[test]
    fn inert_details_publication_rejects_retired_creator() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (_, runtime) = html::parse_inert_dom_string_with_runtime("<body></body>");
        let mut batch = InertDetailsImport::capture(scope, &state, &runtime).unwrap();
        batch.imported(runtime.borrow().document(), state.document_root(), false).unwrap();
        state.retire_execution();
        assert!(batch.publish(&state, state.document_root()).is_err());
        assert!(state.popover_publications.borrow().inert_documents.is_empty());
        assert!(state.popover_publications.borrow().toggles.is_empty());
    }

    #[test]
    fn inert_details_publication_capacity_refusal_keeps_existing_queues() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = inert_details_owner_fixture(scope, &state);
        let document = state.dom.borrow().node_document(node).unwrap().unwrap();
        let (_, runtime) = html::parse_inert_dom_string_with_runtime("<details open></details>");
        let mut batch = InertDetailsImport::capture(scope, &state, &runtime).unwrap();
        let original = runtime.borrow().next_attribute_presence_change(oryn_dom::MutationCursor(0), "open").change.unwrap().node;
        batch.imported(runtime.borrow().document(), document, false).unwrap();
        batch.imported(original, node, false).unwrap();
        let allocation = state.capture_retained_allocation(scope,
            Arc::new(std::sync::atomic::AtomicBool::new(false))).unwrap();
        {
            let mut wake = state.deferred_platform_tasks.borrow_mut();
            let additional = 4096-wake.len();
            reserve_queue_growth(&allocation, &mut wake,
                &mut state.deferred_task_queue_storage.borrow_mut(), additional).unwrap();
            for _ in 0..additional { wake.push_back(DeferredPlatformTask::ToggleEvent); }
        }
        let owners = state.popover_publications.borrow().inert_documents.len();
        let tasks = state.popover_publications.borrow().toggles.len();
        assert!(batch.publish(&state, document).is_err());
        assert_eq!(state.popover_publications.borrow().inert_documents.len(), owners);
        assert_eq!(state.popover_publications.borrow().toggles.len(), tasks);
        assert_eq!(state.deferred_platform_tasks.borrow().len(), 4096);
    }

    #[test]
    fn dom_parser_details_group_preserves_initial_coalesced_transitions() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"
            globalThis.groupDoc=new DOMParser().parseFromString(
                '<details id=first name=g open></details><details id=second name=g open></details>','text/html');
            globalThis.groupEvents=[];
            for(const details of groupDoc.querySelectorAll('details'))
                details.ontoggle=e=>groupEvents.push(details.id+'|'+e.oldState+'|'+e.newState);
        "#).unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        collect_slot_notifications(scope).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
        let source = v8::String::new(scope,
            "groupEvents.join(',')==='first|closed|open,second|closed|closed'&&groupDoc.getElementById('first').open&&!groupDoc.getElementById('second').open").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }

    #[test]
    fn interrupted_beforetoggle_keeps_operation_and_listener_cursor() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse(
                "<body><button id=s></button><div id=p popover><input autofocus></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,"globalThis.calls=[];globalThis.p=document.getElementById('p');p.addEventListener('beforetoggle',()=>calls.push(1));p.addEventListener('beforetoggle',()=>calls.push(2));p.querySelector('input').addEventListener('focus',()=>calls.push(3));").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let target = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "p")
            .unwrap()
            .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(toggle(scope, &state, target, Some(true), None).is_err());
        assert_eq!(state.popover_publications.borrow().queue.len(), 1);
        assert!(delay(&state).unwrap() > Duration::ZERO);
        assert!(!state.dom.borrow().is_popover_open(target));
        state.popover_publications.borrow_mut().retry = None;
        poll(scope, &state).unwrap();
        let source = v8::String::new(
            scope,
            "calls.join(',')==='1,2,3'&&p.matches(':popover-open')",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        assert!(state.popover_publications.borrow().queue.is_empty());
    }
    #[test]
    fn queued_toggle_keeps_listener_progress_after_wake_task_is_consumed() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body><div id=p popover></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,"globalThis.calls=[];globalThis.p=document.getElementById('p');p.addEventListener('toggle',e=>calls.push('first:'+e.newState));p.addEventListener('toggle',e=>calls.push('second:'+e.newState));p.showPopover();").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(state.drain_platform_completions(scope, 32).is_err());
        assert_eq!(state.popover_publications.borrow().toggles.len(), 1);
        assert!(delay(&state).unwrap() > Duration::ZERO);
        state.popover_publications.borrow_mut().toggle_retry = None;
        poll(scope, &state).unwrap();
        let source = v8::String::new(scope, "calls.join(',')==='first:open,second:open'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn popover_invoker_reflection_drives_native_activation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body><button id='source' type='button'></button><div id='p' popover></div></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"(()=>{
            const eq=(a,b,m)=>{if(a!==b)throw Error(m)};
            const p=document.getElementById('p');
            for(const tag of ['button','input']) {
                const e=document.createElement(tag);document.body.append(e);
                eq(e.popoverTargetElement,null,'unset target');eq(e.popoverTargetAction,'toggle','unset action');
                e.setAttribute('popovertarget','p');eq(e.popoverTargetElement,p,'native ID lookup');
                p.id='changed';eq(e.popoverTargetElement,null,'live ID lookup');p.id='p';
                e.popoverTargetElement=p;eq(e.popoverTargetElement,p,'explicit target');
                eq(e.getAttribute('popovertarget'),'','explicit attribute storage');
                p.id='changed';eq(e.popoverTargetElement,p,'explicit identity survives ID change');p.id='p';
                for(const [raw,expected] of [['SHOW','show'],['Hide','hide'],['toggle','toggle'],['invalid','toggle'],['','toggle']]) {
                    e.popoverTargetAction=raw;eq(e.popoverTargetAction,expected,'canonical action');
                    eq(e.getAttribute('popovertargetaction'),raw,'raw action');
                }
                let rejected=false;try{e.popoverTargetElement={}}catch(error){rejected=error instanceof TypeError}
                eq(rejected,true,'Element conversion');eq(e.popoverTargetElement,p,'failed conversion preserves target');
                const foreign=document.implementation.createHTMLDocument('other');foreign.adoptNode(e);
                eq(e.popoverTargetElement,null,'different document excludes target');document.body.append(e);
                eq(e.popoverTargetElement,p,'return to document preserves identity');
                e.setAttribute('popovertarget','missing');eq(e.popoverTargetElement,null,'attribute replacement invalidates explicit target');
                e.popoverTargetElement=null;eq(e.hasAttribute('popovertarget'),false,'null removes attribute');
                e.remove();
            }
            const b=document.getElementById('source');b.popoverTargetElement=p;b.popoverTargetAction='SHOW';
            return true;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
        let node = state.dom.borrow().find_by_id("source").unwrap();
        assert!(invoke(scope, &state, node).unwrap());
        let source = v8::String::new(scope, "document.getElementById('p').matches(':popover-open')").unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }
    #[test]
    fn popover_native_state_reentrancy_and_coalesced_toggle_payload() {
        super::super::tests::initialize_v8();
        let parsed = html::parse(
            "<body><button id='source'>Open</button><div id='p' popover></div><div id='q' popover></div></body>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"
            const p=document.getElementById('p'),q=document.getElementById('q'),source=document.getElementById('source');
            globalThis.records=[];
            p.addEventListener('beforetoggle',e=>{if(e.newState==='open')p.showPopover();});
            p.addEventListener('toggle',e=>records.push([e instanceof ToggleEvent,e.oldState,e.newState,e.source===source,e.isTrusted]));
            p.showPopover({source});if(!p.matches(':popover-open'))throw Error('native state');
            p.hidePopover();p.showPopover({source});
            if(records.length!==0)throw Error('toggle must be a task');
            const event=new ToggleEvent('toggle',{oldState:'closed',newState:'open',source});
            if(event.oldState!=='closed'||event.newState!=='open'||event.source!==source)throw Error('constructor payload');
            true
        "#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .boolean_value(scope)
        );
        state.drain_platform_completions(scope, 32).unwrap();
        let source=v8::String::new(scope,"records.length===1 && JSON.stringify(records[0])==='[true,\"closed\",\"open\",true,true]'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .boolean_value(scope)
        );
    }
}
