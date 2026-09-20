//! Regression sources for transfer-list conversion, getter order, and backing metadata.
use super::*;
use crate::message_wire::{MessageHub, MessageRequest, MessageResponse, MessageTransport};
struct Messages(std::sync::Mutex<MessageHub>);
impl MessageTransport for Messages {
    fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
        self.0
            .lock()
            .unwrap()
            .handle(1, request, &BTreeMap::from([(1, "https://a.test".into())]))
    }
}
use super::super::test_eval::eval;
#[test]
fn detached_transfer_validation_follows_the_entire_message_graph() {
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
        assert!(eval(scope,r#"(()=>{
            const carrier=new MessageChannel(),marker={};
            function expectMarker(value,list){try{carrier.port1.postMessage(value,list);throw Error('accepted')}catch(e){if(e!==marker)throw e}}
            const detached=new ArrayBuffer(4);structuredClone(detached,{transfer:[detached]});
            expectMarker({detached,get last(){throw marker}},[detached]);
            const closed=new MessageChannel();closed.port1.close();expectMarker({port:closed.port1,get last(){throw marker}},[closed.port1]);
            const locked=new ReadableStream();locked.getReader();expectMarker({stream:locked,get last(){throw marker}},[locked]);
            const reentrant=new ArrayBuffer(4);expectMarker({get first(){structuredClone(reentrant,{transfer:[reentrant]});return reentrant},get last(){throw marker}},[reentrant]);
            let called=false;for(const list of [[detached,detached],[{}]]){
                try{carrier.port1.postMessage({get x(){called=true;throw marker}},list);throw Error('accepted')}catch(e){if(e.name!=='DataCloneError')throw e}
            }
            if(called)throw Error('early transfer-list validation ran message getter');
            try{carrier.port1.postMessage({detached},[detached]);throw Error('accepted')}catch(e){if(e.name!=='DataCloneError')throw e}
            return true;
        })()"#).is_true());
        state.retire_execution();
    }
}
#[test]
fn wire_transfers_preserve_resizable_buffers_and_length_tracking_views() {
    super::super::tests::initialize_v8();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body>").into_runtime().unwrap(),
    ));
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    let data = eval(
        scope,
        "globalThis.backing=new ArrayBuffer(4,{maxByteLength:16});new Uint8Array(backing).set([1,2,3,4]);globalThis.graph={backing,tracking:new Uint8Array(backing),fixed:new Uint8Array(backing,0,2)};graph",
    );
    let buffer = eval(scope, "backing");
    let wire = serialize_wire(scope, data, &[buffer]).unwrap();
    assert!(eval(scope, "backing.byteLength===0").is_true());
    assert!(wire.hosts.iter().any(|host| matches!(
        host,
        crate::message_wire::WireHost::TransferredArrayBuffer { index: 0, .. }
    )));
    let (copy, ports) = deserialize_wire(scope, wire).unwrap();
    assert!(ports.is_empty());
    let key = v8::String::new(scope, "copy").unwrap();
    context.global(scope).set(scope, key.into(), copy);
    assert!(eval(scope,"copy.backing.resizable&&copy.backing.maxByteLength===16&&copy.tracking.buffer===copy.backing&&copy.fixed.buffer===copy.backing&&copy.tracking.join(',')==='1,2,3,4'").is_true());
    assert!(
        eval(
            scope,
            "copy.backing.resize(8);copy.tracking.length===8&&copy.fixed.length===2"
        )
        .is_true()
    );
}
#[test]
fn postmessage_sequence_overload_observes_iterator_once() {
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
        const carrier=new MessageChannel();let gets=0;
        const first=new ArrayBuffer(2);carrier.port1.postMessage(first,new Set([first]));if(first.byteLength!==0)throw Error('Set overload');
        const second=new ArrayBuffer(2);const sequence={get [Symbol.iterator](){gets++;return function*(){yield second}}};carrier.port1.postMessage(second,sequence);if(gets!==1||second.byteLength!==0)throw Error('iterator conversion');
        const third=new ArrayBuffer(2);carrier.port1.postMessage(third,{transfer:new Set([third])});if(third.byteLength!==0)throw Error('dictionary overload');
        const marker={};let messageRead=false;try{carrier.port1.postMessage({get x(){messageRead=true}},{get [Symbol.iterator](){throw marker}});throw Error('accepted')}catch(e){if(e!==marker)throw e}
        return !messageRead;
    })()"#).is_true());
    state.retire_execution();
}

