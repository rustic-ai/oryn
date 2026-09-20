//! Fetch drains the actual Body stream before granting native HTTP admission.
use super::*;
const LIMIT: usize = 16 * 1024 * 1024;
const PENDING: usize = crate::platform_admission::ACTIVE_LIMIT;
#[derive(Default)]
pub(super) struct Registry {
    pending: BTreeMap<u64, Pending>,
    dependents: Vec<(std::rc::Weak<V8BindingState>, u64)>,
}
struct Pending {
    _request: v8::Global<v8::Object>,
    request: PlatformHttpRequest,
    signal: u64,
    resolver: v8::Global<v8::PromiseResolver>,
    owner: std::rc::Weak<V8BindingState>,
    stream: u64,
    _wrapper: stream_graph::Owned<v8::Object>,
    generation: u64,
    authority: u64,
}
fn aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    signal: u64,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    Ok(match &*platform_object(state, signal)? {
        PlatformObjectRecord::AbortSignal(signal) if signal.aborted => Some(
            signal
                .reason
                .as_ref()
                .map(|reason| reason.value(scope))
                .unwrap_or_else(|| v8::undefined(scope).into()),
        ),
        _ => None,
    })
}
pub(super) fn reject_aborted_request(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    request_id: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (stream, foreign) = match &*platform_object(state, request_id)? {
        PlatformObjectRecord::Request(request) => {
            (request.body_object_id, request.body_owner.clone())
        }
        _ => return Err(V8BindingError::Allocation("aborted fetch Request")),
    };
    if let Some(stream) = stream {
        let owner = body_owner(state, &foreign)?;
        cancel_stream(scope, &owner, stream, reason)?;
    }
    Ok(())
}
pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    request: PlatformHttpRequest,
    signal: u64,
    resolver: v8::Local<v8::PromiseResolver>,
) -> Result<(), V8BindingError> {
    if let Some(reason) = aborted(scope, state, signal)? {
        resolver.reject(scope, reason);
        return Ok(());
    }
    if state.execution_retired() {
        return Err(V8BindingError::Message(
            "fetch execution realm retired".into(),
        ));
    }
    if request.body.len() > LIMIT {
        return Err(V8BindingError::Message(
            "fetch request body exceeds native limit".into(),
        ));
    }
    record_platform_trace(
        scope,
        "request_started",
        serde_json::json!({"method":&request.method,"url":&request.url,"bytes":request.body.len()}),
    );
    let head_request = request.method == "HEAD";
    submit_platform_operation(
        state,
        PlatformHostRequest::HttpStart { request },
        PendingPlatformOperation::FetchStart {
            head_request,
            resolver: v8::Global::new(scope, resolver),
            signal_id: signal,
        },
    )?;
    Ok(())
}
pub(super) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    request_id: u64,
    request: PlatformHttpRequest,
    signal: u64,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("fetch body resolver"))?;
    let promise = resolver.get_promise(scope);
    if state.fetch_bodies.borrow().pending.len() >= PENDING {
        return Err(V8BindingError::Message(
            "pending fetch body capacity exceeded".into(),
        ));
    }
    let (stream, foreign) = match &*platform_object(state, request_id)? {
        PlatformObjectRecord::Request(request) => {
            (request.body_object_id, request.body_owner.clone())
        }
        _ => return Err(V8BindingError::Allocation("fetch Request")),
    };
    let Some(stream) = stream else {
        dispatch(scope, state, request, signal, resolver)?;
        return Ok(promise);
    };
    let owner = body_owner(state, &foreign)?;
    let (disturbed, locked) = response_stream_state(&owner, Some(stream))?;
    let used = matches!(&*platform_object(state,request_id)?,PlatformObjectRecord::Request(request) if request.body_used);
    if used || disturbed || locked {
        let reason = readable_streams::type_error(scope, "fetch request body is unusable")?;
        resolver.reject(scope, reason);
        return Ok(promise);
    }
    let wrapper = new_platform_object(scope, &owner, "ReadableStream", stream)?;
    let request_wrapper = new_platform_object(scope, state, "Request", request_id)?;
    if !Rc::ptr_eq(state, &owner) {
        let mut source = owner.fetch_bodies.borrow_mut();
        source.dependents.retain(|(requester, id)| {
            requester
                .upgrade()
                .is_some_and(|requester| requester.fetch_bodies.borrow().pending.contains_key(id))
        });
        if source.dependents.len() >= PENDING {
            return Err(V8BindingError::Message(
                "foreign fetch body dependency capacity exceeded".into(),
            ));
        }
        source.dependents.push((Rc::downgrade(state), request_id));
    }
    state.fetch_bodies.borrow_mut().pending.insert(
        request_id,
        Pending {
            _request: v8::Global::new(scope, request_wrapper),
            request,
            signal,
            resolver: v8::Global::new(scope, resolver),
            owner: Rc::downgrade(&owner),
            stream,
            _wrapper: stream_graph::Owned::new(scope, wrapper),
            generation: state.document_generation(),
            authority: state.document_authority_epoch(),
        },
    );
    // Install cancellation custody before pull() can invoke author callbacks.
    let consumed =
        response_body_operation_bounded(scope, state, request_id, "arrayBuffer", Some(LIMIT));
    let setup = (|| {
        let value = consumed?;
        let body = v8::Local::<v8::Promise>::try_from(value)
            .map_err(|_| V8BindingError::Allocation("fetch body promise"))?;
        let data = v8::BigInt::new_from_u64(scope, request_id);
        let success = v8::Function::builder(fulfilled)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("fetch body continuation"))?;
        let failure = v8::Function::builder(rejected)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("fetch body rejection"))?;
        body.then2(scope, success, failure)
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Some(reason) = aborted(scope, state, signal)? {
            abort(scope, state, signal, reason)?;
        }
        Ok::<(), V8BindingError>(())
    })();
    if let Err(error) = setup {
        let reason = readable_streams::type_error(scope, &error.to_string())?;
        let pending = { state.fetch_bodies.borrow_mut().pending.remove(&request_id) };
        if let Some(pending) = pending {
            resolver.reject(scope, reason);
            cancel(scope, &pending, reason)?;
        }
    }
    Ok(promise)
}
fn cancel(
    scope: &mut v8::PinScope,
    pending: &Pending,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if let Some(owner) = pending
        .owner
        .upgrade()
        .filter(|owner| !owner.execution_retired())
    {
        cancel_stream(scope, &owner, pending.stream, reason)?;
    }
    Ok(())
}
pub(super) fn cancel_stream(
    scope: &mut v8::PinScope,
    owner: &Rc<V8BindingState>,
    stream: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (terminal, host_stream) = match &*platform_object(owner, stream)? {
        PlatformObjectRecord::ReadableStream(record) => (
            record.closed || record.error.is_some(),
            record.host_stream_id,
        ),
        _ => return Ok(()),
    };
    if !terminal {
        let context = owner
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("fetch body cancellation realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        // Source cancellation cannot replace the fetch rejection reason.
        v8::tc_scope!(let scope,scope);
        let result = service_worker_events::cancel_body(scope, owner, stream, reason);
        if result.is_err() || scope.exception().is_some() {
            if let Some(stream) = host_stream
                && let Some(host) = owner.platform_host.borrow().as_ref()
            {
                host.abandon_http_stream(stream);
            }
            scope.reset();
        }
    }
    Ok(())
}

fn settle(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    success: bool,
) -> Result<(), V8BindingError> {
    let state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("fetch body realm"))?;
    let id = v8::Local::<v8::BigInt>::try_from(args.data())
        .map_err(|_| V8BindingError::Allocation("fetch body identity"))?
        .u64_value()
        .0;
    let pending = { state.fetch_bodies.borrow_mut().pending.remove(&id) };
    let Some(mut pending) = pending else {
        return Ok(());
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    if !success {
        let reason = args.get(0);
        resolver.reject(scope, reason);
        cancel(scope, &pending, reason)?;
        return Ok(());
    }
    if let Some(reason) = aborted(scope, &state, pending.signal)? {
        resolver.reject(scope, reason);
        cancel(scope, &pending, reason)?;
        return Ok(());
    }
    if state.execution_retired()
        || state.document_generation() != pending.generation
        || state.document_authority_epoch() != pending.authority
        || pending
            .owner
            .upgrade()
            .is_none_or(|owner| owner.execution_retired())
    {
        let reason = dom_exception_value(scope, "AbortError")?;
        resolver.reject(scope, reason);
        cancel(scope, &pending, reason)?;
        return Ok(());
    }
    let Ok(bytes) = v8::Local::<v8::ArrayBuffer>::try_from(args.get(0)) else {
        let reason = readable_streams::type_error(scope, "fetch body did not produce bytes")?;
        resolver.reject(scope, reason);
        cancel(scope, &pending, reason)?;
        return Ok(());
    };
    if bytes.byte_length() > LIMIT {
        let reason = readable_streams::type_error(scope, "fetch body exceeds native limit")?;
        resolver.reject(scope, reason);
        cancel(scope, &pending, reason)?;
        return Ok(());
    }
    pending.request.body = bytes
        .get_backing_store()
        .iter()
        .map(|byte| byte.get())
        .collect();
    if let Err(error) = dispatch(scope, &state, pending.request, pending.signal, resolver) {
        let reason = readable_streams::type_error(scope, &error.to_string())?;
        resolver.reject(scope, reason);
    }
    Ok(())
}
fn fulfilled(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    if let Err(error) = settle(scope, &args, true) {
        throw_binding_error(scope, &error);
    }
}
fn rejected(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    if let Err(error) = settle(scope, &args, false) {
        throw_binding_error(scope, &error);
    }
}
pub(super) fn abort(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    signal: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let ids = state
        .fetch_bodies
        .borrow()
        .pending
        .iter()
        .filter_map(|(id, pending)| (pending.signal == signal).then_some(*id))
        .collect::<Vec<_>>();
    let mut failure = None;
    for id in ids {
        let pending = { state.fetch_bodies.borrow_mut().pending.remove(&id) };
        if let Some(pending) = pending {
            v8::Local::new(scope, &pending.resolver).reject(scope, reason);
            if let Err(error) = cancel(scope, &pending, reason) {
                state.fetch_bodies.borrow_mut().pending.insert(id, pending);
                failure = Some(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(super) fn source_retired(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let pending = { state.fetch_bodies.borrow_mut().pending.remove(&id) };
    if let Some(pending) = pending {
        let reason = dom_exception_value(scope, "AbortError")?;
        v8::Local::new(scope, &pending.resolver).reject(scope, reason);
        cancel(scope, &pending, reason)?;
    }
    Ok(())
}
pub(super) fn retire(state: &V8BindingState) {
    let (pending, dependents) = {
        let mut registry = state.fetch_bodies.borrow_mut();
        (
            std::mem::take(&mut registry.pending),
            std::mem::take(&mut registry.dependents),
        )
    };
    for (requester, id) in dependents {
        if let Some(requester) = requester
            .upgrade()
            .filter(|requester| !requester.execution_retired())
            && requester.fetch_bodies.borrow().pending.contains_key(&id)
        {
            requester
                .deferred_platform_tasks
                .borrow_mut()
                .push_back(DeferredPlatformTask::FetchBodySourceRetired { request: id });
        }
    }
    for pending in pending.into_values() {
        if let Some(owner) = pending.owner.upgrade() {
            if let Ok(record) = platform_object(&owner, pending.stream)
                && let PlatformObjectRecord::ReadableStream(stream) = &*record
                && let Some(host_stream) = stream.host_stream_id
                && let Some(host) = owner.platform_host.borrow().as_ref()
            {
                host.abandon_http_stream(host_stream);
            }
            if !owner.execution_retired() {
                owner.deferred_platform_tasks.borrow_mut().push_back(
                    DeferredPlatformTask::CancelFetchBody {
                        stream: pending.stream,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<PlatformHttpRequest>>,
    }
    impl PlatformHost for Host {
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            if let PlatformHostRequest::HttpStart { request } = request {
                self.requests.borrow_mut().push(request);
            }
            Ok(())
        }
        fn cancel(&self, _: HostOperationId) {}
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
            Vec::new()
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn fetch_dispatches_actual_delayed_chunks_once_after_body_close() {
        super::super::tests::initialize_v8();
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
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        eval(
            scope,
            "globalThis.upload=new ReadableStream({start(c){globalThis.controller=c}});globalThis.result=fetch('https://example.test/upload',{method:'POST',body:upload,duplex:'half'});result.catch(()=>{});",
        );
        scope.perform_microtask_checkpoint();
        assert!(host.requests.borrow().is_empty());
        eval(scope, "controller.enqueue(new Uint8Array([0,255,65]));");
        scope.perform_microtask_checkpoint();
        assert!(host.requests.borrow().is_empty());
        eval(
            scope,
            "controller.enqueue(new Uint8Array([66,0]));controller.close();",
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(host.requests.borrow().len(), 1);
        assert_eq!(host.requests.borrow()[0].body, vec![0, 255, 65, 66, 0]);
        scope.perform_microtask_checkpoint();
        assert_eq!(host.requests.borrow().len(), 1);
        assert!(state.fetch_bodies.borrow().pending.is_empty());
    }
    #[test]
    fn abort_error_and_limit_during_drain_never_admit_http() {
        super::super::tests::initialize_v8();
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
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        eval(
            scope,
            "globalThis.reason={abort:1};globalThis.ac=new AbortController;globalThis.cancelled=null;globalThis.error=null;fetch('https://example.test/upload',{method:'POST',duplex:'half',signal:ac.signal,body:new ReadableStream({cancel(r){cancelled=r}})}).catch(e=>error=e);ac.abort(reason);",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "error===reason&&cancelled===reason").is_true());
        assert!(host.requests.borrow().is_empty());
        eval(
            scope,
            "globalThis.sourceError={source:1};globalThis.seen=null;fetch('https://example.test/upload',{method:'POST',duplex:'half',body:new ReadableStream({start(c){c.error(sourceError)}})}).catch(e=>seen=e);",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "seen===sourceError").is_true());
        eval(
            scope,
            "globalThis.tooBig=null;fetch('https://example.test/upload',{method:'POST',duplex:'half',body:new ReadableStream({start(c){c.enqueue(new Uint8Array(16*1024*1024+1));c.close()}})}).catch(e=>tooBig=e);",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "tooBig instanceof TypeError").is_true());
        assert!(host.requests.borrow().is_empty());
        assert!(state.fetch_bodies.borrow().pending.is_empty());
    }
    #[test]
    fn foreign_upload_abort_preserves_source_reason_and_caller_context() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (_source, body) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let source = install_generated_bindings(scope, first, dom, RealmId(91), 1).unwrap();
            let body = eval(
                scope,
                "globalThis.cancelReason=null;new ReadableStream({cancel(reason){cancelReason=reason;throw Error('cancel failure')}})",
            );
            (source, v8::Global::new(scope, body))
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, second, dom, RealmId(92), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let key = v8::String::new(scope, "foreignBody").unwrap();
        let body = v8::Local::new(scope, &body);
        second.global(scope).set(scope, key.into(), body).unwrap();
        let reason = eval(
            scope,
            "globalThis.reason={abort:true};globalThis.failure=null;const ac=new AbortController();fetch('https://example.test/upload',{method:'POST',duplex:'half',body:foreignBody,signal:ac.signal}).catch(e=>failure=e);ac.abort(reason);reason",
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(scope.get_current_context(), second);
        assert!(eval(scope, "failure===reason").is_true());
        assert!(host.requests.borrow().is_empty());
        assert!(state.fetch_bodies.borrow().pending.is_empty());
        let scope = &mut v8::ContextScope::new(scope, first);
        assert!(eval(scope, "cancelReason").strict_equals(reason));
    }
    #[test]
    fn foreign_body_source_retirement_rejects_requester_without_http() {
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
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let (source, body) = {
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let source = install_generated_bindings(scope, context, dom, RealmId(1), 2).unwrap();
            let body = eval(
                scope,
                "new ReadableStream({start(c){globalThis.pendingController=c}})",
            );
            (source, v8::Global::new(scope, body))
        };
        let key = v8::String::new(scope, "foreignBody").unwrap();
        let body = v8::Local::new(scope, body);
        context.global(scope).set(scope, key.into(), body);
        eval(
            scope,
            "globalThis.failure=null;fetch('https://example.test/upload',{method:'POST',duplex:'half',body:foreignBody}).catch(e=>failure=e);",
        );
        scope.perform_microtask_checkpoint();
        assert!(host.requests.borrow().is_empty());
        source.retire_execution();
        state.drain_platform_completions(scope, 32).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "failure&&failure.name==='AbortError'").is_true());
        assert!(host.requests.borrow().is_empty());
        assert!(state.fetch_bodies.borrow().pending.is_empty());
    }
}
