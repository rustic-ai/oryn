//! Shared tokenizer ownership at document.write reentry boundaries. The gate
//! never fetches resources; typed parser events remain executable-owned.
use super::*;
use oryn_dom::html;
static WRITTEN_INPUT_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_WRITTEN_INPUT_BYTES: usize = 32 * 1024 * 1024;
pub struct DocumentParserGate {
    parser: RefCell<Option<html::StreamingParser>>,
    feeding: Cell<bool>,
    author_effect: Cell<bool>,
    aborted: Cell<bool>,
    input: RefCell<VecDeque<html::ParserInput>>,
    resources: RefCell<Vec<html::ParserEvent>>,
    paused: Cell<bool>,
    closed: Cell<bool>,
    transferred: Cell<bool>,
    owned: bool,
    document: NodeId,
    epoch: u64,
    bytes: Cell<usize>,
}
impl DocumentParserGate {
    pub fn new(
        parser: html::StreamingParser,
        document: NodeId,
        epoch: u64,
        owned: bool,
    ) -> Rc<Self> {
        Rc::new(Self {
            parser: RefCell::new(Some(parser)),
            feeding: Cell::new(false),
            author_effect: Cell::new(false),
            aborted: Cell::new(false),
            input: RefCell::new(VecDeque::new()),
            resources: RefCell::new(Vec::new()),
            paused: Cell::new(false),
            closed: Cell::new(false),
            transferred: Cell::new(!owned),
            owned,
            document,
            epoch,
            bytes: Cell::new(0),
        })
    }
    pub fn author_effect(&self) -> bool {
        self.author_effect.get()
    }
    pub fn commit_author_effect(&self) {
        self.author_effect.set(true);
    }
    pub fn document(&self) -> NodeId {
        self.document
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn closed(&self) -> bool {
        self.closed.get()
    }
    pub fn close(&self) {
        if self.owned {
            self.closed.set(true);
        }
    }
    pub fn can_finish_locally(&self) -> bool {
        self.owned
            && !self.transferred.get()
            && !self.paused.get()
            && !self.pending_events()
            && !self.has_input()
            && self.live()
    }
    pub fn abort(&self) {
        self.aborted.set(true);
        self.parser.borrow_mut().take();
        self.input.borrow_mut().clear();
        self.resources.borrow_mut().clear();
        self.closed.set(true);
        self.release_bytes();
    }
    pub fn owned(&self) -> bool {
        self.owned
    }
    pub fn unclaimed(&self) -> bool {
        self.owned && !self.transferred.get() && self.live()
    }
    pub fn claim_for_execution(&self) {
        self.transferred.set(true);
    }
    pub fn pause(&self) {
        self.paused.set(true);
    }
    pub fn resume(&self) {
        self.paused.set(false);
    }
    pub fn pending_events(&self) -> bool {
        !self.resources.borrow().is_empty()
            || self
                .parser
                .borrow()
                .as_ref()
                .is_some_and(|parser| parser.has_pending_events())
    }
    pub fn has_input(&self) -> bool {
        !self.input.borrow().is_empty()
    }
    pub fn take_input(&self) -> VecDeque<html::ParserInput> {
        std::mem::take(&mut *self.input.borrow_mut())
    }
    pub fn push_until_resource_or_script(&self, source: &str) -> html::ParseProgress {
        self.feed(|parser| parser.push_until_resource_or_script(source))
    }
    pub fn push_input_until_resource_or_script(
        &self,
        source: &html::ParserInput,
    ) -> html::ParseProgress {
        self.feed(|parser| parser.push_input_until_resource_or_script(source))
    }
    fn feed(
        &self,
        feed: impl FnOnce(&mut html::StreamingParser) -> html::ParseProgress,
    ) -> html::ParseProgress {
        // Author constructors can inspect lifecycle state or retire this gate.
        // Never keep a RefCell borrow of the parser across token callbacks.
        let parser = self.parser.borrow_mut().take().expect("live parser gate");
        self.feeding.set(true);
        struct Slice<'a> {
            gate: &'a DocumentParserGate,
            parser: Option<html::StreamingParser>,
        }
        impl Drop for Slice<'_> {
            fn drop(&mut self) {
                self.gate.feeding.set(false);
                if !self.gate.aborted.get() {
                    *self.gate.parser.borrow_mut() = self.parser.take();
                }
            }
        }
        let mut slice = Slice {
            gate: self,
            parser: Some(parser),
        };
        feed(slice.parser.as_mut().unwrap())
    }
    pub fn take_events(&self) -> Vec<html::ParserEvent> {
        let mut events = std::mem::take(&mut *self.resources.borrow_mut());
        events.extend(self.take_tokenizer_events());
        events
    }
    fn take_tokenizer_events(&self) -> Vec<html::ParserEvent> {
        self.parser
            .borrow_mut()
            .as_mut()
            .map(|parser| parser.take_events())
            .unwrap_or_default()
    }
    fn retain_resources(&self, events: Vec<html::ParserEvent>) -> Result<(), V8BindingError> {
        let mut retained = self.resources.borrow_mut();
        if retained.len().saturating_add(events.len()) > 1024 {
            return Err(V8BindingError::DocumentWriteLimit {
                resource: "parser resource records",
            });
        }
        retained.extend(events);
        Ok(())
    }
    pub fn prepare_finish(&self) -> bool {
        let parser = self
            .parser
            .borrow_mut()
            .take()
            .expect("live parser completion");
        self.feeding.set(true);
        struct Completion<'a> {
            gate: &'a DocumentParserGate,
            parser: Option<html::StreamingParser>,
        }
        impl Drop for Completion<'_> {
            fn drop(&mut self) {
                self.gate.feeding.set(false);
                if !self.gate.aborted.get() {
                    *self.gate.parser.borrow_mut() = self.parser.take();
                }
            }
        }
        let mut completion = Completion {
            gate: self,
            parser: Some(parser),
        };
        completion.parser.as_mut().unwrap().prepare_finish()
    }
    pub fn finish(&self) -> html::ParsedDocument {
        let parser = self
            .parser
            .borrow_mut()
            .take()
            .expect("one parser completion");
        self.release_bytes();
        parser.finish()
    }
    fn release_bytes(&self) {
        let bytes = self.bytes.replace(0);
        WRITTEN_INPUT_BYTES.fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
    }
    fn reserve_bytes(&self, length: usize) -> Result<(), V8BindingError> {
        let local = self
            .bytes
            .get()
            .checked_add(length)
            .filter(|bytes| *bytes <= MAX_WRITTEN_INPUT_BYTES)
            .ok_or(V8BindingError::DocumentWriteLimit {
                resource: "parser input bytes",
            })?;
        let mut used = WRITTEN_INPUT_BYTES.load(std::sync::atomic::Ordering::Acquire);
        loop {
            let next = used
                .checked_add(length)
                .filter(|bytes| *bytes <= MAX_WRITTEN_INPUT_BYTES)
                .ok_or(V8BindingError::DocumentWriteLimit {
                    resource: "aggregate written parser input",
                })?;
            match WRITTEN_INPUT_BYTES.compare_exchange_weak(
                used,
                next,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.bytes.set(local);
                    return Ok(());
                }
                Err(actual) => used = actual,
            }
        }
    }
    pub fn live(&self) -> bool {
        !self.aborted.get() && (self.feeding.get() || self.parser.borrow().is_some())
    }
    fn append(&self, source: html::ParserInput) -> Result<(), V8BindingError> {
        if self.input.borrow().len() >= 1024 {
            return Err(V8BindingError::DocumentWriteLimit {
                resource: "parser input records",
            });
        }
        self.reserve_bytes(source.len())?;
        self.input.borrow_mut().push_back(source);
        Ok(())
    }
}