#[test]
fn native_queued_messages_follow_port_transfer_before_task_start() {
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
        "globalThis.channel=new MessageChannel();globalThis.seen=[];channel.port1.onmessage=e=>seen.push('old:'+e.data);channel.port2.postMessage('one');channel.port2.postMessage('two');",
    );
    message_ports::poll_external(scope, &state).unwrap();
    let port = eval(scope, "channel.port1");
    let wire = serialize_wire(scope, port, &[port]).unwrap();
    let transport = state.message_transport.borrow().clone().unwrap();
    let transferred = wire.ports[0].clone();
    assert!(
        transport
            .request(
                state
                    .scoped_message_receive(MessageRequest::PortPending {
                        port: transferred.clone(),
                    })
                    .unwrap()
            )
            .is_err(),
        "transit authority must not be readable before broker receipt"
    );
    // Serialization exports transit capabilities. Exercise the broker receipt
    // that production delivery performs before importing destination wrappers.
    let MessageResponse::Channel([sender, receiver]) = transport
        .request(
            state
                .scoped_message_request(MessageRequest::CreateChannel)
                .unwrap(),
        )
        .unwrap()
    else {
        panic!("carrier channel response")
    };
    assert!(matches!(
        transport
            .request(
                state
                    .scoped_message_request(MessageRequest::PostPort {
                        port: sender.clone(),
                        payload: wire,
                    })
                    .unwrap()
            )
            .unwrap(),
        MessageResponse::Done
    ));
    let MessageResponse::PortMessages { mut messages, .. } = transport
        .request(
            state
                .scoped_message_receive(MessageRequest::PollPort {
                    port: receiver.clone(),
                    max: 1,
                })
                .unwrap(),
        )
        .unwrap()
    else {
        panic!("carrier delivery response")
    };
    assert_eq!(messages.len(), 1);
    let wire = messages.pop().unwrap();
    assert_eq!(wire.ports[0].id, transferred.id);
    assert_ne!(wire.ports[0].secret, transferred.secret);
    for token in [sender, receiver] {
        assert!(matches!(
            transport
                .request(MessageRequest::ClosePort { port: token })
                .unwrap(),
            MessageResponse::Done
        ));
    }
    let (copy, _) = deserialize_wire(scope, wire).unwrap();
    let key = v8::String::new(scope, "moved").unwrap();
    context.global(scope).set(scope, key.into(), copy);
    eval(scope, "moved.onmessage=e=>seen.push('new:'+e.data)");
    for _ in 0..4 {
        message_ports::poll_external(scope, &state).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
    }
    assert!(eval(scope, "seen.join(',')==='new:one,new:two'").is_true());
    state.retire_execution();
}
#[test]
fn transferred_port_keeps_started_dispatch_and_does_not_replay_callbacks() {
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
        "globalThis.channel=new MessageChannel();globalThis.carrier=new MessageChannel();globalThis.seen=[];carrier.port1.onmessage=e=>{e.data.onmessage=e=>seen.push('new:'+e.data)};channel.port1.addEventListener('message',e=>{seen.push('first:'+e.data);carrier.port2.postMessage(channel.port1,[channel.port1])},{once:true});channel.port1.onmessage=e=>seen.push('last:'+e.data);channel.port2.postMessage('one');channel.port2.postMessage('two');",
    );
    message_ports::poll_external(scope, &state).unwrap();
    native_platform::publication::fail_after_next_callback();
    assert!(state.drain_platform_completions(scope, 32).is_err());
    assert!(eval(scope, "seen.join(',')==='first:one'").is_true());
    assert_eq!(state.message_port_dispatches.borrow().len(), 1);
    for _ in 0..8 {
        message_ports::poll_external(scope, &state).unwrap();
        state.drain_platform_completions(scope, 32).unwrap();
    }
    assert!(eval(scope, "seen.join(',')==='first:one,last:one,new:two'").is_true());
    assert!(state.message_port_dispatches.borrow().is_empty());
    state.retire_execution();
}

