//! Window posted-message tasks. Serialization and origin authority are native;
//! author replacements of MessageEvent or dispatchEvent cannot intercept delivery.
use super::*;
use crate::{origin::Origin, realm::RealmDescriptor};

pub(super) struct Queue {
    origin: Origin,
    pub(super) tasks: VecDeque<Message>,
    bytes: usize,
    delivering: bool,
}

impl Queue {
    pub(super) fn new(context: v8::Local<v8::Context>) -> Self {
        Self {
            origin: context
                .get_slot::<RealmDescriptor>()
                .map(|realm| realm.origin.clone())
                .unwrap_or_else(Origin::opaque),
            tasks: VecDeque::new(),
            bytes: 0,
            delivering: false,
        }
    }

    pub(super) fn clear(&mut self, state: &V8BindingState) {
        for message in self.tasks.drain(..) {
            message.delivery.cancel(state);
        }
        self.bytes = 0;
    }
}

pub type SourceResolver =
    Rc<dyn Fn(&mut v8::PinScope) -> Result<v8::Global<v8::Value>, V8BindingError>>;
enum Source {
    Local(v8::Global<v8::Object>),
    External(v8::Global<v8::Value>),
    Resolve(SourceResolver),
}
pub(super) struct Message {
    delivery: message_delivery::Delivery,
    target_origin: Option<Origin>,
    source: Option<Source>,
    admitted: bool,
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(post)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.postMessage"))?;
    let key = v8::String::new(scope, "postMessage")
        .ok_or(V8BindingError::Allocation("postMessage name"))?;
    function.set_name(key);
    context
        .global(scope)
        .create_data_property(scope, key.into(), function.into());
    Ok(())
}

fn post(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let receiver = args.this();
    let Some(context) = receiver.get_creation_context(scope) else {
        return;
    };
    let Some(target) = context.get_slot::<V8BindingState>().filter(|state| {
        state.binding_global == BindingGlobal::Window
            && global_events::is_global(scope, state, receiver)
    }) else {
        throw_type_error(scope, "postMessage requires a Window receiver");
        return;
    };
    post_to(scope, &args, &target);
}

/// Cross-origin callable data holds the original target Context in V8's traced
/// graph. The callable is bound to that Window; its receiver is irrelevant.
/// Native origin checks and incumbent-source capture still apply on every call.
pub fn cross_origin_post(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(context) = window_proxy::callback_context(scope, &args) else {
        throw_type_error(scope, "Missing native Window target");
        return;
    };
    let Some(target) = context
        .get_slot::<V8BindingState>()
        .filter(|state| state.binding_global == BindingGlobal::Window)
    else {
        throw_type_error(scope, "postMessage requires a Window target");
        return;
    };
    post_to(scope, &args, &target);
}

