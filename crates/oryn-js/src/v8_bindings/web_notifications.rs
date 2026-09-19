//! Web Notification construction and storage data ownership. Native platform
//! permission authority is read directly; no author value grants permission.
//! Display delivery requires a platform notification backend (not yet attached).
use super::*;
use crate::platform_control::{PermissionDescriptor, PermissionState, Request, Response};
fn failure() -> V8BindingError {
    V8BindingError::JavaScriptException
}
fn private<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.notification.values").ok_or_else(failure)?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn brand(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or_else(failure)?;
    let id = stateful_id(scope, object)?;
    let valid = matches!(
        state.platform_objects.borrow().get(&id),
        Some(PlatformObjectRecord::WebNotification)
    );
    if !valid {
        throw_type_error(scope, "Notification receiver required");
        return Err(failure());
    }
    Ok((state, id))
}
fn permission(state: &V8BindingState) -> &'static str {
    if !state.secure_context.get() {
        return "denied";
    }
    let descriptor = PermissionDescriptor {
        name: "notifications".into(),
        sysex: None,
        pan_tilt_zoom: None,
        allow_without_gesture: None,
    };
    match native_platform::request(state, Request::Permission { descriptor }) {
        Ok(Response::Permission {
            state: PermissionState::Granted,
            ..
        }) => "granted",
        Ok(Response::Permission {
            state: PermissionState::Denied,
            ..
        }) => "denied",
        _ => "default",
    }
}
fn permission_getter(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    if let Some(value) = v8::String::new(scope, permission(&state)) {
        rv.set(value.into())
    }
}
fn request_permission(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let callback = args.get(0);
    if !callback.is_undefined() && !callback.is_function() {
        throw_type_error(scope, "Notification permission callback must be callable");
        return;
    }
    // Without a native prompt/display adapter, the UA leaves an undecided
    // permission at default. Existing native grants/denials remain authoritative.
    let Some(value) = v8::String::new(scope, permission(&state)) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    if let Ok(callback) = v8::Local::<v8::Function>::try_from(callback) {
        v8::tc_scope!(let tc,scope);
        let receiver = v8::undefined(tc);
        callback.call(tc, receiver.into(), &[value.into()]);
    }
    resolver.resolve(scope, value.into());
    rv.set(resolver.get_promise(scope).into());
}
fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        brand(scope, args.this())?;
        let key = private(scope)?;
        let values = args
            .this()
            .get_private(scope, key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(failure)?;
        values.get(scope, args.data()).ok_or_else(failure)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn close(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    if let Err(error) = brand(scope, args.this()) {
        throw_binding_error(scope, &error)
    }
}
fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<'s, v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or_else(failure)?;
    match options {
        Some(options) => options.get(scope, key.into()).ok_or_else(failure),
        None => Ok(v8::undefined(scope).into()),
    }
}
fn put(
    scope: &mut v8::PinScope,
    values: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, name).ok_or_else(failure)?;
    values
        .create_data_property(scope, key.into(), value)
        .ok_or_else(failure)?;
    Ok(())
}
pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    if args.length() == 0 {
        throw_type_error(scope, "Notification requires a title");
        return Err(failure());
    }
    let title = args.get(0).to_string(scope).ok_or_else(failure)?;
    let options = args.get(1);
    let options = if options.is_null_or_undefined() {
        None
    } else {
        match v8::Local::<v8::Object>::try_from(options) {
            Ok(value) => Some(value),
            Err(_) => {
                throw_type_error(scope, "Notification options require a dictionary");
                return Err(failure());
            }
        }
    };
    let values = v8::Object::new(scope);
    put(scope, values, "title", title.into())?;
    // Web IDL dictionary getters run in lexicographic member order. Data is
    // serialized only after all option conversions, before permission/display.
    let mut data = v8::null(scope).into();
    let mut renotify = false;
    let mut tag_empty = true;
    let mut silent = false;
    let mut vibration = false;
    let mut actions_present = false;
    for name in [
        "actions",
        "badge",
        "body",
        "data",
        "dir",
        "icon",
        "image",
        "lang",
        "navigate",
        "renotify",
        "requireInteraction",
        "silent",
        "tag",
        "timestamp",
        "vibrate",
    ] {
        let value = get(scope, options, name)?;
        let converted: v8::Local<v8::Value> = match name {
            "data" => {
                data = if value.is_undefined() {
                    v8::null(scope).into()
                } else {
                    value
                };
                continue;
            }
            "actions" => {
                if !value.is_undefined() {
                    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
                        throw_type_error(scope, "Notification actions require a sequence");
                        failure()
                    })?;
                    let key = v8::Symbol::get_iterator(scope);
                    let method = object.get(scope, key.into()).ok_or_else(failure)?;
                    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| {
                        throw_type_error(scope, "Notification actions require an iterable");
                        failure()
                    })?;
                    iterable_conversion::consume(scope, object, method, |scope, entry| {
                        actions_present = true;
                        let entry = if entry.is_null_or_undefined() {
                            None
                        } else {
                            Some(v8::Local::<v8::Object>::try_from(entry).map_err(|_| {
                                throw_type_error(
                                    scope,
                                    "Notification action requires a dictionary",
                                );
                                failure()
                            })?)
                        };
                        for field in ["action", "icon", "navigate", "title"] {
                            let value = get(scope, entry, field)?;
                            if value.is_undefined() && matches!(field, "action" | "title") {
                                throw_type_error(
                                    scope,
                                    "Notification action requires action and title",
                                );
                                return Err(failure());
                            }
                            if !value.is_undefined() {
                                value.to_string(scope).ok_or_else(failure)?;
                            }
                        }
                        Ok(())
                    })?;
                }
                let array = v8::Array::new(scope, 0);
                array
                    .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                    .ok_or_else(failure)?;
                array.into()
            }
            "vibrate" => {
                let mut numbers = Vec::new();
                if !value.is_undefined() {
                    vibration = true;
                    let iterable = if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
                        let key = v8::Symbol::get_iterator(scope);
                        let method = object.get(scope, key.into()).ok_or_else(failure)?;
                        if method.is_null_or_undefined() {
                            None
                        } else {
                            Some((
                                object,
                                v8::Local::<v8::Function>::try_from(method).map_err(|_| {
                                    throw_type_error(scope, "vibrate requires an iterable");
                                    failure()
                                })?,
                            ))
                        }
                    } else {
                        None
                    };
                    if let Some((object, method)) = iterable {
                        iterable_conversion::consume(scope, object, method, |scope, value| {
                            numbers
                                .push(value.uint32_value(scope).ok_or_else(failure)?.min(10_000));
                            Ok(())
                        })?;
                    } else {
                        numbers.push(value.uint32_value(scope).ok_or_else(failure)?.min(10_000));
                    }
                }
                numbers.truncate(99);
                if numbers.len() % 2 == 0 {
                    numbers.pop();
                }
                let entries = numbers
                    .into_iter()
                    .map(|value| v8::Integer::new_from_unsigned(scope, value).into())
                    .collect::<Vec<_>>();
                let array = v8::Array::new_with_elements(scope, &entries);
                array
                    .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                    .ok_or_else(failure)?;
                array.into()
            }
            "renotify" | "requireInteraction" => {
                let flag = value.boolean_value(scope);
                if name == "renotify" {
                    renotify = flag;
                }
                v8::Boolean::new(scope, flag).into()
            }
            "silent" => {
                if value.is_null_or_undefined() {
                    v8::null(scope).into()
                } else {
                    silent = value.boolean_value(scope);
                    v8::Boolean::new(scope, silent).into()
                }
            }
            "timestamp" => {
                let number = if value.is_undefined() {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64()
                        * 1000.
                } else {
                    value.number_value(scope).ok_or_else(failure)?
                };
                v8::Number::new(scope, number).into()
            }
            _ => {
                let text = if value.is_undefined() {
                    v8::String::new(scope, if name == "dir" { "auto" } else { "" })
                        .ok_or_else(failure)?
                } else {
                    value.to_string(scope).ok_or_else(failure)?
                };
                if name == "dir"
                    && !matches!(
                        text.to_rust_string_lossy(scope).as_str(),
                        "auto" | "ltr" | "rtl"
                    )
                {
                    throw_type_error(scope, "Invalid Notification direction");
                    return Err(failure());
                }
                if name == "tag" {
                    tag_empty = text.length() == 0;
                }
                if matches!(name, "badge" | "icon" | "image" | "navigate") && !value.is_undefined()
                {
                    let text = text.to_rust_string_lossy(scope);
                    let base = if state.binding_global == BindingGlobal::Worker {
                        state.worker_base_url.borrow().clone().unwrap_or_default()
                    } else {
                        document_base_url(scope, state, true)?
                    };
                    let resolved = url::Url::parse(&base)
                        .ok()
                        .and_then(|base| base.join(&text).ok())
                        .or_else(|| url::Url::parse(&text).ok())
                        .map(|url| url.to_string())
                        .unwrap_or_default();
                    v8::String::new(scope, &resolved)
                        .ok_or_else(failure)?
                        .into()
                } else {
                    text.into()
                }
            }
        };
        put(scope, values, name, converted)?;
    }
    let wire = structured_clone::serialize_storage(scope, data).ok_or_else(failure)?;
    if actions_present || renotify && tag_empty || silent && vibration {
        throw_type_error(scope, "Invalid non-persistent Notification options");
        return Err(failure());
    }
    let data = structured_clone::deserialize_storage(scope, wire).ok_or_else(failure)?;
    put(scope, values, "data", data)?;
    let id = register_platform_object(state, PlatformObjectRecord::WebNotification);
    bind_platform_object(scope, state, id, args.this());
    let key = private(scope)?;
    args.this()
        .set_private(scope, key, values.into())
        .ok_or_else(failure)?;
    // Permission refusal and an unavailable native display service both produce
    // the standard asynchronous error event, never a fabricated show receipt.
    state
        .deferred_platform_tasks
        .borrow_mut()
        .push_back(DeferredPlatformTask::Event {
            target_id: id,
            event_type: "error",
        });
    platform_listener_graph::reconcile_activity(scope, state);
    Ok(())
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    constructor: v8::Local<v8::Function>,
    prototype: v8::Local<v8::Object>,
    window: bool,
) -> Result<(), V8BindingError> {
    for name in [
        "title",
        "dir",
        "lang",
        "body",
        "navigate",
        "tag",
        "image",
        "icon",
        "badge",
        "vibrate",
        "timestamp",
        "renotify",
        "silent",
        "requireInteraction",
        "data",
        "actions",
    ] {
        let key = v8::String::new(scope, name).ok_or_else(failure)?;
        let get = v8::Function::builder(getter)
            .data(key.into())
            .build(scope)
            .ok_or_else(failure)?;
        let undefined = v8::undefined(scope);
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), undefined.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        prototype
            .define_property(scope, key.into(), &descriptor)
            .ok_or_else(failure)?;
    }
    let function = v8::Function::new(scope, close).ok_or_else(failure)?;
    set_object_property(scope, prototype, "close", function.into())?;
    let key = v8::String::new(scope, "permission").ok_or_else(failure)?;
    let get = v8::Function::new(scope, permission_getter).ok_or_else(failure)?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    constructor
        .define_property(scope, key.into(), &descriptor)
        .ok_or_else(failure)?;
    let zero = v8::Integer::new(scope, 0);
    set_object_property(scope, constructor.into(), "maxActions", zero.into())?;
    if window {
        let function = v8::Function::new(scope, request_permission).ok_or_else(failure)?;
        set_object_property(
            scope,
            constructor.into(),
            "requestPermission",
            function.into(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn notification_storage_rejects_shared_memory_in_graph_order_before_permission() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let shared = v8::SharedArrayBuffer::new(scope, 8).unwrap();
        let global = context.global(scope);
        set_object_property(scope, global, "shared", shared.into()).unwrap();
        assert!(eval(scope,r#"(()=>{let before=false,after=false;try{new Notification('message',{data:[{get x(){before=true}},shared,{get x(){after=true}}]});throw Error('accepted shared memory')}catch(e){if(e.name!=='DataCloneError')throw e}if(!before||after)throw Error('clone ordering');const marker={};try{new Notification('message',{get data(){throw marker}});throw Error('getter ignored')}catch(e){if(e!==marker)throw e}return true})()"#).is_true());
    }
    #[test]
    fn notification_data_is_stable_cloned_native_storage_and_failure_is_async() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const data={x:1};data.self=data;globalThis.note=new Notification('title',{body:'body',data,vibrate:[10,20,30]});globalThis.errors=0;note.onerror=e=>{if(!e.isTrusted||e.target!==note)throw Error('event');errors++};data.x=2;return note.data===note.data&&note.data!==data&&note.data.self===note.data&&note.data.x===1&&note.title==='title'&&note.body==='body'&&Object.isFrozen(note.vibrate)&&errors===0})()"#).is_true());
        state.drain_platform_completions(scope, 16).unwrap();
        assert!(eval(scope, "errors===1").is_true());
    }
}
