//! Native channel endpoints. Payloads contain serialized data, never sender JS handles.
use super::*;

pub(super) type Port = Rc<RefCell<Endpoint>>;

#[derive(Clone)]
struct RemoteEndpoint {
    transport: std::sync::Arc<dyn crate::message_wire::MessageTransport>,
    token: crate::message_wire::PortToken,
}
pub(super) struct Endpoint {
    remote: Option<RemoteEndpoint>,
    owner: RcWeak<V8BindingState>,
    id: u64,
    peer: RcWeak<RefCell<Endpoint>>,
    started: bool,
    closed: bool,
    scheduled: usize,
    queue: VecDeque<message_delivery::Delivery>,
    delivering: bool,
    peer_close_pending: bool,
    peer_closed: bool,
    queued_bytes: usize,
}

fn endpoint(state: &V8BindingState, id: u64) -> Result<Option<Port>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::MessagePort(port) => Ok(Some(port.clone())),
        PlatformObjectRecord::DetachedMessagePort => Ok(None),
        _ => Err(V8BindingError::Allocation("MessagePort endpoint")),
    }
}

pub(super) fn transfer_identity(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<(u64, u64)> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if object.internal_field_count() == 0 {
        return None;
    }
    let state = binding_state_for_receiver(scope, object)?;
    let id = stateful_id(scope, object).ok()?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)?
        .to_local(scope)?
        .strict_equals(value)
    {
        return None;
    }
    matches!(
        state.platform_objects.borrow().get(&id),
        Some(PlatformObjectRecord::MessagePort(_) | PlatformObjectRecord::DetachedMessagePort)
    )
    .then_some((state.instance_id, id))
}

pub(super) fn transferable(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<(Rc<V8BindingState>, u64, Port)> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if object.internal_field_count() == 0 {
        return None;
    }
    let state = binding_state_for_receiver(scope, object)?;
    let id = stateful_id(scope, object).ok()?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value))
    {
        return None;
    }
    let port = endpoint(&state, id).ok()??;
    if port.borrow().closed {
        return None;
    }
    Some((state, id, port))
}

pub(super) fn detach(state: &V8BindingState, id: u64, port: &Port) {
    state
        .platform_objects
        .borrow_mut()
        .insert(id, PlatformObjectRecord::DetachedMessagePort);
    let mut endpoint = port.borrow_mut();
    endpoint.owner = RcWeak::new();
    endpoint.started = false;
    endpoint.scheduled = 0;
}

pub(super) fn receive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    port: Port,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = register_platform_object(state, PlatformObjectRecord::MessagePort(port.clone()));
    {
        let mut endpoint = port.borrow_mut();
        endpoint.owner = Rc::downgrade(state);
        endpoint.id = id;
    }
    new_platform_object(scope, state, "MessagePort", id).ok()
}

pub(super) fn channel(state: &Rc<V8BindingState>) -> Result<(u64, u64), V8BindingError> {
    let remote = if let Some(transport) = state
        .message_transport
        .borrow()
        .clone()
        .filter(|_| !state.execution_tasks_stopped())
    {
        match transport
            .request(
                state.scoped_message_request(crate::message_wire::MessageRequest::CreateChannel)?,
            )
            .map_err(|_| V8BindingError::Allocation("MessageChannel transport"))?
        {
            crate::message_wire::MessageResponse::Channel(tokens) => Some((transport, tokens)),
            _ => return Err(V8BindingError::Allocation("MessageChannel response")),
        }
    } else {
        None
    };
    let make = || {
        let port = Rc::new(RefCell::new(Endpoint {
            remote: None,
            owner: Rc::downgrade(state),
            id: 0,
            peer: RcWeak::new(),
            started: false,
            closed: false,
            scheduled: 0,
            queue: VecDeque::new(),
            delivering: false,
            peer_close_pending: false,
            peer_closed: false,
            queued_bytes: 0,
        }));
        let id = register_platform_object(state, PlatformObjectRecord::MessagePort(port.clone()));
        port.borrow_mut().id = id;
        (id, port)
    };
    let (a, first) = make();
    let (b, second) = make();
    first.borrow_mut().peer = Rc::downgrade(&second);
    second.borrow_mut().peer = Rc::downgrade(&first);
    if let Some((transport, [a, b])) = remote {
        first.borrow_mut().remote = Some(RemoteEndpoint {
            transport: transport.clone(),
            token: a,
        });
        second.borrow_mut().remote = Some(RemoteEndpoint {
            transport,
            token: b,
        });
    }
    Ok((a, b))
}

