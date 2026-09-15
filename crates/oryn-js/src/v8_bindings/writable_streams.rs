//! Native ordered writable operations and asynchronous sink completion.
use super::*;

pub(super) struct Request {
    pub(super) chunk: Option<v8::Global<v8::Value>>,
    resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct Abort {
    reason: v8::Global<v8::Value>,
    resolver: v8::Global<v8::PromiseResolver>,
    already_erroring: bool,
}

pub(super) fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    reason: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("writable abort promise"))?;
    {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if let Some(abort) = &stream.pending_abort {
            return Ok(v8::Local::new(scope, &abort.resolver).get_promise(scope));
        }
        if stream.closed || (stream.error.is_some() && !stream.busy && stream.started) {
            let undefined = v8::undefined(scope).into();
            resolver.resolve(scope, undefined);
            return Ok(resolver.get_promise(scope));
        }
        stream.pending_abort = Some(Abort {
            reason: v8::Global::new(scope, reason),
            resolver: v8::Global::new(scope, resolver),
            already_erroring: stream.error.is_some(),
        });
    }
    error(scope, state, id, reason)?;
    drain_error(scope, state, id)?;
    Ok(resolver.get_promise(scope))
}

fn abort_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    abort_reaction(scope, args, false);
}
fn abort_rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    abort_reaction(scope, args, true);
}
fn abort_reaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rejected: bool,
) {
    let result = (|| -> Result<(), V8BindingError> {
        let (state, id) = reaction_owner(scope, &args)?;
        let resolver = {
            let mut record = platform_object_mut(&state, id)?;
            let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                return Err(V8BindingError::Allocation("WritableStream record"));
            };
            v8::Local::new(
                scope,
                stream
                    .abort_completion
                    .take()
                    .ok_or(V8BindingError::Allocation("abort resolver"))?,
            )
        };
        let failure = rejected.then(|| args.get(0));
        settle_abort(scope, &state, id, resolver, failure)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn settle_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    failure: Option<v8::Local<'s, v8::Value>>,
) -> Result<(), V8BindingError> {
    if let Some(error) = failure {
        resolver.reject(scope, error);
    } else {
        let undefined = v8::undefined(scope).into();
        resolver.resolve(scope, undefined);
    }
    if let PlatformObjectRecord::WritableStream(stream) = &mut *platform_object_mut(state, id)? {
        stream.abort_in_flight = false;
    }
    drain_error(scope, state, id)?;
    writable_writers::refresh(scope, state, id)
}

fn finish_abort(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    abort: Abort,
) -> Result<(), V8BindingError> {
    let (callback, source, error) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        let callback = match &stream.kind {
            WritableKind::Sink { abort, .. } => abort.clone(),
            _ => None,
        };
        (callback, stream.source.clone(), stream.error.clone())
    };
    let resolver = v8::Local::new(scope, abort.resolver);
    if abort.already_erroring {
        let error = v8::Local::new(
            scope,
            error.ok_or(V8BindingError::Allocation("abort error"))?,
        );
        return settle_abort(scope, state, id, resolver, Some(error));
    }
    let value = if let Some(callback) = callback {
        let callback = v8::Local::new(scope, callback);
        let source = v8::Local::new(scope, source);
        let reason = v8::Local::new(scope, abort.reason);
        v8::tc_scope!(let scope, scope);
        match callback.call(scope, source, &[reason]) {
            Some(value) => value,
            None => {
                let error = scope.exception().ok_or_else(|| {
                    V8BindingError::EventCallback("abort callback terminated".into())
                })?;
                scope.reset();
                return settle_abort(scope, state, id, resolver, Some(error));
            }
        }
    } else {
        v8::undefined(scope).into()
    };
    let controller_id = match &*platform_object(state, id)? {
        PlatformObjectRecord::WritableStream(stream) => stream.controller_id,
        _ => return Err(V8BindingError::Allocation("WritableStream record")),
    };
    let controller = new_platform_object(
        scope,
        state,
        "WritableStreamDefaultController",
        controller_id,
    )?;
    if let PlatformObjectRecord::WritableStream(stream) = &mut *platform_object_mut(state, id)? {
        stream.abort_completion = Some(v8::Global::new(scope, resolver));
    }
    let data = controller;
    let fulfilled = v8::Function::builder(abort_fulfilled)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("abort fulfillment"))?;
    let rejected = v8::Function::builder(abort_rejected)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("abort rejection"))?;
    resolved_promise(scope, value)?
        .then2(scope, fulfilled, rejected)
        .ok_or(V8BindingError::Allocation("abort reactions"))?;
    Ok(())
}

