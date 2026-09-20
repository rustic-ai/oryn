//! Native StorageEvent payloads and Window storage-area wrapper identity.
use super::*;
#[derive(Default)]
struct Areas {
    local: Option<v8::Weak<v8::Object>>,
    session: Option<v8::Weak<v8::Object>>,
}
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "StorageEvent".into(),
        member: "TypeError: incompatible storage event or storage area".into(),
    }
}
fn brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.native.storage-area")
        .ok_or(V8BindingError::Allocation("storage brand"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
pub(super) fn retain(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    object: v8::Local<v8::Object>,
    local: bool,
) -> Result<(), V8BindingError> {
    let key = brand(scope)?;
    let value = v8::Boolean::new(scope, true);
    if object.set_private(scope, key, value.into()) != Some(true) {
        return Err(invalid());
    }
    if context.get_slot::<RefCell<Areas>>().is_none() {
        context.set_slot(Rc::new(RefCell::new(Areas::default())));
    }
    let retained = context_intrinsics::retain_object_in_context(
        scope,
        context,
        if local {
            "storage-area-local"
        } else {
            "storage-area-session"
        },
        object,
    )?;
    let areas = context.get_slot::<RefCell<Areas>>().ok_or_else(invalid)?;
    let mut areas = areas.borrow_mut();
    if local {
        areas.local = Some(retained);
    } else {
        areas.session = Some(retained);
    }
    Ok(())
}
pub(super) fn validate_area<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<(), V8BindingError> {
    area(scope, object.into()).map(|_| ())
}
pub(super) fn owner_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    validate_area(scope, object)?;
    let context = object.get_creation_context(scope).ok_or_else(invalid)?;
    let state = context.get_slot::<V8BindingState>().ok_or_else(invalid)?;
    let global = context.global(scope);
    if !global_events::check_window_access(scope, &state, global) {
        return Err(V8BindingError::JavaScriptException);
    }
    if state.execution_retired() {
        return Err(V8BindingError::Unsupported {
            interface: "Storage".into(),
            member: "InvalidStateError".into(),
        });
    }
    Ok(state)
}

pub(super) fn install_accessor(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    name: &str,
    local: bool,
) -> Result<(), V8BindingError> {
    let context = v8::Local::new(scope, context);
    storage_properties::install_intrinsics(scope, context)?;
    let data = v8::Boolean::new(scope, local);
    let getter = v8::Function::builder(read_area)
        .data(data.into())
        .build(scope)
        .ok_or_else(invalid)?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, name).ok_or_else(invalid)?;
    if context
        .global(scope)
        .define_property(scope, name.into(), &descriptor)
        != Some(true)
    {
        return Err(invalid());
    }
    Ok(())
}
fn read_area(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let receiver = args.this();
        if receiver.is_proxy() {
            return Err(invalid());
        }
        let context = receiver.get_creation_context(scope).ok_or_else(invalid)?;
        let state = context.get_slot::<V8BindingState>().ok_or_else(invalid)?;
        if !context.global(scope).strict_equals(receiver.into()) {
            return Err(invalid());
        }
        if !global_events::check_window_access(scope, &state, receiver) {
            return Err(V8BindingError::JavaScriptException);
        }
        let local = args.data().is_true();
        if native_platform::has_backend(&state) {
            match native_platform::request(
                &state,
                crate::platform_control::Request::Storage {
                    local,
                    operation: crate::platform_control::WindowStorageOperation::Access,
                },
            )? {
                crate::platform_control::Response::Storage { .. } => {}
                _ => return Err(invalid()),
            }
        }
        let areas = context.get_slot::<RefCell<Areas>>().ok_or_else(invalid)?;
        let area = {
            let areas = areas.borrow();
            if local {
                areas.local.clone()
            } else {
                areas.session.clone()
            }
        }
        .ok_or_else(invalid)?;
        area.to_local(scope).ok_or_else(invalid)
    })();
    match result {
        Ok(value) => rv.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn area<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.is_proxy() {
        return Err(invalid());
    }
    let key = brand(scope)?;
    if !object
        .get_private(scope, key)
        .is_some_and(|value| value.is_true())
    {
        return Err(invalid());
    }
    Ok(value)
}
fn string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    nullable: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if nullable && value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    if value.is_undefined() {
        return v8_string(scope, "");
    }
    let text = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if nullable {
        Ok(text.into())
    } else {
        v8_string(scope, &text.to_rust_string_lossy(scope))
    }
}
fn attach(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    values: &[v8::Local<v8::Value>],
) -> Result<(), V8BindingError> {
    let mut items = vec![v8_string(scope, "StorageEvent")?];
    items.extend_from_slice(values);
    let payload = v8::Array::new_with_elements(scope, &items);
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) {
        return Err(invalid());
    }
    event.set_internal_field(2, payload.into());
    Ok(())
}
pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let mut values = Vec::new();
    for name in ["key", "newValue", "oldValue", "storageArea", "url"] {
        let value =
            event_init_member(scope, init, name).ok_or(V8BindingError::JavaScriptException)?;
        values.push(if name == "storageArea" {
            area(scope, value)?
        } else {
            string(scope, value, name != "url")?
        });
    }
    attach(scope, event, &values)
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if event.is_proxy() {
        return Err(invalid());
    }
    event_id(scope, event)?;
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    if !payload.get_index(scope, 0).is_some_and(|value| {
        value.is_string() && value.to_rust_string_lossy(scope) == "StorageEvent"
    }) {
        return Err(invalid());
    }
    let index = ["key", "newValue", "oldValue", "storageArea", "url"]
        .iter()
        .position(|member| *member == name)
        .ok_or_else(invalid)?;
    payload
        .get_index(scope, index as u32 + 1)
        .ok_or_else(invalid)
}
pub(super) fn init_legacy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    if args.length() < 1 {
        return Err(invalid());
    }
    let event_type = required_string(scope, args, 0)?;
    let bubbles = args.get(1).boolean_value(scope);
    let cancelable = args.get(2).boolean_value(scope);
    let key = string(scope, args.get(3), true)?;
    let old = string(scope, args.get(4), true)?;
    let new = string(scope, args.get(5), true)?;
    let url = string(scope, args.get(6), false)?;
    let storage = area(scope, args.get(7))?;
    attribute(scope, event, "key")?;
    let id = event_id(scope, event)?;
    let mut events = state.events.borrow_mut();
    let record = events.get_mut(&id).ok_or_else(invalid)?;
    if record.dispatching {
        return Ok(());
    }
    record.event_type = event_type;
    record.bubbles = bubbles;
    record.cancelable = cancelable;
    record.is_trusted = false;
    record.default_prevented = false;
    record.propagation_stopped = false;
    record.immediate_stopped = false;
    record.target = None;
    record.platform_target = None;
    record.initialized = true;
    drop(events);
    let empty = v8::undefined(scope);
    event.set_internal_field(4, empty.into());
    event.set_internal_field(5, empty.into());
    attach(scope, event, &[key, new, old, storage, url])
}
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    value: &crate::platform_control::StorageEventRecord,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let areas = scope
        .get_current_context()
        .get_slot::<RefCell<Areas>>()
        .ok_or_else(invalid)?;
    let storage = {
        let areas = areas.borrow();
        if value.local {
            areas.local.clone()
        } else {
            areas.session.clone()
        }
    }
    .ok_or_else(invalid)?;
    let storage = storage.to_local(scope).ok_or_else(invalid)?;
    let event =
        create_event_object_with_interface(scope, state, "StorageEvent", "storage", false, false)?;
    let mut values = Vec::new();
    for value in [&value.key, &value.new_value, &value.old_value] {
        values.push(match value {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        });
    }
    values.push(storage.into());
    values.push(v8_string(scope, &value.url)?);
    attach(scope, event, &values)?;
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or_else(invalid)?
        .is_trusted = true;
    Ok(event)
}
#[cfg(test)]
pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: &crate::platform_control::StorageEventRecord,
) -> Result<(), V8BindingError> {
    let event = create(scope, state, value)?;
    global_events::dispatch(scope, state, event).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::super::test_eval::eval;
    use super::*;
    struct NativeStorage(std::sync::atomic::AtomicBool);
    impl crate::platform_control::Backend for NativeStorage {
        fn request(
            &self,
            _creator: crate::worker_wire::Creator,
            request: crate::platform_control::Request,
        ) -> Result<crate::platform_control::Response, String> {
            use crate::platform_control::{Request, Response, WindowStorageOperation as Operation};
            if self.0.load(std::sync::atomic::Ordering::Acquire) {
                return Err("SecurityError: opaque storage".into());
            }
            match request {
                Request::Storage {
                    operation: Operation::Access,
                    ..
                } => Ok(Response::Storage {
                    response: serde_json::Value::Null,
                }),
                Request::Storage {
                    operation: Operation::Get { .. },
                    ..
                } => Ok(Response::Storage {
                    response: serde_json::json!({"result":"value","value":"native"}),
                }),
                Request::Storage {
                    operation: Operation::Set { .. },
                    ..
                } => Err("QuotaExceededError: test quota".into()),
                _ => panic!("unexpected storage request"),
            }
        }
    }
    struct Messages;
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            _request: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    #[test]
    fn native_storage_getters_and_operations_use_sealed_owner_and_reject_restore() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        let backend = std::sync::Arc::new(NativeStorage(std::sync::atomic::AtomicBool::new(false)));
        state.set_native_platform_backend(backend.clone());
        assert!(eval(scope,r#"globalThis.__oryn_context_call=()=>{throw Error('authored transport')};globalThis.__oryn_page_id=999;globalThis.saved=localStorage;let restore=false,quota=false,brand=false;try{__oryn_restore_storage([['key','forged']],[])}catch(e){restore=e instanceof TypeError}try{saved.setItem('key','value')}catch(e){quota=e.name==='QuotaExceededError'}try{Storage.prototype.getItem.call(history,'key')}catch(e){brand=e instanceof TypeError}restore&&quota&&brand&&saved===localStorage&&saved.getItem('key')==='native'&&Object.getOwnPropertyDescriptor(globalThis,'localStorage').set===undefined"#).is_true());
        backend.0.store(true, std::sync::atomic::Ordering::Release);
        assert!(eval(scope,r#"let denied=0;for(const action of [()=>localStorage,()=>sessionStorage,()=>saved.getItem('key')]){try{action()}catch(e){if(e.name==='SecurityError')denied++}}denied===3"#).is_true());
    }
    #[test]
    fn standalone_storage_restore_converts_before_atomic_publish() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"localStorage.setItem('old','retained');let failed=false;try{__oryn_restore_storage([['first','value'],[{toString(){localStorage.getItem('old');throw Error('conversion')}},'bad']],[])}catch(e){failed=e.message==='conversion'}failed&&localStorage.getItem('old')==='retained'&&localStorage.getItem('first')===null"#).is_true());
        assert!(eval(scope,r#"__oryn_restore_storage([[{toString(){localStorage.setItem('inner','temporary');return 'outer'}},'accepted']],[]);localStorage.getItem('outer')==='accepted'&&localStorage.getItem('inner')===null"#).is_true());
    }
    #[test]
    fn storage_event_native_payload_and_area_survive_author_replacements() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"globalThis.SE=StorageEvent;globalThis.area=localStorage;let order=[];const e=new SE('custom',{get key(){order.push('key');return 7},get newValue(){order.push('new');return 'value'},get oldValue(){order.push('old');return null},get storageArea(){order.push('area');return area},get url(){order.push('url');return 'https://writer.test/source'}});let invalid=false;try{new SE('x',{storageArea:Object.create(Storage.prototype)})}catch(error){invalid=error instanceof TypeError}const getter=Object.getOwnPropertyDescriptor(SE.prototype,'key').get;let brand=false;try{getter.call(new Event('x'))}catch(error){brand=error instanceof TypeError}e.key==='7'&&e.oldValue===null&&e.newValue==='value'&&e.storageArea===area&&!e.isTrusted&&order.join(',')==='key,new,old,area,url'&&invalid&&brand"#).is_true());
        eval(
            scope,
            r#"globalThis.seen=[];addEventListener('storage',e=>{seen.push([e instanceof SE,e.isTrusted,e.key,e.oldValue,e.newValue,e.url,e.storageArea===area,e.bubbles,e.cancelable]);e.initStorageEvent('wrong');});globalThis.StorageEvent=()=>{throw Error('author constructor')};globalThis.dispatchEvent=()=>{throw Error('author dispatcher')};globalThis.localStorage={fake:true};"#,
        );
        dispatch(
            scope,
            &state,
            &crate::platform_control::StorageEventRecord {
                id: 1,
                local: true,
                key: Some("key".into()),
                old_value: None,
                new_value: Some("new".into()),
                url: "https://writer.test/path#source".into(),
            },
        )
        .unwrap();
        assert_eq!(
            eval(scope, "JSON.stringify(seen)").to_rust_string_lossy(scope),
            r#"[[true,true,"key",null,"new","https://writer.test/path#source",true,false,false]]"#
        );
        assert!(eval(scope,r#"(()=>{const e=new SE('old');e.initStorageEvent('new',true,true,'k','old','new','https://writer.test/',area);return e.type==='new'&&e.key==='k'&&e.oldValue==='old'&&e.newValue==='new'&&e.storageArea===area&&e.bubbles&&e.cancelable&&!e.isTrusted})()"#).is_true());
    }
}

#[cfg(test)]
mod area_retention_tests {
    use super::*;
    #[test]
    fn original_area_wrappers_remain_identical_then_collect_with_context() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, areas, anchor) = {
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(990), 1).unwrap();
            let areas = context.get_slot::<RefCell<Areas>>().unwrap();
            let first = areas.borrow().local.clone().unwrap();
            let second = areas.borrow().session.clone().unwrap();
            assert!(
                !first
                    .to_local(scope)
                    .unwrap()
                    .strict_equals(second.to_local(scope).unwrap().into())
            );
            let event = super::super::test_eval::eval(scope, r#"(()=>{
                const original=localStorage;const event=new StorageEvent('storage',{storageArea:original});
                Object.defineProperty(globalThis,'localStorage',{value:'replacement',configurable:true});
                if(event.storageArea!==original)throw Error('area identity');return event;
            })()"#).try_cast::<v8::Object>().unwrap();
            install_accessor(scope, context, "localStorage", true).unwrap();
            assert!(
                super::super::test_eval::eval(scope, "localStorage")
                    .strict_equals(first.to_local(scope).unwrap().into())
            );
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, context),
                areas,
                v8::Global::new(scope, event),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let event = v8::Local::new(scope, &anchor);
            let key = v8::String::new(scope, "storageArea").unwrap();
            assert!(
                event.get(scope, key.into()).unwrap().strict_equals(
                    areas
                        .borrow()
                        .local
                        .as_ref()
                        .unwrap()
                        .to_local(scope)
                        .unwrap()
                        .into()
                )
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(context.to_local(handles).is_none());
        assert!(
            areas
                .borrow()
                .local
                .as_ref()
                .unwrap()
                .to_local(handles)
                .is_none()
        );
        assert!(
            areas
                .borrow()
                .session
                .as_ref()
                .unwrap()
                .to_local(handles)
                .is_none()
        );
    }
}

#[cfg(test)]
mod native_area_upgrade_tests {
    use super::*;

    #[test]
    fn native_storage_events_upgrade_each_original_area_without_public_getters() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(991), 1).unwrap();
        super::super::test_eval::eval(
            scope,
            r#"
            globalThis.storageSeen=[];
            const originalLocal=localStorage, originalSession=sessionStorage;
            addEventListener('storage',event=>storageSeen.push([
                event.storageArea===originalLocal ? 'local' :
                    event.storageArea===originalSession ? 'session' : 'wrong',
                event.isTrusted,event.key,event.oldValue,event.newValue,event.url,
                event.bubbles,event.cancelable
            ]));
            for(const name of ['localStorage','sessionStorage']) {
                Object.defineProperty(globalThis,name,{
                    configurable:true,get(){throw Error('author storage getter')}
                });
            }
        "#,
        );
        for local in [true, false] {
            dispatch(
                scope,
                &state,
                &crate::platform_control::StorageEventRecord {
                    id: if local { 1 } else { 2 },
                    local,
                    key: Some("key".into()),
                    old_value: Some("old".into()),
                    new_value: None,
                    url: "https://writer.test/path#source".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(
            super::super::test_eval::eval(scope, "JSON.stringify(storageSeen)")
                .to_rust_string_lossy(scope),
            r#"[["local",true,"key","old",null,"https://writer.test/path#source",false,false],["session",true,"key","old",null,"https://writer.test/path#source",false,false]]"#
        );
    }

    #[test]
    fn collected_storage_area_fails_before_event_creation_and_live_cache_recovers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (retired_context, collected_area) = {
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(992), 1).unwrap();
            let areas = context.get_slot::<RefCell<Areas>>().unwrap();
            let area = areas.borrow().local.clone().unwrap();
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (v8::Weak::new(scope, context), area)
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(retired_context.to_local(handles).is_none());
        assert!(collected_area.to_local(handles).is_none());
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(993), 1).unwrap();
        let areas = context.get_slot::<RefCell<Areas>>().unwrap();
        for local in [true, false] {
            let record = crate::platform_control::StorageEventRecord {
                id: 1,
                local,
                key: None,
                old_value: None,
                new_value: None,
                url: "https://writer.test/".into(),
            };
            // The live Context normally traces its own areas. Inject an
            // actually collected weak handle only at the native cache boundary
            // to exercise defensive failure without altering that lifetime rule.
            let original = {
                let mut areas = areas.borrow_mut();
                let slot = if local {
                    &mut areas.local
                } else {
                    &mut areas.session
                };
                slot.replace(collected_area.clone()).unwrap()
            };
            let events_before = state.events.borrow().len();
            let next_event_before = state.next_event_id.get();
            let Err(error) = create(scope, &state, &record) else {
                panic!("collected storage area must not produce an event");
            };
            assert!(matches!(
                error,
                V8BindingError::Unsupported { interface, member }
                    if interface == "StorageEvent"
                        && member == "TypeError: incompatible storage event or storage area"
            ));
            assert_eq!(state.events.borrow().len(), events_before);
            assert_eq!(state.next_event_id.get(), next_event_before);
            {
                let mut areas = areas.borrow_mut();
                let slot = if local {
                    &mut areas.local
                } else {
                    &mut areas.session
                };
                *slot = Some(original.clone());
            }
            let event = create(scope, &state, &record).unwrap();
            assert!(
                attribute(scope, event, "storageArea")
                    .unwrap()
                    .strict_equals(original.to_local(scope).unwrap().into())
            );
            assert_eq!(state.events.borrow().len(), events_before + 1);
        }
    }
}
