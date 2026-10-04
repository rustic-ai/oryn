//! Native byte-stream pull-into descriptors. Buffers are transferred through V8
//! backing stores; constructor/property lookups never determine view identity.
use super::*;
#[derive(Default)]
pub(super) struct State {
    pub(super) pending: VecDeque<PullInto>,
    pub(super) auto_allocate: Option<usize>,
    pub(super) close_requested: bool,
}
pub(super) struct PullInto {
    pub(super) buffer: stream_graph::Owned<v8::ArrayBuffer>,
    // The pull-into capacity survives author transfer of the current buffer.
    buffer_byte_length: usize,
    pub(super) resolver: Option<stream_graph::Owned<v8::PromiseResolver>>,
    orphan: bool,
    default_read: bool,
    tee_delivery: bool,
    offset: usize,
    length: usize,
    filled: usize,
    minimum: usize,
    kind: ViewKind,
    pub(super) request: Option<u64>,
}
#[derive(Clone, Copy)]
enum ViewKind {
    U8,
    U8Clamped,
    I8,
    U16,
    I16,
    U32,
    I32,
    F16,
    F32,
    F64,
    I64,
    U64,
    DataView,
}
impl ViewKind {
    fn size(self) -> usize {
        match self {
            Self::U8 | Self::U8Clamped | Self::I8 | Self::DataView => 1,
            Self::U16 | Self::I16 | Self::F16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            _ => 8,
        }
    }
    fn of(value: v8::Local<v8::Value>) -> Result<Self, V8BindingError> {
        Ok(if value.is_uint8_array() {
            Self::U8
        } else if value.is_uint8_clamped_array() {
            Self::U8Clamped
        } else if value.is_int8_array() {
            Self::I8
        } else if value.is_uint16_array() {
            Self::U16
        } else if value.is_int16_array() {
            Self::I16
        } else if value.is_uint32_array() {
            Self::U32
        } else if value.is_int32_array() {
            Self::I32
        } else if value.is_float16_array() {
            Self::F16
        } else if value.is_float32_array() {
            Self::F32
        } else if value.is_float64_array() {
            Self::F64
        } else if value.is_big_int64_array() {
            Self::I64
        } else if value.is_big_uint64_array() {
            Self::U64
        } else if value.is_data_view() {
            Self::DataView
        } else {
            return Err(type_error("read requires an ArrayBufferView"));
        })
    }
    fn view<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        buffer: v8::Local<'s, v8::ArrayBuffer>,
        offset: usize,
        bytes: usize,
    ) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
        macro_rules! make {
            ($kind:ident) => {
                v8::$kind::new(scope, buffer, offset, bytes / self.size()).map(Into::into)
            };
        }
        let value = match self {
            Self::U8 => make!(Uint8Array),
            Self::U8Clamped => make!(Uint8ClampedArray),
            Self::I8 => make!(Int8Array),
            Self::U16 => make!(Uint16Array),
            Self::I16 => make!(Int16Array),
            Self::U32 => make!(Uint32Array),
            Self::I32 => make!(Int32Array),
            Self::F16 => {
                v8::Float16Array::new(scope, buffer, offset, bytes / self.size()).map(|view| {
                    // SAFETY: a native Float16Array is a Value. v8 150.4 exposes its
                    // checked Value-to-Float16Array cast but omits the reverse From impl.
                    unsafe { v8::Local::<v8::Value>::cast_unchecked(view) }
                })
            }
            Self::F32 => make!(Float32Array),
            Self::F64 => make!(Float64Array),
            Self::I64 => make!(BigInt64Array),
            Self::U64 => make!(BigUint64Array),
            Self::DataView => Some(v8::DataView::new(scope, buffer, offset, bytes).into()),
        };
        value.ok_or(V8BindingError::Allocation("BYOB result view"))
    }
}
fn type_error(message: &str) -> V8BindingError {
    V8BindingError::Message(format!("TypeError: {message}"))
}
pub(super) fn enabled(state: &V8BindingState, stream: u64) -> bool {
    matches!(state.platform_objects.borrow().get(&stream),Some(PlatformObjectRecord::ReadableStream(record)) if record.bytes.is_some())
}
fn transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::ArrayBuffer>,
) -> Result<v8::Local<'s, v8::ArrayBuffer>, V8BindingError> {
    if !buffer.is_detachable() || buffer.was_detached() {
        return Err(type_error("buffer cannot be transferred"));
    }
    let backing = buffer.get_backing_store();
    if buffer.detach(None) != Some(true) {
        return Err(type_error("buffer transfer failed"));
    }
    Ok(v8::ArrayBuffer::with_backing_store(scope, &backing))
}
pub(super) fn acquire(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
) -> Result<u64, V8BindingError> {
    if !enabled(state, stream) {
        return Err(type_error("BYOB reader requires a byte stream"));
    }
    let id = readable_streams::acquire(scope, state, stream)?;
    if let PlatformObjectRecord::ReadableReader { byob, .. } = &mut *platform_object_mut(state, id)?
    {
        *byob = true;
    }
    Ok(id)
}
fn read_into<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    reader: u64,
    value: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    require_byob: bool,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let result = (|| {
        if let Some(error) = readable_streams::reader_error(scope, state, reader)? {
            return readable_streams::reject(scope, error);
        }
        let stream = match &*platform_object(state, reader)? {
            PlatformObjectRecord::ReadableReader {
                stream_id, byob, ..
            } if !require_byob || *byob => *stream_id,
            _ => return Err(type_error("incompatible BYOB reader")),
        };
        let kind = ViewKind::of(value)?;
        let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
            .map_err(|_| type_error("read requires a view"))?;
        let length = view.byte_length();
        let offset = view.byte_offset();
        if length == 0 {
            return Err(type_error("BYOB view must not be empty"));
        }
        let minimum = option_value(scope, options, "min")
            .filter(|value| !value.is_undefined())
            .map(|value| {
                value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)
            })
            .transpose()?
            .unwrap_or(1.0);
        if !minimum.is_finite()
            || minimum.trunc() < 1.0
            || minimum.trunc() > length as f64 / kind.size() as f64
        {
            return Err(type_error("invalid BYOB minimum"));
        }
        if let Some(error) = readable_streams::stored_error(scope, state, stream)? {
            return readable_streams::reject(scope, error);
        }
        let buffer = view
            .buffer(scope)
            .ok_or_else(|| type_error("view buffer is unavailable"))?;
        let buffer = transfer(scope, buffer)?;
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or(V8BindingError::Allocation("BYOB read promise"))?;
        let promise = resolver.get_promise(scope);
        let mut already_pending = false;
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(state, stream)?
        {
            record.disturbed = true;
            already_pending = !record
                .bytes
                .as_ref()
                .ok_or_else(|| type_error("not a byte stream"))?
                .pending
                .is_empty();
            record.bytes.as_mut().unwrap().pending.push_back(PullInto {
                buffer_byte_length: buffer.byte_length(),
                buffer: stream_graph::Owned::new(scope, buffer),
                resolver: Some(stream_graph::Owned::new(scope, resolver)),
                orphan: false,
                default_read: !require_byob,
                tee_delivery: false,
                offset,
                length,
                filled: 0,
                minimum: minimum.trunc() as usize * kind.size(),
                kind,
                request: None,
            });
        }
        if !already_pending {
            drain(scope, state, stream)?;
        }
        request_host_read(state, stream)?;
        readable_streams::pull(scope, state, stream)?;
        stream_graph::refresh(scope, state)?;
        Ok(promise)
    })();
    match result {
        Ok(promise) => Ok(promise),
        Err(V8BindingError::JavaScriptException) => Err(V8BindingError::JavaScriptException),
        Err(error) => {
            let error = readable_streams::type_error(scope, &error.to_string())?;
            readable_streams::reject(scope, error)
        }
    }
}
fn settle(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
    descriptor: PullInto,
    done: bool,
) -> Result<(), V8BindingError> {
    invalidate(state, descriptor.request);
    let original = stream_graph::local(scope, &descriptor.buffer);
    let buffer = transfer(scope, original)?;
    let value = if done && descriptor.default_read {
        v8::undefined(scope).into()
    } else {
        descriptor
            .kind
            .view(scope, buffer, descriptor.offset, descriptor.filled)?
    };
    let result = read_result(scope, state, stream, done, value)?;
    if let Some(resolver) = &descriptor.resolver {
        stream_graph::local(scope, resolver).resolve(scope, result.into());
    }
    Ok(())
}
pub(super) fn drain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    stream: u64,
) -> Result<(), V8BindingError> {
    loop {
        let mut invalid = false;
        let next = {
            let mut record = platform_object_mut(state, stream)?;
            let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
                return Err(type_error("byte stream missing"));
            };
            let Some(bytes) = record.bytes.as_mut() else {
                return Ok(());
            };
            let Some(mut descriptor) = bytes.pending.pop_front() else {
                return Ok(());
            };
            while descriptor.filled < descriptor.length {
                let Some(chunk) = record.queue.pop_front() else {
                    break;
                };
                let chunk = stream_graph::local(scope, chunk);
                let view = v8::Local::<v8::Uint8Array>::try_from(chunk)
                    .map_err(|_| type_error("invalid byte queue chunk"))?;
                let mut amount = view
                    .byte_length()
                    .min(descriptor.length - descriptor.filled);
                if descriptor.filled + amount >= descriptor.minimum {
                    amount = (descriptor.filled + amount) / descriptor.kind.size()
                        * descriptor.kind.size()
                        - descriptor.filled;
                }
                if amount == 0 {
                    record.queue.push_front(stream_graph::Owned::new(scope, chunk));
                    break;
                }
                let mut data = vec![0; amount];
                view.copy_contents(&mut data);
                let buffer = stream_graph::local(scope, &descriptor.buffer);
                let target = v8::Uint8Array::new(
                    scope,
                    buffer,
                    descriptor.offset + descriptor.filled,
                    amount,
                )
                .ok_or(V8BindingError::Allocation("BYOB fill view"))?;
                let backing = target
                    .buffer(scope)
                    .ok_or_else(|| type_error("BYOB buffer detached"))?
                    .get_backing_store();
                let start = target.byte_offset();
                for (index, byte) in data.into_iter().enumerate() {
                    backing[start + index].set(byte);
                }
                descriptor.filled += amount;
                if amount < view.byte_length() {
                    let buffer = view
                        .buffer(scope)
                        .ok_or_else(|| type_error("byte queue detached"))?;
                    let remainder = v8::Uint8Array::new(
                        scope,
                        buffer,
                        view.byte_offset() + amount,
                        view.byte_length() - amount,
                    )
                    .ok_or(V8BindingError::Allocation("byte queue remainder"))?;
                    record.queue.push_front(stream_graph::Owned::new(
                        scope,
                        v8::Local::<v8::Value>::from(remainder),
                    ));
                }
            }
            if descriptor.filled >= descriptor.minimum && !descriptor.orphan {
                let remainder = descriptor.filled % descriptor.kind.size();
                if remainder > 0 {
                    let buffer = stream_graph::local(scope, &descriptor.buffer);
                    let view = v8::Uint8Array::new(
                        scope,
                        buffer,
                        descriptor.offset + descriptor.filled - remainder,
                        remainder,
                    )
                    .ok_or(V8BindingError::Allocation("BYOB remainder"))?;
                    let mut copied = vec![0; remainder];
                    view.copy_contents(&mut copied);
                    let buffer = array_buffer_from_bytes(scope, &copied);
                    let view = v8::Uint8Array::new(scope, buffer, 0, remainder)
                        .ok_or(V8BindingError::Allocation("BYOB remainder copy"))?;
                    record.queue.push_front(stream_graph::Owned::new(
                        scope,
                        v8::Local::<v8::Value>::from(view),
                    ));
                    descriptor.filled -= remainder;
                }
            }
            let terminal = record.closed && record.queue.is_empty();
            if (descriptor.filled >= descriptor.minimum
                && descriptor.filled % descriptor.kind.size() == 0)
                || (terminal && descriptor.filled % descriptor.kind.size() == 0)
            {
                Some((descriptor, terminal))
            } else if terminal || (bytes.close_requested && record.queue.is_empty()) {
                bytes.pending.push_front(descriptor);
                invalid = true;
                None
            } else {
                bytes.pending.push_front(descriptor);
                None
            }
        };
        if invalid {
            let reason = readable_streams::type_error(
                scope,
                "closed byte stream has an incomplete element",
            )?;
            readable_streams::error(scope, state, stream, reason)?;
            return Err(type_error("closed byte stream has an incomplete element"));
        }
        let Some((descriptor, terminal)) = next else {
            return Ok(());
        };
        if descriptor.tee_delivery {
            if descriptor.filled > 0 {
                let buffer = stream_graph::local(scope, &descriptor.buffer);
                let view = v8::Uint8Array::new(scope, buffer, descriptor.offset, descriptor.filled)
                    .ok_or(V8BindingError::Allocation("tee byte result"))?;
                if let PlatformObjectRecord::ReadableStream(record) =
                    &mut *platform_object_mut(state, stream)?
                {
                    record.queue.push_back(stream_graph::Owned::new(
                        scope,
                        v8::Local::<v8::Value>::from(view),
                    ));
                }
            }
            invalidate(state, descriptor.request);
            stream_tee::pump(scope, state, stream)?;
            continue;
        }
        if descriptor.orphan {
            if descriptor.filled > 0 {
                let buffer = stream_graph::local(scope, &descriptor.buffer);
                let view = v8::Uint8Array::new(scope, buffer, descriptor.offset, descriptor.filled)
                    .ok_or(V8BindingError::Allocation("released byte descriptor"))?;
                if let PlatformObjectRecord::ReadableStream(record) =
                    &mut *platform_object_mut(state, stream)?
                {
                    record.queue.push_front(stream_graph::Owned::new(
                        scope,
                        v8::Local::<v8::Value>::from(view),
                    ));
                }
            }
            continue;
        }
        handle_queue_drain(scope, state, stream)?;
        let done = terminal;
        settle(scope, state, stream, descriptor, done)?;
    }
}
pub(super) fn enqueue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    controller: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let stream = match &*platform_object(state, controller)? {
        PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
        _ => return Err(type_error("byte controller missing")),
    };
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
        .map_err(|_| type_error("byte enqueue requires a view"))?;
    let length = view.byte_length();
    let offset = view.byte_offset();
    if length == 0 {
        return Err(type_error("byte chunk must not be empty"));
    }
    {
        let record = platform_object(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &*record else {
            return Err(type_error("byte stream missing"));
        };
        if record.closed
            || record.error.is_some()
            || record
                .bytes
                .as_ref()
                .is_some_and(|bytes| bytes.close_requested)
        {
            return Err(type_error("byte stream is not readable"));
        }
    }
    let input = view
        .buffer(scope)
        .ok_or_else(|| type_error("byte buffer missing"))?;
    let buffer = transfer(scope, input)?;
    let array = v8::Uint8Array::new(scope, buffer, offset, length)
        .ok_or(V8BindingError::Allocation("byte chunk"))?;
    let reader = match &*platform_object(state, stream)? {
        PlatformObjectRecord::ReadableStream(record) => record.reader_id,
        _ => None,
    };
    let byob_reader = reader.is_some_and(|reader| {
        matches!(
            state.platform_objects.borrow().get(&reader),
            Some(PlatformObjectRecord::ReadableReader { byob: true, .. })
        )
    });
    let mut invalidated = Vec::new();
    let mut default_descriptor = None;
    let pending = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Err(type_error("byte stream missing"));
        };
        let bytes = record.bytes.as_mut().unwrap();
        if let Some(descriptor) = bytes.pending.front_mut() {
            let old = stream_graph::local(scope, &descriptor.buffer);
            let replacement = if old.strict_equals(input.into()) {
                buffer
            } else {
                transfer(scope, old)?
            };
            descriptor.buffer = stream_graph::Owned::new(scope, replacement);
            invalidated.extend(descriptor.request.take());
        }
        if bytes
            .pending
            .front()
            .is_some_and(|descriptor| descriptor.orphan)
            && let Some(descriptor) = bytes.pending.pop_front()
            && descriptor.filled > 0
        {
            let source = stream_graph::local(scope, &descriptor.buffer);
            let view = v8::Uint8Array::new(scope, source, descriptor.offset, descriptor.filled)
                .ok_or(V8BindingError::Allocation("released byte prefix"))?;
            let mut prefix = vec![0; descriptor.filled];
            view.copy_contents(&mut prefix);
            let copied = array_buffer_from_bytes(scope, &prefix);
            let view = v8::Uint8Array::new(scope, copied, 0, prefix.len())
                .ok_or(V8BindingError::Allocation("released byte prefix copy"))?;
            record.queue.push_back(stream_graph::Owned::new(
                scope,
                v8::Local::<v8::Value>::from(view),
            ));
        }
        if !byob_reader {
            if bytes
                .pending
                .front()
                .is_some_and(|descriptor| descriptor.default_read)
            {
                default_descriptor = bytes.pending.pop_front();
            }
            false
        } else {
            let pending = !bytes.pending.is_empty();
            if pending {
                record.queue.push_back(stream_graph::Owned::new(
                    scope,
                    v8::Local::<v8::Value>::from(array),
                ));
            }
            pending
        }
    };
    for request in invalidated {
        invalidate(state, Some(request));
    }
    if let Some(descriptor) = default_descriptor {
        let value = {
            let mut record = platform_object_mut(state, stream)?;
            let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
                return Err(type_error("byte stream missing"));
            };
            if record.queue.is_empty() {
                array.into()
            } else {
                record.queue.push_back(stream_graph::Owned::new(
                    scope,
                    v8::Local::<v8::Value>::from(array),
                ));
                stream_graph::local(scope, record.queue.pop_front().unwrap())
            }
        };
        if let Some(resolver) = descriptor.resolver {
            let result = read_result(scope, state, stream, false, value)?;
            stream_graph::local(scope, resolver).resolve(scope, result.into());
        }
        return readable_streams::pull(scope, state, stream);
    }
    if pending {
        drain(scope, state, stream)?;
        request_host_read(state, stream)?;
        readable_streams::pull(scope, state, stream)
    } else {
        readable_enqueue_default(scope, state, controller, array.into())
    }
}
pub(super) fn fail(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let pending = {
        let mut record = platform_object_mut(state, stream)?;
        match &mut *record {
            PlatformObjectRecord::ReadableStream(record) => record
                .bytes
                .as_mut()
                .map(|bytes| std::mem::take(&mut bytes.pending))
                .unwrap_or_default(),
            _ => VecDeque::new(),
        }
    };
    for descriptor in pending {
        invalidate(state, descriptor.request);
        if let Some(resolver) = descriptor.resolver {
            stream_graph::local(scope, resolver).reject(scope, reason);
        }
    }
    Ok(())
}