pub(super) fn enqueue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    chunk: Option<v8::Local<'s, v8::Value>>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("writable operation promise"))?;
    {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if let Some(error) = &stream.error {
            let error = v8::Local::new(scope, error);
            resolver.reject(scope, error);
            return Ok(resolver.get_promise(scope));
        }
        if stream.closed || stream.closing {
            let error =
                readable_streams::type_error(scope, "Writable stream is closing or closed")?;
            resolver.reject(scope, error);
            return Ok(resolver.get_promise(scope));
        }
        if chunk.is_none() {
            stream.closing = true;
        }
        stream.queue.push_back(Request {
            chunk: chunk.map(|value| v8::Global::new(scope, value)),
            resolver: v8::Global::new(scope, resolver),
        });
    }
    writable_writers::refresh(scope, state, id)?;
    pump(scope, state, id)?;
    Ok(resolver.get_promise(scope))
}

fn reaction_owner(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let controller = v8::Local::<v8::Object>::try_from(args.data())
        .map_err(|_| V8BindingError::Allocation("writable reaction controller"))?;
    let state = binding_state_for_receiver(scope, controller)
        .ok_or(V8BindingError::Allocation("writable reaction state"))?;
    let controller_id = stateful_id(scope, controller)?;
    let id = match &*platform_object(&state, controller_id)? {
        PlatformObjectRecord::WritableController { stream_id } => *stream_id,
        _ => return Err(V8BindingError::Allocation("writable reaction record")),
    };
    Ok((state, id))
}

fn fulfilled(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| {
        let (state, id) = reaction_owner(scope, &args)?;
        finish(scope, &state, id, None)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn rejected(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| {
        let (state, id) = reaction_owner(scope, &args)?;
        finish(scope, &state, id, Some(args.get(0)))
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn observe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let controller_id = match &*platform_object(state, id)? {
        PlatformObjectRecord::WritableStream(stream) => stream.controller_id,
        _ => return Err(V8BindingError::Allocation("WritableStream record")),
    };
    let data = new_platform_object(
        scope,
        state,
        "WritableStreamDefaultController",
        controller_id,
    )?;
    let fulfilled = v8::Function::builder(fulfilled)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("writable fulfillment reaction"))?;
    let rejected = v8::Function::builder(rejected)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("writable rejection reaction"))?;
    resolved_promise(scope, value)?
        .then2(scope, fulfilled, rejected)
        .ok_or(V8BindingError::Allocation("writable promise reactions"))?;
    Ok(())
}

pub(super) fn start(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    callback: Option<v8::Global<v8::Function>>,
) -> Result<(), V8BindingError> {
    let (source, controller_id, transform) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        let transform = match &stream.kind {
            WritableKind::Transform { controller_id, .. } => Some(*controller_id),
            _ => None,
        };
        (stream.source.clone(), stream.controller_id, transform)
    };
    let value = if let Some(callback) = callback {
        let receiver = v8::Local::new(scope, source);
        let callback = v8::Local::new(scope, callback);
        let controller = match transform {
            Some(id) => new_platform_object(scope, state, "TransformStreamDefaultController", id)?,
            None => new_platform_object(
                scope,
                state,
                "WritableStreamDefaultController",
                controller_id,
            )?,
        };
        callback
            .call(scope, receiver, &[controller.into()])
            .ok_or(V8BindingError::JavaScriptException)?
    } else {
        v8::undefined(scope).into()
    };
    observe(scope, state, id, value)
}

