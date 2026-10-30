//! Generated BarProp objects backed by receiver-owned native window policy.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> v8::Global<v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        let value = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        v8::Global::new(scope, value)
    }

    #[test]
    fn native_bars_follow_receiver_realm_security_and_retirement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let first = v8::Context::new(handles, Default::default());
        let second = v8::Context::new(handles, Default::default());
        let token = v8::String::new(handles, "bars-shared-origin").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        for (context, popup, realm) in [(first, false, 92), (second, true, 93)] {
            context.set_slot(Rc::new(crate::realm::WindowPopupPolicy(popup)));
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            install_generated_bindings(scope, context, dom, RealmId(realm), 1).unwrap();
        }
        let scope = &mut v8::ContextScope::new(handles, first);
        let other = second.global(scope);
        let key = v8::String::new(scope, "other").unwrap();
        first.global(scope).create_data_property(scope, key.into(), other.into());
        let result = eval(scope, r#"(()=>{
            globalThis.barGetter=Object.getOwnPropertyDescriptor(window,'toolbar').get;
            globalThis.barSetter=Object.getOwnPropertyDescriptor(window,'toolbar').set;
            globalThis.visibleGetter=Object.getOwnPropertyDescriptor(BarProp.prototype,'visible').get;
            globalThis.foreignBar=barGetter.call(other);
            if(foreignBar!==other.toolbar||Object.getPrototypeOf(foreignBar)!==other.BarProp.prototype)return false;
            if(visibleGetter.call(foreignBar)!==false||visibleGetter.call(toolbar)!==true)return false;
            let traps=0;
            for(const fake of [{},Window.prototype,Object.create(window),new Proxy(window,{get(){traps++;throw Error('trap')}})]){
                for(const run of [()=>barGetter.call(fake),()=>barSetter.call(fake,'bad')]){
                    try{run();return false}catch(e){if(!(e instanceof TypeError))return false}
                }
            }
            return traps===0;
        })()"#);
        assert!(v8::Local::new(scope, &result).is_true());
        let different = v8::String::new(scope, "bars-other-origin").unwrap();
        second.set_security_token(different.into());
        let result = eval(scope, r#"(()=>{
            for(const run of [()=>barGetter.call(other),()=>barSetter.call(other,'bad'),()=>visibleGetter.call(foreignBar)]){
                try{run();return false}catch(e){if(e.name!=='SecurityError')return false}
            }
            return true;
        })()"#);
        assert!(v8::Local::new(scope, &result).is_true());
        second.set_security_token(token.into());
        first.get_slot::<V8BindingState>().unwrap().retire_execution();
        // A borrowed callback must select the receiving bar's live owner.
        let result = eval(scope, "barGetter.call(other)===foreignBar&&visibleGetter.call(foreignBar)===false");
        assert!(v8::Local::new(scope, &result).is_true());
        second.get_slot::<V8BindingState>().unwrap().retire_execution();
        let result = eval(scope, r#"(()=>{
            for(const run of [()=>barGetter.call(other),()=>visibleGetter.call(foreignBar)]){
                try{run();return false}catch(e){}
            }
            return true;
        })()"#);
        assert!(v8::Local::new(scope, &result).is_true());
    }

    #[test]
    fn native_bars_preserve_brand_aliases_and_popup_visibility() {
        super::super::tests::initialize_v8();
        for popup in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            context.set_slot(Rc::new(crate::realm::WindowPopupPolicy(popup)));
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            install_generated_bindings(scope, context, dom, RealmId(91), 1).unwrap();
            let source = v8::String::new(scope, r#"(()=>{
                const names=['locationbar','menubar','personalbar','scrollbars','statusbar','toolbar'];
                const objects=names.map(name=>window[name]);
                const visible=Object.getOwnPropertyDescriptor(BarProp.prototype,'visible');
                if(!visible||visible.set!==undefined||visible.get.name!=='get visible'||visible.get.length!==0)return false;
                if(!visible.enumerable||!visible.configurable)return false;
                try{new BarProp();return false}catch(e){if(!(e instanceof TypeError))return false}
                for(let i=0;i<names.length;i++){
                    const name=names[i], object=objects[i], d=Object.getOwnPropertyDescriptor(window,name);
                    if(object!==d.get.call(window)||!(object instanceof BarProp)||Object.prototype.toString.call(object)!=='[object BarProp]')return false;
                    if(d.get.name!=='get '+name||d.set.name!=='set '+name||d.set.length!==1)return false;
                    for(const fake of [{},BarProp.prototype,Object.create(object),new Proxy(object,{get(){throw Error('trap')}})]){
                        try{visible.get.call(fake);return false}catch(e){if(!(e instanceof TypeError))return false}
                    }
                    d.set.call(window,'author');
                    if(window[name]!=='author'||d.get.call(window)!==object)return false;
                    if(objects.slice(0,i).includes(object))return false;
                }
                return objects.every(object=>object.visible===EXPECTED);
            })()"#.replace("EXPECTED", if popup { "false" } else { "true" }).as_str()).unwrap();
            let result = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
            assert!(result.is_true(), "popup={popup}");
        }
    }
}

pub(super) fn is_member(member: &str) -> bool {
    matches!(member, "locationbar" | "menubar" | "personalbar" | "scrollbars" | "statusbar" | "toolbar")
}

fn key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, name)?;
    Some(v8::Private::for_api(scope, Some(name)))
}

pub(super) fn get(scope: &mut v8::PinScope, state: &V8BindingState,
    args: v8::FunctionCallbackArguments, member: &str, mut rv: v8::ReturnValue) {
    if state.execution_retired() || state.binding_global != BindingGlobal::Window
        || !global_events::is_global(scope, state, args.this()) {
        throw_type_error(scope, "Window bar requires a live Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, state, args.this()) { return; }
    let result = (|| -> Result<_, V8BindingError> {
        let slot = key(scope, &format!("oryn.native.window.bar.{member}"))
            .ok_or(V8BindingError::Allocation("Window bar slot"))?;
        if let Some(value) = args.this().get_private(scope, slot).filter(|value| value.is_object()) {
            return Ok(value);
        }
        let context = args.this().get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("Window bar realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let object = new_stateful_object(scope, state, "BarProp", 1)?;
        let owner = key(scope, "oryn.native.bar.owner")
            .ok_or(V8BindingError::Allocation("BarProp owner slot"))?;
        if object.set_private(scope, owner, args.this().into()) != Some(true)
            || args.this().set_private(scope, slot, object.into()) != Some(true) {
            return Err(V8BindingError::Allocation("BarProp native identity"));
        }
        Ok(object.into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(super) fn visible(scope: &mut v8::PinScope, state: &V8BindingState,
    args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let owner = key(scope, "oryn.native.bar.owner")
        .and_then(|key| args.this().get_private(scope, key))
        .and_then(|value| value.try_cast::<v8::Object>().ok());
    let Some(owner) = owner.filter(|owner| !args.this().is_proxy()
        && state.binding_global == BindingGlobal::Window
        && global_events::is_global(scope, state, *owner)) else {
        throw_type_error(scope, "BarProp.visible requires a BarProp receiver");
        return;
    };
    if !global_events::check_window_access(scope, state, owner) { return; }
    match native_platform::window_is_popup(scope, state) {
        Ok(popup) => rv.set_bool(!popup),
        Err(error) => throw_binding_error(scope, &error),
    }
}
