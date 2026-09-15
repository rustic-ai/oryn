//! Native channel endpoints. Payloads contain serialized data, never sender JS handles.
use super::*;

pub(super) type Port = Rc<RefCell<Endpoint>>;

pub(super) struct Endpoint {
    owner: RcWeak<V8BindingState>,
    id: u64,
    peer: RcWeak<RefCell<Endpoint>>,
    started: bool,
    closed: bool,
    scheduled: usize,
    queue: VecDeque<structured_clone::SerializedValue>,
    queued_bytes: usize,
}

fn endpoint(state: &V8BindingState, id: u64) -> Result<Option<Port>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::MessagePort(port) => Ok(Some(port.clone())),
        PlatformObjectRecord::DetachedMessagePort => Ok(None),
        _ => Err(V8BindingError::Allocation("MessagePort endpoint")),
    }
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

pub(super) fn channel(state: &Rc<V8BindingState>) -> (u64, u64) {
    let make = || {
        let port = Rc::new(RefCell::new(Endpoint {
            owner: Rc::downgrade(state),
            id: 0,
            peer: RcWeak::new(),
            started: false,
            closed: false,
            scheduled: 0,
            queue: VecDeque::new(),
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
    (a, b)
}

fn schedule(port: &Port) {
    let mut endpoint = port.borrow_mut();
    while endpoint.started
        && endpoint.scheduled < endpoint.queue.len()
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
    let peer = {
        let mut endpoint = port.borrow_mut();
        endpoint.closed = true;
        if discard {
            endpoint.queue.clear();
            endpoint.queued_bytes = 0;
            endpoint.scheduled = 0;
        }
        std::mem::take(&mut endpoint.peer).upgrade()
    };
    if let Some(peer) = peer {
        let mut peer = peer.borrow_mut();
        peer.peer = RcWeak::new();
        if let Some(owner) = peer.owner.upgrade() {
            owner
                .deferred_platform_tasks
                .borrow_mut()
                .push_back(DeferredPlatformTask::Event {
                    target_id: peer.id,
                    event_type: "close",
                });
        }
    }
}

pub(super) fn close(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    if let Some(port) = endpoint(state, id)? {
        disentangle(&port, false);
    }
    Ok(())
}

pub(super) fn retire(state: &V8BindingState) {
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
    // Support the sequence overload as well as StructuredSerializeOptions.
    let options = if options.is_array() {
        let dictionary = v8::Object::new(scope);
        let key =
            v8::String::new(scope, "transfer").ok_or(V8BindingError::Allocation("transfer"))?;
        dictionary.create_data_property(scope, key.into(), options);
        dictionary.into()
    } else {
        options
    };
    let Some(transfers) = structured_clone::transfer_list(scope, options) else {
        return Ok(());
    };
    if transfers.iter().any(|value| {
        transferable(scope, *value).is_some_and(|(owner, port_id, _)| {
            owner.instance_id == state.instance_id && port_id == id
        })
    }) {
        let error = dom_exception_value(scope, "DataCloneError")?;
        scope.throw_exception(error);
        return Ok(());
    }
    let peer = port.and_then(|port| port.borrow().peer.upgrade());
    let doomed = peer.as_ref().is_some_and(|peer| {
        transfers.iter().any(|value| {
            transferable(scope, *value).is_some_and(|(_, _, port)| Rc::ptr_eq(peer, &port))
        })
    });
    let Some(serialized) = structured_clone::serialize(scope, value, &transfers) else {
        return Ok(());
    };
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
        receiver.queue.push_back(serialized);
        drop(receiver);
        schedule(&peer);
    }
    Ok(())
}

pub(super) fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let Some(port) = endpoint(state, id)? else {
        return Ok(());
    };
    let message = {
        let mut endpoint = port.borrow_mut();
        endpoint.scheduled = endpoint.scheduled.saturating_sub(1);
        if !endpoint.started {
            return Ok(());
        }
        let message = endpoint.queue.pop_front();
        if let Some(message) = &message {
            endpoint.queued_bytes -= message.retained_bytes();
        }
        message
    };
    if let Some(message) = message {
        let data = {
            v8::tc_scope!(let tc, scope);
            structured_clone::deserialize_with_ports(tc, message).map(|(value, ports)| {
                (
                    v8::Global::new(tc, value),
                    ports
                        .into_iter()
                        .map(|port| v8::Global::new(tc, port))
                        .collect::<Vec<_>>(),
                )
            })
        };
        if let Some((data, ports)) = data {
            let data = v8::Local::new(scope, &data);
            let ports = ports
                .iter()
                .map(|port| v8::Local::new(scope, port))
                .collect::<Vec<_>>();
            let event = message_events::delivery(scope, state, data, &ports)?;
            dispatch_platform_event(scope, state, id, "message", Some(event))?;
        } else {
            dispatch_platform_event(scope, state, id, "messageerror", None)?;
        }
    }
    // One event per task. The host performs its microtask checkpoint before
    // processing this queued successor, including messages posted by a handler.
    schedule(&port);
    Ok(())
}