pub(super) fn pump(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let context = match &*platform_object(state, id)? {
        PlatformObjectRecord::WritableStream(stream) => stream.creation_context.to_local(scope),
        _ => None,
    }
    .ok_or(V8BindingError::Allocation("writable creation context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    if transform_streams::write_blocked(state, id)? {
        return Ok(());
    }
    let (callback, source, controller_id, transform, transform_controller, chunk) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if !stream.started || stream.busy || stream.closed || stream.error.is_some() {
            return Ok(());
        }
        let Some(request) = stream.queue.front() else {
            return Ok(());
        };
        let closing = request.chunk.is_none();
        let (callback, transform, transform_controller) = match &stream.kind {
            WritableKind::Sink { write, close, .. } => (
                if closing {
                    close.clone()
                } else {
                    write.clone()
                },
                None,
                None,
            ),
            WritableKind::Transform {
                transform,
                flush,
                readable_controller_id,
                controller_id,
                ..
            } => (
                if closing {
                    flush.clone()
                } else {
                    transform.clone()
                },
                Some(*readable_controller_id),
                Some(*controller_id),
            ),
        };
        let chunk = request.chunk.clone();
        stream.busy = true;
        (
            callback,
            stream.source.clone(),
            stream.controller_id,
            transform,
            transform_controller,
            chunk,
        )
    };
    let codec = match &*platform_object(state, id)? {
        PlatformObjectRecord::WritableStream(WritableStreamRecord {
            kind: WritableKind::Transform { codec_id, .. },
            ..
        }) => *codec_id,
        _ => None,
    };
    if let Some(codec) = codec {
        let chunk = chunk.map(|chunk| v8::Local::new(scope, chunk));
        v8::tc_scope!(let scope, scope);
        match text_encoding::transform_chunk(scope, state, codec, chunk) {
            Ok(output) => {
                if let Some(output) = output {
                    let controller =
                        transform.ok_or(V8BindingError::Allocation("encoding readable"))?;
                    if let Err(error) = readable_enqueue(scope, state, controller, output) {
                        let reason = readable_streams::type_error(scope, &error.to_string())?;
                        return finish(scope, state, id, Some(reason));
                    }
                }
                let undefined = v8::undefined(scope).into();
                return observe(scope, state, id, undefined);
            }
            Err(error) => {
                let reason = if let Some(exception) = scope.exception() {
                    scope.reset();
                    exception
                } else {
                    readable_streams::type_error(scope, &error.to_string())?
                };
                return finish(scope, state, id, Some(reason));
            }
        }
    }
    let controller = match transform_controller {
        Some(id) => new_platform_object(scope, state, "TransformStreamDefaultController", id)?,
        None => new_platform_object(
            scope,
            state,
            "WritableStreamDefaultController",
            controller_id,
        )?,
    };
    let value = if let Some(callback) = callback {
        let receiver = v8::Local::new(scope, source);
        let callback = v8::Local::new(scope, callback);
        let arguments = match chunk {
            Some(chunk) => vec![v8::Local::new(scope, chunk), controller.into()],
            None if transform.is_some() => vec![controller.into()],
            None => vec![],
        };
        v8::tc_scope!(let scope, scope);
        match callback.call(scope, receiver, &arguments) {
            Some(value) => value,
            None => {
                let reason = scope.exception().ok_or_else(|| {
                    V8BindingError::EventCallback("writable callback terminated".into())
                })?;
                scope.reset();
                return finish(scope, state, id, Some(reason));
            }
        }
    } else {
        if let (Some(controller_id), Some(chunk)) = (transform, chunk) {
            let chunk = v8::Local::new(scope, chunk);
            readable_enqueue(scope, state, controller_id, chunk)?;
        }
        v8::undefined(scope).into()
    };
    observe(scope, state, id, value)
}

