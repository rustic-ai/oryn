//! Native Clear Key access selection. Codec admission is shared with URL media;
//! an unsupported codec cannot become supported through a key-system request.
use super::*;
use serde_json::{Value, json};
struct AccessPrototype(v8::Weak<v8::Object>);
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "MediaKeySystemAccess".into(),
        member: name.into(),
    }
}
fn private<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Private> {
    let key = v8::String::new(scope, name).unwrap();
    v8::Private::for_api(scope, Some(key))
}
fn field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).unwrap();
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn sequence<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
    mut convert: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Value>,
    ) -> Result<T, V8BindingError>,
) -> Result<Vec<T>, V8BindingError> {
    let value = v8::Local::new(scope, value);
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))?;
    let symbol = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, symbol.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| error("TypeError"))?;
    let mut values = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        if values.len() >= 1024 {
            return Err(error("QuotaExceededError"));
        }
        values.push(convert(scope, value)?);
        Ok(())
    })?;
    Ok(values)
}
fn string(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String, V8BindingError> {
    Ok(value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope))
}
fn strings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<String>, V8BindingError> {
    sequence(scope, value, string)
}
fn configuration(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Value, V8BindingError> {
    if !value.is_object() && !value.is_null_or_undefined() {
        return Err(error("TypeError"));
    }
    let object = if value.is_null_or_undefined() {
        v8::Object::new(scope)
    } else {
        value
            .to_object(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    let mut output = serde_json::Map::new();
    // Web IDL dictionary members convert in lexical order, before policy selection.
    for name in [
        "audioCapabilities",
        "distinctiveIdentifier",
        "initDataTypes",
        "label",
        "persistentState",
        "sessionTypes",
        "videoCapabilities",
    ] {
        let value = field(scope, object, name)?;
        if value.is_undefined() {
            continue;
        }
        let converted = match name {
            "audioCapabilities" | "videoCapabilities" => {
                Value::Array(sequence(scope, value, |scope, value| {
                    let object = value
                        .to_object(scope)
                        .ok_or(V8BindingError::JavaScriptException)?;
                    let mut cap = serde_json::Map::new();
                    for name in ["contentType", "encryptionScheme", "robustness"] {
                        let value = field(scope, object, name)?;
                        if !value.is_undefined() {
                            cap.insert(
                                name.into(),
                                if name == "encryptionScheme" && value.is_null() {
                                    Value::Null
                                } else {
                                    Value::String(string(scope, value)?)
                                },
                            );
                        }
                    }
                    Ok(Value::Object(cap))
                })?)
            }
            "initDataTypes" | "sessionTypes" => json!(strings(scope, value)?),
            _ => {
                let text = string(scope, value)?;
                if matches!(name, "distinctiveIdentifier" | "persistentState")
                    && !matches!(text.as_str(), "required" | "optional" | "not-allowed")
                {
                    return Err(error("TypeError"));
                }
                Value::String(text)
            }
        };
        output.insert(name.into(), converted);
    }
    Ok(Value::Object(output))
}
fn select(mut config: Value, mut codec: impl FnMut(&str) -> bool) -> Option<Value> {
    let map = config.as_object_mut()?;
    if map.get("distinctiveIdentifier").and_then(Value::as_str) == Some("required")
        || map.get("persistentState").and_then(Value::as_str) == Some("required")
    {
        return None;
    }
    let sessions = map
        .get("sessionTypes")
        .cloned()
        .unwrap_or(json!(["temporary"]));
    if sessions
        .as_array()?
        .iter()
        .any(|value| value.as_str() != Some("temporary"))
    {
        return None;
    }
    map.insert("sessionTypes".into(), sessions);
    map.insert("distinctiveIdentifier".into(), json!("not-allowed"));
    map.insert("persistentState".into(), json!("not-allowed"));
    map.entry("label").or_insert(json!(""));
    if let Some(types) = map.get_mut("initDataTypes") {
        let types = types.as_array_mut()?;
        if !types.is_empty() {
            types.retain(|value| matches!(value.as_str(), Some("webm" | "cenc" | "keyids")));
            if types.is_empty() {
                return None;
            }
        }
    }
    let mut capabilities = 0;
    for (name, prefix) in [
        ("audioCapabilities", "audio/"),
        ("videoCapabilities", "video/"),
    ] {
        if let Some(caps) = map.get_mut(name) {
            let caps = caps.as_array_mut()?;
            let requested = !caps.is_empty();
            caps.retain(|cap| {
                let mime = cap.get("contentType").and_then(Value::as_str).unwrap_or("");
                let robustness = cap.get("robustness").and_then(Value::as_str).unwrap_or("");
                let scheme = cap.get("encryptionScheme").and_then(Value::as_str);
                mime.to_ascii_lowercase().starts_with(prefix)
                    && robustness.is_empty()
                    && scheme.is_none_or(|scheme| scheme == "cenc")
                    && codec(mime)
            });
            if requested && caps.is_empty() {
                return None;
            }
            capabilities += caps.len();
        }
    }
    if capabilities == 0 {
        return None;
    }
    Some(config)
}
fn illegal(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor")
}
fn access_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::String>, V8BindingError> {
    let key = private(scope, "oryn.eme.configuration");
    object
        .get_private(scope, key)
        .and_then(|v| v8::Local::<v8::String>::try_from(v).ok())
        .ok_or(error("TypeError"))
}
fn key_system(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match access_data(scope, args.this()).and_then(|_| v8_string(scope, "org.w3.clearkey")) {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn get_configuration(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match access_data(scope, args.this())
        .and_then(|data| v8::json::parse(scope, data).ok_or(V8BindingError::JavaScriptException))
    {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn request(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = binding_state_for_receiver(scope, args.this()).ok_or(error("TypeError"))?;
        let brand = private(scope, "oryn.native.navigator.platform");
        if args
            .this()
            .get_private(scope, brand)
            .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
            .map(|v| v.u64_value())
            != Some((state.instance_id, true))
        {
            return Err(error("TypeError"));
        }
        let key_system = string(scope, args.get(0))?;
        let configurations = sequence(scope, args.get(1), configuration)?;
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or(V8BindingError::Allocation("EME access promise"))?;
        let promise = resolver.get_promise(scope);
        let selected = (|| -> Result<Value, V8BindingError> {
            if key_system.is_empty() || configurations.is_empty() {
                return Err(error("TypeError"));
            }
            if state.execution_retired()
                || state.execution_suspended()
                || !state.secure_context.get()
            {
                return Err(error("InvalidStateError"));
            }
            state.refresh_native_container_policy()?;
            if !state.native_permissions_report_only_allows("encrypted-media") {
                reporting::permission_native(&state, "encrypted-media", "report");
            }
            if !state.native_permissions_policy_allows("encrypted-media") {
                reporting::permission_native(&state, "encrypted-media", "enforce");
                return Err(error("SecurityError"));
            }
            if key_system != "org.w3.clearkey" {
                return Err(error("NotSupportedError"));
            }
            configurations
                .into_iter()
                .find_map(|config| {
                    select(config, |mime| {
                        matches!(
                            native_platform::media_request(
                                &state,
                                crate::media_capture::Request::UrlCanPlay { mime: mime.into() }
                            ),
                            Ok(crate::media_capture::Response::UrlSupport(true))
                        )
                    })
                })
                .ok_or(error("NotSupportedError"))
        })();
        match selected {
            Ok(config) => {
                let prototype = scope
                    .get_current_context()
                    .get_slot::<AccessPrototype>()
                    .ok_or(error("InvalidStateError"))?;
                let prototype = prototype
                    .0
                    .to_local(scope)
                    .ok_or(error("InvalidStateError"))?;
                let access = v8::Object::new(scope);
                access
                    .set_prototype(scope, prototype.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                let data = v8::String::new(scope, &config.to_string())
                    .ok_or(V8BindingError::Allocation("EME configuration"))?;
                let key = private(scope, "oryn.eme.configuration");
                access
                    .set_private(scope, key, data.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                resolver
                    .resolve(scope, access.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
            Err(failure) => {
                let name = match &failure {
                    V8BindingError::Unsupported { member, .. } => member.as_str(),
                    _ => "NotSupportedError",
                };
                let exception = if name == "TypeError" {
                    let message =
                        v8::String::new(scope, "Empty key system or configurations").unwrap();
                    v8::Exception::type_error(scope, message)
                } else {
                    dom_exception_value(scope, name)?
                };
                resolver
                    .reject(scope, exception)
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
        }
        Ok(promise.into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    navigator: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let constructor = v8::Function::new(scope, illegal)
        .ok_or(V8BindingError::Allocation("MediaKeySystemAccess"))?;
    let prototype = field(scope, constructor.into(), "prototype")?
        .to_object(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let function = v8::Function::new(scope, get_configuration).unwrap();
    let key = v8::String::new(scope, "getConfiguration").unwrap();
    prototype.create_data_property(scope, key.into(), function.into());
    let getter = v8::Function::new(scope, key_system).unwrap();
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "keySystem").unwrap();
    prototype.define_property(scope, key.into(), &descriptor);
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "MediaKeySystemAccess").unwrap();
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
    context
        .global(scope)
        .create_data_property(scope, name.into(), constructor.into());
    let retained = context_intrinsics::retain_object_in_context(
        scope,
        context,
        "media-key-system-access-prototype",
        prototype,
    )?;
    context.set_slot(Rc::new(AccessPrototype(retained)));
    let function = v8::Function::new(scope, request).unwrap();
    let key = v8::String::new(scope, "requestMediaKeySystemAccess").unwrap();
    navigator.create_data_property(scope, key.into(), function.into());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clearkey_configuration_requires_real_codec_admission_and_no_robustness_or_identity() {
        let config = json!({"initDataTypes":["unknown","webm"],"videoCapabilities":[{"contentType":"video/webm;codecs=vp8"}]});
        assert!(select(config.clone(), |_| false).is_none());
        let selected = select(config.clone(), |_| true).unwrap();
        assert_eq!(selected["initDataTypes"], json!(["webm"]));
        assert_eq!(selected["persistentState"], "not-allowed");
        for (name, value) in [
            ("distinctiveIdentifier", json!("required")),
            ("persistentState", json!("required")),
            ("sessionTypes", json!(["persistent-license"])),
        ] {
            let mut invalid = config.clone();
            invalid[name] = value;
            assert!(select(invalid, |_| true).is_none())
        }
        let mut invalid = config;
        invalid["videoCapabilities"][0]["robustness"] = json!("HW_SECURE_ALL");
        assert!(select(invalid, |_| true).is_none());
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Messages;
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            _: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    struct Backend(AtomicBool);
    impl crate::platform_control::Backend for Backend {
        fn request(
            &self,
            _: crate::worker_wire::Creator,
            request: crate::platform_control::Request,
        ) -> Result<crate::platform_control::Response, String> {
            match request {
                crate::platform_control::Request::Media {
                    request: crate::media_capture::Request::UrlCanPlay { .. },
                } => Ok(crate::platform_control::Response::Media {
                    response: crate::media_capture::Response::UrlSupport(
                        self.0.load(Ordering::Relaxed),
                    ),
                }),
                _ => Err("unexpected EME request".into()),
            }
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn encrypted_media_policy_reports_and_real_codec_admission_are_independent() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.secure_context.set(true);
        let global = context.global(scope);
        let navigator = field(scope, global, "navigator")
            .unwrap()
            .to_object(scope)
            .unwrap();
        install(scope, context, navigator).unwrap();
        state.set_message_transport(Arc::new(Messages));
        let backend = Arc::new(Backend(AtomicBool::new(true)));
        state.set_native_platform_backend(backend.clone());
        eval(
            scope,
            "globalThis.reports=new ReportingObserver(()=>{},{types:['permissions-policy-violation']});reports.observe();globalThis.config=[{initDataTypes:['webm'],videoCapabilities:[{contentType:'video/webm;codecs=vp8'}]}];globalThis.outcome='';",
        );
        state.set_native_permissions_policy(Rc::new(|_| false));
        eval(
            scope,
            "navigator.requestMediaKeySystemAccess('org.w3.clearkey',config).catch(e=>outcome=e.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"outcome==='SecurityError' && reports.takeRecords().some(r=>r.body.featureId==='encrypted-media'&&r.body.disposition==='enforce')").is_true());
        state.set_native_permissions_policy(Rc::new(|_| true));
        state.set_native_permissions_report_only(Rc::new(|_| false));
        eval(
            scope,
            "navigator.requestMediaKeySystemAccess('org.w3.clearkey',config).then(v=>{globalThis.access=v;outcome='ok'})",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"outcome==='ok' && access instanceof MediaKeySystemAccess && access.keySystem==='org.w3.clearkey' && reports.takeRecords().some(r=>r.body.disposition==='report')").is_true());
        assert!(eval(scope,"(()=>{let c=access.getConfiguration();c.videoCapabilities.length=0;return access.getConfiguration().videoCapabilities.length===1})()").is_true());
        backend.0.store(false, Ordering::Relaxed);
        eval(
            scope,
            "navigator.requestMediaKeySystemAccess('org.w3.clearkey',config).catch(e=>outcome=e.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "outcome==='NotSupportedError'").is_true());
    }
}

#[cfg(test)]
mod prototype_retention_tests {
    use super::*;
    #[test]
    fn secure_access_prototype_is_traced_after_constructor_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, prototype, anchor) = {
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(993), 1).unwrap();
            let prototype = context.get_slot::<AccessPrototype>().unwrap().0.clone();
            assert!(
                super::super::test_eval::eval(scope, "MediaKeySystemAccess.prototype")
                    .strict_equals(prototype.to_local(scope).unwrap().into())
            );
            super::super::test_eval::eval(scope, "MediaKeySystemAccess=function replacement(){};");
            let object = v8::Object::new(scope);
            object.set_prototype(scope, prototype.to_local(scope).unwrap().into());
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, context),
                prototype,
                v8::Global::new(scope, object),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            assert!(context.to_local(handles).is_some());
            let retained_context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, retained_context);
            let object = v8::Local::new(scope, &anchor);
            assert!(
                object
                    .get_prototype(scope)
                    .unwrap()
                    .strict_equals(prototype.to_local(scope).unwrap().into())
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(context.to_local(handles).is_none());
        assert!(prototype.to_local(handles).is_none());
    }
}
