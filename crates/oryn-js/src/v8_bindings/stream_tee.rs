use super::*;

pub(super) struct Tee {
    pub(super) branches: [u64; 2],
    clone_bytes: bool,
    pumping: bool,
    scheduled: bool,
    cancel_started: bool,
    pub(super) cancelled: [Option<stream_graph::Owned<v8::Value>>; 2],
    pub(super) cancellation: stream_graph::Owned<v8::PromiseResolver>,
}

pub(super) fn create(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: u64,
    clone_bytes: bool,
) -> Result<[u64; 2], V8BindingError> {
    if let PlatformObjectRecord::ReadableStream(stream) = &*platform_object(state, source)?
        && stream.locked
    {
        return Err(V8BindingError::Unsupported {
            interface: "ReadableStream".into(),
            member: "stream is locked".into(),
        });
    }
    let cancellation =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("tee cancellation"))?;
    let byte_source = byte_streams::enabled(state, source);
    let owning = owning_streams::enabled(state, source);
    let branches = [
        new_byte_stream_record(scope, state, vec![])?,
        new_byte_stream_record(scope, state, vec![])?,
    ];
    for (index, id) in branches.iter().enumerate() {
        if let PlatformObjectRecord::ReadableStream(stream) = &mut *platform_object_mut(state, *id)?
        {
            stream.owning = owning;
            stream.bytes = byte_source.then(Default::default);
            stream.queue.clear();
            stream.closed = false;
            stream.tee_source = Some((source, index));
        }
    }
    if let PlatformObjectRecord::ReadableStream(stream) = &mut *platform_object_mut(state, source)?
    {
        stream.locked = true;
        stream.tee = Some(Rc::new(RefCell::new(Tee {
            branches,
            clone_bytes: clone_bytes || byte_source,
            pumping: false,
            scheduled: false,
            cancel_started: false,
            cancelled: [None, None],
            cancellation: stream_graph::Owned::new(scope, cancellation),
        })));
    }
    pump(scope, state, source)?;
    Ok(branches)
}

pub(super) fn pump(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: u64,
) -> Result<(), V8BindingError> {
    if !stream_graph::algorithms_available(&state) {
        return Ok(());
    }
    let (tee, controller) = match &*platform_object(state, source)? {
        PlatformObjectRecord::ReadableStream(stream) => (stream.tee.clone(), stream.controller_id),
        _ => return Ok(()),
    };
    let Some(tee) = tee else {
        return Ok(());
    };
    if tee.borrow().scheduled || tee.borrow().pumping || tee.borrow().cancel_started {
        return Ok(());
    }
    let data = new_platform_object(scope, state, "ReadableStreamDefaultController", controller)?;
    let callback = v8::Function::builder(deliver_microtask)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("tee delivery microtask"))?;
    tee.borrow_mut().scheduled = true;
    scope.enqueue_microtask(callback);
    Ok(())
}