fn schedule(port: &Port) {
    let mut endpoint = port.borrow_mut();
    if endpoint.started
        && !endpoint.delivering
        && endpoint.scheduled == 0
        && !endpoint.queue.is_empty()
        && let Some(owner) = endpoint.owner.upgrade()
    {
        endpoint.scheduled += 1;
        owner
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::PortMessage {
                port_id: endpoint.id,
            });
    }
    if endpoint.started
        && !endpoint.delivering
        && endpoint.scheduled == 0
        && endpoint.queue.is_empty()
        && endpoint.peer_close_pending
        && let Some(owner) = endpoint.owner.upgrade()
    {
        endpoint.peer_close_pending = false;
        owner
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::Event {
                target_id: endpoint.id,
                event_type: "close",
            });
    }
}

pub(super) fn start(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    let Some(port) = endpoint(state, id)? else {
        return Ok(());
    };
    port.borrow_mut().started = true;
    schedule(&port);
    Ok(())
}

fn disentangle(port: &Port, discard: bool) {
    let remote = port.borrow_mut().remote.take();
    let native = remote.is_some();
    if let Some(remote) = remote {
        let _ = remote
            .transport
            .request(crate::message_wire::MessageRequest::ClosePort { port: remote.token });
    }
    let (peer, discarded) = {
        let mut endpoint = port.borrow_mut();
        endpoint.closed = true;
        let discarded = if discard {
            std::mem::take(&mut endpoint.queue)
        } else {
            VecDeque::new()
        };
        if discard {
            endpoint.queued_bytes = 0;
            endpoint.scheduled = 0;
        }
        (std::mem::take(&mut endpoint.peer).upgrade(), discarded)
    };
    if let Some(owner) = port.borrow().owner.upgrade() {
        for delivery in discarded {
            delivery.cancel(&owner);
        }
    }
    if let Some(peer) = peer {
        {
            let mut endpoint = peer.borrow_mut();
            endpoint.peer = RcWeak::new();
            endpoint.peer_closed = true;
            if !native {
                endpoint.peer_close_pending = true;
            }
        }
        schedule(&peer);
    }
}

/// Payload-owned endpoint that never committed into a receiving wrapper.
pub(super) fn discard_unreceived(port: &Port) {
    disentangle(port, true);
}

pub(super) fn close(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    if let Some(port) = endpoint(state, id)? {
        // Tasks queued before close remain deliverable, even when only the
        // first native task has been polled. Stage the bounded native queue
        // before revoking the endpoint; no author callback runs here.
        let remote = port.borrow().remote.clone();
        if let Some(remote) = remote {
            for _ in 0..32 {
                let response = remote
                    .transport
                    .request(state.scoped_message_receive(
                        crate::message_wire::MessageRequest::PollPort {
                            port: remote.token.clone(),
                            max: 32,
                        },
                    )?)
                    .map_err(|_| V8BindingError::Allocation("MessagePort close queue"))?;
                let crate::message_wire::MessageResponse::PortMessages { messages, .. } = response
                else {
                    return Err(V8BindingError::Allocation(
                        "MessagePort close queue response",
                    ));
                };
                let done = messages.len() < 32;
                for value in messages {
                    let delivery = message_delivery::Delivery::wire(state, value, String::new());
                    let mut endpoint = port.borrow_mut();
                    endpoint.queued_bytes = endpoint.queued_bytes.saturating_add(delivery.bytes());
                    endpoint.queue.push_back(delivery);
                }
                if done {
                    break;
                }
            }
        }
        disentangle(&port, false);
        schedule(&port);
    }
    Ok(())
}

pub(super) fn retire(state: &V8BindingState) {
    for (_, (_, delivery)) in std::mem::take(&mut *state.message_port_dispatches.borrow_mut()) {
        delivery.cancel(state);
    }
    let ports: Vec<_> = state
        .platform_objects
        .borrow()
        .values()
        .filter_map(|record| {
            if let PlatformObjectRecord::MessagePort(port) = record {
                Some(port.clone())
            } else {
                None
            }
        })
        .collect();
    for port in ports {
        disentangle(&port, true);
    }
}

