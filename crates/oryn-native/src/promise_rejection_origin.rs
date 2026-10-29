// Promise creation policy is a V8-traced private edge, not a native handle map.
// The compiled script's host-defined metadata remains attached to delayed
// callbacks; creation context and script provenance are separate authorities.
fn promise_origin_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, "oryn.promise.creation-policy")?;
    Some(v8::Private::for_api(scope, Some(name)))
}

extern "C" fn promise_creation_hook(
    phase: v8::PromiseHookType,
    promise: v8::Local<v8::Promise>,
    _parent: v8::Local<v8::Value>,
) {
    if phase != v8::PromiseHookType::Init { return }
    v8::callback_scope!(unsafe scope, promise);
    let Some(context) = promise.get_creation_context(scope) else { return };
    let Some(environment) = context.get_slot::<EnvironmentDescriptor>() else { return };
    let epoch = context.get_slot::<V8BindingState>()
        .map_or(0, |state| state.document_authority_epoch());
    let muted = oryn_js::v8_bindings::current_script_muted_errors(scope);
    let Some(key) = promise_origin_key(scope) else { return };
    let metadata = v8::Array::new(scope, 3);
    let null = v8::null(scope);
    if metadata.set_prototype(scope, null.into()) != Some(true) { return }
    let id = v8::BigInt::new_from_u64(scope, environment.id.0);
    let epoch = v8::BigInt::new_from_u64(scope, epoch);
    let muted = v8::Boolean::new(scope, muted);
    metadata.set_index(scope, 0, id.into());
    metadata.set_index(scope, 1, epoch.into());
    metadata.set_index(scope, 2, muted.into());
    promise.set_private(scope, key, metadata.into());
}

fn promise_creation_policy(
    scope: &mut v8::PinScope,
    promise: v8::Local<v8::Promise>,
) -> Option<(EnvironmentId, u64, bool)> {
    let key = promise_origin_key(scope)?;
    let metadata = promise.get_private(scope, key)?.try_cast::<v8::Array>().ok()?;
    let id = metadata.get_index(scope, 0)?.try_cast::<v8::BigInt>().ok()?;
    let (id, exact_id) = id.u64_value();
    let epoch = metadata.get_index(scope, 1)?.try_cast::<v8::BigInt>().ok()?;
    let (epoch, exact_epoch) = epoch.u64_value();
    let muted = metadata.get_index(scope, 2)?;
    (exact_id && exact_epoch && muted.is_boolean())
        .then(|| (EnvironmentId(id), epoch, muted.boolean_value(scope)))
}