impl Drop for DocumentParserGate {
    fn drop(&mut self) {
        self.release_bytes();
    }
}

/// Runs a tokenizer slice with the document's entered constructor realm.
/// The closure and its V8 scope are never retained by the native parser.
fn parser_owner(
    scope: &v8::PinScope,
    origin: &Rc<V8BindingState>,
    target: &html::ParserNode,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let document = target
        .owner
        .borrow()
        .node_document(target.node)?
        .unwrap_or(target.node);
    if Rc::ptr_eq(&origin.dom, &target.owner)
        && origin.document_root() == document
        && !origin.execution_retired()
        && !origin.execution_suspended()
    {
        return Ok(origin.clone());
    }
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .filter(|state| Rc::ptr_eq(&state.dom, &target.owner))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let exact = states
        .iter()
        .filter(|state| state.document_root() == document)
        .collect::<Vec<_>>();
    // An inert document may borrow its arena's realm, but an existing retired
    // or suspended document cannot borrow a different live document's authority.
    let owner = if exact.is_empty() {
        states
            .iter()
            .find(|state| !state.execution_retired() && !state.execution_suspended())
    } else {
        exact
            .into_iter()
            .find(|state| !state.execution_retired() && !state.execution_suspended())
    };
    owner
        .cloned()
        .ok_or(V8BindingError::Allocation("parser node binding owner"))
}

