//! Realm intrinsics are edges in V8's traced graph, not native strong roots.
use super::*;

// User slot zero belongs to WindowProxy's descriptor graph. Keep caches on the
// exact Context: a navigable WindowProxy can be detached and rebound to another.
const INTRINSIC_DATA_SLOT: i32 = 1;
struct IntrinsicDataInitialized;
#[cfg(test)]
thread_local! {
    static FAIL_RETAIN_PUBLICATION: Cell<bool> = const { Cell::new(false) };
}

fn retain_in_context(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let scope = &mut v8::ContextScope::new(scope, context);
    let initialized = context.get_slot::<IntrinsicDataInitialized>().is_some();
    let holder = if initialized {
        context
            .get_embedder_data(scope, INTRINSIC_DATA_SLOT)
            .and_then(|value| value.try_cast::<v8::Object>().ok())
            .ok_or(V8BindingError::Allocation("context intrinsic holder"))?
    } else {
        let holder = v8::Object::new(scope);
        let null = v8::null(scope);
        if holder.set_prototype(scope, null.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        holder
    };
    let name = v8::String::new(scope, &format!("oryn.context.intrinsic.{name}"))
        .ok_or(V8BindingError::Allocation("context intrinsic key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    #[cfg(test)]
    if FAIL_RETAIN_PUBLICATION.with(|fail| fail.replace(false)) {
        return Err(V8BindingError::Allocation("context intrinsic publication"));
    }
    if holder.set_private(scope, key, value) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    if !initialized {
        context.set_embedder_data(INTRINSIC_DATA_SLOT, holder.into());
        context.set_slot(Rc::new(IntrinsicDataInitialized));
    }
    Ok(())
}
pub(super) fn retain_function_in_context(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    name: &str,
    value: v8::Local<v8::Function>,
) -> Result<v8::Weak<v8::Function>, V8BindingError> {
    retain_in_context(scope, context, name, value.into())?;
    Ok(v8::Weak::new(scope, value))
}
pub(super) fn retain_object_in_context(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    name: &str,
    value: v8::Local<v8::Object>,
) -> Result<v8::Weak<v8::Object>, V8BindingError> {
    retain_in_context(scope, context, name, value.into())?;
    Ok(v8::Weak::new(scope, value))
}
pub(super) fn retain_function(
    scope: &mut v8::PinScope,
    name: &str,
    value: v8::Local<v8::Function>,
) -> Result<v8::Weak<v8::Function>, V8BindingError> {
    let context = scope.get_current_context();
    retain_function_in_context(scope, context, name, value)
}
pub(super) fn retain_object(
    scope: &mut v8::PinScope,
    name: &str,
    value: v8::Local<v8::Object>,
) -> Result<v8::Weak<v8::Object>, V8BindingError> {
    let context = scope.get_current_context();
    retain_object_in_context(scope, context, name, value)
}

pub(super) fn retain_object_for(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    name: &str,
    value: v8::Local<v8::Object>,
) -> Result<v8::Weak<v8::Object>, V8BindingError> {
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("intrinsic owner Context"))?;
    retain_object_in_context(
        scope,
        context,
        &format!("owner-{}-{name}", state.instance_id),
        value,
    )
}

/// Roll back an unpublished native cache edge. Names are private owner keys.
pub(super) fn release_object_for(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    name: &str,
) -> Result<(), V8BindingError> {
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("intrinsic owner Context"))?;
    if context.get_slot::<IntrinsicDataInitialized>().is_none() {
        return Ok(());
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let holder = context
        .get_embedder_data(scope, INTRINSIC_DATA_SLOT)
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .ok_or(V8BindingError::Allocation("context intrinsic holder"))?;
    let name = v8::String::new(
        scope,
        &format!("oryn.context.intrinsic.owner-{}-{name}", state.instance_id),
    )
    .ok_or(V8BindingError::Allocation("context intrinsic key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if holder.delete_private(scope, key) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traced_intrinsic_survives_public_replacement_but_does_not_root_its_context() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, function, anchor) = {
            v8::scope!(let handles,&mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let source = v8::String::new(scope, "(()=>42)").unwrap();
            let function = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .try_cast::<v8::Function>()
                .unwrap();
            let function = retain_function(scope, "fixture", function).unwrap();
            let anchor = v8::Object::new(scope);
            (
                v8::Weak::new(scope, context),
                function,
                v8::Global::new(scope, anchor),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let function = function.to_local(scope).unwrap();
            let undefined = v8::undefined(scope);
            assert_eq!(
                function
                    .call(scope, undefined.into(), &[])
                    .unwrap()
                    .int32_value(scope),
                Some(42)
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let scope,&mut isolate);
        assert!(context.to_local(scope).is_none());
        assert!(function.to_local(scope).is_none());
    }
    #[test]
    fn discarded_frame_cache_releases_native_edges_but_preserves_retained_proxy() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><iframe id=f></iframe>")
                .into_runtime()
                .unwrap(),
        ));
        let frame = dom
            .borrow()
            .query_selector(dom.borrow().document(), "iframe")
            .unwrap()
            .unwrap();
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let proxy = v8::Object::new(scope);
        let retained = v8::Global::new(scope, proxy);
        state.set_frame_window(frame, v8::Global::new(scope, proxy));
        state.set_frame_committed_window(frame, v8::Global::new(scope, proxy));
        assert!(state.frame_window(frame).is_some());
        state.retire_execution();
        assert_eq!(refresh_retired_frames(scope, &state).unwrap(), 2);
        assert!(state.frame_window(frame).is_none());
        assert!(state.frame_committed_windows.borrow().is_empty());
        assert!(v8::Local::new(scope, &retained).strict_equals(proxy.into()));
        assert_eq!(refresh_retired_frames(scope, &state).unwrap(), 0);
    }
    #[test]
    fn installed_intrinsic_caches_preserve_detached_document_then_allow_collection() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, document) = {
            v8::scope!(let handles,&mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            let source=v8::String::new(scope,"(()=>{const selected=document.createRange();selected.selectNodeContents(document.body);document.getSelection().addRange(selected);document.body.savedRange=selected;new URLSearchParams('a=1').entries();new Headers().entries();new FormData().entries();trustedTypes.createPolicy('default',{createHTML:s=>s});Reflect.set=()=>{throw Error('authored replacement')};document.adoptedStyleSheets.extra=42;if(document.adoptedStyleSheets.extra!==42)throw Error('lost intrinsic');return document})()").unwrap();
            let document = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let state = context.get_slot::<V8BindingState>().unwrap();
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, context),
                v8::Global::new(scope, document),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let document = v8::Local::new(scope, &document);
            let key = v8::String::new(scope, "nodeType").unwrap();
            assert_eq!(
                document.get(scope, key.into()).unwrap().int32_value(scope),
                Some(9)
            );
            let source=v8::String::new(scope,"document.body.savedRange.startContainer===document.body && document.getSelection().getRangeAt(0)===document.body.savedRange").unwrap();
            assert!(
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
        drop(document);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let scope,&mut isolate);
        assert!(
            context.to_local(scope).is_none(),
            "native intrinsic caches must not retain an otherwise unreachable retired Document"
        );
    }
}

// Retirement changes the retention edge, not the selected Range's identity.
// The owning Window graph retains selected wrappers while any authored object
// can still reach it; the native state must no longer keep that graph alive.
pub(super) fn refresh_retired_selection(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    selected: &BTreeSet<u64>,
) -> Result<usize, V8BindingError> {
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("retired selection Context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let values = {
        let objects = state.range_objects.borrow();
        objects
            .iter()
            .filter(|(id, _)| selected.contains(id))
            .filter_map(|(_, object)| object.handle.to_local(scope).map(Into::into))
            .collect::<Vec<v8::Local<v8::Value>>>()
    };
    let values = v8::Array::new_with_elements(scope, &values);
    retain_in_context(
        scope,
        context,
        &format!("retired-selection-{}", state.instance_id),
        values.into(),
    )?;
    Ok(state
        .range_objects
        .borrow_mut()
        .values_mut()
        .map(|object| usize::from(object.selection_root.take().is_some()))
        .sum())
}

pub(super) fn refresh_retired_frames(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<usize, V8BindingError> {
    if state.frame_windows.borrow().is_empty() && state.frame_committed_windows.borrow().is_empty()
    {
        return Ok(0);
    }
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("retired frames Context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let values = {
        let windows = state.frame_windows.borrow();
        let committed = state.frame_committed_windows.borrow();
        windows
            .values()
            .chain(committed.values())
            .map(|window| v8::Local::new(scope, window).into())
            .collect::<Vec<v8::Local<v8::Value>>>()
    };
    let values = v8::Array::new_with_elements(scope, &values);
    retain_in_context(
        scope,
        context,
        &format!("retired-frames-{}", state.instance_id),
        values.into(),
    )?;
    let released =
        state.frame_windows.borrow().len() + state.frame_committed_windows.borrow().len();
    state.frame_windows.borrow_mut().clear();
    state.frame_committed_windows.borrow_mut().clear();
    Ok(released)
}

#[cfg(test)]
mod exact_context_retention_tests {
    use super::*;
    fn call_number(scope: &mut v8::PinScope, value: &v8::Weak<v8::Function>) -> i32 {
        let function = value.to_local(scope).unwrap();
        let undefined = v8::undefined(scope);
        function
            .call(scope, undefined.into(), &[])
            .unwrap()
            .int32_value(scope)
            .unwrap()
    }
    #[test]
    fn rebound_window_proxy_keeps_original_context_cache_separate() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (old, next, first, second, first_anchor, second_anchor) = {
            v8::scope!(let handles, &mut isolate);
            let old = v8::Context::new(handles, Default::default());
            let (first, first_anchor, sentinel) = {
                let scope = &mut v8::ContextScope::new(handles, old);
                let value = super::super::test_eval::eval(scope, "(()=>17)")
                    .try_cast::<v8::Function>()
                    .unwrap();
                let first = retain_function_in_context(scope, old, "same-name", value).unwrap();
                let sentinel = v8::Object::new(scope);
                old.set_embedder_data(0, sentinel.into());
                (
                    first,
                    v8::Global::new(scope, value),
                    v8::Weak::new(scope, sentinel),
                )
            };
            let proxy = old.global(handles);
            crate::v8_context_bridge::detach_global(handles, old);
            let next = v8::Context::new(
                handles,
                v8::ContextOptions {
                    global_object: Some(proxy.into()),
                    ..Default::default()
                },
            );
            let scope = &mut v8::ContextScope::new(handles, next);
            let value = super::super::test_eval::eval(scope, "(()=>29)")
                .try_cast::<v8::Function>()
                .unwrap();
            let second = retain_function_in_context(scope, next, "same-name", value).unwrap();
            assert_eq!(call_number(scope, &first), 17);
            assert_eq!(call_number(scope, &second), 29);
            let old_holder = old.get_embedder_data(scope, INTRINSIC_DATA_SLOT).unwrap();
            let new_holder = next.get_embedder_data(scope, INTRINSIC_DATA_SLOT).unwrap();
            assert!(!old_holder.strict_equals(new_holder));
            assert!(
                old.get_embedder_data(scope, 0)
                    .unwrap()
                    .strict_equals(sentinel.to_local(scope).unwrap().into())
            );
            (
                v8::Weak::new(scope, old),
                v8::Weak::new(scope, next),
                first,
                second,
                first_anchor,
                v8::Global::new(scope, value),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            assert!(old.to_local(handles).is_some());
            let context = next.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            assert_eq!(call_number(scope, &first), 17);
            assert_eq!(call_number(scope, &second), 29);
        }
        drop(first_anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            assert!(old.to_local(handles).is_none());
            assert!(first.to_local(handles).is_none());
            assert!(next.to_local(handles).is_some());
            assert!(second.to_local(handles).is_some());
        }
        drop(second_anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(next.to_local(handles).is_none());
        assert!(second.to_local(handles).is_none());
    }
    #[test]
    fn failed_publication_keeps_original_cache_and_does_not_publish_empty_holder() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let original = super::super::test_eval::eval(scope, "(()=>41)")
            .try_cast::<v8::Function>()
            .unwrap();
        FAIL_RETAIN_PUBLICATION.with(|fail| fail.set(true));
        assert!(matches!(
            retain_function_in_context(scope, context, "atomic", original),
            Err(V8BindingError::Allocation("context intrinsic publication"))
        ));
        assert!(context.get_slot::<IntrinsicDataInitialized>().is_none());
        let weak = retain_function_in_context(scope, context, "atomic", original).unwrap();
        let replacement = super::super::test_eval::eval(scope, "(()=>53)")
            .try_cast::<v8::Function>()
            .unwrap();
        FAIL_RETAIN_PUBLICATION.with(|fail| fail.set(true));
        assert!(matches!(
            retain_function_in_context(scope, context, "atomic", replacement),
            Err(V8BindingError::Allocation("context intrinsic publication"))
        ));
        let holder = context
            .get_embedder_data(scope, INTRINSIC_DATA_SLOT)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let name = v8::String::new(scope, "oryn.context.intrinsic.atomic").unwrap();
        let key = v8::Private::for_api(scope, Some(name));
        assert!(
            holder
                .get_private(scope, key)
                .unwrap()
                .strict_equals(original.into())
        );
        assert_eq!(call_number(scope, &weak), 41);
    }
    #[test]
    fn state_owned_cache_uses_exact_context_from_a_foreign_caller() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let owner = v8::Context::new(handles, Default::default());
        let caller = v8::Context::new(handles, Default::default());
        let state = {
            let scope = &mut v8::ContextScope::new(handles, owner);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, owner, dom, RealmId(997), 1).unwrap()
        };
        let value = {
            let scope = &mut v8::ContextScope::new(handles, owner);
            v8::Object::new(scope)
        };
        let scope = &mut v8::ContextScope::new(handles, caller);
        let retained = retain_object_for(scope, &state, "foreign-caller", value).unwrap();
        assert!(
            retained
                .to_local(scope)
                .unwrap()
                .strict_equals(value.into())
        );
        assert!(caller.get_slot::<IntrinsicDataInitialized>().is_none());
        let holder = owner
            .get_embedder_data(scope, INTRINSIC_DATA_SLOT)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let name = v8::String::new(
            scope,
            &format!(
                "oryn.context.intrinsic.owner-{}-foreign-caller",
                state.instance_id
            ),
        )
        .unwrap();
        let key = v8::Private::for_api(scope, Some(name));
        assert!(
            holder
                .get_private(scope, key)
                .unwrap()
                .strict_equals(value.into())
        );
    }
}
