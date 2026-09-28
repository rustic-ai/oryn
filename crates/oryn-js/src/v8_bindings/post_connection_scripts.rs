//! Retained post-connection batch suffixes. Nested insertions execute their own
//! batch synchronously; infrastructure failure preserves only unfinished work.
use super::*;
#[derive(Default)]
pub(super) struct Registry {
    queue: VecDeque<Batch>,
    admitted: Rc<Cell<usize>>,
    draining: bool,
}
struct Job {
    target: oryn_dom::TrackedNodeLease,
    owner: RcWeak<V8BindingState>,
    origin: RcWeak<V8BindingState>,
    document: oryn_dom::NodeRef,
    generation: u64,
    stream_epoch: u64,
    meta: bool,
}
struct Resource {
    job: Job,
    stage: u8,
    prepared_document: oryn_dom::NodeRef,
}
struct Batch {
    stream_completion: Option<(NodeId, Rc<Cell<bool>>)>,
    resources: VecDeque<Resource>,
    checkpointed: bool,
    jobs: VecDeque<Job>,
    count: usize,
    admitted: Rc<Cell<usize>>,
    retry: std::time::Instant,
}
impl Drop for Batch {
    fn drop(&mut self) {
        self.admitted.set(self.admitted.get() - self.count);
    }
}
pub(super) fn registry(scope: &mut v8::PinScope) -> Rc<RefCell<Registry>> {
    if let Some(registry) = scope.get_slot::<Rc<RefCell<Registry>>>() {
        return registry.clone();
    }
    let registry = Rc::new(RefCell::new(Registry::default()));
    scope.set_slot(registry.clone());
    registry
}
fn owner(
    scope: &mut v8::PinScope,
    target: &oryn_dom::TrackedNodeLease,
) -> Option<(Rc<V8BindingState>, NodeId)> {
    let (dom, node) = target.current();
    let document = dom.borrow().node_document(node).ok().flatten()?;
    let registry = scope.get_slot::<BindingStateRegistry>()?;
    let owner = registry
        .realms
        .borrow()
        .values()
        .filter_map(RcWeak::upgrade)
        .find(|state| {
            !state.execution_retired()
                && Rc::ptr_eq(&state.dom, &dom)
                && state.document_root() == document
        })?;
    Some((owner, node))
}
impl Batch {
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        while let Some(resource) = self.resources.front_mut() {
            let job = &mut resource.job;
            let Some((state, node)) = owner(scope, &job.target) else {
                self.resources.pop_front();
                continue;
            };
            let document = oryn_dom::NodeRef {
                arena: state.dom.borrow().arena().id(),
                node: state.document_root(),
            };
            if document == job.document
                && job.origin.upgrade().is_none_or(|origin| {
                    origin.execution_retired()
                        || origin.document_generation() != job.generation
                        || origin.document_stream_epoch() != job.stream_epoch
                })
            {
                self.resources.pop_front();
                continue;
            }
            job.owner = Rc::downgrade(&state);
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if state.execution_tasks_stopped()
                || !state.dom.borrow().is_connected(node).unwrap_or(false)
            {
                self.resources.pop_front();
                continue;
            }
            if document != resource.prepared_document {
                resource.stage = 0;
                resource.prepared_document = document;
            }
            let Some(context) = state
                .global_object
                .to_local(scope)
                .and_then(|global| global.get_creation_context(scope))
            else {
                return Ok(false);
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            #[cfg(test)]
            if FAIL_RESOURCE_STAGE.with(|fail| {
                if fail.get() == Some(resource.stage) {
                    fail.set(None);
                    true
                } else {
                    false
                }
            }) {
                return Err(V8BindingError::Allocation(
                    "injected insertion resource preparation",
                ));
            }
            match resource.stage {
                0 => prepare_dynamic_frame_subtree(&state, node)?,
                1 => {
                    let image_root = {
                        let dom = state.dom.borrow();
                        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "source") {
                            dom.parent(node)?.unwrap_or(node)
                        } else {
                            node
                        }
                    };
                    prepare_dynamic_image_subtree(scope, &state, image_root)?;
                }
                2 => prepare_dynamic_stylesheet_subtree(&state, node)?,
                3 => prepare_native_module_preloads(scope, &state, node)?,
                _ => {
                    self.resources.pop_front();
                    continue;
                }
            }
            resource.stage += 1;
        }
        if !self.checkpointed {
            self.checkpointed = true;
            checkpoint_dom_lifecycle(scope);
        }
        while let Some(job) = self.jobs.front_mut() {
            let Some((state, node)) = owner(scope, &job.target) else {
                self.jobs.pop_front();
                continue;
            };
            let document = oryn_dom::NodeRef {
                arena: state.dom.borrow().arena().id(),
                node: state.document_root(),
            };
            if document == job.document
                && job.origin.upgrade().is_none_or(|origin| {
                    origin.execution_retired()
                        || origin.document_generation() != job.generation
                        || origin.document_stream_epoch() != job.stream_epoch
                })
            {
                self.jobs.pop_front();
                continue;
            }
            job.owner = Rc::downgrade(&state);
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if state.execution_tasks_stopped()
                || !state.dom.borrow().is_connected(node).unwrap_or(false)
            {
                self.jobs.pop_front();
                continue;
            }
            if state.has_pending_script_error_reports() {
                return Ok(false);
            }
            let Some(context) = state
                .global_object
                .to_local(scope)
                .and_then(|global| global.get_creation_context(scope))
            else {
                return Ok(false);
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            if job.meta {
                prepare_dynamic_referrer_meta(scope, &state, node, true)?;
            } else {
                #[cfg(test)]
                if FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.replace(false)) {
                    return Err(V8BindingError::Allocation(
                        "injected post-connection preparation",
                    ));
                }
                if let Some(source) = prepare_inline_script_or_queue(scope, &state, node)? {
                    let nonce = state
                        .dom
                        .borrow()
                        .element_nonce_for_csp(node)?
                        .map(str::to_owned);
                    if state.authorize_inline_script(
                        &source.policy_text(),
                        nonce.as_deref(),
                        false,
                    )? {
                        execute_inline_classic_script(scope, &state, node, &source)?;
                    } else {
                        state.mark_script_started(node);
                    }
                }
            }
            // Attach before the checkpoint can yield and let the native scheduler
            // take ownership. A later lookup in the pending queue is too late.
            if self
                .stream_completion
                .as_ref()
                .is_some_and(|(target, _)| *target == node)
            {
                let (_, done) = self.stream_completion.take().unwrap();
                if let Some(prepared) = state
                    .pending_dynamic_scripts
                    .borrow_mut()
                    .iter_mut()
                    .find(|job| job.node == node)
                {
                    prepared.stream_completion = Some(html_stream::ScriptCompletion(done));
                } else {
                    done.set(true);
                }
            }
            // Remove before checkpoint: callbacks can start nested insertion batches.
            self.jobs.pop_front();
            scope.perform_microtask_checkpoint();
            if scope.is_execution_terminating() {
                return Ok(false);
            }
        }
        if let Some((_, done)) = self.stream_completion.take() {
            done.set(true);
        }
        Ok(true)
    }
}
fn job(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    meta: bool,
) -> Result<Option<Job>, V8BindingError> {
    #[cfg(test)]
    if FAIL_CAPTURE_AFTER.with(|fail| match fail.get() {
        Some(0) => {
            fail.set(None);
            true
        }
        Some(left) => {
            fail.set(Some(left - 1));
            false
        }
        None => false,
    }) {
        return Err(V8BindingError::Allocation("injected insertion capture"));
    }

    let target = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    let initial = state;
    let document = oryn_dom::NodeRef {
        arena: initial.dom.borrow().arena().id(),
        node: initial.document_root(),
    };
    Ok(Some(Job {
        target,
        owner: Rc::downgrade(initial),
        origin: Rc::downgrade(initial),
        document,
        generation: initial.document_generation(),
        stream_epoch: initial.document_stream_epoch(),
        meta,
    }))
}
/// Check insertion work before any source adoption or destination mutation.
/// Native hierarchy checks must precede this implementation capacity check.
pub(super) fn check_insertion_capacity(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    source: &oryn_dom::DomRuntime,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let roots = if matches!(source.arena().get(node)?.kind, NodeKind::DocumentFragment) {
        light_children(source, node)?
    } else {
        vec![node]
    };
    check_roots_capacity(scope, state, target, source, &roots)
}
/// HTML setters already have a complete detached fragment. No author callback
/// occurs between this check and native replacement/preparation admission.
pub(super) fn check_fragment_capacity(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    fragment: &oryn_dom::DomRuntime,
    root: NodeId,
) -> Result<(), V8BindingError> {
    check_roots_capacity(
        scope,
        state,
        target,
        fragment,
        &fragment.arena().get(root)?.children,
    )
}
fn check_roots_capacity(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    source: &oryn_dom::DomRuntime,
    roots: &[NodeId],
) -> Result<(), V8BindingError> {
    if !state.dom.borrow().is_connected(target)?
        || active_document_state_for_node(scope, state, target).is_none()
    {
        return Ok(());
    }
    let mut count = usize::from(script_children_changed::is_script_parent(state, target)?);
    for &child in roots {
        count = count.saturating_add(1);
        let mut pending = vec![child];
        while let Some(node) = pending.pop() {
            let record = source.arena().get(node)?;
            if source.is_element(node, "http://www.w3.org/1999/xhtml", "meta")
                || is_script_element(source, node)
            {
                count = count.saturating_add(1);
            }
            if count > 4096 {
                return Err(V8BindingError::Allocation(
                    "post-connection admission capacity",
                ));
            }
            pending.extend(record.children.iter().rev().copied());
        }
    }
    let mut registry = state.post_connection_scripts.borrow_mut();
    if registry.admitted.get().saturating_add(count) > 4096 {
        return Err(V8BindingError::Allocation(
            "post-connection admission capacity",
        ));
    }
    registry
        .queue
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("post-connection batch queue"))?;
    Ok(())
}
/// Capture all roots and post-connection targets before starting resources.
/// Each resource phase advances only after success; scripts start only after
/// all resource phases and the old-document lifecycle checkpoint finish.
/// Captured native leases and queue budget outlive the DOM mutation. Dropping
/// this before commit releases every reservation without running resources.
pub(super) struct PreparedInsertion {
    registry: Rc<RefCell<Registry>>,
    batch: Option<Batch>,
}
impl PreparedInsertion {
    /// A native continuation owns this batch directly across retries. The caller
    /// has already committed its mutation segment before polling authored work.
    pub(super) fn poll_retained(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        let Some(batch) = self.batch.as_mut() else {
            return Ok(true);
        };
        if batch.poll(scope)? {
            self.batch.take();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn finish_mutation<T>(
        self,
        scope: &mut v8::PinScope,
        result: Result<T, DomError>,
    ) -> Result<T, V8BindingError> {
        self.finish_result(scope, result.map_err(Into::into))
    }
    pub(super) fn finish_result<T>(
        mut self,
        scope: &mut v8::PinScope,
        result: Result<T, V8BindingError>,
    ) -> Result<T, V8BindingError> {
        match result {
            Ok(value) => {
                self.finish(scope)?;
                Ok(value)
            }
            Err(error) => {
                if let Some(mut batch) = self.batch.take() {
                    let connected = |job: &Job| {
                        let (dom, node) = job.target.current();

                        dom.borrow().is_connected(node).unwrap_or(false)
                    };
                    batch.resources.retain(|resource| connected(&resource.job));
                    batch.jobs.retain(connected);
                    if !batch.resources.is_empty() || !batch.jobs.is_empty() {
                        batch.retry =
                            std::time::Instant::now() + std::time::Duration::from_millis(10);
                        self.registry.borrow_mut().queue.push_back(batch);
                    }
                }
                Err(error)
            }
        }
    }

    pub(super) fn finish(mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        let Some(mut batch) = self.batch.take() else {
            checkpoint_dom_lifecycle(scope);
            return Ok(());
        };
        match batch.poll(scope) {
            Ok(true) => Ok(()),
            pending => {
                batch.retry = std::time::Instant::now() + std::time::Duration::from_millis(10);
                self.registry.borrow_mut().queue.push_back(batch);
                pending.map(|_| ())
            }
        }
    }
}
pub(super) fn capture_replacement(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    roots: &[NodeId],
) -> Result<PreparedInsertion, V8BindingError> {
    let destination = if state.dom.borrow().is_connected(target)? {
        active_document_state_for_node(scope, state, target)
    } else {
        None
    };
    let batch = match destination {
        Some(destination) => capture_batch(scope, state, roots, Some(&destination), Some(target))?,
        None => None,
    };
    Ok(PreparedInsertion {
        registry: state.post_connection_scripts.clone(),
        batch,
    })
}
pub(super) fn capture_insertion(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parent: NodeId,
    insertion: NodeId,
) -> Result<PreparedInsertion, V8BindingError> {
    let destination = if state.dom.borrow().is_connected(parent)? {
        active_document_state_for_node(scope, state, parent)
    } else {
        None
    };
    let roots = {
        let dom = state.dom.borrow();
        if matches!(dom.arena().get(insertion)?.kind, NodeKind::DocumentFragment) {
            light_children(&dom, insertion)?
        } else {
            vec![insertion]
        }
    };
    let batch = match destination {
        Some(destination) => capture_batch(scope, state, &roots, Some(&destination), Some(parent))?,
        None => None,
    };
    Ok(PreparedInsertion {
        registry: state.post_connection_scripts.clone(),
        batch,
    })
}
fn capture_batch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    roots: &[NodeId],
    destination: Option<&Rc<V8BindingState>>,
    changed_parent: Option<NodeId>,
) -> Result<Option<Batch>, V8BindingError> {
    let mut resources = VecDeque::new();
    let mut jobs = VecDeque::new();
    let mut script_jobs = BTreeSet::new();
    for &root in roots {
        if destination.is_none() && !state.dom.borrow().is_connected(root)? {
            continue;
        }
        let Some(initial) =
            active_document_state_for_node(scope, state, root).or_else(|| destination.cloned())
        else {
            continue;
        };
        let candidates = {
            let dom = initial.dom.borrow();
            let mut candidates = vec![root];
            candidates.extend(dom.query_selector_all(root, "script, meta")?);
            candidates
                .into_iter()
                .filter_map(|node| {
                    if dom.is_element(node, "http://www.w3.org/1999/xhtml", "meta") {
                        Some((node, true))
                    } else if is_script_element(&dom, node) {
                        Some((node, false))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        if resources
            .len()
            .saturating_add(jobs.len())
            .saturating_add(candidates.len())
            .saturating_add(1)
            > 4096
        {
            return Err(V8BindingError::Allocation(
                "post-connection admission capacity",
            ));
        }
        resources
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("insertion resource roots"))?;
        if let Some(job) = job(scope, &initial, root, false)? {
            let prepared_document = job.document;
            resources.push_back(Resource {
                job,
                stage: 0,
                prepared_document,
            });
        }
        jobs.try_reserve(candidates.len())
            .map_err(|_| V8BindingError::Allocation("post-connection script jobs"))?;
        for (node, meta) in candidates {
            if script_jobs.insert(node)
                && let Some(job) = job(scope, &initial, node, meta)?
            {
                jobs.push_back(job);
            }
        }
    }
    // Children-changed preparation belongs to the connected parent script,
    // independently of whether any inserted root is itself a script.
    if let Some(parent) = changed_parent
        && script_children_changed::is_script_parent(state, parent)?
        && script_jobs.insert(parent)
    {
        jobs.try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("script children-changed job"))?;
        if let Some(job) = job(scope, state, parent, false)? {
            jobs.push_back(job);
        }
    }
    if resources.is_empty() && jobs.is_empty() {
        return Ok(None);
    }
    let registry = state.post_connection_scripts.clone();
    let count = resources.len() + jobs.len();
    let admitted = {
        let mut registry = registry.borrow_mut();
        if registry.admitted.get().saturating_add(count) > 4096 {
            return Err(V8BindingError::Allocation(
                "post-connection admission capacity",
            ));
        }
        registry
            .queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("post-connection batch queue"))?;
        registry.admitted.clone()
    };
    admitted.set(admitted.get() + count);
    let batch = Batch {
        stream_completion: None,
        resources,
        checkpointed: false,
        jobs,
        count,
        admitted,
        retry: std::time::Instant::now(),
    };
    Ok(Some(batch))
}
/// The HTML stream retains this batch until every preparation step is accepted.
pub(super) fn capture_stream_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    completion: Option<Rc<Cell<bool>>>,
) -> Result<PreparedInsertion, V8BindingError> {
    let mut batch = capture_batch(scope, state, &[node], None, None)?;
    if let Some(done) = completion {
        if let Some(batch) = batch.as_mut() {
            batch.stream_completion = Some((node, done));
        } else {
            done.set(true);
        }
    }
    Ok(PreparedInsertion {
        registry: state.post_connection_scripts.clone(),
        batch,
    })
}
pub(super) fn enqueue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    roots: &[NodeId],
) -> Result<(), V8BindingError> {
    if let Some(batch) = capture_batch(scope, state, roots, None, None)? {
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .push_back(batch);
    }
    Ok(())
}
pub(super) fn prepare(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    roots: &[NodeId],
) -> Result<(), V8BindingError> {
    let batch = capture_batch(scope, state, roots, None, None)?;
    PreparedInsertion {
        registry: state.post_connection_scripts.clone(),
        batch,
    }
    .finish(scope)
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    let now = std::time::Instant::now();
    state
        .post_connection_scripts
        .borrow()
        .queue
        .iter()
        .filter(|batch| {
            batch
                .resources
                .front()
                .map(|resource| &resource.job)
                .or_else(|| batch.jobs.front())
                .is_none_or(|job| {
                    job.owner.upgrade().is_none_or(|owner| {
                        let (dom, node) = job.target.current();
                        !owner.execution_suspended()
                            || !Rc::ptr_eq(&dom, &owner.dom)
                            || dom.borrow().node_document(node).ok().flatten()
                                != Some(owner.document_root())
                    })
                })
        })
        .map(|batch| batch.retry.saturating_duration_since(now))
        .min()
}
pub(super) fn checkpoint_pending(state: &V8BindingState) -> bool {
    state
        .post_connection_scripts
        .try_borrow()
        .map_or(true, |registry| {
            !registry.queue.is_empty() || registry.draining || registry.admitted.get() != 0
        })
}