fn post_to(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    target: &Rc<V8BindingState>,
) {
    if args.length() == 0 {
        throw_type_error(scope, "postMessage requires a message");
        return;
    }
    let source = crate::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>());
    let result = enqueue(
        scope,
        target,
        source.as_ref(),
        args.get(0),
        args.get(1),
        args.get(2),
    );
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn enqueue(
    scope: &mut v8::PinScope,
    target: &Rc<V8BindingState>,
    source: Option<&Rc<V8BindingState>>,
    value: v8::Local<v8::Value>,
    options: v8::Local<v8::Value>,
    transfer: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    // Web IDL selects the dictionary overload for object/null/undefined.
    let dictionary = options.is_object() || options.is_null_or_undefined();
    let include_activation = if dictionary {
        let Some(include) = structured_clone::include_user_activation(scope, options) else {
            return Ok(());
        };
        include
    } else {
        false
    };
    let origin_value = if dictionary {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(options) {
            let key = v8::String::new(scope, "targetOrigin").unwrap();
            let Some(value) = object.get(scope, key.into()) else {
                return Ok(());
            };
            value
        } else {
            v8::undefined(scope).into()
        }
    } else {
        options
    };
    let origin_text = if origin_value.is_undefined() && dictionary {
        "/".into()
    } else {
        let Some(text) = origin_value.to_string(scope) else {
            return Ok(());
        };
        text.to_rust_string_lossy(scope)
    };
    let transfer_options = if dictionary {
        options
    } else {
        let object = v8::Object::new(scope);
        let key = v8::String::new(scope, "transfer").unwrap();
        object.create_data_property(scope, key.into(), transfer);
        object.into()
    };
    let Some(transfers) = structured_clone::transfer_list(scope, transfer_options) else {
        return Ok(());
    };
    let source_origin = source
        .map(|state| state.window_messages.borrow().origin.clone())
        .unwrap_or_else(Origin::opaque);
    let target_origin = match origin_text.as_str() {
        "*" => None,
        "/" => Some(source_origin.clone()),
        text => {
            if Url::parse(text).is_err() {
                let error = dom_exception_value(scope, "SyntaxError")?;
                scope.throw_exception(error);
                return Ok(());
            }
            Some(Origin::from_url(text))
        }
    };
    let Some(mut payload) = structured_clone::serialize(scope, value, &transfers) else {
        return Ok(());
    };
    payload.set_user_activation(include_activation.then(|| {
        source.map_or([false, false], |source| {
            [
                source.has_transient_user_activation(),
                source.has_sticky_user_activation(),
            ]
        })
    }));
    // Serialization and transfer side effects occur even for an inactive target.
    if target.execution_retired() {
        return Ok(());
    }
    let source = source
        .filter(|state| state.binding_global == BindingGlobal::Window)
        .and_then(|state| state.global_object.to_local(scope))
        .map(|window| v8::Global::new(scope, window));
    let bytes = payload.retained_bytes();
    let mut queue = target.window_messages.borrow_mut();
    if queue.tasks.len() >= 1024 || bytes > (32 * 1024 * 1024usize).saturating_sub(queue.bytes) {
        drop(queue);
        let error = dom_exception_value(scope, "QuotaExceededError")?;
        scope.throw_exception(error);
        return Ok(());
    }
    queue.bytes += bytes;
    queue.tasks.push_back(Message {
        delivery: message_delivery::Delivery::local(payload).with_origin(source_origin.serialize()),
        target_origin,
        source: source.map(Source::Local),
        admitted: false,
    });
    Ok(())
}

