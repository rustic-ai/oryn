//! One destination-realm message task, retained until native dispatch completes.
use super::*;

enum Payload {
    Local(structured_clone::SerializedValue),
    Wire(
        crate::message_wire::WireValue,
        structured_clone::SharedReceiveGuard,
    ),
    Decoded(Option<(v8::Global<v8::Value>, Vec<v8::Global<v8::Object>>)>),
}
pub(super) struct Delivery {
    payload: Option<Payload>,
    origin: String,
    activation: Option<[bool; 2]>,
    source: Option<v8::Global<v8::Value>>,
    bytes: usize,
    name: &'static str,
    source_first_port: bool,
    dispatch: Option<native_platform::publication::Notification>,
}
impl Delivery {
    pub(super) fn local(value: structured_clone::SerializedValue) -> Self {
        let bytes = value.retained_bytes();
        Self {
            activation: value.user_activation(),
            payload: Some(Payload::Local(value)),
            origin: String::new(),
            source: None,
            bytes,
            name: "message",
            source_first_port: false,
            dispatch: None,
        }
    }
    pub(super) fn wire(
        state: &V8BindingState,
        value: crate::message_wire::WireValue,
        origin: String,
    ) -> Self {
        let bytes = value.retained_bytes();
        let guard = structured_clone::shared_receive_guard(state, &value);
        Self {
            activation: value.user_activation(),
            payload: Some(Payload::Wire(value, guard)),
            origin,
            source: None,
            bytes,
            name: "message",
            source_first_port: false,
            dispatch: None,
        }
    }
    pub(super) fn connect(mut self) -> Self {
        self.name = "connect";
        self.source_first_port = true;
        self
    }
    pub(super) fn with_origin(mut self, origin: String) -> Self {
        self.origin = origin;
        self
    }
    pub(super) fn set_source(&mut self, source: v8::Global<v8::Value>) {
        self.source = Some(source);
    }
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(super) fn cancel(self, state: &V8BindingState) {
        if let Some(dispatch) = self.dispatch {
            dispatch.cancel(state);
        }
    }
    pub(super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: u64,
    ) -> Result<bool, V8BindingError> {
        if !matches!(self.payload, Some(Payload::Decoded(_))) {
            let payload = self.payload.take().expect("owned message payload");
            let decoded = {
                v8::tc_scope!(let tc,scope);
                let decoded = match payload {
                    Payload::Local(value) => structured_clone::deserialize_with_ports(tc, value),
                    Payload::Wire(value, mut guard) => {
                        guard.handoff_ports();
                        structured_clone::deserialize_wire(tc, value)
                    }
                    Payload::Decoded(_) => unreachable!(),
                };
                decoded.map(|(value, ports)| {
                    (
                        v8::Global::new(tc, value),
                        ports
                            .into_iter()
                            .map(|port| v8::Global::new(tc, port))
                            .collect::<Vec<_>>(),
                    )
                })
            };
            if self.source_first_port
                && let Some((_, ports)) = &decoded
                && let Some(port) = ports.first()
            {
                let port = v8::Local::new(scope, port);
                self.source = Some(v8::Global::new(scope, v8::Local::<v8::Value>::from(port)));
            }
            self.payload = Some(Payload::Decoded(decoded));
        }
        if self.dispatch.is_none() {
            let Some(Payload::Decoded(decoded)) = &self.payload else {
                unreachable!()
            };
            let (event, name) = if let Some((data, ports)) = decoded {
                let data = v8::Local::new(scope, data);
                let ports = ports
                    .iter()
                    .map(|port| v8::Local::new(scope, port))
                    .collect::<Vec<_>>();
                (
                    message_events::delivery(scope, state, data, &ports)?,
                    self.name,
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
                    .ok_or(V8BindingError::Allocation("messageerror payload"))?;
                let id = event_id(scope, event)?;
                if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                    record.is_trusted = true;
                }
                (event, "messageerror")
            };
            message_events::set_user_activation(scope, state, event, self.activation)?;
            if name != "message" && name != "messageerror" {
                let id = event_id(scope, event)?;
                if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                    record.event_type = name.into();
                }
            }
            let fields = event
                .get_internal_field(scope, 2)
                .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
                .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                .ok_or(V8BindingError::Allocation("message fields"))?;
            let key = v8::String::new(scope, "1")
                .ok_or(V8BindingError::Allocation("message origin key"))?;
            let origin = v8::String::new(scope, &self.origin)
                .ok_or(V8BindingError::Allocation("message origin"))?;
            fields
                .create_data_property(scope, key.into(), origin.into())
                .ok_or(V8BindingError::Allocation("message origin"))?;
            if let Some(source) = &self.source {
                let key = v8::String::new(scope, "3")
                    .ok_or(V8BindingError::Allocation("message source key"))?;
                let source = v8::Local::new(scope, source);
                fields
                    .create_data_property(scope, key.into(), source)
                    .ok_or(V8BindingError::Allocation("message source"))?;
            }
            self.dispatch = Some(native_platform::publication::pending_platform_dispatch(
                scope, state, target, name, event,
            ));
        }
        let dispatch = self.dispatch.as_mut().expect("prepared message");
        if !dispatch.resume(scope, state)? {
            return Ok(false);
        }
        dispatch.finish(scope, state);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_wire::{MessageHub, MessageRequest, MessageResponse, MessageTransport};
    struct Messages(std::sync::Mutex<MessageHub>);
    impl MessageTransport for Messages {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            let mut hub = self.0.lock().unwrap();
            hub.set_partition(
                crate::message_wire::Principal::Window(1),
                "https://a.test".into(),
            );
            hub.handle(1, request, &BTreeMap::from([(1, "https://a.test".into())]))
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn local_and_native_ports_preserve_pending_event_and_inflight_close_messages() {
        super::super::tests::initialize_v8();
        for native in [false, true] {
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
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
                "globalThis.c=new MessageChannel();globalThis.seen=[];globalThis.saved=null;c.port1.addEventListener('message',e=>{saved=e;seen.push('first:'+e.data);c.port1.close()},{once:true});c.port1.onmessage=e=>seen.push('last:'+e.data);c.port2.postMessage('one');c.port2.postMessage('two');",
            );
            if native {
                message_ports::poll_external(scope, &state).unwrap();
            }
            let template = state
                .instance_templates
                .borrow_mut()
                .remove("MessageEvent")
                .unwrap();
            assert!(state.drain_platform_completions(scope, 32).is_err());
            state
                .instance_templates
                .borrow_mut()
                .insert("MessageEvent", template);
            assert!(eval(scope, "seen.length===0").is_true());
            native_platform::publication::fail_after_next_callback();
            assert!(state.drain_platform_completions(scope, 32).is_err());
            assert!(eval(scope, "seen.join(',')==='first:one'").is_true());
            for _ in 0..8 {
                state.drain_platform_completions(scope, 32).unwrap();
                scope.perform_microtask_checkpoint();
            }
            assert!(eval(scope,"seen.join(',')==='first:one,last:one,last:two'&&saved instanceof MessageEvent&&saved.isTrusted&&saved.source===null&&saved.origin===''&&saved.eventPhase===0").is_true());
        }
    }
    #[test]
    fn native_peer_close_does_not_detach_the_surviving_port() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages(std::sync::Mutex::new(
            MessageHub::default(),
        ))));
        eval(
            scope,
            "globalThis.c=new MessageChannel();c.port1.start();c.port2.close();",
        );
        message_ports::poll_external(scope, &state).unwrap();
        assert!(eval(scope,"(()=>{const copied=structuredClone(c.port1,{transfer:[c.port1]});if(!(copied instanceof MessagePort))return false;try{structuredClone(c.port2,{transfer:[c.port2]})}catch(e){return e.name==='DataCloneError'}return false})()").is_true());
    }
    #[test]
    fn malformed_message_uses_trusted_messageerror_and_native_fields_ignore_setters() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = eval(
            scope,
            "globalThis.c=new MessageChannel();globalThis.errorEvent=null;globalThis.dataEvent=null;c.port1.onmessageerror=e=>errorEvent=e;c.port1.onmessage=e=>dataEvent=e;c.port1",
        );
        let target = v8::Local::<v8::Object>::try_from(target).unwrap();
        let id = stateful_id(scope, target).unwrap();
        let mut invalid = Delivery::wire(
            &state,
            crate::message_wire::WireValue {
                version: 1,
                bytes: vec![0],
                hosts: vec![],
                buffers: vec![],
                ports: vec![],
                shared: vec![],
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: vec![],
            },
            "https://a.test".into(),
        );
        assert!(invalid.resume(scope, &state, id).unwrap());
        assert!(eval(scope,"errorEvent instanceof MessageEvent&&errorEvent.isTrusted&&errorEvent.data===null&&errorEvent.origin==='https://a.test'&&errorEvent.ports.length===0").is_true());
        let value = eval(scope, "({answer:42})");
        let serialized = structured_clone::serialize(scope, value, &[]).unwrap();
        let mut valid = Delivery::local(serialized);
        eval(
            scope,
            "Object.defineProperty(Array.prototype,'0',{configurable:true,set(){throw Error('setter')}});Object.defineProperty(Array.prototype,'1',{configurable:true,set(){throw Error('setter')}})",
        );
        assert!(valid.resume(scope, &state, id).unwrap());
        eval(scope, "delete Array.prototype[0];delete Array.prototype[1]");
        assert!(eval(scope,"dataEvent.data.answer===42&&dataEvent.origin===''&&Object.isFrozen(dataEvent.ports)").is_true());
    }
    #[test]
    fn broadcast_global_post_order_close_and_preparation_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages(std::sync::Mutex::new(
            MessageHub::default(),
        ))));
        eval(
            scope,
            "globalThis.a=new BroadcastChannel('order');globalThis.b=new BroadcastChannel('order');globalThis.c=new BroadcastChannel('order');globalThis.seen=[];for(const [name,port] of [['a',a],['b',b],['c',c]])port.onmessage=e=>{if(!(e instanceof MessageEvent)||e.origin!=='https://a.test'||e.source!==null)throw Error('fields');seen.push(name+':'+e.data)};a.postMessage('A');c.postMessage('C');b.postMessage('B');",
        );
        let template = state
            .instance_templates
            .borrow_mut()
            .remove("MessageEvent")
            .unwrap();
        assert!(broadcast_channels::poll(scope, &state).is_err());
        state
            .instance_templates
            .borrow_mut()
            .insert("MessageEvent", template);
        native_platform::publication::fail_after_next_callback();
        assert!(broadcast_channels::poll(scope, &state).is_err());
        for _ in 0..8 {
            broadcast_channels::poll(scope, &state).unwrap();
        }
        assert!(eval(scope, "seen.join(',')==='b:A,c:A,a:C,b:C,a:B,c:B'").is_true());
        eval(
            scope,
            "seen.length=0;b.onmessage=e=>{seen.push('b:'+e.data);b.close()};a.postMessage('first');a.postMessage('second');",
        );
        for _ in 0..8 {
            broadcast_channels::poll(scope, &state).unwrap();
        }
        assert!(eval(scope, "seen.join(',')==='b:first,c:first,c:second'").is_true());
    }
}
