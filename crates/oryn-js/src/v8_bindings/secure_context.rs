//! Expose the host's committed secure-context result, including ancestry and
//! inherited origins. A URL or an author-modifiable global is not authority.
use super::*;

pub(super) fn get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "isSecureContext requires a native global");
        return;
    };
    if !matches!(
        state.binding_global,
        BindingGlobal::Window | BindingGlobal::Worker
    ) || !global_events::is_global(scope, &state, args.this())
    {
        throw_type_error(scope, "isSecureContext requires a native global");
        return;
    }
    if !global_events::check_window_access(scope, &state, args.this()) {
        return;
    }
    // This cell is seeded from SecureContextState before binding installation.
    // Realm replacement installs a new owner; retained old globals keep their
    // own committed result, not the successor document's URL or policy.
    rv.set_bool(state.secure_context.get());
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let getter = v8::Function::builder(get)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("isSecureContext getter"))?;
    let name = v8::String::new(scope, "get isSecureContext")
        .ok_or(V8BindingError::Allocation("isSecureContext getter name"))?;
    getter.set_name(name);
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_configurable(true);
    descriptor.set_enumerable(true);
    let name = v8::String::new(scope, "isSecureContext")
        .ok_or(V8BindingError::Allocation("isSecureContext name"))?;
    if context
        .global(scope)
        .define_property(scope, name.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("isSecureContext exposure"));
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
    fn native_secure_flag_controls_window_and_every_worker_subtype() {
        super::super::tests::initialize_v8();
        for kind in [
            None,
            Some(crate::worker_wire::AgentKind::Dedicated),
            Some(crate::worker_wire::AgentKind::Shared),
            Some(crate::worker_wire::AgentKind::Service),
        ] {
            for secure in [false, true] {
                for url in [
                    "http://untrusted.test/",
                    "https://trusted.test/",
                    "about:blank",
                ] {
                    let mut isolate = v8::Isolate::new(Default::default());
                    v8::scope!(let scope, &mut isolate);
                    let context = v8::Context::new(scope, Default::default());
                    context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
                    context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                        id: RealmId(0),
                        origin: crate::origin::Origin::from_url(url),
                        document_generation: 1,
                        frame_id: None,
                    }));
                    if let Some(kind) = kind {
                        context.set_slot(Rc::new(WorkerExposureKind(kind)));
                    }
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                    let document = dom.borrow().document();
                    let state = install_generated_bindings_with_root_for_global(
                        scope,
                        context,
                        dom,
                        RealmId(0),
                        1,
                        document,
                        if kind.is_some() {
                            BindingGlobal::Worker
                        } else {
                            BindingGlobal::Window
                        },
                    )
                    .unwrap();
                    assert_eq!(eval(scope, "isSecureContext").is_true(), secure);
                    assert!(eval(scope, r#"(()=>{
                        const d=Object.getOwnPropertyDescriptor(globalThis,'isSecureContext');
                        if(!d || !d.enumerable || !d.configurable || d.set!==undefined || d.get.length!==0 || d.get.name!=='get isSecureContext')throw Error('descriptor');
                        const original=isSecureContext;
                        isSecureContext=!original;
                        if(isSecureContext!==original)throw Error('writable secure flag');
                        try { d.get.call({}); throw Error('forged global accepted'); } catch(e) { if(!(e instanceof TypeError))throw e; }
                        try { d.get.call(new Proxy(globalThis,{})); throw Error('proxy global accepted'); } catch(e) { if(!(e instanceof TypeError))throw e; }
                        return true;
                    })()"#).is_true());
                    state
                        .commit_location(if secure {
                            "http://changed.test/"
                        } else {
                            "https://changed.test/"
                        })
                        .unwrap();
                    assert_eq!(
                        eval(scope, "isSecureContext").is_true(),
                        secure,
                        "same-document URL metadata cannot manufacture authority"
                    );
                }
            }
        }
    }
    #[test]
    fn borrowed_getter_uses_receiver_realm_and_retained_realm_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let token = v8::String::new(scope, "same-origin-secure-context-test").unwrap();
        let first = v8::Context::new(scope, Default::default());
        first.set_security_token(token.into());
        let first_state = {
            first.set_slot(Rc::new(crate::realm::SecureContextState(true)));
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(1), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        second.set_security_token(token.into());
        let _second_state = {
            second.set_slot(Rc::new(crate::realm::SecureContextState(false)));
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(2), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let other = second.global(scope);
        let name = v8::String::new(scope, "other").unwrap();
        first.global(scope).set(scope, name.into(), other.into());
        assert!(eval(scope, "Object.getOwnPropertyDescriptor(globalThis,'isSecureContext').get.call(other)===false&&isSecureContext===true").is_true());
        first_state.retire_execution();
        assert!(
            eval(
                scope,
                "isSecureContext===true&&other.isSecureContext===false"
            )
            .is_true()
        );
        let foreign = v8::String::new(scope, "different-origin-secure-context-test").unwrap();
        second.set_security_token(foreign.into());
        assert!(eval(scope, "(()=>{try{Object.getOwnPropertyDescriptor(globalThis,'isSecureContext').get.call(other);return false}catch(error){return error.name==='SecurityError'}})()").is_true());
    }
}
