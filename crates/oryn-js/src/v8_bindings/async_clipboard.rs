//! Promise clipboard entry points; authorization is parent-owned, not activation globals.
use super::*;
use crate::clipboard::{ClipboardBackend, ClipboardData};

fn origins(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<Vec<String>, V8BindingError> {
    if state.execution_retired() {
        return Err(failure("InvalidStateError"));
    }
    let mut window = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let mut result = Vec::new();
    loop {
        let context = window
            .get_creation_context(scope)
            .ok_or_else(|| failure("NotAllowedError"))?;
        let descriptor = context
            .get_slot::<crate::realm::RealmDescriptor>()
            .ok_or_else(|| failure("NotAllowedError"))?;
        if descriptor.origin.is_opaque() {
            return Err(failure("NotAllowedError"));
        }
        result.push(descriptor.origin.serialize());
        let parent = window_document::native_parent(scope, window);
        if parent.strict_equals(window.into()) {
            break;
        }
        if result.len() >= 256 {
            return Err(failure("NotAllowedError"));
        }
        window = parent;
    }
    Ok(result)
}
fn failure(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Clipboard".into(),
        member: name.into(),
    }
}
fn backend(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    write: bool,
) -> Result<std::sync::Arc<dyn ClipboardBackend>, V8BindingError> {
    let window = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    {
        let dom = state.dom.borrow();
        if let Some(focused) = dom.focused() {
            if dom.arena().node_document(focused)? != Some(state.document_root()) {
                return Err(failure("NotAllowedError"));
            }
        } else if !window_document::native_parent(scope, window).strict_equals(window.into()) {
            return Err(failure("NotAllowedError"));
        }
    }
    let origins = origins(scope, state)?;
    state.refresh_native_container_policy()?;
    let backend = state
        .clipboard_backend
        .borrow()
        .clone()
        .ok_or_else(|| failure("NotAllowedError"))?;
    backend
        .authorize_script_for(&clipboard_actions::creator(state)?, &origins, write)
        .map_err(|_| failure("NotAllowedError"))?;
    Ok(backend)
}
fn key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.native.clipboard.owner").unwrap();
    v8::Private::for_api(scope, Some(name))
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    // Root the supplied context in this handle scope before Navigator creation.
    let context = v8::Local::new(scope, context);
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let navigator = native_navigator::create(scope, context, &state)?;
    let name = v8::String::new(scope, "navigator").unwrap();
    context
        .global(scope)
        .create_data_property(scope, name.into(), navigator.into());
    if !origins(scope, &state).is_ok_and(|origins| {
        origins.iter().all(|origin| {
            url::Url::parse(origin).is_ok_and(|url| {
                url.scheme() == "https"
                    || url.scheme() == "wss"
                    || (matches!(url.scheme(), "http" | "ws")
                        && url.host_str().is_some_and(|host| {
                            host == "localhost"
                                || host.ends_with(".localhost")
                                || host
                                    .parse::<std::net::IpAddr>()
                                    .is_ok_and(|ip| ip.is_loopback())
                        }))
            })
        })
    }) {
        return Ok(());
    }
    let event_id = register_platform_object(&state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, &state, "EventTarget", event_id)?;
    let clipboard_constructor = v8::Function::new(scope, illegal_clipboard)
        .ok_or(V8BindingError::Allocation("Clipboard"))?;
    let name = v8::String::new(scope, "prototype").unwrap();
    let clipboard_prototype = clipboard_constructor
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Clipboard prototype"))?;
    if let Some(parent) = object.get_prototype(scope) {
        clipboard_prototype.set_prototype(scope, parent);
    }
    object.set_prototype(scope, clipboard_prototype.into());
    let name = v8::String::new(scope, "Clipboard").unwrap();
    context
        .global(scope)
        .create_data_property(scope, name.into(), clipboard_constructor.into());
    let private = key(scope);
    let owner = v8::BigInt::new_from_u64(scope, state.instance_id);
    object.set_private(scope, private, owner.into());
    for (name, index) in [("readText", 0), ("writeText", 1), ("read", 2), ("write", 3)] {
        let data = v8::Integer::new(scope, index);
        let function = v8::Function::builder(call)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("Clipboard operation"))?;
        let name = v8::String::new(scope, name).unwrap();
        clipboard_prototype.create_data_property(scope, name.into(), function.into());
    }
    let constructor = v8::Function::new(scope, item_constructor)
        .ok_or(V8BindingError::Allocation("ClipboardItem"))?;
    let prototype_name = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, prototype_name.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let name = v8::String::new(scope, "oryn.native.clipboard.item.prototype").unwrap();
    let key = v8::Private::for_api(scope, Some(name));
    context.global(scope).set_private(scope, key, prototype);
    if let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype) {
        for (name, index) in [("types", 0), ("presentationStyle", 1)] {
            let data = v8::Integer::new(scope, index);
            let getter = v8::Function::builder(item_attribute)
                .data(data.into())
                .build(scope)
                .ok_or(V8BindingError::Allocation("ClipboardItem getter"))?;
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let name = v8::String::new(scope, name).unwrap();
            prototype.define_property(scope, name.into(), &descriptor);
        }
        let method = v8::Function::new(scope, get_type)
            .ok_or(V8BindingError::Allocation("ClipboardItem getType"))?;
        let name = v8::String::new(scope, "getType").unwrap();
        prototype.create_data_property(scope, name.into(), method.into());
    }
    let supports = v8::Function::builder(supported)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("ClipboardItem.supports"))?;
    let name = v8::String::new(scope, "supports").unwrap();
    supports.set_name(name);
    constructor.create_data_property(scope, name.into(), supports.into());
    let name = v8::String::new(scope, "ClipboardItem").unwrap();
    context
        .global(scope)
        .create_data_property(scope, name.into(), constructor.into());
    let name = v8::String::new(scope, "clipboard").unwrap();
    navigator.define_own_property(
        scope,
        name.into(),
        object.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    let name = v8::String::new(scope, "navigator").unwrap();
    context
        .global(scope)
        .create_data_property(scope, name.into(), navigator.into());
    Ok(())
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let private = key(scope);
    let valid = args
        .this()
        .get_private(scope, private)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .is_some_and(|value| value.u64_value() == (state.instance_id, true));
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        if !valid {
            return Err(failure("TypeError"));
        }
        let operation = args.data().int32_value(scope).unwrap_or(-1);
        if operation == 2 {
            return read_items(scope, &state);
        }
        if operation == 3 {
            begin_write(scope, &state, args.get(0), resolver)?;
            return Ok(resolver.get_promise(scope).into());
        }
        let write = operation == 1;
        // WebIDL argument conversion precedes policy checks and may run author code.
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_authority_epoch(),
            state.document_stream_epoch(),
        );
        let text = if write {
            if args.length() == 0 {
                return Err(failure("TypeError"));
            }
            Some(required_string(scope, &args, 0)?)
        } else {
            None
        };
        if identity
            != (
                state.document_root(),
                state.document_generation(),
                state.document_authority_epoch(),
                state.document_stream_epoch(),
            )
            || state.execution_retired()
        {
            return Err(failure("InvalidStateError"));
        }
        let backend = backend(scope, &state, write)?;
        if let Some(text) = text {
            let data = ClipboardData::text(oryn_dom::dom_string::units(&text));
            data.validate().map_err(|_| failure("QuotaExceededError"))?;
            backend
                .write_for(&clipboard_actions::creator(&state)?, data)
                .map_err(|_| failure("NotAllowedError"))?;
            Ok(v8::undefined(scope).into())
        } else {
            let data = backend
                .read_for(&clipboard_actions::creator(&state)?)
                .map_err(|_| failure("NotAllowedError"))?;
            data.validate().map_err(|_| failure("DataError"))?;
            let text = oryn_dom::dom_string::from_units(data.plain_text());
            Ok(v8_string(scope, &text)?)
        }
    })();
    match result {
        Ok(value) => {
            if !value.strict_equals(resolver.get_promise(scope).into()) {
                resolver.resolve(scope, value);
            }
        }
        Err(error) => {
            if let Ok(exception) = clipboard_exception(scope, &error) {
                resolver.reject(scope, exception);
            }
        }
    }
}

