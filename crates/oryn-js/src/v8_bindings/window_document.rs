//! Native Window document and self-reference properties. Their identity is
//! owned by the realm, not by author-writable bootstrap data properties.
use super::*;

struct Installed;

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    if context.get_slot::<Installed>().is_some() {
        return Ok(());
    }
    let getter = v8::Function::builder(get)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.document getter"))?;
    let name = v8::String::new(scope, "get document")
        .ok_or(V8BindingError::Allocation("Window.document getter name"))?;
    getter.set_name(name);
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(false);
    let name = v8::String::new(scope, "document")
        .ok_or(V8BindingError::Allocation("Window.document name"))?;
    if context
        .global(scope)
        .define_property(scope, name.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("Window.document property"));
    }
    for (name, replaceable) in [
        ("window", false),
        ("self", true),
        ("frames", true),
        ("top", false),
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Window identity name"))?;
        let top = v8::Boolean::new(scope, name == "top");
        let getter = v8::Function::builder(get_window)
            .data(top.into())
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("Window identity getter"))?;
        let getter_name = v8::String::new(scope, &format!("get {name}"))
            .ok_or(V8BindingError::Allocation("Window identity getter name"))?;
        getter.set_name(getter_name);
        let setter: v8::Local<v8::Value> = if replaceable {
            let setter = v8::Function::builder(replace_window_alias)
                .data(key.into())
                .length(1)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("Window identity setter"))?;
            let setter_name = v8::String::new(scope, &format!("set {name}"))
                .ok_or(V8BindingError::Allocation("Window identity setter name"))?;
            setter.set_name(setter_name);
            setter.into()
        } else {
            v8::undefined(scope).into()
        };
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter);
        descriptor.set_enumerable(true);
        descriptor.set_configurable(replaceable);
        if context
            .global(scope)
            .define_property(scope, key.into(), &descriptor)
            != Some(true)
        {
            return Err(V8BindingError::Allocation("Window identity property"));
        }
    }
    let getter = v8::Function::builder(get_frame_element)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.frameElement getter"))?;
    getter.set_name(v8::String::new(scope, "get frameElement").unwrap());
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, "frameElement").unwrap();
    if context
        .global(scope)
        .define_property(scope, name.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("Window.frameElement property"));
    }
    context.set_slot(Rc::new(Installed));
    Ok(())
}

fn get_window(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Window identity requires a Window receiver");
        return;
    };
    if !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "Window identity requires a Window receiver");
        return;
    }
    if global_events::check_window_access(scope, &state, args.this()) {
        if args.data().is_true() {
            let key = top_key(scope);
            if let Some(top) = args.this().get_private(scope, key)
                && top.is_object()
            {
                rv.set(top);
                return;
            }
        }
        rv.set(args.this().into());
    }
}

fn top_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.window.native.top").expect("Window top key");
    v8::Private::for_api(scope, Some(name))
}

fn frame_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.window.native.frameElement").unwrap();
    v8::Private::for_api(scope, Some(name))
}

pub(super) fn set_frame_element(
    scope: &mut v8::PinScope,
    window: v8::Local<v8::Object>,
    frame: Option<v8::Local<v8::Object>>,
) -> Result<(), V8BindingError> {
    let key = frame_key(scope);
    let frame = frame.map_or_else(|| v8::null(scope).into(), Into::into);
    if window.set_private(scope, key, frame) != Some(true) {
        return Err(V8BindingError::Allocation("Window frame identity"));
    }
    Ok(())
}

fn get_frame_element(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Window.frameElement requires a Window receiver");
        return;
    };
    if !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "Window.frameElement requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, &state, args.this()) {
        return;
    }
    if !state.execution_retired() {
        let key = frame_key(scope);
        if let Some(frame) = args.this().get_private(scope, key)
            && frame.is_object()
        {
            rv.set(frame);
            return;
        }
    }
    rv.set(v8::null(scope).into());
}

/// The private edge is traced by V8 and cannot be replaced by author script.
pub(super) fn set_top(
    scope: &mut v8::PinScope,
    window: v8::Local<v8::Object>,
    top: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let key = top_key(scope);
    if window.set_private(scope, key, top.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Window top identity"));
    }
    Ok(())
}

fn replace_window_alias(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Window alias requires a Window receiver");
        return;
    };
    if !global_events::check_window_access(scope, &state, args.this()) {
        return;
    }
    let Ok(name) = v8::Local::<v8::Name>::try_from(args.data()) else {
        throw_type_error(scope, "Invalid Window alias");
        return;
    };
    if args
        .this()
        .define_own_property(scope, name, args.get(0), v8::PropertyAttribute::NONE)
        != Some(true)
    {
        throw_type_error(scope, "Cannot replace Window alias");
    }
}

fn get(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Window.document requires a Window receiver");
        return;
    };
    if !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "Window.document requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, &state, args.this()) {
        return;
    }
    match wrap_node(scope, &state, state.document) {
        Ok(document) => rv.set(document.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
