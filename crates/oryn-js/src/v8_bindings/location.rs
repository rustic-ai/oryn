//! Location URL components and navigation requests owned by the native realm.
use super::*;

type Result<T> = std::result::Result<T, V8BindingError>;

pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_immutable_proto();
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new().getter(named_getter),
    );
}
fn named_getter(
    _scope: &mut v8::PinScope,
    _name: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    _rv: v8::ReturnValue,
) -> v8::Intercepted {
    v8::Intercepted::kNo
}
fn error(member: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Location".into(),
        member: member.into(),
    }
}
fn check(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<()> {
    let global = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| error("SecurityError"))?;
    if crate::v8_context_bridge::can_access_window(scope, global) {
        Ok(())
    } else {
        Err(error("SecurityError"))
    }
}
fn brand(state: &V8BindingState, id: u64) -> Result<()> {
    if matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::Location
    ) {
        Ok(())
    } else {
        Err(error("TypeError"))
    }
}
pub(super) fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Object>> {
    let id = if let Some(id) = state.location_object.get() {
        id
    } else {
        let id = register_platform_object(state, PlatformObjectRecord::Location);
        state.location_object.set(Some(id));
        id
    };
    let object = new_platform_object(scope, state, "Location", id)?;
    let global = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| error("SecurityError"))?;
    let key = v8::String::new(scope, "oryn.location")
        .ok_or(V8BindingError::Allocation("Location root key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    global
        .set_private(scope, key, object.into())
        .ok_or(V8BindingError::Allocation("Location root"))?;
    Ok(object)
}
pub(super) fn install(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<()> {
    let object = object(scope, state)?;
    let name =
        v8::String::new(scope, "valueOf").ok_or(V8BindingError::Allocation("Location valueOf"))?;
    let ordinary = v8::Object::new(scope);
    let value = ordinary
        .get(scope, name.into())
        .ok_or(V8BindingError::Allocation("Object valueOf"))?;
    let immutable = || {
        v8::PropertyAttribute::READ_ONLY
            | v8::PropertyAttribute::DONT_DELETE
            | v8::PropertyAttribute::DONT_ENUM
    };
    object
        .define_own_property(scope, name.into(), value, immutable())
        .ok_or(V8BindingError::Allocation("Location valueOf property"))?;
    let key = v8::Symbol::get_to_primitive(scope);
    let value = v8::undefined(scope);
    object
        .define_own_property(scope, key.into(), value.into(), immutable())
        .ok_or(V8BindingError::Allocation("Location primitive property"))?;
    let member = GENERATED_INTERFACES
        .iter()
        .find(|i| i.name == "Window")
        .and_then(|i| i.members.iter().find(|m| m.name == "location"))
        .ok_or(V8BindingError::Allocation("Window.location metadata"))?;
    let data = v8::Integer::new(scope, member.id.0 as i32);
    let getter = v8::FunctionTemplate::builder(generated_attribute_getter)
        .data(data.into())
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .get_function(scope)
        .ok_or(V8BindingError::Allocation("Window.location getter"))?;
    let setter = v8::FunctionTemplate::builder(generated_attribute_setter)
        .data(data.into())
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .get_function(scope)
        .ok_or(V8BindingError::Allocation("Window.location setter"))?;
    let name = v8::String::new(scope, "location")
        .ok_or(V8BindingError::Allocation("Window.location name"))?;
    let global = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| error("SecurityError"))?;
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
    descriptor.set_configurable(false);
    descriptor.set_enumerable(true);
    if global.define_property(scope, name.into(), &descriptor) != Some(true) {
        return Err(V8BindingError::Allocation("Window.location property"));
    }
    Ok(())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    brand(state, id)?;
    check(scope, state)?;
    let url = state.location_url.borrow();
    let value = match member {
        "href" => url::quirks::href(&url),
        "origin" => return v8_string(scope, &url::quirks::origin(&url)),
        "protocol" => url::quirks::protocol(&url),
        "host" => url::quirks::host(&url),
        "hostname" => url::quirks::hostname(&url),
        "port" => url::quirks::port(&url),
        "pathname" => url::quirks::pathname(&url),
        "search" => url::quirks::search(&url),
        "hash" => url::quirks::hash(&url),
        _ => return Err(error("TypeError")),
    };
    v8_string(scope, value)
}
fn resolve(scope: &mut v8::PinScope, state: &V8BindingState, value: &str) -> Result<Url> {
    let caller = crate::v8_context_bridge::incumbent_slot::<V8BindingState>(scope);
    let base = document_base_url(scope, caller.as_deref().unwrap_or(state), true)?;
    Url::parse(&base)
        .and_then(|base| base.join(value))
        .map_err(|_| error("SyntaxError"))
}
fn navigate(state: &V8BindingState, url: Url, replace: bool, reload: bool) {
    state
        .pending_location_navigation
        .borrow_mut()
        .replace(LocationNavigation {
            url: url.to_string(),
            replace,
            reload,
            post: None,
            referrer_policy: None,
        });
    *state.location_url.borrow_mut() = url;
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<()> {
    brand(state, id)?;
    if member != "href" {
        check(scope, state)?;
    }
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let mut url = state.location_url.borrow().clone();
    match member {
        "href" => url = resolve(scope, state, &value)?,
        "protocol" => {
            url::quirks::set_protocol(&mut url, &value).map_err(|_| error("SyntaxError"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Ok(());
            }
        }
        "host" => {
            let _ = url::quirks::set_host(&mut url, &value);
        }
        "hostname" => {
            let _ = url::quirks::set_hostname(&mut url, &value);
        }
        "port" => {
            let _ = url::quirks::set_port(&mut url, &value);
        }
        "pathname" => url::quirks::set_pathname(&mut url, &value),
        "search" => url::quirks::set_search(&mut url, &value),
        "hash" => {
            let previous = url.fragment().map(str::to_owned);
            url::quirks::set_hash(&mut url, &value);
            if url.fragment() == previous.as_deref() {
                return Ok(());
            }
        }
        _ => return Err(error("TypeError")),
    }
    navigate(state, url, false, false);
    Ok(())
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    brand(state, id)?;
    if member != "replace" {
        check(scope, state)?;
    }
    match member {
        "toString" => return get(scope, state, id, "href").map(Some),
        "assign" | "replace" => {
            let value = args
                .get(0)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            let url = resolve(scope, state, &value)?;
            navigate(state, url, member == "replace", false);
        }
        "reload" => {
            let url = state.location_url.borrow().clone();
            navigate(state, url, true, true);
        }
        _ => return Err(error("TypeError")),
    }
    Ok(None)
}
