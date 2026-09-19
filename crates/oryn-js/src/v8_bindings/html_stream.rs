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
    sink_name: &'static str,
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
            || dom.borrow().node_document(node)? != Some(state.document_root())
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
                let html = sanitizer::html_argument(scope, chunk, false)?;
                let markup =
                    trusted_string(scope, &state, html, TrustedKind::Html, self.sink_name)?;
                let input = ParserInput::dom_string(&markup);
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
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let target = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    let lifetime = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    );
    let options = sanitizer::options(scope, value, false, true)?;
    let (dom, node) = target.current();
    if !Rc::ptr_eq(&dom, &state.dom)
        || state.execution_retired()
        || dom.borrow().node_document(node)? != Some(state.document_root())
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
    let (context_node, sink_name) = {
        let dom = dom.borrow();
        match &dom.arena().get(node)?.kind {
            NodeKind::ShadowRoot { .. } => (
                dom.parent(node)?.ok_or(DomError::StaleNode(node))?,
                "ShadowRoot streamAppendHTMLUnsafe",
            ),
            _ => (node, "Element streamAppendHTMLUnsafe"),
        }
    };
    let context = oryn_dom::sanitizer::FragmentContext::capture(&dom.borrow(), context_node)?;
    let parser = StreamingParser::with_fragment(
        dom.clone(),
        node,
        context,
        Some((options.configuration(false)?, false)),
        options.run_scripts,
    )?;
    let sink = Rc::new(Sink {
        owner: Rc::downgrade(state),
        target,
        lifetime,
        parser: RefCell::new(Some(parser)),
        continuation: RefCell::new(None),
        run_scripts: options.run_scripts,
        sink_name,
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