pub(super) fn post(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    value: v8::Local<v8::Value>,
    options: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let port = endpoint(state, id)?;
    let Some(options) = structured_clone::post_transfer_options(scope, options) else {
        return Ok(());
    };
    let transfers = options.transfers;
    if transfers.iter().any(|value| {
        transferable(scope, *value).is_some_and(|(owner, port_id, _)| {
            owner.instance_id == state.instance_id && port_id == id
        })
    }) {
        let error = dom_exception_value(scope, "DataCloneError")?;
        scope.throw_exception(error);
        return Ok(());
    }
    let remote = port.as_ref().and_then(|port| port.borrow().remote.clone());
    if let Some(remote) = remote {
        let Some(serialized) =
            structured_clone::serialize_wire_with_activation(scope, value, &transfers, |scope| {
                structured_clone::activation_snapshot(scope, options.include_user_activation)
            })
        else {
            return Ok(());
        };
        let _shared_send_guard = structured_clone::shared_send_guard(scope, &serialized);
        let transferred = serialized.ports.clone();
        if let Err(error) = publish_native_window_tree(scope) {
            let _ =
                remote
                    .transport
                    .request(crate::message_wire::MessageRequest::DiscardTransfers {
                        ports: transferred,
                    });
            return Err(error);
        }
        let request =
            match state.scoped_message_request(crate::message_wire::MessageRequest::PostPort {
                port: remote.token,
                payload: serialized,
            }) {
                Ok(request) => request,
                Err(error) => {
                    let _ = remote.transport.request(
                        crate::message_wire::MessageRequest::DiscardTransfers {
                            ports: transferred,
                        },
                    );
                    return Err(error);
                }
            };
        if remote.transport.request(request).is_err() {
            let _ =
                remote
                    .transport
                    .request(crate::message_wire::MessageRequest::DiscardTransfers {
                        ports: transferred,
                    });
            let error = dom_exception_value(scope, "QuotaExceededError")?;
            scope.throw_exception(error);
        }
        return Ok(());
    }
    let peer = port.and_then(|port| port.borrow().peer.upgrade());
    let doomed = peer.as_ref().is_some_and(|peer| {
        transfers.iter().any(|value| {
            transferable(scope, *value).is_some_and(|(_, _, port)| Rc::ptr_eq(peer, &port))
        })
    });
    let Some(mut serialized) = structured_clone::serialize(scope, value, &transfers) else {
        return Ok(());
    };
    serialized.set_user_activation(structured_clone::activation_snapshot(
        scope,
        options.include_user_activation,
    ));
    if doomed {
        return Ok(());
    }
    // Serialization (and transfer validation) occurs even on a disconnected port.
    if let Some(peer) = peer
        && !peer.borrow().closed
    {
        let mut receiver = peer.borrow_mut();
        let bytes = serialized.retained_bytes();
        if receiver.queue.len() >= 1024
            || bytes > (32 * 1024 * 1024usize).saturating_sub(receiver.queued_bytes)
        {
            drop(receiver);
            let error = dom_exception_value(scope, "QuotaExceededError")?;
            scope.throw_exception(error);
            return Ok(());
        }
        receiver.queued_bytes += bytes;
        receiver
            .queue
            .push_back(message_delivery::Delivery::local(serialized));
        drop(receiver);
        schedule(&peer);
    }
    Ok(())
}