pub(super) fn exec(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    command: &str,
    execute: bool,
) -> Result<bool, V8BindingError> {
    let action = match command {
        "copy" => crate::clipboard::ClipboardAction::Copy,
        "cut" => crate::clipboard::ClipboardAction::Cut,
        "paste" => crate::clipboard::ClipboardAction::Paste,
        _ => return Ok(false),
    };
    if backend(scope, state, command != "paste").is_err() {
        return Ok(false);
    }
    if !execute {
        return Ok(true);
    }
    // The permission was granted by the parent for this operation; it never mints
    // a persistent activation or author-visible token.
    let target = state
        .dom
        .borrow()
        .focused_element_in(state.document_root())?
        .unwrap_or(state.document_root());
    let mut grant = clipboard_actions::grant(state, target, action)?;
    clipboard_actions::action(scope, state, &mut grant)
        .map(|result| result.clipboard_changed || result.content_changed || result.event_cancelled)
}

fn item_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.native.clipboard.item").unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn make_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object = v8::Object::new(scope);
    let key = item_key(scope);
    object.set_private(scope, key, entries.into());
    let name = v8::String::new(scope, "oryn.native.clipboard.item.prototype").unwrap();
    let key = v8::Private::for_api(scope, Some(name));
    let global = scope.get_current_context().global(scope);
    if let Some(prototype) = global.get_private(scope, key) {
        object.set_prototype(scope, prototype);
    }
    Ok(object)
}
fn item_constructor(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        if !args.is_construct_call() {
            return Err(failure("TypeError"));
        }
        let object = args
            .get(0)
            .to_object(scope)
            .ok_or_else(|| failure("TypeError"))?;
        let names = object
            .get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
            .ok_or(V8BindingError::JavaScriptException)?;
        if names.length() == 0 || names.length() as usize > crate::clipboard::MAX_CLIPBOARD_ITEMS {
            return Err(failure("TypeError"));
        }
        let entries = v8::Array::new(scope, 0);
        for index in 0..names.length() {
            let name = names
                .get_index(scope, index)
                .ok_or(V8BindingError::JavaScriptException)?;
            let value = object
                .get(scope, name)
                .ok_or(V8BindingError::JavaScriptException)?;
            let promise = resolved_promise(scope, value)?;
            entries.set_index(scope, index * 2, name);
            entries.set_index(scope, index * 2 + 1, promise.into());
        }
        let item = make_item(scope, entries)?;
        let mut style = "unspecified".to_string();
        if !args.get(1).is_null_or_undefined() {
            let options = args
                .get(1)
                .to_object(scope)
                .ok_or_else(|| failure("TypeError"))?;
            let name = v8::String::new(scope, "presentationStyle").unwrap();
            let value = options
                .get(scope, name.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            if !value.is_undefined() {
                style = value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
            }
        }
        if !matches!(style.as_str(), "unspecified" | "inline" | "attachment") {
            return Err(failure("TypeError"));
        }
        let name = v8::String::new(scope, "oryn.native.clipboard.item.style").unwrap();
        let key = v8::Private::for_api(scope, Some(name));
        let value = v8_string(scope, &style)?;
        item.set_private(scope, key, value);
        Ok(item)
    })();
    match result {
        Ok(value) => rv.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn get_type(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Promise>, V8BindingError> {
        let key = item_key(scope);
        let entries = args
            .this()
            .get_private(scope, key)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .ok_or_else(|| failure("TypeError"))?;
        let requested = required_string(scope, &args, 0)?;
        for index in (0..entries.length()).step_by(2) {
            let name = entries
                .get_index(scope, index)
                .ok_or(V8BindingError::JavaScriptException)?;
            if name.to_rust_string_lossy(scope) == requested {
                let promise = entries
                    .get_index(scope, index + 1)
                    .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
                    .ok_or_else(|| failure("DataError"))?;
                let data = v8::String::new(scope, &requested).unwrap();
                let fulfilled = v8::Function::builder(as_blob)
                    .data(data.into())
                    .build(scope)
                    .ok_or(V8BindingError::Allocation("clipboard Blob conversion"))?;
                return promise
                    .then(scope, fulfilled)
                    .ok_or(V8BindingError::JavaScriptException);
            }
        }
        Err(failure("NotFoundError"))
    })();
    match result {
        Ok(promise) => rv.set(promise.into()),
        Err(error) => {
            if let (Some(resolver), Ok(exception)) = (
                v8::PromiseResolver::new(scope),
                clipboard_exception(scope, &error),
            ) {
                resolver.reject(scope, exception);
                rv.set(resolver.get_promise(scope).into());
            }
        }
    }
}
fn as_blob(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        if state.execution_retired() {
            return Err(failure("InvalidStateError"));
        }
        let media_type = args.data().to_rust_string_lossy(scope);
        if let Ok(object) = v8::Local::<v8::Object>::try_from(args.get(0))
            && let Some(owner) = binding_state_for_receiver(scope, object)
            && let Ok(id) = stateful_id(scope, object)
            && let Ok(record) = platform_object(&owner, id)
            && let PlatformObjectRecord::Blob(blob) = &*record
        {
            if blob.media_type != media_type {
                return Err(failure("TypeError"));
            }
            return Ok(object.into());
        }
        if !args.get(0).is_string() {
            return Err(failure("TypeError"));
        }
        let text = args.get(0).to_rust_string_lossy(scope);
        if text.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
            return Err(failure("QuotaExceededError"));
        }
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::Blob(BlobRecord {
                bytes: text.into_bytes(),
                media_type,
                file: None,
            }),
        );
        Ok(new_platform_object(scope, &state, "Blob", id)?.into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn supported(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if args.length() == 0 {
        throw_type_error(scope, "ClipboardItem.supports requires a type");
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    rv.set_bool(matches!(
        value.to_rust_string_lossy(scope).as_str(),
        "text/plain" | "text/html" | "image/png"
    ));
}
fn read_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let data = backend(scope, state, false)?
        .read_for(&clipboard_actions::creator(state)?)
        .map_err(|_| failure("NotAllowedError"))?;
    data.validate().map_err(|_| failure("DataError"))?;
    let output = v8::Array::new(scope, 0);
    if data.items.is_empty() {
        return Ok(output.into());
    }
    let entries = v8::Array::new(scope, 0);
    let mut index = 0;
    for item in data.items {
        let bytes = match item.payload {
            crate::clipboard::ClipboardPayload::Text(units) => {
                String::from_utf16_lossy(&units).into_bytes()
            }
            crate::clipboard::ClipboardPayload::File { bytes, .. } => bytes,
        };
        let bytes = if item.media_type == "text/html" {
            clipboard_actions::sanitized_html_for_state(state, &String::from_utf8_lossy(&bytes))?
                .into_bytes()
        } else if item.media_type == "image/png" {
            sanitize_png(&bytes)?
        } else {
            bytes
        };
        let name = v8::String::new(scope, &item.media_type).unwrap();
        let id = register_platform_object(
            state,
            PlatformObjectRecord::Blob(BlobRecord {
                bytes,
                media_type: item.media_type,
                file: None,
            }),
        );
        let blob = new_platform_object(scope, state, "Blob", id)?;
        let promise = resolved_promise(scope, blob.into())?;
        entries.set_index(scope, index, name.into());
        entries.set_index(scope, index + 1, promise.into());
        index += 2;
    }
    let item = make_item(scope, entries)?;
    output.set_index(scope, 0, item.into());
    Ok(output.into())
}
// Native continuation state is only reachable from native function callback data.
// It is never installed on an author object, and a delayed write rechecks policy.
// Slot zero is always the native PromiseResolver installed by begin_write below.
// It is never replaced; unchecked recovery casts rely on this private-data invariant.
fn begin_write(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    resolver: v8::Local<v8::PromiseResolver>,
) -> Result<(), V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure("TypeError"))?;
    let symbol = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, symbol.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))?;
    let array = v8::Array::new(scope, 0);
    iterable_conversion::consume(scope, object, method, |scope, value| {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure("TypeError"))?;
        let key = item_key(scope);
        if !object
            .get_private(scope, key)
            .is_some_and(|value| value.is_array())
        {
            return Err(failure("TypeError"));
        }
        if array.length() as usize >= crate::clipboard::MAX_CLIPBOARD_ITEMS {
            return Err(failure("QuotaExceededError"));
        }
        array.set_index(scope, array.length(), value);
        Ok(())
    })?;
    backend(scope, state, true)?;
    if array.length() > 1 {
        return Err(failure("NotAllowedError"));
    }
    if array.length() == 0 {
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        return Ok(());
    }
    let item = array
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))?;
    let key = item_key(scope);
    let entries = item
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))?;
    let data = v8::Array::new(scope, 5);
    let resolver_key = v8::String::new(scope, "0").ok_or_else(|| failure("DataError"))?;
    // Define an own immutable slot: poisoned Array.prototype index setters cannot
    // replace or suppress the resolver whose native type is recovered below.
    if data.define_own_property(
        scope,
        resolver_key.into(),
        resolver.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
    ) != Some(true)
    {
        return Err(failure("DataError"));
    }
    data.set_index(scope, 1, entries.into());
    let values = v8::Array::new(scope, 0);
    data.set_index(scope, 2, values.into());
    let owner = v8::BigInt::new_from_u64(scope, state.instance_id);
    data.set_index(scope, 3, owner.into());
    let epoch = v8::BigInt::new_from_u64(scope, state.document_stream_epoch());
    data.set_index(scope, 4, epoch.into());
    continue_write(scope, state, data)
}
fn continue_write(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    data: v8::Local<v8::Array>,
) -> Result<(), V8BindingError> {
    let resolver = data
        .get_index(scope, 0)
        .map(|value| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
        .ok_or_else(|| failure("DataError"))?;
    let entries = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| failure("DataError"))?;
    let values = data
        .get_index(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| failure("DataError"))?;
    let owner = data
        .get_index(scope, 3)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0);
    let epoch = data
        .get_index(scope, 4)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0);
    if owner != Some(state.instance_id)
        || epoch != Some(state.document_stream_epoch())
        || state.execution_retired()
    {
        return Err(failure("InvalidStateError"));
    }
    if values.length() * 2 < entries.length() {
        let promise = entries
            .get_index(scope, values.length() * 2 + 1)
            .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
            .ok_or_else(|| failure("DataError"))?;
        let fulfilled = v8::Function::builder(write_fulfilled)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("clipboard write continuation"))?;
        let rejected = v8::Function::builder(write_rejected)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("clipboard write rejection"))?;
        promise
            .then2(scope, fulfilled, rejected)
            .ok_or(V8BindingError::JavaScriptException)?;
        return Ok(());
    }
    let backend = backend(scope, state, true)?;
    let mut output = ClipboardData::default();
    let mut size = 0usize;
    for index in 0..values.length() {
        let name = entries
            .get_index(scope, index * 2)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        if !matches!(name.as_str(), "text/plain" | "text/html" | "image/png") {
            return Err(failure("NotAllowedError"));
        }
        let value = values
            .get_index(scope, index)
            .ok_or(V8BindingError::JavaScriptException)?;
        let bytes = if value.is_string() {
            value.to_rust_string_lossy(scope).into_bytes()
        } else {
            let object =
                v8::Local::<v8::Object>::try_from(value).map_err(|_| failure("TypeError"))?;
            let owner =
                binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
            if owner.execution_retired() {
                return Err(failure("InvalidStateError"));
            }
            let id = stateful_id(scope, object)?;
            let record = platform_object(&owner, id)?;
            let PlatformObjectRecord::Blob(blob) = &*record else {
                return Err(failure("TypeError"));
            };
            if blob.media_type != name {
                return Err(failure("TypeError"));
            }
            size = size
                .checked_add(blob.bytes.len())
                .ok_or_else(|| failure("QuotaExceededError"))?;
            if size > crate::clipboard::MAX_CLIPBOARD_BYTES {
                return Err(failure("QuotaExceededError"));
            }
            blob.bytes.clone()
        };
        let bytes = if name == "image/png" {
            sanitize_png(&bytes)?
        } else {
            bytes
        };
        let payload = if name.starts_with("text/") {
            let text = String::from_utf8_lossy(&bytes);
            let text = if name == "text/html" {
                clipboard_actions::sanitized_html_for_state(state, &text)?
            } else {
                text.into_owned()
            };
            crate::clipboard::ClipboardPayload::Text(text.encode_utf16().collect())
        } else {
            crate::clipboard::ClipboardPayload::File {
                name: String::new(),
                last_modified: 0,
                bytes,
            }
        };
        output.items.push(crate::clipboard::ClipboardItem {
            media_type: name,
            payload,
        });
    }
    output
        .validate()
        .map_err(|_| failure("QuotaExceededError"))?;
    backend
        .write_for(&clipboard_actions::creator(state)?, output)
        .map_err(|_| failure("NotAllowedError"))?;
    let undefined = v8::undefined(scope);
    resolver.resolve(scope, undefined.into());
    Ok(())
}
fn write_fulfilled(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(values) = data
        .get_index(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    values.set_index(scope, values.length(), args.get(0));
    if let Err(error) = continue_write(scope, &state, data) {
        reject_write(scope, data, &error);
    }
}
fn reject_write(scope: &mut v8::PinScope, data: v8::Local<v8::Array>, error: &V8BindingError) {
    if let Some(resolver) = data
        .get_index(scope, 0)
        .map(|value| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
        && let Ok(exception) = clipboard_exception(scope, error)
    {
        resolver.reject(scope, exception);
    }
}
fn clipboard_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: &V8BindingError,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let name = match error {
        V8BindingError::Unsupported { member, .. } => member.as_str(),
        _ => "OperationError",
    };
    dom_exception_value(scope, name)
}

fn write_rejected(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    if let Some(resolver) = data
        .get_index(scope, 0)
        .map(|value| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
    {
        resolver.reject(scope, args.get(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Default)]
    struct Backend {
        allowed: AtomicBool,
        data: Mutex<ClipboardData>,
    }
    impl ClipboardBackend for Backend {
        fn authorize_script(
            &self,
            origins: &[String],
            _write: bool,
        ) -> Result<(), crate::clipboard::ClipboardError> {
            if self.allowed.load(Ordering::SeqCst) && origins == ["https://clipboard.test"] {
                Ok(())
            } else {
                Err(crate::clipboard::ClipboardError::NotAllowed)
            }
        }
        fn read(&self) -> Result<ClipboardData, crate::clipboard::ClipboardError> {
            Ok(self.data.lock().unwrap().clone())
        }
        fn write(&self, data: ClipboardData) -> Result<(), crate::clipboard::ClipboardError> {
            *self.data.lock().unwrap() = data;
            Ok(())
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn installation_with_an_outer_context_survives_nested_handle_scope_exit() {
        super::super::tests::initialize_v8();
        for (origin, secure) in [
            ("http://clipboard.test", false),
            ("https://clipboard.test", true),
        ] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                id: RealmId(0),
                origin: crate::origin::Origin::from_url(origin),
                document_generation: 1,
                frame_id: None,
            }));
            context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
            {
                // The context handle belongs to the enclosing handle scope.
                v8::scope!(let nested, scope);
                let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                install_generated_bindings(nested, context, dom, RealmId(0), 1).unwrap();
                assert!(
                    eval(
                        nested,
                        "globalThis.installedNavigator = navigator; navigator instanceof Navigator",
                    )
                    .is_true()
                );
                if secure {
                    assert!(
                        eval(
                            nested,
                            r#"(() => {
                        globalThis.installedClipboard = navigator.clipboard;
                        return installedClipboard instanceof Clipboard &&
                            typeof installedClipboard.readText === 'function' &&
                            typeof installedClipboard.writeText === 'function';
                    })()"#
                        )
                        .is_true()
                    );
                } else {
                    assert!(eval(nested, "typeof navigator.clipboard === 'undefined'").is_true());
                }
            }
            assert!(
                eval(
                    scope,
                    "navigator === installedNavigator && navigator instanceof Navigator"
                )
                .is_true()
            );
            if secure {
                assert!(eval(scope, "navigator.clipboard === installedClipboard").is_true());
            } else {
                assert!(eval(scope, "typeof navigator.clipboard === 'undefined'").is_true());
            }
        }
    }
    #[test]
    fn static_operations_keep_idl_metadata_and_fallible_conversion_without_receiver_brands() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://clipboard.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const methods=[[HTMLScriptElement.supports,'supports',1],[ClipboardItem.supports,'supports',1],[AbortSignal.abort,'abort',0],[AbortSignal.any,'any',1],[AbortSignal.timeout,'timeout',1],[Document.parseHTML,'parseHTML',1],[Document.parseHTMLUnsafe,'parseHTMLUnsafe',1],[URL.createObjectURL,'createObjectURL',1],[URL.revokeObjectURL,'revokeObjectURL',1],[Response.json,'json',1],[Response.error,'error',0],[Response.redirect,'redirect',1]];
            for(const [method,name,length] of methods){
                check(method.name===name&&method.length===length,'metadata '+name);
                let converted=false,error;try{new method({toString(){converted=true;return 'classic'},valueOf(){converted=true;return 0}})}catch(e){error=e}
                check(error instanceof TypeError&&!converted,'nonconstructible '+name);
            }
            for(const method of [HTMLScriptElement.supports,ClipboardItem.supports]){
                let missing;try{method()}catch(e){missing=e}check(missing instanceof TypeError,'required argument');
                const marker={};let caught;try{method({toString(){throw marker}})}catch(e){caught=e}check(caught===marker,'original conversion exception');
                let symbol;try{method(Symbol())}catch(e){symbol=e}check(symbol instanceof TypeError,'DOMString symbol rejection');
            }
            let calls=0;check(ClipboardItem.supports.call(null,{toString(){calls++;return 'text/plain'}})&&calls===1,'borrowed clipboard native conversion');
            check(HTMLScriptElement.supports.call({},'speculationrules'),'borrowed script support');
            check(Response.error.call(null).type==='error','borrowed error factory');
            const json=Response.json.call(null,{ok:true});check(json.status===200&&json.headers.get('content-type')==='application/json','borrowed JSON factory');
            const redirect=Response.redirect.call(null,'https://example.test/path',307);check(redirect.status===307&&redirect.headers.get('location')==='https://example.test/path','borrowed redirect factory');
            const blobURL=URL.createObjectURL.call(null,new Blob(['body']));check(blobURL.startsWith('blob:'),'borrowed object URL factory');URL.revokeObjectURL.call(null,blobURL);

            const reason={};check(AbortSignal.abort.call(null,reason).reason===reason&&!AbortSignal.any.call(null,[]).aborted,'borrowed signal factories');
            check(Document.parseHTML.call(null,'<b>safe</b>').body.textContent==='safe','borrowed safe parser');
            check(Document.parseHTMLUnsafe.call(null,'<b>unsafe</b>').body.textContent==='unsafe','borrowed unsafe parser');
            const getter=Object.getOwnPropertyDescriptor(PerformanceObserver,'supportedEntryTypes').get;
            check(Object.isFrozen(getter.call(null))&&getter.call({}).includes('resource'),'static getter no instance brand');
            return true;
        })()"#).is_true());
    }
    #[test]
    fn write_text_reentrant_document_replacement_cannot_change_clipboard() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>old</p>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://clipboard.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let backend = Arc::new(Backend::default());
        backend.allowed.store(true, Ordering::SeqCst);
        state.set_clipboard_backend(backend.clone());
        let promise=v8::Local::<v8::Promise>::try_from(eval(scope,"navigator.clipboard.writeText({toString(){document.open();document.write('<p>replacement</p>');document.close();return 'stale'}})")).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert!(backend.data.lock().unwrap().items.is_empty());
    }
    #[test]
    fn async_clipboard_rechecks_native_policy_after_promised_representation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://clipboard.test"),
            document_generation: 1,
            frame_id: None,
        }));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let backend = Arc::new(Backend::default());
        state.set_clipboard_backend(backend.clone());
        let promise = v8::Local::<v8::Promise>::try_from(eval(
            scope,
            "navigator.clipboard.writeText('denied')",
        ))
        .unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert!(backend.data.lock().unwrap().items.is_empty());
        backend.allowed.store(true, Ordering::SeqCst);
        let promise = v8::Local::<v8::Promise>::try_from(eval(
            scope,
            "navigator.clipboard.writeText('exact')",
        ))
        .unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        assert_eq!(
            backend.data.lock().unwrap().plain_text(),
            &"exact".encode_utf16().collect::<Vec<_>>()
        );
        eval(
            scope,
            "globalThis.finish=null;globalThis.pending=navigator.clipboard.write([new ClipboardItem({'text/plain':new Promise(resolve=>finish=resolve)})]);",
        );
        backend.allowed.store(false, Ordering::SeqCst);
        eval(scope, "finish(new Blob(['late'],{type:'text/plain'}))");
        scope.perform_microtask_checkpoint();
        let promise = v8::Local::<v8::Promise>::try_from(eval(scope, "pending")).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert_eq!(
            backend.data.lock().unwrap().plain_text(),
            &"exact".encode_utf16().collect::<Vec<_>>()
        );
        assert!(eval(scope,"document.queryCommandSupported('paste') && !document.queryCommandEnabled('paste') && !document.execCommand('paste')").is_true());
    }
    #[test]
    fn clipboard_item_keeps_frozen_types_and_converts_promised_text_to_blob() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>text</p>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::RealmDescriptor {
            id: RealmId(0),
            origin: crate::origin::Origin::from_url("https://clipboard.test"),
            document_generation: 1,
            frame_id: None,
        }));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,"globalThis.item=new ClipboardItem({'text/plain':Promise.resolve('hello')});item instanceof ClipboardItem && Object.isFrozen(item.types) && item.types[0]==='text/plain' && ClipboardItem.supports('image/png') && !ClipboardItem.supports('application/unknown')").is_true());
        let promise =
            v8::Local::<v8::Promise>::try_from(eval(scope, "item.getType('text/plain')")).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        let value = promise.result(scope);
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        let id = stateful_id(scope, object).unwrap();
        let state = context.get_slot::<V8BindingState>().unwrap();
        let record = platform_object(&state, id).unwrap();
        let PlatformObjectRecord::Blob(blob) = &*record else {
            panic!("native Blob")
        };
        assert_eq!(blob.bytes, b"hello");
        assert_eq!(blob.media_type, "text/plain");
    }
}

