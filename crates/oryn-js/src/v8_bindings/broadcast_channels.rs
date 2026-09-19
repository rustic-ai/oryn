//! Native BroadcastChannel subscriptions and destination-realm message tasks.
use super::*;
use crate::message_wire::{MessageRequest, MessageResponse, MessageTransport, PortToken};
pub(super) struct Channel {
    pub(super) name: Vec<u16>,
    transport: std::sync::Arc<dyn MessageTransport>,
    token: Option<PortToken>,
    pending: VecDeque<message_delivery::Delivery>,
    polling: bool,
    closed: bool,
    posting: usize,
}
fn error(scope: &mut v8::PinScope, name: &str) -> Result<(), V8BindingError> {
    let value = dom_exception_value(scope, name)?;
    scope.throw_exception(value);
    Err(V8BindingError::JavaScriptException)
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    if args.length() == 0 {
        throw_type_error(scope, "BroadcastChannel requires a name");
        return Err(V8BindingError::JavaScriptException);
    }
    let name = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let mut units = vec![0; name.length()];
    name.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    let Some(transport) = state.message_transport.borrow().clone() else {
        return error(scope, "NotSupportedError");
    };
    let response = if state.execution_tasks_stopped() {
        None
    } else {
        Some(
            transport.request(state.scoped_message_request(MessageRequest::BroadcastOpen {
                target: state.message_target.borrow().clone(),
                name: units.clone(),
            })?),
        )
    };
    let token = match response {
        None => None,
        Some(Ok(MessageResponse::BroadcastOpened(token))) => Some(token),
        Some(Err(message)) => {
            return error(
                scope,
                if message.starts_with("SecurityError") {
                    "SecurityError"
                } else {
                    "QuotaExceededError"
                },
            );
        }
        _ => return Err(V8BindingError::Allocation("BroadcastChannel response")),
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::BroadcastChannel(Rc::new(RefCell::new(Channel {
            name: units,
            transport,
            token,
            pending: VecDeque::new(),
            polling: false,
            closed: false,
            posting: 0,
        }))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn channel(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Channel>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::BroadcastChannel(channel) => Ok(channel.clone()),
        _ => Err(V8BindingError::Allocation("BroadcastChannel receiver")),
    }
}
pub(super) fn close(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    let channel = channel(state, id)?;
    let (transport, token) = {
        let mut channel = channel.borrow_mut();
        channel.closed = true;
        for delivery in channel.pending.drain(..) {
            delivery.cancel(state);
        }
        let token = if channel.posting == 0 {
            channel.token.take()
        } else {
            None
        };
        (channel.transport.clone(), token)
    };
    if let Some(channel) = token {
        let _ = transport.request(MessageRequest::BroadcastClose { channel });
    }
    Ok(())
}
// close() during a message getter cannot revoke a send already admitted by
// the closed-flag check. Retain native routing until all nested sends finish.
struct PostGuard(Rc<RefCell<Channel>>);
impl Drop for PostGuard {
    fn drop(&mut self) {
        let (transport, token) = {
            let mut channel = self.0.borrow_mut();
            channel.posting = channel.posting.saturating_sub(1);
            let token = if channel.closed && channel.posting == 0 {
                channel.token.take()
            } else {
                None
            };
            (channel.transport.clone(), token)
        };
        if let Some(channel) = token {
            let _ = transport.request(MessageRequest::BroadcastClose { channel });
        }
    }
}
pub(super) fn post(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let channel = channel(state, id)?;
    // Eligibility precedes the closed flag and serialization. A retained
    // constructor can still create an inert object in a detached realm.
    if state.execution_tasks_stopped() {
        return Ok(());
    }
    // Exception allocation materializes a platform object and reconciles the
    // listener graph, which reads this channel. End the borrow before either
    // creating that exception or entering structured-clone author code.
    let admitted = {
        let mut owner = channel.borrow_mut();
        if owner.closed {
            None
        } else {
            owner.posting += 1;
            Some((owner.transport.clone(), owner.token.clone()))
        }
    };
    let Some((transport, token)) = admitted else {
        return error(scope, "InvalidStateError");
    };
    let _post = PostGuard(channel);
    let Some(channel) = token else {
        return error(scope, "InvalidStateError");
    };
    let Some(payload) = structured_clone::serialize_wire(scope, value, &[]) else {
        return Ok(());
    };
    let _shared_send_guard = structured_clone::shared_send_guard(scope, &payload);
    if state.execution_tasks_stopped() {
        return Ok(());
    }
    publish_native_window_tree(scope)?;
    match transport
        .request(state.scoped_message_request(MessageRequest::BroadcastPost { channel, payload })?)
    {
        Ok(MessageResponse::Done) => Ok(()),
        Err(message) => error(
            scope,
            if message.starts_with("InvalidStateError") {
                "InvalidStateError"
            } else {
                "QuotaExceededError"
            },
        ),
        _ => Err(V8BindingError::Allocation("BroadcastChannel post response")),
    }
}
pub(super) fn retire(state: &V8BindingState) {
    let ids = state
        .platform_objects
        .borrow()
        .iter()
        .filter_map(|(id, record)| {
            matches!(record, PlatformObjectRecord::BroadcastChannel(_)).then_some(*id)
        })
        .collect::<Vec<_>>();
    for id in ids {
        let _ = close(state, id);
    }
}
struct PollGuard {
    channel: Rc<RefCell<Channel>>,
    state: Rc<V8BindingState>,
    delivery: Option<message_delivery::Delivery>,
}
impl Drop for PollGuard {
    fn drop(&mut self) {
        let mut channel = self.channel.borrow_mut();
        channel.polling = false;
        if let Some(delivery) = self.delivery.take() {
            if channel.token.is_some() && !channel.closed && !self.state.execution_retired() {
                channel.pending.push_front(delivery)
            } else {
                delivery.cancel(&self.state)
            }
        }
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let entries = state
        .platform_objects
        .borrow()
        .iter()
        .filter_map(|(id, record)| match record {
            PlatformObjectRecord::BroadcastChannel(channel) => Some((*id, channel.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut delivered = 0;
    if entries.iter().any(|(_, channel)| channel.borrow().polling) {
        return Ok(0);
    }
    for (id, channel) in entries {
        if state.execution_tasks_stopped() {
            break;
        }
        let (transport, token, needs_poll) = {
            let mut owner = channel.borrow_mut();
            if owner.polling || owner.closed {
                continue;
            }
            let Some(token) = owner.token.clone() else {
                continue;
            };
            owner.polling = true;
            (owner.transport.clone(), token, owner.pending.is_empty())
        };
        let mut guard = PollGuard {
            channel: channel.clone(),
            state: state.clone(),
            delivery: None,
        };
        if needs_poll {
            let response = transport
                .request(state.scoped_message_receive(MessageRequest::BroadcastPoll {
                    channel: token,
                    max: 1,
                })?)
                .map_err(|_| V8BindingError::Allocation("BroadcastChannel poll"))?;
            let MessageResponse::BroadcastMessages(messages) = response else {
                return Err(V8BindingError::Allocation("BroadcastChannel poll response"));
            };
            for message in messages {
                channel
                    .borrow_mut()
                    .pending
                    .push_back(message_delivery::Delivery::wire(
                        state,
                        message.payload,
                        message.origin,
                    ));
            }
        }
        guard.delivery = channel.borrow_mut().pending.pop_front();
        if let Some(delivery) = guard.delivery.as_mut() {
            if !delivery.resume(scope, state, id)? {
                return Ok(delivered);
            }
            guard.delivery.take();
            delivered += 1;
            scope.perform_microtask_checkpoint();
        }
    }
    Ok(delivered)
}

pub(super) fn listener_target_active(channel: &Channel) -> bool {
    channel.token.is_some() && !channel.closed
}

#[cfg(test)]
mod eligibility_tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    struct Transport {
        hub: Mutex<crate::message_wire::MessageHub>,
        calls: AtomicUsize,
    }
    impl MessageTransport for Transport {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut hub = self.hub.lock().unwrap();
            hub.set_partition(crate::message_wire::Principal::Window(1), "site".into());
            hub.handle(1, request, &BTreeMap::from([(1, "https://a.test".into())]))
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn detached_realm_channels_are_inert_before_and_after_creation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport {
            hub: Mutex::new(crate::message_wire::MessageHub::default()),
            calls: AtomicUsize::new(0),
        });
        state.set_message_transport(transport.clone());
        eval(
            scope,
            "globalThis.before=new BroadcastChannel('same');globalThis.portBefore=new MessageChannel();globalThis.reads=0",
        );
        state.retire_execution();
        let calls = transport.calls.load(Ordering::SeqCst);
        assert!(eval(scope,"(()=>{const after=new BroadcastChannel('same'),ports=new MessageChannel();before.postMessage({get value(){reads++;throw Error('ineligible serialization')}});after.postMessage(Symbol());after.close();after.postMessage(Symbol());ports.port1.postMessage('ignored');return after.name==='same'&&reads===0})()").is_true());
        assert_eq!(transport.calls.load(Ordering::SeqCst), calls);
        assert_eq!(poll(scope, &state).unwrap(), 0);
    }
    #[test]
    fn close_inside_serialization_does_not_revoke_an_admitted_broadcast() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport {
            hub: Mutex::new(crate::message_wire::MessageHub::default()),
            calls: AtomicUsize::new(0),
        });
        state.set_message_transport(transport);
        eval(
            scope,
            "globalThis.seen=[];globalThis.sender=new BroadcastChannel('name');globalThis.receiver=new BroadcastChannel('name');receiver.onmessage=e=>seen.push(e.data.answer);sender.postMessage({get answer(){sender.close();try{sender.postMessage('nested');throw Error('closed nested send')}catch(e){if(e.name!=='InvalidStateError')throw e}return 42}})",
        );
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "seen.join(',')==='42'").is_true());
        state.retire_execution();
    }

    #[test]
    fn active_listener_cycle_survives_delivery_then_collects_after_callback_closes_channel() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport {
            hub: Mutex::new(crate::message_wire::MessageHub::default()),
            calls: AtomicUsize::new(0),
        });
        state.set_message_transport(transport);
        eval(
            scope,
            "globalThis.calls=0;globalThis.sender=new BroadcastChannel('cycle');(()=>{const receiver=new BroadcastChannel('cycle');receiver.onmessage=event=>{if(!event.isTrusted||event.data!==17)throw Error('delivery');calls++;receiver.close()}})()",
        );
        let receiver = state
            .platform_listener_roots
            .borrow()
            .keys()
            .copied()
            .find(|id| {
                matches!(
                    state.platform_objects.borrow().get(id),
                    Some(PlatformObjectRecord::BroadcastChannel(_))
                )
            })
            .unwrap();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(
            state
                .platform_listener_roots
                .borrow()
                .contains_key(&receiver)
        );
        eval(scope, "sender.postMessage(17)");
        {
            v8::scope!(let inner,scope);
            assert_eq!(poll(inner, &state).unwrap(), 1);
        }
        assert!(eval(scope, "calls===1").is_true());
        platform_listener_graph::reconcile_activity(scope, &state);
        assert!(
            !state
                .platform_listener_roots
                .borrow()
                .contains_key(&receiver)
        );
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(
            !state
                .platform_listeners
                .borrow()
                .keys()
                .any(|(target, _)| *target == receiver)
        );
        eval(scope, "sender.postMessage(18)");
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(eval(scope, "calls===1").is_true());
        state.retire_execution();
    }

    #[test]
    fn closed_channel_exception_reconciles_listener_roots_without_borrowing_the_admission() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport {
            hub: Mutex::new(crate::message_wire::MessageHub::default()),
            calls: AtomicUsize::new(0),
        });
        state.set_message_transport(transport);
        eval(
            scope,
            "globalThis.channel=new BroadcastChannel('closed');channel.onmessage=()=>channel.name;globalThis.getterCalls=0",
        );
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::BroadcastChannel(_)).then_some(*id)
            })
            .unwrap();
        assert!(state.platform_listener_roots.borrow().contains_key(&id));
        assert!(eval(scope,"(()=>{channel.close();channel.close();for(let i=0;i<2;i++){try{channel.postMessage({get value(){getterCalls++;return 1}});return false}catch(error){if(!(error instanceof DOMException)||error.name!=='InvalidStateError')return false}}return getterCalls===0})()").is_true());
        assert!(!state.platform_listener_roots.borrow().contains_key(&id));
        let owner = channel(&state, id).unwrap();
        assert!(owner.borrow().closed);
        assert_eq!(owner.borrow().posting, 0);
        assert!(owner.borrow().token.is_none());
        state.retire_execution();
    }
}