fn finish(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    failure: Option<v8::Local<v8::Value>>,
) -> Result<(), V8BindingError> {
    let (request, transform) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        let request = if !stream.started {
            stream.started = true;
            None
        } else {
            stream.busy = false;
            stream.queue.pop_front()
        };
        let transform = match &stream.kind {
            WritableKind::Transform {
                readable_controller_id,
                ..
            } => Some(*readable_controller_id),
            _ => None,
        };
        (request, transform)
    };
    if let Some(request) = request {
        let resolver = v8::Local::new(scope, request.resolver);
        if request.chunk.is_none() {
            let pending_abort = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("WritableStream record"));
                };
                if failure.is_none() {
                    stream.error = None;
                }
                stream.pending_abort.take()
            };
            if let Some(abort) = pending_abort {
                let resolver = v8::Local::new(scope, abort.resolver);
                if let Some(error) = failure {
                    resolver.reject(scope, error);
                } else {
                    let undefined = v8::undefined(scope).into();
                    resolver.resolve(scope, undefined);
                }
            }
        }
        if let Some(error) = failure {
            resolver.reject(scope, error);
        } else {
            if request.chunk.is_none() {
                if let PlatformObjectRecord::WritableStream(stream) =
                    &mut *platform_object_mut(state, id)?
                {
                    stream.closed = true;
                }
                if let Some(controller) = transform {
                    readable_close(scope, state, controller)?;
                }
            }
            let undefined = v8::undefined(scope).into();
            resolver.resolve(scope, undefined);
        }
    }
    if let Some(reason) = failure {
        error(scope, state, id, reason)?;
    }
    drain_error(scope, state, id)?;
    writable_writers::refresh(scope, state, id)?;
    pump(scope, state, id)
}

pub(super) fn error(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let transform = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if stream.closed || stream.error.is_some() {
            return Ok(());
        }
        stream.error = Some(v8::Global::new(scope, reason));
        match &stream.kind {
            WritableKind::Transform {
                readable_controller_id,
                ..
            } => Some(*readable_controller_id),
            _ => None,
        }
    };
    if let Some(controller) = transform {
        let readable = match &*platform_object(state, controller)? {
            PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
            _ => return Err(V8BindingError::Allocation("transform readable")),
        };
        readable_streams::error(scope, state, readable, reason)?;
    }
    drain_error(scope, state, id)?;
    writable_writers::refresh(scope, state, id)
}