pub(super) fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    controller: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let stream = match &*platform_object(state, controller)? {
        PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
        _ => return Err(type_error("byte controller missing")),
    };
    let existing = {
        let record = platform_object(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &*record else {
            return Err(type_error("byte stream missing"));
        };
        let Some(descriptor) = record
            .bytes
            .as_ref()
            .and_then(|bytes| bytes.pending.front())
        else {
            return Ok(v8::null(scope).into());
        };
        descriptor.request
    };
    let id = if let Some(id) = existing {
        id
    } else {
        let view = {
            let record = platform_object(state, stream)?;
            let PlatformObjectRecord::ReadableStream(record) = &*record else {
                return Err(type_error("byte stream missing"));
            };
            let descriptor = record.bytes.as_ref().unwrap().pending.front().unwrap();
            let buffer = stream_graph::local(scope, &descriptor.buffer);
            v8::Uint8Array::new(
                scope,
                buffer,
                descriptor.offset + descriptor.filled,
                descriptor.length - descriptor.filled,
            )
            .ok_or(V8BindingError::Allocation("BYOB request view"))?
        };
        let id = register_platform_object(
            state,
            PlatformObjectRecord::ReadableByobRequest {
                stream_id: Some(stream),
                view: Some(stream_graph::Owned::new(
                    scope,
                    v8::Local::<v8::Value>::from(view),
                )),
            },
        );
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(state, stream)?
        {
            record
                .bytes
                .as_mut()
                .unwrap()
                .pending
                .front_mut()
                .unwrap()
                .request = Some(id);
        }
        id
    };
    Ok(new_platform_object(scope, state, "ReadableStreamBYOBRequest", id)?.into())
}
pub(super) fn request_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::ReadableByobRequest { view, .. } = &*record else {
        return Err(type_error("BYOB request missing"));
    };
    Ok(view
        .as_ref()
        .map(|view| stream_graph::local(scope, view))
        .unwrap_or_else(|| v8::null(scope).into()))
}

