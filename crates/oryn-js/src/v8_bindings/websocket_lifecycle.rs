//! Outbound ordering and retained terminal-event delivery belong to the socket,
//! independently of host completion arrival order.
use super::*;
// Web IDL conversions finish before the constructor algorithm admits any IO.
pub(super) fn constructor_arguments(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<(String, Vec<String>), V8BindingError> {
    if args.length() == 0 {
        return Err(V8BindingError::Message(
            "TypeError: WebSocket URL is required".into(),
        ));
    }
    let input = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let value = args.get(1);
    let mut protocols = Vec::new();
    if !value.is_undefined() {
        let iterable = if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            let key = v8::Symbol::get_iterator(scope);
            let method = object
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            if method.is_null_or_undefined() {
                None
            } else {
                Some((
                    object,
                    v8::Local::<v8::Function>::try_from(method).map_err(|_| {
                        V8BindingError::Message(
                            "TypeError: protocol iterator is not callable".into(),
                        )
                    })?,
                ))
            }
        } else {
            None
        };
        if let Some((object, method)) = iterable {
            iterable_conversion::consume(scope, object, method, |scope, item| {
                let protocol = item
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                if protocols.len() >= 1024 || protocol.len() > 65536 {
                    return Err(V8BindingError::Message(
                        "QuotaExceededError: WebSocket protocols exceed capacity".into(),
                    ));
                }
                protocols.push(protocol);
                Ok(())
            })?;
        } else {
            protocols.push(
                value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope),
            );
        }
    }
    let syntax =
        || V8BindingError::Message("SyntaxError: invalid WebSocket URL or protocols".into());
    let base = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("WebSocket realm"))?;
    let base = document_base_url(scope, &base, base.binding_global == BindingGlobal::Window)?;
    let mut url = Url::parse(&input)
        .or_else(|_| Url::parse(&base).and_then(|base| base.join(&input)))
        .map_err(|_| syntax())?;
    match url.scheme() {
        "http" => {
            url.set_scheme("ws").map_err(|_| syntax())?;
        }
        "https" => {
            url.set_scheme("wss").map_err(|_| syntax())?;
        }
        "ws" | "wss" => {}
        _ => return Err(syntax()),
    }
    if url.fragment().is_some() {
        return Err(syntax());
    }
    let mut seen = std::collections::BTreeSet::new();
    for protocol in &protocols {
        if protocol.is_empty()
            || !protocol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || !seen.insert(protocol)
        {
            return Err(syntax());
        }
    }
    Ok((url.to_string(), protocols))
}
#[derive(Default)]
pub(super) struct State {
    pub handshake: Option<HostOperationId>,
    pub protocol: Option<String>,
    pub sending: bool,
    pub close: Option<(Option<u16>, String)>,
    outbound: VecDeque<(PlatformWebSocketFrame, u64)>,
    terminal: bool,
}
pub(super) struct Terminal {
    id: u64,
    code: u16,
    reason: String,
    clean: bool,
    error: bool,
    target: v8::Global<v8::Object>,
    delivery: Option<file_reader::Delivery>,
}
pub(super) fn enqueue(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    frame: PlatformWebSocketFrame,
    bytes: u64,
) -> Result<(), V8BindingError> {
    let full = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WebSocket(socket) = &mut *record else {
            return Err(V8BindingError::Allocation("WebSocket record"));
        };
        if socket.lifecycle.outbound.len() >= 1024
            || bytes > 16 * 1024 * 1024
            || socket
                .lifecycle
                .outbound
                .iter()
                .map(|(_, bytes)| *bytes)
                .sum::<u64>()
                .saturating_add(bytes)
                > 16 * 1024 * 1024
        {
            true
        } else {
            socket.lifecycle.outbound.push_back((frame, bytes));
            false
        }
    };
    if full {
        return terminal(scope, state, id, 1006, String::new(), false, true);
    }
    pump(scope, state, id)
}
pub(super) fn pump(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let operation = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WebSocket(socket) = &mut *record else {
            return Err(V8BindingError::Allocation("WebSocket record"));
        };
        if socket.lifecycle.terminal || socket.lifecycle.sending {
            return Ok(());
        }
        let Some(socket_id) = socket.socket_id else {
            return Ok(());
        };
        if let Some((frame, bytes)) = socket.lifecycle.outbound.pop_front() {
            socket.lifecycle.sending = true;
            Some((
                PlatformHostRequest::WebSocketSend { socket_id, frame },
                PendingPlatformOperation::WebSocketSend {
                    websocket_id: id,
                    bytes,
                },
            ))
        } else if let Some((code, reason)) = socket.lifecycle.close.take() {
            let code = code.or_else(|| (!reason.is_empty()).then_some(1000));
            Some((
                PlatformHostRequest::WebSocketClose {
                    socket_id,
                    code,
                    reason: reason.clone(),
                },
                PendingPlatformOperation::WebSocketClose {
                    websocket_id: id,
                    code,
                    reason,
                },
            ))
        } else {
            None
        }
    };
    if let Some((request, pending)) = operation {
        if submit_platform_operation(state, request, pending).is_err() {
            terminal(scope, state, id, 1006, String::new(), false, true)?;
        }
    }
    Ok(())
}
pub(super) fn terminal(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    code: u16,
    reason: String,
    clean: bool,
    error: bool,
) -> Result<(), V8BindingError> {
    if matches!(&*platform_object(state,id)?,PlatformObjectRecord::WebSocket(socket) if socket.lifecycle.terminal||socket.ready_state==3)
    {
        return Ok(());
    }
    // Root the wrapper before changing observable state or relinquishing transport.
    let target = new_platform_object(scope, state, "WebSocket", id)?;
    let socket_id = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::WebSocket(socket) = &mut *record else {
            return Err(V8BindingError::Allocation("WebSocket record"));
        };
        socket.lifecycle.terminal = true;
        socket.ready_state = 2;
        socket.lifecycle.outbound.clear();
        socket.lifecycle.close = None;
        socket.socket_id.take()
    };
    state
        .deferred_platform_tasks
        .borrow_mut()
        .push_back(DeferredPlatformTask::WebSocketTerminal(Terminal {
            id,
            code,
            reason,
            clean,
            error,
            target: v8::Global::new(scope, target),
            delivery: None,
        }));
    if let (Some(host), Some(socket_id)) = (state.platform_host.borrow().clone(), socket_id) {
        host.abandon_websocket(socket_id);
    }
    Ok(())
}
pub(super) fn deliver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    mut task: Terminal,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    let result = (|| {
        if let PlatformObjectRecord::WebSocket(socket) = &mut *platform_object_mut(state, task.id)?
        {
            socket.ready_state = 3;
        }
        let name = if task.error { "error" } else { "close" };
        if task.delivery.is_none() {
            let event = create_event_object_with_interface(
                scope,
                state,
                if task.error { "Event" } else { "CloseEvent" },
                name,
                false,
                false,
            )?;
            let id = event_id(scope, event)?;
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.is_trusted = true;
            }
            if !task.error {
                attach_close(scope, event, task.code, &task.reason, task.clean)?;
            }
            task.delivery = Some(file_reader::Delivery::from_event(scope, event, name)?);
        }
        task.delivery
            .as_mut()
            .unwrap()
            .resume(scope, state, task.id, &task.target)
    })();
    if result.is_err() {
        state
            .deferred_platform_tasks
            .borrow_mut()
            .push_front(DeferredPlatformTask::WebSocketTerminal(task));
        return result;
    }
    // The fail-the-connection networking task fires error then close before
    // its microtask checkpoint. Retrying close never repeats a published error.
    if task.error {
        task.error = false;
        task.delivery = None;
        return deliver(scope, state, task);
    }
    Ok(())
}

