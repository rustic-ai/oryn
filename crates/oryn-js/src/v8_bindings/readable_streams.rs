//! Native readable-stream terminal state and reader ownership.
use super::*;

pub(super) fn type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let message =
        v8::String::new(scope, message).ok_or(V8BindingError::Allocation("stream error"))?;
    Ok(v8::Exception::type_error(scope, message))
}

pub(super) fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("stream rejection"))?;
    resolver.reject(scope, error);
    Ok(resolver.get_promise(scope))
}

pub(super) fn stored_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    stream_id: u64,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = platform_object(state, stream_id)?;
    let PlatformObjectRecord::ReadableStream(stream) = &*record else {
        return Err(V8BindingError::Allocation("ReadableStream record"));
    };
    Ok(stream
        .error
        .as_ref()
        .map(|error| stream_graph::local(scope, error)))
}

pub(super) fn reader_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    reader_id: u64,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = platform_object(state, reader_id)?;
    let PlatformObjectRecord::ReadableReader { released, .. } = &*record else {
        return Err(V8BindingError::Allocation("ReadableStream reader"));
    };
    if *released {
        Ok(Some(type_error(scope, "Reader has released its stream")?))
    } else {
        Ok(None)
    }
}

pub(super) fn acquire(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream_id: u64,
) -> Result<u64, V8BindingError> {
    let closed = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("reader closed promise"))?;
    let (error, drained) = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        if stream.locked {
            return Err(V8BindingError::Unsupported {
                interface: "ReadableStream".into(),
                member: "TypeError: stream is locked".into(),
            });
        }
        stream.locked = true;
        (
            stream.error.clone(),
            stream.closed && stream.queue.is_empty(),
        )
    };
    if let Some(error) = error {
        let error = stream_graph::local(scope, error);
        closed.reject(scope, error);
        closed.get_promise(scope).mark_as_handled();
    } else if drained {
        let undefined = v8::undefined(scope).into();
        closed.resolve(scope, undefined);
    }
    let reader = register_platform_object(
        state,
        PlatformObjectRecord::ReadableReader {
            byob: false,
            stream_id,
            closed: stream_graph::Owned::new(scope, closed),
            released: false,
        },
    );
    if let PlatformObjectRecord::ReadableStream(stream) =
        &mut *platform_object_mut(state, stream_id)?
    {
        stream.reader_id = Some(reader);
    }
    Ok(reader)
}

pub(super) fn settle_closed(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream_id: u64,
) -> Result<(), V8BindingError> {
    let reader_id = {
        let record = platform_object(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        if !stream.closed || !stream.queue.is_empty() || stream.error.is_some() {
            return Ok(());
        }
        stream.reader_id
    };
    if let Some(reader_id) = reader_id {
        let record = platform_object(state, reader_id)?;
        if let PlatformObjectRecord::ReadableReader {
            closed,
            released: false,
            ..
        } = &*record
        {
            let resolver = stream_graph::local(scope, closed);
            let undefined = v8::undefined(scope).into();
            resolver.resolve(scope, undefined);
        }
    }
    Ok(())
}

pub(super) fn error(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream_id: u64,
    error: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (waiting, reader_id, consumer) = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        if stream.error.is_some()
            || (stream.closed
                && stream.queue.is_empty()
                && stream
                    .bytes
                    .as_ref()
                    .is_none_or(|bytes| bytes.pending.is_empty()))
        {
            return Ok(());
        }
        stream.error = Some(stream_graph::Owned::new(scope, error));
        stream.queue.clear();
        stream.closed = true;
        (
            std::mem::take(&mut stream.waiting),
            stream.reader_id,
            stream.consumer.take(),
        )
    };
    if let Some(consumer) = consumer {
        stream_graph::local(scope, consumer.resolver).reject(scope, error);
    }
    for waiting in waiting {
        stream_graph::local(scope, waiting).reject(scope, error);
    }
    if let Some(reader_id) = reader_id {
        let record = platform_object(state, reader_id)?;
        if let PlatformObjectRecord::ReadableReader {
            closed,
            released: false,
            ..
        } = &*record
        {
            let resolver = stream_graph::local(scope, closed);
            resolver.reject(scope, error);
            resolver.get_promise(scope).mark_as_handled();
        }
    }
    byte_streams::fail(scope, state, stream_id, error)?;
    stream_tee::error_branches(scope, state, stream_id, error)?;
    Ok(())
}