pub(super) fn respond(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    value: v8::Local<v8::Value>,
    replacement: bool,
) -> Result<(), V8BindingError> {
    let stream = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableByobRequest {
            stream_id: Some(stream_id),
            ..
        } => *stream_id,
        _ => return Err(type_error("BYOB request missing")),
    };
    let replacement = if replacement {
        Some(
            v8::Local::<v8::ArrayBufferView>::try_from(value)
                .map_err(|_| type_error("response requires a view"))?,
        )
    } else {
        None
    };
    let amount = if let Some(view) = replacement {
        view.byte_length() as f64
    } else {
        value
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    let amount = amount.trunc();
    if !amount.is_finite() || amount < 0.0 || amount > usize::MAX as f64 {
        return Err(type_error("invalid bytesWritten"));
    }
    let amount = amount as usize;
    {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Err(type_error("byte stream missing"));
        };
        let closed = record.closed;
        let descriptor = record
            .bytes
            .as_mut()
            .and_then(|bytes| bytes.pending.front_mut())
            .filter(|descriptor| descriptor.request == Some(id))
            .ok_or_else(|| type_error("BYOB request is no longer active"))?;
        if (closed && amount != 0)
            || (!closed && amount == 0)
            || amount > descriptor.length - descriptor.filled
        {
            return Err(type_error("invalid BYOB response length"));
        }
        let old = stream_graph::local(scope, &descriptor.buffer);
        let buffer = if let Some(view) = replacement {
            let buffer = view
                .buffer(scope)
                .ok_or_else(|| type_error("replacement buffer missing"))?;
            if view.byte_offset() != descriptor.offset + descriptor.filled
                || buffer.byte_length() != descriptor.buffer_byte_length
            {
                return Err(type_error(
                    "replacement view does not match pull-into descriptor",
                ));
            }
            buffer
        } else {
            old
        };
        let buffer = transfer(scope, buffer)?;
        descriptor.buffer = stream_graph::Owned::new(scope, buffer);
        descriptor.filled += amount;
        descriptor.request = None;
    }
    invalidate(state, Some(id));
    drain(scope, state, stream)?;
    readable_streams::pull(scope, state, stream)?;
    stream_graph::refresh(scope, state)
}
pub(super) fn desired_size(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
) -> Result<Option<f64>, V8BindingError> {
    let record = platform_object(state, stream)?;
    let PlatformObjectRecord::ReadableStream(record) = &*record else {
        return Err(type_error("byte stream missing"));
    };
    if record.error.is_some() {
        return Ok(None);
    }
    if record.closed {
        return Ok(Some(0.0));
    }
    let mut queued = 0.0;
    for value in &record.queue {
        let value = stream_graph::local(scope, value);
        let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
            .map_err(|_| type_error("invalid byte queue"))?;
        queued += view.byte_length() as f64;
    }
    Ok(Some(record.high_water_mark - queued))
}