/// Run one posted-message task in its destination realm. The host performs a
/// microtask checkpoint afterwards, just as for other native event tasks.
struct DeliveryGuard {
    state: Rc<V8BindingState>,
    message: Option<Message>,
    bytes: usize,
}
impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        let mut queue = self.state.window_messages.borrow_mut();
        queue.delivering = false;
        if let Some(message) = self.message.take() {
            if !self.state.execution_retired() {
                queue.tasks.push_front(message)
            } else {
                queue.bytes = queue.bytes.saturating_sub(self.bytes);
                message.delivery.cancel(&self.state)
            }
        } else {
            queue.bytes = queue.bytes.saturating_sub(self.bytes);
        }
    }
}
pub fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    let message = {
        let mut queue = state.window_messages.borrow_mut();
        if queue.delivering {
            return Ok(false);
        }
        let Some(mut message) = queue.tasks.pop_front() else {
            return Ok(false);
        };
        if state.execution_retired()
            || !message.admitted
                && message
                    .target_origin
                    .as_ref()
                    .is_some_and(|origin| origin != &queue.origin)
        {
            queue.bytes = queue.bytes.saturating_sub(message.delivery.bytes());
            message.delivery.cancel(state);
            return Ok(true);
        }
        message.admitted = true;
        queue.delivering = true;
        message
    };
    let bytes = message.delivery.bytes();
    let mut guard = DeliveryGuard {
        state: state.clone(),
        message: Some(message),
        bytes,
    };
    let message = guard.message.as_mut().unwrap();
    if let Some(source) = &message.source {
        let value = match source {
            Source::Local(source) => {
                let source = v8::Local::new(scope, source);
                v8::Global::new(scope, v8::Local::<v8::Value>::from(source))
            }
            Source::External(source) => source.clone(),
            Source::Resolve(resolve) => resolve(scope)?,
        };
        message.delivery.set_source(value);
        message.source = None;
    }
    if !message
        .delivery
        .resume(scope, state, global_events::GLOBAL_TARGET)?
    {
        return Ok(false);
    }
    guard.message.take();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use v8::MapFnTo;

    fn execute(scope: &mut v8::PinScope, source: &str) -> String {
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None).unwrap().run(scope);
        if let Some(error) = scope.exception() {
            panic!("{}", error.to_rust_string_lossy(scope));
        }
        value.unwrap().to_rust_string_lossy(scope)
    }

    fn original_window(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut rv: v8::ReturnValue,
    ) {
        let context = window_proxy::callback_context(scope, &args).unwrap();
        rv.set(context.global(scope).into());
    }

    #[test]
    fn cross_origin_window_messages_ignore_overrides_and_bind_the_native_target() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let caller_template = v8::ObjectTemplate::new(scope);
        window_proxy::configure_template(scope, caller_template);
        let caller = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(caller_template),
                ..Default::default()
            },
        );
        let template = v8::ObjectTemplate::new(scope);
        window_proxy::configure_template(scope, template);
        let target = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(template),
                ..Default::default()
            },
        );
        let mut states = Vec::new();
        for (id, context, url) in [
            (0, caller, "https://caller.test"),
            (1, target, "https://target.test"),
        ] {
            let origin = Origin::from_url(url);
            context.set_security_token(
                v8::String::new(scope, &origin.security_token_key())
                    .unwrap()
                    .into(),
            );
            context.set_slot(Rc::new(RealmDescriptor {
                id: RealmId(id),
                origin,
                document_generation: 1,
                frame_id: None,
            }));
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            states.push(install_generated_bindings(scope, context, dom, RealmId(id), 1).unwrap());
        }
        for context in [caller, target] {
            window_proxy::install_context(
                context,
                window_proxy::native_callbacks(window_proxy::WindowLifecycleCallbacks {
                    close: original_window.map_fn_to(),
                    closed: original_window.map_fn_to(),
                    focus: original_window.map_fn_to(),
                    blur: original_window.map_fn_to(),
                    opener: original_window.map_fn_to(),
                    navigate: original_window.map_fn_to(),
                }),
            );
        }
        {
            let scope = &mut v8::ContextScope::new(scope, target);
            let child_window = target.global(scope);
            let parent_window = caller.global(scope);
            set_native_window_parent(scope, child_window, parent_window).unwrap();
            set_native_window_top(scope, child_window, parent_window).unwrap();
            let key = v8::String::new(scope, "expectedSource").unwrap();
            target.global(scope).create_data_property(
                scope,
                key.into(),
                caller.global(scope).into(),
            );
            execute(
                scope,
                r#"
                globalThis.results=[];globalThis.authorCalled=false;
                const prototype=Object.getPrototypeOf(window);
                if(prototype!==Window.prototype || !(window instanceof EventTarget))throw Error('native prototype chain');
                if(!Reflect.setPrototypeOf(window,prototype) || Reflect.setPrototypeOf(window,{}))throw Error('Window prototype mutability');
                try{Object.setPrototypeOf(window,{})}catch(e){if(!(e instanceof TypeError))throw e}
                if(Object.getPrototypeOf(window)!==prototype)throw Error('Window prototype changed');
                addEventListener('message',e=>{
                    results.push([e.data instanceof Map,e.data.get('x'),
                        e.source===expectedSource,e.origin,e.isTrusted]);
                    e.source.postMessage('reply'+e.data.get('x'),'*');
                });
                Object.defineProperty(globalThis,'postMessage',{configurable:true,get(){authorCalled=true;throw Error('author getter')}});
                Object.defineProperty(globalThis,'parent',{configurable:true,get(){authorCalled=true;throw Error('author parent getter')}});
                frames='override';close=23;focus='override';blur=null;
                'ready'
            "#,
            );
        }
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            let key = v8::String::new(scope, "foreignWindow").unwrap();
            caller.global(scope).create_data_property(
                scope,
                key.into(),
                target.global(scope).into(),
            );
            assert_eq!(
                execute(
                    scope,
                    r#"
                globalThis.saved=foreignWindow.postMessage;
                globalThis.replies=[];
                addEventListener('message',e=>replies.push([e.data,e.origin,e.source===foreignWindow]));
                const check=(value,message)=>{if(!value)throw Error(message)};
                check(saved instanceof Function && saved===foreignWindow.postMessage,'caller realm and cache');
                check(parent===window,'root parent');
                check(foreignWindow.parent===window && foreignWindow.top===window,'native ancestry');
                check(foreignWindow.frames===foreignWindow && foreignWindow.self===foreignWindow,'native aliases');
                check(foreignWindow.length===0,'native child count');
                try{foreignWindow.location.href;throw Error('Location leaked')}catch(e){check(e.name==='SecurityError','Location protection')}
                for(const key of ['close','focus','blur','postMessage']) {
                    const f=foreignWindow[key];check(f.name===key,'method name');
                    check(f.length===(key==='postMessage'?1:0),'method length');
                    check(Object.getOwnPropertyDescriptor(foreignWindow,key).value===f,'method descriptor');
                    try{new f();throw Error('constructable')}catch(e){check(e instanceof TypeError,'constructor error realm')}
                }
                for(const key of ['window','self','location','closed','frames','length','top','opener','parent']) {
                    const d=Object.getOwnPropertyDescriptor(foreignWindow,key);
                    check(d.get.name==='get '+key && d.get.length===0,'getter metadata');
                    if(key==='location')check(d.set.name==='set location' && d.set.length===1,'setter metadata');
                }
                saved(new Map([['x',1]]),'*');
                saved.call({},new Map([['x',2]]),{targetOrigin:'https://TARGET.test:443/path'});
                saved(new Map([['x',3]]));
                saved(new Map([['x',4]]),'https://caller.test');
                try{saved();throw Error('missing argument')}catch(e){check(e instanceof TypeError,'argument error realm')}
                try{foreignWindow.document;throw Error('DOM leaked')}catch(e){check(e.name==='SecurityError','DOM protection')}
                'ok'
            "#
                ),
                "ok"
            );
        }
        {
            let scope = &mut v8::ContextScope::new(scope, target);
            assert_eq!(execute(scope, "results.length"), "0");
            for _ in 0..4 {
                assert!(deliver(scope, &states[1]).unwrap());
            }
            assert!(!deliver(scope, &states[1]).unwrap());
            assert_eq!(
                execute(scope, "JSON.stringify(results)"),
                r#"[[true,1,true,"https://caller.test",true],[true,2,true,"https://caller.test",true]]"#
            );
            assert_eq!(execute(scope, "authorCalled"), "false");
        }
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            assert_eq!(execute(scope, "replies.length"), "0");
            assert!(deliver(scope, &states[0]).unwrap());
            assert!(deliver(scope, &states[0]).unwrap());
            assert!(!deliver(scope, &states[0]).unwrap());
            assert_eq!(
                execute(scope, "JSON.stringify(replies)"),
                r#"[["reply1","https://target.test",true],["reply2","https://target.test",true]]"#
            );
        }
        states[1].retire_execution();
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            assert_eq!(
                execute(
                    scope,
                    "const buffer=new ArrayBuffer(2);saved(buffer,'*',[buffer]);buffer.byteLength"
                ),
                "0"
            );
        }
        assert!(!states[1].has_pending_window_message());
    }
}