pub fn with_parser_construction<R>(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parse: impl FnOnce() -> R,
) -> (R, Option<V8BindingError>) {
    struct Depth(Rc<V8BindingState>, NodeId);
    impl Drop for Depth {
        fn drop(&mut self) {
            let mut depths = self.0.parser_custom_construction_depth.borrow_mut();
            if let Some(depth) = depths.get_mut(&self.1) {
                *depth = depth.saturating_sub(1);
                if *depth == 0 {
                    depths.remove(&self.1);
                }
            }
        }
    }
    let mut error = None;
    let mut boundary: Option<custom_elements::ReactionBoundary> = None;
    let mut depth_guards = Vec::new();
    let mut spans: Vec<(Rc<V8BindingState>, NodeId, u64)> = Vec::new();
    let mut result_roots = Vec::new();
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let mut action = |action: html::ParserAction| -> Option<html::ParserNode> {
        if let html::ParserAction::Failed(failure) = action {
            if error.is_none() {
                error = Some(failure.into());
            }
            return None;
        }
        if error.is_some()
            || state.execution_retired()
            || state.execution_suspended()
            || (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            ) != identity
            || scope.is_execution_terminating()
        {
            html::interrupt_element_construction(&state.dom);
            return None;
        }
        let result = (|| -> Result<Option<html::ParserNode>, V8BindingError> {
            match action {
                html::ParserAction::Failed(_) => unreachable!(),
                html::ParserAction::Checkpoint => {
                    node_migration::resume_publications(scope)?;
                    Ok(Some(html::ParserNode {
                        owner: state.dom.clone(),
                        node: state.document_root(),
                    }))
                }
                html::ParserAction::Adopt { target, parent } => {
                    let source = parser_owner(scope, state, &target)?;
                    let destination = parser_owner(scope, state, &parent)?;
                    let document = parent
                        .owner
                        .borrow()
                        .node_document(parent.node)?
                        .unwrap_or(parent.node);
                    let node =
                        node_migration::adopt(scope, &destination, document, &source, target.node)?;
                    Ok(Some(html::ParserNode {
                        owner: destination.dom.clone(),
                        node,
                    }))
                }
                html::ParserAction::Construct { target, parent: _ } => {
                    // Formatting reconstruction can construct several elements in
                    // one tokenizer call. Complete the prior element's queue before
                    // pushing another; overwriting the guard would pop the new queue.
                    if let Some(previous) = boundary.take() {
                        for (owner, document, start) in spans.drain(..) {
                            let end = owner.dom.borrow().mutation_cursor().0;
                            custom_elements::retain_parser_reaction_span(
                                &owner, start, end, document,
                            );
                        }
                        let completion = previous.complete_result(scope);
                        depth_guards.clear();
                        completion?;
                        if state.execution_retired()
                            || state.execution_suspended()
                            || scope.is_execution_terminating()
                            || (
                                state.document_root(),
                                state.document_generation(),
                                state.document_stream_epoch(),
                            ) != identity
                        {
                            html::interrupt_element_construction(&state.dom);
                            return Ok(None);
                        }
                    }
                    let owner = parser_owner(scope, state, &target)?;
                    let node = target.node;
                    if !custom_elements::has_definition(scope, &owner, node)? {
                        owner
                            .dom
                            .borrow_mut()
                            .arena_mut()
                            .get_mut(node)?
                            .custom_element_shadow_disabled = Some(false);
                        return Ok(Some(target));
                    }
                    // Retain identity before authored construction; customized
                    // built-ins can adopt themselves or any open parser ancestor.
                    let original = oryn_dom::TrackedNodeLease::new(&owner.dom, node)?;
                    let owner_identity = (
                        owner.document_root(),
                        owner.document_generation(),
                        owner.document_stream_epoch(),
                    );
                    state.commit_parser_author_effect();
                    owner.commit_parser_author_effect();
                    let document = owner
                        .dom
                        .borrow()
                        .node_document(node)?
                        .unwrap_or(owner.document_root());
                    *owner
                        .parser_custom_construction_depth
                        .borrow_mut()
                        .entry(document)
                        .or_default() += 1;
                    depth_guards.push(Depth(owner.clone(), document));
                    let start = owner.dom.borrow().mutation_cursor().0;
                    spans.push((owner.clone(), document, start));
                    if v8::StackTrace::current_stack_trace(scope, 1)
                        .is_some_and(|trace| trace.get_frame_count() == 0)
                    {
                        queue_mutation_observer_microtask(scope);
                        scope.perform_microtask_checkpoint();
                    }
                    if state.execution_retired()
                        || state.execution_suspended()
                        || owner.execution_retired()
                        || owner.execution_suspended()
                        || scope.is_execution_terminating()
                        || (
                            state.document_root(),
                            state.document_generation(),
                            state.document_stream_epoch(),
                        ) != identity
                        || (
                            owner.document_root(),
                            owner.document_generation(),
                            owner.document_stream_epoch(),
                        ) != owner_identity
                    {
                        // No constructor was entered. Preserve Initialize.custom so
                        // a suspended parser can attempt construction on resumption.
                        html::interrupt_element_construction(&state.dom);
                        return Ok(None);
                    }
                    boundary = Some(custom_elements::ReactionBoundary::begin_parser_token(scope));
                    let (created, failure, entered, fallback_root) =
                        custom_elements::create_synchronously_parser_outcome(scope, &owner, node);
                    if let Some(root) = fallback_root {
                        result_roots.push(root);
                    }
                    if let Some(failure) = failure {
                        if !entered {
                            return Err(failure);
                        }
                        error = Some(failure);
                    }
                    let (current, current_node) = if created == node {
                        original.current()
                    } else {
                        (owner.dom.clone(), created)
                    };
                    html::interrupt_element_construction(&state.dom);
                    Ok(Some(html::ParserNode {
                        owner: current,
                        node: current_node,
                    }))
                }
            }
        })();
        match result {
            Ok(result) => result,
            Err(failure) => {
                if error.is_none() {
                    error = Some(failure);
                }
                html::interrupt_element_construction(&state.dom);
                None
            }
        }
    };
    let result = html::with_parser_actions(&state.dom, &mut action, parse);
    drop(action);
    if let Some(boundary) = boundary {
        for (owner, document, start) in spans {
            let end = owner.dom.borrow().mutation_cursor().0;
            custom_elements::retain_parser_reaction_span(&owner, start, end, document);
        }
        if let Err(failure) = boundary.complete_result(scope) {
            if error.is_none() {
                error = Some(failure);
            }
        }
    }
    drop(depth_guards);
    if error.is_none() && scope.is_execution_terminating() {
        error = Some(V8BindingError::JavaScriptException);
    }
    (result, error)
}