#[test]
fn media_source_handles_transfer_native_capability_without_exposing_hidden_ports() {
    super::super::tests::initialize_v8();
    for native in [false, true] {
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(WorkerExposureKind(
            crate::worker_wire::AgentKind::Dedicated,
        )));
        let scope = &mut v8::ContextScope::new(scope, context);
        let document = dom.borrow().document();
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(0),
            1,
            document,
            BindingGlobal::Worker,
        )
        .unwrap();
        if native {
            state.set_message_transport(std::sync::Arc::new(Messages(std::sync::Mutex::new(
                MessageHub::default(),
            ))));
        }
        eval(
            scope,
            "globalThis.source=new ManagedMediaSource();globalThis.handle=source.handle;globalThis.carrier=new MessageChannel();globalThis.seen=[];carrier.port1.onmessage=e=>{if(e.ports.length)throw Error('hidden endpoint');seen.push(e.data instanceof MediaSourceHandle);const second=structuredClone(e.data,{transfer:[e.data]});seen.push(second instanceof MediaSourceHandle)};carrier.port2.postMessage(handle,[handle]);",
        );
        assert!(eval(scope,"(()=>{if(source.handle!==handle)throw Error('stable handle');try{structuredClone(handle,{transfer:[handle]})}catch(e){return e.name==='DataCloneError'}return false})()").is_true());
        for _ in 0..4 {
            message_ports::poll_external(scope, &state).unwrap();
            state.drain_platform_completions(scope, 32).unwrap();
        }
        assert!(eval(scope, "seen.join(',')==='true,true'").is_true());
        assert!(eval(scope,"(()=>{const handle=new MediaSource().handle,marker={};try{structuredClone({handle,get last(){throw marker}},{transfer:[handle]})}catch(e){if(e!==marker)throw e}const copy=structuredClone(handle,{transfer:[handle]});try{structuredClone(copy)}catch(e){return e.name==='DataCloneError'}return false})()").is_true());
        state.retire_execution();
    }
}

#[test]
fn message_activation_is_an_opt_in_snapshot_and_never_a_receiver_grant() {
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
            "globalThis.events=[];globalThis.channel=new MessageChannel();channel.port2.onmessage=e=>events.push(e);channel.port1.postMessage('snapshot',{includeUserActivation:true});channel.port1.postMessage('absent');addEventListener('message',e=>events.push(e));postMessage('window',{includeUserActivation:true,targetOrigin:'*'});",
        );
        state.notify_user_activation();
        for _ in 0..4 {
            message_ports::poll_external(scope, &state).unwrap();
            state.drain_platform_completions(scope, 32).unwrap();
        }
        while state.has_pending_window_message() {
            window_messages::deliver(scope, &state).unwrap();
        }
        assert!(eval(scope,"events.length===3&&events[0].userActivation instanceof UserActivation&&!events[0].userActivation.isActive&&!events[0].userActivation.hasBeenActive&&events[1].userActivation===null&&!events[2].userActivation.hasBeenActive&&new MessageEvent('x').userActivation===null").is_true());
        assert!(state.has_sticky_user_activation());
        state.retire_execution();
    }
}
#[test]
fn worker_destination_receives_activation_snapshot_through_wire_metadata() {
    super::super::tests::initialize_v8();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body>").into_runtime().unwrap(),
    ));
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let document = dom.borrow().document();
    let state = install_generated_bindings_with_root_for_global(
        scope,
        context,
        dom,
        RealmId(0),
        1,
        document,
        BindingGlobal::Worker,
    )
    .unwrap();
    eval(
        scope,
        "globalThis.seen=[];onmessage=e=>seen.push(e.userActivation===null?'absent':String(e.userActivation.isActive)+','+String(e.userActivation.hasBeenActive))",
    );
    let value = eval(scope, "null");
    let mut wire = serialize_wire(scope, value, &[]).unwrap();
    wire.set_user_activation(Some([true, true]));
    worker_events::message(scope, &state, wire).unwrap();
    let wire = serialize_wire(scope, value, &[]).unwrap();
    worker_events::message(scope, &state, wire).unwrap();
    assert!(eval(scope, "seen.join('|')==='true,true|absent'").is_true());
    assert!(!state.has_sticky_user_activation());
    assert!(!state.has_transient_user_activation());
    state.retire_execution();
}

