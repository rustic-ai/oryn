//! Location URL components and navigation requests owned by the native realm.
use super::*;
#[path = "location_cross_origin.rs"]
mod cross_origin;

type Result<T> = std::result::Result<T, V8BindingError>;

pub(super) fn configure(scope: &mut v8::PinScope, instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_immutable_proto();
    crate::v8_context_bridge::set_location_structural_semantics(scope, instance);
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new().getter(named_getter),
    );
    cross_origin::configure(scope, instance);
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
    let target = global
        .get_creation_context(scope)
        .ok_or_else(|| error("SecurityError"))?;
    let entry = scope.get_entered_or_microtask_context();
    if entry
        .get_security_token(scope)
        .strict_equals(target.get_security_token(scope))
    {
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
    let global = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| error("SecurityError"))?;
    // A context can receive a replacement Document. Window.location remains
    // unforgeable, so reinstall the same accessors, whose callbacks resolve the
    // current binding state. The private edge is traced with this Window.
    let key = v8::String::new(scope, "oryn.location.accessors")
        .ok_or(V8BindingError::Allocation("Window.location accessor key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    let cached = global
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (getter, setter) = if cached.is_undefined() {
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
        let pair = v8::Array::new_with_elements(scope, &[getter.into(), setter.into()]);
        if global.set_private(scope, key, pair.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        (getter, setter)
    } else {
        let pair = cached
            .try_cast::<v8::Array>()
            .map_err(|_| V8BindingError::Allocation("Window.location accessors"))?;
        let getter = pair
            .get_index(scope, 0)
            .and_then(|value| value.try_cast::<v8::Function>().ok())
            .ok_or(V8BindingError::Allocation("Window.location getter"))?;
        let setter = pair
            .get_index(scope, 1)
            .and_then(|value| value.try_cast::<v8::Function>().ok())
            .ok_or(V8BindingError::Allocation("Window.location setter"))?;
        (getter, setter)
    };
    let name = v8::String::new(scope, "location")
        .ok_or(V8BindingError::Allocation("Window.location name"))?;
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
    if !state.execution_retired() {
        check(scope, state)?;
    }
    if member == "ancestorOrigins" {
        return location_ancestors::get(scope, state, id);
    }
    let url = if state.execution_retired() {
        Url::parse("about:blank").expect("static Location fallback")
    } else {
        state.location_url.borrow().clone()
    };
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
    // URL parsing uses entry settings. A same-origin function from another
    // realm changes the incumbent navigation source, not this parsing base.
    let caller = scope
        .get_entered_or_microtask_context()
        .get_slot::<V8BindingState>();
    let source = caller.as_deref().unwrap_or(state);
    let base =
        Url::parse(&document_base_url(scope, source, true)?).map_err(|_| error("SyntaxError"))?;
    let encoding = hyperlinks::document_encoding(source, source.document_root())?;
    hyperlinks::parse(value, Some(&base), encoding).map_err(|_| error("SyntaxError"))
}
pub(super) fn navigate(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    url: Url,
    replace: bool,
    reload: bool,
) -> Result<()> {
    navigate_impl(scope, state, url, replace, reload, false)
}
pub(super) fn navigate_from_api(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    url: Url,
    replace: bool,
    reload: bool,
) -> Result<()> {
    navigate_impl(scope, state, url, replace, reload, true)
}
fn navigate_impl(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    url: Url,
    replace: bool,
    reload: bool,
    api_dispatched: bool,
) -> Result<()> {
    // URL conversion may execute author code and retire either document.
    // Never publish a navigation or speculative URL from a retired target.
    if state.execution_retired() || state.is_unloading() || state.navigation_discard_pending() {
        return Ok(());
    }
    let document = navigation_api::AdmissionDocument::capture(state);
    let caller = crate::v8_context_bridge::incumbent_slot::<V8BindingState>(scope);
    if caller
        .as_ref()
        .is_some_and(|caller| caller.execution_retired())
    {
        return Err(error("SecurityError"));
    }
    // HTML Location-object navigate replaces an unfinished target document
    // unless the incumbent source currently has transient user activation.
    // Freeze this before navigate callbacks; readyState=complete alone is not
    // the completely-loaded boundary (load listeners still run before it).
    let replace = replace
        || (!api_dispatched
            && !state.document_completely_loaded()
            && !caller
                .as_deref()
                .unwrap_or(state)
                .has_transient_user_activation());
    // Cross-origin javascript: navigation is discarded without running source.
    // The allowed href/replace operations themselves must not throw merely
    // because the URL has this scheme (the pinned cross-origin tests use it).
    if url.scheme() == "javascript" {
        let global = state
            .global_object
            .to_local(scope)
            .ok_or_else(|| error("SecurityError"))?;
        if !crate::v8_context_bridge::can_access_window(scope, global) {
            return Ok(());
        }
    }
    let backend = state.location_navigation_backend.borrow().clone();
    let script_authority = if let Some(backend) = backend {
        let source = caller.as_deref().ok_or_else(|| error("SecurityError"))?;
        let target = state.native_worker_creator()?;
        // A same-origin auxiliary can expose its real Location object to its
        // opener. Authorize on the incumbent source's endpoint, not on the
        // destination endpoint with a creator belonging to another page.
        let source_backend = source
            .location_navigation_backend
            .borrow()
            .clone()
            .ok_or_else(|| error("SecurityError"))?;
        let authority = if let Some(page) = backend.target_page() {
            source_backend.authorize_selected(
                source.native_worker_creator()?,
                page,
                target.clone(),
                url.to_string(),
                source.request_initiator_url()?,
                source.document_referrer_policy(),
            )
        } else {
            source_backend.authorize(
                source.native_worker_creator()?,
                target.clone(),
                url.to_string(),
                source.request_initiator_url()?,
                source.document_referrer_policy(),
            )
        }
        .map_err(|_| error("SecurityError"))?
        .ok_or_else(|| error("SecurityError"))?;
        Some(crate::navigation::ScriptNavigationAdmission { authority, target })
    } else {
        None
    };
    if !api_dispatched {
        let owner = state
            .global_object
            .to_local(scope)
            .and_then(|global| binding_state_for_receiver(scope, global))
            .ok_or_else(|| error("SecurityError"))?;
        let context = owner
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or_else(|| error("SecurityError"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        if !navigation_api::begin_location_navigation(scope, &owner, &url, replace, reload)? {
            return Ok(());
        }
        if !document.current(state) {
            return Ok(());
        }
    }
    state
        .pending_location_navigation
        .borrow_mut()
        .replace(LocationNavigation {
            execution_observer: None,
            navigation_operation: navigation_api::capture_current_native_operation(state),
            download: None,
            navigation_destination: if api_dispatched {
                None
            } else {
                state.pending_navigation_destination()
            },
            script_authority,
            form_authority: None,
            url: url.to_string(),
            replace,
            reload,
            post: None,
            referrer_policy: None,
        });
    *state.location_url.borrow_mut() = url;
    Ok(())
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<()> {
    brand(state, id)?;
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    // Web IDL conversion still happens, but a Location without a relevant
    // document performs no URL parsing, mutation or navigation.
    if state.execution_retired() {
        return Ok(());
    }
    if member != "href" {
        check(scope, state)?;
    }
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
        "search" => {
            let fragment = url.fragment().map(str::to_owned);
            url.set_fragment(None);
            url.set_query(None);
            if !value.is_empty() {
                let query = value
                    .strip_prefix('?')
                    .unwrap_or(&value)
                    .replace('#', "%23");
                let encoding = hyperlinks::document_encoding(state, state.document_root())?;
                url = hyperlinks::parse(&format!("{url}?{query}"), None, encoding)
                    .map_err(|_| error("SyntaxError"))?;
            }
            url.set_fragment(fragment.as_deref());
        }
        "hash" => {
            let previous = url.fragment().map(str::to_owned);
            url::quirks::set_hash(&mut url, &value);
            if url.fragment() == previous.as_deref() {
                return Ok(());
            }
        }
        _ => return Err(error("TypeError")),
    }
    navigate(scope, state, url, false, false)
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    brand(state, id)?;
    if !matches!(member, "replace" | "assign") && !state.execution_retired() {
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
            if state.execution_retired() {
                return Ok(None);
            }
            if member == "assign" {
                check(scope, state)?;
            }
            let url = resolve(scope, state, &value)?;
            navigate(scope, state, url, member == "replace", false)?;
        }
        "reload" => {
            let url = state.location_url.borrow().clone();
            navigate(scope, state, url, true, true)?;
        }
        _ => return Err(error("TypeError")),
    }
    Ok(None)
}

#[cfg(test)]
mod installation_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn same_window_reinstallation_preserves_unforgeable_accessors_and_updates_location() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let first = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><p>old</p>")
                    .into_runtime()
                    .unwrap(),
            )),
            RealmId(321),
            1,
        )
        .unwrap();
        first.commit_location("https://example.test/old").unwrap();
        assert!(eval(scope,"globalThis.oldLocation=location;globalThis.oldDocument=document;globalThis.oldDescriptor=Object.getOwnPropertyDescriptor(globalThis,'location');oldDescriptor.configurable===false&&oldDescriptor.enumerable===true").is_true());
        first.retire_execution();
        first.refresh_connected_wrapper_roots(scope);
        let second = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><p>new</p>")
                    .into_runtime()
                    .unwrap(),
            )),
            RealmId(321),
            2,
        )
        .unwrap();
        second.commit_location("https://example.test/new").unwrap();
        assert!(eval(scope,"(()=>{const current=Object.getOwnPropertyDescriptor(globalThis,'location');return current.get===oldDescriptor.get&&current.set===oldDescriptor.set&&!current.configurable&&current.enumerable&&oldDescriptor.get.call(globalThis)===location&&location.href==='https://example.test/new'&&oldLocation!==location&&oldLocation.href==='about:blank'&&oldDocument.body.textContent==='old'&&document.body.textContent==='new'&&!Reflect.deleteProperty(globalThis,'location')&&!Reflect.defineProperty(globalThis,'location',{value:1})})()").is_true());
    }
    #[test]
    fn authored_nonconfigurable_location_is_not_adopted_as_native_accessor() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        eval(
            scope,
            "Object.defineProperty(globalThis,'location',{value:17,configurable:false})",
        );
        let result = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            RealmId(322),
            1,
        );
        assert!(matches!(
            result,
            Err(V8BindingError::Allocation("Window.location property"))
        ));
        assert_eq!(eval(scope, "location").int32_value(scope), Some(17));
    }
}