pub(super) fn write(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    gate: &Rc<DocumentParserGate>,
    source: String,
) -> Result<(), V8BindingError> {
    write_input(scope, state, gate, html::ParserInput::dom_string(&source))
}
fn write_input(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    gate: &Rc<DocumentParserGate>,
    source: html::ParserInput,
) -> Result<(), V8BindingError> {
    if gate.paused.get() || !gate.live() {
        gate.append(source)?;
        return Ok(());
    }
    if state.document_write_depth.get() >= 32 {
        return Err(V8BindingError::DocumentWriteLimit {
            resource: "nested script depth",
        });
    }
    gate.reserve_bytes(source.len())?;
    let mut pending = VecDeque::from([source]);
    let mut recovering = Vec::new();
    state
        .document_write_depth
        .set(state.document_write_depth.get() + 1);
    let result = (|| {
        while !gate.paused.get()
            && gate.live()
            && state.document_root() == gate.document
            && state.document_stream_epoch() == gate.epoch
            && !state.execution_retired()
        {
            let Some(mut source) = pending.pop_front() else {
                break;
            };
            let (progress, construction_error) = with_parser_construction(scope, state, || {
                gate.push_input_until_resource_or_script(&source)
            });
            let suffix = source.split_off(progress.consumed_bytes);
            if !suffix.is_empty() {
                pending.push_front(suffix);
            }
            let mut events = gate.take_tokenizer_events();
            if let Some(event) = progress.blocked_on {
                events.push(event);
            }
            recovering = events.clone();
            if let Some(error) = construction_error {
                return Err(error);
            }
            let mut deferred = Vec::new();
            let mut remaining = events.into_iter();
            while let Some(event) = remaining.next() {
                recovering = deferred
                    .iter()
                    .cloned()
                    .chain(std::iter::once(event.clone()))
                    .chain(remaining.clone())
                    .collect();
                match event {
                    html::ParserEvent::EventHandlerAttributeChanged { node, name } => {
                        prepare_event_handler_attribute(scope, state, node, &name)?;
                    }
                    event @ (html::ParserEvent::FrameDiscovered { .. }
                    | html::ParserEvent::FrameLocalDocument { .. }) => {
                        let node = match &event {
                            html::ParserEvent::FrameDiscovered { node, .. }
                            | html::ParserEvent::FrameLocalDocument { node } => *node,
                            _ => unreachable!(),
                        };
                        state.schedule_dynamic_frame(node);
                        checkpoint_dom_lifecycle(scope);
                        // This parser event owns navigation. The lifecycle hook
                        // creates only the initial native context synchronously;
                        // don't also leave a duplicate dynamic navigation job.
                        if matches!(event, html::ParserEvent::FrameDiscovered { .. }) {
                            state.acknowledge_parser_frame_navigation(node);
                        }
                        if scope.is_execution_terminating() {
                            return Err(V8BindingError::JavaScriptException);
                        }
                        deferred.push(event);
                    }
                    event @ html::ParserEvent::ScriptReady { node, .. } => {
                        let kind = {
                            let dom = state.dom.borrow();
                            html::script_block_type(
                                dom.attribute_in_null_namespace(node, "type")?,
                                dom.attribute_in_null_namespace(node, "language")?,
                            )
                        };
                        // A data block has no preparation or execution step. Its
                        // tokenizer boundary must not suspend a synchronous
                        // document.write before later markup is inserted. Leave
                        // already-started unset so a later type change can prepare it.
                        if html::is_javascript_mime_type(&kind)
                            || matches!(kind.as_str(), "module" | "importmap" | "speculationrules")
                        {
                            deferred.push(event);
                        }
                    }
                    event => deferred.push(event),
                }
                recovering = deferred.iter().cloned().chain(remaining.clone()).collect();
            }
            let events = deferred;
            recovering = events.clone();
            // Inline classics without an earlier resource boundary may run
            // synchronously. All other events return to the page scheduler.
            if gate.resources.borrow().is_empty()
                && events.len() == 1
                && matches!(events[0], html::ParserEvent::ScriptReady { .. })
            {
                let html::ParserEvent::ScriptReady { node, .. } = events[0] else {
                    unreachable!()
                };
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if !gate.live()
                    || state.document_root() != gate.document
                    || state.document_stream_epoch() != gate.epoch
                    || state.execution_retired()
                {
                    break;
                }
                if scope.is_execution_terminating() {
                    return Err(V8BindingError::JavaScriptException);
                }
                let inline = {
                    let dom = state.dom.borrow();
                    dom.is_connected(node).unwrap_or(false)
                        && dom.node_document(node).ok().flatten() == Some(gate.document)
                        && dom.script_external_source(node)?.is_none()
                        && html::is_javascript_mime_type(&html::script_block_type(
                            dom.attribute_in_null_namespace(node, "type")?,
                            dom.attribute_in_null_namespace(node, "language")?,
                        ))
                };
                if inline {
                    let execution = (|| {
                        if let Some(source) = prepare_inline_script_or_queue(scope, state, node)? {
                            let nonce = state
                                .dom
                                .borrow()
                                .element_nonce_for_csp(node)?
                                .map(str::to_owned);
                            let allowed = state.authorize_inline_script(
                                &source.policy_text(),
                                nonce.as_deref(),
                                true,
                            )?;
                            if allowed {
                                prepare_document_event_handlers(scope, state)?;
                                execute_inline_classic_script(scope, state, node, &source)?;
                            } else {
                                state.dom.borrow_mut().mark_script_started(node)?;
                            }
                        }
                        Ok::<(), V8BindingError>(())
                    })();
                    // Compilation/preparation can retry, but an entered body
                    // or report must never replay after a callback failure.
                    if state
                        .dom
                        .borrow()
                        .script_already_started(node)
                        .unwrap_or(true)
                    {
                        recovering.clear();
                    }
                    execution?;
                    recovering.clear();
                    continue;
                }
            }
            if events
                .iter()
                .any(|event| matches!(event, html::ParserEvent::ScriptReady { .. }))
            {
                gate.parser
                    .borrow_mut()
                    .as_mut()
                    .expect("live parser gate")
                    .prepend_events(events);
                recovering.clear();
                gate.pause();
                break;
            }
            // Discovery itself does not block synchronous tokenization. Keep
            // native resource/policy events in order; any subsequent script
            // yields until the executable has consumed those earlier events.
            gate.retain_resources(events)?;
            recovering.clear();
        }
        if gate.paused.get() {
            let mut input = gate.input.borrow_mut();
            if input.len().saturating_add(pending.len()) > 1024 {
                return Err(V8BindingError::DocumentWriteLimit {
                    resource: "parser input records",
                });
            }
            input.append(&mut pending);
        }
        Ok(())
    })();
    if result.is_err()
        && gate.live()
        && state.document_root() == gate.document
        && state.document_stream_epoch() == gate.epoch
        && !state.execution_retired()
    {
        // The host owns recovery after an interrupted synchronous write. Keep
        // already-tokenized events ahead of its unconsumed source suffix.
        if !recovering.is_empty() {
            gate.parser
                .borrow_mut()
                .as_mut()
                .expect("live parser gate")
                .prepend_events(recovering);
        }
        // A nested write may already have paused on its own external script.
        // Its unread markup precedes this outer write's suffix.
        gate.input.borrow_mut().append(&mut pending);
        gate.pause();
    }
    state
        .document_write_depth
        .set(state.document_write_depth.get().saturating_sub(1));
    result
}