#[test]
fn activation_metadata_is_validated_before_transfer_commit() {
    super::super::tests::initialize_v8();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body>").into_runtime().unwrap(),
    ));
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    // Find the encoded backing overhead at the same varint width as the limit.
    let length = crate::message_wire::MAX_BYTES - 1024;
    let sample = v8::ArrayBuffer::new(scope, length);
    let wire = serialize_wire(scope, sample.into(), &[sample.into()]).unwrap();
    let overhead = wire.retained_bytes() - length;
    drop(wire);
    let length = crate::message_wire::MAX_BYTES - overhead;
    let buffer = v8::ArrayBuffer::new(scope, length);
    {
        v8::tc_scope!(let tc,scope);
        assert!(
            serialize_wire_with_activation(tc, buffer.into(), &[buffer.into()], |_| Some([
                true, true
            ]))
            .is_none()
        );
        assert!(tc.has_caught());
    }
    assert!(!buffer.was_detached());
    assert_eq!(buffer.byte_length(), length);
    let wire = serialize_wire(scope, buffer.into(), &[buffer.into()]).unwrap();
    assert_eq!(wire.retained_bytes(), crate::message_wire::MAX_BYTES);
    assert!(buffer.was_detached());
}
#[test]
fn primitive_postmessage_options_throw_before_message_getters() {
    super::super::tests::initialize_v8();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body>").into_runtime().unwrap(),
    ));
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    assert!(eval(scope,r#"(()=>{const channel=new MessageChannel();let seen=false;for(const options of [1,true,'x',Symbol(),1n]){try{channel.port1.postMessage({get x(){seen=true}},options);throw Error('accepted primitive')}catch(e){if(!(e instanceof TypeError))throw e}}return !seen})()"#).is_true());
}

#[test]
fn unreachable_entangled_peer_collects_and_delivers_one_close() {
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
        {
            v8::scope!(let inner,scope);
            eval(
                inner,
                "globalThis.closes=0;(()=>{const channel=new MessageChannel();globalThis.survivor=channel.port1;survivor.start();survivor.onclose=()=>closes++})()",
            );
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        state.poll_external_ports(scope).unwrap();
        state.drain_platform_completions(scope, 16).unwrap();
        state.drain_platform_completions(scope, 16).unwrap();
        assert!(eval(scope, "closes===1").is_true());
        // An entangled destination with message listeners remains live even
        // before start(); losing only its author reference does not close it.
        {
            v8::scope!(let inner,scope);
            eval(
                inner,
                "(()=>{const channel=new MessageChannel();globalThis.sender=channel.port1;channel.port2.addEventListener('message',()=>{});globalThis.liveCloses=0;sender.start();sender.onclose=()=>liveCloses++})()",
            );
            platform_listener_graph::reconcile_activity(inner, &state);
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        state.poll_external_ports(scope).unwrap();
        state.drain_platform_completions(scope, 16).unwrap();
        assert!(eval(scope, "liveCloses===0").is_true());
        state.retire_execution();
    }
}

struct ModuleOriginTransport([u8; 32]);
impl crate::worker_wire::WorkerTransport for ModuleOriginTransport {
    fn request(
        &self,
        request: crate::worker_wire::AgentServiceRequest,
    ) -> Result<crate::worker_wire::AgentServiceResponse, crate::worker_wire::WorkerError> {
        match request {
            crate::worker_wire::AgentServiceRequest::SharedMemory {
                request: crate::shared_memory::SharedMemoryRequest::CloneOrigin,
                ..
            } => Ok(crate::worker_wire::AgentServiceResponse::SharedMemory(
                crate::shared_memory::SharedMemoryResponse {
                    isolated: false,
                    outcome: crate::shared_memory::SharedMemoryOutcome::CloneOrigin(self.0),
                    descriptor: None,
                    #[cfg(unix)]
                    native_descriptor: None,
                },
            )),
            _ => Err(crate::worker_wire::WorkerError::new(
                "OperationError",
                "unexpected module authority request",
            )),
        }
    }
}
#[test]
fn wasm_module_clone_checks_native_origin_before_recompilation() {
    super::super::tests::initialize_v8();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body>").into_runtime().unwrap(),
    ));
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    state.set_security_origin("https://same.test".into());
    let module = eval(
        scope,
        "new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]))",
    );
    state.set_worker_transport(
        std::sync::Arc::new(ModuleOriginTransport([1; 32])),
        None,
        None,
    );
    // A worker authority without document registration must reject cleanly.
    {
        v8::tc_scope!(let tc, scope);
        assert!(serialize(tc, module, &[]).is_none());
        assert!(tc.has_caught());
    }
    state.set_message_transport(std::sync::Arc::new(Messages(std::sync::Mutex::new(
        MessageHub::default(),
    ))));
    let local = serialize(scope, module, &[]).unwrap();
    let wire = serialize_wire(scope, module, &[]).unwrap();
    {
        v8::tc_scope!(let tc, scope);
        let decoded = deserialize_wire(tc, wire.clone());
        let exception = tc.exception().map(|value| value.to_rust_string_lossy(tc));
        assert!(
            decoded.is_some(),
            "same-origin module clone failed: {exception:?}"
        );
    }
    // Equal serialized origins do not alias two distinct opaque broker origins.
    state.set_worker_transport(
        std::sync::Arc::new(ModuleOriginTransport([2; 32])),
        None,
        None,
    );
    {
        v8::tc_scope!(let tc,scope);
        assert!(deserialize(tc, local).is_none());
        assert!(tc.has_caught());
    }
    {
        v8::tc_scope!(let tc,scope);
        assert!(deserialize_wire(tc, wire.clone()).is_none());
        assert!(tc.has_caught());
    }
    let mut missing = wire.clone();
    missing
        .hosts
        .retain(|host| !matches!(host, crate::message_wire::WireHost::ModuleOrigin(_)));
    assert!(missing.validate().is_err());
    let mut duplicate = wire;
    duplicate
        .hosts
        .push(crate::message_wire::WireHost::ModuleOrigin([1; 32]));
    assert!(duplicate.validate().is_err());
}

