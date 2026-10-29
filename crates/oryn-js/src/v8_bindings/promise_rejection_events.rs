//! Native PromiseRejectionEvent payloads; author objects remain V8-traced edges.
use super::*;

pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<()> {
    if args.length() < 2 {
        throw_type_error(
            scope,
            "PromiseRejectionEvent requires type and eventInitDict",
        );
        return None;
    }
    let event_type = args.get(0).to_string(scope)?;
    let event_type = rust_string_preserving_utf16(scope, event_type).ok()?;
    let value = args.get(1);
    let init = if value.is_null_or_undefined() {
        None
    } else if value.is_object() {
        Some(value.to_object(scope)?)
    } else {
        throw_type_error(scope, "PromiseRejectionEventInit must be an object");
        return None;
    };
    // Inherited members first, then lexicographic own members. Conversion and
    // author getters complete before any native event allocation.
    let bubbles = event_init_member(scope, init, "bubbles")?.boolean_value(scope);
    let cancelable = event_init_member(scope, init, "cancelable")?.boolean_value(scope);
    let composed = event_init_member(scope, init, "composed")?.boolean_value(scope);
    let promise = event_init_member(scope, init, "promise")?;
    if !promise.is_object() {
        throw_type_error(
            scope,
            "PromiseRejectionEventInit.promise requires an object",
        );
        return None;
    }
    let reason = event_init_member(scope, init, "reason")?;
    let fields = v8::Array::new_with_elements(scope, &[promise, reason]);
    let null = v8::null(scope);
    fields.set_prototype(scope, null.into())?;
    let id = allocate_constructor_event_id(scope, state)?;
    state.events.borrow_mut().insert(
        id,
        EventRecord {
            progress: None,
            picture_in_picture_window: None,
            time_stamp: state.performance_now(),
            event_type,
            target: None,
            current_target: None,
            platform_target: None,
            platform_path: Vec::new(),
            platform_current_target: None,
            bubbles,
            cancelable,
            composed,
            is_trusted: false,
            default_prevented: false,
            before_unload_return_value: None,
            propagation_stopped: false,
            immediate_stopped: false,
            in_passive_listener: false,
            dispatching: false,
            initialized: true,
            phase: 0,
        },
    );
    let object = args.this();
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    tag_event_object(scope, state, object);
    install_payload(scope, state, object, fields);
    Some(())
}

fn install_payload(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    fields: v8::Local<v8::Array>,
) {
    object.set_internal_field(2, fields.into());
    let brand = {
        let mut brand = state.promise_rejection_event_brand.borrow_mut();
        if brand.is_none() {
            let symbol = v8::Symbol::new(scope, None);
            *brand = Some(v8::Global::new(scope, symbol));
        }
        v8::Local::new(scope, brand.as_ref().unwrap())
    };
    object.set_internal_field(3, brand.into());
}

pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    promise: v8::Local<v8::Promise>,
    reason: v8::Local<v8::Value>,
    handled: bool,
) -> Result<bool, V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired Promise rejection event owner"));
    }
    let current = scope.get_current_context().get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("Promise rejection event context"))?;
    if !Rc::ptr_eq(&current, state) {
        return Err(V8BindingError::Allocation("foreign Promise rejection event owner"));
    }
    let fields = v8::Array::new_with_elements(scope, &[promise.into(), reason]);
    let null = v8::null(scope);
    fields.set_prototype(scope, null.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let event_type = if handled { "rejectionhandled" } else { "unhandledrejection" };
    // Native templates bypass author replacement of constructors and getters.
    let event = create_event_object_with_interface(
        scope, state, "PromiseRejectionEvent", event_type, false, !handled,
    )?;
    install_payload(scope, state, event, fields);
    let id = event_id(scope, event)?;
    state.events.borrow_mut().get_mut(&id)
        .ok_or(V8BindingError::Allocation("Promise rejection event"))?.is_trusted = true;
    global_events::dispatch(scope, state, event)
}