impl V8BindingState {
    pub fn set_script_report_retirement(&self, cancel: Rc<dyn Fn()>) {
        *self.script_report_retirement.borrow_mut() = Some(cancel);
    }

    pub fn set_meta_csp_policy(
        &self,
        policy: Rc<dyn Fn(&V8BindingState, &str) -> Result<(), String>>,
    ) {
        let previous = self.meta_csp_policy.borrow_mut().take();
        *self.meta_csp_policy.borrow_mut() = Some(match previous {
            Some(previous) => Rc::new(move |state, source| {
                previous(state, source)?;
                policy(state, source)
            }),
            None => policy,
        });
    }

    pub fn set_local_script_policy(
        &self,
        capture: Rc<dyn Fn() -> Result<std::sync::Arc<ScriptPolicySnapshot>, String>>,
        inline: Rc<dyn Fn(&V8BindingState, &str, Option<&str>, bool, bool) -> Result<bool, String>>,
    ) {
        *self.local_script_policy_capture.borrow_mut() = Some(capture);
        *self.local_inline_script_policy.borrow_mut() = Some(inline);
    }
    pub fn capture_local_script_policy(
        &self,
    ) -> Result<Option<std::sync::Arc<ScriptPolicySnapshot>>, V8BindingError> {
        self.local_script_policy_capture
            .borrow()
            .clone()
            .map(|capture| {
                capture().map_err(|_| V8BindingError::DocumentWriteLimit {
                    resource: "standalone script policy snapshot",
                })
            })
            .transpose()
    }
    pub fn set_script_policy_capture(
        &self,
        capture: Rc<dyn Fn(crate::worker_wire::Creator) -> Result<String, String>>,
    ) {
        *self.script_policy_capture.borrow_mut() = Some(capture);
    }
    pub fn capture_script_policy(&self) -> Result<Option<String>, V8BindingError> {
        let capture = self.script_policy_capture.borrow().clone();
        capture
            .map(|capture| {
                capture(self.native_worker_creator()?).map_err(|_| {
                    V8BindingError::DocumentWriteLimit {
                        resource: "native script policy snapshot",
                    }
                })
            })
            .transpose()
    }

    pub fn set_parser_inline_policy(
        &self,
        policy: Rc<
            dyn Fn(
                crate::worker_wire::Creator,
                &str,
                Option<&str>,
                Option<&str>,
                bool,
                bool,
            ) -> Result<bool, String>,
        >,
    ) {
        *self.parser_inline_policy.borrow_mut() = Some(policy);
    }
    pub fn authorize_inline_script(
        &self,
        source: &str,
        nonce: Option<&str>,
        parser_inserted: bool,
    ) -> Result<bool, V8BindingError> {
        self.authorize_inline_script_kind(source, nonce, parser_inserted, false)
    }
    pub fn authorize_inline_script_kind(
        &self,
        source: &str,
        nonce: Option<&str>,
        parser_inserted: bool,
        speculation_rules: bool,
    ) -> Result<bool, V8BindingError> {
        let policy = self.parser_inline_policy.borrow().clone();
        let Some(policy) = policy else {
            return match self.local_inline_script_policy.borrow().clone() {
                Some(policy) => policy(self, source, nonce, parser_inserted, speculation_rules)
                    .map_err(|_| V8BindingError::DocumentWriteLimit {
                        resource: "standalone inline script policy",
                    }),
                None => Ok(true),
            };
        };
        let authority = self.capture_script_policy()?;
        policy(
            self.native_worker_creator()?,
            source,
            nonce,
            authority.as_deref(),
            parser_inserted,
            speculation_rules,
        )
        .map_err(|_| V8BindingError::DocumentWriteLimit {
            resource: "native inline script policy",
        })
    }
    pub fn install_document_parser_gate(&self, gate: Rc<DocumentParserGate>) {
        *self.document_parser_gate.borrow_mut() = Some(gate);
    }
    pub fn take_unclaimed_document_parser_gate(&self) -> Option<Rc<DocumentParserGate>> {
        let gate = self.document_parser_gate.borrow().clone()?;
        if !gate.unclaimed() { None } else { Some(gate) }
    }
}

