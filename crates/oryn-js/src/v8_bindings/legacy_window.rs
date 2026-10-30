//! Native ownership of the obsolete HTML External object.
use super::*;

pub(super) fn external<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    receiver: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if state.binding_global != BindingGlobal::Window || !global_events::is_global(scope, state, receiver) {
        return Err(V8BindingError::Message("TypeError: external requires a Window receiver".into()));
    }
    if !global_events::check_window_access(scope, state, receiver) {
        return Err(V8BindingError::JavaScriptException);
    }
    let name = v8::String::new(scope, "oryn.native.window.external")
        .ok_or(V8BindingError::Allocation("External cache name"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let cached = receiver.get_private(scope, key).ok_or(V8BindingError::JavaScriptException)?;
    if let Ok(object) = v8::Local::<v8::Object>::try_from(cached) {
        return Ok(object);
    }
    let id = register_platform_object(state, PlatformObjectRecord::External);
    let result = (|| {
        let object = new_platform_object(scope, state, "External", id)?;
        if receiver.set_private(scope, key, object.into()) != Some(true) {
            return Err(V8BindingError::Allocation("External cache publication"));
        }
        Ok(object)
    })();
    if result.is_err() {
        state.platform_wrappers.borrow_mut().remove(&id);
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_edges.borrow_mut().discard_unpublished(id);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_window_operations_preserve_brand_identity_and_replaceable_external() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"(() => {
            const throwsType = fn => { try { fn(); return false; } catch(e) { return e instanceof TypeError; } };
            let converted = 0;
            const value = {toString() { ++converted; return 'ignored'; }};
            for (const name of ['captureEvents', 'releaseEvents']) {
                const f = globalThis[name];
                if (f.name !== name || f.length !== 0 ||
                    !throwsType(() => f.call({})) ||
                    !throwsType(() => Reflect.construct(f, [])) ||
                    f.call(globalThis, value) !== undefined) return false;
            }
            const d = Object.getOwnPropertyDescriptor(globalThis, 'external');
            const saved = external;
            if (saved !== external || !(saved instanceof External) ||
                d.get.name !== 'get external' || d.get.length !== 0 ||
                d.set.name !== 'set external' || d.set.length !== 1 ||
                !d.enumerable || !d.configurable || !throwsType(() => d.get.call({})) ||
                !throwsType(() => d.set.call({}, value))) return false;
            for (const name of ['AddSearchProvider', 'IsSearchProviderInstalled']) {
                const f = External.prototype[name];
                if (f.name !== name || f.length !== 0 ||
                    !throwsType(() => f.call({})) ||
                    !throwsType(() => Reflect.construct(f, [])) ||
                    f.call(saved, value) !== undefined) return false;
            }
            external = value;
            const replacement = Object.getOwnPropertyDescriptor(globalThis, 'external');
            if (external !== value || !replacement.writable || !replacement.enumerable ||
                !replacement.configurable || d.get.call(globalThis) !== saved) return false;
            Object.defineProperty(globalThis, 'external', d);
            return external === saved && converted === 0;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }
}