pub(super) fn release(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    reader_id: u64,
) -> Result<(), V8BindingError> {
    let error = type_error(scope, "Reader has released its stream")?;
    let stream_id = {
        let mut record = platform_object_mut(state, reader_id)?;
        let PlatformObjectRecord::ReadableReader {
            stream_id,
            closed,
            released,
            ..
        } = &mut *record
        else {
            return Err(V8BindingError::Allocation("ReadableStream reader"));
        };
        if *released {
            return Ok(());
        }
        *released = true;
        let mut resolver = stream_graph::local(scope, &*closed);
        if resolver.get_promise(scope).state() != v8::PromiseState::Pending {
            resolver = v8::PromiseResolver::new(scope)
                .ok_or(V8BindingError::Allocation("released reader promise"))?;
            *closed = stream_graph::Owned::new(scope, resolver);
        }
        resolver.reject(scope, error);
        resolver.get_promise(scope).mark_as_handled();
        *stream_id
    };
    let waiting = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        stream.locked = false;
        stream.reader_id = None;
        std::mem::take(&mut stream.waiting)
    };
    for resolver in waiting {
        stream_graph::local(scope, resolver).reject(scope, error);
    }
    byte_streams::release(scope, state, stream_id, error)?;
    Ok(())
}

fn discard_callback_result(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
}

pub(super) fn cancel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    callback: Option<stream_graph::Owned<v8::Function>>,
    reason: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    if !stream_graph::algorithms_available(&state) {
        let reason = dom_exception_value(scope, "AbortError")?;
        return reject(scope, reason);
    }

    let source = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.source.clone(),
        _ => return Err(V8BindingError::Allocation("ReadableStream record")),
    };
    let receiver = source
        .map(|s| stream_graph::local(scope, s))
        .unwrap_or_else(|| v8::undefined(scope).into());
    stream_graph::refresh(scope, state)?;
    let value = if let Some(callback) = callback {
        let callback = stream_graph::local(scope, callback);
        v8::tc_scope!(let scope, scope);
        match callback.call(scope, receiver, &[reason]) {
            Some(value) => value,
            None => {
                let error = scope.exception().ok_or_else(|| {
                    V8BindingError::EventCallback("ReadableStream cancel terminated".into())
                })?;
                scope.reset();
                return reject(scope, error);
            }
        }
    } else {
        v8::undefined(scope).into()
    };
    let promise = resolved_promise(scope, value)?;
    let discard = v8::Function::new(scope, discard_callback_result).ok_or(
        V8BindingError::Allocation("stream cancellation fulfillment"),
    )?;
    promise
        .then(scope, discard)
        .ok_or(V8BindingError::Allocation("stream cancellation promise"))
}

// Promise reactions retain the native controller to identify the stream's owner.
// No page-replaceable Promise methods participate in scheduling source callbacks.
fn fulfilled(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let controller = v8::Local::<v8::Object>::try_from(args.data())
            .map_err(|_| V8BindingError::Allocation("stream reaction controller"))?;
        let state = binding_state_for_receiver(scope, controller)
            .ok_or(V8BindingError::Allocation("stream reaction state"))?;
        if !stream_graph::algorithms_available(&state) {
            return Ok(());
        }
        let controller_id = stateful_id(scope, controller)?;
        let id = match &*platform_object(&state, controller_id)? {
            PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
            _ => return Err(V8BindingError::Allocation("stream reaction record")),
        };
        let should_pull = {
            let mut record = platform_object_mut(&state, id)?;
            let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                return Err(V8BindingError::Allocation("ReadableStream record"));
            };
            if !stream.started {
                stream.started = true;
                true
            } else {
                stream.pulling = false;
                std::mem::take(&mut stream.pull_again)
            }
        };
        if should_pull {
            pull(scope, &state, id)?;
        }
        stream_graph::refresh(scope, &state)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn rejected(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let controller = v8::Local::<v8::Object>::try_from(args.data())
            .map_err(|_| V8BindingError::Allocation("stream reaction controller"))?;
        let state = binding_state_for_receiver(scope, controller)
            .ok_or(V8BindingError::Allocation("stream reaction state"))?;
        if !stream_graph::algorithms_available(&state) {
            return Ok(());
        }
        let controller_id = stateful_id(scope, controller)?;
        let id = match &*platform_object(&state, controller_id)? {
            PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
            _ => return Err(V8BindingError::Allocation("stream reaction record")),
        };
        error(scope, &state, id, args.get(0))?;
        stream_graph::refresh(scope, &state)
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
    let promise = resolved_promise(scope, value)?;
    let controller_id = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.controller_id,
        _ => return Err(V8BindingError::Allocation("stream reaction record")),
    };
    let data = new_platform_object(
        scope,
        state,
        "ReadableStreamDefaultController",
        controller_id,
    )?;
    let fulfilled = v8::Function::builder(fulfilled)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("stream fulfillment reaction"))?;
    let rejected = v8::Function::builder(rejected)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("stream rejection reaction"))?;
    promise
        .then2(scope, fulfilled, rejected)
        .ok_or(V8BindingError::Allocation("stream promise reactions"))?;
    Ok(())
}