/// Detached inert parsers share the same process input admission as live gates.
#[derive(Default)]
pub(super) struct DetachedParserInputBudget(usize);
impl DetachedParserInputBudget {
    pub(super) fn reserve(&mut self, length: usize) -> Result<(), V8BindingError> {
        let next = self
            .0
            .checked_add(length)
            .filter(|bytes| *bytes <= 1024 * 1024)
            .ok_or(V8BindingError::DocumentWriteLimit {
                resource: "detached parser input",
            })?;
        let mut used = WRITTEN_INPUT_BYTES.load(std::sync::atomic::Ordering::Acquire);
        loop {
            let total = used
                .checked_add(length)
                .filter(|bytes| *bytes <= MAX_WRITTEN_INPUT_BYTES)
                .ok_or(V8BindingError::DocumentWriteLimit {
                    resource: "aggregate written parser input",
                })?;
            match WRITTEN_INPUT_BYTES.compare_exchange_weak(
                used,
                total,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.0 = next;
                    return Ok(());
                }
                Err(actual) => used = actual,
            }
        }
    }
}
impl Drop for DetachedParserInputBudget {
    fn drop(&mut self) {
        WRITTEN_INPUT_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::AcqRel);
    }
}

#[cfg(test)]
mod input_admission_tests {
    use super::*;
    #[test]
    fn detached_input_admission_rejects_before_growing_retained_usage() {
        let mut budget = DetachedParserInputBudget::default();
        budget.reserve(1024 * 1024).unwrap();
        assert!(budget.reserve(1).is_err());
        assert_eq!(budget.0, 1024 * 1024);
    }
}

impl V8BindingState {
    /// An authored callback cannot be undone by a later byte-decoder restart.
    pub fn commit_parser_author_effect(&self) {
        if let Some(gate) = self.document_parser_gate.borrow().as_ref() {
            gate.commit_author_effect();
        }
    }
}

