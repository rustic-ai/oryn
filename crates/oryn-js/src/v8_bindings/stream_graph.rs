//! Stream values are bootstrapped with a strong handle until an owning native
//! anchor has installed its private tracing edges. All clones share that state.
use super::*;

pub(super) struct Owned<T>(Rc<RefCell<Slot<T>>>);
struct Slot<T> {
    serialized: Option<structured_clone::SerializedValue>,
    strong: Option<v8::Global<T>>,
    weak: Option<v8::Weak<T>>,
    primitive: bool,
}
impl<T> Clone for Owned<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> From<v8::Global<T>> for Owned<T> {
    fn from(value: v8::Global<T>) -> Self {
        Self(Rc::new(RefCell::new(Slot {
            serialized: None,
            strong: Some(value),
            weak: None,
            primitive: false,
        })))
    }
}
impl<T> Owned<T> {
    pub(super) fn new(scope: &mut v8::PinScope, value: impl v8::Handle<Data = T>) -> Self {
        v8::Global::new(scope, value).into()
    }
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        let slot = self.0.borrow();
        if let Some(strong) = &slot.strong {
            Some(v8::Local::new(scope, strong))
        } else {
            slot.weak.as_ref().and_then(|weak| weak.to_local(scope))
        }
    }
}
impl Owned<v8::Value> {
    pub(super) fn serialized(
        scope: &mut v8::PinScope,
        value: structured_clone::SerializedValue,
    ) -> Self {
        let undefined: v8::Local<v8::Value> = v8::undefined(scope).into();
        let owned = Self::new(scope, undefined);
        owned.0.borrow_mut().serialized = Some(value);
        owned
    }
    pub(super) fn is_serialized(&self) -> bool {
        self.0.borrow().serialized.is_some()
    }
    pub(super) fn take_serialized(&self) -> Option<structured_clone::SerializedValue> {
        self.0.borrow_mut().serialized.take()
    }
    pub(super) fn clone_serialized(&self, scope: &mut v8::PinScope) -> Option<Self> {
        let value = self
            .0
            .borrow()
            .serialized
            .as_ref()?
            .clone_owned_chunk(scope)?;
        Some(Self::serialized(scope, value))
    }
}
pub(super) trait Handle<T> {
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>>;
}
impl<T> Handle<T> for Owned<T> {
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Owned::get(self, scope)
    }
}
impl<T> Handle<T> for &Owned<T> {
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Owned::get(self, scope)
    }
}
impl<T> Handle<T> for v8::Global<T> {
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Some(v8::Local::new(scope, self))
    }
}
impl<T> Handle<T> for &v8::Global<T> {
    fn get<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Some(v8::Local::new(scope, *self))
    }
}
pub(super) fn local<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: impl Handle<T>,
) -> v8::Local<'s, T> {
    handle
        .get(scope)
        .expect("live stream operation must be retained by its native tracing anchor")
}
trait Trace {
    fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>>;
    fn traced(&self, scope: &mut v8::PinScope);
    fn retire(&self, preserve_serialized: bool);
}
impl<T> Trace for Owned<T>
where
    for<'s> v8::Local<'s, T>: Into<v8::Local<'s, v8::Value>>,
{
    fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.get(scope).map(Into::into)
    }
    fn retire(&self, preserve_serialized: bool) {
        let mut slot = self.0.borrow_mut();
        if !preserve_serialized {
            slot.serialized.take();
        }
        if !slot.primitive {
            slot.strong = None;
        }
    }
    fn traced(&self, scope: &mut v8::PinScope) {
        let Some(value) = self.get(scope) else {
            return;
        };
        let js: v8::Local<v8::Value> = value.into();
        if js.is_object() {
            let weak = v8::Weak::new(scope, value);
            let mut slot = self.0.borrow_mut();
            slot.weak = Some(weak);
            slot.strong = None;
        } else {
            self.0.borrow_mut().primitive = true;
        }
    }
}