pub(super) fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Allocation("PromiseRejectionEvent receiver");
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    event_id(scope, object)?;
    let expected = state.promise_rejection_event_brand.borrow();
    let expected = expected.as_ref().ok_or_else(invalid)?;
    let expected = v8::Local::new(scope, expected);
    let brand = object
        .get_internal_field(scope, 3)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .ok_or_else(invalid)?;
    if !brand.strict_equals(expected.into()) {
        return Err(invalid());
    }
    let fields = object
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let index = match member {
        "promise" => 0,
        "reason" => 1,
        _ => return Err(invalid()),
    };
    fields.get_index(scope, index).ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_task(scope: &mut v8::PinScope, state: &Rc<V8BindingState>, id: u64)
        -> Result<(), V8BindingError>
    {
        scope.get_slot_mut::<Vec<u64>>().unwrap().push(id);
        if id == 1 { state.queue_promise_rejection_task(scope, 2, record_task)?; }
        Ok(())
    }

    #[test]
    fn promise_rejection_tasks_observe_checkpoints_and_owner_epoch() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_slot(Vec::<u64>::new());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.queue_promise_rejection_task(scope, 1, record_task).unwrap();
        let capacity = state.deferred_platform_tasks.borrow().capacity();
        assert_eq!(state.deferred_task_queue_storage.borrow().as_ref().unwrap().0, capacity);
        let bytes = capacity * std::mem::size_of::<DeferredPlatformTask>();
        assert!(state.idb_payload_ledger.live_bytes() >= bytes);
        assert!(state.has_ready_platform_work());
        state.drain_platform_completions(scope, 16).unwrap();
        assert_eq!(scope.get_slot::<Vec<u64>>().unwrap(), &[1]);
        assert!(state.has_ready_platform_work());
        state.drain_platform_completions(scope, 16).unwrap();
        assert_eq!(scope.get_slot::<Vec<u64>>().unwrap(), &[1, 2]);
        // Empty task queues retain their backing allocation. Its admission
        // must likewise remain live rather than following individual tokens.
        assert!(state.idb_payload_ledger.live_bytes() >= bytes);
        assert_eq!(state.deferred_task_queue_storage.borrow().as_ref().unwrap().0,
            state.deferred_platform_tasks.borrow().capacity());
        state.queue_promise_rejection_task(scope, 3, record_task).unwrap();
        state.advance_document_stream_epoch().unwrap();
        state.drain_platform_completions(scope, 16).unwrap();
        // document.open changes parser stream, not queued realm tasks.
        assert_eq!(scope.get_slot::<Vec<u64>>().unwrap(), &[1, 2, 3]);
        state.queue_promise_rejection_task(scope, 4, record_task).unwrap();
        state.retire_execution();
        assert!(state.queue_promise_rejection_task(scope, 5, record_task).is_err());
        state.drain_platform_completions(scope, 16).unwrap();
        assert_eq!(scope.get_slot::<Vec<u64>>().unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn promise_rejection_native_dispatch_uses_templates_on_each_global() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        for (index, global) in [BindingGlobal::Window, BindingGlobal::Worker, BindingGlobal::AudioWorklet]
            .into_iter().enumerate()
        {
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
            let document = dom.borrow().document();
            let state = install_generated_bindings_with_root_for_global(
                scope, context, dom, RealmId(index as u32), 1, document, global,
            ).unwrap();
            if global == BindingGlobal::AudioWorklet {
                // Exposed=* includes worklets, but their global does not inherit
                // EventTarget. Verify exposure without inventing global methods.
                let script = v8::String::new(scope, r#"
                  (()=>{const p={};const r={};const e=new PromiseRejectionEvent('x',{promise:p,reason:r});
                    return e.promise===p&&e.reason===r&&e instanceof Event;
                  })()
                "#).unwrap();
                assert!(v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap().is_true());
                continue;
            }
            let promise = v8::PromiseResolver::new(scope).unwrap().get_promise(scope);
            let reason = v8::Object::new(scope);
            let root = context.global(scope);
            let key = v8::String::new(scope, "nativePromise").unwrap();
            root.set(scope, key.into(), promise.into()).unwrap();
            let key = v8::String::new(scope, "nativeReason").unwrap();
            root.set(scope, key.into(), reason.into()).unwrap();
            let script = v8::String::new(scope, r#"
              globalThis.saved=[];
              globalThis.eventPrototype=PromiseRejectionEvent.prototype;
              globalThis.PromiseRejectionEvent=function(){throw Error('public constructor called')};
              addEventListener('unhandledrejection',e=>{saved.push(e);e.preventDefault()});
              addEventListener('rejectionhandled',e=>{saved.push(e);e.preventDefault()});
            "#).unwrap();
            v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
            assert!(!dispatch(scope, &state, promise, reason.into(), false).unwrap());
            assert!(dispatch(scope, &state, promise, reason.into(), true).unwrap());
            let script = v8::String::new(scope, r#"
              (()=>{
                if(saved.length!==2)throw Error('delivery');
                for(const [i,e] of saved.entries()){
                  if(Object.getPrototypeOf(e)!==eventPrototype||!e.isTrusted||e.promise!==nativePromise||e.reason!==nativeReason)
                    throw Error('native identity');
                  if(e.bubbles||e.composed||e.cancelable!==(i===0)||e.defaultPrevented!==(i===0))
                    throw Error('native flags');
                }
                return true;
              })()
            "#).unwrap();
            assert!(v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap().is_true());
            // Holding a different realm's state must not authorize dispatch.
            let other = v8::Context::new(scope, Default::default());
            {
                let foreign = &mut v8::ContextScope::new(scope, other);
                assert!(dispatch(foreign, &state, promise, reason.into(), false).is_err());
            }
        }
    }
}
