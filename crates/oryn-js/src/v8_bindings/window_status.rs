//! Native legacy Window status text. Canonical IDL owns the accessor.
use super::*;

fn slot<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, "oryn.native.window.status")?;
    Some(v8::Private::for_api(scope, Some(name)))
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

    fn install(
        scope: &mut v8::PinScope,
        context: v8::Local<v8::Context>,
        id: u32,
    ) -> Rc<V8BindingState> {
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(id),
            origin: crate::origin::Origin::from_url("https://status.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        install_generated_bindings(scope, context, dom, RealmId(id), 1).unwrap()
    }

    #[test]
    fn window_status_uses_receiver_realm_and_survives_document_open() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let first = v8::Context::new(handles, Default::default());
        let (donor, getter, setter) = {
            let scope = &mut v8::ContextScope::new(handles, first);
            let donor = install(scope, first, 81);
            eval(scope, "window.status='donor'");
            let getter = eval(
                scope,
                "Object.getOwnPropertyDescriptor(window,'status').get",
            );
            let getter = v8::Global::new(scope, getter);
            let setter = eval(
                scope,
                "Object.getOwnPropertyDescriptor(window,'status').set",
            );
            (donor, getter, v8::Global::new(scope, setter))
        };
        let second = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, second);
        let receiver = install(scope, second, 82);
        for (name, value) in [("borrowedGet", &getter), ("borrowedSet", &setter)] {
            let name = v8::String::new(scope, name).unwrap();
            let value = v8::Local::new(scope, value);
            assert_eq!(
                second
                    .global(scope)
                    .create_data_property(scope, name.into(), value),
                Some(true)
            );
        }
        assert!(eval(scope, "window.status==='' && borrowedGet.call(window)===''").is_true());
        assert!(
            eval(
                scope,
                "borrowedSet.call(window,'receiver'); window.status==='receiver'"
            )
            .is_true()
        );
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            assert!(eval(scope, "window.status==='donor'").is_true());
        }
        donor.retire_execution();
        assert!(eval(scope, "borrowedSet.call(window,'after donor retirement'); borrowedGet.call(window)==='after donor retirement'").is_true());
        assert!(eval(scope, "document.open(); document.write('<p>new tree</p>'); document.close(); borrowedGet.call(window)==='after donor retirement'").is_true());
        receiver.retire_execution();
        assert!(
            eval(
                scope,
                "(()=>{try{borrowedGet.call(window);return false}catch(e){return true}})()"
            )
            .is_true()
        );
        assert!(eval(scope, "(()=>{let converted=false;try{borrowedSet.call(window,{toString(){converted=true;return 'bad'}});return false}catch(e){return !converted}})()").is_true());
    }

    #[test]
    fn window_status_security_precedes_coercion_and_rechecks_retirement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let first = v8::Context::new(handles, Default::default());
        let second = v8::Context::new(handles, Default::default());
        let token = v8::String::new(handles, "status-shared-origin").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let owner = {
            let scope = &mut v8::ContextScope::new(handles, second);
            let owner = install(scope, second, 84);
            eval(scope, "window.status='target'");
            owner
        };
        let scope = &mut v8::ContextScope::new(handles, first);
        install(scope, first, 83);
        let foreign = second.global(scope);
        let name = v8::String::new(scope, "other").unwrap();
        first
            .global(scope)
            .create_data_property(scope, name.into(), foreign.into());
        assert!(
            eval(
                scope,
                "Object.getOwnPropertyDescriptor(window,'status').get.call(other)==='target'"
            )
            .is_true()
        );
        let different = v8::String::new(scope, "status-different-origin").unwrap();
        second.set_security_token(different.into());
        assert!(eval(scope, r#"(()=>{
            const d=Object.getOwnPropertyDescriptor(window,'status');
            let converted=false;
            for(const run of [()=>d.get.call(other),()=>d.set.call(other,{toString(){converted=true;return 'bad'}})]){
                try{run();return false}catch(e){if(e.name!=='SecurityError')return false}
            }
            return !converted;
        })()"#).is_true());
        second.set_security_token(token.into());
        let scope = &mut v8::ContextScope::new(scope, second);
        assert!(eval(scope, "window.status==='target'").is_true());
        let retire = v8::Function::new(
            scope,
            |scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {
                scope
                    .get_current_context()
                    .get_slot::<V8BindingState>()
                    .unwrap()
                    .retire_execution();
            },
        )
        .unwrap();
        let name = v8::String::new(scope, "retireOwner").unwrap();
        second
            .global(scope)
            .create_data_property(scope, name.into(), retire.into());
        eval(
            scope,
            "window.status={toString(){retireOwner();return 'must not publish'}}",
        );
        assert!(owner.execution_retired());
        let key = slot(scope).unwrap();
        let stored = second.global(scope).get_private(scope, key).unwrap();
        assert_eq!(stored.to_rust_string_lossy(scope), "target");
    }
}

pub(super) fn get(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if state.execution_retired() {
        throw_type_error(scope, "Window.status receiver is retired");
        return;
    }
    if state.binding_global != BindingGlobal::Window
        || !global_events::is_global(scope, state, args.this())
    {
        throw_type_error(scope, "Window.status requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, state, args.this()) {
        return;
    }
    let Some(key) = slot(scope) else {
        throw_type_error(scope, "Cannot allocate Window status slot");
        return;
    };
    match args
        .this()
        .get_private(scope, key)
        .filter(|value| value.is_string())
    {
        Some(value) => rv.set(value),
        None => match v8::String::new(scope, "") {
            Some(value) => rv.set(value.into()),
            None => throw_type_error(scope, "Cannot allocate Window status text"),
        },
    }
}

pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    args: v8::FunctionCallbackArguments,
) {
    if state.execution_retired() {
        throw_type_error(scope, "Window.status receiver is retired");
        return;
    }
    if state.binding_global != BindingGlobal::Window
        || !global_events::is_global(scope, state, args.this())
    {
        throw_type_error(scope, "Window.status requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, state, args.this()) {
        return;
    }
    // Keep the native V8 string: no lossy Unicode round-trip or authored slot.
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    if state.execution_retired() || !global_events::check_window_access(scope, state, args.this()) {
        return;
    }
    let Some(key) = slot(scope) else {
        throw_type_error(scope, "Cannot allocate Window status slot");
        return;
    };
    if args.this().set_private(scope, key, value.into()) != Some(true) {
        throw_type_error(scope, "Cannot update Window status text");
    }
}
