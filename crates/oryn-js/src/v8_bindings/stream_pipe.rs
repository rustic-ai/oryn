//! Native sequential pipe algorithm. No author Promise/reader/writer methods
//! or timer globals participate in dispatch, shutdown or lock release.
use super::*;
#[derive(Default, Clone, Copy)]
pub(super) struct Options {
    pub(super) prevent_close: bool,
    pub(super) prevent_abort: bool,
    pub(super) prevent_cancel: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    Complete,
    Source,
    Destination,
    Signal,
}
pub(super) struct Pipe {
    pub(super) reader: u64,
    pub(super) destination: std::rc::Weak<V8BindingState>,
    pub(super) destination_wrapper: stream_graph::Owned<v8::Object>,
    pub(super) writer: u64,
    pub(super) writer_wrapper: stream_graph::Owned<v8::Object>,
    pub(super) resolver: stream_graph::Owned<v8::PromiseResolver>,
    pub(super) signal: Option<stream_graph::Owned<v8::Object>>,
    signal_owner: Option<std::rc::Weak<V8BindingState>>,
    signal_id: Option<u64>,
    signal_listener: Option<u64>,
    pub(super) reason: Option<stream_graph::Owned<v8::Value>>,
    options: Options,
    stop: Option<Stop>,
    pub(super) active: bool,
    writing: bool,
    waiting: bool,
    actions: usize,
    action_failed: bool,
    action_epoch: u64,
}
fn error(message: &str) -> V8BindingError {
    V8BindingError::Message(format!("TypeError: {message}"))
}
fn field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("pipe option"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn pipe(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Pipe>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::StreamPipe(pipe) => Ok(pipe.clone()),
        _ => Err(error("pipe has retired")),
    }
}
fn destination(pipe: &Pipe) -> Result<Rc<V8BindingState>, V8BindingError> {
    pipe.destination
        .upgrade()
        .filter(|state| stream_graph::algorithms_available(&state))
        .ok_or_else(|| error("pipe destination has retired"))
}
fn observe(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    promise: v8::Local<v8::Promise>,
    kind: u32,
) -> Result<(), V8BindingError> {
    let target = new_platform_object(scope, state, "EventTarget", id)?;
    let mut callbacks = Vec::new();
    for rejected in [false, true] {
        let kind = v8::Integer::new_from_unsigned(scope, kind);
        let rejected = v8::Boolean::new(scope, rejected);
        let epoch = v8::BigInt::new_from_u64(scope, pipe(state, id)?.borrow().action_epoch);
        let data = v8::Array::new_with_elements(
            scope,
            &[target.into(), kind.into(), rejected.into(), epoch.into()],
        );
        callbacks.push(
            v8::Function::builder(reaction)
                .data(data.into())
                .build(scope)
                .ok_or(V8BindingError::Allocation("pipe reaction"))?,
        );
    }
    promise
        .then2(scope, callbacks[0], callbacks[1])
        .ok_or(V8BindingError::Allocation("pipe promise observation"))?;
    Ok(())
}
fn reaction(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let data = v8::Local::<v8::Array>::try_from(args.data())
            .map_err(|_| error("pipe reaction data"))?;
        let target = data
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| error("pipe reaction target"))?;
        let state = binding_state_for_receiver(scope, target)
            .ok_or_else(|| error("pipe reaction owner"))?;
        if !stream_graph::algorithms_available(&state) {
            return Ok(());
        }
        let id = stateful_id(scope, target)?;
        let kind = data
            .get_index(scope, 1)
            .and_then(|value| value.uint32_value(scope))
            .ok_or_else(|| error("pipe reaction kind"))?;
        let rejected = data
            .get_index(scope, 2)
            .ok_or_else(|| error("pipe reaction status"))?
            .boolean_value(scope);
        let epoch = data
            .get_index(scope, 3)
            .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
            .map(|value| value.u64_value().0)
            .ok_or_else(|| error("pipe action epoch"))?;
        if kind == 7 && pipe(&state, id)?.borrow().action_epoch != epoch {
            return Ok(());
        }
        let value = v8::Local::new(scope, args.get(0));
        handle(scope, &state, id, kind, rejected, value)?;
        stream_graph::refresh(scope, &state)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
