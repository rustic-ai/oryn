//! Incremental native HTML sinks owned by the ordinary WritableStream queue.
//! Parser state survives write boundaries; abort never emits EOF or replays input.
use super::*;
use oryn_dom::html::{ParserEvent, ParserInput, StreamingParser};

const MAX_RETAINED_INPUT: usize = 32 * 1024 * 1024;
static INPUT_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// Owned by the native scheduler's immutable preparation, not its fetch result.
// Dropping completed/retired work releases the parser; parking retains it.
pub(super) struct ScriptCompletion(pub(super) Rc<Cell<bool>>);
impl Drop for ScriptCompletion {
    fn drop(&mut self) {
        self.0.set(true);
    }
}
struct WriteContinuation {
    input: ParserInput,
    closing: bool,
    eof: bool,
    checkpointed: bool,
    events: VecDeque<ParserEvent>,
    preparation: Option<post_connection_scripts::PreparedInsertion>,
    blocking: Option<Rc<Cell<bool>>>,
    preparing_completion: Option<Rc<Cell<bool>>>,
}
pub(super) struct Sink {
    owner: RcWeak<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    lifetime: (NodeId, u64, u64, u64),
    parser: RefCell<Option<StreamingParser>>,
    continuation: RefCell<Option<WriteContinuation>>,
    run_scripts: bool,
    target_document: NodeId,
    discard_input: bool,
    cancelled: Cell<bool>,
    bytes: Cell<usize>,
}
impl Sink {
    fn owner(&self) -> Result<Rc<V8BindingState>, V8BindingError> {
        let state = self
            .owner
            .upgrade()
            .ok_or(V8BindingError::Allocation("HTML stream document retired"))?;
        let (dom, node) = self.target.current();
        if self.cancelled.get()
            || state.execution_retired()
            || state.execution_tasks_stopped()
            || (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ) != self.lifetime
            || !Rc::ptr_eq(&dom, &state.dom)
            || dom.borrow().node_document(node)? != Some(self.target_document)
        {
            return Err(V8BindingError::Allocation(
                "HTML stream target authority changed",
            ));
        }
        Ok(state)
    }
    fn runnable(&self) -> bool {
        self.owner
            .upgrade()
            .is_none_or(|state| state.execution_retired() || state.execution_tasks_stopped())
            || self
                .continuation
                .borrow()
                .as_ref()
                .is_none_or(|work| work.blocking.as_ref().is_none_or(|done| done.get()))
    }
    fn reserve(&self, bytes: usize) -> Result<(), V8BindingError> {
        use std::sync::atomic::Ordering;
        let mut used = INPUT_BYTES.load(Ordering::Acquire);
        loop {
            let next = used
                .checked_add(bytes)
                .filter(|next| *next <= MAX_RETAINED_INPUT)
                .ok_or(V8BindingError::DocumentWriteLimit {
                    resource: "HTML stream parser input",
                })?;
            match INPUT_BYTES.compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    self.bytes.set(self.bytes.get() + bytes);
                    return Ok(());
                }
                Err(actual) => used = actual,
            }
        }
    }
    fn release(&self) {
        INPUT_BYTES.fetch_sub(self.bytes.replace(0), std::sync::atomic::Ordering::AcqRel);
    }
    fn reconcile(&self, retained: usize) -> Result<(), V8BindingError> {
        let previous = self.bytes.get();
        if retained > previous {
            self.reserve(retained - previous)?;
        } else {
            self.bytes.set(retained);
            INPUT_BYTES.fetch_sub(previous - retained, std::sync::atomic::Ordering::AcqRel);
        }
        Ok(())
    }
    pub(super) fn abort(&self) {
        self.cancelled.set(true);
        self.parser.borrow_mut().take();
        self.continuation.borrow_mut().take();
        self.release();
    }
    fn events(
        &self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        work: &mut WriteContinuation,
    ) -> Result<bool, V8BindingError> {
        if work.blocking.as_ref().is_some_and(|done| !done.get()) {
            return Ok(false);
        }
        work.blocking = None;
        while let Some(event) = work.events.front() {
            self.owner()?;
            if state.execution_suspended() {
                return Ok(false);
            }
            let mut blocking_node = None;
            match event {
                ParserEvent::ScriptReady { node, scheduling } => {
                    if !self.run_scripts {
                        state.mark_script_started(*node);
                    } else {
                        if work.preparation.is_none() {
                            if *scheduling == oryn_dom::html::ScriptScheduling::Blocking {
                                work.preparing_completion = Some(Rc::new(Cell::new(false)));
                            }
                            work.preparation = Some(post_connection_scripts::capture_stream_event(
                                scope,
                                state,
                                *node,
                                work.preparing_completion.clone(),
                            )?);
                        }
                        if !work.preparation.as_mut().unwrap().poll_retained(scope)? {
                            return Ok(false);
                        }
                        if *scheduling == oryn_dom::html::ScriptScheduling::Blocking {
                            blocking_node = Some(*node);
                        }
                    }
                }
                ParserEvent::ExternalTemplateDiscovered { node } => {
                    prepare_native_module_preloads(scope, state, *node)?
                }
                ParserEvent::EventHandlerAttributeChanged { node, name } => {
                    prepare_event_handler_attribute(scope, state, *node, name)?
                }
                ParserEvent::FrameDiscovered { node, .. }
                | ParserEvent::FrameLocalDocument { node }
                | ParserEvent::StylesheetDiscovered { node, .. }
                | ParserEvent::ModulePreloadDiscovered { node }
                | ParserEvent::InlineStyleReady { node } => {
                    if work.preparation.is_none() {
                        work.preparation = Some(post_connection_scripts::capture_stream_event(
                            scope, state, *node, None,
                        )?);
                    }
                    if !work.preparation.as_mut().unwrap().poll_retained(scope)? {
                        return Ok(false);
                    }
                }
                ParserEvent::EncodingIndicator { .. }
                | ParserEvent::EncodingCommitted
                | ParserEvent::MetaSecurity { .. }
                | ParserEvent::MetaReferrer { .. } => {}
            }
            work.preparation = None;
            if blocking_node.is_some() {
                work.blocking = work.preparing_completion.take().filter(|done| !done.get());
            }
            work.events.pop_front();
            if work.blocking.is_some() {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn accept<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        chunk: Option<v8::Local<'s, v8::Value>>,
    ) -> Result<bool, V8BindingError> {
        let result = self.accept_inner(scope, chunk);
        if result.is_err() {
            self.abort();
        }
        result
    }
    fn accept_inner<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        chunk: Option<v8::Local<'s, v8::Value>>,
    ) -> Result<bool, V8BindingError> {
        let state = self.owner()?;
        // A queue request is converted once. Its suffix and publication frontier
        // remain native while its promise waits for blocking script completion.
        if self.continuation.borrow().is_none() {
            let input = if let Some(chunk) = chunk {
                // Parser options, not individual chunks, carry Trusted Types
                // authority. Conversion still runs once and may reenter/throw.
                let html = chunk
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let markup = rust_string_preserving_utf16(scope, html)?;
                let input = if self.discard_input {
                    ParserInput::unicode("")
                } else {
                    ParserInput::dom_string(&markup)
                };
                self.reserve(input.len())?;
                input
            } else {
                ParserInput::unicode("")
            };
            self.owner()?;
            *self.continuation.borrow_mut() = Some(WriteContinuation {
                input,
                closing: chunk.is_none(),
                eof: false,
                checkpointed: false,
                events: VecDeque::new(),
                preparation: None,
                blocking: None,
                preparing_completion: None,
            });
        }
        let mut work = self.continuation.borrow_mut().take().unwrap();
        let mut parser = self
            .parser
            .borrow_mut()
            .take()
            .ok_or(V8BindingError::Allocation(
                "HTML stream parser is closed or busy",
            ))?;
        let result = (|| {
            loop {
                self.owner()?;
                if state.execution_suspended() {
                    return Ok(false);
                }
                if !self.events(scope, &state, &mut work)? || state.execution_suspended() {
                    return Ok(false);
                }
                if work.closing {
                    if work.eof {
                        break;
                    }
                    let (complete, error) =
                        document_parser_gate::with_parser_construction(scope, &state, || {
                            parser.prepare_finish()
                        });
                    work.events.extend(parser.take_events());
                    work.eof = complete;
                    if let Some(error) = error {
                        return Err(error);
                    }
                    if !complete && work.events.is_empty() && !parser.has_pending_mutations() {
                        return Err(V8BindingError::Allocation(
                            "HTML stream EOF publication interrupted",
                        ));
                    }
                } else {
                    if work.input.is_empty()
                        && !parser.has_pending_mutations()
                        && !parser.has_pending_events()
                    {
                        break;
                    }
                    let (progress, error) =
                        document_parser_gate::with_parser_construction(scope, &state, || {
                            parser.push_input_until_resource_or_script(&work.input)
                        });
                    work.input = work.input.split_off(progress.consumed_bytes);
                    work.events.extend(parser.take_events());
                    if let Some(event) = progress.blocked_on {
                        work.events.push_back(event);
                    }
                    if let Some(error) = error {
                        return Err(error);
                    }
                    if progress.consumed_bytes == 0
                        && work.events.is_empty()
                        && !parser.has_pending_mutations()
                    {
                        return Err(V8BindingError::Allocation(
                            "HTML stream parser publication interrupted",
                        ));
                    }
                }
            }
            if !work.checkpointed {
                work.checkpointed = true;
                checkpoint_dom_lifecycle(scope);
            }
            self.owner()?;
            Ok(!state.execution_suspended())
        })();
        if matches!(result, Ok(false)) {
            self.reconcile(parser.retained_input_bytes() + work.input.len())?;
            *self.parser.borrow_mut() = Some(parser);
            *self.continuation.borrow_mut() = Some(work);
        } else if matches!(result, Ok(true)) && !work.closing {
            self.reconcile(parser.retained_input_bytes())?;
            *self.parser.borrow_mut() = Some(parser);
        } else {
            self.cancelled.set(true);
            self.release();
        }
        result
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) fn pending(state: &V8BindingState) -> bool {
    !state.execution_suspended()&&state.platform_objects.borrow().values().any(|record|matches!(record,
        PlatformObjectRecord::WritableStream(stream)if matches!(&stream.kind,WritableKind::Html(sink)if sink.runnable())&&!stream.busy&&!stream.closed&&stream.error.is_none()&&!stream.queue.is_empty()))
}
pub(super) fn resume(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.execution_suspended() {
        return Ok(());
    }
    let ids=state.platform_objects.borrow().iter().filter_map(|(id,record)|matches!(record,
        PlatformObjectRecord::WritableStream(stream)if matches!(&stream.kind,WritableKind::Html(sink)if sink.runnable())&&!stream.busy&&!stream.closed&&stream.error.is_none()&&!stream.queue.is_empty()).then_some(*id)).take(64).collect::<Vec<_>>();
    for id in ids {
        writable_streams::pump(scope, state, id)?;
    }
    Ok(())
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<'s, v8::Value>,
    member: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let receiver = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    let original_document = state.dom.borrow().node_document(node)?;
    let lifetime = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    );
    let safe = !member.ends_with("Unsafe");
    let operation = member.trim_end_matches("Unsafe").trim_end_matches("HTML");
    let interface = if matches!(
        state.dom.borrow().arena().get(node)?.kind,
        NodeKind::ShadowRoot { .. }
    ) {
        "ShadowRoot"
    } else {
        "Element"
    };
    let options = trusted_parser_options::acquire(
        scope,
        state,
        value,
        safe,
        &format!("{interface} {member}"),
    )?;
    let (dom, node) = receiver.current();
    if !Rc::ptr_eq(&dom, &state.dom)
        || state.execution_retired()
        || dom.borrow().node_document(node)? != original_document
        || lifetime
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            )
    {
        return Err(V8BindingError::Allocation(
            "HTML stream target moved during options",
        ));
    }
    let sibling = matches!(
        operation,
        "streamBefore" | "streamAfter" | "streamReplaceWith"
    );
    let parent = if sibling {
        dom.borrow()
            .parent(node)?
            .ok_or_else(hierarchy_request_error)?
    } else {
        node
    };
    let (parent_kind, context_parent, template_host) = {
        let native = dom.borrow();
        let record = native.arena().get(parent)?;
        (
            record.kind.clone(),
            native.parent(parent)?,
            record.template_host,
        )
    };
    let context_node = match parent_kind {
        NodeKind::Element { .. } => Some(parent),
        NodeKind::ShadowRoot { .. } => context_parent,
        NodeKind::DocumentFragment
            if template_host.is_some() || template_links::host(scope, state, parent)?.is_some() =>
        {
            None
        }
        _ => return Err(hierarchy_request_error()),
    };
    let context = if let Some(context) = context_node {
        oryn_dom::sanitizer::FragmentContext::capture(&dom.borrow(), context)?
    } else {
        let mut context = oryn_dom::sanitizer::FragmentContext::html_body();
        context.disable_context_shadow = true;
        context
    };
    let discard_input = safe && context.safe_sink_is_noop();
    let (target_owner, target_node) = if !sibling
        && dom
            .borrow()
            .is_element(parent, "http://www.w3.org/1999/xhtml", "template")
    {
        if let Some(content) = template_links::content(scope, state, parent)? {
            node_identity::argument(scope, content)?.ok_or_else(hierarchy_request_error)?
        } else {
            (state.clone(), dom.borrow_mut().template_content(parent)?)
        }
    } else {
        (state.clone(), parent)
    };
    let target = oryn_dom::TrackedNodeLease::new(&target_owner.dom, target_node)?;
    let target_document = target_owner
        .dom
        .borrow()
        .node_document(target_node)?
        .ok_or(DomError::StaleNode(target_node))?;
    let children = light_children(&target_owner.dom.borrow(), target_node)?;
    let before = match operation {
        "stream" | "streamAppend" => None,
        "streamPrepend" => children.first().copied(),
        "streamBefore" => Some(node),
        "streamAfter" | "streamReplaceWith" => children
            .iter()
            .position(|child| *child == node)
            .and_then(|index| children.get(index + 1).copied()),
        _ => return Err(hierarchy_request_error()),
    };
    let configuration = options.stream_configuration(safe)?;
    let mut parser = StreamingParser::with_fragment(
        target_owner.dom.clone(),
        target_node,
        context,
        Some((configuration, safe)),
        options.run_scripts,
    )?;
    parser.set_fragment_insertion_before(before)?;
    // Build/validate the parser before destructive changes. Native lifecycle
    // callbacks run only after the stream and its queue have acquired custody.
    if !discard_input {
        if operation == "stream" {
            let mut native = target_owner.dom.borrow_mut();
            let empty = native.create(NodeKind::DocumentFragment);
            native.set_node_document(empty, target_document)?;
            native.replace_dom_children(target_node, empty)?;
        } else if operation == "streamReplaceWith" {
            target_owner
                .dom
                .borrow_mut()
                .remove_child(target_node, node)?;
        }
    }
    let sink = Rc::new(Sink {
        owner: Rc::downgrade(&target_owner),
        target,
        lifetime: (
            target_owner.document_root(),
            target_owner.document_generation(),
            target_owner.document_stream_epoch(),
            target_owner.document_authority_epoch(),
        ),
        target_document,
        discard_input,
        parser: RefCell::new(Some(parser)),
        continuation: RefCell::new(None),
        run_scripts: options.run_scripts,
        cancelled: Cell::new(false),
        bytes: Cell::new(0),
    });
    let stream = reserve_platform_object_id(state);
    let controller = reserve_platform_object_id(state);
    insert_platform_object(
        state,
        controller,
        PlatformObjectRecord::WritableController { stream_id: stream },
    );
    let undefined: v8::Local<v8::Value> = v8::undefined(scope).into();
    insert_platform_object(
        state,
        stream,
        PlatformObjectRecord::WritableStream(WritableStreamRecord {
            creation_context: weak_current_context(scope),
            locked: false,
            closed: false,
            kind: WritableKind::Html(sink),
            error: None,
            queue: VecDeque::new(),
            pending_abort: None,
            abort_in_flight: false,
            abort_completion: None,
            writer_id: None,
            high_water_mark: 1.0,
            busy: false,
            closing: false,
            started: true,
            controller_id: controller,
            source: stream_graph::Owned::new(scope, undefined),
        }),
    );
    new_platform_object(scope, state, "WritableStream", stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn run(source: &str) {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code = v8::String::new(scope, source).unwrap();
        let promise = v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .try_cast::<v8::Promise>()
            .unwrap();
        for _ in 0..256 {
            scope.perform_microtask_checkpoint();
            if promise.state() != v8::PromiseState::Pending {
                break;
            }
        }
        assert_eq!(
            promise.state(),
            v8::PromiseState::Fulfilled,
            "{}",
            promise.result(scope).to_rust_string_lossy(scope)
        );
        assert!(promise.result(scope).is_true());
    }
    #[test]
    fn split_html_stream_preserves_open_nodes_entities_and_dom_string_units() {
        run(r#"(async()=>{
      const root=document.createElement('div'),stream=root.streamAppendHTMLUnsafe(),writer=stream.getWriter();
      if(!(stream instanceof WritableStream)||writer.desiredSize!==1)throw Error('native stream');
      await writer.write('<p id="one">A &am');const first=root.firstChild;
      if(!first||first.id!=='one')throw Error('incremental publication');
      await writer.write('p;\uD800');await writer.write('B</p><span>tail');await writer.close();
      if(root.firstChild!==first||first.textContent!=='A &\uD800B'||root.lastChild.textContent!=='tail')throw Error('parser continuity');
      return true;
    })()"#);
    }
    #[test]
    fn native_stream_template_patch_and_sanitizer_share_live_target_scope() {
        run(r#"(async()=>{
      for(const shadow of [false,true])for(const filter of ['none','template','for']){
        const host=document.createElement('div');document.body.append(host);
        const root=shadow?host.attachShadow({mode:'open'}):host;
        root.setHTMLUnsafe('<div id="p"><?start name="slot">old<?end></div>');
        const options=filter==='template'?{sanitizer:{removeElements:['template']}}:filter==='for'?{sanitizer:{removeAttributes:['for']}}:{};
        const writer=root.streamAppendHTMLUnsafe(options).getWriter();
        await writer.write('<template id="patch" for="slot"><b>');await writer.write('new');await writer.write('</template>');await writer.close();
        if(root.querySelector('#p').textContent!==(filter==='none'?'new':'old'))throw Error('patch filtering');
        if((root.querySelector('#patch')!==null)!==(filter==='for'))throw Error('template retention');
      }return true;
    })()"#);
    }
    #[test]
    fn stream_abort_and_chunk_conversion_failure_release_parser_without_replay() {
        run(r#"(async()=>{
      const root=document.createElement('div'),writer=root.streamAppendHTMLUnsafe().getWriter();
      await writer.write('<p>committed');await writer.abort('stop');let failed=false;
      try{await writer.write('late')}catch(e){failed=true}
      if(!failed||root.textContent!=='committed')throw Error('aborted stream changed DOM');
      const second=root.streamAppendHTMLUnsafe().getWriter();let calls=0,reason=new Error('coercion');
      try{await second.write({toString(){++calls;throw reason}});throw Error('accepted')}catch(e){if(e!==reason)throw e}
      try{await second.write('late')}catch(e){if(e!==reason)throw e}
      if(calls!==1||root.textContent!=='committed')throw Error('conversion replay');return true;
    })()"#);
    }
    #[test]
    fn default_stream_scripts_remain_inert_across_multiple_writes() {
        run(r#"(async()=>{
      const root=document.createElement('div');document.body.append(root);globalThis.streamExecutions=0;
      const writer=root.streamAppendHTMLUnsafe().getWriter();await writer.write('<scr');await writer.write('ipt>streamExecutions++');await writer.write('</script>');await writer.close();
      if(streamExecutions!==0||root.querySelectorAll('script').length!==1)throw Error('default stream script executed');return true;
    })()"#);
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    fn suspend_native(
        scope: &mut v8::PinScope,
        _: v8::FunctionCallbackArguments,
        _: v8::ReturnValue,
    ) {
        scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .unwrap()
            .set_execution_suspended(true);
    }
    #[test]
    fn suspension_inside_conversion_or_inline_script_parks_without_replay() {
        for conversion in [false, true] {
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
            let callback = v8::Function::new(scope, suspend_native).unwrap();
            let name = v8::String::new(scope, "suspendNative").unwrap();
            context
                .global(scope)
                .set(scope, name.into(), callback.into());
            let source = if conversion {
                r#"globalThis.calls=0;globalThis.done=false;globalThis.writer=document.body.streamAppendHTMLUnsafe({runScripts:true}).getWriter();writer.write({toString(){calls++;suspendNative();return '<b id="tail"></b>';}}).then(()=>done=true);"#
            } else {
                r#"globalThis.calls=0;globalThis.done=false;globalThis.writer=document.body.streamAppendHTMLUnsafe({runScripts:true}).getWriter();writer.write('<script>calls++;suspendNative()</script><b id="tail"></b>').then(()=>done=true);"#
            };
            eval(scope, source);
            assert!(state.execution_suspended());
            let sink = state
                .platform_objects
                .borrow()
                .values()
                .find_map(|record| match record {
                    PlatformObjectRecord::WritableStream(stream) => match &stream.kind {
                        WritableKind::Html(sink) => Some(sink.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .unwrap();
            assert!(!sink.cancelled.get());
            assert!(sink.continuation.borrow().is_some());
            assert!(sink.parser.borrow().is_some());
            eval(
                scope,
                "if(done||calls!==1||document.getElementById('tail'))throw Error('inflight suspension boundary')",
            );
            state.set_execution_suspended(false);
            resume(scope, &state).unwrap();
            scope.perform_microtask_checkpoint();
            eval(
                scope,
                "if(!done||calls!==1||!document.getElementById('tail'))throw Error('inflight suspension replay')",
            );
        }
    }
    #[test]
    fn blocking_external_script_keeps_write_suffix_and_conversion_until_terminal_custody() {
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
            r#"globalThis.conversions=0;globalThis.done=false;globalThis.trace=[];globalThis.writer=document.body.streamAppendHTMLUnsafe({runScripts:true}).getWriter();writer.write({toString(){conversions++;return '<script src="https://example.test/slow.js"></script><span id="tail"></span><script>trace.push("inline")</script>';}}).then(()=>done=true);"#,
        );
        scope.perform_microtask_checkpoint();
        let mut jobs = state.take_pending_dynamic_scripts();
        assert_eq!(jobs.len(), 1);
        let accepted = jobs.pop().unwrap();
        assert!(accepted.stream_completion.is_some());
        for _ in 0..3 {
            resume(scope, &state).unwrap();
            scope.perform_microtask_checkpoint();
        }
        eval(
            scope,
            "if(done||conversions!==1||document.getElementById('tail')||trace.length)throw Error('blocking boundary crossed')",
        );
        state.set_execution_suspended(true);
        drop(accepted);
        resume(scope, &state).unwrap();
        eval(
            scope,
            "if(done||document.getElementById('tail'))throw Error('parked write resumed')",
        );
        state.set_execution_suspended(false);
        resume(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        eval(
            scope,
            "if(!done||conversions!==1||!document.getElementById('tail')||trace.join()!=='inline')throw Error('terminal continuation lost')",
        );
        resume(scope, &state).unwrap();
        eval(scope, "if(trace.length!==1)throw Error('script replay')");
    }
    #[test]
    fn abort_during_blocking_script_releases_input_and_never_publishes_suffix() {
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
            r#"globalThis.writer=document.body.streamAppendHTMLUnsafe({runScripts:true}).getWriter();writer.write('<script src="https://example.test/slow.js"></script><b id="tail"></b>').catch(()=>{});"#,
        );
        let jobs = state.take_pending_dynamic_scripts();
        assert_eq!(jobs.len(), 1);
        let sink = state
            .platform_objects
            .borrow()
            .values()
            .find_map(|record| match record {
                PlatformObjectRecord::WritableStream(stream) => match &stream.kind {
                    WritableKind::Html(sink) => Some(sink.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert!(sink.bytes.get() > 0);
        eval(scope, "writer.abort('cancel').catch(()=>{})");
        scope.perform_microtask_checkpoint();
        assert!(sink.cancelled.get());
        assert_eq!(sink.bytes.get(), 0);
        drop(jobs);
        resume(scope, &state).unwrap();
        eval(
            scope,
            "if(document.getElementById('tail'))throw Error('aborted suffix published')",
        );
    }
    #[test]
    fn async_external_script_does_not_hold_html_stream_suffix() {
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
            r#"globalThis.done=false;document.body.streamAppendHTMLUnsafe({runScripts:true}).getWriter().write('<script async src="https://example.test/slow.js"></script><b id="tail"></b>').then(()=>done=true)"#,
        );
        scope.perform_microtask_checkpoint();
        let jobs = state.take_pending_dynamic_scripts();
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].stream_completion.is_none());
        eval(
            scope,
            "if(!done||!document.getElementById('tail'))throw Error('async script blocked stream')",
        );
    }
    #[test]
    fn suspended_writes_resume_and_failed_conversion_releases_retained_parser() {
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
        let source=v8::String::new(scope,"globalThis.target=document.createElement('div');globalThis.writer=target.streamAppendHTMLUnsafe().getWriter();writer.write('<p title=\"incomplete');").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        let sink = state
            .platform_objects
            .borrow()
            .values()
            .find_map(|record| match record {
                PlatformObjectRecord::WritableStream(stream) => match &stream.kind {
                    WritableKind::Html(sink) => Some(sink.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert!(
            sink.bytes.get() > 0,
            "incomplete attribute must remain accounted"
        );
        state.set_execution_suspended(true);
        let source = v8::String::new(
            scope,
            "globalThis.queued=writer.write('\">text</p>');queued",
        )
        .unwrap();
        let queued = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .try_cast::<v8::Promise>()
            .unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(queued.state(), v8::PromiseState::Pending);
        assert!(!pending(&state));
        state.set_execution_suspended(false);
        assert!(pending(&state));
        resume(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(queued.state(), v8::PromiseState::Fulfilled);
        assert_eq!(sink.bytes.get(), 0, "completed token custody released");
        let source = v8::String::new(scope, "writer.write('<span title=\"still-open');").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        assert!(sink.bytes.get() > 0);
        let source=v8::String::new(scope,"globalThis.failed=writer.write({toString(){throw Error('conversion')}});failed.catch(()=>{});").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        assert!(sink.cancelled.get());
        assert_eq!(sink.bytes.get(), 0);
        assert!(
            sink.parser.borrow().is_none(),
            "retained writer must not retain failed parser"
        );
    }
    #[test]
    fn completed_small_chunks_do_not_consume_a_cumulative_stream_quota() {
        super::tests::run(
            r#"(async()=>{
          const root=document.createElement('div'),writer=root.streamAppendHTMLUnsafe().getWriter(),chunk='x'.repeat(1024*1024);
          for(let i=0;i<40;i++){await writer.write(chunk);if(root.textContent.length!==chunk.length)throw Error('chunk');root.textContent='';}
          await writer.close();return true;
        })()"#,
        );
    }
}

#[cfg(test)]
mod family_tests {
    #[test]
    fn complete_streaming_position_matrix_keeps_open_nodes_and_existing_identity() {
        super::tests::run(
            r#"(async()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const unsafe of [false,true])for(const position of ['', 'Append','Prepend','Before','After','ReplaceWith']){
                const parent=document.createElement('div');parent.innerHTML='<i>L</i><b>A</b><i>R</i>';document.body.append(parent);
                const [left,anchor,right]=parent.children;
                const sibling=['Before','After','ReplaceWith'].includes(position),target=sibling?anchor:parent;
                const method='stream'+position+'HTML'+(unsafe?'Unsafe':'');
                const stream=target[method]();check(stream instanceof WritableStream,method+' result');
                if(position==='')check(parent.textContent==='',method+' synchronous replacement');
                if(position==='ReplaceWith')check(anchor.parentNode===null,method+' synchronous removal');
                const writer=stream.getWriter();await writer.write('<span>X');
                const opened=parent.querySelector('span');check(opened&&opened.textContent==='X',method+' incremental');
                await writer.write('Y</span><u>Z</u>');await writer.close();
                check(parent.querySelector('span')===opened,method+' parser identity');
                check(parent.textContent===({'':'XYZ',Append:'LARXYZ',Prepend:'XYZLAR',Before:'LXYZAR',After:'LAXYZR',ReplaceWith:'LXYZR'})[position],method+' position');
                if(position!=='')check(left.parentNode===parent&&right.parentNode===parent,method+' retained siblings');
                parent.remove();
            } return true;
        })()"#,
        );
    }
    #[test]
    fn stream_shadow_template_and_template_child_contexts() {
        super::tests::run(
            r#"(async()=>{
            for(const unsafe of [false,true])for(const position of ['', 'Append','Prepend']){
                for(const template of [false,true]){
                    const host=document.createElement(template?'template':'div');document.body.append(host);
                    const target=template?host:host.attachShadow({mode:'open'});target.innerHTML='<i>A</i>';
                    const root=template?host.content:target,old=root.firstChild;
                    const writer=target['stream'+position+'HTML'+(unsafe?'Unsafe':'')]().getWriter();
                    await writer.write('<b>B');await writer.write('C</b>');await writer.close();
                    if(root.textContent!==({'':'BC',Append:'ABC',Prepend:'BCA'})[position])throw Error('context order');
                    if(position!==''&&old.parentNode!==root)throw Error('context identity');host.remove();
                }
            }
            for(const position of ['Before','After','ReplaceWith']){
                const host=document.createElement('template');host.innerHTML='<i>L</i><b>A</b><i>R</i>';document.body.append(host);
                const target=host.content.children[1],writer=target['stream'+position+'HTMLUnsafe']().getWriter();
                await writer.write('<span>X</span>');await writer.close();
                if(host.content.textContent!==({Before:'LXAR',After:'LAXR',ReplaceWith:'LXR'})[position])throw Error('template sibling');host.remove();
            }return true;
        })()"#,
        );
    }
    #[test]
    fn positioned_stream_removed_anchor_falls_back_without_moving_existing_nodes() {
        super::tests::run(
            r#"(async()=>{
            const host=document.createElement('div');host.innerHTML='<i>A</i><b>B</b>';document.body.append(host);
            const anchor=host.firstChild,writer=host.streamPrependHTMLUnsafe().getWriter();
            await writer.write('<span>X</span>');anchor.remove();
            await writer.write('<em>Y</em>');await writer.close();
            if(host.textContent!=='XBY')throw Error('removed anchor fallback');
            const other=document.createElement('div');const next=host.streamPrependHTMLUnsafe().getWriter();
            other.append(host.firstChild);await next.write('<u>Z</u>');await next.close();
            if(host.textContent!=='BYZ'||other.textContent!=='X')throw Error('foreign anchor');return true;
        })()"#,
        );
    }
    #[test]
    fn safe_streams_filter_before_scripts_handlers_and_customized_constructors() {
        super::tests::run(
            r#"(async()=>{
            globalThis.streamRuns=0;customElements.define('stream-built',class extends HTMLDivElement{constructor(){super();streamRuns++}},{extends:'div'});
            for(const position of ['', 'Append','Prepend','Before','After','ReplaceWith']){
                const host=document.createElement('div'),target=document.createElement('section');host.append(target);document.body.append(host);
                const writer=target['stream'+position+'HTML']({runScripts:true,sanitizer:{removeAttributes:['is']}}).getWriter();
                await writer.write('<div is=stream-built onclick="streamRuns++"><script>streamRuns++</scr');
                await writer.write('ipt><b>ok</b></div>');await writer.close();
                if(streamRuns!==0||host.querySelector('script')||host.querySelector('[onclick]')||host.querySelector('[is]'))throw Error('safe filtering');host.remove();
            }
            const script=document.createElement('script');script.textContent='untouched';const writer=script.streamHTML({runScripts:true}).getWriter();
            await writer.write('replaced');await writer.close();if(script.textContent!=='untouched')throw Error('safe script context');return true;
        })()"#,
        );
    }
    #[test]
    fn unsafe_stream_family_script_order_and_abort_preserve_published_prefix() {
        super::tests::run(
            r#"(async()=>{
            for(const position of ['', 'Append','Prepend','Before','After','ReplaceWith']){
                globalThis.streamOrder=[];const host=document.createElement('div'),target=document.createElement('section');host.append(target);document.body.append(host);
                const writer=target['stream'+position+'HTMLUnsafe']({runScripts:true}).getWriter();
                await writer.write('<b>X</b><script>streamOrder.push(1)</scr');
                if(streamOrder.length)throw Error('early script');await writer.write('ipt><script>streamOrder.push(2)</script><u>Y</u>');
                if(streamOrder.join(',')!=='1,2')throw Error('script order');await writer.abort();
                if(!host.querySelector('b')||!host.querySelector('u'))throw Error('abort erased prefix');host.remove();
            }return true;
        })()"#,
        );
    }
    #[test]
    fn stream_options_reentrancy_and_invalid_parent_fail_before_mutation() {
        super::tests::run(
            r#"(async()=>{
            const host=document.createElement('div');host.textContent='old';document.body.append(host);
            const foreign=document.implementation.createHTMLDocument('');let threw=false;
            try{host.streamHTMLUnsafe({get sanitizer(){foreign.adoptNode(host);return {}}})}catch(e){threw=true}
            if(!threw||host.textContent!=='old')throw Error('reentrant adoption');
            const fragment=document.createDocumentFragment(),child=document.createElement('i');fragment.append(child);
            for(const method of ['streamBeforeHTML','streamAfterHTMLUnsafe','streamReplaceWithHTML']){
                threw=false;try{child[method]()}catch(e){threw=e.name==='HierarchyRequestError'}if(!threw)throw Error('invalid fragment parent');
            }
            const doc=document.implementation.createHTMLDocument(''),comment=doc.appendChild(doc.createComment('x'));
            threw=false;try{comment.streamBeforeHTMLUnsafe()}catch(e){threw=e.name==='HierarchyRequestError'}if(!threw)throw Error('document parent');return true;
        })()"#,
        );
    }
    #[test]
    fn positioned_stream_uses_table_and_foreign_parent_context() {
        super::tests::run(
            r#"(async()=>{
            const table=document.createElement('table');document.body.append(table);const writer=table.streamPrependHTMLUnsafe().getWriter();
            await writer.write('<tr><td>A');await writer.write('</td></tr>');await writer.close();
            if(table.firstChild.tagName!=='TBODY'||table.rows.length!==1)throw Error('table context');
            const svg=document.createElementNS('http://www.w3.org/2000/svg','svg'),rect=document.createElementNS(svg.namespaceURI,'rect');svg.append(rect);document.body.append(svg);
            const next=rect.streamBeforeHTMLUnsafe().getWriter();await next.write('<circle/>');await next.close();
            if(svg.firstChild.localName!=='circle'||svg.firstChild.namespaceURI!==svg.namespaceURI)throw Error('foreign parent context');return true;
        })()"#,
        );
    }
    #[test]
    fn native_table_rows_are_live_ordered_and_exclude_nested_tables() {
        super::tests::run(
            r#"(async()=>{
            const table=document.createElement('table');document.body.append(table);
            const section=tag=>{const n=document.createElement(tag);table.append(n);return n};
            const row=(parent,id)=>{const r=document.createElement('tr');r.id=id;parent.append(r);return r};
            const foot=section('tfoot'),f=row(foot,'f');
            const body=section('tbody'),b=row(body,'b');
            const direct=row(table,'d'),head=section('thead'),h=row(head,'h');
            const nested=document.createElement('table'),cell=document.createElement('td');
            b.append(cell);cell.append(nested);row(nested,'nested');
            const rows=table.rows;
            const ids=()=>Array.from(rows,r=>r.id).join('|');
            if(!(table instanceof HTMLTableElement)||!(rows instanceof HTMLCollection)||rows!==table.rows)throw Error('rows brand/identity');
            if(ids()!=='h|b|d|f'||rows.namedItem('nested')!==null)throw Error('rows order/nested');
            head.append(b);if(ids()!=='h|b|d|f')throw Error('move to head');
            body.append(h);if(ids()!=='b|h|d|f')throw Error('live section move');
            table.remove();if(rows!==table.rows||ids()!=='b|h|d|f')throw Error('detached table');
            foot.remove();if(ids()!=='b|h|d')throw Error('section detach');
            h.remove();if(ids()!=='b|d')throw Error('row detach');
            const foreign=document.implementation.createHTMLDocument('');
            foreign.body.append(foreign.adoptNode(table));
            if(table.rows!==rows||ids()!=='b|d'||rows[0]!==b)throw Error('adopted collection');
            row(table,'new');if(ids()!=='b|d|new')throw Error('adopted live update');
            return true;
        })()"#,
        );
    }
}