fn item_attribute(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let key = item_key(scope);
    let Some(entries) = args
        .this()
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        throw_type_error(scope, "Illegal ClipboardItem receiver");
        return;
    };
    if args.data().int32_value(scope) == Some(1) {
        let name = v8::String::new(scope, "oryn.native.clipboard.item.style").unwrap();
        let key = v8::Private::for_api(scope, Some(name));
        if let Some(value) = args
            .this()
            .get_private(scope, key)
            .filter(|value| value.is_string())
        {
            rv.set(value);
        } else if let Ok(value) = v8_string(scope, "unspecified") {
            rv.set(value);
        }
        return;
    }
    let key_name = v8::String::new(scope, "oryn.native.clipboard.item.types").unwrap();
    let key = v8::Private::for_api(scope, Some(key_name));
    if let Some(cached) = args
        .this()
        .get_private(scope, key)
        .filter(|value| value.is_array())
    {
        rv.set(cached);
        return;
    }
    let names = v8::Array::new(scope, (entries.length() / 2) as i32);
    for index in 0..entries.length() / 2 {
        if let Some(name) = entries.get_index(scope, index * 2) {
            names.set_index(scope, index, name);
        }
    }
    names.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    args.this().set_private(scope, key, names.into());
    rv.set(names.into());
}

fn sanitize_png(bytes: &[u8]) -> Result<Vec<u8>, V8BindingError> {
    if bytes.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
        return Err(failure("QuotaExceededError"));
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| failure("DataError"))?;
    let mut output = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| failure("DataError"))?;
    let bytes = output.into_inner();
    if bytes.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
        return Err(failure("QuotaExceededError"));
    }
    Ok(bytes)
}

fn illegal_clipboard(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    throw_type_error(scope, "Illegal constructor");
}

#[cfg(test)]
mod binary_tests {
    use super::*;
    #[test]
    fn png_clipboard_rejects_forged_format_and_preserves_actual_pixels() {
        assert!(sanitize_png(b"not a PNG").is_err());
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let clean = sanitize_png(encoded.get_ref()).unwrap();
        let decoded = image::load_from_memory_with_format(&clean, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded, image);
        assert!(sanitize_png(&vec![0; crate::clipboard::MAX_CLIPBOARD_BYTES + 1]).is_err());
    }
}