#[test]
fn transferred_stream_module_chunks_reject_in_cross_origin_receiver() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let source_context = v8::Context::new(scope, Default::default());
    let (source_state, payload) = {
        let scope = &mut v8::ContextScope::new(scope, source_context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, source_context, dom, RealmId(11), 1).unwrap();
        state.set_security_origin("https://source.test".into());
        let pair = eval(
            scope,
            "(()=>{const module=new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]));globalThis.rs=new ReadableStream({start(c){c.enqueue(module)}});globalThis.ws=new WritableStream();return [rs,ws]})()",
        );
        let list = v8::Local::<v8::Array>::try_from(pair).unwrap();
        let readable = list.get_index(scope, 0).unwrap();
        let writable = list.get_index(scope, 1).unwrap();
        let payload = serialize(scope, pair, &[readable, writable]).unwrap();
        (state, payload)
    };
    let target_context = v8::Context::new(scope, Default::default());
    let target_state = {
        let scope = &mut v8::ContextScope::new(scope, target_context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, target_context, dom, RealmId(12), 1).unwrap();
        state.set_security_origin("https://target.test".into());
        let pair = deserialize(scope, payload).unwrap();
        let global = target_context.global(scope);
        set_object_property(scope, global, "pair", pair).unwrap();
        eval(
            scope,
            "globalThis.readFailure='pending';globalThis.writeFailure='pending';pair[0].getReader().read().catch(e=>readFailure=e.name);globalThis.writer=pair[1].getWriter();writer.closed.catch(e=>writeFailure=e.name);writer.write(new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]))).catch(()=>{})",
        );
        state
    };
    for _ in 0..64 {
        for (context, state) in [
            (source_context, &source_state),
            (target_context, &target_state),
        ] {
            let scope = &mut v8::ContextScope::new(scope, context);
            state.drain_platform_completions(scope, 32).unwrap();
            scope.perform_microtask_checkpoint();
        }
    }
    {
        let scope = &mut v8::ContextScope::new(scope, target_context);
        assert!(
            eval(
                scope,
                "readFailure==='DataCloneError'&&writeFailure==='DataCloneError'"
            )
            .is_true()
        );
    }
    source_state.retire_execution();
    target_state.retire_execution();
}