pub fn throw_external_message_error(scope: &mut v8::PinScope, name: &str) {
    if let Ok(error) = dom_exception_value(scope, name) {
        scope.throw_exception(error);
    }
}
/// Destination-realm construction and native dispatch, without consulting an
/// author MessageEvent constructor, JSON functions or dispatchEvent property.
fn queue_external(
    state: &Rc<V8BindingState>,
    payload: crate::message_wire::WireValue,
    origin: &str,
    source: Source,
) -> Result<(), V8BindingError> {
    let delivery = message_delivery::Delivery::wire(state, payload, origin.into());
    if state.execution_retired() {
        return Ok(());
    }
    let mut queue = state.window_messages.borrow_mut();
    let bytes = delivery.bytes();
    if queue.tasks.len() >= 1024 || bytes > (32 * 1024 * 1024usize).saturating_sub(queue.bytes) {
        return Err(V8BindingError::Allocation(
            "external message queue capacity",
        ));
    }
    queue.bytes += bytes;
    queue.tasks.push_back(Message {
        delivery,
        target_origin: None,
        source: Some(source),
        admitted: false,
    });
    Ok(())
}
/// Admit the native payload before resolving its source WindowProxy. Failure
/// to create that wrapper retains both payload and resolver for the next task.
pub fn queue_external_window(
    state: &Rc<V8BindingState>,
    payload: crate::message_wire::WireValue,
    origin: &str,
    source: SourceResolver,
) -> Result<(), V8BindingError> {
    queue_external(state, payload, origin, Source::Resolve(source))
}
pub fn deliver_external(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    payload: crate::message_wire::WireValue,
    source: v8::Local<v8::Value>,
    origin: &str,
) -> Result<(), V8BindingError> {
    let source = v8::Global::new(scope, source);
    queue_external(state, payload, origin, Source::External(source))?;
    deliver(scope, state).map(|_| ())
}

