//! Native Window Navigator identity and secure prototype member dispatch.
//! The generated interface owns prototype/constructor exposure; private native
//! slots establish the receiver brand without consulting author properties.
use super::*;

fn key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, name).expect("Navigator private key");
    v8::Private::for_api(scope, Some(name))
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let scope = &mut v8::ContextScope::new(scope, context);
    let navigator = new_stateful_object(scope, state, "Navigator", 1)?;
    let brand = key(scope, "oryn.native.navigator.brand");
    let owner = v8::BigInt::new_from_u64(scope, state.instance_id);
    if navigator.set_private(scope, brand, owner.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Navigator brand"));
    }
    Ok(navigator)
}

fn branded(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> bool {
    if object.is_proxy() {
        return false;
    }
    let brand = key(scope, "oryn.native.navigator.brand");
    object
        .get_private(scope, brand)
        .and_then(|value| value.try_cast::<v8::BigInt>().ok())
        .is_some_and(|value| {
            let (owner, lossless) = value.u64_value();
            lossless && owner != 0
        })
}

pub(super) fn install_service_worker(
    scope: &mut v8::PinScope,
    navigator: v8::Local<v8::Object>,
    container: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    if !branded(scope, navigator) {
        return Err(V8BindingError::Allocation("native Navigator receiver"));
    }
    let slot = key(scope, "oryn.native.navigator.serviceWorker");
    if navigator.set_private(scope, slot, container.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Navigator service container"));
    }
    let prototype = navigator
        .get_prototype(scope)
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .ok_or(V8BindingError::Allocation("Navigator prototype"))?;
    let getter = v8::Function::builder(service_worker)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Navigator.serviceWorker getter"))?;
    getter.set_name(v8::String::new(scope, "get serviceWorker").unwrap());
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let member = v8::String::new(scope, "serviceWorker").unwrap();
    if prototype.define_property(scope, member.into(), &descriptor) != Some(true) {
        return Err(V8BindingError::Allocation(
            "Navigator.serviceWorker property",
        ));
    }
    Ok(())
}

fn service_worker(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut result: v8::ReturnValue,
) {
    if !branded(scope, args.this()) {
        throw_type_error(
            scope,
            "Navigator.serviceWorker requires a Navigator receiver",
        );
        return;
    }
    let slot = key(scope, "oryn.native.navigator.serviceWorker");
    let Some(container) = args
        .this()
        .get_private(scope, slot)
        .filter(|value| value.is_object())
    else {
        throw_type_error(scope, "Navigator has no secure service container");
        return;
    };
    result.set(container);
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
        secure: bool,
        worker: bool,
    ) {
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://navigator.test"),
            document_generation: 1,
            frame_id: None,
        }));
        context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let document = dom.borrow().document();
        install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(0),
            1,
            document,
            if worker {
                BindingGlobal::Worker
            } else {
                BindingGlobal::Window
            },
        )
        .unwrap();
        if worker {
            service_workers::install_worker_container(scope, context).unwrap();
        }
    }

    #[test]
    fn window_navigator_has_generated_identity_and_unforgeable_native_brand() {
        super::super::tests::initialize_v8();
        for secure in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            install(scope, context, secure, false);
            assert!(eval(scope, r#"(()=>{
                const original=Navigator, object=navigator;
                if(typeof original!=='function'||original.name!=='Navigator'||original.length!==0)return false;
                if(object!==navigator||!(object instanceof original)||Object.getPrototypeOf(object)!==original.prototype)return false;
                if(Object.prototype.toString.call(object)!=='[object Navigator]'||original.prototype.constructor!==original)return false;
                for(const invoke of [()=>original(),()=>new original(),()=>Reflect.construct(original,[])]){
                    try{invoke();return false}catch(error){if(!(error instanceof TypeError))return false}
                }
                const get=Object.getOwnPropertyDescriptor(object,'userActivation').get;
                if(get.call(object)!==object.userActivation)return false;
                let traps=0;
                for(const fake of [{},Object.create(original.prototype),new Proxy(object,{get(){traps++;throw Error('trap')}})]){
                    try{get.call(fake);return false}catch(error){if(!(error instanceof TypeError))return false}
                }
                if(traps!==0)return false;
                globalThis.Navigator=function replacement(){};
                return navigator===object&&Object.getPrototypeOf(object)===original.prototype&&get.call(object)===object.userActivation;
            })()"#).is_true());
            if secure {
                assert!(eval(scope, r#"(()=>{
                    const object=navigator, prototype=Object.getPrototypeOf(object);
                    const descriptor=Object.getOwnPropertyDescriptor(prototype,'serviceWorker');
                    if(!descriptor||!descriptor.enumerable||!descriptor.configurable||descriptor.set!==undefined)return false;
                    const get=descriptor.get;
                    if(get.call(object)!==object.serviceWorker)return false;
                    let traps=0;
                    for(const fake of [{},Object.create(prototype),new Proxy(object,{get(){traps++;throw Error('trap')}})]){
                        try{get.call(fake);return false}catch(error){if(!(error instanceof TypeError))return false}
                    }
                    return traps===0;
                })()"#).is_true());
            } else {
                assert!(eval(scope, "!('serviceWorker' in navigator)&&!Object.getOwnPropertyDescriptor(Object.getPrototypeOf(navigator),'serviceWorker')").is_true());
            }
        }
    }

    #[test]
    fn window_navigator_interface_does_not_leak_into_worker_exposure() {
        super::super::tests::initialize_v8();
        for secure in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            install(scope, context, secure, true);
            assert!(eval(scope, "typeof Navigator==='undefined'&&navigator instanceof WorkerNavigator&&Object.getPrototypeOf(navigator)===WorkerNavigator.prototype").is_true());
            assert_eq!(
                eval(scope, "typeof navigator.serviceWorker").to_rust_string_lossy(scope),
                if secure { "object" } else { "undefined" }
            );
        }
    }

    #[test]
    fn secure_getter_uses_the_actual_foreign_navigator_brand_and_container() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let first = v8::Context::new(handles, Default::default());
        let getter = {
            let scope = &mut v8::ContextScope::new(handles, first);
            install(scope, first, true, false);
            eval(
                scope,
                "Object.getOwnPropertyDescriptor(Navigator.prototype,'serviceWorker').get",
            )
        };
        for secure in [false, true] {
            let second = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, second);
            install(scope, second, secure, false);
            let key = v8::String::new(scope, "foreignGetter").unwrap();
            second
                .global(scope)
                .create_data_property(scope, key.into(), getter);
            if secure {
                assert!(
                    eval(
                        scope,
                        "foreignGetter.call(navigator)===navigator.serviceWorker"
                    )
                    .is_true()
                );
            } else {
                assert!(eval(scope, "(()=>{try{foreignGetter.call(navigator);return false}catch(error){return error.name==='TypeError'}})()").is_true());
                assert!(eval(scope, "!('serviceWorker' in navigator)").is_true());
            }
        }
    }
}
