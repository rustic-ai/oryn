//! Native Worker and SharedWorker clients backed by parent-owned agent principals.
use super::*;
use crate::worker_wire::*;
pub(super) fn publish_secure_context(state: &V8BindingState) -> Result<(), V8BindingError> {
    let creator = native_creator(state)?;
    if matches!(creator, Creator::Agent(_)) {
        return Ok(());
    }
    state.refresh_native_container_policy()?;
    let secure = state.secure_context.get();
    let transport = state
        .worker_transport
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("secure context transport"))?;
    match transport.request(AgentServiceRequest::PublishSecureContext { creator, secure }) {
        Ok(AgentServiceResponse::Done) => Ok(()),
        Err(error) => Err(V8BindingError::Unsupported {
            interface: "Window".into(),
            member: error.name,
        }),
        _ => Err(V8BindingError::Allocation("secure context publication")),
    }
}
pub(super) struct Client {
    pub(super) shared: bool,
    transport: std::sync::Arc<dyn WorkerTransport>,
    connection: Option<AgentConnection>,
    pub(super) port: Option<u64>,
    pending: std::collections::VecDeque<Pending>,
    polling: bool,
    suppressed: bool,
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = self
                .transport
                .request(AgentServiceRequest::Release { connection });
        }
    }
}
enum Payload {
    Raw(
        Box<AgentEvent>,
        Option<structured_clone::SharedReceiveGuard>,
    ),
    Message(Option<(v8::Global<v8::Value>, Vec<v8::Global<v8::Object>>)>),
    Empty,
}
struct Pending {
    activation: Option<[bool; 2]>,
    payload: Payload,
    dispatch: Option<native_platform::publication::Notification>,
}
impl Pending {
    fn new(state: &V8BindingState, event: AgentEvent) -> Self {
        let cleanup = match &event {
            AgentEvent::Message { value, .. } => {
                Some(structured_clone::shared_receive_guard(state, value))
            }
            _ => None,
        };
        let activation = match &event {
            AgentEvent::Message { value, .. } => value.user_activation(),
            _ => None,
        };
        Self {
            activation,
            payload: Payload::Raw(Box::new(event), cleanup),
            dispatch: None,
        }
    }
}
/// Retain the exact event through native preparation failure and callback
/// continuation. Reentrant polling cannot take the event owned by this task.
struct PollGuard {
    client: Rc<RefCell<Client>>,
    state: Rc<V8BindingState>,
    pending: Option<Pending>,
}
impl Drop for PollGuard {
    fn drop(&mut self) {
        let mut client = self.client.borrow_mut();
        client.polling = false;
        if let Some(pending) = self.pending.take() {
            if (!client.suppressed
                || matches!(&pending.payload, Payload::Raw(event, _) if matches!(event.as_ref(), AgentEvent::Closed)))
                && client.connection.is_some()
                && !self.state.execution_retired()
            {
                client.pending.push_front(pending);
            } else if let Some(dispatch) = pending.dispatch {
                dispatch.cancel(&self.state);
            }
        }
    }
}
fn discard_pending(state: &V8BindingState, client: &mut Client) {
    for pending in client.pending.drain(..) {
        if let Some(dispatch) = pending.dispatch {
            dispatch.cancel(state);
        }
    }
}
/// The parent capability remains live until Closed delivery and Release succeed,
/// so pending terminal messages retain their listener target as well.
pub(super) fn listener_target_active(client: &Client) -> bool {
    client.connection.is_some()
}
fn error(scope: &mut v8::PinScope, name: &str) -> Result<(), V8BindingError> {
    let exception = dom_exception_value(scope, name)?;
    scope.throw_exception(exception);
    Err(V8BindingError::JavaScriptException)
}
fn units(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u16>, V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let mut units = vec![0; value.length()];
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    Ok(units)
}
fn member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: Option<v8::Local<'s, v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let Some(object) = object else {
        return Ok(v8::undefined(scope).into());
    };
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("worker option"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
    shared: bool,
) -> Result<(), V8BindingError> {
    if args.length() == 0 {
        throw_type_error(scope, "Worker requires a script URL");
        return Err(V8BindingError::JavaScriptException);
    }
    let source = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let base = match state.worker_base_url.borrow().clone() {
        Some(url) => url,
        None => document_base_url(scope, state, true)?,
    };
    let Ok(url) = url::Url::parse(&base).and_then(|base| base.join(&source)) else {
        return error(scope, "SyntaxError");
    };
    let option = args.get(1);
    let mut options = WorkerOptions::default();
    if shared && !option.is_object() && !option.is_null_or_undefined() {
        options.name = units(scope, option)?
    } else {
        if !option.is_object() && !option.is_null_or_undefined() {
            throw_type_error(scope, "Worker options must be a dictionary");
            return Err(V8BindingError::JavaScriptException);
        }
        let object = v8::Local::<v8::Object>::try_from(option).ok();
        // WebIDL dictionary conversion order is observable.
        let credentials = member(scope, object, "credentials")?;
        if !credentials.is_undefined() {
            options.credentials =
                match String::from_utf16_lossy(&units(scope, credentials)?).as_str() {
                    "omit" => WorkerCredentials::Omit,
                    "same-origin" => WorkerCredentials::SameOrigin,
                    "include" => WorkerCredentials::Include,
                    _ => {
                        throw_type_error(scope, "invalid Worker credentials");
                        return Err(V8BindingError::JavaScriptException);
                    }
                }
        }
        let name = member(scope, object, "name")?;
        if !name.is_undefined() {
            options.name = units(scope, name)?;
        }
        let kind = member(scope, object, "type")?;
        if !kind.is_undefined() {
            options.script_type = match String::from_utf16_lossy(&units(scope, kind)?).as_str() {
                "classic" => ScriptType::Classic,
                "module" => ScriptType::Module,
                _ => {
                    throw_type_error(scope, "invalid Worker type");
                    return Err(V8BindingError::JavaScriptException);
                }
            }
        }
    }
    let Some(transport) = state.worker_transport.borrow().clone() else {
        return error(scope, "InvalidStateError");
    };
    publish_secure_context(state)?;
    #[cfg(unix)]
    let _ = native_shared_policy::bind(scope, state);
    let creator = native_creator(state)?;
    let document_source = if matches!(&creator, Creator::Window { .. }) {
        Some(
            (*state.document_provenance(state.document_root())?.ok_or(
                V8BindingError::Allocation("worker Document source provenance"),
            )?)
            .clone(),
        )
    } else {
        None
    };
    let response = transport.request(AgentServiceRequest::Create(AgentCreation {
        document_source,
        kind: if shared {
            AgentKind::Shared
        } else {
            AgentKind::Dedicated
        },
        url: url.to_string(),
        options,
        creator,
    }));
    let connection = match response {
        Ok(AgentServiceResponse::Connected(connection)) => connection,
        Err(failure) => return error(scope, &failure.name),
        _ => return Err(V8BindingError::Allocation("worker creation response")),
    };
    let port = (|| -> Result<Option<u64>, V8BindingError> {
        if shared {
            let token = connection
                .port
                .clone()
                .ok_or(V8BindingError::Allocation("shared worker port capability"))?;
            let port = message_ports::import_token(state, token)
                .ok_or(V8BindingError::Allocation("shared worker port transport"))?;
            let object = message_ports::receive(scope, state, port)
                .ok_or(V8BindingError::Allocation("shared worker port"))?;
            Ok(Some(stateful_id(scope, object)?))
        } else {
            Ok(None)
        }
    })();
    let port = match port {
        Ok(port) => port,
        Err(error) => {
            let _ = transport.request(AgentServiceRequest::Release { connection });
            return Err(error);
        }
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::Worker(Rc::new(RefCell::new(Client {
            shared,
            transport,
            connection: Some(connection),
            port,
            pending: Default::default(),
            polling: false,
            suppressed: false,
        }))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn client(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Client>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::Worker(client) => Ok(client.clone()),
        _ => Err(V8BindingError::Allocation("Worker receiver")),
    }
}
pub(super) fn terminate(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    let client = client(state, id)?;
    let (transport, connection) = {
        let mut owner = client.borrow_mut();
        owner.suppressed = true;
        discard_pending(state, &mut owner);
        (owner.transport.clone(), owner.connection.clone())
    };
    if let Some(connection) = connection {
        if !matches!(
            transport.request(AgentServiceRequest::Terminate { connection }),
            Ok(AgentServiceResponse::Done)
        ) {
            // A lost acknowledgement may follow committed termination. Release
            // is idempotent and cannot affect a successor agent/client identity.
            client
                .borrow_mut()
                .pending
                .push_back(Pending::new(state, AgentEvent::Closed));
            return Err(V8BindingError::Allocation("worker termination"));
        }
        client.borrow_mut().connection = None;
    }
    Ok(())
}

pub(super) fn post(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    if args.length() == 0 {
        throw_type_error(scope, "postMessage requires a message");
        return Err(V8BindingError::JavaScriptException);
    }
    let client = client(state, id)?;
    let (transport, connection) = {
        let client = client.borrow();
        (
            client.transport.clone(),
            if client.suppressed {
                None
            } else {
                client.connection.clone()
            },
        )
    };
    let Some(options) = structured_clone::post_transfer_options(scope, args.get(1)) else {
        return Ok(());
    };
    let transfers = options.transfers;
    let Some(value) =
        structured_clone::serialize_wire_with_activation(scope, args.get(0), &transfers, |scope| {
            structured_clone::activation_snapshot(scope, options.include_user_activation)
        })
    else {
        return Ok(());
    };
    let _shared_send_guard = structured_clone::shared_send_guard(scope, &value);
    let ports = value.ports.clone();
    let result = connection
        .map(|connection| transport.request(AgentServiceRequest::Post { connection, value }));
    if !matches!(&result, Some(Ok(AgentServiceResponse::Done))) {
        let _ = transport.request(AgentServiceRequest::Message(
            crate::message_wire::MessageRequest::DiscardTransfers { ports },
        ));
        if result.is_some() {
            return error(scope, "QuotaExceededError");
        }
    }
    Ok(())
}
pub(super) fn retire(state: &V8BindingState) {
    let clients = state
        .platform_objects
        .borrow()
        .values()
        .filter_map(|record| match record {
            PlatformObjectRecord::Worker(client) => Some(client.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for client in clients {
        let (transport, connection) = {
            let mut owner = client.borrow_mut();
            owner.suppressed = true;
            discard_pending(state, &mut owner);
            (owner.transport.clone(), owner.connection.clone())
        };
        if let Some(connection) = connection
            && matches!(
                transport.request(AgentServiceRequest::Release { connection }),
                Ok(AgentServiceResponse::Done)
            )
        {
            client.borrow_mut().connection = None;
        }
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let clients = state
        .platform_objects
        .borrow()
        .iter()
        .filter_map(|(id, record)| match record {
            PlatformObjectRecord::Worker(client) => Some((*id, client.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut count = 0;
    for (id, client) in clients {
        if state.execution_tasks_stopped() || scope.is_execution_terminating() {
            break;
        }
        let (transport, connection, fetch) = {
            let mut owner = client.borrow_mut();
            if owner.polling {
                continue;
            }
            let Some(connection) = owner.connection.clone() else {
                continue;
            };
            owner.polling = true;
            (
                owner.transport.clone(),
                connection,
                owner.pending.is_empty(),
            )
        };
        let mut guard = PollGuard {
            client: client.clone(),
            state: state.clone(),
            pending: None,
        };
        if fetch {
            let events = match transport.request(AgentServiceRequest::Poll {
                connection: connection.clone(),
                max: 1,
            }) {
                Ok(AgentServiceResponse::Events(events)) => events,
                Err(error) => vec![
                    AgentEvent::Error {
                        message: error.message.encode_utf16().collect(),
                        filename: String::new(),
                        line: 0,
                        column: 0,
                    },
                    AgentEvent::Closed,
                ],
                _ => return Err(V8BindingError::Allocation("worker poll response")),
            };
            client
                .borrow_mut()
                .pending
                .extend(events.into_iter().map(|event| Pending::new(state, event)));
        }
        guard.pending = client.borrow_mut().pending.pop_front();
        let Some(pending) = guard.pending.as_mut() else {
            continue;
        };
        if matches!(&pending.payload, Payload::Raw(event, _) if matches!(event.as_ref(), AgentEvent::Message { .. }))
        {
            let Payload::Raw(event, mut cleanup) =
                std::mem::replace(&mut pending.payload, Payload::Empty)
            else {
                unreachable!()
            };
            let AgentEvent::Message { value, .. } = *event else {
                unreachable!()
            };
            if let Some(cleanup) = &mut cleanup {
                cleanup.handoff_ports();
            }
            let data = {
                v8::tc_scope!(let tc,scope);
                structured_clone::deserialize_wire(tc, value).map(|(data, ports)| {
                    (
                        v8::Global::new(tc, data),
                        ports
                            .into_iter()
                            .map(|port| v8::Global::new(tc, port))
                            .collect::<Vec<_>>(),
                    )
                })
            };
            pending.payload = Payload::Message(data);
        }
        if pending.dispatch.is_none() {
            let prepared = match &pending.payload {
                Payload::Raw(event, _) if matches!(event.as_ref(), AgentEvent::Ready) => None,
                Payload::Raw(event, _) if matches!(event.as_ref(), AgentEvent::Closed) => {
                    let released = transport.request(AgentServiceRequest::Release {
                        connection: connection.clone(),
                    });
                    if !matches!(released, Ok(AgentServiceResponse::Done)) {
                        return Err(V8BindingError::Allocation(
                            "closed worker client release failed",
                        ));
                    }
                    client.borrow_mut().connection = None;
                    None
                }
                Payload::Message(data) => {
                    let (event, name) = if let Some((data, ports)) = data {
                        let data = v8::Local::new(scope, data);
                        let ports = ports
                            .iter()
                            .map(|port| v8::Local::new(scope, port))
                            .collect::<Vec<_>>();
                        (
                            message_events::delivery(scope, state, data, &ports)?,
                            "message",
                        )
                    } else {
                        let event = create_event_object_with_interface(
                            scope,
                            state,
                            "MessageEvent",
                            "messageerror",
                            false,
                            false,
                        )?;
                        message_events::initialize_empty(scope, state, event)
                            .ok_or(V8BindingError::Allocation("worker messageerror"))?;
                        let event_id = event_id(scope, event)?;
                        if let Some(record) = state.events.borrow_mut().get_mut(&event_id) {
                            record.is_trusted = true;
                        }
                        (event, "messageerror")
                    };
                    Some((event, name))
                }
                Payload::Raw(event, _) if matches!(event.as_ref(), AgentEvent::Error { .. }) => {
                    let AgentEvent::Error {
                        message,
                        filename,
                        line,
                        column,
                    } = event.as_ref()
                    else {
                        unreachable!()
                    };
                    let event = create_event_object_with_interface(
                        scope,
                        state,
                        "ErrorEvent",
                        "error",
                        false,
                        true,
                    )?;
                    let data = v8::Array::new(scope, 5);
                    let message =
                        v8::String::new_from_two_byte(scope, message, v8::NewStringType::Normal)
                            .ok_or(V8BindingError::Allocation("worker error message"))?;
                    let key = v8::String::new(scope, "0")
                        .ok_or(V8BindingError::Allocation("worker error index"))?;
                    if data.create_data_property(scope, key.into(), message.into()) != Some(true) {
                        return Err(V8BindingError::Allocation("worker error fields"));
                    }
                    let filename = v8::String::new(scope, filename)
                        .ok_or(V8BindingError::Allocation("worker error filename"))?;
                    let key = v8::String::new(scope, "1")
                        .ok_or(V8BindingError::Allocation("worker error index"))?;
                    if data.create_data_property(scope, key.into(), filename.into()) != Some(true) {
                        return Err(V8BindingError::Allocation("worker error fields"));
                    }
                    let line = v8::Integer::new_from_unsigned(scope, *line);
                    let key = v8::String::new(scope, "2")
                        .ok_or(V8BindingError::Allocation("worker error index"))?;
                    if data.create_data_property(scope, key.into(), line.into()) != Some(true) {
                        return Err(V8BindingError::Allocation("worker error fields"));
                    }
                    let column = v8::Integer::new_from_unsigned(scope, *column);
                    let key = v8::String::new(scope, "3")
                        .ok_or(V8BindingError::Allocation("worker error index"))?;
                    if data.create_data_property(scope, key.into(), column.into()) != Some(true) {
                        return Err(V8BindingError::Allocation("worker error fields"));
                    }
                    let null = v8::null(scope);
                    let key = v8::String::new(scope, "4")
                        .ok_or(V8BindingError::Allocation("worker error index"))?;
                    if data.create_data_property(scope, key.into(), null.into()) != Some(true) {
                        return Err(V8BindingError::Allocation("worker error fields"));
                    }
                    let brand = {
                        let mut brand = state.error_event_brand.borrow_mut();
                        if brand.is_none() {
                            let symbol = v8::Symbol::new(scope, None);
                            *brand = Some(v8::Global::new(scope, symbol));
                        }
                        v8::Local::new(scope, brand.as_ref().unwrap())
                    };
                    event.set_internal_field(2, data.into());
                    event.set_internal_field(3, brand.into());
                    let event_id = event_id(scope, event)?;
                    if let Some(record) = state.events.borrow_mut().get_mut(&event_id) {
                        record.is_trusted = true;
                    }
                    Some((event, "error"))
                }
                _ => {
                    return Err(V8BindingError::Allocation(
                        "parent control event leaked into Worker client",
                    ));
                }
            };
            if let Some((event, name)) = prepared {
                if matches!(name, "message" | "messageerror") {
                    message_events::set_user_activation(scope, state, event, pending.activation)?;
                }
                pending.dispatch = Some(native_platform::publication::pending_platform_dispatch(
                    scope, state, id, name, event,
                ));
            }
        }
        if let Some(dispatch) = &mut pending.dispatch {
            if !dispatch.resume(scope, state)? {
                return Ok(count);
            }
            dispatch.finish(scope, state);
            count += 1;
        }
        guard.pending = None;
        scope.perform_microtask_checkpoint();
    }
    platform_listener_graph::reconcile_activity(scope, state);
    Ok(count)
}

pub(super) fn native_creator(state: &V8BindingState) -> Result<Creator, V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired worker creator"));
    }
    let creator = state
        .worker_creator
        .borrow()
        .clone()
        .unwrap_or_else(|| Creator::Window {
            target: state.message_target.borrow().clone(),
            document_generation: state.document_generation(),
            stream_epoch: state.document_authority_epoch(),
        });
    if !state.document_prerendering()
        && let Creator::Window {
            target: crate::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = &creator
    {
        // Native state, never authored origin or page fields, establishes the
        // exact document lifetime used by the parent's admission check.
        let Some(messages) = state.message_transport.borrow().clone() else {
            return Err(V8BindingError::Allocation(
                "native creator lifetime registration",
            ));
        };
        let response = messages
            .request(crate::message_wire::MessageRequest::RegisterRootDocument {
                document_generation: *document_generation,
                stream_epoch: *stream_epoch,
            })
            .map_err(|error| {
                V8BindingError::Message(format!("native creator lifetime registration: {error}"))
            })?;
        if !matches!(response, crate::message_wire::MessageResponse::Done) {
            return Err(V8BindingError::Message(
                "native creator lifetime registration: invalid acknowledgement".into(),
            ));
        }
    }
    Ok(creator)
}

pub(super) fn has_pending(state: &V8BindingState) -> bool {
    state
        .platform_objects
        .borrow()
        .values()
        .any(|record| match record {
            PlatformObjectRecord::Worker(client) => client.borrow().connection.is_some(),
            _ => false,
        })
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Messages;
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            _: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    struct Transport {
        events: Mutex<std::collections::VecDeque<AgentEvent>>,
        polls: AtomicUsize,
        releases: AtomicUsize,
        fail_release: AtomicBool,
    }
    impl WorkerTransport for Transport {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            match request {
                AgentServiceRequest::Create(_) => {
                    Ok(AgentServiceResponse::Connected(AgentConnection {
                        agent: AgentId(7),
                        client: AgentClientId {
                            id: 11,
                            secret: [3; 32],
                        },
                        port: None,
                    }))
                }
                AgentServiceRequest::Poll { .. } => {
                    self.polls.fetch_add(1, Ordering::SeqCst);
                    Ok(AgentServiceResponse::Events(
                        self.events
                            .lock()
                            .unwrap()
                            .pop_front()
                            .into_iter()
                            .collect(),
                    ))
                }
                AgentServiceRequest::Release { .. } => {
                    self.releases.fetch_add(1, Ordering::SeqCst);
                    if self.fail_release.swap(false, Ordering::SeqCst) {
                        Err(WorkerError::new(
                            "OperationError",
                            "lost release acknowledgement",
                        ))
                    } else {
                        Ok(AgentServiceResponse::Done)
                    }
                }
                _ => Ok(AgentServiceResponse::Done),
            }
        }
    }
    fn install_fixture_source(state: &Rc<V8BindingState>) {
        state.set_document_provenance_capture(Rc::new(|_, _, _| {
            Ok(oryn_dom::DocumentProvenance {
                capability: b"fixture-native-source".to_vec(),
                origin: "https://example.test".into(),
                origin_key: "fixture-origin".into(),
                inert: false,
            })
        }));
        state.refresh_document_provenance().unwrap();
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn transport() -> Arc<Transport> {
        Arc::new(Transport {
            events: Mutex::new(Default::default()),
            polls: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            fail_release: AtomicBool::new(false),
        })
    }
    fn reenter(
        scope: &mut v8::PinScope,
        _: v8::FunctionCallbackArguments,
        mut result: v8::ReturnValue,
    ) {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .unwrap();
        result.set_uint32(poll(scope, &state).unwrap() as u32);
    }
    #[test]
    fn worker_delivery_retains_decoded_payload_callback_progress_and_terminal_receipt() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_fixture_source(&state);
        let transport = transport();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(
            transport.clone(),
            None,
            Some("https://example.test/".into()),
        );
        let callback = v8::Function::new(scope, reenter).unwrap();
        let key = v8::String::new(scope, "reenter").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), callback.into());
        eval(
            scope,
            "globalThis.worker=new Worker('https://example.test/worker.js');globalThis.seen=[];globalThis.saved=null;worker.addEventListener('message',e=>{saved=e;seen.push(reenter()===0?'first':'replayed')},{once:true});worker.addEventListener('message',e=>{if(e!==saved||e.data.text!=='payload'||e.target!==worker)throw Error('identity');seen.push('second')});worker.onerror=e=>seen.push(e.message);",
        );
        let data = eval(scope, "({text:'payload'})");
        let wire = structured_clone::serialize_wire(scope, data, &[]).unwrap();
        transport.events.lock().unwrap().extend([
            AgentEvent::Message {
                client: None,
                value: wire,
            },
            AgentEvent::Error {
                message: "failure".encode_utf16().collect(),
                filename: "worker.js".into(),
                line: 3,
                column: 5,
            },
            AgentEvent::Closed,
        ]);
        let template = state
            .instance_templates
            .borrow_mut()
            .remove("MessageEvent")
            .unwrap();
        assert!(poll(scope, &state).is_err());
        state
            .instance_templates
            .borrow_mut()
            .insert("MessageEvent", template);
        assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
        assert!(eval(scope, "seen.length===0").is_true());
        native_platform::publication::fail_before_next_prepare();
        assert!(poll(scope, &state).is_err());
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='first'").is_true());
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(
            eval(
                scope,
                "seen.join(',')==='first,second'&&saved.currentTarget===null&&saved.eventPhase===0"
            )
            .is_true()
        );
        assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
        let template = state
            .instance_templates
            .borrow_mut()
            .remove("ErrorEvent")
            .unwrap();
        assert!(poll(scope, &state).is_err());
        state
            .instance_templates
            .borrow_mut()
            .insert("ErrorEvent", template);
        eval(
            scope,
            "Object.defineProperty(Array.prototype,'0',{configurable:true,set(){throw Error('inherited setter')}})",
        );
        assert_eq!(poll(scope, &state).unwrap(), 1);
        eval(scope, "delete Array.prototype[0]");
        assert!(eval(scope, "seen.join(',')==='first,second,failure'").is_true());
        transport.fail_release.store(true, Ordering::SeqCst);
        assert!(poll(scope, &state).is_err());
        assert!(has_pending(&state));
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(!has_pending(&state));
        assert_eq!(transport.polls.load(Ordering::SeqCst), 3);
        assert_eq!(transport.releases.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn worker_delivery_resumes_after_execution_termination_without_listener_replay() {
        fn stop(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
            scope.terminate_execution();
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_fixture_source(&state);
        let transport = transport();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(
            transport.clone(),
            None,
            Some("https://example.test/".into()),
        );
        let callback = v8::Function::new(scope, stop).unwrap();
        let key = v8::String::new(scope, "stopExecution").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), callback.into());
        eval(
            scope,
            "globalThis.worker=new Worker('https://example.test/worker.js');globalThis.seen=[];worker.addEventListener('error',()=>{seen.push('first');stopExecution()},{once:true});worker.addEventListener('error',()=>seen.push('second'));",
        );
        transport
            .events
            .lock()
            .unwrap()
            .push_back(AgentEvent::Error {
                message: vec![],
                filename: String::new(),
                line: 0,
                column: 0,
            });
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "seen.join(',')==='first,second'").is_true());
        assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn terminating_worker_discards_suspended_client_dispatch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_fixture_source(&state);
        let transport = transport();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(
            transport.clone(),
            None,
            Some("https://example.test/".into()),
        );
        eval(
            scope,
            "globalThis.worker=new Worker('https://example.test/worker.js');globalThis.seen=[];globalThis.saved=null;worker.addEventListener('error',e=>{saved=e;seen.push('first')});worker.addEventListener('error',()=>seen.push('second'));",
        );
        transport
            .events
            .lock()
            .unwrap()
            .push_back(AgentEvent::Error {
                message: vec![],
                filename: String::new(),
                line: 0,
                column: 0,
            });
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        eval(scope, "worker.terminate()");
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(eval(scope, "seen.join(',')==='first'&&saved.eventPhase===0").is_true());
        assert!(!has_pending(&state));
    }
}

#[cfg(test)]
mod creator_registration_tests {
    use super::*;
    use crate::message_wire::{MessageRequest, MessageResponse, MessageTransport};
    use std::sync::{Arc, Mutex};

    struct Registration {
        reply: u8,
        requests: Mutex<Vec<MessageRequest>>,
    }
    impl MessageTransport for Registration {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            let retiring = matches!(request, MessageRequest::RetireRootDocument { .. });
            assert!(retiring || matches!(request, MessageRequest::RegisterRootDocument { .. }));
            self.requests.lock().unwrap().push(request);
            if retiring {
                return Ok(MessageResponse::Done);
            }
            match self.reply {
                0 => Ok(MessageResponse::Done),
                1 => Ok(MessageResponse::PortPending(false)),
                _ => Err("registration denied".into()),
            }
        }
    }

    #[test]
    fn creator_requires_exact_registration_acknowledgement_and_never_registers_after_retirement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        for reply in 0..3 {
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            let generation = state.document_generation();
            let epoch = state.document_authority_epoch();
            let transport = Arc::new(Registration {
                reply,
                requests: Mutex::new(Vec::new()),
            });
            state.set_message_transport(transport.clone());
            let result = state.scoped_message_request(MessageRequest::CreateChannel);
            if reply == 0 {
                assert!(matches!(result, Ok(MessageRequest::InDocument {
                    target: crate::message_wire::WindowTarget::Root,
                    document_generation, stream_epoch, request,
                }) if document_generation == generation && stream_epoch == epoch && matches!(*request, MessageRequest::CreateChannel)));
            } else {
                let expected = if reply == 1 {
                    "invalid acknowledgement"
                } else {
                    "registration denied"
                };
                assert!(
                    matches!(result, Err(V8BindingError::Message(message)) if message.contains(expected))
                );
            }
            {
                let requests = transport.requests.lock().unwrap();
                assert!(
                    matches!(requests.as_slice(), [MessageRequest::RegisterRootDocument {
                    document_generation, stream_epoch,
                }] if *document_generation == generation && *stream_epoch == epoch)
                );
            }
            state.retire_execution();
            let calls = transport.requests.lock().unwrap().len();
            assert!(
                state
                    .scoped_message_request(MessageRequest::CreateChannel)
                    .is_err()
            );
            assert_eq!(transport.requests.lock().unwrap().len(), calls);
        }
    }
}
