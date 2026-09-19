//! Window scroll APIs share the document's native scrolling state.
use super::*;

struct Installed;

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    if context.get_slot::<Installed>().is_some() {
        return Ok(());
    }
    for name in ["scrollX", "scrollY", "pageXOffset", "pageYOffset"] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Window scroll property"))?;
        let getter = v8::Function::builder(get)
            .data(key.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("Window scroll getter"))?;
        getter.set_name(
            v8::String::new(scope, &format!("get {name}"))
                .ok_or(V8BindingError::Allocation("Window scroll getter name"))?,
        );
        let setter = v8::Function::builder(replace)
            .data(key.into())
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("Window scroll setter"))?;
        setter.set_name(
            v8::String::new(scope, &format!("set {name}"))
                .ok_or(V8BindingError::Allocation("Window scroll setter name"))?,
        );
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if context
            .global(scope)
            .define_property(scope, key.into(), &descriptor)
            != Some(true)
        {
            return Err(V8BindingError::Allocation("Window scroll accessor"));
        }
    }
    for name in ["scroll", "scrollTo", "scrollBy"] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Window scroll operation name"))?;
        let function = v8::Function::builder(scroll)
            .data(key.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("Window scroll operation"))?;
        function.set_name(key);
        if context.global(scope).define_own_property(
            scope,
            key.into(),
            function.into(),
            v8::PropertyAttribute::NONE,
        ) != Some(true)
        {
            return Err(V8BindingError::Allocation("Window scroll method"));
        }
    }
    context.set_slot(Rc::new(Installed));
    Ok(())
}

fn receiver(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> Option<Rc<V8BindingState>> {
    let state = binding_state_for_receiver(scope, object);
    let Some(state) = state.filter(|state| {
        state.binding_global == BindingGlobal::Window
            && global_events::is_global(scope, state, object)
    }) else {
        throw_type_error(scope, "Window scrolling requires a Window receiver");
        return None;
    };
    global_events::check_window_access(scope, &state, object).then_some(state)
}

fn root(state: &V8BindingState) -> Option<NodeId> {
    if state.execution_retired() {
        return None;
    }
    state
        .dom
        .borrow()
        .first_element_child(state.document_root())
        .ok()
        .flatten()
}

fn get(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = receiver(scope, args.this()) else {
        return;
    };
    let offset = root(&state)
        .and_then(|root| state.scroll_offsets.borrow().get(&root).copied())
        .unwrap_or_default();
    let horizontal = matches!(
        args.data().to_rust_string_lossy(scope).as_str(),
        "scrollX" | "pageXOffset"
    );
    rv.set(v8::Number::new(scope, if horizontal { offset.0 } else { offset.1 }).into());
}

fn replace(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(_state) = receiver(scope, args.this()) else {
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.data()) else {
        return;
    };
    if args
        .this()
        .define_own_property(scope, key, args.get(0), v8::PropertyAttribute::NONE)
        != Some(true)
    {
        throw_type_error(scope, "Cannot replace Window scroll property");
    }
}

fn scroll(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(state) = receiver(scope, args.this()) else {
        return;
    };
    let result = (|| {
        let requested = scroll_coordinates(scope, &args)?;
        // Resolve the active document after conversion: getters can navigate or
        // replace its document element.
        if let Some(root) = root(&state) {
            let relative = args.data().to_rust_string_lossy(scope) == "scrollBy";
            perform_scroll(&state, root, requested, relative)?;
        }
        Ok::<_, V8BindingError>(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