/// Shared-worker connect uses the same queued trusted-message lifecycle; its
/// transferred connection port is also the event source.
pub(super) fn connect(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    token: crate::message_wire::PortToken,
) -> Result<(), V8BindingError> {
    let data = match v8::String::new(scope, "") {
        Some(data) => data,
        None => {
            if let Some(transport) = state.message_transport.borrow().clone() {
                let _ = transport
                    .request(crate::message_wire::MessageRequest::ClosePort { port: token });
            }
            return Err(V8BindingError::Allocation("connect data"));
        }
    };
    let mut wire = match structured_clone::serialize_wire(scope, data.into(), &[]) {
        Some(wire) => wire,
        None => {
            if let Some(transport) = state.message_transport.borrow().clone() {
                let _ = transport
                    .request(crate::message_wire::MessageRequest::ClosePort { port: token });
            }
            return Err(V8BindingError::Allocation("connect serialization"));
        }
    };
    wire.ports.push(token);
    let delivery = message_delivery::Delivery::wire(state, wire, String::new()).connect();
    let bytes = delivery.bytes();
    let mut queue = state.window_messages.borrow_mut();
    if queue.tasks.len() >= 1024 || bytes > (32 * 1024 * 1024usize).saturating_sub(queue.bytes) {
        return Err(V8BindingError::Allocation("connect queue capacity"));
    }
    queue.bytes += bytes;
    queue.tasks.push_back(Message {
        delivery,
        target_origin: None,
        source: None,
        admitted: false,
    });
    drop(queue);
    deliver(scope, state).map(|_| ())
}

