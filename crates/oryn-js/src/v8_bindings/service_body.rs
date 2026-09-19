//! Native demand-driven service response reader. This never invokes public
//! Response/ReadableStream/reader methods or buffers an entire response.
use super::*;
const CHUNK: usize = 64 * 1024;
const QUEUED: usize = 4 * 1024 * 1024;
pub(super) enum Poll {
    Pending,
    Chunk(Vec<u8>),
    Done,
}
pub(super) struct Reader {
    source: BodyStreamLease,
    stream: u64,
    reader: u64,
    pending: Option<v8::Global<v8::Promise>>,
    buffered: Vec<u8>,
    offset: usize,
    done: bool,
}
impl Reader {
    pub(super) fn acquire(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        stream: u64,
    ) -> Result<Self, V8BindingError> {
        let context = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("service body source context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let record = platform_object(state, stream)?;
        let PlatformObjectRecord::ReadableStream(body) = &*record else {
            return Err(V8BindingError::Allocation("service body stream"));
        };
        if body.disturbed || body.locked {
            return Err(V8BindingError::Allocation(
                "service response body already used",
            ));
        }
        drop(record);
        // Retain the source before acquiring its exclusive lock: wrapper
        // allocation failure must leave the stream available to another reader.
        let source = BodyStreamLease::capture(scope, state, stream)?;
        let reader = readable_streams::acquire(scope, state, stream)?;
        Ok(Self {
            source,
            stream,
            reader,
            pending: None,
            buffered: Vec::new(),
            offset: 0,
            done: false,
        })
    }
    // Caller must have a parent-issued demand credit. At most one native read
    // is pending; a large author chunk is split without issuing another read.
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        _state: &Rc<V8BindingState>,
    ) -> Result<Poll, V8BindingError> {
        let owner = self.source.owner()?;
        let state = &owner;
        let context = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("service body source context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        if self.offset < self.buffered.len() {
            return Ok(self.chunk());
        }
        if self.done {
            return Ok(Poll::Done);
        }
        if self.pending.is_none() {
            let value = read_native_stream(scope, state, self.reader)?
                .ok_or(V8BindingError::Allocation("service body read"))?;
            let promise = v8::Local::<v8::Promise>::try_from(value)
                .map_err(|_| V8BindingError::Allocation("service body promise"))?;
            promise.mark_as_handled();
            self.pending = Some(v8::Global::new(scope, promise));
        }
        let promise = v8::Local::new(scope, self.pending.as_ref().unwrap());
        match promise.state() {
            v8::PromiseState::Pending => Ok(Poll::Pending),
            v8::PromiseState::Rejected => {
                self.pending = None;
                self.done = true;
                readable_streams::release(scope, state, self.reader)?;
                Err(V8BindingError::Allocation("service body read rejected"))
            }
            v8::PromiseState::Fulfilled => {
                self.pending = None;
                let value = promise.result(scope);
                let result = v8::Local::<v8::Object>::try_from(value)
                    .map_err(|_| V8BindingError::Allocation("native stream result"))?;
                let key = v8::String::new(scope, "done").unwrap();
                if result
                    .get(scope, key.into())
                    .ok_or(V8BindingError::JavaScriptException)?
                    .boolean_value(scope)
                {
                    self.done = true;
                    readable_streams::release(scope, state, self.reader)?;
                    return Ok(Poll::Done);
                }
                let key = v8::String::new(scope, "value").unwrap();
                let value = result
                    .get(scope, key.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                // Fetch byte streams require Uint8Array, not arbitrary views,
                // strings, or author-coercible iterables.
                let bytes = v8::Local::<v8::Uint8Array>::try_from(value).map_err(|_| {
                    V8BindingError::Allocation("service body chunk must be Uint8Array")
                })?;
                if bytes.byte_length() > QUEUED {
                    return Err(V8BindingError::Allocation(
                        "service body queued chunk budget",
                    ));
                }
                self.buffered = vec![0; bytes.byte_length()];
                bytes.copy_contents(&mut self.buffered);
                self.offset = 0;
                Ok(self.chunk())
            }
        }
    }
    fn chunk(&mut self) -> Poll {
        let end = self.offset.saturating_add(CHUNK).min(self.buffered.len());
        let chunk = self.buffered[self.offset..end].to_vec();
        self.offset = end;
        if end == self.buffered.len() {
            self.buffered.clear();
            self.offset = 0;
        }
        Poll::Chunk(chunk)
    }
    pub(super) fn cancel(
        &mut self,
        scope: &mut v8::PinScope,
        _state: &Rc<V8BindingState>,
        reason: v8::Local<v8::Value>,
    ) -> Result<(), V8BindingError> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        self.pending = None;
        self.buffered.clear();
        self.offset = 0;
        // Cancel the underlying source first so pending native reads settle;
        // then release its exclusively owned reader lock.
        let Some(owner) = self.source.owner.upgrade() else {
            return Ok(());
        };
        if owner.execution_retired() {
            return Ok(());
        }
        let state = &owner;
        let context = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("service body source context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let cancelled = service_worker_events::cancel_body(scope, state, self.stream, reason);
        let released = readable_streams::release(scope, state, self.reader);
        cancelled.and(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_reader_preserves_chunk_boundaries_and_ignores_poisoned_methods() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main></main>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,"ReadableStream.prototype.getReader=()=>{throw Error('author')};ReadableStreamDefaultReader.prototype.read=()=>{throw Error('author')}").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let expected = (0..200_000)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let stream = new_byte_stream_record(scope, &state, expected.clone()).unwrap();
        let mut reader = Reader::acquire(scope, &state, stream).unwrap();
        assert!(Reader::acquire(scope, &state, stream).is_err());
        let mut actual = Vec::new();
        let mut count = 0;
        for _ in 0..32 {
            scope.perform_microtask_checkpoint();
            match reader.poll(scope, &state).unwrap() {
                Poll::Pending => {}
                Poll::Chunk(bytes) => {
                    assert!(bytes.len() <= CHUNK);
                    actual.extend(bytes);
                    count += 1;
                }
                Poll::Done => break,
            }
        }
        assert_eq!(actual, expected);
        assert_eq!(count, 4);
        let record = platform_object(&state, stream).unwrap();
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            panic!()
        };
        assert!(stream.disturbed);
        assert!(!stream.locked);
    }
    #[test]
    fn cancellation_discards_buffered_bytes_and_releases_exclusive_reader() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let stream = new_byte_stream_record(scope, &state, vec![7; 200_000]).unwrap();
        let mut reader = Reader::acquire(scope, &state, stream).unwrap();
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
            if matches!(reader.poll(scope, &state).unwrap(), Poll::Chunk(_)) {
                break;
            }
        }
        let reason = dom_exception_value(scope, "AbortError").unwrap();
        reader.cancel(scope, &state, reason).unwrap();
        assert!(reader.buffered.is_empty());
        assert!(matches!(reader.poll(scope, &state).unwrap(), Poll::Done));
        let record = platform_object(&state, stream).unwrap();
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            panic!()
        };
        assert!(!stream.locked);
    }
}