pub(super) fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    reader: u64,
    value: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    read_into(scope, state, reader, value, options, true)
}
pub(super) fn read_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    reader: u64,
) -> Result<Option<v8::Local<'s, v8::Promise>>, V8BindingError> {
    let stream = match &*platform_object(state, reader)? {
        PlatformObjectRecord::ReadableReader {
            stream_id,
            byob: false,
            ..
        } => *stream_id,
        _ => return Ok(None),
    };
    let size = match &*platform_object(state, stream)? {
        PlatformObjectRecord::ReadableStream(record)
            if record.queue.is_empty() && !record.closed =>
        {
            record.bytes.as_ref().and_then(|bytes| bytes.auto_allocate)
        }
        _ => None,
    };
    let Some(size) = size else {
        return Ok(None);
    };
    if size > 64 * 1024 * 1024 {
        let error = readable_streams::type_error(
            scope,
            "autoAllocateChunkSize exceeds native buffer budget",
        )?;
        return Ok(Some(readable_streams::reject(scope, error)?));
    }
    let buffer = v8::ArrayBuffer::new(scope, size);
    let view = v8::Uint8Array::new(scope, buffer, 0, size)
        .ok_or(V8BindingError::Allocation("autoallocated byte view"))?;
    let options = v8::undefined(scope).into();
    read_into(scope, state, reader, view.into(), options, false).map(Some)
}
fn request_host_read(state: &V8BindingState, stream: u64) -> Result<(), V8BindingError> {
    let host = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Ok(());
        };
        if record.closed
            || record.host_read_pending
            || record
                .bytes
                .as_ref()
                .is_none_or(|bytes| bytes.pending.is_empty())
        {
            None
        } else {
            let host = record.host_stream_id;
            if host.is_some() {
                record.host_read_pending = true;
            }
            host
        }
    };
    if let Some(host) = host
        && let Err(error) = submit_platform_operation(
            state,
            PlatformHostRequest::HttpRead {
                stream_id: host,
                max_bytes: 64 * 1024,
            },
            PendingPlatformOperation::BodyRead {
                stream_object_id: stream,
            },
        )
    {
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(state, stream)?
        {
            record.host_read_pending = false;
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn release(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let resolvers = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Ok(());
        };
        let Some(bytes) = record.bytes.as_mut() else {
            return Ok(());
        };
        let mut resolvers = Vec::new();
        for descriptor in &mut bytes.pending {
            if let Some(resolver) = descriptor.resolver.take() {
                resolvers.push(resolver);
            }
            descriptor.orphan = true;
            descriptor.kind = ViewKind::U8;
            descriptor.minimum = 1;
        }
        bytes.pending.truncate(1);
        resolvers
    };
    for resolver in resolvers {
        stream_graph::local(scope, resolver).reject(scope, reason);
    }
    Ok(())
}
pub(super) fn close(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    controller: u64,
) -> Result<(), V8BindingError> {
    let stream = match &*platform_object(state, controller)? {
        PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
        _ => return Err(type_error("byte controller missing")),
    };
    let (queued, invalid) = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Err(type_error("byte stream missing"));
        };
        let bytes = record.bytes.as_mut().unwrap();
        if record.closed || record.error.is_some() || bytes.close_requested {
            return Err(type_error("byte stream is already closing"));
        }
        bytes.close_requested = true;
        (
            !record.queue.is_empty(),
            bytes
                .pending
                .front()
                .is_some_and(|descriptor| descriptor.filled % descriptor.kind.size() != 0),
        )
    };
    if invalid {
        let reason = readable_streams::type_error(scope, "incomplete byte element on close")?;
        readable_streams::error(scope, state, stream, reason)?;
        return Err(type_error("incomplete byte element on close"));
    }
    if !queued {
        readable_close_default(scope, state, controller)?;
    }
    Ok(())
}

