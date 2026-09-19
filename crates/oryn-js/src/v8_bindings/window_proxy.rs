//! Cross-origin Window property descriptors and their native callable lifetime.
//! Host callbacks receive a traced original Context in their private data.
use super::*;
use crate::v8_context_bridge::{CrossContextHandlers, set_cross_context_handlers};
use v8::MapFnTo;

const NAMES: [&str; 13] = [
    "window",
    "self",
    "location",
    "close",
    "closed",
    "focus",
    "blur",
    "frames",
    "length",
    "top",
    "opener",
    "parent",
    "postMessage",
];
const METHODS: [usize; 4] = [3, 5, 6, 12];

/// Native implementations in the order documented by `WINDOW_CROSS_ORIGIN_NAMES`.
/// Each callback uses `callback_context` to access its original Window realm.
/// The location setter must enforce navigation permissions and URL conversion.
pub struct WindowCrossOriginCallbacks {
    pub properties: [v8::FunctionCallback; 13],
    pub location_setter: v8::FunctionCallback,
}
pub const WINDOW_CROSS_ORIGIN_NAMES: [&str; 13] = NAMES;

/// Operations whose authority belongs to the browsing-context coordinator.
/// Property projection below never reads author-writable Window properties.
pub struct WindowLifecycleCallbacks {
    pub close: v8::FunctionCallback,
    pub closed: v8::FunctionCallback,
    pub focus: v8::FunctionCallback,
    pub blur: v8::FunctionCallback,
    pub opener: v8::FunctionCallback,
    pub navigate: v8::FunctionCallback,
}

pub fn native_callbacks(lifecycle: WindowLifecycleCallbacks) -> WindowCrossOriginCallbacks {
    WindowCrossOriginCallbacks {
        properties: [
            native_self.map_fn_to(),
            native_self.map_fn_to(),
            native_location.map_fn_to(),
            lifecycle.close,
            lifecycle.closed,
            lifecycle.focus,
            lifecycle.blur,
            native_self.map_fn_to(),
            native_length.map_fn_to(),
            native_top.map_fn_to(),
            lifecycle.opener,
            native_parent.map_fn_to(),
            window_messages::cross_origin_post.map_fn_to(),
        ],
        location_setter: lifecycle.navigate,
    }
}

fn native_self(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if let Some(context) = callback_context(scope, &args) {
        rv.set(context.global(scope).into());
    }
}

fn native_parent(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if let Some(context) = callback_context(scope, &args) {
        let scope = &mut v8::ContextScope::new(scope, context);
        let window = context.global(scope);
        rv.set(window_document::native_parent(scope, window).into());
    }
}

fn native_top(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if let Some(context) = callback_context(scope, &args) {
        let scope = &mut v8::ContextScope::new(scope, context);
        let window = context.global(scope);
        rv.set(window_document::native_top(scope, window).into());
    }
}