fn handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    kind: u32,
    rejected: bool,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let record = pipe(state, id)?;
    if !record.borrow().active {
        return Ok(());
    }
    if kind == 7 {
        {
            let mut pipe = record.borrow_mut();
            if rejected && !pipe.action_failed {
                pipe.action_failed = true;
                pipe.reason = Some(stream_graph::Owned::new(scope, value));
            }
            pipe.actions = pipe.actions.saturating_sub(1);
        }
        if record.borrow().actions == 0 {
            return finish(scope, state, id);
        }
        return Ok(());
    }
    if kind == 4 {
        if rejected {
            return shutdown(scope, state, id, Stop::Source, Some(value));
        }
        return Ok(());
    }
    if kind == 5 {
        if record.borrow().stop.is_some() {
            return Ok(());
        }
        let reason = if rejected {
            value
        } else {
            readable_streams::type_error(scope, "destination closed before pipe completed")?
        };
        return shutdown(scope, state, id, Stop::Destination, Some(reason));
    }
    if kind == 6 {
        return shutdown(scope, state, id, Stop::Signal, Some(value));
    }
    if kind == 3 {
        record.borrow_mut().writing = false;
        if record.borrow().stop.is_some() {
            return run_shutdown(scope, state, id);
        }
        if rejected {
            return shutdown(scope, state, id, Stop::Destination, Some(value));
        }
        return ready(scope, state, id);
    }
    record.borrow_mut().waiting = false;
    if record.borrow().stop.is_some() {
        return Ok(());
    }
    if rejected {
        return shutdown(
            scope,
            state,
            id,
            if kind == 1 {
                Stop::Destination
            } else {
                Stop::Source
            },
            Some(value),
        );
    }
    if kind == 1 {
        let reader = record.borrow().reader;
        record.borrow_mut().waiting = true;
        let value = read_native_stream(scope, state, reader)?
            .ok_or_else(|| error("pipe read returned no promise"))?;
        let promise = v8::Local::<v8::Promise>::try_from(value)
            .map_err(|_| error("pipe read returned non-promise"))?;
        return observe(scope, state, id, promise, 2);
    }
    let result = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("pipe read result"))?;
    let done = field(scope, result, "done")?.boolean_value(scope);
    if done {
        return shutdown(scope, state, id, Stop::Complete, None);
    }
    let chunk = field(scope, result, "value")?;
    let (destination, writer) = {
        let pipe = record.borrow();
        (destination(&pipe)?, pipe.writer)
    };
    let stream = writer_stream_id(scope, &destination, writer)?;
    record.borrow_mut().writing = true;
    let promise = writable_write(scope, &destination, stream, chunk)?;
    observe(scope, state, id, promise, 3)
}
fn ready(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let record = pipe(state, id)?;
    let (destination, writer) = {
        let mut pipe = record.borrow_mut();
        if !pipe.active || pipe.stop.is_some() || pipe.waiting || pipe.writing {
            return Ok(());
        }
        pipe.waiting = true;
        (destination(&pipe)?, pipe.writer)
    };
    let value = writable_writers::get(scope, &destination, writer, "ready")?;
    let promise =
        v8::Local::<v8::Promise>::try_from(value).map_err(|_| error("writer ready promise"))?;
    observe(scope, state, id, promise, 1)
}
fn shutdown(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    stop: Stop,
    reason: Option<v8::Local<v8::Value>>,
) -> Result<(), V8BindingError> {
    let record = pipe(state, id)?;
    {
        let mut pipe = record.borrow_mut();
        if !pipe.active || pipe.stop.is_some() {
            return Ok(());
        }
        pipe.stop = Some(stop);
        pipe.reason = reason.map(|value| stream_graph::Owned::new(scope, value));
        if pipe.writing {
            return Ok(());
        }
    }
    run_shutdown(scope, state, id)
}
fn run_shutdown(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let record = pipe(state, id)?;
    let (destination, writer, reader, stop, options, reason) = {
        let pipe = record.borrow();
        if !pipe.active || pipe.writing || pipe.actions != 0 {
            return Ok(());
        }
        (
            pipe.destination
                .upgrade()
                .filter(|state| stream_graph::algorithms_available(&state)),
            pipe.writer,
            pipe.reader,
            pipe.stop.ok_or_else(|| error("pipe shutdown state"))?,
            pipe.options,
            pipe.reason.clone(),
        )
    };
    let reason = reason
        .map(|value| stream_graph::local(scope, value))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let mut actions = Vec::new();
    let target = destination
        .as_ref()
        .map(|destination| writer_stream_id(scope, destination, writer))
        .transpose()?;
    if stop == Stop::Complete && !options.prevent_close {
        if let (Some(destination), Some(target)) = (&destination, target) {
            actions.push(writable_close(scope, destination, target)?);
        } else {
            let reason = dom_exception_value(scope, "AbortError")?;
            record.borrow_mut().reason = Some(stream_graph::Owned::new(scope, reason));
        }
    }
    if matches!(stop, Stop::Source | Stop::Signal) && !options.prevent_abort {
        if let (Some(destination), Some(target)) = (&destination, target) {
            actions.push(writable_streams::abort(scope, destination, target, reason)?);
        }
    }
    if matches!(stop, Stop::Destination | Stop::Signal) && !options.prevent_cancel {
        actions.push(cancel_source(scope, state, reader, reason)?);
    }
    {
        let mut pipe = record.borrow_mut();
        pipe.actions = actions.len();
        pipe.action_epoch = pipe
            .action_epoch
            .checked_add(1)
            .ok_or_else(|| error("pipe action identity exhausted"))?;
    }
    if actions.is_empty() {
        return finish(scope, state, id);
    }
    for action in actions {
        observe(scope, state, id, action, 7)?;
    }
    Ok(())
}
fn finish(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let record = pipe(state, id)?;
    let (destination, reader, writer, resolver, reason, listener) = {
        let mut pipe = record.borrow_mut();
        if !pipe.active {
            return Ok(());
        }
        pipe.active = false;
        (
            pipe.destination.upgrade(),
            pipe.reader,
            pipe.writer,
            pipe.resolver.clone(),
            pipe.reason.take(),
            pipe.signal_owner
                .as_ref()
                .and_then(std::rc::Weak::upgrade)
                .zip(pipe.signal_id)
                .zip(pipe.signal_listener.take()),
        )
    };
    if let Some(((owner, signal), registration)) = listener {
        if let Some(listeners) = owner
            .platform_listeners
            .borrow_mut()
            .get_mut(&(signal, "abort".into()))
        {
            listeners.retain(|listener| listener.registration_id != registration);
        }
        platform_listener_graph::refresh(scope, &owner, signal)?;
    }
    if let Some(destination) = destination {
        destination
            .pipe_dependents
            .borrow_mut()
            .retain(|(source, pipe)| {
                !(*pipe == id
                    && source
                        .upgrade()
                        .is_some_and(|source| Rc::ptr_eq(&source, state)))
            });
        if !stream_graph::algorithms_available(&destination) {
            let stream = match &mut *platform_object_mut(&destination, writer)? {
                PlatformObjectRecord::WritableWriter {
                    stream_id,
                    released,
                    ..
                } => {
                    *released = true;
                    Some(*stream_id)
                }
                _ => None,
            };
            if let Some(stream) = stream {
                if let PlatformObjectRecord::WritableStream(record) =
                    &mut *platform_object_mut(&destination, stream)?
                {
                    record.locked = false;
                    record.writer_id = None;
                }
            }
        } else {
            writable_writers::release(scope, &destination, writer)?;
            stream_graph::refresh(scope, &destination)?;
        }
    }
    readable_streams::release(scope, state, reader)?;
    let resolver = stream_graph::local(scope, resolver);
    if let Some(reason) = reason {
        let reason = stream_graph::local(scope, reason);
        resolver.reject(scope, reason);
    } else {
        let undefined = v8::undefined(scope).into();
        resolver.resolve(scope, undefined);
    }
    Ok(())
}
pub(super) fn cancel_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    reader: u64,
    reason: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let stream = match &*platform_object(state, reader)? {
        PlatformObjectRecord::ReadableReader { stream_id, .. } => *stream_id,
        _ => return Err(error("pipe reader retired")),
    };
    if let Some(error) = readable_streams::stored_error(scope, state, stream)? {
        return readable_streams::reject(scope, error);
    }
    let (controller, callback, host, upstream, closed) = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Err(error("pipe stream retired"));
        };
        record.disturbed = true;
        record.queue.clear();
        (
            record.controller_id,
            record.cancel.clone(),
            record.host_stream_id,
            record.tee_source,
            record.closed,
        )
    };
    if closed {
        let undefined = v8::undefined(scope).into();
        return resolved_promise(scope, undefined);
    }
    readable_close_default(scope, state, controller)?;
    byte_streams::cancel_pending(scope, state, stream)?;
    transform_streams::cancel_readable(scope, state, stream, reason)?;
    if let Some((source, index)) = upstream {
        return stream_tee::cancel(scope, state, source, index, reason);
    }
    if let Some(host) = host {
        submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id: host },
            PendingPlatformOperation::WebSocketCommand,
        )?;
    }
    readable_streams::cancel(scope, state, stream, callback, reason)
}

