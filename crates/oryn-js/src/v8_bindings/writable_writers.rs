//! Writer ownership and observable readiness/terminal promises.
use super::*;

pub(super) fn acquire(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<u64, V8BindingError> {
    let ready =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("writer ready"))?;
    let closed =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("writer closed"))?;
    {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if stream.locked {
            return Err(V8BindingError::EventCallback(
                "Writable stream is locked".into(),
            ));
        }
        stream.locked = true;
    }
    let writer = register_platform_object(
        state,
        PlatformObjectRecord::WritableWriter {
            stream_id: id,
            ready: v8::Global::new(scope, ready),
            closed: v8::Global::new(scope, closed),
            released: false,
        },
    );
    if let PlatformObjectRecord::WritableStream(stream) = &mut *platform_object_mut(state, id)? {
        stream.writer_id = Some(writer);
    }
    refresh(scope, state, id)?;
    Ok(writer)
}

pub(super) fn refresh(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let (writer, error, terminal, blocked, closed) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::WritableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        (
            stream.writer_id,
            stream.error.clone(),
            stream.started && !stream.busy && !stream.abort_in_flight,
            !stream.queue.is_empty() && !stream.closing,
            stream.closed,
        )
    };
    let Some(writer) = writer else {
        return Ok(());
    };
    let mut record = platform_object_mut(state, writer)?;
    let PlatformObjectRecord::WritableWriter {
        ready,
        closed: closed_promise,
        released: false,
        ..
    } = &mut *record
    else {
        return Ok(());
    };
    let mut ready_local = v8::Local::new(scope, &*ready);
    let closed_local = v8::Local::new(scope, &*closed_promise);
    if let Some(error) = error {
        let error = v8::Local::new(scope, error);
        if ready_local.get_promise(scope).state() != v8::PromiseState::Pending {
            // Once rejected, preserve the observable promise identity.
            if ready_local.get_promise(scope).state() == v8::PromiseState::Fulfilled {
                ready_local = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("writer rejected ready"))?;
                *ready = v8::Global::new(scope, ready_local);
            }
        }
        ready_local.reject(scope, error);
        ready_local.get_promise(scope).mark_as_handled();
        if terminal {
            closed_local.reject(scope, error);
            closed_local.get_promise(scope).mark_as_handled();
        }
    } else {
        if blocked && !closed {
            if ready_local.get_promise(scope).state() != v8::PromiseState::Pending {
                ready_local = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("writer backpressure"))?;
                *ready = v8::Global::new(scope, ready_local);
            }
        } else {
            let undefined = v8::undefined(scope).into();
            ready_local.resolve(scope, undefined);
        }
        if closed {
            let undefined = v8::undefined(scope).into();
            closed_local.resolve(scope, undefined);
        }
    }
    Ok(())
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let stream_id = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::WritableWriter {
            stream_id,
            ready,
            closed,
            released,
        } = &*record
        else {
            return Err(V8BindingError::Allocation("WritableStream writer"));
        };
        match member {
            "ready" => return Ok(v8::Local::new(scope, ready).get_promise(scope).into()),
            "closed" => return Ok(v8::Local::new(scope, closed).get_promise(scope).into()),
            "desiredSize" if *released => {
                return Err(V8BindingError::EventCallback(
                    "Writer has released its stream".into(),
                ));
            }
            "desiredSize" => *stream_id,
            _ => return Err(V8BindingError::Allocation("writer attribute")),
        }
    };
    let record = platform_object(state, stream_id)?;
    let PlatformObjectRecord::WritableStream(stream) = &*record else {
        return Err(V8BindingError::Allocation("WritableStream record"));
    };
    if stream.error.is_some() {
        return Ok(v8::null(scope).into());
    }
    let size = if stream.closed {
        0.0
    } else {
        1.0 - stream.queue.iter().filter(|r| r.chunk.is_some()).count() as f64
    };
    Ok(v8::Number::new(scope, size).into())
}

pub(super) fn release(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let reason = readable_streams::type_error(scope, "Writer has released its stream")?;
    let stream_id = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WritableWriter {
            stream_id,
            ready,
            closed,
            released,
        } = &mut *record
        else {
            return Err(V8BindingError::Allocation("WritableStream writer"));
        };
        if *released {
            return Ok(());
        }
        *released = true;
        for promise in [ready, closed] {
            let mut resolver = v8::Local::new(scope, &*promise);
            if resolver.get_promise(scope).state() != v8::PromiseState::Pending {
                resolver = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("released writer promise"))?;
                *promise = v8::Global::new(scope, resolver);
            }
            resolver.reject(scope, reason);
            resolver.get_promise(scope).mark_as_handled();
        }
        *stream_id
    };
    if let PlatformObjectRecord::WritableStream(stream) =
        &mut *platform_object_mut(state, stream_id)?
    {
        stream.locked = false;
        stream.writer_id = None;
    }
    Ok(())
}