fn native_length(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(context) = callback_context(scope, &args) else {
        return;
    };
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return;
    };
    match window_named::children(&state) {
        Ok(children) => rv.set_uint32(children.len().try_into().unwrap_or(u32::MAX)),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn native_location(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(context) = callback_context(scope, &args) else {
        return;
    };
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return;
    };
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        location::object(scope, &state).map(|object| v8::Global::new(scope, object))
    };
    match result {
        Ok(object) => rv.set(v8::Local::new(scope, &object).into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub struct WindowCrossOriginState {
    callbacks: WindowCrossOriginCallbacks,
    children: RefCell<Vec<v8::Global<v8::Object>>>,
}
impl WindowCrossOriginState {
    pub fn set_children(&self, children: Vec<v8::Global<v8::Object>>) {
        *self.children.borrow_mut() = children;
    }
}
// Oryn-owned user embedder slot; rusty_v8 offsets this past its own slots.
// Unlike Global handles in Rust context slots, this graph is traced by V8.
const DESCRIPTOR_DATA_SLOT: i32 = 0;
struct DescriptorDataInitialized;
#[derive(Clone)]
struct WeakMapIntrinsics {
    constructor: v8::Global<v8::Function>,
    get: v8::Global<v8::Function>,
    set: v8::Global<v8::Function>,
}
fn weak_map_intrinsics(scope: &mut v8::PinScope) -> Option<WeakMapIntrinsics> {
    if let Some(intrinsics) = scope.get_slot::<WeakMapIntrinsics>() {
        return Some(intrinsics.clone());
    }
    // One private pristine realm per isolate supplies native WeakMap intrinsics.
    // No page can replace these functions, and no author JavaScript is run.
    let pristine = v8::Context::new(scope, v8::ContextOptions::default());
    let intrinsics = {
        let scope = &mut v8::ContextScope::new(scope, pristine);
        let key = v8::String::new(scope, "WeakMap")?;
        let constructor =
            v8::Local::<v8::Function>::try_from(pristine.global(scope).get(scope, key.into())?)
                .ok()?;
        let key = v8::String::new(scope, "prototype")?;
        let prototype = constructor.get(scope, key.into())?.to_object(scope)?;
        let key = v8::String::new(scope, "get")?;
        let get = v8::Local::<v8::Function>::try_from(prototype.get(scope, key.into())?).ok()?;
        let key = v8::String::new(scope, "set")?;
        let set = v8::Local::<v8::Function>::try_from(prototype.get(scope, key.into())?).ok()?;
        WeakMapIntrinsics {
            constructor: v8::Global::new(scope, constructor),
            get: v8::Global::new(scope, get),
            set: v8::Global::new(scope, set),
        }
    };
    scope.set_slot(intrinsics.clone());
    Some(intrinsics)
}
fn context_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<v8::Context>,
    intrinsics: &WeakMapIntrinsics,
) -> Option<v8::Local<'s, v8::Object>> {
    if context.get_slot::<DescriptorDataInitialized>().is_none() {
        let scope = &mut v8::ContextScope::new(scope, context);
        let data = v8::Object::new(scope);
        let null = v8::null(scope).into();
        data.set_prototype(scope, null)?;
        let key = v8::Object::new(scope);
        put(scope, data, "key", key.into())?;
        let constructor = v8::Local::new(scope, &intrinsics.constructor);
        let map = constructor.new_instance(scope, &[])?;
        put(scope, data, "map", map.into())?;
        context.set_embedder_data(DESCRIPTOR_DATA_SLOT, data.into());
        context.set_slot(Rc::new(DescriptorDataInitialized));
    }
    v8::Local::<v8::Object>::try_from(context.get_embedder_data(scope, DESCRIPTOR_DATA_SLOT)?).ok()
}
pub(super) fn cache_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let intrinsics = weak_map_intrinsics(scope)?;
    let caller = scope.get_current_context();
    let caller_data = context_data(scope, caller, &intrinsics)?;
    let target_data = context_data(scope, target, &intrinsics)?;
    let key_name = v8::String::new(scope, "key")?;
    let key = target_data.get(scope, key_name.into())?;
    let map_name = v8::String::new(scope, "map")?;
    let map = caller_data.get(scope, map_name.into())?;
    let get = v8::Local::new(scope, &intrinsics.get);
    let value = get.call(scope, map, &[key])?;
    if let Ok(entry) = v8::Local::<v8::Object>::try_from(value) {
        return Some(entry);
    }
    let entry = v8::Object::new(scope);
    let null = v8::null(scope).into();
    entry.set_prototype(scope, null)?;
    let set = v8::Local::new(scope, &intrinsics.set);
    set.call(scope, map, &[key, entry.into()])?;
    Some(entry)
}

/// Private native cache graph shared with remote Window projections. Entries
/// are keyed by the caller realm and native target context; no authored map or
/// constructor participates in lookup. Callers namespace their native keys.
pub fn native_descriptor_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    cache_entry(scope, target)
}

pub fn install_context(
    context: v8::Local<v8::Context>,
    callbacks: WindowCrossOriginCallbacks,
) -> Rc<WindowCrossOriginState> {
    let state = Rc::new(WindowCrossOriginState {
        callbacks,
        children: RefCell::new(Vec::new()),
    });
    context.set_slot(state.clone());
    state
}