#[cfg(test)]
mod synchronous_recovery_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn inert_written_data_blocks_do_not_suspend_later_classics_or_consume_module_boundary() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_document(dom.clone(), state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        write(scope,&state,&gate,r#"<script id=data type=application/json>not javascript</script><script nomodule>throw Error('nomodule ran')</script><script id=classic>globalThis.writtenRuns=(globalThis.writtenRuns||0)+1</script><script id=module type=module>globalThis.moduleRan=true</script><p id=suffix>later</p>"#.into()).unwrap();
        eval(
            scope,
            "if(writtenRuns!==1||!document.getElementById('classic')||typeof moduleRan!=='undefined')throw Error('script classification or synchronous insertion')",
        );
        let data = dom.borrow().find_by_id("data").unwrap();
        assert!(!dom.borrow().script_already_started(data).unwrap());
        let classic = dom.borrow().find_by_id("classic").unwrap();
        assert!(dom.borrow().script_already_started(classic).unwrap());
        assert!(gate.paused.get());
        assert!(gate.has_input());
        assert!(dom.borrow().find_by_id("suffix").is_none());
        let module = dom.borrow().find_by_id("module").unwrap();
        assert!(
            gate.take_events().iter().any(
                |event| matches!(event,html::ParserEvent::ScriptReady{node,..} if *node==module)
            )
        );
    }
    #[test]
    fn inert_written_data_block_does_not_release_a_following_external_classic() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_document(dom.clone(), state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        write(scope,&state,&gate,r#"<script type=application/json>{}</script><script id=external src=held.js></script><script id=following>globalThis.followingRan=true</script>"#.into()).unwrap();
        assert!(gate.paused.get());
        assert!(gate.has_input());
        assert!(dom.borrow().find_by_id("following").is_none());
        let external = dom.borrow().find_by_id("external").unwrap();
        assert!(!dom.borrow().script_already_started(external).unwrap());
        assert!(gate.take_events().iter().any(
            |event| matches!(event,html::ParserEvent::ScriptReady{node,..} if *node==external)
        ));
        eval(
            scope,
            "if(typeof followingRan!=='undefined')throw Error('external parser barrier released')",
        );
    }
    #[test]
    fn compilation_failure_retains_written_script_and_input_for_host() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_document(dom, state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        FAIL_CLASSIC_COMPILATION_EMPTY.with(|failure| failure.set(true));
        assert!(write(scope,&state,&gate,"<script id=written>globalThis.writtenRuns=(globalThis.writtenRuns||0)+1</script><p id=suffix>suffix</p>".into()).is_err());
        assert!(gate.paused.get());
        assert!(gate.has_input());
        assert_eq!(state.document_write_depth.get(), 0);
        let events = gate.take_events();
        let node = events
            .iter()
            .find_map(|event| match event {
                html::ParserEvent::ScriptReady { node, .. } => Some(*node),
                _ => None,
            })
            .unwrap();
        assert!(!state.dom.borrow().script_already_started(node).unwrap());
        let source = crate::script_source::ScriptSource::dom(
            state
                .dom
                .borrow()
                .arena()
                .child_text_dom_string(node, 1024)
                .unwrap(),
        );
        execute_inline_classic_script(scope, &state, node, &source).unwrap();
        let input = gate.take_input();
        gate.resume();
        for source in input {
            write_input(scope, &state, &gate, source).unwrap();
        }
        eval(
            scope,
            "if(writtenRuns!==1||!document.getElementById('suffix'))throw Error('written source lost or replayed')",
        );
    }
    #[test]
    fn nested_paused_write_precedes_outer_suffix_after_report_failure() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_document(dom, state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        eval(scope, "addEventListener('error',e=>e.preventDefault())");
        native_platform::publication::fail_after_next_callback();
        assert!(write(scope,&state,&gate,r#"<script>document.write('<script src="held.js"><'+ '/script><p id=inner>inner</p>');throw Error('outer')</script><p id=outer>outer</p>"#.into()).is_err());
        let input = gate
            .take_input()
            .into_iter()
            .map(|input| input.as_transport().to_owned())
            .collect::<Vec<_>>()
            .join("");
        assert!(input.find("id=inner").unwrap() < input.find("id=outer").unwrap());
        assert!(
            gate.take_events()
                .iter()
                .any(|event| matches!(event, html::ParserEvent::ScriptReady { .. })),
            "nested external script still belongs to host"
        );
    }

    #[test]
    fn reported_error_keeps_written_suffix_without_requeuing_entered_body() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_document(dom, state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        eval(
            scope,
            "globalThis.trace=[];addEventListener('error',e=>{trace.push('first');e.preventDefault()},{once:true});addEventListener('error',()=>trace.push('tail'))",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(
            write(
                scope,
                &state,
                &gate,
                "<script>trace.push('body');throw Error('written')</script><p id=suffix>suffix</p>"
                    .into()
            )
            .is_err()
        );
        assert!(
            !gate
                .take_events()
                .iter()
                .any(|event| matches!(event, html::ParserEvent::ScriptReady { .. }))
        );
        assert!(gate.has_input());
        global_events::retry_reports_now(&state);
        global_events::poll_reports(scope, &state).unwrap();
        let input = gate.take_input();
        gate.resume();
        for source in input {
            write_input(scope, &state, &gate, source).unwrap();
        }
        eval(
            scope,
            "if(trace.join()!=='body,first,tail'||!document.getElementById('suffix'))throw Error('written callback replay or missing suffix')",
        );
    }
}

#[cfg(test)]
mod parser_owner_tests {
    use super::*;
    #[test]
    fn exact_retired_parser_document_never_borrows_another_document_realm() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let document = dom.borrow().document();
        let inert = html::parse("<body>other</body>");
        let other = dom
            .borrow_mut()
            .import_subtree(&inert.dom, inert.document)
            .unwrap();
        let inert_document = dom
            .borrow_mut()
            .import_subtree(&inert.dom, inert.document)
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let contexts = [
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
        ];
        let mut states = Vec::new();
        for (index, context) in contexts.iter().enumerate() {
            let scope = &mut v8::ContextScope::new(scope, *context);
            states.push(
                install_generated_bindings_with_root(
                    scope,
                    *context,
                    dom.clone(),
                    RealmId(index as u32),
                    1,
                    if index == 2 { other } else { document },
                )
                .unwrap(),
            );
        }
        states[0].retire_execution();
        let scope = &mut v8::ContextScope::new(scope, contexts[2]);
        let target = html::ParserNode {
            owner: dom.clone(),
            node: document,
        };
        assert!(Rc::ptr_eq(
            &parser_owner(scope, &states[0], &target).unwrap(),
            &states[1]
        ));
        states[1].retire_execution();
        assert!(parser_owner(scope, &states[0], &target).is_err());
        assert!(parser_owner(scope, &states[2], &target).is_err());
        let inert = html::ParserNode {
            owner: dom.clone(),
            node: inert_document,
        };
        assert!(Rc::ptr_eq(
            &parser_owner(scope, &states[2], &inert).unwrap(),
            &states[2]
        ));
    }
    #[test]
    fn pre_constructor_microtask_suspension_preserves_first_construction() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        fn park(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .unwrap()
                .set_execution_suspended(true);
        }
        let callback = v8::Function::new(scope, park).unwrap();
        let name = v8::String::new(scope, "parkParser").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into())
            .unwrap();
        let code=v8::String::new(scope,"globalThis.calls=0;customElements.define('x-park-token',class extends HTMLElement{constructor(){super();calls++}});Promise.resolve().then(parkParser)").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let mut parser = html::StreamingParser::with_runtime(dom.clone());
        let input = "<body><x-park-token id=token><span id=later></span></x-park-token>";
        let (progress, error) = with_parser_construction(scope, &state, || {
            parser.push_until_resource_or_script(input)
        });
        assert!(error.is_some());
        assert!(state.execution_suspended());
        assert!(parser.has_pending_mutations());
        assert!(dom.borrow().find_by_id("token").is_none());
        assert!(dom.borrow().find_by_id("later").is_none());
        state.set_execution_suspended(false);
        let mut offset = progress.consumed_bytes;
        while offset < input.len() {
            let (progress, error) = with_parser_construction(scope, &state, || {
                parser.push_until_resource_or_script(&input[offset..])
            });
            assert!(error.is_none(), "{error:?}");
            assert!(progress.consumed_bytes > 0);
            offset += progress.consumed_bytes;
            parser.take_events();
        }
        let code=v8::String::new(scope,"calls===1&&document.getElementById('token') instanceof customElements.get('x-park-token')&&document.getElementById('later').parentNode===document.getElementById('token')").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod domstring_stream_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn written_token_payload_is_decoded_before_constructor_and_attribute_reactions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_runtime(dom),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate.clone());
        eval(
            scope,
            r#"globalThis.trace=[];customElements.define('x-\ud800',class extends HTMLElement{static observedAttributes=['data-v'];constructor(){super();trace.push([this.localName,this.attributes.length,this.childNodes.length]);globalThis.original=this}attributeChangedCallback(n,o,v){trace.push(v)}});"#,
        );
        let mut units = "<body><x-".encode_utf16().collect::<Vec<_>>();
        units.push(0xd800);
        units.extend(" data-v='".encode_utf16());
        units.push(0xdfff);
        units.extend("'><b id=inside></b></x-".encode_utf16());
        units.push(0xd800);
        units.extend("><p id=after></p>".encode_utf16());
        write(
            scope,
            &state,
            &gate,
            oryn_dom::dom_string::from_units(&units),
        )
        .unwrap();
        state.finish_custom_element_reactions(scope).unwrap();
        assert!(eval(scope,r#"trace.length===2&&trace[0][0]==='x-\ud800'&&trace[0][1]===0&&trace[0][2]===0&&trace[1]==='\udfff'&&document.getElementById('inside').parentNode===original&&document.getElementById('after').parentNode!==original"#).is_true());
    }
    #[test]
    fn split_written_entities_and_surrogate_pairs_keep_distinct_source_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            html::StreamingParser::with_runtime(dom),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        write(scope, &state, &gate, "<body><p id=p>&#xF2".into()).unwrap();
        let mut units = "000;".encode_utf16().collect::<Vec<_>>();
        units.push(0xd83d);
        write(
            scope,
            &state,
            &gate,
            oryn_dom::dom_string::from_units(&units),
        )
        .unwrap();
        write(
            scope,
            &state,
            &gate,
            oryn_dom::dom_string::from_units(&[0xde00]),
        )
        .unwrap();
        write(scope, &state, &gate, "</p>".into()).unwrap();
        assert!(
            eval(
                scope,
                r#"document.getElementById('p').textContent==='\u{f2000}\u{1f600}'"#
            )
            .is_true()
        );
    }
    #[test]
    fn multiple_constructions_in_one_tokenizer_call_keep_reaction_stack_order() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.trace=[];for(const name of ['x-first','x-second'])customElements.define(name,class extends HTMLElement{static observedAttributes=['id'];constructor(){super();trace.push(name+':construct')}attributeChangedCallback(){trace.push(name+':attribute')}connectedCallback(){trace.push(name+':connected')}})"#,
        );
        let mut parser = html::StreamingParser::with_runtime(dom);
        let (_, error) = with_parser_construction(scope, &state, || {
            parser.push("<body><x-first id=a></x-first><x-second id=b></x-second>")
        });
        assert!(error.is_none(), "{error:?}");
        state.finish_custom_element_reactions(scope).unwrap();
        assert!(eval(scope,"trace.join()===['x-first:construct','x-first:attribute','x-first:connected','x-second:construct','x-second:attribute','x-second:connected'].join()").is_true());
    }
}

