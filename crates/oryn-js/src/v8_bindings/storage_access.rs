//! Storage Access API decisions belong to the receiver's live native Document.
//! Promise settlement does not run navigation or substitute a replacement realm.
use super::*;
use crate::platform_control::{Request, Response};
fn fully_active(state: &Rc<V8BindingState>) -> bool {
    // Unloading closes task admission but does not alone replace the active
    // Document. Lifecycle authority and connected ancestor identity decide here.
    let mut current = state.clone();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.instance_id)
            || current.execution_retired()
            || current.execution_suspended()
        {
            return false;
        }
        let parent = current.frame_viewport_parent.borrow().clone();
        let Some((parent, frame, generation)) = parent else {
            return true;
        };
        let Some(parent) = parent.upgrade() else {
            return false;
        };
        {
            let dom = parent.dom.borrow();
            if dom.frame_context_generation(frame) != generation
                || !dom.is_connected(frame).unwrap_or(false)
            {
                return false;
            }
        }
        current = parent;
    }
}
fn decision(state: &Rc<V8BindingState>, request: bool) -> Result<bool, &'static str> {
    if !fully_active(state) {
        return Err("InvalidStateError");
    }
    let origin = current_platform_origin(state);
    if origin == "null" || !state.secure_context.get() {
        return if request {
            Err("NotAllowedError")
        } else {
            Ok(false)
        };
    }
    if native_platform::has_backend(state) {
        match native_platform::request(state, Request::WindowState) {
            Ok(Response::WindowState { .. }) => {}
            _ => return Err("InvalidStateError"),
        }
    }
    if request {
        state
            .refresh_native_container_policy()
            .map_err(|_| "InvalidStateError")?;
        if !state.storage_access_allowed()
            || !state.native_permissions_policy_allows("storage-access")
        {
            return Err("NotAllowedError");
        }
    }
    let allowed = if native_platform::has_backend(state) {
        match native_platform::request(state, Request::StorageAccess { request }) {
            Ok(Response::StorageAccess { allowed }) => allowed,
            _ => return Err("InvalidStateError"),
        }
    } else {
        true
    };
    if request && !allowed {
        Err("NotAllowedError")
    } else {
        Ok(allowed)
    }
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let (owner, node) = node_identity::argument(scope, args.this().into())?.ok_or(
            V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            },
        )?;
        if !matches!(
            owner.dom.borrow().arena().get(node)?.kind,
            NodeKind::Document
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            });
        }
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or(V8BindingError::Allocation("storage access promise"))?;
        let promise = resolver.get_promise(scope);
        let request = args.data().is_true();
        let active = active_document_state_for_node(scope, &owner, node);
        let outcome = active
            .filter(|state| state.document_root() == node)
            .map_or(Err("InvalidStateError"), |state| decision(&state, request));
        match outcome {
            Ok(allowed) => {
                let value: v8::Local<v8::Value> = if request {
                    v8::undefined(scope).into()
                } else {
                    v8::Boolean::new(scope, allowed).into()
                };
                if resolver.resolve(scope, value) != Some(true) {
                    return Err(V8BindingError::Allocation("storage access resolve"));
                }
            }
            Err(name) => {
                let error = dom_exception_value(scope, name)?;
                if resolver.reject(scope, error) != Some(true) {
                    return Err(V8BindingError::Allocation("storage access reject"));
                }
            }
        }
        Ok(promise)
    })();
    match result {
        Ok(promise) => rv.set(promise.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "Document")
        .ok_or(V8BindingError::Allocation("Document constructor"))?;
    let constructor = context
        .global(scope)
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("Document"))?;
    let key = v8::String::new(scope, "prototype")
        .ok_or(V8BindingError::Allocation("Document prototype"))?;
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("Document"))?;
    for (name, request) in [("hasStorageAccess", false), ("requestStorageAccess", true)] {
        let data = v8::Boolean::new(scope, request);
        let function = v8::Function::builder(call)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("storage access method"))?;
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("storage access name"))?;
        function.set_name(key);
        let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(function.into(), true);
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(V8BindingError::Allocation("storage access descriptor"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn inherited_blank_storage_access_is_native_promise_and_does_not_force_navigation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        // Match the native host: promise jobs run at its explicit checkpoint.
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(118), 1).unwrap();
        state.secure_context.set(true);
        *state.security_origin.borrow_mut() = Some("https://site.example".into());
        *state.location_url.borrow_mut() = Url::parse("about:blank").unwrap();
        eval(
            scope,
            "globalThis.result=[];globalThis.original=document;globalThis.p=document.requestStorageAccess();result.push(p instanceof Promise);p.then(value=>result.push(value===undefined));document.hasStorageAccess().then(value=>result.push(value));",
        );
        assert!(
            eval(
                scope,
                "result.length===1&&result[0]&&document===original&&document.URL==='about:blank'"
            )
            .is_true()
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"result.length===3&&result.every(Boolean)&&document===original&&document.URL==='about:blank'").is_true());
        eval(
            scope,
            "globalThis.rejected='';document.implementation.createHTMLDocument('inert').requestStorageAccess().catch(error=>rejected=error.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "rejected==='InvalidStateError'").is_true());
        state.set_storage_access_allowed(false);
        eval(
            scope,
            "globalThis.denied='';document.body.setAttribute('sandbox','allow-storage-access-by-user-activation');document.requestStorageAccess().catch(error=>denied=error.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "denied==='NotAllowedError'").is_true());
    }
    #[test]
    fn storage_access_checks_secure_origin_activity_and_native_policy() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(119), 1).unwrap();
        state.secure_context.set(true);
        *state.security_origin.borrow_mut() = Some("null".into());
        assert_eq!(decision(&state, false), Ok(false));
        assert_eq!(decision(&state, true), Err("NotAllowedError"));
        *state.security_origin.borrow_mut() = Some("https://site.example".into());
        state.secure_context.set(false);
        assert_eq!(decision(&state, false), Ok(false));
        assert_eq!(decision(&state, true), Err("NotAllowedError"));
        state.secure_context.set(true);
        state.set_native_permissions_policy(Rc::new(|feature| feature != "storage-access"));
        assert_eq!(decision(&state, true), Err("NotAllowedError"));
        state.set_native_permissions_policy(Rc::new(|_| true));
        assert_eq!(decision(&state, false), Ok(true));
        assert_eq!(decision(&state, true), Ok(true));
        eval(
            scope,
            "globalThis.forged=false;try{Document.prototype.requestStorageAccess.call({})}catch(error){forged=error instanceof TypeError}",
        );
        assert!(eval(scope, "forged").is_true());
    }
}