fn deliver_microtask(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let result = (|| -> Result<(), V8BindingError> {
        let controller = v8::Local::<v8::Object>::try_from(args.data())
            .map_err(|_| V8BindingError::Allocation("tee controller"))?;
        let state = binding_state_for_receiver(scope, controller)
            .ok_or(V8BindingError::Allocation("tee state"))?;
        let controller_id = stateful_id(scope, controller)?;
        let source = match &*platform_object(&state, controller_id)? {
            PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
            _ => return Err(V8BindingError::Allocation("tee source")),
        };
        if let PlatformObjectRecord::ReadableStream(stream) = &*platform_object(&state, source)?
            && let Some(tee) = &stream.tee
        {
            tee.borrow_mut().scheduled = false;
        }
        if !stream_graph::algorithms_available(&state) {
            return Ok(());
        }
        deliver(scope, &state, source)?;
        stream_graph::refresh(scope, &state)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: u64,
) -> Result<(), V8BindingError> {
    let tee = match &*platform_object(state, source)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.tee.clone(),
        _ => None,
    };
    let Some(tee) = tee else {
        return Ok(());
    };
    if tee.borrow().pumping || tee.borrow().cancel_started {
        return Ok(());
    }
    tee.borrow_mut().pumping = true;
    let result: Result<(), V8BindingError> = (|| {
        let (branches, clone_bytes) = {
            let tee = tee.borrow();
            (tee.branches, tee.clone_bytes)
        };
        loop {
            let mut demand = false;
            for branch in branches {
                if let PlatformObjectRecord::ReadableStream(stream) =
                    &*platform_object(state, branch)?
                {
                    demand |= !stream.closed
                        && (stream.consumer.is_some()
                            || !stream.waiting.is_empty()
                            || (stream.queue.len() as f64) < stream.high_water_mark);
                }
            }
            let (chunk, closed, error) = {
                let mut record = platform_object_mut(state, source)?;
                let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                    unreachable!()
                };
                if demand {
                    stream.disturbed = true;
                }
                let chunk = if demand {
                    stream.queue.pop_front()
                } else {
                    None
                };
                (
                    chunk,
                    stream.closed && stream.queue.is_empty(),
                    stream.error.clone(),
                )
            };
            if let Some(error) = error {
                let error = stream_graph::local(scope, error);
                for branch in branches {
                    readable_streams::error(scope, state, branch, error)?;
                }
                let undefined = v8::undefined(scope).into();
                stream_graph::local(scope, &tee.borrow().cancellation).resolve(scope, undefined);
                break;
            }
            if let Some(chunk) = chunk {
                byte_streams::handle_queue_drain(scope, state, source)?;
                if chunk.is_serialized() {
                    let Some(second) = chunk.clone_serialized(scope) else {
                        let error = dom_exception_value(scope, "DataCloneError")?;
                        readable_streams::error(scope, state, source, error)?;
                        return Ok(());
                    };
                    for (branch, chunk) in branches.into_iter().zip([chunk, second]) {
                        let controller = match &*platform_object(state, branch)? {
                            PlatformObjectRecord::ReadableStream(stream) if !stream.closed => {
                                Some(stream.controller_id)
                            }
                            _ => None,
                        };
                        if let Some(controller) = controller {
                            readable_enqueue_chunk(scope, state, controller, chunk)?;
                        }
                    }
                    continue;
                }
                let value = stream_graph::local(scope, chunk);
                // Body tee copies byte chunks before exposing either branch.
                let second = if clone_bytes {
                    let view = v8::Local::<v8::Uint8Array>::try_from(value).map_err(|_| {
                        V8BindingError::Message("Response body chunk is not a Uint8Array".into())
                    })?;
                    let mut bytes = vec![0; view.byte_length()];
                    view.copy_contents(&mut bytes);
                    let buffer = array_buffer_from_bytes(scope, &bytes);
                    v8::Uint8Array::new(scope, buffer, 0, bytes.len())
                        .ok_or(V8BindingError::Allocation("tee chunk"))?
                        .into()
                } else {
                    value
                };
                for (index, branch) in branches.iter().enumerate() {
                    let controller = match &*platform_object(state, *branch)? {
                        PlatformObjectRecord::ReadableStream(stream) if !stream.closed => {
                            Some(stream.controller_id)
                        }
                        _ => None,
                    };
                    if let Some(controller) = controller {
                        readable_enqueue(
                            scope,
                            state,
                            controller,
                            if index == 0 { value } else { second },
                        )?;
                    }
                }
                continue;
            }
            if closed {
                for branch in branches {
                    let controller = match &*platform_object(state, branch)? {
                        PlatformObjectRecord::ReadableStream(stream) => stream.controller_id,
                        _ => unreachable!(),
                    };
                    let already_closed = matches!(&*platform_object(state,branch)?,PlatformObjectRecord::ReadableStream(stream) if stream.closed||stream.bytes.as_ref().is_some_and(|bytes|bytes.close_requested));
                    if !already_closed {
                        readable_close(scope, state, controller)?;
                    }
                    if byte_streams::enabled(state, branch) {
                        byte_streams::drain(scope, state, branch)?;
                    }
                }
                let undefined = v8::undefined(scope).into();
                stream_graph::local(scope, &tee.borrow().cancellation).resolve(scope, undefined);
                break;
            }
            if demand {
                byte_streams::prepare_tee_pull(scope, state, source, branches)?;
                let host = {
                    let mut record = platform_object_mut(state, source)?;
                    let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                        unreachable!()
                    };
                    if stream.host_read_pending {
                        None
                    } else {
                        let host = stream.host_stream_id;
                        if host.is_some() {
                            stream.host_read_pending = true;
                        }
                        host
                    }
                };
                if let Some(stream_id) = host {
                    submit_platform_operation(
                        state,
                        PlatformHostRequest::HttpRead {
                            stream_id,
                            max_bytes: 64 * 1024,
                        },
                        PendingPlatformOperation::BodyRead {
                            stream_object_id: source,
                        },
                    )?;
                } else {
                    readable_streams::pull(scope, state, source)?;
                }
                let has_chunk = matches!(&*platform_object(state, source)?, PlatformObjectRecord::ReadableStream(stream)
                    if !stream.queue.is_empty() || stream.closed || stream.error.is_some());
                if has_chunk {
                    continue;
                }
            }
            break;
        }
        Ok(())
    })();
    tee.borrow_mut().pumping = false;
    if let Err(error) = result {
        let reason = readable_streams::type_error(scope, &error.to_string())?;
        readable_streams::error(scope, state, source, reason)?;
        error_branches(scope, state, source, reason)?;
    }
    Ok(())
}