fn attach_close(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    code: u16,
    reason: &str,
    clean: bool,
) -> Result<(), V8BindingError> {
    let payload = v8::Array::new(scope, 3);
    let brand_name = v8::String::new(scope, "oryn.native.close-event")
        .ok_or(V8BindingError::Allocation("CloseEvent brand"))?;
    let brand = v8::Private::for_api(scope, Some(brand_name));
    let branded = v8::Boolean::new(scope, true);
    if payload.set_private(scope, brand, branded.into()) != Some(true) {
        return Err(V8BindingError::Allocation("CloseEvent brand"));
    }
    let code = v8::Integer::new_from_unsigned(scope, code.into());
    let reason = v8_scalar_string(scope, reason)?;
    let clean = v8::Boolean::new(scope, clean);
    if payload.set_index(scope, 0, code.into()) != Some(true)
        || payload.set_index(scope, 1, reason) != Some(true)
        || payload.set_index(scope, 2, clean.into()) != Some(true)
    {
        return Err(V8BindingError::Allocation("CloseEvent payload"));
    }
    event.set_internal_field(2, payload.into());
    Ok(())
}
pub(super) fn initialize_close_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let code = event_init_member(scope, init, "code").ok_or(V8BindingError::JavaScriptException)?;
    let code = code
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let code = if !code.is_finite() {
        0
    } else {
        code.trunc().rem_euclid(65536.0) as u16
    };
    let reason =
        event_init_member(scope, init, "reason").ok_or(V8BindingError::JavaScriptException)?;
    let reason = if reason.is_undefined() {
        String::new()
    } else {
        reason
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope)
    };
    let clean = event_init_member(scope, init, "wasClean")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    attach_close(scope, event, code, &reason, clean)
}
pub(super) fn close_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or(V8BindingError::Message(
            "TypeError: invalid CloseEvent receiver".into(),
        ))?;
    let brand_name = v8::String::new(scope, "oryn.native.close-event")
        .ok_or(V8BindingError::Allocation("CloseEvent brand"))?;
    let brand = v8::Private::for_api(scope, Some(brand_name));
    if !payload
        .get_private(scope, brand)
        .is_some_and(|value| value.is_true())
    {
        return Err(V8BindingError::Message(
            "TypeError: invalid CloseEvent receiver".into(),
        ));
    }
    payload
        .get_index(
            scope,
            match name {
                "code" => 0,
                "reason" => 1,
                "wasClean" => 2,
                _ => return Err(V8BindingError::Allocation("CloseEvent member")),
            },
        )
        .ok_or(V8BindingError::Allocation("CloseEvent payload"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<(HostOperationId, PlatformHostRequest)>>,
        decisions: RefCell<Vec<(HostOperationId, bool)>>,
        abandoned: RefCell<Vec<u64>>,
    }
    impl PlatformHost for Host {
        fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            self.requests.borrow_mut().push((id, request));
            Ok(())
        }
        fn cancel(&self, _: HostOperationId) {}
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
            Vec::new()
        }
        fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
            self.decisions.borrow_mut().push((id, accepted));
        }
        fn abandon_websocket(&self, id: u64) {
            self.abandoned.borrow_mut().push(id);
        }
    }
    #[test]
    fn constructor_converts_before_admission_and_preserves_iterator_exceptions() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        *state.platform_host.borrow_mut() = Some(host.clone());
        let source=v8::String::new(scope,r#"
          function rejects(f,name){try{f()}catch(e){if(e.name===name)return;throw e}throw Error('accepted '+name)}
          rejects(()=>new WebSocket(),'TypeError');
          for(const url of ['ftp://a.test/','wss://a.test/#','wss://a.test/#x'])rejects(()=>new WebSocket(url),'SyntaxError');
          for(const protocols of [['x','x'],[''],['a b'],['é']])rejects(()=>new WebSocket('wss://a.test/',protocols),'SyntaxError');
          let sentinel={},closed=false;
          let protocols={*[Symbol.iterator](){try{yield {toString(){throw sentinel}}}finally{closed=true}}};
          try{new WebSocket('invalid:URL',protocols);throw Error('accepted')}catch(e){if(e!==sentinel)throw e}
          if(!closed)throw Error('iterator not closed');
          globalThis.ws=new WebSocket('https://a.test/socket',new Set(['x','X']));
          if(ws.url!=='wss://a.test/socket'||ws.extensions!=='')throw Error('normalization');
          ws.binaryType={toString(){ws.binaryType='arraybuffer';return 'invalid'}};
          if(ws.binaryType!=='arraybuffer')throw Error('enum reentry');
          try{ws.binaryType={toString(){throw sentinel}};throw Error('accepted')}catch(e){if(e!==sentinel)throw e}
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(host.requests.borrow().len(), 1);
        assert!(
            matches!(&host.requests.borrow()[0].1,PlatformHostRequest::WebSocketOpen{url,protocols}if url=="wss://a.test/socket"&&protocols==&vec!["x".to_string(),"X".to_string()])
        );
    }
    #[test]
    fn connecting_close_rejects_late_handshake_and_delivers_typed_terminal_once() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        *state.platform_host.borrow_mut() = Some(host.clone());
        let source=v8::String::new(scope,"globalThis.log=[];globalThis.ws=new WebSocket('wss://a.test/');ws.onopen=()=>log.push('open');ws.onerror=()=>log.push('error');ws.onclose=e=>log.push([e instanceof CloseEvent,e.code,e.wasClean,ws.readyState].join(':'));ws.close();if(ws.readyState!==2||log.length)throw Error('synchronous terminal')").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let operation = host.requests.borrow()[0].0;
        state
            .handle_platform_completion(
                scope,
                PlatformHostCompletion {
                    operation_id: operation,
                    result: Ok(PlatformHostResult::WebSocketOpened {
                        socket_id: 99,
                        protocol: String::new(),
                    }),
                },
            )
            .unwrap();
        assert!(host.decisions.borrow().contains(&(operation, false)));
        state.drain_platform_completions(scope, 8).unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        let source=v8::String::new(scope,"if(log.join(',')!=='error,true:1006:false:3')throw Error(log);ws.close();if(log.length!==2)throw Error('duplicate')").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn outbound_sends_are_serialized_before_close_and_use_peer_receipt() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        *state.platform_host.borrow_mut() = Some(host.clone());
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::WebSocket(WebSocketRecord {
                lifecycle: Default::default(),
                buffered_amount: 2,
                url: "wss://a.test/".into(),
                ready_state: 1,
                protocol: String::new(),
                binary_type: "blob".into(),
                socket_id: Some(71),
            }),
        );
        enqueue(
            scope,
            &state,
            id,
            PlatformWebSocketFrame::Text { value: "a".into() },
            1,
        )
        .unwrap();
        enqueue(
            scope,
            &state,
            id,
            PlatformWebSocketFrame::Text { value: "b".into() },
            1,
        )
        .unwrap();
        if let PlatformObjectRecord::WebSocket(socket) =
            &mut *platform_object_mut(&state, id).unwrap()
        {
            socket.ready_state = 2;
            socket.lifecycle.close = Some((Some(1000), "request".into()));
        }
        pump(scope, &state, id).unwrap();
        assert_eq!(host.requests.borrow().len(), 1);
        for index in 0..2 {
            let operation = host.requests.borrow()[index].0;
            state
                .handle_platform_completion(
                    scope,
                    PlatformHostCompletion {
                        operation_id: operation,
                        result: Ok(PlatformHostResult::WebSocketSent),
                    },
                )
                .unwrap();
        }
        assert_eq!(host.requests.borrow().len(), 3);
        assert!(matches!(
            &host.requests.borrow()[2].1,
            PlatformHostRequest::WebSocketClose { socket_id: 71, .. }
        ));
        let operation = host.requests.borrow()[2].0;
        state
            .handle_platform_completion(
                scope,
                PlatformHostCompletion {
                    operation_id: operation,
                    result: Ok(PlatformHostResult::WebSocketClosed {
                        code: 3001,
                        reason: "peer".into(),
                        was_clean: true,
                    }),
                },
            )
            .unwrap();
        assert_eq!(&*host.abandoned.borrow(), &[71]);
        assert!(
            matches!(&*platform_object(&state,id).unwrap(),PlatformObjectRecord::WebSocket(socket)if socket.buffered_amount==0&&socket.ready_state==2)
        );
        state.drain_platform_completions(scope, 8).unwrap();
        assert!(
            matches!(&*platform_object(&state,id).unwrap(),PlatformObjectRecord::WebSocket(socket)if socket.ready_state==3)
        );
    }
    #[test]
    fn terminal_listener_failure_retains_cursor_without_replaying_published_prefix() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::WebSocket(WebSocketRecord {
                lifecycle: Default::default(),
                buffered_amount: 0,
                url: "wss://a.test/".into(),
                ready_state: 1,
                protocol: String::new(),
                binary_type: "blob".into(),
                socket_id: None,
            }),
        );
        let object = new_platform_object(scope, &state, "WebSocket", id).unwrap();
        let key = v8::String::new(scope, "socket").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), object.into())
            .unwrap();
        let source=v8::String::new(scope,"globalThis.calls=[];socket.addEventListener('error',()=>calls.push('first'));socket.addEventListener('error',()=>calls.push('once'),{once:true});socket.onclose=e=>calls.push(e.code)").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        terminal(scope, &state, id, 1006, String::new(), false, true).unwrap();
        file_reader::FAILURE.with(|slot| *slot.borrow_mut() = Some(("once_refresh", 0)));
        assert!(state.drain_platform_completions(scope, 8).is_err());
        assert_eq!(state.deferred_platform_tasks.borrow().len(), 1);
        state.drain_platform_completions(scope, 8).unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        let source = v8::String::new(
            scope,
            "if(calls.join(',')!=='first,once,1006')throw Error(calls)",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}