fn drain_error(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let (reason, requests, abort) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if stream.busy || !stream.started || stream.abort_in_flight {
            return Ok(());
        }
        let Some(reason) = stream.error.clone() else {
            return Ok(());
        };
        stream.abort_in_flight = stream.pending_abort.is_some();
        (
            reason,
            std::mem::take(&mut stream.queue),
            stream.pending_abort.take(),
        )
    };
    let reason = v8::Local::new(scope, reason);
    for request in requests {
        if request.chunk.is_none() && abort.is_some() {
            if let PlatformObjectRecord::WritableStream(stream) =
                &mut *platform_object_mut(state, id)?
            {
                stream.queue.push_back(request);
            }
        } else {
            v8::Local::new(scope, request.resolver).reject(scope, reason);
        }
    }
    if let Some(abort) = abort {
        finish_abort(scope, state, id, abort)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::v8_bindings) fn run(source: &str) {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, source).unwrap();
        let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
        if let Some(error) = scope.exception() {
            panic!("{}", error.to_rust_string_lossy(scope));
        }
        let promise = v8::Local::<v8::Promise>::try_from(result.unwrap()).unwrap();
        scope.perform_microtask_checkpoint();
        assert_ne!(
            promise.state(),
            v8::PromiseState::Pending,
            "test promise remained pending"
        );
        assert_eq!(
            promise.state(),
            v8::PromiseState::Fulfilled,
            "{}",
            promise.result(scope).to_rust_string_lossy(scope)
        );
        assert_eq!(promise.result(scope).to_rust_string_lossy(scope), "ok");
    }

    #[test]
    fn async_start_writes_and_close_are_ordered_and_keep_sink_receiver() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let start,write,close;const calls=[];
            const sink={start(c){check(this===sink,'start receiver');return new Promise(r=>start=r)},
                write(x,c){check(this===sink,'write receiver');check(c instanceof WritableStreamDefaultController,'controller');calls.push(x);return new Promise(r=>write=r)},
                close(){check(this===sink,'close receiver');calls.push('close');return new Promise(r=>close=r)}};
            const w=new WritableStream(sink).getWriter();const a=w.write(1),b=w.write(2),done=w.close();
            await Promise.resolve();check(calls.length===0,'start pending');start();await Promise.resolve();await Promise.resolve();
            check(calls.join()==='1','first pending');write(91);check(await a===undefined,'write result');
            check(calls.join()==='1,2','second pending');write();await b;check(calls.join()==='1,2,close','close after writes');
            let finished=false;done.then(()=>finished=true);await Promise.resolve();check(!finished,'close pending');close(42);
            check(await done===undefined,'close result');let error;try{await w.write(3)}catch(e){error=e}check(error instanceof TypeError,'closed write');return 'ok';
        })()"#);
    }

    #[test]
    fn start_and_write_failures_reject_queued_and_future_operations() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};const reason={failure:1};
            for(const sink of [{start(){return Promise.reject(reason)}},{write(){throw reason}}, {write(){return {then(resolve,reject){reject(reason)}}}}]){
                const w=new WritableStream(sink).getWriter();const a=w.write(1),b=w.write(2),c=w.close();
                for(const p of [a,b,c]){let e;try{await p}catch(x){e=x}check(e===reason,'queued original error')}
                let e;try{await w.write(3)}catch(x){e=x}check(e===reason,'future original error');
            }return 'ok';
        })()"#);
    }

    #[test]
    fn controller_error_waits_for_inflight_write_before_rejecting_queue() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let controller,finish;const reason={failure:1};let calls=0;
            const w=new WritableStream({start(c){controller=c},write(){calls++;return new Promise(r=>finish=r)}}).getWriter();
            const a=w.write(1),b=w.write(2);let rejected=false;b.catch(e=>{check(e===reason,'stored error');rejected=true});
            await Promise.resolve();await Promise.resolve();controller.error(reason);await Promise.resolve();check(!rejected,'wait for inflight write');
            finish();await a;try{await b}catch(e){check(e===reason,'queued error')}check(calls===1,'queued callback skipped');return 'ok';
        })()"#);
    }

    #[test]
    fn abort_waits_for_inflight_write_and_awaits_sink_cleanup() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let write,cleanup;let aborted=false;const reason={abort:1};
            const sink={write(){return new Promise(r=>write=r)},abort(x){check(this===sink,'abort receiver');check(x===reason,'abort reason');aborted=true;return new Promise(r=>cleanup=r)}};
            const w=new WritableStream(sink).getWriter();const a=w.write(1),b=w.write(2);b.catch(()=>{});
            await Promise.resolve();await Promise.resolve();const abort=w.abort(reason);check(!aborted,'abort waits');write();await a;
            let error;try{await b}catch(e){error=e}check(error===reason,'queued abort');check(aborted,'cleanup invoked');
            let settled=false;abort.then(()=>settled=true);await Promise.resolve();check(!settled,'cleanup pending');cleanup(99);check(await abort===undefined,'abort result');return 'ok';
        })()"#);
    }

    #[test]
    fn transform_flush_waits_before_closing_and_propagates_failures() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let flush;
            const transformer={transform(x,c){check(this===transformer,'transform receiver');c.enqueue(x+1)},flush(c){check(this===transformer,'flush receiver');return new Promise(r=>flush=()=>{c.enqueue(9);r()})}};
            const t=new TransformStream(transformer),r=t.readable.getReader(),w=t.writable.getWriter();
            const read=r.read();await w.write(1);check((await read).value===2,'transform chunk');const close=w.close();
            await Promise.resolve();let ended=false;r.closed.then(()=>ended=true);await Promise.resolve();check(!ended,'flush pending');
            flush();await close;check((await r.read()).value===9,'flush chunk');check((await r.read()).done,'readable closed');
            for(const callback of ['transform','flush']){
                const reason={failure:callback};const t=new TransformStream({[callback](){return Promise.reject(reason)}});
                const r=t.readable.getReader(),w=t.writable.getWriter();const read=r.read();
                const op=callback==='transform'?w.write(1):w.close();
                for(const p of [op,read,r.closed]){let e;try{await p}catch(x){e=x}check(e===reason,'transform failure propagates')}
            }return 'ok';
        })()"#);
    }
    #[test]
    fn writer_ready_closed_release_and_reacquisition_preserve_queued_writes() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let finish;
            const s=new WritableStream({write(){return new Promise(r=>finish=r)}}),w=s.getWriter();
            const initiallyReady=w.ready;check(w.closed===w.closed,'closed identity');await initiallyReady;
            const write=w.write(1);const pressured=w.ready;check(pressured!==initiallyReady,'new ready promise');check(pressured===w.ready,'ready identity');
            check(w.desiredSize===0,'desired size');let ready=false;pressured.then(()=>ready=true,()=>{});
            await Promise.resolve();check(!ready,'backpressure pending');w.releaseLock();check(!s.locked,'released');
            for(const p of [w.ready,w.closed,w.write(2),w.close(),w.abort()]){let e;try{await p}catch(x){e=x}check(e instanceof TypeError,'released writer error')}
            const next=s.getWriter();let nextReady=false;next.ready.then(()=>nextReady=true);await Promise.resolve();check(!nextReady,'new writer sees pressure');
            finish();await write;await next.ready;check(next.desiredSize===1,'queue drained');
            let closed=false;next.closed.then(()=>closed=true);await Promise.resolve();check(!closed,'closed pending');await next.close();await next.closed;
            check(next.desiredSize===0,'closed size');check(closed,'closed settled');return 'ok';
        })()"#);
    }

    #[test]
    fn locked_stream_operations_reject_and_writer_errors_preserve_identity() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let controller;const reason={failure:1};
            const s=new WritableStream({start(c){controller=c}}),w=s.getWriter();
            for(const p of [s.close(),s.abort()]){let e;try{await p}catch(x){e=x}check(e instanceof TypeError,'locked stream')}
            await w.ready;controller.error(reason);
            for(const p of [w.ready,w.closed]){let e;try{await p}catch(x){e=x}check(e===reason,'writer original error')}
            check(w.desiredSize===null,'errored size');w.releaseLock();const next=s.getWriter();
            for(const p of [next.ready,next.closed,next.write(1)]){let e;try{await p}catch(x){e=x}check(e===reason,'new writer original error')}
            return 'ok';
        })()"#);
    }

    #[test]
    fn in_flight_close_settles_abort_without_calling_sink_abort() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const fails of [false,true]){
                let finish,fail,aborts=0;const reason={close:1};
                const w=new WritableStream({close(){return new Promise((r,j)=>{finish=r;fail=j})},abort(){aborts++}}).getWriter();
                const close=w.close();await Promise.resolve();await Promise.resolve();const abort=w.abort({abort:1});
                if(fails)fail(reason);else finish();
                for(const p of [close,abort]){let e;try{await p}catch(x){e=x}check(fails?e===reason:e===undefined,'close/abort completion')}
                check(aborts===0,'close owns completion');if(!fails)await w.closed;
            }return 'ok';
        })()"#);
    }
    #[test]
    fn abort_cleanup_settles_before_queued_close_and_writer_closed() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const fails of [false,true]){
                let write,cleanup,fail;const reason={abort:1},cleanupError={cleanup:1},events=[];
                const w=new WritableStream({write(){return new Promise(r=>write=r)},abort(){return new Promise((r,j)=>{cleanup=r;fail=j})}}).getWriter();
                const a=w.write(1),close=w.close();close.catch(e=>{check(e===reason,'close error');events.push('close')});
                w.closed.catch(e=>{check(e===reason,'closed error');events.push('closed')});
                await Promise.resolve();await Promise.resolve();const abort=w.abort(reason);
                abort.then(()=>events.push('abort'),e=>{check(e===cleanupError,'cleanup error');events.push('abort')});
                write();await a;await Promise.resolve();check(events.length===0,'cleanup must complete first');
                if(fails)fail(cleanupError);else cleanup();try{await abort}catch(e){check(e===cleanupError,'abort result')}
                try{await close}catch(e){}try{await w.closed}catch(e){}
                check(events.join()==='abort,close,closed','completion order: '+events.join());
            }return 'ok';
        })()"#);
    }
}