// Exact empty-queue check; future retries and other owners remain active.
pub(super) fn queue_empty(state: &V8BindingState) -> bool {
    state.post_connection_scripts.borrow().queue.is_empty()
}

pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let registry = state.post_connection_scripts.clone();
    if registry.borrow().draining {
        return Ok(0);
    }
    registry.borrow_mut().draining = true;
    struct Guard(Rc<RefCell<Registry>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.borrow_mut().draining = false;
        }
    }
    let _guard = Guard(registry.clone());
    let count = registry.borrow().queue.len().min(32);
    let mut delivered = 0;
    for _ in 0..count {
        let Some(mut batch) = registry.borrow_mut().queue.pop_front() else {
            break;
        };
        if batch.retry > std::time::Instant::now() {
            registry.borrow_mut().queue.push_back(batch);
            continue;
        }
        match batch.poll(scope) {
            Ok(true) => delivered += 1,
            pending => {
                batch.retry = std::time::Instant::now() + std::time::Duration::from_millis(10);
                registry.borrow_mut().queue.push_back(batch);
                pending?;
            }
        }
    }
    Ok(delivered)
}
#[cfg(test)]
thread_local! {static FAIL_POST_CONNECTION_PREPARE:Cell<bool>=const{Cell::new(false)};static FAIL_CAPTURE_AFTER:Cell<Option<usize>>=const{Cell::new(None)};static FAIL_RESOURCE_STAGE:Cell<Option<u8>>=const{Cell::new(None)};}