/// The original context is retained by V8 tracing, even after proxy navigation.
pub fn callback_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments,
) -> Option<v8::Local<'s, v8::Context>> {
    let data = v8::Local::<v8::Object>::try_from(args.data()).ok()?;
    v8::Local::<v8::Context>::try_from(data.get_internal_field(scope, 0)?).ok()
}
fn denied(scope: &mut v8::PinScope) {
    throw_binding_error(
        scope,
        &V8BindingError::Unsupported {
            interface: "Window".into(),
            member: "SecurityError".into(),
        },
    );
}
fn key_index(scope: &mut v8::PinScope, key: v8::Local<v8::Name>) -> Option<usize> {
    key.is_string()
        .then(|| {
            NAMES
                .iter()
                .position(|name| *name == key.to_rust_string_lossy(scope))
        })
        .flatten()
}
pub(super) fn fallback(scope: &mut v8::PinScope, key: v8::Local<v8::Name>) -> bool {
    if key.is_string() {
        return key.to_rust_string_lossy(scope) == "then";
    }
    [
        v8::Symbol::get_to_string_tag(scope),
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ]
    .into_iter()
    .any(|symbol| key.strict_equals(symbol.into()))
}
fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<v8::Object>,
) -> Option<(v8::Local<'s, v8::Context>, Rc<WindowCrossOriginState>)> {
    let context = holder.get_creation_context(scope)?;
    Some((context, context.get_slot::<WindowCrossOriginState>()?))
}
fn named_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<v8::Object>,
    key: v8::Local<v8::Name>,
) -> Option<v8::Local<'s, v8::Object>> {
    let key = v8::Local::<v8::String>::try_from(key).ok()?;
    let name = rust_string_preserving_utf16(scope, key).ok()?;
    let context = holder.get_creation_context(scope)?;
    let state = context.get_slot::<V8BindingState>()?;
    window_named::named_child(&state, &name)
        .ok()
        .flatten()
        .map(|window| v8::Local::new(scope, &window))
}
fn callable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<v8::Context>,
    state: &WindowCrossOriginState,
    index: usize,
    setter: bool,
) -> Option<v8::Local<'s, v8::Function>> {
    let entry = cache_entry(scope, context)?;
    let key = v8::String::new(scope, &format!("{index}:{}", u8::from(setter)))?;
    if entry.has_own_property(scope, key.into())? {
        return v8::Local::<v8::Function>::try_from(entry.get(scope, key.into())?).ok();
    }
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(1);
    let data = template.new_instance(scope)?;
    data.set_internal_field(0, context.into());
    let callback = if setter {
        state.callbacks.location_setter
    } else {
        state.callbacks.properties[index]
    };
    let name = if setter {
        format!("set {}", NAMES[index])
    } else if METHODS.contains(&index) {
        NAMES[index].to_owned()
    } else {
        format!("get {}", NAMES[index])
    };
    let length = i32::from(setter || index == 12);
    let function = v8::Function::builder_raw(callback)
        .length(length)
        .data(data.into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    function.set_name(v8::String::new(scope, &name)?);
    entry.create_data_property(scope, key.into(), function.into())?;
    Some(function)
}
fn method_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<v8::Context>,
    state: &WindowCrossOriginState,
    index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    // Cross-origin exposure uses the platform's original methods, regardless
    // of own-property replacements or getters installed by the target page.
    callable(scope, context, state, index, false).map(Into::into)
}
fn put(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) -> Option<()> {
    let key = v8::String::new(scope, key)?;
    object.create_data_property(scope, key.into(), value)?;
    Some(())
}
pub(super) fn data_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
    enumerable: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = descriptor_record(scope)?;
    put(scope, result, "value", value)?;
    let no = v8::Boolean::new(scope, false).into();
    put(scope, result, "writable", no)?;
    let enumerable = v8::Boolean::new(scope, enumerable).into();
    put(scope, result, "enumerable", enumerable)?;
    let yes = v8::Boolean::new(scope, true).into();
    put(scope, result, "configurable", yes)?;
    Some(result)
}
pub(super) fn descriptor_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    // V8 converts interceptor results with ToPropertyDescriptor, including
    // inherited fields. These internal records must not inherit author getters
    // or acquire both accessor and data fields through Object.prototype.
    let result = v8::Object::new(scope);
    let null = v8::null(scope).into();
    if !result.set_prototype(scope, null)? {
        return None;
    }
    Some(result)
}
fn named_descriptor(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(index) = key_index(scope, key) {
        if let Some((context, state)) = target(scope, args.holder()) {
            if METHODS.contains(&index) {
                if let Some(value) = method_value(scope, context, &state, index)
                    && let Some(desc) = data_descriptor(scope, value, false)
                {
                    rv.set(desc.into());
                }
            } else if let Some(function) = callable(scope, context, &state, index, false) {
                let Some(desc) = descriptor_record(scope) else {
                    return v8::Intercepted::kYes;
                };
                put(scope, desc, "get", function.into());
                let setter: v8::Local<v8::Value> = if index == 2 {
                    let Some(setter) = callable(scope, context, &state, index, true) else {
                        return v8::Intercepted::kYes;
                    };
                    setter.into()
                } else {
                    v8::undefined(scope).into()
                };
                put(scope, desc, "set", setter);
                let no = v8::Boolean::new(scope, false).into();
                put(scope, desc, "enumerable", no);
                let yes = v8::Boolean::new(scope, true).into();
                put(scope, desc, "configurable", yes);
                rv.set(desc.into());
            }
        } else {
            denied(scope);
        }
    } else if let Some(child) = named_child(scope, args.holder(), key) {
        if let Some(desc) = data_descriptor(scope, child.into(), false) {
            rv.set(desc.into());
        }
    } else if fallback(scope, key) {
        let undefined = v8::undefined(scope).into();
        if let Some(desc) = data_descriptor(scope, undefined, false) {
            rv.set(desc.into());
        }
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn named_getter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(index) = key_index(scope, key) {
        if let Some((context, state)) = target(scope, args.holder()) {
            if METHODS.contains(&index) {
                if let Some(value) = method_value(scope, context, &state, index) {
                    rv.set(value);
                }
            } else if let Some(function) = callable(scope, context, &state, index, false)
                && let Some(value) = function.call(scope, args.holder().into(), &[])
            {
                rv.set(value);
            }
        } else {
            denied(scope);
        }
    } else if let Some(child) = named_child(scope, args.holder(), key) {
        rv.set(child.into());
    } else if fallback(scope, key) {
        rv.set(v8::undefined(scope).into());
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn named_setter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if key_index(scope, key) == Some(2) {
        if let Some((context, state)) = target(scope, args.holder())
            && let Some(function) = callable(scope, context, &state, 2, true)
        {
            function.call(scope, args.holder().into(), &[value]);
        } else {
            denied(scope);
        }
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn named_query(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    if key_index(scope, key).is_some() || fallback(scope, key) {
        rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    } else if named_child(scope, args.holder(), key).is_some() {
        rv.set_int32(
            (v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY).as_u32() as i32,
        );
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn named_enumerator(
    scope: &mut v8::PinScope,
    _: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    // Construct own array elements directly; ordinary indexed assignment can
    // invoke author setters on Array.prototype during native enumeration.
    let mut keys = Vec::with_capacity(17);
    for name in NAMES.iter().chain(std::iter::once(&"then")) {
        let Some(name) = v8::String::new(scope, name) else {
            return;
        };
        keys.push(name.into());
    }
    for symbol in [
        v8::Symbol::get_to_string_tag(scope),
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ] {
        keys.push(symbol.into());
    }
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
fn indexed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let children = local_children(scope, holder)?;
    children
        .get(index as usize)
        .map(|child| v8::Local::new(scope, child))
}
fn local_children(
    scope: &mut v8::PinScope,
    holder: v8::Local<v8::Object>,
) -> Option<Vec<v8::Global<v8::Object>>> {
    let context = holder.get_creation_context(scope)?;
    if let Some(state) = context.get_slot::<V8BindingState>() {
        return window_named::children(&state).ok();
    }
    Some(
        context
            .get_slot::<WindowCrossOriginState>()?
            .children
            .borrow()
            .clone(),
    )
}
fn indexed_getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(value) = indexed_value(scope, args.holder(), index) {
        rv.set(value.into());
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn indexed_descriptor(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(value) = indexed_value(scope, args.holder(), index) {
        if let Some(desc) = data_descriptor(scope, value.into(), true) {
            rv.set(desc.into());
        }
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn indexed_query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    if indexed_value(scope, args.holder(), index).is_some() {
        rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn indexed_enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    if let Some(children) = local_children(scope, args.holder()) {
        let len = children.len();
        let mut keys = Vec::with_capacity(len);
        for index in 0..len {
            let value = v8::Integer::new_from_unsigned(scope, index as u32);
            keys.push(value.into());
        }
        rv.set(v8::Array::new_with_elements(scope, &keys));
    } else {
        denied(scope);
    }
}

fn named_deleter(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}

fn named_definer(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    _: &v8::PropertyDescriptor,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}

fn indexed_deleter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}

fn indexed_definer(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}

fn indexed_setter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}

fn local_indexed_getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(value) = indexed_value(scope, args.holder(), index) {
        rv.set(value.into());
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}
fn local_indexed_descriptor(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(value) = indexed_value(scope, args.holder(), index) {
        if let Some(desc) = data_descriptor(scope, value.into(), true) {
            rv.set(desc.into());
        }
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}
fn local_indexed_query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    if indexed_value(scope, args.holder(), index).is_some() {
        rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}
fn local_indexed_deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(indexed_value(scope, args.holder(), index).is_none());
    v8::Intercepted::kYes
}
pub fn configure_template(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<v8::ObjectTemplate>,
) {
    // Structural WindowProxy semantics reject author prototype mutations.
    // Do not freeze the inner global template: native binding installation
    // still needs to attach Window.prototype and its named-properties chain.
    configure_local_template(scope, template);
    crate::v8_context_bridge::set_window_failed_access_callback(scope);
    set_cross_context_handlers(
        scope,
        template,
        &CrossContextHandlers {
            named_getter: Some(named_getter.map_fn_to()),
            named_setter: Some(named_setter.map_fn_to()),
            named_query: Some(named_query.map_fn_to()),
            named_deleter: Some(named_deleter.map_fn_to()),
            named_enumerator: Some(named_enumerator.map_fn_to()),
            named_definer: Some(named_definer.map_fn_to()),
            named_descriptor: Some(named_descriptor.map_fn_to()),
            indexed_getter: Some(indexed_getter.map_fn_to()),
            indexed_setter: Some(indexed_setter.map_fn_to()),
            indexed_query: Some(indexed_query.map_fn_to()),
            indexed_deleter: Some(indexed_deleter.map_fn_to()),
            indexed_enumerator: Some(indexed_enumerator.map_fn_to()),
            indexed_definer: Some(indexed_definer.map_fn_to()),
            indexed_descriptor: Some(indexed_descriptor.map_fn_to()),
        },
    );
}

/// Native own indexed access for production Window globals. This does not
/// change the host's cross-origin access policy or install script proxies.
pub fn configure_local_template(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<v8::ObjectTemplate>,
) {
    crate::v8_context_bridge::set_window_index_handlers(
        scope,
        template,
        &CrossContextHandlers {
            indexed_getter: Some(local_indexed_getter.map_fn_to()),
            indexed_query: Some(local_indexed_query.map_fn_to()),
            indexed_deleter: Some(local_indexed_deleter.map_fn_to()),
            indexed_enumerator: Some(indexed_enumerator.map_fn_to()),
            indexed_descriptor: Some(local_indexed_descriptor.map_fn_to()),
            ..Default::default()
        },
    );
}

#[unsafe(no_mangle)]
unsafe extern "C" fn oryn_v8_window_access_denied(
    context: v8::Local<v8::Context>,
    target: v8::Local<v8::Object>,
) {
    // SAFETY: The C++ failed-access callback supplies rooted handles from the
    // currently entered isolate and context. No callback scope escapes.
    v8::callback_scope!(unsafe scope, context);
    if target
        .get_creation_context(scope)
        .and_then(|context| context.get_slot::<WindowCrossOriginState>())
        .is_some()
    {
        denied(scope);
    } else {
        throw_type_error(scope, "no access");
    }
}