pub(super) struct Events {
    id: u64,
    open: bool,
    frames: VecDeque<PlatformWebSocketFrame>,
    target: v8::Global<v8::Object>,
    delivery: Option<file_reader::Delivery>,
}
pub(super) fn queue_events(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    open: bool,
    frames: Vec<PlatformWebSocketFrame>,
) -> Result<(), V8BindingError> {
    let target = new_platform_object(scope, state, "WebSocket", id)?;
    state
        .deferred_platform_tasks
        .borrow_mut()
        .push_back(DeferredPlatformTask::WebSocketEvents(Events {
            id,
            open,
            frames: frames.into(),
            target: v8::Global::new(scope, target),
            delivery: None,
        }));
    Ok(())
}
pub(super) fn deliver_events(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    mut task: Events,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    let mut more = false;
    let result = (|| {
        loop {
            let (ready, binary, url) = match &*platform_object(state, task.id)? {
                PlatformObjectRecord::WebSocket(socket) => (
                    socket.ready_state,
                    socket.binary_type.clone(),
                    socket.url.clone(),
                ),
                _ => return Err(V8BindingError::Allocation("WebSocket record")),
            };
            if ready == 3 && task.delivery.is_none() {
                return Ok(());
            }
            if task.delivery.is_none() {
                let event = if task.open {
                    if ready != 0 {
                        return Ok(());
                    }
                    let event = create_event_object_with_interface(
                        scope, state, "Event", "open", false, false,
                    )?;
                    event
                } else {
                    let Some(frame) = task.frames.front() else {
                        break;
                    };
                    if let PlatformWebSocketFrame::Close { code, reason } = frame {
                        return terminal(
                            scope,
                            state,
                            task.id,
                            code.unwrap_or(1005),
                            reason.clone(),
                            true,
                            false,
                        );
                    }
                    if ready != 1 {
                        task.frames.pop_front();
                        continue;
                    }
                    let data = match frame {
                        PlatformWebSocketFrame::Text { value } => v8_scalar_string(scope, value)?,
                        PlatformWebSocketFrame::Binary { bytes } => {
                            if binary == "arraybuffer" {
                                array_buffer_from_bytes(scope, bytes).into()
                            } else {
                                let id = register_platform_object(
                                    state,
                                    PlatformObjectRecord::Blob(BlobRecord {
                                        bytes: bytes.clone(),
                                        media_type: String::new(),
                                        file: None,
                                    }),
                                );
                                new_platform_object(scope, state, "Blob", id)?.into()
                            }
                        }
                        _ => unreachable!(),
                    };
                    let event = create_event_object_with_interface(
                        scope,
                        state,
                        "MessageEvent",
                        "message",
                        false,
                        false,
                    )?;
                    message_events::initialize_empty(scope, state, event)
                        .ok_or(V8BindingError::Allocation("WebSocket message"))?;
                    let fields = event
                        .get_internal_field(scope, 2)
                        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                        .ok_or(V8BindingError::Allocation("WebSocket message fields"))?;
                    let origin = url::Url::parse(&url)
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_default();
                    let origin = v8_scalar_string(scope, &origin)?;
                    if fields.set_index(scope, 0, data) != Some(true)
                        || fields.set_index(scope, 1, origin) != Some(true)
                    {
                        return Err(V8BindingError::Allocation("WebSocket message fields"));
                    }
                    event
                };
                let id = event_id(scope, event)?;
                if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                    record.is_trusted = true;
                }
                task.delivery = Some(file_reader::Delivery::from_event(
                    scope,
                    event,
                    if task.open { "open" } else { "message" },
                )?);
                if task.open {
                    if let PlatformObjectRecord::WebSocket(socket) =
                        &mut *platform_object_mut(state, task.id)?
                    {
                        socket.protocol = socket.lifecycle.protocol.take().unwrap_or_default();
                        socket.ready_state = 1;
                    }
                }
            }
            task.delivery
                .as_mut()
                .unwrap()
                .resume(scope, state, task.id, &task.target)?;
            task.delivery = None;
            if task.open {
                task.open = false
            } else {
                task.frames.pop_front();
            }
            // One received message is one networking task, including its
            // microtask checkpoint. Do not batch author callbacks into a task.
            if !task.frames.is_empty() {
                more = true;
                return Ok(());
            }
        }
        let socket_id = match &*platform_object(state, task.id)? {
            PlatformObjectRecord::WebSocket(socket) if socket.ready_state == 1 => socket.socket_id,
            _ => None,
        };
        if let Some(socket_id) = socket_id {
            if submit_platform_operation(
                state,
                PlatformHostRequest::WebSocketPoll {
                    socket_id,
                    max_frames: 32,
                },
                PendingPlatformOperation::WebSocketPoll {
                    websocket_id: task.id,
                },
            )
            .is_err()
            {
                terminal(scope, state, task.id, 1006, String::new(), false, true)?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        state
            .deferred_platform_tasks
            .borrow_mut()
            .push_front(DeferredPlatformTask::WebSocketEvents(task));
    } else if more {
        state
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::WebSocketEvents(task));
    }
    result
}