pub(super) fn retire(state: &V8BindingState) {
    let document = oryn_dom::NodeRef {
        arena: state.dom.borrow().arena().id(),
        node: state.document_root(),
    };
    let mut registry = state.post_connection_scripts.borrow_mut();
    for batch in &mut registry.queue {
        let keep = |job: &Job| {
            let (dom, node) = job.target.current();
            let current = oryn_dom::NodeRef {
                arena: dom.borrow().arena().id(),
                node: dom
                    .borrow()
                    .node_document(node)
                    .ok()
                    .flatten()
                    .unwrap_or(node),
            };
            job.document != document || current != document
        };
        batch.resources.retain(|resource| keep(&resource.job));
        batch.jobs.retain(keep);
    }
    registry
        .queue
        .retain(|batch| !batch.resources.is_empty() || !batch.jobs.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn native_fragment_enqueue_commits_before_fallible_poll_without_recapture() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(317), 1).unwrap();
        eval(scope, "globalThis.nativeFragmentRuns=0");
        let node = {
            let mut dom = state.dom.borrow_mut();
            let parent = dom.find_by_id("target").unwrap();
            let script = dom.create_element("script");
            dom.set_text_content(script, "nativeFragmentRuns++")
                .unwrap();
            dom.append_child(parent, script).unwrap();
            script
        };
        prepare_native_fragment_nodes(scope, &state, &[node]).unwrap();
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        FAIL_POST_CONNECTION_PREPARE.with(|failure| failure.set(true));
        assert!(poll(scope, &state).is_err());
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        eval(
            scope,
            "if(nativeFragmentRuns!==0)throw Error('premature execution')",
        );
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
        eval(
            scope,
            "if(nativeFragmentRuns!==1)throw Error('lost native event')",
        );
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(nativeFragmentRuns!==1)throw Error('replayed native event')",
        );
    }
    #[test]
    fn script_children_change_retains_preparation_after_mutation_failure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.changedRuns=0;globalThis.changedScript=document.createElement('script');globalThis.changedText=document.createTextNode('');changedScript.append(changedText);document.body.append(changedScript)",
        );
        FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
        eval(
            scope,
            "try{changedText.data='changedRuns++'}catch(error){};if(changedRuns!==0||changedText.data!=='changedRuns++')throw Error('mutation or premature execution')",
        );
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(changedRuns!==1)throw Error('lost retry');changedText.data='changedRuns+=100';if(changedRuns!==1)throw Error('already-started repeated')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn partial_native_mutation_error_retains_only_connected_preparation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(316), 1).unwrap();
        eval(scope, "globalThis.partialRuns=[]");
        let (target, fragment, first, second) = {
            let mut dom = state.dom.borrow_mut();
            let target = dom.find_by_id("target").unwrap();
            let fragment = dom.create_fragment();
            dom.set_node_document(fragment, state.document_root())
                .unwrap();
            let first = dom.create_element("script");
            dom.set_text_content(first, "partialRuns.push('first')")
                .unwrap();
            let second = dom.create_element("script");
            dom.set_text_content(second, "partialRuns.push('second')")
                .unwrap();
            dom.append_child(fragment, first).unwrap();
            dom.append_child(fragment, second).unwrap();
            (target, fragment, first, second)
        };
        // The native caller retains the detached suffix it inspects after failure,
        // just as a live author DocumentFragment wrapper retains that subtree.
        let _fragment = oryn_dom::TrackedNodeLease::new(&state.dom, fragment).unwrap();
        let preparation = capture_insertion(scope, &state, target, fragment).unwrap();
        state
            .dom
            .borrow_mut()
            .insert_dom_node(target, first, None)
            .unwrap();
        assert!(
            preparation
                .finish_mutation(scope, Err::<(), _>(DomError::Cycle))
                .is_err()
        );
        {
            let registry = state.post_connection_scripts.borrow();
            let batch = registry.queue.front().unwrap();
            assert_eq!(batch.resources.len(), 1);
            assert_eq!(batch.jobs.len(), 1);
        }
        assert_eq!(state.dom.borrow().parent(second).unwrap(), Some(fragment));
        eval(
            scope,
            "if(partialRuns.length)throw Error('error boundary ran author code')",
        );
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(partialRuns.join()!=='first')throw Error('connected prefix lost or detached suffix executed')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
        let third = {
            let mut dom = state.dom.borrow_mut();
            let third = dom.create_element("script");
            dom.set_text_content(third, "partialRuns.push('third')")
                .unwrap();
            third
        };
        let preparation = capture_insertion(scope, &state, target, third).unwrap();
        state
            .dom
            .borrow_mut()
            .insert_dom_node(target, third, None)
            .unwrap();
        let error = preparation.finish_result(
            scope,
            Err::<(), _>(V8BindingError::Allocation("return wrapper")),
        );
        assert!(matches!(
            error,
            Err(V8BindingError::Allocation("return wrapper"))
        ));
        eval(
            scope,
            "if(partialRuns.join()!=='first')throw Error('wrapper failure ran author code')",
        );
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(partialRuns.join()!=='first,third')throw Error('wrapper failure lost connected preparation')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn reclaimed_staging_journal_preserves_later_custom_element_reactions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(320), 1).unwrap();
        let staging = state.dom.borrow_mut().create_fragment();
        let lease = oryn_dom::TrackedNodeLease::new(&state.dom, staging).unwrap();
        let child = {
            let mut dom = state.dom.borrow_mut();
            dom.set_node_document(staging, state.document_root())
                .unwrap();
            let child = dom.create_element("i");
            dom.append_child(staging, child).unwrap();
            child
        };
        drop(lease);
        assert!(matches!(
            state.dom.borrow().arena().get(staging),
            Err(DomError::StaleNode(_))
        ));
        assert!(matches!(
            state.dom.borrow().arena().get(child),
            Err(DomError::StaleNode(_))
        ));
        let replacement = {
            let mut dom = state.dom.borrow_mut();
            let body = dom
                .query_selector(state.document_root(), "body")
                .unwrap()
                .unwrap();
            let node = dom.create_element("x-journal");
            dom.append_child(body, node).unwrap();
            node
        };
        assert_eq!(replacement.slot, staging.slot);
        assert_ne!(replacement.generation, staging.generation);
        eval(
            scope,
            "globalThis.journalTrace=[];customElements.define('x-journal',class extends HTMLElement{constructor(){super();journalTrace.push('construct')}connectedCallback(){journalTrace.push('connected')}disconnectedCallback(){journalTrace.push('disconnected')}});if(journalTrace.join()!=='construct,connected')throw Error('live replacement reactions');document.body.firstChild.remove();if(journalTrace.join()!=='construct,connected,disconnected')throw Error('later reaction lost or replayed')",
        );
    }
    #[test]
    fn adjacent_html_capture_failure_preserves_all_insertion_positions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(315), 1).unwrap();
        for position in ["beforebegin", "afterbegin", "beforeend", "afterend"] {
            eval(
                scope,
                "globalThis.host=document.createElement('div');globalThis.old=document.createElement('b');host.appendChild(old);document.body.appendChild(host);globalThis.conversions=0",
            );
            FAIL_CAPTURE_AFTER.with(|fail| fail.set(Some(0)));
            eval(
                scope,
                &format!(
                    "globalThis.rejected=false;try{{host.insertAdjacentHTML('{position}',{{toString(){{conversions++;return '<i>new</i><u>tail</u>'}}}})}}catch(error){{rejected=true}};if(!rejected||conversions!==1||document.body.childNodes.length!==1||document.body.firstChild!==host||host.childNodes.length!==1||host.firstChild!==old)throw Error('adjacent capture mutated destination');host.remove()"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
            assert!(state.post_connection_scripts.borrow().queue.is_empty());
        }
    }
    #[test]
    fn markup_capture_failure_preserves_existing_children_and_shadow_root() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(314), 1).unwrap();
        for action in [
            "host.innerHTML=markup",
            "host.outerHTML=markup",
            "host.setHTML(markup)",
            "host.setHTMLUnsafe(markup)",
            "shadow.innerHTML=markup",
            "shadow.setHTML(markup)",
            "shadow.setHTMLUnsafe(markup)",
            "host.appendHTML(markup)",
            "host.prependHTML(markup)",
            "host.beforeHTML(markup)",
            "host.afterHTML(markup)",
            "host.replaceWithHTML(markup)",
            "host.appendHTMLUnsafe(markup)",
            "host.prependHTMLUnsafe(markup)",
            "host.beforeHTMLUnsafe(markup)",
            "host.afterHTMLUnsafe(markup)",
            "host.replaceWithHTMLUnsafe(markup)",
            "shadow.appendHTMLUnsafe(markup)",
            "shadow.prependHTMLUnsafe(markup)",
        ] {
            eval(
                scope,
                "globalThis.host=document.createElement('div');globalThis.old=document.createElement('b');host.appendChild(old);document.body.appendChild(host);globalThis.shadow=host.attachShadow({mode:'open'});globalThis.shadowOld=document.createElement('i');shadow.appendChild(shadowOld);globalThis.markup='<section><b>new</b></section>'",
            );
            FAIL_CAPTURE_AFTER.with(|fail| fail.set(Some(0)));
            eval(
                scope,
                &format!(
                    "globalThis.rejected=false;try{{{action}}}catch(error){{rejected=true}};if(!rejected||host.parentNode!==document.body||host.firstChild!==old||host.childNodes.length!==1||host.shadowRoot!==shadow||shadow.firstChild!==shadowOld||shadow.childNodes.length!==1)throw Error('markup capture mutated destination');host.remove()"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
            assert!(state.post_connection_scripts.borrow().queue.is_empty());
        }
    }
    #[test]
    fn variadic_capture_failure_preserves_staging_and_destination() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(312), 1).unwrap();
        for method in [
            "append",
            "prepend",
            "replaceChildren",
            "before",
            "after",
            "replaceWith",
        ] {
            eval(
                scope,
                "globalThis.host=document.createElement('div');globalThis.old=document.createElement('b');host.appendChild(old);document.body.appendChild(host);globalThis.input=document.createDocumentFragment();globalThis.script=document.createElement('script');script.textContent='throw Error(\"unexpected body\")';input.appendChild(script)",
            );
            FAIL_CAPTURE_AFTER.with(|fail| fail.set(Some(1)));
            let receiver = if matches!(method, "before" | "after" | "replaceWith") {
                "old"
            } else {
                "host"
            };
            eval(
                scope,
                &format!(
                    "globalThis.rejected=false;try{{{receiver}.{method}(input,'tail')}}catch(error){{rejected=true}};if(!rejected||host.firstChild!==old||host.childNodes.length!==1||input.childNodes.length||script.parentNode.nodeType!==11||script.parentNode.childNodes.length!==2)throw Error('capture staging boundary');host.remove()"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
            assert!(state.post_connection_scripts.borrow().queue.is_empty());
        }
    }
    #[test]
    fn adjacent_node_insertion_resolves_after_conversion_and_handles_detached_receiver() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(313), 1).unwrap();
        eval(
            scope,
            r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};const left=document.createElement('div'),right=document.createElement('div'),anchor=document.createElement('i'),extra=document.createElement('b');left.appendChild(anchor);document.body.append(left,right);anchor.insertAdjacentElement({toString(){right.appendChild(anchor);return 'beforebegin'}},extra);check(left.childNodes.length===0&&right.firstChild===extra&&right.lastChild===anchor,'position conversion moved receiver');anchor.insertAdjacentText('afterend',{toString(){left.appendChild(anchor);return 'text'}});check(left.textContent==='text'&&right.childNodes.length===1,'text conversion moved receiver');const detached=document.createElement('i');check(detached.insertAdjacentElement('beforebegin',extra)===null&&extra.parentNode===right,'detached element insertion');let calls=0;detached.insertAdjacentText('afterend',{toString(){calls++;return 'ignored'}});check(calls===1&&!detached.parentNode,'detached text conversion');let rejected=false;try{detached.insertAdjacentText('invalid','x')}catch(error){rejected=error.name==='SyntaxError'}check(rejected,'invalid position still rejects')})()"#,
        );
    }
    #[test]
    fn native_insertion_capture_failure_precedes_destination_mutation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><b>old</b></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(311), 1).unwrap();
        for method in ["appendChild", "insertBefore", "replaceChild"] {
            eval(
                scope,
                "globalThis.captureRuns=0;globalThis.original=document.body.firstChild;globalThis.input=document.createDocumentFragment();globalThis.script=document.createElement('script');script.textContent='captureRuns++';input.appendChild(script)",
            );
            FAIL_CAPTURE_AFTER.with(|fail| fail.set(Some(1)));
            eval(
                scope,
                &format!(
                    "globalThis.rejected=false;try{{document.body.{method}(input,original)}}catch(error){{rejected=true}};if(!rejected||document.body.firstChild!==original||document.body.childNodes.length!==1||input.firstChild!==script||captureRuns)throw Error('capture mutated insertion')"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
            assert!(state.post_connection_scripts.borrow().queue.is_empty());
            eval(
                scope,
                "document.body.appendChild(input);if(captureRuns!==1)throw Error('capture failed retry');script.remove()",
            );
        }
    }
    #[test]
    fn html_insertion_resource_failure_retains_committed_identity_and_script_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(319), 1).unwrap();
        for method in [
            "appendHTMLUnsafe",
            "prependHTMLUnsafe",
            "beforeHTMLUnsafe",
            "afterHTMLUnsafe",
            "replaceWithHTMLUnsafe",
        ] {
            for stage in 0..4 {
                eval(
                    scope,
                    "globalThis.insertTrace=[];globalThis.conversions=0;globalThis.host=document.createElement('div');document.body.appendChild(host)",
                );
                FAIL_RESOURCE_STAGE.with(|fail| fail.set(Some(stage)));
                eval(
                    scope,
                    &format!(
                        "globalThis.rejected=false;try{{host.{method}({{toString(){{conversions++;return '<section id=committed><script>insertTrace.push(1)<'+ '/script><b>complete</b><script>insertTrace.push(2)<'+ '/script></section>'}}}},{{runScripts:true}})}}catch(error){{rejected=true}};globalThis.kept=document.querySelector('#committed');if(!rejected||!kept||kept.textContent.indexOf('complete')<0||insertTrace.length||conversions!==1)throw Error('insertion failure boundary')"
                    ),
                );
                assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
                state
                    .post_connection_scripts
                    .borrow_mut()
                    .queue
                    .front_mut()
                    .unwrap()
                    .retry = std::time::Instant::now();
                poll(scope, &state).unwrap();
                eval(
                    scope,
                    "if(document.querySelector('#committed')!==kept||insertTrace.join()!=='1,2'||conversions!==1)throw Error('retry lost identity/replayed conversion');document.body.replaceChildren()",
                );
                assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
                assert!(state.post_connection_scripts.borrow().queue.is_empty());
            }
        }
    }
    #[test]
    fn legacy_setter_resource_failure_retains_inert_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(308), 1).unwrap();
        for member in ["innerHTML", "outerHTML"] {
            eval(
                scope,
                "globalThis.legacyRuns=0;globalThis.host=document.createElement('div');document.body.appendChild(host)",
            );
            FAIL_RESOURCE_STAGE.with(|fail| fail.set(Some(1)));
            eval(
                scope,
                &format!(
                    "try{{host.{member}='<section><script>legacyRuns++<'+ '/script><b>kept</b></section>'}}catch(error){{}};globalThis.kept=document.querySelector('section');if(!kept||legacyRuns)throw Error('replacement or inertness')"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
            state
                .post_connection_scripts
                .borrow_mut()
                .queue
                .front_mut()
                .unwrap()
                .retry = std::time::Instant::now();
            poll(scope, &state).unwrap();
            eval(
                scope,
                "if(document.querySelector('section')!==kept||legacyRuns)throw Error('retry changed identity or ran inert script');document.body.replaceChildren()",
            );
            assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
        }
    }
    #[test]
    fn oversized_node_insertion_preserves_source_and_destination_across_realms() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><b>source</b></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(303), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        let (destination, document) = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><b>destination</b></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(304), 1).unwrap();
            let document = wrap_node(scope, &state, state.document_root()).unwrap();
            (state, v8::Global::new(scope, document))
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = v8::Local::new(scope, &document);
        let key = v8::String::new(scope, "targetDocument").unwrap();
        first.global(scope).set(scope, key.into(), document.into());
        eval(
            scope,
            r#"(()=>{
   const check=(v,m)=>{if(!v)throw Error(m)};
   const fragment=document.createDocumentFragment();for(let i=0;i<2049;i++)fragment.appendChild(document.createElement('script'));
   const first=fragment.firstChild,local=document.body.firstChild,remote=targetDocument.body.firstChild;
   for(const parent of [document.body,targetDocument.body]){
    let rejected=false;try{parent.appendChild(fragment)}catch(error){rejected=true}
    check(rejected&&fragment.children.length===2049&&fragment.firstChild===first&&first.ownerDocument===document,'source changed on admission failure');
   }
   let rejected=false;try{targetDocument.body.replaceChild(fragment,remote)}catch(error){rejected=true}
   check(rejected&&targetDocument.body.firstChild===remote&&document.body.firstChild===local,'replacement changed destination');
   check(fragment.children.length===2049&&first.ownerDocument===document,'replacement adopted rejected source');
   try{first.appendChild(fragment);throw Error('missing hierarchy error')}catch(error){check(error.name==='HierarchyRequestError','hierarchy validation precedence')}
   check(first.parentNode===fragment,'hierarchy failure mutated fragment');
   targetDocument.body.appendChild(document.createElement('i'));
  })()"#,
        );
        assert_eq!(source.post_connection_scripts.borrow().admitted.get(), 0);
        assert_eq!(
            destination.post_connection_scripts.borrow().admitted.get(),
            0
        );
    }
    #[test]
    fn oversized_html_setter_preserves_old_children_before_admission() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host><b>old</b></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(302), 1).unwrap();
        eval(
            scope,
            r#"globalThis.trace=[];globalThis.host=document.getElementById('host');const old=host.firstChild;const markup=('<script>trace.push("unexpected")<'+ '/script>').repeat(2049);let rejected=false;try{host.setHTMLUnsafe(markup,{runScripts:true})}catch(error){rejected=true}if(!rejected||host.firstChild!==old||host.childNodes.length!==1||trace.length)throw Error('capacity check occurred after replacement');host.setHTMLUnsafe('<script>trace.push("accepted")<'+ '/script>',{runScripts:true});if(trace.join()!=='accepted')throw Error('failed preflight leaked admission');const detached=document.createElement('div');detached.setHTMLUnsafe(markup,{runScripts:true});if(detached.children.length!==2049||trace.length!==1)throw Error('detached fragment charged active preparation budget')"#,
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
        assert!(state.post_connection_scripts.borrow().queue.is_empty());
    }
    #[test]
    fn partial_image_admission_retries_only_unaccepted_sources_before_scripts() {
        struct Host {
            fail: Cell<bool>,
            attempts: RefCell<Vec<String>>,
            accepted: RefCell<Vec<String>>,
        }
        impl PlatformHost for Host {
            fn retire(&self) {}
            fn cancel(&self, _: HostOperationId) {}
            fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
                vec![]
            }
            fn submit(
                &self,
                _: HostOperationId,
                request: PlatformHostRequest,
            ) -> Result<(), String> {
                if let PlatformHostRequest::ImageStart { request, .. } = request {
                    self.attempts.borrow_mut().push(request.url.clone());
                    if request.url.ends_with("/b.png") && self.fail.replace(false) {
                        return Err("injected image admission".into());
                    }
                    self.accepted.borrow_mut().push(request.url);
                }
                Ok(())
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(301), 1).unwrap();
        state.commit_location("https://example.test/page").unwrap();
        let host = Rc::new(Host {
            fail: Cell::new(true),
            attempts: RefCell::new(Vec::new()),
            accepted: RefCell::new(Vec::new()),
        });
        state.set_platform_host(host.clone());
        eval(
            scope,
            "globalThis.trace=[];globalThis.container=document.createElement('div');document.body.append(container)",
        );
        eval(
            scope,
            r#"try{container.setHTMLUnsafe('<section><img src="/a.png"><img src="/b.png"><img src="/c.png"><script>trace.push("script")<'+ '/script></section>',{runScripts:true})}catch(error){};if(trace.length)throw Error('script ran before image admissions')"#,
        );
        assert_eq!(&*host.accepted.borrow(), &["https://example.test/a.png"]);
        let failed = state
            .dom
            .borrow()
            .query_selector_all(state.document_root(), "img")
            .unwrap()[1];
        assert!(!state.image_sources.borrow().contains_key(&failed));
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        assert_eq!(
            &*host.accepted.borrow(),
            &[
                "https://example.test/a.png",
                "https://example.test/b.png",
                "https://example.test/c.png"
            ]
        );
        assert_eq!(
            &*host.attempts.borrow(),
            &[
                "https://example.test/a.png",
                "https://example.test/b.png",
                "https://example.test/b.png",
                "https://example.test/c.png"
            ]
        );
        eval(
            scope,
            "if(trace.join(',')!=='script')throw Error('lost or repeated script')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn resource_stage_failures_keep_roots_and_scripts_until_all_preparation_finishes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(299), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];globalThis.host=document.createElement('div');document.body.append(host)",
        );
        FAIL_RESOURCE_STAGE.with(|fail| fail.set(Some(1)));
        eval(
            scope,
            r#"try{host.setHTMLUnsafe('<section><script>trace.push("done")<'+ '/script><b>complete</b></section>',{runScripts:true})}catch(error){};if(trace.length)throw Error('early script')"#,
        );
        {
            let registry = state.post_connection_scripts.borrow();
            let batch = registry.queue.front().unwrap();
            assert_eq!(batch.resources.len(), 1);
            assert_eq!(batch.resources.front().unwrap().stage, 1);
            assert_eq!(batch.jobs.len(), 1);
            assert!(!batch.checkpointed);
        }
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        FAIL_RESOURCE_STAGE.with(|fail| fail.set(Some(3)));
        assert!(poll(scope, &state).is_err());
        {
            let registry = state.post_connection_scripts.borrow();
            assert_eq!(
                registry
                    .queue
                    .front()
                    .unwrap()
                    .resources
                    .front()
                    .unwrap()
                    .stage,
                3
            );
        }
        eval(
            scope,
            "if(trace.length)throw Error('script before final resource stage')",
        );
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(trace.join(',')!=='done'||host.querySelector('b').textContent!=='complete')throw Error('lost prepared insertion')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn suspended_resource_preparation_has_no_runnable_deadline_and_retirement_releases_it() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(300), 1).unwrap();
        eval(
            scope,
            "globalThis.host=document.createElement('div');document.body.append(host)",
        );
        FAIL_RESOURCE_STAGE.with(|fail| fail.set(Some(0)));
        eval(
            scope,
            r#"try{host.setHTMLUnsafe('<section><script>throw Error("retired body")<'+ '/script></section>',{runScripts:true})}catch(error){}"#,
        );
        state.set_execution_suspended(true);
        assert!(delay(&state).is_none());
        state.set_execution_suspended(false);
        assert!(delay(&state).is_some());
        state.retire_execution();
        assert!(state.post_connection_scripts.borrow().queue.is_empty());
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn html_setter_retry_keeps_original_dom_and_does_not_reconvert_arguments() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(297), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];globalThis.conversions=0;globalThis.options=0;globalThis.host=document.createElement('div');document.body.append(host)",
        );
        FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
        eval(
            scope,
            r#"try{host.setHTMLUnsafe({toString(){conversions++;return '<script>trace.push("a")<'+ '/script><script>trace.push("b")<'+ '/script>'}},{get runScripts(){options++;return true}})}catch(error){};globalThis.first=host.firstChild;if(trace.length||conversions!==1||options!==1||host.children.length!==2)throw Error('setter preparation boundary')"#,
        );
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(trace.join(',')!=='a,b'||host.firstChild!==first||conversions!==1||options!==1)throw Error('replayed setter conversion or replacement')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn html_setter_replacement_cancels_disconnected_retry_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(298), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];globalThis.host=document.createElement('div');document.body.append(host)",
        );
        FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
        eval(
            scope,
            r#"try{host.setHTMLUnsafe('<script>trace.push("obsolete")<'+ '/script>',{runScripts:true})}catch(error){};host.setHTML('<b>replacement</b>')"#,
        );
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(trace.length||host.textContent!=='replacement')throw Error('obsolete setter suffix executed')",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn preparation_failure_retains_entire_suffix_before_any_script_runs() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(294), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];globalThis.f=document.createDocumentFragment();for(const name of ['a','b']){const s=document.createElement('script');s.textContent=`trace.push('${name}')`;f.append(s)}",
        );
        FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
        eval(
            scope,
            "try{document.body.append(f)}catch(e){};if(trace.length)throw Error('body before admission')",
        );
        assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(scope, "if(trace.join()!=='a,b')throw Error('lost suffix')");
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn interrupted_error_listener_keeps_later_script_without_replaying_first_body() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(296), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];addEventListener('error',e=>{trace.push('error-first');e.preventDefault()},{once:true});addEventListener('error',e=>trace.push('error-tail'));globalThis.f=document.createDocumentFragment();let a=document.createElement('script');a.textContent=\"trace.push('body');throw Error('source')\";let b=document.createElement('script');b.textContent=\"trace.push('later')\";f.append(a,b)",
        );
        native_platform::publication::fail_after_next_callback();
        eval(scope, "try{document.body.append(f)}catch(e){}");
        eval(
            scope,
            "if(trace.join()!=='body,error-first')throw Error('unexpected pre-retry order')",
        );
        global_events::retry_reports_now(&state);
        global_events::poll_reports(scope, &state).unwrap();
        state
            .post_connection_scripts
            .borrow_mut()
            .queue
            .front_mut()
            .unwrap()
            .retry = std::time::Instant::now();
        poll(scope, &state).unwrap();
        eval(
            scope,
            "if(trace.join()!=='body,error-first,error-tail,later')throw Error('lost or repeated script suffix')",
        );
    }
    #[test]
    fn nested_insertions_are_synchronous_and_retirement_releases_failed_batches() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(295), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];let a=document.createElement('script');a.textContent=\"trace.push('outer');let b=document.createElement('script');b.textContent=\\\"trace.push('inner')\\\";document.body.append(b);trace.push('after')\";document.body.append(a);if(trace.join()!=='outer,inner,after')throw Error('nested script deferred')",
        );
        FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
        eval(
            scope,
            "try{let c=document.createElement('script');c.textContent=\"trace.push('obsolete')\";document.body.append(c)}catch(e){}",
        );
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 2);
        state.retire_execution();
        assert!(state.post_connection_scripts.borrow().queue.is_empty());
        assert_eq!(state.post_connection_scripts.borrow().admitted.get(), 0);
    }
    #[test]
    fn thrown_range_call_keeps_committed_preparation_but_aborts_unperformed_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        for method in ["deleteContents", "extractContents", "surroundContents"] {
            eval(
                scope,
                "globalThis.runs=0;globalThis.script=document.createElement('script');script.type='text/plain';globalThis.first=document.createTextNode('');globalThis.last=document.createTextNode('globalThis.runs++');script.append(first,last);document.body.append(script);script.type='text/javascript';globalThis.range=document.createRange();range.selectNodeContents(script);globalThis.wrapper=document.createElement('b');wrapper.textContent='untouched';globalThis.threw=false",
            );
            FAIL_POST_CONNECTION_PREPARE.with(|fail| fail.set(true));
            eval(
                scope,
                &format!(
                    "try{{range.{method}(wrapper)}}catch(error){{threw=true}};if(!threw||first.parentNode!==null||last.parentNode!==script||runs!==0||wrapper.textContent!=='untouched')throw Error('Range error boundary')"
                ),
            );
            assert_eq!(state.post_connection_scripts.borrow().queue.len(), 1);
            state
                .post_connection_scripts
                .borrow_mut()
                .queue
                .front_mut()
                .unwrap()
                .retry = std::time::Instant::now();
            poll(scope, &state).unwrap();
            eval(
                scope,
                "if(runs!==1||last.parentNode!==script||first.parentNode!==null||wrapper.parentNode!==null||wrapper.textContent!=='untouched')throw Error('failed Range suffix resumed or committed preparation lost');script.remove()",
            );
        }
    }
}