pub(super) fn handle_queue_drain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    stream: u64,
) -> Result<(), V8BindingError> {
    let close = match &*platform_object(state, stream)? {
        PlatformObjectRecord::ReadableStream(record)
            if !record.closed
                && record.queue.is_empty()
                && record
                    .bytes
                    .as_ref()
                    .is_some_and(|bytes| bytes.close_requested) =>
        {
            Some(record.controller_id)
        }
        _ => None,
    };
    if let Some(controller) = close {
        readable_close_default(scope, state, controller)?;
    }
    Ok(())
}
pub(super) fn close_default_reads(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
) -> Result<(), V8BindingError> {
    let resolvers = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Ok(());
        };
        record
            .bytes
            .as_mut()
            .map(|bytes| {
                bytes
                    .pending
                    .iter_mut()
                    .filter(|descriptor| descriptor.default_read)
                    .filter_map(|descriptor| descriptor.resolver.take())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    for resolver in resolvers {
        let undefined = v8::undefined(scope).into();
        let result = read_result(scope, state, stream, true, undefined)?;
        stream_graph::local(scope, resolver).resolve(scope, result.into());
    }
    Ok(())
}
pub(super) fn cancel_pending(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    stream: u64,
) -> Result<(), V8BindingError> {
    let pending = {
        let mut record = platform_object_mut(state, stream)?;
        let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
            return Ok(());
        };
        record
            .bytes
            .as_mut()
            .map(|bytes| std::mem::take(&mut bytes.pending))
            .unwrap_or_default()
    };
    for descriptor in pending {
        invalidate(state, descriptor.request);
        if let Some(resolver) = descriptor.resolver {
            let undefined = v8::undefined(scope).into();
            let result = read_result(scope, state, stream, true, undefined)?;
            stream_graph::local(scope, resolver).resolve(scope, result.into());
        }
    }
    Ok(())
}