#[cfg(test)]
mod foreign_reader_tests {
    use super::*;
    #[test]
    fn reader_uses_captured_source_realm_instead_of_callers_colliding_id() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (source, stream) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let source = install_generated_bindings(scope, first, dom, RealmId(71), 1).unwrap();
            let stream = new_byte_stream_record(scope, &source, vec![1, 2, 3]).unwrap();
            (source, stream)
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let destination = install_generated_bindings(scope, second, dom, RealmId(72), 1).unwrap();
        let local = new_byte_stream_record(scope, &destination, vec![9]).unwrap();
        let mut reader = Reader::acquire(scope, &source, stream).unwrap();
        assert_eq!(scope.get_current_context(), second);
        {
            let record = platform_object(&source, reader.reader).unwrap();
            let PlatformObjectRecord::ReadableReader { closed, .. } = &*record else {
                panic!("native reader")
            };
            let resolver = stream_graph::local(scope, closed);
            assert_eq!(
                resolver
                    .get_promise(scope)
                    .get_creation_context(scope)
                    .unwrap(),
                first
            );
        }
        let mut bytes = Vec::new();
        for _ in 0..8 {
            scope.perform_microtask_checkpoint();
            match reader.poll(scope, &destination).unwrap() {
                Poll::Chunk(chunk) => bytes.extend(chunk),
                Poll::Done => break,
                Poll::Pending => {}
            }
        }
        assert_eq!(scope.get_current_context(), second);
        assert_eq!(bytes, vec![1, 2, 3]);
        assert_eq!(
            response_stream_state(&destination, Some(local)).unwrap(),
            (false, false)
        );
        assert!(Rc::ptr_eq(&reader.source.owner().unwrap(), &source));
        let cancellable = new_byte_stream_record(scope, &source, vec![4, 5, 6]).unwrap();
        let lease = BodyStreamLease::capture(scope, &source, cancellable).unwrap();
        let reason = v8::Object::new(scope);
        lease.cancel(scope, reason.into()).unwrap();
        assert_eq!(scope.get_current_context(), second);
        assert_eq!(
            response_stream_state(&destination, Some(local)).unwrap(),
            (false, false)
        );
        let record = platform_object(&source, cancellable).unwrap();
        assert!(
            matches!(&*record,PlatformObjectRecord::ReadableStream(stream)if stream.closed&&stream.disturbed)
        );
    }
}