/// Detaching a Window retires its document services, not reachable ECMAScript
/// stream objects. A terminated worker cannot run retained algorithms.
pub(super) fn algorithms_available(state: &V8BindingState) -> bool {
    !state.execution_retired() || state.binding_global == BindingGlobal::Window
}

pub(super) fn is_interface(interface: &str) -> bool {
    matches!(
        interface,
        "ReadableStream"
            | "ReadableStreamDefaultController"
            | "ReadableStreamDefaultReader"
            | "ReadableByteStreamController"
            | "ReadableStreamBYOBReader"
            | "ReadableStreamBYOBRequest"
            | "WritableStream"
            | "WritableStreamDefaultController"
            | "WritableStreamDefaultWriter"
            | "TransformStream"
            | "TransformStreamDefaultController"
            | "TextEncoderStream"
            | "TextDecoderStream"
            | "Request"
            | "Response"
    )
}
pub(super) fn owns(state: &V8BindingState, id: u64) -> bool {
    state
        .platform_objects
        .borrow()
        .get(&id)
        .is_some_and(|record| {
            matches!(
                record,
                PlatformObjectRecord::StreamPipe(_)
                    | PlatformObjectRecord::ReadableStream(_)
                    | PlatformObjectRecord::ReadableController { .. }
                    | PlatformObjectRecord::ReadableByobRequest { .. }
                    | PlatformObjectRecord::ReadableReader { .. }
                    | PlatformObjectRecord::WritableStream(_)
                    | PlatformObjectRecord::WritableController { .. }
                    | PlatformObjectRecord::WritableWriter { .. }
                    | PlatformObjectRecord::TransformController { .. }
                    | PlatformObjectRecord::TransformStream { .. }
                    | PlatformObjectRecord::EncodingStream(_)
                    | PlatformObjectRecord::Request(_)
                    | PlatformObjectRecord::Response(_)
            )
        })
}