pub(super) fn cancel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    source: u64,
    index: usize,
    reason: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let tee = match &*platform_object(state, source)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.tee.clone().unwrap(),
        _ => unreachable!(),
    };
    let (both, resolver) = {
        let mut tee = tee.borrow_mut();
        tee.cancelled[index] = Some(stream_graph::Owned::new(scope, reason));
        let both = tee.cancelled.iter().all(Option::is_some) && !tee.cancel_started;
        if both {
            tee.cancel_started = true;
        }
        (both, tee.cancellation.clone())
    };
    let resolver = stream_graph::local(scope, resolver);
    let promise = resolver.get_promise(scope);
    if both {
        let reasons = v8::Array::new(scope, 2);
        for (index, reason) in tee.borrow().cancelled.iter().enumerate() {
            let reason = stream_graph::local(scope, reason.as_ref().unwrap());
            reasons.set_index(scope, index as u32, reason);
        }
        let (callback, host, upstream, terminal) = {
            let mut record = platform_object_mut(state, source)?;
            let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                unreachable!()
            };
            let terminal = stream.closed || stream.error.is_some();
            stream.disturbed = true;
            stream.closed = true;
            stream.queue.clear();
            (
                stream.cancel.clone(),
                stream.host_stream_id,
                stream.tee_source,
                terminal,
            )
        };
        if terminal {
            let undefined = v8::undefined(scope).into();
            resolver.resolve(scope, undefined);
            return Ok(promise);
        }
        let cancellation = if let Some((parent, branch)) = upstream {
            cancel(scope, state, parent, branch, reasons.into())?
        } else {
            if let Some(stream_id) = host {
                submit_platform_operation(
                    state,
                    PlatformHostRequest::HttpAbort { stream_id },
                    PendingPlatformOperation::WebSocketCommand,
                )?;
            }
            readable_streams::cancel(scope, state, source, callback, reasons.into())?
        };
        resolver.resolve(scope, cancellation.into());
    }
    Ok(promise)
}

pub(super) fn error_branches(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    source: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let tee = match &*platform_object(state, source)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.tee.clone(),
        _ => None,
    };
    if let Some(tee) = tee {
        let branches = tee.borrow().branches;
        for branch in branches {
            readable_streams::error(scope, state, branch, reason)?;
        }
        if !tee.borrow().cancel_started {
            let undefined = v8::undefined(scope).into();
            stream_graph::local(scope, &tee.borrow().cancellation).resolve(scope, undefined);
        }
    }
    Ok(())
}

pub(super) fn demand(state: &V8BindingState, source: u64) -> Result<Option<bool>, V8BindingError> {
    let tee = match &*platform_object(state, source)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.tee.clone(),
        _ => None,
    };
    let Some(tee) = tee else {
        return Ok(None);
    };
    for branch in tee.borrow().branches {
        if let PlatformObjectRecord::ReadableStream(stream) = &*platform_object(state, branch)? {
            if !stream.closed
                && (stream.consumer.is_some()
                    || !stream.waiting.is_empty()
                    || (stream.queue.len() as f64) < stream.high_water_mark)
            {
                return Ok(Some(true));
            }
        }
    }
    Ok(Some(false))
}