pub(super) fn queue_total(
    scope: &mut v8::PinScope,
    record: &ReadableStreamRecord,
) -> Result<f64, V8BindingError> {
    if record.bytes.is_none() {
        return Ok(record.queue.len() as f64);
    }
    let mut total = 0.0;
    for value in &record.queue {
        let value = stream_graph::local(scope, value);
        let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
            .map_err(|_| type_error("invalid byte queue"))?;
        total += view.byte_length() as f64;
    }
    Ok(total)
}

fn invalidate(state: &V8BindingState, id: Option<u64>) {
    if let Some(id) = id
        && let Ok(mut record) = platform_object_mut(state, id)
        && let PlatformObjectRecord::ReadableByobRequest { stream_id, view } = &mut *record
    {
        *stream_id = None;
        *view = None;
    }
}

pub(super) fn prepare_tee_pull(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    source: u64,
    branches: [u64; 2],
) -> Result<(), V8BindingError> {
    if !enabled(state, source) {
        return Ok(());
    }
    let mut request = None;
    for branch in branches {
        let record = platform_object(state, branch)?;
        if let PlatformObjectRecord::ReadableStream(record) = &*record
            && let Some(descriptor) = record
                .bytes
                .as_ref()
                .and_then(|bytes| bytes.pending.front())
        {
            let buffer = stream_graph::local(scope, &descriptor.buffer);
            request = Some((
                buffer.byte_length(),
                descriptor.offset + descriptor.filled,
                descriptor.length - descriptor.filled,
            ));
            break;
        }
    }
    let mut record = platform_object_mut(state, source)?;
    let PlatformObjectRecord::ReadableStream(record) = &mut *record else {
        return Ok(());
    };
    let bytes = record.bytes.as_mut().unwrap();
    if !bytes.pending.is_empty() || record.closed {
        return Ok(());
    }
    let request = request.or_else(|| bytes.auto_allocate.map(|size| (size, 0, size)));
    let Some((size, offset, length)) = request else {
        return Ok(());
    };
    if size > 64 * 1024 * 1024 {
        return Err(type_error("tee pull buffer exceeds native budget"));
    }
    let buffer = v8::ArrayBuffer::new(scope, size);
    bytes.pending.push_back(PullInto {
        buffer_byte_length: size,
        buffer: stream_graph::Owned::new(scope, buffer),
        resolver: None,
        orphan: false,
        default_read: false,
        tee_delivery: true,
        offset,
        length,
        filled: 0,
        minimum: 1,
        kind: ViewKind::U8,
        request: None,
    });
    Ok(())
}

