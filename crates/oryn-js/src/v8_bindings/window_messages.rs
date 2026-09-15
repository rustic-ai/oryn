//! Window posted-message tasks. Serialization and origin authority are native;
//! author replacements of MessageEvent or dispatchEvent cannot intercept delivery.
use super::*;
use crate::{origin::Origin, realm::RealmDescriptor};

pub(super) struct Queue {
    origin: Origin,
    pub(super) tasks: VecDeque<Message>,
    bytes: usize,
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
        }
    }

    pub(super) fn clear(&mut self) {
        self.tasks.clear();
        self.bytes = 0;
    }
}

pub(super) struct Message {
    payload: structured_clone::SerializedValue,
    target_origin: Option<Origin>,
    source_origin: String,
    source: Option<v8::Global<v8::Object>>,
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
    let Some(payload) = structured_clone::serialize(scope, value, &transfers) else {
        return Ok(());
    };
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
        payload,
        target_origin,
        source_origin: source_origin.serialize(),
        source,
    });
    Ok(())
}

/// Run one posted-message task in its destination realm. The host performs a
/// microtask checkpoint afterwards, just as for other native event tasks.
pub fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    let message = {
        let mut queue = state.window_messages.borrow_mut();
        let Some(message) = queue.tasks.pop_front() else {
            return Ok(false);
        };
        queue.bytes -= message.payload.retained_bytes();
        if state.execution_retired()
            || message
                .target_origin
                .as_ref()
                .is_some_and(|origin| origin != &queue.origin)
        {
            return Ok(true);
        }
        message
    };
    let Some((data, ports)) = structured_clone::deserialize_with_ports(scope, message.payload)
    else {
        return Err(V8BindingError::Allocation("Window message deserialization"));
    };
    let event = message_events::delivery(scope, state, data, &ports)?;
    let fields = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Window message fields"))?;
    let origin = v8::String::new(scope, &message.source_origin)
        .ok_or(V8BindingError::Allocation("Window message origin"))?;
    fields.set_index(scope, 1, origin.into());
    let source = message
        .source
        .as_ref()
        .map(|source| v8::Local::new(scope, source).into())
        .unwrap_or_else(|| v8::null(scope).into());
    fields.set_index(scope, 3, source);
    global_events::dispatch(scope, state, event)?;
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
        let mut callbacks = [original_window.map_fn_to(); 13];
        callbacks[12] = cross_origin_post.map_fn_to();
        for context in [caller, target] {
            window_proxy::install_context(
                context,
                window_proxy::WindowCrossOriginCallbacks {
                    properties: callbacks,
                    location_setter: original_window.map_fn_to(),
                },
            );
        }
        {
            let scope = &mut v8::ContextScope::new(scope, target);
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
                close=23;focus='override';blur=null;
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
