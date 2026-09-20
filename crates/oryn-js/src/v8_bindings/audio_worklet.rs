//! Native processor registry in an independently owned AudioWorklet realm.
use super::*;
use std::sync::Arc;
#[derive(Default)]
struct Global {
    constructors: RefCell<BTreeMap<String, v8::Global<v8::Function>>>,
    processors: RefCell<BTreeMap<u64, v8::Global<v8::Object>>>,
    constructing: RefCell<Option<v8::Global<v8::Object>>>,
}
pub fn definitions(scope: &mut v8::PinScope) -> Vec<String> {
    scope
        .get_current_context()
        .get_slot::<Global>()
        .map(|global| global.constructors.borrow().keys().cloned().collect())
        .unwrap_or_default()
}
fn failure(message: &str) -> V8BindingError {
    V8BindingError::Message(message.into())
}
pub fn install(scope: &mut v8::PinScope, sample_rate: f64) -> Result<(), V8BindingError> {
    let context = scope.get_current_context();
    context.set_slot(Rc::new(Global::default()));
    for (name, value) in [
        ("sampleRate", sample_rate),
        ("currentTime", 0.),
        ("currentFrame", 0.),
    ] {
        let key = v8::String::new(scope, name).unwrap();
        let value = v8::Number::new(scope, value);
        context
            .global(scope)
            .define_own_property(
                scope,
                key.into(),
                value.into(),
                v8::PropertyAttribute::READ_ONLY,
            )
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    let register_function = v8::Function::new(scope, register)
        .ok_or(V8BindingError::Allocation("registerProcessor"))?;
    let processor_function = v8::Function::new(scope, processor)
        .ok_or(V8BindingError::Allocation("AudioWorkletProcessor"))?;
    for (name, function) in [
        ("registerProcessor", register_function),
        ("AudioWorkletProcessor", processor_function),
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("AudioWorklet function name"))?;
        context
            .global(scope)
            .define_own_property(
                scope,
                key.into(),
                function.into(),
                v8::PropertyAttribute::NONE,
            )
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(())
}
fn register(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let global = scope
            .get_current_context()
            .get_slot::<Global>()
            .ok_or(failure("AudioWorklet registry unavailable"))?;
        let name = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        if name.is_empty() || name.len() > 65536 || global.constructors.borrow().contains_key(&name)
        {
            return Err(V8BindingError::Unsupported {
                interface: "AudioWorkletGlobalScope".into(),
                member: "NotSupportedError".into(),
            });
        }
        let constructor = v8::Local::<v8::Function>::try_from(args.get(1))
            .map_err(|_| failure("TypeError: processor constructor"))?;
        if !crate::v8_context_bridge::is_constructor(constructor.into()) {
            return Err(failure("TypeError: processor must be a constructor"));
        }
        let key = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        let prototype = v8::Local::<v8::Object>::try_from(prototype)
            .map_err(|_| failure("TypeError: processor prototype"))?;
        let key = v8::String::new(scope, "process").unwrap();
        if !prototype
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?
            .is_function()
        {
            return Err(failure("TypeError: processor process must be callable"));
        }
        if global.constructors.borrow().len() >= 256 {
            return Err(failure("QuotaExceededError: processor registry"));
        }
        global
            .constructors
            .borrow_mut()
            .insert(name, v8::Global::new(scope, constructor));
        Ok(())
    })();
    if let Err(error) = result
        && !scope.is_execution_terminating()
    {
        throw_binding_error(scope, &error);
    }
}
fn processor(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut result: v8::ReturnValue,
) {
    let global = scope.get_current_context().get_slot::<Global>();
    let port = global
        .as_ref()
        .and_then(|global| global.constructing.borrow_mut().take());
    let Some(port) = port else {
        throw_type_error(scope, "Illegal AudioWorkletProcessor constructor");
        return;
    };
    let key = v8::String::new(scope, "port").unwrap();
    let port = v8::Local::new(scope, port);
    args.this().define_own_property(
        scope,
        key.into(),
        port.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    result.set(args.this().into());
}
pub fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    name: &str,
    value: crate::message_wire::WireValue,
) -> Result<(), V8BindingError> {
    let mut cleanup = structured_clone::shared_receive_guard(state, &value);
    let global = scope
        .get_current_context()
        .get_slot::<Global>()
        .ok_or(failure("AudioWorklet registry unavailable"))?;
    if global.processors.borrow().contains_key(&id) || global.processors.borrow().len() >= 256 {
        return Err(failure("AudioWorklet processor identity/capacity"));
    }
    let constructor = global
        .constructors
        .borrow()
        .get(name)
        .cloned()
        .ok_or(failure("InvalidStateError: processor not registered"))?;
    cleanup.handoff_ports();
    let (options, ports) = structured_clone::deserialize_wire(scope, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    if ports.len() != 1 {
        return Err(failure("AudioWorklet processor port missing"));
    }
    *global.constructing.borrow_mut() = Some(v8::Global::new(scope, ports[0]));
    let constructor = v8::Local::new(scope, constructor);
    let object = constructor.new_instance(scope, &[options]);
    global.constructing.borrow_mut().take();
    let object = object.ok_or(V8BindingError::JavaScriptException)?;
    global
        .processors
        .borrow_mut()
        .insert(id, v8::Global::new(scope, object));
    Ok(())
}

use crate::worker_wire::{
    AgentConnection, AgentCreation, AgentEvent, AgentKind, AgentServiceRequest,
    AgentServiceResponse, ScriptType, WorkerOptions, WorkerTransport, WorkletCommand,
};
pub(super) struct Context {
    wrapper: u64,
    definitions: BTreeSet<String>,
    nodes: BTreeMap<u64, u64>,
    offline: bool,
    worklet: Option<u64>,
    sample_rate: f64,
    length: u32,
    transport: Arc<dyn WorkerTransport>,
    connection: Option<AgentConnection>,
    first: Option<u64>,
    pending: BTreeMap<u64, v8::Global<v8::PromiseResolver>>,
    events: VecDeque<AgentEvent>,
    closed: bool,
}
impl Drop for Context {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = self
                .transport
                .request(AgentServiceRequest::Release { connection });
        }
    }
}
#[derive(Default)]
pub(super) struct WindowState {
    contexts: Vec<Rc<RefCell<Context>>>,
}
pub(super) enum Object {
    Context(Rc<RefCell<Context>>),
    Worklet(Rc<RefCell<Context>>),
    Node {
        context: Rc<RefCell<Context>>,
        port: u64,
    },
}
impl Object {
    pub(super) fn interface(&self) -> &'static str {
        match self {
            Self::Context(c) => {
                if c.borrow().offline {
                    "OfflineAudioContext"
                } else {
                    "AudioContext"
                }
            }
            Self::Worklet(_) => "AudioWorklet",
            Self::Node { .. } => "AudioWorkletNode",
        }
    }
}
pub(super) fn related(object: &Object) -> Vec<u64> {
    match object {
        Object::Context(context) => context.borrow().worklet.into_iter().collect(),
        Object::Worklet(context) => vec![context.borrow().wrapper],
        Object::Node { context, port } => vec![context.borrow().wrapper, *port],
    }
}
fn context_for(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Context>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::AudioWorklet(
            Object::Context(c) | Object::Worklet(c) | Object::Node { context: c, .. },
        ) => Ok(c.clone()),
        _ => Err(failure("TypeError: audio context receiver")),
    }
}
fn identity(state: &V8BindingState) -> Result<u64, V8BindingError> {
    let id = state.next_platform_operation_id.get();
    state.next_platform_operation_id.set(
        id.checked_add(1)
            .ok_or(failure("audio operation identity exhausted"))?,
    );
    Ok(id)
}
pub(super) fn construct_context(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
    offline: bool,
) -> Result<(), V8BindingError> {
    let channels = if !offline {
        2
    } else {
        args.get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    let length = if !offline {
        1
    } else {
        args.get(1)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    let sample_rate = if !offline {
        let options = args.get(0);
        if options.is_object() {
            let object = options
                .to_object(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let key = v8::String::new(scope, "sampleRate").unwrap();
            let value = object
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            if value.is_undefined() {
                48000.
            } else {
                value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
            }
        } else {
            48000.
        }
    } else {
        args.get(2)
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    if channels == 0
        || channels > 32
        || length == 0
        || !sample_rate.is_finite()
        || !(8000.0..=96000.0).contains(&sample_rate)
    {
        return Err(V8BindingError::Unsupported {
            interface: "OfflineAudioContext".into(),
            member: "NotSupportedError".into(),
        });
    }
    state.audio_worklet.borrow_mut().contexts.retain(|context| {
        let context = context.borrow();
        !context.closed || context.connection.is_some()
    });
    if state.audio_worklet.borrow().contexts.len() >= 32 {
        return Err(failure("QuotaExceededError: audio contexts"));
    }
    let transport = state
        .worker_transport
        .borrow()
        .clone()
        .ok_or(failure("InvalidStateError: audio worklet transport"))?;
    let context = Rc::new(RefCell::new(Context {
        wrapper: 0,
        definitions: BTreeSet::new(),
        nodes: BTreeMap::new(),
        offline,
        worklet: None,
        sample_rate,
        length,
        transport,
        connection: None,
        first: None,
        pending: BTreeMap::new(),
        events: VecDeque::new(),
        closed: false,
    }));
    let id = register_platform_object(
        state,
        PlatformObjectRecord::AudioWorklet(Object::Context(context.clone())),
    );
    bind_platform_object(scope, state, id, args.this());
    context.borrow_mut().wrapper = id;
    state.audio_worklet.borrow_mut().contexts.push(context);
    Ok(())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let context = context_for(state, id)?;
    match member {
        "audioWorklet" => {
            let existing = context.borrow().worklet;
            let id = if let Some(id) = existing {
                id
            } else {
                let id = register_platform_object(
                    state,
                    PlatformObjectRecord::AudioWorklet(Object::Worklet(context.clone())),
                );
                context.borrow_mut().worklet = Some(id);
                id
            };
            Ok(new_platform_object(scope, state, "AudioWorklet", id)?.into())
        }
        "sampleRate" => Ok(v8::Number::new(scope, context.borrow().sample_rate).into()),
        "length" => Ok(v8::Integer::new_from_unsigned(scope, context.borrow().length).into()),
        "currentTime" => Ok(v8::Number::new(scope, 0.).into()),
        "state" => v8_string(
            scope,
            if context.borrow().closed {
                "closed"
            } else {
                "suspended"
            },
        ),
        "port" => {
            let port = match &*platform_object(state, id)? {
                PlatformObjectRecord::AudioWorklet(Object::Node { port, .. }) => *port,
                _ => return Err(failure("TypeError: AudioWorkletNode")),
            };
            Ok(new_platform_object(scope, state, "MessagePort", port)?.into())
        }
        _ => Err(failure("unsupported audio property")),
    }
}
pub(super) fn add_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let input = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let credentials = if args.get(1).is_object() {
        let object = args
            .get(1)
            .to_object(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let key = v8::String::new(scope, "credentials").unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if value.is_undefined() {
            crate::worker_wire::WorkerCredentials::SameOrigin
        } else {
            match value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
                .as_str()
            {
                "omit" => crate::worker_wire::WorkerCredentials::Omit,
                "include" => crate::worker_wire::WorkerCredentials::Include,
                "same-origin" => crate::worker_wire::WorkerCredentials::SameOrigin,
                _ => return Err(failure("TypeError: invalid worklet credentials")),
            }
        }
    } else {
        crate::worker_wire::WorkerCredentials::SameOrigin
    };
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("addModule promise"))?;
    let promise = resolver.get_promise(scope);
    let context = context_for(state, id)?;
    let operation = identity(state)?;
    let result = (|| -> Result<(), V8BindingError> {
        let url = Url::parse(&state.native_document_base_url(true)?)
            .and_then(|base| base.join(&input))
            .map_err(|_| V8BindingError::Unsupported {
                interface: "AudioWorklet".into(),
                member: "SyntaxError".into(),
            })?;
        workers::publish_secure_context(state)?;
        let mut context = context.borrow_mut();
        if context.closed || context.pending.len() >= 64 {
            return Err(failure("InvalidStateError: audio context unavailable"));
        }
        if let Some(connection) = context.connection.clone() {
            match context.transport.request(AgentServiceRequest::Worklet {
                connection,
                command: WorkletCommand::LoadModule {
                    id: operation,
                    url: url.to_string(),
                    credentials,
                },
            }) {
                Ok(AgentServiceResponse::Done) => {}
                other => return Err(failure(&format!("worklet module admission: {other:?}"))),
            }
        } else {
            let options = WorkerOptions {
                audio_sample_rate: Some(context.sample_rate),
                credentials,
                script_type: ScriptType::Module,
                ..Default::default()
            };
            let creator = workers::native_creator(state)?;
            match context
                .transport
                .request(AgentServiceRequest::Create(AgentCreation {
                    kind: AgentKind::AudioWorklet,
                    url: url.to_string(),
                    options,
                    creator,
                })) {
                Ok(AgentServiceResponse::Connected(connection)) => {
                    context.connection = Some(connection);
                    context.first = Some(operation);
                }
                other => return Err(failure(&format!("worklet creation: {other:?}"))),
            }
        }
        context
            .pending
            .insert(operation, v8::Global::new(scope, resolver));
        Ok(())
    })();
    if let Err(error) = result {
        let name = if matches!(&error,V8BindingError::Unsupported{member,..}if member=="SyntaxError")
        {
            "SyntaxError"
        } else {
            "AbortError"
        };
        let error = dom_exception_value(scope, name)?;
        resolver.reject(scope, error);
    }
    Ok(promise.into())
}
pub(super) fn construct_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let owner = v8::Local::<v8::Object>::try_from(args.get(0))
        .map_err(|_| failure("TypeError: AudioContext"))?;
    let context = context_for(state, stateful_id(scope, owner)?)?;
    let name = args
        .get(1)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    if !context.borrow().definitions.contains(&name) {
        return Err(V8BindingError::Unsupported {
            interface: "AudioWorkletNode".into(),
            member: "InvalidStateError".into(),
        });
    }
    let connection = context
        .borrow()
        .connection
        .clone()
        .ok_or(failure("InvalidStateError: no loaded processors"))?;
    let (port, remote) = message_ports::channel(state)?;
    let remote_object = new_platform_object(scope, state, "MessagePort", remote)?;
    let options = args.get(2);
    let options = if options.is_undefined() {
        v8::Object::new(scope).into()
    } else {
        options
    };
    let wire = structured_clone::serialize_wire(scope, options, &[remote_object.into()])
        .ok_or(V8BindingError::JavaScriptException)?;
    let _send = structured_clone::shared_send_guard(scope, &wire);
    let transferred = wire.ports.clone();
    let processor_id = identity(state)?;
    let command = WorkletCommand::Construct {
        id: processor_id,
        name,
        value: Box::new(wire),
    };
    match context
        .borrow()
        .transport
        .request(AgentServiceRequest::Worklet {
            connection,
            command,
        }) {
        Ok(AgentServiceResponse::Done) => {}
        other => {
            let _ = context
                .borrow()
                .transport
                .request(AgentServiceRequest::Message(
                    crate::message_wire::MessageRequest::DiscardTransfers { ports: transferred },
                ));
            message_ports::close(state, port)?;
            return Err(failure(&format!("processor admission: {other:?}")));
        }
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::AudioWorklet(Object::Node {
            context: context.clone(),
            port,
        }),
    );
    bind_platform_object(scope, state, id, args.this());
    context.borrow_mut().nodes.insert(processor_id, id);
    Ok(())
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    state.audio_worklet.borrow().contexts.iter().any(|context| {
        let c = context.borrow();
        !c.closed && (c.connection.is_some() || !c.pending.is_empty())
    })
}
pub(super) fn retire(state: &V8BindingState) {
    for context in &state.audio_worklet.borrow().contexts {
        let mut c = context.borrow_mut();
        c.closed = true;
        c.pending.clear();
        c.events.clear();
        if let Some(connection) = c.connection.take()
            && c.transport
                .request(AgentServiceRequest::Release {
                    connection: connection.clone(),
                })
                .is_err()
        {
            c.connection = Some(connection);
        }
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired() || state.execution_suspended() {
        return Ok(0);
    }
    let contexts = state.audio_worklet.borrow().contexts.clone();
    let mut count = 0;
    for context in contexts {
        let admission = {
            let c = context.borrow();
            if c.closed {
                None
            } else {
                c.connection
                    .clone()
                    .map(|connection| (connection, c.transport.clone(), c.events.is_empty()))
            }
        };
        let Some((connection, transport, empty)) = admission else {
            continue;
        };
        if empty {
            match transport.request(AgentServiceRequest::Poll {
                connection,
                max: 32,
            }) {
                Ok(AgentServiceResponse::Events(events)) => {
                    context.borrow_mut().events.extend(events)
                }
                other => return Err(failure(&format!("worklet poll: {other:?}"))),
            }
        }
        loop {
            let event = { context.borrow().events.front().cloned() };
            let Some(event) = event else { break };
            if let AgentEvent::WorkletDefinitions(names) = &event {
                if names.len() > 256 || names.iter().any(|name| name.len() > 65536) {
                    return Err(failure("worklet registry metadata bound"));
                }
                context.borrow_mut().definitions = names.iter().cloned().collect();
            }
            if let AgentEvent::WorkletFinished { id, result: Err(_) } = &event {
                let target = { context.borrow().nodes.get(id).copied() };
                if let Some(target) = target {
                    let event = create_event_object(scope, state, "processorerror", false, false)?;
                    let id = event_id(scope, event)?;
                    state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
                    let creator = workers::native_creator(state)?;
                    native_platform::publication::enqueue(
                        state,
                        creator,
                        native_platform::publication::Payload::Platform {
                            target,
                            name: "processorerror".into(),
                            event: v8::Global::new(scope, event),
                        },
                    )?;
                }
            }
            if matches!(event, AgentEvent::Closed) {
                loop {
                    let pending = {
                        context
                            .borrow()
                            .pending
                            .first_key_value()
                            .map(|(id, resolver)| (*id, v8::Local::new(scope, resolver)))
                    };
                    let Some((id, resolver)) = pending else { break };
                    let error = dom_exception_value(scope, "AbortError")?;
                    resolver
                        .reject(scope, error)
                        .ok_or(V8BindingError::JavaScriptException)?;
                    context.borrow_mut().pending.remove(&id);
                }
                context.borrow_mut().closed = true;
            }
            let first = { context.borrow().first };
            let result = match &event {
                AgentEvent::Ready => first.map(|id| (id, Ok(()))),
                AgentEvent::WorkletFinished { id, result } => Some((*id, result.clone())),
                AgentEvent::Error { message, .. } => first.map(|id| {
                    (
                        id,
                        Err(crate::worker_wire::WorkerError::new(
                            "AbortError",
                            String::from_utf16_lossy(message),
                        )),
                    )
                }),
                _ => None,
            };
            if let Some((id, result)) = result {
                let resolver = {
                    context
                        .borrow()
                        .pending
                        .get(&id)
                        .map(|resolver| v8::Local::new(scope, resolver))
                };
                if let Some(resolver) = resolver {
                    match result {
                        Ok(()) => {
                            let undefined = v8::undefined(scope);
                            resolver
                                .resolve(scope, undefined.into())
                                .ok_or(V8BindingError::JavaScriptException)?;
                        }
                        Err(error) => {
                            let error = dom_exception_value(scope, &error.name)?;
                            resolver
                                .reject(scope, error)
                                .ok_or(V8BindingError::JavaScriptException)?;
                        }
                    }
                    let mut c = context.borrow_mut();
                    c.pending.remove(&id);
                    if c.first == Some(id) {
                        c.first = None;
                    }
                }
            }
            context.borrow_mut().events.pop_front();
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let context = context_for(state, id)?;
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("audio close promise"))?;
    let promise = resolver.get_promise(scope);
    let connection = {
        let c = context.borrow();
        c.connection
            .clone()
            .map(|connection| (connection, c.transport.clone()))
    };
    if let Some((connection, transport)) = connection {
        match transport.request(AgentServiceRequest::Release { connection }) {
            Ok(AgentServiceResponse::Done) => {
                context.borrow_mut().connection = None;
            }
            other => return Err(failure(&format!("audio close cleanup: {other:?}"))),
        }
    }
    context.borrow_mut().closed = true;
    loop {
        let pending = {
            context
                .borrow()
                .pending
                .first_key_value()
                .map(|(id, resolver)| (*id, v8::Local::new(scope, resolver)))
        };
        let Some((id, pending)) = pending else { break };
        let error = dom_exception_value(scope, "AbortError")?;
        pending
            .reject(scope, error)
            .ok_or(V8BindingError::JavaScriptException)?;
        context.borrow_mut().pending.remove(&id);
    }
    let value = v8::undefined(scope);
    resolver
        .resolve(scope, value.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(promise.into())
}