#[cfg(test)]
mod declarative_registry_parser_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn live_scoped_host_declarative_shadow_constructs_from_document_registry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.scoped=new CustomElementRegistry();globalThis.GlobalChild=class extends HTMLElement{constructor(){super();calls.push('global')}};globalThis.ScopedChild=class extends HTMLElement{constructor(){super();calls.push('scoped')}};customElements.define('x-registry-child',GlobalChild);scoped.define('x-registry-child',ScopedChild);globalThis.host=document.createElement('div',{customElementRegistry:scoped});host.id='host';document.body.append(host)",
        );
        let host = dom.borrow().find_by_id("host").unwrap();
        let fragment_context =
            oryn_dom::sanitizer::FragmentContext::capture(&dom.borrow(), host).unwrap();
        let mut parser =
            html::StreamingParser::with_fragment(dom, host, fragment_context, None, false).unwrap();
        let mut input = html::ParserInput::unicode(
            "<section><template shadowrootmode=open><x-registry-child></x-registry-child></template><x-registry-child></x-registry-child></section>",
        );
        while !input.is_empty() {
            let (progress, error) = with_parser_construction(scope, &state, || {
                parser.push_input_until_resource_or_script(&input)
            });
            assert!(error.is_none(), "{error:?}");
            input = input.split_off(progress.consumed_bytes);
            parser.take_events();
        }
        let (complete, error) = with_parser_construction(scope, &state, || parser.prepare_finish());
        assert!(complete);
        assert!(error.is_none());
        state.finish_custom_element_reactions(scope).unwrap();
        assert!(eval(scope,"host.firstElementChild.shadowRoot.firstElementChild instanceof GlobalChild&&host.firstElementChild.lastElementChild instanceof ScopedChild&&calls.join()==='global,scoped'").is_true());
    }
}