fn abort_event(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let result = (|| -> Result<(), V8BindingError> {
        let target = v8::Local::<v8::Object>::try_from(args.data())
            .map_err(|_| error("pipe abort target"))?;
        let state =
            binding_state_for_receiver(scope, target).ok_or_else(|| error("pipe abort owner"))?;
        if !stream_graph::algorithms_available(&state) {
            return Ok(());
        }
        let id = stateful_id(scope, target)?;
        let record = pipe(&state, id)?;
        let (owner, signal) = {
            let pipe = record.borrow();
            (
                pipe.signal_owner
                    .as_ref()
                    .and_then(std::rc::Weak::upgrade)
                    .ok_or_else(|| error("pipe signal retired"))?,
                pipe.signal_id.ok_or_else(|| error("pipe signal missing"))?,
            )
        };
        let reason = match &*platform_object(&owner, signal)? {
            PlatformObjectRecord::AbortSignal(signal) => signal
                .reason
                .as_ref()
                .map(|reason| reason.value(scope))
                .unwrap_or_else(|| v8::undefined(scope).into()),
            _ => return Err(error("pipe signal missing")),
        };
        shutdown(scope, &state, id, Stop::Signal, Some(reason))?;
        stream_graph::refresh(scope, &state)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

pub(super) fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    source: u64,
    target: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let target = v8::Local::<v8::Object>::try_from(target)
        .map_err(|_| error("pipe destination must be a writable stream"))?;
    let destination =
        binding_state_for_receiver(scope, target).ok_or_else(|| error("pipe destination owner"))?;
    let target_id = stateful_id(scope, target)?;
    if !matches!(
        &*platform_object(&destination, target_id)?,
        PlatformObjectRecord::WritableStream(_)
    ) {
        return Err(error("pipe destination must be a writable stream"));
    }
    let mut configuration = Options::default();
    let mut signal = None;
    let mut signal_owner = None;
    let mut signal_id = None;
    if !options.is_null_or_undefined() {
        let options = options
            .to_object(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        configuration.prevent_abort = field(scope, options, "preventAbort")?.boolean_value(scope);
        configuration.prevent_cancel = field(scope, options, "preventCancel")?.boolean_value(scope);
        configuration.prevent_close = field(scope, options, "preventClose")?.boolean_value(scope);
        let value = field(scope, options, "signal")?;
        if !value.is_undefined() {
            let object = v8::Local::<v8::Object>::try_from(value)
                .map_err(|_| error("pipe signal must be AbortSignal"))?;
            let owner = binding_state_for_receiver(scope, object)
                .ok_or_else(|| error("pipe signal owner"))?;
            let id = stateful_id(scope, object)?;
            if !matches!(
                &*platform_object(&owner, id)?,
                PlatformObjectRecord::AbortSignal(_)
            ) {
                return Err(error("pipe signal must be AbortSignal"));
            }
            signal = Some(stream_graph::Owned::new(scope, object));
            signal_owner = Some(Rc::downgrade(&owner));
            signal_id = Some(id);
        }
    }
    if !stream_graph::algorithms_available(&state)
        || !stream_graph::algorithms_available(&destination)
    {
        return Err(error("pipe environment retired"));
    }
    let reader = readable_streams::acquire(scope, state, source)?;
    let writer = match writable_writers::acquire(scope, &destination, target_id) {
        Ok(writer) => writer,
        Err(error) => {
            readable_streams::release(scope, state, reader)?;
            return Err(error);
        }
    };
    let _reader_wrapper = new_platform_object(scope, state, "ReadableStreamDefaultReader", reader)?;
    let writer_wrapper =
        new_platform_object(scope, &destination, "WritableStreamDefaultWriter", writer)?;
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("pipe result"))?;
    let promise = resolver.get_promise(scope);
    let record = Rc::new(RefCell::new(Pipe {
        reader,
        destination: Rc::downgrade(&destination),
        destination_wrapper: stream_graph::Owned::new(scope, target),
        writer,
        writer_wrapper: stream_graph::Owned::new(scope, writer_wrapper),
        resolver: stream_graph::Owned::new(scope, resolver),
        signal,
        signal_owner,
        signal_id,
        signal_listener: None,
        reason: None,
        options: configuration,
        stop: None,
        active: true,
        writing: false,
        waiting: false,
        actions: 0,
        action_failed: false,
        action_epoch: 0,
    }));
    let id = register_platform_object(state, PlatformObjectRecord::StreamPipe(record.clone()));
    let hidden = new_platform_object(scope, state, "EventTarget", id)?;
    destination
        .pipe_dependents
        .borrow_mut()
        .retain(|(source, _)| {
            source
                .upgrade()
                .is_some_and(|source| stream_graph::algorithms_available(&source))
        });
    destination
        .pipe_dependents
        .borrow_mut()
        .push((Rc::downgrade(state), id));
    let signal_owner = {
        record
            .borrow()
            .signal_owner
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
    };
    if let Some(owner) = signal_owner {
        let signal = record.borrow().signal_id.unwrap();
        let aborted = match &*platform_object(&owner, signal)? {
            PlatformObjectRecord::AbortSignal(signal) => signal.aborted,
            _ => false,
        };
        if aborted {
            let reason = match &*platform_object(&owner, signal)? {
                PlatformObjectRecord::AbortSignal(signal) => signal
                    .reason
                    .as_ref()
                    .map(|reason| reason.value(scope))
                    .unwrap_or_else(|| v8::undefined(scope).into()),
                _ => unreachable!(),
            };
            shutdown(scope, state, id, Stop::Signal, Some(reason))?;
            stream_graph::refresh(scope, state)?;
            return Ok(promise);
        }
        let callback = v8::Function::builder(abort_event)
            .data(hidden.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("pipe abort callback"))?;
        let registration = owner.next_event_listener_id.get();
        owner.next_event_listener_id.set(
            registration
                .checked_add(1)
                .ok_or_else(|| error("listener identity exhausted"))?,
        );
        owner
            .platform_listeners
            .borrow_mut()
            .entry((signal, "abort".into()))
            .or_default()
            .push(PlatformEventListener {
                registration_id: registration,
                global_handler: false,
                callback: platform_listener_graph::Callback::new(scope, callback.into()),
                capture: false,
                once: true,
                passive: false,
                signal: None,
            });
        record.borrow_mut().signal_listener = Some(registration);
        platform_listener_graph::refresh(scope, &owner, signal)?;
    }
    let closed = match &*platform_object(state, reader)? {
        PlatformObjectRecord::ReadableReader { closed, .. } => {
            stream_graph::local(scope, closed).get_promise(scope)
        }
        _ => return Err(error("pipe reader missing")),
    };
    observe(scope, state, id, closed, 4)?;
    let closed = writable_writers::get(scope, &destination, writer, "closed")?;
    observe(
        scope,
        state,
        id,
        v8::Local::<v8::Promise>::try_from(closed).map_err(|_| error("writer closed promise"))?,
        5,
    )?;
    ready(scope, state, id)?;
    stream_graph::refresh(scope, state)?;
    stream_graph::refresh(scope, &destination)?;
    Ok(promise)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> String {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    }
    macro_rules! fixture{($scope:ident,$state:ident)=>{super::super::tests::initialize_v8();let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));let mut isolate=v8::Isolate::new(Default::default());v8::scope!(let $scope,&mut isolate);let context=v8::Context::new($scope,Default::default());let $scope=&mut v8::ContextScope::new($scope,context);let $state=install_generated_bindings($scope,context,dom,RealmId(0),1).unwrap();}}
    #[test]
    fn native_pipe_is_sequential_and_releases_both_locks_after_close() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.log=[];globalThis.result='pending';globalThis.failure='';
          globalThis.source=new ReadableStream({start(c){c.enqueue(1);c.enqueue(2);c.close()}});
          globalThis.target=new WritableStream({write(value){log.push('write'+value);if(value===1)return new Promise(resolve=>globalThis.releaseWrite=resolve)},close(){log.push('close')}});
          source.pipeTo(target).then(()=>result='ok',e=>failure=String(e));if(!source.locked||!target.locked)throw Error('pipe locks');true"#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "log.join(',')"), "write1");
        eval(scope, "releaseWrite();true");
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "failure||result"), "ok");
        assert_eq!(eval(scope, "log.join(',')"), "write1,write2,close");
        assert_eq!(eval(scope, "String(source.locked||target.locked)"), "false");
        let _ = state;
    }
    #[test]
    fn prevent_close_and_prevent_cancel_preserve_their_native_endpoints() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{let closed=0;const source=new ReadableStream({start(c){c.close()}});const target=new WritableStream({close(){closed++}});await source.pipeTo(target,{preventClose:true});if(closed||source.locked||target.locked)throw Error('preventClose');await target.getWriter().write('still writable');
          const reason={failure:1};let cancelled=0;const remaining=new ReadableStream({start(c){c.enqueue(3)},cancel(){cancelled++}});const failed=new WritableStream({write(){throw reason}});let caught;try{await remaining.pipeTo(failed,{preventCancel:true})}catch(e){caught=e}if(caught!==reason||cancelled||remaining.locked||failed.locked)throw Error('preventCancel');result='ok';})().catch(e=>failure=String(e));true"#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn abort_during_write_waits_for_write_then_propagates_original_reason() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.log=[];globalThis.result='pending';globalThis.failure='';globalThis.reason={abort:1};globalThis.abort=new AbortController();
          globalThis.source=new ReadableStream({start(c){c.enqueue(9)},cancel(value){if(value!==reason)throw Error('source reason');log.push('cancel')}});
          globalThis.target=new WritableStream({write(){log.push('write');return new Promise(resolve=>globalThis.releaseWrite=resolve)},abort(value){if(value!==reason)throw Error('destination reason');log.push('abort')}});
          source.pipeTo(target,{signal:abort.signal}).then(()=>failure='unexpected success',e=>result=e===reason?'ok':'wrong reason');true"#,
        );
        scope.perform_microtask_checkpoint();
        eval(scope, "abort.abort(reason);true");
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "log.join(',')"), "write");
        eval(scope, "releaseWrite();true");
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "failure||result"), "ok");
        assert_eq!(
            eval(
                scope,
                "String(log.includes('cancel')&&log.includes('abort')&&!source.locked&&!target.locked)"
            ),
            "true"
        );
        let _ = state;
    }
    #[test]
    fn option_getters_reject_with_original_exception_without_acquiring_locks() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';const marker={};globalThis.source=new ReadableStream();globalThis.target=new WritableStream();
          source.pipeTo(target,{get preventAbort(){throw marker}}).then(()=>result='unexpected',e=>result=e===marker?'ok':'wrong exception');true"#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "result"), "ok");
        assert_eq!(eval(scope, "String(source.locked||target.locked)"), "false");
        let _ = state;
    }
    #[test]
    fn terminated_worker_destination_rejects_pipe_and_releases_source_without_retired_callbacks() {
        fixture!(scope, state);
        let destination;
        {
            v8::scope!(let inner,scope);
            let context = v8::Context::new(inner, Default::default());
            let inner = &mut v8::ContextScope::new(inner, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let document = dom.borrow().document();
            destination = install_generated_bindings_with_root_for_global(
                inner,
                context,
                dom,
                RealmId(99),
                1,
                document,
                BindingGlobal::Worker,
            )
            .unwrap();
            eval(
                inner,
                "globalThis.target=new WritableStream({start(){return new Promise(()=>{})},abort(){throw Error('retired abort callback')}});true",
            );
            let name = v8::String::new(inner, "target").unwrap();
            let target = context.global(inner).get(inner, name.into()).unwrap();
            let source_global = state.global_object.to_local(inner).unwrap();
            source_global.set(inner, name.into(), target).unwrap();
        }
        eval(
            scope,
            "globalThis.cancelled=false;globalThis.result='pending';globalThis.source=new ReadableStream({cancel(){cancelled=true}});source.pipeTo(target).catch(e=>result=e.name);true",
        );
        scope.perform_microtask_checkpoint();
        destination.retire_execution();
        state.drain_platform_completions(scope, 256).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "result"), "AbortError");
        assert_eq!(eval(scope, "String(cancelled&&!source.locked)"), "true");
    }
    #[test]
    fn detached_window_destination_keeps_reachable_pure_pipe_algorithms() {
        fixture!(scope, state);
        let destination;
        {
            v8::scope!(let inner,scope);
            let context = v8::Context::new(inner, Default::default());
            let inner = &mut v8::ContextScope::new(inner, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            destination = install_generated_bindings(inner, context, dom, RealmId(98), 1).unwrap();
            eval(
                inner,
                "globalThis.log=[];globalThis.target=new WritableStream({write(value){log.push(value)},close(){log.push('closed')}});globalThis.inspect=()=>log.join(',');true",
            );
            for property in ["target", "inspect"] {
                let name = v8::String::new(inner, property).unwrap();
                let value = context.global(inner).get(inner, name.into()).unwrap();
                state
                    .global_object
                    .to_local(inner)
                    .unwrap()
                    .set(inner, name.into(), value)
                    .unwrap();
            }
        }
        destination.retire_execution();
        eval(
            scope,
            "globalThis.result='pending';globalThis.source=new ReadableStream({start(c){c.enqueue(7);c.close()}});source.pipeTo(target).then(()=>result='ok',e=>result=String(e));true",
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "result"), "ok");
        assert_eq!(eval(scope, "inspect()"), "7,closed");
        assert_eq!(eval(scope, "String(source.locked||target.locked)"), "false");
    }
    #[test]
    fn prevent_abort_preserves_destination_on_source_failure() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{const reason={source:1};let aborted=0;const source=new ReadableStream({start(c){c.error(reason)}});const target=new WritableStream({abort(){aborted++}});let caught;try{await source.pipeTo(target,{preventAbort:true})}catch(e){caught=e}if(caught!==reason||aborted||source.locked||target.locked)throw Error('preventAbort');await target.getWriter().write(7);result='ok';})().catch(e=>failure=String(e));true"#,
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
}

pub(super) fn retire_destination(state: &V8BindingState) {
    if state.binding_global == BindingGlobal::Window {
        return;
    }
    for (source, pipe_id) in std::mem::take(&mut *state.pipe_dependents.borrow_mut()) {
        if let Some(source) = source
            .upgrade()
            .filter(|source| stream_graph::algorithms_available(&source))
        {
            source
                .deferred_platform_tasks
                .borrow_mut()
                .push_back(DeferredPlatformTask::PipeDestinationRetired { pipe_id });
        }
    }
}
pub(super) fn destination_retired(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let Ok(record) = pipe(state, id) else {
        return Ok(());
    };
    {
        let mut pipe = record.borrow_mut();
        if !pipe.active {
            return Ok(());
        }
        pipe.writing = false;
        pipe.actions = 0;
        pipe.stop = None;
        pipe.action_epoch = pipe
            .action_epoch
            .checked_add(1)
            .ok_or_else(|| error("pipe action identity exhausted"))?;
    }
    let reason = dom_exception_value(scope, "AbortError")?;
    shutdown(scope, state, id, Stop::Destination, Some(reason))
}
