//! Explicit native historical-runner authority. Production installs no hooks.
use super::*;

/// Revocation invalidates retained functions as well as current globals. Enabling
/// creates new native credentials bound to this state and document lifetime.
pub fn set_historical_input_authority(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    enabled: bool,
) -> Result<(), V8BindingError> {
    let context = scope.get_current_context();
    let current = context
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("historical input context"))?;
    if !Rc::ptr_eq(&current, state)
        || state.execution_retired()
        || state.binding_global != BindingGlobal::Window
    {
        return Err(V8BindingError::Message(
            "historical input realm mismatch".into(),
        ));
    }
    state.historical_input_authority.set(None);
    if !enabled {
        return Ok(());
    }
    if !cfg!(feature = "historical-test-input") {
        return Err(V8BindingError::Message(
            "historical input feature is disabled".into(),
        ));
    }
    let serial =
        state
            .historical_input_serial
            .get()
            .checked_add(1)
            .ok_or(V8BindingError::Allocation(
                "historical input authority exhausted",
            ))?;
    state.historical_input_serial.set(serial);
    let stamp = (
        state.document_generation(),
        state.document_stream_epoch(),
        serial,
    );
    let data = v8::Array::new(scope, 4);
    for (index, value) in [state.instance_id, stamp.0, stamp.1, stamp.2]
        .into_iter()
        .enumerate()
    {
        let value = v8::BigInt::new_from_u64(scope, value);
        data.set_index(scope, index as u32, value.into());
    }
    macro_rules! install {
        ($name:literal,$callback:ident) => {{
            let function = v8::Function::builder($callback)
                .data(data.into())
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("historical input function"))?;
            let name = v8::String::new(scope, $name)
                .ok_or(V8BindingError::Allocation("historical input name"))?;
            if context
                .global(scope)
                .set(scope, name.into(), function.into())
                != Some(true)
            {
                return Err(V8BindingError::JavaScriptException);
            }
        }};
    }
    install!("__oryn_edit_control", edit);
    install!("__oryn_dispatch_user_input", dispatch);
    install!("__oryn_native_drag_input", drag);
    install!("__oryn_select_file_bytes", files);
    install!("__oryn_resolve_keyboard_target", keyboard);
    install!("__oryn_pointer_boundary_snapshot", boundary);
    install!("__oryn_focus_from_pointer", focus);
    state.historical_input_authority.set(Some(stamp));
    Ok(())
}
fn permitted(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> bool {
    if !cfg!(feature = "historical-test-input") {
        return false;
    }
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return false;
    };
    let Some(stamp) = state.historical_input_authority.get() else {
        return false;
    };
    if state.execution_retired()
        || (stamp.0, stamp.1) != (state.document_generation(), state.document_stream_epoch())
    {
        return false;
    }
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return false;
    };
    for (index, expected) in [state.instance_id, stamp.0, stamp.1, stamp.2]
        .into_iter()
        .enumerate()
    {
        let Some(value) = data.get_index(scope, index as u32) else {
            return false;
        };
        let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) else {
            return false;
        };
        if value.u64_value() != (expected, true) {
            return false;
        }
    }
    true
}
macro_rules! guarded {
    ($name:ident,$callback:path) => {
        fn $name(
            scope: &mut v8::PinScope,
            args: v8::FunctionCallbackArguments,
            rv: v8::ReturnValue,
        ) {
            if !permitted(scope, &args) {
                throw_binding_error(
                    scope,
                    &V8BindingError::Message(
                        "historical native input authority expired or absent".into(),
                    ),
                );
                return;
            }
            $callback(scope, args, rv);
        }
    };
}
guarded!(edit, edit_control_callback);
guarded!(dispatch, dispatch_user_input_event);
guarded!(drag, native_drag::input_callback);
guarded!(files, file_lists::select_bytes);
guarded!(keyboard, keyboard_input_target);
guarded!(boundary, pointer_boundary_snapshot);
guarded!(focus, focus_from_pointer);

#[cfg(test)]
mod tests {
    use super::*;
    fn run(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .boolean_value(scope)
    }
    #[test]
    fn ordinary_window_has_no_native_input_hooks_even_in_historical_build() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id='t'>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(run(
            scope,
            "['__oryn_edit_control','__oryn_dispatch_user_input','__oryn_native_drag_input','__oryn_select_file_bytes','__oryn_resolve_keyboard_target','__oryn_pointer_boundary_snapshot','__oryn_focus_from_pointer'].every(k=>!(k in globalThis))"
        ));
        #[cfg(not(feature = "historical-test-input"))]
        assert!(set_historical_input_authority(scope, &state, true).is_err());
        // Production native dispatch remains available without author hooks.
        assert!(run(
            scope,
            "globalThis.log=[];t.focus();t.onkeydown=e=>{log.push([e.isTrusted,e instanceof KeyboardEvent,e.key,e.composed]);e.preventDefault()};true"
        ));
        dispatch_native_keyboard(
            scope,
            &state,
            &NativeKeyboardInput {
                key: vec![97],
                down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(run(
            scope,
            "t.value===''&&JSON.stringify(log)==='[[true,true,\"a\",true]]'"
        ));
    }
    #[cfg(feature = "historical-test-input")]
    #[test]
    fn native_revocation_and_renewal_do_not_reauthorize_retained_functions() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id='t'>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        set_historical_input_authority(scope, &state, true).unwrap();
        assert!(run(
            scope,
            "globalThis.old=__oryn_focus_from_pointer;old(t);document.activeElement===t"
        ));
        set_historical_input_authority(scope, &state, false).unwrap();
        assert!(run(
            scope,
            "(()=>{try{old(t);return false}catch(e){return true}})()"
        ));
        set_historical_input_authority(scope, &state, true).unwrap();
        assert!(run(
            scope,
            "(()=>{try{old(t);return false}catch(e){__oryn_focus_from_pointer(t);return true}})()"
        ));
        state
            .document_stream_epoch
            .set(state.document_stream_epoch() + 1);
        assert!(run(
            scope,
            "(()=>{try{__oryn_focus_from_pointer(t);return false}catch(e){return true}})()"
        ));
    }
}