pub(super) fn validate_controller(
    state: &V8BindingState,
    id: u64,
    interface: &str,
) -> Result<(), V8BindingError> {
    if !matches!(
        interface,
        "ReadableByteStreamController" | "ReadableStreamDefaultController"
    ) {
        return Ok(());
    }
    let stream = match &*platform_object(state, id)? {
        PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
        _ => return Err(type_error("incompatible readable controller")),
    };
    if enabled(state, stream) != (interface == "ReadableByteStreamController") {
        return Err(type_error("incompatible readable controller"));
    }
    Ok(())
}
pub(super) fn validate_reader(
    state: &V8BindingState,
    id: u64,
    interface: &str,
) -> Result<(), V8BindingError> {
    if !matches!(
        interface,
        "ReadableStreamDefaultReader" | "ReadableStreamBYOBReader"
    ) {
        return Ok(());
    }
    if !matches!(&*platform_object(state,id)?,PlatformObjectRecord::ReadableReader{byob,..} if *byob==(interface=="ReadableStreamBYOBReader"))
    {
        return Err(type_error("incompatible readable reader"));
    }
    Ok(())
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
    fn byob_transfers_buffers_and_preserves_view_kind_offset_and_element_alignment() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{let c;const s=new ReadableStream({type:'bytes',start(x){c=x}});const r=s.getReader({mode:'byob'});
          const input=new Uint16Array(new ArrayBuffer(12),2,3);const read=r.read(input,{min:1});if(input.buffer.byteLength!==0)throw Error('read transfer');
          const chunk=new Uint8Array([1,2,3]);c.enqueue(chunk);if(chunk.buffer.byteLength!==0)throw Error('enqueue transfer');
          const first=await read;if(!(first.value instanceof Uint16Array)||first.value.byteOffset!==2||first.value.byteLength!==2||first.done)throw Error('typed result');
          const rest=await r.read(new Uint8Array(8));if(rest.value[0]!==3||rest.value.byteLength!==1)throw Error('alignment remainder');
          c.close();const end=await r.read(new DataView(new ArrayBuffer(4),1,2));if(!end.done||!(end.value instanceof DataView)||end.value.byteLength!==0||end.value.byteOffset!==1)throw Error('closed typed result');result='ok';})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn request_response_invalidates_views_and_close_requires_closed_response() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{let c;const s=new ReadableStream({type:'bytes',start(x){c=x}});const r=s.getReader({mode:'byob'});
          const p=r.read(new Uint8Array(8));const request=c.byobRequest,view=request.view;view[0]=42;request.respond(1);
          if(view.byteLength!==0||request.view!==null)throw Error('request invalidation');let threw=false;try{request.respond(1)}catch(e){threw=e instanceof TypeError}if(!threw)throw Error('stale request');
          if((await p).value[0]!==42)throw Error('written byte');const end=r.read(new Uint8Array(5));const closed=c.byobRequest;c.close();closed.respond(0);if(!(await end).done)throw Error('closed response');result='ok';})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn autoallocation_and_released_reader_preserve_outstanding_native_request() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{let c;const s=new ReadableStream({type:'bytes',autoAllocateChunkSize:4,start(x){c=x}});const a=s.getReader();const old=a.read();old.catch(()=>{});const request=c.byobRequest;a.releaseLock();const b=s.getReader();const next=b.read();
          if(c.byobRequest!==request)throw Error('released request identity');request.view[0]=11;request.respond(1);
          let rejected=false;try{await old}catch(e){rejected=e instanceof TypeError}if(!rejected)throw Error('released read');const value=await next;if(value.done||value.value[0]!==11)throw Error('next reader bytes');
          await b.cancel();result='ok';})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn incomplete_element_close_errors_pending_read_and_reader_closed() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
          (async()=>{let c;const s=new ReadableStream({type:'bytes',start(x){c=x}});const r=s.getReader({mode:'byob'});r.closed.catch(()=>{});const p=r.read(new Uint16Array(1));p.catch(()=>{});c.enqueue(new Uint8Array([7]));let threw=false;try{c.close()}catch(e){threw=e instanceof TypeError}if(!threw)throw Error('close error');for(const value of [p,r.closed]){let caught=false;try{await value}catch(e){caught=e instanceof TypeError}if(!caught)throw Error('pending rejection')}result='ok';})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn closed_response_keeps_aligned_bytes_and_transfers_the_result() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
        (async()=>{let c;const s=new ReadableStream({type:'bytes',start(x){c=x}});const r=s.getReader({mode:'byob'});const p=r.read(new Uint16Array(4),{min:3});let q=c.byobRequest;q.view[0]=9;q.respond(2);q=c.byobRequest;const alias=q.view;c.close();q.respond(0);const v=await p;if(!v.done||v.value.byteLength!==2||new Uint8Array(v.value.buffer)[0]!==9||alias.byteLength!==0)throw Error('closed descriptor');result='ok'})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn closed_queued_source_rejects_an_unreachable_minimum() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
        (async()=>{const s=new ReadableStream({type:'bytes',start(c){c.enqueue(new Uint8Array([1,2]));c.close()}});const r=s.getReader({mode:'byob'});r.closed.catch(()=>{});let caught=false;try{await r.read(new Uint8Array(8),{min:4})}catch(e){caught=e instanceof TypeError}if(!caught)throw Error('minimum');result='ok'})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
    #[test]
    fn byte_tee_branches_have_independent_transferred_storage() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.failure='';
        (async()=>{const s=new ReadableStream({type:'bytes',start(c){c.enqueue(new Uint8Array([5,6]));c.close()}});const [a,b]=s.tee();const ar=a.getReader({mode:'byob'}),br=b.getReader();const [x,y]=await Promise.all([ar.read(new Uint8Array(4)),br.read()]);x.value[0]=99;if(y.value[0]!==5||x.done||y.done)throw Error('tee storage');if(!(await ar.read(new Uint8Array(1))).done||!(await br.read()).done)throw Error('tee close');result='ok'})().catch(e=>failure=String(e));true"#,
        );
        for _ in 0..16 {
            scope.perform_microtask_checkpoint();
        }
        assert_eq!(eval(scope, "failure||result"), "ok");
        let _ = state;
    }
}
