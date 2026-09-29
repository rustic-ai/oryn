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
        ("parent", true),
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Window identity name"))?;
        let relation = v8::Integer::new(
            scope,
            match name {
                "top" => 1,
                "parent" => 2,
                _ => 0,
            },
        );
        let getter = v8::Function::builder(get_window)
            .data(relation.into())
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
    let name =
        v8::String::new(scope, "stop").ok_or(V8BindingError::Allocation("Window.stop name"))?;
    let function = v8::Function::builder(stop)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.stop"))?;
    function.set_name(name);
    if context.global(scope).define_own_property(
        scope,
        name.into(),
        function.into(),
        v8::PropertyAttribute::NONE,
    ) != Some(true)
    {
        return Err(V8BindingError::Allocation("Window.stop property"));
    }
    context.set_slot(Rc::new(Installed));
    Ok(())
}

fn get_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
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
        let relation = args.data().int32_value(scope).unwrap_or(0);
        let window = args.this();
        match relation {
            1 => rv.set(native_top(scope, window).into()),
            2 => rv.set(native_parent(scope, window).into()),
            _ => rv.set(window.into()),
        }
    }
}

fn top_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.window.native.top").expect("Window top key");
    v8::Private::for_api(scope, Some(name))
}

fn parent_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.window.native.parent").expect("Window parent key");
    v8::Private::for_api(scope, Some(name))
}

fn opener_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.window.native.opener").expect("Window opener key");
    v8::Private::for_api(scope, Some(name))
}

pub(super) fn native_opener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let key = opener_key(scope);
    window
        .get_private(scope, key)
        .filter(|value| value.is_object())
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(super) fn set_opener(
    scope: &mut v8::PinScope,
    window: v8::Local<v8::Object>,
    opener: Option<v8::Local<v8::Object>>,
) -> Result<(), V8BindingError> {
    let key = opener_key(scope);
    let opener = opener.map_or_else(|| v8::null(scope).into(), Into::into);
    if window.set_private(scope, key, opener) != Some(true) {
        return Err(V8BindingError::Allocation("Window opener identity"));
    }
    Ok(())
}

pub(super) fn native_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let key = parent_key(scope);
    window
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(window)
}

pub(super) fn native_top<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let key = top_key(scope);
    window
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(window)
}

/// Browsing-context ancestry is a V8-traced private edge. Replacing the public
/// replaceable `parent` property cannot redirect native navigation or routing.
pub(super) fn set_parent(
    scope: &mut v8::PinScope,
    window: v8::Local<v8::Object>,
    parent: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let key = parent_key(scope);
    if window.set_private(scope, key, parent.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Window parent identity"));
    }
    Ok(())
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

/// Native embedding identity for algorithms that must cross origin boundaries.
/// This does not expose the frame to author script or consult replaceable fields.
pub(super) fn native_frame<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let name = v8::String::new(scope, "oryn.window.native.embeddingFrame")?;
    let key = v8::Private::for_api(scope, Some(name));
    window
        .get_private(scope, key)
        .and_then(|value| value.try_cast::<v8::Object>().ok())
}
pub(super) fn set_embedding_frame(
    scope: &mut v8::PinScope,
    window: v8::Local<v8::Object>,
    frame: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "oryn.window.native.embeddingFrame")
        .ok_or(V8BindingError::Allocation("native embedding frame key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    window
        .set_private(scope, key, frame.into())
        .filter(|set| *set)
        .ok_or(V8BindingError::Allocation(
            "native embedding frame identity",
        ))?;
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
        let window = v8::Local::new(scope, args.this());
        // The container is a native private edge even when its document was
        // initially cross-origin. Domain relaxation can change access after
        // commit, so compare the Window and container documents now.
        if let Some(frame) = native_frame(scope, window)
            && let Some(parent) = native_parent(scope, window).get_creation_context(scope)
            && let Some(child) = window.get_creation_context(scope)
            && parent
                .get_slot::<V8BindingState>()
                .is_some_and(|parent| !parent.execution_retired())
            && parent
                .get_security_token(scope)
                .strict_equals(child.get_security_token(scope))
        {
            rv.set(frame.into());
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
    match wrap_node(scope, &state, state.document_root()) {
        Ok(document) => rv.set(document.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn stop(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(state) = binding_state_for_receiver(scope, args.this()).filter(|state| {
        state.binding_global == BindingGlobal::Window
            && global_events::is_global(scope, state, args.this())
    }) else {
        throw_type_error(scope, "Window.stop requires a Window receiver");
        return;
    };
    if !global_events::check_window_access(scope, &state, args.this()) || state.execution_retired()
    {
        return;
    }
    if let Err(error) = state.stop_pending_document_load() {
        throw_binding_error(scope, &error);
        return;
    }
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Err(error) = navigation_api::stop_navigation(scope, &state) {
        throw_binding_error(scope, &error);
    }
}
