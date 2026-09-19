//! Transferable streams use native message endpoints and sequential promise
//! receipts. No author reader, writer, Promise, or MessagePort method is called.
use super::*;
const PULL: u32 = 0;
const CANCEL: u32 = 1;
const WRITE: u32 = 2;
const CLOSE: u32 = 3;
const ABORT: u32 = 4;
const CHUNK: u32 = 5;
const END: u32 = 6;
const ERROR: u32 = 7;
const ACK: u32 = 8;
#[derive(Clone, Copy)]
pub(super) struct Descriptor {
    pub(super) readable: Option<u32>,
    pub(super) writable: Option<u32>,
}
impl Descriptor {
    pub(super) fn ports(self) -> impl Iterator<Item = u32> {
        self.readable.into_iter().chain(self.writable)
    }
}
#[derive(Clone)]
pub(super) struct Source {
    pub(super) owner: Rc<V8BindingState>,
    pub(super) id: u64,
    readable: Option<u64>,
    writable: Option<u64>,
}
struct Leg {
    stream: u64,
    readable: bool,
    source: u64,
    destination: u64,
    port: message_ports::Port,
}
pub(super) struct Plan {
    source: Source,
    legs: Vec<Leg>,
    locks: Vec<(bool, u64)>,
    committed: bool,
}
#[derive(Clone, Copy)]
enum Role {
    ReadSource,
    WriteSource,
    ReadDestination,
    WriteDestination,
}
pub(super) struct Bridge {
    role: Role,
    stream: u64,
    lock: Option<u64>,
    _stream: v8::Global<v8::Object>,
    _port: v8::Global<v8::Object>,
    pending: BTreeMap<u32, v8::Global<v8::PromiseResolver>>,
    requests: BTreeSet<u32>,
}
fn failure() -> V8BindingError {
    V8BindingError::Allocation("stream transfer bridge")
}
pub(super) fn identify(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<Source> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if object.internal_field_count() == 0 {
        return None;
    }
    let owner = binding_state_for_receiver(scope, object)?;
    let id = stateful_id(scope, object).ok()?;
    if !owner
        .platform_wrappers
        .borrow()
        .get(&id)?
        .to_local(scope)?
        .strict_equals(value)
    {
        return None;
    }
    let (readable, writable) = match &*platform_object(&owner, id).ok()? {
        PlatformObjectRecord::ReadableStream(_) => (Some(id), None),
        PlatformObjectRecord::WritableStream(_) => (None, Some(id)),
        PlatformObjectRecord::TransformStream {
            readable_id,
            writable_id,
        } => (Some(*readable_id), Some(*writable_id)),
        _ => return None,
    };
    Some(Source {
        owner,
        id,
        readable,
        writable,
    })
}
impl Source {
    pub(super) fn valid(&self) -> bool {
        !self.owner.execution_retired()&&self.readable.is_none_or(|id|matches!(self.owner.platform_objects.borrow().get(&id),Some(PlatformObjectRecord::ReadableStream(stream))if !stream.locked))&&self.writable.is_none_or(|id|matches!(self.owner.platform_objects.borrow().get(&id),Some(PlatformObjectRecord::WritableStream(stream))if !stream.locked))
    }
}
impl Plan {
    pub(super) fn prepare(source: Source) -> Result<Self, V8BindingError> {
        let mut plan = Self {
            source,
            legs: Vec::new(),
            locks: Vec::new(),
            committed: false,
        };
        for (stream, readable) in plan
            .source
            .readable
            .map(|id| (id, true))
            .into_iter()
            .chain(plan.source.writable.map(|id| (id, false)))
        {
            let (local, remote) = message_ports::channel(&plan.source.owner)?;
            let port = match &*platform_object(&plan.source.owner, remote)? {
                PlatformObjectRecord::MessagePort(port) => port.clone(),
                _ => return Err(failure()),
            };
            plan.legs.push(Leg {
                stream,
                readable,
                source: local,
                destination: remote,
                port,
            });
        }
        Ok(plan)
    }
    pub(super) fn source(&self) -> &Source {
        &self.source
    }
    pub(super) fn append_ports(
        &self,
        ports: &mut Vec<(Rc<V8BindingState>, u64, message_ports::Port)>,
    ) -> Descriptor {
        let mut descriptor = Descriptor {
            readable: None,
            writable: None,
        };
        for leg in &self.legs {
            let index = ports.len() as u32;
            ports.push((self.source.owner.clone(), leg.destination, leg.port.clone()));
            if leg.readable {
                descriptor.readable = Some(index)
            } else {
                descriptor.writable = Some(index)
            }
        }
        descriptor
    }
    pub(super) fn commit(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        if !self.source.valid() {
            return Err(failure());
        }
        let original = self
            .source
            .readable
            .or(self.source.writable)
            .ok_or_else(failure)?;
        let context = match &*platform_object(&self.source.owner, original)? {
            PlatformObjectRecord::ReadableStream(stream) => stream.creation_context.to_local(scope),
            PlatformObjectRecord::WritableStream(stream) => stream.creation_context.to_local(scope),
            _ => None,
        }
        .ok_or_else(failure)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let result = (|| -> Result<(), V8BindingError> {
            for leg in &self.legs {
                let owner = &self.source.owner;
                let lock = if leg.readable {
                    readable_streams::acquire(scope, owner, leg.stream)?
                } else {
                    writable_writers::acquire(scope, owner, leg.stream)?
                };
                self.locks.push((leg.readable, lock));
                let interface = if leg.readable {
                    "ReadableStream"
                } else {
                    "WritableStream"
                };
                let stream = new_platform_object(scope, owner, interface, leg.stream)?;
                let port_object = new_platform_object(scope, owner, "MessagePort", leg.source)?;
                let bridge = Bridge {
                    role: if leg.readable {
                        Role::ReadSource
                    } else {
                        Role::WriteSource
                    },
                    stream: leg.stream,
                    lock: Some(lock),
                    _stream: v8::Global::new(scope, stream),
                    _port: v8::Global::new(scope, port_object),
                    pending: BTreeMap::new(),
                    requests: BTreeSet::new(),
                };
                attach(scope, owner, leg.source, bridge)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.rollback(scope);
            return Err(error);
        }
        self.committed = true;
        Ok(())
    }
    pub(super) fn rollback(&mut self, scope: &mut v8::PinScope) {
        for leg in &self.legs {
            self.source
                .owner
                .stream_transfers
                .borrow_mut()
                .remove(&leg.source);
        }
        for (readable, lock) in self.locks.drain(..) {
            if readable {
                let _ = readable_streams::release(scope, &self.source.owner, lock);
            } else {
                let _ = writable_writers::release(scope, &self.source.owner, lock);
            }
        }
        self.committed = false;
    }
}
impl Drop for Plan {
    fn drop(&mut self) {
        if !self.committed {
            for leg in &self.legs {
                let _ = message_ports::close(&self.source.owner, leg.source);
                let _ = message_ports::close(&self.source.owner, leg.destination);
            }
        }
    }
}
fn callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    port: u64,
    kind: u32,
) -> Result<v8::Local<'s, v8::Function>, V8BindingError> {
    let port = new_platform_object(scope, state, "MessagePort", port)?;
    let kind = v8::Integer::new_from_unsigned(scope, kind);
    let data = v8::Array::new_with_elements(scope, &[port.into(), kind.into()]);
    v8::Function::builder(operation)
        .data(data.into())
        .build(scope)
        .ok_or_else(failure)
}
fn attach(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    port: u64,
    bridge: Bridge,
) -> Result<(), V8BindingError> {
    let target = new_platform_object(scope, state, "MessagePort", port)?;
    let handler = v8::Function::builder(message)
        .data(target.into())
        .build(scope)
        .ok_or_else(failure)?;
    let registration = state.next_event_listener_id.get();
    state
        .next_event_listener_id
        .set(registration.checked_add(1).ok_or_else(failure)?);
    state
        .stream_transfers
        .borrow_mut()
        .insert(port, Rc::new(RefCell::new(bridge)));
    for name in ["message", "messageerror", "close"] {
        state
            .platform_listeners
            .borrow_mut()
            .entry((port, name.into()))
            .or_default()
            .push(PlatformEventListener {
                registration_id: registration,
                global_handler: false,
                callback: platform_listener_graph::Callback::new(scope, handler.into()),
                capture: false,
                once: false,
                passive: false,
                signal: None,
            });
    }
    message_ports::start(state, port)?;
    platform_listener_graph::refresh(scope, state, port)?;
    Ok(())
}
fn bridge(state: &V8BindingState, port: u64) -> Option<Rc<RefCell<Bridge>>> {
    state.stream_transfers.borrow().get(&port).cloned()
}
fn send(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    port: u64,
    kind: u32,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    v8::tc_scope!(let tc,scope);
    let kind = v8::Integer::new_from_unsigned(tc, kind);
    let packet = v8::Array::new_with_elements(tc, &[kind.into(), value]);
    let options = v8::undefined(tc).into();
    let result = message_ports::post(tc, state, port, packet.into(), options);
    if tc.has_caught() {
        return Err(failure());
    }
    result
}
fn observe(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    port: u64,
    kind: u32,
    promise: v8::Local<v8::Promise>,
) -> Result<(), V8BindingError> {
    let target = new_platform_object(scope, state, "MessagePort", port)?;
    let mut callbacks = Vec::new();
    for rejected in [false, true] {
        let kind = v8::Integer::new_from_unsigned(scope, kind);
        let rejected = v8::Boolean::new(scope, rejected);
        let data =
            v8::Array::new_with_elements(scope, &[target.into(), kind.into(), rejected.into()]);
        callbacks.push(
            v8::Function::builder(reaction)
                .data(data.into())
                .build(scope)
                .ok_or_else(failure)?,
        );
    }
    promise
        .then2(scope, callbacks[0], callbacks[1])
        .ok_or_else(failure)?;
    Ok(())
}
fn retire_one(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    port: u64,
) -> Result<(), V8BindingError> {
    let Some(bridge) = state.stream_transfers.borrow_mut().remove(&port) else {
        return Ok(());
    };
    let bridge = bridge.borrow();
    if let Some(lock) = bridge.lock {
        match bridge.role {
            Role::ReadSource => readable_streams::release(scope, state, lock)?,
            Role::WriteSource => writable_writers::release(scope, state, lock)?,
            _ => {}
        }
    }
    drop(bridge);
    message_ports::close(state, port)
}
pub(super) fn retire(state: &V8BindingState) {
    state.stream_transfers.borrow_mut().clear();
}
pub(super) struct DecodeGuard {
    state: Rc<V8BindingState>,
    before: BTreeSet<u64>,
    committed: bool,
}
impl DecodeGuard {
    pub(super) fn new(state: &Rc<V8BindingState>) -> Self {
        Self {
            state: state.clone(),
            before: state.stream_transfers.borrow().keys().copied().collect(),
            committed: false,
        }
    }
    pub(super) fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for DecodeGuard {
    fn drop(&mut self) {
        if !self.committed {
            let added = self
                .state
                .stream_transfers
                .borrow()
                .keys()
                .filter(|id| !self.before.contains(id))
                .copied()
                .collect::<Vec<_>>();
            for port in added {
                self.state.stream_transfers.borrow_mut().remove(&port);
                let _ = message_ports::close(&self.state, port);
            }
        }
    }
}

fn cancel_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    stream: u64,
    reason: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let reader = match &*platform_object(state, stream)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.reader_id.ok_or_else(failure)?,
        _ => return Err(failure()),
    };
    stream_pipe::cancel_source(scope, state, reader, reason)
}
fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    port: u64,
    reason: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let Some(record) = bridge(state, port) else {
        return Ok(());
    };
    let (role, stream, pending) = {
        let mut bridge = record.borrow_mut();
        (
            bridge.role,
            bridge.stream,
            std::mem::take(&mut bridge.pending),
        )
    };
    for pending in pending.into_values() {
        v8::Local::new(scope, &pending).reject(scope, reason);
    }
    match role {
        Role::ReadDestination => readable_streams::error(scope, state, stream, reason)?,
        Role::WriteDestination => writable_streams::error(scope, state, stream, reason)?,
        Role::ReadSource => {
            cancel_source(scope, state, stream, reason)?.mark_as_handled();
        }
        Role::WriteSource => {
            writable_streams::abort(scope, state, stream, reason)?.mark_as_handled();
        }
    }
    retire_one(scope, state, port)
}
fn operation(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Promise>, V8BindingError> {
        let data = v8::Local::<v8::Array>::try_from(args.data()).map_err(|_| failure())?;
        let target = data
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(failure)?;
        let state = binding_state_for_receiver(scope, target).ok_or_else(failure)?;
        let port = stateful_id(scope, target)?;
        let kind = data
            .get_index(scope, 1)
            .and_then(|value| value.uint32_value(scope))
            .ok_or_else(failure)?;
        let resolver = v8::PromiseResolver::new(scope).ok_or_else(failure)?;
        let promise = resolver.get_promise(scope);
        let Some(record) = bridge(&state, port) else {
            let error = readable_streams::type_error(scope, "transferred stream is retired")?;
            resolver.reject(scope, error);
            return Ok(promise);
        };
        {
            let mut bridge = record.borrow_mut();
            if bridge.pending.contains_key(&kind) {
                return Err(failure());
            }
            bridge
                .pending
                .insert(kind, v8::Global::new(scope, resolver));
        }
        if send(scope, &state, port, kind, args.get(0)).is_err() {
            let error = dom_exception_value(scope, "DataCloneError")?;
            let _ = send(scope, &state, port, ERROR, error);
            fail(scope, &state, port, error)?;
        }
        Ok(promise)
    })();
    match result {
        Ok(promise) => rv.set(promise.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn reaction(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let data = v8::Local::<v8::Array>::try_from(args.data()).map_err(|_| failure())?;
        let target = data
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(failure)?;
        let state = binding_state_for_receiver(scope, target).ok_or_else(failure)?;
        let port = stateful_id(scope, target)?;
        let kind = data
            .get_index(scope, 1)
            .and_then(|value| value.uint32_value(scope))
            .ok_or_else(failure)?;
        let rejected = data
            .get_index(scope, 2)
            .ok_or_else(failure)?
            .boolean_value(scope);
        let Some(record) = bridge(&state, port) else {
            return Ok(());
        };
        record.borrow_mut().requests.remove(&kind);
        if rejected {
            let _ = send(scope, &state, port, ERROR, args.get(0));
            return retire_one(scope, &state, port);
        }
        let mut terminal = kind != PULL && kind != WRITE;
        let mut code = ACK + kind;
        let mut value = args.get(0);
        if kind == PULL {
            let result = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure())?;
            let key = v8::String::new(scope, "done").ok_or_else(failure)?;
            terminal = result
                .get(scope, key.into())
                .ok_or_else(failure)?
                .boolean_value(scope);
            code = if terminal { END } else { CHUNK };
            let key = v8::String::new(scope, "value").ok_or_else(failure)?;
            value = result.get(scope, key.into()).ok_or_else(failure)?;
        }
        if send(scope, &state, port, code, value).is_err() {
            let error = dom_exception_value(scope, "DataCloneError")?;
            let _ = send(scope, &state, port, ERROR, error);
            return fail(scope, &state, port, error);
        }
        if terminal && record.borrow().requests.is_empty() {
            retire_one(scope, &state, port)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn message(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let target = v8::Local::<v8::Object>::try_from(args.data()).map_err(|_| failure())?;
        let state = binding_state_for_receiver(scope, target).ok_or_else(failure)?;
        let port = stateful_id(scope, target)?;
        let Some(record) = bridge(&state, port) else {
            return Ok(());
        };
        let event = v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_| failure())?;
        let event_id = event_id(scope, event)?;
        if !state
            .events
            .borrow()
            .get(&event_id)
            .is_some_and(|event| event.event_type == "message")
        {
            let error = dom_exception_value(scope, "DataCloneError")?;
            return fail(scope, &state, port, error);
        }
        let fields = event
            .get_internal_field(scope, 2)
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .ok_or_else(failure)?;
        let packet = fields
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .ok_or_else(failure)?;
        let kind = packet
            .get_index(scope, 0)
            .and_then(|value| value.uint32_value(scope))
            .ok_or_else(failure)?;
        let value = packet.get_index(scope, 1).ok_or_else(failure)?;
        if kind == ERROR {
            return fail(scope, &state, port, value);
        }
        let (role, stream, lock) = {
            let bridge = record.borrow();
            (bridge.role, bridge.stream, bridge.lock)
        };
        match role {
            Role::ReadSource | Role::WriteSource => {
                if !record.borrow_mut().requests.insert(kind) {
                    return Err(failure());
                }
                let promise = match (role, kind) {
                    (Role::ReadSource, PULL) => {
                        let promise = read_native_stream(scope, &state, lock.ok_or_else(failure)?)?
                            .ok_or_else(failure)?;
                        v8::Local::<v8::Promise>::try_from(promise).map_err(|_| failure())?
                    }
                    (Role::ReadSource, CANCEL) => cancel_source(scope, &state, stream, value)?,
                    (Role::WriteSource, WRITE) => writable_write(scope, &state, stream, value)?,
                    (Role::WriteSource, CLOSE) => writable_close(scope, &state, stream)?,
                    (Role::WriteSource, ABORT) => {
                        writable_streams::abort(scope, &state, stream, value)?
                    }
                    _ => return Err(failure()),
                };
                observe(scope, &state, port, kind, promise)?;
            }
            Role::ReadDestination | Role::WriteDestination => {
                let pending_kind = if matches!(kind, CHUNK | END) {
                    PULL
                } else {
                    kind.checked_sub(ACK).ok_or_else(failure)?
                };
                let pending = record.borrow_mut().pending.remove(&pending_kind);
                if matches!(role, Role::ReadDestination) {
                    let (controller, closed) = match &*platform_object(&state, stream)? {
                        PlatformObjectRecord::ReadableStream(stream) => {
                            (stream.controller_id, stream.closed)
                        }
                        _ => return Err(failure()),
                    };
                    match kind {
                        CHUNK => {
                            if !closed {
                                readable_enqueue(scope, &state, controller, value)?;
                            }
                        }
                        END => {
                            if !closed {
                                readable_close(scope, &state, controller)?;
                            }
                        }
                        kind if kind == ACK + CANCEL => {}
                        _ => return Err(failure()),
                    }
                } else if !matches!(pending_kind, WRITE | CLOSE | ABORT) {
                    return Err(failure());
                }
                if let Some(pending) = pending {
                    let value = v8::undefined(scope).into();
                    v8::Local::new(scope, &pending).resolve(scope, value);
                }
                if (kind == END || matches!(pending_kind, CANCEL | CLOSE | ABORT))
                    && record.borrow().pending.is_empty()
                {
                    retire_one(scope, &state, port)?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}

pub(super) fn receive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    descriptor: Descriptor,
    ports: &[v8::Global<v8::Object>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let mut readable = None;
    let mut writable = None;
    for (index, is_readable) in descriptor
        .readable
        .map(|id| (id, true))
        .into_iter()
        .chain(descriptor.writable.map(|id| (id, false)))
    {
        let port = v8::Local::new(scope, ports.get(index as usize).ok_or_else(failure)?);
        let port_id = stateful_id(scope, port)?;
        let stream = reserve_platform_object_id(state);
        let controller = reserve_platform_object_id(state);
        let undefined: v8::Local<v8::Value> = v8::undefined(scope).into();
        if is_readable {
            let pull = callback(scope, state, port_id, PULL)?;
            let cancel = callback(scope, state, port_id, CANCEL)?;
            insert_platform_object(
                state,
                controller,
                PlatformObjectRecord::ReadableController { stream_id: stream },
            );
            insert_platform_object(
                state,
                stream,
                PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
                    owning: false,
                    bytes: None,
                    tee: None,
                    tee_source: None,
                    consumer: None,
                    disturbed: false,
                    creation_context: weak_current_context(scope),
                    error: None,
                    reader_id: None,
                    transform_writable_id: None,
                    high_water_mark: 0.0,
                    started: true,
                    pulling: false,
                    pull_again: false,
                    source: Some(stream_graph::Owned::new(scope, undefined)),
                    queue: VecDeque::new(),
                    waiting: VecDeque::new(),
                    closed: false,
                    locked: false,
                    pull: Some(stream_graph::Owned::new(scope, pull)),
                    cancel: Some(stream_graph::Owned::new(scope, cancel)),
                    controller_id: controller,
                    host_stream_id: None,
                    host_read_pending: false,
                }),
            );
            readable = Some(stream);
        } else {
            let write = callback(scope, state, port_id, WRITE)?;
            let close = callback(scope, state, port_id, CLOSE)?;
            let abort = callback(scope, state, port_id, ABORT)?;
            insert_platform_object(
                state,
                controller,
                PlatformObjectRecord::WritableController { stream_id: stream },
            );
            insert_platform_object(
                state,
                stream,
                PlatformObjectRecord::WritableStream(WritableStreamRecord {
                    creation_context: weak_current_context(scope),
                    locked: false,
                    closed: false,
                    kind: WritableKind::Sink {
                        write: Some(stream_graph::Owned::new(scope, write)),
                        close: Some(stream_graph::Owned::new(scope, close)),
                        abort: Some(stream_graph::Owned::new(scope, abort)),
                    },
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
            writable = Some(stream);
        }
        let object = new_platform_object(
            scope,
            state,
            if is_readable {
                "ReadableStream"
            } else {
                "WritableStream"
            },
            stream,
        )?;
        let bridge = Bridge {
            role: if is_readable {
                Role::ReadDestination
            } else {
                Role::WriteDestination
            },
            stream,
            lock: None,
            _stream: v8::Global::new(scope, object),
            _port: v8::Global::new(scope, port),
            pending: BTreeMap::new(),
            requests: BTreeSet::new(),
        };
        attach(scope, state, port_id, bridge)?;
    }
    let object = match (readable, writable) {
        (Some(readable_id), Some(writable_id)) => {
            let id = register_platform_object(
                state,
                PlatformObjectRecord::TransformStream {
                    readable_id,
                    writable_id,
                },
            );
            new_platform_object(scope, state, "TransformStream", id)?
        }
        (Some(id), None) => new_platform_object(scope, state, "ReadableStream", id)?,
        (None, Some(id)) => new_platform_object(scope, state, "WritableStream", id)?,
        _ => return Err(failure()),
    };
    stream_graph::refresh(scope, state)?;
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_wire::{MessageHub, MessageRequest, MessageResponse, MessageTransport};
    struct Messages(std::sync::Mutex<MessageHub>);
    impl MessageTransport for Messages {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            self.0.lock().unwrap().handle(
                1,
                request,
                &BTreeMap::from([(1, "https://a.test".into())]),
            )
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn drain(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) {
        for _ in 0..64 {
            message_ports::poll_external(scope, state).unwrap();
            state.drain_platform_completions(scope, 64).unwrap();
            scope.perform_microtask_checkpoint();
        }
    }
    #[test]
    fn readable_writable_and_transform_transfer_preserve_backpressure_and_hidden_ports() {
        super::super::tests::initialize_v8();
        for native in [false, true] {
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            if native {
                state.set_message_transport(std::sync::Arc::new(Messages(std::sync::Mutex::new(
                    MessageHub::default(),
                ))));
            }
            eval(
                scope,
                r#"globalThis.original=new ReadableStream({start(c){c.enqueue({value:7});c.close()}});
              globalThis.writes=[];globalThis.sink=new WritableStream({write(v){writes.push(v);return new Promise(r=>globalThis.releaseWrite=r)},close(){writes.push('closed')}});
              globalThis.transform=new TransformStream({transform(v,c){c.enqueue(v+1)}});globalThis.events=[];globalThis.failure='';
              globalThis.carrier=new MessageChannel();carrier.port1.onmessage=async e=>{
                try{
                  if(e.ports.length!==0)throw Error('hidden transfer ports exposed');
                  const {readable,writable,transform}=e.data;
                  const reader=readable.getReader();const first=await reader.read();events.push(first.value.value);events.push((await reader.read()).done);
                  const writer=writable.getWriter();await writer.write('one');events.push('written');await writer.close();
                  const output=transform.readable.getReader(),input=transform.writable.getWriter();const read=output.read();await input.write(8);events.push((await read).value);await input.close();events.push((await output.read()).done);
                }catch(e){failure=String(e)}
              };
              carrier.port2.postMessage({readable:original,writable:sink,transform},[original,sink,transform]);
              if(!original.locked||!sink.locked||!transform.readable.locked||!transform.writable.locked)throw Error('source locks');
            "#,
            );
            drain(scope, &state);
            assert!(
                eval(
                    scope,
                    "failure===''&&writes.join(',')==='one'&&events.join(',')==='7,true'"
                )
                .is_true()
            );
            eval(scope, "releaseWrite()");
            drain(scope, &state);
            assert!(eval(scope,"failure===''&&writes.join(',')==='one,closed'&&events.join(',')==='7,true,written,9,true'").is_true());
            assert!(state.stream_transfers.borrow().is_empty());
        }
    }
    #[test]
    fn stream_transfer_validation_precedes_locking_and_getter_failures_roll_back() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const source=new ReadableStream(),sink=new WritableStream(),marker={};
            try{structuredClone({get x(){if(source.locked||sink.locked)throw Error('early lock');throw marker}},{transfer:[source,sink]})}catch(e){if(e!==marker)throw e}
            if(source.locked||sink.locked)throw Error('failed transfer locked source');
            const reader=source.getReader();try{structuredClone(source,{transfer:[source]});throw Error('locked transfer accepted')}catch(e){if(e.name!=='DataCloneError')throw e}reader.releaseLock();
            for(const list of [[source,source],[source,new Blob()]]){try{structuredClone(source,{transfer:list});throw Error('invalid list accepted')}catch(e){if(e.name!=='DataCloneError')throw e}}
            if(source.locked)throw Error('invalid transfer partially committed');
            const transformed=new TransformStream();Object.defineProperty(transformed,'readable',{get(){throw marker}});Object.defineProperty(transformed,'writable',{get(){throw marker}});
            const copy=structuredClone(transformed,{transfer:[transformed]});return copy instanceof TransformStream&&copy.readable instanceof ReadableStream&&copy.writable instanceof WritableStream;
        })()"#).is_true());
        state.retire_execution();
    }
    #[test]
    fn transferred_read_cancellation_and_uncloneable_chunks_settle_both_sides() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.cancelled=null;globalThis.cancelDone=false;globalThis.readDone=false;
          const source=new ReadableStream({cancel(reason){cancelled=reason}});globalThis.copy=structuredClone(source,{transfer:[source]});globalThis.reader=copy.getReader();reader.read().then(r=>readDone=r.done);reader.cancel({why:3}).then(()=>cancelDone=true);
          globalThis.badSource=new ReadableStream({start(c){c.enqueue(()=>{})}});const bad=structuredClone(badSource,{transfer:[badSource]});globalThis.badError='';bad.getReader().read().catch(e=>badError=e.name);
        "#,
        );
        drain(scope, &state);
        assert!(eval(scope,"cancelDone&&readDone&&cancelled.why===3&&badError==='DataCloneError'&&!badSource.locked").is_true());
        assert!(state.stream_transfers.borrow().is_empty());
    }
}