#[cfg(test)]
mod retained_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn window_message_handlers_keep_registration_order_and_consumed_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope, "onmessage===null&&onmessageerror===null").is_true());
        eval(
            scope,
            r#"
            globalThis.seen=[];globalThis.saved=[];globalThis.fields=[];
            onmessage=()=>seen.push('obsolete');
            addEventListener('message',event=>seen.push('listener:'+event.data));
            onmessage=function(event){
                seen.push('handler:'+event.data);saved.push(event);
                fields.push(this===window&&event.target===window&&event.currentTarget===window&&
                    event.source===window&&event.origin==='null'&&event.isTrusted&&event.eventPhase===2);
            };
            postMessage('one','*');postMessage('two','*');
        "#,
        );
        native_platform::publication::fail_after_next_callback();
        assert!(deliver(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='handler:one'&&fields[0]").is_true());
        assert!(state.has_pending_window_message());
        assert!(deliver(scope, &state).unwrap());
        assert!(deliver(scope, &state).unwrap());
        assert!(!deliver(scope, &state).unwrap());
        assert!(eval(scope,"seen.join(',')==='handler:one,listener:one,handler:two,listener:two'&&fields.every(Boolean)&&saved.every(event=>event instanceof MessageEvent&&event.currentTarget===null&&event.eventPhase===0)").is_true());
        eval(scope, "onmessage=null;postMessage('three','*')");
        assert!(deliver(scope, &state).unwrap());
        eval(
            scope,
            "onmessage=event=>seen.push('reinstalled:'+event.data);postMessage('four','*')",
        );
        assert!(deliver(scope, &state).unwrap());
        assert!(eval(scope,"seen.join(',')==='handler:one,listener:one,handler:two,listener:two,listener:three,listener:four,reinstalled:four'").is_true());
        assert!(!deliver(scope, &state).unwrap());
    }

    #[test]
    fn document_open_clears_window_handlers_but_keeps_queued_messages() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"
            globalThis.seen=[];globalThis.fields=[];
            globalThis.originalWindow=window;globalThis.originalDocument=document;
            onmessage=()=>seen.push('obsolete-handler');
            onmessageerror=()=>seen.push('obsolete-error');
            addEventListener('message',()=>seen.push('obsolete-listener'));
            postMessage('before','*');document.open();
            globalThis.handlersCleared=onmessage===null&&onmessageerror===null;
            onmessage=function(event){
                seen.push(event.data);
                fields.push(this===originalWindow&&event.source===originalWindow&&
                    event.target===originalWindow&&event.currentTarget===originalWindow&&event.isTrusted);
            };
            postMessage('after','*');document.write('<body>replacement');document.close();
        "#,
        );
        assert!(state.has_pending_window_message());
        assert!(eval(scope,"handlersCleared&&seen.length===0&&window===originalWindow&&document===originalDocument").is_true());
        assert!(deliver(scope, &state).unwrap());
        assert!(deliver(scope, &state).unwrap());
        assert!(!deliver(scope, &state).unwrap());
        assert!(eval(scope,"seen.join(',')==='before,after'&&fields.length===2&&fields.every(Boolean)&&window===originalWindow&&document===originalDocument").is_true());
    }

    #[test]
    fn malformed_wire_window_messageerror_handler_retains_trusted_delivery() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"
            globalThis.seen=[];globalThis.saved=null;globalThis.fields=false;
            onmessage=()=>seen.push('unexpected-message');
            onmessageerror=function(event){
                seen.push('handler');saved=event;
                fields=this===window&&event instanceof MessageEvent&&event.type==='messageerror'&&
                    event.isTrusted&&event.target===window&&event.currentTarget===window&&
                    event.source===null&&event.origin==='https://sender.test'&&event.data===null&&
                    event.ports.length===0&&Object.isFrozen(event.ports);
            };
            addEventListener('messageerror',()=>seen.push('listener'));
        "#,
        );
        let wire = crate::message_wire::WireValue {
            version: 1,
            bytes: vec![0],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
        };
        queue_external_window(
            &state,
            wire,
            "https://sender.test",
            Rc::new(|scope| {
                let source = v8::null(scope);
                Ok(v8::Global::new(scope, v8::Local::<v8::Value>::from(source)))
            }),
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(deliver(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='handler'&&fields").is_true());
        assert!(state.has_pending_window_message());
        assert!(deliver(scope, &state).unwrap());
        assert!(!deliver(scope, &state).unwrap());
        assert!(eval(scope,"seen.join(',')==='handler,listener'&&saved.currentTarget===null&&saved.eventPhase===0").is_true());
    }

    #[test]
    fn window_messages_retain_origin_source_and_callback_progress() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.seen=[];globalThis.saved=null;addEventListener('message',e=>{saved=e;seen.push('first:'+e.data)},{once:true});addEventListener('message',e=>{if(e.source!==self)throw Error('source');seen.push('last:'+e.data)});postMessage('one','*');postMessage('two','*');",
        );
        let template = state
            .instance_templates
            .borrow_mut()
            .remove("MessageEvent")
            .unwrap();
        assert!(deliver(scope, &state).is_err());
        state
            .instance_templates
            .borrow_mut()
            .insert("MessageEvent", template);
        native_platform::publication::fail_after_next_callback();
        assert!(deliver(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='first:one'").is_true());
        assert!(deliver(scope, &state).unwrap());
        assert!(deliver(scope, &state).unwrap());
        assert!(!deliver(scope, &state).unwrap());
        assert!(eval(scope,"seen.join(',')==='first:one,last:one,last:two'&&saved.eventPhase===0&&saved instanceof MessageEvent").is_true());
    }
    #[test]
    fn external_window_source_preparation_preserves_native_payload() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.result='';onmessage=e=>result=e.data+'|'+e.origin+'|'+(e.source===null)",
        );
        let data = eval(scope, "'payload'");
        let wire = structured_clone::serialize_wire(scope, data, &[]).unwrap();
        let attempts = Rc::new(Cell::new(0));
        let calls = attempts.clone();
        queue_external_window(
            &state,
            wire,
            "https://sender.test",
            Rc::new(move |scope| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    return Err(V8BindingError::Allocation("source preparation"));
                }
                let null = v8::null(scope);
                Ok(v8::Global::new(scope, v8::Local::<v8::Value>::from(null)))
            }),
        )
        .unwrap();
        assert!(deliver(scope, &state).is_err());
        assert!(state.has_pending_window_message());
        assert!(deliver(scope, &state).unwrap());
        assert_eq!(attempts.get(), 2);
        assert!(eval(scope, "result==='payload|https://sender.test|true'").is_true());
    }
}