pub(super) fn start(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    callback: Option<stream_graph::Owned<v8::Function>>,
) -> Result<(), V8BindingError> {
    let (source, controller_id) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        (stream.source.clone(), stream.controller_id)
    };
    stream_graph::refresh(scope, state)?;
    let value = if let Some(callback) = callback {
        let callback = stream_graph::local(scope, callback);
        let receiver = source
            .map(|s| stream_graph::local(scope, s))
            .unwrap_or_else(|| v8::undefined(scope).into());
        let controller = new_platform_object(
            scope,
            state,
            "ReadableStreamDefaultController",
            controller_id,
        )?;
        callback
            .call(scope, receiver, &[controller.into()])
            .ok_or(V8BindingError::JavaScriptException)?
    } else {
        v8::undefined(scope).into()
    };
    observe(scope, state, id, value)
}

pub(super) fn pull(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    if !stream_graph::algorithms_available(&state) {
        return Ok(());
    }

    let upstream = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.tee_source,
        _ => None,
    };
    if let Some((source, _)) = upstream {
        return stream_tee::pump(scope, state, source);
    }
    let tee_demand = stream_tee::demand(state, id)?;
    if tee_demand == Some(false) {
        return Ok(());
    }
    let transform = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.transform_writable_id,
        _ => None,
    };
    if let Some(writable) = transform {
        return writable_streams::pump(scope, state, writable);
    }
    let (callback, source, controller_id) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        if !stream.started
            || stream.closed
            || stream.error.is_some()
            || stream
                .bytes
                .as_ref()
                .is_some_and(|bytes| bytes.close_requested)
            || (tee_demand != Some(true)
                && stream.waiting.is_empty()
                && stream.consumer.is_none()
                && stream
                    .bytes
                    .as_ref()
                    .is_none_or(|bytes| bytes.pending.is_empty())
                && byte_streams::queue_total(scope, stream)? >= stream.high_water_mark)
        {
            return Ok(());
        }
        let Some(callback) = stream.pull.clone() else {
            return Ok(());
        };
        if stream.pulling {
            stream.pull_again = true;
            return Ok(());
        }
        stream.pulling = true;
        (callback, stream.source.clone(), stream.controller_id)
    };
    stream_graph::refresh(scope, state)?;
    let callback = stream_graph::local(scope, callback);
    let receiver = source
        .map(|s| stream_graph::local(scope, s))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let controller = new_platform_object(
        scope,
        state,
        "ReadableStreamDefaultController",
        controller_id,
    )?;
    v8::tc_scope!(let scope, scope);
    match callback.call(scope, receiver, &[controller.into()]) {
        Some(value) => observe(scope, state, id, value),
        None => {
            let reason = scope.exception().ok_or_else(|| {
                V8BindingError::EventCallback("ReadableStream pull terminated".into())
            })?;
            scope.reset();
            error(scope, state, id, reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(source: &str) {
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
    fn errors_reject_pending_future_reads_and_closed_with_original_identity() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            const reason={failure:1}; let c;
            const s=new ReadableStream({start(x){c=x}});const r=s.getReader();
            check(r.closed===r.closed,'closed identity');
            const first=r.read(),second=r.read();c.error(reason);c.error({wrong:true});
            for(const p of [first,second,r.read(),r.closed,r.cancel()]) {
                let caught;try{await p;throw Error('fulfilled')}catch(e){caught=e}check(caught===reason,'stored error identity');
            }
            r.releaseLock();const next=s.getReader();let caught;try{await next.read()}catch(e){caught=e}check(caught===reason,'new reader error');
            return 'ok';
        })()"#);
    }

    #[test]
    fn closed_waits_for_drain_and_release_rejects_only_the_old_reader() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let c;
            const s=new ReadableStream({start(x){c=x}});const r=s.getReader();let closed=false;
            r.closed.then(()=>closed=true,()=>{});await Promise.resolve();check(!closed,'premature closed');
            c.enqueue(1);c.enqueue(2);c.close();await Promise.resolve();check(!closed,'queued close');
            check((await r.read()).value===1,'first');check(!closed,'queue not drained');
            check((await r.read()).value===2,'second');await r.closed;check(closed,'close after drain');
            check((await r.read()).done,'done');r.releaseLock();check(!s.locked,'unlocked');
            for(const p of [r.read(),r.closed,r.cancel()]){let e;try{await p}catch(x){e=x}check(e instanceof TypeError,'released reader rejection')}
            const next=s.getReader();check((await next.read()).done,'new reader');return 'ok';
        })()"#);
    }

    #[test]
    fn cancellation_discards_queue_and_release_preserves_the_stream() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let c,cancelled;
            const s=new ReadableStream({start(x){c=x},cancel(reason){cancelled=reason}});
            const r=s.getReader();const pending=r.read();r.closed.catch(()=>{});r.releaseLock();
            try{await pending;throw Error('read fulfilled')}catch(e){check(e instanceof TypeError,'pending release')}
            c.enqueue(7);const next=s.getReader();check((await next.read()).value===7,'retained stream');
            c.enqueue(8);const reason={cancel:1};await next.cancel(reason);check(cancelled===reason,'cancel reason');
            check((await next.read()).done,'queue discarded');await next.closed;return 'ok';
        })()"#);
    }
    #[test]
    fn close_requested_can_be_cancelled_or_errored_before_drain() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            let c,cancels=0;const s=new ReadableStream({start(x){c=x},cancel(){cancels++}});
            c.enqueue(1);c.close();const r=s.getReader();await r.cancel();await r.closed;
            check((await r.read()).done,'cancel drains closed queue');check(cancels===1,'cancel called');
            let d;const t=new ReadableStream({start(x){d=x}});d.enqueue(2);d.close();const q=t.getReader();
            const reason={x:1};d.error(reason);
            for(const p of [q.read(),q.closed]){let caught;try{await p}catch(e){caught=e}check(caught===reason,'error discards closed queue')}
            return 'ok';
        })()"#);
    }

    #[test]
    fn asynchronous_start_and_pull_serialize_demand_and_preserve_source_receiver() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            let finishStart,finishPull,calls=0,active=0,max=0;
            const source={start(c){check(this===source,'start receiver');return new Promise(r=>finishStart=r)},
                pull(c){check(this===source,'pull receiver');calls++;active++;max=Math.max(max,active);
                    c.enqueue(calls);return new Promise(r=>finishPull=()=>{active--;r()})}};
            // The source is initialized before its synchronous start callback runs.
            const s=new ReadableStream(source);const r=s.getReader();const a=r.read(),b=r.read();
            await Promise.resolve();check(calls===0,'start pending');finishStart();
            check((await a).value===1,'first chunk');check(calls===1,'pull pending');
            finishPull();check((await b).value===2,'second chunk');check(max===1,'serialized pulls');
            await r.cancel();finishPull();return 'ok';
        })()"#);
    }

    #[test]
    fn start_rejection_and_pull_throw_or_rejection_error_the_stream() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};const reason={failure:1};
            for(const source of [
                {start(){return Promise.reject(reason)}},
                {pull(){throw reason}},
                {pull(){return {then(resolve,reject){reject(reason)}}}}
            ]){
                const r=new ReadableStream(source).getReader();
                for(const p of [r.read(),r.closed]){let e;try{await p}catch(x){e=x}check(e===reason,'original failure')}
                let e;try{await r.read()}catch(x){e=x}check(e===reason,'future read');
            }
            return 'ok';
        })()"#);
    }
    #[test]
    fn cancellation_awaits_thenables_discards_values_and_preserves_failures() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            const reason={cancel:1};let finish;
            const source={cancel(value){check(this===source,'cancel receiver');check(value===reason,'cancel reason');return {then(resolve){finish=resolve}}}};
            const r=new ReadableStream(source).getReader();let settled=false;
            const cancellation=r.cancel(reason).then(value=>{check(value===undefined,'cancel result');settled=true});
            await r.closed;await Promise.resolve();check(!settled,'await cancellation');finish(42);await cancellation;
            for(const cancel of [()=>{throw reason},()=>Promise.reject(reason)]){
                const reader=new ReadableStream({cancel}).getReader();let error;
                try{await reader.cancel()}catch(e){error=e}check(error===reason,'cancel failure');
                await reader.closed;check((await reader.read()).done,'stream stays closed');
            }
            return 'ok';
        })()"#);
    }
    #[test]
    fn synchronous_start_failure_preserves_exception_identity() {
        run(r#"(async()=>{
            const reason={failure:1};let caught;
            try{new ReadableStream({start(){throw reason}})}catch(e){caught=e}
            if(caught!==reason)throw Error('start exception identity');return 'ok';
        })()"#);
    }
}