#[derive(Default)]
struct Entry {
    values: Vec<Box<dyn Trace>>,
    promises: Vec<Owned<v8::PromiseResolver>>,
    links: Vec<u64>,
}
impl Entry {
    fn value<T: 'static>(&mut self, value: &Owned<T>)
    where
        for<'s> v8::Local<'s, T>: Into<v8::Local<'s, v8::Value>>,
    {
        self.values.push(Box::new(value.clone()));
    }
    fn promise(&mut self, value: &Owned<v8::PromiseResolver>) {
        self.value(value);
        self.promises.push(value.clone());
    }
}
fn entries(state: &V8BindingState) -> BTreeMap<u64, Entry> {
    let mut entries = BTreeMap::new();
    let records = state.platform_objects.borrow();
    for (id, record) in records.iter() {
        let mut entry = Entry::default();
        match record {
            PlatformObjectRecord::StreamPipe(pipe) => {
                let pipe = pipe.borrow();
                if pipe.active {
                    entry.links.push(pipe.reader);
                    entry.value(&pipe.destination_wrapper);
                    entry.value(&pipe.writer_wrapper);
                    entry.promise(&pipe.resolver);
                    if let Some(signal) = &pipe.signal {
                        entry.value(signal);
                    }
                    if let Some(reason) = &pipe.reason {
                        entry.value(reason);
                    }
                }
            }
            PlatformObjectRecord::ReadableStream(stream) => {
                entry.links.push(stream.controller_id);
                entry.links.extend(stream.reader_id);
                entry.links.extend(stream.transform_writable_id);
                if let Some((source, _)) = stream.tee_source {
                    entry.links.push(source);
                }
                for value in [&stream.error, &stream.source].into_iter().flatten() {
                    entry.value(value);
                }
                for value in [&stream.pull, &stream.cancel].into_iter().flatten() {
                    entry.value(value);
                }
                for value in &stream.queue {
                    entry.value(value);
                }
                if let Some(bytes) = &stream.bytes {
                    for descriptor in &bytes.pending {
                        entry.value(&descriptor.buffer);
                        if let Some(resolver) = &descriptor.resolver {
                            entry.promise(resolver);
                        }
                        entry.links.extend(descriptor.request);
                    }
                }
                for resolver in &stream.waiting {
                    entry.promise(resolver);
                }
                if let Some(consumer) = &stream.consumer {
                    entry.promise(&consumer.resolver);
                    entry.value(&consumer.source.wrapper);
                }
                if let Some(tee) = &stream.tee {
                    let tee = tee.borrow();
                    entry.links.extend(tee.branches);
                    for value in tee.cancelled.iter().flatten() {
                        entry.value(value);
                    }
                    entry.promise(&tee.cancellation);
                }
            }
            PlatformObjectRecord::WritableStream(stream) => {
                entry.links.push(stream.controller_id);
                entry.links.extend(stream.writer_id);
                entry.value(&stream.source);
                if let Some(error) = &stream.error {
                    entry.value(error);
                }
                if let Some(resolver) = &stream.abort_completion {
                    entry.promise(resolver);
                }
                if let Some(abort) = &stream.pending_abort {
                    entry.value(&abort.reason);
                    entry.promise(&abort.resolver);
                }
                for request in &stream.queue {
                    if let Some(chunk) = &request.chunk {
                        entry.value(chunk);
                    }
                    entry.promise(&request.resolver);
                }
                match &stream.kind {
                    WritableKind::Html(_) => {}
                    WritableKind::Sink {
                        write,
                        close,
                        abort,
                    } => {
                        for value in [write, close, abort].into_iter().flatten() {
                            entry.value(value);
                        }
                    }
                    WritableKind::Transform {
                        transform,
                        flush,
                        controller_id,
                        readable_controller_id,
                        codec_id,
                    } => {
                        entry
                            .links
                            .extend([*controller_id, *readable_controller_id]);
                        entry.links.extend(*codec_id);
                        for value in [transform, flush].into_iter().flatten() {
                            entry.value(value);
                        }
                    }
                }
            }
            PlatformObjectRecord::ReadableController { stream_id }
            | PlatformObjectRecord::WritableController { stream_id } => {
                entry.links.push(*stream_id)
            }
            PlatformObjectRecord::ReadableByobRequest { stream_id, view } => {
                entry.links.extend(*stream_id);
                if let Some(view) = view {
                    entry.value(view);
                }
            }
            PlatformObjectRecord::ReadableReader {
                stream_id,
                closed,
                released,
                ..
            } => {
                if !released {
                    entry.links.push(*stream_id);
                }
                entry.promise(closed);
            }
            PlatformObjectRecord::WritableWriter {
                stream_id,
                ready,
                closed,
                released,
            } => {
                if !released {
                    entry.links.push(*stream_id);
                }
                entry.promise(ready);
                entry.promise(closed);
            }
            PlatformObjectRecord::TransformController {
                readable_id,
                readable_controller_id,
                writable_id,
            } => entry
                .links
                .extend([*readable_id, *readable_controller_id, *writable_id]),
            PlatformObjectRecord::TransformStream {
                readable_id,
                writable_id,
            } => entry.links.extend([*readable_id, *writable_id]),
            PlatformObjectRecord::EncodingStream(stream) => {
                entry.links.extend([stream.readable_id, stream.writable_id])
            }
            PlatformObjectRecord::Response(response) => {
                if let Some(owner) = &response.body_owner {
                    entry.value(&owner.wrapper);
                } else {
                    entry.links.extend(response.body_stream_id);
                }
            }
            PlatformObjectRecord::Request(request) => {
                if let Some(owner) = &request.body_owner {
                    entry.value(&owner.wrapper);
                } else {
                    entry.links.extend(request.body_object_id);
                }
            }
            _ => continue,
        }
        entries.insert(*id, entry);
    }
    entries
}
fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("stream graph key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
/// No author properties or callbacks are consulted while linking these native
/// anchors. Temporary locals protect every anchor until the whole graph is set.
pub(super) fn refresh(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    if !algorithms_available(state) {
        state.stream_activity_roots.borrow_mut().clear();
        return Ok(());
    }
    reclaim(state);
    let entries = entries(state);
    let mut anchors = BTreeMap::new();
    {
        let mut saved = state.stream_anchors.borrow_mut();
        saved.retain(|id, _| entries.contains_key(id));
        for id in entries.keys() {
            let anchor =
                if let Some(anchor) = saved.get(id).and_then(|anchor| anchor.to_local(scope)) {
                    anchor
                } else {
                    let anchor = v8::Object::new(scope);
                    let releases = state.stream_anchor_releases.clone();
                    let id = *id;
                    saved.insert(
                        id,
                        v8::Weak::with_finalizer(
                            scope,
                            anchor,
                            Box::new(move |_| releases.borrow_mut().push(id)),
                        ),
                    );
                    anchor
                };
            anchors.insert(*id, anchor);
        }
    }
    let edges = key(scope, "oryn.stream.edges")?;
    let owner = key(scope, "oryn.stream.owner")?;
    for (id, entry) in &entries {
        let anchor = anchors[id];
        // Keep the previous array local until this native scope ends: a caller
        // may have removed queued values into temporary Rust variables.
        let _previous = anchor.get_private(scope, edges);
        let mut values = entry
            .values
            .iter()
            .filter_map(|value| value.value(scope))
            .collect::<Vec<_>>();
        for link in &entry.links {
            if let Some(anchor) = anchors.get(link) {
                values.push((*anchor).into());
            }
        }
        if let Some(wrapper) = state
            .platform_wrappers
            .borrow()
            .get(id)
            .and_then(|wrapper| wrapper.to_local(scope))
        {
            values.push(wrapper.into());
        }
        let array = v8::Array::new_with_elements(scope, &values);
        anchor
            .set_private(scope, edges, array.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Some(wrapper) = state
            .platform_wrappers
            .borrow()
            .get(id)
            .and_then(|wrapper| wrapper.to_local(scope))
        {
            wrapper
                .set_private(scope, owner, anchor.into())
                .ok_or(V8BindingError::JavaScriptException)?;
        }
        for resolver in &entry.promises {
            if let Some(resolver) = resolver.get(scope) {
                let promise = resolver.get_promise(scope);
                promise
                    .set_private(scope, owner, anchor.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
        }
    }
    // Pending network reads are independent native activity. Pure JS promises
    // and stream algorithms retain each other through the traced graph instead.
    let active = state
        .pending_platform_operations
        .borrow()
        .values()
        .filter_map(|operation| match operation {
            PendingPlatformOperation::BodyRead { stream_object_id } => Some(*stream_object_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut active = active;
    for task in state.deferred_platform_tasks.borrow().iter() {
        if let DeferredPlatformTask::PipeDestinationRetired { pipe_id } = task {
            active.insert(*pipe_id);
        }
    }
    {
        let mut roots = state.stream_activity_roots.borrow_mut();
        roots.retain(|id, _| active.contains(id));
        for id in active {
            if let Some(anchor) = anchors.get(&id) {
                roots.insert(id, v8::Global::new(scope, *anchor));
            }
        }
    }
    // Only release bootstrap strong handles after every edge installation has
    // succeeded. Allocation failure leaves the existing graph safely retained.
    for entry in entries.values() {
        for value in &entry.values {
            value.traced(scope);
        }
    }
    Ok(())
}

pub(super) fn retire(state: &V8BindingState) {
    state.stream_activity_roots.borrow_mut().clear();
    for entry in entries(state).values() {
        for value in &entry.values {
            value.retire(state.binding_global == BindingGlobal::Window);
        }
    }
}

/// Finalizers only enqueue identities. Actual native record removal and host
/// cancellation happen at ordinary checkpoints, never while V8 is collecting.
pub(super) fn reclaim(state: &V8BindingState) {
    let Ok(mut records) = state.platform_objects.try_borrow_mut() else {
        return;
    };
    let releases = std::mem::take(&mut *state.stream_anchor_releases.borrow_mut());
    for id in releases {
        // A live replacement anchor can exist after a deferred metadata borrow;
        // never let an older finalizer remove its current native record.
        if state
            .stream_anchors
            .borrow()
            .get(&id)
            .is_some_and(|anchor| !anchor.is_empty())
        {
            continue;
        }
        if let Some(PlatformObjectRecord::ReadableStream(stream)) = records.remove(&id)
            && let Some(host) = stream.host_stream_id
        {
            state.stream_abort_pending.borrow_mut().insert(host);
        }
        state.stream_anchors.borrow_mut().remove(&id);
        state.stream_activity_roots.borrow_mut().remove(&id);
    }
    drop(records);
    if state.execution_retired() {
        state.stream_abort_pending.borrow_mut().clear();
        return;
    }
    let aborts = state
        .stream_abort_pending
        .borrow()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    for stream_id in aborts {
        if submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        )
        .is_ok()
        {
            state.stream_abort_pending.borrow_mut().remove(&stream_id);
        } else {
            break;
        } // Admission pressure is retried; do not lose cleanup work.
    }
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
    macro_rules! fixture {($scope:ident,$state:ident)=>{
        super::super::tests::initialize_v8();let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));let mut isolate=v8::Isolate::new(Default::default());v8::scope!(let $scope,&mut isolate);let context=v8::Context::new($scope,Default::default());let $scope=&mut v8::ContextScope::new($scope,context);let $state=install_generated_bindings($scope,context,dom,RealmId(0),1).unwrap();
    }}
    fn checkpoint(scope: &mut v8::PinScope, state: &V8BindingState) {
        scope.perform_microtask_checkpoint();
        {
            v8::scope!(let inner,scope);
            refresh(inner, state).unwrap();
        }
    }
    #[test]
    fn unreachable_stream_callback_queue_error_and_transform_cycles_collect() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.streams=[];for(let i=0;i<32;i++){
            let control;const readable=new ReadableStream({start(c){control=c},pull(){return readable}});
            control.enqueue({readable});const reader=readable.getReader();reader.closed.catch(()=>readable);
            const writable=new WritableStream({write(){return writable},abort(){return writable}});
            const writer=writable.getWriter();writer.closed.catch(()=>writer);
            const transform=new TransformStream({transform(chunk,c){c.enqueue({transform})}});
            streams.push([readable,reader,writable,writer,transform]);
        }true"#,
        );
        checkpoint(scope, &state);
        scope.low_memory_notification();
        assert!(
            state
                .stream_anchors
                .borrow()
                .values()
                .any(|anchor| !anchor.is_empty())
        );
        eval(scope, "streams=null;true");
        checkpoint(scope, &state);
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(
            state
                .stream_anchors
                .borrow()
                .values()
                .all(v8::Weak::is_empty)
        );
        reclaim(&state);
        assert!(state.stream_anchors.borrow().is_empty());
        assert!(
            !state
                .platform_objects
                .borrow()
                .values()
                .any(|record| matches!(
                    record,
                    PlatformObjectRecord::ReadableStream(_)
                        | PlatformObjectRecord::WritableStream(_)
                        | PlatformObjectRecord::TransformStream { .. }
                ))
        );
    }
    #[test]
    fn held_reader_promise_and_controller_keep_pending_read_and_callback_identity() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.result='pending';globalThis.pullCalls=0;
          (()=>{const source={start(c){globalThis.control=c},get pull(){return ()=>{pullCalls++}}};
          const stream=new ReadableStream(source);const reader=stream.getReader();
          globalThis.pendingRead=reader.read();pendingRead.then(result=>{globalThis.result=result.value});})();true"#,
        );
        checkpoint(scope, &state);
        let controller = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::ReadableController { .. }).then_some(*id)
            })
            .unwrap();
        eval(scope, "control=null;true");
        scope.low_memory_notification();
        {
            v8::scope!(let inner,scope);
            let value = v8::String::new(inner, "retained").unwrap();
            readable_enqueue(inner, &state, controller, value.into()).unwrap();
            readable_close(inner, &state, controller).unwrap();
            refresh(inner, &state).unwrap();
        }
        checkpoint(scope, &state);
        assert_eq!(eval(scope, "result"), "retained");
        assert_eq!(eval(scope, "String(pullCalls>0)"), "true");
        eval(scope, "control=null;pendingRead=null;true");
        checkpoint(scope, &state);
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(state.stream_activity_roots.borrow().is_empty());
    }
    #[test]
    fn tee_branches_retain_source_and_release_cancellation_graph() {
        fixture!(scope, state);
        eval(
            scope,
            r#"globalThis.cancelled='no';globalThis.parts=(()=>{const stream=new ReadableStream({cancel(reasons){cancelled=reasons.join(',')}});return stream.tee()})();true"#,
        );
        checkpoint(scope, &state);
        scope.low_memory_notification();
        eval(
            scope,
            "globalThis.cancellation=Promise.all([parts[0].cancel('left'),parts[1].cancel('right')]);parts=null;true",
        );
        checkpoint(scope, &state);
        assert_eq!(eval(scope, "cancelled"), "left,right");
        eval(scope, "cancellation=null;true");
        checkpoint(scope, &state);
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(state.stream_activity_roots.borrow().is_empty());
    }
    #[test]
    fn detached_window_codec_graph_survives_gc_only_while_reachable() {
        fixture!(scope, state);
        eval(
            scope,
            "globalThis.result='pending';globalThis.codec=new TextEncoderStream();globalThis.reader=codec.readable.getReader();globalThis.writer=codec.writable.getWriter();globalThis.pending=reader.read().then(r=>result=String(r.value));true",
        );
        checkpoint(scope, &state);
        state.retire_execution();
        scope.low_memory_notification();
        eval(scope, "writer.write('A');true");
        checkpoint(scope, &state);
        assert_eq!(eval(scope, "result"), "65");
        assert!(state.stream_activity_roots.borrow().is_empty());
        eval(
            scope,
            "codec=null;reader=null;writer=null;pending=null;true",
        );
        checkpoint(scope, &state);
        scope.low_memory_notification();
        scope.low_memory_notification();
        reclaim(&state);
        assert!(state.stream_anchors.borrow().is_empty());
        assert!(
            !state
                .platform_objects
                .borrow()
                .values()
                .any(|record| matches!(record, PlatformObjectRecord::EncodingStream(_)))
        );
    }
    #[test]
    fn native_body_read_roots_only_pending_operation_and_retirement_clears_activity() {
        fixture!(scope, state);
        eval(scope, "globalThis.stream=new ReadableStream();true");
        checkpoint(scope, &state);
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::ReadableStream(_)).then_some(*id)
            })
            .unwrap();
        state.pending_platform_operations.borrow_mut().insert(
            HostOperationId(991),
            PendingPlatformOperation::BodyRead {
                stream_object_id: id,
            },
        );
        {
            v8::scope!(let inner,scope);
            refresh(inner, &state).unwrap();
        }
        eval(scope, "stream=null;true");
        scope.low_memory_notification();
        assert!(state.stream_activity_roots.borrow().contains_key(&id));
        state.retire_execution();
        assert!(state.stream_activity_roots.borrow().is_empty());
        scope.perform_microtask_checkpoint();
    }
    #[test]
    fn owned_bootstrap_is_released_only_after_private_edge_installation() {
        fixture!(scope, state);
        let owned;
        {
            v8::scope!(let inner,scope);
            let value = v8::Object::new(inner);
            owned = Owned::new(inner, value);
        }
        scope.low_memory_notification();
        assert!(owned.0.borrow().strong.is_some());
        assert!(owned.get(scope).is_some());
        // The native graph publishes anchors before calling Trace::traced.
        {
            v8::scope!(let inner,scope);
            let value = owned.get(inner).unwrap();
            let holder = v8::Object::new(inner);
            let key = key(inner, "oryn.test.owner").unwrap();
            holder.set_private(inner, key, value.into()).unwrap();
            owned.traced(inner);
            assert!(owned.0.borrow().strong.is_none());
            assert!(owned.get(inner).is_some());
        }
        let _ = state;
    }
}
