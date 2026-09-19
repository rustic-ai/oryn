//! Window's admitted agent-cluster keying. Cache the native answer on the realm
//! so a removed frame retains its answer without consulting a successor Window.
use super::*;
pub(super) fn value(state: &V8BindingState) -> bool {
    if let Some(value) = state.origin_agent_cluster.get() {
        return value;
    }
    // Bare standalone bindings have no response policy/parent cluster registry.
    // They use site keying for tuple origins, origin keying for opaque origins.
    let origin = state
        .document_provenance(state.document_root())
        .ok()
        .flatten()
        .map(|source| source.origin.clone())
        .unwrap_or_else(|| state.current_location_url());
    crate::origin::Origin::from_url(&origin).is_opaque()
}
fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let context = scope.get_current_context();
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return;
    };
    if !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "originAgentCluster receiver is not a Window");
        return;
    }
    rv.set_bool(value(&state));
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let getter = v8::Function::builder(getter)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("originAgentCluster getter"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "originAgentCluster")
        .ok_or(V8BindingError::Allocation("originAgentCluster name"))?;
    if context
        .global(scope)
        .define_property(scope, key.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("originAgentCluster exposure"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_choice_overrides_tuple_fallback_and_checks_receiver() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test").unwrap();
        assert!(!value(&state));
        state.set_origin_agent_cluster(true);
        assert!(value(&state));
        let script=v8::String::new(scope,"const getter=Object.getOwnPropertyDescriptor(globalThis,'originAgentCluster').get; getter.call(globalThis)").unwrap();
        assert!(
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let script = v8::String::new(
            scope,
            "(()=>{try{getter.call({});return false}catch(e){return e.name==='TypeError'}})()",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        state.set_origin_agent_cluster(false);
        assert!(!value(&state));
        state.retire_execution();
        assert!(!value(&state));
    }
}