struct DeliveryGuard {
    port: Port,
    state: Rc<V8BindingState>,
    id: u64,
    delivery: Option<message_delivery::Delivery>,
    bytes: usize,
}
impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        // A running task belongs to its original target even if a listener
        // transfers that endpoint. Future, unstarted tasks follow the port.
        if let Some(delivery) = self.delivery.take() {
            if !self.state.execution_retired() {
                self.state
                    .message_port_dispatches
                    .borrow_mut()
                    .insert(self.id, (self.port.clone(), delivery));
                self.state
                    .deferred_platform_tasks
                    .borrow_mut()
                    .push_front(DeferredPlatformTask::PortMessage { port_id: self.id });
                return;
            }
            delivery.cancel(&self.state);
        }
        let mut port = self.port.borrow_mut();
        port.delivering = false;
        port.queued_bytes = port.queued_bytes.saturating_sub(self.bytes);
        drop(port);
        schedule(&self.port);
    }
}
pub(super) fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let resumed = state.message_port_dispatches.borrow_mut().remove(&id);
    let (port, delivery) = if let Some(resumed) = resumed {
        resumed
    } else {
        let Some(port) = endpoint(state, id)? else {
            return Ok(());
        };
        let remote = {
            let mut endpoint = port.borrow_mut();
            endpoint.scheduled = endpoint.scheduled.saturating_sub(1);
            if !endpoint.started || endpoint.delivering {
                return Ok(());
            }
            if endpoint.queue.is_empty() && !endpoint.closed {
                endpoint.remote.clone()
            } else {
                None
            }
        };
        if let Some(remote) = remote {
            // Dequeue only when this task starts. Until then the parent queue
            // and every embedded transfer/shared claim follow endpoint transfer.
            let request =
                state.scoped_message_receive(crate::message_wire::MessageRequest::PollPort {
                    port: remote.token,
                    max: 1,
                })?;
            let response = remote
                .transport
                .request(request)
                .map_err(|_| V8BindingError::Allocation("MessagePort poll"))?;
            let crate::message_wire::MessageResponse::PortMessages {
                messages,
                peer_closed,
            } = response
            else {
                return Err(V8BindingError::Allocation("MessagePort poll response"));
            };
            for value in messages {
                let delivery = message_delivery::Delivery::wire(state, value, String::new());
                let mut endpoint = port.borrow_mut();
                endpoint.queued_bytes = endpoint.queued_bytes.saturating_add(delivery.bytes());
                endpoint.queue.push_back(delivery);
            }
            if peer_closed {
                let mut endpoint = port.borrow_mut();
                endpoint.peer_close_pending = true;
                endpoint.peer_closed = true;
            }
        }
        let delivery = {
            let mut endpoint = port.borrow_mut();
            let delivery = endpoint.queue.pop_front();
            if delivery.is_some() {
                endpoint.delivering = true;
            }
            delivery
        };
        let Some(delivery) = delivery else {
            schedule(&port);
            return Ok(());
        };
        (port, delivery)
    };
    let bytes = delivery.bytes();
    let mut guard = DeliveryGuard {
        port,
        state: state.clone(),
        id,
        delivery: Some(delivery),
        bytes,
    };
    if guard.delivery.as_mut().unwrap().resume(scope, state, id)? {
        guard.delivery.take();
    }
    Ok(())
}

/// Export the detached native endpoints as one atomic transfer batch. No JS
/// property is consulted and no endpoint is transferable twice.
pub(super) fn export_tokens(
    scope: &mut v8::PinScope,
    ports: &[Port],
) -> Option<Vec<crate::message_wire::PortToken>> {
    if ports.is_empty() {
        return Some(Vec::new());
    }
    let endpoints = ports
        .iter()
        .map(|port| port.borrow().remote.clone())
        .collect::<Option<Vec<_>>>()?;
    let transport = endpoints[0].transport.clone();
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    let request = state
        .scoped_message_request(crate::message_wire::MessageRequest::Transfer {
            ports: endpoints.iter().map(|p| p.token.clone()).collect(),
        })
        .ok()?;
    match transport.request(request) {
        Ok(crate::message_wire::MessageResponse::Transferred(tokens)) => Some(tokens),
        _ => {
            if let Ok(error) = dom_exception_value(scope, "DataCloneError") {
                scope.throw_exception(error);
            }
            None
        }
    }
}
pub(super) fn import_token(
    state: &Rc<V8BindingState>,
    token: crate::message_wire::PortToken,
) -> Option<Port> {
    let transport = state.message_transport.borrow().clone()?;
    Some(Rc::new(RefCell::new(Endpoint {
        remote: Some(RemoteEndpoint { transport, token }),
        owner: RcWeak::new(),
        id: 0,
        peer: RcWeak::new(),
        started: false,
        closed: false,
        scheduled: 0,
        queue: VecDeque::new(),
        delivering: false,
        peer_close_pending: false,
        peer_closed: false,
        queued_bytes: 0,
    })))
}
pub(super) fn transfer_live(port: &Port) -> bool {
    !port.borrow().closed
}
pub(super) fn wire_transfer_supported(port: &Port) -> bool {
    port.borrow().remote.is_some()
}
pub(super) fn poll_external(
    _scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<usize, V8BindingError> {
    let ports = state
        .platform_objects
        .borrow()
        .values()
        .filter_map(|record| match record {
            PlatformObjectRecord::MessagePort(port) => Some(port.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut scheduled = 0;
    for port in ports {
        if state.execution_tasks_stopped() {
            break;
        }
        let mut endpoint = port.borrow_mut();
        if endpoint.remote.is_none()
            || endpoint.closed
            || !endpoint.started
            || endpoint.delivering
            || endpoint.scheduled != 0
        {
            continue;
        }
        if endpoint.queue.is_empty() {
            let remote = endpoint.remote.as_ref().unwrap();
            let request =
                state.scoped_message_receive(crate::message_wire::MessageRequest::PortPending {
                    port: remote.token.clone(),
                })?;
            match remote
                .transport
                .request(request)
                .map_err(|_| V8BindingError::Allocation("MessagePort readiness"))?
            {
                crate::message_wire::MessageResponse::PortPending(false) => continue,
                crate::message_wire::MessageResponse::PortPending(true) => {}
                _ => return Err(V8BindingError::Allocation("MessagePort readiness response")),
            }
        }
        endpoint.scheduled = 1;
        state
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::PortMessage {
                port_id: endpoint.id,
            });
        scheduled += 1;
    }
    Ok(scheduled)
}

/// Entanglement roots a port with message listeners, never its unused peer.
/// Queued/started dispatch tasks independently retain their destination wrapper.
pub(super) fn listener_target_active(port: &Port) -> bool {
    let port = port.borrow();
    port.scheduled > 0
        || !port.queue.is_empty()
        || (!port.closed
            && ((port.remote.is_some() && !port.peer_closed) || port.peer.upgrade().is_some()))
}
pub(super) fn release_collected(state: &V8BindingState, id: u64) {
    let port = {
        let records = state.platform_objects.borrow();
        match records.get(&id) {
            Some(PlatformObjectRecord::MessagePort(port)) => Some(port.clone()),
            _ => None,
        }
    };
    let Some(port) = port else { return };
    // An old wrapper can finalize after its endpoint moved to another realm.
    let owned = {
        let endpoint = port.borrow();
        endpoint.id == id
            && endpoint
                .owner
                .upgrade()
                .is_some_and(|owner| owner.instance_id == state.instance_id)
    };
    if owned {
        disentangle(&port, true);
        state.platform_objects.borrow_mut().remove(&id);
    }
}

#[cfg(test)]
mod dispatch_regressions {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> String {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    }
    #[test]
    fn native_port_handler_dispatch_is_once_ordered_and_stoppable() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.port=new MessageChannel().port1;globalThis.order=[];
            port.addEventListener('close',e=>order.push('capture:'+e.eventPhase),true);
            port.addEventListener('close',()=>order.push('before'));
            port.onclose=()=>order.push('old');
            port.addEventListener('close',()=>order.push('once'),{once:true});
            port.onclose=function(e){if(this!==port||e.target!==port||e.currentTarget!==port||!e.isTrusted)throw Error('port event identity');order.push('handler')};true"#,
        );
        let key = v8::String::new(scope, "port").unwrap();
        let port = context
            .global(scope)
            .get(scope, key.into())
            .unwrap()
            .to_object(scope)
            .unwrap();
        let id = stateful_id(scope, port).unwrap();
        assert!(!dispatch_platform_event_capturing(scope, &state, id, "close", None).unwrap());
        assert_eq!(
            eval(scope, "order.join(',')"),
            "capture:2,before,handler,once"
        );
        assert!(!dispatch_platform_event_capturing(scope, &state, id, "close", None).unwrap());
        assert_eq!(
            eval(scope, "order.join(',')"),
            "capture:2,before,handler,once,capture:2,before,handler"
        );
        eval(
            scope,
            "order=[];port.onclose=e=>{order.push('stop');e.stopImmediatePropagation()};port.addEventListener('close',()=>order.push('must-not-run'));true",
        );
        assert!(!dispatch_platform_event_capturing(scope, &state, id, "close", None).unwrap());
        assert_eq!(eval(scope, "order.join(',')"), "capture:2,before,stop");
    }
}
